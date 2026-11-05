#!/usr/bin/env bash
# The selftest's harness: the fixture, the scratch repo helper and the case functions
# (t, ta). Sourced by selftest.sh, never run on its own.
. "$(dirname "${BASH_SOURCE[0]}")/prelude.sh"

# The suite runs against a FIXTURE product, never the checkout it is called from: a fresh org
# under GUARDS_SELFTEST_ROOT (default ~/.cache) holding a product with the shared part where a product keeps it
# (<product>/shared/schwarzwald/guards), its config and the few files the cases name. The config
# is config.example.env, or with GUARDS_SELFTEST_PRODUCT=<product root> that product's own
# guards.env, its rule dirs and its case files, so the shared cases also run against the real
# config. So every verdict is about the
# guards and the config, never about which tree or machine ran the suite. Not under /tmp or
# $TMPDIR: the guards treat those as scratch, where a delete is always fine, so a fixture there
# could never show a delete outside the org being refused.
unset GIT_INDEX_FILE GIT_DIR GIT_WORK_TREE GIT_PREFIX
command -v jq >/dev/null 2>&1 || {
    echo "guards-selftest: jq is required and absent -- abstaining would hide every case" >&2
    exit 1
}
mkdir -p "${GUARDS_SELFTEST_ROOT:-$HOME/.cache}" || exit 1
SELFTEST_ROOT="$(mktemp -d "${GUARDS_SELFTEST_ROOT:-$HOME/.cache}/guards-selftest.XXXXXX")"
SELFTEST_ROOT="$(cd "$SELFTEST_ROOT" && pwd -P)"
trap 'rm -rf "$SELFTEST_ROOT"' EXIT
ORG="$SELFTEST_ROOT/org"
REPO="$ORG/product"
GUARDS_HOME="$REPO/shared/schwarzwald/guards"
mkdir -p "$GUARDS_HOME" "$REPO/config/schwarzwald" "$REPO/.runs" "$ORG/sibling" \
    "$REPO/crates/core/src" "$REPO/scripts/lint" "$REPO/docs" "$REPO/xtask/src"
cp -Rp "$GUARDS_SRC/." "$GUARDS_HOME/"
if [ -n "${GUARDS_SELFTEST_PRODUCT:-}" ]; then
    cp "$GUARDS_SELFTEST_PRODUCT/config/schwarzwald/guards.env" "$REPO/config/schwarzwald/guards.env" || exit 1
else
    cp "$GUARDS_SRC/config.example.env" "$REPO/config/schwarzwald/guards.env"
fi
printf 'fn a() {}\n' >"$REPO/crates/core/src/lib.rs"
printf 'all:\n' >"$REPO/Makefile"
printf '[workspace]\n' >"$REPO/Cargo.toml"
printf '# fixture\n' >"$REPO/README.md"
printf 'fn main() {}\n' >"$REPO/xtask/src/api_md.rs"
printf '#!/usr/bin/env bash\n' >"$REPO/scripts/bench-run.py"
printf '#!/usr/bin/env bash\n' >"$REPO/scripts/lint/guards-selftest.sh"
git -C "$REPO" init -q && git -C "$REPO" -c user.name=probe -c user.email=probe@local \
    -c commit.gpgsign=false commit -q --allow-empty -m base
# The guards treat "$TMPDIR/..." as scratch, which is safe to delete. The suite runs with a
# private $TMPDIR that is never an ancestor of $REPO, so no verdict depends on where it sits.
SELFTEST_TMP="$SELFTEST_ROOT/tmp"
mkdir -p "$SELFTEST_TMP"
export TMPDIR="$SELFTEST_TMP"
ENGINE="$GUARDS_HOME/engine.sh"
# The scratch roots come from the fixture's config like every other value: an inherited
# GUARD_SCRATCH_ROOTS is cleared per case below.
# The product's config is the fixture's, whatever GUARD_REPO a case points elsewhere.
unset GUARD_REPO
export GUARD_CONFIG="$REPO/config/schwarzwald/guards.env"

# The machine-sensing rules neutralised, so a verdict never depends on who ran the suite.
NEUTRAL_ENV=(GUARD_SELFTEST_NO_LSOF=1 GUARD_SELFTEST_NO_PROCFS=1 GUARD_SELFTEST_NO_INDEX=1)
# Every variable the config may set is cleared for each case, so the fixture's config is the
# only source: an exported GUARD_LANE or GUARD_APP_RE in the caller's shell changes nothing.
# shellcheck source=/dev/null
. "$GUARDS_HOME/config.sh"
UNSET_ENV=(-u FULL_SUITE_OK -u LOCK_OK -u GUARD_LANE -u GUARD_SCRATCH_DIR)
for _k in $GUARD_CONFIG_KEYS; do
    UNSET_ENV+=(-u "$_k")
