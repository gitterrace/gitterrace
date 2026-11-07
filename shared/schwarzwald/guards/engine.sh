#!/usr/bin/env bash
#
# The tool-use guard ENGINE: one copy for every harness. An adapter translates
# its harness's input into the contract in README.md and maps the verdict back.
#
# Exit status carries NO verdict: bash exits 2 on a syntax error, which is also
# a harness "block" code, so a guard file that failed to parse would look
# like a refusal. A broken engine emits no verdict line and the adapter degrades
# loudly.
#
# The witness log is TAGGED with session and entrypoint: the selftest calls the
# engine hundreds of times, and an untagged log would "prove" a harness
# dispatches when it does not.
set -uo pipefail

GUARD_HOME="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Every per-product value comes from <product>/config/schwarzwald/guards.env (config.sh says how
# it is read). This part sits at <product>/shared/schwarzwald/guards/, so the product root is
# three levels up. A config that is missing or not data gives no verdict: the adapter degrades.
GUARD_CONFIG_LOADED=""
# shellcheck source=/dev/null
if . "$GUARD_HOME/config.sh" 2>/dev/null; then
    guard_config_locate "$GUARD_HOME"
    guard_config_load && GUARD_CONFIG_LOADED=1
else
    GUARD_PRODUCT_ROOT="${GUARD_HOME%/*/*/*}"
    GUARD_REPO="${GUARD_REPO:-$GUARD_PRODUCT_ROOT}"
    GUARD_CONFIG_PROBLEM="config.sh did not load from $GUARD_HOME"
fi
: "${GUARD_PRODUCT:=guard}"
# Containment's boundary is the ORG root (the checkout's parent): cross-repo
# work is routine, so a repo-scoped boundary would block normal work.
GUARD_ORG="${GUARD_REPO%/*}"
# Agent scratch roots containment treats like $TMPDIR (literal, absolute, space-separated). The
# session's own dir arrives as GUARD_SCRATCH_DIR from the adapter. Never $HOME itself.
: "${GUARD_SCRATCH_ROOTS:=$HOME/.jcode/scratch}"

# The fallback is the ONE deny list and heredoc splitter, shared so the healthy
# engine can never be laxer than the degraded path. Unloadable means no verdict.
GUARD_FALLBACK_LOADED=1
# shellcheck source=/dev/null
. "$GUARD_HOME/rules/fallback/fail-closed.sh" 2>/dev/null || GUARD_FALLBACK_LOADED=""

GUARD_VERDICT=allow

# Attribution is OFF by default: the selftest, the post-lint relay and every
# adapter read this stderr as prose, so an unconditional tag would change it.
GUARD_ACTIVE_RULE=""

# The engine sets GUARD_ACTIVE_RULE around each call, so a rule never names
# itself (and so can never name the wrong rule). Empty means the engine spoke.
guard_attribute() {
    [ -n "${GUARD_ATTRIBUTION:-}" ] || return 0
    printf '%s: %s\n' "$1" "${GUARD_ACTIVE_RULE:-engine}" >&2
}

guard_block() {
    GUARD_VERDICT=block
    guard_attribute guard-block
    printf 'Blocked: %s\n' "$1" >&2
    shift
    local line
    for line in "$@"; do printf '         %s\n' "$line" >&2; done
}

guard_advise() {
    guard_attribute guard-advise
    local line
    for line in "$@"; do printf '%s\n' "$line" >&2; done
    guard_spool "$@"
}

# A per-session spool file, because under jcode post_tool is a detached
# observer whose stderr nobody reads. Keyed on the session so lanes sharing the
# checkout never see each other's advisories. The drainer is a turn_end hook in
# the user's own config, which nothing in this repo may write.
guard_spool() {
    [ "${GUARD_SPOOL_ENABLED:-1}" = 1 ] || return 0
    local dir="${GUARD_SPOOL_DIR:-${TMPDIR:-/tmp}/${GUARD_SPOOL_NAME:-$GUARD_PRODUCT-guard-spool}}"
    mkdir -p "$dir" 2>/dev/null || return 0
    local f="$dir/${GUARD_SESSION_ID:-unknown}.log"
    local line
    for line in "$@"; do
        printf '%s %s %s\n' "$(date +%H:%M:%S)" \
            "${GUARD_EVENT:-pre_tool}/${GUARD_ACTIVE_RULE:-engine}" "$line"
    done >>"$f" 2>/dev/null

    local n
    n=$(wc -l <"$f" 2>/dev/null || echo 0)
    if [ "${n:-0}" -gt 2000 ]; then
        tail -n 1000 "$f" >"$f.tmp" 2>/dev/null && mv "$f.tmp" "$f" 2>/dev/null
    fi
    return 0
}

