#!/usr/bin/env python3
"""The CI tiers hold: every job runs on each pull request, or a tag cannot ship without it.

1. ci.yml has one job that tests, a `uses:` of lint-and-test.yml. A job added straight to ci.yml
   would dodge the tiers, so it is red here. Beside it may stand a job gated on the schedule
   alone that runs no make, cargo or workflow: it only decides whether the nightly run goes.
2. A lint-and-test.yml job gated `if: inputs.full` (the full tier, nightly and manual runs only)
   must be in COVERS, naming the release-check.yml job that runs the same OS again before dist
   publishes, and that job's matrix must hold that OS. Every other job is the fast tier, which
   runs on every pull request and push.
3. COVERS names no job lint-and-test.yml lacks.
4. Every release-check.yml job that runs `make check` checks out with `fetch-depth: 0`: the
   history lints refuse a shallow clone.
5. The git-cliff version in lint.toml is the one the lint-tools action downloads, digest-checked,
   for both runner kinds.
6. The release gate runs `make check` with the same arguments as the Linux gate of
   lint-and-test.yml (check-linux): a flag only CI sets, such as the Windows cross-check, would
   let a tag skip a check main passed, or fail on a step main never ran.
7. Every `run: |` block of a composite action under .github/actions parses under shellcheck
   (errors only). actionlint checks workflow steps but not composite actions, so a syntax
   error there, or a comment starting `# shellcheck` that reads as a broken directive, would
   first show on a runner. Skipped with a note when shellcheck is not installed.

Read with a small indentation reader (2-space job keys under `jobs:`), no YAML library.

  python3 scripts/lint/ci-check.py [--repo DIR]
  python3 scripts/lint/ci-check.py --self-test
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WF = ".github/workflows"
# Full-tier job -> (release-check job, the OS its matrix must hold).
COVERS = {
    "check-macos": ("check", "macos-latest"),
    "windows": ("check", "windows-latest"),
    # The memory budget runs on the full tier only: the release check runs it on Linux.
    "memory": ("memory", "ubuntu-latest"),
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
    # A job that runs on the schedule alone and only decides whether the nightly run goes ahead
    # tests nothing, so it cannot dodge a tier.
    gates = [
        j
        for j, body in ci.items()
        if j not in calls
        and re.search(r"^    if: github\.event_name == 'schedule'\s*$", body, re.M)
        and "uses:" not in body.replace("uses: actions/", "")
        and "make " not in body
        and "cargo " not in body
    ]
    if len(ci) - len(gates) != 1 or len(calls) != 1:
        extra = sorted(set(ci) - set(calls) - set(gates))
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
    bad += gate_problems(tiers, release)
    bad += cliff_problems(repo)
    bad += action_shell_problems(repo)
    return bad


def run_blocks(text: str) -> list[str]:
    """The bodies of `run: |` blocks, dedented by their first line."""
    out = []
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if not re.match(r"^\s*(- )?run: \|\s*$", line):
            continue
        body, indent = [], None
        for nxt in lines[i + 1 :]:
            if nxt.strip() and indent is None:
                indent = len(nxt) - len(nxt.lstrip())
            if nxt.strip() and len(nxt) - len(nxt.lstrip()) < (indent or 0):
                break
            body.append(nxt[indent or 0 :])
        out.append("\n".join(body) + "\n")
    return out


def action_shell_problems(repo: Path) -> list[str]:
    sc = shutil.which("shellcheck")
    if not sc:
        print("ci-check: shellcheck not installed, composite actions not checked")
        return []
    bad = []
    for action in sorted((repo / ".github/actions").glob("*/action.y*ml")):
        for n, block in enumerate(run_blocks(action.read_text())):
            r = subprocess.run(
                [sc, "-s", "bash", "-S", "error", "-f", "gcc", "-"],
                input=block,
                capture_output=True,
                text=True,
                check=False,
            )
            if r.returncode:
                rel = action.relative_to(repo)
                first = r.stdout.strip().splitlines()[:1]
                bad.append(f"{rel}: run block {n + 1} fails shellcheck: {first}")
    return bad


# `make [-k] check ARGS`, the arguments as one string.
MAKE_CHECK = re.compile(r"^\s+- run: make (?:-k )?check\b(.*)$", re.M)


def gate_problems(tiers: dict[str, str], release: dict[str, str]) -> list[str]:
    """Rule 6: each release-check `make check` takes check-linux's arguments."""
    linux = MAKE_CHECK.findall(tiers.get("check-linux", ""))
    if not linux:
        return []
    want = linux[0].split()
    bad = []
    for j, body in release.items():
        for got in MAKE_CHECK.findall(body):
            if sorted(got.split()) != sorted(want):
                bad.append(
                    f"release-check.yml: job {j} runs make check{got}, the Linux gate "
                    f"runs make check {' '.join(want)}: give both the same arguments"
                )
    return bad


