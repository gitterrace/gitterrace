#!/usr/bin/env node
// Takes the website and README screenshots of jq and ripgrep (public MIT projects, the same as
// the live demo `make demo` builds). Self-contained: checks both out at the pins in
// scripts/lib/corpus-lib.mjs (cached in ~/.cache/gitterrace-shots), and serves them with the
// release binary on a free port, with scripts/release/site/site-shots/config as the config
// folder (no tendir in Send to) and a throwaway state folder, so neither the user's config nor a
// running preview is touched. Light and dark images per shot, 1280x800 at 2x, into
// target/site-shots/ (ignored). `make site-shots` builds first and follows this with
// site-render, which writes the WebPs into docs/screenshots/ (the site and the README both link
// those), and `cargo xtask generate`. Not in check: it needs browsers and the network on the
// first run.
//
//   node scripts/release/site/site-shots-run.mjs [NAME...]   all shots by default
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { root, loadPlaywright, launch, freePort } from '../../lib/browser-lib.mjs';
import { checkout } from '../../lib/corpus-lib.mjs';

const dirs = checkout();

const bin = join(root, 'target/release/git-terrace');
const port = await freePort();
const state = mkdtempSync(join(tmpdir(), 'gt-shots-state-'));
const server = spawn(bin, ['serve', '--addr', `127.0.0.1:${port}`, '--no-watch', '--config', join(root, 'scripts/release/site/site-shots/config'), ...Object.values(dirs)], {
  stdio: ['ignore', 'ignore', 'inherit'],
  env: { ...process.env, XDG_STATE_HOME: state },
});
const stop = () => { server.kill(); rmSync(state, { recursive: true, force: true }); };
process.on('exit', stop);
const base = `http://127.0.0.1:${port}`;
for (let i = 0, up = false; !up; i++) {
  try { up = (await fetch(base + '/r/jq/')).ok && (await fetch(base + '/r/ripgrep/')).ok; } catch {}
  if (!up && i > 600) throw new Error(`${bin} did not serve /r/jq and /r/ripgrep on ${base}`);
  if (!up) await new Promise((r) => setTimeout(r, 100));
}

const only = process.argv.slice(2);
const out = join(root, 'target/site-shots');
mkdirSync(out, { recursive: true });

// Stable frames: no animation, no caret, no live-update pulse.
const FREEZE = '*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}';

/**
 * @typedef {{ width: number, height: number }} Size
 * @type {{ name: string, path: string, viewport?: Size, prepare?: (p: any) => Promise<void> }[]}
 */
const SHOTS = [
  { name: 'overview', path: '/r/ripgrep/' },
  // Hotspots has the page header, tab bar, key numbers and the scatter above the fold.
  { name: 'insights', path: '/r/jq/stats/hotspots' },
  // A commit page: message, changed files and a highlighted diff that fits one screen.
  { name: 'commit', path: '/r/ripgrep/commit/068a63e' },
  // configure.ac from the top: 13 blocks and 8 authors in the first 28 lines (scanned over jq and
  // ripgrep).
  { name: 'blame', path: '/r/jq/blame/release/configure.ac' },
  // Bus factor, solo and stale lines, and the owners of each directory.
  { name: 'ownership', path: '/r/jq/stats/ownership' },
  {
    name: 'send',
    // L14-17 sit below the window at the top of the page, so the marked band shows behind it.
    path: '/r/ripgrep/blob/release/crates/core/main.rs#L14-L17',
    async prepare(p) {
      await p.waitForSelector('#actsel');
      await p.click('#actsel');
      await p.waitForSelector('.actmenu');
      await p.click('.actmenu li:has-text("Send to…") >> nth=0');
      await p.waitForSelector('.send, dialog, [role=dialog]', { timeout: 5000 });
      // The site shows kinds of targets, not one app. Serve with an actions.toml without tendir.
      const text = await p.evaluate(() => document.body.innerText);
      if (/tendir/i.test(text)) throw new Error('send shot shows tendir');
      // The fixture's three targets, so a config that did not load fails here, not on the site.
      // Targets arrive after the window opens.
      await p.waitForFunction(() => document.querySelectorAll('select option').length > 1, null, { timeout: 5000 }).catch(() => {});
      const opts = await p.$$eval('select option', (os) => os.map((o) => o.textContent.trim()));
      for (const want of ['Mail draft', 'New issue', 'New merge request']) {
        if (!opts.includes(want)) throw new Error(`send shot: no "${want}" target, got ${JSON.stringify(opts)}`);
      }
      // Same frame as the other shots: the app header at the top, the marked lines in view.
      await p.evaluate(() => window.scrollTo(0, 0));
      // No focus ring on the text field: the frame shows the window at rest.
      await p.evaluate(() => /** @type {HTMLElement} */ (document.activeElement)?.blur());
    },
  },
];

SHOTS.push(
  // The repository page on a phone: the 390 px layout, 2x like the rest.
  { name: 'narrow', path: '/r/ripgrep/', viewport: { width: 390, height: 780 } },
  {
    // The actions menu on a marked line, with the coding agent entry beside the other targets.
    name: 'agent',
    path: '/r/ripgrep/blob/release/crates/core/main.rs#L14-L17',
    async prepare(p) {
      await p.waitForSelector('#actsel');
      await p.click('#actsel');
      await p.waitForSelector('.actmenu');
      const text = await p.evaluate(() => {
        return /** @type {HTMLElement} */ (document.querySelector('.actmenu')).innerText;
      });
      if (!/Ask Claude Code/.test(text)) {
        throw new Error(`agent shot: no "Ask Claude Code" action, got ${JSON.stringify(text)}`);
      }
      // No tooltip over the menu: the pointer rests in a corner.
      await p.mouse.move(5, 5);
      await p.evaluate(() => /** @type {HTMLElement} */ (document.activeElement)?.blur());
      await p.evaluate(() => window.scrollTo(0, 0));
    },
  },
);

const todo = SHOTS.filter((s) => !only.length || only.includes(s.name));
const pw = await loadPlaywright();
const browser = await launch(pw, 'chromium');
for (const scheme of ['light', 'dark']) {
  const ctx = await browser.newContext({ colorScheme: scheme, viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2 });
  for (const s of todo) {
    const p = await ctx.newPage();
    if (s.viewport) await p.setViewportSize(s.viewport);
    await p.goto(base + s.path, { waitUntil: 'networkidle', timeout: 60000 });
    await p.addStyleTag({ content: FREEZE });
    await p.evaluate(() => document.fonts.ready);
    if (s.prepare) await s.prepare(p);
    await p.waitForTimeout(400);
    const file = join(out, `${s.name}-${scheme}.png`);
    await p.screenshot({ path: file });
    await p.close();
    console.log(`wrote target/site-shots/${s.name}-${scheme}.png`);
  }
  await ctx.close();
}
await browser.close();
stop();
