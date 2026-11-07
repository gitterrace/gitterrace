#!/usr/bin/env bash
# Sourced by selftest.sh.
echo
echo "advisories -- these must never block"
t ADVISE new_script_duplication "$REPO" write '{"file_path":"'"$REPO"'/scripts/bench-chart-run.py"}' "new script whose noun exists"
t ALLOW new_script_duplication "$REPO" write '{"file_path":"'"$REPO"'/scripts/zzqqxx-check.py"}' "new script, novel noun"

EVENT=post_tool
t ADVISE post_check_generated "$REPO" edit '{"file_path":"'"$REPO"'/Makefile"}' "Makefile edit reminds about the generation gate" GUARD_GENERATED_FEEDS='*/Makefile */xtask/src/*'
t ADVISE post_check_generated "$REPO" edit '{"file_path":"'"$REPO"'/xtask/src/api_md.rs"}' "a generator edit reminds about the generation gate" GUARD_GENERATED_FEEDS='*/Makefile */xtask/src/*'
t ALLOW post_check_generated "$REPO" edit '{"file_path":"'"$REPO"'/README.md"}' "an ordinary file says nothing" GUARD_GENERATED_FEEDS='*/Makefile */xtask/src/*'
t ADVISE post_check_generated "$REPO" edit '{"file_path":"'"$REPO"'/README.md"}' "an install's own feed list" GUARD_GENERATED_FEEDS='*/README.md'
# Advice about a file that did not change is noise.
req_fail=$(printf '{"event":"post_tool","tool":"edit","cwd":"%s","status":"error","input":{"file_path":"%s/Makefile"}}' "$REPO" "$REPO")
err_fail=$(printf '%s' "$req_fail" | bash "$ENGINE" 2>&1 >/dev/null)
CASES=$((CASES + 1))
if [ -z "$err_fail" ]; then
    printf '  ok   [ALLOW ] a FAILED tool call produces no advice\n'
else
    printf '  FAIL expected silence after a failed call -- got: %s\n' "$err_fail"
    FAILED=1
fi
EVENT=pre_tool

# Rules whose verdict comes from MACHINE STATE. `t` pins that state off,
# so these are hand-rolled: the state is replaced by a stub the case controls (a `lsof`/`ps`
# on PATH), which proves the rule's own plumbing fires and
# stays quiet. Verdicts are read by attribution so a sibling rule cannot satisfy a case.
_hr_dir=$(mktemp -d "$_SCRATCH_PREFIX.XXXXXX")
_hr_repo="$_hr_dir/repo"
mkdir -p "$_hr_repo/target/debug" "$_hr_dir/shim"
: >"$_hr_repo/target/debug/.cargo-lock"
_hr_stub() { printf '#!/usr/bin/env bash\n%s\n' "$2" >"$1" && chmod +x "$1"; }

echo "state-driven rules -- stubbed machine state and delegated scripts"
# cargo_lock_contention: two holders of the cargo lock.
_hr_stub "$_hr_dir/shim/lsof" 'printf "COMMAND PID\ncargo 1\ncargo 2\n"'
_hr FIRE cargo_lock_contention pre_tool bash "$_hr_repo" '{"command":"cargo test"}' "a foreground build while two cargos hold the lock" PATH="$_hr_dir/shim:$PATH"
_hr_stub "$_hr_dir/shim/lsof" 'printf "COMMAND PID\ncargo 1\n"'
_hr QUIET cargo_lock_contention pre_tool bash "$_hr_repo" '{"command":"cargo test"}' "one holder is not contention" PATH="$_hr_dir/shim:$PATH"
_hr_stub "$_hr_dir/shim/lsof" 'printf "COMMAND PID\ncargo 1\ncargo 2\n"'
_hr QUIET cargo_lock_contention pre_tool bash "$_hr_repo" '{"command":"LOCK_OK=1 cargo test"}' "LOCK_OK forces the build through" PATH="$_hr_dir/shim:$PATH"

# process_table_pressure: a fake `ps` reports a full table, then an almost empty one.
_hr_stub "$_hr_dir/shim/ps" 'yes S | head -n 200000'
_hr FIRE process_table_pressure pre_tool bash "$_hr_repo" '{"command":"echo hi"}' "a saturated process table warns" PATH="$_hr_dir/shim:$PATH"
_hr_stub "$_hr_dir/shim/ps" 'echo S'
_hr QUIET process_table_pressure pre_tool bash "$_hr_repo" '{"command":"echo hi"}' "a near-empty process table stays quiet" PATH="$_hr_dir/shim:$PATH"
rm -f "$_hr_dir/shim/ps" "$_hr_dir/shim/lsof"

