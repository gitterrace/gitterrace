#!/usr/bin/env python3
"""The pre-push rules of a release, run by scripts/hooks/pre-push with git's stdin and args.

A push is refused when it would make CI read a changelog its tag does not match:
1. a `v*` tag whose commit has no `make release-gate` pass record (.git/release-gate/<sha>)
   naming that tag and the GitHub CI run that passed on the commit;
2. a `v*` tag pushed without main, or with a main at another commit: the tag travels only in
   the gate's one push of both (git hands a hook the refs, not --atomic itself, so the gate's
   push is the one place that pushes both);
3. main whose top CHANGELOG section is a released version (a v<x.y.z> tag, local or on the
   remote) whose text differs from that section at the tag, unless the same push moves the tag
   to the pushed main commit. That covers both an edit of a released section and main moving
   past its tag with section edits.
Deletes always pass: re-tagging deletes the remote tag first (make release-gate RETAG=1).

  scripts/hooks/pre-push <remote> <url> < refs     the hook
  python3 scripts/release/push-check.py --self-test  red and green cases on a fixture repo
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ZERO = re.compile(r"^0+$")
HEAD = re.compile(r"^## \[(\d[^\]]*)\]", re.M)
MAIN = "refs/heads/main"


def git(repo: Path, *args: str) -> tuple[int, str]:
    p = subprocess.run(["git", *args], cwd=repo, capture_output=True, text=True)
    return p.returncode, p.stdout.strip()


def top_section(text: str) -> tuple[str | None, str]:
    """The newest `## [x.y.z]` heading and its text, up to the next one."""
    for part in re.split(r"^(?=## \[)", text, flags=re.M):
        m = HEAD.match(part)
        if m:
            return m.group(1), part.strip()
    return None, ""


def section_at(repo: Path, sha: str, version: str) -> str | None:
    rc, text = git(repo, "show", f"{sha}:CHANGELOG.md")
    if rc != 0:
        return None
    for part in re.split(r"^(?=## \[)", text, flags=re.M):
        m = HEAD.match(part)
        if m and m.group(1) == version:
            return part.strip()
    return None


def peel(repo: Path, ref: str) -> str | None:
    rc, out = git(repo, "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}")
    return out if rc == 0 and out else None


def remote_ref(repo: Path, remote: str, ref: str) -> str | None:
    """The commit `ref` names on the remote (peeled), None when absent or unreachable."""
    rc, out = git(repo, "ls-remote", remote, ref, f"{ref}^{{}}")
    if rc != 0 or not out:
        return None
    rows = dict(line.split("\t")[::-1] for line in out.splitlines())
    return rows.get(f"{ref}^{{}}") or rows.get(ref)


def record_dir(repo: Path) -> Path:
    _, common = git(repo, "rev-parse", "--git-common-dir")
    return (repo / common / "release-gate").resolve()


def problems(repo: Path, remote: str, lines: list[str]) -> list[str]:
    pushed: dict[str, str] = {}
    for line in lines:
        parts = line.split()
        if len(parts) == 4 and not ZERO.match(parts[1]):
            pushed[parts[2]] = parts[1]
    bad = []
    main = pushed.get(MAIN)
    for ref, sha in pushed.items():
        if not ref.startswith("refs/tags/v"):
            continue
        commit = peel(repo, sha) or sha
        tag = ref.removeprefix("refs/tags/")
        rec = record_dir(repo) / commit
        lines_ = rec.read_text().splitlines() if rec.is_file() else []
        if f"tag {tag}" not in lines_ or not any(
            line.startswith("ci ") and line[3:].strip() for line in lines_
        ):
            bad.append(
                f"{tag}: {commit[:12]} has no release-gate pass record with a GitHub CI run,"
                " run make release-gate"
            )
        if main is None:
            bad.append(
                f"{tag}: pushed without main; make release-gate pushes both in one atomic"
                f" push: git push --atomic {remote} main {tag}"
            )
        elif main != commit:
            bad.append(f"{tag}: points at {commit[:12]}, not at the main pushed with it")
    if main is not None:
        _, text = git(repo, "show", f"{main}:CHANGELOG.md")
        version, section = top_section(text)
        if version is not None:
            ref = f"refs/tags/v{version}"
            if ref in pushed:
                if (peel(repo, pushed[ref]) or pushed[ref]) != main:
                    bad.append(f"main: v{version} is pushed with it but does not point at it")
            else:
                for where, commit in (
                    ("local", peel(repo, ref)),
                    (remote, remote_ref(repo, remote, ref)),
                ):
                    if commit and commit != main and section_at(repo, commit, version) != section:
                        bad.append(
                            f"main: CHANGELOG section {version} differs from the {where} tag"
                            f" v{version} ({commit[:12]}); re-tag with make release-gate"
                            f" VERSION={version} RETAG=1, never push them apart"
                        )
    return bad


def self_test() -> int:
    fails = []
    with tempfile.TemporaryDirectory() as tmp:
        env = {
            **os.environ,
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
        }
        os.environ.update(env)
        far, repo = Path(tmp, "far.git"), Path(tmp, "repo")
        git(Path(tmp), "init", "-q", "--bare", "-b", "main", str(far))
        git(Path(tmp), "init", "-q", "-b", "main", str(repo))
        for k, v in (
            ("user.name", "t"),
            ("user.email", "t@t"),
            ("core.hooksPath", "/dev/null"),
        ):
            git(repo, "config", k, v)
        git(repo, "remote", "add", "origin", str(far))

        def commit(log: str) -> str:
            (repo / "CHANGELOG.md").write_text(log)
            (repo / "n").write_text(str(len(log)) + log)
            git(repo, "add", "-A")
            git(repo, "commit", "-qm", "c")
            return git(repo, "rev-parse", "HEAD")[1]

        def check(name: str, want_red: bool, *refs: tuple[str, str]) -> None:
            lines = [f"x {sha} {ref} {'0' * 40}" for ref, sha in refs]
            got = problems(repo, "origin", lines)
            if bool(got) != want_red:
                fails.append(f"{name}: want {'red' if want_red else 'green'}, got {got}")

        def record(sha: str, ci: str = "https://ci/run/1") -> None:
            record_dir(repo).mkdir(exist_ok=True)
            (record_dir(repo) / sha).write_text(f"tag v1.0.0\nci {ci}\n")

        released = "# Changelog\n\n## [1.0.0]\n\n- a\n"
        a = commit(released)
        git(repo, "tag", "v1.0.0")
        check("tag without a pass record", True, (MAIN, a), ("refs/tags/v1.0.0", a))
        record(a, ci="")
        check("record without a CI run", True, (MAIN, a), ("refs/tags/v1.0.0", a))
        record(a)
        check(
            "atomic main and tag with a record",
            False,
            (MAIN, a),
            ("refs/tags/v1.0.0", a),
        )
        check("tag alone, remote main lacks it", True, ("refs/tags/v1.0.0", a))
        git(repo, "push", "-q", "--atomic", "origin", "main", "v1.0.0")
        check("tag alone, remote main contains it", True, ("refs/tags/v1.0.0", a))
        b = commit(released + "\n- fixed by hand\n")
        check("main with an edited released section, stale tag", True, (MAIN, b))
        git(repo, "tag", "-f", "v1.0.0", b)
        check("local tag moved, remote tag stale", True, (MAIN, b))
        record(b)
        check(
            "re-tag pushed atomically with main",
            False,
            (MAIN, b),
            ("refs/tags/v1.0.0", b),
        )
        check(
            "tag pushed with a main that does not contain it",
            True,
            (MAIN, a),
            ("refs/tags/v1.0.0", b),
        )
        c = commit(released + "\n- fixed by hand\n- later\n")
        check(
            "tag pushed with a main past it",
            True,
            (MAIN, c),
            ("refs/tags/v1.0.0", b),
        )
        git(repo, "reset", "-q", "--hard", b)
        if problems(repo, "origin", [f"(delete) {'0' * 40} refs/tags/v1.0.0 {a}"]):
            fails.append("delete of a remote tag: want green")
        git(repo, "push", "-q", "-f", "--atomic", "origin", "main", "v1.0.0")
        (repo / "other").write_text("x")
        git(repo, "add", "other")
        git(repo, "commit", "-qm", "d")
        d = git(repo, "rev-parse", "HEAD")[1]
        check("main past its tag, section unchanged", False, (MAIN, d))
        e = commit(released + "\n- fixed by hand\n- again\n")
        check("main past its tag with section edits", True, (MAIN, e))
    for f in fails:
        print(f"push-check self-test: {f}", file=sys.stderr)
    print("push-check self-test: " + ("FAIL" if fails else "ok"))
    return 1 if fails else 0


def main(argv: list[str]) -> int:
    if argv[1:] == ["--self-test"]:
        return self_test()
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    repo = Path(git(Path.cwd(), "rev-parse", "--show-toplevel")[1])
    bad = problems(repo, argv[1], sys.stdin.read().splitlines())
    for b in bad:
        print(f"pre-push: {b}", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
