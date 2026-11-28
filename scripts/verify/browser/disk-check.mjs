#!/usr/bin/env node
// Guard: --from-disk serves the working tree's stylesheets and scripts over the binary's. The
// page asks for them as `/static/app.js?v=<build>`, and a route on the bare name never matched
// that, so a disk edit never reached the page. The stylesheets come as bundles (base.css is
// app.css and others joined), so a route per file never covered them either. This copies the
// static folder, plants a rule in its app.css and a global in its app.js, loads a served page
// through routeFromDisk and reads both back. The same page through the old bare-name glob must
// NOT show them: that proves the probe can go red. The css and the js are read separately, so a
// route covering only one file type, or only unbundled sheets, also goes red.
//
//   node scripts/verify/browser/disk-check.mjs [--bin target/debug/git-terrace] [--browser webkit]
import { cpSync, appendFileSync, readdirSync, readFileSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import {
  root,
  loadPlaywright,
  launch,
  makeRepo,
  serve,
  routeFromDisk,
  removeAll,
} from '../../lib/browser-lib.mjs';

const arg = (k, d) => {
  const i = process.argv.indexOf(k);
  return i < 0 ? d : process.argv[i + 1];
};
const bin = resolve(arg('--bin', join(root, 'target/debug/git-terrace')));
const name = arg('--browser', 'webkit');

/** What the old route did: a glob on the bare file name, which a `?v=` URL never matches. */
async function bareNameRoute(ctx, dir) {
  for (const f of readdirSync(dir).filter((n) => /\.(js|css)$/.test(n))) {
    const contentType = f.endsWith('.css') ? 'text/css' : 'text/javascript';
    const body = readFileSync(join(dir, f));
    await ctx.route(`**/static/${f}`, (q) => q.fulfill({ contentType, body }));
  }
}

/** In the page: the planted stylesheet rule and script global, as the page sees them. */
const seen = () => ({
  css: getComputedStyle(document.documentElement).getPropertyValue('--disk-probe').trim(),
  js: /** @type {any} */ (window).__diskProbe === 1,
});

const pw = await loadPlaywright([name]);
const dir = makeRepo();
const copy = mkdtempSync(join(tmpdir(), 'gt-disk-'));
cpSync(join(root, 'crates/gitterrace-web/static'), copy, { recursive: true });
appendFileSync(join(copy, 'app.css'), '\n:root{--disk-probe:planted}\n');
appendFileSync(join(copy, 'app.js'), '\nwindow.__diskProbe = 1;\n');
const srv = await serve(bin, dir);
const b = await launch(pw, name);
let bad = 0;
const say = (ok, m) => { if (!ok) bad++; console.log(ok ? 'ok  ' : 'FAIL', m); };
try {
  const load = async (route) => {
    const ctx = await b.newContext();
    await route(ctx);
    const p = await ctx.newPage();
    await p.goto(srv.base + '/', { waitUntil: 'load' });
    const m = await p.evaluate(seen);
    await ctx.close();
    return m;
  };
  const disk = await load((ctx) => routeFromDisk(ctx, copy));
  const css = disk.css || 'absent';
  say(disk.css === 'planted', `from-disk: a disk edit to a stylesheet shows in the page (${css})`);
  say(disk.js, 'from-disk: a disk edit to a script shows in the page');
  const old = await load((ctx) => bareNameRoute(ctx, copy));
  say(old.css !== 'planted' && !old.js,
    'from-disk: the bare-name route never shows the edits, so the probe can go red');
} finally {
  await b.close();
  await srv.stop();
  removeAll(dir);
  removeAll(copy);
}
process.exit(bad ? 1 : 0);
