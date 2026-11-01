#!/bin/sh
# Copy the selection as markdown. Payload fields come from the browsed repository: treat
# them as untrusted.
#
# Needs (first present wins):
#   macOS:         pbcopy (built in)
#   Linux Wayland: wl-copy (wl-clipboard)
#   Linux X11:     xclip or xsel
#   WSL:           clip.exe (built in)
# Always needs jq (macOS: brew install jq; Debian/Ubuntu: apt install jq).
set -eu
need() { command -v "$1" >/dev/null 2>&1 || { echo "gitterrace: $1 not found. $2"; exit 1; }; }
need jq "Install it with your package manager (brew install jq, apt install jq)."
if command -v pbcopy >/dev/null 2>&1; then set -- pbcopy
elif command -v wl-copy >/dev/null 2>&1; then set -- wl-copy
elif command -v xclip >/dev/null 2>&1; then set -- xclip -selection clipboard
elif command -v xsel >/dev/null 2>&1; then set -- xsel --clipboard --input
elif command -v clip.exe >/dev/null 2>&1; then set -- clip.exe
else
  echo "gitterrace: no clipboard tool found. Install wl-clipboard (Wayland) or xclip (X11)."
  exit 1
fi
jq -r .markdown | "$@"
echo "copied $GITTERRACE_TITLE"
