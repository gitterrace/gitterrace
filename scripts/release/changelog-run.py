#!/usr/bin/env python3
"""Empty the generated Unreleased section of CHANGELOG.md, run by `make changelog` before
git-cliff writes the section of the release about to be tagged in its place. With --drop it
removes the section of an unpublished version instead, so make release-gate regenerates it
from the commits (and today's date) rather than keep a hand edit or a stale date.

  python3 scripts/release/changelog-run.py
  python3 scripts/release/changelog-run.py --drop x.y.z [--file PATH]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

LOG = Path(__file__).resolve().parents[2] / "CHANGELOG.md"
BLOCK = re.compile(
    r"(<!-- BEGIN GENERATED: unreleased -->\n).*?(<!-- END GENERATED: unreleased -->)",
    re.S,
)


def drop(text: str, version: str) -> str:
    """The text without any `## [version]` section, up to the next version heading."""
    pat = re.compile(rf"^## \[{re.escape(version)}\].*?(?=^## \[|\Z)", re.M | re.S)
    return pat.sub("", text)


if __name__ == "__main__":
    args = sys.argv[1:]
    log = Path(args[args.index("--file") + 1]) if "--file" in args else LOG
    if args[:1] == ["--drop"] and len(args) >= 2:
        log.write_text(drop(log.read_text(), args[1]))
    elif args and args[0] != "--file":
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    else:
        log.write_text(BLOCK.sub(r"\1\2", log.read_text()))
