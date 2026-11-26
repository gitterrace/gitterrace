#!/usr/bin/env node
// Builds the live demo into target/demo: `make demo` runs it after the release build. Checks
// jq and ripgrep out at the pins in scripts/lib/corpus-lib.mjs (the screenshots' trees), exports
// both with the release binary as one multi-repo site in public mode, with an empty config
// folder and a throwaway state folder so the user's settings never reach a published page, and
// credits each project in its banner. Then the safety checks: the CSP in _headers matches every
// page (site-check.py csp), and no page names a host path (the cache folder or the home folder).
// Old commits may carry a home path in their own content, which the demo shows as history, so
// every home folder form and every .personal-denylist match is masked first. The check then
// runs on the masked pages and still fails on any form the mask missed.
//
//   node scripts/release/demo-export.mjs [--self-test]
import { execFileSync } from 'node:child_process';
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { root } from '../lib/browser-lib.mjs';
import { PINS, checkout, corpus } from '../lib/corpus-lib.mjs';

// A home folder in POSIX, Windows and JSON-escaped Windows form, up to the user name.
const HOME = /(?:\/Users|\/home|[A-Za-z]:(?:\\\\|\\|\/)Users)(?:\\\\|\\|\/)[^\\/\s"'<>&]+/g;

/** The case-insensitive patterns in .personal-denylist, one per line. */
const denylist = (file) =>
  existsSync(file)
    ? readFileSync(file, 'utf8')
        .split('\n')
        .map((l) => l.trim())
        .filter((l) => l && !l.startsWith('#'))
        .map((l) => new RegExp(l, 'gi'))
    : [];

/** `text` with every home folder as `~` and every denylist match as `[redacted]`. */
const mask = (text, deny) =>
  deny.reduce((t, re) => t.replace(re, '[redacted]'), text.replace(HOME, '~'));

/** True when `text` still names a host path or a denylist match. */
const leaks = (text, hosts, deny) =>
  hosts.some((l) => text.includes(l)) ||
  new RegExp(HOME.source).test(text) ||
  deny.some((re) => new RegExp(re.source, 'i').test(text));

if (process.argv.includes('--self-test')) {
  const deny = [/alice smith/gi];
  const cases = [
    ['"/Users/alice/.cache/x"', '"~/.cache/x"'],
    ['/home/alice/src', '~/src'],
    ['C:\\Users\\alice\\x', '~\\x'],
    ['"C:\\\\Users\\\\alice\\\\x"', '"~\\\\x"'],
    ['C:/Users/alice/x', '~/x'],
    ['by Alice Smith', 'by [redacted]'],
    ['/usr/home-made/x', '/usr/home-made/x'],
  ];
  const bad = cases.filter(([i, o]) => mask(i, deny) !== o || leaks(mask(i, deny), [], deny));
  for (const [i, o] of bad) console.error(`self-test: mask(${i}) is ${mask(i, deny)}, wanted ${o}`);
  const red = ['/Users/bob', '/home/bob', 'C:\\Users\\bob', 'ALICE SMITH', '/tmp/x'];
  const missed = red.filter((t) => !leaks(t, ['/tmp/x'], deny));
  for (const t of missed) console.error(`self-test: leak check passed ${t}`);
  if (bad.length || missed.length) process.exit(1);
  console.log('demo-export self-test: ok');
  process.exit(0);
}

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
const deny = denylist(join(root, '.personal-denylist'));
const hosts = [corpus, homedir(), tmpdir()];
const bad = walk(out).filter((f) => {
  const text = readFileSync(f, 'latin1');
  const masked = mask(text, deny);
  if (masked !== text) writeFileSync(f, masked, 'latin1');
  return leaks(masked, hosts, deny);
});
for (const f of bad) console.error(`demo-export: ${f.slice(root.length + 1)} names a host path`);
if (!readdirSync(out).includes('_headers')) bad.push('_headers');
if (bad.length) process.exit(1);
console.log(`demo-export: ${Object.keys(dirs).join(', ')} in target/demo, checks pass`);
