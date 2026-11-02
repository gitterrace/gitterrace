#!/bin/sh
# Append the selection to today's Obsidian daily note. Payload fields come from the browsed
# repository: treat them as untrusted. They are written as file content, never run.
# A new note or an inbox append needs no script: use obsidian://new (see docs/actions.md).
#
# Settings (the action's [action.vars] table, delivered as GITTERRACE_VAR_*):
#   VAULT   vault directory, default ~/Notes
#   FOLDER  daily notes folder in the vault, default Daily
#
# Needs (all OSes): jq (brew install jq, apt install jq). date and mkdir are POSIX.
set -eu
# pipefail is POSIX since 2024 but older dash lacks it, so it is set only where supported.
# shellcheck disable=SC3040
(set -o pipefail) 2>/dev/null && set -o pipefail
if command -v jq >/dev/null 2>&1; then :; else
  echo "gitterrace: jq not found. Install it with your package manager (brew install jq, apt install jq)."
  exit 1
fi
vault="${GITTERRACE_VAR_VAULT:-$HOME/Notes}"
folder="${GITTERRACE_VAR_FOLDER:-Daily}"
# vars arrive unexpanded, so a literal ~/ is meant here
# shellcheck disable=SC2088
case "$vault" in
"~/"*) vault="$HOME/${vault#"~/"}" ;;
*) ;;
esac
note="$vault/$folder/$(date +%F).md"
mkdir -p "$(dirname "$note")"
{
  printf '\n'
  jq -r .markdown
} >>"$note"
echo "appended to $(basename "$note")"
