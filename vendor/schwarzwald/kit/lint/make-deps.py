#!/usr/bin/env python3
"""CONV-MAKE-DEPS: `make deps` installs every tool the gate runs, at the version CI pins.

A product names its installer and its CI tool step in lint.toml `[deps]`:

  installer  the script `make deps` runs. It lists the commands it makes sure of in a
             `DEPS_TOOLS=(...)` array and pins a version as `<NAME>_VERSION=x.y.z`.
  ci         the file that installs the same tools in CI (a composite action or a workflow).
  given      commands present before the installer runs (git, the shell, the toolchain manager).
  pins       NAME = a regex with one group that finds that version in `ci`.

1. Every tool `make lint` runs (each `run <name> <tool>` line of run.sh and the tool of each
   `[run] steps` entry) is one the installer makes sure of, or one of `given`. npm and npx
   count as node.
2. Each pin the installer sets equals the version `ci` installs, so a laptop and CI lint alike.

An empty `installer` turns the check off for a product without `make deps`.

  make-deps.py [--repo DIR]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-MAKE-DEPS"
RUNNER = Path(__file__).resolve().parent / "run.sh"
VIA = {"npx": "node", "npm": "node"}
RUN_LINE = re.compile(r"(?:^\s*|[;&|]\s*|then\s+)run\s+[\w-]+\s+([\w.-]+)\s", re.M)


def installs(script: str) -> set[str]:
    m = re.search(r"^DEPS_TOOLS=\(([^)]*)\)", script, re.M)
    return set(m.group(1).split()) if m else set()


def wanted(runner: str, steps: list[str]) -> set[str]:
    tools = {t for t in RUN_LINE.findall(runner) if not t.startswith("$")}
    return tools | {s.split("|")[1] for s in steps if s.count("|") >= 3}


def problems(script: str, ci: str, runner: str, steps: list[str], deps: dict) -> list[str]:
    bad = []
    have = installs(script) | set(deps["given"])
    if not installs(script):
        bad.append(f"{deps['installer']}: no DEPS_TOOLS=(...) list of the tools it installs")
    for tool in sorted(wanted(runner, steps)):
        if VIA.get(tool, tool) not in have:
            bad.append(f"make lint runs {tool}, which {deps['installer']} does not install")
    for name, rx in deps["pins"].items():
        mine = re.search(rf"^{name}_VERSION=([\w.-]+)$", script, re.M)
        theirs = re.search(rx, ci)
        if not mine:
            bad.append(f"{deps['installer']}: no {name}_VERSION pin")
        elif not theirs:
            bad.append(f"{deps['ci']}: no version for {name} ({rx})")
        elif mine.group(1) != theirs.group(1):
            bad.append(f"{name} is {mine.group(1)} in {deps['installer']} "
                       f"but {theirs.group(1)} in {deps['ci']}")
    return bad


def self_test() -> bool:
    deps = {"installer": "i.sh", "ci": "ci.yml", "given": ["git"],
            "pins": {"RUFF": r"ruff==([\d.]+)"}}
    script = "DEPS_TOOLS=(vale node ruff)\nRUFF_VERSION=1.0\n"
    runner = "if x; then run typos typos y\n[[ -f a ]] && run vale vale y\n# run me now\n"
    steps = ["pyright|npx|hint|cmd", "g|git|hint|cmd"]
    ok = (problems(script.replace("vale", "vale typos"), "ruff==1.0", runner, steps, deps) == []
          and problems(script, "ruff==1.0", runner, steps, deps)
          and problems(script.replace("vale", "vale typos"), "ruff==1.1", runner, steps, deps)
          and problems("RUFF_VERSION=1.0\n", "ruff==1.0", "", [], deps))
    return config.self_check(RULE, ok, "a missing tool or a drifted pin must fail")


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    deps = cfg["deps"]
    if not deps["installer"]:
        print(f"PASS [{RULE}] no [deps] installer, nothing to check")
        return 0
    try:
        script = (root / deps["installer"]).read_text(encoding="utf-8")
        ci = (root / deps["ci"]).read_text(encoding="utf-8") if deps["ci"] else ""
    except OSError as e:
        print(f"FAIL [{RULE}] {e}")
        return 1
    bad = problems(script, ci, RUNNER.read_text(encoding="utf-8"), cfg["run"]["steps"], deps)
    if bad:
        print(f"FAIL [{RULE}] {len(bad)} gap(s) between make deps, make lint and CI")
        for b in bad:
            print(f"  {b}")
        return 1
    print(f"PASS [{RULE}] make deps installs every tool make lint runs, at CI's versions")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
