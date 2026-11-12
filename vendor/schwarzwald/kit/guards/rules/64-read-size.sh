#!/usr/bin/env bash
#
# Unbounded reads: a whole big file pulled into the context when a region would do. Warn mode by
# default (GUARD_READ_MODE=warn), so a week of logs shows the real sizes before anything blocks.

# Echo the line count of $1 when it is a regular text file, nothing otherwise. Images and
# documents are not lines, and a missing file is the tool's own error to give.
_read_size_lines() {
    local f="$1"
    case "$f" in
        /*) ;;
        *) f="${GUARD_CWD:-.}/$f" ;;
    esac
    case "$f" in
        *.png | *.jpg | *.jpeg | *.gif | *.webp | *.heic | *.pdf | *lane-accept*) return 0 ;;
    esac
    [ -f "$f" ] || return 0
    local n
    n=$(wc -l <"$f" 2>/dev/null) || return 0
    printf '%s' "${n//[[:space:]]/}"
}

_read_size_report() {
    local file="$1" lines="$2" how="$3" max="${GUARD_READ_MAX_LINES:-400}"
    local msg="$how of $file takes all $lines lines into the context (limit $max)."
    if [ "${GUARD_READ_MODE:-warn}" = block ]; then
        guard_block "$msg" \
            "Read a region (offset and limit), or search first: rg -n '<pattern>' $file" \
            "A build log or a long diff goes to a file and is grepped, not read whole."
    else
        guard_advise "Reminder: $msg" \
            "Read a region (offset and limit), or search first: rg -n '<pattern>' $file"
    fi
}

guard_register read_size
guard_rule_read_size() {
    local max="${GUARD_READ_MAX_LINES:-400}"
    [[ $max =~ ^[0-9]+$ ]] || max=400
    local n
    # Not caught: a file that grows past the limit between this check and the read, a Read of a
    # directory, and any read through a tool other than Read, cat, head or tail.
    if [ "$GUARD_TOOL" = read ]; then
        [ -n "$GUARD_FILE" ] || return 0
        [ -z "${GUARD_READ_BOUND:-}" ] || return 0
        n=$(_read_size_lines "$GUARD_FILE")
        [ -n "$n" ] && [ "$n" -gt "$max" ] && _read_size_report "$GUARD_FILE" "$n" "A whole Read"
        return 0
    fi
    [ "$GUARD_TOOL" = bash ] || return 0

    # One plain command, an optional `cd dir &&` before it. A pipe, a redirect, a chain or a
    # substitution means the output is filtered or sent elsewhere, and passes. lane-accept
    # output is read whole on purpose.
    local head
    head=$(guard_cmd_unquoted)
    local cwd="${GUARD_CWD:-.}" cd_re='^[[:space:]]*cd[[:space:]]+([^;&|<>$`[:space:]]+)[[:space:]]*&&[[:space:]]*'
    if [[ $head =~ $cd_re ]]; then
        case "${BASH_REMATCH[1]}" in
            /*) cwd="${BASH_REMATCH[1]}" ;;
            *) cwd="$cwd/${BASH_REMATCH[1]}" ;;
        esac
        head="${head#"${BASH_REMATCH[0]}"}"
    fi
    guard_match "$head" 'lane-accept' && return 0
    guard_match "$head" '[|;&<>$`(]' && return 0
    local verb_re='^[[:space:]]*(cat|head|tail)[[:space:]]+(.*[^[:space:]])[[:space:]]*$'
    [[ $head =~ $verb_re ]] || return 0
    local verb="${BASH_REMATCH[1]}" args="${BASH_REMATCH[2]}" a files=()
    # A count (-n, -c, -20) bounds head and tail; tail -f never ends and is a different problem.
    if [ "$verb" != cat ] && guard_match " $args" '[[:space:]]-([0-9]+|[A-Za-z]*[nc])([[:space:]]|[0-9]|$)|[[:space:]]--(lines|bytes)'; then
        return 0
    fi
    # Head and tail show ten lines with no count: not a whole read.
    [ "$verb" = cat ] || return 0
    for a in $args; do
        case "$a" in -*) ;; *) files+=("$a") ;; esac
    done
    local f saved="$GUARD_CWD"
    GUARD_CWD="$cwd"
    for f in "${files[@]}"; do
        n=$(_read_size_lines "$f")
        if [ -n "$n" ] && [ "$n" -gt "$max" ]; then
            GUARD_CWD="$saved"
            _read_size_report "$f" "$n" "cat"
            return 0
        fi
    done
    GUARD_CWD="$saved"
    return 0
}
