#!/usr/bin/env bash
# The destructive class, judged by the ONE deny list in fail-closed.sh. The
# engine's own list was narrower than its fallback's (it allowed
# `git reset --hard`, `command git stash`). GUARD_FALLBACK_HEALTHY=1 turns off
# what only a degraded guard refuses: `rm -r`, in-org `rm`, `$var` commands.

guard_register destructive_class
guard_rule_destructive_class() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # Lets the selftest ask whether a DEDICATED rule still refuses on its own.
    [ -z "${GUARD_SELFTEST_NO_DESTRUCTIVE:-}" ] || return 0
    local reason
    GUARD_FALLBACK_HEALTHY=1 GUARD_FALLBACK_ORG="$GUARD_ORG" _gf_engine_judge && return 0
    reason="$GUARD_FALLBACK_REASON"
    guard_block \
        "$reason" \
        "This is the destructive class in a SHARED checkout: scripts/guards/rules/fallback/fail-closed.sh is its single definition." \
        "To baseline ONE file: git show HEAD:<path> > /tmp/<session>-baseline, measure, copy back."
    return 0
}
