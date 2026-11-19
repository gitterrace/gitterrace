#!/usr/bin/env python3
"""Proves a test-only change keeps what the tests caught: compares two coverage-run.sh outputs.

  coverage-check.py BEFORE AFTER    exits 1 when AFTER lost anything BEFORE had
  coverage-check.py --self-test     a planted loss of each kind must fail, an equal run pass

Fails when, for product code (crates/*/src above each file's first #[cfg(test)]):
  - a line covered before is not covered after (the set, not the percentage),
  - a crate's branch coverage percentage drops,
  - a mutant caught before (caught.txt or timeout.txt) is missed after.
Lines compare by number, so the product source must be the same on both sides: a commit that
also edits product code needs its own proof. Missing mutant output on one side is an error,
never a pass.
"""

from __future__ import annotations

import re
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

CRATE = re.compile(r"^crates/([^/]+)/src/")


def tests_start(run: Path) -> dict[str, int]:
    tsv = run / "tests-start.tsv"
    out: dict[str, int] = {}
    if tsv.exists():
        for row in tsv.read_text().splitlines():
            path, line = row.split("\t")
            out[path] = int(line)
    return out


def read_lcov(run: Path):
    """Covered lines per file, and (hit, total) branches per crate, product code only."""
    lcov = run / "lcov.info"
    if not lcov.exists():
        raise SystemExit(f"coverage-check: {lcov} is missing")
    starts = tests_start(run)
    covered: dict[str, set[int]] = defaultdict(set)
    branches: dict[str, list[int]] = defaultdict(lambda: [0, 0])
    path = None
    for row in lcov.read_text().splitlines():
        if row.startswith("SF:"):
            path = row[3:]
            if not CRATE.match(path):
                path = None
            continue
        if path is None:
            continue
        end = starts.get(path, 10**9)
        if row.startswith("DA:"):
            line, hits = row[3:].split(",")[:2]
            if int(line) < end and int(hits) > 0:
                covered[path].add(int(line))
        elif row.startswith("BRDA:"):
            line, _, _, taken = row[5:].split(",")
            if int(line) < end:
                m = CRATE.match(path)
                if m is None:
                    continue
                crate = m.group(1)
                branches[crate][1] += 1
                branches[crate][0] += taken not in ("-", "0")
    return covered, branches


def caught(run: Path) -> set[str] | None:
    """Caught mutants by description, None when this run did not mutate."""
    dirs = [d for d in (run / "mutants.out", run) if (d / "caught.txt").exists()]
    if not dirs:
        return None
    out: set[str] = set()
    for name in ("caught.txt", "timeout.txt"):
        f = dirs[0] / name
        if f.exists():
            out |= {r.strip() for r in f.read_text().splitlines() if r.strip()}
    return out


def compare(before: Path, after: Path) -> list[str]:
    errors = []
    cov_b, br_b = read_lcov(before)
    cov_a, br_a = read_lcov(after)
    for path in sorted(cov_b):
        lost = sorted(cov_b[path] - cov_a.get(path, set()))
        if lost:
            errors.append(f"{path}: {len(lost)} line(s) no longer covered: {lost[:20]}")
    for crate in sorted(br_b):
        hb, tb = br_b[crate]
        ha, ta = br_a.get(crate, [0, 0])
        pb, pa = hb / tb if tb else 1.0, ha / ta if ta else 1.0
        if pa < pb:
            errors.append(f"{crate}: branch coverage {pb:.2%} -> {pa:.2%}")
    mb, ma = caught(before), caught(after)
    if (mb is None) != (ma is None):
        errors.append("mutants ran on one side only")
    elif mb is not None and ma is not None:
        for m in sorted(mb - ma):
            errors.append(f"mutant no longer caught: {m}")
    return errors


def report(before: Path, after: Path) -> int:
    errors = compare(before, after)
    for e in errors:
        print(f"coverage-check: {e}", file=sys.stderr)
    if not errors:
        cov, _ = read_lcov(after)
        lines = sum(len(v) for v in cov.values())
        print(f"coverage-check: ok, {lines} covered lines kept, branches per crate not lower")
    return 1 if errors else 0


def self_test() -> int:
    base = (
        "SF:crates/gitterrace-web/src/a.rs\nDA:1,3\nDA:2,1\nDA:9,1\n"
        "BRDA:1,0,0,1\nBRDA:1,0,1,1\nend_of_record\n"
    )
    cases = {
        "equal": (base, "caught.txt", 0),
        "line lost": (base.replace("DA:2,1", "DA:2,0"), "caught.txt", 1),
        "branch drop": (base.replace("BRDA:1,0,1,1", "BRDA:1,0,1,0"), "caught.txt", 1),
        "mutant missed": (base, "missed.txt", 1),
        "test line only": (base.replace("DA:9,1", "DA:9,0"), "caught.txt", 0),
    }
    failed = 0
    with tempfile.TemporaryDirectory() as tmp:
        for name, (lcov, mfile, want) in cases.items():
            sides = []
            for side, (text, mf) in {
                "b": (base, "caught.txt"),
                "a": (lcov, mfile),
            }.items():
                d = Path(tmp, name, side)
                (d / "mutants.out").mkdir(parents=True)
                (d / "lcov.info").write_text(text)
                (d / "tests-start.tsv").write_text("crates/gitterrace-web/src/a.rs\t5\n")
                (d / "mutants.out" / "caught.txt").write_text("")
                (d / "mutants.out" / mf).write_text("src/a.rs:1:1: replace f -> bool with true\n")
                sides.append(d)
            got = 1 if compare(*sides) else 0
            if got != want:
                print(
                    f"coverage-check self-test: {name}: want {want}, got {got}",
                    file=sys.stderr,
                )
                failed = 1
    if not failed:
        print(f"coverage-check self-test: {len(cases)} cases ok")
    return failed


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    return report(Path(sys.argv[1]), Path(sys.argv[2]))


if __name__ == "__main__":
    sys.exit(main())