def cliff_problems(repo: Path) -> list[str]:
    cfg = repo / "schwarzwald/config/lint.toml"
    tools = repo / ".github/actions/lint-tools/action.yml"
    if not cfg.exists() or not tools.exists():
        return []
    m = re.search(r'^git_cliff\s*=\s*"([^"]*)"', cfg.read_text(), re.M)
    if not m or not m.group(1):
        return []
    want, text = m.group(1), tools.read_text()
    found = re.findall(r"git-cliff/releases/download/v([0-9.]+)/", text)
    bad = []
    if want not in found:
        bad.append(
            f"lint-tools action installs no git-cliff {want}, the pin in lint.toml"
        )
    if set(found) - {want}:
        bad.append(
            f"lint-tools action: git-cliff {sorted(set(found) - {want})} is not {want}"
        )
    if len(found) < 2:
        bad.append(
            "lint-tools action: git-cliff needs a pinned download on macOS and Linux"
        )
    if not re.search(r"git-cliff[^\n]*\n[^\n]*\n?[^\n]*(sha256sum|shasum)", text):
        bad.append("lint-tools action: git-cliff download is not digest-checked")
    return bad


CALL = "  t:\n    uses: ./.github/workflows/lint-and-test.yml\n"
NIGHTLY = (
    "  changed:\n    if: github.event_name == 'schedule'\n    runs-on: ubuntu-latest\n"
    "    steps:\n      - run: {run}\n"
)
FAST = "  deny:\n    runs-on: ubuntu-latest\n"
LINUX = (
    "  check-linux:\n    runs-on: ubuntu-latest\n    steps:\n"
    "      - run: make -k check CROSS=x MEMORY=0\n"
)
MAC = "  check-macos:\n    if: inputs.full\n    runs-on: macos-latest\n"
WIN = (
    "  windows:\n    if: inputs.full\n    runs-on: windows-latest\n"
    "  memory:\n    if: inputs.full\n    runs-on: ubuntu-latest\n"
)
REL = (
    "  check:\n    strategy:\n      matrix:\n"
    "        os: [ubuntu-22.04, macos-latest, windows-latest]\n"
    "  memory:\n    runs-on: ubuntu-latest\n"
)
TOOLS = (
    "runs:\n  steps:\n    - run: |\n"
    "        curl -o a https://github.com/orhun/git-cliff/releases/download/v{v}/a.tgz\n"
    "        sha256sum -c x\n"
    "        curl -o b https://github.com/orhun/git-cliff/releases/download/v{v}/b.tgz\n"
    "        shasum -a 256 -c x\n"
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


def shell_self_test() -> int:
    if not shutil.which("shellcheck"):
        print(
            "ci-check self-test: shellcheck not installed, action shell cases skipped"
        )
        return 0
    ok = "runs:\n  steps:\n    - run: |\n        # The tool release\n        echo hi\n"
    for body, red in (
        (ok, False),
        # A comment that reads as a shellcheck directive.
        (ok.replace("The tool release", "shellcheck from its release"), True),
        # A syntax error.
        (ok.replace("echo hi", "if true; then echo hi"), True),
    ):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t) / ".github/actions/x"
            d.mkdir(parents=True)
            (d / "action.yml").write_text(body)
            got = action_shell_problems(Path(t))
        if bool(got) != red:
            print(f"ci-check self-test: action shell case {body!r}: {got}")
            return 1
    return 0


def self_test() -> int:
    cases = [
        (CALL, FAST + MAC + WIN, REL, False),
        # A new job added only to ci.yml.
        (CALL + FAST, FAST + MAC + WIN, REL, True),
        # A nightly gate that only decides whether to run passes; one that runs the tests,
        # or is not gated on the schedule, is a job dodging the tiers.
        (CALL + NIGHTLY.format(run="gh run list"), FAST + MAC + WIN, REL, False),
        (CALL + NIGHTLY.format(run="cargo test"), FAST + MAC + WIN, REL, True),
        (
            CALL + NIGHTLY.format(run="gh run list").replace("'schedule'", "'push'"),
            FAST + MAC + WIN,
            REL,
            True,
        ),
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
        # The release gate drops a flag the Linux gate sets (the Windows cross-check).
        (
            CALL,
            LINUX + MAC + WIN,
            REL
            + GATE.format(depth="          fetch-depth: 0\n").replace(
                "make check", "make -k check MEMORY=0"
            ),
            True,
        ),
        (
            CALL,
            LINUX + MAC + WIN,
            REL
            + GATE.format(depth="          fetch-depth: 0\n").replace(
                "make check", "make -k check MEMORY=0 CROSS=x"
            ),
            False,
        ),
    ]
    cases += [
        # The git-cliff pin drifts from lint.toml, or only one runner kind installs it.
        (CALL, FAST + MAC + WIN, REL, True, TOOLS.format(v="2.10.0")),
        (
            CALL,
            FAST + MAC + WIN,
            REL,
            True,
            TOOLS.format(v="2.14.2").split("        curl -o b")[0],
        ),
        (CALL, FAST + MAC + WIN, REL, False, TOOLS.format(v="2.14.2")),
    ]
    for i, (ci, tiers, rel, red, *tools) in enumerate(cases):
        with tempfile.TemporaryDirectory() as t:
            got = problems(fixture(Path(t), ci, tiers, rel, *tools))
        if bool(got) != red:
            print(
                f"ci-check self-test: case {i} {'passed' if red else 'failed'}: {got}"
            )
            return 1
    if shell_self_test():
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
