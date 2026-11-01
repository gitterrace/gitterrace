#!/bin/sh
# Append the selection to today's daily note: <vault>/Daily/YYYY-MM-DD.md.
# Payload fields come from the browsed repository: treat them as untrusted. They are
# written as file content only, never run.
set -eu
vault="${GT_VAULT:-$HOME/Notes}"
note="$vault/Daily/$(date +%F).md"
mkdir -p "$(dirname "$note")"
{ printf '\n'; jq -r .markdown; } >> "$note"
echo "appended to $(basename "$note")"
