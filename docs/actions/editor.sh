#!/bin/sh
# Open the file at the selected line in the editor. Use with cwd = "repo".
# This opens the working-tree copy, which may differ from the commit you are viewing.
# Payload fields come from the browsed repository: treat them as untrusted. The path is
# passed as one argument after --, never through a shell string.
set -eu
cat >/dev/null
f="$GITTERRACE_ROOT/$GITTERRACE_PATH"
code --goto "$f:${GITTERRACE_LINE_START:-1}"   # or: ${EDITOR:-vi} "+${GITTERRACE_LINE_START:-1}" -- "$f"
echo "opened $GITTERRACE_PATH"
