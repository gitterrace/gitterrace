#!/usr/bin/env python3
"""Benchmark harness for gitterrace on large public repos (stdlib, git, the release binary).

Run by `make bench`. Reads bench/corpus.toml, clones each repo into $GITTERRACE_BENCH_DIR
(default ~/.cache/gitterrace-bench), starts `git-terrace` on a free port per repo, measures it,
and writes bench/results/<date>-<machine label>.json (no hostname). Methodology: docs/benchmarks.md.

  scripts/bench-run.py [--repos a,b] [--large] [--runs N] [--bin PATH] [--idle-secs S]
  scripts/bench-run.py --synthetic      # tiny generated repo, one run: the cheap guard
  scripts/bench-run.py --fetch-only     # clone and backfill, time nothing

A full run lands in bench/results and `make bench` then runs bench-report. A smoke run (fewer
--runs or a shorter --idle-secs) goes to $GITTERRACE_BENCH_DIR/runs, never into the repo.

Every timed phase waits for a quiet machine first: the 1-minute load average must stay under
BENCH_MAX_LOAD (default: half the core count) for QUIET_SECS. The load seen is recorded per
phase, and bench-report --check refuses a run that was timed above the limit.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime
import http.client
import json
import os
import platform
import re
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any
from urllib.parse import quote

Json = dict[str, Any]

ROOT = Path(__file__).resolve().parent.parent
STATS = [
    "",
    "/types",
    "/languages",
    "/frequency",
    "/hotspots",
    "/ownership",
    "/patterns",
    "/contributors",
]
TEXT_BAD = {
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".ico",
    ".woff",
    ".woff2",
    ".pdf",
    ".zip",
    ".gz",
    ".bin",
    ".svg",
    ".lock",
    ".so",
    ".a",
    ".o",
    ".wasm",
    ".pack",
    ".idx",
    ".mp4",
}


def parse_toml(text: str) -> Json:
    """The small TOML subset corpus.toml uses: [[array]], [a.b] tables, scalars."""
    out: Json = {}
    cur: Json | None = None
    for raw in text.splitlines():
        line = re.sub(r"\s+#.*$", "", raw).strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[["):
            cur = {}
            out.setdefault(line[2:-2].strip(), []).append(cur)
        elif line.startswith("["):
            table: Json = out
            for part in line[1:-1].strip().split("."):
                table = table.setdefault(part, {})
            cur = table
        elif cur is None:
            sys.exit(f"corpus.toml: key outside a table: {line}")
        else:
            k, v = (s.strip() for s in line.split("=", 1))
            cur[k] = json.loads(v) if v[0] in '"[0123456789' else v == "true"
    return out


def run(cmd: list[str], cwd: str | Path | None = None, check: bool = True) -> str:
    return subprocess.run(
        cmd, cwd=cwd, check=check, capture_output=True, text=True
    ).stdout


def free_gb(path: str | Path) -> float:
    return shutil.disk_usage(path).free / 2**30


def dir_mb(path: str | Path) -> float:
    n = 0
    for p in Path(path).rglob("*"):
        try:
            if p.is_file() and not p.is_symlink():
                n += p.stat().st_size
        except OSError:
            pass
    return n / 2**20


def rss_mb(pid: int) -> float:
    if Path(f"/proc/{pid}/status").exists():
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) / 1024
    out = subprocess.run(
        ["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True
    ).stdout
    return int(out.strip() or 0) / 1024


def cpu_secs(pid: int) -> float:
    """Total CPU seconds of a process, from /proc on Linux and ps on macOS."""
    stat = Path(f"/proc/{pid}/stat")
    if stat.exists():
        f = stat.read_text().rsplit(")", 1)[1].split()
        return (int(f[11]) + int(f[12])) / os.sysconf("SC_CLK_TCK")
    t = subprocess.run(
        ["ps", "-o", "time=", "-p", str(pid)], capture_output=True, text=True
    ).stdout.strip()
    secs = 0.0
    for part in t.split(":"):
        secs = secs * 60 + float(part)
    return secs


def stats(xs: list[float]) -> dict[str, float]:
    return {
        "median": statistics.median(xs),
        "min": min(xs),
        "max": max(xs),
        "n": len(xs),
    }


def get(port: int, path: str, timeout: float = 600) -> tuple[int, float, float, int]:
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    t0 = time.perf_counter()
    c.request("GET", path)
    r = c.getresponse()
    t1 = time.perf_counter()
    body = r.read()
    t2 = time.perf_counter()
    c.close()
    return r.status, (t1 - t0) * 1000, (t2 - t0) * 1000, len(body)


def site_prefix(port: int) -> str:
    """Where the one served repo's pages live, from /api/repos (`/r/<name>`).

    Asking the server keeps naming in one place. A probe built on it never times a redirect."""
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
    c.request("GET", "/api/repos")
    items = json.loads(c.getresponse().read())
    c.close()
    if len(items) != 1:
        raise RuntimeError(f"/api/repos lists {len(items)} repositories, expected 1")
    return str(items[0]["href"]).rstrip("/")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def isolated_config(logdir: str | Path) -> Path:
    """An empty config dir, so no developer config.toml or actions.toml shapes a result."""
    d = Path(logdir) / "config"
    d.mkdir(exist_ok=True)
    (d / "config.toml").write_text("")
    return d


def text_limit_bytes():
    """The default text-file limit, read from the config schema so it has one source."""
    schema = (ROOT / "crates/gitterrace-config/src/schema.rs").read_text()
    m = re.search(r"text_kb: u64 = (\d+)", schema)
    if m is None:
        sys.exit("no text_kb default in crates/gitterrace-config/src/schema.rs")
    return int(m.group(1)) * 1024


class Server:
    def __init__(self, binary: str, repo: str | Path, logdir: str | Path) -> None:
        self.port = free_port()
        # Held for the server's whole life and closed in stop(), so no with block fits.
        self.log = open(Path(logdir) / f"server-{self.port}.log", "w")  # noqa: SIM115
        self.t0 = time.perf_counter()
        self.proc = subprocess.Popen(
            [
                binary,
                "--config",
                str(isolated_config(logdir)),
                str(repo),
                "--addr",
                f"127.0.0.1:{self.port}",
            ],
            stdout=self.log,
            stderr=self.log,
            cwd=repo,
        )
        self.pid = self.proc.pid

    def wait_ready(self, timeout: float = 14400) -> tuple[float, float | None, float]:
        """Index build: seconds until the repo's api/progress says ready, and peak RSS meanwhile.

        Progress lives under the repo prefix, which /api/repos names once the server listens."""
        peak, listening = 0.0, None
        prefix: str | None = None
        while time.perf_counter() - self.t0 < timeout:
            if self.proc.poll() is not None:
                raise RuntimeError(f"server exited early, see {self.log.name}")
            peak = max(peak, rss_mb(self.pid))
            try:
                if prefix is None:
                    prefix = site_prefix(self.port)
                    listening = time.perf_counter() - self.t0
                c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
                c.request("GET", f"{prefix}/api/progress")
                r = c.getresponse()
                if r.status != 200:
                    raise RuntimeError(
                        f"{prefix}/api/progress answered HTTP {r.status}"
                    )
                body = json.loads(r.read())
                c.close()
                if body.get("ready"):
                    return time.perf_counter() - self.t0, listening, peak
            except (OSError, ValueError):
                pass
            time.sleep(0.2)
        raise RuntimeError("index did not become ready in time")

    def stop(self):
        self.proc.terminate()
        try:
            self.proc.wait(30)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()


def ensure_clone(repo: Json, base: Path, logdir: Path) -> Path:
    """Bare, blobless clone of the pinned sha, then every blob of its history.

    A local repo is measured at its HEAD and has no pin: a sha there would only go stale."""
    if repo.get("local"):
        if "sha" in repo:
            sys.exit(
                f"{repo['name']} is local, measured at HEAD: drop its sha from the corpus"
            )
        return (ROOT / repo["local"]).resolve()
    sha, dest = repo["sha"], Path(base) / f"{repo['name']}.git"
    mark = dest / "bench-ready"
    if mark.exists() and mark.read_text().strip() == sha:
        return dest
    need = repo.get("need_gb", 1)
    if free_gb(base) < need:
        sys.exit(
            f"refusing to clone {repo['name']}: {free_gb(base):.1f} GB free in {base}, needs {need} GB"
        )
    if not dest.exists():
        run(["git", "init", "-q", "--bare", str(dest)])
        run(["git", "remote", "add", "origin", repo["url"]], cwd=dest)
    print(f"fetching {repo['name']} @ {sha[:10]} into {dest}", flush=True)
    run(["git", "fetch", "-q", "--filter=blob:none", "origin", sha], cwd=dest)
    run(["git", "update-ref", "refs/heads/bench", sha], cwd=dest)
    run(["git", "symbolic-ref", "HEAD", "refs/heads/bench"], cwd=dest)
    print(f"backfilling blobs of {repo['name']}", flush=True)
    run(["git", "backfill"], cwd=dest)
    mark.write_text(sha + "\n")
    return dest


def discover(repo_dir: str | Path, rev: str) -> Json:
    """Probe defaults: largest file under the text limit, deepest folder, newest commit.

    A larger file would make /blob and /blame answer with the refusal page."""
    limit = text_limit_bytes()
    files = []
    for line in run(["git", "ls-tree", "-r", "-l", "-z", rev], cwd=repo_dir).split(
        "\0"
    ):
        if "\t" not in line:
            continue
        meta, path = line.split("\t", 1)
        size = meta.split()[-1]
        if (
            size.isdigit()
            and int(size) <= limit
            and Path(path).suffix.lower() not in TEXT_BAD
        ):
            files.append((int(size), path))
    files.sort(reverse=True)
    deep = max(
        (p.rsplit("/", 1)[0] for _, p in files if "/" in p),
        key=lambda d: d.count("/"),
        default="",
    )
    return {
        "blob": files[0][1],
        "blame": files[0][1],
        "deep_tree": deep,
        "commit": run(["git", "rev-parse", rev], cwd=repo_dir).strip(),
    }


def pages(probe: Json, search: str) -> dict[str, str]:
    p = {
        "overview": "/",
        "tree-root": "/tree/HEAD",
        "log": "/log/HEAD",
        "refs": "/refs",
        "commit-largest": f"/commit/{probe['commit']}",
        "diff-fragment": f"/diff/{probe['commit']}/0",
        "search": f"/search?q={quote(search)}",
    }
    if probe.get("deep_tree"):
        p["tree-deep"] = f"/tree/HEAD/{quote(probe['deep_tree'])}"
    p["blob-largest"] = f"/blob/HEAD/{quote(probe['blob'])}"
    p["blame-long-history"] = f"/blame/HEAD/{quote(probe['blame'])}"
    for s in STATS:
        p["stats" + (s or "-overview")] = "/stats" + s
    return p


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


def foreign_cores() -> float:
    """CPU in cores of every process that is not this harness or its descendants."""
    try:
        out = run(["ps", "-Ao", "pid=,ppid=,pcpu="])
    except (OSError, subprocess.CalledProcessError):
        return 0.0
    rows = [line.split() for line in out.splitlines() if len(line.split()) == 3]
    parent = {int(p): int(pp) for p, pp, _ in rows}
    me = os.getpid()

    def ours(pid: int) -> bool:
        seen = 0
        while pid > 1 and seen < 64:
            if pid == me:
                return True
            pid, seen = parent.get(pid, 1), seen + 1
        return False

    return sum(float(c) for p, _, c in rows if not ours(int(p))) / 100


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
    phase: str, loads: Json, wait: bool = True, sampler: LoadSampler | None = None
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
        if os.getloadavg()[0] < limit:
            calm += 1
        else:
            if calm == 0 or calm % 60 == 0:
                print(
                    f"  waiting for load {os.getloadavg()[0]:.1f} < {limit:.1f} before {phase}",
                    flush=True,
                )
            calm = 0
        time.sleep(1)
    loads[phase] = round(os.getloadavg()[0], 2)
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


def idle_cpu(server: Server, secs: int, with_events: bool) -> float:
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


def bench_repo(
    repo: Json,
    repo_dir: Path,
    binary: str,
    runs: int,
    idle_secs: int,
    logdir: Path,
    base: Path,
    wait: bool = True,
    samples: int = 1,
) -> Json:
    name = repo["name"]
    print(f"== {name} ({repo['class']}) @ {repo.get('sha', 'HEAD')[:10]}", flush=True)
    rev = repo["sha"] if not repo.get("local") else "HEAD"
    probe = discover(repo_dir, rev)
    probe.update(
        {k: repo[k] for k in ("blob", "blame", "deep_tree", "commit") if k in repo}
    )
    # The cache lives under the git dir: .git/gitterrace, or ./gitterrace in a bare clone.
    cache = (
        Path(repo_dir)
        / (".git" if (Path(repo_dir) / ".git").exists() else "")
        / "gitterrace"
    )
    shutil.rmtree(cache, ignore_errors=True)
    # One untimed read warms the OS cache so cold means the first request, not the disk.
    run(["git", "rev-list", "--count", rev], cwd=repo_dir)
    loads: Json = {}
    sampler = LoadSampler()
    try:
        res = bench_phases(
            repo,
            repo_dir,
            binary,
            runs,
            idle_secs,
            logdir,
            base,
            wait,
            samples,
            cache,
            probe,
            loads,
            sampler,
        )
    finally:
        sampled = sampler.finish()
    res["load"]["sampled"] = sampled
    return res


def bench_phases(
    repo: Json,
    repo_dir: Path,
    binary: str,
    runs: int,
    idle_secs: int,
    logdir: Path,
    base: Path,
    wait: bool,
    samples: int,
    cache: Path,
    probe: Json,
    loads: Json,
    sampler: LoadSampler,
) -> Json:
    """The timed phases of bench_repo, each started through quiet() so the sampler follows."""
    name = repo["name"]
    quiet("index", loads, wait, sampler)
    srv = Server(binary, repo_dir, logdir)
    # A local repo is measured at its HEAD, so report that, not the corpus pin.
    res = {
        "sha": probe["commit"] if repo.get("local") else repo["sha"],
        "class": repo["class"],
        "probe": probe,
        "config": "empty config.toml, no actions.toml",
        "load": {"limit": max_load(), "phases": loads},
    }
    try:
        secs, listen, peak = srv.wait_ready()
        res["index"] = {"secs": secs, "listen_secs": listen, "peak_rss_mb": peak}
        print(f"  index {secs:.1f} s, peak RSS {peak:.0f} MB", flush=True)
        rss_warm = rss_mb(srv.pid)
        quiet("pages", loads, wait, sampler)
        res["pages"] = measure_pages(
            srv.port, pages(probe, repo.get("search", "fn")), runs
        )
        res["memory"] = {
            "rss_after_index_mb": rss_warm,
            "rss_after_pages_mb": rss_mb(srv.pid),
        }
        res["index"]["cache_mb"] = dir_mb(cache)
        quiet("idle", loads, wait, sampler)
        res["idle"] = {
            "secs": idle_secs,
            "cpu_pct": idle_cpu(srv, idle_secs, False),
            "cpu_pct_events_client": idle_cpu(srv, idle_secs, True),
        }
        print(
            f"  idle {res['idle']['cpu_pct']:.2f}% / with /events {res['idle']['cpu_pct_events_client']:.2f}%",
            flush=True,
        )
    finally:
        srv.stop()
    # Cold metrics are one sample per server start, so repeat them from an empty cache and a
    # fresh server. The warm and idle numbers above already are medians or windows.
    plan = pages(probe, repo.get("search", "fn"))
    index = [res["index"]["secs"]]
    cold = {k: [p["cold"]["total_ms"]] for k, p in res["pages"].items()}
    for i in range(2, samples + 1):
        shutil.rmtree(cache, ignore_errors=True)
        quiet(f"index#{i}", loads, wait, sampler)
        srv = Server(binary, repo_dir, logdir)
        try:
            secs, _, _ = srv.wait_ready()
            index.append(secs)
            quiet(f"pages#{i}", loads, wait, sampler)
            prefix = site_prefix(srv.port)
            for label, page in plan.items():
                status, _, total, _ = get(srv.port, prefix + page)
                if status != 200:
                    raise RuntimeError(f"{label} answered HTTP {status}, not 200")
                cold[label].append(total)
        finally:
            srv.stop()
        print(f"  cold round {i}: index {secs:.1f} s", flush=True)
    res["index"]["samples"] = index
    res["index"]["secs"] = statistics.median(index)
    for label, xs in cold.items():
        res["pages"][label]["cold"]["samples"] = xs
        res["pages"][label]["cold"]["total_ms"] = statistics.median(xs)
    exports = [
        export(name, repo_dir, binary, logdir, base, loads, wait, i, sampler)
        for i in range(1, samples + 1)
    ]
    secs = [e["secs"] for e in exports]
    res["export"] = {
        **exports[-1],
        "secs": statistics.median(secs),
        "samples": secs,
        "ok": all(e["ok"] for e in exports),
    }
    res["cold_samples"] = samples
    return res


def export(
    name: str,
    repo_dir: Path,
    binary: str,
    logdir: Path,
    base: Path,
    loads: Json,
    wait: bool,
    i: int,
    sampler: LoadSampler | None = None,
) -> Json:
    """One timed export. The output can be gigabytes, so it goes under the bench dir."""
    out = Path(base) / f"export-{name}"
    shutil.rmtree(out, ignore_errors=True)
    quiet("export" if i == 1 else f"export#{i}", loads, wait, sampler)
    t = time.perf_counter()
    r = subprocess.run(
        [binary, "--config", str(isolated_config(logdir)), "export", str(out)],
        cwd=repo_dir,
        capture_output=True,
        text=True,
    )
    secs = time.perf_counter() - t
    res = {"secs": secs, "out_mb": dir_mb(out), "ok": r.returncode == 0}
    shutil.rmtree(out, ignore_errors=True)
    return res


def judge(res: Json, budgets: Json) -> Json:
    """Budget status per metric class: pass or over."""
    b, out = budgets.get(res["class"], {}), {}

    def put(
        key: str, value: float, limit: float | None, samples: list[float] | None = None
    ) -> None:
        if limit is not None:
            out[key] = {
                "value": value,
                "limit": limit,
                "status": "pass" if value <= limit else "over",
            }
            if samples:
                out[key]["min"], out[key]["max"] = min(samples), max(samples)

    pg = res["pages"].values()
    put(
        "page_warm_ms",
        max(p["warm_total_ms"]["median"] for p in pg),
        b.get("page_warm_ms"),
    )
    slow = max(pg, key=lambda p: p["cold"]["total_ms"])
    put(
        "page_cold_ms",
        slow["cold"]["total_ms"],
        b.get("page_cold_ms"),
        slow["cold"].get("samples"),
    )
    put(
        "index_secs",
        res["index"]["secs"],
        b.get("index_secs"),
        res["index"].get("samples"),
    )
    put(
        "idle_cpu_pct",
        max(res["idle"]["cpu_pct"], res["idle"]["cpu_pct_events_client"]),
        b.get("idle_cpu_pct"),
    )
    put("rss_mb", res["memory"]["rss_after_pages_mb"], b.get("rss_mb"))
    if "export" in res:
        put(
            "export_secs",
            res["export"]["secs"],
            b.get("export_secs"),
            res["export"].get("samples"),
        )
    return out


def os_name():
    """Readable OS: "macOS 26.6.2", else the system and release as reported."""
    mac = platform.mac_ver()[0]
    if mac:
        return f"macOS {mac}"
    return f"{platform.system()} {platform.release()}"


def disk_name():
    """Disk size and medium under the clone dir, e.g. "1.0 TB SSD". Off macOS, the size alone."""
    size = f"{shutil.disk_usage(Path.home()).total / 1e12:.1f} TB"
    try:
        info = run(["diskutil", "info", "/"])
    except (OSError, subprocess.CalledProcessError):
        return size
    return f"{size} SSD" if re.search(r"Solid State:\s+Yes", info) else size


def machine(binary: str) -> Json:
    ram = 0
    with contextlib.suppress(ValueError, OSError):
        ram = os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES") / 2**30
    cpu = platform.processor()
    try:
        cpu = run(["sysctl", "-n", "machdep.cpu.brand_string"]).strip()
    except (OSError, subprocess.CalledProcessError):
        for line in (
            Path("/proc/cpuinfo").read_text().splitlines()
            if Path("/proc/cpuinfo").exists()
            else []
        ):
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    return {
        # A neutral label from the CPU, never platform.node(): results are published.
        "label": re.sub(r"[^a-z0-9]+", "-", cpu.lower()).strip("-")
        or platform.machine(),
        "os": os_name(),
        "disk": disk_name(),
        "arch": platform.machine(),
        "cpu": cpu,
        "cpus": os.cpu_count(),
        "ram_gb": round(ram, 1),
        "git": run(["git", "--version"]).strip(),
        "gitterrace": run([binary, "--version"]).strip(),
        "gitterrace_sha": run(["git", "rev-parse", "HEAD"], cwd=ROOT).strip(),
    }


def synthetic(base: str | Path) -> tuple[Json, Path]:
    d = Path(base) / "synthetic"
    run(["git", "init", "-q", str(d)])
    env = ["-c", "user.name=b", "-c", "user.email=b@example.com"]
    for i in range(30):
        f = d / "src" / "a" / "b" / f"f{i % 5}.rs"
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text("".join(f"fn item_{i}_{n}() {{}}\n" for n in range(i + 3)))
        run(["git", "add", "-A"], cwd=d)
        run(["git", *env, "commit", "-q", "-m", f"commit {i}"], cwd=d)
    sha = run(["git", "rev-parse", "HEAD"], cwd=d).strip()
    return {
        "name": "synthetic",
        "class": "small",
        "sha": sha,
        "local": str(d),
        "search": "item",
    }, d


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--repos", help="comma list of corpus names (default: all but opt-in)"
    )
    ap.add_argument(
        "--large", action="store_true", help="include opt-in repos (also BENCH_LARGE=1)"
    )
    ap.add_argument(
        "--fetch-only", action="store_true", help="clone and backfill, time nothing"
    )
    ap.add_argument("--runs", type=int, default=FULL_RUNS)
    ap.add_argument("--idle-secs", type=int, default=FULL_IDLE_SECS)
    ap.add_argument("--bin", default=str(ROOT / "target/release/git-terrace"))
    ap.add_argument(
        "--synthetic", action="store_true", help="tiny generated repo, one run (guard)"
    )
    a = ap.parse_args()
    a.bin = str(Path(a.bin).resolve())
    if not Path(a.bin).exists():
        sys.exit(f"no binary at {a.bin}, run `make build` (release) first")
    corpus = parse_toml((ROOT / "bench/corpus.toml").read_text())
    base = Path(
        os.environ.get("GITTERRACE_BENCH_DIR", Path.home() / ".cache/gitterrace-bench")
    )
    base.mkdir(parents=True, exist_ok=True)
    logdir = Path(tempfile.mkdtemp(prefix="gt-bench-"))
    todo = []
    if a.synthetic:
        a.runs, a.idle_secs = 1, 1
        repo, d = synthetic(logdir)
        todo = [(repo, d)]
    else:
        large = a.large or os.environ.get("BENCH_LARGE") == "1"
        want = a.repos.split(",") if a.repos else None
        for r in corpus["repo"]:
            if (want and r["name"] not in want) or (
                not want and r.get("opt_in") and not large
            ):
                continue
            if r.get("opt_in") and not large:
                sys.exit(
                    f"{r['name']} is opt-in: set BENCH_LARGE=1 (about {r.get('need_gb')} GB)"
                )
            todo.append((r, ensure_clone(r, base, logdir)))
    if a.fetch_only:
        print("fetched: " + ", ".join(r["name"] for r, _ in todo))
        return 0
    result = {
        "when": str(datetime.date.today()),
        "runs": a.runs,
        "idle_secs": a.idle_secs,
        "machine": machine(a.bin),
        "repos": {},
    }
    over = []
    # Only a full run is published and repeats the cold phases. A smoke run (fewer runs, shorter
    # idle) samples once and stays in the cache.
    full = not a.synthetic and a.runs == FULL_RUNS and a.idle_secs == FULL_IDLE_SECS
    for repo, d in todo:
        res = bench_repo(
            repo,
            d,
            a.bin,
            a.runs,
            a.idle_secs,
            logdir,
            base,
            not a.synthetic,
            int(repo.get("cold_samples", COLD_SAMPLES)) if full else 1,
        )
        res["budget"] = judge(res, corpus.get("budgets", {}))
        over += [
            f"{repo['name']}: {k} {v['value']:.1f} > {v['limit']}"
            for k, v in res["budget"].items()
            if v["status"] == "over"
        ]
        result["repos"][repo["name"]] = res
    out_dir = (
        logdir if a.synthetic else ROOT / "bench/results" if full else base / "runs"
    )
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"{datetime.date.today()}-{result['machine']['label']}.json"
    # Repos can run in separate passes on one day (control last, after a history rewrite).
    # A same-day file from this machine keeps its other repos, the new pass wins per repo.
    if path.exists():
        prev = json.loads(path.read_text())
        result["repos"] = {**prev.get("repos", {}), **result["repos"]}
    path.write_text(json.dumps(result, indent=1, sort_keys=True) + "\n")
    print(f"wrote {path}")
    if a.synthetic:
        shutil.rmtree(logdir, ignore_errors=True)
    for line in over:
        print("OVER BUDGET", line)
    return 1 if over and os.environ.get("BENCH_STRICT") == "1" else 0


if __name__ == "__main__":
    sys.exit(main())
