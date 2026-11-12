#!/usr/bin/env bash
# A write that fails silently, in one of two shapes, judged once so one call gets one advisory:
#   pipe: a mutating command piped into head/tail/grep with no pipefail. A rejected write
#         piped into `| tail -1` reports success. Not caught: reads, a pipe into `tee`.
#   subst: a FAILED `$(cat f)` is a valid EMPTY string: `--notes "$(cat f)"` after f vanished
#         wrote an empty field at rc=0. Judged for commit, tag and product CLI writes. Not
#         caught: reads, substitutions that read no file (`$(date)`).
# `set -o pipefail` or GUARD_ALLOW_SILENT=1 opts out of both.
guard_register no_silent_failure
guard_rule_no_silent_failure() {
    [ "$GUARD_TOOL" = bash ] || return 0
    [ -n "$GUARD_CMD" ] || return 0
    guard_cmd_has 'set -o pipefail|set -eo? *pipefail|GUARD_ALLOW_SILENT=1' && return 0

    local mutates='' pipe='' subst='' msg=()
    [ -n "${GUARD_WRITE_CMDS_RE:-}" ] && guard_cmd_is "$GUARD_WRITE_CMDS_RE" && mutates='product CLI write'
    guard_cmd_is 'git[[:space:]]+(commit|tag)' && mutates='git write'
    # A substitution that reads a FILE: `$(cat f)`, `$(< f)`, `$(head f)`. Only the narrow verbs.
    [ -n "$mutates" ] && guard_cmd_has '\$\([[:space:]]*(cat|head|tail)[[:space:]]|\$\([[:space:]]*<' && subst=1
    guard_cmd_is 'git[[:space:]]+(add|push|merge|rebase|reset)' && mutates='git write'
    guard_cmd_is 'cargo[[:space:]]+(install|publish|fix)' && mutates='cargo write'
    guard_cmd_is '(npm|pnpm|yarn)[[:space:]]+(install|publish)' && mutates='package install'
    [ -n "$mutates" ] && guard_cmd_has '\|[[:space:]]*(head|tail|grep)([[:space:]]|$)' && pipe=1
    [ -n "$pipe$subst" ] || return 0

    [ -z "$pipe" ] || msg+=(
        "NOTE: a $mutates is piped into head/tail/grep, so the pipeline's exit status is"
        "      the filter's (0) and a REJECTED command looks identical to one that"
        "      worked: a rejected 'git commit ... | tail -1' reads as success"
        "      and nothing was committed."
        "      ${GUARD_PIPE_HINT:-Check the status first, then truncate.}")
    [ -z "$subst" ] || msg+=(
        "NOTE: a $mutates takes an argument from \$(cat ...), and a FAILED read there"
        "      is a valid EMPTY STRING, not an error. The command then succeeds with"
        "      an argument nobody supplied. This exact shape once wrote a record with empty"
        "      notes at rc=0 after its temp file had already been removed."
        "      Read it first, then check:"
        "          body=\$(cat f) || exit 1; [ -n \"\$body\" ] || exit 1")
    guard_advise "${msg[@]}"
}
