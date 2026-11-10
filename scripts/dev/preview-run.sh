#!/usr/bin/env bash
# Rebuild git-terrace and restart the preview server, run by `make preview`. The address is
# PREVIEW_ADDR, else DEFAULT_ADDR from the config schema. PREVIEW_REPOS="a b" picks the repos,
# default this checkout. The log goes to /tmp/gt-<port>.log.
set -euo pipefail
cd "$(dirname "$0")/../.."
schema=crates/gitterrace-config/src/schema.rs
addr=${PREVIEW_ADDR:-$(sed -n 's/.*DEFAULT_ADDR: &str = "\(.*\)";/\1/p' "$schema")}
log=/tmp/gt-${addr##*:}.log
cargo build -q --release -p gitterrace
pkill -f "target/release/git-terrace .*$addr" 2>/dev/null || true
sleep 0.3
# shellcheck disable=SC2086  # PREVIEW_REPOS is a space-separated list of repos
nohup ./target/release/git-terrace ${PREVIEW_REPOS:-$PWD} --addr "$addr" >"$log" 2>&1 &
for _ in $(seq 1 50); do
  if curl -s -o /dev/null -m 1 "$addr"/; then
    echo "preview up: http://$addr ($(git rev-parse --short HEAD))"
    exit 0
  fi
  sleep 0.2
done
echo "preview did not come up, see $log" >&2
exit 1
