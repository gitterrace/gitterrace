#!/usr/bin/env python3
"""CONV-COMMIT-DATES: every commit's author date equals its committer date, and no commit is older
than its parent.

A commit fast-forwarded by hand instead of through the landing tool keeps the date it was first
written, so the log shows work out of order and a date no longer says when a change reached main.
The landing tool stamps both dates to the landing time, one second apart, so its commits pass.

  commit-dates.py [--repo DIR] [--base SHA]   the commits in BASE..HEAD, or all of HEAD without it

The landing tool's message gate passes `--base {base}`. Without a base it reads the whole history,
which only suits a repository whose history already holds to the rule.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-COMMIT-DATES"


def problems(root: Path, base: str | None) -> list[str]:
    span = f"{base}..HEAD" if base else "HEAD"
    out = []
    lines = config.git(root, "log", "--reverse", "--format=%H %at %ct %P", span).splitlines()
    dates = {}
    for line in lines:
        sha, at, ct, *parents = line.split()
        dates[sha] = int(ct)
        if at != ct:
            out.append(f"{sha[:10]}: author date {at} is not its committer date {ct}")
        for p in parents[:1]:
            pt = dates.get(p)
            if pt is None:
                pt = int(config.git(root, "log", "-1", "--format=%ct", p).strip())
            if int(ct) < pt:
                out.append(f"{sha[:10]}: committed {pt - int(ct)} s before its parent {p[:10]}")
    return out


def main(argv: list[str]) -> int:
    root, args = config.take_repo(argv)
    base = None
    if args[:1] == ["--base"] and len(args) > 1:
        base = args[1]
    if (why := config.shallow(root)):
        print(f"FAIL [{RULE}] {why}", file=sys.stderr)
        return 1
    bad = problems(root, base)
    for b in bad:
        print(f"FAIL [{RULE}] {b}", file=sys.stderr)
    if bad:
        print("  Land through the landing tool, which stamps both dates to the landing time.",
              file=sys.stderr)
        return 1
    print(f"PASS [{RULE}] author and committer dates agree and only move forward")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
