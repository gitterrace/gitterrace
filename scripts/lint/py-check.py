#!/usr/bin/env python3
"""A lint step whose script imports tomllib at module level runs on `$PY`.

tomllib is Python 3.11+. The lint runner picks such a python and exports it as `$PY`, but a
step that names a bare `python3` gets whatever is first on PATH, which on macOS can be 3.9: the
step then fails on a machine that has a newer python too. Red when a `[run] steps` entry of
schwarzwald/config/lint.toml runs such a script with anything but `$PY`. A script that imports
tomllib only inside a function, for a mode the step does not use, is not counted.

Also red when pyrightconfig.json's pythonVersion is below 3.11 while a script imports tomllib:
pyright then reports tomllib missing, whichever python3 is first on PATH.

  python3 scripts/lint/py-check.py              check the steps
  python3 scripts/lint/py-check.py --self-test  red and green on fixtures
"""

from __future__ import annotations

import json
import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# A top-level import, not one indented inside a function.
TOP_IMPORT = re.compile(r"^(import tomllib|from tomllib import)", re.M)
STEP = re.compile(r'^\s*"([^"|]+)\|[^|]*\|[^|]*\|(.*)",?\s*$', re.M)
SCRIPT = re.compile(r"(\S+)\s+((?:scripts|vendor)/\S+\.py)\b")


MIN_PY = (3, 11)


def pyright_problems(repo: Path) -> list[str]:
    cfg = repo / "pyrightconfig.json"
    if not cfg.is_file():
        return []
    want = json.loads(cfg.read_text(encoding="utf-8")).get("pythonVersion", "")
    try:
        have = tuple(int(x) for x in want.split("."))
    except ValueError:
        return [f"pyrightconfig.json: pythonVersion {want!r} is not a version"]
    uses = any(
        TOP_IMPORT.search(p.read_text(encoding="utf-8")) for p in (repo / "scripts").rglob("*.py")
    )
    if uses and have < MIN_PY:
        return [
            f"pyrightconfig.json: pythonVersion {want} is below 3.11 but scripts import "
            "tomllib, so pyright fails on it under an older python3"
        ]
    return []


def problems(repo: Path) -> list[str]:
    return step_problems(repo) + pyright_problems(repo)


def step_problems(repo: Path) -> list[str]:
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
    for ver, red in (("3.9", True), ("3.11", False), ("3.12", False)):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            (d / "scripts").mkdir()
            (d / "scripts" / "fixture.py").write_text("import tomllib\n")
            (d / "pyrightconfig.json").write_text(json.dumps({"pythonVersion": ver}))
            if bool(pyright_problems(d)) != red:
                print(f"py-check self-test: pythonVersion {ver} should be red={red}")
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
