#!/usr/bin/env bash
# land.sh: the landed ledger, squash-plan landing and the checks around them. Runs in throwaway
# repositories. Every case checks the exit code and the refusal text.
#
#   bash tools/land_test.sh
#   LAND=/path/to/other-land.sh bash tools/land_test.sh   # a land.sh without the checks must FAIL
set -euo pipefail
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE GIT_PREFIX GIT_COMMON_DIR LAND_TOOL
LAND="${LAND:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/land.sh}"
. "$(dirname "${BASH_SOURCE[0]}")/test-sandbox.sh"
sandbox_make land-test
sandbox_enter --multi
t=$SANDBOX_REPO
G="git -c user.name=t -c user.email=t@example.invalid"
fail=0
check() { # check <label> <command...>: a failing command fails the case
  local label=$1; shift
  if "$@"; then echo "ok   $label"; else echo "FAIL $label"; fail=1; fi
}
# setup <on|off>: a main repo with one commit and a clone with one change to land
setup() {
  rm -rf "$t/main" "$t/clone"
  git init -q -b main "$t/main"; mkdir -p "$t/main/config/schwarzwald" "$t/main/scripts"
  [[ $1 == off ]] || echo "MAIN_CHECKOUT_LANDS_ONLY='1'" >"$t/main/config/schwarzwald/guards.env"
  # MAIN_TOML / MAIN_CHECK: the gate main carries (land.toml text, land-check.sh text)
  [[ -z ${MAIN_TOML:-} ]] || printf '%s' "$MAIN_TOML" >"$t/main/config/schwarzwald/land.toml"
  [[ -z ${MAIN_CHECK:-} ]] || printf '%s' "$MAIN_CHECK" >"$t/main/scripts/land-check.sh"
  echo a >"$t/main/a"; git -C "$t/main" add -A; $G -C "$t/main" commit -q -m "feat(x): first"
  git clone -q "$t/main" "$t/clone"
  echo b >"$t/clone/b"; git -C "$t/clone" add b; $G -C "$t/clone" commit -q -m "feat(x): second"
  sha=$(git -C "$t/clone" rev-parse HEAD)
}
# run <label> <want-rc> <needle> <land args...>: needle must appear in stderr
run() {
  local label=$1 want=$2 needle=$3 rc=0 err; shift 3
  err=$(bash "$LAND" "$@" 2>&1 >/dev/null) || rc=$?
  if [[ $rc == "$want" && $err == *"$needle"* ]]; then echo "ok   $label"; else
    echo "FAIL $label (want rc=$want '$needle', got rc=$rc: $err)"; fail=1; fi
}
one() { run "$1" "$2" "$3" --no-gate "$t/main" "$t/clone" "$sha"; }
# plan <base> <sha-lines...>: write the plan file
plan() { local base=$1; shift; printf '%s\n' "$@" >"$t/plan"; planbase=$base; }
runplan() { run "$1" "$2" "$3" --no-gate --plan "$t/main" "$t/clone" "$planbase" "$t/plan"; }
head_of() { git -C "$t/$1" rev-parse HEAD; }

