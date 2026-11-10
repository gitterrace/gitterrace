#!/usr/bin/env python3
"""Every step of `make check` has a landing decision: the gate runs it, or it is skipped with a
reason.

The steps are the prerequisites of the Makefile's `check` target. The gate is
schwarzwald/config/land.toml `[landing] gate`; a step is gated when a gate command is
`make ... <step> ...`. A step the gate leaves out is named in `[landing] skip` with its reason.
So a step added to `make check` cannot reach main unseen by the gate: it is red here until it is
gated or skipped on purpose. Also red: a skip entry for a step `make check` no longer has, an
empty reason, and a step both gated and skipped.

  python3 scripts/lint/gate-check.py [--repo DIR]
  python3 scripts/lint/gate-check.py --self-test   red and green on fixture trees
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parents[2]
MAKE_CMD = re.compile(r"(?:^|[;&|]\s*)make\s+((?:[a-z][a-z0-9-]*\s*)+)")


def check_steps(makefile: str) -> list[str]:
    """The prerequisites of `check:`, backslash continuations joined."""
    text = makefile.replace("\\\n", " ")
    m = re.search(r"^check:([^=\n][^\n]*)$", text, re.M)
    return m.group(1).split() if m else []


def gated(gate: list[str]) -> set[str]:
    return {
        t for cmd in gate for m in MAKE_CMD.finditer(cmd) for t in m.group(1).split()
    }


def problems(repo: Path) -> list[str]:
    steps = check_steps((repo / "Makefile").read_text())
    if not steps:
        return ["Makefile: no `check:` target with prerequisites"]
    landing = tomllib.loads((repo / "schwarzwald/config/land.toml").read_text())[
        "landing"
    ]
    run = gated(landing.get("gate", []))
    skip: dict[str, str] = landing.get("skip", {})
    bad = []
    for s in steps:
        if s in run and s in skip:
            bad.append(f"{s}: both in [landing] gate and skip, pick one")
        elif s not in run and s not in skip:
            bad.append(
                f"{s}: a `make check` step the landing gate never runs. Add `make {s}` to "
                f"[landing] gate, or name it in [landing] skip with the reason"
            )
    for s, why in skip.items():
        if s not in steps:
            bad.append(f"[landing] skip names {s}, which is not a `make check` step")
        elif not str(why).strip():
            bad.append(f"[landing] skip {s}: the reason is empty")
    return bad


def fixture(d: Path, check: str, gate: list[str], skip: dict[str, str]) -> Path:
    (d / "schwarzwald/config").mkdir(parents=True, exist_ok=True)
    (d / "Makefile").write_text(f"check: {check}\n\nfmt:\n\ttrue\n")
    lines = [
        "[landing]",
        "gate = [" + ", ".join(f'"{g}"' for g in gate) + "]",
        "[landing.skip]",
    ]
    lines += [f'{k} = "{v}"' for k, v in skip.items()]
    (d / "schwarzwald/config/land.toml").write_text("\n".join(lines) + "\n")
    return d


def self_test() -> int:
    cases = [
        # (check prerequisites, gate, skip, red?)
        ("fmt \\\n  lint", ["make fmt lint"], {}, False),
        ("fmt lint", ["cargo xtask check && make fmt"], {"lint": "slow"}, False),
        # The bug this guards: a new step the gate never runs.
        ("fmt send", ["make fmt"], {}, True),
        # A stale skip entry for a step that left `make check`.
        ("fmt", ["make fmt"], {"gone": "was slow"}, True),
        ("fmt lint", ["make fmt"], {"lint": " "}, True),
        ("fmt", ["make fmt"], {"fmt": "both"}, True),
    ]
    for i, (check, gate, skip, red) in enumerate(cases):
        with tempfile.TemporaryDirectory() as t:
            got = problems(fixture(Path(t), check, gate, skip))
        if bool(got) != red:
            print(
                f"gate-check self-test: case {i} {'passed' if red else 'failed'}: {got}"
            )
            return 1
    print("gate-check self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    repo = (
        Path(sys.argv[sys.argv.index("--repo") + 1]) if "--repo" in sys.argv else ROOT
    )
    bad = problems(repo)
    for b in bad:
        print(f"gate-check: {b}", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
