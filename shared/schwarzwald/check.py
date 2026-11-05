#!/usr/bin/env python3
"""check.py [<copy>]
    exit 0 if this copy of schwarzwald equals its manifest.json, file contents and exec bits, 1 naming
    the first file that differs. <copy> defaults to the directory this file is in. No network, no
    schwarzwald checkout: a product's gate runs the copy's own check.py."""
import hashlib, json, os, sys
from pathlib import Path

SKIP = {'manifest.json', 'Cargo.lock'}


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
