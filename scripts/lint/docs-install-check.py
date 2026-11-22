#!/usr/bin/env python3
"""The docs teach one spelling of the command and one order of install ways.

- bare-binary: `git-terrace` in prose or a shell example. People type `git terrace`. The
  binary name stays where a program is spawned or a file is named: json, toml, ini, xml and
  Dockerfile fences, a path (`target/debug/git-terrace`), and the one sentence that says what
  the installer installs ("which git runs as").
- cargo-first: a doc that shows `cargo install gitterrace` before a brew line, or with no brew
  line at all, teaches the fallback as the default.
- crates-io: `cargo install gitterrace` with no `--git` asks crates.io, where the package is
  not published, so the command fails for whoever copies it.

  python3 scripts/lint/docs-install-check.py              check README, CONTRIBUTING and docs/
  python3 scripts/lint/docs-install-check.py --self-test  every rule red and green on planted text
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPAWN_FENCES = {"json", "toml", "ini", "xml", "dockerfile", "yaml"}
BARE = re.compile(r"(?<![\w/.\-])git-terrace(?![\w\-])")
CARGO = re.compile(r"\bcargo install\b(?!\s+(?:cargo-|--locked\s+cargo-))[^\n]*\bgitterrace\b")
BREW = re.compile(r"\bbrew (?:install|upgrade)\b")
EXPLAINS = "which git runs as"


def scan(text: str, name: str) -> list[str]:
    out: list[str] = []
    fence = None
    first_brew = None
    for n, line in enumerate(text.splitlines(), 1):
        m = re.match(r"\s*```(\w*)", line)
        if m:
            fence = None if fence is not None else (m.group(1).lower() or "text")
            continue
        spawn = fence in SPAWN_FENCES
        if not spawn and BARE.search(line) and EXPLAINS not in line:
            out.append(f"{name}:{n}: bare `git-terrace`: write `git terrace`")
        if first_brew is None and BREW.search(line):
            first_brew = n
        if CARGO.search(line):
            if not re.search(r"--git\b", line):
                out.append(f"{name}:{n}: `cargo install gitterrace` asks crates.io: add --git")
            if first_brew is None:
                out.append(f"{name}:{n}: cargo install before any brew line: brew comes first")
    return out


def files(root: Path) -> list[Path]:
    found = [root / "README.md", root / "CONTRIBUTING.md", *sorted((root / "docs").rglob("*.md"))]
    return [f for f in found if f.is_file()]


def self_test() -> int:
    red = {
        "bare-binary": "Run `git-terrace serve` now.\n",
        "bare-in-sh": "```sh\ngit-terrace service install\n```\n",
        "cargo-first": (
            "```sh\ncargo install --locked --git u gitterrace\nbrew install t/gitterrace\n```\n"
        ),
        "cargo-only": "```sh\ncargo install --locked --git u gitterrace\n```\n",
        "crates-io": "```sh\nbrew install t/gitterrace\ncargo install gitterrace\n```\n",
    }
    green = {
        "json-spawn": '```json\n{ "command": "git-terrace" }\n```\n',
        "path": "`target/debug/git-terrace`\n",
        "explained": "Installs `git-terrace`, which git runs as `git terrace`.\n",
        "brew-first": "brew install t/gitterrace\ncargo install --locked --git u gitterrace\n",
        "tool": "cargo install cargo-llvm-cov\n",
    }
    ok = True
    for k, t in red.items():
        if not scan(t, k):
            print(f"self-test: {k} should be red")
            ok = False
    for k, t in green.items():
        if scan(t, k):
            print(f"self-test: {k} should be green: {scan(t, k)}")
            ok = False
    print("self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    bad = [m for f in files(ROOT) for m in scan(f.read_text(), str(f.relative_to(ROOT)))]
    print("\n".join(bad) if bad else "docs-install-check: ok")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