# Does the command INVOKE $1 in command position? An unanchored match would
# fire on prose that merely mentions the command.
guard_cmd_is() { guard_match "$GUARD_CMD" "$GUARD_ANCHOR($1)"; }

guard_cmd_has() { guard_match "$GUARD_CMD" "$1"; }

# Separators include newline. The prefix group takes wrappers that run their
# argument unchanged, plus VAR=value: a wrapper missing here is a way to say the
# same command that no rule sees.
GUARD_ANCHOR=$'(^|[;&|\n]|&&|\\|\\|)[[:space:]]*(((sudo|env|command|builtin|exec|nohup|time|nice)[[:space:]]+(-[^[:space:]]+[[:space:]]+)*|[A-Za-z_][A-Za-z0-9_]*=[^ ]*[[:space:]]+))*'

# In-process ERE, no fork per rule: a guard set that slows the session gets
# turned off. The pattern goes through a variable, unquoted.
guard_match() {
    local subject="$1" pattern="$2"
    [[ $subject =~ $pattern ]]
}

# The command with heredoc BODIES removed (prose is not an invocation). Not a
# cut at the first `<<`: that would drop `cat <<<x; git stash`. Bodies the shell
# executes (`bash <<EOF`, `$(...)` in an unquoted body) are kept.
GUARD_HEAD_CACHE=""
guard_cmd_head() {
    if [ -n "${GUARD_HEAD_CACHE:-}" ]; then
        printf '%s' "$GUARD_HEAD_CACHE"
        return 0
    fi
    _gf_strip_heredocs "${GUARD_CMD:-}"
    printf '%s' "${GUARD_FALLBACK_VALUE%$'\n'}"
}

# The command with quoted runs removed, for a FLAG or fragment subject that
# GUARD_ANCHOR cannot place. Naive about nesting on purpose: a confusing quote
# arrangement leaves MORE text, so the rule still fires.
guard_cmd_unquoted() {
    printf '%s' "$(guard_cmd_head)" | sed "s/'[^']*'//g; s/\"[^\"]*\"//g"
}

