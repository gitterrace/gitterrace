#!/usr/bin/env python3
"""The CI tiers hold: every job runs on each pull request, or a tag cannot ship without it.

1. ci.yml has exactly one job, a `uses:` of lint-and-test.yml. A job added straight to ci.yml
   would dodge the tiers, so it is red here.
2. A lint-and-test.yml job gated `if: inputs.full` (the full tier, manual and weekly runs only)
   must be in COVERS, naming the release-check.yml job that runs the same OS again before dist
   publishes, and that job's matrix must hold that OS. Every other job is the fast tier, which
   runs on every pull request and push.
3. COVERS names no job lint-and-test.yml lacks.
4. Every release-check.yml job that runs `make check` checks out with `fetch-depth: 0`: the
   history lints refuse a shallow clone.

Read with a small indentation reader (2-space job keys under `jobs:`), no YAML library.

  python3 scripts/lint/ci-check.py [--repo DIR]
  python3 scripts/lint/ci-check.py --self-test
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WF = ".github/workflows"
# Full-tier job -> (release-check job, the OS its matrix must hold).
COVERS = {
    "check-macos": ("check", "macos-latest"),
    "windows": ("check", "windows-latest"),
}


def jobs(text: str) -> dict[str, str]:
    """Each job under `jobs:` with its body text."""
    out: dict[str, str] = {}
    cur = None
    inside = False
    for line in text.splitlines():
        if re.match(r"^jobs:\s*$", line):
            inside = True
            continue
        if inside and re.match(r"^\S", line):
            inside = False
        if not inside:
            continue
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if m:
            cur = m.group(1)
            out[cur] = ""
        elif cur:
            out[cur] += line + "\n"
    return out


def problems(repo: Path) -> list[str]:
    wf = repo / WF
    bad = []
    ci = jobs((wf / "ci.yml").read_text())
    calls = [
        j
        for j, body in ci.items()
        if re.search(
            r"^    uses: \./\.github/workflows/lint-and-test\.yml\s*$", body, re.M
        )
    ]
    if len(ci) != 1 or len(calls) != 1:
        extra = sorted(set(ci) - set(calls))
        bad.append(
            f"ci.yml: one job, the lint-and-test.yml call, expected; has {sorted(ci)}. "
            f"Move {extra or 'the jobs'} into lint-and-test.yml under a tier"
        )
    tiers = jobs((wf / "lint-and-test.yml").read_text())
    release = jobs((wf / "release-check.yml").read_text())
    for j, body in tiers.items():
        if not re.search(r"^    if: \$?\{?\{? ?inputs\.full", body, re.M):
            continue
        if j not in COVERS:
            bad.append(
                f"lint-and-test.yml: full-tier job {j} has no release-check cover in "
                f"scripts/lint/ci-check.py COVERS, so a tag could ship without it"
            )
            continue
        rj, os_ = COVERS[j]
        if rj not in release or os_ not in release[rj]:
            bad.append(
                f"release-check.yml: job {rj} does not run {os_}, which covers {j}"
            )
    for j in COVERS:
        if j not in tiers:
            bad.append(
                f"scripts/lint/ci-check.py COVERS names {j}, not a lint-and-test.yml job"
            )
    for j, body in release.items():
        if "make check" in body and not re.search(
            r"^\s+fetch-depth: 0\s*$", body, re.M
        ):
            bad.append(
                f"release-check.yml: job {j} runs make check on a shallow checkout; "
                f"add fetch-depth: 0 to its checkout"
            )
    return bad


CALL = "  t:\n    uses: ./.github/workflows/lint-and-test.yml\n"
FAST = "  deny:\n    runs-on: ubuntu-latest\n"
MAC = "  check-macos:\n    if: inputs.full\n    runs-on: macos-latest\n"
WIN = "  windows:\n    if: inputs.full\n    runs-on: windows-latest\n"
REL = (
    "  check:\n    strategy:\n      matrix:\n"
    "        os: [ubuntu-22.04, macos-latest, windows-latest]\n"
)
GATE = (
    "  gate:\n    steps:\n      - uses: actions/checkout@x\n        with:\n"
    "{depth}      - run: make check\n"
)


def fixture(d: Path, ci: str, tiers: str, rel: str, tools: str = "") -> Path:
    if tools:
        (d / "schwarzwald/config").mkdir(parents=True)
        (d / "schwarzwald/config/lint.toml").write_text(
            '[changelog]\ngit_cliff = "2.14.2"\n'
        )
        (d / ".github/actions/lint-tools").mkdir(parents=True)
        (d / ".github/actions/lint-tools/action.yml").write_text(tools)
    (d / WF).mkdir(parents=True)
    for name, body in (
        ("ci.yml", ci),
        ("lint-and-test.yml", tiers),
        ("release-check.yml", rel),
    ):
        (d / WF / name).write_text(f"name: x\non: push\njobs:\n{body}")
    return d


def self_test() -> int:
    cases = [
        (CALL, FAST + MAC + WIN, REL, False),
        # A new job added only to ci.yml.
        (CALL + FAST, FAST + MAC + WIN, REL, True),
        # A new full-tier job no release check covers.
        (CALL, FAST + MAC + WIN + "  msrv:\n    if: inputs.full\n", REL, True),
        # The release check stops running Windows.
        (CALL, FAST + MAC + WIN, REL.replace(", windows-latest", ""), True),
        # A cover for a job that is gone.
        (CALL, FAST + MAC, REL, True),
        # A gate job running make check on a shallow checkout.
        (CALL, FAST + MAC + WIN, REL + GATE.format(depth=""), True),
        (
            CALL,
            FAST + MAC + WIN,
            REL + GATE.format(depth="          fetch-depth: 0\n"),
            False,
        ),
    ]
    for i, (ci, tiers, rel, red, *tools) in enumerate(cases):
        with tempfile.TemporaryDirectory() as t:
            got = problems(fixture(Path(t), ci, tiers, rel, *tools))
        if bool(got) != red:
            print(
                f"ci-check self-test: case {i} {'passed' if red else 'failed'}: {got}"
            )
            return 1
    print("ci-check self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    repo = (
        Path(sys.argv[sys.argv.index("--repo") + 1]) if "--repo" in sys.argv else ROOT
    )
    bad = problems(repo)
    for b in bad:
        print(f"ci-check: {b}", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
