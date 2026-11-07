#!/usr/bin/env bash
# stale-index.sh refuses a staged tree that undoes a recent HEAD commit, and passes new work.
#
#   bash hooks/stale-index_test.sh
#   HOOK=/usr/bin/true bash hooks/stale-index_test.sh   # must FAIL
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
hook="${HOOK:-${here}/stale-index.sh}"
. "${here}/../tools/test-sandbox.sh"
sandbox_make stale-index-test
sandbox_enter
mkdir -p config/schwarzwald
printf '[commit]\nallow_prefix = "DEMO_"\n' >config/schwarzwald/lint.toml
fail=0
# expect <label> <rc wanted>, with the environment the caller set
expect() {
  local rc=0
  bash "${hook}" >/dev/null 2>&1 || rc=1
  if [[ ${rc} == "$2" ]]; then
    echo "  ok   $1"
  else
    echo "  FAIL $1 (rc ${rc})"
    fail=1
  fi
}
printf 'one\n' >a && printf 'keep\n' >b && git add a b && git commit -qm base
stale=$(git write-tree)
printf 'two\n' >a && git commit -qam change
git rm -q b && git commit -qm drop

git read-tree "${stale}"
expect "refuses a stale index that reverts a change and re-adds a deletion" 1
DEMO_ALLOW_RESTORE=1 expect "passes the same tree with the restore override" 0
SCHWARZWALD_ALLOW_RESTORE=1 expect "an override with another prefix does not count" 1
DEMO_STALE_DEPTH=1 expect "a depth of 1 sees only the last commit, which dropped b" 1
git read-tree HEAD && printf 'three\n' >a && git add a
expect "passes an ordinary change on HEAD" 0
git read-tree HEAD && git rm -q --cached a
expect "passes an ordinary deletion" 0
exit "${fail}"
