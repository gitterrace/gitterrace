#!/usr/bin/env bash
#
# Shared-checkout hygiene: several agents work in this tree at once, so a
# command scoped to "everything modified" is scoped to other people's work.

guard_register git_add_scope
guard_rule_git_add_scope() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # It has committed another agent's half-finished work here.
    guard_cmd_is 'git( +-C +[^ ]+)? +add( +-[A-Za-z]+)* +(\.|\./|-A|--all)([[:space:]]|$)' &&
        guard_block \
            "'git add .' / '-A' in a shared checkout stages OTHER sessions' work." \
            "Stage explicit paths: git add <path> <path>."
    return 0
}

# Anything staged in the shared index is swept into the next session's bare
# commit, and stale entries piled up to 109 in one day. A private index or a
# worktree (its own index) is the way to stage.
guard_register shared_index_staging
guard_rule_shared_index_staging() {
    [ "$GUARD_TOOL" = bash ] || return 0
    guard_match "$(guard_cmd_head)" "$GUARD_ANCHOR"'git( +-C +[^ ]+)? +(add|rm|mv)([[:space:]]|$)' || return 0
    guard_cmd_has 'GIT_INDEX_FILE=' && return 0
    # A worktree under GUARD_WORKTREE_DIR has its own index.
    local wt="${GUARD_WORKTREE_DIR:-.wt}"
    case " $GUARD_CMD $GUARD_CWD" in *"/$wt/"* | *" $wt/"* | *"=$wt/"*) return 0 ;; esac
    guard_block \
        "git add/rm/mv writes the SHARED index, which every other session's commit reads." \
        "Stage only in a private index: GIT_INDEX_FILE=.git/index-<lane> git read-tree HEAD && git add ... && git commit"
    return 0
}

# `git commit-tree` plus `git update-ref` makes a commit that runs NO hook: neither the pre-commit
# gate nor the post-commit index sync. Commit with `git commit`, from a private index.
guard_register commit_plumbing
guard_rule_commit_plumbing() {
    [ "$GUARD_TOOL" = bash ] || return 0
    guard_match "$(guard_cmd_head)" "$GUARD_ANCHOR"'git( +-C +[^ ]+)? +(commit-tree|update-ref +(-[a-z]+ +)*(refs/heads/|HEAD))' || return 0
    guard_block \
        "git commit-tree / update-ref on a branch skips every hook: the pre-commit gate and the post-commit index sync." \
        "Commit with: GIT_INDEX_FILE=.git/index-<lane> git read-tree HEAD && git add ... && git commit"
    return 0
}

guard_register git_commit_all
guard_rule_git_commit_all() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # Advisory, NOT a block, unlike `git add .`: `commit -a` is often what someone
    # means, and a guard that fires on ordinary intent gets routed around.
    guard_cmd_is 'git( +-C +[^ ]+)? +commit( +-[A-Za-z]*)*a' &&
        guard_advise \
            "NOTE: 'git commit -a' stages every modified tracked file, and this checkout is shared." \
            "      If another session has uncommitted work, this commits it too. Prefer explicit paths." \
            "      (Advisory only, deliberately: it fires on ordinary intent too.)"
    return 0
}

guard_register git_commit_index_scope
guard_rule_git_commit_index_scope() {
    [ "$GUARD_TOOL" = bash ] || return 0
    guard_cmd_is 'git( +-C +[^ ]+)? +commit' || return 0
    guard_cmd_has 'commit[^;|&]* -- ' && return 0
    guard_cmd_has 'commit( +-[A-Za-z]*)*a' && return 0
    guard_cmd_has 'commit[^;|&]*--amend' && return 0

    local staged
    # Without this switch the rule reads LIVE repo state, and staging any file
    # flipped six unrelated selftest ALLOW cases red.
    [ -n "${GUARD_SELFTEST_NO_INDEX:-}" ] && return 0
    # The index of the checkout the AGENT is in, not the guards' own clone (GUARD_REPO).
    local repo
    repo=$(guard_cwd_repo)
    [ -n "$repo" ] || return 0
    staged=$(git -C "$repo" diff --cached --name-only 2>/dev/null | head -20)
    [ -n "$staged" ] || return 0

    guard_advise \
        "NOTE: a bare 'git commit' takes the whole INDEX, which is shared with every other session here." \
        "      Staged right now:" \
        "$(
            # shellcheck disable=SC2086 # word-split ON PURPOSE: one indented line per staged path
            printf '        %s\n' $staged
        )" \
        "      If any of those are not yours, put the pathspec on the COMMIT instead:" \
        "        git commit <path> <path> -F -   (bypasses the index for those paths)" \
        "      Twice measured: two commits here each carry another session's work."
    return 0
}

