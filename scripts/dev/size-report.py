#!/usr/bin/env python3
"""Count repository lines per area and per kind, and print the total.

Counted: rs py js mjs json sh css html toml. Skipped: vendor/, *.lock,
tests/bench/results and golden/. Kinds: prod (non-test, non-comment Rust and
web assets), test (tests/ files, tests.rs, the tail after `#[cfg(test)]`),
comment (Rust and script comment lines), generated (json), tooling (scripts/,
xtask, tests/ harness scripts). Area is the first two path parts under crates/,
else the first part. Usage: size-report.py [--repo DIR] [--self-test]. Stdlib only.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

EXTS = {"rs", "py", "js", "mjs", "json", "sh", "css", "html", "toml"}
SKIP = ("vendor/", "tests/bench/results", "golden/")
LIMIT = 100_000


def files(repo: Path) -> list[str]:
    out = subprocess.run(
        ["git", "ls-files"], cwd=repo, capture_output=True, text=True, check=True
    ).stdout.split("\n")
    keep = []
    for p in out:
        if not p or p.endswith(".lock") or any(s in p for s in SKIP):
            continue
        if p.rsplit(".", 1)[-1] in EXTS:
            keep.append(p)
    return keep


def area(p: str) -> str:
    parts = p.split("/")
    if parts[0] == "crates" and len(parts) > 1:
        return "crates/" + parts[1]
    return parts[0] if len(parts) > 1 else "(root)"


def is_comment(line: str, ext: str) -> bool:
    s = line.strip()
    if ext in ("rs", "js", "mjs"):
        return s.startswith("//")
    if ext in ("py", "sh", "toml"):
        return s.startswith("#") and not s.startswith("#!")
    return False


def classify(p: str, text: str) -> dict[str, int]:
    ext = p.rsplit(".", 1)[-1]
    name = p.rsplit("/", 1)[-1]
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    c: dict[str, int] = defaultdict(int)
    whole = None
    if ext == "json":
        whole = "generated"
    elif p.startswith(("scripts/", "xtask/")) or "/scripts/" in p:
        whole = "tooling"
    elif "/tests/" in p or p.startswith("tests/") or name == "tests.rs":
        whole = "test"
    in_tests = False
    for ln in lines:
        if not in_tests and ext == "rs" and ln.strip() == "#[cfg(test)]":
            in_tests = True
        if whole == "test" or in_tests:
            c["test"] += 1
        elif is_comment(ln, ext):
            c["comment"] += 1
        elif whole:
            c[whole] += 1
        elif ln.strip():
            c["prod"] += 1
        else:
            c["blank"] += 1
    return c


def count(repo: Path):
    areas: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))
    kinds: dict[str, int] = defaultdict(int)
    for p in files(repo):
        try:
            text = (repo / p).read_text(errors="replace")
        except OSError:
            continue
        for k, n in classify(p, text).items():
            areas[area(p)][k] += n
            kinds[k] += n
    return areas, kinds


def report(repo: Path) -> int:
    areas, kinds = count(repo)
    cols = ["prod", "test", "comment", "generated", "tooling", "blank"]
    print("area".ljust(28) + "".join(c.rjust(10) for c in cols) + "total".rjust(10))
    for a, d in sorted(areas.items(), key=lambda kv: -sum(kv[1].values())):
        cells = "".join(str(d.get(c, 0)).rjust(10) for c in cols)
        print(a.ljust(28) + cells + str(sum(d.values())).rjust(10))
    total = sum(kinds.values())
    cells = "".join(str(kinds.get(c, 0)).rjust(10) for c in cols)
    print("TOTAL".ljust(28) + cells + str(total).rjust(10))
    print(f"lines: {total} (target under {LIMIT})")
    return 0


def self_test() -> int:
    with tempfile.TemporaryDirectory() as d:
        r = Path(d)
        sh = lambda *a: subprocess.run(a, cwd=r, check=True, capture_output=True)  # noqa: E731
        sh("git", "init", "-q")
        (r / "crates/x/src").mkdir(parents=True)
        (r / "crates/x/src/lib.rs").write_text("// c\nfn a() {}\n\n#[cfg(test)]\nmod t {}\n")
        (r / "scripts").mkdir()
        (r / "scripts/a.sh").write_text("#!/bin/sh\n# c\necho\n")
        (r / "golden").mkdir()
        (r / "golden/g.html").write_text("x\n")
        sh("git", "add", "-A")
        areas, kinds = count(r)
        want = {"comment": 2, "prod": 1, "blank": 1, "test": 2, "tooling": 2}
        want["tooling"] = 2
        # shebang line counts as tooling, the echo line too
        assert dict(kinds) == want, dict(kinds)
        assert "golden" not in areas and areas["crates/x"]["test"] == 2
    print("size-report self-test ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    repo = Path(sys.argv[sys.argv.index("--repo") + 1]) if "--repo" in sys.argv else Path(".")
    return report(repo)


if __name__ == "__main__":
    sys.exit(main())
