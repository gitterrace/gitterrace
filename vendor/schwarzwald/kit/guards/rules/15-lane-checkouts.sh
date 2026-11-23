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
        "Make the clone: bash ${GUARD_KIT_DIR}/tools/clone.sh <label>   (it lands at <repo>/$wt/<label>, hooks installed)"
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
    case "$src" in /*) ;; *) src="$(_contain_base "$head" "${GUARD_CWD:-$PWD}")/$src" ;; esac
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
        "Make it: bash ${GUARD_KIT_DIR}/tools/clone.sh <label> --repo $main   (installs the hooks too)"
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
        "A lane clone made with ${GUARD_KIT_DIR}/tools/clone.sh has them already."
    return 0
}

# A feature lane (GUARD_LANE=feature, from the product's lane variable or the roles file, so
# unset keeps today's behaviour) neither runs the product's full gate nor writes the main
# checkout. Every lane running the full gate runs the same gate on one tree; a lane writing
# the main checkout collides with the others. Both only ever ADD a refusal.

guard_register feature_lane_full_gate
guard_rule_feature_lane_full_gate() {
    [ "$GUARD_TOOL" = bash ] || return 0
    [ "${GUARD_LANE:-}" = "feature" ] || return 0
    [ -n "${GUARD_FULL_GATE_CMDS:-}" ] || return 0
    guard_cmd_is "$GUARD_FULL_GATE_CMDS" || return 0
    guard_cmd_has 'GATE_OK=1' && return 0
    guard_block \
        "this session is a FEATURE lane and the full gate (${GUARD_FULL_GATE_CMDS}) runs once, on the coordinator's tree." \
        "Run the targeted tests for what you changed and leave the full gate to the coordinator." \
        "To force it anyway, prefix GATE_OK=1."
    return 0
}

# The physical path of $1: its deepest existing ancestor resolved with pwd -P (every symlink
# followed), the not-yet-existing rest appended. Empty when nothing resolves.
_role_real() {
    local p="$1" rest="" d
    case "$p" in /*) ;; *) p="${GUARD_CWD:-$PWD}/$p" ;; esac
    p=$(_contain_norm "$p")
    while [ ! -d "$p" ] && [ "$p" != / ]; do
        rest="/${p##*/}$rest"
        p="${p%/*}"
        [ -n "$p" ] || p=/
    done
    d=$(cd -P "$p" 2>/dev/null && pwd -P) || return 0
    [ "$d" = / ] && d=""
    printf '%s%s' "$d" "$rest"
}

# Is path $1 in the main checkout proper, a lane clone under <main>/<wt>/ excluded?
_role_in_main() {
    local main real wt common
    wt=$(_lane_wt)
    main="${GUARD_MAIN_CHECKOUT:-}"
    if [ -z "$main" ] && [ -n "${GUARD_REPO:-}" ]; then
        # Unset: the main worktree of the install's repository, the parent of its common git
        # dir. An install inside a lane clone has no main to guard, so nothing is derived.
        common=$(env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE git -C "$GUARD_REPO" rev-parse \
            --path-format=absolute --git-common-dir 2>/dev/null) || return 1
        main="${common%/.git}"
        [ "$main" != "$common" ] || return 1
        case "$main" in */"$wt"/*) return 1 ;; esac
    fi
    case "$main" in /*) ;; *) return 1 ;; esac
    main=$(_role_real "$main")
    [ -n "$main" ] || return 1
    real=$(_role_real "$1")
    [ -n "$real" ] || return 1
    case "$real" in
        "$main/$wt"/*) return 1 ;;
        "$main" | "$main"/*) return 0 ;;
    esac
    return 1
}

guard_register feature_lane_main_checkout
guard_rule_feature_lane_main_checkout() {
    [ "${GUARD_LANE:-}" = "feature" ] || return 0
    local target="" head
    case "$GUARD_TOOL" in
        write | edit | multiedit) target="$GUARD_FILE" ;;
        bash)
            head="$(guard_cmd_head)"
            guard_cmd_is 'git( +-C +[^ ]+)? +(commit|add|tag|push|merge|rebase|cherry-pick|revert|am|apply|reset|checkout|restore|switch|stash|clean|rm|mv|pull|branch|update-ref)([[:space:]]|$)' || return 0
            target=$(_contain_base "$head" "${GUARD_CWD:-$PWD}")
            if guard_match "$head" "$GUARD_ANCHOR"'git +-C +([^ ;&|]+)'; then
                target=$(_contain_expand "${BASH_REMATCH[${#BASH_REMATCH[@]} - 1]}")
                case "$target" in /*) ;; *) target="$(_contain_base "$head" "${GUARD_CWD:-$PWD}")/$target" ;; esac
            fi
            ;;
        *) return 0 ;;
    esac
    [ -n "$target" ] || return 0
    _role_in_main "$target" || return 0
    guard_block \
        "this session is a FEATURE lane and $target is in the main checkout, which takes landings only." \
        "Work in your lane clone: <main>/$(_lane_wt)/<your label> (bash ${GUARD_KIT_DIR}/tools/clone.sh <label>) and hand the result to the coordinator."
    return 0
}
