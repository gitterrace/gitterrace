#!/usr/bin/env python3
"""The newest changelog section, before its tag exists, is exactly what git-cliff makes of it.

The shared CONV-CHANGELOG lint compares tagged sections only, so a hand edit to the section of
a release about to be tagged shows first in the release workflow, after the tag is pushed. This
check closes that window: when the top `## [x.y.z]` has no `v<x.y.z>` tag yet, it must equal
`git-cliff --unreleased --tag v<x.y.z> --strip header`, and the generated Unreleased block must
sit above it, where `make changelog` and the tagged releases keep it.

  python3 scripts/lint/changelog-check.py              check the tree
  python3 scripts/lint/changelog-check.py --self-test  prove it goes red on planted cases
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FILE = ROOT / "CHANGELOG.md"
MARKER = "<!-- BEGIN GENERATED: unreleased -->"
HEAD = re.compile(r"^## \[(\d[^\]]*)\]", re.M)


def top(text: str) -> tuple[str | None, str]:
    """The newest version heading and its section, up to the next one."""
    parts = re.split(r"^(?=## \[)", text, flags=re.M)
    for part in parts:
        m = HEAD.match(part)
        if m:
            return m.group(1), part.strip()
    return None, ""


def problems(text: str, generated: str, version: str | None) -> list[str]:
    out = []
    if version is None:
        return out
    marker_at, section_at = text.find(MARKER), text.find(f"## [{version}]")
    if marker_at < 0 or marker_at > section_at:
        out.append(f"the Unreleased block is not above [{version}]")
    _, have = top(text)
    _, want = top(generated)
    if have != want:
        out.append(f"[{version}] differs from git-cliff; run make changelog, never edit it")
    return out


def tagged(version: str) -> bool:
    ref = f"refs/tags/v{version}"
    return (
        subprocess.run(
            ["git", "-C", str(ROOT), "rev-parse", "-q", "--verify", ref],
            capture_output=True,
        ).returncode
        == 0
    )


def self_test() -> bool:
    sec = "## [0.2.0] - 2026-01-02\n\n### Fixed\n\n- a: b\n"
    good = f"# Changelog\n\n{MARKER}\n<!-- END -->\n\n{sec}\n## [0.1.0] - 2026-01-01\n"
    edited = good.replace("- a: b", "- a: c")
    moved = f"# Changelog\n\n{sec}{MARKER}\n<!-- END -->\n\n## [0.1.0] - 2026-01-01\n"
    return (
        not problems(good, sec, "0.2.0")
        and bool(problems(edited, sec, "0.2.0"))
        and bool(problems(moved, sec, "0.2.0"))
    )


def main(argv: list[str]) -> int:
    if argv[:1] == ["--self-test"]:
        ok = self_test()
        print("changelog-check self-test: " + ("ok" if ok else "FAILED"))
        return 0 if ok else 1
    text = FILE.read_text()
    version, _ = top(text)
    if version is None or tagged(version):
        print("changelog-check: no untagged release section, nothing to compare")
        return 0
    # git-cliff reads the release from the commits since the last tag: a copy without that
    # history (a fixture repository, a shallow clone) has nothing to compare against.
    if not any(tagged(v) for v in HEAD.findall(text) if v != version):
        print("changelog-check: no earlier release tag in this history, nothing to compare")
        return 0
    if not shutil.which("git-cliff"):
        print("FAIL changelog-check: git-cliff is not installed", file=sys.stderr)
        return 1
    gen = subprocess.run(
        ["git-cliff", "--unreleased", "--tag", f"v{version}", "--strip", "header"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    bad = problems(text, gen, version)
    for b in bad:
        print(f"FAIL changelog-check: {b}", file=sys.stderr)
    if not bad:
        print(f"changelog-check: [{version}] is what git-cliff makes, ready to tag")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
