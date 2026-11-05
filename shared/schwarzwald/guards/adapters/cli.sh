#!/usr/bin/env bash
#
# ADAPTER: a generic command-line harness, written against the contract alone.
# CONV-GUARD-ADAPTERS asserts it reaches the agent adapters' verdicts. Anything
# can ask the guards a question:
#
#   scripts/guards/adapters/cli.sh --tool bash --command 'git add .'
#   scripts/guards/adapters/cli.sh --tool write --file docs/generated/database-schema.md   # lint-path-ok: illustrative
#   scripts/guards/adapters/cli.sh --event post_tool --tool edit --file Makefile
#
# Exit: 0 allowed, 1 blocked (reason on stderr), 2 usage error. NOT the
# agents' 2-means-block: 2 for usage is the shell convention.
set -uo pipefail

_self="${BASH_SOURCE[0]}"
case "$_self" in
    /*) ;;
    *) _self="$PWD/$_self" ;;
esac
_dir="${_self%/*}"
# Canonical via the `cd` builtin: a `..` component would break every path
# derived by stripping components.
_back="$PWD"
CDPATH='' cd -- "$_dir" 2>/dev/null && _dir="$PWD"
CDPATH='' cd -- "$_back" 2>/dev/null || :

# The fallback loads FIRST, builtins only; it is armed once the args parse.
# shellcheck source=/dev/null
. "${_dir%/*}/rules/fallback/fail-closed.sh" || {
    echo "cli: cannot load the fail-closed fallback from ${_dir%/*}/rules/fallback/" >&2
    exit 2
}

# shellcheck source=/dev/null
. "$_dir/adapter-plumbing.sh" || {
    echo "cli: cannot load adapter plumbing from $_dir/adapter-plumbing.sh" >&2
    exit 2
}
guard_adapter_init "${BASH_SOURCE[0]}"

usage() {
    sed -n '3,/^set -/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
}

EVENT=pre_tool
TOOL=""
CWD="$PWD"
COMMAND=""
FILE=""
STATUS=ok
QUIET=""

# Not ${2:?...}: that exits 1, this adapter's "blocked" code.
need_value() {
    [ $# -ge 2 ] && [ -n "${2:-}" ] || {
        echo "cli: $1 needs a value" >&2
        exit 2
    }
}

while [ $# -gt 0 ]; do
    case "$1" in
        --event)
            need_value "$1" "${2:-}"
            EVENT="$2"
            shift 2
            ;;
        --tool)
            need_value "$1" "${2:-}"
            TOOL="$2"
            shift 2
            ;;
        --cwd)
            need_value "$1" "${2:-}"
            CWD="$2"
            shift 2
            ;;
        --command)
            need_value "$1" "${2:-}"
            COMMAND="$2"
            shift 2
            ;;
        --file)
            need_value "$1" "${2:-}"
            FILE="$2"
            shift 2
            ;;
        --status)
            need_value "$1" "${2:-}"
            STATUS="$2"
            shift 2
            ;;
        --quiet)
            QUIET=1
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "cli: unknown argument: $1" >&2
            echo "try --help" >&2
            exit 2
            ;;
    esac
done

if [ -z "$TOOL" ]; then
    if [ -n "$COMMAND" ]; then
        TOOL=bash
    elif [ -n "$FILE" ]; then
        TOOL="write"
    else
        echo "cli: nothing to check -- pass --command or --file" >&2
        exit 2
    fi
fi

# A missing --cwd lands outside the org, where the engine allows: a typo would
# silently turn the guards off.
if [ ! -d "$CWD" ]; then
    echo "cli: --cwd does not exist: $CWD" >&2
    exit 2
fi

# From here an unsettled exit is judged by the fallback. The cwd is resolved
# with the builtin so the fallback sees it even when the next fork fails.
[ -n "$QUIET" ] && exec 2>/dev/null
_back="$PWD"
cd "$CWD" 2>/dev/null && CWD="$PWD"
cd "$_back" || exit 2
# shellcheck disable=SC2034 # read by the sourced fallback
{
    GUARD_TOOL="$TOOL"
    GUARD_EVENT="$EVENT"
    GUARD_CWD="$CWD"
    GUARD_CMD="$COMMAND"
}
guard_fallback_arm 1

if [ -n "$FILE" ]; then
    case "$FILE" in
        /*) ;;
        *) FILE="$CWD/$FILE" ;;
    esac
fi

command -v jq >/dev/null 2>&1 ||
    guard_fallback_degraded "jq is not on PATH, so no request can be built."
INPUT=$(jq -cn --arg c "$COMMAND" --arg f "$FILE" \
    '{} + (if $c != "" then {command:$c} else {} end)
        + (if $f != "" then {file_path:$f} else {} end)')

REQUEST=$(guard_adapter_request "$EVENT" "$TOOL" "$CWD" "$INPUT" "$STATUS")

case "$(guard_adapter_run "$REQUEST" cli "${GUARD_SESSION_ID:-}" "$GUARD_ADAPTER_DEADLINE")" in
    block) guard_fallback_settle 1 ;;
    allow) guard_fallback_settle 0 ;;
    *) guard_fallback_degraded "the engine produced NO VERDICT (broken, killed, or past the ${GUARD_ADAPTER_DEADLINE}s deadline)." ;;
esac
