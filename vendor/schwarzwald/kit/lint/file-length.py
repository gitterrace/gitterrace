#!/usr/bin/env python3
"""CONV-FILE-LENGTH: no source file is longer than the cap, and no source line is wider.

lint.toml `[lines]`: `max_lines` (600), `max_width` (0 = off, else a cap on COMMENT lines, which
a formatter never wraps), `globs` (the files counted) and `exclude` (vendored or generated
paths). A file over the cap is split by responsibility, never trimmed for the count.

`max_code_width` (0 = off) caps CODE lines, which a formatter leaves alone inside strings and
macro bodies. A file's current excess is its allowance, kept in
`schwarzwald/config/wide-allowed.json` as {"wide_allowed": {"path": count}}. The count may only
fall: more wide lines fail, and fewer fail until the allowance is lowered (a ratchet).

  file-length.py [--repo DIR]
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-FILE-LENGTH"
ALLOWED = "schwarzwald/config/wide-allowed.json"
COMMENT = re.compile(r"^\s*(//|#|\*|/\*)")


def problems(rel: str, text: str, cap: int, width: int) -> list[str]:
    lines = text.splitlines()
    out = [f"{rel}: {len(lines)} lines, the cap is {cap}"] if len(lines) > cap else []
    if width:
        out += [f"{rel}:{n}: comment is {len(x)} wide, the cap is {width}"
                for n, x in enumerate(lines, 1) if len(x) > width and COMMENT.match(x)]
    return out


def wide_code(text: str, width: int) -> int:
    """How many code (non-comment) lines are wider than width."""
    return sum(1 for x in text.splitlines() if len(x) > width and not COMMENT.match(x))


def code_problems(rel: str, text: str, width: int, allowed: int) -> list[str]:
    n = wide_code(text, width)
    if n > allowed:
        return [f"{rel}: {n} code lines wider than {width}, allowed {allowed} (wrap the new one)"]
    if n < allowed:
        return [f"{rel}: {n} code lines wider than {width}, allowed {allowed}: lower {ALLOWED}"]
    return []


def self_test() -> bool:
    ok = (problems("a", "x\n" * 5, 4, 0) and not problems("a", "x\n" * 4, 4, 0)
          and problems("a", "// " + "x" * 20 + "\n", 9, 10) and not problems("a", "code " + "x" * 20 + "\n", 9, 10))
    wide = "code " + "x" * 20 + "\n"
    ok = bool(ok and code_problems("a", wide, 9, 0) and not code_problems("a", wide, 9, 1)
              and code_problems("a", wide, 9, 2) and not code_problems("a", "// " + wide, 9, 0))
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
    width = s["max_code_width"]
    allowed = {}
    if width and (root / ALLOWED).exists():
        allowed = json.loads((root / ALLOWED).read_text(encoding="utf-8")).get("wide_allowed", {})
    for rel in config.tracked(root, cfg["scope"]["exclude"] + s["exclude"]):
        if not config.matches(rel, s["globs"]):
            continue
        try:
            text = (root / rel).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        seen += 1
        bad += problems(rel, text, s["max_lines"], s["max_width"])
        if width:
            bad += code_problems(rel, text, width, allowed.get(rel, 0))
    if bad:
        print(f"FAIL [{RULE}] {len(bad)} over the cap")
        for b in bad[:60]:
            print(f"  {b}")
        return 1
    print(f"PASS [{RULE}] {seen} files within {s['max_lines']} lines")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