rm -rf "$_hr_dir"

echo
echo "robustness -- the engine must never be the thing that breaks a session"
# Two classes, kept apart. A payload that PARSES but asks about nothing (`{}`,
# `null`) is a complete question with nothing to guard: allow. A payload that does
# not parse (truncated, empty, not JSON) is a question the engine could not read, so
# it emits NO verdict, which every pre adapter hands to the fail-closed fallback.
for payload in '{}' 'null'; do
    out=$(printf '%s' "$payload" | bash "$ENGINE" 2>/dev/null)
    CASES=$((CASES + 1))
    if [ "$out" = "verdict: allow" ]; then
        printf '  ok   [ALLOW ] a request that asks about nothing allows: %s\n' "${payload:0:28}"
    else
        printf '  FAIL a request that asks about nothing must allow (%s): %s\n' "$out" "${payload:0:28}"
        FAILED=1
    fi
done
for payload in '{"event":"pre_tool","tool":"bash","cwd":"'"$REPO"'","input":{"command":' '' 'not json at all'; do
    out=$(printf '%s' "$payload" | bash "$ENGINE" 2>/dev/null)
    CASES=$((CASES + 1))
    if [ -z "$out" ]; then
        printf '  ok   [NOVERD] an unparseable request gives NO verdict, never allow: %s\n' "${payload:0:28}"
    else
        printf '  FAIL an unparseable request must give no verdict (%s): %s\n' "$out" "${payload:0:28}"
        FAILED=1
    fi
done
COVERED="$COVERED malformed "

# A verdict is ALWAYS emitted on a completed run, and the exit status is ALWAYS
# 0. Exit status carries no verdict: bash exits 2 on a syntax error and 2 is also
# a harness's "block" code, so conflating them makes a broken guard file
# indistinguishable from a considered refusal.
# GUARD_REPO pinned for the same reason as in t(): a COPIED engine scopes its
# org to its own location and would abstain here, making this assertion pass
# vacuously.
out=$(printf '{"event":"pre_tool","tool":"bash","cwd":"%s","input":{"command":"git add ."}}' "$REPO" | GUARD_REPO="$REPO" bash "$ENGINE" 2>/dev/null)
rc=$?
CASES=$((CASES + 1))
if [ "$rc" -eq 0 ] && [ "$out" = "verdict: block" ]; then
    printf '  ok   [BLOCK ] a block is a VERDICT LINE, not an exit code (rc=0)\n'
else
    printf '  FAIL a block must exit 0 and print a verdict line -- rc=%s out=%s\n' "$rc" "$out"
    FAILED=1
fi
COVERED="$COVERED verdict_contract "

