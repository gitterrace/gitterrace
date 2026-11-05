#!/usr/bin/env python3
"""The tools part on a throwaway repository: each tool does its job and refuses what it must.

  land.sh        lands a clean commit with a fresh date and the same message, refuses a partial
                 step (exit 3), a dirty main (exit 1), and skips a change already on main
  id-scan        finds an id and an own sha in the history
  ratchet-check  its own self-test: growth, a raised count and a dropped ratchet go red

    selftest.py      exit 0 when every case holds, else one line per broken case
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ENV = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.invalid", GIT_COMMITTER_NAME="t",
           GIT_COMMITTER_EMAIL="t@example.invalid", GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1")


def run(*args: str, cwd: Path | None = None, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(list(args), cwd=cwd, capture_output=True, text=True, env=env or ENV)


def git(root: Path, *args: str, date: str | None = None) -> str:
    env = dict(ENV, **({"GIT_AUTHOR_DATE": date, "GIT_COMMITTER_DATE": date} if date else {}))
    r = run("git", "-C", str(root), *args, env=env)
    if r.returncode:
        raise SystemExit(f"git {' '.join(args)}: {r.stderr}")
    return r.stdout.strip()


def commit(root: Path, name: str, text: str, msg: str, date: str = "1700000000 +0000") -> str:
    (root / name).write_text(text)
    git(root, "add", name)
    git(root, "commit", "-qm", msg, date=date)
    return git(root, "rev-parse", "HEAD")


def land_cases(d: Path) -> list[str]:
    bad = []
    main, clone = d / "main", d / "clone"
    main.mkdir()
    git(main, "init", "-q", "-b", "main")
    commit(main, "a.txt", "a\n", "feat: a")
    git(d, "clone", "-q", str(main), str(clone))
    good = commit(clone, "b.txt", "b\n", "feat: b\n\nThe b file.")
    fix = commit(clone, "c.txt", "c\n", "fixup! feat: b")
    land = ["bash", str(HERE / "land.sh"), str(main), str(clone)]
    r = run(*land, good)
    if r.returncode != 0 or git(main, "log", "-1", "--format=%B") != "feat: b\n\nThe b file.":
        bad.append(f"land.sh did not land a clean commit with its message: {r.stderr.strip()}")
    elif int(git(main, "log", "-1", "--format=%at")) <= 1700000000:
        bad.append("land.sh kept the old author date")
    if run(*land, good).stdout.strip() != f"already on main: {good}":
        bad.append("land.sh landed a change already on main again")
    r = run(*land, fix)
    if r.returncode != 3:
        bad.append(f"land.sh let a fixup! subject through (exit {r.returncode})")
    (main / "dirty").write_text("x")
    if run(*land, fix).returncode != 1:
        bad.append("land.sh landed onto a dirty main")
    (main / "dirty").unlink()
    if run("bash", str(HERE / "land.sh"), str(main)).returncode != 1:
        bad.append("land.sh ran without a repository argument")
    return bad


def history_cases(d: Path) -> list[str]:
    bad = []
    repo = d / "hist"
    repo.mkdir()
    git(repo, "init", "-q", "-b", "main")
    commit(repo, "a.md", "start\n", "docs: start")
    old = commit(repo, "a.md", "start\nThe rule, see 4f4f4a for details.\n", "docs: the rule, see 4f4f4a")
    commit(repo, "r.json", f'{{"sha": "{old[:10]}"}}\n', "docs: results")
    commit(repo, "a.md", "start\nThe rule.\n", "docs: the rule")
    tip = commit(repo, "r.json", '{"sha": "HEAD"}\n', "docs: results at HEAD")
    (repo / ".git/info/exclude").write_text(".personal-denylist\nconfig/\n")
    (repo / ".personal-denylist").write_text("4f4f4a\n")
    (repo / "config/schwarzwald").mkdir(parents=True)
    (repo / "config/schwarzwald/lint.toml").write_text(
        "[history]\n"
        "rewrites = [[',? ?see (?:{ids})( for details)?', ''], ['\\\\b(?:{ids})\\\\b', 'the rule']]\n"
        "sha_fields = ['(\"sha\": \")([0-9a-f]{7,40})(\")']\n")
    scan = [sys.executable, str(HERE / "id-scan.py"), str(repo)]
    r = run(*scan, tip)
    if r.returncode != 1 or "4f4f4a" not in r.stdout or f"own sha {old[:10]}" not in r.stdout:
        bad.append(f"id-scan missed the id or the own sha in the old history: {r.stdout.strip()}")
    return bad


def main() -> int:
    with tempfile.TemporaryDirectory() as d:
        bad = land_cases(Path(d)) + history_cases(Path(d))
    r = run(sys.executable, str(HERE / "ratchet-check.py"), "--self-test")
    if r.returncode != 0:
        bad.append(f"ratchet-check self-test failed: {(r.stdout + r.stderr).strip()[-300:]}")
    for b in bad:
        print(f"FAIL {b}")
    print("ok" if not bad else f"{len(bad)} broken")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
