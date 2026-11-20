#!/usr/bin/env python3
"""Fail when inline unit tests break the layout CONTRIBUTING#testing states.

A source file holds at most one inline test module (`mod tests {`, `mod x_tests {`), it is the last inline item
in the file (`mod x;` file modules may follow), and test code carries no section-divider banners. Platform variants nest inside the
one module (`#[cfg(unix)] mod unix { ... }`). Run by the lint runner as the test-layout hook.

  python3 scripts/lint/test-layout-check.py [--self-test] [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GLOBS = ("crates/**/*.rs", "xtask/**/*.rs")
GATE = re.compile(r"^#\[cfg\((?:[^()\n]|\((?:[^()\n]|\([^()\n]*\))*\))*\btest\b[^\n]*\]\n(?:#\[[^\n]*\n)*(?:pub(?:\([a-z]+\))? )?mod \w*tests \{\n", re.M)
BANNER = re.compile(r"^\s*//\s*(?:[-=#*]{4,}|[-=#*]{2,}\s.*\s[-=#*]{2,})\s*$", re.M)


def problems_in(text: str) -> list[str]:
    out = []
    mods = list(GATE.finditer(text))
    if len(mods) > 1:
        out.append("more than one top-level test module, nest the extras inside `mod tests`")
    if mods:
        close = re.search(r"^\}\n?", text[mods[-1].end():], re.M)
        tail = text[mods[-1].end() + close.end():] if close else ""
        tail = re.sub(r"(?:#\[[^\n]*\n)*(?:pub(?:\([a-z]+\))? )?mod \w+;\n?", "", tail)
        if tail.strip():
            out.append("code after the test module, `mod tests` goes last")
        if BANNER.search(text[mods[0].start():]):
            out.append("section-divider banner in test code, use blank lines")
    return out


def problems(root: Path) -> list[str]:
    out = []
    for f in sorted({f for g in GLOBS for f in root.glob(g)}):
        if "target" in f.parts or f.parent.name == "tests":
            continue
        out += [f"{f.relative_to(root)}: {m}" for m in problems_in(f.read_text())]
    return out


def self_test() -> int:
    green = "fn a() {}\n\n#[cfg(test)]\nmod tests {\n    #[cfg(unix)]\n    mod unix {}\n}\n"
    reds = {
        "two modules": green + "\n#[cfg(all(test, unix))]\nmod unix_tests {\n}\n",
        "code after": green + "\nfn late() {}\n",
        "banner": green.replace("mod unix {}", "// ------\n    mod unix {}"),
    }
    bad = [] if not problems_in(green) else ["green case flagged"]
    bad += [n for n, t in reds.items() if not problems_in(t)]
    for b in bad:
        print(f"self-test failed: {b}")
    return 1 if bad else 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    root = Path(next((a for a in argv if not a.startswith("-")), ROOT))
    found = problems(root)
    for p in found:
        print(p)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
