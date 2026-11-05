#!/usr/bin/env bash
#
# ADAPTER: jcode post_tool -> the shared guard engine. Advisory only: every
# path exits 0, since a post hook's failure makes a successful edit look failed.
_self="${BASH_SOURCE[0]}"
case "$_self" in
    /*) ;;
    *) _self="$PWD/$_self" ;;
esac
_dir="${_self%/*}"
# adapters -> guards -> schwarzwald -> shared -> product -> org; must match the engine's org.
_org="${_dir%/*/*/*/*/*}"

# Outside the org, nothing: this runs in every jcode session on the machine.
case "${JCODE_HOOK_CWD:-$PWD}" in
    "$_org" | "$_org"/*) ;;
    *) exit 0 ;;
esac

# shellcheck source=/dev/null
. "$_dir/adapter-plumbing.sh" || exit 0
guard_adapter_init "${BASH_SOURCE[0]}"
[ -f "$GUARD_ENGINE" ] || exit 0

REQUEST=$(guard_adapter_request post_tool "${JCODE_HOOK_TOOL_NAME:-}" \
    "${JCODE_HOOK_CWD:-$PWD}" "${JCODE_HOOK_TOOL_INPUT:-}" "${JCODE_HOOK_STATUS:-ok}")

guard_adapter_run "$REQUEST" jcode-post "${JCODE_HOOK_SESSION_ID:-}" >/dev/null

exit 0