done
# The product's own lane variable (GUARD_LANE_ENV), read into GUARD_LANE by the engine.
_lane_env=$(GUARD_CONFIG="$REPO/config/schwarzwald/guards.env" bash -c \
    '. "$1/config.sh"; unset GUARD_LANE_ENV; guard_config_load >/dev/null 2>&1; printf %s "${GUARD_LANE_ENV:-}"' _ "$GUARDS_HOME")
[ -z "$_lane_env" ] || UNSET_ENV+=(-u "$_lane_env")
# The product's own rules and cases, copied into the fixture at the same relative paths.
PRODUCT_CASES=""
PRODUCT_RULE_DIRS=""
if [ -n "${GUARDS_SELFTEST_PRODUCT:-}" ]; then
    _pv=$(bash -c '. "$1/config.sh"; for k in $GUARD_CONFIG_KEYS; do unset "$k"; done
        guard_config_load >/dev/null 2>&1; printf "%s\n%s\n" "${GUARD_PRODUCT_RULE_DIRS:-}" "${GUARD_PRODUCT_CASES:-}"' _ "$GUARDS_HOME")
    PRODUCT_RULE_DIRS=$(sed -n 1p <<<"$_pv")
    for _d in $PRODUCT_RULE_DIRS; do
        mkdir -p "$REPO/$_d" && cp -Rp "$GUARDS_SELFTEST_PRODUCT/$_d/." "$REPO/$_d/"
    done
    for _c in $(sed -n 2p <<<"$_pv"); do
        mkdir -p "$REPO/${_c%/*}" && cp -p "$GUARDS_SELFTEST_PRODUCT/$_c" "$REPO/$_c"
        PRODUCT_CASES="$PRODUCT_CASES $REPO/$_c"
    done
fi
FAILED=0
FAIL_LOG=""
CASES=0
# Which rule each case exercises, so coverage is checkable rather than assumed.
COVERED=" "

# A disposable git repo under .runs/ (inside the org: the engine only acts there) holding
# byte copies of the given repo-relative paths, committed.
_SCRATCH_PREFIX="$REPO/.runs/guards-scratch.$$"
_scratch_repo() {
    local dir rel
    dir=$(mktemp -d "$_SCRATCH_PREFIX.XXXXXX")
    for rel in "$@"; do
        mkdir -p "$dir/$(dirname "$rel")"
        cp "$REPO/$rel" "$dir/$rel"
    done
    git -C "$dir" init -q &&
        git -C "$dir" add -- "$@" &&
        git -C "$dir" -c user.name=probe -c user.email=probe@local commit -qm base
    printf '%s' "$dir"
}

# A second install of the guards under test, laid out as a product keeps it: $1/shared/
# schwarzwald/guards plus $1/config/schwarzwald/guards.env. Prints the guards directory. The
# adapters find the product and the org by stripping fixed levels, so a bare copy elsewhere
# would ask about the wrong tree.
_guards_install() {
    local p="$1"
    mkdir -p "$p/shared/schwarzwald/guards" "$p/config/schwarzwald"
    cp -Rp "$GUARDS_SRC/." "$p/shared/schwarzwald/guards/" 2>/dev/null
    cp "$REPO/config/schwarzwald/guards.env" "$p/config/schwarzwald/guards.env"
    # The config names the product's rule files and counts them, so a second install without them
    # is an INCOMPLETE product, not the one under test.
    for _d in $PRODUCT_RULE_DIRS; do
        mkdir -p "$p/$_d" && cp -Rp "$REPO/$_d/." "$p/$_d/"
    done
    printf '%s' "$p/shared/schwarzwald/guards"
}
# GUARDS_SELFTEST_ONLY, an ERE over the case label, runs only the matching t and ta cases (the
# hand-rolled cases always run). A red probe sets it to the case it must turn red, so it pays
# for one case instead of the suite. Unset, every case runs.
_only() {
    [ -n "${GUARDS_SELFTEST_ONLY:-}" ] || return 0
    [[ $1 =~ $GUARDS_SELFTEST_ONLY ]]
}
# $1=want $2=got $3=label $4=rule (optional): count one case, record coverage, print ok or FAIL.
_assert() {
    CASES=$((CASES + 1))
    [ -z "${4:-}" ] || COVERED="$COVERED$4 "
    if [ "$2" = "$1" ]; then
        printf '  ok   [%-6s] %s\n' "$2" "$3"
    else
        printf '  FAIL expected %s got %s -- %s\n' "$1" "$2" "$3"
        FAIL_LOG="${FAIL_LOG}  FAIL expected $1 got $2 -- $3
"
        FAILED=1
    fi
}
# $1=expect BLOCK|ALLOW|ADVISE  $2=rule-id  $3=cwd  $4=tool  $5=input-json  $6=label [KEY=VAL ...]
t() {
    local expect="$1" rule="$2" cwd="$3" tool="$4" input="$5" label="$6"
    shift 6
    _only "$label" || return 0
    local req out err got vf
    # The shared destructive-class rule is OFF for every other rule's cases: it
    # also refuses `git stash`, `sed -i` and a checkout of a path, so with it on a
    # case for the dedicated rule would pass with that rule removed. Only
    # destructive_class's own cases run it.
    local nodc=1
    [ "$rule" != destructive_class ] || nodc=""
    req=$(printf '{"event":"%s","tool":"%s","cwd":"%s","status":"ok","input":%s}' \
        "${EVENT:-pre_tool}" "$tool" "$cwd" "$input")
    vf=$(mktemp "${TMPDIR:-/tmp}/guards-selftest.XXXXXX")
    # Every input is stated in `$req`: the ambient variables are cleared and the
    # machine-sensing rules neutralised, so the verdict cannot depend on who ran
    # the suite. GUARD_REPO is pinned here rather than exported.
    err=$(printf '%s' "$req" |
        env "${UNSET_ENV[@]}" \
            GUARD_REPO="${CASE_REPO:-$REPO}" \
            "${NEUTRAL_ENV[@]}" ${nodc:+GUARD_SELFTEST_NO_DESTRUCTIVE=1} "$@" bash "$ENGINE" 2>&1 >"$vf")
    out=$(cat "$vf")
    rm -f "$vf"

    if [ "$out" = "verdict: block" ]; then
        got=BLOCK
    elif [ -n "$err" ]; then
        got=ADVISE
    else
        got=ALLOW
    fi

    # An ADVISE verdict from `t` is UNATTRIBUTED: it classifies on any stderr, so
    # a SIBLING advising on the same input reads as this rule advising. `ta`
    # attributes but reads only the advise channel, so it cannot serve a BLOCK
    # rule.
    #
    # So when the verdict is ADVISE and the case expected one, re-ask with
    # attribution and fail if a DIFFERENT rule spoke.
    if [ "$got" = ADVISE ] && [ "$expect" = ADVISE ]; then
        local who
        who=$(printf '%s' "$req" |
            env "${UNSET_ENV[@]}" \
                GUARD_REPO="${CASE_REPO:-$REPO}" GUARD_ATTRIBUTION=1 \
                "${NEUTRAL_ENV[@]}" ${nodc:+GUARD_SELFTEST_NO_DESTRUCTIVE=1} "$@" \
                bash "$ENGINE" 2>&1 >/dev/null |
            sed -n 's/^guard-advise: //p' | sort -u | tr '\n' ' ')
        case " $who " in
            *" $rule "*) ;;
            *)
                printf '  FAIL expected ADVISE from %s but the advice came from: %s -- %s\n' \
                    "$rule" "${who:-<none>}" "$label"
                FAIL_LOG="${FAIL_LOG}  FAIL expected ADVISE from $rule but the advice came from: ${who:-<none>} -- $label
