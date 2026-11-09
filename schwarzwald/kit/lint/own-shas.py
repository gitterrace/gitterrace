#!/usr/bin/env python3
"""CONV-NO-OWN-SHAS: nothing in the tree or a commit message names a commit of this repository.

A sha of the repository's own history points a reader at a commit they may not have. Say what
the commit did instead. Any 7 to 40 hex run that is a prefix of an own commit fails. Own commits
are `git rev-list --all`. Upstream shas (a pinned corpus) are not in it, so they pass.
Golden digests (only their `#` notes are read), `[scope] exclude` and lock files are not scanned.

  own-shas.py [--repo DIR]                  the tree
  own-shas.py [--repo DIR] --messages BASE  subject and body of every commit in BASE..HEAD
  own-shas.py [--repo DIR] --message FILE   one commit message (the commit-msg hook)
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-NO-OWN-SHAS"
SHA = re.compile(r"\b[0-9a-f]{7,40}\b")


class Own:
    def __init__(self, root: Path) -> None:
        self.by7: dict[str, list[str]] = {}
        for s in config.git(root, "rev-list", "--all").split():
            self.by7.setdefault(s[:7], []).append(s)

    def names(self, text: str) -> list[str]:
        return [m.group(0) for m in SHA.finditer(text)
                if any(s.startswith(m.group(0)) for s in self.by7.get(m.group(0)[:7], []))]


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
    if (why := config.shallow(root)):
        print(f"FAIL [{RULE}] {why}", file=sys.stderr)
        return 1
    own = Own(root)
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
