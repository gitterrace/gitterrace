#!/usr/bin/env bash
# Sourced by selftest.sh.
echo
echo "bypasses -- every spelling a reviewer measured getting past the destructive class"
# Why: every BLOCK row below is a spelling that once got past the destructive class: a
# command named by a variable, an
# interpreter's -e/-c body holding the shell command in a nested quote, text
# piped into a shell, rm of an in-repo path and find -delete while degraded,
# git worktree remove --force, git -c alias.*, and a command run from a cwd
# OUTSIDE the org that reaches back in with -C, --git-dir or cd.
# Columns: the degraded judge, the healthy ENGINE, the cwd (REPO, OUT = /tmp,
# NONE = unknown), the command (\n is a newline, %REPO% this checkout).
# A degraded BLOCK with an engine ALLOW is legal only for the checks the
# fallback keeps to itself (rm of an in-org path, an unresolved $var command):
# the invariant below asserts that by asking the healthy judge.
FC_BYPASS=""
read -r -d '' FC_BYPASS <<'CORPUS'
BLOCK@BLOCK@REPO@true <<<x; git stash
BLOCK@BLOCK@REPO@cat <<<x; git stash
BLOCK@BLOCK@REPO@cat <<EOF\nx\nEOF\ngit stash
BLOCK@BLOCK@REPO@git commit -F - <<'MSG' && git stash\nmsg\nMSG
BLOCK@BLOCK@REPO@g=git; $g stash
BLOCK@BLOCK@REPO@g=git; ${g} stash
BLOCK@ALLOW@REPO@$GIT stash
BLOCK@ALLOW@REPO@"$BIN" stash
BLOCK@BLOCK@REPO@perl -e 'system("git stash")'
BLOCK@BLOCK@REPO@python3 -c "import os; os.system('git stash')"
BLOCK@BLOCK@REPO@ruby -e 'system "git stash"'
BLOCK@BLOCK@REPO@node --eval 'require("child_process").execSync("git stash")'
BLOCK@BLOCK@REPO@osascript -e 'do shell script "git stash"'
BLOCK@BLOCK@REPO@bash <<<'git stash'
BLOCK@BLOCK@REPO@printf 'git stash' | bash
BLOCK@BLOCK@REPO@echo 'git reset --hard' | /bin/sh
BLOCK@ALLOW@REPO@rm -f crates/core/src/lib.rs
BLOCK@ALLOW@REPO@rm crates/core/src/lib.rs
BLOCK@ALLOW@REPO@rm -rf crates
BLOCK@BLOCK@REPO@find . -delete
BLOCK@BLOCK@REPO@find crates -name '*.rs' -delete
BLOCK@BLOCK@REPO@git worktree remove --force x
BLOCK@BLOCK@REPO@git worktree remove x --force
BLOCK@BLOCK@REPO@git -c alias.s=stash s
BLOCK@BLOCK@REPO@git -c alias.x='!git stash' x
BLOCK@BLOCK@REPO@git "stash"
BLOCK@BLOCK@REPO@"git" stash
BLOCK@BLOCK@REPO@git 'stash'
BLOCK@BLOCK@REPO@git reset '--hard'
BLOCK@BLOCK@REPO@command git stash
BLOCK@BLOCK@REPO@/usr/bin/git stash
BLOCK@BLOCK@REPO@git -c x=y stash
BLOCK@BLOCK@REPO@git reset --hard
BLOCK@BLOCK@REPO@git clean -fdx
BLOCK@BLOCK@OUT@git -C %REPO% stash
BLOCK@BLOCK@OUT@git --git-dir=%REPO%/.git stash
BLOCK@BLOCK@OUT@cd %REPO% && git stash
BLOCK@BLOCK@OUT@cd %REPO%/crates && git reset --hard
BLOCK@BLOCK@NONE@git stash
ALLOW@ALLOW@REPO@git status
ALLOW@ALLOW@REPO@git diff
ALLOW@ALLOW@REPO@ls
ALLOW@ALLOW@REPO@grep 'git stash' docs
ALLOW@ALLOW@REPO@cat > $TMPDIR/x.txt <<'EOF'\ngit stash\nEOF
ALLOW@ALLOW@REPO@git commit -F - <<'MSG' -- a.txt\nnever git stash\nMSG
ALLOW@ALLOW@REPO@python3 - <<'PY'\nprint("git stash")\nPY
ALLOW@ALLOW@REPO@python3 -c 'print(1)'
ALLOW@ALLOW@REPO@perl -e 'print "hello"'
ALLOW@ALLOW@REPO@perl -ne print f
ALLOW@ALLOW@REPO@rm -f /tmp/zz 2>/dev/null
ALLOW@ALLOW@REPO@rm -f $TMPDIR/zz
ALLOW@ALLOW@REPO@find /tmp/x -delete
ALLOW@ALLOW@REPO@find crates -name '*.rs'
ALLOW@ALLOW@REPO@git worktree remove .wt/x
ALLOW@ALLOW@REPO@git worktree list
ALLOW@ALLOW@REPO@x=1; echo $x
ALLOW@ALLOW@REPO@echo hi | grep h
ALLOW@ALLOW@REPO@echo "a << b"; git status
ALLOW@ALLOW@REPO@echo "do not git stash"
ALLOW@ALLOW@OUT@git stash
ALLOW@ALLOW@OUT@git -C /tmp/other stash
ALLOW@ALLOW@OUT@cd ~/Downloads && git stash
ALLOW@ALLOW@OUT@ls %REPO%
CORPUS

