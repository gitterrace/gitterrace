#!/usr/bin/env python3
"""CONV-ACTIONS-PIN: third-party CI inputs are pinned by content, not by a movable name.

  1. Every `uses:` in a workflow or composite action (`.github/**.yml`, `ci/**.yml` at any
     depth) names a full 40-hex commit sha and carries a version comment (`# v4.2.2`, `# v1`).
     A flow mapping (`- {uses: x/y@main}`) counts. Local actions (`./...`) are exempt.
  2. Every `FROM` in a Dockerfile (`Dockerfile`, `*.Dockerfile`, `Dockerfile.*`) names its image
     by `@sha256:` digest (`scratch` excepted).
  3. Every Dockerfile RUN and every workflow `run:` block that downloads with `curl` has one
     sha256 digest handed to `sha256sum -c` per download.

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
# `uses:` as a block key or inside a flow mapping, the ref unquoted or quoted.
USES = re.compile(r"""(?:^|[{,\s])uses:\s*["']?([^\s"',}]+)["']?\s*[,}]*(.*)$""")
SHA = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
COMMENT = re.compile(r"^\s*#\s*v?\d+(?:\.\d+)*\b")  # a version, never a branch name
RUN_KEY = re.compile(r"^(\s*)(?:-\s+)?run:\s*(.*)$")
FROM = re.compile(r"^FROM\s+(?:--\S+\s+)?(\S+)", re.M)
RUN = re.compile(r"^RUN\b(?:[^\n]*\\\n)*[^\n]*", re.M)


def downloads(rel: str, n: int, body: str) -> list[str]:
    # A check is a 64-hex digest the step hands to `sha256sum -c`: one digest per download.
    fetches = len(re.findall(r"\bcurl\b", body))
    sums = len(re.findall(r"\b[0-9a-f]{64}\b", body)) if re.search(r"\bsha256sum\s+(?:-\S*c|--check)", body) else 0
    if fetches and re.search(r"https?://", body) and sums < fetches:
        return [f"{rel}:{n}: {fetches} downloads, {sums} sha256 checks"]
    return []


def run_blocks(text: str) -> list[tuple[int, str]]:
    """Each `run:` value with its line: the inline value, or the indented block below `|`/`>`."""
    lines, out, i = text.splitlines(), [], 0
    while i < len(lines):
        m = RUN_KEY.match(lines[i])
        i += 1
        if not m:
            continue
        start, body = i, [m.group(2)]
        if m.group(2).strip()[:1] in ("|", ">"):
            indent = len(m.group(1)) + 1
            while i < len(lines) and (not lines[i].strip() or len(lines[i]) - len(lines[i].lstrip()) > indent):
                body.append(lines[i])
                i += 1
        out.append((start, "\n".join(body)))
    return out


def workflow_problems(rel: str, text: str) -> list[str]:
    out = []
    for n, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("#"):
            continue
        m = USES.search(line)
        if not m or m.group(1).startswith("./"):
            continue
        ref, rest = m.groups()
        if not SHA.match(ref):
            out.append(f"{rel}:{n}: {ref} is not pinned by a full commit sha")
        elif not COMMENT.match(rest):
            out.append(f"{rel}:{n}: {ref} lacks a version comment (# v1.2.3)")
    for n, body in run_blocks(text):
        out += downloads(rel, n, body)
    return out


def docker_problems(rel: str, text: str) -> list[str]:
    out = []
    for m in FROM.finditer(text):
        ref = m.group(1)
        if ref != "scratch" and "@sha256:" not in ref:
            out.append(f"{rel}: FROM {ref} is not pinned by a sha256 digest")
    for m in RUN.finditer(text):
        out += downloads(rel, text.count(chr(10), 0, m.start()) + 1, m.group(0))
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
            (docker_problems, f"FROM ubuntu@sha256:{sha}\nRUN curl -fsSL https://e.org/x \\\n && echo '{'0' * 64}  x' | sha256sum -c -\n")]
    ok = all(fn("t", t) for fn, t in bad) and not any(fn("t", t) for fn, t in good)
    return config.self_check(RULE, ok, "planted unpinned inputs must fail and pinned ones pass")


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    bad, seen = [], 0
    for rel in config.tracked(root, cfg["scope"]["exclude"]):
        wf = rel.endswith((".yml", ".yaml")) and (rel.startswith((".github/", "ci/")) or "/ci/" in rel)
        if not (wf or is_docker(rel)):
            continue
        seen += 1
        text = (root / rel).read_text(encoding="utf-8", errors="replace")
        bad += workflow_problems(rel, text) if wf else docker_problems(rel, text)
    return config.report(RULE, bad, f"{len(bad)} unpinned input(s)", f"every input pinned ({seen} files)",
                         "Pin by full sha (with a version comment) or sha256 digest.")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
