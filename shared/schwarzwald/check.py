#!/usr/bin/env python3
"""check.py [<copy>]
exit 0 if this copy of schwarzwald equals its manifest.json, file contents and exec bits, 1 naming
the first file that differs. <copy> defaults to the directory this file is in. No network, no
schwarzwald checkout: a product's gate runs the copy's own check.py.

This is integrity, not authenticity: it proves the copy matches its own manifest.json, and whoever
can edit the copy can rewrite the manifest too. `sync.py --check <product>` in a schwarzwald checkout
compares each part to dist/, with exec bits and this file, and is the authenticity check."""

import codecs
import hashlib
import json
import os
import re
import subprocess
import sys
import unicodedata
from pathlib import Path

# A file that never leaves schwarzwald lives in its internal/ folder, which build.py never reads. The
# marker line in a file's first MARKER_LINES lines is the backstop: build.py --check fails on a marked
# file under source/, and a copy refuses one whatever its manifest says.
MARKER_LINES = 5
# Strict, after NFKC: any case, any spacing around the colon, words may follow. The literal is split
# so this file does not read as marked.
MARKER = re.compile(r'^\W*schwarzwald' r'\s*:\s*never-sync\b', re.M | re.I)
# Any line that names the marker in any spelling: build.py --check fails on one MARKER rejects.
NEVER_SYNC = re.compile(r'never\W?sync', re.I)
SCRIPT_SUFFIXES = ('.py', '.sh', '.cjs', '.mjs')


def read_text(path):
    """The file as text, NFKC normalised: utf-16 or utf-32 on its BOM, else utf-8, else latin-1 so
    a file with a NUL or a stray byte is still read whole. None when it cannot be read."""
    try:
        data = path.read_bytes()
    except OSError:
        return None
    if data[:4] in (codecs.BOM_UTF32_LE, codecs.BOM_UTF32_BE):
        enc = 'utf-32'
    elif data[:2] in (codecs.BOM_UTF16_LE, codecs.BOM_UTF16_BE):
        enc = 'utf-16'
    else:
        enc = 'utf-8-sig'
    try:
        text = data.decode(enc)
    except UnicodeError:
        text = data.decode('latin-1')
    return unicodedata.normalize('NFKC', text)


def has_marker(path):
    """True when one of the file's first lines is the never-sync marker."""
    text = read_text(path)
    if text is None:
        return False
    return bool(MARKER.search('\n'.join(text.splitlines()[:MARKER_LINES])))


def stray_markers(d, rels):
    """(path, line number) for each line among rels that names never-sync in a form MARKER does not
    accept, or below the first MARKER_LINES lines: an author who meant to mark the file did not."""
    out = []
    for rel in rels:
        text = read_text(d / rel) or ''
        for n, line in enumerate(text.splitlines(), 1):
            if NEVER_SYNC.search(line) and not (n <= MARKER_LINES and MARKER.search(line)):
                out.append((rel, n))
    return out


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
    """sha256 per file under d, by relative path, the way the manifest lists them. Only
    manifest.json at the root, and target/ and Cargo.lock at a crate root, are left out: a dotfile
    or a target/ anywhere else is a file the manifest must list."""
    out = {}
    for dirpath, dirs, files in os.walk(str(d), followlinks=False):
        here = Path(dirpath)
        crate = here != d and (here / 'Cargo.toml').is_file()
        if crate:
            dirs[:] = [n for n in dirs if n != 'target']
        dirs.sort()
        for name in sorted(files):
            p = here / name
            if p.is_symlink() or (here == d and name == 'manifest.json') or (crate and name == 'Cargo.lock'):
                continue
            out[str(p.relative_to(d))] = hashlib.sha256(p.read_bytes()).hexdigest()
    return dict(sorted(out.items()))


def links_of(d):
    """Every symlink under d, relative to it, never followed. A copy holds only real files: a link
    pointing out of it changes with its target and dangles on another machine."""
    out = []
    for dirpath, dirs, files in os.walk(str(d), followlinks=False):
        for name in dirs + files:
            p = Path(dirpath) / name
            if p.is_symlink():
                out.append(str(p.relative_to(d)))
    return sorted(out)


def exec_of(d, files):
    """The files among `files` that carry any exec bit, sorted."""
    return sorted(rel for rel in files if (d / rel).stat().st_mode & 0o111)


def partial_exec(d, files):
    """The files among `files` whose exec bits are set for some of user, group and other but not
    for the owner: a mode like 655, which an owner-only test would read as not executable."""
    return sorted(rel for rel in files if (d / rel).stat().st_mode & 0o111 and not (d / rel).stat().st_mode & 0o100)


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
    for rel in links_of(d):
        print(f'symlink in the copy: {rel}; a copy holds only real files')
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
    for rel in partial_exec(d, have):
        print(f'exec bits set without the owner bit: {rel}')
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
