#!/usr/bin/env python3
"""CONV-NO-DASH-STANDINS: no em dash, no ` -- `, no semicolon or other mark used as a dash.

The owner forbids the em dash and anything standing in for it. Three shapes fail:
  1. An em dash (U+2014) in any tracked text file. Rewrite with a period, comma, colon or
     parentheses.
  2. ` -- ` in markdown prose or in a code comment. ` -- ` collides with a CLI's own `--`
     separator, so it is a stand-in or a typo, never notation. Skipped: fenced and indented
     code, inline code spans, BEGIN/END GENERATED blocks, a section banner (`// -- name ---`).
  3. A floating mark in a comment or in prose: `,` `:` or `;` with one space before it and a
     space or the line end after (`fail ; use x`). It is what a rewrite of ` -- ` leaves behind.
     A Makefile's `#:` help marker is notation, so only the note after it is read.
     Two spaces before it is an aligned column and passes. A line with ` ? ` is notation, skipped.

Not scanned: `[scope] exclude` and `golden` paths, lock files, binary and data formats
(.json .svg .xcstrings .png .pdf), LICENSE, files under a `generated` folder, red-cases.
String literals count for shape 1 only: shapes 2 and 3 read comments and prose.

  dash-standins.py [--repo DIR]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-NO-DASH-STANDINS"
EM = "\u2014"
SKIP_SUFFIX = (".json", ".svg", ".xcstrings", ".png", ".pdf", ".lock", ".jpg", ".ico", ".woff2")
SLASH = (".rs", ".swift", ".js", ".mjs", ".ts", ".c", ".h", ".go", ".java", ".kt")
HASH = (".py", ".sh", ".toml", ".yml", ".yaml", ".rb", ".cfg", ".env")
HASH_NAMES = ("Makefile", "Dockerfile")
SPAN = re.compile(r"`[^`]*`")
FENCE = re.compile(r"^\s*(```|~~~)")
FLOATING = re.compile(r"(?:^|(?<! ) )[,:;](?= |$)")


def comment_start(line: str, slash: bool) -> int:
    """Index of the line comment's marker, -1 when none. Quote aware, so a `#` in a string is kept."""
    quote, i = None, 0
    while i < len(line):
        c = line[i]
        if quote:
            if c == "\\":
                i += 2
                continue
            if c == quote:
                quote = None
        elif c == '"' or (c == "'" and not slash):
            quote = c
        elif slash and line.startswith("//", i) and (i == 0 or line[i - 1] != ":"):
            return i
        elif not slash and c == "#" and (i == 0 or line[i - 1] in " \t"):
            return i
        i += 1
    return -1


def prose_problems(text: str) -> list[str]:
    """Shapes 2 and 3 for one piece of prose (a markdown line or a comment body)."""
    flat = SPAN.sub("x", text)
    out = []
    if " -- " in flat:
        out.append("' -- ' stands in for a dash")
    if " ? " not in flat and FLOATING.search(flat):
        out.append("a floating , : or ; stands in for a dash")
    return out


def scan_markdown(rel: str, text: str) -> list[str]:
    hits, fenced, generated = [], False, False
    for n, line in enumerate(text.splitlines(), 1):
        if FENCE.match(line):
            fenced = not fenced
            continue
        if "BEGIN GENERATED" in line:
            generated = True
        if "END GENERATED" in line:
            generated = False
            continue
        if fenced or generated or line.lstrip().startswith("|"):
            continue
        if line.startswith("    ") and not line.strip().startswith(("-", "*", ">")):
            continue
        hits += [f"{rel}:{n}: {p}: {line.strip()[:90]}" for p in prose_problems(line)]
    return hits


def scan_code(rel: str, text: str, slash: bool, make: bool = False) -> list[str]:
    hits = []
    for n, line in enumerate(text.splitlines(), 1):
        if n == 1 and line.startswith("#!"):
            continue
        at = comment_start(line, slash)
        if at < 0:
            continue
        body = line[at:].lstrip("/#!").strip()
        if make and line[at:].startswith("#:"):
            body = body[1:].strip()  # the `#:` help marker is notation, its note is prose
        if re.match(r"^--\s", body) and ("---" in body or body.endswith("--")):
            continue  # a section banner
        hits += [f"{rel}:{n}: {p}: {line.strip()[:90]}" for p in prose_problems(" " + body)]
    return hits


def scan(rel: str, text: str) -> list[str]:
    hits = [f"{rel}:{n}: em dash: {ln.strip()[:90]}" for n, ln in enumerate(text.splitlines(), 1) if EM in ln]
    path = Path(rel)
    if rel.endswith(".md"):
        hits += scan_markdown(rel, text)
    elif path.suffix in SLASH:
        hits += scan_code(rel, text, True)
    elif path.suffix in HASH or path.name in HASH_NAMES:
        hits += scan_code(rel, text, False, path.name == "Makefile")
    return hits


def self_test() -> bool:
    bad = ["a \u2014 b\n", "x -- y\n", "fail ; use x\n"]
    ok = ["use `a -- b` here\n", "name   : meaning\n", "```\nrun -- args\n```\n", "plain text, fine; yes\n"]
    code_bad = ["# a -- b\n", "// fail : use x\n"]
    code_ok = ["x = 'a -- b'  # fine\n", "// -- reads ----\n"]
    good = all(scan("t.md", t) for t in bad) and not any(scan("t.md", t) for t in ok)
    good = good and all(scan("t.py" if t[0] == "#" else "t.rs", t) for t in code_bad)
    good = good and not any(scan("t.py" if t[0] == "x" else "t.rs", t) for t in code_ok)
    if not good:
        print(f"FAIL [{RULE}] self-test: a planted stand-in must fail and clean text must pass", file=sys.stderr)
    return good


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    scope = cfg["scope"]
    names = [
        r for r in config.tracked(root, scope["exclude"])
        if not r.endswith(SKIP_SUFFIX) and "red-cases" not in Path(r).parts and Path(r).name != "LICENSE"
        and "generated" not in (p.lower() for p in Path(r).parts) and not config.matches(r, scope["golden"])
    ]
    hits, seen = [], 0
    for rel in names:
        try:
            text = (root / rel).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        seen += 1
        hits += scan(rel, text)
    if seen < scope["min_files"]:
        print(f"FAIL [{RULE}] only {seen} files scanned, the census is broken")
        return 1
    if hits:
        print(f"FAIL [{RULE}] {len(hits)} dash stand-in(s)")
        for h in hits[:60]:
            print(f"  {h}")
        print("  Rewrite with a period, comma, colon or parentheses. Notation goes in backticks.")
        return 1
    print(f"PASS [{RULE}] no dash stand-in in {seen} files")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
