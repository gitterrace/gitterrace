#!/usr/bin/env python3
"""drift.py [--repo DIR]
    exit 0 when the product's tool configs keep to the shared bases, 1 with one line per problem.
    - vale: styles/Schwarzwald is the shared style, and no other style under styles/ defines a rule
      with the file name of a shared rule.
    - ruff: ruff.toml extends the shared base and sets none of its keys.
    - typos, clippy, deny, rust-toolchain, cliff: the tool cannot extend, so the root file must carry
      every base key and value, and every base list item. cliff-base.toml names its exempt keys.
    - Cargo.toml: every [workspace.lints] entry of workspace-lints.toml is there at the same level or
      a stricter one (allow, warn, deny, forbid).
    - sgconfig.yml: carries every line of sgconfig-base.yml.
Python 3.11 or newer, standard library only. Reads the shared files next to it."""
import argparse
import sys
from pathlib import Path

import tomllib

HERE = Path(__file__).resolve().parent
EXTEND = 'shared/schwarzwald/configs/ruff-base.toml'
CARRIED = {'_typos.toml': 'typos-base.toml', 'clippy.toml': 'clippy-base.toml', 'deny.toml': 'deny-base.toml',
           'rust-toolchain.toml': 'rust-toolchain-base.toml', 'cliff.toml': 'cliff-base.toml'}
# Keys a product words itself: the changelog header and body carry its name and links.
EXEMPT = {'cliff.toml': {'changelog.header', 'changelog.body'}}
LEVELS = ['allow', 'warn', 'deny', 'forbid']


def load(path):
    return tomllib.loads(path.read_text())


def dropped(base, prod, path='', exempt=()):
    """One line per base key the product drops or changes, and per base list item it drops."""
    out = []
    for k, v in base.items():
        here = f'{path}{k}'
        if here in exempt:
            continue
        if k not in prod:
            out.append(f'{here}: dropped')
        elif isinstance(v, dict):
            out += dropped(v, prod[k], here + '.', exempt) if isinstance(prod[k], dict) else [f'{here}: not a table']
        elif isinstance(v, list):
            out += [f'{here}: drops {item!r}' for item in v if item not in prod[k]]
        elif prod[k] != v:
            out.append(f'{here}: overrides {v!r} with {prod[k]!r}')
    return out


def level(v):
    return v['level'] if isinstance(v, dict) else v


def weaker_lints(base, prod):
    """One line per shared lint the product drops or sets below the shared level."""
    out = []
    for table, lints in base.get('workspace', {}).get('lints', {}).items():
        have = prod.get('workspace', {}).get('lints', {}).get(table, {})
        for name, v in lints.items():
            if name not in have:
                out.append(f'workspace.lints.{table}.{name}: dropped')
            elif LEVELS.index(level(have[name])) < LEVELS.index(level(v)):
                out.append(f'workspace.lints.{table}.{name}: {level(have[name])!r} is weaker than {level(v)!r}')
            elif isinstance(v, dict) and have[name].get('priority') != v.get('priority'):
                out.append(f'workspace.lints.{table}.{name}: priority differs from {v.get("priority")!r}')
    return out


def overridden(base, prod, path=''):
    """One line per base key the product sets itself, which the tool would take over the base."""
    out = []
    for k, v in base.items():
        if k in prod:
            out += overridden(v, prod[k], f'{path}{k}.') if isinstance(v, dict) and isinstance(prod[k], dict) \
                else [f'{path}{k}: set in the product, replacing the shared value; use extend-select or ignore']
    return out


def vale_problems(repo):
    rules = {p.name: p.read_bytes() for p in sorted((HERE / 'vale/Schwarzwald').glob('*.yml'))}
    styles = repo / 'styles'
    own = styles / 'Schwarzwald'
    out = []
    if not own.is_dir():
        out.append('styles/Schwarzwald: missing; link it to shared/schwarzwald/configs/vale/Schwarzwald')
    else:
        out += [f'styles/Schwarzwald/{n}: {"differs from the shared rule" if (own / n).exists() else "missing"}'
                for n, data in rules.items() if not (own / n).exists() or (own / n).read_bytes() != data]
        out += [f'styles/Schwarzwald/{p.name}: not a shared rule' for p in sorted(own.glob('*.yml')) if p.name not in rules]
    for d in sorted(p for p in styles.glob('*') if p.is_dir() and p.name != 'Schwarzwald'):
        out += [f'styles/{d.name}/{n}: redefines the shared rule Schwarzwald.{n[:-4]}' for n in rules if (d / n).exists()]
    return out


def problems(repo):
    out = vale_problems(repo) if (repo / 'styles').is_dir() else []
    if (repo / 'ruff.toml').is_file():
        prod = load(repo / 'ruff.toml')
        if prod.get('extend') != EXTEND:
            out.append(f'ruff.toml: extend is {prod.get("extend")!r}, want {EXTEND!r}')
        out += [f'ruff.toml: {p}' for p in overridden(load(HERE / 'ruff-base.toml'), prod)]
    for name, base in CARRIED.items():
        if (repo / name).is_file():
            out += [f'{name}: {p}' for p in dropped(load(HERE / base), load(repo / name), exempt=EXEMPT.get(name, ()))]
    if (repo / 'Cargo.toml').is_file():
        out += [f'Cargo.toml: {p}' for p in weaker_lints(load(HERE / 'workspace-lints.toml'), load(repo / 'Cargo.toml'))]
    if (repo / 'sgconfig.yml').is_file():
        have = {ln.strip() for ln in (repo / 'sgconfig.yml').read_text().splitlines()}
        out += [f'sgconfig.yml: drops {ln!r}' for ln in (HERE / 'sgconfig-base.yml').read_text().splitlines()
                if ln.strip() and not ln.lstrip().startswith('#') and ln.strip() not in have]
    return out


if __name__ == '__main__':
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--repo', type=Path, default=Path.cwd())
    found = problems(ap.parse_args().repo.resolve())
    print('\n'.join(found) or 'ok')
    sys.exit(1 if found else 0)
