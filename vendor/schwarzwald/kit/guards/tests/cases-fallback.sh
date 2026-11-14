#!/usr/bin/env bash
# Sourced by selftest.sh.
echo
echo "fail-closed -- a guard that cannot answer still refuses the destructive class"
# Why: when the engine cannot answer, rules/fallback/fail-closed.sh refuses the
# destructive class and allows the rest. Each case breaks a COPY of the guards one
# way and asks an adapter.
# Copied from the tree under test (GUARDS_SRC), so a red-case probe reaches it.
FC_OUT=""
FC_RC=0

# $1=mutation: none, norules, noengine, badengine (a parse error, so no
# verdict line), slowengine (hangs past the deadline). Prints the copy's path.
_fc_copy() {
    local d
    # Under .runs/, not $TMPDIR: the guards treat scratch as deletable, and a fixture whose "org" is
    # scratch cannot show the org boundary. Cleaned with the fixture.
    d=$(_guards_install "$(mktemp -d "$_SCRATCH_PREFIX.fc.XXXXXX")")
    case "$1" in
        norules) rm -f "$d"/rules/*.sh ;;
        noengine) rm -f "$d/engine.sh" ;;
        badengine) { printf 'if then\n' && cat "$GUARDS_SRC/engine.sh"; } >"$d/engine.sh" ;;
        slowengine) { printf '#!/usr/bin/env bash\nexec sleep 30\n' && cat "$GUARDS_SRC/engine.sh"; } >"$d/engine.sh" ;;
    esac
    printf '%s' "$d"
}

# $1=adapter (cli, jcode, or <name>-starved: no fork possible)
# $2=copy  $3=command  [$4=adapter path relative to the copy]. Sets FC_RC, FC_OUT.
_fc_run() {
    local a="$1" d="$2" c="$3" rel="${4:-}" lim=""
    # The child limits ITSELF, then execs the adapter: $1 empty means no limit.
    # shellcheck disable=SC2016 # expanded by that child, not here
    local starve='[ -z "$1" ] || ulimit -u 1; shift; exec bash "$@"'
    case "$a" in *-starved) lim=1 a="${a%-starved}" ;; esac
    local input
    # Through stdin, not --arg: Linux caps one argument at 128 KB, so a 300 KB case made jq
    # fail, the adapter got no input and allowed it.
    input=$(printf '%s' "$c" | jq -Rsc '{command:.}')
    case "$a" in
        cli)
            FC_OUT=$(env -u GUARD_SCRATCH_DIR bash -c "$starve" \
                _ "$lim" "$d/${rel:-adapters/cli.sh}" --cwd "$d" --command "$c" 2>&1)
            FC_RC=$?
            ;;
        jcode)
            # The input on stdin AND in the 16 KB-capped env copy, as jcode sends it.
            FC_OUT=$(printf '%s' "$input" | JCODE_HOOK_CWD="$d" JCODE_HOOK_TOOL_NAME=bash JCODE_HOOK_TOOL_INPUT="${input:0:16384}" \
                env -u GUARD_SCRATCH_DIR bash -c "$starve" \
                _ "$lim" "$d/${rel:-adapters/jcode-pre.sh}" 2>&1)
            FC_RC=$?
            ;;
    esac
}

# $1=expected rc  $2=regex the output must hold, or empty for NO output  $3=label
_fc_expect() {
    local said=1
    if [ -n "$2" ]; then
        grep -q -- "$2" <<<"$FC_OUT" || said=0
    else
        [ -z "$FC_OUT" ] || said=0
    fi
    CASES=$((CASES + 1))
    if [ "$FC_RC" -eq "$1" ] && [ "$said" = 1 ]; then
        printf '  ok   [rc=%s  ] %s\n' "$FC_RC" "$3"
    else
        printf '  FAIL expected rc=%s and /%s/, got rc=%s -- %s\n' "$1" "$2" "$FC_RC" "$3"
        printf '%s\n' "$FC_OUT" | head -4 | sed 's/^/       out: /'
        FAIL_LOG="${FAIL_LOG}  FAIL expected rc=$1 got rc=$FC_RC -- $3
"
        FAILED=1
    fi
}
FC_REFUSED='fail-closed fallback'
FC_LOUD='NOT running'

# The positive control: a HEALTHY copy is judged by the engine, not the
# fallback, and the fallback stays silent. Without it every refusal below could
# be the engine's own.
fc=$(_fc_copy none)
export GUARD_ADAPTER_DEADLINE=60
_fc_run cli "$fc" 'git stash'
_fc_expect 1 'Blocked: git stash in a shared checkout' "control: a healthy engine refuses git stash itself"
case "$FC_OUT" in *"$FC_REFUSED"* | *"$FC_LOUD"*)
    FAILED=1 && printf '  FAIL the fallback spoke on a HEALTHY engine\n'
    ;;
esac
_fc_run cli "$fc" 'git status'
_fc_expect 0 '' "control: a healthy engine allows git status, and nothing warns"
unset GUARD_ADAPTER_DEADLINE
# A failed fork exits 128, which every harness reads as allow. The EXIT trap
# is what turns that into a judgement.
_fc_run jcode-starved "$fc" 'git stash'
_fc_expect 2 "$FC_REFUSED" "fork-starved jcode-pre refuses git stash (exit 2)"
_fc_run cli-starved "$fc" 'git reset --hard'
_fc_expect 1 "$FC_REFUSED" "fork-starved cli refuses git reset --hard (exit 1)"
_fc_run jcode-starved "$fc" 'git status'
_fc_expect 0 "$FC_LOUD" "fork-starved jcode-pre allows git status, loudly"
rm -rf "$fc"

fc=$(_fc_copy norules)
_fc_run cli "$fc" 'git stash'
_fc_expect 1 "$FC_REFUSED" "no rules: cli refuses git stash"
_fc_run cli "$fc" 'rm -rf crates/core'
_fc_expect 1 "$FC_REFUSED" "no rules: cli refuses rm -rf"
_fc_run cli "$fc" 'git status'
_fc_expect 0 "$FC_LOUD" "no rules: cli allows git status, loudly"
_fc_run jcode "$fc" 'git stash'
_fc_expect 2 "$FC_REFUSED" "no rules: jcode-pre refuses git stash"
# Paths are derived by stripping components, so a `.` or `..` in the adapter's
# own path once moved the org and the fallback file out from under it.
_fc_run jcode "$fc" 'git stash' adapters/../adapters/jcode-pre.sh
_fc_expect 2 "$FC_REFUSED" "jcode-pre via adapters/../adapters still refuses git stash"
_fc_run cli "$fc" 'git stash' adapters/../adapters/./cli.sh
_fc_expect 1 "$FC_REFUSED" "cli via adapters/../adapters/./ still refuses git stash"
rm -rf "$fc"

fc=$(_fc_copy noengine)
_fc_run jcode "$fc" 'git checkout -- Cargo.toml'
_fc_expect 2 "$FC_REFUSED" "engine missing: jcode-pre refuses git checkout --"
rm -rf "$fc"

fc=$(_fc_copy badengine)
_fc_run cli "$fc" 'git clean -fdx'
_fc_expect 1 "$FC_REFUSED" "engine does not parse: cli refuses git clean -f"
_fc_run cli "$fc" 'git diff HEAD'
_fc_expect 0 "$FC_LOUD" "engine does not parse: cli allows git diff, loudly"
rm -rf "$fc"

# A hung engine is killed at the deadline and the fallback answers, inside
# jcode's own 5 s hook timeout (which would ALLOW the call).
fc=$(_fc_copy slowengine)
fc_t0=$SECONDS
GUARD_ADAPTER_DEADLINE=1 _fc_run jcode "$fc" 'git stash'
_fc_expect 2 'deadline' "hung engine: jcode-pre refuses git stash at the deadline"
_assert yes "$([ $((SECONDS - fc_t0)) -lt 30 ] && echo yes || echo no)" \
    "hung engine: answered in $((SECONDS - fc_t0)) s, before its 30 s sleep ended"
rm -rf "$fc"

# The fallback's own judgement, asked directly. The class is a denylist; the
# ALLOW rows are the read-only and prose cases a fail-closed guard must not
# refuse, or it is routed around.
FC_CORPUS=""
read -r -d '' FC_CORPUS <<'CORPUS'
BLOCK@git stash
BLOCK@git stash push -m x
BLOCK@git stash pop
ALLOW@git stash list
ALLOW@git stash show -p
BLOCK@git -C ../sibling stash
BLOCK@git -C "a dir" stash
BLOCK@cd x && git stash
BLOCK@echo hi; git stash
BLOCK@FOO=1 git stash
BLOCK@/usr/bin/git stash
BLOCK@bash -c 'git stash'
BLOCK@eval 'git stash'
BLOCK@echo "$(git stash)"
BLOCK@(git stash)
BLOCK@git reset --hard HEAD~1
ALLOW@git reset --soft HEAD~1
ALLOW@git reset HEAD -- a.txt
BLOCK@git clean -fdx
ALLOW@git clean -n
BLOCK@git checkout -- Cargo.toml
BLOCK@git checkout .
BLOCK@git checkout Cargo.toml
ALLOW@git checkout main
ALLOW@git checkout -b feat
BLOCK@git restore Cargo.toml
ALLOW@git restore --staged Cargo.toml
BLOCK@git read-tree -u HEAD
BLOCK@git checkout-index -a -f
BLOCK@git switch -f main
ALLOW@git switch main
BLOCK@rm -rf crates/core
BLOCK@rm --recursive x
BLOCK@rm -f a.txt
ALLOW@rm -f /tmp/a.txt
BLOCK@sed -i.bak s/a/b/ f
ALLOW@sed -n 5p f
BLOCK@perl -pi -e s/a/b/ f
ALLOW@perl -ne print f
ALLOW@git status
ALLOW@git diff HEAD
ALLOW@git log --oneline -5
ALLOW@ls -la
ALLOW@grep -rn 'git stash' docs
ALLOW@git commit -m 'never git stash here' -- a.txt
ALLOW@echo "do not git stash"
BLOCK@echo rm -rf is refused by design
ALLOW@git clean --dry-run
ALLOW@git clean --exclude=f
ALLOW@git commit -m 'git reset --hard'
ALLOW@git read-tree --index-output=f HEAD
ALLOW@git read-tree --prefix=x/ HEAD
ALLOW@git read-tree -i HEAD
ALLOW@git read-tree HEAD
ALLOW@git reset --hard=x
ALLOW@git reset --soft
ALLOW@git reset HEAD x
ALLOW@git restore --staged x
ALLOW@git switch -c f main
ALLOW@git switch -fc main
ALLOW@git worktree add --force x
ALLOW@git worktree add -f x
ALLOW@git worktree prune -f
ALLOW@git worktree remove x
ALLOW@gsed -n p f
ALLOW@perl --version
ALLOW@perl -e 'print 1'
ALLOW@perl -I/inc -e 1
ALLOW@perl -Mi -e 1
ALLOW@perl -mstrict -e 1
ALLOW@perl -Mstrict -e 1
ALLOW@sed --expression=s/a/b/ f
ALLOW@sed -n 1p f
BLOCK@/usr/bin/git reset --hard
BLOCK@/usr/bin/sed -i s f
BLOCK@command git reset --hard
BLOCK@echo hi; git reset --hard
BLOCK@env sed -i s f
BLOCK@git --git-dir=.git reset --hard
BLOCK@git -C /tmp/x clean -fd
BLOCK@git -c core.x=1 reset --hard
BLOCK@git -C x reset --hard
BLOCK@git checkout -- x
BLOCK@git clean --force
BLOCK@git clean --force=1
BLOCK@git clean -e -f
BLOCK@git clean -f
BLOCK@git clean -nf
BLOCK@git clean -xdf
BLOCK@git read-tree --reset -u HEAD
BLOCK@git read-tree --reset HEAD
BLOCK@git read-tree -m HEAD
BLOCK@git read-tree -mu HEAD
BLOCK@git reset --hard; sed -i s f
BLOCK@git reset --keep HEAD~
BLOCK@git reset --merge
BLOCK@git reset -q --hard
BLOCK@git switch --discard-changes main
BLOCK@git switch --force main
BLOCK@git worktree remove -f x
BLOCK@git worktree remove -ff x
BLOCK@gsed --in-place s/a/b/ f
BLOCK@gsed -i s f
BLOCK@perl -0777 -i -pe 1 f
BLOCK@perl -I -i -e 1
BLOCK@perl -i -pe 1 f
BLOCK@perl -pi -e 1 f
BLOCK@perl -wi -pe 1 f
BLOCK@sed --in-place=.bak s/a/b/ f
BLOCK@sed -E -i s/a/b/ f
BLOCK@sed -e s/a/b/ -i f
BLOCK@sed -i s/a/b/ f
BLOCK@sed -ni p f
BLOCK@x=sed; $x -i s f
CORPUS
fc_judged=$(
    # shellcheck source=/dev/null
    . "$GUARDS_SRC/rules/fallback/fail-closed.sh" || exit 1
    # Pinned: a COPY derives its org from its own location, which is not
    # where these cases' cwd lives. Both are read by the sourced judge.
    # shellcheck disable=SC2034
    GUARD_FALLBACK_ORG="$ORG"
    # shellcheck disable=SC2034
    GUARD_TOOL=bash
    while IFS='@' read -r want cmd; do
        [ -n "$want" ] || continue
        GUARD_CWD="$REPO" GUARD_CMD="$cmd"
        if guard_fallback_judge; then got=ALLOW; else got=BLOCK; fi
        printf '%s %s %s\n' "$want" "$got" "$cmd"
    done <<<"$FC_CORPUS"
    # shellcheck disable=SC2034 # read by the sourced judge
    GUARD_CWD=/tmp GUARD_CMD='git stash'
    if guard_fallback_judge; then got=ALLOW; else got=BLOCK; fi
    printf 'ALLOW %s git stash with a cwd outside the org\n' "$got"
)
fc_n=0
while read -r want got cmd; do
    [ -n "$want" ] || continue
    fc_n=$((fc_n + 1))
    CASES=$((CASES + 1))
    if [ "$want" = "$got" ]; then
        printf '  ok   [%-6s] fallback judge: %s\n' "$got" "$cmd"
    else
        printf '  FAIL expected %s got %s -- fallback judge: %s\n' "$want" "$got" "$cmd"
        FAIL_LOG="${FAIL_LOG}  FAIL expected $want got $got -- fallback judge: $cmd
"
        FAILED=1
    fi
done <<<"$fc_judged"
# A corpus that did not load or did not parse reads like a clean run.
fc_want=$(printf '%s\n' "$FC_CORPUS" | grep -c '@')
CASES=$((CASES + 1))
if [ "$fc_n" -eq $((fc_want + 1)) ]; then
    printf '  ok   [COUNT ] fallback judge answered all %s corpus rows\n' "$fc_n"
else
    printf '  FAIL fallback judge answered %s of %s rows (did fail-closed.sh load?)\n' "$fc_n" "$((fc_want + 1))"
    FAILED=1
fi
COVERED="$COVERED fail_closed "

echo
echo "one deny list -- the healthy engine refuses everything the fail-closed fallback refuses"
# Why: the engine and rules/fallback/fail-closed.sh must refuse the same
# destructive class. rules/80-destructive-class.sh asks the fallback's own judge,
# so there is one list; each case below is a spelling a narrower engine list let
# through (a here-string before the verb, an absolute git path, `command git`,
# `reset --hard`, `clean -fdx`, a cd out of the org then a recursive delete, a
# relative path climbing out of the org, an oversize payload, an unparseable request).
_dc_stash="git"" stash"
_dc_rm="r""m"
t BLOCK destructive_class "$REPO" bash '{"command":"cat <<<x; '"$_dc_stash"'"}' "here-string, then stash"
t BLOCK destructive_class "$REPO" bash '{"command":"/usr/bin/'"$_dc_stash"'"}' "stash by absolute git path"
t BLOCK destructive_class "$REPO" bash '{"command":"command '"$_dc_stash"'"}' "stash behind command"
t BLOCK destructive_class "$REPO" bash '{"command":"env -i FOO=1 '"$_dc_stash"'"}' "stash behind env with options"
t BLOCK destructive_class "$REPO" bash '{"command":"\"git\" stash"}' "stash with the verb quoted"
t BLOCK destructive_class "$REPO" bash '{"command":"git commit -F - <<'"'"'MSG'"'"' && '"$_dc_stash"'"}' "stash after a heredoc operator on the same line"
t BLOCK destructive_class "$REPO" bash '{"command":"git commit -F - <<'"'"'MSG'"'"'\nbody\nMSG\n'"$_dc_stash"'"}' "stash on a line after the heredoc terminator"
t BLOCK destructive_class "$REPO" bash '{"command":"bash <<EOF\n'"$_dc_stash"'\nEOF"}' "stash inside a heredoc fed to a shell"
t BLOCK destructive_class "$REPO" bash '{"command":"git reset --hard"}' "reset --hard"
t BLOCK destructive_class "$REPO" bash '{"command":"git -C ../sibling reset --hard HEAD~1"}' "reset --hard via -C"
t BLOCK destructive_class "$REPO" bash '{"command":"command git reset --hard"}' "reset --hard behind command"
t BLOCK destructive_class "$REPO" bash '{"command":"git reset --merge"}' "reset --merge"
t BLOCK destructive_class "$REPO" bash '{"command":"git clean -fdx"}' "clean -fdx"
t BLOCK destructive_class "$REPO" bash '{"command":"git clean --force"}' "clean --force"
t BLOCK destructive_class "$REPO" bash '{"command":"git switch -f main"}' "switch -f"
t BLOCK destructive_class "$REPO" bash '{"command":"git restore Cargo.toml"}' "restore of a path"
t ALLOW destructive_class "$REPO" bash '{"command":"git reset --soft HEAD~1"}' "reset --soft is not the class"
t ALLOW destructive_class "$REPO" bash '{"command":"git clean -n"}' "clean -n is a dry run"
t ALLOW destructive_class "$REPO" bash '{"command":"git status"}' "benign: status"
t ALLOW destructive_class "$REPO" bash '{"command":"git diff"}' "benign: diff"
t ALLOW destructive_class "$REPO" bash '{"command":"ls"}' "benign: ls"
t ALLOW destructive_class "$REPO" bash '{"command":"grep '"'"''"$_dc_stash"''"'"' docs"}' "benign: the verb as grep's quoted argument"
t ALLOW destructive_class "$REPO" bash '{"command":"git commit -m x"}' "benign: commit -m"
t ALLOW destructive_class "$REPO" bash '{"command":"echo \"a << b\"; git status"}' "benign: << inside quotes"
t ALLOW destructive_class "$REPO" bash '{"command":"git commit -F - <<'"'"'MSG'"'"'\nfix\n'"$_dc_stash"' and git reset --hard are named here\nMSG"}' "benign: a heredoc that only feeds git commit -F -"
t ALLOW destructive_class "$REPO" bash '{"command":"git stash list"}' "benign: stash list"
t ALLOW destructive_class /tmp bash '{"command":"'"$_dc_stash"'"}' "outside the org there is still no opinion"
# Containment: the same class of hole, in the verb the fallback refuses outright.
t BLOCK containment "$REPO" bash '{"command":"cd /etc && '"$_dc_rm"' -rf x"}' "recursive delete after a cd out of the org"
t BLOCK containment "$REPO" bash '{"command":"'"$_dc_rm"' -rf ../../x"}' "relative path that climbs out of the org"
t BLOCK containment "$REPO" bash '{"command":"'"$_dc_rm"' -rf '"$ORG"'/../zz"}' "absolute path that climbs out through the org prefix"
t BLOCK containment "$REPO" bash '{"command":"command '"$_dc_rm"' -rf /etc/zz"}' "delete behind command"
t ALLOW containment "$REPO" bash '{"command":"cd crates && '"$_dc_rm"' -rf x"}' "recursive delete after a cd inside the org"
t ALLOW containment "$REPO" bash '{"command":"'"$_dc_rm"' -rf ../sibling/zz"}' "relative sibling repo in the org"
t ALLOW containment "$REPO" bash '{"command":"git commit -F - <<'"'"'MSG'"'"'\nthe old rule let '"$_dc_rm"' -rf ../../x through\nMSG"}' "prose in a heredoc body is not a delete"

# The engine's own refusal to guess. jq is absent and the request is JSON the
# engine cannot read: the engine gives NO verdict and the adapter's fallback
# judges the raw command, instead of the engine saying allow.
_dc_nojq=$(printf '{"event":"pre_tool","tool":"bash","cwd":"%s","input":{"command":"x"}}' "$REPO" |
    env "${UNSET_ENV[@]}" PATH=/nonexistent /bin/bash "$ENGINE" 2>/dev/null)
CASES=$((CASES + 1))
if [ -z "$_dc_nojq" ]; then
    printf '  ok   [NOVERD] a missing jq gives NO verdict, never allow\n'
else
    printf '  FAIL a missing jq must give no verdict (%s)\n' "$_dc_nojq"
    FAIL_LOG="${FAIL_LOG}  FAIL a missing jq must give no verdict
"
    FAILED=1
fi

# Oversize input. Past any size the healthy engine must still
# read the whole command.
_dc_big=$(head -c 20000 /dev/zero | tr '\0' x)
fc=$(_fc_copy none)
_fc_run cli "$fc" "echo $_dc_big; $_dc_stash"
_fc_expect 1 'Blocked' "healthy cli refuses a stash after 20 KB of arguments"
_fc_run jcode "$fc" "echo $_dc_big; $_dc_stash"
_fc_expect 2 'Blocked' "healthy jcode-pre refuses a stash after 20 KB of arguments"
# Padding that makes the judge slow must not buy an allow: past the deadline the engine gives no
# verdict and the fallback must still answer, inside jcode's own 5 s hook timeout (which ALLOWS).
_dc_pad=$(head -c 30000 /dev/zero | tr '\0' "'")
_dc_huge=$(head -c 300000 /dev/zero | tr '\0' x)
for _dc_c in "echo $_dc_pad; git reset --ha""rd" "echo $_dc_huge; git reset --ha""rd" "echo $(printf '"a" %.0s' $(seq 6000)); git reset --ha""rd" "echo $_dc_huge"; do
    _dc_t0=$SECONDS
    _fc_run jcode "$fc" "$_dc_c"
    _fc_expect 2 'Blocked' "healthy jcode-pre refuses ${#_dc_c} bytes of padding (a script file is the way)"
    _assert yes "$([ $((SECONDS - _dc_t0)) -lt 6 ] && echo yes || echo no)" \
        "... and answers inside the hook timeout ($((SECONDS - _dc_t0)) s)"
done
rm -rf "$fc"
fc=$(_fc_copy noengine)
_fc_run jcode "$fc" "echo $_dc_big; $_dc_stash"
_fc_expect 2 "$FC_REFUSED" "degraded jcode-pre refuses a stash after 20 KB of arguments"
_fc_run jcode "$fc" "echo $_dc_big; git status"
_fc_expect 0 "$FC_LOUD" "degraded jcode-pre allows a long read-only command, loudly"
rm -rf "$fc"
COVERED="$COVERED destructive_class "

