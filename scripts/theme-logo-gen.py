#!/usr/bin/env python3
"""Write the plain logos in docs/brand from the README headers (stdlib only).

The header SVGs are the single source: the terrace mark, the outlined "git" and "terrace",
then the tagline. This keeps the first three and drops the tagline, and also writes the
mark alone, for each theme:

    logo-{light,dark}.svg   mark and wordmark, cropped
    mark-{light,dark}.svg   the mark alone, square

    python3 scripts/theme-logo-gen.py [--check]

`make theme` runs the check, so the logos cannot drift from the headers.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import quote as urlquote

ROOT = Path(__file__).resolve().parent.parent
BRAND = ROOT / "docs/brand"
TEMPLATES = ROOT / "crates/gitterrace-web/templates"
# The header's wordmark ends near x=545 and its tagline starts below y=128; the mark's group
# draws the 32 grid scaled by 3.2 from (40, 40), so the 28-unit stair box spans y 46 to 136,
# the wordmark's height from the top of "t" to the tail of "g".
LOGO_BOX = (24, 18, 540, 120)
MARK_AT = 'transform="translate(40 40) scale(3.2)"'
MARK_GROUP = re.compile(r"<g " + re.escape(MARK_AT) + r">.*?</g>", re.S)
PATHS = re.compile(r'<path fill="#[0-9a-f]{6}"(?: data-[a-z]+="[^"]*")* d="[^"]*"/>')
# The wordmark's two halves must stay visibly apart in every theme.
MIN_SPLIT = 3.0
# The step-in mark: one clockwise outline on the 32 grid. Stem x8-14, crossbar y8-14, tread
# y18-24, slab y24-30, every part STROKE thick. Every convex corner is rounded by CONVEX and
# every concave one by CONCAVE. The line y=SPLIT colours it: ink above, accent below.
STAIR = (
    (8, 2), (14, 2), (14, 8), (20, 8), (20, 14), (14, 14), (14, 18), (24, 18), (24, 24),
    (30, 24), (30, 30), (14, 30), (14, 24), (8, 24), (8, 14), (2, 14), (2, 8), (8, 8),
)  # fmt: skip
STROKE, SPLIT, CONVEX, CONCAVE = 6, 24, 1.5, 1.0
INKS = {"light": ("#21272e", "#5b5e8d"), "dark": ("#e3ecf6", "#9ca1d4")}
# The in-app header draws the same outline with the page's own colours.
APP_LOGO = re.compile(r'<svg class="logo" [^>]*>.*?</svg>', re.S)

Point = tuple[float, float]
# One outline segment: start, end and the arc's (radius, sweep), or None for a line.
Segment = tuple[Point, Point, "tuple[float, int] | None"]


def num(v: float) -> str:
    """A coordinate with at most three decimals and no trailing zeros, so output is stable."""
    return f"{round(v, 3):.3f}".rstrip("0").rstrip(".").replace("-0", "0") or "0"


def segments(scale: float, round_: bool) -> list[Segment]:
    """STAIR as lines and quarter arcs at `scale`, starting after the first corner."""
    n, out = len(STAIR), []
    corners: list[tuple[Point, Point, float, int]] = []
    for i, (x, y) in enumerate(STAIR):
        (ax, ay), (bx, by) = STAIR[i - 1], STAIR[(i + 1) % n]
        convex = (x - ax) * (by - y) - (y - ay) * (bx - x) > 0
        r = (CONVEX if convex else CONCAVE) if round_ else 0.0

        def sgn(v: float) -> int:
            return (v > 0) - (v < 0)

        s = (x + sgn(ax - x) * r, y + sgn(ay - y) * r)
        e = (x + sgn(bx - x) * r, y + sgn(by - y) * r)
        corners.append((s, e, r, 1 if convex else 0))
    for k in range(1, n + 1):
        prev, (s, e, r, sweep) = corners[k - 1], corners[k % n]
        out.append((prev[1], s, None))
        if r:
            out.append((s, e, (r, sweep)))
    return [((a[0] * scale, a[1] * scale), (b[0] * scale, b[1] * scale),
             (arc[0] * scale, arc[1]) if arc else None) for a, b, arc in out]  # fmt: skip


def halves(scale: float = 1.0, round_: bool = True) -> tuple[str, str]:
    """The ink and accent outlines: STAIR cut at y=SPLIT into two closed paths. Every quarter
    arc lies between its end points, so a segment's side is its end points' side; a segment
    on the cut belongs to neither, the closing Z draws it."""
    cut = SPLIT * scale
    segs = [s for s in segments(scale, round_) if (s[0][1], s[1][1]) != (cut, cut)]
    above = [max(a[1], b[1]) <= cut for a, b, _ in segs]
    paths = []
    for side in (True, False):
        # Rotate so the run of this side starts at index 0: the outline crosses the cut twice.
        start = next(
            i for i in range(len(segs)) if above[i] == side and above[i - 1] != side
        )
        run = [segs[(start + i) % len(segs)] for i in range(len(segs))]
        run = run[
            : next(
                i for i, _ in enumerate(run) if above[(start + i) % len(segs)] != side
            )
        ]
        d = f"M{num(run[0][0][0])} {num(run[0][0][1])}"
        for _, (x, y), arc in run:
            if arc:
                d += f"A{num(arc[0])} {num(arc[0])} 0 0 {arc[1]} {num(x)} {num(y)}"
            else:
                d += f"L{num(x)} {num(y)}"
        paths.append(d + "Z")
    # The two halves share the cut, and a renderer that antialiases each path on its own (WebKit)
    # lets the background through there: a hairline at the colour change. The ink half carries
    # bands below the cut that the accent, drawn after it, covers completely, so the cut is
    # always ink under accent. A thin one reaches into the inner corners (the accent's fillet at
    # x=13-14 leaves y<24.29 open), a deep one sits where the slab is solid, at least one pixel
    # at every size the mark is drawn.
    bands = [(13.7, 24.4, 0.25), (14.0, 28.0, 3.0)] if round_ else [(14.0, 24.0, 3.0)]
    band = "".join(
        f"M{num(left * scale)} {num(cut)}H{num(right * scale)}"
        f"V{num(cut + depth * scale)}H{num(left * scale)}Z"
        for left, right, depth in bands
    )
    return paths[0] + band, paths[1]


def mark_group(theme: str) -> str:
    """The mark as the README header draws it, in `theme`'s colours."""
    (ink, acc), (top, bottom) = INKS[theme], halves()
    return f'<g {MARK_AT}><path fill="{ink}" d="{top}"/><path fill="{acc}" d="{bottom}"/></g>'


