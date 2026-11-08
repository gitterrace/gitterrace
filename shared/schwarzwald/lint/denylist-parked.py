#!/usr/bin/env python3
"""CONV-DENYLIST-PARKED: the private denylist never sits inside a repository.

Real names and ids live in one file outside every repository (lint.toml [denylist] path). A file
named `.personal-denylist` anywhere under the product root fails, tracked or ignored, and so does
any file git does not ignore that holds half the denylist's entries or more (a renamed or edited
copy). A configured path inside the root fails too.

  denylist-parked.py [--repo DIR]
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-DENYLIST-PARKED"
OLD_NAME = ".personal-denylist"
SKIP = {".git", "target", "node_modules", ".build", "DerivedData"}


def entries(data: bytes) -> set[bytes]:
    return {x.strip() for x in data.splitlines() if x.strip() and not x.strip().startswith(b"#")}


def parked(root: Path, list_path: Path) -> list[str]:
    """Files under root that are the denylist: by name, or holding half its entries or more.

    The name is looked for everywhere but `.git` and build output. The content is compared in
    every file git does not ignore, vendor folders included, as a set of lines, so an added
    newline or a reordered copy still counts."""
    want = entries(list_path.read_bytes()) if list_path.is_file() else set()
    need = max(1, (len(want) + 1) // 2)
    found: list[str] = []
    for here, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in SKIP]
        found += [f"{Path(here, n).relative_to(root)}: the private denylist is parked in the repository"
                  for n in files if n == OLD_NAME]
    if not want:
        return found
    for rel in config.tracked(root, []):
        p = root / rel
        if p.name == OLD_NAME or p.is_symlink() or not p.is_file() or p.stat().st_size > 16 << 20:
            continue
        if len(want & entries(p.read_bytes())) >= need:
            found.append(f"{rel}: holds {len(want & entries(p.read_bytes()))} of the private denylist's {len(want)} entries")
    return found


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    try:
        list_path = config.denylist_path(root, cfg)
    except SystemExit as e:
        print(f"FAIL [{RULE}] {e.code}")
        return 1
    bad = parked(root, list_path)
    if bad:
        print(f"FAIL [{RULE}] {len(bad)} private denylist cop{'y' if len(bad) == 1 else 'ies'} inside the repository")
        for b in bad:
            print(f"  {b}")
        print(f"  Move it out: it lives at {list_path} and nowhere else. Delete the copy once that file has every line.")
        return 1
    print(f"PASS [{RULE}] no private denylist inside the repository")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
