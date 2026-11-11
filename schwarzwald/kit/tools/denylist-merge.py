#!/usr/bin/env python3
"""Refresh the personal denylist by MERGE: the union, never a shrink.

  denylist-merge.py [--repo DIR] [--history FILE] < ids

Reads board ids (one per line, any text: six-hex tokens are taken) from stdin, and unions them
with the current list (lint.toml [denylist] path, outside every repository; every line kept as it is, regexes too) and the history
file (`--history`, default lint.toml [denylist] history_ids). Writes the file sorted by the
bare ids first; an existing entry is never removed. The product's refresh command pipes its
tracker's every code, closed and deleted rows included, into this.
"""

from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lint" / "lib"))
import config  # noqa: E402

HEX6 = re.compile(r"\b[0-9a-f]{6}\b")


def source_ids(text: str) -> set[str]:
    """The ids in one source. A tracker's JSON export gives only its `code` fields, so a hex
    word inside a title or a note never joins the list; plain text gives every hex token."""
    try:
        data = json.loads(text)
    except ValueError:
        return set(HEX6.findall(text))
    found: set[str] = set()
    stack = [data]
    while stack:
        item = stack.pop()
        if isinstance(item, dict):
            code = item.get("code")
            if isinstance(code, str) and HEX6.fullmatch(code):
                found.add(code)
            stack.extend(item.values())
        elif isinstance(item, list):
            stack.extend(item)
    return found


def merge(current: str, *sources: str) -> str:
    keep = [x.strip() for x in current.splitlines() if x.strip()]
    ids = {x for x in keep if HEX6.fullmatch(x)} | {t for s in sources for t in source_ids(s)}
    other = [x for x in keep if not HEX6.fullmatch(x)]
    return "\n".join(sorted(ids) + other) + "\n"


def in_work_tree(p: Path) -> Path | None:
    """The first folder at or above p (symlinks resolved) that holds a .git, else None."""
    q = Path(os.path.realpath(p))
    for d in (q, *q.parents):
        if (d / ".git").exists():
            return d
    return None


def write_private(path: Path, text: str) -> None:
    """Write with the folder at 0700 and the file at 0600, never wider, even on the first write
    or when the folder already existed. Refuses (exit) a path inside any git work tree, symlinks
    resolved, and a symlink at the path. The text goes to a temp file in the same folder that is
    then renamed, so a crash never leaves a truncated list."""
    path = Path(os.path.abspath(path))
    if path.is_symlink():
        sys.exit(f"refused: the denylist path {path} is a symlink")
    if repo := in_work_tree(path.parent):
        sys.exit(f"refused: the denylist path {path} lies inside the git work tree {repo}")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.parent.chmod(0o700)
    tmp = path.parent / f".{path.name}.{os.getpid()}.tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    finally:
        tmp.unlink(missing_ok=True)
    path.chmod(0o600)


def main(argv: list[str]) -> int:
    root, args = config.take_repo(argv)
    cfg = config.load(root)
    hist = args[args.index("--history") + 1] if "--history" in args else cfg["denylist"]["history_ids"]
    path = config.denylist_path(root, cfg)
    cur = path.read_text(encoding="utf-8") if path.exists() else ""
    extra = [sys.stdin.read()]
    if hist:
        extra.append(Path(hist).expanduser().read_text(encoding="utf-8", errors="replace"))
    out = merge(cur, *extra)
    write_private(path, out)
    print(f"{path}: {len(HEX6.findall(out))} ids")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
