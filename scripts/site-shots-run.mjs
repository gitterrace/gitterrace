// Takes the website and README screenshots from a running git-terrace that serves jq and
// ripgrep (public MIT projects, the same as the demo). Light and dark images per shot, 1280x800
// at 2x, into docs/screenshots/. A release asset: run it by hand (`make site-shots`), not in check.
//
//   node scripts/site-shots-run.mjs [BASE_URL] [NAME...]   default http://127.0.0.1:4242, all shots
import { mkdirSync, statSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { root, loadPlaywright, launch } from './browser-lib.mjs';

const base = (process.argv[2] || 'http://127.0.0.1:4242').replace(/\/$/, '');
const only = process.argv.slice(3);
const out = join(root, 'docs/screenshots');
mkdirSync(out, { recursive: true });

// Stable frames: no animation, no caret, no live-update pulse.
const FREEZE = '*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}';

const SHOTS = [
  { name: 'overview', path: '/r/ripgrep/' },
  // Hotspots has the page header, tab bar, key numbers and the scatter above the fold.
  { name: 'insights', path: '/r/jq/stats/hotspots' },
  // A commit page: message, changed files and a highlighted diff that fits one screen.
  { name: 'commit', path: '/r/ripgrep/commit/068a63e' },
  // configure.ac from the top: 13 blocks and 8 authors in the first 28 lines (scanned over jq and ripgrep).
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
      // Same frame as the other shots: the app header at the top, the marked lines in view.
      await p.evaluate(() => window.scrollTo(0, 0));
    },
  },
];

const todo = SHOTS.filter((s) => !only.length || only.includes(s.name));
const pw = await loadPlaywright();
const browser = await launch(pw, 'chromium');
for (const scheme of ['light', 'dark']) {
  const ctx = await browser.newContext({ colorScheme: scheme, viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2 });
  for (const s of todo) {
    const p = await ctx.newPage();
    await p.goto(base + s.path, { waitUntil: 'networkidle', timeout: 60000 });
    await p.addStyleTag({ content: FREEZE });
    await p.evaluate(() => document.fonts.ready);
    if (s.prepare) await s.prepare(p);
    await p.waitForTimeout(400);
    const file = join(out, `${s.name}-${scheme}.png`);
    await p.screenshot({ path: file });
    await p.close();
    console.log(`wrote docs/screenshots/${s.name}-${scheme}.png`);
  }
  await ctx.close();
}
await browser.close();

// Smaller files when a PNG optimizer is installed; the shots are the same either way.
const files = todo.flatMap((s) => ['light', 'dark'].map((c) => join(out, `${s.name}-${c}.png`)));
for (const [tool, args] of [['oxipng', ['-q', '-o', '4', '--strip', 'safe']], ['pngquant', ['--force', '--skip-if-larger', '--ext', '.png', '--quality', '70-95']]]) {
  try {
    execFileSync(tool, [...args, ...files], { stdio: 'ignore' });
    console.log(`optimized with ${tool}`);
    break;
  } catch {}
}
for (const f of files) console.log(`${f.slice(root.length + 1)} ${(statSync(f).size / 1024).toFixed(0)} KiB`);
