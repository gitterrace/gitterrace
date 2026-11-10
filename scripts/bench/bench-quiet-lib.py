#!/usr/bin/env python3
"""Quiet-machine gate for the benchmark harness (scripts/bench/bench-run.py): load sampling and the
wait for a calm machine before each timed phase. Imported, not run."""

from __future__ import annotations

import contextlib
import http.client
import importlib
import os
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

bl = importlib.import_module("bench-lib")

Json = dict[str, Any]
run = bl.run
cpu_secs = bl.cpu_secs
stats = bl.stats
get = bl.get
site_prefix = bl.site_prefix
QUIET_SECS = 20
# A published run uses these. Anything less is a smoke run, written to the cache.
FULL_RUNS, FULL_IDLE_SECS = 5, 60
# Cold metrics (index, cold page, export) per repo: fresh server and empty cache each time.
# corpus.toml lowers it per repo (cold_samples = 1) where a cold index takes too long.
COLD_SAMPLES = 3


def max_load():
    return float(os.environ.get("BENCH_MAX_LOAD", (os.cpu_count() or 2) / 2))


# Seconds between load samples while a phase runs.
SAMPLE_SECS = 5


class LoadSampler:
    """Samples foreign CPU every SAMPLE_SECS for the phase named last by quiet().

    The guard checks load only when a phase starts, this catches work that arrives during it.
    Load average cannot be used mid-phase: a linux index alone drives it past 30. So a sample
    is the CPU, in cores, of every process outside the harness's own tree (the server and
    git it spawns). finish() gives max and mean per phase, stored as load.sampled and judged
    against the same limit as the load guard."""

    def __init__(self) -> None:
        self.phase: str | None = None
        self.samples: dict[str, list[float]] = {}
        self.stop_ev = threading.Event()
        self.lock = threading.Lock()
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    def loop(self) -> None:
        while not self.stop_ev.wait(SAMPLE_SECS):
            self.sample()

    def sample(self) -> None:
        with self.lock:
            phase = self.phase
        if phase:
            cores = foreign_cores()
            with self.lock:
                self.samples.setdefault(phase, []).append(cores)

    def enter(self, phase: str) -> None:
        with self.lock:
            self.phase = phase
        self.sample()

    def pause(self) -> None:
        """Between phases (waiting for quiet) nothing is timed, so nothing is sampled."""
        with self.lock:
            self.phase = None

    def finish(self) -> Json:
        self.stop_ev.set()
        self.thread.join()
        return {
            p: {
                "max": round(max(xs), 2),
                "mean": round(statistics.mean(xs), 2),
                "n": len(xs),
            }
            for p, xs in self.samples.items()
        }


def split_cores(rows: list[list[str]], me: int) -> tuple[float, float]:
    """(own, foreign) CPU in cores from `pid ppid pcpu` rows: own is `me` and its descendants."""
    parent = {int(p): int(pp) for p, pp, _ in rows}

    def ours(pid: int) -> bool:
        seen = 0
        while pid > 1 and seen < 64:
            if pid == me:
                return True
            pid, seen = parent.get(pid, 1), seen + 1
        return False

    own = sum(float(c) for p, _, c in rows if ours(int(p)))
    other = sum(float(c) for p, _, c in rows if not ours(int(p)))
    return own / 100, other / 100


def process_cores() -> tuple[float, float]:
    try:
        out = run(["ps", "-Ao", "pid=,ppid=,pcpu="])
    except (OSError, subprocess.CalledProcessError):
        return 0.0, 0.0
    rows = [line.split() for line in out.splitlines() if len(line.split()) == 3]
    return split_cores(rows, os.getpid())


def foreign_cores() -> float:
    """CPU in cores of every process that is not this harness or its descendants."""
    return process_cores()[1]


def load_now() -> float:
    """The 1-minute load average without the harness's own tree: its server and git count as
    load otherwise, and a run would wait for a quiet machine it is itself keeping busy."""
    return max(0.0, os.getloadavg()[0] - process_cores()[0])


# Background daemons known to skew a run. Their CPU share is recorded when above 10%.
NOISY = ("mediaanalysisd", "photoanalysisd", "mds_stores", "backupd")


def busy_background():
    """CPU percent of known noisy daemons above 10% right now, by process name."""
    try:
        out = run(["ps", "-Ao", "pcpu=,comm="])
    except (OSError, subprocess.CalledProcessError):
        return {}
    seen = {}
    for line in out.splitlines():
        pct, _, comm = line.strip().partition(" ")
        name = Path(comm.strip()).name
        if name in NOISY and float(pct) > 10:
            seen[name] = seen.get(name, 0) + float(pct)
    return seen