"
                FAILED=1
                CASES=$((CASES + 1))
                COVERED="$COVERED$rule "
                return
                ;;
        esac
    fi

    # The same for BLOCK: an unattributed BLOCK passes while a SIBLING blocks the input, so a
    # rule shadowed by a broader one (git_add_scope under shared_index_staging) passes its own
    # cases with its body deleted, and its red-case probe stays green. `scope` names the
    # engine's org boundary, not a rule: any rule may be the one that blocks there.
    if [ "$got" = BLOCK ] && [ "$expect" = BLOCK ] && [ "$rule" != scope ]; then
        local blocker
        blocker=$(printf '%s' "$req" |
            env "${UNSET_ENV[@]}" \
                GUARD_REPO="${CASE_REPO:-$REPO}" GUARD_ATTRIBUTION=1 \
                "${NEUTRAL_ENV[@]}" ${nodc:+GUARD_SELFTEST_NO_DESTRUCTIVE=1} "$@" \
                bash "$ENGINE" 2>&1 >/dev/null |
            sed -n 's/^guard-block: //p' | sort -u | tr '\n' ' ')
        case " $blocker " in
            *" $rule "*) ;;
            *)
                printf '  FAIL expected BLOCK from %s but the block came from: %s -- %s\n' \
                    "$rule" "${blocker:-<none>}" "$label"
                FAIL_LOG="${FAIL_LOG}  FAIL expected BLOCK from $rule but the block came from: ${blocker:-<none>} -- $label
