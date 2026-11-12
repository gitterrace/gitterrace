#!/usr/bin/env python3
"""golden.py change: a real answer change between two commits fails the re-bless proof, an
unchanged one passes, a compare that writes no report fails, and an option-shaped commit is
refused. Runs in a throwaway repo with a stub dump and compare, so no product build is needed.

  python3 tools/golden_test.py
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
TOOL = HERE / "golden.py"
ENV = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")} | {
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@example.invalid",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@example.invalid",
}
DUMP = "import os, pathlib; pathlib.Path(os.environ['GOLDEN_DUMP'], 'page').write_text(open('answer').read())"
# The stub compare: a report always, and exit 1 when the two dumps differ.
COMPARE = """import sys, pathlib
old, new, report = sys.argv[1:4]
same = pathlib.Path(old, 'page').read_text() == pathlib.Path(new, 'page').read_text()
pathlib.Path(report).write_text('real 0\\n' if same else 'real 1\\n')
sys.exit(0 if same else 1)
"""


def git(root: Path, *a: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *a],
        env=ENV,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def run(root: Path, *a: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(TOOL), *a],
        cwd=root,
        env=ENV,
        capture_output=True,
        text=True,
    )


def main() -> int:
    fail = 0
    with tempfile.TemporaryDirectory() as t:
        root = Path(t) / "product"
        (root / "schwarzwald/config").mkdir(parents=True)
        git(root, "init", "-q")
        cfg = (
            "[golden]\n"
            'tests = "t/*.py"\nmarker = "golden/"\nmatch = \'t/(?P<test>[^/]+)\\.py\'\n'
            f'run = ["{sys.executable}"]\neach = ["t/{{test}}.py"]\n'
            f'dump = ["{sys.executable}", "-c", "{DUMP}"]\n'
            f'compare = ["{sys.executable}", "compare.py", "{{old}}", "{{new}}", "{{report}}"]\n'
        )
        (root / "compare.py").write_text(COMPARE)
        (root / "schwarzwald/config/golden.toml").write_text(cfg)
        (root / "answer").write_text("42 commits\n")
        git(root, "add", "-A")
        git(root, "commit", "-qm", "one")
        a = git(root, "rev-parse", "HEAD")
        git(root, "commit", "-q", "--allow-empty", "-m", "two")
        b = git(root, "rev-parse", "HEAD")
        (root / "answer").write_text("43 commits\n")
        git(root, "commit", "-qam", "three")
        c = git(root, "rev-parse", "HEAD")
        cases = [
            (
                "an unchanged answer passes the re-bless proof",
                ("change", a, b),
                0,
                "no real change",
            ),
            (
                "a changed answer fails the re-bless proof",
                ("change", b, c),
                1,
                "REAL change",
            ),
            (
                "an option-shaped commit is refused",
                ("change", "--output=x", c),
                1,
                "is not a commit",
            ),
        ]
        for label, args, want, needle in cases:
            r = run(root, *args)
            ok = r.returncode == want and needle in r.stdout + r.stderr
            print(
                ("ok   " if ok else "FAIL ")
                + label
                + (
                    ""
                    if ok
                    else f" (rc {r.returncode}: {(r.stdout + r.stderr)[-300:]})"
                )
            )
            fail |= not ok
        (root / "compare.py").write_text("import sys\nsys.exit(0)\n")
        r = run(root, "change", b, c)
        ok = r.returncode == 1 and "wrote no report" in r.stdout
        print(("ok   " if ok else "FAIL ") + "a compare that writes no report fails")
        fail |= not ok
        left = git(root, "worktree", "list").splitlines()
        ok = len(left) == 1
        print(("ok   " if ok else "FAIL ") + "no worktree is left behind")
        fail |= not ok
    return 1 if fail else 0


if __name__ == "__main__":
    sys.exit(main())
