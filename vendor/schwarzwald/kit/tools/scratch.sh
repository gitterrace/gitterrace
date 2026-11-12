#!/usr/bin/env bash
# Self-cleaning temp for red-case and golden runners: every worktree, target dir and scratch
# file a run makes sits under one known root, and goes when the run ends, pass or fail.
#
#   . vendor/schwarzwald/kit/tools/scratch.sh
#   scratch_init <name> [--keep] || exit 1   checks the disk floor, makes the run dir, sets the trap
#   d=$(scratch_dir <sub>) || exit 1          a fresh dir inside the run dir
#   wt=$(scratch_worktree <repo> <rev>) || exit 1
#                                            a detached worktree inside the run dir, refused
#                                            loudly when git fails (a full disk among others)
#   export CARGO_TARGET_DIR="$(scratch_dir target)"   so the build output goes with the run
#
# The root is SCRATCH_ROOT, default $TMPDIR/schwarzwald-scratch (per user on macOS), else
# $XDG_RUNTIME_DIR/schwarzwald-scratch, else ~/.cache/schwarzwald-scratch: never a fixed name in
# shared /tmp. The root must be a real dir (not a symlink) this user owns; it is set to 0700.
# Each run gets
# <root>/<name>.<pid>.XXXX, so the kit's tools/disk-sweep.py can find what a killed run left behind.
# --keep (or SCRATCH_KEEP=1) keeps the run dir and prints where it is, for debugging. Worktrees
# are unregistered from their repository on exit too, so no stale entry is left in it.

SCRATCH_RUN=""

scratch_cleanup() {
    local wt
    [ -n "$SCRATCH_RUN" ] || return 0
    if [ "${SCRATCH_KEEP:-0}" = 1 ]; then
        echo "scratch: kept $SCRATCH_RUN" >&2
        return 0
    fi
    # scratch_worktree runs in a $(...) subshell, so the worktrees are listed in a file, not a variable.
    local repos=()
    if [ -f "$SCRATCH_RUN/.worktrees" ]; then
        while IFS= read -r wt; do
            git -C "${wt%%|*}" worktree remove --force "${wt#*|}" >/dev/null 2>&1 || true
            repos+=("${wt%%|*}")
        done < "$SCRATCH_RUN/.worktrees"
    fi
    rm -rf "$SCRATCH_RUN"
    for wt in "${repos[@]+"${repos[@]}"}"; do
        git -C "$wt" worktree prune >/dev/null 2>&1 || true
    done
    SCRATCH_RUN=""
}

scratch_root() { # the default root, the same rule tools/disk-sweep.py uses
    if [ -n "${SCRATCH_ROOT:-}" ]; then printf '%s\n' "$SCRATCH_ROOT"
    elif [ -n "${TMPDIR:-}" ]; then printf '%s\n' "${TMPDIR%/}/schwarzwald-scratch"
    elif [ -n "${XDG_RUNTIME_DIR:-}" ]; then printf '%s\n' "${XDG_RUNTIME_DIR%/}/schwarzwald-scratch"
    else printf '%s\n' "${HOME:?scratch: no TMPDIR, XDG_RUNTIME_DIR or HOME}/.cache/schwarzwald-scratch"
    fi
}

scratch_init() {
    local name="${1:?scratch_init needs a run name}" here root
    shift
    [ "${1:-}" = --keep ] && SCRATCH_KEEP=1
    here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    root="$(scratch_root)"
    root="${root%/}"
    . "$here/disk-floor.sh"
    disk_floor "${SCRATCH_PRODUCT:-$PWD}" "$root" || return 1
    [ ! -L "$root" ] || { echo "scratch: refused, the root $root is a symlink" >&2; return 1; }
    (umask 077 && mkdir -p "$root") || { echo "scratch: refused, cannot make $root" >&2; return 1; }
    [ -O "$root" ] || { echo "scratch: refused, the root $root is not owned by this user" >&2; return 1; }
    chmod 700 "$root" || { echo "scratch: refused, cannot set $root to 0700" >&2; return 1; }
    SCRATCH_RUN="$(mktemp -d "$root/$name.$$.XXXX")" || { echo "scratch: refused, cannot make a run dir under $root" >&2; return 1; }
    trap scratch_cleanup EXIT
    trap 'scratch_cleanup; exit 130' INT
    trap 'scratch_cleanup; exit 143' TERM
}

scratch_dir() {
    local d="$SCRATCH_RUN/${1:?scratch_dir needs a name}"
    [ -n "$SCRATCH_RUN" ] || { echo "scratch: scratch_init first" >&2; return 1; }
    mkdir -p "$d" || { echo "scratch: refused, cannot make $d" >&2; return 1; }
    printf '%s\n' "$d"
}

scratch_worktree() {
    local repo="${1:?scratch_worktree needs a repository}" rev="${2:-HEAD}" wt out
    [ -n "$SCRATCH_RUN" ] || { echo "scratch: scratch_init first" >&2; return 1; }
    wt="$(mktemp -d "$SCRATCH_RUN/wt.XXXX")" || { echo "scratch: refused, cannot make a worktree dir under $SCRATCH_RUN" >&2; return 1; }
    printf '%s|%s\n' "$repo" "$wt" >> "$SCRATCH_RUN/.worktrees" || { echo "scratch: refused, cannot record $wt" >&2; return 1; }
    if ! out="$(git -C "$repo" worktree add -q --detach "$wt" "$rev" 2>&1)"; then
        echo "scratch: refused, could not make a worktree of $repo at $rev: ${out:-git failed}" >&2
        return 1
    fi
    # git can exit 0 with a checkout it could not finish writing: hold it to the commit.
    if [ -n "$(git -C "$wt" status --porcelain 2>/dev/null)" ] || ! git -C "$wt" rev-parse -q --verify HEAD >/dev/null; then
        echo "scratch: refused, the worktree at $wt is incomplete (a full disk?)" >&2
        return 1
    fi
    printf '%s\n' "$wt"
}
