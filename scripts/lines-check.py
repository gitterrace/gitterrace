#!/usr/bin/env python3
"""Source hygiene guard (stdlib only), run by `make lines`, part of `make check`.

1. No source file is longer than 600 lines.
2. No code or test comment points at a design doc section or a process label.

  python3 scripts/lines-check.py              check the tree (tracked and untracked, not ignored)
  python3 scripts/lines-check.py --self-test  prove both rules go red and green on fixtures
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MAX_LINES = 600
LENGTH_GLOBS = ("*.rs", "*.py", "*.mjs", "*.js", "*.sh", "*.css", "*.html")
COMMENT_EXTS = (".rs", ".py", ".mjs", ".js", ".sh")
# Vendored third-party files and the synced theme copy are not ours to split or reword.
EXCLUDE = ("static/vendor/", "theme/schwarzwald/")

# Process labels in comments. Percent-encodings such as %C3 sit in code, not in comments, and
# the whole-word match keeps C3 inside longer tokens out.
LABELS = re.compile(
    r"§"
    r"|\b[Ss]ection \d+\.\d+"
    r"|\bdesign (doc|section)"
    r"|\b[Ll]ane [A-K]\b"
    r"|\b(C[1-3]|K[1-5]|N[1-4])\b"
)
# An agent name counts as a person when capitalised, or lowercase before a colon. A lowercase
# `swan` or `rabbit` in plain prose stays legal.
AGENTS = "snake|t-rex|squid|swan|tiger|turkey|whale|spider|parrot|jaguar|rabbit"
PERSON = re.compile(
    rf"\b({'|'.join(a.capitalize() if '-' not in a else 'T-[Rr]ex' for a in AGENTS.split('|'))})\b"
    rf"|\b({AGENTS}):"
    rf"|(?:@|\bby |\bask |\btold )({AGENTS})\b"
)
# `//` and `#` start a comment after whitespace or at line start. `://` and `#[` are not.
COMMENT = re.compile(r"(?:^|\s)(?://+!?|#)(?=\s|$)(.*)$|^\s*\*(?!/)\s?(.*)$")


def files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z", "-co", "--exclude-standard", "--", *LENGTH_GLOBS],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return sorted(f for f in out.split("\0") if f and not any(x in f for x in EXCLUDE))


def too_long(root: Path, names: list[str]) -> list[str]:
    bad = []
    for n in names:
        try:
            k = len((root / n).read_text(errors="replace").splitlines())
        except OSError:
            continue
        if k > MAX_LINES:
            bad.append(f"{n}: {k} lines, limit {MAX_LINES}")
    return bad


def label_hits(root: Path, names: list[str]) -> list[str]:
    bad = []
    for n in names:
        if not n.endswith(COMMENT_EXTS) or n == "scripts/lines-check.py":
            continue
        try:
            lines = (root / n).read_text(errors="replace").splitlines()
        except OSError:
            continue
        for i, line in enumerate(lines, 1):
            m = COMMENT.search(line)
            if not m:
                continue
            text = m.group(1) if m.group(1) is not None else m.group(2) or ""
            if LABELS.search(text) or PERSON.search(text):
                bad.append(f"{n}:{i}: process label in a comment: {text.strip()[:70]}")
    return bad


def run(root: Path, names: list[str]) -> list[str]:
    return too_long(root, names) + label_hits(root, names)


def self_test() -> None:
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        (root / "ok600.rs").write_text("fn a() {}\n" * 600)
        (root / "big601.rs").write_text("fn a() {}\n" * 601)
        assert too_long(root, ["ok600.rs"]) == [], "600 lines must pass"
        assert len(too_long(root, ["big601.rs"])) == 1, "601 lines must fail"
        green = [
            'let s = "%C3%A9"; // encodes e acute',
            'let u = "http://x"; // plain',
            "# a rabbit in the garden",
            "#[derive(Debug)]",
            "// the snake_case name",
        ]
        red = [
            "// see design doc 4",
            "// per section 3.2",
            "// signed off as K3",
            "# Squid wants this",
            "// ask Tiger",
            "// \u00a7 4",
        ]
        (root / "g.rs").write_text("\n".join(green) + "\n")
        assert label_hits(root, ["g.rs"]) == [], label_hits(root, ["g.rs"])
        for i, r in enumerate(red):
            (root / f"r{i}.rs").write_text(r + "\n")
            assert label_hits(root, [f"r{i}.rs"]), f"should flag: {r}"
    print("lines-check self-test: ok")


def main() -> int:
    if "--self-test" in sys.argv:
        self_test()
        return 0
    bad = run(ROOT, files())
    for b in bad:
        print(b)
    if bad:
        print(f"lines-check: {len(bad)} problem(s)")
        return 1
    print("lines-check: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
