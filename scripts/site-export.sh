#!/usr/bin/env bash
# Build the click-around demo for demo.gitterrace.dev: a static export of well-known public
# repositories, each at a release tag, the way `git terrace` serves a folder of repos (an index,
# then each repository under /r/<name>/). The list is demo/repos.txt. Each repository is checked
# out on a branch named after its tag, so pages show a branch, not "HEAD is detached".
#
#   scripts/site-export.sh [OUT] [COMMITS]   default OUT=target/demo, COMMITS=300
set -euo pipefail
cd "$(dirname "$0")/.."
out="${1:-target/demo}"
commits="${2:-300}"
case "$out" in target/*) ;; *) echo "OUT must be under target/ (it is replaced)"; exit 2 ;; esac

bin="${TEST_BIN:-./target/release/git-terrace}"
[ -n "${TEST_BIN:-}" ] || cargo build -q --release -p gitterrace
src="$(mktemp -d)"
trap 'rm -rf "$src"' EXIT
mkdir -p "$src/repos"
names=()
credits=()
while read -r name url tag; do
  case "$name" in ''|'#'*) continue ;; esac
  # A full clone: gitterrace reads objects locally and never fetches missing blobs.
  git clone -q --no-checkout "$url" "$src/repos/$name"
  git -C "$src/repos/$name" checkout -q -B "$tag" "refs/tags/$tag"
  names+=("$name@$tag")
  credits+=(--project "$name=${url%.git}")
done < demo/repos.txt
[ "${#names[@]}" -gt 0 ] || { echo "demo/repos.txt lists no repositories"; exit 1; }

rm -rf "$out"
"$bin" export "$out" "$src/repos" --commits "$commits" --no-actions --no-watch \
  --config "$src/cfg" "${credits[@]}"
cp demo/_headers "$out/_headers"
echo "demo: ${names[*]}, $(find "$out" -type f | wc -l | tr -d ' ') files, $(du -sh "$out" | cut -f1) in $out"
