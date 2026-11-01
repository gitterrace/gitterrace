#!/bin/sh
# POST the payload as-is. Keep the URL here, in your own config folder, never in a repo.
# Payload fields come from the browsed repository: treat them as untrusted on the
# receiving side too.
set -eu
curl -fsS -m 8 -H 'Content-Type: application/json' --data-binary @- \
  "${GT_WEBHOOK:-https://hooks.example.com/gitterrace}" >/dev/null
echo "posted"
