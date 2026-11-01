#!/usr/bin/env bash
# Rebuild and restart the preview server on 127.0.0.1:4242. Arguments are the repositories
# (or folders) to serve; with none, PREVIEW_REPO, or else this checkout. Only replaces our
# own process.
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "$#" -eq 0 ]; then
  if [ -n "${PREVIEW_REPO:-}" ]; then
    set -- "$PREVIEW_REPO"
  else
    set -- "$PWD"
  fi
fi
cargo build -q --release -p gitterrace
pkill -f 'target/release/git-terrace .*127.0.0.1:4242' 2>/dev/null || true
sleep 0.3
nohup ./target/release/git-terrace "$@" --addr 127.0.0.1:4242 >/tmp/gt-4242.log 2>&1 &
for _ in $(seq 1 50); do
  curl -s -o /dev/null -m 1 127.0.0.1:4242/ && { echo "preview up: http://127.0.0.1:4242 ($(git rev-parse --short HEAD)), serving: $*"; exit 0; }
  sleep 0.2
done
echo "preview did not come up; see /tmp/gt-4242.log" >&2; exit 1
