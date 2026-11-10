// Fit: a region that scrolls down never scrolls sideways, and a page head shows every control
// whole. Walks every page kind of layout-shift, a Markdown file with an outline, a source file
// with symbols and the built site's docs pages, at 1440, 900 and 360 px.
// Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
import { createServer } from 'node:http';
import { readFileSync, existsSync, statSync, readdirSync, writeFileSync } from 'node:fs';
import { sh } from '../../lib/browser-lib.mjs';
import { join, extname, resolve } from 'node:path';

const WIDTHS = [1440, 900, 360];
const TYPES = {
  '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.mjs': 'text/javascript',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.woff2': 'font/woff2',
  '.json': 'application/json',
};

/** Serves `dir` on a free port; resolves to its base URL and a close function. */
function serveDir(dir) {
  const s = createServer((q, res) => {
    let f = join(dir, decodeURIComponent(new URL(q.url, 'http://x').pathname));
    if (existsSync(f) && statSync(f).isDirectory()) f = join(f, 'index.html');
    if (!existsSync(f)) { res.writeHead(404); res.end(); return; }
    res.writeHead(200, { 'content-type': TYPES[extname(f)] || 'application/octet-stream' });
    res.end(readFileSync(f));
  });
  return new Promise((ok) => s.listen(0, '127.0.0.1', () => {
    const { port } = /** @type {import('node:net').AddressInfo} */ (s.address());
    ok({ base: `http://127.0.0.1:${port}`, close: () => s.close() });
  }));
}

/** Regions meant to scroll down that scroll sideways too, in the page. */
function sideways() {
  const bad = [];
  for (const e of document.querySelectorAll('*')) {
    if (!e.getClientRects().length) continue;
    const cs = getComputedStyle(e);
    const y = /auto|scroll/.test(cs.overflowY), x = /auto|scroll/.test(cs.overflowX);
    // A region meant to scroll down: the outline rail and menu, or any box that scrolls on y with
    // a capped height. A wide table sets overflow-x alone, which computes y to auto as well, but
    // has no cap, so it is left to scroll sideways.
    // A .scroll wrapper is the sideways kind by name, capped or not.
    const down = e.matches('.outline.rail, .outline-pop') ||
      (y && cs.maxHeight !== 'none' && !e.matches('.scroll'));
    if (!down || !x) continue;
    if (e.scrollWidth > e.clientWidth + 1) {
      const cls = e.className && typeof e.className === 'string'
        ? '.' + e.className.trim().split(/\s+/).join('.') : '';
      const id = e.tagName.toLowerCase() + cls;
      bad.push(`${id} scrolls sideways (${e.scrollWidth} > ${e.clientWidth})`);
    }
  }
  return bad;
}

/** Children of a head row, and any filter field (the outline's too), that leave the viewport or
 *  are cut by an ancestor. */
