#!/usr/bin/env python3
"""CONV-CSS-SCALE: no raw colour and no off-scale px in a stylesheet outside the token file.

Colours and sizes come from tokens. Two shapes fail:
  1. A literal colour: `#888`, a `%23888` inside a data URI (an SVG fill that ignores the theme),
     or `rgb()`, `hsl()`, `oklch()` and kin carrying a number.
  2. A px literal off the scale, in a property of `[css] props`. A shorthand is read value by
     value, so `padding:3px 8px` and the size in `font:600 9px/1 x` both count. A size must be a
     multiple of `[css] grid` or one of `[css] sub_grid` (contract/design.md: 1, 2, 6, and 0),
     and a font size one of `[css] type_scale`. Comments, `url()` bodies and the fallback of a
     `var()` are not read for px.

Settings are `[css]` in lint.toml: globs, exempt (files that may hold raw values, the token
file), grid, sub_grid, type_scale, props, min_files.

  css-scale.py [--repo DIR]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-CSS-SCALE"
COMMENT = re.compile(r"/\*.*?\*/", re.S)
URL = re.compile(r"url\((?:[^()\"']|\"[^\"]*\"|'[^']*')*\)")
DECL = re.compile(r"(?:^|[;{}])\s*([a-z-]+)\s*:\s*([^;{}]*)", re.I)
PX = re.compile(r"(?<![\w.#-])(\d*\.?\d+)px\b")
HEX = re.compile(r"(?<![\w&])#(?:[0-9a-f]{8}|[0-9a-f]{6}|[0-9a-f]{4}|[0-9a-f]{3})\b", re.I)
ENCODED = re.compile(r"%23(?:[0-9a-f]{8}|[0-9a-f]{6}|[0-9a-f]{3,4})\b", re.I)
FUNC = re.compile(r"\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch)\(([^()]*(?:\([^()]*\)[^()]*)*)\)", re.I)
TYPE_PROPS = {"font", "font-size", "letter-spacing"}
TOKEN_REF = re.compile(r"var\(--[\w-]+(?:,[^()]*)?\)")


def colour_problems(css: str) -> list[str]:
    out = [m.group(0) for m in HEX.finditer(URL.sub("url()", css))]
    out += [m.group(0) for m in ENCODED.finditer(css)]
    for m in FUNC.finditer(css):
        # A function made only of tokens (`hsl(var(--a) var(--b))`) carries no literal.
        if re.search(r"\d", TOKEN_REF.sub("", m.group(1))):
            out.append(m.group(0)[:40])
    return out


def px_problems(css: str, c: dict) -> list[str]:
    props, sub = set(c["props"]), {float(v) for v in c["sub_grid"]}
    scale = {float(v) for v in c["type_scale"]}
    out = []
    for m in DECL.finditer(URL.sub("url()", css)):
        prop, value = m.group(1).lower(), m.group(2)
        if prop not in props:
            continue
        for n in PX.finditer(TOKEN_REF.sub("", value)):
            v = float(n.group(1))
            if prop in TYPE_PROPS:
                ok = v in scale or (prop == "letter-spacing" and v in sub)
            else:
                ok = v in sub or v % c["grid"] == 0
            if not ok:
                out.append(f"{prop}: {n.group(0)}")
    return out


def check_text(text: str, c: dict) -> list[tuple[int, str]]:
    """(line, problem) for one stylesheet."""
    plain = COMMENT.sub(lambda m: re.sub(r"[^\n]", " ", m.group(0)), text)
    hits = []
    for n, line in enumerate(plain.splitlines(), 1):
        hits += [(n, f"raw colour {x}") for x in colour_problems(line)]
        hits += [(n, f"off-scale {p}") for p in px_problems(line, c)]
    return hits


def self_test(c: dict) -> bool:
    red = [
        "a{color:#888}", ".x{background:rgba(0,0,0,.4)}", ".x{padding:3px 8px}", ".x{font-size:22px}",
        ".av{font:600 9px/1.2 sans-serif}",
        "select{--chev:url(\"data:image/svg+xml,%3Cpath stroke='%23888'/%3E\")}",
    ]
    green = [
        ".x{padding:var(--space-2) var(--space-3);margin:0 0 16px;gap:2px;height:var(--row-h)}",
        ".x{padding:1px 6px;outline:2px solid var(--focus);font:var(--text-sm)/1.4 var(--mono)}",
        "/* a 3px note and #888 in a comment */\n.x{--hue:hsl(var(--h) var(--s) var(--l));width:35px;top:var(--pin,150px)}",
    ]
    ok = all(check_text(t, c) for t in red) and not any(check_text(t, c) for t in green)
    return config.self_check(RULE, ok, "a planted raw colour or off-scale px must fail and the tokens pass")


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    c = cfg["css"]
    if not self_test(c):
        return 2
    files = [f for f in config.tracked(root, cfg["scope"]["exclude"])
             if config.matches(f, c["globs"]) and not config.matches(f, c["exempt"])]
    if len(files) < c["min_files"]:
        return config.report(RULE, [f"only {len(files)} stylesheets scanned, the census is broken"], "census")
    bad = [f"{f}:{n}: {p}" for f in files
           for n, p in check_text((root / f).read_text(encoding="utf-8", errors="replace"), c)]
    return config.report(RULE, bad, f"{len(bad)} raw colour(s) or off-scale px value(s)",
                         f"no raw colour or off-scale px in {len(files)} stylesheets",
                         "Use a token: --space-N, --text-N, --radius, --row-h, --control-h, a colour role.")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
