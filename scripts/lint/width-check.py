#!/usr/bin/env python3
"""Design-pointer guard (stdlib only), run by `make lines`.

The shared file-length lint caps file length, comment width and code width. This adds what it
does not: no comment points at a design doc section.

  python3 scripts/lint/width-check.py              check the tree (tracked and untracked,
                                                   not ignored)
  python3 scripts/lint/width-check.py --self-test  prove the rule goes red and green on fixtures
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
COMMENT_EXTS = (".rs", ".py", ".mjs", ".js", ".sh")
EXCLUDE = ("static/vendor/", "vendor/schwarzwald/kit/")
DOC_REF = re.compile(r"§|\b[Ss]ection \d+\.\d+|\bdesign (doc|section)")
# `//` and `#` start a comment after whitespace or at line start. `://` and `#[` are not.
COMMENT = re.compile(r"(?:^|\s)(?://+!?|#)(?=\s|$)(.*)$|^\s*\*(?!/)\s?(.*)$")


def files() -> list[str]:
    out = subprocess.run(
        [
            "git",
            "ls-files",
            "-z",
            "-c",
            "--",
            *(f"*{e}" for e in COMMENT_EXTS),
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return sorted(f for f in out.split("\0") if f and not any(x in f for x in EXCLUDE))


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
    bad = doc_refs(ROOT, files())
    for b in bad:
        print(b)
    if bad:
        print(f"width-check: {len(bad)} problem(s)")
        return 1
    print("width-check: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
