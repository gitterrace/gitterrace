#!/usr/bin/env bash
#
# [CONV-GUARD-WIRING]: the guards a harness will ACTUALLY run must reach the engine.
# Why: twice a correct detector existed and was invoked by nobody: jcode read [hooks] from
# user config only, and .jcode/config.toml pointed at a deleted file, in both cases every other
# guard check stayed green.
# Checks every hook entrypoint named in a project-scope config in this checkout exists, is
# executable, and dispatches to the shared engine.
# Does not check user-scope config, whether the harness reads the config at all, or a config
# pointing at a non-adapter script beyond a path grep.
# Anchors on the KEY (pre_tool=/post_tool=), not a quoted path
# or any scripts/*.sh mention, since both shapes have misfired in the past.
# Run from the product root, or name it: wiring.sh [product-root].
. "$(dirname "${BASH_SOURCE[0]}")/prelude.sh"
GATE_ROOT="$(cd "${1:-$PWD}" && pwd)" || exit 2
cd "$GATE_ROOT" || exit 2

RULE=CONV-GUARD-WIRING
fail=0
checked=0

# Every config in THIS CHECKOUT that can name a hook entrypoint. A file that does
# not exist is not a failure: project-scope config is optional, and its absence
# means "this harness is wired globally or not at all", which is a legitimate
# choice this rule has no opinion about.
#
# `.jcode/config.toml` is the project-scope config in this checkout that names hook
# entrypoints. The rule's own red-case probe mutates it, so it must be read here.
configs="
.jcode/config.toml
"

report_bad() {
    emit_fail "$RULE" "$1"
    fail=$((fail + 1))
}

for config in $configs; do
    [ -f "$config" ] || continue

    # Pull the hook entrypoints out of the config. See the file header for why
    # the extraction is scoped to lines that ASSIGN a hook rather than the file.
    hooks=$(grep -E '^[^#]*((pre_tool|post_tool)[[:space:]]*=)' "$config" 2>/dev/null |
        grep -oE '(\$[A-Za-z_][A-Za-z_0-9]*/)?[^"'"'"' ]*scripts/[^"'"'"' ]*\.sh' |
        grep -v '[*?]' |
        sort -u)
    [ -n "$hooks" ] || continue

    while IFS= read -r hook; do
        [ -n "$hook" ] || continue
        checked=$((checked + 1))

        # Resolve a checkout-root variable so the existence check below is real.
        hook=${hook#\$GATE_ROOT/}

        # An absolute path from another machine or another checkout is its own
        # failure mode: it silently guards the WRONG TREE, or nothing.
        case "$hook" in
            "$GATE_ROOT"/*) rel="${hook#"$GATE_ROOT"/}" ;;
            /*)
                report_bad "$config names a hook OUTSIDE this checkout:
    $hook
  It will guard a different tree (or nothing). Point it at a path inside this checkout."
                continue
                ;;
            *) rel="$hook" ;;
        esac

        if [ ! -f "$rel" ]; then
            report_bad "$config points at a hook that DOES NOT EXIST:
    $hook
  A missing entrypoint does not fail loudly -- the harness dispatches, nothing
  runs, and every guard is silently off. Fix the path or remove the entry."
            continue
        fi

        if [ ! -x "$rel" ]; then
            report_bad "$config points at a hook that is NOT EXECUTABLE:
    $hook
  Fix: chmod +x $rel"
            continue
        fi

        # It exists and runs, but does it reach the shared engine? A hook that
        # is a real script yet dispatches nowhere is the two-dispatcher era
        # returning by the back door: guard logic living outside the shared guards,
        # invisible to every other check in this file's family.
        # A product shim that execs the shared adapter counts: that is how the hook path stays put.
        if ! grep -q 'guards/engine.sh\|guard_adapter_run\|adapters/adapter-plumbing.sh\|schwarzwald/guards/adapters/' "$rel" 2>/dev/null; then
            report_bad "$config names a hook that never reaches the shared engine:
    $hook
  It runs, so nothing looks broken, but the shared guards are not what it
  enforces. Point it at shared/schwarzwald/guards/adapters/ -- one guard set,
  many harnesses (AGENTS.md)."
        fi
    done <<EOF
$hooks
EOF
done

if [ "$fail" -eq 0 ]; then
    if [ "$checked" -eq 0 ]; then
        # Not a pass dressed as one: say that nothing was found, because "no
        # project-scope hooks configured" and "hooks configured correctly" are
        # different states and a reader must be able to tell them apart.
        emit_pass "$RULE" "no project-scope hook entrypoints configured (user scope is not this rule's business)"
    else
        emit_pass "$RULE" "$checked hook entrypoint(s) exist, are executable, and dispatch to the shared engine"
    fi
    exit 0
fi

printf '  %sA guard nobody dispatches is indistinguishable from no guard at all.%s\n' "$YELLOW" "$NC"
exit 1
