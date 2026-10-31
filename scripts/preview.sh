#!/usr/bin/env bash
# Rebuild and restart the owner's preview server on 127.0.0.1:4242 against the
# tendir repo. Safe to run any time: it only replaces our own process.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -q --release -p gitterrace
pkill -f 'target/release/git-terrace .*127.0.0.1:4242' 2>/dev/null || true
sleep 0.3
nohup ./target/release/git-terrace "${PREVIEW_REPO:-$HOME/github/tendir/tendir}" \
  --addr 127.0.0.1:4242 >/tmp/gt-4242.log 2>&1 &
for _ in $(seq 1 50); do
  curl -s -o /dev/null -m 1 127.0.0.1:4242/ && { echo "preview up: http://127.0.0.1:4242 ($(git rev-parse --short HEAD))"; exit 0; }
  sleep 0.2
done
echo "preview did not come up; see /tmp/gt-4242.log" >&2; exit 1
