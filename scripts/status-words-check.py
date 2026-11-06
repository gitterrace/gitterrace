#!/usr/bin/env python3
"""Flags two status facts run together without punctuation in user-visible strings.

Example: "on main clean" should read "on main, clean". Checks Rust string literals, templates
and JS UI text. Not a full parser, it looks for a status word right after a name or a
placeholder.

  python3 scripts/status-words-check.py              check the tree
  python3 scripts/status-words-check.py --self-test  prove it goes red on a planted case
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
COMMENT_EXTS = {".rs": "//", ".mjs": "//", ".py": "#", ".sh": "#"}
HISTORY = re.compile(r"\b(?:no longer|used to|previously|was changed)\b", re.I)
# Existing comments with history wording, `path:line` entries. Only shrinks.
ALLOW: set[str] = {"crates/gitterrace-web/src/wiring_live_tests.rs:543"}
EXTS = (".rs", ".html", ".js", ".mjs", ".md")
EXCLUDE = ("static/vendor/", "shared/schwarzwald/", "target/", "CHANGELOG.md")
W = r"(?:clean|dirty|ahead|behind|detached|untracked|modified)"
PATTERNS = [
    # "on main clean", "branch {name} ahead 2"
    re.compile(
        rf"\b(?:on|branch) (?:\{{\{{?[^}}]*\}}\}}?|(?!(?:a|an|the|any|this|that|no)\b)[\w./-]+) {W}\b(?! (?:tree|checkout|state))"
    ),
    # "{branch} clean", "{{ name }} dirty"
    re.compile(rf"\{{[a-z_]+\}}\s+{W}(?=\s*(?:[\d\"'`<]|$))"),
    # template: a name, endif, then a bare status word span with no separator
    re.compile(rf"</(?:a|b)>(?:\{{% endif %\}}.*?)?\{{% else %\}} <span[^>]*>{W}<"),
]
SKIP = re.compile(r"^\s*(//|#|\*)")


def scan(text: str, name: str) -> list[str]:
    out = []
    for i, line in enumerate(text.splitlines(), 1):
        if SKIP.match(line):
            continue
        if any(p.search(line) for p in PATTERNS):
            out.append(
                f"{name}:{i}: status facts run together, add a comma: {line.strip()[:90]}"
            )
    return out


def scan_comments(text: str, name: str, marker: str) -> list[str]:
    """Comments state what is true now, not what the code used to do."""
    out = []
    for i, line in enumerate(text.splitlines(), 1):
        at = line.find(marker)
        if at < 0 or (marker == "#" and line.startswith("#!")):
            continue
        if at > 0 and not line[:at].strip() == "" and f" {marker}" not in line[at - 1 :]:
            continue
        m = HISTORY.search(line[at:])
        if m and f"{name}:{i}" not in ALLOW:
            out.append(f"{name}:{i}: comment looks back ('{m.group(0)}'), say what holds now")
    return out


def files() -> list[Path]:
    r = subprocess.run(
        ["git", "ls-files", "-co", "--exclude-standard"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return [
        ROOT / f
        for f in r.stdout.split("\n")
        if f.endswith(EXTS + tuple(COMMENT_EXTS))
        and not any(x in f for x in EXCLUDE)
        and f != "scripts/status-words-check.py"
    ]


def self_test() -> int:
    bad = [
        'println!("on main clean");',
        'format!("{branch} dirty")',
        '</a>{% else %} <span class="muted">clean</span>',
    ]
    good = [
        'println!("on main, clean");',
        'format!("{branch}, dirty")',
        "a clean checkout",
    ]
    ok = all(scan(b, "x") for b in bad) and not any(scan(g, "x") for g in good)
    red = ["// no longer needed", "    /// used to be slow", "let a = 1; // previously 2"]
    ok = ok and all(scan_comments(c, "x", "//") for c in red)
    ok = ok and bool(scan_comments("# was changed", "x", "#"))
    ok = ok and not scan_comments("// the cache holds now\nlet s = \"no longer\";", "x", "//")
    ok = ok and not scan_comments("#!/bin/sh -e", "x", "#")
    print("self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    hits = []
    for f in files():
        text = f.read_text(errors="ignore")
        hits += scan(text, str(f.relative_to(ROOT)))
        if f.suffix in COMMENT_EXTS:
            hits += scan_comments(text, str(f.relative_to(ROOT)), COMMENT_EXTS[f.suffix])
    print("\n".join(hits) or "status-words: ok")
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main())
