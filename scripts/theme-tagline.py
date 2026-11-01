#!/usr/bin/env python3
"""Rewrite the tagline in docs/brand/readme-header-{light,dark}.svg.

The header's text is outlined (no font is needed to view it), so a wording change has to be
rendered again. This draws TAGLINE with the vendored Bricolage Grotesque at the header's
size and position and swaps it into both files, keeping each file's colour.

    uv run --with fonttools --with brotli python scripts/theme-tagline.py [--check]

`make theme` runs the check, so the outlined words cannot drift from TAGLINE, and the
names check reads TAGLINE as prose.
"""
import re
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

TAGLINE = "Your repository's history, step by step. Offline, one binary."
ROOT = Path(__file__).resolve().parent.parent
FONT = ROOT / "theme/schwarzwald/fonts/bricolage-grotesque/bricolage-grotesque-latin-wght-normal.woff2"
X, BASELINE, SIZE, WEIGHT = 174.0, 150.0, 22.0, 400


def outline(text: str) -> str:
    font = instantiateVariableFont(TTFont(FONT), {"wght": WEIGHT})
    scale = SIZE / font["head"].unitsPerEm
    cmap, glyphs, hmtx = font.getBestCmap(), font.getGlyphSet(), font["hmtx"]
    pen = SVGPathPen(glyphs, ntos=lambda v: f"{v:.2f}".rstrip("0").rstrip("."))
    x = X
    for ch in text:
        name = cmap[ord(ch)]
        glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, x, BASELINE)))
        x += hmtx[name][0] * scale
    return pen.getCommands()


def main() -> int:
    import sys

    check = "--check" in sys.argv
    d = outline(TAGLINE)
    stale = []
    for theme in ("light", "dark"):
        path = ROOT / f"docs/brand/readme-header-{theme}.svg"
        svg = path.read_text()
        last = list(re.finditer(r'<path fill="(#[0-9a-f]{6})" d="[^"]*"/>', svg))[-1]
        new = svg[: last.start()] + f'<path fill="{last.group(1)}" d="{d}"/>' + svg[last.end() :]
        if new == svg:
            continue
        if check:
            stale.append(str(path.relative_to(ROOT)))
        else:
            path.write_text(new)
            print(f"wrote {path.relative_to(ROOT)}")
    for s in stale:
        print(f"{s}: tagline does not match TAGLINE; run this script without --check")
    return 1 if stale else 0


raise SystemExit(main())
