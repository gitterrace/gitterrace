#!/usr/bin/env bash
# Proves at one commit that the pairwise equivalence golden loses nothing against the full
# matrix: runs both cell sets, classifies every shared key (chrome-only, tie-only, real) and
# writes $GOLDEN_PROOF_DIR/<commit>.txt (default target/golden-proof). Real must be 0.
# Optional second argument: an older full golden to compare the full run to.
#
#   scripts/golden-check.sh [commit] [full-golden]
set -euo pipefail
commit=$(git rev-parse --short "${1:-HEAD}")
out=${GOLDEN_PROOF_DIR:-target/golden-proof}
mkdir -p "$out"
export TEST_GOLDEN_PROOF="$out/$commit.txt"
if [[ -n ${2:-} ]]; then
  TEST_GOLDEN_FULL=$(realpath "$2")
  export TEST_GOLDEN_FULL
fi
cargo test --release -p gitterrace-web --test equivalence_pages \
  the_pairwise_golden_agrees_with_the_full_matrix -- --exact --nocapture
echo "proof: $TEST_GOLDEN_PROOF"
