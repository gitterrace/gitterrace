#!/usr/bin/env python3
"""CONV-PERSONAL-DATA: the tree and the commit messages carry no private details.

Ported from tendir, one scan with three sources of patterns:
  1. A committed generic pattern for incident stories in comments and test names: "crash
     2026-09-07", "regression 2026-01-02". A test states the invariant, not the day it broke.
  2. A LOCAL, gitignored denylist, `.personal-denylist` at the repository root: one
     case-insensitive regex per line (an agent or person name, a hostname, a real tracker
     id), `#` for comments. Without the file only the generic patterns run, so a fresh clone
     passes. The identities themselves never get committed.
  3. Every tracked file, binaries included: a builder's absolute home path (`/Users/<name>/`,
     `/home/<name>/`) names the machine that wrote it. Placeholder homes pass. Write `$HOME` or
     `~` instead. A `.local` mDNS hostname names it too.

  personal-data.py                 the tree
  personal-data.py --messages BASE subject and body of every commit in BASE..HEAD
  personal-data.py --message FILE  one commit message (the commit-msg hook)
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RULE = "CONV-PERSONAL-DATA"

GENERIC = re.compile(
    r"(?i)\b(crash|incident|outage|regression|hang|trap)\s+(of\s+|on\s+)?20\d{2}-\d{2}-\d{2}"
    r"|\bthe\s+owner[\u2019']s\s+(actual|real)\s+(repo|repository|data|machine)"
)
HOST = re.compile(r"\b[a-z0-9][\w-]*\.local\b(?!/)")
SKIP = (
    "target/",
    "static/vendor/",
    "theme/schwarzwald/",
    "scripts/lint/personal-data.py",
)


def notes(rel: str, text: str) -> str:
    """What of a file is prose: a golden digest file only its `#` note lines."""
    if "/golden/" in rel and not rel.endswith(".md"):
        return "\n".join(x if x.startswith("#") else "" for x in text.splitlines())
    return text


# Placeholder names, single letters and dot-dirs are what docs, fixtures and tests write on
# purpose. `linuxbrew` is the Homebrew prefix on Linux, not a person.
PLACEHOLDER_HOMES = (
    "dev", "you", "me", "user", "username", "runner", "name", "someone", "somebody",
    "victim", "attacker", "test", "tester", "example", "alice", "bob", "alex", "robin",
    "linuxbrew",
)  # fmt: skip
HOME_PATH = re.compile(
    # Root-anchored: `$TMP/home/x` or `fixtures/Users/x` is a fake home under another dir.
    rb"(?<![A-Za-z0-9_.}$)/-])"
    rb"/(?:Users|home)/(?!(?:"
    + b"|".join(n.encode() for n in PLACEHOLDER_HOMES)
    + rb"|[A-Za-z]|\.[A-Za-z0-9_-]+|<[a-z]+>|\$|\{)[/\s\"'])"
    rb"[A-Za-z0-9._-]+(?:/|(?=[\s\"'`;)|&]|$))"
)


def git(*args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(ROOT), *args], capture_output=True, text=True, check=True
    ).stdout


def tracked() -> list[str]:
    out = git("ls-files", "-z", "--cached", "--others", "--exclude-standard")
    return [r for r in out.split("\0") if r and not any(s in r for s in SKIP)]


def denylist() -> list[re.Pattern[str]]:
    path = ROOT / ".personal-denylist"
    if not path.exists():
        return []
    pats = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        # A bare six-hex line is a tracker id: CONV-NO-INTERNAL-REFS matches those as tokens.
        if line and not line.startswith("#") and not re.fullmatch(r"[0-9a-f]{6}", line):
            pats.append(re.compile(line, re.IGNORECASE))
    return pats


def line_hit(line: str, pats: list[re.Pattern[str]]) -> bool:
    return any(p.search(line) for p in (GENERIC, HOST, *pats)) or bool(
        HOME_PATH.search(line.encode())
    )


def self_test() -> bool:
    """Runs on every invocation: a pattern that silently stops matching is a dead guard."""
    must_hit = (
        b'database "/Users/jdoe/Library/Developer"',
        b'ORG = Path("/home/jdoe/src/plugins")',
        b"\x00/Users/jdoe/.cargo/registry/src/x.rs\x00",
        b"find /Users/jdoe -name x",
        b'cd "/home/jdoe"',
    )
    must_pass = (
        b'"$TMP/home/Projects/a"',
        b"/Users/dev/Library",
        b"/home/runner/work",
        b"/Users/x/y",
        b"/home/linuxbrew/.linuxbrew/bin",
        b"$HOME/Library",
        b"/Users/<name>/",
    )
    ok = True
    for s in must_hit:
        if not HOME_PATH.search(s):
            print(
                f"FAIL [{RULE}] self-test: {s!r} must count as a home path",
                file=sys.stderr,
            )
            ok = False
    for s in must_pass:
        if HOME_PATH.search(s):
            print(f"FAIL [{RULE}] self-test: {s!r} is a placeholder", file=sys.stderr)
            ok = False
    for s in ("// served by mac-mini.local", "// a regression 2026-09-07 broke it"):
        if not line_hit(s, []):
            print(f"FAIL [{RULE}] self-test: {s!r} must be refused", file=sys.stderr)
            ok = False
    for s in ("~/.local/state/gitterrace", "// the regression test"):
        if line_hit(s, []):
            print(f"FAIL [{RULE}] self-test: {s!r} must pass", file=sys.stderr)
            ok = False
    orca = [re.compile("orca", re.IGNORECASE)]
    golden = "cc82 x [as-is] /?orca\n# blessed by orca\n"
    if notes("t/golden/e.txt", golden).count("orca") != 1:
        print(f"FAIL [{RULE}] self-test: golden notes, never digests", file=sys.stderr)
        ok = False
    if not line_hit("asked by Orca", orca) or line_hit("asked by Orca", []):
        print(
            f"FAIL [{RULE}] self-test: a denylisted name, and only it, flags",
            file=sys.stderr,
        )
        ok = False
    return ok


def scan_tree(pats: list[re.Pattern[str]]) -> tuple[int, list[str]]:
    hits = []
    names = tracked()
    for rel in names:
        try:
            data = (ROOT / rel).read_bytes()
        except OSError:
            continue
        m = HOME_PATH.search(data)
        if m and "/golden/" not in rel:
            hits.append(
                f"{rel}: absolute home path {m.group().decode(errors='replace')}"
            )
        if b"\0" in data[:8192]:
            continue
        text = notes(rel, data.decode(errors="replace"))
        for n, line in enumerate(text.splitlines(), 1):
            if line_hit(line, pats):
                hits.append(f"{rel}:{n}: {line.strip()[:100]}")
    return len(names), hits


def message_hits(msg: str, pats: list[re.Pattern[str]]) -> list[str]:
    lines = [x for x in msg.splitlines() if not x.startswith("#")]
    subject = next((x for x in lines if x.strip()), "")
    return [
        f'commit "{subject[:60]}": {x.strip()[:100]}'
        for x in lines
        if line_hit(x, pats)
    ]


def report(hits: list[str], what: str) -> int:
    if hits:
        print(f"FAIL [{RULE}] {len(hits)} line(s) carry personal data in {what}.")
        for h in hits[:60]:
            print(f"  {h}")
        print(
            "  State the invariant with neutral fixtures. Real names, hosts and ids go in the"
        )
        print(
            "  local .personal-denylist, never in the repo. Write $HOME or ~ for a home path."
        )
        return 1
    print(f"PASS [{RULE}] no personal data in {what}")
    return 0


def main(args: list[str]) -> int:
    if not self_test():
        return 2
    pats = denylist()
    if args[:1] == ["--messages"]:
        out = git("log", "-z", "--format=%B", f"{args[1]}..HEAD")
        hits = [h for m in out.split("\0") if m.strip() for h in message_hits(m, pats)]
        return report(hits, f"the messages of {args[1]}..HEAD")
    if args[:1] == ["--message"]:
        msg = Path(args[1]).read_text(encoding="utf-8", errors="replace")
        return report(message_hits(msg, pats), "the commit message")
    n, hits = scan_tree(pats)
    if n < 100:
        print(
            f"FAIL [{RULE}] scanned only {n} file(s): the census is broken",
            file=sys.stderr,
        )
        return 2
    return report(hits, f"the tree ({n} files)")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
