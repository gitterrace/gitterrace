#!/usr/bin/env python3
"""Run every shared lint on its red and green case: the red one must fail, the green one pass.

Each `red-cases/<script>/` (the lint's file name without `.py`) holds `red/` and `green/`, each a small tree. The runner copies a tree
into a fresh repository with one commit, puts the product's lint.toml in place when `--repo`
names a product (so the cases run against that product's real config), fills `{HEAD}` with the
commit's sha, and runs the rule. A file named `message` is checked as a commit message, the
rest as the tree. A rule that passes its red case is a dead guard.

  red.py [--repo DIR] [script...]   exit 0 when every case goes the way it must
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CASES = HERE / "red-cases"
SCRIPTS = {
    "internal-refs": "CONV-NO-INTERNAL-REFS",
    "personal-data": "CONV-PERSONAL-DATA",
    "own-shas": "CONV-NO-OWN-SHAS",
    "commit-subject": "CONV-COMMIT-SUBJECT",
}


def git(root: Path, *args: str) -> str:
    # A hook runs us with GIT_DIR and GIT_INDEX_FILE set: inherited, they point `git init` in a
    # scratch dir at the real repository and re-initialize it as bare.
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=True,
                          env=env).stdout


def run_case(rule: str, case: Path, product: Path | None) -> int:
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        git(root, "init", "-q")
        shutil.copytree(case, root, dirs_exist_ok=True)
        if product and (product / "config/schwarzwald/lint.toml").exists():
            (root / "config/schwarzwald").mkdir(parents=True)
            shutil.copy(product / "config/schwarzwald/lint.toml", root / "config/schwarzwald/lint.toml")
        git(root, "add", "-A")
        git(root, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-qm", "case")
        head = git(root, "rev-parse", "HEAD").strip()
        for f in root.rglob("*"):
            if f.is_file() and ".git" not in f.parts and "{HEAD}" in f.read_text(errors="replace"):
                f.write_text(f.read_text().replace("{HEAD}", head[:10]))
        args = ["--repo", str(root)]
        if (root / "message").exists():
            args += ["--message", str(root / "message")]
        else:
            args += [str(p.relative_to(root)) for p in sorted(root.rglob("*"))
                     if p.is_file() and ".git" not in p.parts and "config" not in p.parts]
            if rule != "internal-refs":
                args = args[:2]  # these scan the whole tree
        r = subprocess.run([sys.executable, str(HERE / f"{rule}.py"), *args], capture_output=True, text=True)
        return r.returncode


def main(argv: list[str]) -> int:
    product = None
    if "--repo" in argv:
        i = argv.index("--repo")
        product, argv = Path(argv[i + 1]).resolve(), argv[:i] + argv[i + 2 :]
    rules = argv or sorted(SCRIPTS)
    bad = [f"{d.name}: a case dir for no rule" for d in CASES.iterdir() if d.is_dir() and d.name not in SCRIPTS]
    bad += [f"{r}: no red case" for r in SCRIPTS if not (CASES / r / "red").is_dir()]
    for rule in rules:
        if not (CASES / rule / "red").is_dir():
            continue
        red, green = run_case(rule, CASES / rule / "red", product), run_case(rule, CASES / rule / "green", product)
        if red != 1:
            bad.append(f"{rule}: its red case exits {red}, must fail with 1")
        if green != 0:
            bad.append(f"{rule}: its green case exits {green}, must pass")
        print(f"{'OK ' if red == 1 and green == 0 else 'RED'} {SCRIPTS[rule]}: red exits {red}, green exits {green}")
    for b in bad:
        print(f"FAIL {b}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
