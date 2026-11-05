#!/usr/bin/env bash
#
# Machine capacity. A lane that cannot fork SPINS: every call fails at once
# and it retries, feeding the saturation. Measures the machine, since the
# engine never sees a spawn.

guard_register process_table_pressure
guard_rule_process_table_pressure() {
    [ "$GUARD_TOOL" = bash ] || return 0

    # Live machine state: a busy box would flip ALLOW cases suite-wide.
    [ -n "${GUARD_SELFTEST_NO_PROCFS:-}" ] && return 0

    # With the table already full `ps` fails and the rule abstains: it catches
    # the approach to the cliff, not the fall.
    local n
    n=$(ps -u "$(id -u)" 2>/dev/null | wc -l | tr -d ' ') || return 0
    [ -n "$n" ] || return 0
    case "$n" in
        '' | *[!0-9]*) return 0 ;;
    esac

    # A fraction of `ulimit -u`, what fork() enforces; a typed number fires
    # constantly elsewhere.
    local limit threshold
    limit=$(ulimit -u 2>/dev/null)
    case "$limit" in
        '' | *[!0-9]*) limit=0 ;;
    esac
    if [ "$limit" -gt 0 ]; then
        threshold=$((limit * 7 / 10))
    else
        threshold=7000
    fi
    [ "$n" -lt "$threshold" ] && return 0

    # Zombies hold table slots: on the run that motivated this, most of the
    # table was <defunct>.
    local zombies
    # pgrep cannot match on status, hence ps|grep.
    # shellcheck disable=SC2009
    zombies=$(ps -u "$(id -u)" -o stat= 2>/dev/null | grep -c '^Z' || true)

    guard_advise \
        "WARNING: $n processes for this user against a limit of $limit ($zombies defunct)." \
        "         At the limit EVERY fork fails (os error 35), including these hooks," \
        "         which fail OPEN -- so the guards vanish exactly when they matter." \
        "         A lane that cannot fork does not stop, it retries in a loop and makes" \
        "         it worse. Stop lanes from the TUI now; 'swarm stop' only deregisters" \
        "         them, and every lane shares the jcode host pid so pkill matches nothing." \
        "         Check with: ps -u \"\$(id -u)\" | wc -l"
    return 0
}