# 0: the file keeps its exec bit
check "land.sh is executable" test -x "$LAND"
# 1: a hand commit made around the tool (--no-verify) since land-tip is refused
setup on
head_of main >"$t/main/.git/land-tip"
echo c >"$t/main/c"; git -C "$t/main" add c; $G -C "$t/main" commit -q --no-verify -m "feat(x): sneaked"
one "a hand commit that is not in the landed file is refused" 1 "landed"
# 2: flag off, the same history lands
setup off
head_of main >"$t/main/.git/land-tip"
echo c >"$t/main/c"; git -C "$t/main" add c; $G -C "$t/main" commit -q --no-verify -m "feat(x): sneaked"
git -C "$t/clone" pull -q --rebase "$t/main" 2>/dev/null; sha=$(head_of clone)
one "flag off, a hand commit lands anyway" 0 ""
# 2b: every spelling the config shape allows turns the ledger check on, 0 turns it off
for v in "'1'" '"1"' '1' '"${MAIN_CHECKOUT_LANDS_ONLY:-1}"'; do
  for want in 1 0; do
    setup off
    printf 'MAIN_CHECKOUT_LANDS_ONLY=%s\n' "${v//1/$want}" >"$t/main/config/schwarzwald/guards.env"
    git -C "$t/main" add -A; $G -C "$t/main" commit -q --no-verify -m "chore(x): flag"
    git -C "$t/clone" pull -q --rebase "$t/main" 2>/dev/null; sha=$(head_of clone)
    head_of main >"$t/main/.git/land-tip"
    echo c >"$t/main/c"; git -C "$t/main" add c; $G -C "$t/main" commit -q --no-verify -m "feat(x): sneaked"
    git -C "$t/clone" pull -q --rebase "$t/main" 2>/dev/null; sha=$(head_of clone)
    if [[ $want == 1 ]]; then one "flag spelt ${v}: a hand commit is refused" 1 "landed"
    else one "flag spelt ${v//1/0}: a hand commit lands" 0 ""; fi
  done
done
# 3: dirty main
setup on
echo dirty >>"$t/main/a"
one "a dirty main checkout is refused" 1 "dirty"
# 4: a landing carries no trailer, equal dates, lands in the ledger, a second landing passes
setup on
one "flag on, a clean landing passes" 0 ""
land=$(head_of main)
check "a landed message carries no trailer" test "$(git -C "$t/main" log -1 --format=%B)" = "$(git -C "$t/clone" log -1 --format=%B "$sha")"
check "author and committer dates are equal" test "$(git -C "$t/main" log -1 --format=%ad)" = "$(git -C "$t/main" log -1 --format=%cd)"
check "the landed sha is in the ledger" grep -qx "$land" "$t/main/.git/landed"
check "land-tip is the landed tip" test "$(cat "$t/main/.git/land-tip")" = "$land"
check "the ledger is not committed" test -z "$(git -C "$t/main" ls-files landed)"
echo d >"$t/clone/d"; git -C "$t/clone" add d; $G -C "$t/clone" commit -q -m "feat(x): third"
sha=$(head_of clone)
git -C "$t/clone" pull -q --rebase "$t/main" 2>/dev/null || true
sha=$(head_of clone)
one "a second landing after a landed one passes" 0 ""

