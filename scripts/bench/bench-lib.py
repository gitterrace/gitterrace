#!/usr/bin/env python3
"""Helpers for the benchmark harness (scripts/bench/bench-run.py): subprocess and OS probes, the
server under test, corpus clones and the page plan. Imported, not run."""

from __future__ import annotations

import http.client
import json
import os
import re
import shutil
import socket
import statistics
import subprocess
import sys
import time
from pathlib import Path
from typing import Any
from urllib.parse import quote

Json = dict[str, Any]

ROOT = Path(__file__).resolve().parents[2]
# The insights tabs. Pages that moved into a section of a tab answer a redirect, never timed.
STATS = [
    "",
    "/languages",
    "/hotspots",
    "/ownership",
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


def own_shas(root: str | Path) -> set[str]:
    """Every commit of the repository at `root`: the control repository is gitterrace itself,
    and a page never names its own commits."""
    out = run(["git", "rev-list", "--all"], cwd=root, check=False)
    return set(out.split())


OWN_HEX = re.compile(r"\b[0-9a-f]{7,40}\b")
STAMP = re.compile(r"(v\d+\.\d+\.\d+)(?:-([0-9a-f]{7}))?")


def version_stamp(text: str) -> str:
    """`gitterrace v0.1.0-1161f1d` from `--version` output: the version and the build's 7-hex
    commit, never the -dirty mark (that is its own flag). Plain `gitterrace v0.1.0` when the
    binary names no commit."""
    m = STAMP.search(text)
    if not m:
        return text.strip()
    return "gitterrace " + m.group(1) + (f"-{m.group(2)}" if m.group(2) else "")


def commit_of(text: str) -> str | None:
    """The build's 7-hex commit from `--version` output, None when it names none."""
    m = STAMP.search(text)
    return m.group(2) if m else None


def scrub_own(v: Any, own: set[str]) -> Any:
    """`v` with every sha (or 7+ hex prefix) of `own` replaced by `HEAD`. The control repo is
    measured at its HEAD, which is what the run date names."""
    if isinstance(v, dict):
        return {k: scrub_own(x, own) for k, x in v.items()}
    if isinstance(v, list):
        return [scrub_own(x, own) for x in v]
    if isinstance(v, str):
        return OWN_HEX.sub(
            lambda m: "HEAD" if any(o.startswith(m.group(0)) for o in own) else m.group(0),
            v,
        )
    return v


def run(cmd: list[str], cwd: str | Path | None = None, check: bool = True) -> str:
    return subprocess.run(cmd, cwd=cwd, check=check, capture_output=True, text=True).stdout


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


def child_pids(pid: int) -> list[int]:
    """The direct children of `pid`: the index build runs in one."""
    out = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True).stdout
    return [int(x) for x in out.split()]


def footprint_mb(pid: int) -> float | None:
    """Dirty memory the kernel charges the process: `footprint` on macOS, Private_Dirty from
    smaps_rollup on Linux. Clean mapped pages (pack files, the index) are not counted."""
    rollup = Path(f"/proc/{pid}/smaps_rollup")
    if rollup.exists():
        kb = sum(
            int(line.split()[1])
            for line in rollup.read_text().splitlines()
            if line.startswith("Private_Dirty:")
        )
        # Private_Dirty holds the dirty anonymous pages too; RssAnon adds the clean ones
        # (zero pages touched only by reads). The larger never counts a page twice.
        return max(kb, rss_anon_kb(pid)) / 1024
    if not shutil.which("footprint"):
        return None
    out = subprocess.run(["footprint", str(pid)], capture_output=True, text=True).stdout
    m = re.search(r"Footprint:\s*([\d.]+)\s*(KB|MB|GB)", out)
    if not m:
        return None
    return float(m.group(1)) * {"KB": 1 / 1024, "MB": 1.0, "GB": 1024.0}[m.group(2)]


def rss_anon_kb(pid: int) -> int:
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        if line.startswith("RssAnon:"):
            return int(line.split()[1])
    return 0


def releases(port: int, prefix: str) -> int:
    """Idle releases so far, as the repo's /api/progress counts them."""
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
    c.request("GET", f"{prefix}/api/progress")
    n = int(json.loads(c.getresponse().read()).get("releases", -1))
    c.close()
    return n


