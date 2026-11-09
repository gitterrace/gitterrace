#!/usr/bin/env bash
# The disk floor in bash, for hooks and shell gates: source it and call disk_floor, or run it.
#
#   . schwarzwald/kit/tools/disk-floor.sh; disk_floor <product-root> [path...] || exit 1
#   bash schwarzwald/kit/tools/disk-floor.sh <product-root> [path...]
#
# The same check as disk-floor.py: refuses (returns 1, one line on stderr) when free space on
# the disk holding a path (default the product root and $TMPDIR) is below the floor. The floor
# is `floor_gib` under [disk] in <product-root>/schwarzwald/config/lint.toml, default 50;
# DISK_FLOOR_GIB wins, and 0 turns the check off for one run. Unreadable free space returns 2.
# The one-line form for a hook that cannot source a file:
#   [ "$(df -Pk "$PWD" | awk 'NR==2{print $4}')" -ge $(( ${DISK_FLOOR_GIB:-50} * 1048576 )) ] || { echo "disk floor" >&2; exit 1; }

disk_floor() {
    local root="${1:-.}" floor="${DISK_FLOOR_GIB:-}" p q kib worst=0 line
    shift || true
    if [ -z "$floor" ]; then
        floor=50
        if [ -f "$root/schwarzwald/config/lint.toml" ]; then
            local in=0
            while IFS= read -r line; do
                case "$line" in
                    '['*) [ "$line" = "[disk]" ] && in=1 || in=0 ;;
                    floor_gib*) [ "$in" = 1 ] && { floor="${line#*=}"; floor="${floor//[[:space:]]/}"; } ;;
                esac
            done < "$root/schwarzwald/config/lint.toml"
        fi
    fi
    case "$floor" in ''|*[!0-9]*) echo "disk-floor: refused, the floor '$floor' is not a whole number of GiB" >&2; return 2 ;; esac
    if [ "$floor" -eq 0 ]; then echo "disk-floor: DISK_FLOOR_GIB=0, the free-space check is off for this run" >&2; return 0; fi
    [ $# -gt 0 ] || set -- "$root" "${TMPDIR:-/tmp}"
    for p in "$@"; do
        q="$p"
        while [ ! -e "$q" ] && [ "$q" != / ] && [ -n "$q" ]; do q="$(dirname "$q")"; done
        kib="$(df -Pk "$q" 2>/dev/null | awk 'NR==2{print $4}')"
        case "$kib" in ''|*[!0-9]*) echo "disk-floor: refused, cannot read the free space of $p" >&2; worst=2; continue ;; esac
        if [ "$kib" -lt $((floor * 1048576)) ]; then
            echo "disk-floor: refused, $((kib / 1048576)) GiB free on the disk holding $p, the floor is $floor GiB. Free space first: schwarzwald/kit/tools/disk-sweep.py lists stale build output (DISK_FLOOR_GIB overrides the floor for an emergency)" >&2
            [ "$worst" -ge 1 ] || worst=1
        fi
    done
    # A local snapshot pins deleted blocks: cleanup frees nothing until it is released.
    if command -v tmutil >/dev/null 2>&1; then
        local snaps
        snaps="$(tmutil listlocalsnapshots / 2>/dev/null | grep -v '^Snapshots for' | tr '\n' ' ')"
        [ -z "${snaps// /}" ] || echo "WARNING: local snapshot(s) pin deleted blocks: $snaps. Deletes will not free space until they are released: run sudo tmutil deletelocalsnapshots <date> yourself, or release them in the backup app's own settings (this tool never calls sudo)" >&2
    fi
    return "$worst"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    disk_floor "$@"
    exit $?
fi
