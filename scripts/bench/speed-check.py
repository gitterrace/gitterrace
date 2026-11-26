#!/usr/bin/env python3
"""Speed gates on the bench clones: fails when a cold insights page is over its gate.

Cold is the first request to a page on a server just started over the built index, so the
page memo is empty and the visitor pays for the tables the page needs. Each sample is its
own server; the gate is on the median. The 1-minute load is printed next to every timing, and
a run above --max-load waits for a quieter machine first, as the bench does.

  scripts/bench/speed-check.py --bin PATH [--repos rust,linux] [--runs 3] [--max-load N]
                               [--only a,b]
  scripts/bench/speed-check.py --self-test     # the verdict logic, no server

Each repository is the bench clone of that name ($GITTERRACE_BENCH_DIR/<name>.git), with its
own gates. It must be indexed already: the check times pages, not the index build.
"""

from __future__ import annotations

import argparse
import importlib
import os
import shutil
import statistics
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
bl = importlib.import_module("bench-lib")

# Page and path. Every one is a page a visitor opens first after a repository turns ready.
PAGES: dict[str, str] = {
    "languages": "/stats/languages",
    "hotspots-all": "/stats/hotspots?period=all",
    "hotspots-1y": "/stats/hotspots?period=1y",
    "hotspots-90d": "/stats/hotspots?period=90d",
    "ownership-all": "/stats/ownership?period=all",
    "ownership-1y": "/stats/ownership?period=1y",
    "overview": "/",
    "stats-overview": "/stats",
    "contributors": "/stats/contributors",
}
# Repository, page, gate in ms. Languages is the absolute goal. Hotspots and ownership hold
# 1.5x the time of building them in memory (docs/benchmarks.md, "Insights pages on a large
# history"). The warm-up renders the rest before ready, so their gate is a memo hit's.
WARM_HIT_MS = 300
GATES: dict[str, dict[str, float]] = {
    "rust": {
        "languages": 66,
        "hotspots-all": 230,
        "hotspots-1y": WARM_HIT_MS,
        "hotspots-90d": WARM_HIT_MS,
        "ownership-all": 115,
        "ownership-1y": 115,
        "overview": WARM_HIT_MS,
        "stats-overview": WARM_HIT_MS,
        "contributors": WARM_HIT_MS,
    },
    "linux": {
        "hotspots-1y": WARM_HIT_MS,
        "hotspots-90d": WARM_HIT_MS,
        "overview": WARM_HIT_MS,
        "stats-overview": WARM_HIT_MS,
        "contributors": WARM_HIT_MS,
    },
}
# A page that has not answered by then is over every gate; the run moves on.
PAGE_TIMEOUT_SECS = 900


def wait_load(limit: float) -> float:
    while (load := os.getloadavg()[0]) > limit:
        time.sleep(5)
    return load


def verdict(repo: str, samples: dict[str, list[float]]) -> list[tuple[str, float, float, bool]]:
    """(page, median ms, gate ms, over) per page of `repo` that has samples."""
    out = []
    for name, ms in samples.items():
        if ms:
            med = statistics.median(ms)
            gate = GATES[repo][name]
            out.append((name, med, gate, med > gate))
    return out


def measure(binary: str, repo: Path, names: list[str], runs: int, max_load: float):
    # A private directory: a fixed shared name is guessable and guessable.
    logdir = Path(tempfile.mkdtemp(prefix="gitterrace-speed-check-"))
    try:
        return sample(binary, repo, names, runs, max_load, logdir)
    finally:
        shutil.rmtree(logdir, ignore_errors=True)


def sample(binary: str, repo: Path, names: list[str], runs: int, max_load: float, logdir: Path):
    samples: dict[str, list[float]] = {n: [] for n in names}
    for run in range(runs):
        for name in names:
            load = wait_load(max_load)
            srv = bl.Server(binary, repo, logdir)
            try:
                srv.wait_ready()
                prefix = bl.site_prefix(srv.port)
                status, _, ms, _ = bl.get(srv.port, prefix + PAGES[name], PAGE_TIMEOUT_SECS)
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
    v = {n: (m, o) for n, m, _, o in verdict("rust", {"languages": [70, 50, 60]})}
    assert v == {"languages": (60, False)}, v
    v = {n: o for n, _, _, o in verdict("rust", {"hotspots-all": [231, 240, 10]})}
    assert v == {"hotspots-all": True}, v
    v = {n: o for n, _, _, o in verdict("linux", {"overview": [3833, 3800, 3900]})}
    assert v == {"overview": True}, v
    assert verdict("rust", {"ownership-all": []}) == []
    # Every gate names a known page, and every page is gated on rust.
    for repo, gates in GATES.items():
        assert set(gates) <= set(PAGES), (repo, set(gates) - set(PAGES))
    assert set(GATES["rust"]) == set(PAGES), set(PAGES) - set(GATES["rust"])
    # No shared, guessable temp path: logs go to a mkdtemp directory.
    src = Path(__file__).read_text(encoding="utf-8")
    assert "mkdtemp(" in src and "/" + "tmp" not in src, "use a private temp dir"
    print("speed-check self-test ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    ap.add_argument("--bin")
    bench = Path(os.environ.get("GITTERRACE_BENCH_DIR", Path.home() / ".cache/gitterrace-bench"))
    ap.add_argument("--repos", default=",".join(GATES))
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--max-load", type=float, default=float(os.cpu_count() or 2))
    ap.add_argument("--only", default="")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.bin:
        ap.error("--bin is required")
    # The server runs with the bench clone as cwd, so a relative --bin would not resolve there.
    a.bin = str(Path(a.bin).resolve())
    only = [n for n in a.only.split(",") if n]
    unknown = [n for n in only if n not in PAGES]
    if unknown:
        sys.exit(f"unknown page {unknown}, known: {', '.join(PAGES)}")
    over = []
    for repo in [r for r in a.repos.split(",") if r]:
        if repo not in GATES:
            sys.exit(f"no gates for {repo}, known: {', '.join(GATES)}")
        path = bench / f"{repo}.git"
        if not (path / "gitterrace" / "MANIFEST").exists():
            sys.exit(f"{path} has no built index: serve it once first")
        names = [n for n in GATES[repo] if not only or n in only]
        for name, med, gate, bad in verdict(repo, measure(a.bin, path, names, a.runs, a.max_load)):
            verdict_word = "OVER" if bad else "ok"
            print(f"{repo:6} {name:14} median {med:10.1f} ms  gate {gate:5.0f} ms  {verdict_word}")
            if bad:
                over.append(f"{repo}/{name}")
    if over:
        print(f"speed gates over: {', '.join(over)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
