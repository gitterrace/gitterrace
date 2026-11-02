#!/usr/bin/env python3
"""Fail when a test module file is not compiled for tests only.

The ast-grep rules (no-bare-file-write, no-super-outside-tests) skip files named `tests.rs`
or `*_tests.rs`, because a test module kept in its own file has no inline `mod tests` for
them to see. That skip is only sound if every such file really is test code, so each must be
declared by its parent under `#[cfg(test)]` or a `cfg_attr`/`cfg` that names `test`. A
production file named `tests.rs` fails here. Run by scripts/lint-run.sh as the tests hook.

  python3 scripts/tests-check.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GLOBS = (
    "crates/**/tests.rs",
    "crates/**/*_tests.rs",
    "xtask/**/tests.rs",
    "xtask/**/*_tests.rs",
)
# The attributes right above a `mod NAME;`, then the declaration itself.
DECL = r"((?:[ \t]*#\[[^\]]*(?:\[[^\]]*\][^\]]*)*\][ \t]*\n|[ \t]*//[^\n]*\n)*)[ \t]*(?:pub(?:\([a-z]+\))? )?mod {name};"


def parent(file: Path) -> Path | None:
    """The file that declares `file` as a module: `dir.rs` or `dir/mod.rs` beside it."""
    d = file.parent
    for p in (d.with_suffix(".rs"), d / "mod.rs", d / "lib.rs", d / "main.rs"):
        if p.is_file() and p != file:
            return p
    return None


def by_path(file: Path) -> tuple[Path, re.Match[str]] | None:
    """A `#[path = "<file>"] mod x;` beside `file`: the declaring file and its attributes."""
    attr = re.escape(f'#[path = "{file.name}"]')
    pat = re.compile(DECL.format(name=r"\w+"))
    for p in sorted(file.parent.glob("*.rs")):
        if p == file:
            continue
        for m in pat.finditer(p.read_text()):
            if re.search(attr, m.group(1)):
                return p, m
    return None


def test_only(attrs: str) -> bool:
    """`#[cfg(test)]`, or a `cfg`/`cfg_attr` whose condition names `test`."""
    return bool(re.search(r"#\[cfg\((?:[^()]|\([^()]*\))*\btest\b", attrs))


def problems(root: Path) -> list[str]:
    out = []
    files = sorted({f for g in GLOBS for f in root.glob(g)})
    for f in files:
        if "/tests/" in f.relative_to(root).as_posix():
            continue  # integration tests: a crate's tests/ folder is test-only by cargo
        rel = f.relative_to(root)
        found = by_path(f)
        p = found[0] if found else parent(f)
        if p is None:
            out.append(f"{rel}: no parent module file found")
            continue
        m = (
            found[1]
            if found
            else re.search(DECL.format(name=re.escape(f.stem)), p.read_text())
        )
        if m is None:
            out.append(f"{rel}: {p.relative_to(root)} does not declare `mod {f.stem};`")
        elif not test_only(m.group(1)):
            out.append(f"{rel}: declared without #[cfg(test)] in {p.relative_to(root)}")
    return out


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT
    bad = problems(root)
    for b in bad:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