def app_logo() -> str:
    """The mark in the app's page header, coloured by the page's tokens through app.css."""
    top, bottom = halves()
    return (
        '<svg class="logo" viewBox="2 2 28 28" aria-hidden="true">'
        f'<path class="i" d="{top}"/><path class="a" d="{bottom}"/></svg>'
    )


def quote(svg: str) -> str:
    """Percent-encode an SVG for a data: URL, keeping it readable (spaces, slashes, colons)."""
    return urlquote(svg, safe=" /:=;,.-()'")


def favicon() -> str:
    """The tab icon: the outline at half scale with square corners, so every edge lands on a
    whole pixel at 16 px, light ink by default and dark ink under a dark scheme."""
    top, bottom = halves(0.5, round_=False)
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-label="gitterrace" '
        f'viewBox="0 0 16 16" width="16" height="16"><title>gitterrace</title>{style()}'
        f'<path class="i" d="{top}"/><path class="a" d="{bottom}"/></svg>'
    )


def style() -> str:
    """Ink and accent classes that follow the system colour scheme."""
    (li, la), (di, da) = INKS["light"], INKS["dark"]
    return (
        f"<style>.i{{fill:{li}}}.a{{fill:{la}}}"
        f"@media (prefers-color-scheme:dark){{.i{{fill:{di}}}.a{{fill:{da}}}}}</style>"
    )


def contrast(a: str, b: str) -> float:
    """WCAG contrast ratio of two #rrggbb colours."""

    def lum(h: str) -> float:
        c = [int(h[i : i + 2], 16) / 255 for i in (1, 3, 5)]
        c = [x / 12.92 if x <= 0.03928 else ((x + 0.055) / 1.055) ** 2.4 for x in c]
        return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]

    hi, lo = sorted((lum(a), lum(b)), reverse=True)
    return (hi + 0.05) / (lo + 0.05)


def find(pattern: str, text: str, flags: int = 0) -> re.Match[str]:
    """re.search that stops the script when the header layout no longer has `pattern`."""
    m = re.search(pattern, text, flags)
    if not m:
        raise SystemExit(f"header layout changed: no match for {pattern}")
    return m


