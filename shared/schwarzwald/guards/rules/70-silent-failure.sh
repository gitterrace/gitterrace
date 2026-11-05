#!/usr/bin/env bash
# A mutating command piped into a truncating filter, with no pipefail: a
# rejected write piped into `| tail -1` or `| grep` reports success. Not caught:
# reads, a pipe into `tee`, `set -o pipefail`, GUARD_ALLOW_SILENT=1.

guard_register no_silent_mutation
guard_rule_no_silent_mutation() {
    [ "$GUARD_TOOL" = bash ] || return 0
    [ -n "$GUARD_CMD" ] || return 0

    guard_cmd_has 'set -o pipefail|set -eo? *pipefail|GUARD_ALLOW_SILENT=1' && return 0

    local mutates=''
    [ -n "${GUARD_WRITE_CMDS_RE:-}" ] && guard_cmd_is "$GUARD_WRITE_CMDS_RE" && mutates='product CLI write'
    guard_cmd_is 'git[[:space:]]+(commit|add|push|merge|rebase|tag|reset)' && mutates='git write'
    guard_cmd_is 'cargo[[:space:]]+(install|publish|fix)' && mutates='cargo write'
    guard_cmd_is '(npm|pnpm|yarn)[[:space:]]+(install|publish)' && mutates='package install'
    [ -n "$mutates" ] || return 0

    guard_cmd_has '\|[[:space:]]*(head|tail|grep)([[:space:]]|$)' || return 0

    guard_advise \
        "NOTE: a $mutates is piped into head/tail/grep, so the pipeline's exit status is" \
        "      the filter's (0) and a REJECTED command looks identical to one that" \
        "      worked: a rejected 'git commit ... | tail -1' reads as success" \
        "      and nothing was committed." \
        "      Check the status first, then truncate:" \
        "          out=\$(cmd 2>&1) || { printf '%s\\n' \"\$out\"; exit 1; }" \
        "      or put 'set -o pipefail' ahead of the pipeline."
}

# A FAILED command substitution is a valid EMPTY string: `--notes "$(cat f)"`
# after f vanished wrote an empty field at rc=0. Not caught: reads
# (noisy, not silent), substitutions that read no file (`$(date)`), guarded
# ones, GUARD_ALLOW_SILENT=1.
guard_register no_empty_substitution
guard_rule_no_empty_substitution() {
    [ "$GUARD_TOOL" = bash ] || return 0
    [ -n "$GUARD_CMD" ] || return 0

    guard_cmd_has 'set -o pipefail|set -eo? *pipefail|GUARD_ALLOW_SILENT=1' && return 0

    local mutates=''
    [ -n "${GUARD_WRITE_CMDS_RE:-}" ] && guard_cmd_is "$GUARD_WRITE_CMDS_RE" && mutates='product CLI write'
    guard_cmd_is 'git[[:space:]]+(commit|tag)' && mutates='git write'
    [ -n "$mutates" ] || return 0

    # A substitution that reads a FILE: `$(cat f)`, `$(< f)`, `$(head f)`.
    guard_cmd_has '\$\([[:space:]]*(cat|head|tail)[[:space:]]|\$\([[:space:]]*<' || return 0

    guard_advise \
        "NOTE: a $mutates takes an argument from \$(cat ...), and a FAILED read there" \
        "      is a valid EMPTY STRING, not an error. The command then succeeds with" \
        "      an argument nobody supplied. This exact shape once wrote a record with empty" \
        "      notes at rc=0 after its temp file had already been removed." \
        "      Read it first, then check:" \
        "          body=\$(cat f) || exit 1; [ -n \"\$body\" ] || exit 1"
}
