#!/usr/bin/env bash
#
# The throwaway repo every script test that runs git, make or the script under test with a repo as
# cwd must use. Source it, then:
#
#     . "$(dirname "${BASH_SOURCE[0]}")/../tools/test-sandbox.sh"
#     sandbox_make my-test          # SANDBOX (a mktemp dir, removed on exit) and SANDBOX_REPO
#     ... cp fixtures into "$SANDBOX_REPO" using paths relative to the real repo ...
#     sandbox_enter                 # git init, isolate git, cd into the throwaway repo
#
# WHY. A broken heredoc in a draft test once ran the REAL xcframework.sh with the primary
# checkout as cwd and committed other sessions' working-tree hunks. A test that builds its fixture
# by `cd`ing into a path variable is one empty variable away from doing that to the shared checkout.
#
# WHAT IT DOES, three independent layers so one slip is not enough:
#   1. cwd is the throwaway repo, and `sandbox_assert_isolated` refuses to continue unless it is.
#   2. GIT_DIR and GIT_WORK_TREE point at it, so a `git commit` run from ANY directory, including a
#      stray `cd` back to the primary checkout, lands in the throwaway repo and never in the real one.
#   3. git's global and system config are replaced by a private file (user t <t@t>, no hooksPath, no
#      signing), so the real repo's hooks and identity cannot reach a test commit either.
# The real repo stays reachable, read-only, through `sandbox_real`.

if [ -n "${TEST_SANDBOX_LOADED:-}" ]; then
    # shellcheck disable=SC2317
    return 0 2>/dev/null || exit 0
fi
TEST_SANDBOX_LOADED=1

SANDBOX_REAL_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)  # tools/ sits two levels under the product root in dist/ and in a synced copy

sandbox_die() {
    printf 'test-sandbox: %s\n' "$*" >&2
    exit 97
}

# sandbox_make <name>: create the sandbox directory and arrange its removal.
sandbox_make() {
    local base="${JCODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}"
    SANDBOX=$(mktemp -d "$base/${1:?sandbox_make needs a name}.XXXXXX") || sandbox_die "mktemp failed"
    SANDBOX=$(cd "$SANDBOX" && pwd -P)
    SANDBOX_REPO="$SANDBOX/repo"
    mkdir -p "$SANDBOX_REPO" "$SANDBOX/bin"
    case "$SANDBOX" in
        "$SANDBOX_REAL_ROOT" | "$SANDBOX_REAL_ROOT"/*) sandbox_die "the sandbox $SANDBOX is inside the real repo" ;;
    esac
    trap 'rm -rf "$SANDBOX"' EXIT
}

# sandbox_enter [--multi]: initialise the throwaway repo, isolate git from the real one, and cd into it.
# --multi is for a test that builds several repos under the sandbox with git -C: it keeps the config
# isolation and the cwd, and leaves GIT_DIR and GIT_WORK_TREE unset so those repos resolve themselves.
sandbox_enter() {
    local multi=0
    [ "${1:-}" != "--multi" ] || multi=1
    [ -n "${SANDBOX:-}" ] && [ -d "$SANDBOX_REPO" ] || sandbox_die "sandbox_make has not run"
    export GIT_CONFIG_GLOBAL="$SANDBOX/gitconfig"
    export GIT_CONFIG_NOSYSTEM=1
    printf '[user]\n\tname = t\n\temail = t@t\n[commit]\n\tgpgsign = false\n[init]\n\tdefaultBranch = main\n' \
        >"$GIT_CONFIG_GLOBAL"
    env -u GIT_DIR -u GIT_WORK_TREE git init -q -b main "$SANDBOX_REPO" >/dev/null ||
        sandbox_die "git init failed"
    if [ "$multi" = 0 ]; then
        export GIT_DIR="$SANDBOX_REPO/.git" GIT_WORK_TREE="$SANDBOX_REPO"
    fi
    cd "$SANDBOX_REPO" || sandbox_die "cannot enter $SANDBOX_REPO"
    sandbox_assert_isolated
}

# sandbox_assert_isolated: stop the test unless every layer holds. Cheap enough to call before a
# step that would hurt if it ran in the real checkout.
sandbox_assert_isolated() {
    local here top
    here=$(pwd -P)
    case "$here" in
        "$SANDBOX"/*) ;;
        *) sandbox_die "cwd $here is outside the sandbox $SANDBOX" ;;
    esac
    [ "$here" != "$SANDBOX_REAL_ROOT" ] || sandbox_die "cwd is the real checkout"
    top=$(git rev-parse --show-toplevel 2>/dev/null) || sandbox_die "git sees no repository here"
    [ "$(cd "$top" && pwd -P)" = "$SANDBOX_REPO" ] ||
        sandbox_die "git resolves to $top, not the throwaway repo $SANDBOX_REPO"
    [ "$(git config user.email)" = "t@t" ] || sandbox_die "git config leaks the real identity"
}

# sandbox_real <command...>: run a READ-ONLY command against the real repo (a test that compares
# against the shipping tree, or asks the real repo's history something).
sandbox_real() {
    (cd "$SANDBOX_REAL_ROOT" && env -u GIT_DIR -u GIT_WORK_TREE -u GIT_CONFIG_GLOBAL -u GIT_CONFIG_NOSYSTEM "$@")
}
