#!/usr/bin/env bash
# Records what the tests of one revision cover, for the before/after proof coverage-check.py reads:
#
#   coverage-run.sh REV OUT    writes OUT/lcov.info (lines and branches) and, with MUTANTS_PKG
#                              and MUTANTS_FILES set, OUT/mutants.out from cargo-mutants
#
# REV is checked out in a fresh clone under target/coverage/, so the caller's checkout is never
# touched. Branch coverage needs a nightly compiler (`-Z coverage-options=branch` is refused on
# the pinned stable), so both sides of a proof run on the same NIGHTLY. cargo-mutants runs on the
# pinned toolchain. The release-only tests (memory budget, latency) are not in this run: their
# own release gates keep them.
#
#   NIGHTLY        toolchain for llvm-cov (default nightly)
#   MUTANTS_PKG    crate to mutate, e.g. gitterrace-web
#   MUTANTS_FILES  space-separated globs of the src files whose tests moved
set -euo pipefail
cd "$(dirname "$0")/../.."
rev=${1:?usage: coverage-run.sh REV OUT}
out=${2:?usage: coverage-run.sh REV OUT}
nightly=${NIGHTLY:-nightly}
root=$PWD/target/coverage
src=$root/src-$(git rev-parse --short "$rev")
mkdir -p "$out" "$root"
out=$(cd "$out" && pwd)

if [[ ! -d $src ]]; then
  git clone --quiet --no-local . "$src"
fi
git -C "$src" checkout --quiet --detach "$rev"

# One target dir for both revisions: dependencies build once.
export CARGO_TARGET_DIR=$root/target
(
  cd "$src"
  cargo "+$nightly" llvm-cov clean --workspace
  cargo "+$nightly" llvm-cov --workspace --all-features --branch --no-fail-fast \
    --lcov --output-path "$out/lcov.info"
)
# Paths in the report name this clone: strip it so two revisions compare file by file.
sed -i.bak "s|^SF:$src/|SF:|" "$out/lcov.info" && mv "$out/lcov.info.bak" "$root/"
git -C "$src" rev-parse HEAD >"$out/rev"
# Where each src file's inline tests start: the compare counts only the lines above, so a moved
# test never reads as lost product coverage.
(cd "$src" && git ls-files 'crates/*/src/*.rs' | while read -r f; do
  n=$(grep -n -m1 '^#\[cfg(test)\]' "$f" | cut -d: -f1 || true)
  [[ -z $n ]] || printf '%s\t%s\n' "$f" "$n"
done) >"$out/tests-start.tsv"

if [[ -n ${MUTANTS_PKG:-} ]]; then
  read -r -a globs <<<"${MUTANTS_FILES:?set the src globs to mutate}"
  file_args=()
  for g in "${globs[@]}"; do file_args+=(--file "$g"); done
  (
    cd "$src"
    unset CARGO_TARGET_DIR
    # Exit 2 means some mutants were missed: the compare decides, not this run.
    cargo mutants --package "$MUTANTS_PKG" "${file_args[@]}" --timeout-multiplier 3 \
      --jobs 4 --output "$out" || [[ $? == 2 ]]
  )
fi
echo "coverage-run: $rev -> $out"
