#!/usr/bin/env bash
#
# CONV-GUARD-ADAPTERS: every adapter must reach the same verdict.
# Why: the predecessor rule (CONV-HOOK-PARITY) compared two independent guard implementations by
# hand-enumerated input and missed 39 of 123 real disagreements.
# With one implementation now, only the thin per-harness translation layer can diverge, and that
# is fully enumerable: a fixed, small number of adapters either agree or they do not.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# The adapters of the fixture install (lib.sh): they find the product and the org by their
# own place, so they must sit where a product keeps them. cli.sh resolves a relative --cwd
# against the caller's cwd, so the cases run from inside the fixture product.
A="$GUARDS_HOME/adapters"
cd "$REPO" || exit 1

for f in "$A/jcode-pre.sh" "$A/jcode-post.sh" "$A/cli.sh"; do
    [ -f "$f" ] || {
        echo "FAIL  [CONV-GUARD-ADAPTERS] missing adapter: $f -- this check cannot run, which is not the same as passing"
        exit 1
    }
done

command -v jq >/dev/null 2>&1 || {
    echo "FAIL  [CONV-GUARD-ADAPTERS] jq is required and absent -- abstaining would hide real drift"
    exit 1
}

# Run one command through every PRE adapter and compare VERDICTS.
#
# The CLI adapter is included deliberately, and it is the load-bearing one. The
# whole justification for this architecture is "supporting a new harness is one
# adapter and nothing else". That was an argument from structure with no check
# behind it: the same shape as the parity rule that reported agreement it had
# never measured. cli.sh was written against the documented contract alone,
# borrowing nothing from the agent harnesses, so its agreement here is evidence
# that the contract is genuinely harness-neutral rather than jcode-shaped.
#
# Verdicts, not exit codes: cli.sh uses 1 for blocked (2 means usage error to
# every shell) while the agent harnesses use 2. Mapping the verdict to the
# harness is an adapter's JOB, so comparing raw exit codes would flag the one
# thing that is legitimately different.
verdicts() {
    local cmd="$1" cwd="$2" jc cl in
    # As jcode sends it: the whole input on stdin, a copy in the variable. Not
    # inherited stdin: the loop below feeds the corpus on it, and jcode-pre
    # reads its stdin.
    in=$(jq -cn --arg c "$cmd" '{command:$c}')
    printf '%s' "$in" | JCODE_HOOK_CWD="$cwd" JCODE_HOOK_TOOL_NAME=bash \
        JCODE_HOOK_TOOL_INPUT="$in" \
        bash "$A/jcode-pre.sh" >/dev/null 2>&1
    [ $? -eq 2 ] && jc=block || jc=allow

    bash "$A/cli.sh" --tool bash --cwd "$cwd" --command "$cmd" >/dev/null 2>&1
    case $? in
        1) cl=block ;;
        0) cl=allow ;;
        *) cl="usage-error" ;;
    esac

    printf '%s %s' "$jc" "$cl"
}

# Every shape that has actually mattered here: the guards' own subjects, the
# evasions found by adversarial probing, and the ALLOW cases that prove neither
# adapter over-blocks. Scoped inputs matter as much as commands: the org root
# is where one harness resets cwd to, and treating it as foreign silently
# disabled every guard for that harness once already.
bad=0
checked=0
while IFS='@' read -r cmd cwd; do
    [ -n "${cmd:-}" ] || continue
    cmd="${cmd//REPOPATH/$REPO}"
    case "$cwd" in
        REPO) cwd="$REPO" ;;
        ORG) cwd="$ORG" ;;
        OUT) cwd="/tmp" ;;
    esac
    read -r jc cl <<<"$(verdicts "$cmd" "$cwd")"
    checked=$((checked + 1))
    if [ "$jc" != "$cl" ]; then
        echo "  jcode=$jc cli=$cl   [$cwd] $cmd"
        bad=$((bad + 1))
    fi
