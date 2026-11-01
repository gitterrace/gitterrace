// Renders the two raster images the site needs from the brand SVGs, at deploy time.
// They are generated, not committed (.gitignore), because the SVGs are the single source:
//   site/assets/apple-touch-icon.png  180x180, the mark on the page background (iOS home screen)
//   site/assets/og.png                1200x630, the light README header (link previews)
// Chromium draws the SVGs, since simpler renderers drop the stroked path of the mark.
//
//   node scripts/site-render.mjs
import { readFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { root, loadPlaywright, launch } from './browser-lib.mjs';

const brand = (f) => 'data:image/svg+xml,' + encodeURIComponent(readFileSync(join(root, 'docs/brand', f), 'utf8'));
// The light page background, from the shared tokens, so the PNG files match the site.
const bg = /--n-1:(#[0-9a-f]{6})/.exec(readFileSync(join(root, 'crates/gitterrace-web/static/tokens.css'), 'utf8'))[1];
const out = join(root, 'site/assets');
mkdirSync(out, { recursive: true });

const shots = [
  { file: 'apple-touch-icon.png', w: 180, h: 180, img: brand('mark-light.svg'), size: 'width:140px;height:140px' },
  { file: 'og.png', w: 1200, h: 630, img: brand('readme-header-light.svg'), size: 'width:1000px' },
];
const pw = await loadPlaywright();
const b = await launch(pw, 'chromium');
for (const s of shots) {
  const p = await b.newPage({ viewport: { width: s.w, height: s.h }, colorScheme: 'light' });
  await p.setContent(`<body style="margin:0;height:100vh;display:grid;place-items:center;background:${bg}"><img src="${s.img}" style="${s.size}"></body>`);
  await p.waitForFunction(() => document.images[0].complete);
  await p.screenshot({ path: join(out, s.file) });
  console.log(`wrote site/assets/${s.file} (${s.w}x${s.h})`);
}
await b.close();

// Every .png the generated pages link must now exist, or the deploy ships a dead image.
const html = ['index.html', '404.html'].map((f) => readFileSync(join(root, 'site', f), 'utf8')).join('');
const missing = [...html.matchAll(/assets\/([\w-]+\.png)/g)].map((m) => m[1]).filter((f) => !shots.some((s) => s.file === f));
if (missing.length) { console.error(`site-render: linked but not rendered: ${missing.join(', ')}`); process.exit(1); }
