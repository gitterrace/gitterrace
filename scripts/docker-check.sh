#!/usr/bin/env bash
# The Docker image still serves a mounted folder: lint the Dockerfile with hadolint, build the
# image, then run it against one repository and against a folder of two, both mounted
# read-only, and fetch the home page and a repository page. A second start on the same cache
# volume reuses the index. Needs docker.
#
#   scripts/docker-check.sh              hadolint, build, serve checks
#   scripts/docker-check.sh --self-test  the Dockerfile's fixed rules, no docker
#
# The self-test runs in `make docs-check`. The full run is the docker job of ci.yml.
# DOCKER_JOBS sets the compile jobs of the build (default 2, the same as the Dockerfile, so a
# busy machine stays usable).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
image=gitterrace-check
port=${DOCKER_CHECK_PORT:-18080}
hadolint=hadolint/hadolint:v2.14.0@sha256:
hadolint+=27086352fd5e1907ea2b934eb1023f217c5ae087992eb59fde121dce9c9ff21e

fail() {
  echo "docker-check: $*" >&2
  exit 1
}

# The channel in a file: rust-toolchain.toml form, or the Dockerfile's ARG form.
toolchain_pin() {
  sed -n -e 's/^channel *= *"\(.*\)"/\1/p' -e 's/^ARG RUST_TOOLCHAIN=\(.*\)/\1/p' "$1"
}

# The rules a Dockerfile edit could break without a failing build: the pin, the read-only
# public mode, a non-root user, trusted mounts, and a build context free of local state.
self_test() {
  local df=$root/Dockerfile ig=$root/.dockerignore want got
  want=$(toolchain_pin "$root/rust-toolchain.toml")
  got=$(toolchain_pin "$df")
  [ -n "$want" ] && [ "$want" = "$got" ] ||
    fail "Dockerfile pins '$got', rust-toolchain.toml pins '$want'"
  grep -q '^ENTRYPOINT .*"--public"' "$df" || fail "the entrypoint does not pass --public"
  grep -q '^USER 65532' "$df" || fail "the image does not run as the nonroot user"
  grep -q 'directory = \*' "$df" || fail "mounted repositories are not trusted by git"
  grep -q '^ENV .*XDG_CACHE_HOME=/cache' "$df" || fail "the index cache is not on /cache"
  local from
  while read -r from; do
    [[ $from == *@sha256:* ]] || fail "base image not pinned by digest: $from"
  done < <(sed -n 's/^FROM \([^ ]*\).*/\1/p' "$df")
  grep -q '^FROM ' "$df" || fail "the Dockerfile has no FROM"
  for p in target .git .personal-denylist; do
    grep -qx "$p" "$ig" || fail ".dockerignore lets $p into the build context"
  done
  echo "docker-check self-test: ok (toolchain $want)"
}

if [ "${1:-}" = --self-test ]; then
  self_test
  exit 0
fi
[ "$#" -eq 0 ] || fail "usage: scripts/docker-check.sh [--self-test]"
self_test
command -v docker >/dev/null || fail "needs docker"

tmp=$(mktemp -d)
volume=gitterrace-check-$$
cid=
cleanup() {
  [ -n "$cid" ] && docker rm -f "$cid" >/dev/null 2>&1 || true
  docker volume rm "$volume" >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT

docker run --rm -i "$hadolint" hadolint - <"$root/Dockerfile" || fail "hadolint"
echo "PASS: hadolint"

docker build -q -t "$image" --build-arg JOBS="${DOCKER_JOBS:-2}" "$root" >/dev/null
size=$(docker image inspect -f '{{.Size}}' "$image" | awk '{ printf "%.1f MB", $1 / 1e6 }')
echo "PASS: build ($size)"

# A repository with two commits, made by the host's git, owned by the host's uid.
repo() {
  git init -q -b main "$1"
  echo "# $2" >"$1/README.md"
  git -C "$1" add README.md
  git -C "$1" -c user.name=t -c user.email=t@example.com commit -q -m "first of $2"
  echo more >>"$1/README.md"
  git -C "$1" -c user.name=t -c user.email=t@example.com commit -q -am "second of $2"
}
repo "$tmp/one" one
mkdir "$tmp/many"
repo "$tmp/many/alpha" alpha
repo "$tmp/many/beta" beta

# Starts the image on folder $1 and waits for the home page. Sets cid, and secs to the seconds
# it took.
start() {
  local t0 t1
  t0=$(date +%s.%N 2>/dev/null || date +%s)
  cid=$(docker run -d -p "127.0.0.1:$port:8080" -v "$1":/repos:ro -v "$volume":/cache "$image")
  for _ in $(seq 1 300); do
    curl -fsS -o /dev/null "http://localhost:$port/" 2>/dev/null && break
    sleep 0.1
  done
  t1=$(date +%s.%N 2>/dev/null || date +%s)
  secs=$(awk -v a="$t0" -v b="$t1" 'BEGIN { printf "%.2f", b - a }')
}
stop() {
  docker rm -f "$cid" >/dev/null
  cid=
}
# Fetches path $1 and checks that the page holds text $2.
page() {
  local body
  body=$(curl -fsSL "http://localhost:$port$1") || {
    docker logs "$cid" >&2 || true
    fail "GET $1 failed"
  }
  grep -q -- "$2" <<<"$body" || fail "GET $1 does not show '$2'"
  echo "PASS: GET $1 shows '$2'"
}

start "$tmp/one"
page / "second of one"
# One repository is served as /r/repos/, after the mount point, and / redirects there.
page /r/repos/log "first of one"
page /r/repos/blob/HEAD/README.md "more"
stop
echo "cold start: ${secs}s"
start "$tmp/one"
page / "second of one"
docker logs "$cid" 2>&1 | grep -qi "permission denied" && fail "the cache was not writable"
stop
echo "warm start: ${secs}s"

start "$tmp/many"
page / "alpha"
page / "beta"
page /r/alpha/ "second of alpha"
page /r/beta/log "first of beta"
stop
echo "two repositories: ${secs}s"
echo "docker-check: ok"
