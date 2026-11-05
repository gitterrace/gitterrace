#!/usr/bin/env bash
#
# Build and test workflow guards. The advisories name a cheaper alternative
# once: each underlying command is legitimate on its own.

guard_register workspace_test
guard_rule_workspace_test() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # Cost, not correctness. Quoted runs stripped: `--workspace` is a flag, so
    # GUARD_ANCHOR cannot keep prose about it from matching.
    guard_match "$(guard_cmd_unquoted)" 'cargo (nextest run|test)[^&|;]*--workspace' || return 0
    guard_cmd_has 'FULL_SUITE_OK|--doc' && return 0
    guard_block \
        "a bare full-workspace test run is the gate's job and holds the cargo lock for minutes." \
        "Scope it while iterating: cargo test -p <crate> [<name>]. The gate runs it: make test (or make check)." \
        "'cargo test --doc --workspace' is fine. To force, prefix FULL_SUITE_OK=1."
    return 0
}

guard_register unquoted_include_glob
guard_rule_unquoted_include_glob() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # zsh's "no matches found" reads as "no hits", not as a failed command.
    guard_match "$(guard_cmd_unquoted)" 'include=[*]' &&
        guard_block \
            "zsh will not expand an unquoted --include glob (you get 'no matches found')." \
            "Quote it, e.g. --include='*.rs'."
    return 0
}

guard_register worktree_location
guard_rule_worktree_location() {
    [ "$GUARD_TOOL" = bash ] || return 0
    guard_cmd_has 'git( -C [^ ]+)? worktree add' || return 0
    local wt="${GUARD_WORKTREE_DIR:-.wt}"
    case "$GUARD_CMD" in *"$wt/"*) return 0 ;; esac
    guard_block \
        "worktrees live in ONE place -- <checkout>/$wt/<name> (git-ignored, skipped by the linters)." \
        "Use: git worktree add -b <branch> $wt/<name> main" \
        "Run 'git worktree list' first; never touch another session's."
    return 0
}

guard_register pipe_masks_exit_code
guard_rule_pipe_masks_exit_code() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # The classes exclude NEWLINE: crossing lines matched six lines of a note
    # that merely discussed the rule. A real pipeline is one line.
    guard_match "$(guard_cmd_head)" '(make|cargo|swift)[^|;'$'\n'']*\|[ \t]*(tail|head)[^&;'$'\n'']*&&' || return 0
    guard_block \
        "piping make/cargo/swift into tail or head makes the following '&&' gate on TAIL's exit code (always 0), masking a real failure." \
        "Capture instead: redirect to a file, echo the exit code, then grep the full file for error:/FAILED."
    return 0
}

guard_register rtk_proxy_reads
guard_rule_rtk_proxy_reads() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # A silently empty read made a history scan report 0 matches as fact.
    guard_cmd_has 'rtk proxy' && return 0
    guard_cmd_has "${GUARD_ANCHOR}"'git( -C [^ ]+)? (show [^ |]*:|cat-file -p)' || return 0
    guard_block \
        "'git show <ref>:<path>' and 'git cat-file -p' go through the rtk proxy, which has returned SILENTLY EMPTY output for these." \
        "Use 'rtk proxy git ...' to get the raw bytes, and sanity-check the byte count (wc -c) before trusting a count derived from it."
    return 0
}

guard_register cargo_lock_contention
guard_rule_cargo_lock_contention() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # A queued foreground build gets SIGTERMed at the tool cap: no result at all.
    guard_cmd_has 'LOCK_OK' && return 0
    [ "${GUARD_BACKGROUND:-false}" = "true" ] && return 0
    guard_cmd_has "(make[[:space:]]+(${GUARD_BUILD_TARGETS_RE:-check|test|build})([[:space:]]|\$)|cargo[[:space:]]+(test|build)|cargo[[:space:]]+nextest[[:space:]]+run)" || return 0

    local base l n
    # The checkout the agent builds in, never the guards' own clone.
    local repo
    repo=$(guard_cwd_repo)
    for base in "$GUARD_CWD" ${repo:+"$repo"}; do
        l="$base/target/debug/.cargo-lock"
        [ -e "$l" ] || continue
        # Live machine state: without the switch a concurrent build flips
        # unrelated selftest cases to BLOCK.
        if [ -n "${GUARD_SELFTEST_NO_LSOF:-}" ]; then
            n=0
        else
            n=$(lsof "$l" 2>/dev/null | tail -n +2 | wc -l | tr -d ' ')
        fi
        if [ "${n:-0}" -ge 2 ]; then
            guard_block \
                "$n cargo processes already hold target/debug/.cargo-lock (other sessions build concurrently)." \
                "Re-issue with run_in_background: true and read the captured exit code from the notification." \
                "Never kill another session's build. To force, prefix LOCK_OK=1."
        fi
        break
    done
    return 0
}

guard_register bare_cargo_bypasses_make
guard_rule_bare_cargo_bypasses_make() {
    [ "$GUARD_TOOL" = bash ] || return 0
    guard_cmd_is 'cargo[[:space:]]' || return 0
    guard_cmd_has 'cargo[[:space:]]+(build|check|clippy|test|nextest|doc|bench|run)' || return 0
    guard_cmd_has '(-p[[:space:]]|--package|--bin[[:space:]]|--example|--doc)' && return 0
    # Nested under a make recipe already: the build lock exports its holder in the variable
    # GUARD_BUILD_LOCK_ENV names.
    [[ ${GUARD_BUILD_LOCK_ENV:-} =~ ^[A-Z][A-Z0-9_]*$ ]] && [ -n "${!GUARD_BUILD_LOCK_ENV:-}" ] && return 0
    # Repeating advice after a typed override teaches people to ignore it.
    guard_cmd_has '(FULL_SUITE_OK|LOCK_OK)' && return 0

    guard_advise \
        "NOTE: a workspace-wide cargo run outside make skips the build lock and the auto-GC." \
        "      Prefer the make target that wraps it${GUARD_MAKE_HINT:+ ($GUARD_MAKE_HINT)}," \
        "      or scope it with -p <crate> if this is deliberate one-off work. Advisory only."
    return 0
}

guard_register long_run_detached
guard_rule_long_run_detached() {
    [ "$GUARD_TOOL" = bash ] || return 0
    # A capped foreground job can surface as "exit 0": the harness reports its
    # OWN exit code. Only the targets that genuinely run long (GUARD_LONG_TARGETS_RE).
    guard_cmd_has "make[[:space:]]+(${GUARD_LONG_TARGETS_RE:-check|test|lint})([[:space:]]|\$)" || return 0
    [ "${GUARD_BACKGROUND:-false}" = "true" ] && return 0
    guard_cmd_has "${GUARD_DETACHED_RE:+$GUARD_DETACHED_RE|}nohup|setsid|&[[:space:]]*\$" && return 0
    [[ ${GUARD_BUILD_LOCK_ENV:-} =~ ^[A-Z][A-Z0-9_]*$ ]] && [ -n "${!GUARD_BUILD_LOCK_ENV:-}" ] && return 0

    guard_advise \
        "NOTE: this target runs for tens of minutes; a foreground cap will kill it mid-run and may report the WRAPPER's exit code." \
        "      Prefer run_in_background: true, then read the exit code from the notification." \
        "      Read the STATUS, not the log's length: a backgrounded redirect buffers. Advisory only."
    return 0
}
