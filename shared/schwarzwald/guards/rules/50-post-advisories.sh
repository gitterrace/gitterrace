#!/usr/bin/env bash
#
# POST-TOOL advisories: advise only (the write already happened).
_post_applies() {
    [ "${GUARD_EVENT:-}" = post_tool ] || return 1
    case "$GUARD_TOOL" in write | edit | multiedit) ;; *) return 1 ;; esac
    [ -n "$GUARD_FILE" ] || return 1
    case "${GUARD_STATUS:-ok}" in ok | success | "") ;; *) return 1 ;; esac
    return 0
}

# A file that feeds a GENERATED file or a separate generation gate: clippy and the tests never
# see it go stale. The globs (GUARD_GENERATED_FEEDS) and the regenerate command
# (GUARD_GENERATED_HINT) are the product's. Not caught: a feed the product did not list.
guard_register post_check_generated post_tool
guard_rule_post_check_generated() {
    _post_applies || return 0
    _edit_glob_hit "$GUARD_FILE" "${GUARD_GENERATED_FEEDS:-}" || return 0
    guard_advise \
        "Reminder: this file feeds a generated file, checked by a separate gate from clippy and tests." \
        "${GUARD_GENERATED_HINT:-Regenerate, then run the generation check.}" \
        "Read its exit code directly, never through '| tail'."
    return 0
}