done <<'EOF'
git add .@REPO
git add .@ORG
git add .@OUT
git add a.txt@REPO
git stash@REPO
git stash list@REPO
sed -i '' s/a/b/ f@REPO
sed -n 5p f@REPO
cargo nextest run --workspace@REPO
cargo test -p core@REPO
make check | tail -5 && echo ok@REPO
git show HEAD:README.md@REPO
grep -r --include=*.rs foo .@REPO
git worktree add -b x /tmp/x@REPO
make check@ORG
make check@REPO
rm -rf /etc/zz-probe@REPO
rm -rf /etc/zz-probe@ORG
rm -rf /tmp/zz-probe@REPO
rm -rf ../sibling/zz-probe@REPO
rm -rf ~/.jcode/scratch/zz-probe@REPO
rm -f ~/.jcode/config.toml@REPO
git status@REPO
git -C REPOPATH stash@OUT
cd REPOPATH && git reset --hard@OUT
git stash@OUT
perl -e 'system("git stash")'@REPO
find . -delete@REPO
EOF

# A floor, for the same reason every other rule here has one: if the corpus is
# emptied or the loop breaks, "0 inputs, 0 disagreements" is true of the empty
# set and reads exactly like a clean tree.
MIN_INPUTS=18
if [ "$checked" -lt "$MIN_INPUTS" ]; then
    echo "FAIL  [CONV-GUARD-ADAPTERS] corpus collapsed to $checked inputs (expected at least $MIN_INPUTS)"
    exit 1
fi

# The jcode POST adapter must run clean: it always exits 0 (advice goes to stderr).
post_bad=0
for f in "$REPO/xtask/src/api_md.rs" "$REPO/Makefile" "$REPO/README.md"; do
    JCODE_HOOK_CWD="$REPO" JCODE_HOOK_TOOL_NAME=Edit JCODE_HOOK_STATUS=ok \
        JCODE_HOOK_TOOL_INPUT="$(jq -cn --arg p "$f" '{file_path:$p}')" \
        bash "$A/jcode-post.sh" >/dev/null 2>&1
    rc=$?
    checked=$((checked + 1))
    if [ "$rc" -ne 0 ]; then
        echo "  [post] jcode-post exited $rc   $f"
        post_bad=$((post_bad + 1))
    fi
done

# The CLI adapter is a PUBLIC INTERFACE: a git hook, CI step or human can call
# it: so its failure modes are part of the contract, not an afterthought.
# Every case here is a bug that was actually present, found by exercising the
# interface rather than reading it, and both failed in the dangerous direction
# (a caller error that looks like a VERDICT):
#
#   cli.sh --command          reported 1 (BLOCKED). ${2:?...} exits 1 under
#                             set -u, and 1 is this adapter's blocked code.
#   cli.sh --cwd /nonexistent reported 0 (ALLOWED). Only a RELATIVE cwd was
#                             validated, so an absolute typo landed outside the
#                             org tree, and silently turned the guards off.
#
# A checker that cannot tell "I refuse to answer" from "I looked and it is
# fine" is worse than none: the reason the engine reports on stdout, not exit.
cli_bad=0
cli_case() {
    local want="$1" label="$2"
    shift 2
    bash "$A/cli.sh" "$@" >/dev/null 2>&1
    local got=$?
    if [ "$got" != "$want" ]; then
        echo "  cli: expected exit $want, got $got -- $label"
        cli_bad=$((cli_bad + 1))
    fi
    checked=$((checked + 1))
}

# 2 = usage error, and it must never be confusable with 0 (allow) or 1 (block).
cli_case 2 "no arguments"
cli_case 2 "unknown flag" --bogus
cli_case 2 "--command with no value" --command
cli_case 2 "--file with no value" --file
cli_case 2 "--cwd with no value" --cwd
cli_case 2 "absolute --cwd that does not exist" --cwd /nonexistent-zz --command ls
cli_case 2 "relative --cwd that does not exist" --cwd nope-zz/rel --command ls
cli_case 0 "--help" --help
# And the verdicts themselves, including a relative cwd resolving to the org root.
cli_case 1 "git add . is blocked" --command "git add ."
cli_case 0 "git status is allowed" --command "git status"
cli_case 1 "relative --cwd resolves and stays in scope" --cwd .. --command "git add ."

if [ "$cli_bad" -ne 0 ]; then
    echo "FAIL  [CONV-GUARD-ADAPTERS] $cli_bad CLI contract violation(s)"
    echo "  A usage error must never be reported as a verdict."
    exit 1
fi

# THE ABSENT-INPUT PATH. Every case above SETS a tool input, so none of them
# exercised the default, and the default was broken for months.
#
# `${VAR:-\{\}}` expands to the LITERAL five characters `\{\}`: backslashes are
# not removed inside a `${...:-word}` default. That is not valid JSON, the
# engine's parse failed, and a failed parse ABSTAINS by design: verdict allow,
# no witness line, no advisory, exit 0. Indistinguishable from a clean run.
# The harness never sets the post_tool input, so EVERY real post_tool dispatch
# took that path and all ten advisory rules had never once fired live. The
# conclusion drawn at the time was that the harness does not dispatch post_tool
# at all; a probe wired straight to it proved it does, on every call.

