#!/usr/bin/env python3
"""Changelog guard (stdlib only), run by `make docs-check`: CHANGELOG.md is what git-cliff
generates from the tagged history, so a hand-edited line fails. Only released sections are
compared. The Unreleased section and a section for a version not tagged yet are left out,
because they are written by `make changelog` just before the tag.

  python3 scripts/changelog-check.py
  python3 scripts/changelog-check.py --self-test
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
from pathlib import Path

WANT = "2.14.2"  # keep equal to `want` in scripts/changelog-gen.sh
ROOT = Path(__file__).resolve().parent.parent


def sections(text: str, tags: set[str]) -> dict[str, str]:
    """Map each `## [x.y.z]` heading with a tag to its body, header and other sections out."""
    out: dict[str, str] = {}
    parts = re.split(r"^(?=## \[)", text, flags=re.M)
    for part in parts:
        m = re.match(r"## \[(\d[^\]]*)\]", part)
        if m and f"v{m.group(1)}" in tags:
            out[m.group(1)] = part.strip()
    return out


def tags() -> set[str]:
    out = subprocess.run(
        ["git", "tag", "--list", "v[0-9]*"],
        cwd=ROOT,
        capture_output=True,
        check=True,
        text=True,
    )
    return set(out.stdout.split())


def diff(file_text: str, generated: str, known: set[str]) -> list[str]:
    have, want = sections(file_text, known), sections(generated, known)
    bad = []
    for v in sorted(set(have) | set(want)):
        if have.get(v) != want.get(v):
            bad.append(
                f"CHANGELOG.md [{v}] differs from `git cliff` output; regenerate it, never edit it by hand"
            )
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        gen = "# Changelog\n\n## [0.1.0] - 2026-01-01\n\n### Added\n\n- A: b\n"
        assert not diff(gen, gen, {"v0.1.0"})
        assert diff(gen.replace("A: b", "A: c"), gen, {"v0.1.0"})
        assert not diff(gen + "\n## [Unreleased]\n- x\n", gen, {"v0.1.0"})
        print("changelog-check self-test ok")
        return 0
    known = tags()
    if not known:
        print("changelog-check: no release tags yet, nothing to compare")
        return 0
    if not shutil.which("git-cliff"):
        print(
            f"changelog-check: git-cliff {WANT} is not installed (brew install git-cliff)",
            file=sys.stderr,
        )
        return 1
    gen = subprocess.run(
        ["git-cliff"], cwd=ROOT, capture_output=True, check=True, text=True
    ).stdout
    bad = diff((ROOT / "CHANGELOG.md").read_text(encoding="utf-8"), gen, known)
    for b in bad:
        print(b, file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