# DEGRADATION. The engine must fail OPEN (never block a session) and LOUDLY
# (never allow silently): a silent allow is indistinguishable from a clean run.
# Exercised on a COPY, with the copy's own derived org root as cwd: any other cwd exits at the
# engine's scope guard before loading rules and reads as "silent".
#
# TWO warnings are asserted, from the two INDEPENDENT layers, because either
# one alone is enough to make this case pass: a case that passes when half the
# mechanism is gone is testing the half that remains.
degraded=$(_guards_install "$(mktemp -d "${TMPDIR:-/tmp}/guards-degrade.XXXXXX")")
rm -f "$degraded"/rules/*.sh
deg_out=$(bash "$degraded/adapters/cli.sh" --cwd "$degraded" --command 'git add .' 2>&1)
deg_rc=$?
rm -rf "$degraded"
CASES=$((CASES + 1))
deg_engine=$(printf '%s' "$deg_out" | grep -c 'failed to load rules')
deg_adapter=$(printf '%s' "$deg_out" | grep -c 'NOT running')
if [ "$deg_rc" -eq 0 ] && [ "$deg_engine" -ge 1 ] && [ "$deg_adapter" -ge 1 ]; then
    printf '  ok   [ALLOW ] no rules: fails OPEN (rc=0), warned by BOTH engine and adapter\n'
else
    printf '  FAIL no-rules degradation: rc=%s engine_warn=%s adapter_warn=%s\n' \
        "$deg_rc" "$deg_engine" "$deg_adapter"
    printf '       out: %s\n' "$deg_out"
    FAILED=1
fi
COVERED="$COVERED degradation "

# A MISSING RULE FILE must not be silent either, and this is a different failure
# from "no rules at all": with one file deleted, the rest still satisfy
# GUARD_MIN_RULES, so a count floor set below the real count cannot see one
# category go missing.
#
# The gate catches a rule deleted from the TREE (its cases fail). This catches a
# damaged INSTALL, in a checkout no gate ever runs in.
partial=$(_guards_install "$(mktemp -d "${TMPDIR:-/tmp}/guards-partial.XXXXXX")")
rm -f "$partial/rules/05-containment.sh"
par_out=$(bash "$partial/adapters/cli.sh" --cwd "$partial" --command 'rm -rf /etcXX/zz' 2>&1)
par_rc=$?
rm -rf "$partial"
CASES=$((CASES + 1))
# The engine gives no verdict, so the fail-closed fallback judges: `rm -r` is
# in its class and is refused even with containment gone.
if [ "$par_rc" -eq 1 ] && grep -q 'INCOMPLETE' <<<"$par_out" &&
    grep -q 'fail-closed fallback' <<<"$par_out"; then
    printf '  ok   [BLOCK ] a missing rule FILE says INCOMPLETE, and the fallback still refuses rm -r\n'
else
    printf '  FAIL missing-rule-file degradation: rc=%s out=%s\n' "$par_rc" "$par_out"
    FAILED=1
fi
COVERED="$COVERED partial_install "


# A product rule dir is loaded after the shared rules, and a product rule file that goes
# missing is INCOMPLETE too: the floor counts the product's files apart from the shared ones.
prod=$(mktemp -d "${TMPDIR:-/tmp}/guards-product.XXXXXX")
prod_g=$(_guards_install "$prod")
# This case's product has exactly one rule file of its own, whatever the product under test has.
rm -rf "$prod/rules"
mkdir -p "$prod/rules"
printf '%s\n' 'guard_register product_probe' 'guard_rule_product_probe() {' \
    '    guard_cmd_is "product-probe" && guard_block "the product rule spoke."' '    return 0' '}' >"$prod/rules/90-probe.sh"
printf '%s\n' "GUARD_PRODUCT_RULE_DIRS='rules'" "GUARD_PRODUCT_RULE_FILES='1'" >>"$prod/config/schwarzwald/guards.env"
pr_out=$(env "${UNSET_ENV[@]}" -u GUARD_CONFIG bash "$prod_g/adapters/cli.sh" --cwd "$prod" --command 'product-probe' 2>&1)
pr_rc=$?
_assert BLOCK "$([ "$pr_rc" -eq 1 ] && grep -q 'the product rule spoke' <<<"$pr_out" && echo BLOCK || echo "rc=$pr_rc")" \
    "a product rule dir named in the config is loaded" product_rules
# GUARD_REPO pointing at another checkout (a caller's scratch repo, a sibling project) still
# loads the install's OWN product rules: they belong to the product, not to the cwd.
other=$(mktemp -d "${TMPDIR:-/tmp}/guards-other.XXXXXX")
pr_out=$(env "${UNSET_ENV[@]}" -u GUARD_CONFIG GUARD_REPO="$other" bash "$prod_g/adapters/cli.sh" --cwd "$prod" --command 'product-probe' 2>&1)
pr_rc=$?
_assert BLOCK "$([ "$pr_rc" -eq 1 ] && grep -q 'the product rule spoke' <<<"$pr_out" && echo BLOCK || echo "rc=$pr_rc")" \
    "product rules load from the install's product, whatever GUARD_REPO says" product_rules
rm -rf "$other"
rm -f "$prod/rules/90-probe.sh"
pr_out=$(env "${UNSET_ENV[@]}" -u GUARD_CONFIG bash "$prod_g/adapters/cli.sh" --cwd "$prod" --command 'git status' 2>&1)
_assert INCOMPLETE "$(grep -q 'INCOMPLETE -- 15 shared and 0 product' <<<"$pr_out" && echo INCOMPLETE || echo silent)" \
    "a missing PRODUCT rule file says INCOMPLETE" product_rules
rm -rf "$prod"

# The config is the product's, so a missing one or one that is not data is no config at all:
# no verdict, the fallback judges loudly. A line that runs code is never run.
cfg=$(mktemp -d "${TMPDIR:-/tmp}/guards-config.XXXXXX")
cfg_g=$(_guards_install "$cfg")
rm -f "$cfg/config/schwarzwald/guards.env"
cf_out=$(env "${UNSET_ENV[@]}" -u GUARD_CONFIG bash "$cfg_g/adapters/cli.sh" --cwd "$cfg" --command 'git stash' 2>&1)
cf_rc=$?
_assert REFUSED "$([ "$cf_rc" -eq 1 ] && grep -q 'no usable config' <<<"$cf_out" && grep -q 'fail-closed fallback' <<<"$cf_out" && echo REFUSED || echo "rc=$cf_rc")" \
    "no config: no verdict, and the fallback refuses git stash" config_shape
printf '%s\n' 'GUARD_PRODUCT="$(touch '"$cfg"'/ran)"' >"$cfg/config/schwarzwald/guards.env"
cf_out=$(env "${UNSET_ENV[@]}" -u GUARD_CONFIG bash "$cfg_g/adapters/cli.sh" --cwd "$cfg" --command 'git status' 2>&1)
_assert REFUSED "$(grep -q 'line not data' <<<"$cf_out" && [ ! -e "$cfg/ran" ] && echo REFUSED || echo ran)" \
    "a config line that is code is refused and never run" config_shape
printf '%s\n' "GUARD_NOT_A_KEY='x'" >"$cfg/config/schwarzwald/guards.env"
cf_out=$(env "${UNSET_ENV[@]}" -u GUARD_CONFIG bash "$cfg_g/adapters/cli.sh" --cwd "$cfg" --command 'git status' 2>&1)
_assert NAMED "$(grep -q 'unknown key GUARD_NOT_A_KEY' <<<"$cf_out" && echo NAMED || echo silent)" \
    "a config key the guards do not read is named on every call" config_shape
# One stray key must not degrade every guard to the fallback: the ENGINE still blocks, with
# rules the fallback does not hold (git add -A), and the key is named.
{ cat "$REPO/config/schwarzwald/guards.env"; printf '%s\n' "GUARD_NOT_A_KEY='x'"; } >"$cfg/config/schwarzwald/guards.env"
cf_out=$(env "${UNSET_ENV[@]}" -u GUARD_CONFIG bash "$cfg_g/adapters/cli.sh" --cwd "$cfg" --command 'git stash' 2>&1)
cf_rc=$?
_assert BLOCK "$([ "$cf_rc" -eq 1 ] && grep -q 'unknown key GUARD_NOT_A_KEY' <<<"$cf_out" && ! grep -q 'NOT running' <<<"$cf_out" && echo BLOCK || echo "rc=$cf_rc")" \
    "an unknown key keeps the engine's BLOCK for git stash and names the key" config_shape
cf_out=$(env "${UNSET_ENV[@]}" -u GUARD_CONFIG bash "$cfg_g/adapters/cli.sh" --cwd "$cfg" --command 'git add -A' 2>&1)
cf_rc=$?
_assert BLOCK "$([ "$cf_rc" -eq 1 ] && grep -q 'unknown key GUARD_NOT_A_KEY' <<<"$cf_out" && echo BLOCK || echo "rc=$cf_rc")" \
    "an unknown key does not degrade the guards: git add -A is still blocked" config_shape
{ cat "$REPO/config/schwarzwald/guards.env"; printf '%s\n' "MAIN_CHECKOUT_LANDS_ONLY='1'"; } >"$cfg/config/schwarzwald/guards.env"
cf_out=$(env "${UNSET_ENV[@]}" -u GUARD_CONFIG bash "$cfg_g/adapters/cli.sh" --cwd "$cfg" --command 'git add -A' 2>&1)
cf_rc=$?
_assert BLOCK "$([ "$cf_rc" -eq 1 ] && ! grep -q 'unknown key\|NOT running\|no usable config' <<<"$cf_out" && echo BLOCK || echo "rc=$cf_rc")" \
    "MAIN_CHECKOUT_LANDS_ONLY set does not degrade the guards" config_shape
rm -rf "$cfg"
