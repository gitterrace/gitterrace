#!/usr/bin/env python3
"""Rewrite the tagline in docs/brand/readme-header-{light,dark}.svg.

The header's text is outlined (no font is needed to view it), so a wording change has to be
rendered again. This draws TAGLINE with the vendored Bricolage Grotesque at the header's
size and position and swaps it into both files, keeping each file's colour. The path carries
data-text (the words it spells) and data-sha (sha256 of its path data).

    scripts/release/site/theme-tagline-gen.py --check
        stdlib only: data-text equals TAGLINE, data-sha matches the path
    uv run --with fonttools --with brotli python \
        scripts/release/site/theme-tagline-gen.py    rewrite

`make theme` runs the check, so the outlined words cannot drift from TAGLINE, and the
names check reads TAGLINE as prose.
"""

from __future__ import annotations

import hashlib
import html
import re
import sys
from pathlib import Path

TAGLINE = "A local, read-only git browser: code, history and insights, offline."
ROOT = Path(__file__).resolve().parents[3]
FONT = (
    ROOT
    / "schwarzwald/kit/fonts/bricolage-grotesque/bricolage-grotesque-latin-wght-normal.woff2"
)
X, BASELINE, SIZE, WEIGHT = 174.0, 150.0, 22.0, 400


def outline(text: str) -> str:
    # fontTools is only present under the `uv run --with fonttools` rewrite, never in lint.
    from fontTools.pens.svgPathPen import SVGPathPen  # pyright: ignore[reportMissingImports]
    from fontTools.pens.transformPen import TransformPen  # pyright: ignore[reportMissingImports]
    from fontTools.ttLib import TTFont  # pyright: ignore[reportMissingImports]
    from fontTools.varLib.instancer import instantiateVariableFont  # pyright: ignore[reportMissingImports]

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


PATH = re.compile(
    r'<path fill="(#[0-9a-f]{6})"(?: data-text="([^"]*)" data-sha="([0-9a-f]{64})")? d="([^"]*)"/>'
)


def sha(d: str) -> str:
    return hashlib.sha256(d.encode()).hexdigest()


def main() -> int:
    check = "--check" in sys.argv
    d = None if check else outline(TAGLINE)
    stale: list[str] = []
    for theme in ("light", "dark"):
        path = ROOT / f"docs/brand/readme-header-{theme}.svg"
        svg = path.read_text()
        last = list(PATH.finditer(svg))[-1]
        rel = str(path.relative_to(ROOT))
        if check:
            text, digest, cur = last.group(2), last.group(3), last.group(4)
            if text is None or html.unescape(text) != TAGLINE or digest != sha(cur):
                stale.append(rel)
            continue
        assert d is not None  # outlined above whenever not --check
        new = (
            svg[: last.start()]
            + f'<path fill="{last.group(1)}" data-text="{html.escape(TAGLINE)}" data-sha="{sha(d)}" d="{d}"/>'
            + svg[last.end() :]
        )
        if new != svg:
            path.write_text(new)
            print(f"wrote {rel}")
    for s in stale:
        print(f"{s}: tagline does not match TAGLINE; run this script without --check")
    return 1 if stale else 0


if __name__ == "__main__":
    raise SystemExit(main())
