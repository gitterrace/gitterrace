#!/usr/bin/env python3
"""The disk guard on throwaway dirs: each part refuses what it must, seen red.

  disk-floor     a floor above the current free space refuses (py and sh), one below passes,
                 DISK_FLOOR_GIB=0 passes, a floor in lint.toml is read, an unreadable floor refuses
  scratch.sh     a worktree create that hits ENOSPC fails the runner non-zero, so does one git
                 reports as done but left incomplete; the run dir goes on exit, pass or fail,
                 and --keep keeps it
  disk-sweep     a dry run lists a planted stale target and a stale lane subdir, not a fresh one
                 or one a process sits in; a scratch run with unlanded commits is kept, --apply
                 removes only what it listed and prints the bytes

    disk_test.py   exit 0 when every case holds, else one line per broken case
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ENV = {k: v for k, v in os.environ.items() if not k.startswith(("GIT_", "DISK_", "SCRATCH_"))} | dict(
    GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.invalid", GIT_COMMITTER_NAME="t",
    GIT_COMMITTER_EMAIL="t@example.invalid", GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1")
OLD = time.time() - 24 * 3600


def run(*args: str, env: dict[str, str] | None = None, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(list(args), capture_output=True, text=True, env=ENV | (env or {}), cwd=cwd)


def free_gib(p: Path) -> int:
    return shutil.disk_usage(p).free >> 30


def floor_cases(d: Path) -> list[str]:
    bad = []
    above, below = str(free_gib(d) + 100), "1"
    for name, cmd in (("disk-floor.py", [sys.executable, str(HERE / "disk-floor.py"), "--repo", str(d)]),
                      ("disk-floor.sh", ["bash", str(HERE / "disk-floor.sh"), str(d)])):
        r = run(*cmd, str(d), env={"DISK_FLOOR_GIB": above})
        if r.returncode != 1 or "refused" not in r.stderr:
            bad.append(f"{name} passed a floor above the free space: exit {r.returncode} {r.stderr.strip()}")
        if run(*cmd, str(d), env={"DISK_FLOOR_GIB": below}).returncode:
            bad.append(f"{name} refused a floor below the free space")
        if run(*cmd, str(d), env={"DISK_FLOOR_GIB": "0"}).returncode:
            bad.append(f"{name} refused with the check turned off")
        if run(*cmd, str(d), env={"DISK_FLOOR_GIB": "lots"}).returncode != 2:
            bad.append(f"{name} did not refuse an unreadable floor with exit 2")
    cfg = d / "schwarzwald/config"
    cfg.mkdir(parents=True)
    (cfg / "lint.toml").write_text(f"[history]\nfloor_gib = 1\n\n[disk]\nfloor_gib = {above}\n")
    for name, cmd in (("disk-floor.py", [sys.executable, str(HERE / "disk-floor.py"), "--repo", str(d), str(d)]),
                      ("disk-floor.sh", ["bash", str(HERE / "disk-floor.sh"), str(d), str(d)])):
        if run(*cmd).returncode != 1:
            bad.append(f"{name} did not read [disk] floor_gib from the product's lint.toml")
        if run(*cmd, env={"DISK_FLOOR_GIB": below}).returncode:
            bad.append(f"{name}: DISK_FLOOR_GIB did not override the product's floor")
    shutil.rmtree(d / "schwarzwald")
    # A local snapshot: a stubbed tmutil lists one, every tool names it and the release command.
    stub = d / "stubbin"
    stub.mkdir()
    (stub / "tmutil").write_text('#!/bin/sh\n[ "$1" = listlocalsnapshots ] && { echo "Snapshots for disk /:"; '
                                 'echo "com.example.backup.2026-11-05-120000.local"; }\nexit 0\n')
    (stub / "tmutil").chmod(0o755)
    path = {"PATH": f"{stub}:{ENV['PATH']}", "DISK_FLOOR_GIB": below}
    for name, cmd in (("disk-floor.py", [sys.executable, str(HERE / "disk-floor.py"), "--repo", str(d), str(d)]),
                      ("disk-floor.sh", ["bash", str(HERE / "disk-floor.sh"), str(d), str(d)]),
                      ("disk-sweep.py", [sys.executable, str(HERE / "disk-sweep.py"), "--scratch", str(d / "none"), str(d)])):
        r = run(*cmd, env=path)
        if "com.example.backup.2026-11-05-120000.local" not in r.stderr or "deletelocalsnapshots" not in r.stderr:
            bad.append(f"{name} did not warn about a local snapshot that pins deleted blocks")
        if "sudo tmutil" in r.stderr and "never calls sudo" not in r.stderr:
            bad.append(f"{name} names sudo without saying it never runs it")
    shutil.rmtree(stub)
    return bad


RUNNER = r'''
set -u
. "$1/scratch.sh"
scratch_init golden ${KEEP:-} || exit 1
wt=$(scratch_worktree "$2" HEAD) || exit 1
t=$(scratch_dir target) || exit 1
echo "RUN $SCRATCH_RUN"
test -f "$wt/a.txt" || exit 1
'''


def scratch_cases(d: Path) -> list[str]:
    bad = []
    repo, root, fake = d / "repo", d / "scratch-root", d / "fakebin"
    repo.mkdir()
    run("git", "-C", str(repo), "init", "-q", "-b", "main")
    (repo / "a.txt").write_text("a\n")
    run("git", "-C", str(repo), "add", "a.txt")
    run("git", "-C", str(repo), "commit", "-qm", "feat: a")
    env = {"SCRATCH_ROOT": str(root), "SCRATCH_PRODUCT": str(repo), "DISK_FLOOR_GIB": "1"}
    r = run("bash", "-c", RUNNER, "_", str(HERE), str(repo), env=env)
    if r.returncode or "RUN " not in r.stdout:
        return [f"scratch.sh runner failed on a healthy disk: {r.stderr.strip()}"]
    if any(root.iterdir()):
        bad.append("scratch.sh left its run dir behind after a pass")
    if run("git", "-C", str(repo), "worktree", "list", "--porcelain").stdout.count("worktree ") != 1:
        bad.append("scratch.sh left a worktree registered in the repository")
    # ENOSPC: git refuses the worktree with the error a full disk gives.
    real = shutil.which("git", path=ENV.get("PATH"))
    fake.mkdir()
    (fake / "git").write_text(f'#!/bin/sh\nfor a; do [ "$a" = add ] && {{ echo "fatal: could not write index: '
                              f'No space left on device" >&2; exit 128; }}; done\nexec "{real}" "$@"\n')
    (fake / "git").chmod(0o755)
    path = {"PATH": f"{fake}:{ENV['PATH']}"}
    r = run("bash", "-c", RUNNER, "_", str(HERE), str(repo), env=env | path)
    if r.returncode == 0 or "scratch: refused" not in r.stderr or "No space left" not in r.stderr or "RUN " in r.stdout:
        bad.append(f"a golden worktree create that hit ENOSPC did not fail the run: exit {r.returncode}")
    # A checkout git reports as done but could not finish writing.
    (fake / "git").write_text(f'#!/bin/sh\n"{real}" "$@" || exit $?\nfor a; do [ "$a" = add ] && '
                              f'rm -f "$(ls -d "$SCRATCH_ROOT"/*/wt.* | tail -1)/a.txt"; done\nexit 0\n')
    r = run("bash", "-c", RUNNER, "_", str(HERE), str(repo), env=env | path)
    if r.returncode == 0 or "incomplete" not in r.stderr:
        bad.append(f"an incomplete worktree that git reported as made did not fail the run: exit {r.returncode}")
    if any(root.iterdir()):
        bad.append("scratch.sh left its run dir behind after a failure")
    # The floor runs first: below it nothing is made.
    r = run("bash", "-c", RUNNER, "_", str(HERE), str(repo), env=env | {"DISK_FLOOR_GIB": str(free_gib(d) + 100)})
    if r.returncode == 0 or "refused" not in r.stderr:
        bad.append("scratch_init started a run below the disk floor")
    r = run("bash", "-c", RUNNER, "_", str(HERE), str(repo), env=env | {"KEEP": "--keep"})
    kept = r.stdout.split("RUN ")[-1].strip()
    if r.returncode or not kept or not Path(kept).is_dir() or "kept" not in r.stderr:
        bad.append("scratch.sh --keep did not keep the run dir")
    run("git", "-C", str(repo), "worktree", "prune")
    shutil.rmtree(root, ignore_errors=True)
    bad += root_cases(d, repo)
    return bad


def root_cases(d: Path, repo: Path) -> list[str]:
    """ST-8: with no TMPDIR the scratch root is per user ($XDG_RUNTIME_DIR, else ~/.cache), never
    shared /tmp; a root this user owns ends at 0700; a symlinked root is refused. disk-sweep's
    default --scratch is the same root."""
    bad = []
    home = d / "home"
    home.mkdir()
    noenv = {"HOME": str(home), "SCRATCH_PRODUCT": str(repo), "DISK_FLOOR_GIB": "1",
             "TMPDIR": "", "XDG_RUNTIME_DIR": ""}
    r = run("bash", "-c", RUNNER, "_", str(HERE), str(repo), env=noenv | {"KEEP": "--keep"})
    kept = r.stdout.split("RUN ")[-1].strip()
    want = home / ".cache/schwarzwald-scratch"
    if r.returncode or not kept.startswith(str(want) + "/"):
        bad.append(f"scratch.sh without TMPDIR did not use ~/.cache: exit {r.returncode} run {kept!r} {r.stderr.strip()[-200:]}")
    elif oct(want.stat().st_mode & 0o777) != "0o700":
        bad.append(f"scratch.sh made its root at {oct(want.stat().st_mode & 0o777)}, not 0700")
    if kept.startswith("/tmp/") or kept.startswith("/private/tmp/"):
        bad.append("scratch.sh without TMPDIR put the run in shared /tmp")
    wide = d / "wide-root"
    wide.mkdir()
    wide.chmod(0o777)
    r = run("bash", "-c", RUNNER, "_", str(HERE), str(repo), env=noenv | {"SCRATCH_ROOT": str(wide)})
    if r.returncode or oct(wide.stat().st_mode & 0o777) != "0o700":
        bad.append(f"scratch.sh left a world-writable root at {oct(wide.stat().st_mode & 0o777)}: exit {r.returncode}")
    link = d / "link-root"
    link.symlink_to(wide)
    r = run("bash", "-c", RUNNER, "_", str(HERE), str(repo), env=noenv | {"SCRATCH_ROOT": str(link)})
    if r.returncode == 0 or "refused" not in r.stderr:
        bad.append(f"scratch.sh accepted a symlinked root: exit {r.returncode}")
    # disk-sweep: no --scratch, no TMPDIR: a stale run in ~/.cache/schwarzwald-scratch is listed
    stale = want / "old.1.zzzz"
    stale.mkdir(parents=True, exist_ok=True)
    (stale / "log").write_text("x\n")
    age(stale, OLD)
    empty = d / "emptyroot"
    empty.mkdir()
    r = run(sys.executable, str(HERE / "disk-sweep.py"), str(empty), env=noenv)
    if str(stale) not in r.stdout:
        bad.append(f"disk-sweep without TMPDIR did not default to ~/.cache/schwarzwald-scratch: {r.stdout.strip()[-200:]}")
    shutil.rmtree(want, ignore_errors=True)
    return bad


def age(p: Path, when: float) -> None:
    for dirpath, dirnames, filenames in os.walk(p, topdown=False):
        for n in dirnames + filenames:
            os.utime(os.path.join(dirpath, n), (when, when), follow_symlinks=False)
    os.utime(p, (when, when))


def plant_target(checkout: Path, when: float, lane: str | None = None) -> Path:
    t = checkout / "target"
    out = t / lane if lane else t
    (out / "debug/deps").mkdir(parents=True, exist_ok=True)
    (out / "CACHEDIR.TAG").write_text("Signature: 8a477f597d28d172789f06886806bc55\n")
    (out / "debug/deps/libx.rlib").write_bytes(b"x" * 4096)
    (checkout / "Cargo.toml").write_text("[package]\nname = \"x\"\n")
    age(out, when)
    return out


def sweep_cases(d: Path) -> list[str]:
    bad = []
    org, scratch = d / "org", d / "scratch"
    stale = plant_target(org / "stale", OLD)
    fresh = plant_target(org / "fresh", time.time())
    held = plant_target(org / "held", OLD)
    locked = plant_target(org / "locked", OLD)
    (locked / "debug/.cargo-lock").write_text("")
    age(locked, OLD)
    main_t = plant_target(org / "main", time.time())
    lane = plant_target(org / "main", OLD, lane="lane-a")
    src = org / "stale/src/lib.rs"
    src.parent.mkdir(parents=True)
    src.write_text("// source\n")
    age(src.parent, OLD)
    old_run, new_run, kept_run = scratch / "red.1.aaaa", scratch / "red.2.bbbb", scratch / "red.3.cccc"
    for r in (old_run, new_run, kept_run):
        r.mkdir(parents=True)
        (r / "out.log").write_text("log\n")
    clone = kept_run / "clone"
    clone.mkdir()
    run("git", "-C", str(clone), "init", "-q", "-b", "main")
    (clone / "b.txt").write_text("b\n")
    run("git", "-C", str(clone), "add", "b.txt")
    run("git", "-C", str(clone), "commit", "-qm", "feat: b")
    age(old_run, OLD)
    age(kept_run, OLD)
    sweep = [sys.executable, str(HERE / "disk-sweep.py"), "--scratch", str(scratch), str(org)]
    sitter = subprocess.Popen(["sleep", "60"], cwd=held)
    # A cargo between processes: nothing runs there, but its lock is held.
    holder = subprocess.Popen([sys.executable, "-c", "import fcntl, os, time; f = open(os.environ['LOCK']); "
                               "fcntl.flock(f, fcntl.LOCK_EX); print('held', flush=True); time.sleep(60)"],
                              stdout=subprocess.PIPE, text=True, env=ENV | {"LOCK": str(locked / "debug/.cargo-lock")})
    holder.stdout.readline()
    try:
        time.sleep(0.3)
        r = run(*sweep)
        if str(locked) in {ln.split()[-1] for ln in r.stdout.splitlines() if ln.startswith("stale")}:
            bad.append(f"disk-sweep dry run listed a target whose cargo lock is held: {locked}")
        # The lock alone, where lsof cannot see the open file: only ps and git on PATH.
        nolsof = d / "nolsof"
        nolsof.mkdir()
        for tool in ("ps", "git"):
            (nolsof / tool).symlink_to(shutil.which(tool, path=ENV["PATH"]))
        r2 = run(*sweep, env={"PATH": str(nolsof)})
        if str(locked) in {ln.split()[-1] for ln in r2.stdout.splitlines() if ln.startswith("stale")}:
            bad.append(f"disk-sweep without lsof listed a target whose cargo lock is held: {locked}")
        bad += apply_cases(r, sweep, d, stale, lane, old_run, fresh, held, main_t, new_run, kept_run, src, org)
        if not locked.exists():
            bad.append("disk-sweep --apply removed a target whose cargo lock is held")
    finally:
        holder.kill()
        holder.wait()
        sitter.kill()
        sitter.wait()
    return bad


def apply_cases(r, sweep, d, stale, lane, old_run, fresh, held, main_t, new_run, kept_run, src, org) -> list[str]:
    bad = []
    listed = {ln.split()[-1] for ln in r.stdout.splitlines() if ln.startswith("stale")}
    if r.returncode:
        return [f"disk-sweep dry run failed: {r.stderr.strip()}"]
    for want, p in (("a stale target", stale), ("a stale lane subdir", lane), ("a stale scratch run", old_run)):
        if str(p) not in listed:
            bad.append(f"disk-sweep dry run did not list {want}: {p}")
    for nope, p in (("a fresh target", fresh), ("a target a process sits in", held), ("a fresh scratch run", new_run),
                    ("a live target holding a stale lane", main_t), ("a scratch run with unlanded commits", kept_run)):
        if str(p) in listed:
            bad.append(f"disk-sweep dry run listed {nope}: {p}")
    if not all(p.exists() for p in (stale, lane, old_run)):
        bad.append("disk-sweep removed something without --apply")
    if "would reclaim" not in r.stdout.splitlines()[-1]:
        bad.append("disk-sweep dry run did not end with the bytes it would reclaim")
    r = run(*sweep, "--apply")
    if r.returncode or "reclaimed: " not in r.stdout or "measured" not in r.stdout:
        bad.append(f"disk-sweep --apply did not print the measured bytes reclaimed: {r.stdout.strip()[-200:]}")
    if any(p.exists() for p in (stale, lane, old_run)):
        bad.append("disk-sweep --apply left a stale dir it listed")
    if not all(p.exists() for p in (fresh, held, main_t, new_run, kept_run, src, org / "stale/Cargo.toml")):
        bad.append("disk-sweep --apply removed a fresh dir, a held one, unlanded commits or a source file")
    bundles = d / "bundles"
    r = run(*sweep, "--apply", "--bundle", str(bundles))
    if kept_run.exists() or not list(bundles.glob("*.bundle")):
        bad.append(f"disk-sweep --bundle did not save the unlanded commits before removing: {r.stdout.strip()[-200:]}")
    return bad


def pushed_clone(origin: Path, at: Path) -> Path:
    """A clone of origin whose every commit the remote has."""
    run("git", "clone", "-q", str(origin), str(at))
    return at


def safety_cases(d: Path) -> list[str]:
    """The audit's proofs (ST-5, ST-6, ST-9), each under this test's own temp root."""
    bad = []
    seed = d / "seed"
    seed.mkdir()
    run("git", "-C", str(seed), "init", "-q", "-b", "main")
    (seed / "a.txt").write_text("a\n")
    run("git", "-C", str(seed), "add", "a.txt")
    run("git", "-C", str(seed), "commit", "-qm", "feat: a")
    origin = d / "origin.git"
    run("git", "clone", "-q", "--bare", str(seed), str(origin))
    scratch = d / "scratch"
    runs = {}
    # runA: a pushed clone with an uncommitted file
    a = pushed_clone(origin, scratch / "runA" / "c")
    (a / "wip.txt").write_text("not committed\n")
    runs["uncommitted work"] = a.parent
    # runB: a pushed clone holding a nested repo with an unpushed commit
    b = pushed_clone(origin, scratch / "runB" / "c")
    inner = b / "vendor/inner"
    inner.mkdir(parents=True)
    run("git", "-C", str(inner), "init", "-q", "-b", "main")
    (inner / "i.txt").write_text("i\n")
    run("git", "-C", str(inner), "add", "i.txt")
    run("git", "-C", str(inner), "commit", "-qm", "feat: inner")
    (b / ".git/info/exclude").write_text("vendor/\n")
    runs["a nested repo with an unpushed commit"] = b.parent
    # runD: a commit reachable only from a tag
    t = pushed_clone(origin, scratch / "runD" / "c")
    run("git", "-C", str(t), "checkout", "-q", "--detach")
    (t / "t.txt").write_text("t\n")
    run("git", "-C", str(t), "add", "t.txt")
    run("git", "-C", str(t), "commit", "-qm", "feat: tagged")
    run("git", "-C", str(t), "tag", "only-here")
    run("git", "-C", str(t), "checkout", "-q", "main")
    runs["a tag-only commit"] = t.parent
    # runS: a commit only in the stash
    s = pushed_clone(origin, scratch / "runS" / "c")
    (s / "a.txt").write_text("stashed\n")
    run("git", "-C", str(s), "stash", "-q")
    runs["a stash-only change"] = s.parent
    # runC: a clean pushed clone, the one a sweep may remove
    clean = pushed_clone(origin, scratch / "runC" / "c")
    for r in (*runs.values(), clean.parent):
        age(r, OLD)
    # ST-6: a target that is a symlink to a dir outside the root
    root, elsewhere = d / "root", d / "elsewhere"
    plant_target(elsewhere / "prod", time.time())
    real_lane = plant_target(elsewhere / "prod", OLD, lane="lane1")
    (root / "prod").mkdir(parents=True)
    (root / "prod/target").symlink_to(elsewhere / "prod/target")
    sweep = [sys.executable, str(HERE / "disk-sweep.py"), "--hours", "1", "--scratch", str(scratch), str(root)]
    r = run(*sweep, "--apply")
    for why, p in runs.items():
        if not p.exists():
            bad.append(f"disk-sweep --apply removed a scratch run with {why}: {r.stdout.strip()[-300:]}")
    if clean.parent.exists():
        bad.append(f"disk-sweep --apply kept a clean pushed scratch run: {r.stdout.strip()[-300:]}")
    if not real_lane.exists():
        bad.append("disk-sweep --apply deleted through a symlinked target into a dir outside the root")
    # ST-9: a run that turns fresh between the listing and the delete is kept. size() runs after
    # the pick and before the remove, so a wrapped size() writes into the run at that moment.
    late = scratch / "runL"
    late.mkdir()
    (late / "log").write_text("x\n")
    age(late, OLD)
    code = (
        "import importlib.util, sys, pathlib\n"
        f"spec = importlib.util.spec_from_file_location('ds', {str(HERE / 'disk-sweep.py')!r})\n"
        "m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)\n"
        "orig = m.size\n"
        "def size(p):\n"
        "    if p.name == 'runL': (p / 'new').write_text('started')\n"
        "    return orig(p)\n"
        "m.size = size\n"
        f"sys.exit(m.main(['--hours', '1', '--apply', '--scratch', {str(scratch)!r}]))\n")
    probe = d / "probe.py"
    probe.write_text(code)
    r = run(sys.executable, str(probe))
    if r.returncode not in (0, 1) or "Traceback" in r.stderr:
        bad.append(f"the ST-9 probe did not run: {r.stderr.strip()[-300:]}")
    if not late.exists():
        bad.append(f"disk-sweep removed a run that turned fresh after the listing: {r.stdout.strip()[-300:]}")
    return bad


def main() -> int:
    bad = []
    with tempfile.TemporaryDirectory() as d:
        base = Path(d).resolve()
        for name, case in (("floor", floor_cases), ("scratch", scratch_cases), ("sweep", sweep_cases),
                           ("safety", safety_cases)):
            (base / name).mkdir()
            bad += case(base / name)
    for b in bad:
        print(f"FAIL {b}")
    print("ok" if not bad else f"{len(bad)} broken")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
