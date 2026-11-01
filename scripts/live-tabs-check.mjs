// Guard for the live transport: many open tabs must neither starve navigation (WebKit allows 6
// HTTP/1.1 connections per host) nor stop live updates. Serves a temp repo on a spare port, opens
// N tabs in one context, times one more navigation (< 1 s), then with 3 tabs touches a tracked
// file and asserts every tab shows the Working tree chip within 2 s.
//
//   node scripts/live-tabs-check.mjs [--bin target/debug/git-terrace] [--from-disk] [--tabs 8,12]
//        [--browsers webkit,chromium]
// --from-disk serves crates/gitterrace-web/static/*.js from the working tree over the (possibly
// older) binary's copies, to test edits before a rebuild. Playwright is resolved from
// $PLAYWRIGHT_CORE or the website checkout; no network is used. Not in `make`: it needs a browser.
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, readFileSync } from 'node:fs';
import { tmpdir, homedir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import net from 'node:net';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const arg = (k, d) => { const i = process.argv.indexOf(k); return i < 0 ? d : process.argv[i + 1]; };
const bin = resolve(arg('--bin', join(root, 'target/debug/git-terrace')));
const fromDisk = process.argv.includes('--from-disk');
const tabCounts = arg('--tabs', '8,12').split(',').map(Number);
const browsers = arg('--browsers', 'webkit,chromium').split(',');
const pw = await import(process.env.PLAYWRIGHT_CORE || join(homedir(), 'github/tendir/website/node_modules/playwright-core/index.mjs'));
const chromiumPath = join(homedir(), 'Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing');

const port = await new Promise((ok) => { const s = net.createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => ok(p)); }); });
const dir = mkdtempSync(join(tmpdir(), 'gt-live-'));
const sh = (...a) => execFileSync(a[0], a.slice(1), { cwd: dir, stdio: 'pipe', env: { ...process.env, GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t' } });
sh('git', 'init', '-q', '-b', 'main');
writeFileSync(join(dir, 'a.txt'), 'one\n');
sh('git', 'add', '.'); sh('git', 'commit', '-q', '-m', 'init');
const server = spawn(bin, ['serve', '--addr', `127.0.0.1:${port}`, '--poll', '250', '--config', join(dir, 'cfg'), dir], { stdio: 'ignore' });
const origin = `http://127.0.0.1:${port}`;
for (let i = 0; i < 100; i++) { try { if ((await fetch(origin + '/')).ok) break; } catch {} await new Promise((r) => setTimeout(r, 100)); }

// Land on a page that follows the working tree, wherever the repo is mounted.
const home = await (await fetch(origin + '/')).text();
const m = /href="(\/r\/[^"\/]+)\/?"/.exec(home);
const url = (m ? origin + m[1] : origin) + '/worktree';

console.log('url', url);
let failed = false;
const fail = (msg) => { failed = true; console.log('FAIL', msg); };
async function ctxFor(name) {
  const b = await (name === 'webkit' ? pw.webkit.launch() : pw.chromium.launch({ executablePath: chromiumPath }));
  const ctx = await b.newContext();
  if (fromDisk) for (const f of ['live.js', 'actions.js', 'app.js']) {
    await ctx.route(`**/static/${f}`, (r) => r.fulfill({ contentType: 'text/javascript', body: readFileSync(join(root, 'crates/gitterrace-web/static', f)) }));
  }
  return { b, ctx };
}

try {
  for (const name of browsers) {
    for (const n of tabCounts) {
      const { b, ctx } = await ctxFor(name);
      for (let i = 0; i < n; i++) {
        const p = await ctx.newPage();
        await p.goto(url, { waitUntil: 'load', timeout: 5000 }).catch(() => fail(`${name} ${n} tabs: tab ${i} did not load within 5 s`));
      }
      const last = await ctx.newPage();
      const t = Date.now();
      try {
        await last.goto(url, { waitUntil: 'load', timeout: 10000 });
        const ms = Date.now() - t;
        console.log(`${name}: after ${n} tabs next page loaded in ${ms} ms`);
        if (ms > 1000) fail(`${name} ${n} tabs: ${ms} ms > 1000`);
      } catch { fail(`${name} ${n} tabs: next page did NOT load within 10 s`); }
      await b.close();
    }
  }
  for (const name of browsers) {
    const { b, ctx } = await ctxFor(name);
    // Tabs are opened one after another; only the last is visible in headless, so all three are
    // exercised through the leader relay or their own stream alike.
    const pages = [];
    for (let i = 0; i < 3; i++) { const p = await ctx.newPage(); await p.goto(url, { waitUntil: 'load' }); pages.push(p); }
    await new Promise((r) => setTimeout(r, 1000));
    writeFileSync(join(dir, 'a.txt'), `one\ntwo ${name}\n`);
    const t = Date.now();
    const results = await Promise.all(pages.map((p) => p.waitForSelector('#wtchip:not([hidden])', { timeout: 2000 }).then(() => Date.now() - t, () => null)));
    results.forEach((ms, i) => { if (ms == null) fail(`${name} tab ${i}: no working tree notice within 2 s`); });
    console.log(`${name}: working tree notice per tab (ms): ${results.join(', ')}`);
    await b.close();
    sh('git', 'checkout', '-q', '--', 'a.txt');
    await new Promise((r) => setTimeout(r, 800));
  }
} finally { server.kill(); }
console.log(failed ? 'RESULT: FAIL' : 'RESULT: ok');
process.exit(failed ? 1 : 0);
