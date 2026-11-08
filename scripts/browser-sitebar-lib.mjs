// Checks of the generated site's chrome: the bar on every page.
// Imported by scripts/browser-check.mjs, not run on its own.
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join, relative } from 'node:path';
import { createServer } from 'node:http';
import { root, launch } from './browser-lib.mjs';

const siteDir = join(root, 'site');
const types = { html: 'text/html', css: 'text/css', svg: 'image/svg+xml', webp: 'image/webp',
  png: 'image/png', woff2: 'font/woff2' };

/** Serves site/ with its _headers CSP, as the host does. @returns {Promise<any>} */
async function serve() {
  const head = readFileSync(join(siteDir, '_headers'), 'utf8');
  const csp = /Content-Security-Policy:\s*(.+)/.exec(head)[1].trim();
  const server = createServer((q, s) => {
    const path = decodeURIComponent(q.url.split(/[?#]/)[0]).replace(/\/$/, '/index.html');
    const p = join(siteDir, path);
    if (!p.startsWith(siteDir) || !existsSync(p)) { s.writeHead(404); s.end(); return; }
    const type = types[p.split('.').pop()] || 'application/octet-stream';
    s.writeHead(200, { 'content-type': type, 'content-security-policy': csp });
    s.end(readFileSync(p));
  });
  await new Promise((ok) => server.listen(0, '127.0.0.1', () => ok(undefined)));
  return { server, url: `http://127.0.0.1:${(/** @type {any} */ (server.address())).port}` };
}

/** Every page of the site as its URL path. */
function pages() {
  const out = [];
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const f = join(d, e.name);
      if (e.isDirectory()) walk(f);
      else if (e.name.endsWith('.html')) {
        out.push('/' + relative(siteDir, f).replace(/index\.html$/, ''));
      }
    }
  };
  walk(siteDir);
  return out.sort();
}

/** @param {any} env what browser-check.mjs set up (pw, r, browsers, ...) */
export function checks(env) {
  const { pw, r, browsers } = env;

  // The bar on every page, 404 included, at 390 and 1440 px, in light and dark: one header.top
  // first in the body after the skip link, 56 px tall plus its hairline, its background dark
  // (luminance under 0.1) in both schemes, its text light, Docs, the source link and the theme
  // button inside it and in the viewport, no sideways scroll. A script error fails.
  async function siteBar() {
    const { server, url } = await serve();
    const all = pages();
    try {
      for (const name of browsers) {
        const b = await launch(pw, name);
        for (const scheme of ['light', 'dark']) {
          for (const width of [390, 1440]) {
            const viewport = { width, height: 900 };
            const ctx = await b.newContext({ colorScheme: scheme, viewport });
            const p = await ctx.newPage();
            const errors = [];
            p.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
            p.on('pageerror', (e) => errors.push(String(e)));
            const bad = [];
            for (const path of all) {
              await p.goto(url + path, { waitUntil: 'load' });
              const m = await p.evaluate(() => {
                const lum = (c) => {
                  const [r, g, b] = (c.match(/[\d.]+/g) || []).map(Number);
                  const f = (v) => {
                    v /= 255;
                    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
                  };
                  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
                };
                const bars = document.querySelectorAll('header.top');
                const top = bars[0];
                if (!top) return { count: 0 };
                const box = top.getBoundingClientRect();
                const inside = (sel) => {
                  const e = top.querySelector(sel);
                  if (!e) return false;
                  const q = e.getBoundingClientRect();
                  return q.width > 0 && q.left >= 0 && q.right <= innerWidth + 0.5;
                };
                return {
                  count: bars.length,
                  afterSkip: top.previousElementSibling?.classList.contains('skip') ?? false,
                  height: Math.round(box.height),
                  bg: lum(getComputedStyle(top).backgroundColor),
                  fg: lum(getComputedStyle(top.querySelector('.brand')).color),
                  docs: inside('a[href="/docs/"]'), gh: inside('.gh'), theme: inside('#theme'),
                  scroll: document.documentElement.scrollWidth - innerWidth,
                };
              });
              const why = [];
              if (m.count !== 1) why.push(`${m.count} bars`);
              else {
                if (!m.afterSkip) why.push('not right after the skip link');
                if (Math.abs(m.height - 57) > 1) why.push(`${m.height} px tall`);
                if (m.bg > 0.1) why.push(`background luminance ${m.bg.toFixed(2)}, not dark`);
                if (m.fg < 0.5) why.push(`text luminance ${m.fg.toFixed(2)}, not light`);
                if (!m.docs || !m.gh || !m.theme) why.push(`links in view: ${JSON.stringify(m)}`);
                if (m.scroll > 0) why.push(`page scrolls sideways by ${m.scroll} px`);
              }
              if (why.length) bad.push(`${path}: ${why.join(', ')}`);
            }
            const where = `site-bar ${name} ${scheme} ${width}`;
            if (errors.length) bad.push(`errors: ${errors.join(' | ')}`);
            if (bad.length) r.fail(`${where}: ${bad.join('; ')}`);
            else r.ok(`${where}: one dark bar on all ${all.length} pages, links in view`);
            await ctx.close();
          }
        }
        await b.close();
      }
    } finally { server.close(); }
  }

  return { siteBar };
}
