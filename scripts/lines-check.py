#!/usr/bin/env python3
"""Source hygiene guard (stdlib only), run by `make lines`, part of `make check`.

1. No source file is longer than 600 lines.
2. No code or test comment points at a design doc section or carries a label, process word,
   id or name from shared/schwarzwald/rules/deny.toml and the local .personal-denylist.

  python3 scripts/lines-check.py              check the tree (tracked and untracked, not ignored)
  python3 scripts/lines-check.py --self-test  prove both rules go red and green on fixtures
"""

from __future__ import annotations

import importlib
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    tomllib = importlib.import_module("tomllib")
except (
    ModuleNotFoundError
):  # python older than 3.11: run again under the first newer one
    import os
    import shutil

    for _c in ("python3.15", "python3.14", "python3.13", "python3.12", "python3.11"):
        _p = shutil.which(_c)
        if _p:
            os.execv(_p, [_p, *sys.argv])
    sys.exit("lines-check: needs python 3.11 or newer (tomllib)")

ROOT = Path(__file__).resolve().parent.parent
MAX_LINES = 600
# rustfmt max_width. rustfmt never wraps a comment, so a wide one is caught here.
MAX_WIDTH = 100
LENGTH_GLOBS = ("*.rs", "*.py", "*.mjs", "*.js", "*.sh", "*.css", "*.html")
COMMENT_EXTS = (".rs", ".py", ".mjs", ".js", ".sh")
# Vendored third-party files and the synced theme copy are not ours to split or reword.
EXCLUDE = ("static/vendor/", "shared/schwarzwald/")

# A pointer at a design doc section. The label, plan, process and id shapes are not here: they
# come from shared/schwarzwald/rules/deny.toml, so every product refuses the same words.
DOC_REF = re.compile(r"§|\b[Ss]ection \d+\.\d+|\bdesign (doc|section)")
DENY_TABLES = ("task_id", "plan", "label", "process")
DENY_FILE = ROOT / "shared" / "schwarzwald" / "rules" / "deny.toml"
PRODUCT = "gitterrace"


def deny_patterns() -> tuple[list[re.Pattern[str]], list[re.Pattern[str]]]:
    """The shared deny patterns for comments, and the English the id patterns skip."""
    raw = tomllib.loads(DENY_FILE.read_text(encoding="utf-8"))
    pats, skip = [], []
    for t in DENY_TABLES:
        pats += [
            re.compile(p.replace("{product}", re.escape(PRODUCT)))
            for p in raw[t]["patterns"]
        ]
        if "not_id" in raw[t]:
            skip.append(re.compile(raw[t]["not_id"]))
    return pats, skip


# The one local, gitignored list (the shared lints read it too): a bare six-hex line is a real
# id, a plain word is a person's name, refused in comments. Regex lines are the other lints'.
LOCAL = ROOT / ".personal-denylist"


def local_lists() -> tuple[list[str], set[str]]:
    """(names, ids) of the local denylist. Absent: both empty, and the gate says so."""
    try:
        lines = LOCAL.read_text().splitlines()
    except OSError:
        print("denylist absent: generic patterns only")
        return [], set()
    items = [n.strip() for n in lines if n.strip() and not n.strip().startswith("#")]
    ids = {n for n in items if re.fullmatch(r"[0-9a-f]{6}", n)}
    names = [
        re.escape(n)
        for n in items
        if re.fullmatch(r"[A-Za-z][\w-]*", n) and n not in ids
    ]
    return names, ids


def person_pattern(names: list[str]) -> re.Pattern[str] | None:
    """A name counts when capitalised, or lowercase before a colon, after @, `by`, `ask`, `told`."""
    if not names:
        return None
    alt = "|".join(names)
    cap = "|".join(n[0].upper() + n[1:] for n in names)
    return re.compile(rf"\b({cap})\b|\b({alt}):|(?:@|\bby |\bask |\btold )({alt})\b")


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


# Wide lines that predate the width rule: strings and macro bodies rustfmt leaves alone. A
# count may only go down: a new wide line fails, and a fixed one must lower its count here.
WIDE_ALLOWED: dict[str, int] = json.loads(
    (Path(__file__).resolve().parent / "guards" / "wide-allowed.json").read_text()
)["wide_allowed"]

RAW_OPEN = re.compile(r'r(#+)"')
URL = re.compile(r"\bhttps?://\S{40,}")


def wide_lines(text: str) -> list[int]:
    """Lines past MAX_WIDTH, except inside a raw string (fixture bytes such as SVG or TOML,
    where a wrap would change the content) and lines carrying one long URL."""
    out, close = [], None
    for i, line in enumerate(text.splitlines(), 1):
        inside = close is not None
        if close is not None and close in line:
            close = None
        else:
            for m in RAW_OPEN.finditer(line):
                end = '"' + m.group(1)
                if end not in line[m.end() :]:
                    close = end
                    break
        raw = inside or close is not None
        if len(line) > MAX_WIDTH and not raw and not URL.search(line):
            out.append(i)
    return out


