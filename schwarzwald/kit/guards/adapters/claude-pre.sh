#!/usr/bin/env bash
#
# ADAPTER: Claude Code PreToolUse -> the shared guard engine. No guard logic lives here.
# Claude Code sends {tool_name, tool_input, cwd} on stdin: exit 0 allows, exit 2 blocks and
# shows stderr. The settings file names this script, never inline shell.
_self="${BASH_SOURCE[0]}"
case "$_self" in
    /*) ;;
    *) _self="$PWD/$_self" ;;
esac
_dir="${_self%/*}"
_back="$PWD"
CDPATH='' cd -- "$_dir" 2>/dev/null && _dir="$PWD"
CDPATH='' cd -- "$_back" 2>/dev/null || :

# shellcheck source=/dev/null
. "${_dir%/*}/rules/fallback/fail-closed.sh" || exit 0
PAYLOAD=""
while IFS= read -r -d '' -n 16384 _chunk; do
    PAYLOAD="$PAYLOAD$_chunk"
done
PAYLOAD="$PAYLOAD$_chunk"
# shellcheck disable=SC2034 # read by the sourced fallback
{
    guard_fallback_json_string "$PAYLOAD" tool_name
    GUARD_TOOL="$GUARD_FALLBACK_VALUE"
    guard_fallback_json_string "$PAYLOAD" cwd
    GUARD_CWD="$GUARD_FALLBACK_VALUE"
    guard_fallback_json_string "$PAYLOAD" command
    GUARD_CMD="$GUARD_FALLBACK_VALUE"
}
guard_fallback_arm 2

# shellcheck source=/dev/null
. "$_dir/adapter-plumbing.sh" ||
    guard_fallback_degraded "the adapter plumbing did not load from $_dir/adapter-plumbing.sh."
guard_adapter_init "${BASH_SOURCE[0]}"

[ -f "$GUARD_ENGINE" ] ||
    guard_fallback_degraded "the guard engine is missing at $GUARD_ENGINE."
command -v jq >/dev/null 2>&1 ||
    guard_fallback_degraded "jq is not on PATH, so the payload cannot be parsed."

REQUEST=$(printf '%s' "$PAYLOAD" | jq -c '{
    event: "pre_tool",
    tool: (.tool_name // ""),
    cwd: (.cwd // ""),
    status: "ok",
    input: (.tool_input // {})
}' 2>/dev/null)
[ -n "$REQUEST" ] ||
    guard_fallback_degraded "the payload did not parse as JSON."

case "$(guard_adapter_run "$REQUEST" claude-pre "${CLAUDE_SESSION_ID:-}" "$GUARD_ADAPTER_DEADLINE")" in
    block) guard_fallback_settle 2 ;;
    allow) guard_fallback_settle 0 ;;
    *) guard_fallback_degraded "the engine produced NO VERDICT (broken, killed, or past the ${GUARD_ADAPTER_DEADLINE}s deadline)." ;;
esac
