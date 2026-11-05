#!/usr/bin/env bash
#
# FAIL-CLOSED FALLBACK: answers when the engine cannot (fork pressure, no jq,
# no verdict, a dead adapter: a failed fork exits 128, read as "allow"). Owner
# ruling: refuse the DESTRUCTIVE class, allow everything else.
#
# Builtins only, NO FORK: it must answer when the process table is full. It is
# also THE ONE DENY LIST: rules/80 asks guard_fallback_judge with
# GUARD_FALLBACK_HEALTHY=1, because a second list drifted (`git reset --hard`
# passed the engine and failed here).
#
# Cruder than the engine on purpose. A verb counts ANYWHERE in a segment
# (`echo rm -rf x`, `find -exec rm`). Quoted text is prose unless it is code
# (`bash -c`, `eval`, a double-quoted `$(`), and confusing quoting errs toward
# refusing: a false refusal costs one retry, a false allow an afternoon.

GUARD_FALLBACK_SETTLED=""
GUARD_FALLBACK_BLOCK_RC=2
GUARD_FALLBACK_VALUE=""
GUARD_FALLBACK_REASON=""

# The org root, derived like the engine's, via the `cd` builtin.
_gf_self="${BASH_SOURCE[0]}"
case "$_gf_self" in
    /*) ;;
    *) _gf_self="$PWD/$_gf_self" ;;
esac
# This file sits at <product>/shared/schwarzwald/guards/rules/fallback/, so the product root is
# five directories up and the org is its parent.
if [ -n "${GUARD_REPO:-}" ]; then
    GUARD_FALLBACK_ORG="${GUARD_REPO%/*}"
else
    _gf_back="$PWD"
    if cd "${_gf_self%/*}/../../../../.." 2>/dev/null; then
        GUARD_FALLBACK_ORG="${PWD%/*}"
    else
        GUARD_FALLBACK_ORG="${_gf_self%/*/*/*/*/*/*/*}"
    fi
    cd "$_gf_back" 2>/dev/null || :
fi
# The product's name for the warnings, read with builtins (config.sh forks nothing). A config
# that is missing or not data changes nothing here: the deny list below needs no config.
# shellcheck source=/dev/null
if . "${_gf_self%/*}/../../config.sh" 2>/dev/null; then
    guard_config_locate "${_gf_self%/*/*/*}"
    guard_config_load 2>/dev/null || :
fi

# From here on, an exit that was not settled is judged by the fallback, with
# $1 as the harness's block exit code.
guard_fallback_arm() {
    GUARD_FALLBACK_BLOCK_RC="${1:-2}"
    trap 'guard_fallback_on_exit $?' EXIT
}

# A considered exit (an engine verdict, a usage error): the trap stays quiet.
guard_fallback_settle() {
    GUARD_FALLBACK_SETTLED=1
    exit "${1:-0}"
}

guard_fallback_on_exit() {
    [ -z "$GUARD_FALLBACK_SETTLED" ] || return 0
    guard_fallback_degraded "the adapter died before the engine answered (exit $1: a failed fork, or a crash)."
}

# The guards are not running. Say so, judge with the fallback, exit.
guard_fallback_degraded() {
    GUARD_FALLBACK_SETTLED=1
    echo "WARNING: ${GUARD_PRODUCT:-guard} guards are NOT running -- $1" >&2
    echo "         Fallback: the destructive class is REFUSED until this is fixed; everything else is allowed unguarded." >&2
    echo "         Check: bash -n ${GUARD_ENGINE:-shared/schwarzwald/guards/engine.sh}" >&2
    if guard_fallback_judge; then
        exit 0
    fi
    printf 'Blocked: %s\n' "$GUARD_FALLBACK_REASON" >&2
    echo "         The guards are DEGRADED, so this is the fail-closed fallback, not the engine." >&2
    echo "         Retry once they recover; read-only commands still pass." >&2
    exit "$GUARD_FALLBACK_BLOCK_RC"
}

# The string value of "$2" in the JSON text $1, into GUARD_FALLBACK_VALUE.
# Regex, not a parser: the first occurrence, cut off where a truncation cut it.
guard_fallback_json_string() {
    GUARD_FALLBACK_VALUE=""
    local re='"'"$2"'"[[:space:]]*:[[:space:]]*"(([^"\\]|\\.)*)'
    [[ ${1:-} =~ $re ]] || return 0
    # Patterns and replacements go through variables: bash 3.2 mangles a
    # backslash written literally inside ${v//pat/rep}.
    # shellcheck disable=SC1003 # a lone backslash, not an escaped quote
    local v="${BASH_REMATCH[1]}" bs='\' q='"' sl='/' nl=$'\n' sp=' ' one=$'\001' p
    p="$bs$bs" && v="${v//"$p"/$one}"
    p="${bs}n" && v="${v//"$p"/$nl}"
    p="${bs}r" && v="${v//"$p"/$nl}"
    p="${bs}t" && v="${v//"$p"/$sp}"
    p="$bs$q" && v="${v//"$p"/$q}"
    p="$bs$sl" && v="${v//"$p"/$sl}"
    v="${v//"$one"/$bs}"
    GUARD_FALLBACK_VALUE="$v"
}

# Drop quoted prose, keep quoted code, into GUARD_FALLBACK_VALUE. A dropped run
# leaves a placeholder so `git -C "a dir" stash` keeps its shape; a kept run is
# re-read so nested quotes count. Code: after -c/eval, a double-quoted `$(`, a
# command piped into a shell, an interpreter's -c/-e body (whose own string
# literals are kept too: `os.system('git stash')`).
_gf_unquote() {
    local rest="$1" out="" pre q body tail sq="'" dq='"'
    while [ -n "$rest" ]; do
        pre="${rest%%[\"\']*}"
        if [ "$pre" = "$rest" ]; then
            out="$out$rest"
            break
        fi
        out="$out$pre"
        q="${rest:${#pre}:1}"
        rest="${rest:$((${#pre} + 1))}"
        body="${rest%%"$q"*}"
        if [ "$body" = "$rest" ]; then
            out="$out ; $rest"
            break
        fi
        rest="${rest:$((${#body} + 1))}"
        _gf_code_kind "$out"
        if [ -n "${GUARD_FALLBACK_ALLCODE:-}" ] || [ "$GUARD_FALLBACK_VALUE" = interp ]; then
            body="${body//"$sq"/ }"
            out="$out ; ${body//"$dq"/ } ; "
            continue
        fi
        tail="${out%"${out##*[! ]}"}"
        case "$tail" in
            *-c | *eval)
                rest=" ; $body ; $rest"
                continue
                ;;
        esac
        if [ "$q" = '"' ]; then
            # shellcheck disable=SC2016 # the literal text `$(` and a backtick
            case "$body" in
                *'$('* | *'`'*)
                    rest=" ; $body ; $rest"
                    continue
                    ;;
            esac
        fi
        # A quoted run with no whitespace can BE the command (`"git" stash`,
        # `'rm' -rf x`), so it stays a word; one with whitespace is prose.
        case "$body" in
            "" | *[[:space:]]*) out="${out}Q" ;;
            *) out="$out$body" ;;
        esac
    done
    GUARD_FALLBACK_VALUE="$out"
}

# Is the quoted run in $1 an interpreter body (`perl -e`, `python3 -c`)? Sets
# GUARD_FALLBACK_VALUE to `interp` or empty. Shells are re-read instead.
_gf_code_kind() {
    local tail="${1%"${1##*[! ]}"}" last seg w nl=$'\n' tab=$'\t'
    GUARD_FALLBACK_VALUE=""
    last="${tail##*[ "$tab"]}"
    case "$last" in
        --eval | --print | --command | -[A-Za-z]*[ceEp] | -[ceEp]) ;;
        *) return 0 ;;
    esac
    seg="${tail##*[;&|(\`"$nl"]}"
    for w in $seg; do
        case "${w##*/}" in
            perl* | python* | ruby* | node | nodejs | osascript | deno | bun | php | lua* | Rscript | swift)
                GUARD_FALLBACK_VALUE=interp
                return 0
                ;;
        esac
    done
    return 0
}

