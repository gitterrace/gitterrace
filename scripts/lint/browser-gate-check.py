#!/usr/bin/env python3
"""Fail unless a hosted gate runs the real-browser checks, and runs all of them.

Nothing but a manual `make browser-check` ran scripts/verify/browser/browser-check.mjs, so a red
case reached main unseen. lint-and-test.yml must have a fast-tier job `browser` (no `if:`, so it
runs on every push) that runs `make browser-check`. Nothing may narrow the run: not a `--checks`
or `--browsers` flag in the job or in the Makefile's browser targets, and the runner's `--checks`
default must stay the whole ALL list. Then every case in ALL is reachable from that job
(scripts/lint/browser-all-check.py proves the libs and ALL agree).

  python3 scripts/lint/browser-gate-check.py [--repo DIR]
  python3 scripts/lint/browser-gate-check.py --self-test   red and green on fixture trees
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUNNER = "scripts/verify/browser/browser-check.mjs"
WORKFLOW = ".github/workflows/lint-and-test.yml"
NARROW = re.compile(r"--(?:checks|browsers)\b")


def job_body(text: str, name: str) -> str | None:
    """The body of job `name` under `jobs:` (2-space job keys), or None."""
    cur, inside, body = None, False, ""
    for line in text.splitlines():
        if re.match(r"^jobs:\s*$", line):
            inside = True
        elif inside and re.match(r"^\S", line):
            inside = False
        elif inside:
            m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
            if m:
                cur = m.group(1)
            elif cur == name:
                body += line + "\n"
    return body if cur is not None and (body or re.search(rf"^  {name}:", text, re.M)) else None


def recipe(makefile: str, target: str) -> str:
    m = re.search(rf"^{re.escape(target)}:[^\n]*\n((?:\t[^\n]*\n)*)", makefile, re.M)
    return m.group(1) if m else ""


# The step that makes each artifact browser-check.mjs lists in NEEDS, as the job must run it.
PRODUCERS = {"target/site/": r"\bcargo\s+xtask\s+site\b"}
READS = re.compile(r"target/(?!debug|release|shots)[\w-]+")


def artifact_problems(body: str, runner: str, libs: str) -> list[str]:
    """Every artifact a case reads is in NEEDS and has a producer step in the job."""
    found = re.search(r"const NEEDS = \{(.*?)\n\};", runner, re.S)
    needs = re.findall(r"'(target/[^']+)':", found.group(1)) if found else []
    bad = []
    for art in needs:
        pat = next((p for k, p in PRODUCERS.items() if art.startswith(k)), None)
        if pat is None:
            bad.append(f"{RUNNER}: NEEDS {art} has no producer in PRODUCERS of this check")
        elif not re.search(pat, body):
            bad.append(f"{WORKFLOW}: job browser never builds {art}, which cases read")
    for read in sorted(set(READS.findall(libs))):
        if not any(n.startswith(read + "/") for n in needs):
            bad.append(f"browser libs read {read} but {RUNNER} NEEDS does not list it")
    return bad


def problems(workflow: str, makefile: str, runner: str, libs: str = "") -> list[str]:
    body = job_body(workflow, "browser")
    if body is None:
        return [f"{WORKFLOW}: no `browser` job, so no gate runs {RUNNER}"]
    bad = []
    if re.search(r"^    if: ", body, re.M):
        bad.append(f"{WORKFLOW}: job browser is conditional; it runs on every push")
    if not re.search(r"\bmake\b[^\n]*\bbrowser-check\b", body):
        bad.append(f"{WORKFLOW}: job browser does not run `make browser-check`")
    for line in body.splitlines():
        if "browser-check" in line and NARROW.search(line):
            bad.append(f"{WORKFLOW}: job browser narrows the run ({line.strip()}); it runs all")
    for t in ("browser-check", "browser-reap", "browser-disk", "browser-err"):
        if NARROW.search(recipe(makefile, t)):
            bad.append(f"Makefile: {t} filters cases or browsers; the gate runs them all")
    if not re.search(r"arg\('--checks', ALL\.join\(','\)\)", runner):
        bad.append(f"{RUNNER}: --checks no longer defaults to the whole ALL list")
    if not re.search(r"const ALL = \[[^\]]*'[^\]]*\];", runner):
        bad.append(f"{RUNNER}: ALL is empty or unreadable")
    return bad + artifact_problems(body, runner, libs)


MK = "browser-check:\n\tnode x.mjs{flag}\nbrowser-reap:\n\tnode y\nbrowser-disk:\n\tnode z\n"
JS = (
    "const ALL = ['a', 'b'];\nconst wanted = arg('--checks', ALL.join(','));\n"
    "const NEEDS = {\n  'target/site/_headers': ['a'],\n};\n"
)
JOB = (
    "jobs:\n  browser:\n    runs-on: ubuntu-latest\n    steps:\n"
    "      - run: cargo xtask site\n      - run: make browser-deps\n"
    "      - run: make browser-check BIN=b{flag}\n"
)


def self_test() -> int:
    cases = [
        ((JOB.format(flag=""), MK.format(flag=""), JS), False),
        (("jobs:\n  other:\n    runs-on: x\n", MK.format(flag=""), JS), True),
        (
            (
                JOB.replace("    runs-on", "    if: inputs.full\n    runs-on"),
                MK.format(flag=""),
                JS,
            ),
            True,
        ),
        ((JOB.format(flag=" --checks a"), MK.format(flag=""), JS), True),
        ((JOB.format(flag=""), MK.format(flag=" --browsers chromium"), JS), True),
        ((JOB.format(flag=""), MK.format(flag=""), JS.replace("ALL.join(',')", "'a'")), True),
        (
            (JOB.format(flag="").replace("browser-check", "browser-deps"), MK.format(flag=""), JS),
            True,
        ),
    ]
    cases += [
        ((JOB.format(flag="").replace("cargo xtask site", "true"), MK.format(flag=""), JS), True),
        ((JOB.format(flag=""), MK.format(flag=""), JS.replace("site/_headers", "other/x")), True),
    ]
    unlisted = "join(root, 'target/other/x')"
    if not problems(JOB.format(flag=""), MK.format(flag=""), JS, unlisted):
        print("browser-gate-check self-test: an unlisted artifact read passed")
        return 1
    for i, ((wf, mk, js), red) in enumerate(cases):
        got = problems(wf, mk, js, "join(root, 'target/site/x')")
        if bool(got) != red:
            print(f"browser-gate-check self-test: case {i} {'passed' if red else 'failed'}: {got}")
            return 1
    print("browser-gate-check self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    repo = Path(sys.argv[sys.argv.index("--repo") + 1]) if "--repo" in sys.argv else ROOT
    if not (repo / RUNNER).exists():
        return 0
    bad = problems(
        (repo / WORKFLOW).read_text(),
        (repo / "Makefile").read_text(),
        (repo / RUNNER).read_text(),
        "\n".join(p.read_text() for p in sorted((repo / RUNNER).parent.glob("*.mjs"))),
    )
    for b in bad:
        print(f"browser-gate-check: {b}", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
