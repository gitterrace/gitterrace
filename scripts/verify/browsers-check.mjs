#!/usr/bin/env node
// Every launcher the browser harness can start is installed by `make browser-deps` and by
// scripts/dev/deps-run.sh --browsers (what CI runs), so a browser is never installed yet unused
// or launchable yet missing.
//
//   node scripts/verify/browsers-check.mjs
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { LAUNCHERS, root } from '../lib/browser-lib.mjs';

const want = [...LAUNCHERS].sort().join(' ');
let bad = 0;
for (const f of ['Makefile', 'scripts/dev/deps-run.sh']) {
  const text = readFileSync(join(root, f), 'utf8');
  const m = /\binstall ((?:chromium|webkit|firefox)[a-z ]*)$/m.exec(text);
  const got = m ? m[1].trim().split(/\s+/).sort().join(' ') : '(no install line)';
  if (got !== want) {
    console.error(`browsers-check: ${f} installs [${got}], launch() has [${want}]`);
    bad = 1;
  }
}
process.exit(bad);
