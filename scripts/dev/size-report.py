#!/usr/bin/env python3
"""Report the project's size: gitterrace's languages insight, split into prod, test and comment.

The languages insight (`git-terrace insights languages`) is the one size metric: the total
and the lines per language come from it, the same numbers the web Languages page and
`/api/v1/insights/languages` show for the HEAD tree. This script counts nothing for the total.
It keeps only a breakdown of that total by kind, and the breakdown sums to it: lines of the
files below are classified, every other line of the total is `other`. A classified count above
the total, or a sum that differs from it, is an error.

Classified: rs py js mjs json sh css html toml at HEAD, minus vendor/, *.lock,
tests/bench/results and golden/. Kinds: prod (non-test, non-comment Rust and
web assets), test (tests/ files, tests.rs, the tail after `#[cfg(test)]`),
comment (Rust and script comment lines), generated (json), tooling (scripts/,
xtask, tests/ harness scripts), blank. Area is the first two path parts under crates/,
else the first part.

Usage: size-report.py [--repo DIR] [--self-test]. Stdlib only. The binary is $GITTERRACE_BIN,
else target/debug/git-terrace of the repository, built with cargo when missing.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path
from typing import Any

EXTS = {"rs", "py", "js", "mjs", "json", "sh", "css", "html", "toml"}
SKIP = ("vendor/", "tests/bench/results", "golden/")
LIMIT = 100_000


def files(repo: Path) -> list[str]:
    out = subprocess.run(
        ["git", "ls-tree", "-r", "--name-only", "HEAD"],
        cwd=repo,
        capture_output=True,
        text=True,
        check=True,
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


def blobs(repo: Path, paths: list[str]) -> dict[str, str]:
    """The text of each path at HEAD, read through one `git cat-file --batch`."""
    if not paths:
        return {}
    ask = "".join(f"HEAD:{p}\n" for p in paths).encode()
    out = subprocess.run(
        ["git", "cat-file", "--batch"], cwd=repo, input=ask, capture_output=True, check=True
    ).stdout
    texts, at = {}, 0
    for p in paths:
        end = out.index(b"\n", at)
        head = out[at:end].split()
        if len(head) < 3 or head[1] != b"blob":
            at = end + 1
            continue
        size = int(head[2])
        texts[p] = out[end + 1 : end + 1 + size].decode(errors="replace")
        at = end + 1 + size + 1
    return texts


def count(repo: Path):
    areas: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))
    kinds: dict[str, int] = defaultdict(int)
    paths = files(repo)
    for p, text in blobs(repo, paths).items():
        for k, n in classify(p, text).items():
            areas[area(p)][k] += n
            kinds[k] += n
    return areas, kinds


def binary(repo: Path) -> list[str]:
    """The gitterrace command: $GITTERRACE_BIN, else the repository's debug build."""
    if os.environ.get("GITTERRACE_BIN"):
        return [os.environ["GITTERRACE_BIN"]]
    exe = Path(os.environ.get("CARGO_TARGET_DIR", repo / "target")) / "debug" / "git-terrace"
    subprocess.run(["cargo", "build", "-q", "-p", "gitterrace"], cwd=repo, check=True)
    return [str(exe)]


def languages(repo: Path, cmd: list[str]) -> dict[str, Any]:
    """The `languages` insight of `repo` at HEAD, from `git-terrace insights languages --json`."""
    run = subprocess.run(
        [*cmd, "insights", "languages", "--json"],
        cwd=repo,
        capture_output=True,
        text=True,
        timeout=900,
    )
    if run.returncode:
        raise SystemExit(f"size-report: the languages insight failed: {run.stderr[-400:]}")
    return json.loads(run.stdout)["data"]


def breakdown(total: int, kinds: dict[str, int]) -> dict[str, int]:
    """`kinds` plus `other`, the rest of `total`. Sums to `total` or the two disagree."""
    split = sum(kinds.values())
    if split > total:
        raise SystemExit(
            f"size-report: the kinds count {split} lines, more than the languages total {total}"
        )
    parts = {**kinds, "other": total - split}
    if sum(parts.values()) != total:
        raise SystemExit(f"size-report: the breakdown sums to {sum(parts.values())}, not {total}")
    return parts


def report(repo: Path) -> int:
    data = languages(repo, binary(repo))
    total = data["total_lines"]
    areas, kinds = count(repo)
    parts = breakdown(total, kinds)
    print(f"lines: {total} ({data['files']} files, languages insight at HEAD)")
    for lang in data["languages"][:12]:
        share = lang["lines"] * 100 / max(total, 1)
        print(f"  {lang['language']:<16}{lang['lines']:>10}{share:>7.1f}%")
    print("breakdown (sums to the total)")
    for k in ("prod", "test", "comment", "generated", "tooling", "blank", "other"):
        print(f"  {k:<16}{parts.get(k, 0):>10}")
    print("area".ljust(28) + "classified".rjust(10))
    for a, d in sorted(areas.items(), key=lambda kv: -sum(kv[1].values())):
        print(a.ljust(28) + str(sum(d.values())).rjust(10))
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
        (r / "NOTES.txt").write_text("one\ntwo\n")
        sh("git", "add", "-A")
        sh("git", "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init")
        areas, kinds = count(r)
        want = {"comment": 2, "prod": 1, "blank": 1, "test": 2, "tooling": 2}
        # shebang line counts as tooling, the echo line too
        assert dict(kinds) == want, dict(kinds)
        assert "golden" not in areas and areas["crates/x"]["test"] == 2
        # the breakdown sums to the total, and disagreement is an error
        parts = breakdown(10, dict(kinds))
        assert sum(parts.values()) == 10 and parts["other"] == 2, parts
        try:
            breakdown(7, dict(kinds))
        except SystemExit:
            pass
        else:
            raise AssertionError("a classified count above the total must fail")
        # the total is gitterrace's: ask it about this repository and compare
        data = languages(r, binary(Path(__file__).resolve().parents[2]))
        assert data["total_lines"] == sum(x["lines"] for x in data["languages"]), data
        parts = breakdown(data["total_lines"], dict(kinds))
        assert sum(parts.values()) == data["total_lines"], (parts, data)
    print("size-report self-test ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    repo = Path(sys.argv[sys.argv.index("--repo") + 1]) if "--repo" in sys.argv else Path(".")
    return report(repo)


if __name__ == "__main__":
    sys.exit(main())
