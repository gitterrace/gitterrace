#!/usr/bin/env node
// Browser checks for the served pages, one harness. Serves a temp repository with a given binary
// on a free port and runs the checks below in real browsers. Not in `make check`: it needs
// browsers. Run it with `make browser-check BIN=target/debug/git-terrace`.
//
//   node scripts/verify/browser/browser-check.mjs [--bin PATH] [--checks a,b]
//        [--browsers webkit,chromium,firefox] [--tabs 8,12] [--from-disk]
//
// Every check is an export of a scripts/verify/browser/*-lib.mjs `checks(env)`, described where it
// is defined. `--checks` names them in kebab-case; an unknown name prints the list.
//
// --from-disk serves crates/gitterrace-web/static/*.js and *.css from the working tree over the
// (possibly older) binary's copies, to test edits before a rebuild. Playwright comes from
// $PLAYWRIGHT_CORE or the repo's node_modules, installed by `make browser-deps` (see
// lib/browser-lib.mjs).
import {
  readFileSync,
  readdirSync,
  writeFileSync,
  rmSync,
  mkdirSync,
  mkdtempSync,
  existsSync,
} from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import {
  root,
  RESCAN_SECS,
  loadPlaywright,
  launch,
  routeFromDisk,
  makeRepo,
  serve,
  openPage,
  reporter,
  sh,
} from '../../lib/browser-lib.mjs';

const arg = (k, d) => { const i = process.argv.indexOf(k); return i < 0 ? d : process.argv[i + 1]; };
const bin = resolve(arg('--bin', join(root, 'target/debug/git-terrace')));
const fromDisk = process.argv.includes('--from-disk');
const tabCounts = arg('--tabs', '8,12').split(',').map(Number);
const browsers = arg('--browsers', 'webkit,chromium,firefox').split(',');
const ALL = ['truncation', 'live-tabs', 'mermaid', 'chip', 'doctabs', 'chords', 'header', 'marked', 'menu-same', 'menu-keys', 'gutter', 'overlays', 'overlay-native', 'paned-nojs', 'tips', 'date-tips', 'repo-switch', 'outline', 'mark-seam', 'about-place', 'site-theme', 'install-tabs', 'site-bar', 'site-rail', 'site-tables', 'doc-menus', 'bar-steady', 'side-width', 'calm-header', 'header-theme', 'tabs-one-line', 'tab-levels', 'huge-diff', 'diff-find', 'sticky-heads', 'md-anchor', 'live-scroll', 'live-read', 'worktree-scroll', 'sticky-line', 'seg-one-row', 'period-steady', 'period-click', 'range-graph', 'export-from-disk', 'export-switch', 'inline-controls', 'section-gaps', 'path-cells', 'view-file', 'code-rows', 'mark-flat', 'tab-stops', 'help-keys', 'side-order', 'settings', 'lone-rows', 'page-heads', 'crumb-flow', 'compare-head', 'commit-head', 'search-head', 'small-layout', 'send-dialog', 'keyboard-walk', 'reflow', 'text-spacing', 'targets', 'keys-off', 'axe-all', 'forced-colors', 'reduced-motion', 'outline-menu', 'avatar-fit', 'graph-view', 'layout-shift', 'head-align', 'symbols', 'outline-current', 'no-sideways-scroll', 'head-fits', 'repos-filter', 'log-filter', 'log-filter-help', 'home-live', 'row-overlap', 'blame-fit', 'inset-balance', 'anatomy', 'popup-place', 'buttons', 'line-hover', 'font-sample', 'file-card-fragment', 'phone-layout'];
const wanted = arg('--checks', ALL.join(',')).split(',');
for (const c of wanted) if (!ALL.includes(c)) { console.error(`unknown check ${c}; known: ${ALL.join(', ')}`); process.exit(2); }

const pw = await loadPlaywright(browsers);
const dir = makeRepo();
// The chords check needs one action, key "x", that touches a marker file. It has to exist
// before the server starts (read at load) and outside the repository (refused inside one).
const cfg = mkdtempSync(join(tmpdir(), 'gt-cfg-'));
const marker = join(cfg, 'chord-ran');
writeFileSync(join(cfg, 'actions.toml'), `version = 1\n[[action]]\nid = "mark"\nlabel = "Mark"\nkinds = ["page"]\ncommand = ["/usr/bin/touch", ${JSON.stringify(marker)}]\nkey = "x"\n[[action]]\nid = "lines"\nlabel = "Lines only"\nkinds = ["line"]\ncommand = ["/usr/bin/true"]\n`, { mode: 0o600 });
writeFileSync(join(cfg, 'config.toml'), `[watch]\nrescan_secs = ${RESCAN_SECS}\n`);
const srv = await serve(bin, dir, cfg);
const r = reporter();
console.log('serving', srv.base, 'with', bin);

async function context(name, opts = {}) {
  const b = await launch(pw, name);
  const ctx = await b.newContext(opts);
  await ctx.addCookies([await srv.owner()]);
  // Every script and stylesheet in the folder, read at run time, so a new or forgotten file is
  // never served stale from the binary while the check claims to test the working tree.
  if (fromDisk) await routeFromDisk(ctx);
  return { b, ctx };
}


const env = { pw, dir, srv, r, browsers, tabCounts, marker, context, arg, bin, cfg };
const LIB = /^[a-z]+-lib\.mjs$/;
// Check names whose export is not the camelCase of the name.
const ALIAS = { 'paned-nojs': 'panedNoJs', 'view-file': 'viewFileColumn', 'crumb-flow': 'crumbHeads', 'compare-head': 'compareHeads', 'commit-head': 'commitHeads', 'search-head': 'searchHeads' };
const exported = {};
const here = dirname(fileURLToPath(import.meta.url));
for (const f of readdirSync(here).filter((n) => LIB.test(n)).sort()) {
  Object.assign(exported, (await import(`./${f}`)).checks?.(env) ?? {});
}
const checks = Object.fromEntries(ALL.map((c) => [c, exported[ALIAS[c] ?? c.replace(/-(\w)/g, (_, x) => x.toUpperCase())]]));
for (const c of ALL) if (!checks[c]) { console.error(`no export for check ${c}`); process.exit(2); }
try {
  for (const c of wanted) { console.log(`-- ${c}`); await checks[c](); }
} finally {
  srv.stop();
  rmSync(dir, { recursive: true, force: true });
  rmSync(cfg, { recursive: true, force: true });
}
process.exit(r.result());
