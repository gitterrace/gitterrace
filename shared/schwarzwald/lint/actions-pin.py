#!/usr/bin/env python3
"""CONV-ACTIONS-PIN: third-party CI inputs are pinned by content, not by a movable name.

  1. Every `uses:` in `.github/**.yml` names a full 40-hex commit sha and carries a version
     comment (`# v4`). Local actions (`./...`) are exempt.
  2. Every `FROM` in a Dockerfile (`Dockerfile`, `*.Dockerfile`, `Dockerfile.*`) names its image
     by `@sha256:` digest (`scratch` excepted).
  3. Every RUN that downloads with `curl` has one `sha256sum` check per download.

  actions-pin.py [--repo DIR]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-ACTIONS-PIN"
USES = re.compile(r"^\s*(?:-\s*)?uses:\s*(\S+)(.*)$")
SHA = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
COMMENT = re.compile(r"^\s*#\s*\S+")
FROM = re.compile(r"^FROM\s+(?:--\S+\s+)?(\S+)", re.M)
RUN = re.compile(r"^RUN\b(?:[^\n]*\\\n)*[^\n]*", re.M)


def workflow_problems(rel: str, text: str) -> list[str]:
    out = []
    for n, line in enumerate(text.splitlines(), 1):
        m = USES.match(line)
        if not m or m.group(1).startswith("./"):
            continue
        ref, rest = m.groups()
        if not SHA.match(ref):
            out.append(f"{rel}:{n}: {ref} is not pinned by a full commit sha")
        elif not COMMENT.match(rest):
            out.append(f"{rel}:{n}: {ref} lacks a version comment")
    return out


def docker_problems(rel: str, text: str) -> list[str]:
    out = []
    for m in FROM.finditer(text):
        ref = m.group(1)
        if ref != "scratch" and "@sha256:" not in ref:
            out.append(f"{rel}: FROM {ref} is not pinned by a sha256 digest")
    for m in RUN.finditer(text):
        run = m.group(0)
        fetches, sums = len(re.findall(r"\bcurl\b", run)), len(re.findall(r"\bsha256sum\b", run))
        if fetches and re.search(r"https?://", run) and sums < fetches:
            out.append(f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: {fetches} downloads, {sums} sha256 checks")
    return out


def is_docker(rel: str) -> bool:
    name = Path(rel).name
    return name == "Dockerfile" or name.endswith(".Dockerfile") or name.startswith("Dockerfile.")


def self_test() -> bool:
    sha = "a" * 40
    bad = [(workflow_problems, "uses: x/y@v4\n"), (workflow_problems, f"uses: x/y@{sha}\n"),
           (docker_problems, "FROM ubuntu:24.04\n"),
           (docker_problems, "RUN curl -fsSL https://e.org/x -o /tmp/x\n")]
    good = [(workflow_problems, f"uses: x/y@{sha} # v4\n"), (workflow_problems, "uses: ./local\n"),
            (docker_problems, f"FROM ubuntu@sha256:{sha}\nRUN curl -fsSL https://e.org/x \\\n && sha256sum -c -\n")]
    ok = all(fn("t", t) for fn, t in bad) and not any(fn("t", t) for fn, t in good)
    if not ok:
        print(f"FAIL [{RULE}] self-test: planted unpinned inputs must fail and pinned ones pass", file=sys.stderr)
    return ok


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    bad, seen = [], 0
    for rel in config.tracked(root, cfg["scope"]["exclude"]):
        wf = rel.startswith(".github/") and rel.endswith((".yml", ".yaml"))
        if not (wf or is_docker(rel)):
            continue
        seen += 1
        text = (root / rel).read_text(encoding="utf-8", errors="replace")
        bad += workflow_problems(rel, text) if wf else docker_problems(rel, text)
    if bad:
        print(f"FAIL [{RULE}] {len(bad)} unpinned input(s)")
        for b in bad[:60]:
            print(f"  {b}")
        print("  Pin by full sha (with a version comment) or sha256 digest.")
        return 1
    print(f"PASS [{RULE}] every input pinned ({seen} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
