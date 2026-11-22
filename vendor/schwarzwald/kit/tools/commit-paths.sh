#!/usr/bin/env bash
#
# commit-paths.sh: commit only the named paths through a private index.
#
#   commit-paths.sh [-C repo] -m message -- path...
#
# Parallel lanes share one working tree, so the shared .git/index is never touched. The script
# points GIT_INDEX_FILE at .git/index-<lane> (lane is $LANE, else the basename of a mktemp),
# runs read-tree HEAD, adds only the given paths, commits, prints the new sha and deletes the
# private index. It refuses an empty path list and any path outside the repository.
set -euo pipefail

repo=.
msg=
while [ $# -gt 0 ]; do
    case "$1" in
        -C) repo=${2:?-C needs a directory}; shift 2 ;;
        -m) msg=${2:?-m needs a message}; shift 2 ;;
        --) shift; break ;;
        *) echo "commit-paths: unknown argument $1" >&2; exit 2 ;;
    esac
done
[ -n "$msg" ] || { echo "commit-paths: -m message is required" >&2; exit 2; }
[ $# -gt 0 ] || { echo "commit-paths: no paths given" >&2; exit 2; }

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE
top=$(git -C "$repo" rev-parse --show-toplevel) || { echo "commit-paths: $repo is not a repository" >&2; exit 2; }
top=$(cd "$top" && pwd -P)
gitdir=$(git -C "$top" rev-parse --absolute-git-dir)

rel=()
for p in "$@"; do
    case "$p" in /*) abs=$p ;; *) abs=$(cd "$repo" && pwd -P)/$p ;; esac
    dir=$(dirname "$abs")
    [ -d "$dir" ] || { echo "commit-paths: $p is outside the repository" >&2; exit 2; }
    abs=$(cd "$dir" && pwd -P)/$(basename "$abs")
    case "$abs" in
        "$top"/*) rel+=("${abs#"$top"/}") ;;
        *) echo "commit-paths: $p is outside the repository" >&2; exit 2 ;;
    esac
done

if [ -n "${LANE:-}" ]; then
    idx="$gitdir/index-$LANE"
else
    idx=$(mktemp "$gitdir/index-XXXXXX")
    rm -f "$idx"
fi
trap 'rm -f "$idx"' EXIT
export GIT_INDEX_FILE=$idx
git -C "$top" read-tree HEAD 2>/dev/null || git -C "$top" read-tree --empty
git -C "$top" add -- "${rel[@]}"
git -C "$top" commit -q -m "$msg"
git -C "$top" rev-parse --short HEAD
