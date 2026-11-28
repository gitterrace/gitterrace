#!/usr/bin/env bash
# make release-gate VERSION=x.y.z [RETAG=1]: run what the release workflow runs on the exact
# commit, before the tag leaves this machine, then push main and the tag in one atomic push.
# Refuses a dirty tree or a HEAD that is not main. The steps, with no early exit after the
# changelog, so one run lists every failure:
# 1. The changelog section is regenerated, never taken as written: an old section of x.y.z is
#    removed, make changelog writes it from the commits dated by the bump commit (the kit's
#    release-date.sh: its committer date in UTC, never today), and a change is committed. A
#    hand edit cannot survive, and a gate that runs across midnight changes nothing. The same
#    script gives SOURCE_DATE_EPOCH, exported for every step that builds.
# 2. Both changelog checks against the candidate, never a stale tag: the untagged-section check
#    with no v<x.y.z> left, then the shared tagged-section lint with the tag on the candidate.
# 3. The release check's gate on macOS with the memory budget, and the macOS smoke test.
# 4. hosted CI on its own runners for the candidate sha: it is pushed to a temporary branch,
#    ci.yml runs there with full=true, the gate waits for it and deletes the branch. Races that
#    only show on x86_64 runners never show in act on an arm64 Mac, so act cannot stand in.
#    ACT=1 also runs the release check's Ubuntu jobs through act, as an extra.
#    The benchmark regression check (bench-report.py --regress) refuses a newest run that is
#    worse than the previous release's outside the noise band, or that has no same-machine
#    baseline. It has no override.
# On pass it writes .git/release-gate/<sha> (the tag and the CI run), which scripts/hooks/pre-push
# requires, and pushes. On fail it deletes the local tag. RETAG=1 re-tags an unpublished
# release: the remote tag is moved by the same atomic push. PUSH=0 stops after the pass record.
# Usage: gate-run.sh <version> | --self-test
set -uo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
check_yml=.github/workflows/release-check.yml

# The checks after the changelog, one per line, in order. @tag tags the candidate, @ci runs CI
# on the hosted runners. The self-test reads this list, so a step dropped from it is red there.
steps() {
  cat <<'EOF2'
python3 scripts/lint/changelog-check.py
@tag
python3 vendor/schwarzwald/kit/lint/changelog.py --repo .
make -k check CROSS=x86_64-pc-windows-gnu
bash scripts/release/browser-run.sh
bash scripts/dev/macos-run.sh
python3 scripts/verify/ci-budget-check.py
python3 scripts/bench/bench-report.py --regress
@ci
EOF2
  [ "${ACT:-0}" = 1 ] && act_steps
  return 0
}

act_steps() {
  local job act="act workflow_call -W $check_yml --input plan={}"
  for job in gate "check --matrix os:ubuntu-22.04" deny msrv; do echo "$act -j $job"; done
}

die() {
  echo "release-gate: $*" >&2
  exit 1
}

# Regenerates the x.y.z section and commits it when it changed. TEST_RELEASE_CHANGELOG replaces
# make changelog (the fixture has no git-cliff).
changelog() {
  local version=$1
  python3 "$root/scripts/release/changelog-run.py" --drop "$version" --file CHANGELOG.md ||
    die "cannot remove the old $version section"
  bash -c "${TEST_RELEASE_CHANGELOG:-make changelog VERSION=$version}" ||
    die "make changelog VERSION=$version failed"
  [ -z "$(git status --porcelain)" ] && return 0
  git commit -qm "chore(release): $version" -- CHANGELOG.md ||
    die "cannot commit the regenerated $version section"
  [ -z "$(git status --porcelain)" ] || die "make changelog touched more than CHANGELOG.md"
  echo "release-gate: committed the regenerated $version section"
}

# Hosted CI for commit $1: a temporary branch, ci.yml full, wait, delete the branch. Prints
# the run's URL. TEST_RELEASE_CI replaces the workflow run (the fixture has no CI host).
ci() {
  local sha=$1 branch="release-gate/${1:0:12}" id="" url rc=0
  git push -q origin "$sha:refs/heads/$branch" || {
    echo "release-gate: cannot push $branch" >&2
    return 1
  }
  if [ -n "${TEST_RELEASE_CI:-}" ]; then
    bash -c "$TEST_RELEASE_CI" && url="test-ci $sha" || rc=1
  else
    gh workflow run ci.yml --ref "$branch" -f full=true >&2 || rc=1
    for _ in $(seq 1 60); do
      [ "$rc" = 0 ] || break
      id=$(gh run list -w ci.yml -b "$branch" -c "$sha" -e workflow_dispatch -L 1 \
        --json databaseId -q '.[0].databaseId // ""') || true
      [ -n "$id" ] && break
      sleep 5
    done
    [ -n "$id" ] || rc=1
    if [ "$rc" = 0 ]; then
      gh run watch "$id" --exit-status -i 30 >&2 || rc=1
      [ "$(gh run view "$id" --json headSha -q .headSha)" = "$sha" ] || rc=1
      [ "$(gh run view "$id" --json conclusion -q .conclusion)" = success ] || rc=1
      url=$(gh run view "$id" --json url -q .url)
    fi
  fi
  git push -q origin ":refs/heads/$branch" || echo "release-gate: delete $branch by hand" >&2
  [ "$rc" = 0 ] || return 1
  echo "$url"
}

