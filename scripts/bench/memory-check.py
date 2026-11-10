#!/usr/bin/env python3
"""Steady-state memory guard: an idle server's memory must not grow with the history it serves.

Builds synthetic repositories of growing size, serves each with the release binary, waits for
the index, visits the bench's page set (`pages` in bench-lib.py) and then leaves the server
idle until it reports an idle release in `/api/progress`. It then reads RSS and the dirty
footprint the way bench-run samples them (`rss_mb` and `footprint_mb` in bench-lib.py).
Fails when the idle footprint (RSS elsewhere) is over the bound at any size, or grows by more
than the slack from the smallest repository to the largest: memory that scales with history
while nobody browses is the bug this catches, whatever cache holds it.

  scripts/bench/memory-check.py [--bin PATH] [--sizes 2000,8000,32000] [--bound-mb 100]
"""

from __future__ import annotations

import argparse
import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
# RECORDS_IDLE is 30 s and the sweeper checks every quarter of it: a release later than this
# is a server that does not release.
RELEASE_WAIT_SECS = 120
SLACK_MB = 25


def bench():
    spec = importlib.util.spec_from_file_location(
        "bench_lib", ROOT / "scripts/bench/bench-lib.py"
    )
    if spec is None or spec.loader is None:
        sys.exit("cannot load scripts/bench/bench-lib.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


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


def assert_zero_decay(binary: str) -> None:
    """The footprint is honest only when freed pages go back at once: a binary running with
    jemalloc's default decay shows pages it no longer holds, and they grow with the work done.
    Read back what the binary runs with before measuring it."""
    env = {"TEST_ALLOC_DECAY": "1", "PATH": "/usr/bin:/bin"}
    out = subprocess.run([binary], env=env, capture_output=True, text=True)
    if out.returncode != 0 or out.stdout.strip() != "0 0 0 0":
        sys.exit(f"{binary} does not run with zero page decay: {out.stdout.strip()!r}")


def measure(
    br: Any, binary: str, repo: Path, logdir: Path
) -> tuple[float, float | None]:
    srv = br.Server(binary, repo, logdir)
    try:
        srv.wait_ready()
        prefix = br.site_prefix(srv.port)
        for path in br.pages(br.discover(repo, "HEAD"), "fn").values():
            br.get(srv.port, prefix + path)
        # Idle means after the server released what the pages built, not after a guessed time.
        try:
            br.wait_released(srv.port, RELEASE_WAIT_SECS)
        except RuntimeError as e:
            sys.exit(f"{e}: the server keeps its caches")
        return br.rss_mb(srv.pid), br.footprint_mb(srv.pid)
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
    assert_zero_decay(binary)
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