def wait_released(port: int, timeout: float = 120) -> int:
    """Blocks until the server has released its idle caches once. The release runs one idle
    period (30 s) after the last read: a server that never releases fails here."""
    prefix, end = site_prefix(port), time.time() + timeout
    while (n := releases(port, prefix)) < 1:
        if time.time() > end:
            raise RuntimeError(f"no idle release within {timeout:.0f} s")
        time.sleep(0.5)
    return n


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

    def wait_ready(self, timeout: float = 14400) -> tuple[float, float, float]:
        """Seconds until the repo's api/progress leaves indexing (the index holds HEAD), seconds
        until it says ready (the warm-up is done too), and peak RSS meanwhile.

        Progress lives under the repo prefix, which /api/repos names once the server listens."""
        peak, indexed = 0.0, None
        prefix: str | None = None
        # Peak RSS and physical footprint per index step, charged to the step the last
        # answer named. Footprint leaves out clean mapped pack files, the gated number.
        self.phase_rss: dict[str, float] = {}
        self.phase_fp: dict[str, float] = {}
        self.peak_footprint = 0.0
        # The index build runs in a child process, so the server's own footprint leaves it out.
        self.peak_child_footprint = 0.0
        self.phases: list[dict[str, Any]] = []
        step = "start"
        while time.perf_counter() - self.t0 < timeout:
            if self.proc.poll() is not None:
                raise RuntimeError(f"server exited early, see {self.log.name}")
            now = rss_mb(self.pid)
            peak = max(peak, now)
            self.phase_rss[step] = max(self.phase_rss.get(step, 0.0), now)
            fp = footprint_mb(self.pid) or 0.0
            self.peak_footprint = max(self.peak_footprint, fp)
            self.phase_fp[step] = max(self.phase_fp.get(step, 0.0), fp)
            child = sum(footprint_mb(k) or 0.0 for k in child_pids(self.pid))
            self.peak_child_footprint = max(self.peak_child_footprint, child)
            try:
                if prefix is None:
                    prefix = site_prefix(self.port)
                c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
                c.request("GET", f"{prefix}/api/progress")
                r = c.getresponse()
                if r.status != 200:
                    raise RuntimeError(f"{prefix}/api/progress answered HTTP {r.status}")
                body = json.loads(r.read())
                c.close()
                step = body.get("phase") or step
                self.phases = body.get("phases", [])
                now = time.perf_counter() - self.t0
                if indexed is None and body.get("state") in INDEXED_STATES:
                    indexed = now
                if body.get("ready"):
                    return indexed if indexed is not None else now, now, peak
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
            sys.exit(f"{repo['name']} is local, measured at HEAD: drop its sha from the corpus")
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
    for line in run(["git", "ls-tree", "-r", "-l", "-z", rev], cwd=repo_dir).split("\0"):
        if "\t" not in line:
            continue
        meta, path = line.split("\t", 1)
        size = meta.split()[-1]
        if size.isdigit() and int(size) <= limit and Path(path).suffix.lower() not in TEXT_BAD:
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


# Budgets a run fails on without BENCH_STRICT: the index build and the time from start to a
# ready repository, which block a release.
INDEX_GATES = ("index_secs", "ready_secs", "rss_mb")
# The repository states of /api/progress once the index holds HEAD.
INDEXED_STATES = ("warming", "ready")


def phase_table(
    phases: list[dict[str, Any]], rss: dict[str, float], fp: dict[str, float]
) -> list[Json]:
    """The server's step times, CPU seconds and worker busy share, each with the peak RSS and
    peak footprint sampled while it ran."""
    out = []
    for p in phases:
        row = {"name": p["name"], "secs": round(float(p["secs"]), 3)}
        # Kept as null when the step has none, so a missing key means an old server.
        for k in ("cpu_secs", "busy"):
            if k in p:
                row[k] = None if p[k] is None else round(float(p[k]), 3)
        if p["name"] in rss:
            row["rss_mb"] = round(rss[p["name"]], 1)
        if p["name"] in fp:
            row["footprint_mb"] = round(fp[p["name"]], 1)
        out.append(row)
    return out


