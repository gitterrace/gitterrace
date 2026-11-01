#!/bin/sh
# Open a file at a line in the editor, on any OS. Use with
#   command = ["~/.config/gitterrace/editor.sh", "{line_start}", "{root}/{path}"]
#   stdin   = "none"
# This opens the working-tree copy, which may differ from the commit you are viewing. Each
# argument arrives as one argv element, never through a shell string.
#
# Needs (all OSes): an editor. Picks $VISUAL, then $EDITOR, then code, then vi.
# Line jump is known for code, vim/nvim/vi, nano, emacs, subl, zed and hx; any other
# editor opens the file without a line.
set -eu
if [ "$#" != 2 ]; then
  echo "gitterrace: editor.sh takes a line and a file: \"{line_start}\", \"{root}/{path}\"."
  exit 1
fi
l=${1:-1}
f=$2
ed="${VISUAL:-${EDITOR:-}}"
if [ -z "$ed" ]; then
  if command -v code >/dev/null 2>&1; then ed=code; else ed="vi"; fi
fi
# $ed is the user's own setting and may carry flags (for example "code -w").
set -f
# shellcheck disable=SC2086
set -- $ed
set +f
if ! command -v "$1" >/dev/null 2>&1; then
  echo "gitterrace: editor $1 not found. Set \$VISUAL or \$EDITOR, or install vi or code."
  exit 1
fi
case "$(basename "$1")" in
  code|code-insiders|codium) "$@" --goto "$f:$l" ;;
  vim|nvim|vi) "$@" "+$l" -- "$f" ;;
  nano|emacs|emacsclient) "$@" "+$l" "$f" ;;
  subl|zed|hx) "$@" "$f:$l" ;;
  *) "$@" "$f" ;;
esac
echo "opened $f"
