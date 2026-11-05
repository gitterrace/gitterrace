#!/usr/bin/env bash
# Two proofs over the equivalence goldens, with distinct names and outputs.
#
# change OLD NEW: the re-bless proof. Renders the golden cell set (equivalence and revisions)
# at both commits, then compares old against new per cell: unchanged, chrome-only, tie-only,
# REAL, only-old, only-new. Each REAL cell is listed with its first differing line. Writes
# $GOLDEN_PROOF_DIR/change-<old>-<new>.txt and exits non-zero on any REAL or one-sided cell.
# This is the proof a re-bless cites.
#
# completeness [COMMIT] [FULL-GOLDEN]: at one commit, the pairwise golden against the full
# matrix. Says whether the pairwise set loses cells ("lost"), never whether output changed
# between commits. Writes $GOLDEN_PROOF_DIR/completeness-<commit>.txt.
#
#   scripts/golden-check.sh change OLD NEW
#   scripts/golden-check.sh completeness [commit] [full-golden]
#   scripts/golden-check.sh --self-test   (a planted change in one cell must report REAL 1)
set -euo pipefail
cd "$(dirname "$0")/.."
# The oracle crate and test target come from config/schwarzwald/land.toml (tomllib, python 3.11+).
py=""
for c in python3 python3.15 python3.14 python3.13 python3.12 python3.11; do
  if command -v "$c" >/dev/null && "$c" -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then
    py=$c
    break
  fi
done
[[ -n $py ]] || {
  echo "golden-check: needs python 3.11 or newer" >&2
  exit 1
}
oracle_cfg() {
  "$py" -c 'import sys, tomllib; print(tomllib.load(open("config/schwarzwald/land.toml", "rb"))["oracle"][sys.argv[1]])' "$1"
}
oracle_crate=$(oracle_cfg crate)
oracle_test=$(oracle_cfg test)
# cargo runs the test from the crate dir, so the proof path must not depend on the cwd.
resolve_out() {
  mkdir -p "$1"
  (cd "$1" && pwd)
}
out=$(resolve_out "${GOLDEN_PROOF_DIR:-target/golden-proof}")
target=$(resolve_out "${CARGO_TARGET_DIR:-target}")

# Runs the change classifier over two dump folders, writes the report to $3.
classify() {
  TEST_CHANGE_OLD=$1 TEST_CHANGE_NEW=$2 TEST_CHANGE_REPORT=$3 TEST_CHANGE_HEAD=${4:-} \
    cargo test --release -q -p "$oracle_crate" --test "$oracle_test" \
    a_rebless_proof_classifies_old_against_new -- --exact >/dev/null 2>&1 || true
  # A build that lacks the test writes nothing: that must never read as a pass.
  [[ -s $3 ]] || {
    echo "golden-check: the classifier wrote no report" >&2
    exit 1
  }
  ! grep -q '^  \(real\|only-old\|only-new\) ' "$3"
}

# Dumps every golden answer of COMMIT into folder $2, from a detached worktree.
dump_at() {
  local wt
  wt=$(mktemp -d)/wt
  git worktree add -q --detach "$wt" "$1"
  for t in every_cell_matches_the_oracle other_revisions_match_the_oracle; do
    # The golden check itself may fail at either commit; the dump is written before it.
    # A separate target dir: the old build must not replace the test binary this script
    # classifies with, which cargo would otherwise reuse as fresh.
    (cd "$wt" && CARGO_TARGET_DIR=$target/golden-check TEST_ORACLE_DUMP=$2 cargo test \
      --release -q -p "$oracle_crate" --test "$oracle_test" "$t" -- --exact >/dev/null 2>&1) || true
  done
  git worktree remove --force "$wt"
  [[ -n $(ls "$2") ]] || {
    echo "golden-check: no answers dumped at $1" >&2
    exit 1
  }
}

case ${1:-} in
--self-test)
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  mkdir "$tmp/old" "$tmp/new"
  cell() { printf '%s\n<html><header>%s</header>\n<main>\n<p>%s commits</p>\n</main></html>' "$2" "$3" "$4" >"$1"; }
  cell "$tmp/old/a" "[as-is] /stats?period=1m" h 42
  cell "$tmp/old/b" "[as-is] /stats?period=1w" h 7
  cell "$tmp/new/a" "[as-is] /stats?period=1m" moved 42
  cell "$tmp/new/b" "[as-is] /stats?period=1w" h 8
  if classify "$tmp/old" "$tmp/new" "$tmp/r.txt"; then
    echo "self-test: a planted body change passed the re-bless proof" >&2
    exit 1
  fi
  if ! grep -q "chrome-only 1, tie-only 0, REAL 1," "$tmp/r.txt" ||
    ! grep -q "^  real \[as-is\] /stats?period=1w" "$tmp/r.txt"; then
    echo "self-test: the planted change is not reported as REAL:" >&2
    cat "$tmp/r.txt" >&2
    exit 1
  fi
  echo "golden-check self-test: ok (planted change reported REAL 1)"
  ;;
change)
  old=$(git rev-parse --short "$2")
  new=$(git rev-parse --short "$3")
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  dump_at "$old" "$work/old"
  dump_at "$new" "$work/new"
  report="$out/change-$old-$new.txt"
  rc=0
  classify "$work/old" "$work/new" "$report" "re-bless proof, $old -> $new" || rc=1
  sed -n 2p "$report"
  grep -A2 '^  real ' "$report" | head -30 || true
  echo "change report: $report"
  exit "$rc"
  ;;
completeness)
  commit=$(git rev-parse --short "${2:-HEAD}")
  export TEST_GOLDEN_PROOF="$out/completeness-$commit.txt"
  if [[ -n ${3:-} ]]; then
    TEST_GOLDEN_FULL=$(realpath "$3")
    export TEST_GOLDEN_FULL
  fi
  cargo test --release -p "$oracle_crate" --test "$oracle_test" \
    the_pairwise_golden_agrees_with_the_full_matrix -- --exact --nocapture
  echo "completeness proof (not a re-bless proof): $TEST_GOLDEN_PROOF"
  ;;
*)
  echo "usage: golden-check.sh change OLD NEW | completeness [commit] [full-golden] | --self-test" >&2
  exit 2
  ;;
esac
