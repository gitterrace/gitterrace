#!/usr/bin/env python3
"""Solve gitterrace's colour tokens from a tendir theme.

tendir's theme system (tendir/docs/contributing/design/colour-system.md)
defines ink, accent, signals and pigments. A code, diff and data product needs
more roles: page surfaces, diff rows and word highlights, a heat ramp, syntax
tokens, chart lines, selection and focus. This script derives each of them
from the theme's own tokens by a fixed rule, never by eye, checks every floor,
and prints the result.

    scripts/solve-theme.py                 # report for all four themes
    scripts/solve-theme.py --css           # the Kandel colour blocks
    scripts/solve-theme.py --write         # rewrite tokens.css between markers
    scripts/solve-theme.py --check         # fail if tokens.css drifted or a floor fails
    scripts/solve-theme.py --markdown      # tables for docs/theme-extension.md

Input: tendir/docs/generated/theme-tokens.json (TENDIR_THEME_TOKENS overrides
the path). No dependencies: the colour math is vendored below.
"""

import json
import math
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOKENS_CSS = ROOT / "crates/gitterrace-web/static/tokens.css"
DEFAULT_JSON = ROOT.parent / "tendir/docs/generated/theme-tokens.json"
THEMES = ["kandel", "elz", "hochburg", "schauinsland"]
BEGIN, END = "/* BEGIN solved colours */", "/* END solved colours */"

# ---------------------------------------------------------------- the rules
#
# Every constant below is a rule, named after the tendir rule it extends.
RULES = {
    # Page surfaces: tendir.sh's grounds (see `grounds()`), so gitterrace and
    # tendir's website are one family. The reading surface is the page and the
    # card; every other neutral is primary ink laid over it at a fixed share,
    # the construction of tendir's chipFillQuiet.
    "surface_share": {
        #            light  dark
        "bg": (0.000, 0.000),
        "surface": (0.000, 0.000),
        "soft": (0.035, 0.045),
        "hover": (0.060, 0.075),
        "line": (0.110, 0.110),
        "line_strong": (0.220, 0.200),
    },
    # Selected row: tendir's selection wash, at its 12% (hover 5%) share.
    "selection_share": 0.12,
    # Diff rows and their word highlight: the signal over the surface at a
    # fixed share. A row must carry at least the themed-fill cast (CIEDE2000
    # 2.0 light, 4.0 dark) from its own grey; the highlight steps one pill
    # share above the row.
    "row_share": {"light": 0.10, "dark": 0.14},
    "hl_share": {"light": 0.24, "dark": 0.28},
    "cast": {"light": 2.0, "dark": 4.0},
    # Text selection: tendir's choiceFill share (accent at 24% light, 35% dark).
    "choice_share": {"light": 0.24, "dark": 0.35},
    # Heat ramp: seq-0 is the hover surface, seq-4 the accentInk, and the three
    # steps between take equal OKLCH lightness steps at the accent hue, chroma
    # rising linearly, so the ramp reads in greyscale.
    # Syntax: tendir entity pigments, by role (Schauinsland: secondary ink).
    "syntax": {
        "comment": "ink.tertiary",
        "keyword": "entity.Purple",
        "string": "entity.Green",
        "constant": "entity.Blue",
        "entity": "accentInk",
        "tag": "entity.Teal",
        "param": "entity.Orange",
    },
    # Categorical series: entity pigments in a fixed order, "other" neutral.
    "cat": ["Blue", "Orange", "Green", "Pink", "Teal", "Red", "Yellow"],
    # Pigment chroma band (tendir: Kandel and Hochburg 0.105, Elz 0.125).
    "band": {"kandel": 0.105, "hochburg": 0.105, "elz": 0.125, "schauinsland": 0.0},
}

# ---------------------------------------------------------------- colour math


def hex_rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i : i + 2], 16) / 255 for i in (0, 2, 4))


def rgb_hex(rgb):
    return "#" + "".join(f"{round(max(0, min(1, c)) * 255):02x}" for c in rgb)


