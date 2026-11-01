#!/usr/bin/env python3
"""Fail if theme/schwarzwald differs from its manifest (stdlib only).

Compares the sha256 of every file under theme/schwarzwald with manifest.json,
and reports missing, changed and unlisted files. Refresh the snapshot with
schwarzwald's sync.py, never by editing it.
"""
import hashlib, json, sys
from pathlib import Path

SNAP = Path(__file__).resolve().parent.parent / "theme/schwarzwald"


def main():
    manifest = SNAP / "manifest.json"
    if not manifest.is_file():
        print(f"no manifest: {manifest}")
        return 1
    want = json.loads(manifest.read_text())["files"]
    have = {
        str(p.relative_to(SNAP)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(SNAP.rglob("*"))
        if p.is_file() and p.name != "manifest.json"
    }
    bad = [f"missing: {r}" for r in want if r not in have]
    bad += [f"differs: {r}" for r in want if r in have and have[r] != want[r]]
    bad += [f"not in manifest: {r}" for r in have if r not in want]
    for line in bad:
        print(line)
    return 1 if bad else 0


sys.exit(main())