# plan mode: clone holds two commits and a fold, base is main's tip
plan_setup() {
  setup "$1"
  base=$(head_of main)
  echo c >"$t/clone/c"; git -C "$t/clone" add c; $G -C "$t/clone" commit -q -m "feat(x): third"
  echo d >"$t/clone/d"; git -C "$t/clone" add d; $G -C "$t/clone" commit -q -m "docs(x): note for the third"
  s1=$(git -C "$t/clone" rev-parse HEAD~2) s2=$(git -C "$t/clone" rev-parse HEAD~1) s3=$(head_of clone)
  git -C "$t/clone" config user.name t; git -C "$t/clone" config user.email t@example.invalid
}
# 5: a good plan lands, with equal dates, +1 s per commit, a fold, no trailer, ledger entries
plan_setup on
plan "$base" "$s1" "$s2" "$s3 fold"
runplan "a good plan lands" 0 ""
check "plan: two commits on base, the fold squashed" test "$(git -C "$t/main" rev-list --count "$base..HEAD")" = 2
check "plan: tree equals the clone's tree" test "$(git -C "$t/main" rev-parse 'HEAD^{tree}')" = "$(git -C "$t/clone" rev-parse 'HEAD^{tree}')"
check "plan: author date equals committer date on every commit" test -z "$(git -C "$t/main" log --format='%at %ct' "$base..HEAD" | awk '$1 != $2')"
check "plan: dates rise by one second" test "$(git -C "$t/main" log --reverse --format=%ct "$base..HEAD" | awk 'NR==1{a=$1} NR==2{print $1-a}')" = 1
check "plan: no trailer in a message" test -z "$(git -C "$t/main" log --format=%B "$base..HEAD" | grep ':[ ]*land' || true)"
check "plan: every landed sha is in the ledger" test "$(git -C "$t/main" rev-list "$base..HEAD" | grep -cxFf "$t/main/.git/landed")" = 2
# 6: a stale plan (base is not main's tip)
plan_setup on
echo z >"$t/main/z"; git -C "$t/main" add z; LAND_TOOL=1 $G -C "$t/main" commit -q -m "feat(x): moved"
plan "$base" "$s1" "$s2" "$s3 fold"
runplan "a stale base is refused" 1 "stale plan"
# 6b: a plan whose last sha is not the clone's HEAD
plan_setup on
plan "$base" "$s1" "$s2"
runplan "a plan that ends before the clone HEAD is refused" 1 "stale plan"
# 7: a plan with an unequal tree (the plan drops a commit the HEAD tree has, as a fold-less skip)
plan_setup on
plan "$base" "$s1" "$s3"
runplan "a plan with an unequal tree is refused" 1 "tree"
# 8: a noise subject
plan_setup on
echo e >"$t/clone/e"; git -C "$t/clone" add e; $G -C "$t/clone" commit -q -m "fixup! feat(x): third"
s4=$(head_of clone)
plan "$base" "$s1" "$s2" "$s3" "$s4"
runplan "a noise subject is refused" 3 "partial step"
# 9: product gate and message gate come from config/schwarzwald/land.toml
toml() { # toml <gate-cmd> <messages-cmd>: the land.toml main carries (the gate is read from main)
  MAIN_TOML=$(printf '[landing]\ngate = ["%s"]\nmessages = "%s"\n' "$1" "$2")
}
toml "echo FAIL check-\$((1+1)); seq 40; exit 1" "true"; plan_setup on; MAIN_TOML=
plan "$base" "$s1" "$s2" "$s3 fold"
runplan "a red gate shows its FAIL lines, not just the tail" 1 "FAIL check-2"
toml "echo gate-broke; exit 1" "true"; plan_setup on; MAIN_TOML=
plan "$base" "$s1" "$s2" "$s3 fold"
runplan "a red product gate is refused" 1 "gate-broke"
toml "true" "echo msg-broke-{base}; exit 1"; plan_setup on; MAIN_TOML=
plan "$base" "$s1" "$s2" "$s3 fold"
runplan "a red message gate is refused" 1 "msg-broke-$base"
# 9a: no land.toml, no gate; the full land-check.sh (main's) runs only with --full-gate
MAIN_CHECK=$(printf '#!/usr/bin/env bash\necho full-broke; exit 1\n'); plan_setup on; MAIN_CHECK=
plan "$base" "$s1" "$s2" "$s3 fold"
run "--full-gate runs land-check.sh" 1 "full-broke" --full-gate --no-gate --plan "$t/main" "$t/clone" "$planbase" "$t/plan"
runplan "a land-check.sh is not run by default" 0 ""
# 9b: main moves while the gate runs, so the merge is not a fast-forward
toml "git -C $t/main commit -q --allow-empty --no-verify -m 'feat(x): raced'" "true"; plan_setup on; MAIN_TOML=
plan "$base" "$s1" "$s2" "$s3 fold"
GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@e.invalid GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@e.invalid runplan "a non-fast-forward is refused" 1 "fast-forward"
# 9d (ST-1): the gate comes from main's tip, never from the commit being landed. main gates on
# `test -f OK`; the candidate deletes OK and rewrites the gate to a command of its own.
pwn="$t/gate-ran"
for mode in single plan; do
  rm -f "$pwn"
  MAIN_TOML=$(printf '[landing]\ngate = ["test -f OK"]\nmessages = "true"\n')
  if [[ $mode == single ]]; then setup off; else plan_setup on; fi
  MAIN_TOML=
  echo ok >"$t/main/OK"; git -C "$t/main" add OK; LAND_TOOL=1 $G -C "$t/main" commit -q -m "chore(x): ok"
  git -C "$t/clone" pull -q --rebase "$t/main" 2>/dev/null
  git -C "$t/clone" rm -q OK
  printf '[landing]\ngate = ["echo PWNED > %s; true"]\n' "$pwn" >"$t/clone/config/schwarzwald/land.toml"
  git -C "$t/clone" add -A; $G -C "$t/clone" commit -q -m "feat(x): own gate"
  if [[ $mode == single ]]; then
    sha=$(head_of clone)
    run "single: a candidate that rewrites the gate is refused" 1 "gate" "$t/main" "$t/clone" "$sha"
    run "single: --gate-change still runs main's gate" 1 "test -f OK" --gate-change "$t/main" "$t/clone" "$sha"
  else
    base=$(head_of main)
    # shellcheck disable=SC2046
    plan "$base" $(git -C "$t/clone" rev-list --reverse "$base..HEAD")
    runplan "plan: a candidate that rewrites the gate is refused" 1 "gate"
    run "plan: --gate-change still runs main's gate" 1 "test -f OK" --gate-change --plan "$t/main" "$t/clone" "$planbase" "$t/plan"
  fi
  check "$mode: the candidate's own gate never ran" test ! -e "$pwn"
