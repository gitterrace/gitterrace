#!/usr/bin/env bash
#
# Process-kill hygiene: a pattern kill that matches the HUMAN's running app, not
# just the instance this session launched.

guard_register broad_process_kill
guard_rule_broad_process_kill() {
    [ "$GUARD_TOOL" = bash ] || return 0

    # `pkill -f ".../<app>"` kills the owner's running app too; the only signal
    # is the human's window vanishing. Advisory: no work on disk is lost.
    guard_match "$(guard_cmd_head)" "$GUARD_ANCHOR"'(pkill|killall)( |$)' || return 0

    # Not caught: an ANCHORED pattern on a command line the script built
    # (`pkill -f "^$BIN serve --transport http"`).
    guard_cmd_has 'pkill +(-[A-Za-z]+ +)*-f +"?\^' && return 0

    # Not caught: `pkill -F <pidfile>`, a kill by explicit pid.
    guard_cmd_has 'pkill +(-[A-Za-z]+ +)*-F( |$)' && return 0

    guard_advise \
        "This kills EVERY match, including an app the human is using." \
        "" \
        "  A broad \`pkill -f\` on an app binary killed the owner's running session" \
        "  The agent already had its own pid from launching the" \
        "  instance, so the wide pattern gained nothing." \
        "" \
        "  Prefer, in order:" \
        "    kill \"\$pid\"           the pid you got back when YOU launched it" \
        "    pkill -f '^<full cmdline>'   anchored, so it cannot match another instance" \
        "" \
        "  Check what you are about to kill first: pgrep -af '<pattern>'"
}
