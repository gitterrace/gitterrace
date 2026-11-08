#!/usr/bin/env python3
"""Speed gates on the rust bench clone: fails when a cold insights page is over its gate.

Cold is the first request to a page on a server just started over the built index, so the
page memo is empty and the visitor pays for the tables the page needs. Each sample is its
own server; the gate is on the median. The 1-minute load is printed next to every timing, and
a run above --max-load waits for a quieter machine first, as the bench does.

  scripts/speed-check.py --bin PATH [--repo DIR] [--runs 3] [--max-load N] [--only a,b]
  scripts/speed-check.py --self-test     # the verdict logic, no server

The repository defaults to the bench clone ($GITTERRACE_BENCH_DIR/rust.git). It must be
indexed already: the check times pages, not the index build.
"""

from __future__ import annotations

import argparse
import importlib
import os
import statistics
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
bl = importlib.import_module("bench-lib")

# Page, gate in ms. Languages is the absolute goal; hotspots and ownership hold the 1.5x of
# the in-memory build they replaced (docs/benchmarks.md, "File pages on a large history").
GATES: dict[str, tuple[str, float]] = {
    "languages": ("/stats/languages", 66),
    "hotspots-all": ("/stats/hotspots?period=all", 230),
    "hotspots-1y": ("/stats/hotspots?period=1y", 230),
    "ownership-all": ("/stats/ownership?period=all", 115),
    "ownership-1y": ("/stats/ownership?period=1y", 115),
}
# A page that has not answered by then is over every gate; the run moves on.
PAGE_TIMEOUT_SECS = 900


def wait_load(limit: float) -> float:
    while (load := os.getloadavg()[0]) > limit:
        time.sleep(5)
    return load


def verdict(samples: dict[str, list[float]]) -> list[tuple[str, float, float, bool]]:
    """(page, median ms, gate ms, over) per page that has samples."""
    out = []
    for name, ms in samples.items():
        if ms:
            med = statistics.median(ms)
            gate = GATES[name][1]
            out.append((name, med, gate, med > gate))
    return out


def measure(binary: str, repo: Path, names: list[str], runs: int, max_load: float):
    logdir = Path(os.environ.get("TMPDIR", "/tmp")) / "gitterrace-speed-check"
    logdir.mkdir(parents=True, exist_ok=True)
    samples: dict[str, list[float]] = {n: [] for n in names}
    for run in range(runs):
        for name in names:
            load = wait_load(max_load)
            srv = bl.Server(binary, repo, logdir)
            try:
                srv.wait_ready()
                prefix = bl.site_prefix(srv.port)
                status, _, ms, _ = bl.get(
                    srv.port, prefix + GATES[name][0], PAGE_TIMEOUT_SECS
                )
            except TimeoutError:
                status, ms = 0, PAGE_TIMEOUT_SECS * 1000.0
            finally:
                srv.stop()
            if status not in (0, 200):
                sys.exit(f"{name}: HTTP {status}")
            samples[name].append(ms)
            print(f"run {run + 1} {name:14} {ms:10.1f} ms  load {load:.1f}", flush=True)
    return samples


def self_test() -> int:
    v = dict((n, (m, over)) for n, m, _, over in verdict({"languages": [70, 50, 60]}))
    assert v == {"languages": (60, False)}, v
    v = dict((n, over) for n, _, _, over in verdict({"hotspots-1y": [231, 240, 10]}))
    assert v == {"hotspots-1y": True}, v
    assert verdict({"ownership-all": []}) == []
    print("speed-check self-test ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    ap.add_argument("--bin")
    bench = Path(
        os.environ.get("GITTERRACE_BENCH_DIR", Path.home() / ".cache/gitterrace-bench")
    )
    ap.add_argument("--repo", type=Path, default=bench / "rust.git")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--max-load", type=float, default=float(os.cpu_count() or 2))
    ap.add_argument("--only", default="")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.bin:
        ap.error("--bin is required")
    if not (a.repo / "gitterrace" / "MANIFEST").exists():
        sys.exit(f"{a.repo} has no built index: serve it once first")
    names = [n for n in a.only.split(",") if n] or list(GATES)
    unknown = [n for n in names if n not in GATES]
    if unknown:
        sys.exit(f"unknown gate {unknown}, known: {', '.join(GATES)}")
    over = []
    for name, med, gate, bad in verdict(
        measure(a.bin, a.repo, names, a.runs, a.max_load)
    ):
        print(
            f"{name:14} median {med:10.1f} ms  gate {gate:5.0f} ms  {'OVER' if bad else 'ok'}"
        )
        if bad:
            over.append(name)
    if over:
        print(f"speed gates over: {', '.join(over)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
