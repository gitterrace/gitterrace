#!/usr/bin/env python3
"""Runs the shared file-length lint over git-tracked files only.

The kit lists tracked and untracked files, so an untracked build directory or a dependency
folder in a clone is counted against the caps. Here the list is `git ls-files` alone, which
is what a landed commit holds. The lint itself is unchanged.

  python3 scripts/lint/lines-check.py [--repo DIR]
"""

from __future__ import annotations

import importlib
import runpy
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
LINT = ROOT / "vendor" / "schwarzwald" / "kit" / "lint"


def tracked(root: Path, exclude: list[str]) -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [f for f in out.split("\0") if f and not matches(f, exclude)]


sys.dont_write_bytecode = True  # the kit copy must stay equal to its manifest
sys.path.insert(0, str(LINT / "lib"))
config: Any = importlib.import_module("config")
matches = config.matches
setattr(config, "tracked", tracked)  # noqa: B010

if __name__ == "__main__":
    sys.argv = [str(LINT / "file-length.py"), *(sys.argv[1:] or ["--repo", str(ROOT)])]
    runpy.run_path(sys.argv[0], run_name="__main__")
