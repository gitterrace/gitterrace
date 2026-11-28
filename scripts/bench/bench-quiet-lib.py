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
# A module global, not bl.cpu_secs: the self-test swaps it for a probe.
cpu_secs = bl.cpu_secs
QUIET_SECS = 20
# A published run uses these. Anything less is a smoke run, written to the cache.
FULL_RUNS, FULL_IDLE_SECS = 5, 60


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
        self.worst: dict[str, tuple[float, float, str]] = {}
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
            procs = foreign_procs()
            total, single, name = foreign_summary(procs)
            with self.lock:
                self.samples.setdefault(phase, []).append(total / 100)
                old = self.worst.get(phase, (0.0, 0.0, ""))
                self.worst[phase] = (
                    max(old[0], total),
                    max(old[1], single),
                    name if single >= old[1] else old[2],
                )

    def enter(self, phase: str) -> None:
        with self.lock:
            self.phase = phase
        self.sample()

    def pause(self) -> None:
        """Between phases (waiting for quiet) nothing is timed, so nothing is sampled."""
        with self.lock:
            self.phase = None

    def foreign(self) -> Json:
        """Per phase: the max total and max single-process foreign CPU in percent of one core,
        and the name of the busiest process seen. Judged against TOTAL_PCT and SINGLE_PCT."""
        with self.lock:
            return {
                p: {"total_pct": round(t, 1), "single_pct": round(s, 1), "top": n}
                for p, (t, s, n) in self.worst.items()
            }

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
        out = bl.run(["ps", "-Ao", "pid=,ppid=,pcpu="])
    except (OSError, subprocess.CalledProcessError):
        return 0.0, 0.0
    rows = [line.split() for line in out.splitlines() if len(line.split()) == 3]
    return split_cores(rows, os.getpid())


# Quiet means: no foreign process over SINGLE_PCT and all of them under TOTAL_PCT, in percent of
# one core. One busy process adds about 1 to the load average of a 10 core machine, so the load
# guard alone cannot see it.
SINGLE_PCT, TOTAL_PCT = 10.0, 50.0


def foreign_procs(me: int | None = None) -> list[tuple[float, str]]:
    """(cpu percent, name) of every process outside the harness's own tree, busiest first."""
    try:
        out = bl.run(["ps", "-Ao", "pid=,ppid=,pcpu=,comm="])
    except (OSError, subprocess.CalledProcessError):
        return []
    rows = [line.split(None, 3) for line in out.splitlines()]
    return foreign_rows([r for r in rows if len(r) == 4], me or os.getpid())


def foreign_rows(rows: list[list[str]], me: int) -> list[tuple[float, str]]:
    """The foreign part of `pid ppid pcpu comm` rows, busiest first."""
    parent = {int(r[0]): int(r[1]) for r in rows}

    def ours(pid: int) -> bool:
        seen = 0
        while pid > 1 and seen < 64:
            if pid == me:
                return True
            pid, seen = parent.get(pid, 1), seen + 1
        return False

    found = [(float(r[2]), Path(r[3].strip()).name) for r in rows if not ours(int(r[0]))]
    return sorted(found, reverse=True)


def foreign_summary(procs: list[tuple[float, str]]) -> tuple[float, float, str]:
    """(total percent, busiest percent, busiest name) of a foreign_procs list."""
    if not procs:
        return 0.0, 0.0, ""
    return sum(c for c, _ in procs), procs[0][0], procs[0][1]


def over_budget(total: float, single: float) -> bool:
    return single > SINGLE_PCT or total > TOTAL_PCT


def describe(procs: list[tuple[float, str]], top: int = 3) -> str:
    label = lambda n: f"{n} (known noisy)" if n in NOISY else n  # noqa: E731
    return ", ".join(f"{label(n)} {c:.0f}%" for c, n in procs[:top]) or "none"


def foreign_cores() -> float:
    """CPU in cores of every process that is not this harness or its descendants."""
    return process_cores()[1]


def load_now() -> float:
    """The 1-minute load average without the harness's own tree: its server and git count as
    load otherwise, and a run would wait for a quiet machine it is itself keeping busy."""
    return max(0.0, os.getloadavg()[0] - process_cores()[0])


# Daemons known to skew a run. Labels in messages only: the gate is the CPU budget above.
NOISY = ("mediaanalysisd", "photoanalysisd", "mds_stores", "backupd")


