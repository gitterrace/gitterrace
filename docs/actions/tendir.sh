#!/bin/sh
# Send to tendir. Every payload field comes from the browsed repository: treat it as
# untrusted. Never eval it, quote every expansion, URL-encode before building a link.
#
# Opens tendir's prefill sheet (tendir://new-task) so you confirm before anything is
# written. The link is refused, not truncated, past its caps (title 500 characters,
# notes 20 000), so the script cuts first. Set GITTERRACE_VAR_MODE=cli (the action's
# [action.vars] MODE, default link) to add directly with
# `tendir add` instead (for tendir builds without the prefill sheet). The same
# fallback is used when no URL opener exists or the opener fails (Linux without a
# tendir:// handler), provided the tendir CLI is installed.
#
# Needs: jq, plus
#   macOS: open (built in) and the tendir app
#   Linux: xdg-open (xdg-utils) with a tendir:// handler, or the tendir CLI
#   WSL:   wslview (wslu) or explorer.exe, or the tendir CLI
set -eu
if command -v jq >/dev/null 2>&1; then :; else
  echo "gitterrace: jq not found. Install it with your package manager (brew install jq, apt install jq)."
  exit 1
fi
payload=$(cat)
title=$(printf '%s' "$payload" | jq -r '.title[0:500]')
notes=$(printf '%s' "$payload" | jq -r '.markdown[0:20000]')
add_cli() {
  tendir add --notes "$notes" -- "$title" >/dev/null
  echo "added: $title"
  exit 0
}
have_cli=0
if command -v tendir >/dev/null 2>&1; then have_cli=1; fi
case "${GITTERRACE_VAR_MODE:-link}" in
  link|cli) ;;
  *) echo "gitterrace: unknown MODE for tendir.sh. Use link or cli."; exit 1 ;;
esac
if [ "${GITTERRACE_VAR_MODE:-link}" = cli ]; then
  if [ "$have_cli" = 0 ]; then
    echo "gitterrace: tendir CLI not found. Install tendir or use MODE link."
    exit 1
  fi
  add_cli
fi
if command -v open >/dev/null 2>&1; then set -- open
elif command -v xdg-open >/dev/null 2>&1; then set -- xdg-open
elif command -v wslview >/dev/null 2>&1; then set -- wslview
elif command -v explorer.exe >/dev/null 2>&1; then set -- explorer.exe
else
  set --
fi
if [ "$#" = 0 ]; then
  if [ "$have_cli" = 1 ]; then add_cli; fi
  echo "gitterrace: no URL opener or tendir CLI found. Install xdg-utils or tendir."
  exit 1
fi
enc() { printf '%s' "$1" | jq -sRr @uri; }
url="tendir://new-task?title=$(enc "$title")&notes=$(enc "$notes")"
if "$@" "$url" >/dev/null 2>&1; then
  echo "opened in tendir: $title"
  exit 0
fi
if [ "$have_cli" = 1 ]; then add_cli; fi
echo "gitterrace: could not open tendir:// link. Install the tendir app or CLI."
exit 1
