#!/usr/bin/env python3
"""List, or remove, stale build output and stale scratch runs.

  disk-sweep.py [--hours N] [--apply] [--bundle DIR] [--scratch DIR]... ROOT...

Under each ROOT it finds:
  target dirs   a `target/` holding a CACHEDIR.TAG or a `debug/`/`release/` dir (cargo output),
                and the per-lane subdirs of one (each child of target/ that is itself cargo
                output), not written for N hours (default 6), whose checkout no running cargo
                or rustc names in its command line or working directory
  scratch runs  each child of a --scratch DIR (default: $SCRATCH_ROOT, else
                $TMPDIR, $XDG_RUNTIME_DIR or ~/.cache, each /schwarzwald-scratch) not
                written for N hours that no process has open or as its working directory

It never removes a source tree: only a target dir or a scratch run. A scratch run holding a
repo (nested ones too) with commits no remote has (on any branch, tag, stash or a detached
HEAD) is skipped, or with --bundle DIR first bundled into DIR and then removed. A run with
uncommitted changes is always kept. A symlinked target, lane or run is never followed. Each
dir is checked again right before it is removed. The listing is the default; --apply
removes. Either way the last line is the bytes it would reclaim or did reclaim.

Bytes. A dir's size is its allocated blocks (st_blocks), labelled "up to": an APFS clone
(`cp -c`) shares blocks with its source and a local snapshot (Time Machine, a backup agent) pins
deleted blocks, so removing a dir can free far less. --apply therefore reports the real gain,
free space on each filesystem measured before and after, and says so when it falls short of the
listed size. Any local snapshot is named, with the release command, which this tool never runs.

Shared targets. Pointing every lane of a product at one CARGO_TARGET_DIR, with a subdir per lane
(`<product>/target/<lane>`), means a new clone is not a new 10-60 GB target, and the sweep prunes
the lane subdirs a finished lane leaves: `disk-sweep.py --lane-target <product> <lane>` prints
the path to export.

Exit: 0 done, 1 something could not be removed, 2 bad arguments.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

if sys.version_info < (3, 11):
    newer = [f"python3.{n}" for n in range(20, 10, -1) if shutil.which(f"python3.{n}")]
    if not newer:
        sys.exit("disk-sweep.py needs python 3.11 or newer")
    os.execvp(newer[0], [newer[0], *sys.argv])

SKIP_WALK = {".git", "node_modules"}


def is_cargo_output(d: Path) -> bool:
    return (d / "CACHEDIR.TAG").exists() or (d / "debug").is_dir() or (d / "release").is_dir()


def newest_write(d: Path, stop_after: float) -> float:
    """The newest mtime under d. Stops early once one is newer than stop_after: that is fresh."""
    newest = d.stat().st_mtime
    for dirpath, dirnames, filenames in os.walk(d):
        for name in dirnames + filenames:
            try:
                m = os.lstat(os.path.join(dirpath, name)).st_mtime
            except OSError:
                continue
            if m > newest:
                newest = m
                if newest > stop_after:
                    return newest
    return newest


def size(d: Path) -> int:
    """Allocated bytes under d, each inode once. An upper bound on what removing d frees."""
    total, seen = 0, set()
    for dirpath, _, filenames in os.walk(d):
        for name in filenames:
            try:
                st = os.lstat(os.path.join(dirpath, name))
            except OSError:
                continue
            if (st.st_dev, st.st_ino) in seen:
                continue
            seen.add((st.st_dev, st.st_ino))
            total += st.st_blocks * 512
    return total


def free_bytes(paths: list[Path]) -> dict[int, int]:
    """Free bytes per filesystem (by st_dev) holding each path."""
    out = {}
    for p in paths:
        q = p
        while not q.exists() and q != q.parent:
            q = q.parent
        try:
            out[os.stat(q).st_dev] = shutil.disk_usage(q).free
        except OSError:
            pass
    return out


def snapshot_warning() -> str:
    """disk-floor.py's snapshot check, from the copy beside this one."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("diskfloor", Path(__file__).resolve().parent / "disk-floor.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.snapshot_warning(mod.snapshots())


