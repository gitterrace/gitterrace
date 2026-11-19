#!/usr/bin/env python3
"""A lint step whose script imports tomllib at module level runs on `$PY`.

tomllib is Python 3.11+. The lint runner picks such a python and exports it as `$PY`, but a
step that names a bare `python3` gets whatever is first on PATH, which on macOS can be 3.9: the
step then fails on a machine that has a newer python too. Red when a `[run] steps` entry of
schwarzwald/config/lint.toml runs such a script with anything but `$PY`. A script that imports
tomllib only inside a function, for a mode the step does not use, is not counted.

  python3 scripts/lint/py-check.py              check the steps
  python3 scripts/lint/py-check.py --self-test  red and green on fixtures
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# A top-level import, not one indented inside a function.
TOP_IMPORT = re.compile(r"^(import tomllib|from tomllib import)", re.M)
STEP = re.compile(r'^\s*"([^"|]+)\|[^|]*\|[^|]*\|(.*)",?\s*$', re.M)
SCRIPT = re.compile(r"(\S+)\s+((?:scripts|vendor)/\S+\.py)\b")


def problems(repo: Path) -> list[str]:
    text = (repo / "schwarzwald/config/lint.toml").read_text(encoding="utf-8")
    bad = []
    for name, cmd in STEP.findall(text):
        for runner, script in SCRIPT.findall(cmd):
            path = repo / script
            if runner == "$PY" or not path.is_file():
                continue
            if TOP_IMPORT.search(path.read_text(encoding="utf-8")):
                bad.append(
                    f"step {name}: {script} imports tomllib, so run it as `$PY {script}`, "
                    f"not `{runner}`"
                )
    return bad


def self_test() -> int:
    # A made-up script, named through a variable so the path checks do not take it for a real one.
    fx = "fixture.py"
    cases = [
        (f"$PY scripts/{fx}", "import tomllib\n", False),
        (f"python3 scripts/{fx}", "import sys\n", False),
        # Imported only inside a function: a mode the step does not run.
        (f"python3 scripts/{fx}", "def f():\n    import tomllib\n", False),
        # The class this guards: a bare python3 for a module-level tomllib import.
        (f"python3 scripts/{fx}", "import tomllib\n", True),
        (f"python3.9 scripts/{fx} --self-test", "from tomllib import loads\n", True),
    ]
    for i, (cmd, source, red) in enumerate(cases):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            (d / "schwarzwald/config").mkdir(parents=True)
            (d / "scripts").mkdir()
            (d / "scripts" / fx).write_text(source)
            (d / "schwarzwald/config/lint.toml").write_text(
                f'[run]\nsteps = [\n  "a|python3|install|{cmd}",\n]\n'
            )
            got = problems(d)
        if bool(got) != red:
            print(f"py-check self-test: case {i} {'passed' if red else 'failed'}: {got}")
            return 1
    print("py-check self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    bad = problems(ROOT)
    for b in bad:
        print(f"py-check: {b}", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