"
                FAILED=1
                CASES=$((CASES + 1))
                COVERED="$COVERED$rule "
                return
                ;;
        esac
    fi

    # The mirror of the ADVISE attribution above. A case expecting ALLOW fails on
    # ANY stderr, so a SIBLING advising on the same input reads as this rule
    # misbehaving, and the message names the wrong rule. Naming the speaker turns a
    # misleading failure into a directly actionable one.
    if [ "$got" = ADVISE ] && [ "$expect" = ALLOW ]; then
        local speaker
        speaker=$(printf '%s' "$req" |
            env "${UNSET_ENV[@]}" \
                GUARD_REPO="${CASE_REPO:-$REPO}" GUARD_ATTRIBUTION=1 \
                "${NEUTRAL_ENV[@]}" ${nodc:+GUARD_SELFTEST_NO_DESTRUCTIVE=1} "$@" \
                bash "$ENGINE" 2>&1 >/dev/null |
            sed -n 's/^guard-advise: //p' | sort -u | tr '\n' ' ')
        CASES=$((CASES + 1))
        COVERED="$COVERED$rule "
        # An ALLOW case is a claim about THIS rule, not about global silence, and
        # attribution is what lets the two be told apart. Two rules can both be
        # right on one input. Failing here would force one of them to be wrong.
        case " $speaker " in
            *" $rule "*) ;;
            *)
                printf '  ok   [ALLOW ] %s (a sibling spoke: %s)\n' "$label" "${speaker% }"
                return
                ;;
        esac
        printf '  FAIL expected ALLOW from %s but %s advised -- %s\n' \
            "$rule" "${speaker:-<unattributed>}" "$label"
        FAIL_LOG="${FAIL_LOG}  FAIL expected ALLOW from $rule but ${speaker:-<unattributed>} advised -- $label
"
        FAILED=1
        return
    fi

    _assert "$expect" "$got" "$label" "$rule"
}

# $1=expect ADVISE|SILENT  $2=rule-id  $3=command  $4=label
# ATTRIBUTED assertion: did THIS rule speak, rather than "was there any stderr"?
# `t` above classifies on whether stderr is non-empty, which is an AGGREGATE
# reading and cannot see one rule among several: when a sibling advises on
# every input of a rule, a `t` case passes with that rule NEUTERED.
# GUARD_ATTRIBUTION makes the engine prefix each message with the id of the
# rule that emitted it, so this asserts a rule BY NAME. A sibling advising on
# the same input is now irrelevant to the verdict. SILENT is the direction
# that matters most: grepping for a rule's own TEXT would prove firing but
# nothing would prove NOT firing, and over-reach is what gets a guard routed
# around. Both directions ask the same question of the same channel.
ta() {
    local expect="$1" rule="$2" cmd="$3" label="$4"
    _only "$label" || return 0
    local req err got
    req=$(printf '{"event":"pre_tool","tool":"bash","cwd":"%s","status":"ok","input":%s}' \
        "$REPO" "$(jq -cn --arg c "$cmd" '{command:$c}')")
    err=$(printf '%s' "$req" |
        env "${UNSET_ENV[@]}" \
            GUARD_REPO="${CASE_REPO:-$REPO}" GUARD_ATTRIBUTION=1 \
            "${NEUTRAL_ENV[@]}" \
            bash "$ENGINE" 2>&1 >/dev/null)

    if grep -q "^guard-advise: $rule\$" <<<"$err"; then
        got=ADVISE
    else
        got=SILENT
    fi

    _assert "$expect" "$got" "$label" "$rule"
}
# A hand-rolled state case, shared because a product's cases use it too and a partial run
# may load those without cases-state.
# _hr WANT(FIRE|QUIET) RULE EVENT TOOL CWD INPUT-JSON LABEL [KEY=VAL ...]
_hr() {
    local want="$1" rule="$2" event="$3" tool="$4" cwd="$5" input="$6" label="$7" err got
    shift 7
    err=$(printf '{"event":"%s","tool":"%s","cwd":"%s","status":"ok","input":%s}' \
        "$event" "$tool" "$cwd" "$input" |
        env "${UNSET_ENV[@]}" \
            GUARD_REPO="$cwd" GUARD_ATTRIBUTION=1 GUARD_SELFTEST_NO_INDEX=1 \
            GUARD_SELFTEST_NO_DESTRUCTIVE=1 "$@" \
            bash "$ENGINE" 2>&1 >/dev/null)
    if grep -Eq "^guard-(advise|block): $rule\$" <<<"$err"; then got=FIRE; else got=QUIET; fi
    _assert "$want" "$got" "$label" "$rule"
}
