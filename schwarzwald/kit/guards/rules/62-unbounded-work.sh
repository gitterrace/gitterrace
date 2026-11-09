#!/usr/bin/env bash
#
# Unbounded work: a command that cannot finish in reasonable time, or cannot finish at all.

# A broad root: the filesystem, home, the cwd, the org and its checkouts, or a
# target/ build tree, where a walk reaches millions of build artifacts. Derived, never literal.
guard_is_broad_root() {
    local p="${1%/}"
    p="${p#\"}"
    p="${p%\"}"
    p="${p%/}"
    # shellcheck disable=SC2016 # literal $HOME text is what the path holds
    case "$p" in
        "" | / | . | ./ | "~" | '$HOME' | '${HOME}') return 0 ;;
    esac
    # A build tree is broad at any depth: target/ alone holds hundreds of thousands of files.
    case "$p" in
        target | ./target | */target | target/* | */target/*) return 0 ;;
    esac
    local home="${HOME%/}" org="${GUARD_ORG%/}"
    [ -n "$home" ] && [ "$p" = "$home" ] && return 0
    [ -n "$org" ] || return 1
    [ "$p" = "$org" ] || [ "$p" = "${org%/*}" ] && return 0
    case "$p" in "$org"/*) [ "${p#"$org"/}" = "${p##*/}" ] && return 0 ;; esac
    return 1
}

guard_register unbounded_search
guard_rule_unbounded_search() {
    [ "$GUARD_TOOL" = bash ] || return 0
    local head
    head=$(guard_cmd_unquoted)

    # `grep -r` and `find` ignore .gitignore: a `grep -rn <word> <repo>` ran two
    # hours. Not caught: a scoped path, stdin, `--exclude-dir`,
    # `find -maxdepth`/`-prune`/`-path`, rg without --no-ignore.
    if guard_match "$head" "$GUARD_ANCHOR"'grep( +-[A-Za-z]*[rR][A-Za-z]*| +--recursive)( |$)' &&
        ! guard_match "$head" '--exclude-dir'; then
        local args
        args=$(printf '%s' "$head" | sed -E 's/.*grep +//; s/[|;&].*//')
        # No path operand means the cwd, a checkout root here.
        local paths p broad=""
        paths=$(printf '%s' "$args" | tr ' ' '\n' | grep -v '^-' | tail -n +2)
        [ -z "$paths" ] && broad=1
        while IFS= read -r p; do
            [ -n "$p" ] && guard_is_broad_root "$p" && broad=1
        done <<<"$paths"
        if [ -n "$broad" ]; then
            guard_block "recursive grep over a broad root walks ignored trees (target/, .build/, node_modules, dist/)." \
                "One such grep ran for two hours at full CPU." \
                "Use rg (skips .gitignore'd files) or git grep, or scope the path:" \
                "  rg -n '<pattern>' crates/        git grep -n '<pattern>' -- '*.rs'" \
                "Need every file? Add --exclude-dir=target --exclude-dir=.build --exclude-dir=node_modules."
            return 0
        fi
    fi
    if guard_match "$head" "$GUARD_ANCHOR"'rg .*(--no-ignore|-uu)( |$)' &&
        ! guard_match "$head" '(-g|--glob) |--max-depth|-d [0-9]'; then
        guard_block "rg --no-ignore/-uu walks target/ and other build output." \
            "Scope it: add -g '!target' -g '!.build' -g '!node_modules', or --max-depth N, or a narrower path."
        return 0
    fi
    if guard_match "$head" "$GUARD_ANCHOR"'find( |$)' &&
        guard_is_broad_root "$(printf '%s' "$head" | sed -E 's/.*(^|[;&|[:space:]])find +//; s/ .*//')" &&
        ! guard_match "$head" '-prune|-maxdepth|-path|-not -path|! -path'; then
        guard_block "find at a broad root walks ignored trees (target/, .build/, DerivedData)." \
            "Prefer: rg --files -g '<glob>' <path>   (respects .gitignore)" \
            "Or bound it: find <path> -maxdepth 3 ...   or   find <path> -path '*/target' -prune -o ... -print"
        return 0
    fi
}

guard_register unbounded_wait
guard_rule_unbounded_wait() {
    [ "$GUARD_TOOL" = bash ] || return 0
    local head
    head=$(guard_cmd_head)

    # A bare `wait` waits for EVERY child, so a never-ending `&` job turns into
    # a hang: ten `yes` burners pinned every core for eleven minutes. `wait $pid`
    # and kill-then-wait are fine.
    local start='(^|[;&|[:space:]])(yes|tail +-[fF]|sleep +(inf|infinity)|while +(true|:))([^;&|]*)&'
    guard_match "$head" "$start" || return 0
    local rest
    rest=$(printf '%s' "$head" | sed -E "s/.*$start//")
    local before_wait
    before_wait=$(printf '%s' "$rest" | sed -E 's/(^|[;&|[:space:]])wait( *(2?>[^ ]*|;|$|&&|\|\|)).*//')
    [ "$before_wait" != "$rest" ] || return 0
    guard_match "$before_wait" '(^|[;&|[:space:]])(kill|pkill)( |$)' && return 0
    guard_block "a bare \`wait\` also waits for the never-ending background job (yes, tail -f, while true)." \
        "The script hangs with that job still burning. Kill it first, or wait only for what you need:" \
        "  kill \"\${BURN[@]}\"; wait        or        wait \"\$writer_pid\""
}
