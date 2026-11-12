#!/usr/bin/env python3
"""CONV-ROOT-ENTRIES: the repository root holds only what the product allows, and no strays.

Two checks on the top level, both from lint.toml [root]:

- every tracked top-level entry is on `allow` (a file or folder name). A new root file fails
  until it moves under a folder or joins the list, next to the reason it must sit at the root.
  An empty `allow` turns this check off.
- nothing untracked and unignored sits at the root or anywhere inside the folders on
  `scratch_free` (default `.agents`): a session's note, log, draft or new skill left there reads
  as part of the project to every later session. Scratch goes to an ignored folder, a real file
  is committed.

  root-entries.py [--repo DIR]
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-ROOT-ENTRIES"


def not_allowed(names: list[str], allow: list[str]) -> list[str]:
    """Tracked top-level entries that are not on allow, once each."""
    tops = sorted({n.split("/", 1)[0] for n in names if n})
    return [t for t in tops if t not in allow]


def strays(listing: list[str], scratch_free: list[str]) -> list[str]:
    """Untracked, unignored paths at the root, or anywhere inside a scratch-free folder."""
    out = []
    for p in listing:
        if p.rstrip("/").count("/") == 0 or any(
            p.startswith(d.rstrip("/") + "/") for d in scratch_free
        ):
            out.append(p)
    return out


def self_test() -> bool:
    return not_allowed(["a/x", "a/y", "b", "README.md"], ["a", "README.md"]) == [
        "b"
    ] and strays(
        ["note.md", "docs/x.md", ".agents/draft.md", ".agents/skills/new/"], [".agents"]
    ) == ["note.md", ".agents/draft.md", ".agents/skills/new/"]


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        print(f"FAIL [{RULE}] self-test")
        return 2
    allow, free = cfg["root"]["allow"], cfg["root"]["scratch_free"]
    tracked = config.git(root, "ls-files", "-z").split("\0")
    others = config.git(
        root,
        "ls-files",
        "--others",
        "--exclude-standard",
        "--directory",
        "--no-empty-directory",
    ).splitlines()
    bad = (not_allowed(tracked, allow) if allow else []), strays(others, free)
    if bad[0] or bad[1]:
        print(
            f"FAIL [{RULE}] {len(bad[0]) + len(bad[1])} root entr{'y' if len(bad[0]) + len(bad[1]) == 1 else 'ies'}"
        )
        for b in bad[0]:
            print(f"  {b}: tracked at the root but not on lint.toml [root] allow")
        for b in bad[1]:
            print(f"  {b}: untracked and unignored at the root")
        print(
            "  Move a file under a folder, add a root file to [root] allow with its reason, commit a real"
        )
        print(
            "  new file, or ignore a tool's cache. Scratch goes to an ignored folder."
        )
        return 1
    print(f"PASS [{RULE}] the root holds only allowed and tracked entries")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
