#!/usr/bin/env python3
"""Helpers for the benchmark harness (scripts/bench-run.py): subprocess and OS probes, the
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
        return kb / 1024
    if not shutil.which("footprint"):
        return None
    out = subprocess.run(["footprint", str(pid)], capture_output=True, text=True).stdout
    m = re.search(r"Footprint:\s*([\d.]+)\s*(KB|MB|GB)", out)
    if not m:
        return None
    return float(m.group(1)) * {"KB": 1 / 1024, "MB": 1.0, "GB": 1024.0}[m.group(2)]


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