# Heredoc bodies out, into GUARD_FALLBACK_VALUE. Cutting at the first `<<` hid
# `cat <<<x; git stash`. `<<<` drops only the operator; `<<WORD` drops the body
# but keeps the rest of its line and everything after the terminator; a body
# fed to a shell, or an unquoted body's `$(`/backtick lines, is kept as code;
# a `<<` inside quotes is no operator.
_gf_strip_heredocs() {
    local rest="${1:-}" out="" nl=$'\n' tab=$'\t' bt='`'
    local line cur acc pre after q1 sq dq c d quoted keep seg w more t
    local -a pd=() pk=()
    local np=0 ph=0
    while :; do
        if [[ $rest == *"$nl"* ]]; then
            line="${rest%%"$nl"*}"
            rest="${rest#*"$nl"}"
            more=1
        else
            line="$rest"
            rest=""
            more=0
        fi
        if [ "$ph" -lt "$np" ]; then
            t="${line#"${line%%[!"$tab"]*}"}"
            if [ "$t" = "${pd[$ph]}" ]; then
                ph=$((ph + 1))
            else
                case "${pk[$ph]}" in
                    all) out="$out$line$nl" ;;
                    subst)
                        case "$line" in
                            *"\$("* | *"$bt"*) out="$out$line$nl" ;;
                        esac
                        ;;
                esac
            fi
        else
            cur="$line"
            acc=""
            while [[ $cur == *'<<'* ]]; do
                pre="${cur%%<<*}"
                after="${cur#*<<}"
                q1="$out$acc$pre"
                sq="${q1//[^\']/}"
                dq="${q1//[^\"]/}"
                # Inside double quotes it is an operator only after a `$(`:
                # `"$(cat <<'EOF' ... EOF)"` is the usual message body.
                quoted=0
                if [ $((${#sq} % 2)) -eq 1 ]; then
                    quoted=1
                elif [ $((${#dq} % 2)) -eq 1 ]; then
                    t="${q1##*\"}"
                    case "$t" in
                        *"\$("* | *"$bt"*) ;;
                        *) quoted=1 ;;
                    esac
                fi
                if [ "$quoted" = 1 ]; then
                    acc="$acc$pre<<"
                    cur="$after"
                    continue
                fi
                seg="${q1##*[;&|(\`"$nl"]}"
                keep=""
                for w in $seg; do
                    case "${w##*/}" in
                        bash | sh | zsh | dash | ksh | eval | source | .) keep=all ;;
                    esac
                done
                if [ "${after:0:1}" = "<" ]; then
                    # The word stays; fed to a shell, `eval` keeps it as code.
                    if [ -n "$keep" ]; then acc="$acc$pre eval "; else acc="$acc$pre "; fi
                    cur="${after#<}"
                    continue
                fi
                [ "${after:0:1}" = "-" ] && after="${after:1}"
                after="${after#"${after%%[! ]*}"}"
                c="${after:0:1}"
                quoted=0
                case "$c" in
                    "'" | '"')
                        after="${after:1}"
                        d="${after%%"$c"*}"
                        after="${after#*"$c"}"
                        quoted=1
                        ;;
                    *)
                        d="${after%%[ "$tab";&|()<>]*}"
                        after="${after:${#d}}"
                        case "$d" in *[\'\"\\]*) quoted=1 ;; esac
                        d="${d//[\'\"\\]/}"
                        ;;
                esac
                if [ -z "$d" ]; then
                    acc="$acc$pre<<"
                    cur="$after"
                    continue
                fi
                if [ -z "$keep" ]; then
                    if [ "$quoted" = 1 ]; then keep=drop; else keep=subst; fi
                fi
                pd[np]="$d"
                pk[np]="$keep"
                np=$((np + 1))
                acc="$acc$pre "
                cur="$after"
            done
            out="$out$acc$cur$nl"
        fi
        [ "$more" = 1 ] || break
    done
    GUARD_FALLBACK_VALUE="$out"
}

# Returns 0 to allow, 1 to refuse with GUARD_FALLBACK_REASON set. Globbing is
# off throughout, so a `*` in the command is a word, not this cwd's files.
guard_fallback_judge() {
    local had_noglob="" rc
    case "$-" in *f*) had_noglob=1 ;; esac
    set -f
    _gf_judge
    rc=$?
    [ -n "$had_noglob" ] || set +f
    return "$rc"
}

_gf_engine_judge() {
    guard_fallback_judge
}

# Degraded-only checks (the engine has a better answer, or the refusal would
# cost every session): `rm -r`, in-org `rm`, a command named by an unresolved
# `$var` (`"$BIN" --help` is ordinary work).
_gf_judge() {
    GUARD_FALLBACK_REASON=""
    # A post_tool call has already run; there is nothing left to refuse.
    [ "${GUARD_EVENT:-pre_tool}" = pre_tool ] || return 0
    case "${GUARD_TOOL:-}" in
        bash | Bash | BASH) ;;
        *) return 0 ;;
    esac
    local org="$GUARD_FALLBACK_ORG" cwd="${GUARD_CWD:-}" inorg=1
    # Outside the org, judged only if the command can reach back in
    # (`git -C <repo> stash` from /tmp). Cheap test first. An UNKNOWN cwd is
    # judged.
    if [ -n "$cwd" ]; then
        case "$cwd" in
            "$org" | "$org"/*) ;;
            *)
                case "${GUARD_CMD:-}" in
                    *"$org"* | *'~'* | *HOME* | *..*) inorg=0 ;;
                    *) return 0 ;;
                esac
                ;;
        esac
    fi

    # shellcheck disable=SC1003 # a lone backslash, not an escaped quote
    local s="${GUARD_CMD:-}" c bs='\' allcode=""
    _gf_strip_heredocs "$s"
    s="$GUARD_FALLBACK_VALUE"
    s="${s//"$bs"/}"
    # Text piped into a shell is code, quotes and all: `printf 'git stash' | bash`.
    local pipesh='\|[[:space:]]*(([^[:space:]]*/)?(env|exec|command|sudo|nohup)[[:space:]]+)*([^[:space:]]*/)?(ba|z|da|k)?sh([[:space:]]|$)'
    [[ $s =~ $pipesh ]] && allcode=1
    # `${NAME}` becomes `$NAME`, so its braces are not read as a group below.
    local brace='\$\{([A-Za-z_][A-Za-z0-9_]*)\}'
    while [[ $s =~ $brace ]]; do
        s="${s/"${BASH_REMATCH[0]}"/\$${BASH_REMATCH[1]}}"
    done
    GUARD_FALLBACK_ALLCODE="$allcode" _gf_unquote "$s"
    s="$GUARD_FALLBACK_VALUE"
    for c in ';' '&' '|' '(' ')' '{' '}' '`' $'\n'; do
        s="${s//"$c"/ ; }"
    done

    local IFS=$' \t'
    local -a W
    # shellcheck disable=SC2206 # word splitting is the point; globbing is off
    W=($s)
    if [ "$inorg" = 0 ]; then
        _gf_reaches_org || return 0
        # The cwd was left, and containment does not run for a foreign cwd.
        cwd=""
    fi
    local healthy="${GUARD_FALLBACK_HEALTHY:-}"
    [ "$inorg" = 1 ] || healthy=""
    local n=${#W[@]} i=0 w cp=1 k found
    # Assignments seen in command position, so `g=git; $g stash` is `git stash`.
    local -a an=() av=()
    while [ "$i" -lt "$n" ]; do
        w="${W[$i]:-}"
        if [ "$w" = ";" ]; then
            cp=1
            i=$((i + 1))
            continue
        fi
        if [ "$cp" = 1 ]; then
            case "$w" in
                [A-Za-z_]*=*)
                    an[${#an[@]}]="${w%%=*}"
                    av[${#av[@]}]="${w#*=}"
                    i=$((i + 1))
                    continue
                    ;;
                sudo | env | command | builtin | exec | nohup | time | nice | xargs | -*)
                    i=$((i + 1))
                    continue
                    ;;
                '$'[A-Za-z_]*)
                    found=""
                    k=${#an[@]}
                    while [ "$k" -gt 0 ]; do
                        k=$((k - 1))
                        if [ "\$${an[$k]}" = "$w" ]; then
                            found=1
                            w="${av[$k]}"
                            W[i]="$w"
                            break
                        fi
                    done
                    if [ -z "$found" ] && [ -z "$healthy" ]; then
                        _gf_refuse "a command named by a variable ($w) cannot be judged while the guards are degraded."
                        return 1
                    fi
                    ;;
            esac
            cp=0
        fi
        case "${w##*/}" in
            git) _gf_git "$((i + 1))" || return 1 ;;
            rm) [ -n "$healthy" ] || _gf_rm "$((i + 1))" "$cwd" || return 1 ;;
            find) _gf_find "$((i + 1))" "$cwd" || return 1 ;;
            sed | gsed | perl)
                _gf_segment "$((i + 1))"
                _gf_flags "${w##*/}" "$GUARD_FALLBACK_VALUE" || return 1
                ;;
        esac
        i=$((i + 1))
    done
    return 0
}

