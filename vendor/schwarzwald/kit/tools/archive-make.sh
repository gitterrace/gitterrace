#!/usr/bin/env bash
# Archive a repository's working tree as it is on disk, minus what git ignores.
#
#   bash vendor/schwarzwald/kit/tools/archive-make.sh [--repo <repo>] [--with-git] [<out>]
#
# Takes every file git would see: tracked files as they are now (uncommitted edits included),
# untracked files no .gitignore, .git/info/exclude or core.excludesFile rule ignores, hidden
# files and folders alike, and each submodule by the same rule. Ignored files (build output,
# caches, an ignored .env) stay out. --with-git also packs the .git folder, so the archive
# restores the full history.
#
# <out> names the archive and its extension picks the format: .tar.zst, .tar.gz, .tgz, .tar.xz,
# .tar or .zip. Default: <repo-name>-<short-sha>[-dirty].tar.zst beside the repo. Names with
# spaces, newlines, a leading `-` or non-ASCII bytes pack as they are.
#
# Refused: a repo that is no git checkout, an out path inside the repo (it would pack itself),
# an out file that exists, and an unknown extension. The last line printed is the archive path.
set -euo pipefail
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE
usage() { sed -n '2,17s/^# \{0,1\}//p' "$0" >&2; exit 1; }
refuse() { echo "archive-make.sh: refused: $*" >&2; exit 1; }
repo="" out="" with_git=0
while [ $# -gt 0 ]; do
    case "$1" in
        --repo) repo="${2:?--repo needs a path}"; shift 2 ;;
        --with-git) with_git=1; shift ;;
        -*) usage ;;
        *) [ -z "$out" ] || usage; out="$1"; shift ;;
    esac
done
repo="$(git -C "${repo:-$PWD}" rev-parse --show-toplevel 2>/dev/null)" ||
    refuse "${repo:-$PWD} is in no git checkout"
repo="$(cd "$repo" && pwd -P)"
if [ -z "$out" ]; then
    sha="$(git -C "$repo" rev-parse --short HEAD 2>/dev/null || echo empty)"
    dirty=""
    [ -z "$(git -C "$repo" status --porcelain)" ] || dirty="-dirty"
    out="$(dirname "$repo")/$(basename "$repo")-$sha$dirty.tar.zst"
fi
case "$out" in /*) ;; *) out="$PWD/$out" ;; esac
out_dir="$(cd "$(dirname "$out")" 2>/dev/null && pwd -P)" || refuse "$(dirname "$out") does not exist"
out="$out_dir/$(basename "$out")"
case "$out" in "$repo"/*) refuse "$out is inside the repo it packs" ;; esac
[ ! -e "$out" ] || refuse "$out exists"
case "$out" in
    *.tar.zst) fmt=(--zstd) ;;
    *.tar.gz | *.tgz) fmt=(-z) ;;
    *.tar.xz) fmt=(-J) ;;
    *.tar) fmt=() ;;
    *.zip) fmt=(--format zip) ;;
    *) refuse "$out: name it .tar.zst, .tar.gz, .tgz, .tar.xz, .tar or .zip" ;;
esac
# bsdtar (libarchive, the macOS tar) writes every format above and reads a NUL list with -n.
tar_bin=tar
tar --version 2>/dev/null | grep -q bsdtar || tar_bin=bsdtar
command -v "$tar_bin" >/dev/null || refuse "bsdtar (libarchive) is not installed"

# list_paths <dir> <prefix>: every path git sees in <dir> and does not ignore, NUL-ended, as
# ./<prefix><path>. A submodule (a folder with its own .git) is listed by its own rules.
list_paths() {
    local dir=$1 prefix=$2 p
    git -C "$dir" ls-files -z --cached --others --exclude-standard --deduplicate |
        while IFS= read -r -d '' p; do
            if [ -d "$dir/$p" ] && [ ! -L "$dir/$p" ] && [ -e "$dir/$p/.git" ]; then
                list_paths "$dir/$p" "$prefix$p/"
            elif [ -e "$dir/$p" ] || [ -L "$dir/$p" ]; then
                # A tracked file deleted on disk is not there to pack.
                printf './%s%s\0' "$prefix" "$p"
            fi
        done
}
list="$(mktemp "${TMPDIR:-/tmp}/archive-make.XXXXXX")"
trap 'rm -f "$list"' EXIT
list_paths "$repo" "" >"$list"
if [ "$with_git" = 1 ]; then
    [ -d "$repo/.git" ] || refuse "--with-git needs a .git folder in $repo, not a .git file"
    (cd "$repo" && find ./.git -print0) >>"$list"
fi
[ -s "$list" ] || refuse "$repo has no file to pack"
count="$(tr -cd '\0' <"$list" | wc -c | tr -d ' ')"
# -n: the list names every entry, so a folder in it adds itself and never its ignored content.
"$tar_bin" -C "$repo" -c ${fmt[@]+"${fmt[@]}"} -n -f "$out" --null -T "$list"
echo "archive-make.sh: packed $count entries from $repo ($(du -h "$out" | cut -f1 | tr -d ' '))" >&2
echo "$out"
