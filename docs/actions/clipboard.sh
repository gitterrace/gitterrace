#!/bin/sh
# Copy the selection as markdown, on any OS. Use with stdin = "markdown": the text arrives
# on stdin and goes straight to the clipboard tool, never parsed. On macOS alone,
# command = ["pbcopy"] does the same with no script.
#
# Needs (first present wins):
#   macOS:         pbcopy (built in)
#   Linux Wayland: wl-copy (wl-clipboard)
#   Linux X11:     xclip or xsel
#   WSL:           clip.exe (built in)
#   Windows:       a POSIX sh is needed. On native Windows use command = ["clip.exe"] instead
set -eu
# pipefail is POSIX since 2024 but older dash lacks it, so it is set only where supported.
# shellcheck disable=SC3040
(set -o pipefail) 2>/dev/null && set -o pipefail
if command -v pbcopy >/dev/null 2>&1; then
  set -- pbcopy
elif command -v wl-copy >/dev/null 2>&1; then
  set -- wl-copy
elif command -v xclip >/dev/null 2>&1; then
  set -- xclip -selection clipboard
elif command -v xsel >/dev/null 2>&1; then
  set -- xsel --clipboard --input
elif command -v clip.exe >/dev/null 2>&1; then
  set -- clip.exe
else
  echo "gitterrace: no clipboard tool found. Install wl-clipboard (Wayland) or xclip (X11)."
  exit 1
fi
"$@"
echo "copied ${GITTERRACE_TITLE:-the selection}"
