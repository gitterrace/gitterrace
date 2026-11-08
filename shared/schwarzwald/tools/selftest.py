#!/usr/bin/env python3
"""The tools part on a throwaway repository: each tool does its job and refuses what it must.

  land.sh        lands a clean commit with a fresh date and the same message, refuses a partial
                 step (exit 3), a dirty main (exit 1), and skips a change already on main
  id-scan        finds the id and the own sha in an old history
  ratchet-check  its own self-test: growth, a raised count and a dropped ratchet go red
  disk guard     disk_test.py: the floor refuses, an ENOSPC worktree fails the run, the sweep
                 lists only stale, closed build output

    selftest.py      exit 0 when every case holds, else one line per broken case
"""

from __future__ import annotations

import os
import re
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
    # The fixture has no land.toml, so the landing gate is opted out of explicitly.
    land = ["bash", str(HERE / "land.sh"), "--no-gate", str(main), str(clone)]
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


def history_repo(d: Path) -> tuple[Path, str, str, str]:
    """A throwaway history with an id in an old commit and an own sha in a sha field: repo, base, old, tip."""
    repo = d / "hist"
    repo.mkdir()
    git(repo, "init", "-q", "-b", "main")
    base = commit(repo, "a.md", "start\n", "docs: start")
    old = commit(repo, "a.md", "start\nThe rule, see 4f4f4a for details.\n", "docs: the rule, see 4f4f4a")
    commit(repo, "r.json", f'{{"sha": "{old[:10]}"}}\n', "docs: results")
    commit(repo, "a.md", "start\nThe rule.\n", "docs: the rule")
    tip = commit(repo, "r.json", '{"sha": "HEAD"}\n', "docs: results at HEAD")
    (repo / ".git/info/exclude").write_text("config/\n")
    keep = d / "private"
    keep.mkdir()
    (keep / "denylist").write_text("4f4f4a\n")
    (repo / "config/schwarzwald").mkdir(parents=True)
    (repo / "config/schwarzwald/lint.toml").write_text(f'[denylist]\npath = "{keep}/denylist"\n')
    return repo, base, old, tip


def id_scan(repo: Path, rev: str) -> subprocess.CompletedProcess[str]:
    return run(sys.executable, str(HERE / "id-scan.py"), str(repo), rev)


def history_cases(d: Path) -> list[str]:
    repo, _, old, tip = history_repo(d)
    r = id_scan(repo, tip)
    if r.returncode != 1 or "4f4f4a" not in r.stdout or f"own sha {old[:10]}" not in r.stdout:
        return [f"id-scan missed the id or the own sha in the old history: {r.stdout.strip()}"]
    return []


GIT_WRITE = re.compile(
    r"(^|[^\w./-])git\s+(-c\s+\S+\s+|-C\s+\S+\s+)*"
    r"(init|commit|add|reset|checkout|rm|apply|stash|clean|switch|merge|rebase|tag|branch|"
    r"update-ref|read-tree|commit-tree|config|worktree|clone)(\s|$)")
MAKE_RUN = re.compile(r"(^|[\s;&|(]|\$\()([A-Z_]+=\S+\s+)*make([\s>]|$)")


def sandbox_cases(root: Path) -> list[str]:
    """Every *_test.sh under root that writes to a git repo or runs make sources tools/test-sandbox.sh."""
    tests = sorted(root.rglob("*_test.sh"))
    if not tests:
        return [f"sandbox scan found ZERO *_test.sh under {root}; a check that cannot look has not passed"]
    bad = []
    for f in tests:
        text = f.read_text()
        if "test-sandbox.sh" in text:
            continue
        for n, line in enumerate(text.splitlines(), 1):
            code = line.strip()
            if code and not code.startswith("#") and (GIT_WRITE.search(code) or MAKE_RUN.search(code)):
                bad.append(f"{f.relative_to(root)}:{n} mutates a repo or runs make without sourcing test-sandbox.sh")
                break
    return bad


