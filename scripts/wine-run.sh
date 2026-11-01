#!/usr/bin/env bash
# Build git-terrace for Windows and run scripts/smoke-check.sh against it under Wine in Docker.
# Needs docker and cargo-xwin. Usage: scripts/wine-run.sh [--no-build].
# The target follows the host: aarch64-pc-windows-msvc on arm64 (arm64 Wine, native speed),
# x86_64-pc-windows-msvc elsewhere. Wine is never installed on the host.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
case "$(uname -m)" in
arm64 | aarch64) target=aarch64-pc-windows-msvc ;;
*) target=x86_64-pc-windows-msvc ;;
esac
tdir=${CARGO_TARGET_DIR:-$root/target}
if [ "${1:-}" != --no-build ]; then
  CARGO_TARGET_DIR=$tdir cargo xwin build --release --target "$target" -p gitterrace
fi
exe="$tdir/$target/release/git-terrace.exe"
[ -f "$exe" ] || {
  echo "missing $exe" >&2
  exit 1
}
docker build -q -t gitterrace-wine -f "$root/ci/wine.Dockerfile" "$root/ci" >/dev/null
docker run --rm \
  -v "$exe:/work/git-terrace.exe:ro" \
  -v "$root/scripts/smoke-check.sh:/work/smoke-check.sh:ro" \
  -e GT_WINE=1 gitterrace-wine \
  bash -c 'wineboot -i >/dev/null 2>&1; bash /work/smoke-check.sh /work/git-terrace.exe 4261'
