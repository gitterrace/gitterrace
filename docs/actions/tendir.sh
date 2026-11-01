#!/bin/sh
# Send to tendir. Every payload field comes from the browsed repository: treat it as
# untrusted. Never eval it, quote every expansion, URL-encode before building a link.
#
# Opens tendir's prefill sheet (tendir://new-task) so you confirm before anything is
# written. The link is refused, not truncated, past its caps (title 500 characters,
# notes 20 000), so the script cuts first. Set GT_TENDIR=cli to add directly with
# `tendir add` instead (for tendir builds without the prefill sheet).
set -eu
payload=$(cat)
title=$(printf '%s' "$payload" | jq -r '.title[0:500]')
notes=$(printf '%s' "$payload" | jq -r '.markdown[0:20000]')
if [ "${GT_TENDIR:-link}" = cli ]; then
  tendir add --notes "$notes" -- "$title" >/dev/null
  echo "added: $title"
  exit 0
fi
enc() { printf '%s' "$1" | jq -sRr @uri; }
url="tendir://new-task?title=$(enc "$title")&notes=$(enc "$notes")"
open "$url"
echo "opened in tendir: $title"