def quiet(
    phase: str, loads: Json, wait: bool = True, sampler: Any | None = None
) -> None:
    """Wait until the machine is quiet, then record the load the phase starts under."""
    if sampler:
        sampler.pause()
    limit, calm, start = max_load(), 0, time.monotonic()
    while wait and calm < QUIET_SECS:
        if time.monotonic() - start > 1800:
            sys.exit(
                f"machine not quiet for 30 min before {phase} (load limit {limit}), not timing"
            )
        if load_now() < limit:
            calm += 1
        else:
            if calm == 0 or calm % 60 == 0:
                print(
                    f"  waiting for load {load_now():.1f} < {limit:.1f} before {phase}",
                    flush=True,
                )
            calm = 0
        time.sleep(1)
    loads[phase] = round(load_now(), 2)
    noise = busy_background()
    if noise:
        loads.setdefault("background", {})[phase] = noise
    if sampler:
        sampler.enter(phase)


def measure_pages(port: int, plan: dict[str, str], runs: int) -> Json:
    """Cold and warm timings per page. Anything but 200, a redirect included, fails the run."""
    out: Json = {}
    prefix = site_prefix(port)
    for label, page in plan.items():
        path = prefix + page
        status, ttfb, total, size = get(port, path)
        if status != 200:
            raise RuntimeError(f"{label} {path} answered HTTP {status}, not 200")
        cold = {"ttfb_ms": ttfb, "total_ms": total}
        ttfbs, totals = [], []
        for _ in range(runs):
            status, a, b, _ = get(port, path)
            if status != 200:
                raise RuntimeError(f"{label} {path} answered HTTP {status}, not 200")
            ttfbs.append(a)
            totals.append(b)
        out[label] = {
            "path": path,
            "status": status,
            "bytes": size,
            "cold": cold,
            "warm_ttfb_ms": stats(ttfbs),
            "warm_total_ms": stats(totals),
        }
        print(
            f"  {label:<22} {status} cold {total:8.0f} ms  warm {statistics.median(totals):7.0f} ms",
            flush=True,
        )
    return out


CONTROL_MIN_PCT = 50
_control_done = False


def cpu_control(secs: float = 1.5) -> float:
    """CPU percent the probe reads for a process that spins; it must read high.

    A probe with the wrong pid or a cpu_secs stuck at 0 reports an idle server and passes, so
    the idle numbers are trusted only after a known busy loop reads above CONTROL_MIN_PCT."""
    spin = "while True: pass"
    p = subprocess.Popen([sys.executable, "-c", spin])
    try:
        time.sleep(0.3)
        a, t = cpu_secs(p.pid), time.perf_counter()
        time.sleep(secs)
        b, t2 = cpu_secs(p.pid), time.perf_counter()
    finally:
        p.kill()
        p.wait()
    return 100 * (b - a) / (t2 - t)


def require_control() -> None:
    global _control_done
    if _control_done:
        return
    pct = cpu_control()
    if pct < CONTROL_MIN_PCT:
        raise RuntimeError(
            f"idle CPU probe is broken: a busy loop read {pct:.0f}%, under "
            f"{CONTROL_MIN_PCT}%, so a 0% server reading would mean nothing"
        )
    _control_done = True


def idle_cpu(server: Any, secs: int, with_events: bool) -> float:
    require_control()
    stop = threading.Event()
    if with_events:

        def hold():
            try:
                c = http.client.HTTPConnection("127.0.0.1", server.port, timeout=5)
                c.request("GET", f"{site_prefix(server.port)}/events")
                r = c.getresponse()
                while not stop.is_set():
                    with contextlib.suppress(OSError):
                        r.fp.readline()
            except OSError:
                pass

        threading.Thread(target=hold, daemon=True).start()
        time.sleep(1)
    a, t = cpu_secs(server.pid), time.perf_counter()
    time.sleep(secs)
    b, t2 = cpu_secs(server.pid), time.perf_counter()
    stop.set()
    return 100 * (b - a) / (t2 - t)


def self_test() -> int:
    """The control passes a probe that sees the spinner and fails one stuck at 0."""
    global cpu_secs, _control_done
    real = cpu_secs
    verdicts = []
    try:
        for probe in (lambda _pid: time.perf_counter(), lambda _pid: 0.0):
            cpu_secs, _control_done = probe, False
            try:
                require_control()
                verdicts.append(True)
            except RuntimeError as e:
                verdicts.append("probe is broken" not in str(e))
    finally:
        cpu_secs = real
        _control_done = False
    ok = verdicts == [True, False]
    print("self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(self_test())
