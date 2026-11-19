#!/usr/bin/env bash
# The bench harness's red cases, run by `make bench-guard`: an index, or a time to ready, over
# its budget fails the synthetic run, and the failure names its slowest phase.
#   scripts/bench/bench-budget-check.sh <git-terrace binary>
set -euo pipefail
cd "$(dirname "$0")/../.."
bin=${1:?usage: bench-budget-check.sh <git-terrace binary>}
if out=$(TEST_BENCH_INDEX_SECS=0 python3 scripts/bench/bench-run.py --synthetic --bin "$bin"); then
  echo "bench-budget-check: an index over budget passed the run" >&2
  exit 1
fi
grep -qE 'INDEX OVER BUDGET .*slowest phase: [a-z_]+' <<<"$out" || {
  echo "bench-budget-check: the over-budget run did not name its slowest phase" >&2
  exit 1
}
if out=$(TEST_BENCH_READY_SECS=0 python3 scripts/bench/bench-run.py --synthetic --bin "$bin"); then
  echo "bench-budget-check: a time to ready over budget passed the run" >&2
  exit 1
fi
grep -qE 'INDEX OVER BUDGET .*ready_secs .*slowest phase: [a-z_]+' <<<"$out" || {
  echo "bench-budget-check: the over-budget ready did not name its slowest phase" >&2
  exit 1
}
echo "bench-budget-check: ok"