done
# 9c: no land.toml, or no [landing] in it: loud line and refused, --no-gate passes
plan_setup on
plan "$base" "$s1" "$s2" "$s3 fold"
run "no land.toml is refused" 1 "landing gate: none configured" --plan "$t/main" "$t/clone" "$planbase" "$t/plan"
run "no land.toml is refused with the way out" 1 "--no-gate" --plan "$t/main" "$t/clone" "$planbase" "$t/plan"
MAIN_TOML=$(printf '[other]\nx = 1\n'); plan_setup on; MAIN_TOML=
plan "$base" "$s1" "$s2" "$s3 fold"
run "a land.toml without [landing] is refused" 1 "landing gate: none configured" --plan "$t/main" "$t/clone" "$planbase" "$t/plan"
runplan "--no-gate lands without a gate" 0 ""
# 10: dirty checkout in plan mode
plan_setup on
echo dirty >>"$t/main/a"
plan "$base" "$s1" "$s2" "$s3 fold"
runplan "plan: a dirty checkout is refused" 1 "dirty"
# 11: a clone not rebased onto a moved main is refused, and the landed log prints
setup on
echo z >"$t/main/z"; git -C "$t/main" add z; LAND_TOOL=1 $G -C "$t/main" commit -q -m "feat(x): moved"
one "a clone behind main is refused" 1 "rebase onto main"
setup on
out=$(bash "$LAND" --no-gate "$t/main" "$t/clone" "$sha" 2>&1)
check "the landed log shows identities, dates and subject" test "$(printf '%s\n' "$out" | tail -1 | grep -c 't <t@example.invalid>|t <t@example.invalid> .*|.* feat(x): second')" = 1
# 12: single mode runs the same gate on the landed result, and a red gate leaves main untouched.
# The gate here fails when the result holds a file that only main's tip has: green on the commit's
# own base, red once landed, the stale-golden shape.
single_gate() { # single_gate <gate-cmd> <messages-cmd>: main carries land.toml, the clone a change
  setup off
  printf '[landing]\ngate = ["%s"]\nmessages = "%s"\n' "$1" "$2" >"$t/main/config/schwarzwald/land.toml"
  git -C "$t/main" add -A; LAND_TOOL=1 $G -C "$t/main" commit -q -m "chore(x): gate"
  git -C "$t/clone" pull -q --rebase "$t/main" 2>/dev/null; sha=$(head_of clone)
}
single_gate "test ! -f b || { echo stale-golden; exit 1; }" "true"
before=$(head_of main)
run "single: a red gate on the landed result is refused" 1 "stale-golden" "$t/main" "$t/clone" "$sha"
check "single: a red gate leaves main untouched" test "$(head_of main)" = "$before"
check "single: a red gate leaves the main tree clean" test -z "$(git -C "$t/main" status --porcelain)"
single_gate "true" "echo msg-broke-{base}; exit 1"
run "single: a red message gate is refused, with the base" 1 "msg-broke-$(head_of main)" "$t/main" "$t/clone" "$sha"
single_gate "true" "true"
run "single: a green gate lands" 0 "" "$t/main" "$t/clone" "$sha"
check "single: the landed commit is main's tip" test "$(git -C "$t/main" log -1 --format=%s)" = "feat(x): second"
setup off
run "single: no land.toml is refused" 1 "landing gate: none configured" "$t/main" "$t/clone" "$sha"
# 13: a gate runs with the product's own git identity, not the one land.sh sets for its commits:
# a gate that makes fixture commits would otherwise see other shas.
leak='test -z ${GIT_AUTHOR_NAME:-}${GIT_AUTHOR_DATE:-}${GIT_COMMITTER_NAME:-}${GIT_COMMITTER_DATE:-}${LAND_TOOL:-} || { echo identity-leaked; exit 1; }'
single_gate "$leak" "true"
run "single: a gate does not inherit land.sh's identity and dates" 0 "" "$t/main" "$t/clone" "$sha"
MAIN_TOML=$(printf '[landing]\ngate = ["%s"]\nmessages = "true"\n' "$leak"); plan_setup on; MAIN_TOML=
plan "$base" "$s1" "$s2" "$s3 fold"
run "plan: a gate does not inherit land.sh's identity and dates" 0 "" --plan "$t/main" "$t/clone" "$planbase" "$t/plan"
# 14: a gate that checks the hooks are wired sees the target repository's core.hooksPath
hookgate='test \"$(git config --get core.hooksPath)\" = scripts/hooks || { echo hooks-unwired; exit 1; }'
single_gate "$hookgate" "true"
git -C "$t/main" config core.hooksPath scripts/hooks
run "single: the gate sees the target's core.hooksPath" 0 "" "$t/main" "$t/clone" "$sha"
single_gate "$hookgate" "true"
run "single: without core.hooksPath on the target the gate sees none" 1 "hooks-unwired" "$t/main" "$t/clone" "$sha"
# 14b: git config from the environment (GIT_CONFIG_COUNT/KEY/VALUE, GIT_CONFIG_PARAMETERS) does
# not reach a gate: the fixture keeps its own hooks
cfggate='h=$(git config --get core.hooksPath || true); test \"$h\" != /leaked || { echo config-leaked; exit 1; }'
single_gate "$cfggate" "true"
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/leaked \
  run "single: GIT_CONFIG_COUNT does not leak into the gate" 0 "" "$t/main" "$t/clone" "$sha"