def merge_cases() -> list[str]:
    """denylist-merge.py unions and never shrinks: a refresh with fewer ids keeps every old entry."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("denylist_merge", HERE / "denylist-merge.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    cur = "aaaaaa\nbbbbbb\nsecret-name\n"
    out = mod.merge(cur, "cccccc and nothing else\n", "dddddd\n").split()
    bad = [f"denylist-merge dropped {x!r}" for x in ("aaaaaa", "bbbbbb", "secret-name") if x not in out]
    bad += [f"denylist-merge lost the new id {x!r}" for x in ("cccccc", "dddddd") if x not in out]
    js = '[{"code": "eeeeee", "title": "see abc123 in the note"}]'
    if "abc123" in mod.merge("", js).split() or "eeeeee" not in mod.merge("", js).split():
        bad.append("denylist-merge took a hex word from a JSON title, or missed its code field")
    if mod.merge(cur, "").split() != cur.split():
        bad.append("denylist-merge with an empty refresh changed the list")
    bad += private_write_cases("denylist-merge", lambda p: mod.write_private(p, "aaaaaa\n"))
    return bad


def private_write_cases(name: str, write) -> list[str]:
    """ST-7: the private list's folder ends at 0700 even when it existed wider, and a path inside
    a git work tree, or a symlink at the path, is refused (SystemExit) and writes nothing."""
    bad = []
    with tempfile.TemporaryDirectory() as d:
        root = Path(d).resolve()
        wide = root / "wide"
        wide.mkdir(mode=0o755)
        wide.chmod(0o755)
        try:
            write(wide / "list")
        except SystemExit as e:
            bad.append(f"{name} refused a plain private path: {e}")
        if oct(wide.stat().st_mode & 0o777) != "0o700":
            bad.append(f"{name} left an existing folder at {oct(wide.stat().st_mode & 0o777)}, not 0700")
        other = root / "other"
        (other / ".git").mkdir(parents=True)
        for label, target in (("inside another repo", other / "sub/denylist"),):
            try:
                write(target)
                bad.append(f"{name} wrote the list {label}")
            except SystemExit:
                pass
            if target.exists():
                bad.append(f"{name} left a file {label}")
        (wide / "link").symlink_to(other / "leak")
        try:
            write(wide / "link")
            bad.append(f"{name} wrote through a symlink at the path")
        except SystemExit:
            pass
        if (other / "leak").exists():
            bad.append(f"{name} wrote the list through a symlink into another repo")
    return bad


def main() -> int:
    with tempfile.TemporaryDirectory() as d:
        bad = land_cases(Path(d)) + history_cases(Path(d)) + sandbox_cases(HERE.parent) + merge_cases()
    r = run(sys.executable, str(HERE / "ratchet-check.py"), "--self-test")
    if r.returncode != 0:
        bad.append(f"ratchet-check self-test failed: {(r.stdout + r.stderr).strip()[-300:]}")
    r = run(sys.executable, str(HERE / "disk_test.py"))
    if r.returncode != 0:
        bad.append(f"disk guard self-test failed: {(r.stdout + r.stderr).strip()[-600:]}")
    r = run(sys.executable, str(HERE / "bulk-read_test.py"))
    if r.returncode != 0:
        bad.append(f"bulk-read self-test failed: {(r.stdout + r.stderr).strip()[-600:]}")
    old = Path("/usr/bin/python3")
    if old.exists() and run(str(old), "-c", "import tomllib").returncode != 0:
        with tempfile.TemporaryDirectory() as d:
            probe = Path(d) / "probe.py"
            probe.write_text("import sys; sys.path.insert(0, sys.argv[1]); import config; "
                             "print(config.tomllib.__name__)\n")
            # A parent that re-exec'd once leaves no flag behind to block this child.
            r = run(str(old), str(probe), str(HERE.parent / "lint" / "lib"))
            parent = Path(d) / "parent.py"
            parent.write_text("import subprocess, sys; sys.path.insert(0, sys.argv[1]); import config; "
                              "sys.exit(subprocess.run([sys.argv[2], sys.argv[3], sys.argv[1]]).returncode)\n")
            inherited = run(str(old), str(parent), str(HERE.parent / "lint" / "lib"), str(old), str(probe))
        if r.returncode != 0 or "tomllib" not in r.stdout:
            bad.append(f"lint config on {old} (no tomllib) did not re-exec under python 3.11+: "
                       f"{(r.stdout + r.stderr).strip()[-200:]}")
        if inherited.returncode != 0 or "tomllib" not in inherited.stdout:
            bad.append(f"a {old} child of a re-exec'd lint did not re-exec: "
                       f"{(inherited.stdout + inherited.stderr).strip()[-200:]}")
    for b in bad:
        print(f"FAIL {b}")
    print("ok" if not bad else f"{len(bad)} broken")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
