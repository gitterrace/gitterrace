#!/usr/bin/env python3
"""CONV-COMMIT-SUBJECT: the subject reads `type(scope): subject`.

Types come from lint.toml `[commit] types`. git's own subjects pass: `Merge branch ...` (and pull
request, remote-tracking branch, tag), and `Revert "<s>"`, `fixup! <s>`, `squash! <s>` when <s>
passes (the landing tool refuses the last two, `rules/deny.toml` [partial_subject]). Every line counts:
the commit-msg hook strips `#` lines itself when git will. `<allow_prefix>ALLOW_FREE_SUBJECT=1` in the environment
overrides, `[commit] allow_prefix` names the prefix.

  commit-subject.py [--repo DIR] --message FILE
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-COMMIT-SUBJECT"
# The subjects git and the forges write themselves, in their own words only.
MERGE = re.compile(r"^Merge (branch|branches|pull request|remote-tracking branch|tag|commit) ")
REVERT = re.compile(r'^Revert "(.+)"$')
FIXUP = re.compile(r"^(fixup|squash|amend)! (.+)$")


def subject_ok(subject: str, types: list[str]) -> bool:
    form = re.compile(rf"^({'|'.join(map(re.escape, types))})(\([A-Za-z0-9_./,+ -]+\))?!?: \S")
    if not subject or MERGE.match(subject) or form.match(subject):
        return True
    # A revert or fixup names the subject it acts on, which keeps to the form itself.
    inner = REVERT.match(subject) or FIXUP.match(subject)
    return bool(inner) and subject_ok(inner.groups()[-1], types)


def self_test() -> bool:
    types = config.DEFAULTS["commit"]["types"]
    good = ["fix(web): one rule", "docs: x", "feat(web)!: new route", "Merge branch 'x'", ""]
    bad = ["Empty states: the copy", "wip: scrolling", "fix:no space", "fix(web) x", "Merge anything",
           "fixup! wip", 'Revert "wip"']
    ok = all(subject_ok(s, types) for s in good) and not any(subject_ok(s, types) for s in bad)
    if not ok:
        print(f"FAIL [{RULE}] self-test: subject form", file=sys.stderr)
    return ok


def main(argv: list[str]) -> int:
    root, args = config.take_repo(argv)
    cfg = config.load(root)["commit"]
    if not self_test():
        return 2
    if args[:1] != ["--message"]:
        print(__doc__, file=sys.stderr)
        return 2
    if os.environ.get(cfg["allow_prefix"] + "ALLOW_FREE_SUBJECT") == "1":
        return 0
    lines = config.message_lines(Path(args[1]).read_text(encoding="utf-8", errors="replace"))
    subject = next((x for x in lines if x.strip()), "")
    if subject_ok(subject, cfg["types"]):
        return 0
    print(f'FAIL [{RULE}] the subject must be "type(scope): subject", type one of: {" ".join(cfg["types"])}')
    print(f"  Got: {subject}")
    print(f"  ({cfg['allow_prefix']}ALLOW_FREE_SUBJECT=1 overrides.)")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
