#!/usr/bin/env python3
"""Source width and design-pointer guard (stdlib only), run by `make lines`.

The shared file-length lint caps file length and comment width. This adds what it does not:

1. No Rust line is wider than rustfmt's max_width. rustfmt leaves strings, macro bodies and
   comments alone, so a wide one is caught here. Counts in guards/wide-allowed.json only fall.
2. No comment points at a design doc section.

  python3 scripts/width-check.py              check the tree (tracked and untracked, not ignored)
  python3 scripts/width-check.py --self-test  prove both rules go red and green on fixtures
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MAX_WIDTH = 100
COMMENT_EXTS = (".rs", ".py", ".mjs", ".js", ".sh")
EXCLUDE = ("static/vendor/", "shared/schwarzwald/")
DOC_REF = re.compile(r"§|\b[Ss]ection \d+\.\d+|\bdesign (doc|section)")
# `//` and `#` start a comment after whitespace or at line start. `://` and `#[` are not.
COMMENT = re.compile(r"(?:^|\s)(?://+!?|#)(?=\s|$)(.*)$|^\s*\*(?!/)\s?(.*)$")
RAW_OPEN = re.compile(r'r(#+)"')
URL = re.compile(r"\bhttps?://\S{40,}")


def files() -> list[str]:
    out = subprocess.run(
        [
            "git",
            "ls-files",
            "-z",
            "-co",
            "--exclude-standard",
            "--",
            *(f"*{e}" for e in COMMENT_EXTS),
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return sorted(f for f in out.split("\0") if f and not any(x in f for x in EXCLUDE))


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


def too_wide(root: Path, names: list[str], allowed: dict[str, int]) -> list[str]:
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
                f"{n}: {len(rows)} wide lines, allowed {cap}: lower wide-allowed.json"
            )
    return bad


def doc_refs(root: Path, names: list[str]) -> list[str]:
    bad = []
    for n in names:
        try:
            lines = (root / n).read_text(errors="replace").splitlines()
        except OSError:
            continue
        for i, line in enumerate(lines, 1):
            m = COMMENT.search(line)
            text = (
                (m.group(1) if m.group(1) is not None else m.group(2) or "")
                if m
                else ""
            )
            if DOC_REF.search(text):
                bad.append(f"{n}:{i}: design pointer in a comment: {text.strip()[:70]}")
    return bad


def self_test() -> None:
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
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
        (root / "g.rs").write_text(
            '// the dashboard\n#[derive(Debug)]\nlet s = "design doc";\n'
        )
        assert doc_refs(root, ["g.rs"]) == [], doc_refs(root, ["g.rs"])
        for i, r in enumerate(
            ["// see design doc 4", "// per section 3.2", "// \u00a7 4"]
        ):
            (root / f"r{i}.rs").write_text(r + "\n")
            assert doc_refs(root, [f"r{i}.rs"]), f"should flag: {r}"
    print("width-check self-test: ok")


def main() -> int:
    if "--self-test" in sys.argv:
        self_test()
        return 0
    allowed = json.loads(
        (ROOT / "scripts" / "guards" / "wide-allowed.json").read_text()
    )["wide_allowed"]
    names = files()
    bad = too_wide(ROOT, names, allowed) + doc_refs(ROOT, names)
    for b in bad:
        print(b)
    if bad:
        print(f"width-check: {len(bad)} problem(s)")
        return 1
    print("width-check: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
