#!/usr/bin/env bash
#
# What is written into a script is a command run later: a lane wrote a broad `pkill -f` into
# .runs/hh-cycle.sh, ran it, and killed the owner's app. Content written to a script file
# (guard_is_script_path) is judged by the SAME bash rules as a typed command, called here by
# name so each pattern keeps its one definition. Prose files are not judged.

guard_register script_content
guard_rule_script_content() {
    case "$GUARD_TOOL" in write | edit | multiedit | apply_patch) ;; *) return 0 ;; esac
    [ -n "${GUARD_CONTENT:-}" ] || return 0
    guard_is_script_path "${GUARD_FILE:-}" || return 0
    local tool="$GUARD_TOOL" cmd="${GUARD_CMD:-}" head="${GUARD_HEAD_CACHE:-}" body="" line r
    # Comment lines are prose about the code, not code.
    while IFS= read -r line || [ -n "$line" ]; do
        case "${line#"${line%%[![:space:]]*}"}" in '#'* | '') continue ;; esac
        body="$body$line"$'\n'
    done <<<"$GUARD_CONTENT"
    [ -n "$body" ] || return 0
    GUARD_TOOL=bash
    GUARD_CMD="$body"
    _gf_strip_heredocs "$GUARD_CMD"
    GUARD_HEAD_CACHE="${GUARD_FALLBACK_VALUE%$'\n'}"
    for r in broad_process_kill git_stash git_checkout_discards containment; do
        "guard_rule_$r" || true
        [ "$GUARD_VERDICT" = block ] && break
    done
    # The destructive class through its one definition, the fallback's deny list, not the
    # destructive_class rule: that rule stands down when the selftest isolates a dedicated rule.
    if [ "$GUARD_VERDICT" != block ] &&
        ! GUARD_EVENT=pre_tool GUARD_FALLBACK_HEALTHY=1 GUARD_FALLBACK_ORG="$GUARD_ORG" _gf_engine_judge; then
        guard_block "$GUARD_FALLBACK_REASON" \
            "This is the destructive class, written into a script: running it would do the same."
    fi
    GUARD_TOOL="$tool"
    GUARD_CMD="$cmd"
    GUARD_HEAD_CACHE="$head"
    [ "$GUARD_VERDICT" = block ] &&
        printf '         (in the script %s: running it would do this)\n' "${GUARD_FILE##*/}" >&2
    return 0
}
