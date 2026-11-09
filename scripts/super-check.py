#!/usr/bin/env python3
"""Counts `super::` outside tests and holds it to the ratchet (stdlib only).

AGENTS.md names items by a `crate::` path. The ast-grep rule no-super-outside-tests sees only
`use` declarations and macro bodies with a `use`, so this counts every other `super::` path
in non-test Rust, in expressions and inside macro bodies too (it reads text, not a tree). The
count lives in scripts/guards/super-paths.json, a ratchet that schwarzwald/config/ratchet.toml
names, so a new guard needs no change to the shared check; ratchet-check.py fails when it
rises, and this check fails when the tree holds more than the file allows or fewer (lower the
count then).

  python3 scripts/super-check.py              compare the tree with the recorded count
  python3 scripts/super-check.py --self-test  prove a planted new use goes red
"""

from __future__ import annotations

import json
import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = "scripts/guards/super-paths.json"
KEY = "super_paths"
SUPER = re.compile(r"\bsuper::")
# A `use super::` line, in a macro body too, is already an error of the ast-grep rule
# no-super-outside-tests, so it is reported there and not counted a second time here.
USE_SUPER = re.compile(r"\buse\s+super::")
TEST_MOD = re.compile(r"^\s*(pub(\([a-z ]+\))?\s+)?mod\s+\w*tests\b[^;{]*\{")
TEST_FILE = re.compile(r"(^|/)(tests\.rs|[^/]*_tests\.rs)$|/tests/")


def strip_tests(text: str) -> list[str]:
    """Lines outside an inline test module and outside line comments."""
    out: list[str] = []
    depth = 0
    for line in text.splitlines():
        if depth:
            depth += line.count("{") - line.count("}")
            continue
        if TEST_MOD.match(line):
            depth = max(line.count("{") - line.count("}"), 0)
            continue
        if line.lstrip().startswith("//"):
            continue
        out.append(line)
    return out


def count(root: Path) -> int:
    total = 0
    for sub in ("crates", "xtask"):
        for f in sorted((root / sub).rglob("*.rs")):
            rel = f.relative_to(root).as_posix()
            if "/target/" in f"/{rel}" or TEST_FILE.search(rel):
                continue
            total += sum(
                len(SUPER.findall(line))
                for line in strip_tests(f.read_text())
                if not USE_SUPER.search(line)
            )
    return total


def recorded(root: Path) -> int:
    try:
        return int(json.loads((root / REGISTRY).read_text())[KEY]["outside_tests"])
    except (OSError, KeyError, ValueError) as e:
        sys.exit(f"{REGISTRY}: no {KEY}.outside_tests count ({e})")


def check(root: Path) -> list[str]:
    have, allowed = count(root), recorded(root)
    if have > allowed:
        return [
            f"{have} `super::` paths outside tests, {allowed} allowed: use crate:: paths"
        ]
    if have < allowed:
        return [
            f"{have} `super::` paths now, {REGISTRY} {KEY} says {allowed}: lower it to {have}"
        ]
    return []


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        t = Path(tmp)
        (t / "scripts" / "guards").mkdir(parents=True)
        (t / "crates" / "a" / "src").mkdir(parents=True)
        (t / "xtask").mkdir()
        src = t / "crates" / "a" / "src" / "lib.rs"
        src.write_text(
            "fn f() { super::g(); }\n#[cfg(test)]\nmod tests {\n    use super::*;\n}\n"
        )
        (t / REGISTRY).write_text(json.dumps({KEY: {"outside_tests": 1}}))
        ok = check(t) == []
        src.write_text(src.read_text() + "m! {\n    use super::x;\n}\n")
        once = check(t) == []
        src.write_text(src.read_text() + "m! {\n    super::h();\n}\n")
        red = len(check(t)) == 1
    good = ok and once and red
    print("self-test", "ok" if good else "FAILED")
    return 0 if good else 1


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    bad = check(ROOT)
    print("\n".join(bad) if bad else "super-check: ok")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