def processes() -> list[tuple[str, str]]:
    """(command line, working dir) of every process this user can see: the command line from ps,
    the working dir from /proc or, where there is none (macOS), from lsof."""
    out = subprocess.run(["ps", "-axww", "-o", "pid=,command="], capture_output=True, text=True).stdout
    cmds = {}
    for line in out.splitlines():
        pid, _, cmd = line.strip().partition(" ")
        cmds[pid] = cmd
    cwds: dict[str, str] = {}
    if Path("/proc/self/cwd").exists():
        for pid in cmds:
            try:
                cwds[pid] = os.readlink(f"/proc/{pid}/cwd")
            except OSError:
                pass
    elif shutil.which("lsof"):
        out = subprocess.run(["lsof", "-n", "-P", "-w", "-a", "-d", "cwd", "-Fpn"], capture_output=True, text=True).stdout
        pid = ""
        for ln in out.splitlines():
            if ln.startswith("p"):
                pid = ln[1:]
            elif ln.startswith("n") and pid:
                cwds[pid] = ln[1:]
    return [(cmd, cwds.get(pid, "")) for pid, cmd in cmds.items()]


def open_paths(dirs: list[Path]) -> list[str]:
    """Every path under dirs a process has open (lsof +D-free: one lsof, filtered here)."""
    if not shutil.which("lsof") or not dirs:
        return []
    out = subprocess.run(["lsof", "-n", "-P", "-w", "-Fn"], capture_output=True, text=True).stdout
    return [ln[1:] for ln in out.splitlines() if ln.startswith("n/") and any(under(ln[1:], d) for d in dirs)]


def under(path: str, d: Path) -> bool:
    s = str(d)
    return path == s or path.startswith(s + "/")


BUILD = ("cargo", "rustc", "rust-analyzer", "clippy-driver")


def lock_held(f: Path) -> bool:
    """Someone holds a flock on f (cargo's .cargo-lock, a product build lock). Probing takes no
    lasting lock: a free lock is released at once. A lock dir (mkdir-style) counts as held."""
    import fcntl
    if f.is_dir():
        return True
    try:
        fd = os.open(f, os.O_RDONLY)
    except OSError:
        return False
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fcntl.flock(fd, fcntl.LOCK_UN)
        return False
    except OSError:
        return True
    finally:
        os.close(fd)


LOCK_NAMES = (".cargo-lock", ".build.lock")


def held_lock(checkout: Path, target: Path) -> bool:
    """A cargo lock in the target (its profile dirs, two levels down) or the product's build lock
    in the checkout is held: a cargo between processes still owns the dir."""
    near = [checkout / n for n in LOCK_NAMES] + [target / n for n in LOCK_NAMES]
    for sub in (target, *[c for c in target.iterdir() if c.is_dir()]) if target.is_dir() else ():
        near += [sub / n for n in LOCK_NAMES] + [c / n for c in sub.iterdir() if c.is_dir() for n in LOCK_NAMES]
    return any(f.exists() and lock_held(f) for f in near)


def busy_build(checkout: Path, target: Path, procs: list[tuple[str, str]]) -> bool:
    """A build tool runs in the checkout or the target, names one of them on its command line, or
    holds a cargo or build lock there."""
    if held_lock(checkout, target):
        return True
    for cmd, cwd in procs:
        if not any(w in cmd for w in BUILD):
            continue
        if str(checkout) in cmd or str(target) in cmd or (cwd and (under(cwd, checkout) or under(cwd, target))):
            return True
    return False


def targets(root: Path) -> list[tuple[Path, Path, list[Path]]]:
    """(target dir, its checkout, its per-lane subdirs) for every cargo target dir under root.
    A symlinked target or lane is never taken: removing through it would delete a dir outside
    the root. Symlinked dirs are not walked into either."""
    found = []
    real_root = os.path.realpath(root)
    for dirpath, dirnames, _ in os.walk(root, followlinks=False):
        p = Path(dirpath)
        t = p / "target"
        if "target" in dirnames and not t.is_symlink() and inside(t, real_root) and is_cargo_output(t):
            lanes = [c for c in sorted(t.iterdir()) if not c.is_symlink() and c.is_dir()
                     and c.name not in ("debug", "release") and is_cargo_output(c)]
            found.append((t, p, lanes))
            dirnames.remove("target")
        dirnames[:] = [d for d in dirnames if d not in SKIP_WALK]
    return found


