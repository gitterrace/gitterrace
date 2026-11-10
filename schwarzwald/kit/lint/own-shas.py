#!/usr/bin/env python3
"""CONV-NO-OWN-SHAS: nothing in the tree or a commit message names a commit of this repository.

A sha of the repository's own history points a reader at a commit they may not have. Say what
the commit did instead. Any 7 to 40 hex run that is a prefix of an own commit fails. Own commits
are `git rev-list --all`. Upstream shas (a pinned corpus) are not in it, so they pass.
Golden digests (only their `#` notes are read), `[scope] exclude` and lock files are not scanned.
`[own_shas] allow` lists files that may name one recorded build commit: the sha a `record` file
(a glob of JSON files) holds under `key` and no other. A path ending in `#NAME` allows it only
inside that file's generated block (`BEGIN GENERATED: NAME` to `END GENERATED: NAME`). An
allowed sha goes stale if the product's history is ever rewritten: the recorded commit then
names nothing, so re-run the benchmark and record the new commit.

  own-shas.py [--repo DIR]                  the tree
  own-shas.py [--repo DIR] --messages BASE  subject and body of every commit in BASE..HEAD
  own-shas.py [--repo DIR] --message FILE   one commit message (the commit-msg hook)
"""

from __future__ import annotations

import json
import re
import sys
from collections import Counter
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


def recorded(root: Path, own_cfg: dict) -> set[str]:
    """The build commits the `record` files hold under `key`, anywhere in the JSON."""
    out: set[str] = set()

    def walk(v: object) -> None:
        if isinstance(v, dict):
            for k, x in v.items():
                if k == own_cfg["key"] and isinstance(x, str) and SHA.fullmatch(x):
                    out.add(x)
                walk(x)
        elif isinstance(v, list):
            for x in v:
                walk(x)

    for rel in config.tracked(root, []):
        if own_cfg["record"] and config.matches(rel, [own_cfg["record"]]) and rel.endswith(".json"):
            try:
                walk(json.loads((root / rel).read_text(encoding="utf-8")))
            except (OSError, ValueError):
                continue
    return out


def allowed_text(rel: str, text: str, allow: list[str]) -> tuple[bool, str]:
    """Whether `rel` may name a recorded commit, and the part of `text` where it may."""
    for a in allow:
        path, _, block = a.partition("#")
        if rel != path and not config.matches(rel, [path]):
            continue
        if not block:
            return True, text
        m = re.search(rf"<!-- BEGIN GENERATED: {re.escape(block)} -->(.*?)<!-- END GENERATED: {re.escape(block)} -->",
                      text, re.S)
        return True, m.group(1) if m else ""
    return False, ""


def scan_tree(root: Path, own: Own, scope: dict, own_cfg: dict | None = None) -> tuple[int, list[str]]:
    own_cfg = own_cfg or {"allow": [], "record": "", "key": "commit"}
    keep = recorded(root, own_cfg) if own_cfg["allow"] else set()
    names = [r for r in config.tracked(root, scope["exclude"]) if not r.endswith(".lock")]
    hits = []
    for rel in names:
        try:
            text = notes(rel, (root / rel).read_text(encoding="utf-8"), scope["golden"])
        except (OSError, UnicodeDecodeError):
            continue
        ok, zone = allowed_text(rel, text, own_cfg["allow"])
        # A recorded build commit passes where it is allowed, in that zone only.
        spare = Counter(t for t in own.names(zone) if ok and t in keep)
        left = []
        for t in own.names(text):
            if spare[t] > 0:
                spare[t] -= 1
            else:
                left.append(t)
        hits += [f"{rel}: names commit {t[:10]} of this repository" for t in left]
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
    n, hits = scan_tree(root, own, cfg["scope"], cfg["own_shas"])
    return report(hits, f"the tree ({n} files)")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
