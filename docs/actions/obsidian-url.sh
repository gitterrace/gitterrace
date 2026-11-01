#!/bin/sh
# Hand the selection to Obsidian, which does the write. Payload fields come from the
# browsed repository: treat them as untrusted. Everything is URL-encoded by jq.
set -eu
md=$(jq -r .markdown | jq -sRr @uri)
open "obsidian://new?vault=Notes&file=Inbox%2Fgitterrace&append=true&content=$md"
echo "sent to Obsidian"