# A copied `.git/index` is a snapshot: committing from it writes the older tree
# and undoes any commit landed since. Measured: it dropped a lane's new file.
guard_register private_index_copy
guard_rule_private_index_copy() {
    [ "$GUARD_TOOL" = bash ] || return 0
    guard_cmd_has 'cp( +-[A-Za-z]+)* +\.git/index +' || return 0
    guard_block \
        "copying .git/index makes a private index that can undo another session's newer commit." \
        "Build it from HEAD right before committing: GIT_INDEX_FILE=.git/index-x git read-tree HEAD"
    return 0
}

guard_register git_stash
guard_rule_git_stash() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # It swept another session's work here, and the pop then failed. `list` and
    # `show` stay allowed: they are how you FIND stranded work.
    guard_match "$(guard_cmd_head)" "$GUARD_ANCHOR"'git( +-C +[^ ]+)? +stash' || return 0
    guard_cmd_has 'git( -C [^ ]+)? stash (list|show)' && return 0
    guard_block \
        "git stash in a shared checkout sweeps up OTHER sessions' uncommitted work -- it is not scoped to your files." \
        "To baseline ONE file: git show HEAD:<path> > /tmp/<session>-baseline, measure, copy back."
    return 0
}

guard_register git_commit_backticks
guard_rule_git_commit_backticks() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # Scoped to `git commit -m` only: a general rule would block wanted $(...).
    guard_cmd_is 'git( +-C +[^ ]+)? +commit[^;]*-m' || return 0
    guard_cmd_has 'commit[^;]*-F' && return 0
    guard_cmd_has '`' &&
        guard_block \
            "backticks inside a 'git commit -m' message are command-substituted -- message fragments silently vanish." \
            "Use a quoted heredoc: git commit -F - <<'MSG' ... MSG"
    return 0
}

# Three lanes touched git under a brief saying FORBIDDEN in capitals: a missing
# mechanism, not a wording problem.
guard_register lane_git_write
guard_rule_lane_git_write() {
    # GUARD_LANE describes the WORK, not the harness, so a rule may read it.
    # Unset means coordinator: this only ever ADDS a refusal. Reads stay open.
    [ "$GUARD_TOOL" = bash ] || return 0
    [ "${GUARD_LANE:-}" = "ro" ] || return 0
    guard_cmd_is 'git( +-C +[^ ]+)? +(commit|add|tag|push|merge|rebase|cherry-pick|revert|am|apply)' || return 0
    guard_block \
        "this session is a READ-ONLY lane (GUARD_LANE=ro) and git writes are withheld by its brief." \
        "Report what you changed and let the coordinator land it. Reads (log, show, diff, status) stay open." \
        "If the brief meant to permit git, the coordinator sets GUARD_LANE=rw or leaves it unset."
    return 0
}

# A pathspec commits the file AS ON DISK and discards the staged hunk, so
# `git apply --cached` then `git commit <path>` adopts the neighbour's lines.
# Measured: main referenced a type whose file was still untracked. Speaks only
# for a path both staged and further modified.
guard_register git_commit_pathspec_discards_hunk
guard_rule_git_commit_pathspec_discards_hunk() {
    [ "$GUARD_TOOL" = bash ] || return 0
    guard_cmd_is 'git( +-C +[^ ]+)? +commit' || return 0
    guard_cmd_has 'commit( +-[A-Za-z]*)*a' && return 0
    [ -n "${GUARD_SELFTEST_NO_INDEX:-}" ] && return 0

    # NO `head`: truncating before the match missed a path that sorted 22nd.
    local both repo
    repo=$(guard_cwd_repo)
    [ -n "$repo" ] || return 0
    both=$(cd "$repo" 2>/dev/null &&
        comm -12 \
            <(git diff --cached --name-only 2>/dev/null | sort) \
            <(git diff --name-only 2>/dev/null | sort) 2>/dev/null)
    [ -n "$both" ] || return 0

    # Every match, not the first: fixing one and re-running discards the rest.
    local hits="" n=0
    while IFS= read -r p; do
        [ -n "$p" ] || continue
        case "$GUARD_CMD" in *"$p"*)
            hits="$hits        $p
"
            n=$((n + 1))
            ;;
        esac
    done <<EOF
$both
EOF
    [ $n -eq 0 ] && return 0

    local subject="that path is" verb="it"
    [ $n -gt 1 ] && subject="those $n paths are" && verb="each"

    guard_advise \
        "NOTE: $subject BOTH staged and further modified, and a pathspec commits" \
        "      the file as it is ON DISK, discarding what you staged for $verb:" \
        "${hits%
}" \
        "      If you staged a partial hunk of a shared file, this commits the" \
        "      other lines too. Check with: git diff <path>" \
        "      To keep the staged selection, commit without the pathspec."
    return 0
}
