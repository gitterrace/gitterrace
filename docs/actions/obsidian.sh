#!/bin/sh
# Send the selection to Obsidian. Payload fields come from the browsed repository: treat
# them as untrusted. They are written as file content or URL-encoded by jq, never run.
#
# Settings (the action's [action.vars] table, delivered as GITTERRACE_VAR_*):
#   MODE    daily (default): append to <VAULT>/<FOLDER>/YYYY-MM-DD.md, FOLDER defaults to Daily
#           inbox: append to <VAULT>/<FOLDER>/gitterrace.md, FOLDER defaults to Inbox
#           url: let the Obsidian app write it through obsidian://new
#   VAULT   daily and inbox: vault directory, default ~/Notes
#           url: vault name as Obsidian knows it, default Notes
#   FOLDER  see MODE
#
# Needs (all OSes): jq (brew install jq, apt install jq). date and mkdir are POSIX.
# url mode also needs Obsidian and a URL opener (first present wins):
#   macOS: open (built in)
#   Linux: xdg-open (xdg-utils)
#   WSL:   wslview (wslu) or explorer.exe
set -eu
if command -v jq >/dev/null 2>&1; then :; else
  echo "gitterrace: jq not found. Install it with your package manager (brew install jq, apt install jq)."
  exit 1
fi
mode="${GITTERRACE_VAR_MODE:-daily}"
vault="${GITTERRACE_VAR_VAULT:-}"
folder="${GITTERRACE_VAR_FOLDER:-}"
case "$mode" in
  daily|inbox)
    [ -n "$vault" ] || vault="$HOME/Notes"
    # vars arrive unexpanded, so a literal ~/ is meant here
    # shellcheck disable=SC2088
    case "$vault" in "~/"*) vault="$HOME/${vault#"~/"}" ;; esac
    if [ "$mode" = daily ]; then
      [ -n "$folder" ] || folder=Daily
      note="$vault/$folder/$(date +%F).md"
    else
      [ -n "$folder" ] || folder=Inbox
      note="$vault/$folder/gitterrace.md"
    fi
    mkdir -p "$(dirname "$note")"
    { printf '\n'; jq -r .markdown; } >> "$note"
    echo "appended to $(basename "$note")"
    ;;
  url)
    [ -n "$vault" ] || vault=Notes
    [ -n "$folder" ] || folder=Inbox
    if command -v open >/dev/null 2>&1; then set -- open
    elif command -v xdg-open >/dev/null 2>&1; then set -- xdg-open
    elif command -v wslview >/dev/null 2>&1; then set -- wslview
    elif command -v explorer.exe >/dev/null 2>&1; then set -- explorer.exe
    else
      echo "gitterrace: no URL opener found. Install xdg-utils (xdg-open)."
      exit 1
    fi
    enc() { printf '%s' "$1" | jq -sRr @uri; }
    md=$(jq -r .markdown | jq -sRr @uri)
    "$@" "obsidian://new?vault=$(enc "$vault")&file=$(enc "$folder/gitterrace")&append=true&content=$md"
    echo "sent to Obsidian"
    ;;
  *)
    echo "gitterrace: unknown MODE '$mode' for obsidian.sh. Use daily, inbox or url."
    exit 1
    ;;
esac