def build(header: str) -> dict[str, str]:
    mark = MARK_GROUP.search(header)
    # The mark draws paths of its own (the tread), so count the text outside the group.
    paths = PATHS.findall(MARK_GROUP.sub("", header))
    if not mark or len(paths) != 3:
        raise SystemExit(
            "header layout changed: expected the mark group and 3 text paths"
        )
    git, terrace = (find(r'fill="(#[0-9a-f]{6})"', p).group(1) for p in paths[:2])
    if contrast(git, terrace) < MIN_SPLIT:
        raise SystemExit(
            f"'git' {git} and 'terrace' {terrace} differ by {contrast(git, terrace):.2f}:1, "
            f"under {MIN_SPLIT}:1, so the two halves of the wordmark read as one colour"
        )
    x, y, w, h = LOGO_BOX
    head = '<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-label="gitterrace"'
    logo = (
        f'{head} viewBox="{x} {y} {w} {h}" width="{w}" height="{h}">'
        f"<title>gitterrace</title>{mark.group(0)}{paths[0]}{paths[1]}</svg>\n"
    )
    inner = mark.group(0).replace(f" {MARK_AT}", "")
    square = (
        f'{head} viewBox="0 0 32 32" width="32" height="32">'
        f"<title>gitterrace</title>{inner}</svg>\n"
    )
    return {"logo": logo, "mark": square}


def adaptive(light: str) -> str:
    """The mark for the site's icon: light ink by default, dark ink under a dark scheme. A
    tab bar follows the system theme, and one fixed ink vanishes on one of them (dark stem on
    a dark tab bar measured 1.09:1)."""
    (li, la), body = INKS["light"], find(r"<g>.*</g>", light, re.S).group(0)
    body = body.replace(f'fill="{li}"', 'class="i"').replace(
        f'fill="{la}"', 'class="a"'
    )
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-label="gitterrace" '
        f'viewBox="0 0 32 32" width="32" height="32"><title>gitterrace</title>{style()}{body}</svg>\n'
    )


def sync(path: Path, text: str, check: bool, stale: list[Path]) -> None:
    """Write `text` to `path`, or under --check record the path when it differs."""
    if path.exists() and path.read_text() == text:
        return
    if check:
        stale.append(path.relative_to(ROOT))
    else:
        path.write_text(text)
        print(f"wrote {path.relative_to(ROOT)}")


def main() -> int:
    check = "--check" in sys.argv
    stale: list[Path] = []
    marks: dict[str, str] = {}
    for theme in ("light", "dark"):
        path = BRAND / f"readme-header-{theme}.svg"
        header = path.read_text()
        if not MARK_GROUP.search(header):
            raise SystemExit(f"{path}: no mark group {MARK_AT}")
        header = MARK_GROUP.sub(lambda _: mark_group(theme), header, count=1)  # noqa: B023
        sync(path, header, check, stale)
        built = build(header)
        marks[theme] = built["mark"]
        for name, svg in built.items():
            sync(BRAND / f"{name}-{theme}.svg", svg, check, stale)
    sync(BRAND / "mark.svg", adaptive(marks["light"]), check, stale)
    # The app inlines its favicon (the CSP refuses a separate request-free icon otherwise),
    # so the one include every page head uses carries exactly this SVG.
    tpl = TEMPLATES / "_favicon.html"
    link = f'<link rel="icon" href="data:image/svg+xml,{quote(favicon())}">\n'
    sync(tpl, link, check, stale)
    # Every page header draws the same mark inline.
    for page in sorted(TEMPLATES.glob("*.html")):
        text = page.read_text()
        if APP_LOGO.search(text):
            sync(page, APP_LOGO.sub(lambda _: app_logo(), text), check, stale)
    for s in stale:
        print(f"{s}: does not match the README header; run scripts/theme-logo-gen.py")
    # Every template that writes its own <head> takes the icon from that include, so a new
    # page shell cannot ship without a favicon.
    bare = [
        t.relative_to(ROOT)
        for t in sorted(tpl.parent.glob("*.html"))
        if "<head>" in t.read_text()
        and '{% include "_favicon.html" %}' not in t.read_text()
    ]
    for t in bare:
        print(f'{t}: has a <head> without {{% include "_favicon.html" %}}')
    return 1 if stale or bare else 0


if __name__ == "__main__":
    raise SystemExit(main())
