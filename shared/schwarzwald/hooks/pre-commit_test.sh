#!/usr/bin/env bash
# The pre-commit hook, opt-in through MAIN_CHECKOUT_LANDS_ONLY=1 in config/schwarzwald/guards.env:
# the main checkout refuses a commit without LAND_TOOL=1, a linked worktree never does, and with
# the flag off nothing fires. Runs in a throwaway repository.
#
#   bash hooks/pre-commit_test.sh
#   HOOK=/path/to/always-pass.sh bash hooks/pre-commit_test.sh   # must FAIL
set -euo pipefail
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE GIT_PREFIX GIT_COMMON_DIR LAND_TOOL
HOOK="${HOOK:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/pre-commit}"
HOOK="$(cd "$(dirname "${HOOK}")" && pwd)/$(basename "${HOOK}")"  # absolute: the cases cd away
. "$(dirname "${BASH_SOURCE[0]}")/../tools/test-sandbox.sh"
sandbox_make pre-commit-test
sandbox_enter --multi
d=$SANDBOX_REPO
git -C "$d" init -q -b main
git -C "$d" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty -m init
mkdir -p "$d/config/schwarzwald"
git -C "$d" worktree add -q "$d/wt" -b side
fail=0
# expect <label> <ok|refused> <dir> [env...]
expect() {
  local label=$1 want=$2 dir=$3 got=ok; shift 3
  (cd "$dir" && env "$@" bash "$HOOK" >/dev/null 2>&1) || got=refused
  if [[ $got == "$want" ]]; then echo "ok   $label"; else echo "FAIL $label (want $want)"; fail=1; fi
}
echo "MAIN_CHECKOUT_LANDS_ONLY='1'" >"$d/config/schwarzwald/guards.env"
cp "$d/config/schwarzwald/guards.env" "$d/wt/" 2>/dev/null || true
mkdir -p "$d/wt/config/schwarzwald"; cp "$d/config/schwarzwald/guards.env" "$d/wt/config/schwarzwald/"
expect "flag on, main checkout, no stamp is refused" refused "$d" X=1
expect "flag on, main checkout, LAND_TOOL=1 passes" ok "$d" LAND_TOOL=1
expect "flag on, linked worktree passes" ok "$d/wt" X=1
# Every spelling the config shape allows turns the check on; 0 in each turns it off.
for v in "'1'" '"1"' '1' '"${MAIN_CHECKOUT_LANDS_ONLY:-1}"'; do
  printf 'GUARD_PRODUCT=%s\nMAIN_CHECKOUT_LANDS_ONLY=%s\n' "'t'" "$v" >"$d/config/schwarzwald/guards.env"
  expect "flag spelt $v, main checkout, no stamp is refused" refused "$d" X=1
  printf 'MAIN_CHECKOUT_LANDS_ONLY=%s\n' "${v//1/0}" >"$d/config/schwarzwald/guards.env"
  expect "flag spelt ${v//1/0}, main checkout, no stamp passes" ok "$d" X=1
done
# The caller's environment cannot switch the product's flag off.
printf 'MAIN_CHECKOUT_LANDS_ONLY="${MAIN_CHECKOUT_LANDS_ONLY:-1}"\n' >"$d/config/schwarzwald/guards.env"
expect "flag on, MAIN_CHECKOUT_LANDS_ONLY=0 in the environment is still refused" refused "$d" MAIN_CHECKOUT_LANDS_ONLY=0
echo "MAIN_CHECKOUT_LANDS_ONLY='0'" >"$d/config/schwarzwald/guards.env"
expect "flag off, main checkout, no stamp passes" ok "$d" X=1
rm "$d/config/schwarzwald/guards.env"
expect "no guards.env passes" ok "$d" X=1
exit "$fail"
