#!/usr/bin/env python3
"""CONV-NO-INTERNAL-REFS: no tracker ids, planning labels or process words in the tree or messages.

A reader of the tree cannot open a tracker row. Say what the row says instead. The patterns are
`rules/deny.toml` ([task_id], [plan], [label], [process], messages add [message]), plus the
product's `[refs] patterns` from lint.toml. Real ids come from the local `.personal-denylist`
(its bare six-hex lines), refused as bare tokens.

Scope, from lint.toml [scope]: markdown read whole (`prose`), comments and ignore reasons in
code (`code`), `## ` help lines of a Makefile (`make`), only the `#` note lines of golden files
(`golden`). Files under `design` also carry no work breakdown, owner or estimate section.

  internal-refs.py [--repo DIR] [<file>...]       the tree, or only these files
  internal-refs.py [--repo DIR] --messages BASE   subject and body of every commit in BASE..HEAD
  internal-refs.py [--repo DIR] --message FILE    one commit message (the commit-msg hook)
The self-test runs on every invocation: a pattern that stops matching is a dead guard.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-NO-INTERNAL-REFS"
COMMENT = re.compile(r"(?:^|\s)(?://+!?|#)(?=\s|$)(.*)$|^\s*\*(?!/)\s?(.*)$")
IGNORE = re.compile(r'#\[ignore\s*=\s*"([^"]*)"')
HEX6 = re.compile(r"\b[0-9a-f]{6}\b")


class Refs:
    def __init__(self, cfg: dict[str, dict], ids: set[str]) -> None:
        d = config.deny(cfg["product"]["name"])
        extra = [re.compile(p) for p in cfg["refs"]["patterns"]]
        self.tree = d["task_id"] + d["plan"] + d["label"] + d["process"] + extra
        self.message = self.tree + d["message"]
        self.breakdown = d["design_breakdown"]
        self.not_id = d["not_id"] + [re.compile(p) for p in cfg["refs"]["not_id"]]
        self.scope = cfg["scope"]
        self.ids = ids

    def hit(self, text: str, pats: list[re.Pattern[str]]) -> bool:
        for n in self.not_id:
            text = n.sub("", text)
        return any(p.search(text) for p in pats) or any(t in self.ids for t in HEX6.findall(text))

    def kind(self, path: str) -> str | None:
        s = self.scope
        if config.matches(path, s["exclude"]):
            return None
        if config.matches(path, s["golden"]):
            return "prose" if path.endswith(".md") else "golden"
        for k in ("prose", "make", "code"):
            if config.matches(path, s[k]):
                return k
        return None

    def scan(self, path: str, body: str) -> list[str]:
        kind = self.kind(path)
        if not kind:
            return []
        design = config.matches(path, self.scope["design"])
        bad = []
        for n, line in enumerate(body.splitlines(), 1):
            if design and any(p.search(line) for p in self.breakdown):
                bad.append(f"{path}:{n}: a design doc section on who, when or how long: {line}")
            elif any(self.hit(t, self.tree) for t in texts(kind, line)):
                bad.append(f"{path}:{n}: {line.strip()[:140]}")
        return bad

    def message_hits(self, msg: str) -> list[str]:
        lines = config.message_lines(msg)
        subject = next((x for x in lines if x.strip()), "")
        return [f'commit "{subject[:60]}": {x.strip()[:100]}' for x in lines if self.hit(x, self.message)]


def texts(kind: str, line: str) -> list[str]:
    if kind == "prose":
        return [line]
    if kind == "make":
        return [line[3:]] if line.startswith("## ") else []
    if kind == "golden":
        return [line] if line.startswith("#") else []
    out = [m.group(1) for m in IGNORE.finditer(line)]
    m = COMMENT.search(line)
    if m:
        out.append(m.group(1) if m.group(1) is not None else m.group(2) or "")
    return out


def self_test(cfg: dict[str, dict]) -> bool:
    r = Refs({**cfg, "scope": config.DEFAULTS["scope"], "refs": config.DEFAULTS["refs"]}, set())
    must_hit = [
        ("a.rs", "// see task 1a2b3c"),
        ("a.rs", "// the coordinator records it"),
        ("a.rs", '#[ignore = "red until 4f4f4a lands: see 4f4f4a"]'),
        ("a.rs", "// moved by G12"),
        ("Makefile", "## x: red until the board says so"),
        ("docs/a.md", "Planned in phase 3."),
        ("docs/design/a.md", "## Work breakdown"),
        ("crates/x/tests/golden/e.txt", "# blessed for task 1a2b3c"),
    ]
    must_pass = [
        ("a.rs", "// the dashboard and the keyboard"),
        ("a.rs", "let g12 = 1; // twelve"),
        ("Makefile", "\t# lane"),
        ("docs/design/a.md", "## On-disk format"),
        ("crates/x/tests/golden/e.txt", "cc82ec7d44f8 ca5f68 [as-is] /?period=1m (#abc123)"),
        ("shared/x/a.md", "the coordinator"),
    ]
    ok = True
    for path, line in must_hit:
        if not r.scan(path, line):
            print(f"FAIL [{RULE}] self-test: {line!r} in {path} must be refused", file=sys.stderr)
            ok = False
    for path, line in must_pass:
        if r.scan(path, line):
            print(f"FAIL [{RULE}] self-test: {line!r} in {path} must pass", file=sys.stderr)
            ok = False
    r.ids = {"9e9e9e"}
    if not r.scan("a.rs", "// the id 9e9e9e"):
        print(f"FAIL [{RULE}] self-test: a denylisted id must flag", file=sys.stderr)
        ok = False
    r.ids = set()
    if r.message_hits("fix(web): hint\n\nThe page reads one rule.\n") or not r.message_hits("fix: x\n\nSee #a1b2c3."):
        print(f"FAIL [{RULE}] self-test: message rules", file=sys.stderr)
        ok = False
    if r.message_hits("fix: x\n# lane 3 in a stripped line"):
        print(f"FAIL [{RULE}] self-test: a stripped comment line is skipped", file=sys.stderr)
        ok = False
    return ok


def report(bad: list[str], what: str) -> int:
    if bad:
        print(f"FAIL [{RULE}] {len(bad)} tracker id, planning label or process word(s) in {what}")
        for b in bad[:60]:
            print(f"  {b}")
        print("  Say what the row says, not its id or label. Ids go on the tracker.")
        return 1
    print(f"PASS [{RULE}] no tracker ids, planning labels or process words in {what}")
    return 0


def main(argv: list[str]) -> int:
    root, args = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test(cfg):
        return 2
    _, ids = config.denylist(root)
    r = Refs(cfg, ids)
    if args[:1] == ["--messages"]:
        return report([h for m in config.messages(root, args[1]) for h in r.message_hits(m)],
                      f"the messages of {args[1]}..HEAD")
    if args[:1] == ["--message"]:
        msg = Path(args[1]).read_text(encoding="utf-8", errors="replace")
        return report(r.message_hits(msg), "the commit message")
    files = args or config.tracked(root, cfg["scope"]["exclude"])
    bad: list[str] = []
    seen = 0
    for f in files:
        p = root / f
        if not r.kind(f) or not p.is_file():
            continue
        seen += 1
        bad += r.scan(f, p.read_text(errors="replace"))
    if not args and seen < cfg["scope"]["min_files"]:
        print(f"FAIL [{RULE}] scanned only {seen} file(s): the census is broken")
        return 2
    return report(bad, f"the tree ({seen} files)")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
