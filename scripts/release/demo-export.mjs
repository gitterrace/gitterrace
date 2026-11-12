#!/usr/bin/env node
// Builds the live demo into target/demo: `make demo` runs it after the release build. Checks
// jq and ripgrep out at the pins in scripts/lib/corpus-lib.mjs (the screenshots' trees), exports
// both with the release binary as one multi-repo site in public mode, with an empty config
// folder and a throwaway state folder so the user's settings never reach a published page, and
// credits each project in its banner. Then the safety checks: the CSP in _headers matches every
// page (site-check.py csp), and no page names a host path (the cache folder or the home folder).
//
//   node scripts/release/demo-export.mjs
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, readdirSync, rmSync, statSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { root } from '../lib/browser-lib.mjs';
import { PINS, checkout, corpus } from '../lib/corpus-lib.mjs';

const out = join(root, 'target/demo');
const bin = join(root, 'target/release/git-terrace');
const dirs = checkout();
const config = mkdtempSync(join(tmpdir(), 'gt-demo-config-'));
const state = mkdtempSync(join(tmpdir(), 'gt-demo-state-'));
try {
  const projects = Object.entries(PINS).flatMap(([n, { url }]) => ['--project', `${n}=${url}`]);
  execFileSync(
    bin,
    ['--public', '--config', config, 'export', out, ...Object.values(dirs), '--quiet', ...projects],
    { stdio: 'inherit', env: { ...process.env, XDG_STATE_HOME: state } },
  );
} finally {
  rmSync(config, { recursive: true, force: true });
  rmSync(state, { recursive: true, force: true });
}

execFileSync('python3', [join(root, 'scripts/release/site/site-check.py'), 'csp', out], {
  stdio: 'inherit',
});

/** Every file under `dir`. */
const walk = (dir) =>
  readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? walk(p) : [p];
  });
const leaks = [corpus, homedir(), tmpdir()];
const bad = walk(out).filter((f) => {
  const text = readFileSync(f, 'latin1');
  return leaks.some((l) => text.includes(l));
});
for (const f of bad) console.error(`demo-export: ${f.slice(root.length + 1)} names a host path`);
if (!readdirSync(out).includes('_headers')) bad.push('_headers');
if (bad.length) process.exit(1);
console.log(`demo-export: ${Object.keys(dirs).join(', ')} in target/demo, checks pass`);
