#!/usr/bin/env python3
"""A release never takes its gate from the landing cache.

land.sh keeps a green gate by tree and toolchain, so a second landing of the same tree skips
it. That is right for a landing and wrong for a release: the advisory database moved since,
and a tag must be judged today. Two contracts, both red here:

1. Every call of land.sh or land-run.sh in a release path passes `--no-cache`. The release
   paths are everything under scripts/release/, the release workflows and the Makefile targets
   named `release*`. A call is a non-comment line naming land.sh or land-run.sh, with its
   backslash continuations. A new release script is covered by its folder, a flag dropped in a
   refactor is red on the line that lost it.
2. Every landing gate command that runs `cargo deny` (directly or as `make deny`) is listed in
   schwarzwald/config/land.toml `[landing] fresh`, so land.sh never caches a gate holding it.

  python3 scripts/lint/land-cache-check.py              check the tree
  python3 scripts/lint/land-cache-check.py --self-test  red and green on fixtures
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parents[2]
CALL = re.compile(r"\bland(?:-run)?\.sh\b")
TARGET = re.compile(r"^(release[\w-]*):")


def calls(text: str) -> list[tuple[int, str]]:
    """The (line number, joined command) of each non-comment call of land.sh."""
    out, lines, i = [], text.splitlines(), 0
    while i < len(lines):
        start, cmd = i + 1, lines[i]
        while cmd.rstrip().endswith("\\") and i + 1 < len(lines):
            i += 1
            cmd = cmd.rstrip()[:-1] + " " + lines[i].strip()
        i += 1
        if not cmd.lstrip().startswith("#") and CALL.search(cmd):
            out.append((start, cmd))
    return out


def release_makefile(text: str) -> str:
    """Only the recipes of the `release*` targets, other lines blanked to keep numbers."""
    keep, on = [], False
    for line in text.splitlines():
        if line and not line.startswith(("\t", " ", "#")):
            on = bool(TARGET.match(line))
        keep.append(line if on else "")
    return "\n".join(keep)


def sources(repo: Path) -> list[tuple[str, str]]:
    found = []
    for f in sorted((repo / "scripts/release").rglob("*")):
        if f.is_file() and f.suffix in {".sh", ".py", ".mjs", ".toml", ""}:
            found.append((str(f.relative_to(repo)), f.read_text(encoding="utf-8")))
    for f in sorted((repo / ".github").rglob("release*.y*ml")):
        found.append((str(f.relative_to(repo)), f.read_text(encoding="utf-8")))
    mk = repo / "Makefile"
    if mk.is_file():
        found.append(("Makefile", release_makefile(mk.read_text(encoding="utf-8"))))
    return found


def problems(repo: Path) -> list[str]:
    bad = []
    for name, text in sources(repo):
        for n, cmd in calls(text):
            if "--no-cache" not in cmd:
                bad.append(f"{name}:{n}: a release path calls land.sh without --no-cache")
    toml = repo / "schwarzwald/config/land.toml"
    if toml.is_file():
        land = tomllib.loads(toml.read_text(encoding="utf-8")).get("landing", {})
        fresh = set(land.get("fresh", []))
        for cmd in land.get("gate", []):
            if re.search(r"\bcargo deny\b|\bmake\b.*\bdeny\b", cmd) and cmd not in fresh:
                bad.append(f"land.toml: gate command `{cmd}` must be listed in [landing] fresh")
    return bad


def self_test() -> int:
    # Built from parts: the names lint reads a literal path as a reference to a real file.
    REL = "scripts/release/" + "fixture.sh"
    land = '[landing]\ngate = ["make deny"]\nfresh = ["make deny"]\n'
    cases = [
        (
            "a release script without the flag",
            {REL: "bash land.sh r c s\n"},
            land,
            True,
        ),
        (
            "a land-run call without the flag",
            {REL: "bash scripts/dev/land-run.sh r c s\n"},
            land,
            True,
        ),
        (
            "the flag on a continuation line",
            {REL: "bash land.sh \\\n  --no-cache r c s\n"},
            land,
            False,
        ),
        (
            "a call with the flag",
            {REL: "bash land.sh --no-cache r c s\n"},
            land,
            False,
        ),
        ("a comment naming land.sh", {REL: "# land.sh caches\n"}, land, False),
        (
            "a release workflow without the flag",
            {".github/workflows/release.yml": "run: bash land.sh r c s\n"},
            land,
            True,
        ),
        (
            "a release make target without the flag",
            {"Makefile": "release-x:\n\tbash land.sh r c s\n"},
            land,
            True,
        ),
        (
            "a release make target with the flag",
            {"Makefile": "release-x:\n\tbash land.sh --no-cache r c s\n"},
            land,
            False,
        ),
        (
            "a land call in a non-release target",
            {"Makefile": "check:\n\tbash land.sh r c s\n"},
            land,
            False,
        ),
        ("a deny gate missing from fresh", {}, '[landing]\ngate = ["make deny"]\n', True),
        (
            "a cargo deny gate missing from fresh",
            {},
            '[landing]\ngate = ["cargo deny check"]\nfresh = []\n',
            True,
        ),
        ("an unrelated gate", {}, '[landing]\ngate = ["make lint"]\n', False),
    ]
    fails = 0
    for name, files, toml, red in cases:
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            (d / "schwarzwald/config").mkdir(parents=True)
            (d / "schwarzwald/config/land.toml").write_text(toml, encoding="utf-8")
            for rel, body in files.items():
                (d / rel).parent.mkdir(parents=True, exist_ok=True)
                (d / rel).write_text(body, encoding="utf-8")
            got = problems(d)
        if bool(got) != red:
            print(f"land-cache-check self-test: wrong verdict for {name}: {got}")
            fails += 1
    if not fails:
        print("land-cache-check self-test: ok")
    return 1 if fails else 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    bad = problems(ROOT)
    for b in bad:
        print(f"land-cache-check: {b}", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
