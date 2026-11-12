#!/usr/bin/env python3
"""No workflow expression splices outside data into a shell script.

GitHub expands `${{ }}` before the shell runs, so a value inside a `run:` block is code: a
tag, branch or PR title holding shell syntax runs with the job's token. Values reach a script
through `env:` instead. Inside `run:` only expressions over runner.os, matrix.* and env.* are
allowed, values the workflow or the dist plan sets and no pusher picks. Every other context
(github.*, inputs.*, needs.*.outputs.*, steps.*.outputs.*, secrets.*, vars.*) is red.

Checks .github/workflows/*.yml and .github/actions/*/action.yml, read line by line with
indentation, no YAML library.

  python3 scripts/lint/inject-check.py [--repo DIR]
  python3 scripts/lint/inject-check.py --self-test
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXPR = re.compile(r"\$\{\{(.*?)\}\}")
RUN = re.compile(r"^(\s*)(?:-\s+)?run:\s*(.*)$")
# A context name in an expression: a word followed by `.` or `[`.
CONTEXT = re.compile(r"(?<![\w.'\"])([A-Za-z_][\w-]*)\s*[.\[]")
ALLOWED = {"matrix", "env"}
ALLOWED_PATHS = {"runner.os"}


def files(repo: Path) -> list[Path]:
    gh = repo / ".github"
    return sorted(gh.glob("workflows/*.yml")) + sorted(gh.glob("actions/*/action.yml"))


def bad_expr(expr: str) -> str | None:
    """The first disallowed context path in one expression, or None."""
    for m in CONTEXT.finditer(expr):
        name = m.group(1)
        if name in ALLOWED:
            continue
        tail = re.match(r"[\w.\-\[\]'*]*", expr[m.end(1) :])
        path = name + (tail.group(0) if tail else "")
        if path in ALLOWED_PATHS:
            continue
        return path
    return None


def problems_in(name: str, text: str) -> list[str]:
    bad = []
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        m = RUN.match(lines[i])
        if not m:
            i += 1
            continue
        key_indent = len(m.group(1)) + (2 if lines[i].lstrip().startswith("-") else 0)
        body = [(i, m.group(2))]
        j = i + 1
        if m.group(2).strip() in ("|", ">", "|-", ">-", "|+", ">+"):
            body = []
            while j < len(lines):
                ln = lines[j]
                if ln.strip() and len(ln) - len(ln.lstrip()) <= key_indent:
                    break
                body.append((j, ln))
                j += 1
        for n, ln in body:
            for e in EXPR.findall(ln):
                path = bad_expr(e)
                if path:
                    bad.append(
                        f"{name}:{n + 1}: `{path}` expands inside run:, pass it through env:"
                    )
        i = j
    return bad


def problems(repo: Path) -> list[str]:
    bad = []
    for f in files(repo):
        bad += problems_in(str(f.relative_to(repo)), f.read_text())
    return bad


def self_test() -> int:
    cases = [
        ("      - run: dist ${{ github.ref_name }}\n", True),
        ("      - run: |\n          echo ${{ needs.plan.outputs.tag }}\n", True),
        ("        run: |\n          x ${{ steps.a.outputs.b }}\n", True),
        ("        run: echo ${{ inputs.full && 'a' || 'b' }}\n", True),
        ("        run: echo ${{ github.event.pull_request.title }}\n", True),
        ("        run: ${{ matrix.install_dist.run }}\n", False),
        ("        run: |\n          if [ ${{ runner.os }} = x ]; then :; fi\n", False),
        ("        run: echo ${{ env.FOO }}\n", False),
        (
            "        env:\n          T: ${{ github.ref_name }}\n        run: |\n"
            '          echo "$T"\n      - name: next\n        if: ${{ github.ref }}\n',
            False,
        ),
    ]
    good = True
    for i, (text, red) in enumerate(cases):
        got = problems_in(f"case{i}", text)
        if bool(got) != red:
            print(
                f"inject-check self-test: case {i} {'passed' if red else 'failed'}: {got}"
            )
            good = False
    print("inject-check self-test:", "ok" if good else "FAILED")
    return 0 if good else 1


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    repo = (
        Path(sys.argv[sys.argv.index("--repo") + 1]) if "--repo" in sys.argv else ROOT
    )
    bad = problems(repo)
    for b in bad:
        print(f"inject-check: {b}", file=sys.stderr)
    if not bad:
        print(
            f"inject-check: {len(files(repo))} workflow files splice no outside data into run:"
        )
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
