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
# This file sits at <product>/vendor/schwarzwald/kit/guards/rules/fallback/ or
# <product>/schwarzwald/kit/guards/rules/fallback/; the org is the product's parent.
if [ -n "${GUARD_REPO:-}" ]; then
    GUARD_FALLBACK_ORG="${GUARD_REPO%/*}"
else
    _gf_back="$PWD"
    if cd "${_gf_self%/*}" 2>/dev/null; then
        _gf_here="$PWD"
    else
        _gf_here="${_gf_self%/*}"
    fi
    case "$_gf_here" in
        */schwarzwald/kit/guards/rules/fallback) _gf_here="${_gf_here%/schwarzwald/kit/guards/rules/fallback}" ;;
        *) _gf_here="${_gf_here%/*/*/*/*/*}" ;;
    esac
    _gf_here="${_gf_here%/vendor}"
    GUARD_FALLBACK_ORG="${_gf_here%/*}"
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
    # Too big to judge before the harness gives up: refused here, before the engine spends
    # its deadline on it. A post_tool call has already run.
    case "${GUARD_TOOL:-}" in bash | Bash | BASH) ;; *) return 0 ;; esac
    [ "${GUARD_EVENT:-pre_tool}" = pre_tool ] || return 0
    if guard_fallback_too_big "${GUARD_CMD:-}"; then
        GUARD_FALLBACK_SETTLED=1
        printf 'Blocked: %s\n' "$GUARD_FALLBACK_REASON" >&2
        exit "$GUARD_FALLBACK_BLOCK_RC"
    fi
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
    echo "         Check: bash -n ${GUARD_ENGINE:-${_gf_self%/rules/fallback/*}/engine.sh}" >&2
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
# Where jq would read it otherwise (a `\u` escape, the key twice: jq takes the
# last) GUARD_FALLBACK_JSON_DOUBT is set, and the judge refuses.
GUARD_FALLBACK_JSON_DOUBT=""
guard_fallback_json_string() {
    GUARD_FALLBACK_VALUE=""
    local LC_ALL=C
    # Past the size the judge refuses anyway, the payload is not parsed: the value is a refusal.
    if [ "${#1}" -gt $((GUARD_FALLBACK_MAX_BYTES * 2)) ]; then
        GUARD_FALLBACK_JSON_DOUBT="the payload is ${#1} bytes, over what a guard can read in time"
        return 0
    fi
    # The key's opening by a short regex, the value's end by splitting on `"`: a regex over
    # the whole value is far slower than linear in bash (4 s at 20 KB).
    # shellcheck disable=SC1003 # bs is one literal backslash
    local re='"'"$2"'"[[:space:]]*:[[:space:]]*"' m pre rest v="" p bs='\' IFS had_noglob=""
    local -a parts=()
    [[ ${1:-} =~ $re ]] || return 0
    m="${BASH_REMATCH[0]}"
    pre="${1%%"$m"*}"
    rest="${1:$((${#pre} + ${#m}))}"
    case "$-" in *f*) had_noglob=1 ;; esac
    set -f
    IFS='"'
    # shellcheck disable=SC2206 # splitting on quotes is the scan
    parts=(${rest}x)
    [ -n "$had_noglob" ] || set +f
    IFS=$' \t\n'
    # A piece ending in an odd run of backslashes ends in an escaped quote: the value goes on.
    # The run is counted from the end (a `##*` pattern over a long piece is quadratic), and the
    # pieces are joined once at the end (appending to one string per piece is quadratic too).
    local k c used=0 n="${#parts[@]}"
    while [ "$used" -lt "$n" ]; do
        p="${parts[$used]}"
        used=$((used + 1))
        k=0
        while [ "$k" -lt "${#p}" ]; do
            c="${p:$((${#p} - k - 1)):1}"
            [ "$c" = "$bs" ] || break
            k=$((k + 1))
        done
        [ $((k % 2)) -eq 1 ] || break
    done
    IFS='"'
    v="${parts[*]:0:$used}"
    IFS=$' \t\n'
    # No closing quote (a truncated payload): the scan's own trailing `x` goes.
    [ "$used" -lt "$n" ] || v="${v%x}"
    [[ ${1:$((${#pre} + ${#m} + ${#v}))} =~ $re ]] && GUARD_FALLBACK_JSON_DOUBT="the key $2 occurs twice"
    # One pass over the backslash-separated pieces: `${v//\\\"/\"}` per escape kind is
    # quadratic in bash on a value with many escapes (88 s for 6000 `\"`).
    local nl=$'\n' piece first acc=() esc=""
    case "$v" in
        *"$bs"*) ;;
        *)
            GUARD_FALLBACK_VALUE="$v"
            return 0
            ;;
    esac
    case "$-" in *f*) had_noglob=1 ;; *) had_noglob="" ;; esac
    set -f
    IFS="$bs"
    # shellcheck disable=SC2206 # splitting on backslashes is the scan
    parts=(${v}x)
    [ -n "$had_noglob" ] || set +f
    IFS=$' \t\n'
    first=1
    for piece in "${parts[@]}"; do
        if [ -n "$first" ]; then
            acc[${#acc[@]}]="$piece"
            first=""
            continue
        fi
        if [ -n "$esc" ]; then
            # The previous piece was empty: this backslash was escaped by the one before it.
            esc=""
            acc[${#acc[@]}]="$bs$piece"
            continue
        fi
        case "$piece" in
            "") esc=1 ;;
            n* | r*) acc[${#acc[@]}]="$nl${piece:1}" ;;
            t*) acc[${#acc[@]}]=" ${piece:1}" ;;
            u*)
                GUARD_FALLBACK_JSON_DOUBT="the value of $2 holds a \\u escape"
                acc[${#acc[@]}]="$piece"
                ;;
            *) acc[${#acc[@]}]="$piece" ;;
        esac
    done
    IFS=""
    v="${acc[*]}"
    IFS=$' \t\n'
    GUARD_FALLBACK_VALUE="${v%x}"
}

# Drop quoted prose, keep quoted code, into GUARD_FALLBACK_VALUE. A dropped run
# leaves a placeholder so `git -C "a dir" stash` keeps its shape; a kept run is
# re-read so nested quotes count. Code: after -c/eval, a double-quoted `$(`, a
# command piped into a shell, an interpreter's -c/-e body (whose own string
# literals are kept too: `os.system('git stash')`).
_gf_unquote() {
    local rest="$1" out="" pre q body tail sq="'" dq='"' near
    while [ -n "$rest" ]; do
        _gf_late && break
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
        # The context is the end of the output only: a suffix pattern over all of it is
        # quadratic in bash, and the words that make a body code sit just before it.
        near="${out: -256}"
        [ "${#out}" -ge 256 ] || near="$out"
        _gf_code_kind "$near"
        if [ -n "${GUARD_FALLBACK_ALLCODE:-}" ] || [ "$GUARD_FALLBACK_VALUE" = interp ]; then
            body="${body//"$sq"/ }"
            out="$out ; ${body//"$dq"/ } ; "
            continue
        fi
        tail="${near%"${near##*[! ]}"}"
        case "$tail" in
            *-c | *eval)
                rest=" ; $body ; $rest"
                continue
                ;;
        esac
        # `alias gs='git stash'` defines a command: its value is code.
        case "$tail" in
            *alias\ [A-Za-z_]*=)
                rest=" ; $body ; $rest"
                continue
                ;;
        esac
        if [ "$q" = '"' ]; then
            # shellcheck disable=SC2016 # the literal text `$(` and a backtick
            case "$body" in
                *'$('* | *'`'*)
                    rest=" $body $rest"
                    continue
                    ;;
            esac
        fi
        # A quoted run with no whitespace joins the word beside it, as the shell
        # does (`"git" stash`, `--ha""rd`, `st''ash`); an empty one adds nothing.
        # One with whitespace is prose: a placeholder word of its own.
        case "$body" in
            *[[:space:]]*) out="$out Q " ;;
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
        [ "${#w}" -le 256 ] || continue
        case "${w##*/}" in
            perl* | python* | ruby* | node | nodejs | osascript | deno | bun | php | lua* | Rscript | swift)
                GUARD_FALLBACK_VALUE=interp
                return 0
                ;;
        esac
    done
    return 0
}

# What the command in words $1 does with its stdin, into GUARD_FALLBACK_VALUE:
# `all` for a shell (anywhere in the words), `interp` for an interpreter given
# no script or `-`, empty for anything else (`python3 x.py` reads data).
_gf_stdin_kind() {
    local w st="" first="${2:-}"
    GUARD_FALLBACK_VALUE=""
    for w in $1; do
        [ "${#w}" -le 256 ] || w=x
        # $2 set: only the command word counts (`| grep sh` is not a shell).
        if [ -n "$first" ]; then
            case "$w" in
                env | exec | command | sudo | nohup | time | nice | -* | [A-Za-z_]*=*) continue ;;
            esac
            first=""
            case "${w##*/}" in
                bash | sh | zsh | dash | ksh | eval | source | . | perl* | python* | ruby* | node | nodejs | osascript | deno | bun | php | lua* | Rscript | swift) ;;
                *) return 0 ;;
            esac
        fi
        case "${w##*/}" in
            bash | sh | zsh | dash | ksh | eval | source | .)
                GUARD_FALLBACK_VALUE=all
                return 0
                ;;
        esac
        case "$st" in
            "")
                case "${w##*/}" in
                    perl* | python* | ruby* | node | nodejs | osascript | deno | bun | php | lua* | Rscript | swift) st=interp ;;
                esac
                ;;
            interp)
                case "$w" in
                    -) break ;;
                    -* | [0-9]\>* | \>* | \<*) ;;
                    *) st=script ;;
                esac
                ;;
        esac
    done
    [ "$st" != interp ] || GUARD_FALLBACK_VALUE=interp
    return 0
}

