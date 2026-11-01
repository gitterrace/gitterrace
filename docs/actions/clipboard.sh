#!/bin/sh
# Copy the selection as markdown. Payload fields come from the browsed repository: treat
# them as untrusted.
set -eu
jq -r .markdown | pbcopy
echo "copied $GITTERRACE_TITLE"