def ready_recorded(run: Json, name: str) -> list[str]:
    """A run from harness 4 records, per repo, the seconds to ready next to the index build's,
    and ready is never before the index is done. Older runs predate it."""
    if run.get("harness", 1) < 4:
        return []
    errs: list[str] = []
    for n, r in run["repos"].items():
        idx = r.get("index", {})
        if "ready_secs" not in idx:
            errs.append(f"{name}: {n} recorded no ready_secs")
        elif idx["ready_secs"] < idx.get("secs", 0):
            errs.append(f"{name}: {n} was ready before its index was built")
    return errs


def ready_recorded_test() -> None:
    """A harness 4 run without ready_secs, or ready before the index, is refused; harness 3
    passes untouched."""

    def run(index: Json, harness: int = 4) -> Json:
        return {"harness": harness, "repos": {"r": {"index": index}}}

    good = {"secs": 5.0, "ready_secs": 6.0}
    bad = [
        i
        for i in ({"secs": 5.0}, {"secs": 5.0, "ready_secs": 4.0})
        if not ready_recorded(run(i), "t")
    ]
    if ready_recorded(run(good), "t") or ready_recorded(run({"secs": 1}, 3), "t") or bad:
        sys.exit(f"bench self-test failed: ready guard passed {bad or 'a good run'}")


def phase_cpu(run: Json, name: str) -> list[str]:
    """A run from harness 3 records, per index step, its CPU seconds and worker busy share
    (null for a step where none applies), so a slow step reads as short of cores or as
    waiting. Older runs predate it. `name` labels the run in the errors."""
    if run.get("harness", 1) < 3:
        return []
    errs: list[str] = []
    for n, r in run["repos"].items():
        steps = r.get("index", {}).get("phases", [])
        if not steps:
            errs.append(f"{name}: {n} recorded no index steps")
        for p in steps:
            for key in ("cpu_secs", "busy"):
                if key not in p:
                    errs.append(f"{name}: {n} step {p['name']} has no {key}")
        if steps and not any(p.get("busy") is not None for p in steps):
            errs.append(f"{name}: {n} has no step with a worker busy share")
    return errs


def phase_cpu_test() -> None:
    """A harness 3 run whose index steps lack CPU seconds or a busy share is refused, a
    harness 2 run passes untouched, and the step table keeps a null as null."""

    def run(steps: list[Json], harness: int = 3) -> Json:
        return {"harness": harness, "repos": {"r": {"index": {"phases": steps}}}}

    good = [
        {"name": "rev_list", "secs": 1.0, "cpu_secs": 0.9, "busy": None},
        {"name": "stream", "secs": 9.0, "cpu_secs": 70.0, "busy": 0.8},
    ]
    no_cpu = [{k: v for k, v in p.items() if k != "cpu_secs"} for p in good]
    no_busy = [{k: v for k, v in p.items() if k != "busy"} for p in good]
    never_busy = [{**p, "busy": None} for p in good]
    bad = [s for s in (no_cpu, no_busy, never_busy, []) if not phase_cpu(run(s), "t")]
    if phase_cpu(run(good), "t") or phase_cpu(run(no_cpu, 2), "t") or bad:
        sys.exit(f"bench self-test failed: step CPU guard passed {bad or 'a good run'}")
    rows = phase_table(good + [{"name": "old", "secs": 1}], {}, {})
    if rows[0]["busy"] is not None or "busy" in rows[2] or rows[1]["busy"] != 0.8:
        sys.exit(f"bench self-test failed: phase table {rows}")


def slowest_page(pages: Json) -> str:
    """The page with the highest warm median. The page budget is a ceiling, so this page sets
    it, and which page that is differs per repo (a big blame on one, a big diff on another)."""
    return max(pages, key=lambda k: pages[k]["warm_total_ms"]["median"])


def slowest(phases: list[Json], key: str) -> str:
    """The step with the most `key`, "unknown" without a step table. A sub-step (dotted
    name) is part of its parent, so it only names the cause when it is the larger part."""
    rows = [p for p in phases if key in p]
    if not rows:
        return "unknown"
    return max(rows, key=lambda p: p[key])["name"]


def index_over(name: str, budget: Json) -> list[str]:
    """Index time or index memory over budget: always fails the run, naming the step."""
    return [
        f"{name}: {k} {v['value']:.1f} > {v['limit']} (slowest phase: {v.get('phase')})"
        for k, v in budget.items()
        if k in INDEX_GATES and v["status"] == "over"
    ]
