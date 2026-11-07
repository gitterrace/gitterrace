#!/usr/bin/env python3
"""check.py [<copy>]
    exit 0 if this copy of schwarzwald equals its manifest.json, file contents and exec bits, 1 naming
    the first file that differs. <copy> defaults to the directory this file is in. No network, no
    schwarzwald checkout: a product's gate runs the copy's own check.py."""
import fnmatch, hashlib, json, os, re, sys
from pathlib import Path

SKIP = {'manifest.json', 'Cargo.lock'}
# Files that never leave schwarzwald are listed in never-sync.txt, which stays in schwarzwald: sync.py and
# build.py --check enforce it. A copy refuses any file outside its manifest, and a file that carries the
# marker line in its first MARKER_LINES lines whatever its manifest says, since the marker needs no list.
NEVER_SYNC = 'never-sync.txt'
MARKER_LINES = 5
MARKER = re.compile(r'^\W*schwarzwald' r': never-sync\W*$', re.M)


def never_sync_list(path):
    """The entries of a never-sync list: one path or glob per line, relative to dist/, # comments."""
    if not path.is_file():
        return []
    return [line for line in (raw.split('#', 1)[0].strip() for raw in path.read_text().splitlines()) if line]


def has_marker(path):
    """True when one of the file's first lines is the never-sync marker and nothing else."""
    try:
        with path.open(errors='replace') as f:
            head = ''.join(line for _, line in zip(range(MARKER_LINES), f))
    except (OSError, UnicodeError):
        return False
    return bool(MARKER.search(head))


def never_sync_hits(d, rels, entries):
    """The paths among rels, relative to d, that a never-sync entry or marker keeps out."""
    return sorted(rel for rel in rels if any(fnmatch.fnmatchcase(rel, e) for e in entries) or has_marker(d / rel))


def name_leaks(d, rels, entries):
    """(path, name) for each file among rels whose text names a never-sync entry by its file stem."""
    names = {n for n in (re.split(r'[*?[]', Path(e).stem)[0] for e in entries) if len(n) >= 4}
    out = []
    for rel in rels:
        try:
            text = (d / rel).read_text(errors='replace')
        except OSError:
            continue
        out += [(rel, n) for n in sorted(names) if n in text or n in rel]
    return out


def files_of(d):
    """sha256 per file under d, by relative path, the way the manifest lists them."""
    return {str(p.relative_to(d)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(d.rglob('*')) if p.is_file() and p.name not in SKIP and not p.name.startswith('.')
            and 'target' not in p.relative_to(d).parts}


def exec_of(d, files):
    """The files among `files` whose owner exec bit is set, sorted."""
    return sorted(rel for rel in files if os.access(d / rel, os.X_OK))


def check(d):
    mf = d / 'manifest.json'
    if not mf.is_file():
        print(f'no manifest: {mf}')
        return 1
    m, have = json.loads(mf.read_text()), files_of(d)
    for rel in never_sync_hits(d, have, []):
        print(f'never-sync file in the copy: {rel}; it never leaves schwarzwald')
        return 1
    for rel in sorted(set(m['files']) | set(have)):
        if have.get(rel) != m['files'].get(rel):
            state = 'missing' if rel not in have else 'not in manifest' if rel not in m['files'] else 'differs'
            print(f'{state}: {rel}')
            return 1
    want, got = set(m.get('exec', [])), set(exec_of(d, have))
    for rel in sorted(want ^ got):
        print(f'exec bit {"missing" if rel in want else "not in manifest"}: {rel}')
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(check(Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent))