# Runs the gate in repo $1 for version $2. TEST_RELEASE_STEP replaces each real check command.
gate() {
  local repo=$1 version=$2 tag="v$2" sha dir fails=0 line run=""
  cd "$repo" || die "no repo $repo"
  [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "VERSION=x.y.z is required, got '$version'"
  [ -z "$(git status --porcelain)" ] || die "the tree is dirty, commit or stash first"
  [ "$(git rev-parse HEAD)" = "$(git rev-parse --verify --quiet refs/heads/main)" ] &&
    [ "$(git symbolic-ref -q HEAD)" = refs/heads/main ] || die "HEAD is not main"
  if git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null ||
    git ls-remote --exit-code -q origin "refs/tags/$tag" >/dev/null 2>&1; then
    [ "${RETAG:-0}" = 1 ] || die "$tag exists; RETAG=1 re-tags an unpublished release"
  fi
  # The checks compare against the local tags: a stale one would be read as the release.
  git tag -d "$tag" >/dev/null 2>&1 || true
  changelog "$version"
  sha=$(git rev-parse HEAD)
  SOURCE_DATE_EPOCH=$(bash "$root/vendor/schwarzwald/kit/tools/release-date.sh" --repo "$repo" \
    --epoch "$version") || die "no release date for $version"
  export SOURCE_DATE_EPOCH
  while IFS= read -r line; do
    case $line in
    @tag)
      git tag "$tag" "$sha" || die "cannot tag $tag"
      continue
      ;;
    @ci)
      echo "release-gate: hosted CI for $sha"
      run=$(ci "$sha") || {
        echo "release-gate: FAIL: hosted CI for $sha" >&2
        fails=$((fails + 1))
      }
      continue
      ;;
    *) ;;
    esac
    echo "release-gate: $line"
    if [ -n "${TEST_RELEASE_STEP:-}" ]; then line=$TEST_RELEASE_STEP; fi
    bash -c "$line" || {
      echo "release-gate: FAIL: $line" >&2
      fails=$((fails + 1))
    }
  done < <(steps)
  if [ "$fails" -ne 0 ]; then
    git tag -d "$tag" >/dev/null 2>&1 || echo "release-gate: could not delete the local $tag" >&2
    die "$fails step(s) failed, the local $tag is deleted"
  fi
  dir="$(git rev-parse --git-common-dir)/release-gate"
  mkdir -p "$dir" || die "cannot make $dir"
  printf 'tag %s\nci %s\n' "$tag" "$run" >"$dir/$sha" || die "cannot write the pass record"
  echo "release-gate: pass, recorded $dir/$sha"
  [ "${PUSH:-1}" = 1 ] || return 0
  git push --atomic origin main "+refs/tags/$tag:refs/tags/$tag" ||
    die "the atomic push of main and $tag failed"
}