def inside(p: Path, real_root: str) -> bool:
    """p, symlinks resolved, lies at or below the resolved root."""
    r = os.path.realpath(p)
    return r == real_root or r.startswith(real_root.rstrip("/") + "/")


def git_out(d: str, *args: str) -> str | None:
    # --no-optional-locks: a status must not rewrite the index, or the probe itself makes the
    # run look freshly written.
    r = subprocess.run(["git", "--no-optional-locks", "-C", d, *args], capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else None


def repos_in(run: Path) -> list[tuple[Path, bool]]:
    """Every git work tree in run, nested ones too: (dir, is a linked worktree)."""
    out = []
    for dirpath, dirnames, filenames in os.walk(run, followlinks=False):
        if ".git" in dirnames or ".git" in filenames:
            out.append((Path(dirpath), ".git" in filenames))
        # Keep walking below a work tree: a nested repo (vendored, ignored) holds its own work.
        dirnames[:] = [d for d in dirnames if d not in SKIP_WALK]
    return out


def dirty(run: Path) -> list[Path]:
    """Work trees in run with uncommitted or untracked changes: a bundle cannot save those, so
    such a run is always kept."""
    return [d for d, _ in repos_in(run) if git_out(str(d), "status", "--porcelain") != ""]


def unlanded(run: Path) -> list[Path]:
    """Git work trees inside a scratch run holding commits that removing the run would lose: a
    clone's commits no remote has (any ref: branches, tags, the stash, a detached HEAD; all of
    them when it has no remote), a worktree's detached HEAD no branch of its repository holds.
    Nested repos count too."""
    out = []
    for d, worktree in repos_in(run):
        dirpath = str(d)
        if not git_out(dirpath, "rev-parse", "-q", "--verify", "HEAD"):
            continue
        if worktree:  # a worktree: removing it drops only what no branch holds
            lost = not git_out(dirpath, "branch", "-a", "--contains", "HEAD", "--format=%(refname)")
        elif git_out(dirpath, "remote"):
            lost = git_out(dirpath, "rev-list", "-1", "HEAD", "--all", "--not", "--remotes") != ""
        else:
            lost = True
        if lost:
            out.append(d)
    return out


def save_bundles(run: Path, dest: Path) -> bool:
    """Bundle every checkout in run whose commits would be lost. False when one bundle failed."""
    for repo in unlanded(run):
        dest.mkdir(parents=True, exist_ok=True)
        out = dest / f"{run.name}-{repo.name}.bundle"
        r = subprocess.run(["git", "-C", str(repo), "bundle", "create", str(out), "--all"], capture_output=True, text=True)
        if r.returncode or not out.exists():
            print(f"FAIL  bundle  {repo}: {r.stderr.strip()}", file=sys.stderr)
            return False
        print(f"saved {out}")
    return True


def human(n: int) -> str:
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if n < 1024 or unit == "TiB":
            return f"{n:.1f} {unit}" if unit != "B" else f"{n} B"
        n /= 1024
    return str(n)


def default_scratch() -> Path:
    """The default scratch root: $SCRATCH_ROOT, $TMPDIR, $XDG_RUNTIME_DIR, else
    ~/.cache, each with schwarzwald-scratch. Never a fixed name in shared /tmp."""
    if os.environ.get("SCRATCH_ROOT"):
        return Path(os.environ["SCRATCH_ROOT"]).resolve()
    base = os.environ.get("TMPDIR") or os.environ.get("XDG_RUNTIME_DIR") or str(Path.home() / ".cache")
    return Path(base).resolve() / "schwarzwald-scratch"


def main(argv: list[str]) -> int:
    if argv[:1] == ["--lane-target"] and len(argv) == 3:
        # One shared target per product, a subdir per lane: the sweep prunes stale lane subdirs.
        t = Path(argv[1]).resolve() / "target" / argv[2]
        print(t)
        return 0
    hours, apply, bundle, scratch, roots = 6.0, False, None, [], []
    it = iter(argv)
    try:
        for a in it:
            if a == "--hours":
                hours = float(next(it))
            elif a == "--apply":
                apply = True
            elif a == "--bundle":
                bundle = Path(next(it)).resolve()
            elif a == "--scratch":
                scratch.append(Path(next(it)).resolve())
            elif a.startswith("-"):
                raise ValueError(a)
            else:
                roots.append(Path(a).resolve())
    except (StopIteration, ValueError):
        print(__doc__, file=sys.stderr)
        return 2
    if not roots and not scratch:
        print(__doc__, file=sys.stderr)
        return 2
    scratch = scratch or [default_scratch()]
    cutoff = time.time() - hours * 3600
    procs = processes()
    opened = open_paths(roots + scratch)
    picked: list[tuple[str, Path]] = []

    def stale(d: Path, checkout: Path | None) -> str:
        if newest_write(d, cutoff) > cutoff:
            return "fresh"
        if checkout and busy_build(checkout, d, procs):
            return "in use"
        if any(under(o, d) for o in opened) or any(cwd and under(cwd, d) for _, cwd in procs):
            return "open"
        return ""

    for root in roots:
        for t, checkout, lanes in targets(root):
            why = stale(t, checkout)
            if not why:
                picked.append(("target", t))
                continue
            print(f"keep  {why:7} {t}")
            # The whole dir is live, but a lane subdir in it may be stale on its own.
            for lane in lanes:
                lwhy = stale(lane, checkout)
                if lwhy:
                    print(f"keep  {lwhy:7} {lane}")
                else:
                    picked.append(("target", lane))
    for s in scratch:
        if not s.is_dir():
            continue
        for run in sorted(s.iterdir()):
            if run.is_symlink() or not run.is_dir():
                continue
            why = stale(run, None)
            if why:
                print(f"keep  {why:7} {run}")
            elif wip := dirty(run):
                print(f"keep  dirty   {run} (uncommitted changes in {wip[0]})")
            elif (held := unlanded(run)) and not bundle:
                print(f"keep  unlanded {run} (commits only in {held[0]}, --bundle DIR saves them first)")
            else:
                picked.append(("scratch", run))
    total, done = 0, 0
    failed = 0
    before = free_bytes([d for _, d in picked])
    for kind, d in picked:
        n = size(d)
        if apply:
            # Re-checked just before the delete (ST-9): a lane may have started in the dir, or
            # changed it, since the listing.
            if d.is_symlink() or newest_write(d, cutoff) > cutoff or (kind == "scratch" and dirty(d)) \
                    or (kind == "scratch" and not bundle and unlanded(d)) \
                    or any(cwd and under(cwd, d) for _, cwd in processes()):
                print(f"keep  changed {d} (in use or written since the listing)")
                continue
            if kind == "scratch" and bundle and not save_bundles(d, bundle):
                failed += 1
                continue
            try:
                shutil.rmtree(d)
            except OSError as e:
                print(f"FAIL  remove  {d}: {e}", file=sys.stderr)
                failed += 1
                continue
            print(f"gone  {kind:7} up to {human(n):>10}  {d}")
        else:
            print(f"stale {kind:7} up to {human(n):>10}  {d}")
        total += n
        done += 1
    snap = snapshot_warning()
    if snap:
        print(snap, file=sys.stderr)
    if not apply:
        print(f"would reclaim (--apply removes): up to {total} bytes ({human(total)}) in {done} dirs, "
              f"less where blocks are shared with a clone or held by a snapshot")
        return 0
    time.sleep(1)  # let the filesystem settle its free count
    after = free_bytes([d.parent for _, d in picked])
    gained = max(0, sum(after.get(k, v) - v for k, v in before.items()))
    print(f"reclaimed: {gained} bytes ({human(gained)}) of free space, measured, in {done} dirs "
          f"(listed size up to {human(total)})")
    if total and gained < total // 2:
        print(f"note: free space grew by {human(gained)}, less than the {human(total)} listed. Blocks shared with an "
              f"APFS clone stay with the source, and a local snapshot or a backup agent keeps deleted blocks "
              f"until it lets go{'' if snap else '; free space can also lag a delete for a while'}", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
