#!/usr/bin/env python3
"""Fail when a lazily loaded static script has no ready marker, or a check waits on a bad one.

app.js injects scripts on first use (gtLazy). Keys or clicks sent before such a script has run
are lost, so gtLazy sets data-<name>-ready on <html> once the script ran, and browser checks
wait for it with ready(p, name) from scripts/lib/browser-lib.mjs. This fails when:
  - gtLazy stops setting the marker (every lazy script would lose it at once)
  - a gtLazy("name") call names a script file that does not exist
  - a check calls ready(p, 'name') for a name no gtLazy call loads (it would wait forever)
  - a check polls a ...Ready attribute itself instead of the helper

  python3 scripts/lint/lazy-ready-check.py              check the tree
  python3 scripts/lint/lazy-ready-check.py --self-test  prove each case goes red
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STATIC = ROOT / "crates/gitterrace-web/static"
CHECKS = ROOT / "scripts/verify/browser"
MARKER = re.compile(r'dataset\[[^\]]*\+\s*"Ready"\]\s*=\s*"1"')


def lazy_names(sources: list[str]) -> set[str]:
    return {m for s in sources for m in re.findall(r'gtLazy\("([a-z-]+)"', s)}


def problems(app: str, sources: list[str], files: set[str], checks: dict[str, str]) -> list[str]:
    bad = []
    if not MARKER.search(app):
        bad.append('app.js: gtLazy does not set dataset[<name> + "Ready"] = "1" after a run')
    names = lazy_names([app, *sources])
    for n in sorted(names - files):
        bad.append(f"gtLazy({n!r}) names {n}.js, which is not in static/")
    for fname, text in sorted(checks.items()):
        for n in re.findall(r"\bready\(\w+, '([a-z-]+)'", text):
            if n not in names:
                bad.append(f"{fname}: ready(p, {n!r}) waits on a script no gtLazy call loads")
        if re.search(r"waitForFunction[^;]*\w+Ready", text):
            bad.append(f"{fname}: polls a Ready attribute itself, use ready(p, name)")
    return bad


def check_tree() -> int:
    srcs = {p.name: p.read_text(encoding="utf-8") for p in sorted(STATIC.glob("*.js"))}
    files = {n[:-3] for n in srcs}
    checks = {p.name: p.read_text(encoding="utf-8") for p in sorted(CHECKS.glob("*.mjs"))}
    bad = problems(srcs["app.js"], list(srcs.values()), files, checks)
    for line in bad:
        print(line)
    if bad:
        return 1
    print(f"lazy-ready-check: ok ({len(lazy_names(list(srcs.values())))} lazy scripts)")
    return 0


GOOD_APP = 'x; dataset[name + "Ready"] = "1"; gtLazy("one");'


def self_test() -> int:
    green = problems(GOOD_APP, [], {"one"}, {"c.mjs": "await ready(p, 'one');"})
    no_marker = problems('gtLazy("one");', [], {"one"}, {})
    missing = problems(GOOD_APP, [], set(), {})
    unknown = problems(GOOD_APP, [], {"one"}, {"c.mjs": "await ready(p, 'two');"})
    adhoc = problems(GOOD_APP, [], {"one"}, {"c.mjs": "p.waitForFunction(() => d.oneReady);"})
    ok = not green and no_marker and missing and unknown and adhoc
    print("lazy-ready-check self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


def main() -> int:
    return self_test() if "--self-test" in sys.argv[1:] else check_tree()


if __name__ == "__main__":
    sys.exit(main())
