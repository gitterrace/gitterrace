#!/usr/bin/env bash
# commit-paths.sh: red cases (no paths, a path outside the repo, shared index left unchanged).
#
#   bash tools/commit-paths_test.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
CP="${CP:-$here/commit-paths.sh}"
. "$here/test-sandbox.sh"
sandbox_make commit-paths-test
sandbox_enter --multi
t=$SANDBOX_REPO
fail=0
check() { if "$@"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
echo a >"$t/a"; echo b >"$t/b"
git -C "$t" add a
git -C "$t" commit -q -m first
echo a2 >"$t/a"
git -C "$t" add b   # staged in the shared index: must stay staged and uncommitted
before=$(git -C "$t" ls-files -s | cksum)

no_paths() { ! bash "$CP" -C "$t" -m x -- 2>/dev/null; }
outside() { echo z >"$SANDBOX/z"; ! bash "$CP" -C "$t" -m x -- "$SANDBOX/z" 2>/dev/null; }
good() {
    local sha
    sha=$(LANE=t1 bash "$CP" -C "$t" -m second -- a)
    [ "$(git -C "$t" show --name-only --format= "$sha")" = a ] && [ ! -e "$t/.git/index-t1" ]
}
shared_index_unchanged() {
    [ "$(git -C "$t" ls-files -s | cksum)" = "$before" ] && ! git -C "$t" ls-tree HEAD --name-only | grep -qx b
}
check no_paths
check outside
check good
check shared_index_unchanged
exit "$fail"
