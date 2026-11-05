#!/usr/bin/env bash
# Discard hygiene: `git checkout -- <path>`, `restore <path>`,
# `checkout-index`, `read-tree -u` discard work that may not be the caller's.
# Blocks. Not caught: branch work (`checkout <branch>`, `-b`, `--track`) or a
# bare `--staged` restore. A discard is an argument NAMING AN EXISTING FILE,
# or an explicit `--`. The verb list is a denylist, not exhaustive.

guard_register git_checkout_discards
guard_rule_git_checkout_discards() {
    [ "$GUARD_TOOL" = bash ] || return 0
    guard_match "$(guard_cmd_head)" \
        "$GUARD_ANCHOR"'git( +-C +[^ ]+)? +(checkout|restore|checkout-index|read-tree)( |$)' || return 0
    # read-tree without -u or -m is an index-only load.
    if guard_cmd_has 'git( -C [^ ]+)? read-tree'; then
        guard_cmd_has 'git( -C [^ ]+)? read-tree [^|;&]*(-[A-Za-z]*[um]|--reset)' || return 0
        guard_block \
            "git read-tree -u/-m overwrites working-tree files -- same destruction as 'git checkout --', and this checkout is SHARED." \
            "To baseline ONE file: git show HEAD:<path> > /tmp/<session>-baseline, measure, copy back."
        return 0
    fi
    if guard_cmd_has 'git( -C [^ ]+)? checkout-index'; then
        guard_block \
            "git checkout-index overwrites working-tree files from the INDEX -- same destruction as 'git checkout --', and this checkout is SHARED." \
            "To baseline ONE file: git show HEAD:<path> > /tmp/<session>-baseline, measure, copy back."
        return 0
    fi
    guard_cmd_has 'git( -C [^ ]+)? checkout +-[bB]' && return 0
    guard_cmd_has 'git( -C [^ ]+)? checkout +--track' && return 0
    if guard_cmd_has 'git( -C [^ ]+)? restore +--staged'; then
        guard_cmd_has 'git( -C [^ ]+)? restore [^|;&]*(--worktree|[[:space:]]-[A-Za-z]*W)' || return 0
    fi

    _guard_discard_is_path=0
    guard_cmd_has 'git( -C [^ ]+)? (checkout|restore)( +-[A-Za-z-]+)* +--( |$)' && _guard_discard_is_path=1
    guard_cmd_has 'git( -C [^ ]+)? restore' && _guard_discard_is_path=1
    if [ "$_guard_discard_is_path" = 0 ]; then
        for _guard_arg in $(guard_cmd_head); do
            case "$_guard_arg" in
                git | checkout | restore | -*) continue ;;
            esac
            # Relative to the AGENT's cwd: the engine's own cwd is wherever the
            # harness started it, so a bare `-e` asked about a different tree.
            case "$_guard_arg" in
                /*) _guard_p="$_guard_arg" ;;
                *) _guard_p="${GUARD_CWD:-$PWD}/$_guard_arg" ;;
            esac
            [ -e "$_guard_p" ] && _guard_discard_is_path=1 && break
        done
    fi
    [ "$_guard_discard_is_path" = 1 ] || return 0
    guard_block \
        "git checkout/restore of a PATH discards uncommitted work there, and this checkout is SHARED -- the work may not be yours." \
        "To baseline ONE file: git show HEAD:<path> > /tmp/<session>-baseline, measure, copy back." \
        "Branch work (checkout <branch>, -b, --track) and 'restore --staged' are unaffected."
    return 0
}
