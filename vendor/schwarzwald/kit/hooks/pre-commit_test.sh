#!/usr/bin/env bash
# The pre-commit hook, opt-in through MAIN_CHECKOUT_LANDS_ONLY=1 in schwarzwald/config/guards.env
# plus the local landing.target marker: only the marked checkout refuses a commit without
# LAND_TOOL=1. A full clone of the same repository and a linked worktree never do, and with the flag
# off nothing fires. Runs in a throwaway repository.
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
mkdir -p "$d/schwarzwald/config"
git -C "$d" worktree add -q "$d/wt" -b side
git -C "$d" config --local landing.target "$d"
fail=0
# expect <label> <ok|refused> <dir> [env...]
expect() {
  local label=$1 want=$2 dir=$3 got=ok; shift 3
  (cd "$dir" && env "$@" bash "$HOOK" >/dev/null 2>&1) || got=refused
  if [[ $got == "$want" ]]; then echo "ok   $label"; else echo "FAIL $label (want $want)"; fail=1; fi
}
echo 'MAIN_CHECKOUT_LANDS_ONLY="1"' >"$d/schwarzwald/config/guards.env"
cp "$d/schwarzwald/config/guards.env" "$d/wt/" 2>/dev/null || true
mkdir -p "$d/wt/schwarzwald/config"; cp "$d/schwarzwald/config/guards.env" "$d/wt/schwarzwald/config/"
expect "flag on, main checkout, no stamp is refused" refused "$d" X=1
expect "flag on, main checkout, LAND_TOOL=1 passes" ok "$d" LAND_TOOL=1
expect "flag on, linked worktree passes" ok "$d/wt" X=1
# The flag is tracked, so every clone carries it: a clone without the local marker commits freely,
# and the same clone refuses once it is marked.
git -C "$d" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty -m flag
c="$d-clone"
git clone -q "$d" "$c"
mkdir -p "$c/schwarzwald/config"
cp "$d/schwarzwald/config/guards.env" "$c/schwarzwald/config/"
expect "flag on, a full clone without the marker passes" ok "$c" X=1
git -C "$c" config --local landing.target "$c"
expect "flag on, the same clone once marked is refused" refused "$c" X=1
git -C "$c" config --local landing.target "$d"
expect "flag on, a marker naming another checkout passes" ok "$c" X=1
git -C "$d" config --local --unset landing.target
expect "flag on, the main checkout without the marker passes" ok "$d" X=1
git -C "$d" config --local landing.target "$d"
# Both forms the config shape allows turn the check on; 0 in each turns it off.
for v in '"1"' '"${MAIN_CHECKOUT_LANDS_ONLY:-1}"'; do
  printf 'GUARD_PRODUCT=%s\nMAIN_CHECKOUT_LANDS_ONLY=%s\n' '"t"' "$v" >"$d/schwarzwald/config/guards.env"
  expect "flag spelt $v, main checkout, no stamp is refused" refused "$d" X=1
  printf 'MAIN_CHECKOUT_LANDS_ONLY=%s\n' "${v//1/0}" >"$d/schwarzwald/config/guards.env"
  expect "flag spelt ${v//1/0}, main checkout, no stamp passes" ok "$d" X=1
done
# The caller's environment cannot switch the product's flag off.
printf 'MAIN_CHECKOUT_LANDS_ONLY="${MAIN_CHECKOUT_LANDS_ONLY:-1}"\n' >"$d/schwarzwald/config/guards.env"
expect "flag on, MAIN_CHECKOUT_LANDS_ONLY=0 in the environment is still refused" refused "$d" MAIN_CHECKOUT_LANDS_ONLY=0
echo 'MAIN_CHECKOUT_LANDS_ONLY="0"' >"$d/schwarzwald/config/guards.env"
expect "flag off, main checkout, no stamp passes" ok "$d" X=1
rm "$d/schwarzwald/config/guards.env"
mkdir -p "$d/config/schwarzwald"
echo 'MAIN_CHECKOUT_LANDS_ONLY="1"' >"$d/config/schwarzwald/guards.env"
expect "a guards.env at the old config/schwarzwald/ path is not honoured" ok "$d" X=1
rm -r "$d/config"
expect "no guards.env passes" ok "$d" X=1
exit "$fail"
