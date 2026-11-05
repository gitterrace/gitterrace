#!/usr/bin/env python3
"""CONV-NO-INTERNAL-REFS: no task ids, planning labels or process words in the tree or messages.

A reader of the tree cannot open a tracker row. Say what the task says instead. Three classes:
  - task ids, only WITH context ("task 1a2b3c", "(#abc123)", "filed as `abc123`", "see
    1a2b3c"), so bare hex, colors and digests pass. Real ids also come from the gitignored
    `.personal-denylist` (CONV-PERSONAL-DATA reads the same file).
  - planning labels: plan/wave/phase/step numbers, lane labels, and the label shapes G12,
    G3b, P4, C5, C5a, rev2, K3, N2.
  - process words: lane, coordinator, board, sign-off, "until the ... land".
Scope: comments and #[ignore] reasons in code, Makefile help, lint rule files, and the
markdown of docs/, bench/, README.md and AGENTS.md, and the notes of golden files (`#` lines,
never the digests). Vendored files apart. Design
docs (docs/design/) also carry no work breakdown, owner or estimate section.

  internal-refs.py [<file>...]        the tree, or only these files
  internal-refs.py --messages BASE    subject and body of every commit in BASE..HEAD
  internal-refs.py --message FILE     one commit message (the commit-msg hook)
The self-test runs on every invocation: a pattern that stops matching is a dead guard.
"""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys

RULE = "CONV-NO-INTERNAL-REFS"
ROOT = pathlib.Path(__file__).resolve().parents[2]

# Ported from tendir: ids only with context.
TASK_ID = (
    r"\b[Tt]asks?[ _]#?[`']?[0-9a-f]{6,}\b|\(#[0-9a-f]{6}\)"
    r"|\([`'][0-9a-f]{6}[`']\)|\b(on|in|per|see|from|closing|filed as) +`[0-9a-f]{6}`"
    r"|\b([Bb]oard|[Rr]ows?|[Ll]anes?|[Ss]ee|[Bb]ehind)[ :]+#?(?=[0-9a-f]*\d)[0-9a-f]{6}\b"
)
NOT_TASK_ID = re.compile(
    r"([Tt]asks?|[Ss]ee|[Rr]ows?)[ _](facade|decade|deface|defaced|accede|beaded|cabbed"
    r"|acceded|debase|deeded|accede)\b"
)
# Ported from tendir: a label needs a number after the word, so "dashboard" and "steps" pass.
PLAN = (
    r"\b[Pp]lan[ #]*[0-9]+\b|\b[Ww]ave[ #]+[0-9]+|\b[Pp]hase[ #]+[0-9]+|\b[Ss]tep[ #]+[0-9]+\b"
    r"|\b[Ss]lice[ #]+[0-9]+|\b[Ll]ane[ #]+([A-Z]|[0-9]+)\b"
)
# gitterrace's own label shapes.
LABEL = r"(?<![%\w])(G[0-9]+[a-z]?|P[1-9]|C[1-9][ab]?|K[1-5]|N[1-4]|rev[0-9]+)\b"
PROCESS = (
    r"\b([Ll]anes?|[Cc]oordinators?|[Bb]oards?|[Ss]ign-?offs?|[Ss]igned[- ]off)\b"
    r"|\buntil the [\w -]{1,40} lands?\b|\bre-bless report\b"
)
REF = re.compile(f"{TASK_ID}|{PLAN}|{LABEL}|{PROCESS}")
# Messages add the hook's bare `#<6 hex>` (ported from tendir's commit-msg): a color has no
# place in a commit message.
MESSAGE_REF = re.compile(
    f"{TASK_ID}|#[0-9a-f]{{6}}(?![0-9A-Za-z])|{PLAN}|{LABEL}|{PROCESS}"
)
# A file under docs/design/ states the product's design, never who builds it, in what order
# or how long.
BREAKDOWN = re.compile(
    r"^#+ .*\b(work breakdown|lanes?|estimates?|owners?|milestones?|schedule|delivery)\b",
    re.IGNORECASE,
)

