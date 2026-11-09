#!/usr/bin/env python3
"""Benchmark harness for gitterrace on large public repos (stdlib, git, the release binary).

Run by `make bench`. Reads tests/bench/corpus.toml, clones each repo into $GITTERRACE_BENCH_DIR
(default ~/.cache/gitterrace-bench), starts `git-terrace` on a free port per repo, measures it,
and writes tests/bench/results/<date>-<machine label>.json (no hostname).
Methodology: docs/benchmarks.md.

  scripts/bench-run.py [--repos a,b] [--large] [--runs N] [--bin PATH] [--idle-secs S]
  scripts/bench-run.py --synthetic      # tiny generated repo, one run: the cheap guard
  scripts/bench-run.py --fetch-only     # clone and backfill, time nothing

A full run lands in tests/bench/results and `make bench` then runs bench-report. A smoke run (fewer
--runs or a shorter --idle-secs) goes to $GITTERRACE_BENCH_DIR/runs, never into the repo.

Every timed phase waits for a quiet machine first: the 1-minute load average must stay under
BENCH_MAX_LOAD (default: half the core count) for QUIET_SECS. The load seen is recorded per
phase, and bench-report --check refuses a run that was timed above the limit.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime
import importlib
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

bl = importlib.import_module("bench-lib")
bq = importlib.import_module("bench-quiet-lib")
bc = importlib.import_module("bench-chart-lib")

Json = dict[str, Any]
LoadSampler = bq.LoadSampler
Server = bl.Server
parse_toml = bl.parse_toml
run = bl.run
dir_mb = bl.dir_mb
rss_mb = bl.rss_mb
get = bl.get
site_prefix = bl.site_prefix
isolated_config = bl.isolated_config
ensure_clone = bl.ensure_clone
discover = bl.discover
pages = bl.pages
ROOT = bl.ROOT
FULL_RUNS = bq.FULL_RUNS
HARNESS = 2
FULL_IDLE_SECS = bq.FULL_IDLE_SECS
COLD_SAMPLES = bq.COLD_SAMPLES
max_load = bq.max_load
quiet = bq.quiet
measure_pages = bq.measure_pages
idle_cpu = bq.idle_cpu


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
    sampler: Any,
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
        res["index"]["phases"] = bl.phase_table(srv.phases, srv.phase_rss)
        print(f"  index {secs:.1f} s, peak RSS {peak:.0f} MB", flush=True)
        for p in res["index"]["phases"]:
            print(
                f"    {p['name']:<12} {p['secs']:8.1f} s  {p.get('rss_mb', 0):6.0f} MB",
                flush=True,
            )
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
        # Footprint is read after the server released what the pages built, never before.
        res["memory"]["footprint_releases"] = bl.wait_released(srv.port)
        res["memory"]["footprint_idle_mb"] = bl.footprint_mb(srv.pid)
        res["memory"]["rss_idle_mb"] = rss_mb(srv.pid)
        res["memory"]["footprint_start_mb"] = start_footprint(binary, logdir)
        print(
            f"  idle footprint {mb(res['memory']['footprint_idle_mb'])}, fresh server "
            f"{mb(res['memory']['footprint_start_mb'])}, idle RSS {res['memory']['rss_idle_mb']:.0f} MB",
            flush=True,
        )
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


def mb(x: float | None) -> str:
    return "n/a" if x is None else f"{x:.0f} MB"


def start_footprint(binary: str, logdir: Path) -> float | None:
    """Footprint of a just-started server: one on a one-commit repo, once it is ready. The
    serving-memory goal is measured above this, as memory_budget.rs does."""
    d = Path(logdir) / "fresh"
    if not d.exists():
        run(["git", "init", "-q", str(d)])
        (d / "a").write_text("a\n")
        run(["git", "add", "a"], cwd=d)
        run(
            [
                "git",
                "-c",
                "user.name=b",
                "-c",
                "user.email=b@example.com",
                "commit",
                "-qm",
                "a",
            ],
            cwd=d,
        )
    srv = Server(binary, d, logdir)
    try:
        srv.wait_ready()
        return bl.footprint_mb(srv.pid)
    finally:
        srv.stop()


def export(
    name: str,
    repo_dir: Path,
    binary: str,
    logdir: Path,
    base: Path,
    loads: Json,
    wait: bool,
    i: int,
    sampler: Any | None = None,
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


def field(res: Json, path: str) -> float:
    """A dotted result field, e.g. index.peak_rss_mb."""
    cur: Any = res
    for part in path.split("."):
        cur = cur[part]
    return float(cur)


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
    phases = res["index"].get("phases", [])
    if "index_secs" in out:
        out["index_secs"]["phase"] = bl.slowest(phases, "secs")
    put(
        "idle_cpu_pct",
        max(field(res, f) for f in bc.SOURCES["idle_cpu_pct"]),
        b.get("idle_cpu_pct"),
    )
    put("rss_mb", max(field(res, f) for f in bc.SOURCES["rss_mb"]), b.get("rss_mb"))
    if "rss_mb" in out:
        out["rss_mb"]["phase"] = bl.slowest(phases, "rss_mb")
    # The serving-memory goal: idle, after a release, at most the cache budget plus margin
    # above a just-started server (docs/benchmarks.md, memory_budget.rs).
    fp, start = (
        res["memory"].get("footprint_idle_mb"),
        res["memory"].get("footprint_start_mb"),
    )
    if fp is not None and start is not None:
        put("footprint_idle_mb", fp - start, b.get("footprint_idle_mb"))
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
        # The version only: the build's own commit is never recorded (dirty is its own flag).
        "gitterrace": re.sub(
            r"(v\d+\.\d+\.\d+)\S*", r"\1", run([binary, "--version"]).strip()
        ),
        "dirty": "dirty" in run([binary, "--version"]),
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


def preflight_index() -> None:
    """Refuse to time anything while a desktop indexer can rescan the bench or target dirs.

    A rescan burns CPU during a timed phase and skews the numbers, the same reason the quiet
    check exists. The guard is a refusal, not a warning, because a published number is
    permanent. `make setup` fixes it.
    """
    dis = importlib.import_module("indexing-run")
    bad = dis.missing(dis.bench_dirs())
    if bad:
        sys.exit(
            "not excluded from search indexing (it skews timings): "
            + ", ".join(map(str, bad))
            + "\nrun `make setup`"
        )


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
    ap.add_argument(
        "--preflight",
        action="store_true",
        help="only check the indexer exclusion, then exit",
    )
    a = ap.parse_args()
    if a.preflight:
        preflight_index()
        return 0
    if not a.synthetic and not a.fetch_only:
        preflight_index()
    a.bin = str(Path(a.bin).resolve())
    if not Path(a.bin).exists():
        sys.exit(f"no binary at {a.bin}, run `make build` (release) first")
    corpus = parse_toml((ROOT / "tests/bench/corpus.toml").read_text())
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
        # 2: every repo records memory.footprint_idle_mb after an idle release.
        "harness": HARNESS,
        "when": str(datetime.date.today()),
        "runs": a.runs,
        "idle_secs": a.idle_secs,
        "machine": machine(a.bin),
        "repos": {},
    }
    over, fails = [], []
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
        res["budget"] = judge(res, budgets(corpus))
        fails += bl.index_over(repo["name"], res["budget"])
        over += [
            f"{repo['name']}: {k} {v['value']:.1f} > {v['limit']}"
            for k, v in res["budget"].items()
            if v["status"] == "over"
        ]
        result["repos"][repo["name"]] = res
    out_dir = (
        logdir
        if a.synthetic
        else ROOT / "tests/bench/results"
        if full
        else base / "runs"
    )
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"{datetime.date.today()}-{result['machine']['label']}.json"
    # Repos can run in separate passes on one day (control last, after a history rewrite).
    # A same-day file from this machine keeps its other repos, the new pass wins per repo.
    if path.exists():
        prev = json.loads(path.read_text())
        result["repos"] = {**prev.get("repos", {}), **result["repos"]}
    result = bl.scrub_own(result, bl.own_shas(ROOT))
    path.write_text(json.dumps(result, indent=1, sort_keys=True) + "\n")
    print(f"wrote {path}")
    if a.synthetic:
        shutil.rmtree(logdir, ignore_errors=True)
    for line in over:
        print("OVER BUDGET", line)
    for line in fails:
        print("INDEX OVER BUDGET", line)
    if fails:
        return 1
    return 1 if over and os.environ.get("BENCH_STRICT") == "1" else 0


def budgets(corpus: Json) -> Json:
    """The corpus budgets. TEST_BENCH_INDEX_SECS sets every class's index_secs, so the guard
    sees a run fail red on the tiny synthetic repo."""
    b = corpus.get("budgets", {})
    forced = os.environ.get("TEST_BENCH_INDEX_SECS")
    if forced is None:
        return b
    return {k: {**v, "index_secs": float(forced)} for k, v in b.items()}


if __name__ == "__main__":
    sys.exit(main())
