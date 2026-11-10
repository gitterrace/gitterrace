#!/usr/bin/env python3
"""Fail when a markdown.css selector is not scoped to .markdown-body, .code or .diff.

Rendered documents share the page with the app, so an unscoped class there (.seg once)
restyles app widgets. Run by the lint runner as the css-scope hook.

  python3 scripts/lint/css-scope-check.py
"""

from __future__ import annotations

import re
import sys
from collections.abc import Iterator
from pathlib import Path

CSS = Path(__file__).resolve().parents[2] / "crates/gitterrace-web/static/markdown.css"
SCOPED = re.compile(
    r"^(\.markdown-body|\.code|\.diff|:is\(\.markdown-body,\.code,\.diff\))(?![\w-])"
)


def selectors(text: str) -> Iterator[str]:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    text = re.sub(r"url\([^)]*\)", "url()", text)
    for prelude in re.findall(r"(?:^|[};])\s*([^{};]+)\{", text):
        prelude = prelude.strip()
        if prelude.startswith("@"):
            continue
        depth, cur = 0, ""
        for ch in prelude:
            depth += ch in "(["
            depth -= ch in ")]"
            if ch == "," and depth == 0:
                yield cur.strip()
                cur = ""
            else:
                cur += ch
        yield cur.strip()


def main() -> int:
    bad = [s for s in selectors(CSS.read_text()) if not SCOPED.match(s)]
    for s in bad:
        print(
            f"markdown.css: selector not scoped to .markdown-body, .code or .diff: {s}"
        )
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
