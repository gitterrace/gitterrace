#!/usr/bin/env python3
"""Fail when a stylesheet sizes or places a box in viewport-width units.

`vw` (and `vi`, `vmin`, `vmax` and their s/l/d forms) measure the window with its scrollbar.
Under a classic scrollbar, the kind Linux and Windows draw, the page column is about 15px
narrower than 100vw, so a popover of `min(320px,92vw)` ends past the column and the page scrolls
sideways. macOS overlay scrollbars hide it, which is how such a width ships. A box is sized by
the box it hangs from instead: a percentage of its containing block.

Refused: a viewport-width unit in a property that sizes or places a box (width, inline-size,
their min- and max- forms, flex and flex-basis, left, right, inset and its inline forms, margin
and padding), and in a custom property, which could carry it to one. Font sizes may use them: a
fluid type size does not overflow. A custom property named `--text-*` is such a size.

  python3 scripts/lint/css-vw-check.py              check the tree
  python3 scripts/lint/css-vw-check.py --self-test  prove a planted rule goes red
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SHEETS = ("crates/gitterrace-web/static/*.css", "xtask/templates/site.css")
BOX = re.compile(
    r"^((min-|max-)?(width|inline-size)|flex(-basis)?|left|right|inset(-inline(-start|-end)?)?"
    r"|(margin|padding)(-left|-right|-inline(-start|-end)?)?|translate|transform|--(?!text-).*)$"
)
UNIT = re.compile(r"(?<![\w-])[\d.]+(?:[sld]?v(?:w|i|min|max))\b")


def problems(css: str, name: str) -> list[str]:
    """Each declaration in `css` that sizes or places a box in viewport-width units."""
    css = re.sub(r"/\*.*?\*/", lambda m: "\n" * m.group(0).count("\n"), css, flags=re.S)
    bad = []
    for m in re.finditer(r"(?<![\w-])(--[\w-]+|[a-z-]+)\s*:\s*([^;{}]*)", css):
        prop, value = m.group(1), m.group(2)
        if BOX.match(prop) and UNIT.search(value):
            line = css.count("\n", 0, m.start()) + 1
            bad.append(f"{name}:{line}: {prop}:{value.strip()} measures the window, scrollbar")
    return bad


def check() -> list[str]:
    bad = []
    for pattern in SHEETS:
        for f in sorted(ROOT.glob(pattern)):
            bad += problems(f.read_text(), f.name)
    return bad


def self_test() -> int:
    red = [
        ".outline-pop{position:absolute;right:0;width:min(320px,92vw)}",
        "@media (max-width:600px){.ov.send{width:100vw;max-width:none}}",
        ".pop{max-width:calc(100vw - 2*var(--page-pad))}",
        ".tip{left:50vw}",
        ".sheet{inline-size:100dvw}",
        ":root{--pop-w:92vw}",
        ".wide{margin-left:calc(50% - 50vw)}",
    ]
    green = [
        ".outline-pop{position:absolute;right:0;width:min(320px,100%)}",
        "h1{font-size:clamp(2rem,3.2vw,2.5rem)}",
        ":root{--text-display:clamp(2.125rem,1.2rem + 3.6vw,4.625rem)}",
        ".ov{max-height:70vh;padding:12vh 0 0}",
        "/* vw would count the scrollbar: width:92vw */.a{width:92%}",
    ]
    fails = [c for c in red if not problems(c, "red")]
    fails += [c for c in green if problems(c, "green")]
    for c in fails:
        print(f"css-vw self-test: wrong verdict for {c}")
    return 1 if fails else 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    bad = check()
    for b in bad:
        print(b)
    if bad:
        print("Size the box by its containing block (a percentage), not by the window.")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
