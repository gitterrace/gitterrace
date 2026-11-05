#!/usr/bin/env python3
"""Every third-party `uses:` in workflows and composite actions is pinned by a full commit sha
with a version comment. Local (./) actions are exempt.

The Dockerfiles under ci/ follow the same rule: each FROM names its image by sha256 digest, and
each `curl` that downloads a file also has its own sha256 check in the same RUN, so nothing unpinned is
fetched at build time.

  python3 scripts/actions-pin-check.py
  python3 scripts/actions-pin-check.py --self-test   each rule goes red on a planted offender
"""

from __future__ import annotations

import pathlib
import re
import sys
from collections.abc import Callable

USES = re.compile(r"^\s*(?:-\s*)?uses:\s*(\S+)(.*)$")
SHA = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
COMMENT = re.compile(r"^\s*#\s*\S+")


def problems(path: pathlib.Path | str, text: str) -> list[str]:
    out: list[str] = []
    for n, line in enumerate(text.splitlines(), 1):
        m = USES.match(line)
        if not m or m.group(1).startswith("./"):
            continue
        ref, rest = m.groups()
        if not SHA.match(ref):
            out.append(f"{path}:{n}: {ref} is not pinned by a full commit sha")
        elif not COMMENT.match(rest):
            out.append(f"{path}:{n}: {ref} lacks a version comment")
    return out


FROM = re.compile(r"^FROM\s+(?:--\S+\s+)?(\S+)", re.M)


def docker_problems(path: pathlib.Path | str, text: str) -> list[str]:
    """Unpinned `FROM` lines and downloads with no checksum in the same RUN instruction."""
    out: list[str] = []
    for m in FROM.finditer(text):
        ref = m.group(1)
        if ref != "scratch" and "@sha256:" not in ref:
            out.append(f"{path}: FROM {ref} is not pinned by a sha256 digest")
    # A RUN instruction ends at the first line that does not continue with a backslash.
    for m in re.finditer(r"^RUN\b(?:[^\n]*\\\n)*[^\n]*", text, re.M):
        run = m.group(0)
        fetches = len(re.findall(r"\bcurl\b", run))
        sums = len(re.findall(r"\bsha256sum\b", run))
        if fetches and re.search(r"https?://", run) and sums < fetches:
            n = text.count("\n", 0, m.start()) + 1
            out.append(
                f"{path}:{n}: {fetches} downloads but {sums} sha256 checks, each needs its own"
            )
    return out


def self_test() -> int:
    sha = "a" * 40
    cases: list[tuple[Callable[[str, str], list[str]], str, str]] = [
        (problems, "uses: x/y@v4\n", "not pinned"),
        (problems, f"uses: x/y@{sha}\n", "lacks a version comment"),
        (docker_problems, "FROM ubuntu:24.04\n", "not pinned by a sha256 digest"),
        (
            docker_problems,
            "RUN curl -fsSL https://example.org/x -o /tmp/x\n",
            "1 downloads but 0 sha256",
        ),
        (
            docker_problems,
            "RUN curl -fsSL https://e.org/a -o a \\\n && curl -fsSL https://e.org/b -o b \\\n"
            " && echo 'x  a' | sha256sum -c -\n",
            "2 downloads but 1 sha256",
        ),
    ]
    for fn, text, want in cases:
        if not any(want in p for p in fn("t", text)):
            print(f"self-test: {text.strip()!r} was not reported as '{want}'")
            return 1
    good: list[tuple[Callable[[str, str], list[str]], str]] = [
        (problems, f"uses: x/y@{sha} # v4\n"),
        (
            docker_problems,
            f"FROM ubuntu@sha256:{sha}\nRUN curl -fsSL https://e.org/x \\\n && sha256sum -c -\n",
        ),
        (
            docker_problems,
            "RUN curl -fsSL https://e.org/a -o a && echo 'x  a' | sha256sum -c - \\\n"
            " && curl -fsSL https://e.org/b -o b && echo 'y  b' | sha256sum -c -\n",
        ),
    ]
    for fn, text in good:
        if fn("t", text):
            print(f"self-test: a pinned input was reported: {fn('t', text)}")
            return 1
    print("actions-pin self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    root = pathlib.Path(__file__).resolve().parent.parent
    gh = root / ".github"
    files = sorted([*gh.rglob("*.yml"), *gh.rglob("*.yaml")])
    bad = [p for f in files for p in problems(f.relative_to(root), f.read_text())]
    for f in sorted((root / "ci").glob("*.Dockerfile")):
        bad += docker_problems(f.relative_to(root), f.read_text())
    print("\n".join(bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