COMMENT = re.compile(r"(?:^|\s)(?://+!?|#)(?=\s|$)(.*)$|^\s*\*(?!/)\s?(.*)$")
IGNORE = re.compile(r'#\[ignore\s*=\s*"([^"]*)"')
CODE = (".rs", ".py", ".mjs", ".js", ".sh", ".yml", ".yaml", ".toml")
SKIP = (
    "static/vendor/",
    "shared/schwarzwald/",
    "scripts/lint/internal-refs.py",
)
GOLDEN = "/golden/"


def denied_ids() -> set[str]:
    """Six-hex lines of the local denylist: real ids, refused as bare tokens."""
    p = ROOT / ".personal-denylist"
    if not p.exists():
        return set()
    lines = (x.strip() for x in p.read_text(encoding="utf-8").splitlines())
    return {x for x in lines if re.fullmatch(r"[0-9a-f]{6}", x)}


def tracked() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [f for f in out.split("\0") if f]


def scope(path: str) -> str | None:
    """'prose' for markdown read whole, 'code' for comments, 'make' for help, else None."""
    if any(s in path for s in SKIP):
        return None
    if GOLDEN in path:
        # Golden bodies are digests; their notes are prose: a `.md` whole, `#` lines else.
        return "prose" if path.endswith(".md") else "golden"
    if path.endswith(".md") and (
        path.startswith(("docs/", "bench/")) or path in ("README.md", "AGENTS.md")
    ):
        return "prose"
    if path == "Makefile":
        return "make"
    if path.endswith(CODE):
        return "code"
    return None


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


HEX6 = re.compile(r"\b[0-9a-f]{6}\b")


def hit(text: str, ids: set[str], pattern: re.Pattern[str] = REF) -> bool:
    if NOT_TASK_ID.search(text):
        text = NOT_TASK_ID.sub("", text)
    return bool(pattern.search(text)) or any(t in ids for t in HEX6.findall(text))


def scan(path: str, body: str, ids: set[str]) -> list[str]:
    kind = scope(path)
    if not kind:
        return []
    bad = []
    design = path.startswith("docs/design/") and path.endswith(".md")
    for n, line in enumerate(body.splitlines(), 1):
        if design and BREAKDOWN.search(line):
            bad.append(
                f"{path}:{n}: a design doc section on who, when or how long: {line}"
            )
            continue
        if any(hit(t, ids) for t in texts(kind, line)):
            bad.append(f"{path}:{n}: {line.strip()[:140]}")
    return bad


def message_hits(msg: str, ids: set[str]) -> list[str]:
    """Subject and body; lines git strips (starting with `#`) are skipped."""
    lines = [x for x in msg.splitlines() if not x.startswith("#")]
    subject = next((x for x in lines if x.strip()), "")
    return [
        f'commit "{subject[:60]}": {x.strip()[:100]}'
        for x in lines
        if hit(x, ids, MESSAGE_REF)
    ]