def lin(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def delin(c):
    return 12.92 * c if c <= 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055


def rgb_oklab(rgb):
    r, g, b = (lin(c) for c in rgb)
    l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b
    m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b
    s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b
    l, m, s = (math.copysign(abs(x) ** (1 / 3), x) for x in (l, m, s))
    return (
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    )


def oklab_rgb(lab):
    L, a, b = lab
    l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3
    m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3
    s = (L - 0.0894841775 * a - 1.2914855480 * b) ** 3
    r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
    g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
    bb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
    return tuple(delin(c) if c > 0 else 0.0 for c in (r, g, bb))


def oklch(h):
    L, a, b = rgb_oklab(hex_rgb(h))
    C = math.hypot(a, b)
    H = math.degrees(math.atan2(b, a)) % 360
    return L, C, H


def in_gamut(L, C, H):
    a, b = C * math.cos(math.radians(H)), C * math.sin(math.radians(H))
    L_, m, s = (
        L + 0.3963377774 * a + 0.2158037573 * b,
        L - 0.1055613458 * a - 0.0638541728 * b,
        L - 0.0894841775 * a - 1.2914855480 * b,
    )
    l3, m3, s3 = L_**3, m**3, s**3
    rgb = (
        4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
        -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
        -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3,
    )
    return all(-1e-6 <= c <= 1 + 1e-6 for c in rgb)


def from_lch(L, C, H):
    # Clamp chroma into sRGB with the hue and lightness held.
    while C > 0 and not in_gamut(L, C, H):
        C -= 0.001
    C = max(C, 0)
    a, b = C * math.cos(math.radians(H)), C * math.sin(math.radians(H))
    return rgb_hex(oklab_rgb((L, a, b)))


def over(fg, bg, share):
    """fg laid over bg at `share` opacity, composited in sRGB as a browser does."""
    f, b = hex_rgb(fg), hex_rgb(bg)
    return rgb_hex(tuple(share * x + (1 - share) * y for x, y in zip(f, b)))


def luminance(h):
    r, g, b = (lin(c) for c in hex_rgb(h))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a, b):
    la, lb = sorted((luminance(a), luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def apca(text, bg):
    """APCA-W3 0.0.98G lightness contrast, absolute Lc."""

    def y(h):
        r, g, b = hex_rgb(h)
        v = 0.2126729 * r**2.4 + 0.7151522 * g**2.4 + 0.0721750 * b**2.4
        return v if v > 0.022 else v + (0.022 - v) ** 1.414

    yt, yb = y(text), y(bg)
    if abs(yb - yt) < 0.0005:
        return 0.0
    if yb > yt:
        s = (yb**0.56 - yt**0.57) * 1.14
        lc = 0 if s < 0.1 else s - 0.027
    else:
        s = (yb**0.65 - yt**0.62) * 1.14
        lc = 0 if s > -0.1 else s + 0.027
    return abs(lc * 100)


def rgb_lab(rgb):
    r, g, b = (lin(c) for c in rgb)
    x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047
    y = 0.2126 * r + 0.7152 * g + 0.0722 * b
    z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883

    def f(t):
        return t ** (1 / 3) if t > 216 / 24389 else (24389 / 27 * t + 16) / 116

    fx, fy, fz = f(x), f(y), f(z)
    return 116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)


def de2000(h1, h2, rgb=False):
    L1, a1, b1 = rgb_lab(h1 if rgb else hex_rgb(h1))
    L2, a2, b2 = rgb_lab(h2 if rgb else hex_rgb(h2))
    C1, C2 = math.hypot(a1, b1), math.hypot(a2, b2)
    Cb = (C1 + C2) / 2
    G = 0.5 * (1 - math.sqrt(Cb**7 / (Cb**7 + 25**7)))
    a1p, a2p = a1 * (1 + G), a2 * (1 + G)
    C1p, C2p = math.hypot(a1p, b1), math.hypot(a2p, b2)
    h1p = math.degrees(math.atan2(b1, a1p)) % 360 if C1p else 0
    h2p = math.degrees(math.atan2(b2, a2p)) % 360 if C2p else 0
    dL, dC = L2 - L1, C2p - C1p
    dh = 0
    if C1p * C2p:
        dh = h2p - h1p
        if dh > 180:
            dh -= 360
        elif dh < -180:
            dh += 360
    dH = 2 * math.sqrt(C1p * C2p) * math.sin(math.radians(dh / 2))
    Lb, Cbp = (L1 + L2) / 2, (C1p + C2p) / 2
    hb = h1p + h2p
    if C1p * C2p:
        if abs(h1p - h2p) > 180:
            hb = hb + 360 if hb < 360 else hb - 360
        hb /= 2
    T = (
        1
        - 0.17 * math.cos(math.radians(hb - 30))
        + 0.24 * math.cos(math.radians(2 * hb))
        + 0.32 * math.cos(math.radians(3 * hb + 6))
        - 0.20 * math.cos(math.radians(4 * hb - 63))
    )
    Sl = 1 + 0.015 * (Lb - 50) ** 2 / math.sqrt(20 + (Lb - 50) ** 2)
    Sc, Sh = 1 + 0.045 * Cbp, 1 + 0.015 * Cbp * T
    Rt = (
        -2
        * math.sqrt(Cbp**7 / (Cbp**7 + 25**7))
        * math.sin(math.radians(60 * math.exp(-(((hb - 275) / 25) ** 2))))
    )
    return math.sqrt(
        (dL / Sl) ** 2 + (dC / Sc) ** 2 + (dH / Sh) ** 2 + Rt * (dC / Sc) * (dH / Sh)
    )


# Machado, Oliveira and Fernandes (2009), severity 1.0, on linear RGB.
CVD = {
    "protan": ((0.152286, 1.052583, -0.204868), (0.114503, 0.786281, 0.099216), (-0.003882, -0.048116, 1.051998)),
    "deutan": ((0.367322, 0.860646, -0.227968), (0.280085, 0.672501, 0.047413), (-0.011820, 0.042940, 0.968881)),
    "tritan": ((1.255528, -0.076749, -0.178779), (-0.078411, 0.930809, 0.147602), (0.004733, 0.691367, 0.303900)),
}


def simulate(h, kind):
    v = [lin(c) for c in hex_rgb(h)]
    m = CVD[kind]
    out = [sum(m[i][j] * v[j] for j in range(3)) for i in range(3)]
    return tuple(delin(max(0, min(1, c))) for c in out)


def cvd_min(a, b):
    return min(de2000(simulate(a, k), simulate(b, k), rgb=True) for k in CVD)


# ---------------------------------------------------------------- the solver


def grey_of(h):
    L, _, _ = oklch(h)
    return from_lch(L, 0, 0)


def pigment(h, floor_on, floor, band):
    """tendir's pigment rule: keep the lightness unless it misses the floor,
    then move the least lightness that clears it, hue held, chroma in band."""
    L, C, H = oklch(h)
    C = min(C, band)
    dark_bg = luminance(floor_on[0]) < 0.18
    step = 0.002 if dark_bg else -0.002
    out = from_lch(L, C, H)
    while min(contrast(out, s) for s in floor_on) < floor:
        L += step
        out = from_lch(L, C, H)
    return out


def cast_fill(colour, surface, share, cast):
    """tendir's selection wash: `colour` over `surface` at `share`, its chroma
    raised (lightness and hue held) until the result stands `cast` CIEDE2000
    from its own grey. A neutral colour (Schauinsland) stays neutral."""
    L, C, H = oklch(colour)
    out = over(colour, surface, share)
    if C < 0.008:
        return out
    while de2000(out, grey_of(out)) < cast and C < 0.4:
        C += 0.002
        out = over(from_lch(L, C, H), surface, share)
    return out


def grounds(theme, mode):
    """tendir website's three grounds (website/scripts/sync-theme-tokens.mjs,
    `grounds()`): light is the theme background mixed toward white, dark is
    its lightness mixed toward black, keeping the hue at the chroma share the
    light ground keeps. Reused so gitterrace and tendir.sh share one family."""
    mix_light = {"forest": 0.88, "surface": 0.95, "raised": 0.80}
    mix_dark = {"forest": 0.70, "surface": 0.62, "raised": 0.54}
    bg = theme[mode]["background"]["hex"].lower()
    if mode == "light":
        return {k: over("#ffffff", bg, w) for k, w in mix_light.items()}
    bg_light = theme["light"]["background"]["hex"].lower()
    c_light = oklch(bg_light)[1]
    _, c_dark, h_dark = oklch(bg)
    share = oklch(over("#ffffff", bg_light, mix_light["forest"]))[1] / c_light if c_light > 1e-4 else 0
    out = {}
    for k, w in mix_dark.items():
        plain = over("#000000", bg, w)
        out[k] = plain if (c_dark < 1e-4 or share == 0) else from_lch(oklch(plain)[0], c_dark * share, h_dark)
    return out


def solve(theme, mode):
    t = theme[mode]
    hx = {k: v["hex"].lower() for k, v in t.items()}
    rules = RULES
    li = 0 if mode == "light" else 1
    g = grounds(theme, mode)
    # The page is tendir.sh's reading surface; the header band is its forest.
    # Every other neutral is ink over the page at a fixed share, so lines and
    # hover states sit on the same hue family instead of on pure grey.
    window = g["surface"]
    ink = hx["ink.primary"]
    s = {}
    for name, shares in rules["surface_share"].items():
        s[name] = over(ink, window, shares[li])
    tok = {
        "n-0": s["surface"],
        "n-1": s["bg"],
        "n-2": s["soft"],
        "n-3": s["hover"],
        "n-4": s["line"],
        "n-5": s["line_strong"],
        "n-6": hx["ink.tertiary"],
        "n-7": hx["ink.secondary"],
        "n-8": over(ink, hx["ink.secondary"], 0.5),
        "n-9": ink,
        "accent": hx["accentInk"],
        "accent-fg": hx["accentInk"],
        "accent-fill": hx["accent"],
    }
    surface = tok["n-0"]
    tok["accent-soft"] = cast_fill(hx["selectionWash"], surface, rules["selection_share"], rules["cast"][mode])
    # Header band: tendir.sh's forest ground, the one place gitterrace shows
    # the theme's own background hue, exactly as tendir's header does.
    tok["header"] = g["forest"]
    # Raised (popover, dialog): the page surface in light; in dark tendir's
    # raised ground, which lifts off the page without turning grey.
    tok["raised"] = surface if mode == "light" else g["raised"]
    # Scrim: tendir's SCRIM veil, black at 30% in both modes, so it only darkens.
    tok["scrim"] = "#0000004d"
    tok["selection"] = over(hx["accent"], surface, rules["choice_share"][mode])
    tok["focus"] = hx["accentInk"]
    tok["ring"] = hx["accentInk"] + ("66" if mode == "light" else "80")
    tok["on-accent"] = hx.get("onFill", "#ffffff").lower()
    for role, sig in (("add", "success"), ("del", "overdue"), ("warn", "warning")):
        tok[role] = hx[sig]
        tok[f"{role}-bg"] = over(hx[sig], surface, rules["row_share"][mode])
        if role != "warn":
            tok[f"{role}-hl"] = over(hx[sig], surface, rules["hl_share"][mode])
    tok["info"] = hx["deferred"]
    tok["info-bg"] = s["soft"]
    tok["done"] = hx["done"]
    # Heat ramp.
    L0, C0, _ = oklch(tok["n-3"])
    L4, C4, H4 = oklch(hx["accentInk"])
    tok["seq-0"] = tok["n-3"]
    for i in range(1, 5):
        f = i / 4
        tok[f"seq-{i}"] = from_lch(L0 + (L4 - L0) * f, C0 + (C4 - C0) * f, H4)
    tok["seq-4"] = hx["accentInk"]
    band = rules["band"][theme["id"]]
    code_bg = [s["soft"], tok["n-0"], tok["n-1"]]
    for i, name in enumerate(rules["cat"]):
        tok[f"cat-{i + 1}"] = pigment(hx[f"entity.{name}"], [tok["n-0"], tok["n-1"]], 3.0, max(band, 0.0))
    tok["cat-8"] = s["line_strong"]
    for role, src in rules["syntax"].items():
        if src.startswith("entity.") and band == 0:
            src = "ink.secondary"
        tok[f"syn-{role}"] = pigment(hx[src], code_bg, 4.5, max(band, 0.08) if band else 0)
    return tok


# ---------------------------------------------------------------- checks


def checks(tok, mode):
    """Every floor, as (name, value, floor, kind)."""
    bg, surf, soft = tok["n-1"], tok["n-0"], tok["n-2"]
    rows = []

    def c(name, fg, back, floor):
        rows.append((name, contrast(fg, back), floor, "wcag"))

    for rung, floor in (("n-9", 7), ("n-7", 4.5), ("n-6", 3)):
        for back in ("n-0", "n-1", "n-2", "n-3", "accent-soft"):
            c(f"{rung} on {back}", tok[rung], tok[back], floor)
    # tendir's choiceFill floors: primary 7 (it states 8.5), secondary 4.5.
    c("n-9 on selection", tok["n-9"], tok["selection"], 7)
    c("n-7 on selection", tok["n-7"], tok["selection"], 4.5)
    for sig in ("add", "del", "warn", "accent"):
        for back in ("n-0", "n-1", "accent-soft"):
            c(f"{sig} on {back}", tok[sig], tok[back], 4.5)
        c(f"{sig} on {sig}-bg" if sig != "accent" else "accent on n-2", tok[sig], tok[f"{sig}-bg"] if sig != "accent" else soft, 4.5)
    for role in ("add", "del"):
        c(f"n-9 on {role}-hl", tok["n-9"], tok[f"{role}-hl"], 7)
        c(f"n-7 on {role}-bg", tok["n-7"], tok[f"{role}-bg"], 4.5)
    c("n-9 on header", tok["n-9"], tok["header"], 7)
    c("n-7 on header", tok["n-7"], tok["header"], 4.5)
    c("accent on header", tok["accent"], tok["header"], 4.5)
    c("n-9 on raised", tok["n-9"], tok["raised"], 7)
    c("n-7 on raised", tok["n-7"], tok["raised"], 4.5)
    c("on-accent on accent-fill", tok["on-accent"], tok["accent-fill"], 4.5)
    c("focus on n-0", tok["focus"], surf, 3)
    c("focus on n-1", tok["focus"], bg, 3)
    c("seq-4 on n-0", tok["seq-4"], surf, 3)
    for i in range(1, 8):
        c(f"cat-{i} on n-0", tok[f"cat-{i}"], surf, 3)
    for k in ("comment", "keyword", "string", "constant", "entity", "tag", "param"):
        c(f"syn-{k} on n-2", tok[f"syn-{k}"], soft, 4.5)
    cast = RULES["cast"][mode]
    for f in ("add-bg", "del-bg", "warn-bg", "accent-soft"):
        src = {"add-bg": "add", "del-bg": "del", "warn-bg": "warn", "accent-soft": "accent"}[f]
        if oklch(tok[src])[1] < 0.008:
            continue  # a neutral source (Schauinsland) carries no cast by rule
        rows.append((f"{f} cast", de2000(tok[f], grey_of(tok[f])), cast, "de00"))
    rows.append(("add/del CVD", cvd_min(tok["add"], tok["del"]), 0, "info"))
    rows.append(("add-bg/del-bg CVD", cvd_min(tok["add-bg"], tok["del-bg"]), 0, "info"))
    rows.append(("add-hl/del-hl CVD", cvd_min(tok["add-hl"], tok["del-hl"]), 0, "info"))
    Ls = [oklch(tok[f"seq-{i}"])[0] for i in range(5)]
    mono = all((b - a) * (Ls[4] - Ls[0]) > 0 for a, b in zip(Ls, Ls[1:]))
    rows.append(("seq lightness monotonic", 1.0 if mono else 0.0, 1.0, "bool"))
    steps = [de2000(tok[f"seq-{i}"], tok[f"seq-{i + 1}"]) for i in range(4)]
    rows.append(("seq min adjacent step", min(steps), 5.0, "de00"))
    rows.append(("n-9 on n-1 APCA", apca(tok["n-9"], bg), 75, "apca"))
    rows.append(("n-7 on n-1 APCA", apca(tok["n-7"], bg), 60, "apca"))
    rows.append(("n-6 on n-1 APCA", apca(tok["n-6"], bg), 45, "apca"))
    return rows


def load():
    path = Path(os.environ.get("TENDIR_THEME_TOKENS", DEFAULT_JSON))
    data = json.loads(path.read_text())
    # The JSON export omits selectionWash; read it from theme.rs, the source
    # the JSON is generated from.
    rs = (path.parent.parent.parent / "crates/tendir-types/src/theme.rs").read_text()
    washes = {}
    for block in rs.split('id: "')[1:]:
        tid = block.split('"', 1)[0]
        m = re.search(r'selection_wash: p\("(#\w+)", "(#\w+)"\)', block)
        if m:
            washes[tid] = (m[1].lower(), m[2].lower())
    themes = {}
    for tid in THEMES:
        th = {m: dict(v) for m, v in data["themes"][tid].items()}
        for i, mode in enumerate(("light", "dark")):
            th[mode]["selectionWash"] = {"hex": washes[tid][i]}
        th["id"] = tid
        themes[tid] = th
    return themes


def solve_all():
    out = {}
    for tid, th in load().items():
        out[tid] = {m: solve(th, m) for m in ("light", "dark")}
    return out


ORDER = [
    "n-0", "n-1", "n-2", "n-3", "n-4", "n-5", "n-6", "n-7", "n-8", "n-9",
    "header", "raised", "scrim",
    "accent", "accent-fg", "accent-fill", "accent-soft", "selection", "focus", "ring", "on-accent",
    "add", "add-bg", "add-hl", "del", "del-bg", "del-hl", "warn", "warn-bg", "info", "info-bg", "done",
    "seq-0", "seq-1", "seq-2", "seq-3", "seq-4",
    "cat-1", "cat-2", "cat-3", "cat-4", "cat-5", "cat-6", "cat-7", "cat-8",
    "syn-comment", "syn-keyword", "syn-string", "syn-constant", "syn-entity", "syn-tag", "syn-param",
]


def css_vars(tok):
    parts = [f"--{k}:{tok[k]};" for k in ORDER]
    lines, line = [], ""
    for p in parts:
        if len(line) + len(p) > 150:
            lines.append(line.rstrip())
            line = ""
        line += p + " "
    lines.append(line.rstrip())
    return "\n".join(lines)


def css(themes):
    k = themes["kandel"]
    return "\n".join(
        [
            BEGIN,
            ":root{color-scheme:light;",
            css_vars(k["light"]),
            "--shadow-1:none; --shadow-2:0 8px 28px rgba(16,20,28,.12);",
            "}",
            "@media (prefers-color-scheme:dark){:root:not([data-theme=light]){color-scheme:dark;",
            css_vars(k["dark"]),
            "--shadow-1:none; --shadow-2:0 8px 28px rgba(0,0,0,.5);",
            "}}",
            ":root[data-theme=dark]{color-scheme:dark;",
            css_vars(k["dark"]),
            "--shadow-1:none; --shadow-2:0 8px 28px rgba(0,0,0,.5);",
            "}",
            END,
        ]
    )


def fmt_lch(h):
    if len(h) > 7:
        return "alpha"
    L, C, H = oklch(h)
    return f"{L:.3f} {C:.3f} " + ("-" if C < 0.008 else f"{H:.0f}")


def report(themes):
    bad = 0
    for tid, modes in themes.items():
        for mode, tok in modes.items():
            print(f"== {tid} {mode}")
            for k in ORDER:
                print(f"  {k:14} {tok[k]:10} {fmt_lch(tok[k])}")
            for name, val, floor, kind in checks(tok, mode):
                ok = kind == "info" or val >= floor
                bad += not ok
                print(f"  {'ok ' if ok else 'BAD'} {name:28} {val:7.2f}  floor {floor}")
    return bad


def markdown(themes):
    roles = [
        ("n-0", "surface"), ("n-1", "page bg"), ("n-2", "soft / code surface"), ("n-3", "hover, seq-0"),
        ("n-4", "line, chart grid"), ("n-5", "line-strong, axis, scrollbar thumb"), ("n-8", "ink between primary and secondary"),
        ("header", "header band"), ("raised", "popover, dialog"),
        ("accent-soft", "selected row"), ("selection", "text selection"), ("focus", "focus ring"),
        ("add-bg", "added row"), ("add-hl", "added word"), ("del-bg", "deleted row"), ("del-hl", "deleted word"),
        ("warn-bg", "warning row"), ("seq-1", "heat 1"), ("seq-2", "heat 2"), ("seq-3", "heat 3"), ("seq-4", "heat 4"),
        ("syn-comment", "comment"), ("syn-keyword", "keyword"), ("syn-string", "string"), ("syn-constant", "constant"),
        ("syn-entity", "entity"), ("syn-tag", "tag"), ("syn-param", "parameter"),
    ]
    against = {"n-0": "n-9", "n-1": "n-9", "n-2": "n-9", "n-3": "n-9", "n-4": "n-0", "n-5": "n-0", "n-8": "n-1",
               "header": "n-9", "raised": "n-9", "accent-soft": "n-9", "selection": "n-9", "focus": "n-0", "add-bg": "n-9", "add-hl": "n-9",
               "del-bg": "n-9", "del-hl": "n-9", "warn-bg": "n-9", "seq-1": "n-0", "seq-2": "n-0", "seq-3": "n-0", "seq-4": "n-0"}
    out = []
    for mode in ("light", "dark"):
        out.append(f"#### {mode.capitalize()}\n")
        out.append("Each cell: hex, OKLCH L C H, then contrast against the pair named in the second column.\n")
        out.append("| Token | Role (contrast vs) | " + " | ".join(t.capitalize() for t in themes) + " |")
        out.append("|---|---|" + "---|" * len(themes))
        for k, role in roles:
            vs = against.get(k, "n-2")
            cells = []
            for tid in themes:
                tok = themes[tid][mode]
                a, b = tok[k], tok[vs]
                cells.append(f"`{a}` {fmt_lch(a)} {contrast(a, b):.1f}:1")
            out.append(f"| `--{k}` | {role} (vs `{vs}`) | " + " | ".join(cells) + " |")
        out.append("")
    out.append("#### Separation and casts\n")
    out.append("| Check | " + " | ".join(f"{t.capitalize()} light | dark" for t in themes) + " |")
    out.append("|---|" + "---|---|" * len(themes))
    names = [r[0] for r in checks(themes["kandel"]["light"], "light") if r[3] in ("de00", "info")]
    for n in names:
        cells = []
        for tid in themes:
            for mode in ("light", "dark"):
                v = {r[0]: r[1] for r in checks(themes[tid][mode], mode)}.get(n)
                cells.append("n/a" if v is None else f"{v:.1f}")
        out.append(f"| {n} | " + " | ".join(cells) + " |")
    return "\n".join(out)


def splice(text, block):
    a, b = text.index(BEGIN), text.index(END) + len(END)
    return text[:a] + block + text[b:]


def main(argv):
    themes = solve_all()
    if "--css" in argv:
        print(css(themes))
    elif "--markdown" in argv:
        print(markdown(themes))
    elif "--write" in argv:
        TOKENS_CSS.write_text(splice(TOKENS_CSS.read_text(), css(themes)))
    elif "--check" in argv:
        bad = sum(
            1 for m in themes.values() for mode, tok in m.items()
            for r in checks(tok, mode) if r[3] != "info" and r[1] < r[2]
        )
        text = TOKENS_CSS.read_text()
        drift = splice(text, css(themes)) != text
        if drift:
            print("tokens.css drifted from the solver: run scripts/solve-theme.py --write")
        if bad:
            print(f"{bad} floor(s) fail: run scripts/solve-theme.py for the report")
        return 1 if drift or bad else 0
    else:
        return 1 if report(themes) else 0
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
