#!/usr/bin/env python3
"""Fail when a browser check waits a fixed time instead of a condition.

A `waitForTimeout(n)` or a `setTimeout`-based sleep in scripts/verify/browser/*.mjs passes on an
idle machine and flakes under load: the page is read before it settled, or after more time than
it needed. Wait on the condition instead: `document.fonts.ready`, a selector, `waitForFunction`,
`waitForURL` or an `expect`-style poll. The sleeps that remain are counted per file in
scripts/guards/browser-sleeps.json, a ratchet: a count may fall, never rise, and a file whose
count fell must have its entry lowered, so the list cannot hide a new sleep behind an old one.

  python3 scripts/lint/sleeps-check.py              check the tree
  python3 scripts/lint/sleeps-check.py --self-test  prove a planted sleep goes red
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DIR = ROOT / "scripts/verify/browser"
ALLOWED = ROOT / "scripts/guards/browser-sleeps.json"
SLEEP = re.compile(r"\b(?:waitForTimeout|setTimeout)\s*\(")


def count(text: str) -> int:
    return len(SLEEP.findall(text))


def problems(found: dict[str, int], allowed: dict[str, int]) -> list[str]:
    bad = []
    for name in sorted(set(found) | set(allowed)):
        have, may = found.get(name, 0), allowed.get(name, 0)
        if have > may:
            bad.append(
                f"{name}: {have} fixed sleep(s), {may} allowed. Wait on a condition "
                "(selector, document.fonts.ready, waitForFunction, waitForURL)"
            )
        elif have < may:
            bad.append(f"{name}: {have} fixed sleep(s) left, lower its entry from {may}")
    return bad


def check() -> list[str]:
    found = {}
    for f in sorted(DIR.glob("*.mjs")):
        n = count(f.read_text())
        if n:
            found[f.name] = n
    allowed = json.loads(ALLOWED.read_text())["browser_sleeps"]
    return problems(found, allowed)


def self_test() -> int:
    wait = "await p.waitForTimeout(300);"
    nap = "await new Promise((ok) => setTimeout(ok, 600));"
    cases = [
        ("a new waitForTimeout", {"a.mjs": count(wait)}, {}, True),
        ("a new setTimeout sleep", {"a.mjs": count(nap)}, {"a.mjs": 0}, True),
        ("a sleep past the allowed count", {"a.mjs": 2}, {"a.mjs": 1}, True),
        ("a stale entry", {}, {"a.mjs": 1}, True),
        ("a clean file", {}, {}, False),
        ("sleeps at the allowed count", {"a.mjs": 1}, {"a.mjs": 1}, False),
        ("a poll on a selector", {"a.mjs": count("await p.waitForSelector('x');")}, {}, False),
    ]
    fails = [n for n, found, allowed, red in cases if bool(problems(found, allowed)) != red]
    for n in fails:
        print(f"sleeps self-test: wrong verdict for {n}")
    return 1 if fails else 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    bad = check()
    for b in bad:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
