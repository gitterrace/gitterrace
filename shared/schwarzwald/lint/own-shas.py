#!/usr/bin/env python3
"""CONV-NO-OWN-SHAS: nothing in the tree or a commit message names a commit of this repository.

A sha of the repository's own history goes stale with every history rewrite and points a
reader at a commit they may not have. Say what the commit did instead. Any 7 to 40 hex run that
is a prefix of an own commit fails. Own commits are `git rev-list --all`, plus every commit of
the pre-rewrite bundles named in lint.toml `[history] bundles`: a file outside the repository
(`~` expands) listing one bundle path per line, `#` for comments. A rewritten history's old
shas are just as dead. Upstream shas (a pinned corpus) are in neither, so they pass, and the
ones that happen to collide go in `[history] keep`.
Golden digests (only their `#` notes are read), `[scope] exclude` and lock files are not scanned.

  own-shas.py [--repo DIR]                  the tree
  own-shas.py [--repo DIR] --messages BASE  subject and body of every commit in BASE..HEAD
  own-shas.py [--repo DIR] --message FILE   one commit message (the commit-msg hook)
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-NO-OWN-SHAS"
SHA = re.compile(r"\b[0-9a-f]{7,40}\b")


def git(root: Path, *args: str) -> str:
    # A hook runs us with GIT_DIR and GIT_INDEX_FILE set: inherited, they point `git init` in a
    # scratch dir at the real repository and re-initialize it as bare.
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=True,
                          env=env).stdout


def bundle_commits(listing: str) -> list[str]:
    """Every commit of every bundle the listing file names."""
    if not listing:
        return []
    path = Path(listing).expanduser()
    if not path.exists():
        print(f"note: [history] bundles {listing} absent, reachable commits only", file=sys.stderr)
        return []
    out: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        with tempfile.TemporaryDirectory() as d:
            subprocess.run(["git", "clone", "-q", "--mirror", str(Path(line).expanduser()), d],
                           capture_output=True, check=True)
            out += git(Path(d), "rev-list", "--all").split()
    return out


class Own:
    def __init__(self, root: Path, history: dict) -> None:
        self.keep = set(history["keep"])
        self.by7: dict[str, list[str]] = {}
        for s in git(root, "rev-list", "--all").split() + bundle_commits(history["bundles"]):
            self.by7.setdefault(s[:7], []).append(s)

    def names(self, text: str) -> list[str]:
        return [m.group(0) for m in SHA.finditer(text)
                if m.group(0) not in self.keep
                and any(s.startswith(m.group(0)) for s in self.by7.get(m.group(0)[:7], []))]


def notes(rel: str, text: str, golden: list[str]) -> str:
    if config.matches(rel, golden) and not rel.endswith(".md"):
        return "\n".join(x for x in text.splitlines() if x.startswith("#"))
    return text


def scan_tree(root: Path, own: Own, scope: dict) -> tuple[int, list[str]]:
    names = [r for r in config.tracked(root, scope["exclude"]) if not r.endswith(".lock")]
    hits = []
    for rel in names:
        try:
            text = notes(rel, (root / rel).read_text(encoding="utf-8"), scope["golden"])
        except (OSError, UnicodeDecodeError):
            continue
        hits += [f"{rel}: names commit {t[:10]} of this repository" for t in own.names(text)]
    return len(names), hits


def commit(root: Path, msg: str) -> str:
    git(root, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "--allow-empty", "-m", msg)
    return git(root, "rev-parse", "HEAD").strip()


def self_test() -> bool:
    """A planted own sha fails, an unknown one passes, a bundle's old sha fails, a kept one passes."""
    history = dict(config.DEFAULTS["history"])
    scope = config.DEFAULTS["scope"]
    with tempfile.TemporaryDirectory() as d:
        root, old = Path(d) / "r", Path(d) / "old"
        for r in (root, old):
            r.mkdir()
            git(r, "init", "-q")
        (root / "a.txt").write_text("x\n")
        git(root, "add", "a.txt")
        head = commit(root, "x")
        if scan_tree(root, Own(root, history), scope)[1]:
            print(f"FAIL [{RULE}] self-test: a clean tree must pass", file=sys.stderr)
            return False
        gone = commit(old, "before the rewrite")
        git(old, "bundle", "create", "-q", str(Path(d) / "old.bundle"), "--all")
        (Path(d) / "bundles").write_text(f"# pre-rewrite\n{Path(d) / 'old.bundle'}\n")
        history["bundles"] = str(Path(d) / "bundles")
        (root / "a.txt").write_text(f"measured at {head[:10]}, and 0123abcd99, once {gone[:8]}\n")
        (root / "g/golden").mkdir(parents=True)
        (root / "g/golden/e.txt").write_text(f"{head[:12]} x\n# blessed at {head[:9]}\n")
        if len(scan_tree(root, Own(root, history), scope)[1]) != 3:
            print(f"FAIL [{RULE}] self-test: an own sha, a bundle's sha and a golden note must fail", file=sys.stderr)
            return False
        history["keep"] = [gone[:8]]
        if len(scan_tree(root, Own(root, history), scope)[1]) != 2:
            print(f"FAIL [{RULE}] self-test: a kept sha must pass", file=sys.stderr)
            return False
    return True


def report(hits: list[str], what: str) -> int:
    if hits:
        print(f"FAIL [{RULE}] {len(hits)} commit(s) of this repository named in {what}")
        for h in hits[:60]:
            print(f"  {h}")
        print("  Say what the commit did, not its sha.")
        return 1
    print(f"PASS [{RULE}] no commit of this repository named in {what}")
    return 0


def main(argv: list[str]) -> int:
    root, args = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    own = Own(root, cfg["history"])
    if args[:1] == ["--messages"]:
        hits = [f"message names commit {t[:10]}" for m in config.messages(root, args[1]) for t in own.names(m)]
        return report(hits, f"the messages of {args[1]}..HEAD")
    if args[:1] == ["--message"]:
        msg = "\n".join(config.message_lines(Path(args[1]).read_text(encoding="utf-8", errors="replace")))
        return report([f"message names commit {t[:10]}" for t in own.names(msg)], "the commit message")
    n, hits = scan_tree(root, own, cfg["scope"])
    return report(hits, f"the tree ({n} files)")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
