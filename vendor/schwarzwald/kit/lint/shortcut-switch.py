#!/usr/bin/env python3
"""CONV-SHORTCUT-SWITCH: a keydown handler on a printable key consults the shortcut-off switch.

WCAG 2.1.4: a single printable-key shortcut can be turned off, remapped or scoped to focus. In
every script of `[a11y] scripts`, a `keydown` listener (`addEventListener('keydown', ...)`,
`onkeydown = ...`) whose inline body compares a key with a one-character literal (`e.key === 'j'`,
`case 'j':`) must also name the switch, `[a11y] shortcut_switch` (default `shortcutsOff`).
A comparison on a line that also tests `ctrlKey`, `metaKey` or `altKey` is a modifier chord, which
2.1.4 does not cover, so it does not count.
A handler passed by name is not followed: pass an inline function, or consult the switch in
the one handler the listener calls.

  shortcut-switch.py [--repo DIR]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-SHORTCUT-SWITCH"
LISTENER = re.compile(r"""addEventListener\(\s*['"]keydown['"]|\bonkeydown\s*=""")
PRINTABLE = re.compile(r"""\bkey\s*[!=]==?\s*(['"])[^\s'"\\]\1|\bcase\s+(['"])[^\s'"\\]\2\s*:""")
CHORD = re.compile(r"\b(ctrlKey|metaKey|altKey)\b")


def bare_printable(b: str) -> bool:
    """A printable-key comparison on a line with no modifier test: a single-key shortcut."""
    return any(PRINTABLE.search(ln) and not CHORD.search(ln) for ln in b.splitlines())


def body(text: str, start: int) -> str:
    """The first `{...}` block after `start`, braces matched, or the rest of the line when none."""
    i = text.find("{", start)
    nl = text.find("\n", start)
    if i < 0 or (0 <= nl < i and "=>" in text[start:nl]):
        return text[start : nl if nl >= 0 else len(text)]
    depth = 0
    for j in range(i, len(text)):
        depth += (text[j] == "{") - (text[j] == "}")
        if depth == 0:
            return text[i : j + 1]
    return text[i:]


def problems(rel: str, text: str, switch: str) -> list[str]:
    out = []
    for m in LISTENER.finditer(text):
        b = body(text, m.end())
        if bare_printable(b) and switch not in b:
            out.append(f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: a printable-key keydown handler never reads {switch}")
    return out


def self_test() -> bool:
    red = "document.addEventListener('keydown', (e) => {\n  if (e.key === 'j') next();\n});\n"
    red_case = "el.onkeydown = function (e) {\n  switch (e.key) { case 'k': prev(); }\n};\n"
    green = "document.addEventListener('keydown', (e) => {\n  if (shortcutsOff) return;\n  if (e.key === 'j') next();\n});\n"
    named = "document.addEventListener('keydown', (e) => {\n  if (e.key === 'Escape') close();\n});\n"
    chord = "document.addEventListener('keydown', (e) => {\n  if ((e.metaKey || e.ctrlKey) && e.key === 's') save();\n});\n"
    mixed = "document.addEventListener('keydown', (e) => {\n  if (e.ctrlKey && e.key === 'n') down();\n  if (e.key === 'j') next();\n});\n"
    ok = (
        bool(problems("r", red, "shortcutsOff")) and bool(problems("r", red_case, "shortcutsOff"))
        and bool(problems("r", mixed, "shortcutsOff"))
        and not problems("g", green, "shortcutsOff") and not problems("g", named, "shortcutsOff")
        and not problems("g", chord, "shortcutsOff")
    )
    return config.self_check(RULE, ok, "a planted unguarded printable-key handler must fail and the guarded one pass")


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    switch = cfg["a11y"]["shortcut_switch"]
    bad, seen = [], 0
    for rel in config.tracked(root, cfg["scope"]["exclude"]):
        if not config.matches(rel, cfg["a11y"]["scripts"]) or config.matches(rel, cfg["scope"]["golden"]):
            continue
        seen += 1
        bad += problems(rel, (root / rel).read_text(encoding="utf-8", errors="replace"), switch)
    return config.report(RULE, bad, f"{len(bad)} printable-key handler(s) ignore the shortcut switch",
                         f"every printable-key handler reads {switch} ({seen} scripts)",
                         f"Return early when {switch} is set, before the key is compared.")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
