#!/usr/bin/env python3
"""CONV-PERSONAL-DATA: the tree and the commit messages carry no private details.

Three sources of patterns:
  1. `rules/deny.toml` [incident] and [host]: incident stories with a date ("crash 2026-09-07"),
     the owner's own data, and `.local` mDNS hostnames. A test states the invariant.
  2. The private denylist (lint.toml [denylist] path, outside every repository): one
     case-insensitive regex per line (a person or agent name, a hostname), `#` for comments. Bare six-hex lines are
     ids and belong to CONV-NO-INTERNAL-REFS. Without the file only the generic patterns run.
  3. Every tracked file, binaries included: an absolute home path (`/Users/<name>/`,
     `/home/<name>/`) names the machine that wrote it. Placeholder homes pass, plus the
     product's `[personal] placeholder_homes`. Write `$HOME` or `~` instead.
Paths in `[scope] exclude` and `[personal] exempt` are not scanned.

  personal-data.py [--repo DIR]                  the tree
  personal-data.py [--repo DIR] --messages BASE  subject and body of every commit in BASE..HEAD
  personal-data.py [--repo DIR] --message FILE   one commit message (the commit-msg hook)
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-PERSONAL-DATA"
# Placeholder names, single letters and dot-dirs are what docs, fixtures and tests write on
# purpose. `linuxbrew` is the Homebrew prefix on Linux, not a person.
PLACEHOLDER_HOMES = (
    "dev", "you", "me", "user", "username", "runner", "name", "someone", "somebody",
    "victim", "attacker", "test", "tester", "example", "alice", "bob", "alex", "robin",
    "linuxbrew",
)  # fmt: skip


def home_path(extra: list[str]) -> re.Pattern[bytes]:
    names = b"|".join(re.escape(n).encode() for n in (*PLACEHOLDER_HOMES, *extra))
    return re.compile(
        # Root-anchored: `$TMP/home/x` or `fixtures/Users/x` is a fake home under another dir.
        rb"(?<![A-Za-z0-9_.}$)/-])"
        rb"/(?:Users|home)/(?!(?:" + names + rb"|[A-Za-z]|\.[A-Za-z0-9_-]+|<[a-z]+>|\$|\{)[/\s\"'])"
        rb"[A-Za-z0-9._-]+(?:/|(?=[\s\"'`;)|&]|$))"
    )


def notes(rel: str, text: str, golden: list[str]) -> str:
    """What of a file is prose: a golden digest file only its `#` note lines."""
    if config.matches(rel, golden) and not rel.endswith(".md"):
        return "\n".join(x if x.startswith("#") else "" for x in text.splitlines())
    return text


class Personal:
    def __init__(self, cfg: dict[str, dict], pats: list[re.Pattern[str]]) -> None:
        d = config.deny(cfg["product"]["name"])
        self.lines = d["incident"] + d["host"] + pats
        self.home = home_path(cfg["personal"]["placeholder_homes"])
        self.golden = cfg["scope"]["golden"]
        self.skip = cfg["scope"]["exclude"] + cfg["personal"]["exempt"]

    def line_hit(self, line: str) -> bool:
        line = line[: config.MAX_LINE]
        return any(p.search(line) for p in self.lines) or bool(self.home.search(line.encode()))

    def scan(self, rel: str, data: bytes) -> list[str]:
        hits = []
        m = self.home.search(data)
        if m and not config.matches(rel, self.golden):
            hits.append(f"{rel}: absolute home path {m.group().decode(errors='replace')}")
        if b"\0" in data[:8192]:
            return hits
        text = notes(rel, data.decode(errors="replace"), self.golden)
        return hits + [f"{rel}:{n}: {x.strip()[:100]}" for n, x in enumerate(text.splitlines(), 1) if self.line_hit(x)]

    def message_hits(self, msg: str) -> list[str]:
        lines = config.message_lines(msg)
        subject = next((x for x in lines if x.strip()), "")
        return [f'commit "{subject[:60]}": {x.strip()[:100]}' for x in lines if self.line_hit(x)]


def self_test(cfg: dict[str, dict]) -> bool:
    """Runs on every invocation: a pattern that silently stops matching is a dead guard."""
    p = Personal({**cfg, "personal": config.DEFAULTS["personal"]}, [])
    home = "/" + "Users"
    must_hit = [f'database "{home}/jdoe/Library"', "/home/jdoe/src/x", f"find {home}/jdoe -name x",
                "// served by mac-mini.local", "// a regression 2026-09-07 broke it"]
    must_pass = ['"$TMP/home/Projects/a"', f"{home}/dev/Library", "/home/runner/work", f"{home}/x/y",
                 "/home/linuxbrew/.linuxbrew/bin", "$HOME/Library", f"{home}/<name>/",
                 "~/.local/state/app", "// the regression test"]
    ok = True
    for s in must_hit:
        if not p.line_hit(s):
            print(f"FAIL [{RULE}] self-test: {s!r} must be refused", file=sys.stderr)
            ok = False
    for s in must_pass:
        if p.line_hit(s):
            print(f"FAIL [{RULE}] self-test: {s!r} must pass", file=sys.stderr)
            ok = False
    p.lines.append(re.compile("orca", re.IGNORECASE))
    if not p.line_hit("asked by Orca") or p.scan("t/golden/e.txt", b"cc82 x /?orca\n# blessed\n"):
        print(f"FAIL [{RULE}] self-test: a denylisted name flags, golden digests never", file=sys.stderr)
        ok = False
    return ok


def report(hits: list[str], what: str) -> int:
    if hits:
        print(f"FAIL [{RULE}] {len(hits)} line(s) carry personal data in {what}.")
        for h in hits[:60]:
            print(f"  {h}")
        print("  State the invariant with neutral fixtures. Real names, hosts and ids go in the")
        print("  private denylist, never in a repo. Write $HOME or ~ for a home path.")
        return 1
    print(f"PASS [{RULE}] no personal data in {what}")
    return 0


def main(argv: list[str]) -> int:
    root, args = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test(cfg):
        return 2
    pats, _ = config.denylist(root)
    p = Personal(cfg, pats)
    if args[:1] == ["--messages"]:
        return report(config.widened(root, args[1]) + [h for m in config.messages(root, args[1]) for h in p.message_hits(m)],
                      f"the messages of {args[1]}..HEAD")
    if args[:1] == ["--message"]:
        msg = Path(args[1]).read_text(encoding="utf-8", errors="replace")
        return report(p.message_hits(msg), "the commit message")
    names = config.tracked(root, p.skip)
    hits: list[str] = []
    for rel in names:
        f = root / rel
        # A tracked symlink can point at a FIFO or a device: reading it would hang the gate.
        if f.is_symlink() or not f.is_file():
            continue
        try:
            hits += p.scan(rel, f.read_bytes())
        except OSError:
            continue
    if len(names) < cfg["scope"]["min_files"]:
        print(f"FAIL [{RULE}] scanned only {len(names)} file(s): the census is broken", file=sys.stderr)
        return 1
    return report(hits, f"the tree ({len(names)} files)")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
