#!/usr/bin/env python3
"""Docs guard (stdlib only), run by `make docs-check`: every `make <target>` named in tracked
text is a target the Makefile defines, so a renamed target cannot leave a stale instruction."""

import re
import subprocess
import sys

TEXT = (".md", ".py", ".sh", ".txt", ".yml", ".mjs", ".rs", ".toml")


def files():
    out = subprocess.run(
        ["git", "ls-files", "-z"], capture_output=True, check=True, text=True
    )
    return [
        f
        for f in out.stdout.split("\0")
        if f.endswith(TEXT) and not f.startswith("theme/")
    ]


def targets():
    text = open("Makefile", encoding="utf-8").read()
    return set(re.findall(r"^([A-Za-z0-9_.-]+):", text, re.M))


def main():
    known = targets()
    bad = []
    for path in files():
        if path.endswith("make-targets-check.py"):
            continue
        try:
            text = open(path, encoding="utf-8").read()
        except (OSError, UnicodeDecodeError):
            continue
        for n, line in enumerate(text.splitlines(), 1):
            for m in re.finditer(r"`make ([a-z][a-z0-9-]*)", line):
                if m.group(1) not in known:
                    bad.append(
                        f"{path}:{n}: make {m.group(1)} is not a Makefile target"
                    )
    print("\n".join(bad))
    return 1 if bad else 0


sys.exit(main())