self_test() {
  local tmp fails=0 out sha before
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' RETURN
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
  git init -q --bare -b main "$tmp/far.git"
  git init -q -b main "$tmp/r"
  git -C "$tmp/r" config user.name t
  git -C "$tmp/r" config user.email t@t
  git -C "$tmp/r" remote add origin "$tmp/far.git"
  printf '# Changelog\n\n' >"$tmp/r/CHANGELOG.md"
  echo a >"$tmp/r/f" && git -C "$tmp/r" add f CHANGELOG.md && git -C "$tmp/r" commit -qm a
  # The fixture's changelog generator: the section from the commits, dated today.
  export TEST_RELEASE_CHANGELOG="printf '## [1.0.0] - today\n\n- generated\n' >>CHANGELOG.md"
  fail() {
    echo "release-gate self-test: $*" >&2
    fails=$((fails + 1))
  }
  red() { # name, then the command; red when it passes
    if out=$("${@:2}" 2>&1); then fail "$1: want refused, passed: $out"; fi
  }
  green() {
    if ! out=$("${@:2}" 2>&1); then fail "$1: want pass, refused: $out"; fi
  }
  far() { git -C "$tmp/far.git" rev-parse --verify --quiet "$1" 2>/dev/null; }
  pass() { env TEST_RELEASE_STEP=true TEST_RELEASE_CI=true "$@"; }
  red "a version that is not x.y.z" pass bash "$0" --in "$tmp/r" 1.0
  echo dirty >"$tmp/r/f"
  red "a dirty tree" pass bash "$0" --in "$tmp/r" 1.0.0
  git -C "$tmp/r" checkout -q f
  git -C "$tmp/r" checkout -q --detach
  red "a HEAD that is not main" pass bash "$0" --in "$tmp/r" 1.0.0
  git -C "$tmp/r" checkout -q main
  red "a failing step" env TEST_RELEASE_STEP=false TEST_RELEASE_CI=true \
    bash "$0" --in "$tmp/r" 1.0.0
  red "a failing hosted CI run" env TEST_RELEASE_STEP=true TEST_RELEASE_CI=false \
    bash "$0" --in "$tmp/r" 1.0.0
  git -C "$tmp/r" rev-parse --verify --quiet refs/tags/v1.0.0 >/dev/null &&
    fail "a failing step left the local tag"
  [ -n "$(ls "$tmp/r/.git/release-gate" 2>/dev/null)" ] && fail "a failing step wrote a pass record"
  far v1.0.0 >/dev/null && fail "a failing step pushed the tag"
  [ -n "$(git -C "$tmp/far.git" for-each-ref refs/heads/release-gate)" ] &&
    fail "a CI run left its temporary branch"
  # A hand-edited section with an old date: the gate must replace it, not keep it.
  git -C "$tmp/r" reset -q --hard main~0
  printf '## [1.0.0] - 2000-01-01\n\n- by hand\n' >>"$tmp/r/CHANGELOG.md"
  git -C "$tmp/r" commit -qam "docs(changelog): by hand"
  before=$(git -C "$tmp/r" rev-parse HEAD)
  green "a passing gate" pass bash "$0" --in "$tmp/r" 1.0.0
  sha=$(git -C "$tmp/r" rev-parse HEAD)
  [ "$sha" != "$before" ] || fail "the regenerated section was not committed"
  grep -q 'by hand\|2000-01-01' "$tmp/r/CHANGELOG.md" && fail "a hand edit survived the gate"
  [ "$(grep -c '^## \[1.0.0\]' "$tmp/r/CHANGELOG.md")" = 1 ] || fail "the section is not there once"
  grep -q '^ci ' "$tmp/r/.git/release-gate/$sha" 2>/dev/null || fail "a pass wrote no CI record"
  [ "$(far main)" = "$sha" ] && [ "$(far v1.0.0)" = "$sha" ] ||
    fail "a pass did not push main and the tag"
  [ -n "$(git -C "$tmp/far.git" for-each-ref refs/heads/release-gate)" ] &&
    fail "a passing CI run left its temporary branch"
  red "an existing tag without RETAG" pass bash "$0" --in "$tmp/r" 1.0.0
  git -C "$tmp/r" tag -d v1.0.0 >/dev/null
  red "a remote tag without RETAG" pass bash "$0" --in "$tmp/r" 1.0.0
  echo b >"$tmp/r/f" && git -C "$tmp/r" commit -qam "fix: b"
  # git-cliff's date moved on: the regenerated section differs and is committed again.
  export TEST_RELEASE_CHANGELOG="printf '## [1.0.0] - tomorrow\n\n- generated\n' >>CHANGELOG.md"
  green "RETAG=1 moves the tag on both sides" pass RETAG=1 bash "$0" --in "$tmp/r" 1.0.0
  [ "$(far v1.0.0)" = "$(git -C "$tmp/r" rev-parse HEAD)" ] ||
    fail "RETAG=1 left the remote tag behind"
  [ "$(far main)" = "$(far v1.0.0)" ] || fail "RETAG=1 pushed main and the tag apart"
  grep -q ' - today' "$tmp/r/CHANGELOG.md" && fail "RETAG=1 kept the old date"
  # Both changelog checks, the macOS gate and smoke test, and hosted CI are steps; act is
  # extra, and with ACT=1 every Ubuntu job of the release check runs through it.
  local list jobs job
  list=$(steps)
  for want in scripts/lint/changelog-check.py kit/lint/changelog.py "CROSS=x86_64-pc-windows-gnu" \
    scripts/dev/macos-run.sh scripts/release/browser-run.sh "bench-report.py --regress" @ci @tag; do
    grep -qF -- "$want" <<<"$list" || fail "no step runs $want"
  done
  [ "$(sed -n 1p <<<"$list")" = python3\ scripts/lint/changelog-check.py ] &&
    [ "$(sed -n 2p <<<"$list")" = @tag ] ||
    fail "the untagged-section check does not run before @tag"
  grep -q '^act ' <<<"$list" && fail "act runs without ACT=1"
  grep -q 'MEMORY=0' <<<"$list" && fail "the gate leaves the memory budget out"
  list=$(ACT=1 steps)
  jobs=$(awk '/^jobs:/ { on = 1; next } on && /^  [a-z][a-z0-9_-]*:$/ { sub(/:$/, ""); print $1 }' \
    "$root/$check_yml")
  [ -n "$jobs" ] || fail "no jobs read from $check_yml"
  for job in $jobs; do
    # The memory job runs on macOS inside make check; every other job runs through act.
    [ "$job" = memory ] && continue
    grep -qE -- "-j $job( |\$)" <<<"$list" || fail "release-check job '$job' is not run through act"
  done
  [ "$fails" -eq 0 ] || {
    echo "release-gate self-test: FAIL" >&2
    return 1
  }
  echo "release-gate self-test: ok"
}

case "${1:-}" in
--self-test) self_test ;;
--in) gate "$2" "${3:-}" ;;
*) gate "$root" "${1:-}" ;;
esac
