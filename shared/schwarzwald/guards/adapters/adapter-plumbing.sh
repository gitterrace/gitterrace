#!/usr/bin/env bash
#
# Shared adapter plumbing: anything duplicated across adapters belongs here,
# since per-file copies are how the dispatchers drifted apart.

guard_adapter_init() {
    local self="${1:?adapter path required}"
    case "$self" in
        /*) ;;
        *) self="$PWD/$self" ;;
    esac
    GUARD_ADAPTER_DIR="${self%/*}"
    # Canonical via the `cd` builtin, so a warning names the real engine path.
    local back="$PWD"
    CDPATH='' cd -- "$GUARD_ADAPTER_DIR" 2>/dev/null && GUARD_ADAPTER_DIR="$PWD"
    CDPATH='' cd -- "$back" 2>/dev/null || :
    GUARD_ENGINE="${GUARD_ADAPTER_DIR%/*}/engine.sh"
}

# Build a request: event, tool, cwd, raw-input-json, status. The default is
# assigned apart from the printf: `${4:-\{\}}` expanded to literal `\{\}`, the
# engine abstained silently, and no post_tool advisory ever ran live.
guard_adapter_request() {
    local input="${4:-}"
    [ -n "$input" ] || input='{}'
    printf '{"event":"%s","tool":"%s","cwd":"%s","status":"%s","input":%s}' \
        "$1" "$2" "$3" "${5:-ok}" "$input"
}

# Below jcode's 5 s hook timeout, which ALLOWS the call (14 such timeouts in
# its log). Whole seconds: bash 3.2's `read -t` takes no fraction.
: "${GUARD_ADAPTER_DEADLINE:=4}"

# Echo the verdict word (allow/block/none). $4 is a deadline in seconds; post
# adapters pass none, their lint advisories may be slow. stderr is INHERITED,
# never captured, so the reason reaches the session untouched.
guard_adapter_run() {
    local request="$1" entrypoint="$2" session="${3:-}" deadline="${4:-}"
    local left="" line v=none pid
    if [ -n "$deadline" ]; then
        left=$((deadline - SECONDS))
        [ "$left" -ge 1 ] || left=1
    fi
    exec 3< <(GUARD_ENTRYPOINT="$entrypoint" GUARD_SESSION_ID="$session" \
        exec bash "$GUARD_ENGINE" <<<"$request")
    pid=$!
    while IFS= read -r ${left:+-t "$left"} line <&3; do
        case "$line" in
            *"verdict: block"*) v=block && break ;;
            *"verdict: allow"*) v=allow && break ;;
        esac
    done
    [ "$v" != none ] || kill "$pid" 2>/dev/null
    exec 3<&-
    printf '%s' "$v"
}
