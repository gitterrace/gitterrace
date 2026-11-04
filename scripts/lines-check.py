#!/usr/bin/env python3
"""Source hygiene guard (stdlib only), run by `make lines`, part of `make check`.

1. No source file is longer than 600 lines.
2. No code or test comment, Makefile help line or #[ignore] reason points at a design doc
   section, a process label (G12, P3, C5), a tracker word (lane, board, coordinator), a
   tracker id (six hex digits next to board, task, row or lane, or any id in the local list)
   or a local name. No test fn is named after a label (g3b_..., p4_...).
3. With --messages BASE: no commit subject or body in BASE..HEAD names a label, a six-hex id
   or a local name. land-check runs this.

  python3 scripts/lines-check.py              check the tree (tracked and untracked, not ignored)
  python3 scripts/lines-check.py --messages BASE  check the messages of BASE..HEAD
  python3 scripts/lines-check.py --self-test  prove every rule goes red and green on fixtures
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MAX_LINES = 600
LENGTH_GLOBS = ("*.rs", "*.py", "*.mjs", "*.js", "*.sh", "*.css", "*.html")
COMMENT_EXTS = (".rs", ".py", ".mjs", ".js", ".sh")
# Vendored third-party files and the synced theme copy are not ours to split or reword.
EXCLUDE = ("static/vendor/", "theme/schwarzwald/", "/golden/")
# Prose the label rule reads whole (every line is prose). Generated bench pages included.
DOC_GLOBS = ("docs/**.md", "bench/**.md")

# Process labels in comments. Percent-encodings such as %C3 sit in code, not in comments, and
# the whole-word match keeps C3 inside longer tokens out.
LABELS = re.compile(
    r"§"
    r"|\b[Ss]ection \d+\.\d+"
    r"|\bdesign (doc|section)"
    r"|\b[Ll]ane [A-Z]\b"
    r"|\b(G\d+[a-c]?|P[1-9]|C[1-9]|K[1-5]|N[1-4])\b"
    r"|\b([Ll]anes?|[Cc]oordinators?|[Bb]oards?)\b"
    r"|\b([Ss]ign-?offs?|[Ss]igned[- ]off|re-bless report|[Ww]ave \d+|[Pp]hase \d+|rev\d+)\b"
    # A developer's machine: home folders (placeholders such as /home/x apart), checkouts,
    # mDNS hostnames. Product paths such as ~/.cargo or ~/.local/state are fine.
    r"|/Users/\w|/home/(?!x\b|me\b|linuxbrew\b)\w|~/github/\w|\b[\w-]+\.local\b"
    r"|\b(board|task|row|lane)\W{1,3}[0-9a-f]{6}\b|\b[0-9a-f]{6}\W{1,3}(board|task|row|lane)\b"
)
# Labels and ids in commit messages: a six-hex token with a digit and a letter is a tracker
# id there (a commit names another commit by subject, never by short hash).
MESSAGE = re.compile(
    r"\b(G\d+[a-c]?|P[1-9]|C[1-9])\b|\b(?=[0-9a-f]*[0-9])(?=[0-9a-f]*[a-f])[0-9a-f]{6}\b"
)
LABEL_FN = re.compile(r"\bfn [gp]\d+[a-c]?_\w")
IGNORE = re.compile(r'#\[ignore\s*=\s*"([^"]*)"')
HEX6 = re.compile(r"^[0-9a-f]{6}$")
SHA = re.compile(r"\b[0-9a-f]{7,40}\b")
# Optional local list of extra names and tracker ids to refuse, one per line (a line of six hex
# digits is an id, any other a name). The file is gitignored, so none of them ever lands.
LOCAL = ROOT / ".lines-check.local"


def local_entries() -> list[str]:
    try:
        lines = LOCAL.read_text().splitlines()
    except OSError:
        return []
    return [n.strip() for n in lines if n.strip() and not n.startswith("#")]


def local_names() -> list[str]:
    return [re.escape(n) for n in local_entries() if not HEX6.match(n)]


def local_ids() -> list[str]:
    return [n for n in local_entries() if HEX6.match(n)]


def person_pattern(names: list[str]) -> re.Pattern[str] | None:
    """A name counts when capitalised, or lowercase before a colon, after @, `by`, `ask`, `told`."""
    if not names:
        return None
    alt = "|".join(names)
    cap = "|".join(n[0].upper() + n[1:] for n in names)
    return re.compile(rf"\b({cap})\b|\b({alt}):|(?:@|\bby |\bask |\btold )({alt})\b")


# `//` and `#` start a comment after whitespace or at line start. `://` and `#[` are not.
COMMENT = re.compile(r"(?:^|\s)(?://+!?|#)(?=\s|$)(.*)$|^\s*\*(?!/)\s?(.*)$")


def files() -> list[str]:
    out = subprocess.run(
        [
            "git",
            "ls-files",
            "-z",
            "-co",
            "--exclude-standard",
            "--",
            *LENGTH_GLOBS,
            "Makefile",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    md = subprocess.run(
        ["git", "ls-files", "-z", "-co", "--exclude-standard", "--", *DOC_GLOBS],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    out += "\0" + md
    return sorted(f for f in out.split("\0") if f and not any(x in f for x in EXCLUDE))


def too_long(root: Path, names: list[str]) -> list[str]:
    bad = []
    for n in names:
        try:
            k = len((root / n).read_text(errors="replace").splitlines())
        except OSError:
            continue
        if k > MAX_LINES:
            bad.append(f"{n}: {k} lines, limit {MAX_LINES}")
    return bad


def flagged(text: str, person: re.Pattern[str] | None, ids: list[str]) -> bool:
    return bool(
        LABELS.search(text)
        or (person and person.search(text))
        or any(re.search(rf"\b{i}\b", text) for i in ids)
    )


def prose(n: str, line: str) -> list[str]:
    """The comment, help text or ignore reason on one line of file `n`."""
    if n.endswith("Makefile"):
        return [line[2:]] if line.startswith("## ") else []
    if n.endswith(".md"):
        return [line]
    out = [g.group(1) for g in IGNORE.finditer(line)] if n.endswith(".rs") else []
    m = COMMENT.search(line)
    if m:
        out.append(m.group(1) if m.group(1) is not None else m.group(2) or "")
    return out


def label_hits(
    root: Path,
    names: list[str],
    extra: list[str] | None = None,
    ids: list[str] | None = None,
) -> list[str]:
    person = person_pattern(local_names() if extra is None else extra)
    ids = local_ids() if ids is None else ids
    bad = []
    for n in names:
        if (
            not n.endswith((*COMMENT_EXTS, "Makefile", ".md"))
            or n == "scripts/lines-check.py"
        ):
            continue
        try:
            lines = (root / n).read_text(errors="replace").splitlines()
        except OSError:
            continue
        for i, line in enumerate(lines, 1):
            if n.endswith(".rs") and LABEL_FN.search(line):
                bad.append(
                    f"{n}:{i}: test named after a process label: {line.strip()[:70]}"
                )
            for text in prose(n, line):
                if flagged(text, person, ids):
                    bad.append(f"{n}:{i}: process label in prose: {text.strip()[:70]}")
    return bad


def message_hits(messages: list[str], extra: list[str], ids: list[str]) -> list[str]:
    """Commit messages (subject and body) naming a label, a six-hex id or a local name."""
    person = person_pattern(extra)
    bad = []
    for msg in messages:
        subject = msg.strip().splitlines()[0] if msg.strip() else ""
        for line in msg.splitlines():
            if MESSAGE.search(line) or flagged_name(line, person, ids):
                bad.append(f'commit "{subject[:60]}": {line.strip()[:70]}')
    return bad


def flagged_name(text: str, person: re.Pattern[str] | None, ids: list[str]) -> bool:
    return bool((person and person.search(text)) or any(i in text for i in ids))


def messages(base: str) -> list[str]:
    out = subprocess.run(
        ["git", "log", "-z", "--format=%B", f"{base}..HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [m for m in out.split("\0") if m.strip()]


def own_shas(root: Path) -> set[str]:
    out = subprocess.run(
        ["git", "rev-list", "--all"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    return set(out.split())


def sha_hits(root: Path, own: set[str]) -> list[str]:
    """Any tracked text file naming a commit of this repository (7+ hex digits of a sha in
    `git rev-list --all`). Upstream corpus shas are not in that list, so they stay."""
    if not own:
        return []
    by7: dict[str, list[str]] = {}
    for s in own:
        by7.setdefault(s[:7], []).append(s)
    names = subprocess.run(
        ["git", "ls-files", "-z", "-co", "--exclude-standard"],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split("\0")
    bad = []
    for n in names:
        if not n or any(x in n for x in EXCLUDE):
            continue
        try:
            text = (root / n).read_text()
        except (OSError, UnicodeDecodeError):
            continue
        for m in SHA.finditer(text):
            t = m.group(0)
            if any(s.startswith(t) for s in by7.get(t[:7], [])):
                bad.append(f"{n}: names commit {t[:10]} of this repository")
    return bad


def run(root: Path, names: list[str]) -> list[str]:
    return (
        too_long(root, [n for n in names if not n.endswith(".md")])
        + label_hits(root, names)
        + sha_hits(root, own_shas(root))
    )


def self_test() -> None:
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        (root / "ok600.rs").write_text("fn a() {}\n" * 600)
        (root / "big601.rs").write_text("fn a() {}\n" * 601)
        assert too_long(root, ["ok600.rs"]) == [], "600 lines must pass"
        assert len(too_long(root, ["big601.rs"])) == 1, "601 lines must fail"
        green = [
            'let s = "%C3%A9"; // encodes e acute',
            'let u = "http://x"; // plain',
            "# a plain word in prose",
            "// the 3 rows of a1b2c3d4 stay",
            "// commit facade and decade, plain words",
            "#[derive(Debug)]",
            "// the case_name field",
        ]
        red = [
            "// see design doc 4",
            "// signed-off by the owner",
            "// the re-bless report says so",
            "// done in wave 2, phase 3",
            "// the rev2 table",
            "// built on /Users/x/src",
            "// copied from ~/github/x",
            "// served by mac-mini.local",
            "// per section 3.2",
            "// signed off as K3",
            "// lane Q owns this",
            "// Lane is a word, lanes too",
            "// the coordinator records it",
            "// see board 1a2b3c",
            "// row 1a2b3c is open",
            "// moved by G12, see P3 and C5",
            "// signed off as N2",
            "// \u00a7 4",
        ]
        (root / "g.rs").write_text("\n".join(green) + "\n")
        assert label_hits(root, ["g.rs"]) == [], label_hits(root, ["g.rs"])
        # The optional local name list is the only place a person name can come from.
        (root / "n.rs").write_text("// ask Orca\n// orca: later\n// the orca swims\n")
        assert label_hits(root, ["n.rs"], []) == [], "no list, no name rule"
        assert len(label_hits(root, ["n.rs"], ["orca"])) == 2, "listed name must flag"
        # Ignore reasons, Makefile help and test names are prose too.
        (root / "i.rs").write_text(
            '#[ignore = "red until 4f4f4a: lane"]\nfn g3b_a() {}\n'
        )
        assert len(label_hits(root, ["i.rs"], [])) == 2, label_hits(root, ["i.rs"], [])
        (root / "Makefile").write_text(
            "## x: red until the board says so\nx:\n\t# lane\n"
        )
        assert len(label_hits(root, ["Makefile"], [])) == 1, "help text must flag"
        (root / "ok.rs").write_text(
            '#[ignore = "red until renames follow"]\nfn a_g3b() {}\n'
        )
        assert label_hits(root, ["ok.rs"], []) == [], label_hits(root, ["ok.rs"], [])
        # A local id flags as a bare token, wherever it stands.
        (root / "id.rs").write_text("// see 9e9e9e\n")
        assert label_hits(root, ["id.rs"], [], []) == [], "no list, no id rule"
        assert label_hits(root, ["id.rs"], [], ["9e9e9e"]), "listed id must flag"
        # Commit messages: subject and body.
        clean = [
            "feat(web): count code age through Counted\n\nThe page reads one rule.\n"
        ]
        assert message_hits(clean, ["orca"], []) == [], message_hits(
            clean, ["orca"], []
        )
        for msg in [
            "fix(web): G12 hint\n",
            "fix(web): hint\n\nSee P3.\n",
            "fix(web): hint\n\nrow 1a2b3c\n",
            "fix(web): hint\n\nasked by orca\n",
        ]:
            assert message_hits([msg], ["orca"], []), f"should flag: {msg!r}"
        # A commit of this repository, in any tracked file; an unknown sha is fine.
        sha_root = root / "shas"
        sha_root.mkdir()
        subprocess.run(["git", "init", "-q"], cwd=sha_root, check=True)
        (sha_root / "a.txt").write_text("x\n")
        subprocess.run(["git", "add", "a.txt"], cwd=sha_root, check=True)
        subprocess.run(
            ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit"]
            + ["-q", "-m", "x"],
            cwd=sha_root,
            check=True,
        )
        own = own_shas(sha_root)
        assert sha_hits(sha_root, own) == [], "a clean tree must pass"
        (sha_root / "a.txt").write_text(
            f"measured at {next(iter(own))[:10]}, and 0123abcd99\n"
        )
        assert len(sha_hits(sha_root, own)) == 1, sha_hits(sha_root, own)
        for i, r in enumerate(red):
            (root / f"r{i}.rs").write_text(r + "\n")
            assert label_hits(root, [f"r{i}.rs"]), f"should flag: {r}"
    print("lines-check self-test: ok")


def main() -> int:
    if "--self-test" in sys.argv:
        self_test()
        return 0
    if "--messages" in sys.argv:
        base = sys.argv[sys.argv.index("--messages") + 1]
        bad = message_hits(messages(base), local_names(), local_ids())
        for b in bad:
            print(b)
        print(
            f"lines-check messages: {len(bad)} problem(s)"
            if bad
            else "lines-check messages: ok"
        )
        return 1 if bad else 0
    bad = run(ROOT, files())
    for b in bad:
        print(b)
    if bad:
        print(f"lines-check: {len(bad)} problem(s)")
        return 1
    print("lines-check: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
