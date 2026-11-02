#!/usr/bin/env node
// Renders the macOS app icon for `install-handler` from the brand mark:
//   crates/gitterrace/assets/gitterrace.icns
// The light mark on a rounded square in the dark page colour, on Apple's 1024 grid (an 824
// square, radius 185). Dark on purpose: macOS dims and tints icons in dark mode, and a light
// tile then turns grey under a dark mark, while the light mark on dark reads in both. Every
// size iconutil wants is drawn. The mark's 32 grid holds the 28-unit stair box, drawn 560
// wide (the art box), so the image is 640 and the stair keeps the ratio of every other size.
// Chromium draws the SVG, and the same build of it gives the same pixels each run. The .icns
// is committed because the binary embeds it; rerun this (`make app-icon`) when
// docs/brand/mark-dark.svg changes.
//
//   node scripts/icon-render.mjs
import { readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { root, loadPlaywright, launch } from './browser-lib.mjs';

const mark = 'data:image/svg+xml,' + encodeURIComponent(readFileSync(join(root, 'docs/brand/mark-dark.svg'), 'utf8'));
// The dark page background: the second --n-1 in tokens.css, the dark theme's.
const bg = [...readFileSync(join(root, 'crates/gitterrace-web/static/tokens.css'), 'utf8').matchAll(/--n-1:(#[0-9a-f]{6})/g)][1][1];
// The art box is 560 for the 28-unit stair, so the 32-unit image is 560 * 32 / 28.
const ART = (560 * 32) / 28;
const set = join(root, 'target/app-icon/gitterrace.iconset');
rmSync(set, { recursive: true, force: true });
mkdirSync(set, { recursive: true });

const pw = await loadPlaywright();
const b = await launch(pw, 'chromium');
for (const px of [16, 32, 64, 128, 256, 512, 1024]) {
  const p = await b.newPage({ viewport: { width: px, height: px } });
  const k = px / 1024;
  await p.setContent(`<body style="margin:0;width:${px}px;height:${px}px;display:grid;place-items:center;background:transparent">
<div style="width:${824 * k}px;height:${824 * k}px;border-radius:${185 * k}px;background:${bg};display:grid;place-items:center;box-shadow:0 ${10 * k}px ${20 * k}px rgba(0,0,0,.18)">
<img src="${mark}" style="width:${ART * k}px;height:${ART * k}px"></div></body>`);
  await p.waitForFunction(() => document.images[0].complete);
  const png = await p.screenshot({ omitBackground: true });
  // iconutil wants 16, 32, 128, 256 and 512, each at 1x and @2x: a render at px is px at 1x
  // and px/2 at @2x.
  const sizes = [16, 32, 128, 256, 512];
  if (sizes.includes(px)) writeFileSync(join(set, `icon_${px}x${px}.png`), png);
  if (sizes.includes(px / 2)) writeFileSync(join(set, `icon_${px / 2}x${px / 2}@2x.png`), png);
  await p.close();
}
await b.close();
const outDir = join(root, 'crates/gitterrace/assets');
mkdirSync(outDir, { recursive: true });
const r = spawnSync('iconutil', ['-c', 'icns', set, '-o', join(outDir, 'gitterrace.icns')], { stdio: 'inherit' });
if (r.status !== 0) { console.error('app-icon: iconutil failed (macOS only)'); process.exit(1); }
console.log('wrote crates/gitterrace/assets/gitterrace.icns');
