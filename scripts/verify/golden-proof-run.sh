#!/usr/bin/env bash
# The two product steps the shared golden tool (vendor/schwarzwald/kit/tools/golden.py) calls for a
# re-bless proof, kept here because they name this product's oracle test:
#
#   golden-proof-run.sh dump              writes every golden answer into $GOLDEN_DUMP
#   golden-proof-run.sh classify OLD NEW REPORT   classifies old against new, exits non-zero on any
#                                     REAL, only-old or only-new cell
#   golden-proof-run.sh --self-test       a planted change in one cell must report REAL 1
#
# Run the proof itself with `python3 vendor/schwarzwald/kit/tools/golden.py change OLD NEW`.
set -euo pipefail
cd "$(dirname "$0")/../.."
oracle_cfg() {
  bash scripts/lib/python-run.sh -c 'import sys, tomllib
cfg = tomllib.load(open("schwarzwald/config/land.toml", "rb"))
print(cfg["oracle"][sys.argv[1]])' "$1"
}
crate=$(oracle_cfg crate)
test=$(oracle_cfg test)

classify() {
  TEST_CHANGE_OLD=$1 TEST_CHANGE_NEW=$2 TEST_CHANGE_REPORT=$3 \
    cargo test --release --no-fail-fast -q -p "$crate" --test "$test" \
    a_rebless_proof_classifies_old_against_new -- --exact >/dev/null 2>&1 || true
  # A build that lacks the test writes nothing: that must never read as a pass.
  [[ -s $3 ]] || {
    echo "golden-proof-run: the classifier wrote no report" >&2
    exit 1
  }
  ! grep -q '^  \(real\|only-old\|only-new\) ' "$3"
}

case ${1:-} in
dump)
  : "${GOLDEN_DUMP:?set by vendor/schwarzwald/kit/tools/golden.py}"
  # Its own target dir: the old build must not replace the binary classify uses.
  CARGO_TARGET_DIR=$PWD/target/golden-check TEST_ORACLE_DUMP=$GOLDEN_DUMP \
    cargo test --release --no-fail-fast -q -p "$crate" --test "$test" \
    every_cell_matches_the_oracle other_revisions_match_the_oracle -- --exact
  ;;
classify)
  classify "$2" "$3" "$4"
  ;;
--self-test)
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  mkdir "$tmp/old" "$tmp/new"
  cell() {
    printf '%s\n<html><header>%s</header>\n<main>\n<p>%s commits</p>\n</main></html>' \
      "$2" "$3" "$4" >"$1"
  }
  cell "$tmp/old/a" "[as-is] /stats?period=1m" h 42
  cell "$tmp/old/b" "[as-is] /stats?period=1w" h 7
  cell "$tmp/new/a" "[as-is] /stats?period=1m" moved 42
  cell "$tmp/new/b" "[as-is] /stats?period=1w" h 8
  if classify "$tmp/old" "$tmp/new" "$tmp/r.txt"; then
    echo "self-test: a planted body change passed the re-bless proof" >&2
    exit 1
  fi
  grep -q "chrome-only 1, tie-only 0, REAL 1," "$tmp/r.txt" || {
    echo "self-test: the planted change is not reported as REAL" >&2
    exit 1
  }
  echo "golden-proof-run self-test: ok (planted change reported REAL 1)"
  ;;
*)
  echo "usage: golden-proof-run.sh dump | classify OLD NEW REPORT | --self-test" >&2
  exit 2
  ;;
esac
