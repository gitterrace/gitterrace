#!/bin/sh
# POST the payload as-is. Keep the URL in your own config folder (the action's
# [action.vars] URL), never in a repo. Payload fields come from the browsed repository:
# treat them as untrusted on the receiving side too.
#
# Settings: GITTERRACE_VAR_URL, the endpoint (default is a placeholder).
# Needs (all OSes): curl (brew install curl, apt install curl).
set -eu
if command -v curl >/dev/null 2>&1; then :; else
  echo "gitterrace: curl not found. Install it with your package manager."
  exit 1
fi
curl -fsS -m 8 -H 'Content-Type: application/json' --data-binary @- \
  "${GITTERRACE_VAR_URL:-https://hooks.example.com/gitterrace}" >/dev/null
echo "posted"
