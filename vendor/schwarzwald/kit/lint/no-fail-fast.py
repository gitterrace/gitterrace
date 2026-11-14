#!/usr/bin/env python3
"""CONV-NO-FAIL-FAST: a test run lists every failure, not the first.

  1. Every `cargo test` in a Makefile (lint.toml `[scope] make`), a workflow (`.github/**.yml`,
     `ci/**.yml`) or a shell script carries `--no-fail-fast`. `cargo test --doc` too: a failing
     doctest stops the rest the same way. `cargo nextest` is left to its profile
     (`fail-fast = false` in .config/nextest.toml), which this rule reads when present.
  2. Every workflow job with a `strategy:` sets `fail-fast: false`, so one red cell of a matrix
     does not cancel the others.
  3. A workflow `run:` step that calls `make` with two or more targets passes `-k`, so a red
     first gate does not hide the second's result.

  no-fail-fast.py [--repo DIR]
"""

from __future__ import annotations

import re
import shlex
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-NO-FAIL-FAST"
CARGO_TEST = re.compile(r"\bcargo\s+(?:\+\S+\s+)?test\b(?P<rest>[^;&|\n]*)")
STRATEGY = re.compile(r"^(?P<indent>\s*)strategy:\s*$")
FAIL_FAST = re.compile(r"^\s*fail-fast:\s*false\s*$")
RUN_KEY = re.compile(r"^(\s*)(?:-\s+)?run:\s*(.*)$")
MAKE = re.compile(r"(?:^|[\s;&|(])make\b(?P<args>[^;&|\n]*)")
NEXTEST_FAIL_FAST = re.compile(r"^\s*fail-fast\s*=\s*true\b", re.M)


HEREDOC = re.compile(r"<<-?\s*(['\"]?)(?P<tag>[A-Za-z_][A-Za-z0-9_]*)\1")


def joined(text: str) -> list[tuple[int, str]]:
    """Logical lines with their first line number: a trailing backslash continues the line. A
    heredoc's body is data (a guard case table, a message), not commands, so it is left out."""
    out, buf, start, until = [], "", 0, None
    for n, line in enumerate(text.splitlines(), 1):
        if until is not None:
            if line.strip() == until:
                until = None
            continue
        if not buf:
            start = n
        if line.rstrip().endswith("\\"):
            buf += line.rstrip()[:-1] + " "
            continue
        out.append((start, buf + line))
        doc = HEREDOC.search(buf + line)
        if doc:
            until = doc.group("tag")
        buf = ""
    if buf:
        out.append((start, buf))
    return out


def unquoted(line: str) -> str:
    """The line with its quoted strings blanked: `cargo test` inside quotes is data (a guard case's
    JSON, an echo), not a command that runs."""
    return re.sub(r"'[^']*'|\"(?:[^\"\\\\]|\\\\.)*\"", lambda q: " " * len(q.group(0)), line)


def cargo_test_problems(rel: str, text: str) -> list[str]:
    out = []
    for n, line in joined(text):
        if line.lstrip().startswith("#"):
            continue
        for m in CARGO_TEST.finditer(unquoted(line)):
            # Arguments after `--` go to the test binary, not to cargo.
            cargo_args = m.group("rest").split(" -- ")[0]
            if "--no-fail-fast" not in cargo_args:
                out.append(f"{rel}:{n}: cargo test without --no-fail-fast")
    return out


def strategy_problems(rel: str, text: str) -> list[str]:
    lines, out = text.splitlines(), []
    for i, line in enumerate(lines):
        m = STRATEGY.match(line)
        if not m:
            continue
        indent = len(m.group("indent"))
        block = []
        for nxt in lines[i + 1:]:
            if nxt.strip() and len(nxt) - len(nxt.lstrip()) <= indent:
                break
            block.append(nxt)
        if not any(FAIL_FAST.match(b) for b in block):
            out.append(f"{rel}:{i + 1}: a strategy without fail-fast: false")
    return out