def too_wide(
    root: Path, names: list[str], allowed: dict[str, int] | None = None
) -> list[str]:
    allowed = WIDE_ALLOWED if allowed is None else allowed
    bad = []
    for n in names:
        if not n.endswith(".rs"):
            continue
        try:
            text = (root / n).read_text(errors="replace")
        except OSError:
            continue
        rows = wide_lines(text)
        cap = allowed.get(n, 0)
        lines = text.splitlines()
        if len(rows) > cap:
            for i in rows:
                if cap == 0 or lines[i - 1].lstrip().startswith("//"):
                    bad.append(
                        f"{n}:{i}: {len(lines[i - 1])} columns, limit {MAX_WIDTH}"
                    )
            if cap:
                bad.append(
                    f"{n}: {len(rows)} wide lines, allowed {cap} (wrap the new one)"
                )
        elif len(rows) < cap:
            bad.append(
                f"{n}: {len(rows)} wide lines, allowed {cap}: lower WIDE_ALLOWED"
            )
    return bad


def label_hits(
    root: Path,
    names: list[str],
    extra: list[str] | None = None,
    ids: set[str] | None = None,
) -> list[str]:
    if extra is None:
        extra, local_ids = local_lists()
        ids = local_ids if ids is None else ids
    person = person_pattern(extra)
    ids = ids or set()
    pats, skip = deny_patterns()
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
            plain = text
            for k in skip:
                plain = k.sub("", plain)
            hit = (
                DOC_REF.search(text)
                or any(p.search(plain) for p in pats)
                or (person and person.search(text))
                or any(i in text for i in ids)
            )
            if hit:
                bad.append(f"{n}:{i}: process label in a comment: {text.strip()[:70]}")
    return bad


def run(root: Path, names: list[str]) -> list[str]:
    stale = [
        f"wide-allowed.json: entry {n}: the list stays empty, wrap the line"
        for n in WIDE_ALLOWED
    ]
    return (
        stale + too_long(root, names) + too_wide(root, names) + label_hits(root, names)
    )


def self_test() -> None:
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        (root / "ok600.rs").write_text("fn a() {}\n" * 600)
        (root / "big601.rs").write_text("fn a() {}\n" * 601)
        assert too_long(root, ["ok600.rs"]) == [], "600 lines must pass"
        assert len(too_long(root, ["big601.rs"])) == 1, "601 lines must fail"
        ok = "    /// " + "x" * 92 + "\n//! " + "y" * 96 + "\n"
        (root / "w.rs").write_text(ok + 'let s = r#"\n' + "z" * 140 + '\n"#;\n')
        (root / "w2.rs").write_text("    // " + "x" * 94 + "\n")
        (root / "w3.rs").write_text('    let s = "' + "x" * 95 + '";\n')
        assert too_wide(root, ["w.rs"], {}) == [], "100 columns and raw fixtures pass"
        assert len(too_wide(root, ["w2.rs"], {})) == 1, "a 101 column comment fails"
        assert len(too_wide(root, ["w3.rs"], {})) == 1, "a 101 column string fails"
        assert too_wide(root, ["w3.rs"], {"w3.rs": 1}) == [], "an allowed count passes"
        assert too_wide(root, ["w.rs"], {"w.rs": 1}), (
            "an allowance above the count fails"
        )
        green = [
            'let s = "%C3%A9"; // encodes e acute',
            'let u = "http://x"; // plain',
            "# a plain word in prose",
            "// the dashboard and the keyboard",
            "#[derive(Debug)]",
            "// the case_name field",
        ]
        red = [
            "// see design doc 4",
            "// per section 3.2",
            "// signed off as K3",
            "// lane Q owns this",
            "// signed off as N2",
            "// \u00a7 4",
            "// the coordinator records it",
            "// done in wave 2",
            "// see task 1a2b3c",
        ]
        (root / "g.rs").write_text("\n".join(green) + "\n")
        assert label_hits(root, ["g.rs"]) == [], label_hits(root, ["g.rs"])
        # The optional local name list is the only place a person name can come from.
        (root / "n.rs").write_text("// ask Orca\n// orca: later\n// the orca swims\n")
        assert label_hits(root, ["n.rs"], []) == [], "no list, no name rule"
        assert len(label_hits(root, ["n.rs"], ["orca"])) == 2, "listed name must flag"
        (root / "i.rs").write_text("// ref abc123 here\n")
        assert label_hits(root, ["i.rs"], [], set()) == [], "no list, no id rule"
        assert label_hits(root, ["i.rs"], [], {"abc123"}), "a listed id must flag"
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
