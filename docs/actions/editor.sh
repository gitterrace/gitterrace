#!/bin/sh
# Open a file at a line in the editor, on any OS. Use with
#   command = ["~/.config/gitterrace/editor.sh", "{line_start}", "{root}/{path}"]
#   stdin   = "none"
# This opens the working-tree copy, which may differ from the commit you are viewing. Each
# argument arrives as one argv element, never through a shell string.
#
# Needs (all OSes): an editor. Picks $VISUAL, then $EDITOR, then the first of code, zed,
# zeditor found, then vi. Line jump is known for code, vim/nvim/vi, nano, emacs, subl,
# zed/zeditor and hx. Any other editor opens the file without a line.
set -eu
# pipefail is POSIX since 2024 but older dash lacks it, so it is set only where supported.
# shellcheck disable=SC3040
(set -o pipefail) 2>/dev/null && set -o pipefail
if [ "$#" != 2 ]; then
  echo "gitterrace: editor.sh takes a line and a file: \"{line_start}\", \"{root}/{path}\"."
  exit 1
fi
l=${1:-1}
f=$2
ed="${VISUAL:-${EDITOR:-}}"
if [ -z "$ed" ]; then
  ed="vi"
  for c in code zed zeditor; do
    if command -v "$c" >/dev/null 2>&1; then
      ed=$c
      break
    fi
  done
fi
# $ed is the user's own setting and may carry flags (for example "code -w").
set -f
# shellcheck disable=SC2086
set -- $ed
set +f
if ! command -v "$1" >/dev/null 2>&1; then
  echo "gitterrace: editor $1 not found. Set \$VISUAL or \$EDITOR, or install vi, code or zed."
  exit 1
fi
case "$(basename "$1")" in
code | code-insiders | codium) "$@" --goto "$f:$l" ;;
vim | nvim | vi) "$@" "+$l" -- "$f" ;;
nano | emacs | emacsclient) "$@" "+$l" "$f" ;;
subl | zed | zeditor | hx) "$@" "$f:$l" ;;
*) "$@" "$f" ;;
esac
echo "opened $f"
