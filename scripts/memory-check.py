#!/usr/bin/env python3
"""Steady-state memory guard: an idle server's memory must not grow with the history it serves.

Builds synthetic repositories of growing size, serves each with the release binary, waits for
the index, visits the bench's page set (`pages` in bench-lib.py) and then leaves the server
idle past its record release (`RECORDS_IDLE` in gitterrace-web/src/app.rs). It then reads RSS
the way bench-run samples it (`ps -o rss`) and, on macOS, the dirty footprint (`footprint`).
Fails when the idle footprint (RSS elsewhere) is over the bound at any size, or grows by more
than the slack from the smallest repository to the largest: memory that scales with history
while nobody browses is the bug this catches, whatever cache holds it.

  scripts/memory-check.py [--bin PATH] [--sizes 2000,8000,32000] [--bound-mb 100]
"""

from __future__ import annotations

import argparse
import importlib.util
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
# RECORDS_IDLE is 30 s and the sweeper checks every quarter of it: 45 s covers both.
IDLE_SECS = 45
SLACK_MB = 25


def bench():
    spec = importlib.util.spec_from_file_location(
        "bench_lib", ROOT / "scripts/bench-lib.py"
    )
    if spec is None or spec.loader is None:
        sys.exit("cannot load scripts/bench-lib.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def footprint_mb(pid: int) -> float | None:
    """Dirty memory the kernel charges the process (macOS only); clean mapped pages excluded."""
    if not shutil.which("footprint"):
        return None
    out = subprocess.run(["footprint", str(pid)], capture_output=True, text=True).stdout
    m = re.search(r"Footprint:\s*([\d.]+)\s*(KB|MB|GB)", out)
    if not m:
        return None
    return float(m.group(1)) * {"KB": 1 / 1024, "MB": 1.0, "GB": 1024.0}[m.group(2)]


def make_repo(d: Path, commits: int) -> None:
    """`commits` commits, each touching three of 2000 files, written by fast-import in one go."""
    subprocess.run(["git", "init", "-q", "-b", "main", str(d)], check=True)
    lines = []
    for i in range(commits):
        lines += [
            "commit refs/heads/main",
            f"committer u{i % 50} <u{i % 50}@example.com> {1_600_000_000 + i * 600} +0000",
            "data <<EOF",
            f"feat(s{i % 7}): change {i}",
            "EOF",
        ]
        for k in range(3):
            body = "".join(f"fn f_{i}_{k}_{n}() {{}}\n" for n in range(4 + i % 9))
            data = body.encode()
            path = f"src/m{(i * 3 + k) % 40}/f{(i * 7 + k) % 2000}.rs"
            lines += [f"M 644 inline {path}", f"data {len(data)}", body.rstrip("\n")]
        lines.append("")
    subprocess.run(
        ["git", "fast-import", "--quiet"],
        cwd=d,
        input="\n".join(lines) + "\n",
        text=True,
        check=True,
    )
    subprocess.run(["git", "checkout", "-q", "main"], cwd=d, check=True)


def measure(
    br: Any, binary: str, repo: Path, logdir: Path
) -> tuple[float, float | None]:
    srv = br.Server(binary, repo, logdir)
    try:
        srv.wait_ready()
        prefix = br.site_prefix(srv.port)
        for path in br.pages(br.discover(repo, "HEAD"), "fn").values():
            br.get(srv.port, prefix + path)
        time.sleep(IDLE_SECS)
        return br.rss_mb(srv.pid), footprint_mb(srv.pid)
    finally:
        srv.stop()


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--bin", default=str(ROOT / "target/release/git-terrace"))
    ap.add_argument("--sizes", default="2000,8000,32000")
    ap.add_argument("--bound-mb", type=float, default=100)
    a = ap.parse_args()
    binary = str(Path(a.bin).resolve())
    if not Path(binary).exists():
        sys.exit(f"no binary at {binary}, run `make build` first")
    br = bench()
    seen = []
    with tempfile.TemporaryDirectory(prefix="gt-mem-") as tmp:
        for n in (int(s) for s in a.sizes.split(",")):
            d = Path(tmp) / f"r{n}"
            make_repo(d, n)
            rss, fp = measure(br, binary, d, Path(tmp))
            got = fp if fp is not None else rss
            seen.append(got)
            fps = f"{fp:.0f} MB" if fp is not None else "n/a"
            print(f"{n:>7} commits: idle RSS {rss:.0f} MB, footprint {fps}", flush=True)
    bad = [m for m in seen if m > a.bound_mb]
    grew = seen[-1] - seen[0]
    if bad or grew > SLACK_MB:
        print(
            f"FAIL: idle memory {', '.join(f'{m:.0f}' for m in seen)} MB; bound {a.bound_mb:.0f} "
            f"MB, growth {grew:.0f} MB (slack {SLACK_MB} MB)"
        )
        return 1
    print(f"ok: idle memory flat within {SLACK_MB} MB and under {a.bound_mb:.0f} MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
