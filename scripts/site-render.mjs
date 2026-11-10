#!/usr/bin/env node
// Renders the raster images the site needs from the brand SVGs, and the site's WebP
// screenshots (below). The link preview is rendered at deploy and ignored; the two plated
// marks are committed, since the site and the org page link them:
//   docs/brand/avatar-512.png         512x512, the plated mark (the org avatar, committed)
//   docs/brand/apple-touch-icon.png   180x180, the plated mark (iOS home screen, committed;
//                                     the site copies it to assets/icons/)
//   target/site/assets/og.png         1200x630, the light README header (link previews)
// Chromium draws the SVGs, the same renderer as the app icon.
//
//   node scripts/site-render.mjs
import { existsSync, readFileSync, mkdirSync, readdirSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { root, loadPlaywright, launch } from './lib/browser-lib.mjs';

const brand = (f) => 'data:image/svg+xml,' + encodeURIComponent(readFileSync(join(root, 'docs/brand', f), 'utf8'));
// The light page background, from the shared tokens, so the PNG files match the site.
const bg = /--n-1:light-dark\((#[0-9a-f]{6}),/.exec(readFileSync(join(root, 'crates/gitterrace-web/static/tokens.css'), 'utf8'))[1];
// Into the built site, so `make docs-gen` (which empties it) runs first.
const out = join(root, 'target/site/assets');
mkdirSync(out, { recursive: true });
const plate = brand('mark-plate.svg');

const shots = [
  { file: 'apple-touch-icon.png', dir: join(root, 'docs/brand'), w: 180, h: 180, img: plate, size: 'width:180px;height:180px', bg: 'transparent' },
  { file: 'avatar-512.png', dir: join(root, 'docs/brand'), w: 512, h: 512, img: plate, size: 'width:512px;height:512px', bg: 'transparent' },
  { file: 'og.png', w: 1200, h: 630, img: brand('readme-header-light.svg'), size: 'width:1000px' },
];
const pw = await loadPlaywright();
const b = await launch(pw, 'chromium');
for (const s of shots) {
  const p = await b.newPage({ viewport: { width: s.w, height: s.h }, colorScheme: 'light' });
  await p.setContent(`<body style="margin:0;height:100vh;display:grid;place-items:center;background:${s.bg ?? bg}"><img src="${s.img}" style="${s.size}"></body>`);
  await p.waitForFunction(() => document.images[0].complete);
  await p.screenshot({ path: join(s.dir ?? out, s.file), omitBackground: s.bg === 'transparent' });
  console.log(`wrote ${s.file} (${s.w}x${s.h})`);
}
await b.close();

// The site's screenshots, from the raw PNG files `make site-shots` leaves in target/site-shots/
// (ignored, 2560x1600): a full-size `{name}.webp` for the lightbox and a 1280x800
// `{name}.thumb.webp` for the tile, both into docs/screenshots/ and committed, the one source the
// site copies. Without raw PNG files (a fresh clone, a deploy) the committed WebPs stay as they
// are. `make docs-gen` checks that the folder holds exactly these files at exactly these sizes.
const rawDir = join(root, 'target/site-shots');
const siteShots = join(root, 'docs/screenshots');
const raw = existsSync(rawDir) ? readdirSync(rawDir).filter((f) => f.endsWith('.png')) : [];
if (!raw.length) {
  console.log('site-render: no raw shots in target/site-shots, keeping the committed WebPs');
} else {
  if (spawnSync('cwebp', ['-version']).error) {
    console.error('site-render: cwebp is needed for the screenshots (brew install webp)');
    process.exit(1);
  }
  mkdirSync(siteShots, { recursive: true });
  for (const f of readdirSync(siteShots)) if (f.endsWith('.webp')) rmSync(join(siteShots, f));
  for (const f of raw) {
    const src = join(rawDir, f);
    /** @type {[string[], string][]} */
    const variants = [[[], '.webp'], [['-resize', '1280', '800'], '.thumb.webp']];
    for (const [args, ext] of variants) {
      const r = spawnSync('cwebp', ['-quiet', '-q', '82', ...args, src, '-o', join(siteShots, f.replace(/\.png$/, ext))]);
      if (r.status !== 0) { console.error(`site-render: cwebp failed on ${f}`); process.exit(1); }
    }
  }
  console.log('wrote docs/screenshots/*.webp (2560x1600) and *.thumb.webp (1280x800)');
}

// Every .png the generated pages link must now exist, or the deploy ships a dead image.
const html = ['index.html', '404.html'].map((f) => readFileSync(join(root, 'target/site', f), 'utf8')).join('');
const missing = [...html.matchAll(/assets\/([\w-]+\.png)/g)].map((m) => m[1]).filter((f) => !shots.some((s) => s.file === f));
if (missing.length) { console.error(`site-render: linked but not rendered: ${missing.join(', ')}`); process.exit(1); }
