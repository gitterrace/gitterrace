#!/usr/bin/env python3
"""Write the plain logos in docs/brand from the README headers (stdlib only).

The header SVGs are the single source: the terrace mark, the outlined "git" and "terrace",
then the tagline. This keeps the first three and drops the tagline, and also writes the
mark alone, for each theme:

    logo-{light,dark}.svg   mark and wordmark, cropped
    mark-{light,dark}.svg   the mark alone, square

    python3 scripts/theme-logo.py [--check]

`make theme` runs the check, so the logos cannot drift from the headers.
"""
import re
import sys
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BRAND = ROOT / "docs/brand"
# The header's wordmark ends near x=545 and its tagline starts below y=128; the mark's group
# draws in the 24x24 box scaled by 5.2 from (30, 22).
LOGO_BOX = (24, 18, 540, 120)
MARK_GROUP = re.compile(r"<g transform=\"translate\(30 22\) scale\(5\.2\)\">.*?</g>", re.S)
PATHS = re.compile(r'<path fill="#[0-9a-f]{6}" d="[^"]*"/>')
# The wordmark's two halves must stay visibly apart in every theme.
MIN_SPLIT = 3.0


def contrast(a: str, b: str) -> float:
    """WCAG contrast ratio of two #rrggbb colours."""

    def lum(h: str) -> float:
        c = [int(h[i : i + 2], 16) / 255 for i in (1, 3, 5)]
        c = [x / 12.92 if x <= 0.03928 else ((x + 0.055) / 1.055) ** 2.4 for x in c]
        return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]

    hi, lo = sorted((lum(a), lum(b)), reverse=True)
    return (hi + 0.05) / (lo + 0.05)


def build(header: str) -> dict[str, str]:
    mark = MARK_GROUP.search(header)
    # The mark draws paths of its own (the tread), so count the text outside the group.
    paths = PATHS.findall(MARK_GROUP.sub("", header))
    if not mark or len(paths) != 3:
        raise SystemExit("header layout changed: expected the mark group and 3 text paths")
    git, terrace = (re.search(r'fill="(#[0-9a-f]{6})"', p).group(1) for p in paths[:2])
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
    inner = mark.group(0).replace(' transform="translate(30 22) scale(5.2)"', "")
    square = (
        f'{head} viewBox="0 0 24 24" width="24" height="24">'
        f"<title>gitterrace</title>{inner}</svg>\n"
    )
    return {"logo": logo, "mark": square}


def adaptive(light: str, dark: str) -> str:
    """The mark for favicons: light ink by default, dark ink under a dark scheme. A tab bar
    follows the system theme, and one fixed ink vanishes on one of them (dark stem on a dark
    tab bar measured 1.09:1)."""
    def inks(svg: str) -> tuple[str, str]:
        stem = re.search(r'stroke="(#[0-9a-f]{6})"', svg).group(1)
        tread = re.search(r'<rect fill="(#[0-9a-f]{6})"', svg).group(1)
        return stem, tread
    (ls, lt), (ds, dt) = inks(light), inks(dark)
    body = re.search(r"<g>.*</g>", light, re.S).group(0)
    body = body.replace(f'stroke="{ls}"', 'class="i"').replace(f'<rect fill="{lt}"', '<rect class="a"')
    style = (
        f"<style>.i{{stroke:{ls}}}.a{{fill:{lt}}}"
        f"@media (prefers-color-scheme:dark){{.i{{stroke:{ds}}}.a{{fill:{dt}}}}}</style>"
    )
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-label="gitterrace" '
        f'viewBox="0 0 24 24" width="24" height="24"><title>gitterrace</title>{style}{body}</svg>\n'
    )


def main() -> int:
    check = "--check" in sys.argv
    stale = []
    marks = {}
    for theme in ("light", "dark"):
        header = (BRAND / f"readme-header-{theme}.svg").read_text()
        built = build(header)
        marks[theme] = built["mark"]
        for name, svg in built.items():
            path = BRAND / f"{name}-{theme}.svg"
            if path.exists() and path.read_text() == svg:
                continue
            if check:
                stale.append(path.relative_to(ROOT))
            else:
                path.write_text(svg)
                print(f"wrote {path.relative_to(ROOT)}")
    path, svg = BRAND / "mark.svg", adaptive(marks["light"], marks["dark"])
    if not (path.exists() and path.read_text() == svg):
        if check:
            stale.append(path.relative_to(ROOT))
        else:
            path.write_text(svg)
            print(f"wrote {path.relative_to(ROOT)}")
    # The app inlines the same mark as its favicon (the CSP refuses a separate request-free
    # icon otherwise), so the one include every page head uses must carry exactly this SVG.
    tpl = ROOT / "crates/gitterrace-web/templates/favicon.html"
    m = re.search(r'<link rel="icon" href="data:image/svg\+xml,([^"]*)">', tpl.read_text())
    if not m or urllib.parse.unquote(m.group(1)) != svg.strip():
        stale.append(tpl.relative_to(ROOT))
    for s in stale:
        print(f"{s}: does not match the README header; run scripts/theme-logo.py")
    # Every template that writes its own <head> takes the icon from that include, so a new
    # page shell cannot ship without a favicon.
    bare = [
        t.relative_to(ROOT)
        for t in sorted(tpl.parent.glob("*.html"))
        if "<head>" in t.read_text() and '{% include "favicon.html" %}' not in t.read_text()
    ]
    for t in bare:
        print(f'{t}: has a <head> without {{% include "favicon.html" %}}')
    return 1 if stale or bare else 0


raise SystemExit(main())
