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
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
from config import git  # noqa: E402

HERE = Path(__file__).resolve().parent
CASES = HERE / "red-cases"
# Every lint in this folder is a rule: its file stem names its case dir, its RULE line its id.
# Discovered, not listed, so a new lint without a red case fails here instead of going unchecked.
RUNNERS = {"red", "run"}
SCRIPTS = {
    p.stem: (re.search(r'^RULE = "([^"]+)"', p.read_text(encoding="utf-8"), re.M) or [None, p.stem])[1]
    for p in sorted(HERE.glob("*.py")) if p.stem not in RUNNERS
}


def run_case(rule: str, case: Path, product: Path | None) -> int:
    with tempfile.TemporaryDirectory() as outer:
        # The private list sits beside the case repository, never inside it.
        d = Path(outer) / "repo"
        d.mkdir()
        root = d
        git(root, "init", "-q")
        shutil.copytree(case, root, dirs_exist_ok=True)
        if product and (product / "config/schwarzwald/lint.toml").exists() and not (root / "config").exists():
            (root / "config/schwarzwald").mkdir(parents=True)
            # A product's known-problem lists name its own files, which a case does not have.
            text = (product / "config/schwarzwald/lint.toml").read_text(encoding="utf-8")
            text = re.sub(r"(?ms)^known\s*=\s*\[.*?\]\s*$", "", text)
            (root / "config/schwarzwald/lint.toml").write_text(text, encoding="utf-8")
        # A case's `private-denylist` file stands for the private list: it moves outside the repository
        # and lint.toml [denylist] path names it, `{DENYLIST}` in the file standing for that path.
        private = root / "private-denylist"
        keep = Path(outer) / "private" / "denylist"
        if private.exists():
            keep.parent.mkdir()
            shutil.move(str(private), keep)
            os.utime(keep)  # fresh: a synced file keeps an old time and would read as stale
        refresh = root / "private-refresh.toml"  # the private refresh command, beside the list
        if refresh.exists():
            keep.parent.mkdir(exist_ok=True)
            (keep.parent / "denylist-refresh.toml").write_text(
                refresh.read_text(encoding="utf-8").replace("{DENYLIST}", str(keep)), encoding="utf-8")
            refresh.unlink()
        parked = root / "parked.marker"  # marker: the case parks a `.personal-denylist` in the tree
        if parked.exists():
            parked.rename(root / ".personal-denylist")
        cfg_file = root / "config/schwarzwald/lint.toml"
        cfg_file.parent.mkdir(parents=True, exist_ok=True)
        text = cfg_file.read_text(encoding="utf-8").replace("{DENYLIST}", str(keep)) if cfg_file.exists() else ""
        line = f'path = "{keep}"'
        if "{ROOT}" in text:  # the case names its own path inside the repository
            text = text.replace("{ROOT}", str(root))
        elif "[denylist]" in text:
            text = text.replace("[denylist]", f"[denylist]\n{line}", 1)
        else:
            text = f"{text}\n[denylist]\n{line}\n"
        cfg_file.write_text(text, encoding="utf-8")
        # The config now names a temp path: kept out of the tree the lint scans.
        with (root / ".git/info/exclude").open("a", encoding="utf-8") as ex:
            ex.write("config/schwarzwald/lint.toml\n")
        fifo = root / "fifo.marker"  # marker: a tracked symlink to a FIFO outside the tree
        if fifo.exists():
            fifo.unlink()
            os.mkfifo(Path(outer) / "fifo")
            (root / "link.txt").symlink_to(Path(outer) / "fifo")
        git(root, "add", "-A")
        git(root, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-qm", "case")
        old = root / "stale-denylist.marker"  # marker: the case wants a stale private denylist
        if old.exists():
            old.unlink()
            os.utime(keep, (1, 1))
        if (root / "backdated.marker").exists():  # marker: one more commit whose author date is older
            (root / "backdated.marker").unlink()
            git(root, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-qam",
                "case: backdated", env_extra={"GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z"})
        if (root / "inverted.marker").exists():  # marker: one more commit dated before its parent
            (root / "inverted.marker").unlink()
            old_date = {"GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z", "GIT_COMMITTER_DATE": "2000-01-01T00:00:00Z"}
            git(root, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-qam",
                "case: inverted", env_extra=old_date)
        widen = root / "widen.toml"  # marker: one more commit whose lint.toml switches lints off
        if widen.exists():
            git(root, "add", "-f", "config/schwarzwald/lint.toml")
            git(root, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-qm", "case: base")
            cfg_file.write_text(widen.read_text(encoding="utf-8") + f"\n[denylist]\n{line}\n", encoding="utf-8")
            widen.unlink()
            git(root, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-qam", "case: widen")
        head = git(root, "rev-parse", "HEAD").strip()
        if (root / "shallow.marker").exists():  # marker: the clone is shallow (git reads .git/shallow)
            (root / "shallow.marker").unlink()
            (root / ".git/shallow").write_text(head + "\n")
        for f in root.rglob("*"):
            if f.is_file() and ".git" not in f.parts and "{HEAD}" in f.read_text(errors="replace"):
                f.write_text(f.read_text().replace("{HEAD}", head[:10]))
        args = ["--repo", str(root)]
        if not widen.exists() and git(root, "log", "-1", "--format=%s") == "case: widen\n":
            args += ["--messages", "HEAD~1"]
        elif (root / "message").exists():
            args += ["--message", str(root / "message")]
        else:
            args += [str(p.relative_to(root)) for p in sorted(root.rglob("*"))
                     if p.is_file() and ".git" not in p.parts and "config" not in p.parts]
            if rule != "internal-refs":
                args = args[:2]  # these scan the whole tree
        try:  # a hung rule is a failed one, never a stuck runner
            r = subprocess.run([sys.executable, str(HERE / f"{rule}.py"), *args], capture_output=True,
                               text=True, timeout=60)
        except subprocess.TimeoutExpired:
            return 124
        return r.returncode


def main(argv: list[str]) -> int:
    product = None
    if "--repo" in argv:
        i = argv.index("--repo")
        product, argv = Path(argv[i + 1]).resolve(), argv[:i] + argv[i + 2 :]
    # Below the disk floor a case dir can be half written and pass for the wrong reason. The
    # floor is in the tools part, checked when the product took it.
    floor_py = HERE.parent / "tools/disk-floor.py"
    if floor_py.exists():
        floor = subprocess.run([sys.executable, str(floor_py), "--quiet",
                                *(["--repo", str(product)] if product else []), tempfile.gettempdir()])
        if floor.returncode:
            return floor.returncode
    rules = argv or sorted(SCRIPTS)
    bad = [f"{d.name}: a case dir for no rule" for d in CASES.iterdir() if d.is_dir() and d.name not in SCRIPTS]
    bad += [f"{r}: no red case" for r in SCRIPTS if not (CASES / r / "red").is_dir()]
    for rule in rules:
        if not (CASES / rule / "red").is_dir():
            continue
        reds = {d.name: run_case(rule, d, product) for d in sorted((CASES / rule).glob("red*")) if d.is_dir()}
        greens = {d.name: run_case(rule, d, product) for d in sorted((CASES / rule).glob("green*")) if d.is_dir()}
        for name, code in reds.items():
            if code != 1:
                bad.append(f"{rule}: its {name} case exits {code}, must fail with 1")
        for name, code in greens.items():
            if code != 0:
                bad.append(f"{rule}: its {name} case exits {code}, must pass")
        red = 1 if all(c == 1 for c in reds.values()) else sorted(c for c in reds.values() if c != 1)[0]
        green = 0 if all(c == 0 for c in greens.values()) else sorted(c for c in greens.values() if c)[0]
        print(f"{'OK ' if red == 1 and green == 0 else 'RED'} {SCRIPTS[rule]}: red exits {red}, green exits {green}")
    for b in bad:
        print(f"FAIL {b}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