# The engine's verdict on one command, asked directly with the rule set the
# gate ships (the shared destructive rule ON). BLOCK, ALLOW, or NONE (no
# verdict line).
_bp_engine() {
    local out
    out=$(jq -cn --arg d "$1" --arg c "$2" \
        '{event:"pre_tool",tool:"bash",cwd:$d,status:"ok",input:{command:$c}}' |
        env "${UNSET_ENV[@]}" GUARD_REPO="$REPO" \
            "${NEUTRAL_ENV[@]}" \
            bash "$ENGINE" 2>/dev/null)
    case "$out" in
        *"verdict: block"*) printf BLOCK ;;
        *"verdict: allow"*) printf ALLOW ;;
        *) printf NONE ;;
    esac
}
_bp_cwd() {
    case "$1" in
        REPO) printf '%s' "$REPO" ;;
        OUT) printf /tmp ;;
        *) ;;
    esac
}
_bp_cmd() {
    local c="${1//\\n/$'\n'}"
    printf '%s' "${c//%REPO%/$REPO}"
}
# The fallback judge, degraded and healthy, for every row of both corpora:
# "<degraded> <healthy>" per line, in corpus order.
bp_judged=$(
    # shellcheck source=/dev/null
    . "$GUARDS_SRC/rules/fallback/fail-closed.sh" || exit 1
    # shellcheck disable=SC2034 # read by the sourced judge
    GUARD_FALLBACK_ORG="$ORG" GUARD_TOOL=bash
    while IFS='@' read -r fb eng cw cmd; do
        [ -n "$fb" ] || continue
        # shellcheck disable=SC2034
        GUARD_CWD="$(_bp_cwd "$cw")" GUARD_CMD="$(_bp_cmd "$cmd")"
        if guard_fallback_judge; then d=ALLOW; else d=BLOCK; fi
        if GUARD_FALLBACK_HEALTHY=1 guard_fallback_judge; then h=ALLOW; else h=BLOCK; fi
        printf '%s %s\n' "$d" "$h"
    done <<<"$FC_BYPASS"
    while IFS='@' read -r want cmd; do
        [ -n "$want" ] || continue
        # shellcheck disable=SC2034
        GUARD_CWD="$REPO" GUARD_CMD="$cmd"
        if guard_fallback_judge; then d=ALLOW; else d=BLOCK; fi
        if GUARD_FALLBACK_HEALTHY=1 guard_fallback_judge; then h=ALLOW; else h=BLOCK; fi
        printf '%s %s\n' "$d" "$h"
    done <<<"$FC_CORPUS"
)
_bp_fail() {
    printf '  FAIL %s\n' "$1"
    FAIL_LOG="${FAIL_LOG}  FAIL $1
"
    FAILED=1
}
bp_n=0
bp_inv=0
{
    while IFS='@' read -r fb eng cw cmd; do
        [ -n "$fb" ] || continue
        read -r d h <&3 || d=MISSING
        bp_n=$((bp_n + 1))
        c=$(_bp_cmd "$cmd")
        label="[$cw] $(printf '%s' "$cmd" | tr '\n' '~')"
        e=$(_bp_engine "$(_bp_cwd "$cw")" "$c")
        CASES=$((CASES + 2))
        if [ "$d" = "$fb" ]; then
            printf '  ok   [%-6s] degraded judge: %s\n' "$d" "$label"
        else
            _bp_fail "expected $fb got $d -- degraded judge: $label"
        fi
        if [ "$e" = "$eng" ]; then
            printf '  ok   [%-6s] engine: %s\n' "$e" "$label"
        else
            _bp_fail "expected $eng got $e -- engine: $label"
        fi
        # The invariant: whatever the healthy judge refuses, the engine refuses.
        if [ "$h" = BLOCK ] && [ "$e" != BLOCK ]; then
            _bp_fail "the engine is laxer than the shared deny list ($e) -- invariant: $label"
        fi
        bp_inv=$((bp_inv + 1))
    done <<<"$FC_BYPASS"
    # The same invariant over the original corpus, whose cwd is the checkout.
    while IFS='@' read -r want cmd; do
        [ -n "$want" ] || continue
        read -r d h <&3 || d=MISSING
        [ "$h" = BLOCK ] || continue
        e=$(_bp_engine "$REPO" "$cmd")
        bp_inv=$((bp_inv + 1))
        [ "$e" = BLOCK ] || _bp_fail "the engine is laxer than the shared deny list ($e) -- invariant: $cmd"
    done <<<"$FC_CORPUS"
} 3<<<"$bp_judged"
CASES=$((CASES + 1))
bp_want=$(printf '%s\n' "$FC_BYPASS" | grep -c '@')
if [ "$bp_n" -eq "$bp_want" ] && [ "$bp_n" -gt 0 ]; then
    printf '  ok   [COUNT ] %s bypass rows judged, %s rows held to "healthy judge refuses => engine refuses"\n' "$bp_n" "$bp_inv"
