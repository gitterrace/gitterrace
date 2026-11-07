#!/usr/bin/env python3
"""CONV-CHANGELOG: the changelog is what git-cliff generates from the tagged history.

A hand-edited released line fails. Only released sections are compared. The Unreleased section
and a section for a version not tagged yet are left out, because `tools/changelog-gen.sh`
writes them just before the tag. Settings come from lint.toml `[changelog]`: `git_cliff` pins
the version ("" turns the check off), `file` and `tag_prefix` name the file and the tags.
A pinned check fails when the file is missing, repeats a section, or git-cliff is absent or of
another version, because each of those reads like a pass otherwise.

  changelog.py [--repo DIR]
  changelog.py --self-test
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-CHANGELOG"


def heading_versions(text: str) -> list[str]:
    return re.findall(r"^## \[(\d[^\]]*)\]", text, flags=re.M)


def sections(text: str, tags: set[str], prefix: str) -> dict[str, str]:
    """Map each `## [x.y.z]` heading that has a tag to its body, header and other sections out."""
    out: dict[str, str] = {}
    for part in re.split(r"^(?=## \[)", text, flags=re.M):
        m = re.match(r"## \[(\d[^\]]*)\]", part)
        if m and f"{prefix}{m.group(1)}" in tags:
            out[m.group(1)] = part.strip()
    return out


def diff(file_text: str, generated: str, known: set[str], prefix: str, name: str) -> list[str]:
    have, want = sections(file_text, known, prefix), sections(generated, known, prefix)
    return [
        f"{name} [{v}] differs from `git cliff` output; regenerate it, never edit it by hand"
        for v in sorted(set(have) | set(want))
        if have.get(v) != want.get(v)
    ]


def self_test() -> bool:
    gen = "# Changelog\n\n## [0.1.0] - 2026-01-01\n\n### Added\n\n- A: b\n"
    t = {"v0.1.0"}
    ok = (
        not diff(gen, gen, t, "v", "C")
        and bool(diff(gen.replace("A: b", "A: c"), gen, t, "v", "C"))
        and not diff(gen + "\n## [Unreleased]\n- x\n", gen, t, "v", "C")
        and heading_versions(gen + gen) == ["0.1.0", "0.1.0"]
    )
    if not ok:
        print(f"FAIL [{RULE}] self-test", file=sys.stderr)
    return ok


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        if not self_test():
            return 1
        print("changelog self-test ok")
        return 0
    root, _ = config.take_repo(argv)
    s = config.load(root)["changelog"]
    want, name, prefix = s["git_cliff"], s["file"], s["tag_prefix"]
    if not want:
        print(f"{RULE}: [changelog] git_cliff is empty, no changelog check")
        return 0
    path = root / name
    if not path.exists():
        print(f"FAIL [{RULE}] {name} is missing", file=sys.stderr)
        return 1
    text = path.read_text(encoding="utf-8")
    versions = heading_versions(text)
    dupes = sorted({v for v in versions if versions.count(v) > 1})
    if dupes:
        print(f"FAIL [{RULE}] {name} repeats the section(s): {', '.join(dupes)}", file=sys.stderr)
        return 1
    out = subprocess.run(
        ["git", "-C", str(root), "tag", "--list", f"{prefix}[0-9]*"], capture_output=True, text=True, check=True
    )
    known = set(out.stdout.split())
    if not known:
        print(f"{RULE}: no release tags yet, nothing to compare")
        return 0
    if not shutil.which("git-cliff"):
        print(f"FAIL [{RULE}] git-cliff {want} is not installed (brew install git-cliff)", file=sys.stderr)
        return 1
    have = subprocess.run(["git-cliff", "--version"], capture_output=True, text=True).stdout.split()[-1:]
    if have != [want]:
        print(f"FAIL [{RULE}] git-cliff {' '.join(have)} found, {want} pinned in lint.toml", file=sys.stderr)
        return 1
    gen = subprocess.run(["git-cliff"], cwd=root, capture_output=True, check=True, text=True).stdout
    bad = diff(text, gen, known, prefix, name)
    for b in bad:
        print(f"FAIL [{RULE}] {b}", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
