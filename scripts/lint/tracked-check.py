#!/usr/bin/env python3
"""Proves the gate checks walk git-tracked files only, in a clone full of untracked files.

A copy of the tracked tree gets a fresh history, then an untracked node_modules with tens of
thousands of files (their names alone overflow the argument limit), an untracked build
directory with a long file and an untracked wide file. `make lines names`, the file-list
scripts and the land-check crate selection must all still pass.

  python3 scripts/lint/tracked-check.py --self-test
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GIT = ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid"]
MODULES = 25000


def run(cmd: list[str], cwd: Path) -> tuple[int, str]:
    p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, check=False)
    return p.returncode, p.stdout + p.stderr


def tracked_copy(dest: Path) -> None:
    out = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True
    ).stdout
    for rel in filter(None, out.decode().split("\0")):
        src = ROOT / rel
        if not src.exists():
            continue
        (dest / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dest / rel, follow_symlinks=False)


def litter(repo: Path) -> None:
    mods = repo / "node_modules" / "pkg"
    mods.mkdir(parents=True)
    for i in range(MODULES):
        (mods / f"module_{i:06d}_{'x' * 70}.js").touch()
    build = repo / "target-main" / "debug"
    build.mkdir(parents=True)
    (build / "big.rs").write_text("x\n" * 900)
    (repo / "wide.rs").write_text("let x = 1;" + " " * 150 + "\n")


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp) / "clone"
        repo.mkdir()
        tracked_copy(repo)
        (repo / ".gitignore").unlink(missing_ok=True)
        run(["git", "init", "-q", "-b", "main"], repo)
        run(["git", "config", "core.hooksPath", "scripts/hooks"], repo)
        run(["git", "add", "-A"], repo)
        run([*GIT, "commit", "-q", "-m", "chore: fixture"], repo)
        (repo / "README.md").write_text((repo / "README.md").read_text() + "\n")
        litter(repo)
        steps = [
            ("make lines names", ["make", "lines", "names"]),
            (
                "land-check selection",
                ["bash", "scripts/land-check.sh", "--base", "HEAD", "--explain"],
            ),
        ]
        bad = []
        for name, cmd in steps:
            code, out = run(cmd, repo)
            if code:
                bad.append(f"{name} failed with untracked files present:\n{out[-600:]}")
    for b in bad:
        print(b, file=sys.stderr)
    print("tracked-check", "FAILED" if bad else "ok")
    return 1 if bad else 0


if __name__ == "__main__":
    if "--self-test" not in sys.argv:
        sys.exit(__doc__)
    os.environ.pop("MAKEFLAGS", None)
    sys.exit(self_test())