function headFits() {
  const bad = [];
  const vw = document.documentElement.clientWidth;
  const boxes = [...document.querySelectorAll('.head, .ph-row, .ph>.fbar')]
    .filter((h) => h.getClientRects().length)
    .flatMap((h) => [...h.children]);
  boxes.push(...document.querySelectorAll(
    'input:not([type=hidden],[type=checkbox],[type=radio]), select'));
  {
    for (const el of new Set(boxes)) {
      const c = /** @type {HTMLInputElement} */ (el);
      if (!c.getClientRects().length) continue;
      const b = c.getBoundingClientRect();
      if (!b.width && !b.height) continue;
      const first = c.className ? '.' + String(c.className).trim().split(/\s+/)[0] : '';
      const name = c.tagName.toLowerCase() + first;
      // A placeholder wider than the field's text box is cut too.
      if (c.tagName === 'INPUT' && c.placeholder && !c.value) {
        const cs = getComputedStyle(c);
        const g = document.createElement('canvas').getContext('2d');
        g.font = cs.font;
        const room = c.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
        const need = g.measureText(c.placeholder).width;
        if (need > room + 1) {
          bad.push(`${name} cuts its placeholder "${c.placeholder}" ` +
            `(${need.toFixed(0)} > ${room.toFixed(0)})`);
        }
      }
      if (b.left < -0.5 || b.right > vw + 0.5) {
        bad.push(`${name} leaves the viewport ` +
          `(${b.left.toFixed(1)}..${b.right.toFixed(1)} of ${vw})`);
      }
      for (let a = c.parentElement; a && a !== document.body; a = a.parentElement) {
        const cs = getComputedStyle(a);
        const ab = a.getBoundingClientRect();
        const clipX = cs.overflowX !== 'visible', clipY = cs.overflowY !== 'visible';
        if ((clipX && (b.left < ab.left - 0.5 || b.right > ab.right + 0.5)) ||
            (clipY && (b.top < ab.top - 0.5 || b.bottom > ab.bottom + 0.5))) {
          const cls = String(a.className).split(' ')[0];
          bad.push(`${name} is cut by ${a.tagName.toLowerCase()}.${cls}`);
          break;
        }
      }
    }
  }
  return bad;
}

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context, dir } = env;
  const repoPages = {
    overview: '/', tree: '/tree/HEAD/src', blob: '/blob/HEAD/a.txt', docs: '/blob/HEAD/README.md',
    outline: '/blob/HEAD/fit-outline.md', symbols: '/blob/HEAD/fit-symbols.rs', log: '/log/HEAD',
    graph: '/log/HEAD?view=graph', commit: '/commit/HEAD', blame: '/blame/HEAD/a.txt',
    history: '/history/a.txt', compare: '/compare', search: '/search?q=one', refs: '/refs',
    insights: '/stats', hotspots: '/stats/hotspots',
  };
  async function walk(label, pages, fn) {
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      for (const width of WIDTHS) {
        const p = await ctx.newPage();
        await p.setViewportSize({ width, height: 900 });
        for (const [kind, url] of Object.entries(pages)) {
          await p.goto(url, { waitUntil: 'load' });
          // The outline menu shows its list only when open: open it to measure it.
          await p.evaluate(() => {
            for (const d of document.querySelectorAll('details.outline.menu')) {
              if (d.getClientRects().length) /** @type {HTMLDetailsElement} */ (d).open = true;
            }
          });
          const bad = await p.evaluate(fn);
          const where = `${label} ${name} ${width} ${kind}`;
          if (bad.length) r.fail(`${where}: ${bad.join('; ')}`); else r.ok(where);
        }
        await p.close();
      }
      await b.close();
    }
  }
  /** A Markdown file with headings and a source file with symbols, so both rails are drawn. */
  let made = false;
  function fixtures() {
    if (made || existsSync(join(dir, 'fit-outline.md'))) return;
    made = true;
    const heads = ['Install', 'Quickstart', 'Upgrade and troubleshooting', 'Commands'];
    const md = ['# Fit', ...heads.map((h) => `## ${h}\n\ntext`)];
    writeFileSync(join(dir, 'fit-outline.md'), md.join('\n\n') + '\n');
    const fn = (i) => `pub fn f${i}() -> u32 {\n    0\n}\n`;
    const syms = Array.from({ length: 12 }, (_, i) => fn(i)).join('\n');
    writeFileSync(join(dir, 'fit-symbols.rs'), syms);
    sh(dir, 'git', 'add', '-A');
    sh(dir, 'git', 'commit', '-q', '-m', 'docs: fit fixtures');
  }
  async function all(label, fn) {
    fixtures();
    await new Promise((ok) => setTimeout(ok, 600));
    const pages = Object.fromEntries(Object.entries(repoPages).map(([k, v]) => [k, srv.base + v]));
    const site = resolve(new URL('..', import.meta.url).pathname, 'target/site');
    const s = existsSync(join(site, 'docs')) ? await serveDir(site) : null;
    if (s) for (const d of readdirSync(join(site, 'docs'))) {
      if (existsSync(join(site, 'docs', d, 'index.html'))) {
        pages[`site/docs/${d}`] = `${s.base}/docs/${d}/`;
      }
    }
    try { await walk(label, pages, fn); } finally { s?.close(); }
  }
  return {
    noSidewaysScroll: () => all('no-sideways-scroll', sideways),
    headFits: () => all('head-fits', headFits),
  };
}