single_gate "$cfggate" "true"
GIT_CONFIG_PARAMETERS="'core.hookspath'='/leaked'" \
  run "single: GIT_CONFIG_PARAMETERS does not leak into the gate" 0 "" "$t/main" "$t/clone" "$sha"
# 15 (ST-2): a sha argument that is a git option is refused before any git call runs it
setup off
rm -f "$t/opt-ran"
run "single: a sha that is a git option is refused" 1 "not a commit id" --no-gate "$t/main" "$t/clone" "--upload-pack=touch $t/opt-ran"
check "single: the option sha ran nothing" test ! -e "$t/opt-ran"
run "single: a ref name is refused, only a hex id" 1 "not a commit id" --no-gate "$t/main" "$t/clone" "HEAD"
# 16 (ST-3): a plan line that is a git option is refused before any git call, so is a bad mode
plan_setup on
rm -f "$t/plan-wrote"
plan "$base" "--output=$t/plan-wrote" "$s1" "$s2" "$s3 fold"
runplan "plan: an option as a plan sha is refused" 1 "not a commit id"
check "plan: the option line wrote no file" test ! -e "$t/plan-wrote"
plan "$base" "$s1" "$s2 squash" "$s3 fold"
runplan "plan: an unknown mode word is refused" 1 "unknown mode"
run "plan: a base that is a git option is refused" 1 "not a commit id" --no-gate --plan "$t/main" "$t/clone" "--output=$t/plan-wrote" "$t/plan"
check "plan: the option base wrote no file" test ! -e "$t/plan-wrote"
exit "$fail"
