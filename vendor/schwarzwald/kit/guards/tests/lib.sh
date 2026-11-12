#!/usr/bin/env bash
# The selftest's harness: the fixture, the scratch repo helper and the case functions
# (t, ta). Sourced by selftest.sh, never run on its own.
. "$(dirname "${BASH_SOURCE[0]}")/prelude.sh"

# The suite runs against a FIXTURE product, never the checkout it is called from: a fresh org
# under GUARDS_SELFTEST_ROOT (default ~/.cache) holding a product with the shared part where a product keeps it
# (<product>/schwarzwald/kit/guards), its config and the few files the cases name. The config
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
GUARDS_HOME="$REPO/schwarzwald/kit/guards"
mkdir -p "$GUARDS_HOME" "$REPO/schwarzwald/config" "$REPO/.runs" "$ORG/sibling" \
    "$REPO/crates/core/src" "$REPO/scripts/lint" "$REPO/docs" "$REPO/xtask/src"
cp -Rp "$GUARDS_SRC/." "$GUARDS_HOME/"
if [ -n "${GUARDS_SELFTEST_PRODUCT:-}" ]; then
    cp "$GUARDS_SELFTEST_PRODUCT/schwarzwald/config/guards.env" "$REPO/schwarzwald/config/guards.env" || exit 1
else
    cp "$GUARDS_SRC/config.example.env" "$REPO/schwarzwald/config/guards.env"
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
export GUARD_CONFIG="$REPO/schwarzwald/config/guards.env"

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
# A key ending in _ENV names a variable of the product's own (the lane, the build lock): the
# engine reads that variable, so the caller's value of it would leak into every case. Each is
# cleared too, whatever the product named it.
for _k in $GUARD_CONFIG_KEYS; do
    case "$_k" in *_ENV) ;; *) continue ;; esac
    _named=$(GUARD_CONFIG="$REPO/schwarzwald/config/guards.env" bash -c \
        '. "$1/config.sh"; unset "$2"; guard_config_load >/dev/null 2>&1; printf %s "${!2:-}"' _ "$GUARDS_HOME" "$_k")
    case "$_named" in "" | *[!A-Za-z0-9_]* | [0-9]*) ;; *) UNSET_ENV+=(-u "$_named") ;; esac
done
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

