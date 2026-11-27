#!/usr/bin/env python3
"""Proves no CI job runs close to its timeout: a slow runner would turn a green job red.

  ci-budget-check.py [--runs N]   exits 1 when a job's median duration over the last N successful
                                  main runs (default 10) exceeds 75% of its timeout-minutes
  ci-budget-check.py --self-test  planted slow and fast jobs, with gh stubbed

Timeouts come from the jobs of .github/workflows/*.yml. Durations come from `gh run list` and
`gh run view --json jobs` of ci.yml. A hosted job is matched to its workflow job by name (a
matrix name matches on the text before `${{`). Without gh, a network or any run the check prints
a note and exits 0: it is a step of the release gate (the self-test runs in `make lint`),
so lint never needs a network.
The failure names the job, its median and its limit; split the job, never raise the timeout.
"""

from __future__ import annotations

import json
import re
import statistics
import subprocess
import sys
from collections.abc import Callable
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WF = ".github/workflows"
SHARE = 0.75
RUNS = 10


def parse_jobs(text: str) -> dict[str, tuple[str, int]]:
    """Job id -> (display name, timeout-minutes) for each job that sets a timeout."""
    out: dict[str, tuple[str, int]] = {}
    inside, cur, name, timeout = False, None, "", None

    def flush() -> None:
        if cur and timeout is not None:
            out[cur] = (name or cur, timeout)

    for line in text.splitlines():
        if re.match(r"^jobs:\s*$", line):
            inside = True
            continue
        if inside and re.match(r"^\S", line):
            inside = False
        if not inside:
            continue
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if m:
            flush()
            cur, name, timeout = m.group(1), "", None
        elif m := re.match(r"^    name:\s*(.+?)\s*$", line):
            name = m.group(1).strip("'\"")
        elif m := re.match(r"^    timeout-minutes:\s*(\d+)\s*$", line):
            timeout = int(m.group(1))
    flush()
    return out


def timeouts(root: Path) -> dict[str, int]:
    """Display name -> timeout-minutes over every workflow file."""
    out: dict[str, int] = {}
    for f in sorted((root / WF).glob("*.yml")):
        for name, minutes in parse_jobs(f.read_text()).values():
            out[name] = minutes
    return out


def limit_for(job: str, limits: dict[str, int]) -> tuple[str, int] | None:
    """The workflow job a hosted job name belongs to: the hosted name ends with it, a reusable
    workflow prefixing its caller's name, and a matrix name matches on its fixed prefix."""
    for name, minutes in limits.items():
        fixed = name.split("${{")[0]
        if job == name or job.endswith(" / " + name) or (fixed != name and fixed and fixed in job):
            return name, minutes
    return None


def minutes(job: dict[str, str]) -> float | None:
    try:
        start = datetime.fromisoformat(job["startedAt"].replace("Z", "+00:00"))
        end = datetime.fromisoformat(job["completedAt"].replace("Z", "+00:00"))
    except (KeyError, ValueError, AttributeError):
        return None
    return (end - start).total_seconds() / 60


def problems(limits: dict[str, int], runs: list[list[dict[str, str]]]) -> list[str]:
    """Each run is its list of hosted jobs: name, startedAt, completedAt, conclusion."""
    seen: dict[str, list[float]] = {}
    for jobs in runs:
        for job in jobs:
            if job.get("conclusion") != "success":
                continue
            hit = limit_for(job.get("name", ""), limits)
            m = minutes(job)
            if hit and m is not None:
                seen.setdefault(job["name"], []).append(m)
    bad = []
    for job, mins in sorted(seen.items()):
        hit = limit_for(job, limits)
        assert hit
        median = statistics.median(mins)
        if median > SHARE * hit[1]:
            bad.append(
                f"{job}: median {median:.0f} min over {len(mins)} runs is over "
                f"{SHARE:.0%} of its {hit[1]} min timeout. Split the job, do not raise the timeout"
            )
    return bad


def gh(*args: str) -> str | None:
    try:
        r = subprocess.run(["gh", *args], capture_output=True, text=True, timeout=120, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return r.stdout if r.returncode == 0 else None


def fetch(n: int, call: Callable[..., str | None] = gh) -> list[list[dict[str, str]]] | None:
    listed = call(
        "run",
        "list",
        "-w",
        "ci.yml",
        "-b",
        "main",
        "-s",
        "success",
        "-L",
        str(n),
        "--json",
        "databaseId",
    )
    if listed is None:
        return None
    runs = []
    for r in json.loads(listed):
        view = call("run", "view", str(r["databaseId"]), "--json", "jobs")
        if view is None:
            return None
        runs.append(json.loads(view)["jobs"])
    return runs


def self_test() -> int:
    limits = {"Check (macos-latest)": 60, "Memory budget (${{ matrix.os }})": 60, "Cargo deny": 10}

    def job(name: str, mins: int) -> dict[str, str]:
        return {
            "name": name,
            "conclusion": "success",
            "startedAt": "2026-01-01T10:00:00Z",
            "completedAt": f"2026-01-01T{10 + mins // 60}:{mins % 60:02d}:00Z",
        }

    slow = [[job("Lint and test / Check (macos-latest)", m)] for m in (50, 47, 52)]
    fast = [[job("Lint and test / Check (macos-latest)", m)] for m in (30, 47, 25)]
    matrix = [[job("Lint and test / Memory budget (macos-latest)", 50)]]
    small = [[job("Cargo deny", 9)]]
    failed = [[{**job("Lint and test / Check (macos-latest)", 59), "conclusion": "failure"}]]
    ok = (
        len(problems(limits, slow)) == 1
        and "Check (macos-latest)" in problems(limits, slow)[0]
        and problems(limits, fast) == []
        and len(problems(limits, matrix)) == 1
        and len(problems(limits, small)) == 1
        and problems(limits, failed) == []
        and problems(limits, []) == []
    )
    stub = {"list": json.dumps([{"databaseId": 1}]), "view": json.dumps({"jobs": slow[0]})}
    got = fetch(1, lambda *a: stub[a[1]])
    ok = ok and got == [slow[0]] and fetch(1, lambda *a: None) is None
    live = timeouts(ROOT)
    ok = ok and bool(live) and all(m > 0 for m in live.values())
    print("ci-budget-check: self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


def main(argv: list[str]) -> int:
    if argv == ["--self-test"]:
        return self_test()
    n = RUNS
    if argv[:1] == ["--runs"] and len(argv) == 2:
        n = int(argv[1])
    elif argv:
        print(__doc__)
        return 2
    runs = fetch(n)
    if not runs:
        print("ci-budget-check: no gh, network or successful main run, skipped")
        return 0
    bad = problems(timeouts(ROOT), runs)
    for b in bad:
        print(f"ci-budget-check: {b}", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
