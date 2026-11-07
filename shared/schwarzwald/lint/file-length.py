#!/usr/bin/env python3
"""CONV-FILE-LENGTH: no source file is longer than the cap, and no source line is wider.

lint.toml `[lines]`: `max_lines` (600), `max_width` (0 = off, else a cap on COMMENT lines, which
a formatter never wraps), `globs` (the files counted) and `exclude` (vendored or generated
paths). A file over the cap is split by responsibility, never trimmed for the count.

  file-length.py [--repo DIR]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-FILE-LENGTH"
COMMENT = re.compile(r"^\s*(//|#|\*|/\*)")


def problems(rel: str, text: str, cap: int, width: int) -> list[str]:
    lines = text.splitlines()
    out = [f"{rel}: {len(lines)} lines, the cap is {cap}"] if len(lines) > cap else []
    if width:
        out += [f"{rel}:{n}: comment is {len(x)} wide, the cap is {width}"
                for n, x in enumerate(lines, 1) if len(x) > width and COMMENT.match(x)]
    return out


def self_test() -> bool:
    ok = (problems("a", "x\n" * 5, 4, 0) and not problems("a", "x\n" * 4, 4, 0)
          and problems("a", "// " + "x" * 20 + "\n", 9, 10) and not problems("a", "code " + "x" * 20 + "\n", 9, 10))
    if not ok:
        print(f"FAIL [{RULE}] self-test: an over-cap file must fail and an in-cap one pass", file=sys.stderr)
    return bool(ok)


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    s = cfg["lines"]
    bad, seen = [], 0
    for rel in config.tracked(root, cfg["scope"]["exclude"] + s["exclude"]):
        if not config.matches(rel, s["globs"]):
            continue
        try:
            text = (root / rel).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        seen += 1
        bad += problems(rel, text, s["max_lines"], s["max_width"])
    if bad:
        print(f"FAIL [{RULE}] {len(bad)} over the cap")
        for b in bad[:60]:
            print(f"  {b}")
        return 1
    print(f"PASS [{RULE}] {seen} files within {s['max_lines']} lines")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
