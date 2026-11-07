#!/usr/bin/env python3
"""Refuse to start a build, a gate or a red run when the disk is nearly full.

  disk-floor.py [--repo DIR] [--quiet] [PATH...]

Checks the filesystem of every PATH (default: the product root and $TMPDIR). Below the floor it
prints one line naming the filesystem, the free space and the floor, and exits 1. A path whose
free space cannot be read is a refusal too (exit 2): a check that cannot see is not a pass.

The floor is `[disk] floor_gib` in the product's config/schwarzwald/lint.toml (default 50).
DISK_FLOOR_GIB in the environment wins, for an emergency: DISK_FLOOR_GIB=0 turns the check off
for one run and says so. disk-floor.sh holds the same check in bash for hooks and shell gates.

Exit: 0 every path is above the floor, 1 one is below, 2 the floor or the free space is unreadable.
"""

from __future__ import annotations

import os
import shutil
import sys

if sys.version_info < (3, 11):  # tomllib reads lint.toml
    newer = [f"python3.{n}" for n in range(20, 10, -1) if shutil.which(f"python3.{n}")]
    if not newer:
        sys.exit("disk-floor.py needs python 3.11 or newer for tomllib")
    os.execvp(newer[0], [newer[0], *sys.argv])

import subprocess  # noqa: E402
import tomllib  # noqa: E402
from pathlib import Path  # noqa: E402

DEFAULT_GIB = 50
GIB = 1 << 30
CONFIG = Path("config/schwarzwald/lint.toml")


def repo_root(args: list[str]) -> tuple[Path, list[str]]:
    if "--repo" in args:
        i = args.index("--repo")
        return Path(args[i + 1]).resolve(), args[:i] + args[i + 2 :]
    out = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    return Path(out.stdout.strip() or ".").resolve(), args


def floor_gib(root: Path) -> float:
    env = os.environ.get("DISK_FLOOR_GIB", "").strip()
    if env:
        return float(env)
    path = root / CONFIG
    if path.exists():
        return float(tomllib.loads(path.read_text(encoding="utf-8")).get("disk", {}).get("floor_gib", DEFAULT_GIB))
    return DEFAULT_GIB


def existing(p: Path) -> Path:
    """The path itself, or its nearest existing parent: a worktree not made yet sits on its parent's disk."""
    p = p.absolute()
    while not p.exists() and p != p.parent:
        p = p.parent
    return p


def snapshots(mount: str = "/") -> list[str]:
    """Local APFS snapshots (Time Machine, backup agents) on the volume: `tmutil listlocalsnapshots`.
    Empty where there is no tmutil. Never sudo."""
    if not shutil.which("tmutil"):
        return []
    try:
        r = subprocess.run(["tmutil", "listlocalsnapshots", mount], capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.TimeoutExpired):
        return []
    return [ln.strip() for ln in r.stdout.splitlines() if ln.strip() and not ln.startswith("Snapshots for")]


def snapshot_warning(names: list[str]) -> str:
    if not names:
        return ""
    dates = sorted({d for n in names for d in __import__("re").findall(r"\d{4}-\d{2}-\d{2}-\d{6}", n)})
    how = " or ".join(f"sudo tmutil deletelocalsnapshots {d}" for d in dates[:3]) or "sudo tmutil deletelocalsnapshots <date>"
    return (f"WARNING: {len(names)} local snapshot(s) pin deleted blocks: {', '.join(names[:5])}. Deletes will not "
            f"free space until they are released: run {how} yourself, or release them in the backup app's "
            f"own settings (this tool never calls sudo)")


def main(argv: list[str]) -> int:
    root, args = repo_root(argv)
    quiet = "--quiet" in args
    args = [a for a in args if a != "--quiet"]
    try:
        floor = floor_gib(root)
    except (ValueError, tomllib.TOMLDecodeError) as e:
        print(f"disk-floor: refused, the floor is unreadable: {e}", file=sys.stderr)
        return 2
    if floor <= 0:
        print("disk-floor: DISK_FLOOR_GIB=0, the free-space check is off for this run", file=sys.stderr)
        return 0
    paths = [Path(a) for a in args] or [root, Path(os.environ.get("TMPDIR") or "/tmp")]
    worst = 0
    for p in paths:
        try:
            free = shutil.disk_usage(existing(p)).free
        except OSError as e:
            print(f"disk-floor: refused, cannot read the free space of {p}: {e}", file=sys.stderr)
            worst = max(worst, 2)
            continue
        if free < floor * GIB:
            print(f"disk-floor: refused, {free / GIB:.1f} GiB free on the disk holding {p}, the floor is "
                  f"{floor:g} GiB. Free space first: tools/disk-sweep.py lists stale build output "
                  f"(DISK_FLOOR_GIB overrides the floor for an emergency)", file=sys.stderr)
            worst = max(worst, 1)
        elif not quiet:
            print(f"disk-floor: ok, {free / GIB:.1f} GiB free on the disk holding {p} (floor {floor:g} GiB)")
    # Loud whatever the verdict: with a snapshot held, the sweep that should follow frees nothing.
    if w := snapshot_warning(snapshots()):
        print(w, file=sys.stderr)
    return worst


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
