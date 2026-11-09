#!/usr/bin/env python3
"""Golden tests, the same way in every product: find them, run them, and prove a re-bless.

A golden test compares output against a saved answer. Which files are golden tests and how to
run them is the product's, in schwarzwald/config/golden.toml:

    [golden]
    tests = "crates/*/tests/*.rs"        # glob over tracked files
    marker = "golden/"                   # a test file that names this is a golden test
    match = 'crates/(?P<crate>[^/]+)/tests/(?P<test>[^/]+)\\.rs'   # named groups for {..}
    run = ["cargo", "test", "-q"]        # one command ...
    each = ["-p", "{crate}", "--test", "{test}"]   # ... plus these words per test
    dump = ["cargo", "test", "--release", "-q", "-p", "x", "--test", "y"]   # writes $GOLDEN_DUMP
    compare = ["scripts/classify.sh", "{old}", "{new}", "{report}"]        # exit 0 = no real change

    golden.py list               the run command, one test per line
    golden.py run                run every golden test (a new one is found, never listed by hand)
    golden.py change OLD NEW     dump the answers at both commits (each in a throwaway worktree,
                                 into $GOLDEN_DUMP), then the product's compare. The re-bless proof.
    golden.py --self-test

Git identity variables are dropped for every command: a landing tool stamps its own, and tests
that build histories save answers made with theirs. Commands run as argv, never through a shell.
"""

from __future__ import annotations

import fnmatch
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import tomllib

sys.dont_write_bytecode = True
CONFIG = "schwarzwald/config/golden.toml"
SHA = re.compile(r"^[0-9a-f]{7,64}$")


def clean_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    drop = (
        "GIT_AUTHOR_",
        "GIT_COMMITTER_",
        "GIT_DIR",
        "GIT_INDEX_FILE",
        "GIT_WORK_TREE",
    )
    return {k: v for k, v in os.environ.items() if not k.startswith(drop)} | (
        extra or {}
    )


def load(root: Path) -> dict:
    p = root / CONFIG
    if not p.is_file():
        sys.exit(f"golden: no {CONFIG}, so no golden tests are known here")
    g = tomllib.loads(p.read_text(encoding="utf-8")).get("golden", {})
    missing = [k for k in ("tests", "marker", "match", "run", "each") if not g.get(k)]
    if missing:
        sys.exit(f"golden: {CONFIG} [golden] needs {', '.join(missing)}")
    return g


def targets(root: Path, files: list[str], g: dict) -> list[list[str]]:
    """The per-test words for every tracked file matching tests that names the marker."""
    rx = re.compile(g["match"])
    out = []
    for f in sorted(files):
        if not fnmatch.fnmatch(f, g["tests"]):
            continue
        try:
            if g["marker"] not in (root / f).read_text(
                encoding="utf-8", errors="replace"
            ):
                continue
        except OSError:
            continue
        m = rx.fullmatch(f)
        if not m:
            sys.exit(f"golden: {f} is a golden test but [golden] match does not fit it")
        out.append([w.format(**m.groupdict()) for w in g["each"]])
    return out


def tracked(root: Path) -> list[str]:
    r = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        capture_output=True,
        text=True,
        env=clean_env(),
        check=True,
    )
    return [f for f in r.stdout.split("\0") if f]


def command(root: Path, g: dict) -> list[str]:
    found = targets(root, tracked(root), g)
    if not found:
        sys.exit(
            "golden: no golden test found, refusing (a run over nothing proves nothing)"
        )
    return list(g["run"]) + [w for t in found for w in t]


