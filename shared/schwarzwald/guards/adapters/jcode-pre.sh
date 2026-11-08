#!/usr/bin/env bash
#
# ADAPTER: jcode pre_tool -> the shared guard engine. No guard logic lives in
# an adapter: a hand-kept parity corpus reported 37/37 agreement while a sweep
# found 39 disagreements. jcode: exit 0 allows, exit 2 blocks.
#
# The outside-the-org path is a string comparison and an exit: jcode runs this in
# every session on the machine. The boundary is the ORG root, matching the
# engine. A relative BASH_SOURCE is resolved, not abstained on: abstaining once
# disabled every guard under `bash <relative path to this adapter>`.
_self="${BASH_SOURCE[0]}"
case "$_self" in
    /*) ;;
    *) _self="$PWD/$_self" ;;
esac
_dir="${_self%/*}"
# Canonical via the `cd` builtin: a `..` would shift the org stripped below.
_back="$PWD"
CDPATH='' cd -- "$_dir" 2>/dev/null && _dir="$PWD"
CDPATH='' cd -- "$_back" 2>/dev/null || :
# adapters -> guards -> schwarzwald -> shared -> product -> org
_org="${_dir%/*/*/*/*/*}"

# Stdin, not JCODE_HOOK_TOOL_INPUT: the env copy is truncated at 16 KB, which
# hid a verb and parsed as "allow". A stdin nobody closes gives up after 3 s,
# inside jcode's 5 s timeout.
_input=""
if [ ! -t 0 ]; then
    while IFS= read -r -d '' -n 16384 -t 3 _chunk; do
        _input="$_input$_chunk"
    done
    _input="$_input${_chunk:-}"
fi
[ -n "$_input" ] || _input="${JCODE_HOOK_TOOL_INPUT:-}"

# Outside the org, the deny list still judges a command that could reach back
# in (names the org, `~`, `HOME`, `..`, or changes directory: a symlink outside
# the org can point into it). An EMPTY cwd takes the full path. A cwd reached
# through a symlink is compared where it points, via the `cd -P` builtin.
_cwd="${JCODE_HOOK_CWD:-}"
_pcwd="$_cwd"
if [ -n "$_cwd" ] && CDPATH='' cd -P -- "$_cwd" 2>/dev/null; then
    _pcwd="$PWD"
    CDPATH='' cd -- "$_back" 2>/dev/null || :
fi
_porg="$_org"
CDPATH='' cd -P -- "$_org" 2>/dev/null && _porg="$PWD"
CDPATH='' cd -- "$_back" 2>/dev/null || :
case "$_pcwd" in
    "" | "$_org" | "$_org"/* | "$_porg" | "$_porg"/*) ;;
    *)
        case "$_input" in
            *"$_org"* | *'~'* | *HOME* | *..* | *cd* | *pushd* | *-C* | *--git-dir* | *--work-tree*) ;;
            *) exit 0 ;;
        esac
        # shellcheck source=/dev/null
        . "${_dir%/*}/rules/fallback/fail-closed.sh" || exit 0
        # shellcheck disable=SC2034 # read by the sourced fallback
        {
            GUARD_TOOL="${JCODE_HOOK_TOOL_NAME:-}"
            GUARD_CWD="$_cwd"
            guard_fallback_json_string "$_input" command
            GUARD_CMD="$GUARD_FALLBACK_VALUE"
        }
        guard_fallback_judge && exit 0
        printf 'Blocked: %s\n' "$GUARD_FALLBACK_REASON" >&2
        echo "         The cwd ($_cwd) is outside the org, but the command reaches into it." >&2
        exit 2
        ;;
esac

# From the arm on, an unsettled exit (a failed fork exits 128, which jcode
# reads as "allow") is judged by the fallback.
# shellcheck source=/dev/null
. "${_dir%/*}/rules/fallback/fail-closed.sh" || exit 0
# shellcheck disable=SC2034 # read by the sourced fallback
{
    GUARD_TOOL="${JCODE_HOOK_TOOL_NAME:-}"
    GUARD_CWD="$_cwd"
    guard_fallback_json_string "$_input" command
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
    guard_fallback_degraded "jq is not on PATH, so the tool input cannot be parsed."

# The session's own scratch dir, translated for containment (rules read GUARD_* only).
export GUARD_SCRATCH_DIR="${JCODE_SCRATCH_DIR:-}"

REQUEST=$(guard_adapter_request pre_tool "${JCODE_HOOK_TOOL_NAME:-}" \
    "$_cwd" "$_input" ok)

case "$(guard_adapter_run "$REQUEST" jcode-pre "${JCODE_HOOK_SESSION_ID:-}" "$GUARD_ADAPTER_DEADLINE")" in
    block) guard_fallback_settle 2 ;;
    allow) guard_fallback_settle 0 ;;
    *) guard_fallback_degraded "the engine produced NO VERDICT (broken, killed, or past the ${GUARD_ADAPTER_DEADLINE}s deadline)." ;;
esac
