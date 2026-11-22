#!/usr/bin/env bash
# Land work from a work clone, then sweep that clone away when it is clean and landed.
#
#   scripts/dev/land-run.sh [land.sh options] <repo> <clone-dir> <sha>
#   scripts/dev/land-run.sh [land.sh options] --plan <repo> <clone-dir> <base> <plan>
#   scripts/dev/land-run.sh --self-test
#
# The repo-owned entry around vendor/schwarzwald/kit/tools/land.sh, which lands but never
# removes a clone. Arguments pass through unchanged. Only when land.sh exits 0 does
# scripts/dev/sweep-run.sh look at <clone-dir>, and it keeps the clone when anything is
# unlanded, dirty or in use, so the sweep never fails the landing.
# TEST_LAND_SH replaces land.sh in the self-test only.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)

# clone_arg <args...>: the <clone-dir> land.sh will use, for either call form.
clone_arg() {
  local a plan=0
  for a in "$@"; do [ "$a" = --plan ] && plan=1; done
  if [ "$plan" = 1 ]; then echo "${*: -3:1}"; else echo "${*: -2:1}"; fi
}

# repo_arg <args...>: the <repo> land.sh will use, for either call form.
repo_arg() {
  local a plan=0
  for a in "$@"; do [ "$a" = --plan ] && plan=1; done
  if [ "$plan" = 1 ]; then echo "${*: -4:1}"; else echo "${*: -3:1}"; fi
}

self_test() {
  local got
  got=$(clone_arg /r /r/.wt/c 1234abc)
  [ "$got" = /r/.wt/c ] || {
    echo "land-run self-test: sha form picked '$got'" >&2
    exit 1
  }
  got=$(repo_arg /r /r/.wt/c 1234abc)
  [ "$got" = /r ] || {
    echo "land-run self-test: sha form repo '$got'" >&2
    exit 1
  }
  got=$(clone_arg --full-gate --plan /r /r/.wt/c 1234abc /r/plan)
  [ "$got" = /r/.wt/c ] || {
    echo "land-run self-test: plan form picked '$got'" >&2
    exit 1
  }
  got=$(repo_arg --gate-change --plan /r /r/.wt/c 1234abc /r/plan)
  [ "$got" = /r ] || {
    echo "land-run self-test: plan form repo '$got'" >&2
    exit 1
  }
  # A refused land keeps the clone: the sweep never runs after a non-zero land.sh.
  local t stub
  t=$(cd "$(mktemp -d)" && pwd -P)
  stub=$t/land.sh
  printf '#!/usr/bin/env bash\nexit 3\n' >"$stub"
  mkdir -p "$t/repo/.wt/c"
  if TEST_LAND_SH=$stub bash "$0" --plan "$t/repo" "$t/repo/.wt/c" 1234abc "$t/plan" \
    2>/dev/null; then
    echo "land-run self-test: a refused land exited 0" >&2
    exit 1
  fi
  [ -d "$t/repo/.wt/c" ] || {
    echo "land-run self-test: a refused land lost its clone" >&2
    exit 1
  }
  rm -rf "$t"
  echo "land-run self-test: ok"
}

[ "${1:-}" = --self-test ] && {
  self_test
  exit 0
}
[ $# -ge 3 ] || {
  echo "usage: scripts/dev/land-run.sh [land.sh options] [--plan] <repo> <clone-dir>" \
    "<sha | base plan>" >&2
  exit 2
}
clone=$(clone_arg "$@")
repo=$(cd "$(repo_arg "$@")" && pwd)
bash "${TEST_LAND_SH:-$here/../../vendor/schwarzwald/kit/tools/land.sh}" "$@"
# The sweep is the repo's own copy and runs from the repo, so neither this script nor its cwd
# keeps the clone busy or vanishes under it. A shell the caller left inside the clone still
# keeps it, which the sweep names.
cd "$repo"
bash scripts/dev/sweep-run.sh "$clone" || true
