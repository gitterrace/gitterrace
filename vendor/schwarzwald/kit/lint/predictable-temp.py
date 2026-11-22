#!/usr/bin/env python3
"""CONV-PREDICTABLE-TEMP: a script never writes to a name another local user can predict.

  1. A literal path directly under the shared temp dir (a fixed name there) in a shell or
     python file outside tests. Another user pre-creates it as a symlink and the script then
     appends to or replaces a victim file.
  2. A temp path whose name is suffixed with the shell pid (dollar-dollar): the pid is guessable,
     so it is the same attack. Use mktemp, or a per-user 0700 dir. A pid in a name that is no
     path (a docker volume, a canary value) is fine: the line must hold a slash to count.

  The second defect it catches: any other fixed temp path a future guard rule or tool picks.
  A line with `mktemp` is fine for the pid (the random part follows it).
  Lines that start with a comment are skipped. Tests (`test`, `tests`, `test_*`, `*-test.*`,
  `*_test.*`) red-cases and the lints themselves (their self-tests hold sample text) are exempt: they run in a sandbox of their own.

  predictable-temp.py [--repo DIR]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-PREDICTABLE-TEMP"
SHARED = "/" + "tmp"  # built, so this file does not match itself
PID = "$" + "$"
FIXED = re.compile(r"(?<![\w.}/-])" + SHARED + r"/[\w.${-]")
PID_NAME = re.compile(r"[.\-_]" + re.escape(PID) + r"(?![\w(])|" + re.escape(PID) + r"[.\-_]\w")
EXEMPT = re.compile(r"(^|/)(tests?|red-cases|lint)/|(^|/)test_[^/]*$|[-_.]tests?\.[a-z]+$")


def problems(rel: str, text: str) -> list[str]:
    out = []
    for n, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("#"):
            continue
        if FIXED.search(line):
            out.append(f"{rel}:{n}: fixed path in the shared temp dir")
        elif PID_NAME.search(line) and "/" in line and "mktemp" not in line:
            out.append(f"{rel}:{n}: temp name suffixed with the pid")
    return out


def self_test() -> bool:
    red_fixed = f': "${{W:={SHARED}/x.log}}"\n'
    red_py = f'open("{SHARED}/x.json", "w")\n'
    red_pid = 'f="${TMPDIR:-x}/hits-' + PID + '"\n'
    red_pid2 = "rm -f /x/.a." + PID + "\n"
    green = (
        f"# {SHARED}/x is data in a comment\n"
        f'd="${{TMPDIR:-{SHARED}}}/u"\nt=$(mktemp)\necho "pid ' + PID + f'"\ncd {SHARED}\n'
        # A pid in a name that is not a path (a docker volume, a canary value) writes no file.
        "volume=check-" + PID + "\n" + 'CANARY="canary-$(date +%s)-' + PID + '"\n'
    )
    ok = (
        bool(problems("a.sh", red_fixed)) and bool(problems("a.py", red_py))
        and bool(problems("a.sh", red_pid)) and bool(problems("a.sh", red_pid2))
        and not problems("a.sh", green)
    )
    if not ok:
        print(f"FAIL [{RULE}] self-test: planted fixed temp names must fail and clean ones pass", file=sys.stderr)
    return ok


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    bad, seen = [], 0
    for rel in config.tracked(root, cfg["scope"]["exclude"]):
        if not rel.endswith((".sh", ".py")) or EXEMPT.search(rel) or not (root / rel).is_file():
            continue
        seen += 1
        bad += problems(rel, (root / rel).read_text(encoding="utf-8", errors="replace"))
    return config.report(RULE, bad, f"{len(bad)} predictable temp name(s)",
                         f"no fixed or pid-named temp paths ({seen} files)",
                         "Use mktemp, or a per-user 0700 dir.")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