def busy_background():
    """CPU percent of foreign processes above SINGLE_PCT right now, by process name. After a
    quiet wait it is empty, so an entry means the phase started over budget."""
    seen: dict[str, float] = {}
    for pct, name in foreign_procs():
        if pct > SINGLE_PCT:
            seen[name] = seen.get(name, 0) + pct
    return seen


def quiet(phase: str, loads: Json, wait: bool = True, sampler: Any | None = None) -> None:
    """Wait until the machine is quiet, then record the load the phase starts under."""
    if sampler:
        sampler.pause()
    limit, calm, start = max_load(), 0, time.monotonic()
    while wait and calm < QUIET_SECS:
        procs = foreign_procs()
        total, single, _ = foreign_summary(procs)
        if time.monotonic() - start > 1800:
            sys.exit(
                f"machine not quiet for 30 min before {phase} (load limit {limit}, "
                f"cpu budget {SINGLE_PCT:.0f}% each / {TOTAL_PCT:.0f}% total), not timing: "
                f"{describe(procs)}"
            )
        if load_now() < limit and not over_budget(total, single):
            calm += 1
        else:
            if calm == 0 or calm % 60 == 0:
                print(
                    f"  waiting before {phase}: load {load_now():.1f} (limit {limit:.1f}), "
                    f"top foreign cpu {describe(procs)}",
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


# Process names (substring, lower case) of a running VM stack that skews a timed run.
VM_NAMES = ("com.docker", "qemu", "virtualization")


def noise_sources(procs: list[str], booted: str) -> list[str]:
    """Why the machine is no place for a bench: VM processes, booted simulators. `procs` are
    process names, `booted` is the output of `xcrun simctl list devices booted`."""
    out = sorted({n for n in procs if any(v in n.lower() for v in VM_NAMES)})
    sims = [ln.strip() for ln in booted.splitlines() if "(Booted)" in ln]
    return [f"VM process {n}" for n in out] + [f"booted simulator {s}" for s in sims]


def preflight_noise() -> list[str]:
    try:
        names = [n for _, n in foreign_procs()]
    except OSError:
        names = []
    try:
        booted = bl.run(["xcrun", "simctl", "list", "devices", "booted"])
    except (OSError, subprocess.CalledProcessError):
        booted = ""
    return noise_sources(names, booted)


def measure_pages(port: int, plan: dict[str, str], runs: int) -> Json:
    """Cold and warm timings per page. Anything but 200, a redirect included, fails the run."""
    out: Json = {}
    prefix = bl.site_prefix(port)
    for label, page in plan.items():
        path = prefix + page
        status, ttfb, total, size = bl.get(port, path)
        if status != 200:
            raise RuntimeError(f"{label} {path} answered HTTP {status}, not 200")
        cold = {"ttfb_ms": ttfb, "total_ms": total}
        ttfbs, totals = [], []
        for _ in range(runs):
            status, a, b, _ = bl.get(port, path)
            if status != 200:
                raise RuntimeError(f"{label} {path} answered HTTP {status}, not 200")
            ttfbs.append(a)
            totals.append(b)
        out[label] = {
            "path": path,
            "status": status,
            "bytes": size,
            "cold": cold,
            "warm_ttfb_ms": bl.stats(ttfbs),
            "warm_total_ms": bl.stats(totals),
        }
        print(
            f"  {label:<22} {status} cold {total:8.0f} ms  warm {statistics.median(totals):7.0f} ms",
            flush=True,
        )
    return out


CONTROL_MIN_PCT = 50
# A shared runner can starve one spin for a moment; a broken probe reads low every time. So
# the bar stays and only the number of chances to clear it grows.
CONTROL_TRIES = 3
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
    reads = []
    for _ in range(CONTROL_TRIES):
        reads.append(cpu_control())
        if reads[-1] >= CONTROL_MIN_PCT:
            _control_done = True
            return
    shown = ", ".join(f"{p:.0f}%" for p in reads)
    raise RuntimeError(
        f"idle CPU probe is broken: a busy loop read {shown} in {CONTROL_TRIES} tries, under "
        f"{CONTROL_MIN_PCT}%, so a 0% server reading would mean nothing"
    )


def idle_cpu(server: Any, secs: int, with_events: bool) -> float:
    require_control()
    stop = threading.Event()
    if with_events:

        def hold():
            try:
                c = http.client.HTTPConnection("127.0.0.1", server.port, timeout=5)
                c.request("GET", f"{bl.site_prefix(server.port)}/events")
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


FOREIGN_FROM = 5


def foreign_errors(n: str, load: Json, harness: int) -> list[str]:
    """Phases that saw other processes over the CPU budget, or started beside a busy one. A run
    from before harness 5 recorded no foreign CPU: it is unchecked, not failed."""
    errs = []
    for phase, who in (load["phases"].get("background") or {}).items():
        names = ", ".join(f"{k} {v:.0f}%" for k, v in who.items())
        errs.append(f"{n}: {phase} started beside busy {names}")
    foreign = load.get("foreign")
    if foreign is None:
        if harness >= FOREIGN_FROM:
            errs.append(f"{n}: harness {harness} recorded no foreign CPU")
        else:
            print(f"note: {n} foreign CPU unchecked (harness {harness} < {FOREIGN_FROM})")
        return errs
    for phase, f in foreign.items():
        if over_budget(f["total_pct"], f["single_pct"]):
            errs.append(
                f"{n}: {phase} saw foreign CPU {f['top']} {f['single_pct']:.0f}%, total "
                f"{f['total_pct']:.0f}% (budget {SINGLE_PCT:.0f}% each, {TOTAL_PCT:.0f}%)"
            )
    return errs


def report_test() -> bool:
    """A phase beside a busy process, a background entry and an old run are judged apart."""
    ok = {"phases": {}, "foreign": {"index": {"total_pct": 12.0, "single_pct": 8.0, "top": "x"}}}
    loud = {
        "phases": {},
        "foreign": {"index#2": {"total_pct": 95.0, "single_pct": 90.0, "top": "mediaanalysisd"}},
    }
    bg = {"phases": {"background": {"index": {"mediaanalysisd": 89.9}}}, "foreign": {}}
    got = [foreign_errors("r", x, 5) for x in (ok, loud, bg)]
    if (
        got[0]
        or "mediaanalysisd" not in " ".join(got[1])
        or "mediaanalysisd" not in " ".join(got[2])
    ):
        return False
    return not foreign_errors("r", {"phases": {}}, 4) and bool(
        foreign_errors("r", {"phases": {}}, 5)
    )


def foreign_test() -> bool:
    """The foreign CPU budget and the VM preflight judge red and green cases."""
    ok = True
    rows = [
        ["10", "1", "5", "/bin/bench"],
        ["20", "10", "90", "git"],
        ["30", "1", "89.9", "/usr/libexec/mediaanalysisd"],
        ["31", "1", "3", "x"],
    ]
    procs = foreign_rows(rows, 10)
    t, one, name = foreign_summary(procs)
    ok &= procs[0] == (89.9, "mediaanalysisd") and len(procs) == 2 and name == "mediaanalysisd"
    ok &= over_budget(t, one) and not over_budget(20.0, 9.0) and over_budget(51.0, 9.0)
    ok &= noise_sources(["bash", "com.docker.backend"], "") != []
    ok &= noise_sources(["bash"], "    iPhone (ABC) (Booted)\n") != []
    ok &= noise_sources(["bash", "cargo"], "== Devices ==\n") == []
    return ok


def self_test() -> int:
    """The control passes a probe that sees the spinner, passes one starved on its first
    try only, and fails one stuck at 0."""
    global cpu_secs, _control_done
    real = cpu_secs
    calls = [0]

    def starved_once(_pid: int) -> float:
        # Two reads per try: the first try sees no CPU, later tries see the spinner.
        calls[0] += 1
        return 0.0 if calls[0] <= 2 else time.perf_counter()

    verdicts = []
    try:
        for probe in (lambda _pid: time.perf_counter(), starved_once, lambda _pid: 0.0):
            cpu_secs, _control_done = probe, False
            try:
                require_control()
                verdicts.append(True)
            except RuntimeError as e:
                verdicts.append("probe is broken" not in str(e))
    finally:
        cpu_secs = real
        _control_done = False
    ok = verdicts == [True, True, False]
    ok &= foreign_test() and report_test()
    print("self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(self_test())