# So assert the engine REACHES the rules when the input is absent, by requiring a
# witness line: exit code and verdict are identical on the broken path.
absent_bad=0
for a in jcode-post jcode-pre; do
    w=$(mktemp "${TMPDIR:-/tmp}/guard-absent.XXXXXX")
    ev=post_tool
    [ "$a" = jcode-pre ] && ev=pre_tool
    GUARD_WITNESS="$w" JCODE_HOOK_EVENT="$ev" JCODE_HOOK_TOOL_NAME=bash \
        JCODE_HOOK_CWD="$REPO" JCODE_HOOK_SESSION_ID=absent-input-probe \
        bash "$A/$a.sh" </dev/null >/dev/null 2>&1
    checked=$((checked + 1))
    if ! grep -q absent-input-probe "$w" 2>/dev/null; then
        echo "  [absent-input] $a.sh with NO tool input never reached the engine's rules"
        absent_bad=$((absent_bad + 1))
    fi
    rm -f "$w"
done
if [ "$absent_bad" -ne 0 ]; then
    echo "FAIL  [CONV-GUARD-ADAPTERS] $absent_bad adapter(s) abstain SILENTLY when the tool input is absent"
    echo "  Almost certainly a malformed default in guard_adapter_request: the request"
    echo "  must be valid JSON even when the harness sets no input. See the note there."
    exit 1
fi

# PORTABILITY, checked STATICALLY rather than behaviourally.
# A rule that reads a harness-specific variable (JCODE_HOOK_*) is a
# guard that silently changes behaviour depending on who is calling: the
# exact divergence this architecture exists to make impossible, one level down.
# This has to be a static check, and finding that out was worth the detour: I
# injected `[ -n "${JCODE_HOOK_SESSION_ID:-}" ] || return 0` into a rule and
# the behavioural comparison above stayed GREEN. The variable is absent when
# the check runs, so every adapter degraded IDENTICALLY and agreed on "allow" --
# agreement in the degraded direction reads exactly like agreement in the
# correct one. Under a real jcode session the same rule would block while the
# others allowed, a divergence the comparison can only see by luck.
# So: rules may read GUARD_* (populated from the request by the engine) and
# nothing else. The engine owns translation; a rule never learns who called it.
leak=$(grep -nE '\$\{?(JCODE_)' "$GUARDS_SRC"/rules/*.sh "$GUARDS_SRC"/rules/fallback/*.sh 2>/dev/null |
    grep -v '^\s*#' || true)
if [ -n "$leak" ]; then
    echo "FAIL  [CONV-GUARD-ADAPTERS] a RULE reads a harness-specific variable:"
    printf '%s\n' "$leak" | sed 's/^/  /'
    echo "  Rules must read only GUARD_* (populated from the request by the engine)."
    echo "  A rule that knows which harness called it can diverge per harness, which"
    echo "  is the failure this architecture exists to prevent."
    exit 1
fi

# The ENGINE names no harness variable. One that does means the contract has
# grown a harness-shaped hole.
engine_leak=$(grep -nE '\$\{?(JCODE_)' "$GUARDS_SRC/engine.sh" 2>/dev/null |
    grep -v '^\s*#' || true)
if [ -n "$engine_leak" ]; then
    echo "FAIL  [CONV-GUARD-ADAPTERS] the ENGINE reads a harness-specific variable:"
    printf '%s\n' "$engine_leak" | sed 's/^/  /'
    echo "  Only adapters may know about a harness. Move this into an adapter."
    exit 1
fi

if [ "$bad" -eq 0 ] && [ "$post_bad" -eq 0 ]; then
    echo "PASS  [CONV-GUARD-ADAPTERS] $checked inputs, every adapter agrees (incl. the harness-neutral CLI)"
    exit 0
fi
echo "FAIL  [CONV-GUARD-ADAPTERS] $((bad + post_bad)) input(s) where adapters DISAGREE"
echo "  The guards are shared, so this is a TRANSLATION bug: a mismapped field, a"
echo "  different cwd scope, or a mishandled verdict. Fix the adapter, not the rules."
exit 1
