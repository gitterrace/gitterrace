#!/usr/bin/env bash
# The bench harness's red cases, run by `make bench-guard`: an index, or a time to ready, over
# its budget fails the synthetic run, and the failure names its slowest phase. The run samples
# the index child's footprint.
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
# The index build runs in a child, so the bench must find a server's children. A sleeping
# child stands in for it: the synthetic index ends before the first sample.
python3 - <<'PY' || {
import importlib.util, subprocess, sys
spec = importlib.util.spec_from_file_location("bl", "scripts/bench/bench-lib.py")
bl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bl)
parent = subprocess.Popen(["sh", "-c", "sleep 5 & wait"])
try:
    for _ in range(50):
        if bl.child_pids(parent.pid):
            break
        __import__("time").sleep(0.1)
    else:
        sys.exit(1)
finally:
    parent.kill()
PY
  echo "bench-budget-check: the bench found no child of a server" >&2
  exit 1
}
grep -qE 'index child [0-9]+ MB' <<<"$out" || {
  echo "bench-budget-check: the run did not report the index child footprint" >&2
  exit 1
}
echo "bench-budget-check: ok"
