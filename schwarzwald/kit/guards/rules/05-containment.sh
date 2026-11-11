#!/usr/bin/env bash
#
# Containment: mutating commands must stay inside the org tree. Not a
# security boundary: a symlink, alias, or built path can still escape it.

_contain_verbs='(/usr/bin/|/bin/)?(rm|rmdir|mv|cp|dd|truncate|shred|chown|chmod|ln)'

# Expand $HOME / ${HOME} and a leading ~ ONLY: general expansion would mean
# evaluating the input. The limit is stated in the block message on purpose.
_contain_expand() {
    local p="$1"
    case "$p" in
        \"*\")
            p="${p#\"}"
            p="${p%\"}"
            ;;
        \'*\')
            p="${p#\'}"
            p="${p%\'}"
            ;;
    esac
    # `dd of=/etc/hosts` was allowed: no argument looked absolute. `if=` is NOT
    # stripped: it is a read.
    case "$p" in
        of=*) p="${p#of=}" ;;
    esac
    case "$p" in
        \"*\")
            p="${p#\"}"
            p="${p%\"}"
            ;;
        \'*\')
            p="${p#\'}"
            p="${p%\'}"
            ;;
    esac
    # shellcheck disable=SC2016
    case "$p" in
        '${HOME}'/*) p="$HOME/${p#'${HOME}'/}" ;;
        '$HOME'/*) p="$HOME/${p#'$HOME'/}" ;;
        "~"*) p="$HOME${p#\~}" ;;
    esac
    printf '%s' "$p"
}

# Lexical normalisation, no fork: `<org>/../../etc` passes a prefix test.
_contain_norm() {
    local IFS=/ part had_noglob="" out=()
    case "$-" in *f*) had_noglob=1 ;; esac
    set -f
    for part in $1; do
        case "$part" in
            "" | .) ;;
            ..) [ "${#out[@]}" -gt 0 ] && unset "out[${#out[@]}-1]" ;;
            *) out[${#out[@]}]="$part" ;;
        esac
    done
    [ -n "$had_noglob" ] || set +f
    printf '/%s' "${out[*]:-}"
}

# The cwd after the LAST `cd`, else $2: `cd /x && rm -rf y` names no
# absolute path after the verb.
_contain_base() {
    local base="$2" d rest="$1"
    while [[ $rest =~ $GUARD_ANCHOR(pushd|cd)[[:space:]]+([^[:space:];\&\|]+) ]]; do
        d=$(_contain_expand "${BASH_REMATCH[$((${#BASH_REMATCH[@]} - 1))]}")
        case "$d" in
            /*) base="$d" ;;
            -) ;;
            *) base="$base/$d" ;;
        esac
        rest="${rest#*"${BASH_REMATCH[0]}"}"
    done
    _contain_norm "$base"
}

# Is the normalised absolute path $1 under an agent scratch root: GUARD_SCRATCH_ROOTS
# (guards.env, literal paths) or the session's own GUARD_SCRATCH_DIR (set by the
# adapter)? A root that is relative, holds `..`, or is $HOME or an ancestor of it is
# ignored, so a misconfigured root can never open the whole home directory. Not caught: a
# symlink inside a scratch root that points out of it.
_contain_scratch() {
    local r h="${HOME%/}"
    for r in ${GUARD_SCRATCH_ROOTS:-} ${GUARD_SCRATCH_DIR:-}; do
        r="${r%/}"
        case "$r" in "" | *..* | [!/]*) continue ;; esac
        case "$h/" in "$r"/*) continue ;; esac
        case "$1" in "$r" | "$r"/*) return 0 ;; esac
    done
    return 1
}

_contain_check() {
    local cmd="$1" org="$2" tmp="${TMPDIR:-/tmp}"
    tmp="${tmp%/}"

    guard_match "$cmd" "$GUARD_ANCHOR($_contain_verbs)[[:space:]]" || return 0

    # Only the SEGMENT from the verb on: a whole-line scan blocked a python3 -c
    # whose source merely mentioned "rm -rf ~/Documents/x".
    local seg=""
    local rest="$cmd"
    while [[ $rest =~ $GUARD_ANCHOR($_contain_verbs)[[:space:]][^\;\&\|]* ]]; do
        seg="$seg ${BASH_REMATCH[0]}"
        rest="${rest#*"${BASH_REMATCH[0]}"}"
    done
    [ -n "${seg// /}" ] || return 0
    # The verb must be the first word for dest_only below.
    seg="${seg#"${seg%%[![:space:];&|]*}"}"

    # cp/ln write only the destination: judging the source blocked taking a
    # backup. mv is NOT dest-only: it destroys its source (`mv /etc/hosts /tmp/x`).
    local dest_only=0 is_mv=0
    case "$seg" in
        cp\ * | ln\ * | /bin/cp\ * | /usr/bin/cp\ * | /bin/ln\ * | /usr/bin/ln\ *) dest_only=1 ;;
        mv\ * | /bin/mv\ *) is_mv=1 ;;
    esac

    # Expanded like the loop below: a raw `last` never matched a quoted dest.
    local p last=""
    for p in $seg; do
        case "$p" in -*) continue ;; esac
        last=$(_contain_expand "$p")
    done

    # A relative path is skipped only when it cannot leave the org: no `..` and
    # no `cd`.
    local base moved=0 trash_dest=""
    base=$(_contain_base "$cmd" "${GUARD_CWD:-$org}")
    [ "$base" = "$(_contain_norm "${GUARD_CWD:-$org}")" ] || moved=1
    case "$last" in /*) trash_dest=$(_contain_norm "$last") ;; esac

    for p in $seg; do
        p=$(_contain_expand "$p")
        if [ "$dest_only" -eq 1 ] && [ "$p" != "$last" ]; then
            continue
        fi
        case "$p" in
            /*) p=$(_contain_norm "$p") ;;
            -*) continue ;;
            *)
                case "$p" in
                    *..*) ;;
                    *) [ "$moved" = 1 ] || continue ;;
                esac
                p=$(_contain_norm "$base/$p")
                ;;
        esac
        # Removing is a move to the Trash (rules.md, Parallel work): the Trash is a fine
        # DESTINATION of an mv, and nothing else (not a source, not an rm target).
        if [ "$is_mv" -eq 1 ] && [ "$p" = "$trash_dest" ]; then
            case "$p" in "${HOME%/}/.Trash"/?*) continue ;; esac
        fi
        case "$p" in
            "$org" | "$org"/*) continue ;;
            "$tmp" | "$tmp"/*) continue ;;
            # $TMPDIR is per-user on macOS, so /tmp needs its own exemption.
            /tmp | /tmp/* | /private/tmp | /private/tmp/* | /var/folders/*) continue ;;
            *)
                _contain_scratch "$p" && continue
                guard_block \
                    "this would touch $p, outside the org tree ($org)." \
                    "Destructive commands stay inside the org, or under \$TMPDIR or the agent scratch dir." \
                    "Note the limit: only literal paths, \$HOME/\${HOME} and ~ are resolved. A path built" \
                    "from any other variable, a \$(...) or a symlink is NOT checked -- this raises the cost" \
                    "of an accident, it is not a sandbox."
                return 1
                ;;
        esac
    done
    return 0
}

guard_register containment
guard_rule_containment() {
    [ "$GUARD_TOOL" = bash ] || return 0
    _contain_check "$(guard_cmd_head)" "$GUARD_ORG"
    return 0
}
