#!/usr/bin/env bash
# Rebuild and restart the preview server on the default address (DEFAULT_ADDR in gitterrace-config). Arguments are the repositories
# (or folders) to serve; with none, PREVIEW_REPO, or else this checkout. Only replaces our
# own process.
set -euo pipefail
cd "$(dirname "$0")/.."
ADDR=$(sed -n 's/.*DEFAULT_ADDR: &str = "\(.*\)";/\1/p' crates/gitterrace-config/src/schema.rs)
LOG="/tmp/gt-${ADDR##*:}.log"
if [ "$#" -eq 0 ]; then
  if [ -n "${PREVIEW_REPO:-}" ]; then
    set -- "$PREVIEW_REPO"
  else
    set -- "$PWD"
  fi
fi
cargo build -q --release -p gitterrace
pkill -f "target/release/git-terrace .*$ADDR" 2>/dev/null || true
sleep 0.3
nohup ./target/release/git-terrace "$@" --addr "$ADDR" >"$LOG" 2>&1 &
for _ in $(seq 1 50); do
  curl -s -o /dev/null -m 1 "$ADDR"/ && { echo "preview up: http://$ADDR ($(git rev-parse --short HEAD)), serving: $*"; exit 0; }
  sleep 0.2
done
echo "preview did not come up; see $LOG" >&2; exit 1