def dump_at(root: Path, commit: str, into: Path, g: dict) -> None:
    wt = Path(tempfile.mkdtemp()) / "wt"
    git = ["git", "-C", str(root)]
    subprocess.run(
        [*git, "worktree", "add", "-q", "--detach", str(wt), commit],
        check=True,
        env=clean_env(),
    )
    try:
        into.mkdir(parents=True, exist_ok=True)
        # The dump may fail its own comparison at either commit: the answers are written first.
        subprocess.run(
            g["dump"],
            cwd=wt,
            env=clean_env({"GOLDEN_DUMP": str(into)}),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    finally:
        subprocess.run(
            [*git, "worktree", "remove", "--force", str(wt)], env=clean_env()
        )
    if not any(into.iterdir()):
        sys.exit(
            f"golden: no answers dumped at {commit}, so there is nothing to compare"
        )


def change(root: Path, old: str, new: str, g: dict) -> int:
    for k in ("dump", "compare"):
        if not g.get(k):
            sys.exit(f"golden: {CONFIG} [golden] needs {k} for a re-bless proof")
    shas = []
    for ref in (old, new):
        r = subprocess.run(
            [
                "git",
                "-C",
                str(root),
                "rev-parse",
                "--verify",
                "--end-of-options",
                f"{ref}^{{commit}}",
            ],
            capture_output=True,
            text=True,
            env=clean_env(),
        )
        sha = r.stdout.strip()
        if r.returncode or not SHA.match(sha):
            sys.exit(f"golden: {ref!r} is not a commit")
        shas.append(sha[:12])
    with tempfile.TemporaryDirectory() as t:
        work = Path(t)
        dump_at(root, shas[0], work / "old", g)
        dump_at(root, shas[1], work / "new", g)
        out = root / "target/golden-proof"
        out.mkdir(parents=True, exist_ok=True)
        report = out / f"change-{shas[0]}-{shas[1]}.txt"
        report.unlink(missing_ok=True)
        words = {
            "old": str(work / "old"),
            "new": str(work / "new"),
            "report": str(report),
        }
        rc = subprocess.run(
            [w.format(**words) for w in g["compare"]], cwd=root, env=clean_env()
        ).returncode
        if not report.is_file() or not report.stat().st_size:
            print(f"golden: the compare command wrote no report at {report}")
            return 1
        print(
            f"re-bless proof {shas[0]} -> {shas[1]}: {'no real change' if rc == 0 else 'REAL change'}"
        )
        print(f"report: {report}")
        return 0 if rc == 0 else 1


def self_test() -> int:
    g = {
        "tests": "crates/*/tests/*.rs",
        "marker": "golden/",
        "match": r"crates/(?P<crate>[^/]+)/tests/(?P<test>[^/]+)\.rs",
        "run": ["cargo", "test"],
        "each": ["-p", "{crate}", "--test", "{test}"],
    }
    with tempfile.TemporaryDirectory() as t:
        root = Path(t)
        for rel, body in (
            ("crates/a/tests/pages.rs", 'const G: &str = "tests/golden/x";'),
            ("crates/a/tests/other.rs", "fn plain() {}"),
            ("crates/b/tests/new_golden.rs", 'let p = "golden/y";'),
        ):
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text(body)
        files = [
            "crates/a/tests/other.rs",
            "crates/a/tests/pages.rs",
            "crates/b/tests/new_golden.rs",
        ]
        got = targets(root, files, g)
    if got != [["-p", "a", "--test", "pages"], ["-p", "b", "--test", "new_golden"]]:
        print(f"golden self-test FAILED: {got}")
        return 1
    print("golden self-test: ok")
    return 0


def main(argv: list[str]) -> int:
    if argv[:1] == ["--self-test"]:
        return self_test()
    root = Path(
        subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            env=clean_env(),
        ).stdout.strip()
        or "."
    ).resolve()
    if argv == ["list"]:
        g = load(root)
        print(" ".join(g["run"]))
        for t in targets(root, tracked(root), g):
            print("  " + " ".join(t))
        return 0
    if argv == ["run"]:
        return subprocess.run(
            command(root, load(root)), cwd=root, env=clean_env()
        ).returncode
    if len(argv) == 3 and argv[0] == "change":
        return change(root, argv[1], argv[2], load(root))
    print("usage: golden.py list | run | change OLD NEW | --self-test", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
