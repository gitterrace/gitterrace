#!/usr/bin/env python3
"""Every third-party `uses:` in workflows and composite actions is pinned by a full commit sha
with a version comment. Local (./) actions are exempt.

  python3 scripts/actions-pin-check.py
"""

import pathlib
import re
import sys

USES = re.compile(r"^\s*(?:-\s*)?uses:\s*(\S+)(.*)$")
SHA = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
COMMENT = re.compile(r"^\s*#\s*\S+")


def problems(path, text):
    out = []
    for n, line in enumerate(text.splitlines(), 1):
        m = USES.match(line)
        if not m or m.group(1).startswith("./"):
            continue
        ref, rest = m.groups()
        if not SHA.match(ref):
            out.append(f"{path}:{n}: {ref} is not pinned by a full commit sha")
        elif not COMMENT.match(rest):
            out.append(f"{path}:{n}: {ref} lacks a version comment")
    return out


def main():
    root = pathlib.Path(__file__).resolve().parent.parent / ".github"
    files = sorted([*root.rglob("*.yml"), *root.rglob("*.yaml")])
    bad = [p for f in files for p in problems(f.relative_to(root.parent), f.read_text())]
    print("\n".join(bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