# jq's @sh output is built to be eval'd: the one place the engine treats
# untrusted text as shell.
guard_parse() {
    local payload="$1"
    if ! command -v jq >/dev/null 2>&1; then
        guard_advise \
            "WARNING: $GUARD_PRODUCT guards are ABSTAINING -- jq is not on PATH, so tool input cannot be parsed." \
            "         The engine gives no verdict; the fail-closed fallback judges instead. Install jq (brew install jq)."
        return 1
    fi
    local assigns
    assigns=$(printf '%s' "$payload" | jq -r '
        @sh "GUARD_EVENT=\(.event // "pre_tool")
             GUARD_TOOL=\(.tool // "" | ascii_downcase)
             GUARD_CWD=\(.cwd // "")
             GUARD_STATUS=\(.status // "ok")
             GUARD_CMD=\(.input.command // "")
             GUARD_SCRIPT=\(.input.script // "")
             GUARD_FILE=\(.input.file_path // "")
             GUARD_ACTION=\(.input.action // "")
             GUARD_READ_BOUND=\(if (.input | type) == "object" and (.input | has("offset") or has("limit") or has("start_line")) then "1" else "" end)
             GUARD_MODEL=\(.input.model // "")
             GUARD_PROMPT=\(.input.prompt // .input.initial_message // "")
             GUARD_BACKGROUND=\(.input.run_in_background // false | tostring)"
    ' 2>/dev/null) || return 1
    [ -n "$assigns" ] || return 1
    eval "$assigns" || return 1
    return 0
}

# `guard_register <id> [event]`. Event scoping is enforced here, so a post_tool
# rule with a wrong guard clause still cannot fire on pre_tool.
GUARD_RULES=()
guard_register() { GUARD_RULES+=("${1}:${2:-pre_tool}"); }

# A FLOOR, not an exact count: a stale exact number would block every call. The
# selftest owns exactness.
GUARD_MIN_RULES=10

# Must EQUAL the shared file count in rules/: the selftest's degradation case deletes
# exactly one file, so a lower floor cannot see it go. A product's own rule files are
# counted apart, against GUARD_PRODUCT_RULE_FILES, so a missing one of THOSE is seen too.
GUARD_SHARED_RULE_FILES=15

# Shared rules first, then each product rule dir (space-separated, relative to the product root:
# the install's own product, never GUARD_REPO, which a caller may point at another checkout).
guard_load_rules() {
    local f n=0 m=0 dir
    for f in "$GUARD_HOME"/rules/*.sh; do
        [ -e "$f" ] || continue
        # shellcheck source=/dev/null
        . "$f" || return 1
        n=$((n + 1))
    done
    for dir in ${GUARD_PRODUCT_RULE_DIRS:-}; do
        case "$dir" in /* | *..*) continue ;; esac
        for f in "${GUARD_PRODUCT_ROOT:-$GUARD_REPO}/$dir"/*.sh; do
            [ -e "$f" ] || continue
            # shellcheck source=/dev/null
            . "$f" || return 1
            m=$((m + 1))
        done
    done
    [ "$n" -ge "$GUARD_SHARED_RULE_FILES" ] && [ "$m" -ge "${GUARD_PRODUCT_RULE_FILES:-0}" ] || {
        guard_advise \
            "WARNING: $GUARD_PRODUCT guards are INCOMPLETE -- $n shared and $m product rule file(s) loaded, expected $GUARD_SHARED_RULE_FILES and ${GUARD_PRODUCT_RULE_FILES:-0}." \
            "         A guard category is missing from this install. Re-sync the shared part, or restore ${GUARD_PRODUCT_RULE_DIRS:-the product rules}."
        return 1
    }
    [ "${#GUARD_RULES[@]}" -ge "$GUARD_MIN_RULES" ] || return 1
    return 0
}

guard_main() {
    local payload
    payload=$(cat)

    # An unparseable request is NOT an allow: no verdict line hands it to the
    # fail-closed fallback. A payload that parses but names no tool allows.
    if [ -z "$GUARD_CONFIG_LOADED" ]; then
        guard_advise "WARNING: $GUARD_PRODUCT guards have no usable config: $GUARD_CONFIG_PROBLEM"
        return 0
    fi
    if [ -n "${GUARD_CONFIG_UNKNOWN:-}" ]; then
        guard_advise "WARNING: $GUARD_PRODUCT guard config ${GUARD_CONFIG:-} names a key the guards do not know:$GUARD_CONFIG_UNKNOWN" \
            "         The key is ignored and every guard still judges. Fix the config: a misspelt key leaves its default in force."
    fi
    if [ -z "$GUARD_FALLBACK_LOADED" ]; then
        guard_advise "WARNING: $GUARD_PRODUCT guards cannot load rules/fallback/fail-closed.sh, the shared deny list."
        return 0
    fi
    if ! guard_parse "$payload"; then
        guard_advise "WARNING: $GUARD_PRODUCT guards could not parse the request (not JSON, truncated, or no jq); it is NOT being allowed by the engine."
        return 0
    fi
    GUARD_HEAD_CACHE=""
    _gf_strip_heredocs "${GUARD_CMD:-}"
    GUARD_HEAD_CACHE="${GUARD_FALLBACK_VALUE%$'\n'}"

    [ -n "${GUARD_TOOL:-}" ] || {
        printf 'verdict: allow\n'
        return 0
    }

    # Outside the org tree, only the destructive class: from /tmp,
    # `git -C <repo> stash` destroys the same work.
    case "${GUARD_CWD:-}" in
        "$GUARD_ORG" | "$GUARD_ORG"/*) ;;
        *)
            if [ "$GUARD_TOOL" = bash ] && [ -z "${GUARD_SELFTEST_NO_DESTRUCTIVE:-}" ] &&
                ! GUARD_FALLBACK_HEALTHY=1 GUARD_FALLBACK_ORG="$GUARD_ORG" _gf_engine_judge; then
                GUARD_ACTIVE_RULE=destructive_class
                guard_block "$GUARD_FALLBACK_REASON" \
                    "The cwd (${GUARD_CWD:-unknown}) is outside the org, but the command reaches into it."
                GUARD_ACTIVE_RULE=""
            fi
            printf 'verdict: %s\n' "$GUARD_VERDICT"
            return 0
            ;;
    esac

    guard_witness
    guard_resolve_lane_role

    if ! guard_load_rules; then
        # No verdict line: "allow" here would claim the guards ran.
        guard_advise "WARNING: $GUARD_PRODUCT guards failed to load rules from $GUARD_HOME/rules."
        return 0
    fi

    local entry rule event
    for entry in "${GUARD_RULES[@]}"; do
        rule="${entry%%:*}"
        event="${entry##*:}"
        [ "$event" = "${GUARD_EVENT:-pre_tool}" ] || continue
        GUARD_ACTIVE_RULE="$rule"
        "guard_rule_$rule" || true
        GUARD_ACTIVE_RULE=""
        # First block wins: one reason, and no later rule can overturn it. Under attribution
        # every rule runs, so the selftest can ask whether THIS rule blocks, not just the first.
        [ "$GUARD_VERDICT" = block ] && [ -z "${GUARD_ATTRIBUTION:-}" ] && break
    done

    printf 'verdict: %s\n' "$GUARD_VERDICT"
    return 0
}

# Every lane shares one host process, so a role cannot be exported per lane.
# The session id is a harness fact rules may not read, so the translation to a
# role lives here. Absent from the file adds nothing. Rules read GUARD_LANE only: a product
# whose lane variable has another name sets GUARD_LANE_ENV, and that variable is read into
# GUARD_LANE here.
guard_resolve_lane_role() {
    if [ -z "${GUARD_LANE:-}" ] && [[ ${GUARD_LANE_ENV:-} =~ ^[A-Z][A-Z0-9_]*$ ]]; then
        GUARD_LANE="${!GUARD_LANE_ENV:-}"
    fi
    if [ -z "${GUARD_LANE:-}" ]; then
        local roles="${GUARD_LANE_ROLES:-.runs/lane-roles}" role
        case "$roles" in /*) ;; *) roles="$GUARD_REPO/$roles" ;; esac
        [ -f "$roles" ] || return 0
        [ -n "${GUARD_SESSION_ID:-}" ] || return 0
        role=$(awk -v id="$GUARD_SESSION_ID" '$1 == id { print $2; exit }' "$roles" 2>/dev/null)
        [ -n "$role" ] || return 0
        GUARD_LANE="$role"
    fi
    export GUARD_LANE
}

# The checkout the AGENT works in: the git toplevel of the cwd, not GUARD_REPO. GUARD_REPO
# is where the guards are installed, which is a different clone when one install serves a
# whole org. A rule that reads repo STATE (the index, the diff, target/) asks here. Empty
# when the cwd is in no repository. The caller's GIT_* would point git elsewhere, so they go.
guard_cwd_repo() {
    [ -n "${GUARD_CWD:-}" ] || return 0
    env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE \
        git -C "$GUARD_CWD" rev-parse --show-toplevel 2>/dev/null || :
}

guard_witness() {
    : "${GUARD_WITNESS:=/tmp/jc-witness.log}"
    printf '%s %s %s %s %s\n' "$(date +%H:%M:%S)" \
        "${GUARD_SESSION_ID:-synthetic}" "${GUARD_TOOL:-?}" \
        "${GUARD_ENTRYPOINT:-direct}" "${GUARD_EVENT:-pre_tool}" \
        >>"$GUARD_WITNESS" 2>/dev/null

    if [ -f "$GUARD_WITNESS" ]; then
        local n
        n=$(wc -l <"$GUARD_WITNESS" 2>/dev/null || echo 0)
        if [ "${n:-0}" -gt 5000 ]; then
            tail -n 2500 "$GUARD_WITNESS" >"$GUARD_WITNESS.tmp" 2>/dev/null &&
                mv "$GUARD_WITNESS.tmp" "$GUARD_WITNESS" 2>/dev/null
        fi
    fi
}

if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
    guard_main
fi
