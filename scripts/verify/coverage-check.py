#!/usr/bin/env python3
"""Proves a test-only change keeps what the tests caught: compares two coverage-run.sh outputs.

  coverage-check.py BEFORE AFTER    exits 1 when AFTER lost anything BEFORE had
  coverage-check.py --self-test     a planted loss of each kind must fail, an equal run pass

Fails when, for product code (crates/*/src above each file's first #[cfg(test)]):
  - a line covered before is not covered after (the set, not the percentage),
  - a crate's branch coverage percentage drops,
  - a mutant caught before (caught.txt or timeout.txt) is missed after.
A run holds lcov.info or several lcov-N.info, the same tree measured again. A line that one
run covers and another does not is unstable (a race, a thread that ends after the test): it
counts as covered on the AFTER side when any run hit it and as covered on the BEFORE side only
when every run did, so only a line no run of AFTER reaches can read as lost. The same holds for a
crate's branch percentage (lowest before, highest after). One run per side keeps the old, noisy
reading. Lines compare by number, so the product source must be the same on both sides: a
commit that also edits product code needs its own proof. Missing mutant output on one side
is an error, never a pass.
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


def lcov_files(run: Path) -> list[Path]:
    files = sorted(run.glob("lcov-*.info")) or [run / "lcov.info"]
    for f in files:
        if not f.exists():
            raise SystemExit(f"coverage-check: {f} is missing")
    return files


def read_one(lcov: Path, starts: dict[str, int]):
    """Covered lines per file, and (hit, total) branches per crate, product code only."""
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


def read_lcov(run: Path):
    """(stable, any, low, high): lines every run / some run covered, branch ratio per crate."""
    starts = tests_start(run)
    runs = [read_one(f, starts) for f in lcov_files(run)]
    paths = {p for cov, _ in runs for p in cov}
    stable = {p: set.intersection(*(cov.get(p, set()) for cov, _ in runs)) for p in paths}
    union = {p: set.union(*(cov.get(p, set()) for cov, _ in runs)) for p in paths}
    ratios: dict[str, list[float]] = defaultdict(list)
    for _, br in runs:
        for crate, (h, t) in br.items():
            ratios[crate].append(h / t if t else 1.0)
    low = {c: min(v) for c, v in ratios.items()}
    high = {c: max(v) for c, v in ratios.items()}
    return stable, union, low, high


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
    stable_b, _, low_b, _ = read_lcov(before)
    _, any_a, _, high_a = read_lcov(after)
    for path in sorted(stable_b):
        lost = sorted(stable_b[path] - any_a.get(path, set()))
        if lost:
            errors.append(f"{path}: {len(lost)} line(s) no longer covered: {lost[:20]}")
    for crate in sorted(low_b):
        pb, pa = low_b[crate], high_a.get(crate, 0.0)
        if pa < pb:
            errors.append(f"{crate}: branch coverage {pb:.2%} -> {pa:.2%}")
    mb, ma = caught(before), caught(after)
    if (mb is None) != (ma is None):
        errors.append("mutants ran on one side only")
    elif mb is not None and ma is not None:
        for m in sorted(mb - ma):
            errors.append(f"mutant no longer caught: {m}")
    return errors


def unstable(run: Path) -> dict[str, list[int]]:
    """Lines some runs of one tree covered and others did not, per file."""
    stable, union, _, _ = read_lcov(run)
    return {p: sorted(union[p] - stable[p]) for p in sorted(union) if union[p] - stable[p]}


def report(before: Path, after: Path) -> int:
    errors = compare(before, after)
    for e in errors:
        print(f"coverage-check: {e}", file=sys.stderr)
    # Unstable lines are named, never dropped: a reader sees what the comparison forgave.
    for side, run in (("BEFORE", before), ("AFTER", after)):
        for path, nums in unstable(run).items():
            print(f"coverage-check: unstable in {side}: {path}: {len(nums)} line(s): {nums[:20]}")
    if not errors:
        stable, union, _, _ = read_lcov(after)
        lines = sum(len(v) for v in stable.values())
        flaky = sum(len(union[p] - stable[p]) for p in union)
        print(
            f"coverage-check: ok, {lines} covered lines kept, branches per crate not lower, "
            f"{flaky} unstable line(s) in AFTER"
        )
    return 1 if errors else 0


def self_test() -> int:
    base = (
        "SF:crates/gitterrace-web/src/a.rs\nDA:1,3\nDA:2,1\nDA:9,1\n"
        "BRDA:1,0,0,1\nBRDA:1,0,1,1\nend_of_record\n"
    )
    lost = base.replace("DA:2,1", "DA:2,0")
    # Each side is the list of runs of one tree, as lcov texts, then the mutant file it wrote.
    cases = {
        "equal": ([base], "caught.txt", [base], "caught.txt", 0),
        "line lost": ([base], "caught.txt", [lost], "caught.txt", 1),
        "line lost in every run": ([base] * 3, "caught.txt", [lost] * 3, "caught.txt", 1),
        "line lost after one lucky run": (
            [base] * 3,
            "caught.txt",
            [lost, base, lost],
            "caught.txt",
            0,
        ),
        "line unstable before": ([base, lost, base], "caught.txt", [lost] * 3, "caught.txt", 0),
        "branch drop": (
            [base],
            "caught.txt",
            [base.replace("BRDA:1,0,1,1", "BRDA:1,0,1,0")],
            "caught.txt",
            1,
        ),
        "mutant missed": ([base], "caught.txt", [base], "missed.txt", 1),
        "test line only": (
            [base],
            "caught.txt",
            [base.replace("DA:9,1", "DA:9,0")],
            "caught.txt",
            0,
        ),
    }
    failed = 0
    with tempfile.TemporaryDirectory() as tmp:
        for name, (runs_b, mf_b, runs_a, mf_a, want) in cases.items():
            sides = []
            for side, runs, mf in (("b", runs_b, mf_b), ("a", runs_a, mf_a)):
                d = Path(tmp, name, side)
                (d / "mutants.out").mkdir(parents=True)
                for i, text in enumerate(runs):
                    (d / f"lcov-{i + 1}.info").write_text(text)
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
