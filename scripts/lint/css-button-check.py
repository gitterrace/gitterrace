#!/usr/bin/env python3
"""Fail when a stylesheet gives a button its own background or border outside the button rules.

Buttons share one vocabulary, buttons.css, the section between "buttons: the one vocabulary" and
"end of buttons". A component that paints its own button (.send-go once, whose hover turned it
dark) drifts from the rest in look and in states. A button class is any class the markup puts on
a <button> (templates, Rust, el("button", ...) in static JS) or on an element with .btn, plus the
variants. A selector whose last part is a button or carries one of those classes may set size
and placement, never `background*` or `border`, `border-color`, `border-style`, `border-width`
or a side of them. Rules under forced colors are exempt: that palette is the user's.

  python3 scripts/lint/css-button-check.py              check the tree
  python3 scripts/lint/css-button-check.py --self-test  prove a planted rule goes red
"""

from __future__ import annotations

import re
import sys
from collections.abc import Iterator
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WEB = ROOT / "crates/gitterrace-web"
START = "---- buttons: the one vocabulary"
END = "---- end of buttons ----"
VARIANTS = {"btn", "primary", "ghost", "danger", "icon", "sm"}
PAINT = re.compile(
    r"^(background(-[a-z]+)?|border(-(top|right|bottom|left))?(-(color|style|width))?)$"
)
# Where markup names a button's classes.
MARKUP = [
    re.compile(
        r"<button\s[^<>\"]*(?:\\?\"[^\"\\]*\\?\"[^<>\"]*)*?class=\\?\"([^\"\\]*)"
    ),
    re.compile(r"el\(\s*\"button\"\s*,\s*\"([^\"]*)\""),
    re.compile(r"class=\\?\"([^\"\\]*\bbtn\b[^\"\\]*)"),
]


def button_classes() -> set[str]:
    """Every class the markup puts on a button, and the variants."""
    found = set(VARIANTS)
    files = [
        *WEB.glob("templates/*.html"),
        *WEB.glob("src/**/*.rs"),
        *WEB.glob("static/*.js"),
    ]
    for f in files:
        if "tests" in f.name or "/tests/" in str(f):
            continue
        text = f.read_text(errors="replace")
        for rx in MARKUP:
            for m in rx.finditer(text):
                found.update(
                    c for c in m.group(1).split() if re.fullmatch(r"[\w-]+", c)
                )
    return found


def rules(css: str) -> Iterator[tuple[str, str, bool]]:
    """(selector list, declarations, exempt) for each style rule. Exempt: in the button
    section or under forced colors."""
    css = re.sub(r"url\([^)]*\)", "url()", css)
    i, stack, section = 0, [], False
    buf = ""
    while i < len(css):
        if css.startswith("/*", i):
            j = css.find("*/", i + 2)
            j = len(css) if j < 0 else j + 2
            note = css[i:j]
            if START in note:
                section = True
            if END in note:
                section = False
            i = j
            continue
        ch = css[i]
        if ch == "{":
            head = buf.strip()
            buf = ""
            if head.startswith("@"):
                stack.append(head)
                i += 1
                continue
            j = css.find("}", i)
            j = len(css) if j < 0 else j
            exempt = section or any("forced-colors" in s for s in stack)
            yield head, css[i + 1 : j], exempt
            i = j + 1
            continue
        if ch == "}":
            if stack:
                stack.pop()
            buf = ""
        elif ch == ";" and not stack and buf.strip().startswith("@"):
            buf = ""
        else:
            buf += ch
        i += 1


def split_top(text: str, sep: str) -> list[str]:
    out, depth, cur = [], 0, ""
    for ch in text:
        depth += ch in "(["
        depth -= ch in ")]"
        if ch == sep and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    return [*out, cur]


def paints_button(sel: str, classes: set[str]) -> bool:
    """Whether the last part of `sel` is a button itself, not a glyph or child inside one."""
    last = re.split(r"[\s>+~]+", re.sub(r"\([^()]*\)", "", sel.strip()))[-1]
    if "::" in last:
        return False
    tag = re.match(r"^[a-z]+", last)
    if tag and tag.group(0) not in ("button", "summary"):
        return False
    if tag and tag.group(0) == "button":
        return True
    return any(c in classes for c in re.findall(r"\.([\w-]+)", last))


def problems(css: str, name: str, classes: set[str]) -> list[str]:
    bad = []
    for head, body, exempt in rules(css):
        if exempt:
            continue
        props = [d.split(":", 1)[0].strip() for d in split_top(body, ";") if ":" in d]
        painted = [p for p in props if PAINT.match(p)]
        if not painted:
            continue
        for sel in split_top(head, ","):
            if paints_button(sel, classes):
                bad.append(
                    f"{name}: {sel.strip()} sets {', '.join(painted)} on a button"
                )
    return bad


def check() -> list[str]:
    classes = button_classes()
    bad = []
    for f in sorted((WEB / "static").glob("*.css")):
        bad += problems(f.read_text(), f.name, classes)
    return bad


def self_test() -> int:
    classes = {"send-go", *VARIANTS}
    red = [
        ".send-go{height:28px;background:var(--accent)}",
        ".foot button:hover{border-color:red}",
        ".x .primary{border:0}",
    ]
    green = [
        "/* ---- buttons: the one vocabulary */.primary{background:red}"
        "/* ---- end of buttons ---- */",
        ".send-go{margin-left:auto}",
        ".send-go::before{background:currentColor}",
        ".ov-list li.icon{background:red}",
        "@media (forced-colors:active){button{border:1px solid CanvasText}}",
        "@media (max-width:600px){.send-go{flex:1}}",
    ]
    fails = [c for c in red if not problems(c, "red", classes)]
    fails += [c for c in green if problems(c, "green", classes)]
    for c in fails:
        print(f"css-button self-test: wrong verdict for {c}")
    return 1 if fails else 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    bad = check()
    for b in bad:
        print(b)
    if bad:
        print("Pick a variant in buttons.css instead (see the comment at its top).")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