# Does a command from OUTSIDE the org reach into it (org path in a word, or a
# cd/pushd/-C/--git-dir/--work-tree target resolving inside)?
_gf_reaches_org() {
    local w prev="" p org="$GUARD_FALLBACK_ORG"
    for w in "${W[@]}"; do
        case "$w" in *"$org"*) return 0 ;; esac
        p=""
        case "$prev" in
            cd | pushd | -C | --git-dir | --work-tree) p="$w" ;;
        esac
        case "$w" in
            --git-dir=* | --work-tree=*) p="${w#*=}" ;;
            -C?*) p="${w#-C}" ;;
        esac
        if [ -n "$p" ] && _gf_resolve "$p" "${GUARD_CWD:-}"; then
            case "$GUARD_FALLBACK_VALUE" in
                "$org" | "$org"/*) return 0 ;;
            esac
        fi
        prev="${w##*/}"
    done
    return 1
}

# Lexical normalisation of $1 into GUARD_FALLBACK_VALUE. No fork, no symlinks.
_gf_norm() {
    local IFS=/ part
    local -a st=()
    for part in $1; do
        case "$part" in
            "" | .) ;;
            ..) [ "${#st[@]}" -eq 0 ] || unset "st[$((${#st[@]} - 1))]" ;;
            *) st[${#st[@]}]="$part" ;;
        esac
    done
    GUARD_FALLBACK_VALUE="/${st[*]:-}"
}

# $1 against base $2, absolute and normalised. Only `~`, `$HOME`, `$TMPDIR`
# expand; returns 1 for any other variable or a relative path with no base.
_gf_resolve() {
    local p="$1" base="$2" dh="\$HOME" dt="\$TMPDIR" tmp="${TMPDIR:-/tmp}"
    case "$p" in
        \~ | \~/*) p="$HOME${p#\~}" ;;
        "$dh" | "$dh"/*) p="$HOME${p#"$dh"}" ;;
        "$dt" | "$dt"/*) p="${tmp%/}${p#"$dt"}" ;;
    esac
    case "$p" in
        *'$'*) return 1 ;;
        /*) ;;
        *)
            [ -n "$base" ] || return 1
            p="$base/$p"
            ;;
    esac
    _gf_norm "$p"
    return 0
}

# Safe to delete while degraded: scratch, or outside the org. Unresolvable
# counts as in-org.
_gf_scratch() {
    _gf_resolve "$1" "$2" || return 1
    local p="$GUARD_FALLBACK_VALUE" tmp="${TMPDIR:-/tmp}"
    case "$p" in
        /tmp | /tmp/* | /private/tmp | /private/tmp/* | /var/folders/*) return 0 ;;
        "${tmp%/}" | "${tmp%/}"/*) return 0 ;;
        "$GUARD_FALLBACK_ORG" | "$GUARD_FALLBACK_ORG"/*) return 1 ;;
    esac
    return 0
}

# The words of W from $1 to the end of the segment, space-padded for `case`.
_gf_segment() {
    local j="$1" w out=" "
    while [ "$j" -lt "${#W[@]}" ]; do
        w="${W[$j]:-}"
        [ "$w" = ";" ] && break
        out="$out$w "
        j=$((j + 1))
    done
    GUARD_FALLBACK_VALUE="$out"
}

_gf_refuse() {
    GUARD_FALLBACK_REASON="$1"
    return 1
}

_gf_git() {
    local j="$1" sub args w
    while :; do
        case "${W[$j]:-}" in
            -c)
                # An alias defined on the command line can BE any subcommand,
                # or a shell (`alias.x=!git stash`), and nothing here expands it.
                case "${W[$((j + 1))]:-}" in
                    alias.* | [Aa][Ll][Ii][Aa][Ss].*)
                        _gf_refuse "git -c alias.* defines a subcommand this guard cannot see through."
                        return 1
                        ;;
                esac
                j=$((j + 2))
                ;;
            -c[Aa][Ll][Ii][Aa][Ss].*)
                _gf_refuse "git -c alias.* defines a subcommand this guard cannot see through."
                return 1
                ;;
            -C | --git-dir | --work-tree | --namespace | --exec-path | --config-env) j=$((j + 2)) ;;
            -*) j=$((j + 1)) ;;
            *) break ;;
        esac
    done
    sub="${W[$j]:-}"
    _gf_segment "$((j + 1))"
    args="$GUARD_FALLBACK_VALUE"
    case "$sub" in
        stash)
            case "$args" in
                " list "* | " show "*) return 0 ;;
            esac
            _gf_refuse "git stash sweeps up OTHER sessions' uncommitted work in a shared checkout."
            ;;
        checkout-index)
            _gf_refuse "git checkout-index overwrites working-tree files from the index."
            ;;
        restore)
            local staged=0 wt=0
            for w in $args; do
                case "$w" in
                    --staged) staged=1 ;;
                    --worktree) wt=1 ;;
                    --*) ;;
                    -*S*W* | -*W*S*) staged=1 wt=1 ;;
                    -*S*) staged=1 ;;
                    -*W*) wt=1 ;;
                esac
            done
            if [ "$staged" = 0 ] || [ "$wt" = 1 ]; then
                _gf_refuse "git restore of a path discards uncommitted work there; only 'restore --staged' passes while degraded."
            fi
            ;;
        checkout)
            _gf_checkout "$args"
            ;;
        *)
            w="${args# }"
            _gf_flags "git $sub ${w%% *}" "$args"
            ;;
    esac
}

# A checkout is a DISCARD when it names a path: `--`, `.`, force/patch/ours,
# or an argument that exists on disk. Branch work passes.
_gf_checkout() {
    local w skip=0 p
    for w in $1; do
        if [ "$skip" = 1 ]; then
            skip=0
            continue
        fi
        case "$w" in
            -b | -B | --orphan) skip=1 ;;
            -- | . | ./ | -f | --force | -p | --patch | --ours | --theirs)
                _gf_refuse "git checkout of a PATH discards uncommitted work there, and this checkout is SHARED."
                return 1
                ;;
            -*) ;;
            *)
                case "$w" in
                    /*) p="$w" ;;
                    *) p="${GUARD_CWD:-$PWD}/$w" ;;
                esac
                if [ -e "$p" ]; then
                    _gf_refuse "git checkout of a PATH discards uncommitted work there, and this checkout is SHARED."
                    return 1
                fi
                ;;
        esac
    done
    return 0
}

# Degraded only: `rm -r` anywhere, `rm` of anything but scratch or outside the
# org. An unresolvable path counts as in the org.
_gf_rm() {
    local w ends=""
    _gf_segment "$1"
    for w in $GUARD_FALLBACK_VALUE; do
        if [ -z "$ends" ]; then
            case "$w" in
                --) ends=1 && continue ;;
                --recursive) _gf_refuse "rm -r deletes a tree that may hold another session's work." || return 1 ;;
                --*) continue ;;
                -*r* | -*R*) _gf_refuse "rm -r deletes a tree that may hold another session's work." || return 1 ;;
                -*) continue ;;
            esac
        fi
        # A redirection is not an operand.
        case "$w" in *">"* | *"<"*) continue ;; esac
        _gf_scratch "$w" "$2" ||
            _gf_refuse "rm of $w, inside the org, may delete another session's uncommitted work while the guards are degraded." ||
            return 1
    done
    return 0
}

# `find ... -delete` unless every root is scratch (healthy too).
_gf_find() {
    local w roots=1 seen="" safe=1 del=""
    _gf_segment "$1"
    for w in $GUARD_FALLBACK_VALUE; do
        case "$w" in
            -delete) del=1 ;;
            -* | '!' | '(' | ')') roots=0 ;;
            *">"* | *"<"*) ;;
            *)
                [ "$roots" = 1 ] || continue
                seen=1
                _gf_scratch "$w" "$2" || safe=0
                ;;
        esac
    done
    # No root names the cwd.
    [ -n "$seen" ] || _gf_scratch . "$2" || safe=0
    if [ -n "$del" ] && [ "$safe" = 0 ]; then
        _gf_refuse "find -delete removes every match, including another session's files."
        return 1
    fi
    return 0
}

# Flag-shaped refusals: key | flags refused as whole words (globs) | short
# letters refused in a single-dash cluster | words skipped first | reason.
# A key matches when its words are a prefix of "<verb> <sub> <first arg>".
# Long options other than those listed pass; this is a denylist.
_GF_FLAG_TABLE="sed|--in-place --in-place=*|i||sed -i rewrites a file in place.
gsed|--in-place --in-place=*|i||sed -i rewrites a file in place.
perl||i|-M* -m* -I*|perl -i rewrites a file in place.
git reset|--hard --merge --keep|||git reset --hard/--merge/--keep discards uncommitted work in a shared checkout.
git clean|--force --force=*|f||git clean -f deletes untracked files that may be another session's.
git read-tree|--reset|um||git read-tree -u/-m/--reset overwrites working-tree files.
git switch|-f --force --discard-changes|||git switch --force discards uncommitted work.
git worktree remove|--force -f -ff|||git worktree remove --force deletes a worktree's uncommitted work."

# True when word $1 equals table entry $2, or starts with it when it ends in `*`.
# The only wildcard the table uses is that trailing star, so no glob is needed.
_gf_word_is() {
    local stem="${2%\*}"
    if [ "$stem" = "$2" ]; then
        [ "$1" = "$2" ]
    else
        [[ "$1" == "$stem"* ]]
    fi
}

# The one matcher over _GF_FLAG_TABLE. $1 is "<verb> [<sub> <first arg>]",
# $2 the segment's words. Builtins only, like the rest of this file.
_gf_flags() {
    local rest="$_GF_FLAG_TABLE" line key long short skip reason w g
    while [ -n "$rest" ]; do
        line="${rest%%$'\n'*}"
        if [ "$line" = "$rest" ]; then rest=""; else rest="${rest#*$'\n'}"; fi
        key="${line%%|*}"
        case "$1 " in "$key "*) ;; *) continue ;; esac
        IFS='|' read -r key long short skip reason <<<"$line"
        for w in $2; do
            for g in $skip; do
                _gf_word_is "$w" "$g" && continue 2
            done
            for g in $long; do
                if _gf_word_is "$w" "$g"; then
                    _gf_refuse "$reason"
                    return 1
                fi
            done
            case "$w" in
                --*) ;;
                -*)
                    if [ -n "$short" ] && [[ $w == *["$short"]* ]]; then
                        _gf_refuse "$reason"
                        return 1
                    fi
                    ;;
            esac
        done
        return 0
    done
    return 0
}
