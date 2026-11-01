#!/usr/bin/env bash
# Build the click-around demo for demo.gitterrace.dev: a static export of this repository at
# its newest tag, so the demo always shows a release. Without any tag it falls back to HEAD.
# Exports from a fresh clone, so the demo never shows the maintainer's working-tree changes,
# local branches or stashes.
#
#   scripts/site-export.sh [OUT] [COMMITS]   default OUT=target/demo, COMMITS=300
set -euo pipefail
cd "$(dirname "$0")/.."
out="${1:-target/demo}"
commits="${2:-300}"
case "$out" in target/*) ;; *) echo "OUT must be under target/ (it is replaced)"; exit 2 ;; esac

rev="$(git describe --tags --abbrev=0 2>/dev/null || true)"
if [ -z "$rev" ]; then
  rev="$(git rev-parse --abbrev-ref HEAD)"
  echo "demo: no tag yet, exporting $rev"
fi

cargo build -q --release -p gitterrace
src="$(mktemp -d)"
trap 'rm -rf "$src"' EXIT
git clone -q --no-local --branch "$rev" . "$src/gitterrace" 2>/dev/null \
  || { git clone -q --no-local . "$src/gitterrace" && git -C "$src/gitterrace" checkout -q "$rev"; }

rm -rf "$out"
./target/release/git-terrace export "$out" "$src/gitterrace" --commits "$commits" --no-actions --no-watch \
  --config "$src/cfg"
cp demo/_headers "$out/_headers"
echo "demo: $rev, $(find "$out" -type f | wc -l | tr -d ' ') files, $(du -sh "$out" | cut -f1) in $out"
