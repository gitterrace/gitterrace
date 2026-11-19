// A repository whose history fills every part of a listed row: branches ahead of and behind
// main with long names, an annotated tag with a long name and subject, a lightweight tag and
// long commit subjects. Used by the row-overlap check; any check that needs crowded rows can
// serve it too.

import { mkdtempSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { sh, commitAll } from '../../lib/browser-lib.mjs';

/** A branch name long enough to crowd its row. */
export const LONG = 'a-rather-long-branch-name-that-crowds-the-row';

/** The annotated tag, by name, for a check that opens its page. */
export const LONG_TAG = 'v1.0.0-with-a-long-suffix';

/** Builds the repository in a temp folder and returns its path. */
export function crowdedRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-overlap-'));
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  const commit = (i, subject) => {
    writeFileSync(join(dir, 'f.txt'), `${i}\n`);
    commitAll(dir, subject);
  };
  for (let i = 0; i < 6; i++) {
    commit(i, `feat(core): change number ${i} with a subject long enough to fill a row`);
  }
  sh(dir, 'git', 'tag', '-a', LONG_TAG, '-m', 'Release one with a long annotation subject line');
  sh(dir, 'git', 'tag', 'v0.9');
  for (const [name, n] of [[LONG, 3], ['short', 1], [`feature/${LONG}-too`, 12]]) {
    sh(dir, 'git', 'checkout', '-q', '-b', name, 'main~2');
    for (let i = 0; i < Number(n); i++) commit(100 + i, `fix: work on ${name} step ${i}`);
  }
  sh(dir, 'git', 'checkout', '-q', 'main');
  for (let i = 0; i < 3; i++) commit(1000 + i, `chore: keep main ahead ${i}`);
  return dir;
}
