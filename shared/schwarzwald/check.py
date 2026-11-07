#!/usr/bin/env python3
"""check.py [<copy>]
exit 0 if this copy of schwarzwald equals its manifest.json, file contents and exec bits, 1 naming
the first file that differs. <copy> defaults to the directory this file is in. No network, no
schwarzwald checkout: a product's gate runs the copy's own check.py."""

import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

SKIP = {'manifest.json', 'Cargo.lock'}
# A file that never leaves schwarzwald lives in its internal/ folder, which build.py never reads. The
# marker line in a file's first MARKER_LINES lines is the backstop: build.py --check fails on a marked
# file under source/, and a copy refuses one whatever its manifest says.
MARKER_LINES = 5
MARKER = re.compile(r'^\W*schwarzwald' r': never-sync\W*$', re.M)
SCRIPT_SUFFIXES = ('.py', '.sh', '.cjs', '.mjs')


def has_marker(path):
    """True when one of the file's first lines is the never-sync marker and nothing else."""
    try:
        with path.open(errors='replace') as f:
            head = ''.join(line for _, line in zip(range(MARKER_LINES), f))
    except (OSError, UnicodeError):
        return False
    return bool(MARKER.search(head))


def marked_files(d, rels):
    """The paths among rels, relative to d, that carry the never-sync marker."""
    return sorted(rel for rel in rels if has_marker(d / rel))


def internal_names(internal):
    """The file stems of the scripts under internal/: a copy that names one points at a file it lacks."""
    if not internal.is_dir():
        return []
    stems = {
        p.name.split('.')[0]
        for p in internal.rglob('*')
        if p.is_file() and p.suffix in SCRIPT_SUFFIXES
    }
    return sorted(s for s in stems if len(s) >= 4)


def name_leaks(d, rels, names):
    """(path, name) for each file among rels whose path or text names one of names."""
    out = []
    for rel in rels:
        try:
            text = (d / rel).read_text(errors='replace')
        except OSError:
            continue
        out += [(rel, n) for n in names if n in text or n in rel]
    return out


def files_of(d):
    """sha256 per file under d, by relative path, the way the manifest lists them."""
    return {
        str(p.relative_to(d)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(d.rglob('*'))
        if p.is_file()
        and p.name not in SKIP
        and not p.name.startswith('.')
        and 'target' not in p.relative_to(d).parts
    }


def exec_of(d, files):
    """The files among `files` whose owner exec bit is set, sorted."""
    return sorted(rel for rel in files if os.access(d / rel, os.X_OK))


def untracked(d, rels):
    """The paths among rels that git does not track, when d sits in a git work tree. A commit of
    only the changed paths leaves a new synced file on disk but out of the product's history."""
    try:
        r = subprocess.run(
            ['git', 'ls-files', '-z', '--cached', '--', '.'],
            cwd=d,
            capture_output=True,
            check=False,
        )
    except OSError:
        return []
    if r.returncode != 0:
        return []
    tracked = {p for p in r.stdout.decode(errors='replace').split('\0') if p}
    return sorted(rel for rel in rels if rel not in tracked)


def check(d):
    mf = d / 'manifest.json'
    if not mf.is_file():
        print(f'no manifest: {mf}')
        return 1
    m, have = json.loads(mf.read_text()), files_of(d)
    for rel in marked_files(d, have):
        print(f'never-sync file in the copy: {rel}; it never leaves schwarzwald')
        return 1
    for rel in sorted(set(m['files']) | set(have)):
        if have.get(rel) != m['files'].get(rel):
            state = (
                'missing'
                if rel not in have
                else 'not in manifest'
                if rel not in m['files']
                else 'differs'
            )
            print(f'{state}: {rel}')
            return 1
    want, got = set(m.get('exec', [])), set(exec_of(d, have))
    for rel in sorted(want ^ got):
        print(f'exec bit {"missing" if rel in want else "not in manifest"}: {rel}')
        return 1
    for rel in untracked(d, list(m['files']) + ['manifest.json']):
        print(f'not tracked by git: {rel}; commit the whole copy (git add {d})')
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(
        check(
            Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent
        )
    )