# Heredoc bodies out, into GUARD_FALLBACK_VALUE. Cutting at the first `<<` hid
# `cat <<<x; git stash`. `<<<` drops only the operator; `<<WORD` drops the body
# but keeps the rest of its line and everything after the terminator; a body
# fed to a shell, or an unquoted body's `$(`/backtick lines, is kept as code;
# a `<<` inside quotes is no operator. Linear: the lines are split once, the
# output is collected in an array, and the quote state is carried forward
# rather than recounted over everything before each `<<`.
_gf_strip_heredocs() {
    local nl=$'\n' tab=$'\t' bt='`' had_noglob=""
    local line cur acc pre after c d quoted keep seg t li=0
    local -a pd=() pk=() lines=() o=()
    local np=0 ph=0
    # Quote state of everything emitted so far: single and double parity, and whether the text
    # since the last double quote opened a `$(` or backtick.
    _GF_SQP=0 _GF_DQP=0 _GF_DQSUB=0
    case "$-" in *f*) had_noglob=1 ;; esac
    set -f
    local IFS=$'\n'
    # shellcheck disable=SC2206 # one element per line; globbing is off
    lines=(${1:-})
    IFS=$' \t\n'
    [ -n "$had_noglob" ] || set +f
    while [ "$li" -lt "${#lines[@]}" ]; do
        _gf_late && break
        line="${lines[$li]}"
        li=$((li + 1))
        if [ "$ph" -lt "$np" ]; then
            t="${line#"${line%%[!"$tab"]*}"}"
            if [ "$t" = "${pd[$ph]}" ]; then
                ph=$((ph + 1))
            else
                case "${pk[$ph]}" in
                    all) o[${#o[@]}]="$line" ;;
                    subst)
                        case "$line" in
                            *"\$("* | *"$bt"*) o[${#o[@]}]="$line" ;;
                        esac
                        ;;
                    interp)
                        # A line that runs a command keeps its string literals as words:
                        # `os.system("git stash")`, `run(["git", "stash"])`. A `print`
                        # of the same text is data.
                        case "$line" in
                            *system* | *exec* | *popen* | *Popen* | *spawn* | *subprocess* | *shell* | *"$bt"*)
                                t="${line//\'/ }" && t="${t//\"/ }" && t="${t//,/ }"
                                t="${t//\[/ }" && t="${t//]/ }"
                                o[${#o[@]}]="$t"
                                ;;
                        esac
                        ;;
                esac
            fi
        else
            cur="$line"
            acc=""
            local sqp0="$_GF_SQP" dqp0="$_GF_DQP" dqs0="$_GF_DQSUB"
            while [[ $cur == *'<<'* ]]; do
                pre="${cur%%<<*}"
                after="${cur#*<<}"
                _GF_SQP="$sqp0" _GF_DQP="$dqp0" _GF_DQSUB="$dqs0"
                _gf_qstate "$acc$pre"
                # Inside double quotes it is an operator only after a `$(`:
                # `"$(cat <<'EOF' ... EOF)"` is the usual message body.
                quoted=0
                if [ "$_GF_SQP" = 1 ]; then
                    quoted=1
                elif [ "$_GF_DQP" = 1 ] && [ "$_GF_DQSUB" = 0 ]; then
                    quoted=1
                fi
                if [ "$quoted" = 1 ]; then
                    acc="$acc$pre<<"
                    cur="$after"
                    continue
                fi
                seg="$acc$pre"
                seg="${seg##*[;&|(\`]}"
                _gf_stdin_kind "$seg"
                keep="$GUARD_FALLBACK_VALUE"
                if [ "${after:0:1}" = "<" ]; then
                    # The word stays; fed to a shell, `eval` keeps it as code.
                    if [ "$keep" = all ]; then acc="$acc$pre eval "; else acc="$acc$pre "; fi
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
                # Piped on into a shell or an interpreter: `cat <<EOF | bash`.
                if [ -z "$keep" ]; then
                    case "$after" in
                        *'||'*) ;;
                        *'|'*)
                            t="${after#*|}"
                            _gf_stdin_kind "${t%%[;&|]*}" first
                            keep="$GUARD_FALLBACK_VALUE"
                            ;;
                    esac
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
            _GF_SQP="$sqp0" _GF_DQP="$dqp0" _GF_DQSUB="$dqs0"
            _gf_qstate "$acc$cur"
            o[${#o[@]}]="$acc$cur"
        fi
    done
    if [ "${#o[@]}" -gt 0 ]; then
        IFS=$'\n'
        GUARD_FALLBACK_VALUE="${o[*]}$nl"
        IFS=$' \t\n'
    else
        GUARD_FALLBACK_VALUE=""
    fi
}

# Carry the quote state (_GF_SQP, _GF_DQP, _GF_DQSUB) across the text $1. Naive on
# purpose, as the guard always was: every quote character counts. Counted by word
# splitting, which is linear; `${s//[^q]/}` is not in bash.
_gf_qstate() {
    local s="$1" IFS had_noglob="" n t bt='`'
    local -a parts=()
    case "$s" in *[\"\']*) ;; *)
        case "$s" in *"\$("* | *"$bt"*) _GF_DQSUB=1 ;; esac
        return 0
        ;;
    esac
    case "$-" in *f*) had_noglob=1 ;; esac
    set -f
    IFS="'"
    # shellcheck disable=SC2206 # splitting is the count
    parts=(x${s}x)
    n=$((${#parts[@]} - 1))
    _GF_SQP=$(((_GF_SQP + n) % 2))
    IFS='"'
    # shellcheck disable=SC2206
    parts=(x${s}x)
    n=$((${#parts[@]} - 1))
    [ -n "$had_noglob" ] || set +f
    _GF_DQP=$(((_GF_DQP + n) % 2))
    if [ "$n" -gt 0 ]; then
        t="${parts[${#parts[@]} - 1]}"
        _GF_DQSUB=0
    else
        t="$s"
    fi
    case "$t" in *"\$("* | *"$bt"*) _GF_DQSUB=1 ;; esac
    return 0
}

# Returns 0 to allow, 1 to refuse with GUARD_FALLBACK_REASON set. Globbing is
# off throughout, so a `*` in the command is a word, not this cwd's files.
# Byte-wise (LC_ALL=C): a multibyte locale made every substring copy slower.
guard_fallback_judge() {
    local had_noglob="" rc LC_ALL=C
    case "$-" in *f*) had_noglob=1 ;; esac
    set -f
    _GF_DEADLINE=$((SECONDS + 2))
    _GF_LATE=""
    _gf_judge
    rc=$?
    if [ "$rc" = 0 ] && [ -n "$_GF_LATE" ]; then
        _gf_refuse "the command took too long to judge, so it is refused rather than allowed unjudged."
        rc=1
    fi
    _GF_DEADLINE=""
    [ -n "$had_noglob" ] || set +f
    return "$rc"
}

# The parser below copies the rest of the command once per quote and per line,
# so its cost grows with size times count. Past these bounds a command is
# refused before any parsing: padding must buy a refusal, never a timeout
# (a harness that kills a slow hook ALLOWS the call). Each test is linear.
GUARD_FALLBACK_MAX_BYTES=32768
GUARD_FALLBACK_MAX_QUOTES=1000
GUARD_FALLBACK_MAX_LINES=2000
# Is $1 too big to judge in time? Sets GUARD_FALLBACK_REASON and returns 0 when so.
guard_fallback_too_big() {
    local LC_ALL=C s="${1:-}" IFS had_noglob=""
    local -a parts=()
    if [ "${#s}" -gt "$GUARD_FALLBACK_MAX_BYTES" ]; then
        GUARD_FALLBACK_REASON="the command is ${#s} bytes, over the $GUARD_FALLBACK_MAX_BYTES a guard can judge in time; put it in a script file and run that."
        return 0
    fi
    # Counted by word splitting, which is linear: `${s//[^q]/}` is not in bash.
    case "$-" in *f*) had_noglob=1 ;; esac
    set -f
    IFS="\"'"
    # shellcheck disable=SC2206 # splitting is the count
    parts=(x${s}x)
    local quotes=$((${#parts[@]} - 1))
    IFS=$'\n'
    # shellcheck disable=SC2206
    parts=(x${s}x)
    local line_count=$((${#parts[@]} - 1))
    [ -n "$had_noglob" ] || set +f
    if [ "$quotes" -gt "$GUARD_FALLBACK_MAX_QUOTES" ]; then
        GUARD_FALLBACK_REASON="the command holds $quotes quote characters, over the $GUARD_FALLBACK_MAX_QUOTES a guard can judge in time; write the command to a file with the write tool and run \`bash <file>\`."
        return 0
    fi
    if [ "$line_count" -gt "$GUARD_FALLBACK_MAX_LINES" ]; then
        GUARD_FALLBACK_REASON="the command has $line_count lines, over the $GUARD_FALLBACK_MAX_LINES a guard can judge in time; put it in a script file and run that."
        return 0
    fi
    return 1
}

# Past the judge's deadline? Marks it, so the judge refuses.
_gf_late() {
    [ -n "${_GF_DEADLINE:-}" ] && [ "$SECONDS" -ge "$_GF_DEADLINE" ] || return 1
    _GF_LATE=1
    return 0
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
    if [ -n "$GUARD_FALLBACK_JSON_DOUBT" ] && [ -z "${GUARD_FALLBACK_HEALTHY:-}" ]; then
        _gf_refuse "the payload cannot be read the way jq would read it ($GUARD_FALLBACK_JSON_DOUBT)."
        return 1
    fi
    guard_fallback_too_big "${GUARD_CMD:-}" && return 1
    local org="$GUARD_FALLBACK_ORG" cwd="${GUARD_CWD:-}" inorg=1
    # A cwd reached through a symlink is where the symlink points, and the org
    # is compared in the same physical terms (callees see this one).
    _gf_physical "$org"
    local GUARD_FALLBACK_ORG="$GUARD_FALLBACK_VALUE"
    if [ -n "$cwd" ]; then
        _gf_physical "$cwd"
        cwd="$GUARD_FALLBACK_VALUE"
    fi
    # Outside the org, judged only if the command can reach back in
    # (`git -C <repo> stash` from /tmp, `cd <a symlink into it>`). Cheap test
    # first. An UNKNOWN cwd is judged.
    if [ -n "$cwd" ]; then
        case "$cwd" in
            "$org" | "$org"/* | "$GUARD_FALLBACK_ORG" | "$GUARD_FALLBACK_ORG"/*) ;;
            *)
                case "${GUARD_CMD:-}" in
                    *"$org"* | *'~'* | *HOME* | *..* | *cd* | *pushd* | *-C* | *--git-dir* | *--work-tree*) inorg=0 ;;
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
    local pipesh='\|[[:space:]]*(([^[:space:]]*/)?(env|exec|command|sudo|nohup)[[:space:]]+)*([^[:space:]]*/)?(bash|zsh|dash|ksh|sh)([[:space:]]|$)'
    [[ $s =~ $pipesh ]] && allcode=1
    # `${NAME}` becomes `$NAME`, so its braces are not read as a group below.
    local brace='\$\{([A-Za-z_][A-Za-z0-9_]*)\}'
    while [[ $s =~ $brace ]]; do
        s="${s/"${BASH_REMATCH[0]}"/\$${BASH_REMATCH[1]}}"
    done
    # `git${IFS}stash` is two words; `$'git'` is `'git'`.
    local ifs="\$IFS" ansi="\$'" sq1="'"
    s="${s//"$ifs"/ }"
    s="${s//"$ansi"/$sq1}"
    GUARD_FALLBACK_ALLCODE="$allcode" _gf_unquote "$s"
    s="$GUARD_FALLBACK_VALUE"
    # A substitution's output is a word nobody can read here: it stays one word,
    # `$_GF_SUBST`, and its body is judged as a command of its own at the end.
    local subst='\$\(([^()]*)\)' tick='`([^`]*)`'
    while [[ $s =~ $subst ]] || [[ $s =~ $tick ]]; do
        s="${s/"${BASH_REMATCH[0]}"/\$_GF_SUBST} ; ${BASH_REMATCH[1]}"
    done
    # Words, with a `;` word for every separator. One split on the separators, then one on
    # blanks per piece: `${s//;/ ; }` per separator is quadratic in bash on a long command.
    local IFS=$';&|(){}`\n' p
    local -a W=() parts=() ws=()
    # shellcheck disable=SC2206 # splitting is the point; globbing is off
    parts=(x${s}x)
    parts[0]="${parts[0]#x}"
    p=$((${#parts[@]} - 1))
    parts[p]="${parts[$p]%x}"
    IFS=$' \t'
    for p in "${parts[@]}"; do
        # shellcheck disable=SC2206
        ws=($p)
        # `${ws[@]}` of an empty array is unbound under `set -u` in bash 3.2.
        [ "${#ws[@]}" -eq 0 ] || W+=("${ws[@]}")
        W+=(";")
    done
    if [ "$inorg" = 0 ]; then
        _gf_reaches_org || return 0
        # The cwd was left, and containment does not run for a foreign cwd.
        cwd=""
    fi
    local healthy="${GUARD_FALLBACK_HEALTHY:-}"
    [ "$inorg" = 1 ] || healthy=""
    _gf_expand_words
    local n=${#W[@]} i=0 w cp=1
    _GF_XARGS=""
    while [ "$i" -lt "$n" ]; do
        _gf_late && return 0
        w="${W[$i]:-}"
        case "$w" in
            GIT_CONFIG_KEY_*=[Aa][Ll][Ii][Aa][Ss].*)
                _gf_refuse "GIT_CONFIG_KEY_*=alias.* defines a git subcommand this guard cannot see through."
                return 1
                ;;
        esac
        if [ "$w" = ";" ]; then
            cp=1
            _GF_XARGS=""
            i=$((i + 1))
            continue
        fi
        if [ "$cp" = 1 ]; then
            case "$w" in
                [A-Za-z_]*=* | sudo | env | command | builtin | exec | nohup | time | nice | alias | -* | [0-9]*)
                    i=$((i + 1))
                    continue
                    ;;
                xargs)
                    # The arguments come from stdin: `echo stash | xargs git`.
                    _GF_XARGS=1
                    i=$((i + 1))
                    continue
                    ;;
                '$_GF_SUBST')
                    # `$(which git) stash`: an unknown command is judged as git would be.
                    if [ -n "$healthy" ]; then
                        _gf_git "$((i + 1))" || return 1
                        cp=0
                        i=$((i + 1))
                        continue
                    fi
                    _gf_refuse "a command named by a substitution ($w) cannot be judged while the guards are degraded."
                    return 1
                    ;;
                '$'[A-Za-z_]*)
                    if [ -z "$healthy" ]; then
                        _gf_refuse "a command named by a variable ($w) cannot be judged while the guards are degraded."
                        return 1
                    fi
                    ;;
            esac
            cp=0
        fi
        # Only a command NAME is stripped of its directory: `${w##*/}` is
        # quadratic in bash, and a long word (padding) is no command name.
        [ "${#w}" -le 256 ] || {
            i=$((i + 1))
            continue
        }
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

# Resolve W in place, in order: a `$NAME` word assigned earlier in the command
# (`x=stash; git $x`, `g=git; $g stash`) and an alias in command position
# (`alias g=git; g stash`) become their value.
_gf_expand_words() {
    local i=0 n=${#W[@]} w cp=1 al="" k
    local -a an=() av=() aln=() alv=()
    while [ "$i" -lt "$n" ]; do
        w="${W[$i]}"
        case "$w" in
            '$'[A-Za-z_]*)
                k=${#an[@]}
                while [ "$k" -gt 0 ]; do
                    k=$((k - 1))
                    if [ "\$${an[$k]}" = "$w" ]; then
                        w="${av[$k]}"
                        W[i]="$w"
                        break
                    fi
                done
                ;;
        esac
        if [ "$w" = ";" ]; then
            cp=1
            al=""
        elif [ "$cp" = 1 ]; then
            case "$w" in
                alias) al=1 ;;
                [A-Za-z_]*=*)
                    if [ -n "$al" ]; then
                        aln[${#aln[@]}]="${w%%=*}"
                        alv[${#alv[@]}]="${w#*=}"
                    else
                        an[${#an[@]}]="${w%%=*}"
                        av[${#av[@]}]="${w#*=}"
                    fi
                    ;;
                sudo | env | command | builtin | exec | nohup | time | nice | -*) ;;
                *)
                    k=${#aln[@]}
                    while [ "$k" -gt 0 ]; do
                        k=$((k - 1))
                        if [ "${aln[$k]}" = "$w" ]; then
                            W[i]="${alv[$k]}"
                            break
                        fi
                    done
                    cp=0
                    ;;
            esac
        fi
        i=$((i + 1))
    done
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
            _gf_physical "$GUARD_FALLBACK_VALUE"
            case "$GUARD_FALLBACK_VALUE" in
                "$org" | "$org"/*) return 0 ;;
            esac
        fi
        prev=""
        [ "${#w}" -gt 256 ] || prev="${w##*/}"
    done
    return 1
}

# The physical path of absolute $1 into GUARD_FALLBACK_VALUE: its deepest
# existing directory resolved with `cd -P` (a builtin, no fork), the rest
# appended. A symlink outside the org can point into it.
_gf_physical() {
    local p="$1" tail="" back="$PWD"
    GUARD_FALLBACK_VALUE="$p"
    case "$p" in /*) ;; *) return 0 ;; esac
    while [ -n "$p" ] && ! [ -d "$p" ]; do
        tail="/${p##*/}$tail"
        p="${p%/*}"
    done
    [ -n "$p" ] || p=/
    if CDPATH='' cd -P -- "$p" 2>/dev/null; then
        GUARD_FALLBACK_VALUE="${PWD%/}$tail"
        CDPATH='' cd -- "$back" 2>/dev/null || :
    fi
    return 0
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
            -c[Aa][Ll][Ii][Aa][Ss].* | --config-env=[Aa][Ll][Ii][Aa][Ss].*)
                _gf_refuse "git -c alias.* defines a subcommand this guard cannot see through."
                return 1
                ;;
            --config-env)
                case "${W[$((j + 1))]:-}" in
                    [Aa][Ll][Ii][Aa][Ss].*)
                        _gf_refuse "git --config-env alias.* defines a subcommand this guard cannot see through."
                        return 1
                        ;;
                esac
                j=$((j + 2))
                ;;
            -C | --git-dir | --work-tree | --namespace | --exec-path) j=$((j + 2)) ;;
            -*) j=$((j + 1)) ;;
            *) break ;;
        esac
    done
    sub="${W[$j]:-}"
    _gf_segment "$((j + 1))"
    args="$GUARD_FALLBACK_VALUE"
    # A subcommand nobody can read here (`git $(echo stash)`, `git $x`, `xargs git`)
    # could be any of the class.
    case "$sub" in
        '$'* | *'$_GF_SUBST'*)
            _gf_refuse "git $sub names its subcommand through an expansion this guard cannot see through."
            return 1
            ;;
        ";" | "")
            if [ -n "${_GF_XARGS:-}" ]; then
                _gf_refuse "xargs git takes its subcommand from stdin, which this guard cannot see."
                return 1
            fi
            ;;
    esac
    case "$sub" in
        config)
            # Writing an alias defines a subcommand for every later call; reading one passes.
            case "$args" in
                *" "[Aa][Ll][Ii][Aa][Ss].*)
                    case "$args" in
                        *" --get"* | *" get "* | *" --list "* | *" -l "* | *" list "*) ;;
                        *) _gf_refuse "git config alias.* defines a subcommand this guard cannot see through." ;;
                    esac
                    ;;
            esac
            ;;
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
