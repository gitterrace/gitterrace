#!/usr/bin/env python3
"""`make deps` installs what the gate runs, at the versions CI pins.

1. Every tool `make lint` asks for (a `run <name> <tool>` line in the shared lint runner, and
   the tool of each product step in schwarzwald/config/lint.toml) is a tool deps-run.sh
   installs, or one it needs by name before it starts (git, python3, bash, rustup and cargo).
2. Each version deps-run.sh pins (ruff, cargo-machete, typos) is the version the CI
   lint-tools action installs, so a laptop and CI lint with the same tool.

  python3 scripts/lint/deps-check.py [--repo DIR]
  python3 scripts/lint/deps-check.py --self-test
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# tomllib is 3.11+. The lint runner picks such a python ($PY), but pyright reads the first
# python3 on PATH, which can be an older system one.
import tomllib  # pyright: ignore[reportMissingImports]

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = "scripts/dev/deps-run.sh"
RUNNER = "vendor/schwarzwald/kit/lint/run.sh"
STEPS = "schwarzwald/config/lint.toml"
CI = ".github/actions/lint-tools/action.yml"
# Present before deps-run.sh runs, or the shell it runs in.
GIVEN = {"git", "python3", "bash", "cargo", "rustup"}
# npx and npm come with node.
VIA = {"npx": "node", "npm": "node"}
PINS = {
    "RUFF_VERSION": r"ruff==([\d.]+)",
    "MACHETE_VERSION": r"cargo-machete --version ([\d.]+)",
    "TYPOS_VERSION": r"typos-cli --version ([\d.]+)",
}


def installs(script: str) -> set[str]:
    """The commands deps-run.sh makes sure of: its BREW_CMDS list, ruff and cargo-machete."""
    m = re.search(r"^BREW_CMDS=\(([^)]*)\)", script, re.M)
    cmds = set(m.group(1).split()) if m else set()
    return cmds | {"ruff", "cargo-machete"}


def wanted(runner: str, steps: str) -> set[str]:
    """The tools make lint runs: `run <name> <tool>` lines and each product step's tool."""
    tools = set(re.findall(r"(?:^\s*|[;&|]\s*|then\s+)run\s+[\w-]+\s+([\w.-]+)\s", runner, re.M))
    tools = {t for t in tools if not t.startswith("$")}
    for step in tomllib.loads(steps).get("run", {}).get("steps", []):
        tools.add(step.split("|")[1])
    return tools


def problems(script: str, runner: str, steps: str, ci: str) -> list[str]:
    bad = []
    have = installs(script) | GIVEN
    for tool in sorted(wanted(runner, steps)):
        if VIA.get(tool, tool) not in have:
            bad.append(f"make lint runs {tool}, which {SCRIPT} does not install")
    for var, rx in PINS.items():
        mine = re.search(rf"^{var}=([\d.]+)$", script, re.M)
        theirs = re.search(rx, ci)
        if not mine or not theirs:
            bad.append(f"{var}: no pin in {SCRIPT}" if not mine else f"{var}: not pinned in {CI}")
        elif mine.group(1) != theirs.group(1):
            bad.append(f"{var} is {mine.group(1)} in {SCRIPT} but {theirs.group(1)} in {CI}")
    return bad


def read(root: Path, rel: str) -> str:
    return (root / rel).read_text()


def self_test() -> int:
    script = "BREW_CMDS=(vale node)\nRUFF_VERSION=1.0\nMACHETE_VERSION=2.0\n"
    script += "TYPOS_VERSION=3.0\n"
    runner = "if x; then run typos typos x\n[[ -f a ]] && run vale vale x\n"
    runner += "  run shellcheck shellcheck x\n"
    runner += "# comments that run when a file is there do not count\n"
    steps = '[run]\nsteps = ["pyright|npx|x|y"]\n'
    ci = "ruff==1.0 cargo-machete --version 2.0 typos-cli --version 3.1"
    got = problems(script, runner, steps, ci)
    want = [
        f"make lint runs shellcheck, which {SCRIPT} does not install",
        f"make lint runs typos, which {SCRIPT} does not install",
        f"TYPOS_VERSION is 3.0 in {SCRIPT} but 3.1 in {CI}",
    ]
    ok = problems(
        script.replace("vale", "typos vale shellcheck"),
        runner,
        steps,
        ci.replace("3.1", "3.0"),
    )
    if got != want or ok:
        print(f"deps-check self-test failed: {got} / {ok}")
        return 1
    return 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    root = ROOT
    if "--repo" in sys.argv[1:]:
        root = Path(sys.argv[sys.argv.index("--repo") + 1])
    bad = problems(*(read(root, p) for p in (SCRIPT, RUNNER, STEPS, CI)))
    for b in bad:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