else
    _bp_fail "judged $bp_n of $bp_want bypass rows (did fail-closed.sh load?)"
fi

# The jcode adapter's own front door. jcode sends the whole input on stdin and
# a 16 KB-capped copy in JCODE_HOOK_TOOL_INPUT; jcode-pre must read the stdin,
# refuse for an EMPTY cwd, refuse a command naming the org even from a cwd outside
# it, and check for jq.
# $1=copy $2=cwd $3=stdin ("-" for none) $4=env copy. Sets FC_RC, FC_OUT.
_jc_run() {
    if [ "$3" = - ]; then
        FC_OUT=$(JCODE_HOOK_CWD="$2" JCODE_HOOK_TOOL_NAME=bash JCODE_HOOK_TOOL_INPUT="$4" \
            env -u GUARD_SCRATCH_DIR ${JC_PATH:+PATH="$JC_PATH"} /bin/bash "$1/adapters/jcode-pre.sh" </dev/null 2>&1)
    else
        FC_OUT=$(printf '%s' "$3" | JCODE_HOOK_CWD="$2" JCODE_HOOK_TOOL_NAME=bash JCODE_HOOK_TOOL_INPUT="$4" \
            env -u GUARD_SCRATCH_DIR ${JC_PATH:+PATH="$JC_PATH"} /bin/bash "$1/adapters/jcode-pre.sh" 2>&1)
    fi
    FC_RC=$?
}
fc=$(_fc_copy none)
_jc_in() { jq -cn --arg c "$1" '{command:$c}'; }
S_STASH="st""ash"
_jc_run "$fc" "" - "$(_jc_in "$_dc_stash")"
_fc_expect 2 'Blocked' "jcode-pre with an unknown (empty) cwd refuses git stash"
_jc_run "$fc" / "$(_jc_in "git -C $fc $S_STASH")" ""
_fc_expect 2 'outside the org' "jcode-pre from outside the org refuses git -C <org path> stash"
_jc_run "$fc" / "$(_jc_in "cd $fc && $_dc_stash")" ""
_fc_expect 2 'outside the org' "jcode-pre from outside the org refuses cd <org path> && git stash"
_jc_run "$fc" / "$(_jc_in "$_dc_stash")" ""
_fc_expect 0 '' "jcode-pre from outside the org allows a stash there, silently"
_jc_run "$fc" / "$(_jc_in "ls $fc")" ""
_fc_expect 0 '' "jcode-pre from outside the org allows a read of the org"
_jc_run "$fc" "$fc" '{"command":"git '"$S_STASH"'",}' ""
_fc_expect 2 'Blocked' "jcode-pre refuses a stash in JSON jq rejects (trailing comma)"
_jc_big="$(_jc_in "echo $_dc_big; $_dc_stash")"
_jc_run "$fc" "$fc" "$_jc_big" "${_jc_big:0:16384}"
_fc_expect 2 'Blocked' "jcode-pre reads the whole input from stdin, not the 16 KB env copy"
_jc_run "$fc" "$fc" - "$(_jc_in "$_dc_stash")"
_fc_expect 2 'Blocked' "jcode-pre with no stdin judges the env copy"
_jc_run "$fc" "$fc" "$(_jc_in 'git status')" ""
_fc_expect 0 '' "jcode-pre allows git status from stdin, silently"
JC_PATH=/nonexistent _jc_run "$fc" "$fc" "$(_jc_in "$_dc_stash")" ""
_fc_expect 2 'jq is not on PATH' "jcode-pre without jq degrades to the fallback and refuses git stash"
JC_PATH=/nonexistent _jc_run "$fc" "$fc" "$(_jc_in 'git status')" ""
_fc_expect 0 "$FC_LOUD" "jcode-pre without jq allows git status, loudly"
rm -rf "$fc"

