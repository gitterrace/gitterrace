#!/usr/bin/env bash
#
# Every refusal carries a remedy line: a guard_block with one argument, or a bare
# `Blocked:` printf in the fail-closed fallback with no line after it, tells the agent what
# not to do and nothing to do instead. Read from the source, so a new rule is judged at once.
# Prints one line per offender and returns 1 when there is one. GUARDS_SRC is the tree to read.
remedy_offenders() {
    local src="${1:-$GUARDS_SRC}" f
    for f in "$src"/engine.sh "$src"/rules/*.sh "$src"/rules/fallback/*.sh; do
        awk -v file="${f#"$src"/}" '
            function flush() {
                if (stmt == "") return
                s = stmt
                sub(/^[ \t]*guard_block[ \t]*/, "", s)
                # one argument: a single "..." or "$var" token and nothing after it
                if (s !~ /^("([^"\\]|\\.)*"|\$[A-Za-z_{][A-Za-z0-9_}]*)[ \t]*(\}|$)/ ) { stmt = ""; return }
                print file ":" start ": guard_block has no remedy line"
                stmt = ""
            }
            /^[ \t]*guard_block[ \t]/ { stmt = ""; start = NR; cont = 1 }
            cont {
                line = $0
                if (line ~ /\\[ \t]*$/) { sub(/\\[ \t]*$/, "", line); stmt = stmt line " " }
                else { stmt = stmt line; cont = 0; flush() }
            }
        ' "$f"
    done
    # A remedy that names a kit tool takes the folder from ${GUARD_KIT_DIR}: a product keeps its
    # kit copy in vendor/schwarzwald/kit, schwarzwald/kit or its own --to, so a literal path (or a
    # default for it) names a file that does not exist there. Comment lines are prose.
    local kf
    for kf in "$src"/engine.sh "$src"/rules/*.sh "$src"/rules/fallback/*.sh "$src"/adapters/*.sh; do
        [ -f "$kf" ] || continue
        awk -v file="${kf#"$src"/}" '
            /^[ \t]*#/ { next }
            /(schwarzwald\/kit\/(tools|hooks|lint)|:-[^}]*schwarzwald\/kit[^}]*})/ {
                print file ":" NR ": remedy names a literal kit path, use ${GUARD_KIT_DIR}"
            }' "$kf"
    done
    # The fallback prints its Blocked: line by hand: each must be followed by a remedy line.
    awk '/printf .Blocked: %s\\n. "\$GUARD_FALLBACK_REASON" >&2/ {
            getline nxt
            if (nxt !~ /echo|printf/) print "rules/fallback/fail-closed.sh:" NR-1 ": Blocked: printf has no remedy line"
        }' "$src"/rules/fallback/fail-closed.sh
    awk '/printf .Blocked: %s\\n. "\$GUARD_FALLBACK_REASON" >&2/ {
            getline nxt
            if (nxt !~ /echo|printf/) print "adapters/jcode-pre.sh:" NR-1 ": Blocked: printf has no remedy line"
        }' "$src"/adapters/jcode-pre.sh
}
