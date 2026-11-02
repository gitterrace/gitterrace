#!/usr/bin/env python3
"""Docs guard (stdlib only), run by `make docs-check`: every `make <target>` named in tracked
text is a target the Makefile defines, so a renamed target cannot leave a stale instruction.

Also: every tracked Python script under scripts/ and docs/ runs only from its
`if __name__ == "__main__":` guard. A bare call at module level (main(), run(),
sys.exit(main())) runs on import, so a test or a type checker that imports the file runs it.

  python3 scripts/make-targets-check.py
"""

from __future__ import annotations

import ast
import re
import subprocess
import sys
from pathlib import Path

TEXT = (".md", ".py", ".sh", ".txt", ".yml", ".mjs", ".rs", ".toml")


def files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z"], capture_output=True, check=True, text=True
    )
    return [
        f
        for f in out.stdout.split("\0")
        if f.endswith(TEXT) and not f.startswith("theme/")
    ]


def targets() -> set[str]:
    text = Path("Makefile").read_text(encoding="utf-8")
    return set(re.findall(r"^([A-Za-z0-9_.-]+):", text, re.M))


def is_main_guard(node: ast.stmt) -> bool:
    if not isinstance(node, ast.If) or not isinstance(node.test, ast.Compare):
        return False
    t = node.test
    sides = [t.left, *t.comparators]
    return any(isinstance(s, ast.Name) and s.id == "__name__" for s in sides) and any(
        isinstance(s, ast.Constant) and s.value == "__main__" for s in sides
    )


def unguarded_calls(path: str, source: str) -> list[str]:
    """Module-level code outside the __main__ guard that runs something: any bare call
    statement, or any other statement that calls a function this file defines. Imports,
    defs, classes and constants built by library calls (re.compile) stay allowed."""
    tree = ast.parse(source, path)
    own = {
        n.name
        for n in tree.body
        if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))
    }
    bad: list[str] = []
    for node in tree.body:
        if isinstance(
            node,
            (
                ast.Import,
                ast.ImportFrom,
                ast.FunctionDef,
                ast.AsyncFunctionDef,
                ast.ClassDef,
            ),
        ) or is_main_guard(node):
            continue
        calls = [n for n in ast.walk(node) if isinstance(n, ast.Call)]
        own_calls = [
            c.func.id
            for c in calls
            if isinstance(c.func, ast.Name) and c.func.id in own
        ]
        bare = isinstance(node, ast.Expr) and isinstance(node.value, ast.Call)
        if own_calls or bare:
            func = calls[0].func
            name = (
                own_calls[0]
                if own_calls
                else func.attr
                if isinstance(func, ast.Attribute)
                else getattr(func, "id", "?")
            )
            bad.append(
                f"{path}:{node.lineno}: module-level call to {name}() outside __main__ guard"
            )
    return bad


def script_files() -> list[str]:
    out = subprocess.run(
        [
            "git",
            "ls-files",
            "-z",
            "--",
            ":(glob)scripts/**/*.py",
            ":(glob)docs/**/*.py",
        ],
        capture_output=True,
        check=True,
        text=True,
    )
    return [f for f in out.stdout.split("\0") if f and Path(f).is_file()]


def main() -> int:
    known = targets()
    bad: list[str] = []
    paths = sys.argv[1:] or script_files()
    for path in paths:
        bad += unguarded_calls(path, Path(path).read_text(encoding="utf-8"))
    if sys.argv[1:]:
        print("\n".join(bad))
        return 1 if bad else 0
    for path in files():
        if path.endswith("make-targets-check.py"):
            continue
        try:
            text = Path(path).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        fenced = False
        for n, line in enumerate(text.splitlines(), 1):
            if line.lstrip().startswith("```"):
                fenced = not fenced
            # Backticked anywhere. Bare `make X` too where it is a command: shell scripts,
            # fenced code blocks in markdown, and workflow lines that are not comments.
            yml_cmd = path.endswith(".yml") and not line.lstrip().startswith("#")
            bare = fenced or yml_cmd or path.endswith(".sh")
            pattern = (
                r"(?:^|[\s`;&|(])make ([a-z][a-z0-9-]*)"
                if bare
                else r"`make ([a-z][a-z0-9-]*)"
            )
            for m in re.finditer(pattern, line):
                if m.group(1) not in known:
                    bad.append(
                        f"{path}:{n}: make {m.group(1)} is not a Makefile target"
                    )
    print("\n".join(bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
