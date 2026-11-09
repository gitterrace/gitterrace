#!/usr/bin/env bash
#
# Files that must not be hand-edited at all. In-place sed/perl edits are judged
# by rules/80-destructive-class.sh.

# Glob $2 (a space-separated list, never expanded against the disk) matches path $1?
_edit_glob_hit() {
    local g globs
    read -r -a globs <<<"$2"
    for g in "${globs[@]}"; do
        # shellcheck disable=SC2053 # $g is a pattern on purpose
        [[ $1 == $g ]] && return 0
    done
    return 1
}

guard_register protected_files
guard_rule_protected_files() {
    case "$GUARD_TOOL" in write | edit | multiedit) ;; *) return 0 ;; esac
    [ -n "$GUARD_FILE" ] || return 0
    # A secret leaks, and the synced kit copy (schwarzwald/kit/) is overwritten by the next sync: the same in
    # every product. `.env`/`*.pem` were gaps once. Not caught: a Bash write (`cat > f`, `tee`),
    # which is containment's and the destructive class's to judge, not this rule's.
    case "$GUARD_FILE" in
        */secrets/* | *.key | *.pem | */.env | */.env.*)
            guard_block \
                "$GUARD_FILE is a secret." \
                "Secrets are never written by an agent; ask the owner."
            return 0
            ;;
        */schwarzwald/kit/*)
            guard_block \
                "$GUARD_FILE is the synced schwarzwald copy: the next sync overwrites it." \
                "Change source/ in schwarzwald, then sync it into the product."
            return 0
            ;;
    esac
    # The product's generated, vendored or golden files: hand edits are lost at the next
    # regeneration. Globs and the remedy from config.
    _edit_glob_hit "$GUARD_FILE" "${GUARD_PROTECTED_GLOBS:-}" || return 0
    guard_block \
        "$GUARD_FILE is generated, vendored or a golden file." \
        "${GUARD_PROTECTED_HINT:-Edit the source of truth and regenerate it.}"
    return 0
}

guard_register new_script_duplication
guard_rule_new_script_duplication() {
    case "$GUARD_TOOL" in write | edit | multiedit) ;; *) return 0 ;; esac
    [ -n "$GUARD_FILE" ] || return 0
    # Three implementations of one staleness check coexisted because each author wrote
    # before grepping.
    case "$GUARD_FILE" in
        */scripts/*.sh | */scripts/*.py | */scripts/*.mjs) ;;
        *) return 0 ;;
    esac
    [ -e "$GUARD_FILE" ] && return 0

    # The checkout the file is written in, not the guards' own clone.
    local base tok f hits='' repo
    repo=$(guard_cwd_repo)
    [ -n "$repo" ] || return 0
    base=$(basename "$GUARD_FILE" | sed -E 's/\.(sh|py|mjs)$//')
    for tok in $(printf '%s' "$base" | tr '_-' '  '); do
        [ ${#tok} -gt 3 ] || continue
        # An unmatched glob stays literal, hence -e.
        f=''
        for cand in "$repo"/scripts/*.sh "$repo"/scripts/*.py "$repo"/scripts/*.mjs \
            "$repo"/scripts/*/*.sh "$repo"/scripts/*/*.py; do
            [ -e "$cand" ] && f="$f
$cand"
        done
        f=$(printf '%s' "$f" | grep -i -- "$tok" | head -3)
        [ -n "$f" ] && hits="$hits
  '$tok' already appears in:
$(printf '%s' "$f" | sed 's|^|    |')"
    done
    [ -n "$hits" ] && guard_advise \
        "NOTE: new script $base -- grep the noun first.$hits" \
        "  Extend one of these, or say in the header why a new script is right." \
        "  (Three implementations of one staleness check once existed side by side.)"
    return 0
}