def self_test() -> bool:
    must_hit = [
        ("a.rs", "// see task 1a2b3c"),
        ("a.rs", "// fixed in (#abc123)"),
        ("a.rs", "// filed as `abc123`"),
        ("a.rs", "// board 1a2b3c is open"),
        ("a.rs", "// moved by G12, see P3"),
        ("a.rs", "// the C5a page"),
        ("a.rs", "// the rev2 table"),
        ("a.rs", "// done in wave 2"),
        ("a.rs", "// lane B owns this"),
        ("a.rs", "// the coordinator records it"),
        ("a.rs", "// after sign-off"),
        ("a.rs", "// red until the counting lanes land"),
        ("a.rs", '#[ignore = "red until 4f4f4a lands: see 4f4f4a"]'),
        ("Makefile", "## x: red until the board says so"),
        ("docs/a.md", "Planned in phase 3."),
        ("docs/design/a.md", "## Work breakdown"),
        ("docs/design/a.md", "## Estimates"),
        ("crates/x/tests/golden/e.txt", "# blessed for task 1a2b3c"),
    ]
    must_pass = [
        ("a.rs", "// the dashboard and the keyboard"),
        ("a.rs", '// color "#ffffff" is white'),
        ("a.rs", "// digest deadbeefcafe"),
        ("a.rs", "// see facade for details"),
        ("a.rs", "let g12 = 1; // twelve"),
        ("a.rs", "// the next steps"),
        ("a.rs", '#[ignore = "red until renames follow"]'),
        ("Makefile", "\t# lane"),
        ("docs/design/a.md", "## On-disk format"),
        ("docs/a.md", "| `%C3%A9` | e acute |"),
        (
            "crates/x/tests/golden/e.txt",
            "cc82ec7d44f8 ca5f68 [as-is] /?period=1m (#abc123)",
        ),
    ]
    ok = True
    for path, line in must_hit:
        if not scan(path, line, set()):
            print(
                f"FAIL [{RULE}] self-test: {line!r} in {path} must be refused",
                file=sys.stderr,
            )
            ok = False
    for path, line in must_pass:
        if scan(path, line, set()):
            print(
                f"FAIL [{RULE}] self-test: {line!r} in {path} must pass",
                file=sys.stderr,
            )
            ok = False
    if not scan("a.rs", "// the id 9e9e9e", {"9e9e9e"}) or scan(
        "a.rs", "// the id 9e9e9e", set()
    ):
        print(
            f"FAIL [{RULE}] self-test: a denylisted id, and only it, flags",
            file=sys.stderr,
        )
        ok = False
    if message_hits("fix(web): hint\n\nThe page reads one rule.\n", set()):
        print(f"FAIL [{RULE}] self-test: a plain message must pass", file=sys.stderr)
        ok = False
    for msg in (
        "fix(web): G12 hint",
        "fix: x\n\nSee #a1b2c3.",
        "fix: x\n\nrow 1a2b3c",
        "fix: x\n\nfollows #abcdef",
    ):
        if not message_hits(msg, set()):
            print(
                f"FAIL [{RULE}] self-test: message {msg!r} must be refused",
                file=sys.stderr,
            )
            ok = False
    if message_hits("fix: x\n# lane 3 in a stripped line", set()):
        print(
            f"FAIL [{RULE}] self-test: a stripped comment line is skipped",
            file=sys.stderr,
        )
        ok = False
    return ok


def messages(base: str) -> list[str]:
    out = subprocess.run(
        ["git", "log", "-z", "--format=%B", f"{base}..HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [m for m in out.split("\0") if m.strip()]


def report(bad: list[str], what: str) -> int:
    if bad:
        print(
            f"FAIL [{RULE}] {len(bad)} task id, planning label or process word(s) in {what}"
        )
        for b in bad[:60]:
            print(f"  {b}")
        print(
            "  Say what the task says, not its id or label. Ids go on the task's thread."
        )
        return 1
    print(f"PASS [{RULE}] no task ids, planning labels or process words in {what}")
    return 0


def main(args: list[str]) -> int:
    if not self_test():
        return 2
    ids = denied_ids()
    if args[:1] == ["--messages"]:
        bad = [h for m in messages(args[1]) for h in message_hits(m, ids)]
        return report(bad, f"the messages of {args[1]}..HEAD")
    if args[:1] == ["--message"]:
        msg = pathlib.Path(args[1]).read_text(encoding="utf-8", errors="replace")
        return report(message_hits(msg, ids), "the commit message")
    files = args or tracked()
    bad: list[str] = []
    seen = 0
    for f in files:
        p = ROOT / f
        if not scope(f) or not p.is_file():
            continue
        seen += 1
        bad += scan(f, p.read_text(errors="replace"), ids)
    if not args and seen < 100:
        print(f"FAIL [{RULE}] scanned only {seen} file(s): the census is broken")
        return 2
    return report(bad, f"the tree ({seen} files)")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
