#!/usr/bin/env bash
# Proves at one commit that the pairwise equivalence golden loses nothing against the full
# matrix: runs both cell sets, classifies every shared key (chrome-only, tie-only, real) and
# writes $GOLDEN_PROOF_DIR/<commit>.txt (default target/golden-proof). Real must be 0.
# Optional second argument: an older full golden to compare the full run to.
#
#   scripts/golden-check.sh [commit] [full-golden]
#   scripts/golden-check.sh --self-test    (a relative GOLDEN_PROOF_DIR must resolve absolute)
set -euo pipefail
# cargo runs the test from the crate dir, so the proof path must not depend on the cwd.
resolve_out() {
  mkdir -p "$1"
  (cd "$1" && pwd)
}
if [[ ${1:-} == --self-test ]]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  got=$(cd "$tmp" && resolve_out rel/proof)
  [[ $got == /* && -d $got ]] || {
    echo "self-test: not absolute: $got" >&2
    exit 1
  }
  echo "self-test ok: $got"
  exit 0
fi
commit=$(git rev-parse --short "${1:-HEAD}")
out=$(resolve_out "${GOLDEN_PROOF_DIR:-target/golden-proof}")
export TEST_GOLDEN_PROOF="$out/$commit.txt"
if [[ -n ${2:-} ]]; then
  TEST_GOLDEN_FULL=$(realpath "$2")
  export TEST_GOLDEN_FULL
fi
cargo test --release -p gitterrace-web --test equivalence_pages \
  the_pairwise_golden_agrees_with_the_full_matrix -- --exact --nocapture
echo "proof: $TEST_GOLDEN_PROOF"