def make_problems(rel: str, text: str) -> list[str]:
    out = []
    for n, line in enumerate(text.splitlines(), 1):
        m = RUN_KEY.match(line)
        if not m:
            continue
        for mk in MAKE.finditer(m.group(2)):
            try:
                # Shell words: a quoted VAR="a b" or ${{ matrix.x }} is one word, never a target.
                words = shlex.split(mk.group("args"), comments=True)
            except ValueError:
                continue
            targets = [w for w in words if not w.startswith("-") and "=" not in w]
            flags = [w for w in words if w.startswith("-")]
            keeps_going = any(f == "-k" or f == "--keep-going" or (f.startswith("-") and not f.startswith("--") and "k" in f[1:]) for f in flags)
            if len(targets) >= 2 and not keeps_going:
                out.append(f"{rel}:{n}: make with {len(targets)} targets and no -k")
    return out


def self_test() -> bool:
    red_make = "test:\n\tcargo test --workspace || rc=$$?; \\\n\t  echo\n"
    red_cont = "test:\n\tcargo test \\\n\t  --workspace\n"
    green_make = "test:\n\tcargo test --workspace --no-fail-fast || rc=$$?\n\tcargo test -p x --no-fail-fast -- --test-threads=1\n"
    red_after = "test:\n\tcargo test -p x -- --no-fail-fast\n"
    red_wf = "jobs:\n  a:\n    strategy:\n      matrix:\n        os: [a, b]\n    steps:\n      - run: make lint test\n"
    quoted_ok = "t ALLOW x bash '{\"command\":\"cargo test -p a\"}'\n      - run: make pkg V=\"${{ matrix.n }}\" N=\"a b\"\n"
    heredoc_ok = "while read c; do run \"$c\"; done <<'EOF'\ncargo test -p core@REPO\nEOF\ncargo test --no-fail-fast\n"
    heredoc_red = "cat <<EOF\nx\nEOF\ncargo test -p core\n"
    green_wf = "jobs:\n  a:\n    strategy:\n      fail-fast: false\n      matrix:\n        os: [a, b]\n    steps:\n      - run: make -k lint test\n      - run: make check\n"
    ok = (
        bool(cargo_test_problems("M", red_make)) and bool(cargo_test_problems("M", red_cont))
        and bool(cargo_test_problems("M", red_after)) and not cargo_test_problems("M", green_make)
        and bool(strategy_problems("w", red_wf)) and bool(make_problems("w", red_wf))
        and not strategy_problems("w", green_wf) and not make_problems("w", green_wf)
        and not cargo_test_problems("s", quoted_ok) and not make_problems("w", quoted_ok)
        and not cargo_test_problems("s", heredoc_ok) and bool(cargo_test_problems("s", heredoc_red))
    )
    if not ok:
        print(f"FAIL [{RULE}] self-test: planted fail-fast runs must fail and the clean ones pass", file=sys.stderr)
    return ok


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    bad, seen = [], 0
    nextest = root / ".config/nextest.toml"
    if nextest.is_file() and NEXTEST_FAIL_FAST.search(nextest.read_text(encoding="utf-8", errors="replace")):
        bad.append(".config/nextest.toml: fail-fast = true")
    for rel in config.tracked(root, cfg["scope"]["exclude"]):
        is_make = config.matches(rel, cfg["scope"]["make"])
        is_wf = rel.endswith((".yml", ".yaml")) and (rel.startswith((".github/", "ci/")) or "/ci/" in rel)
        is_sh = rel.endswith(".sh")
        if not (is_make or is_wf or is_sh):
            continue
        seen += 1
        text = (root / rel).read_text(encoding="utf-8", errors="replace")
        bad += cargo_test_problems(rel, text)
        if is_wf:
            bad += strategy_problems(rel, text) + make_problems(rel, text)
    if bad:
        print(f"FAIL [{RULE}] {len(bad)} run(s) stop at the first failure")
        for b in bad[:60]:
            print(f"  {b}")
        print("  Add --no-fail-fast to cargo test, fail-fast: false to the strategy, -k to the make.")
        return 1
    print(f"PASS [{RULE}] every test run lists every failure ({seen} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
