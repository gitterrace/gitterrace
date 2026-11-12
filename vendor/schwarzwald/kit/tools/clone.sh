#!/usr/bin/env bash
# Make a lane's clone, the one way every product makes one.
#
#   bash vendor/schwarzwald/kit/tools/clone.sh <label> [--repo <repo>] [--rev <rev>]
#
# Clones <repo> (default: the checkout the cwd is in) to <repo>/<dir>/<label> with
# `git clone --no-local`, where <dir> is GUARD_WORKTREE_DIR from the product's guards.env
# (default .wt, which the product git-ignores). The clone gets a branch named <label> at <rev>
# (default: the repo's HEAD), and core.hooksPath set to scripts/hooks when the product keeps
# its hooks there. A fresh clone has no hooksPath, so without that step commit-msg and
# pre-commit never run in it. The guards block a `git commit` in such a clone, and a clone
# or a `git worktree add` made any other way.
#
# Refused: a label that is not lowercase kebab, a path that already exists, a repo that is
# itself a lane clone (clone the main checkout), and a disk below the floor. The last line
# printed is the clone's path.
set -euo pipefail
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
usage() { sed -n '2,16s/^# \{0,1\}//p' "$0" >&2; exit 1; }
label="${1:-}"
[ -n "$label" ] && [ "${label#-}" = "$label" ] || usage
shift
repo="" rev=""
while [ $# -gt 0 ]; do
    case "$1" in
        --repo) repo="${2:?--repo needs a path}"; shift 2 ;;
        --rev) rev="${2:?--rev needs a revision}"; shift 2 ;;
        *) usage ;;
    esac
done
[[ $label =~ ^[a-z0-9][a-z0-9-]*$ ]] ||
    { echo "clone.sh: refused: label '$label' is not lowercase kebab (a-z, 0-9, -)" >&2; exit 1; }
repo="$(git -C "${repo:-$PWD}" rev-parse --show-toplevel)" ||
    { echo "clone.sh: refused: ${repo:-$PWD} is in no git checkout" >&2; exit 1; }
dir=".wt"
env_file="$repo/schwarzwald/config/guards.env"
if [ -f "$env_file" ]; then
    line="$(grep -E '^GUARD_WORKTREE_DIR=' "$env_file" | tail -1 || true)"
    [[ $line =~ \"(\$\{GUARD_WORKTREE_DIR:-)?([^\"\}]+)\}?\" ]] && dir="${BASH_REMATCH[2]}"
fi
dir="${GUARD_WORKTREE_DIR:-$dir}"
case "$repo" in
    */"$dir"/*) echo "clone.sh: refused: $repo is itself a lane clone; clone the main checkout" >&2; exit 1 ;;
esac
dest="$repo/$dir/$label"
[ ! -e "$dest" ] || { echo "clone.sh: refused: $dest exists" >&2; exit 1; }
. "$here/disk-floor.sh"
disk_floor "$repo" "$repo" || exit 1
mkdir -p "$repo/$dir"
git clone -q --no-local "$repo" "$dest"
start="$(git -C "$repo" rev-parse --verify --end-of-options "${rev:-HEAD}^{commit}")"
git -C "$dest" switch -q -c "$label" "$start"
if [ -d "$dest/scripts/hooks" ]; then
    git -C "$dest" config core.hooksPath scripts/hooks
else
    echo "clone.sh: note: $dest has no scripts/hooks, so no hooks are installed" >&2
fi
echo "$dest"
