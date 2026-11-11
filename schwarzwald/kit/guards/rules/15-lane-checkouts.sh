#!/usr/bin/env bash
#
# Lane checkouts: every lane works in its own clone at <repo>/<GUARD_WORKTREE_DIR>/<label>,
# made by tools/clone.sh, with the product's hooks installed. A `git worktree add` puts the
# lane's branch in the main repository's refs, a clone anywhere else is a copy no sweep or
# landing finds, and a clone without hooks commits past commit-msg and pre-commit: lane clones
# made by hand had no core.hooksPath in both products. A script's own temporary clone or
# worktree names its paths through variables, which these rules leave alone.

_lane_wt() { printf '%s' "${GUARD_WORKTREE_DIR:-.wt}"; }

guard_register worktree_location
guard_rule_worktree_location() {
    [ "$GUARD_TOOL" = bash ] || return 0
    guard_match "$(guard_cmd_head)" "$GUARD_ANCHOR"'git( +-C +[^ ]+)? +worktree +add([^;&|'$'\n'']*)' || return 0
    case "${BASH_REMATCH[${#BASH_REMATCH[@]} - 1]}" in *'$'*) return 0 ;; esac
    local wt
    wt=$(_lane_wt)
    guard_block \
        "a lane works in a clone, never a worktree: a worktree's branch lives in the main repository's refs." \
        "Make the clone: bash schwarzwald/kit/tools/clone.sh <label>   (it lands at <repo>/$wt/<label>, hooks installed)"
    return 0
}

# `git clone <src> [<dest>]` of a local repository inside the org: <dest> must be
# <src>/<wt>/<label>. A URL source (a release tap, a dependency) is not a lane and passes.
guard_register clone_location
guard_rule_clone_location() {
    [ "$GUARD_TOOL" = bash ] || return 0
    local head seg w wt src="" dest="" skip="" main
    head="$(guard_cmd_head)"
    guard_match "$head" "$GUARD_ANCHOR"'git( +-C +[^ ]+)? +clone[[:space:]](.*)' || return 0
    seg="${BASH_REMATCH[${#BASH_REMATCH[@]} - 1]}"
    seg="${seg%%[;&|$'\n']*}"
    local had_noglob=""
    case "$-" in *f*) had_noglob=1 ;; esac
    set -f
    for w in $seg; do
        if [ -n "$skip" ]; then skip=""; continue; fi
        case "$w" in
            -b | -o | -c | -u | -j | --branch | --origin | --config | --upload-pack | --template | \
                --depth | --reference | --reference-if-able | --separate-git-dir | --jobs | --filter | \
                --shallow-since | --shallow-exclude | --server-option | --bundle-uri)
                skip=1 ;;
            --) ;;
            -*) ;;
            *) if [ -z "$src" ]; then src="$w"; elif [ -z "$dest" ]; then dest="$w"; fi ;;
        esac
    done
    [ -n "$had_noglob" ] || set +f
    [ -n "$src" ] || return 0
    case "$src" in file://*) src="${src#file://}" ;; *://* | *@*:*) return 0 ;; esac
    src=$(_contain_expand "$src")
    case "$src" in *'$'* | *'`'*) return 0 ;; esac
    case "$src" in /*) ;; *) src="${GUARD_CWD:-$PWD}/$src" ;; esac
    src=$(_contain_norm "$src")
    case "$src" in "$GUARD_ORG"/*) ;; *) return 0 ;; esac
    wt=$(_lane_wt)
    main="${src%%/"$wt"/*}"
    if [ -n "$dest" ]; then
        dest=$(_contain_expand "$dest")
        case "$dest" in *'$'* | *'`'*) return 0 ;; esac
        case "$dest" in /*) ;; *) dest="$(_contain_base "$head" "${GUARD_CWD:-$PWD}")/$dest" ;; esac
        dest=$(_contain_norm "$dest")
    else
        dest="${src%/}"
        dest="$(_contain_base "$head" "${GUARD_CWD:-$PWD}")/${dest##*/}"
        dest=$(_contain_norm "${dest%.git}")
    fi
    case "$dest" in "$main/$wt"/?*) return 0 ;; esac
    guard_block \
        "a lane's clone of $main goes to $main/$wt/<label>, nowhere else (here: $dest)." \
        "Make it: bash schwarzwald/kit/tools/clone.sh <label> --repo $main   (installs the hooks too)"
    return 0
}

# A commit in a checkout whose product keeps hooks in scripts/hooks, while git would run none of
# them: core.hooksPath unset in a fresh clone, so commit-msg and pre-commit are skipped silently.
# The main checkout's .git/hooks symlinks count, since git runs what the hooks dir holds.
guard_register hookless_commit
guard_rule_hookless_commit() {
    [ "$GUARD_TOOL" = bash ] || return 0
    guard_cmd_is 'git( +-C +[^ ]+)? +commit([[:space:]]|$)' || return 0
    local repo hooks h missing=""
    repo=$(guard_cwd_repo)
    if guard_match "$(guard_cmd_head)" "$GUARD_ANCHOR"'git +-C +([^ ;&|]+) +commit'; then
        h=$(_contain_expand "${BASH_REMATCH[${#BASH_REMATCH[@]} - 1]}")
        case "$h" in /*) ;; *) h="${GUARD_CWD:-$PWD}/$h" ;; esac
        repo=$(env -u GIT_DIR -u GIT_WORK_TREE git -C "$h" rev-parse --show-toplevel 2>/dev/null || :)
    fi
    [ -n "$repo" ] && [ -d "$repo/scripts/hooks" ] || return 0
    hooks=$(env -u GIT_DIR -u GIT_WORK_TREE git -C "$repo" rev-parse --path-format=absolute \
        --git-path hooks 2>/dev/null) || return 0
    for h in commit-msg pre-commit; do
        [ -f "$repo/scripts/hooks/$h" ] || continue
        [ -x "$hooks/$h" ] || missing="$missing $h"
    done
    [ -n "$missing" ] || return 0
    guard_block \
        "git would skip this checkout's hooks:${missing} (git runs hooks from $hooks)." \
        "Install them: git -C $repo config core.hooksPath scripts/hooks" \
        "A lane clone made with schwarzwald/kit/tools/clone.sh has them already."
    return 0
}
