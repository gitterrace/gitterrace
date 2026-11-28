#!/usr/bin/env python3
"""Fail when a browser case names a port, or a check is missing from ALL in browser-check.mjs.

A check is missing when a lib exports it but ALL does not list it.

Each scripts/verify/browser/*-lib.mjs exports `checks(env)`, which returns an object of check
functions. browser-check.mjs runs only the names in its ALL list, so a check added to a lib and
forgotten there never runs, and nothing says so. This reads both and names every key that is not
reachable from ALL (by camelCase of the name, or through ALIAS). Keys that are helpers rather than
checks are listed in HELPERS below, each with a reason.

  python3 scripts/lint/browser-all-check.py              check the tree
  python3 scripts/lint/browser-all-check.py --self-test  prove a forgotten check goes red
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DIR = ROOT / "scripts/verify/browser"
RUNNER = DIR / "browser-check.mjs"
# Keys a lib returns from checks(env) that are not checks of their own.
HELPERS: dict[str, str] = {}


def camel(name: str) -> str:
    return re.sub(r"-(\w)", lambda m: m.group(1).upper(), name)


def parse_all(runner: str) -> set[str]:
    """The export keys the runner will look up: every ALL name, through ALIAS when it has one."""
    found = re.search(r"const ALL = \[(.*?)\];", runner, re.S)
    names = re.findall(r"'([^']+)'", found.group(1)) if found else []
    alias_src = re.search(r"const ALIAS = \{(.*?)\};", runner, re.S)
    alias = dict(re.findall(r"'([^']+)':\s*'([^']+)'", alias_src.group(1))) if alias_src else {}
    return {alias.get(n, camel(n)) for n in names}


def top_level(src: str) -> list[str]:
    """Split on commas outside brackets, so `k: () => f(a, b)` stays one entry."""
    parts, depth, cur = [], 0, ""
    for ch in src:
        depth += ch in "([{"
        depth -= ch in ")]}"
        if ch == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += ch
    return [*parts, cur]


def exported_keys(lib: str) -> list[str]:
    """Keys of the object `checks(env)` returns, spreads of other libs left out."""
    start = re.search(r"^export function checks\b.*$", lib, re.M)
    if not start:
        return []
    body = lib[start.end() :]
    end = re.search(r"^\}", body, re.M)
    body = body[: end.start()] if end else body
    rets = list(re.finditer(r"^  return \{(.*?)\};?$", body, re.M | re.S))
    if not rets:
        return []
    keys = []
    for part in top_level(rets[-1].group(1)):
        part = part.strip()
        m = re.match(r"[\w$]+", part)
        if m and not part.startswith("..."):
            keys.append(m.group(0))
    return keys


def problems(runner: str, libs: dict[str, str]) -> list[str]:
    known = parse_all(runner)
    bad = []
    for name, text in sorted(libs.items()):
        for key in exported_keys(text):
            if key not in known and key not in HELPERS:
                bad.append(
                    f"{name}: check `{key}` is exported but not in ALL in browser-check.mjs. "
                    "Add it to ALL (and ALIAS if its name is not the camelCase), or to HELPERS here"
                )
    return bad


ADDR = re.compile(r"--addr|127\.0\.0\.1:\$?\{?[0-9]*[1-9]|localhost:[0-9]*[1-9]")


def port_problems(libs: dict[str, str]) -> list[str]:
    """A case names no address or port: `serve()` binds port 0 and its `cli()` runs the binary."""
    return [
        f"{name}:{n}: names an address or port. Use serve() from lib/browser-lib.mjs and its cli()"
        for name, text in sorted(libs.items())
        for n, line in enumerate(text.splitlines(), 1)
        if ADDR.search(line)
    ]


RAW_LAUNCH = re.compile(r"\b(?:chromium|webkit|firefox|pw\[[^\]]*\])\.launch(?:Persistent\w*)?\(")


def launch_problems(files: dict[str, str]) -> list[str]:
    """Only lib/browser-lib.mjs starts a browser: its launch() hooks the same-origin error guard,
    so a browser started any other way would let a 403 pass unseen."""
    return [
        f"{name}:{n}: starts a browser directly. Use launch() from lib/browser-lib.mjs"
        for name, text in sorted(files.items())
        if name != "browser-lib.mjs"
        for n, line in enumerate(text.splitlines(), 1)
        if RAW_LAUNCH.search(line)
    ]


def check_tree() -> int:
    libs = {p.name: p.read_text(encoding="utf-8") for p in sorted(DIR.glob("*-lib.mjs"))}
    bad = problems(RUNNER.read_text(encoding="utf-8"), libs)
    bad += port_problems(libs)
    scripts = [*DIR.glob("*.mjs"), *(ROOT / "scripts/lib").glob("*.mjs")]
    bad += launch_problems({p.name: p.read_text(encoding="utf-8") for p in scripts})
    for line in bad:
        print(line)
    if bad:
        return 1
    print(f"browser-all-check: ok ({len(libs)} libs)")
    return 0


RUNNER_FIXTURE = "const ALL = ['alpha-one', 'beta'];\nconst ALIAS = { 'beta': 'betaCheck' };\n"
GREEN_LIB = "export function checks(env) {\n  return { alphaOne, betaCheck, ...other(env) };\n}\n"
RED_LIB = "export function checks(env) {\n  return {\n    alphaOne,\n    forgotten,\n  };\n}\n"


def self_test() -> int:
    green = problems(RUNNER_FIXTURE, {"g-lib.mjs": GREEN_LIB})
    red = problems(RUNNER_FIXTURE, {"r-lib.mjs": RED_LIB})
    arrow = problems(
        RUNNER_FIXTURE, {"a-lib.mjs": "export function checks(e) {\n  return { x: () => 1 };\n}\n"}
    )
    fixed = port_problems({"f-lib.mjs": "sh(d, bin, '--addr', '127.0.0.1:38625', d);\n"})
    fixed2 = port_problems({"f-lib.mjs": "const u = 'http://127.0.0.1:8080';\n"})
    clean = port_problems({"c-lib.mjs": "const s = await serve(bin, dir);\ns.cli(d, d);\n"})
    raw = launch_problems({"x-lib.mjs": "const b = await pw.chromium.launch();\n"})
    raw2 = launch_problems({"x.mjs": "await firefox.launchPersistentContext(d);\n"})
    raw3 = launch_problems({"x.mjs": "const b = await pw[name].launch();\n"})
    via = launch_problems({"x.mjs": "const b = await launch(pw, 'webkit');\n"})
    own = launch_problems({"browser-lib.mjs": "return pw.chromium.launch({});\n"})
    ok = not green and len(red) == 1 and "forgotten" in red[0] and len(arrow) == 1
    ok = ok and len(fixed) == 1 and len(fixed2) == 1 and not clean
    ok = ok and len(raw) == 1 and len(raw2) == 1 and len(raw3) == 1 and not via and not own
    print("browser-all-check self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    return check_tree()


if __name__ == "__main__":
    sys.exit(main())
