#!/usr/bin/env python3
"""The synced kit copy names no commit of the kit it came from.

That commit lives in another repository, so it never resolves here. The check reads the
copy's provenance: manifest.json must leave its source commit null, and no top-level text
file of the copy may carry a word of 7 to 40 hex digits with a digit and a letter in it. The
lint fixtures deeper in the copy carry made-up ids on purpose.

  python3 scripts/lint/kit-source-check.py [--self-test]
"""

from __future__ import annotations

import json
import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
KIT = Path("vendor/schwarzwald/kit")
HEX = re.compile(r"\b(?=[0-9a-f]*[a-f])(?=[0-9a-f]*[0-9])[0-9a-f]{7,40}\b")


def problems(root: Path) -> list[str]:
    kit = root / KIT
    out = []
    manifest = json.loads((kit / "manifest.json").read_text())
    if manifest.get("schwarzwald") is not None:
        out.append(f"{KIT}/manifest.json: names the source commit, sync with --no-sha")
    for md in sorted(p for p in kit.iterdir() if p.suffix in (".md", ".txt")):
        for n, line in enumerate(md.read_text().splitlines(), 1):
            for word in HEX.findall(line):
                out.append(f"{md.relative_to(root)}:{n}: names commit-like id {word}")
    return out


def self_test() -> int:
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        kit = root / KIT
        kit.mkdir(parents=True)
        (kit / "manifest.json").write_text('{"schwarzwald": null}')
        (kit / "README.md").write_text("- From: schwarzwald, synced 2026-11-12\n")
        assert not problems(root), problems(root)
        (kit / "manifest.json").write_text('{"schwarzwald": "abc1234def"}')
        assert len(problems(root)) == 1
        (kit / "manifest.json").write_text('{"schwarzwald": null}')
        (kit / "NOTES.md").write_text("built at commit `abc1234de`\n")
        assert len(problems(root)) == 1
        (kit / "NOTES.md").unlink()
        (kit / "lint").mkdir()
        (kit / "lint" / "case.md").write_text("fixture id 0123abcd99\n")
        assert not problems(root), problems(root)
    print("PASS [CONV-KIT-SOURCE] self-test")
    return 0


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    found = problems(ROOT)
    for p in found:
        print(p)
    if found:
        print(f"FAIL [CONV-KIT-SOURCE] {len(found)} commit ids in the kit copy")
        return 1
    print("PASS [CONV-KIT-SOURCE] the kit copy names no source commit")
    return 0


if __name__ == "__main__":
    sys.exit(main())