# A second install of the guards under test, laid out as a product keeps it: $1/<kit>/guards
# plus $1/schwarzwald/config/guards.env, where <kit> is $2 (schwarzwald/kit by default, or
# vendor/schwarzwald/kit). Prints the guards directory. The adapters find the product and the
# org from the kit's place, so a bare copy elsewhere would ask about the wrong tree.
_guards_install() {
    local p="$1" kit="${2:-schwarzwald/kit}"
    mkdir -p "$p/$kit/guards" "$p/schwarzwald/config"
    cp -Rp "$GUARDS_SRC/." "$p/$kit/guards/" 2>/dev/null
    cp "$REPO/schwarzwald/config/guards.env" "$p/schwarzwald/config/guards.env"
    # The config names the product's rule files and counts them, so a second install without them
    # is an INCOMPLETE product, not the one under test.
    for _d in $PRODUCT_RULE_DIRS; do
        mkdir -p "$p/$_d" && cp -Rp "$REPO/$_d/." "$p/$_d/"
    done
    printf '%s' "$p/$kit/guards"
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
# One engine run on the request in $1 with GUARD_REPO $2, attribution on, then KEY=VAL... as env.
# Sets _RUN_OUT (the verdict line) and _RUN_ERR (stderr, attribution lines included). Every
# input is stated in the request: the ambient variables are cleared and the machine-sensing
# rules neutralised (unless _RUN_LIVE=1: a state case reads the machine on purpose), so the
# verdict cannot depend on who ran the suite.
_RUN_VF="$SELFTEST_ROOT/verdict"
_run_engine() {
    local req="$1" repo="$2"
    shift 2
    local -a neutral=("${NEUTRAL_ENV[@]}")
    [ -z "${_RUN_LIVE:-}" ] || neutral=(GUARD_ATTRIBUTION=1)
    _RUN_ERR=$(printf '%s' "$req" |
        env "${UNSET_ENV[@]}" GUARD_REPO="$repo" GUARD_ATTRIBUTION=1 \
            "${neutral[@]}" "$@" bash "$ENGINE" 2>&1 >"$_RUN_VF")
    _RUN_OUT=$(<"$_RUN_VF")
}

# $1=expect BLOCK|ALLOW|ADVISE  $2=rule-id  $3=cwd  $4=tool  $5=input-json  $6=label [KEY=VAL ...]
# One run, attributed: the verdict line says BLOCK, any advice says ADVISE, and the
# `guard-block:`/`guard-advise:` lines say WHICH rule spoke. An unattributed verdict would pass
# while a SIBLING blocks or advises on the same input, so a rule shadowed by a broader one
# (git_add_scope under shared_index_staging) would pass its own cases with its body deleted.
# `scope` names the engine's org boundary, not a rule: any rule may block there.
t() {
    local expect="$1" rule="$2" cwd="$3" tool="$4" input="$5" label="$6"
    shift 6
    _only "$label" || return 0
    local got who="" kind
    # The shared destructive-class rule is OFF for every other rule's cases: it also refuses
    # `git stash`, `sed -i` and a checkout of a path, so with it on a case for the dedicated rule
    # would pass with that rule removed. Only destructive_class's own cases run it.
    local nodc=1
    [ "$rule" != destructive_class ] || nodc=""
    _run_engine "$(printf '{"event":"%s","tool":"%s","cwd":"%s","status":"ok","input":%s}' \
        "${EVENT:-pre_tool}" "$tool" "$cwd" "$input")" "${CASE_REPO:-$REPO}" \
        ${nodc:+GUARD_SELFTEST_NO_DESTRUCTIVE=1} "$@"
    local advised
    advised=$(sed -n 's/^guard-advise: //p' <<<"$_RUN_ERR" | sort -u | tr '\n' ' ')
    if [ "$_RUN_OUT" = "verdict: block" ]; then
        got=BLOCK
        who=$(sed -n 's/^guard-block: //p' <<<"$_RUN_ERR" | sort -u | tr '\n' ' ')
    elif [ -n "$advised" ] || [ -n "$(grep -v '^guard-' <<<"$_RUN_ERR")" ]; then
        got=ADVISE
        who="$advised"
    else
        got=ALLOW
    fi
    kind=""
    case "$expect/$got" in
        ADVISE/ADVISE) kind=advice ;;
        BLOCK/BLOCK) [ "$rule" = scope ] || kind=block ;;
        ALLOW/ADVISE)
            # An ALLOW case is a claim about THIS rule, not about global silence: a sibling
            # advising on the same input is fine, and the message names it.
            CASES=$((CASES + 1))
            COVERED="$COVERED$rule "
            case " $who " in
                *" $rule "*) ;;
                *)
                    printf '  ok   [ALLOW ] %s (a sibling spoke: %s)\n' "$label" "${who% }"
                    return
                    ;;
            esac
            printf '  FAIL expected ALLOW from %s but %s advised -- %s\n' "$rule" "${who:-<unattributed>}" "$label"
            FAIL_LOG="${FAIL_LOG}  FAIL expected ALLOW from $rule but ${who:-<unattributed>} advised -- $label
"
            FAILED=1
            return
            ;;
    esac
    if [ -n "$kind" ]; then
        case " $who " in
            *" $rule "*) ;;
            *)
                printf '  FAIL expected %s from %s but the %s came from: %s -- %s\n' \
                    "$expect" "$rule" "$kind" "${who:-<none>}" "$label"
                FAIL_LOG="${FAIL_LOG}  FAIL expected $expect from $rule but the $kind came from: ${who:-<none>} -- $label
"
                FAILED=1
                CASES=$((CASES + 1))
                COVERED="$COVERED$rule "
                return
                ;;
        esac
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
    local expect="$1" rule="$2" cmd="$3" label="$4" got
    _only "$label" || return 0
    _run_engine "$(printf '{"event":"pre_tool","tool":"bash","cwd":"%s","status":"ok","input":%s}' \
        "$REPO" "$(jq -cn --arg c "$cmd" '{command:$c}')")" "${CASE_REPO:-$REPO}"
    if grep -q "^guard-advise: $rule\$" <<<"$_RUN_ERR"; then got=ADVISE; else got=SILENT; fi
    _assert "$expect" "$got" "$label" "$rule"
}
# A hand-rolled state case, shared because a product's cases use it too and a partial run
# may load those without cases-state.
# _hr WANT(FIRE|QUIET) RULE EVENT TOOL CWD INPUT-JSON LABEL [KEY=VAL ...]
_hr() {
    local want="$1" rule="$2" event="$3" tool="$4" cwd="$5" input="$6" label="$7" got
    shift 7
    _RUN_LIVE=1 _run_engine "$(printf '{"event":"%s","tool":"%s","cwd":"%s","status":"ok","input":%s}' \
        "$event" "$tool" "$cwd" "$input")" "$cwd" \
        GUARD_SELFTEST_NO_INDEX=1 GUARD_SELFTEST_NO_DESTRUCTIVE=1 "$@"
    if grep -Eq "^guard-(advise|block): $rule\$" <<<"$_RUN_ERR"; then got=FIRE; else got=QUIET; fi
    _assert "$want" "$got" "$label" "$rule"
}
