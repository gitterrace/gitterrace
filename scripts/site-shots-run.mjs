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
  // configure.ac from the top: 13 blocks and 8 authors in the first 28 lines (scanned over jq and ripgrep).
  { name: 'blame', path: '/r/jq/blame/release/configure.ac' },
  {
    name: 'send',
    path: '/r/jq/blob/release/src/main.c#L108-L110',
    async prepare(p) {
      await p.waitForSelector('#actsel');
      await p.click('#actsel');
      await p.waitForSelector('.actmenu');
      await p.click('.actmenu li:has-text("Send to…") >> nth=0');
      await p.waitForSelector('.send, dialog, [role=dialog]', { timeout: 5000 });
      // A narrow file view jumps sideways to the anchor; keep the code at its left edge.
      await p.evaluate(() => document.querySelectorAll('*').forEach((e) => { if (e.scrollLeft) e.scrollLeft = 0; }));
    },
    // The dialog and the marked lines around it, so the small window reads at site width.
    clip: '.send, dialog, [role=dialog]',
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
    let clip;
    if (s.clip) {
      const b = await p.locator(s.clip).first().boundingBox();
      const pad = 48;
      const x = Math.max(0, b.x - 160);
      const y = Math.max(0, b.y - pad);
      clip = { x, y, width: Math.min(1280 - x, b.width + 320), height: Math.min(800 - y, b.height + pad * 2) };
    }
    await p.screenshot({ path: file, clip });
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
