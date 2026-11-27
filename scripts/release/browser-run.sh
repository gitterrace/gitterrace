#!/usr/bin/env bash
# The release gate's local browser step: a release build, then make browser-check on every
# browser and case. Skipped with a loud note only when the browser deps are missing (run
# make browser-deps); hosted CI runs the same checks in its `browser` job either way.
set -euo pipefail

if [ ! -d node_modules/playwright-core ]; then
  echo "release-gate: SKIPPED browser-check: node_modules/playwright-core is missing." >&2
  echo "release-gate: SKIPPED browser-check: run 'make browser-deps' to run it here." >&2
  exit 0
fi
cargo build --release --locked -p gitterrace
make browser-check BIN=target/release/git-terrace
