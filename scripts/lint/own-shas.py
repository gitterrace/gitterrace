#!/usr/bin/env python3
"""CONV-NO-OWN-SHAS: nothing in the tree or a commit message names a commit of this repository.

A sha of gitterrace's own history goes stale with every history rewrite and points a reader
at a commit they may not have. Say what the commit did instead. Any 7 to 40 hex run that is a
prefix of a sha in `git rev-list --all` fails. Upstream corpus shas (the bench pins of git,
rust and linux) are not in that list, so they stay. Goldens, vendored files and lock files are
not scanned.

  own-shas.py                  the tree
  own-shas.py --messages BASE  subject and body of every commit in BASE..HEAD
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RULE = "CONV-NO-OWN-SHAS"
SHA = re.compile(r"\b[0-9a-f]{7,40}\b")
SKIP = ("target/", "static/vendor/", "theme/schwarzwald/", ".lock")


def notes(rel: str, text: str) -> str:
    """What of a file can name a commit: a golden digest file only its `#` note lines."""
    if "/golden/" in rel and not rel.endswith(".md"):
        return "\n".join(x for x in text.splitlines() if x.startswith("#"))
    return text


def git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args], capture_output=True, text=True, check=True
    ).stdout


class Own:
    def __init__(self, root: Path) -> None:
        self.by7: dict[str, list[str]] = {}
        for s in git(root, "rev-list", "--all").split():
            self.by7.setdefault(s[:7], []).append(s)

    def names(self, text: str) -> list[str]:
        return [
            m.group(0)
            for m in SHA.finditer(text)
            if any(s.startswith(m.group(0)) for s in self.by7.get(m.group(0)[:7], []))
        ]


def scan_tree(root: Path, own: Own) -> tuple[int, list[str]]:
    hits = []
    names = [
        r
        for r in git(
            root, "ls-files", "-z", "--cached", "--others", "--exclude-standard"
        ).split("\0")
        if r and not any(s in r for s in SKIP)
    ]
    for rel in names:
        try:
            text = notes(rel, (root / rel).read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError):
            continue
        hits += [
            f"{rel}: names commit {t[:10]} of this repository" for t in own.names(text)
        ]
    return len(names), hits


def self_test() -> bool:
    """A planted sha of the scanned repository fails, an unknown one passes."""
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        git(root, "init", "-q")
        (root / "a.txt").write_text("x\n")
        git(root, "add", "a.txt")
        git(
            root,
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-qm",
            "x",
        )
        own = Own(root)
        if scan_tree(root, own)[1]:
            print(f"FAIL [{RULE}] self-test: a clean tree must pass", file=sys.stderr)
            return False
        head = git(root, "rev-parse", "HEAD").strip()
        (root / "a.txt").write_text(f"measured at {head[:10]}, and 0123abcd99\n")
        (root / "g/golden").mkdir(parents=True)
        (root / "g/golden/e.txt").write_text(
            f"{head[:12]} x\n# blessed at {head[:9]}\n"
        )
        if len(scan_tree(root, own)[1]) != 2:
            print(
                f"FAIL [{RULE}] self-test: the own sha and the golden note must fail",
                file=sys.stderr,
            )
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


def main(args: list[str]) -> int:
    if not self_test():
        return 2
    own = Own(ROOT)
    if args[:1] == ["--messages"]:
        out = git(ROOT, "log", "-z", "--format=%s%n%n%b", f"{args[1]}..HEAD")
        hits = [
            f"message names commit {t[:10]}"
            for m in out.split("\0")
            for t in own.names(m)
        ]
        return report(hits, f"the messages of {args[1]}..HEAD")
    n, hits = scan_tree(ROOT, own)
    return report(hits, f"the tree ({n} files)")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
