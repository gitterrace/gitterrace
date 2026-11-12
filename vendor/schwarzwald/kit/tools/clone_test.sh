#!/usr/bin/env bash
# clone.sh: the clone lands at <repo>/.wt/<label> on its own branch with the hooks wired, and
# every refusal holds. Runs in throwaway repositories.
#
#   bash tools/clone_test.sh
#   CLONE=/path/to/other-clone.sh bash tools/clone_test.sh   # a clone.sh without the hooks must FAIL
set -euo pipefail
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE GIT_PREFIX GIT_COMMON_DIR
CLONE="${CLONE:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/clone.sh}"
. "$(dirname "${BASH_SOURCE[0]}")/test-sandbox.sh"
sandbox_make clone-test
sandbox_enter --multi
t=$SANDBOX_REPO
export DISK_FLOOR_GIB=0
fail=0
check() { # check <label> <command...>
  local label=$1; shift
  if "$@"; then echo "ok   $label"; else echo "FAIL $label"; fail=1; fi
}
refused() { # refused <label> <needle> <clone.sh args...>: exit 1 and the needle on stderr
  local label=$1 needle=$2 rc=0 err; shift 2
  err=$(bash "$CLONE" "$@" 2>&1 >/dev/null) || rc=$?
  if [[ $rc == 1 && $err == *"$needle"* ]]; then echo "ok   $label"; else
    echo "FAIL $label (want rc=1 '$needle', got rc=$rc: $err)"; fail=1; fi
}
main="$t/main"
mkdir -p "$main/scripts/hooks"
printf '#!/bin/sh\n' >"$main/scripts/hooks/commit-msg"
chmod +x "$main/scripts/hooks/commit-msg"
git init -q -b main "$main"
git -C "$main" add scripts/hooks/commit-msg
git -C "$main" commit -q -m "feat(x): first"

out=$(cd "$main" && bash "$CLONE" fix-a)
dest="$main/.wt/fix-a"
check "the last line is the clone's path" test "$out" = "$dest"
check "the clone is on a branch named after the label" \
  test "$(git -C "$dest" rev-parse --abbrev-ref HEAD)" = fix-a
check "the branch starts at the repo's HEAD" \
  test "$(git -C "$dest" rev-parse HEAD)" = "$(git -C "$main" rev-parse HEAD)"
check "the hooks are wired" test "$(git -C "$dest" config core.hooksPath)" = scripts/hooks
check "the clone owns its objects (--no-local)" \
  test ! -s "$dest/.git/objects/info/alternates"
check "the clone adds nothing to the repo's refs" \
  test "$(git -C "$main" for-each-ref --format=x refs/heads | wc -l | tr -d ' ')" = 1

git -C "$main" commit -q --allow-empty -m "feat(x): second"
first=$(git -C "$main" rev-parse HEAD~1)
bash "$CLONE" at-rev --repo "$main" --rev "$first" >/dev/null
check "--rev starts the branch there" test "$(git -C "$main/.wt/at-rev" rev-parse HEAD)" = "$first"

mkdir -p "$main/schwarzwald/config"
echo 'GUARD_WORKTREE_DIR="${GUARD_WORKTREE_DIR:-lanes}"' >"$main/schwarzwald/config/guards.env"
bash "$CLONE" own-dir --repo "$main" >/dev/null
check "the product's own clone dir from guards.env" test -d "$main/lanes/own-dir/.git"
rm "$main/schwarzwald/config/guards.env"

refused "a label that is not kebab" "lowercase kebab" Bad_Label --repo "$main"
refused "an existing path" "exists" fix-a --repo "$main"
refused "a clone of a clone" "itself a lane clone" deeper --repo "$dest"
nogit="$SANDBOX/nogit"
mkdir -p "$nogit"
GIT_CEILING_DIRECTORIES="$SANDBOX" refused "a directory outside any checkout" "no git checkout" x --repo "$nogit"
rc=0; bash "$CLONE" >/dev/null 2>&1 || rc=$?
check "no label prints the usage" test "$rc" = 1

plain="$t/plain"
git init -q -b main "$plain"
git -C "$plain" commit -q --allow-empty -m "feat(x): first"
err=$(bash "$CLONE" p --repo "$plain" 2>&1 >/dev/null)
check "a repo without scripts/hooks says so" grep -q "no hooks are installed" <<<"$err"
check "and sets no hooksPath" test -z "$(git -C "$plain/.wt/p" config core.hooksPath || true)"

if [[ $fail != 0 ]]; then echo "clone_test: FAILED"; exit 1; fi
echo "clone_test: ok"
