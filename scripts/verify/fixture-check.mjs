#!/usr/bin/env node
// Guard: a browser case that leaves the shared repository changed fails (lib/browser-lib.mjs
// `fixtureDrift`), so no later case passes or fails by the order of the run. Red: a rewritten
// tracked file, a deleted one, a moved tag, another branch left checked out and an uncommitted
// edit must each be reported. Green: a new file in a new commit, and a change put back, must
// not be: nor an edit a case made on purpose and took back, so a case may test a dirty tree.
// Taking the state must not write the repository either. Needs git only, no browser and no
// server.
//
//   node scripts/verify/fixture-check.mjs
import { readFileSync, writeFileSync, rmSync, utimesSync } from 'node:fs';
import { join } from 'node:path';
import {
  makeRepo,
  commitAll,
  sh,
  removeAll,
  fixtureState,
  fixtureDrift,
} from '../lib/browser-lib.mjs';

let bad = 0;
const say = (ok, m) => { if (!ok) bad++; console.log(ok ? 'ok  ' : 'FAIL', m); };

/** What `fixtureDrift` reports after `change` ran in a fresh repository. */
function drift(change) {
  const dir = makeRepo();
  try {
    sh(dir, 'git', 'tag', 'v1');
    const was = fixtureState(dir);
    change(dir);
    return fixtureDrift(was, fixtureState(dir)).join('; ');
  } finally {
    removeAll(dir);
  }
}

const RED = {
  'a rewritten tracked file': { want: /file README\.md changed/, change: (dir) => {
    writeFileSync(join(dir, 'README.md'), '# Other\n');
    commitAll(dir, 'docs: other');
  } },
  'a deleted tracked file': { want: /file a\.txt is gone/, change: (dir) => {
    rmSync(join(dir, 'a.txt'));
    commitAll(dir, 'chore: drop');
  } },
  'a moved tag': { want: /ref refs\/tags\/v1 changed/, change: (dir) => {
    writeFileSync(join(dir, 'new.txt'), 'x\n');
    commitAll(dir, 'feat: new');
    sh(dir, 'git', 'tag', '-f', 'v1');
  } },
  'another branch left checked out': { want: /on side, was on main/, change: (dir) => {
    sh(dir, 'git', 'checkout', '-q', '-b', 'side');
  } },
  'an uncommitted edit': { want: /uncommitted: M a\.txt/, change: (dir) => {
    writeFileSync(join(dir, 'a.txt'), 'dirty\n');
  } },
};
const GREEN = {
  'a new file in a new commit': (dir) => {
    writeFileSync(join(dir, 'case-own.md'), '# Own\n');
    commitAll(dir, 'docs: own fixture');
  },
  'a change put back': (dir) => {
    writeFileSync(join(dir, 'README.md'), '# Other\n');
    commitAll(dir, 'docs: other');
    sh(dir, 'git', 'checkout', '-q', 'HEAD~1', '--', 'README.md');
    commitAll(dir, 'docs: restore');
  },
  'an uncommitted edit taken back': (dir) => {
    writeFileSync(join(dir, 'a.txt'), 'one\ndirty\n');
    sh(dir, 'git', 'checkout', '-q', '--', 'a.txt');
  },
};

/** Whether `fixtureState` left the index file as it was, with an entry whose stat is stale. */
function readOnly() {
  const dir = makeRepo();
  try {
    const later = new Date(Date.now() + 5000);
    utimesSync(join(dir, 'a.txt'), later, later);
    const index = () => readFileSync(join(dir, '.git/index')).toString('hex');
    const was = index();
    fixtureState(dir);
    return index() === was;
  } finally {
    removeAll(dir);
  }
}

for (const [what, { want, change }] of Object.entries(RED)) {
  const got = drift(change);
  say(want.test(got), `red: ${what} is reported${want.test(got) ? '' : ` (got "${got}")`}`);
}
for (const [what, change] of Object.entries(GREEN)) {
  const got = drift(change);
  say(got === '', `green: ${what} is allowed${got ? ` (got "${got}")` : ''}`);
}
say(readOnly(), 'taking the state leaves a stale index unwritten');
process.exit(bad ? 1 : 0);
