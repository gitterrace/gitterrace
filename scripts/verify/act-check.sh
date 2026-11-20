#!/usr/bin/env bash
# make act-check [REV=main]: run ci.yml's fast tier (the Linux gate, deny, MSRV and the Docker
# check) through act on a clean clone of one commit, and on pass write .git/act-pass/<sha>,
# which scripts/hooks/pre-push requires to push main.
# A clean clone, so an uncommitted file can neither fix nor break the run. Needs act and docker.
# Usage: act-check.sh [rev]
set -uo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
rev=${1:-main}

die() {
  echo "act: $*" >&2
  exit 1
}

command -v act >/dev/null || die "act is not installed"
docker info >/dev/null 2>&1 || die "docker is not running"
sha=$(git -C "$root" rev-parse --verify --quiet "$rev^{commit}") || die "no commit $rev"
tmp=$(mktemp -d) || die "no temp dir"
trap 'rm -rf "$tmp"' EXIT
git clone -q --no-checkout "$root" "$tmp/r" || die "cannot clone"
git -C "$tmp/r" checkout -q "$sha" || die "cannot check out $sha"
echo "act: ci.yml push jobs on ${sha:0:12}"
arch=$(uname -m | sed 's/x86_64/amd64/')
# The hosted runner's user and an init, see deploy/test/act.Dockerfile.
docker build -q -t gitterrace-act -f "$root/deploy/test/act.Dockerfile" "$root/deploy/test" ||
  die "cannot build the runner image"
for job in check-linux deny msrv; do
  (cd "$tmp/r" && act workflow_call -W .github/workflows/lint-and-test.yml -j "$job" \
    --container-architecture "linux/$arch" -P ubuntu-latest=gitterrace-act --pull=false \
    --container-options "--init") ||
    die "ci.yml job $job failed on ${sha:0:12}, main stays unpushed"
done
# The Docker job bind-mounts job paths onto the host daemon, which act cannot give it, so the
# same check runs on the host against the same clone.
(cd "$tmp/r" && bash scripts/verify/docker-check.sh) ||
  die "ci.yml failed on ${sha:0:12}, main stays unpushed"
dir="$(git -C "$root" rev-parse --path-format=absolute --git-common-dir)/act-pass"
mkdir -p "$dir" || die "cannot make $dir"
date -u +%FT%TZ >"$dir/$sha" || die "cannot write the pass record"
echo "act: pass, recorded $dir/$sha"
