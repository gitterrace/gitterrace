#!/usr/bin/env python3
"""CONV-MAKE-TARGETS: every `make <target>` named in tracked text is a target a Makefile defines.

A renamed target must not leave a stale instruction. Backticked `make x` counts anywhere. A bare
`make x` counts where it is a command: shell scripts, fenced blocks in markdown and non-comment
workflow lines. Targets are read from the files in lint.toml `[scope] make`. Files in
`[scope] exclude` are not read.

  make-targets.py [--repo DIR]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-MAKE-TARGETS"
TEXT = (".md", ".py", ".sh", ".txt", ".yml", ".yaml", ".mjs", ".rs", ".toml")
TARGET = re.compile(r"^([A-Za-z0-9_.-]+(?:\s+[A-Za-z0-9_.-]+)*)\s*:(?!=)", re.M)
BARE = re.compile(r"(?:^|[\s`;&|(])make ([a-z][a-z0-9-]*)")
TICKED = re.compile(r"`make ([a-z][a-z0-9-]*)")


def targets(text: str) -> set[str]:
    return {t for m in TARGET.finditer(text) for t in m.group(1).split()}


def problems(rel: str, text: str, known: set[str]) -> list[str]:
    out, fenced = [], False
    for n, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("```"):
            fenced = not fenced
        yml = rel.endswith((".yml", ".yaml")) and not line.lstrip().startswith("#")
        pat = BARE if (fenced or yml or rel.endswith(".sh")) else TICKED
        out += [f"{rel}:{n}: make {m.group(1)} is not a Makefile target"
                for m in pat.finditer(line) if m.group(1) not in known]
    return out


def self_test() -> bool:
    known = targets("build:\n\techo\n.PHONY: build\nlint test: build\n")
    ok = (known >= {"build", "lint", "test"} and problems("a.md", "run `make nope`\n", known)
          and not problems("a.md", "run `make lint`, then make nothing\n", known)
          and problems("a.md", "```\nmake nope\n```\n", known))
    if not ok:
        print(f"FAIL [{RULE}] self-test: a stale target must fail and a real one pass", file=sys.stderr)
    return bool(ok)


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    names = config.tracked(root, cfg["scope"]["exclude"])
    makes = [n for n in names if config.matches(n, cfg["scope"]["make"])]
    if not makes:
        print(f"PASS [{RULE}] no Makefile in [scope] make, nothing to check")
        return 0
    known: set[str] = set()
    for m in makes:
        known |= targets((root / m).read_text(encoding="utf-8", errors="replace"))
    bad, seen = [], 0
    for rel in names:
        if not rel.endswith(TEXT) or rel.endswith("make-targets.py") or "red-cases" in Path(rel).parts:
            continue
        try:
            text = (root / rel).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        seen += 1
        bad += problems(rel, text, known)
    if bad:
        print(f"FAIL [{RULE}] {len(bad)} stale make target(s)")
        for b in bad[:60]:
            print(f"  {b}")
        return 1
    print(f"PASS [{RULE}] every named make target exists ({seen} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
