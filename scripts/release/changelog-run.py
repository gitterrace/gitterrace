#!/usr/bin/env python3
"""Empty the generated Unreleased section of CHANGELOG.md, run by `make changelog` before
git-cliff writes the section of the release about to be tagged in its place.

  python3 scripts/release/changelog-run.py
"""

from __future__ import annotations

import re
from pathlib import Path

LOG = Path(__file__).resolve().parents[2] / "CHANGELOG.md"
BLOCK = re.compile(
    r"(<!-- BEGIN GENERATED: unreleased -->\n).*?(<!-- END GENERATED: unreleased -->)",
    re.S,
)

if __name__ == "__main__":
    LOG.write_text(BLOCK.sub(r"\1\2", LOG.read_text()))
