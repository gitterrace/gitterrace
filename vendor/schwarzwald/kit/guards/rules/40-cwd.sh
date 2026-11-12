#!/usr/bin/env bash
#
# Cwd hazard: a harness may reset cwd to the org root (no Makefile), jcode keeps
# an earlier `cd` (wrong repo). Checks the marker file the command needs.

guard_register cwd_has_marker
guard_rule_cwd_has_marker() {
    [ "$GUARD_TOOL" = bash ] || return 0
    [ -n "$GUARD_CWD" ] || return 0

    guard_cmd_has '(^|[;&|] *)cd |git +-C ' && return 0

    local need=''
    guard_cmd_is 'make[[:space:]]' && need=Makefile
    guard_cmd_is 'cargo[[:space:]]' && need=Cargo.toml
    guard_cmd_is 'swift[[:space:]]' && need=Package.swift
    guard_cmd_is 'git[[:space:]]+commit' && need=.git
    [ -n "$need" ] || return 0

    # make reads the cwd's Makefile only; cargo/swift/git walk up.
    if [ "$need" = Makefile ]; then
        [ -f "$GUARD_CWD/Makefile" ] && return 0
    else
        local d="$GUARD_CWD"
        while [ -n "$d" ] && [ "$d" != / ]; do
            [ -e "$d/$need" ] && return 0
            d=${d%/*}
        done
    fi

    guard_block \
        "cwd is '$GUARD_CWD', which has no $need." \
        "Depending on the harness this is either a cwd that RESET to the org root, or a 'cd' from an" \
        "earlier call that PERSISTED -- both produce an error that reads like a build failure." \
        "Prefix an explicit 'cd <repo> &&', or use 'git -C <repo>'."
    return 0
}
