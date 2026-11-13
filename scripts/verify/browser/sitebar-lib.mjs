// Checks of the generated site's chrome: the bar on every page and the docs outline rail.
// Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join, relative } from 'node:path';
import { createServer } from 'node:http';
import { root, launch } from '../../lib/browser-lib.mjs';

const siteDir = join(root, 'target/site');
const types = { html: 'text/html', css: 'text/css', svg: 'image/svg+xml', webp: 'image/webp',
  png: 'image/png', woff2: 'font/woff2' };

/** Serves target/site with its _headers CSP, as the host does. @returns {Promise<any>} */
export async function serve() {
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
export function pages() {
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
  const { pw, r, browsers, srv } = env;

  // The bar on every page, 404 included, at 390, 1100 and 1440 px, in light and dark: one
  // header.top
  // first in the body after the skip link, 56 px tall plus its hairline, its background and text
  // following the scheme (dark bar and light text in dark, light bar and dark text in light),
  // Docs (the link, or its menu on a narrow docs page), the source link and the theme button
  // inside it and in the viewport, no sideways scroll, and main centered on a page without a
  // rail beside it. A script error fails.
  async function siteBar() {
    const { server, url } = await serve();
    const all = pages();
    try {
      for (const name of browsers) {
        const b = await launch(pw, name);
        for (const scheme of ['light', 'dark']) {
          for (const width of [390, 1100, 1440]) {
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
                // rgb() in 0-255, or color(srgb ...) in 0-1 for a mixed, see-through bar.
                const lum = (c) => {
                  const k = c.startsWith('color(') ? 255 : 1;
                  const [r, g, b] = (c.match(/[\d.]+/g) || []).map((v) => Number(v) * k);
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
                  // The hairline under the bar, as the app's header draws it: shown at the top
                  // of the page, not only once it has scrolled.
                  hair: (() => {
                    const s = getComputedStyle(top);
                    const c = s.borderBottomColor;
                    const a = (c.match(/[\d.]+/g) || [])[3];
                    return s.borderBottomStyle !== 'none' && parseFloat(s.borderBottomWidth) >= 1
                      && c !== 'transparent' && (a === undefined || Number(a) > 0.5);
                  })(),
                  bg: lum(getComputedStyle(top).backgroundColor),
                  fg: lum(getComputedStyle(top.querySelector('.brand')).color),
                  docs: inside('a[href="/docs/"]') || inside('.docsmenu>summary'),
                  gh: inside('.gh'), theme: inside('#theme'),
                  scroll: document.documentElement.scrollWidth - innerWidth,
                  // A page with no rail beside main (the landing, a page under 1000 px) holds
                  // main in the middle: equal room left and right, within a pixel.
                  offCenter: (() => {
                    const main = document.querySelector('.frame>main');
                    const side = [...document.querySelectorAll('.frame>.docnav, .frame>.rail')]
                      .some((e) => e.getClientRects().length > 0);
                    if (!main || side) return 0;
                    const q = main.getBoundingClientRect();
                    return Math.round(Math.abs(q.left - (innerWidth - q.right)));
                  })(),
                  // The space between each pair of shown items after the brand: one value, so
                  // no item sits closer to its neighbor than another, chevron or not.
                  gaps: (() => {
                    const shown = [...top.querySelectorAll('.bar > *:not(.brand)')]
                      .map((e) => (e.matches('details') ? e.querySelector('summary') : e))
                      .filter((e) => e && e.getClientRects().length
                        && getComputedStyle(e).display !== 'none');
                    const xs = shown.map((e) => e.getBoundingClientRect());
                    return xs.slice(1).map((x, i) => Math.round(x.left - xs[i].right));
                  })(),
                };
              });
              const why = [];
              if (m.count !== 1) why.push(`${m.count} bars`);
              else {
                if (!m.afterSkip) why.push('not right after the skip link');
                if (Math.abs(m.height - 57) > 1) why.push(`${m.height} px tall`);
                if (!m.hair) why.push('no hairline under the bar at the top of the page');
                const dark = scheme === 'dark';
                if (dark ? m.bg > 0.1 : m.bg < 0.5) {
                  why.push(`background luminance ${m.bg.toFixed(2)}, not ${scheme}`);
                }
                if (dark ? m.fg < 0.5 : m.fg > 0.3) {
                  why.push(`text luminance ${m.fg.toFixed(2)}, not ${dark ? 'light' : 'dark'}`);
                }
                if (!m.docs || !m.gh || !m.theme) why.push(`links in view: ${JSON.stringify(m)}`);
                if (m.scroll > 0) why.push(`page scrolls sideways by ${m.scroll} px`);
                if (m.offCenter > 1) why.push(`main is ${m.offCenter} px off center`);
                if (new Set(m.gaps).size > 1) why.push(`uneven gaps between bar items ${m.gaps}`);
              }
              if (why.length) bad.push(`${path}: ${why.join(', ')}`);
            }
            const where = `site-bar ${name} ${scheme} ${width}`;
            if (errors.length) bad.push(`errors: ${errors.join(' | ')}`);
            if (bad.length) r.fail(`${where}: ${bad.join('; ')}`);
            else r.ok(`${where}: one scheme-following bar on all ${all.length} pages`
              + ', links in view');
            await ctx.close();
          }
        }
        await b.close();
      }
    } finally { server.close(); }
  }

  // The outline rail of a docs page: from 1000 px a page with three or more h2 shows it beside
  // the text, labelled "On this page" in 11 px mono with a hairline after it, each item on a
  // 2 px rail; scrolling to a heading marks its item (and only it) with the accent. The docs
  // list stands beside the text from 1280 px and folds into the bar's Docs menu below, so
  // 1100 px has the menu and the rail. At 390 px and on a page with fewer than three h2 the
  // rail is not shown; below 1280 px the Docs menu
  // opens and lists every docs page, and on a page with a rail the "On this page" menu opens,
  // lists the rail's headings and closes after a pick, which lands on that heading.
  async function siteRail() {
    const { server, url } = await serve();
    const all = pages().filter((p) => p.startsWith('/docs/') && p !== '/docs/');
    try {
      for (const name of browsers) {
        const b = await launch(pw, name);
        for (const width of [390, 1100, 1440]) {
          const ctx = await b.newContext({ viewport: { width, height: 900 } });
          const p = await ctx.newPage();
          const bad = [];
          let railed = 0;
          for (const path of all) {
            await p.goto(url + path, { waitUntil: 'load' });
            const m = await p.evaluate(() => {
              const main = document.querySelector('main');
              const rail = document.querySelector('nav.outline.rail');
              const h2 = main.querySelectorAll('h2').length;
              if (!rail) return { h2, shown: false };
              const label = rail.querySelector('h2');
              const ls = getComputedStyle(label), after = getComputedStyle(label, '::after');
              const a = rail.querySelector('a');
              const rb = rail.getBoundingClientRect(), mb = main.getBoundingClientRect();
              return {
                h2, shown: rail.getClientRects().length > 0 && rb.width > 0,
                label: label.textContent, size: ls.fontSize, mono: /mono/i.test(ls.fontFamily),
                hair: after.borderTopWidth, rail: getComputedStyle(a).borderLeftWidth,
                beside: rb.left >= mb.right - 1,
              };
            });
            const list = await p.$eval('.frame>.docnav', (n) => n.getClientRects().length > 0);
            if (list !== width >= 1280) bad.push(`${path}: docs list shown ${list}`);
            if (width < 1280) {
              const why = await narrowMenus(p, all.length, width >= 1000);
              if (why.length) bad.push(`${path}: ${why.join(', ')}`);
            }
            const want = width >= 1000 && m.h2 >= 3;
            if (m.shown !== want) {
              bad.push(`${path}: ${m.h2} h2, rail shown ${m.shown}`);
              continue;
            }
            if (!want) continue;
            railed++;
            const why = [];
            if (m.label !== 'On this page') why.push(`label ${m.label}`);
            if (m.size !== '11px' || !m.mono) why.push(`label ${m.size} mono ${m.mono}`);
            if (m.hair !== '1px') why.push(`hairline ${m.hair}`);
            if (m.rail !== '2px') why.push(`rail ${m.rail}`);
            if (!m.beside) why.push('rail not beside the text');
            // Scroll a listed heading to the top: the one item marked is the last listed heading
            // above 30% of the window, the line the app's outline reads by.
            const ids = await p.$$eval('nav.outline.rail a',
              (as) => as.map((a) => a.getAttribute('href').slice(1)));
            for (const id of [ids[1], ids[ids.length - 1]]) {
              await p.evaluate((i) => { document.getElementById(i).scrollIntoView(); }, id);
              await p.waitForTimeout(250);
              const on = await p.evaluate(() => {
                const a = [...document.querySelectorAll('nav.outline.rail a.on')];
                // The accent as the page resolves it, from a probe painted with it.
                const probe = document.body.appendChild(document.createElement('i'));
                probe.style.color = 'var(--accent)';
                const accent = getComputedStyle(probe).color;
                probe.remove();
                let want = '';
                for (const l of document.querySelectorAll('nav.outline.rail a')) {
                  const id = l.getAttribute('href').slice(1);
                  const top = document.getElementById(id).getBoundingClientRect().top;
                  if (top <= innerHeight * 0.3) want = id;
                }
                return {
                  want, ids: a.map((x) => x.getAttribute('href').slice(1)),
                  color: a[0] && getComputedStyle(a[0]).borderLeftColor, accent,
                };
              });
              const ok = on.ids.length === 1 && on.ids[0] === on.want;
              if (!ok) why.push(`at #${id} marked ${JSON.stringify(on.ids)}`);
              else if (on.color !== on.accent) why.push(`marked ${on.color}, not ${on.accent}`);
            }
            if (why.length) bad.push(`${path}: ${why.join(', ')}`);
          }
          const where = `site-rail ${name} ${width}`;
          if (width >= 1000 && !railed) bad.push('no page shows a rail');
          if (bad.length) r.fail(`${where}: ${bad.join('; ')}`);
          else r.ok(`${where}: rail on ${railed} pages with 3+ h2, none elsewhere, follows scroll`);
          await ctx.close();
        }
        await b.close();
      }
    } finally { server.close(); }
  }

  // Every table on every page, at 390 and 1440 px: no text in a cell wraps inside a token (a
  // letter or digit on both sides of the line break, so `rescan_secs` or 1440 split over two
  // lines), and no cell with text is crushed narrower than two em of its font. A wide
  // table scrolls inside itself, so neither has a reason to happen.
  async function siteTables() {
    const { server, url } = await serve();
    const all = pages();
    try {
      for (const name of browsers) {
        const b = await launch(pw, name);
        for (const width of [390, 1440]) {
          const ctx = await b.newContext({ viewport: { width, height: 900 } });
          const p = await ctx.newPage();
          let tables = 0;
          const bad = [];
          for (const path of all) {
            await p.goto(url + path, { waitUntil: 'load' });
            await p.evaluate(() => document.fonts.ready);
            const m = await p.evaluate(tableProbe);
            tables += m.tables;
            for (const f of m.found) bad.push(`${path} ${f}`);
          }
          const where = `site-tables ${name} ${width}`;
          if (bad.length) r.fail(`${where}: ${bad.slice(0, 6).join('; ')}`);
          else r.ok(`${where}: ${tables} tables on ${all.length} pages, none broken or crushed`);
          await ctx.close();
        }
        await b.close();
      }
    } finally {
      server.close();
    }
    // Self-test: a cell of "a, b, c" on one line passes, and the same words squeezed to one
    // word per line are crushed.
    const fb = await launch(pw, browsers[0]);
    const fp = await (await fb.newContext({ viewport: { width: 800, height: 600 } })).newPage();
    const cell = (w) => `<main><table style="display:table"><tr><td style="width:${w};` +
      'padding:0;font:16px monospace">alpha, beta, gamma</td></tr></table></main>';
    for (const [w, crushed] of [['40ch', false], ['6ch', true]]) {
      await fp.setContent(cell(w));
      const m = await fp.evaluate(tableProbe);
      const got = m.found.some((f) => f.includes('crushed'));
      const what = `site-tables fixture: a cell of "alpha, beta, gamma" at ${w}`;
      if (got === crushed) r.ok(`${what} is ${crushed ? 'crushed' : 'not crushed'}`);
      else r.fail(`${what} gave ${m.found.join('; ') || 'no findings'}`);
    }
    await fb.close();
  }

  // The real theme button, on the app and on the site: from the light theme to the dark one the
  // header's computed background changes along with the body's, the header dark in dark and
  // light in light, its text and brand the other way round.
  async function headerTheme() {
    const { server, url } = await serve();
    const lum = (c) => {
      const [r, g, b] = (c.match(/[\d.]+/g) || []).map(Number);
      const f = (v) => (v /= 255) <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
    };
    const read = () => {
      const top = document.querySelector('header.top');
      const brand = top.querySelector('.brand');
      return {
        head: getComputedStyle(top).backgroundColor,
        body: getComputedStyle(document.body).backgroundColor,
        text: getComputedStyle(brand).color,
        theme: document.documentElement.dataset.theme || 'system',
      };
    };
    try {
      for (const name of browsers) {
        const b = await launch(pw, name);
        for (const [what, base] of [['site', url + '/'], ['app', srv.base + '/']]) {
          const viewport = { width: 1200, height: 800 };
          const ctx = await b.newContext({ colorScheme: 'dark', viewport });
          const p = await ctx.newPage();
          await p.goto(base, { waitUntil: 'load' });
          // Click the real toggle until the page is on light, then once more for dark.
          const seen = {};
          for (let i = 0; i < 3 && !seen.dark; i++) {
            const m = await p.evaluate(read);
            if (m.theme === 'light' || m.theme === 'dark') seen[m.theme] = m;
            if (!seen.dark) await p.click('#theme');
          }
          const l = seen.light, d = seen.dark;
          const why = [];
          if (!l || !d) why.push('the toggle did not reach both themes');
          else {
            if (l.head === d.head) why.push(`header stays ${d.head}`);
            if (l.body === d.body) why.push('body does not change');
            if (lum(l.head) < 0.5 || lum(l.text) > 0.3) {
              why.push(`light header ${l.head}, text ${l.text}`);
            }
            if (lum(d.head) > 0.1 || lum(d.text) < 0.5) {
              why.push(`dark header ${d.head}, text ${d.text}`);
            }
          }
          const where = `header-theme ${name} ${what}`;
          if (why.length) r.fail(`${where}: ${why.join('; ')}`);
          else r.ok(`${where}: header follows the theme toggle with the body`);
          await ctx.close();
        }
        await b.close();
      }
    } finally { server.close(); }
  }

  return { siteBar, siteRail, siteTables, headerTheme };
}

// Runs in the page: the tables in main and what is wrong in their cells. Each character's line
// is read from its box. A token is broken where two word characters (letters, digits, `_`)
// with nothing between them sit on different lines. A cell is crushed where its text of three
// words or more wraps to one word per line. A line is a change of a character's top, so
// punctuation only separates words and never starts a line.
function tableProbe() {
  const found = [];
  const word = /[\p{L}\p{N}_]/u;
  for (const cell of document.querySelectorAll('main th, main td')) {
    const text = cell.textContent.trim().replace(/\s+/g, ' ');
    if (!text) continue;
    const label = `"${text.slice(0, 32)}"`;
    const walker = document.createTreeWalker(cell, NodeFilter.SHOW_TEXT);
    const range = document.createRange();
    const lines = [];
    let prev = null;
    let lastTop = null;
    let broken = '';
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const s = n.textContent;
      for (let i = 0; i < s.length; i++) {
        if (!word.test(s[i])) {
          if (prev) prev.gap = true;
          continue;
        }
        range.setStart(n, i);
        range.setEnd(n, i + 1);
        const rect = [...range.getClientRects()].find((q) => q.width > 0);
        if (!rect) continue;
        const next = lastTop !== null && rect.top > lastTop + rect.height / 2;
        if (next && !prev.gap && !broken) broken = `${prev.ch}${s[i]}`;
        if (next || !lines.length) lines.push(0);
        if (next || !prev || prev.gap) lines[lines.length - 1]++;
        lastTop = rect.top;
        prev = { ch: s[i], gap: false };
      }
    }
    if (broken) found.push(`${label} broken inside a token at "${broken}"`);
    const words = text.split(' ').length;
    if (words >= 3 && lines.length >= 3 && lines.every((w) => w <= 1)) {
      found.push(`${label} crushed to one word a line`);
    }
  }
  // No column is much wider than its widest content: a yes/no or send/action column sits as
  // tight as its words, and the room goes to the text columns.
  for (const table of document.querySelectorAll('main table')) {
    if (getComputedStyle(table).display === 'block' && innerWidth < 700) continue;
    const head = table.querySelector('tr');
    if (!head) continue;
    [...head.children].forEach((th, col) => {
      const cells = [...table.querySelectorAll('tr')].map((tr) => tr.children[col]).filter(Boolean);
      const range = document.createRange();
      let ink = 0;
      for (const c of cells) {
        // A cell that wraps fills its column: its column is as wide as it needs.
        range.selectNodeContents(c);
        const rects = [...range.getClientRects()].filter((q) => q.width > 0);
        const tops = new Set(rects.map((q) => Math.round(q.top)));
        ink = tops.size > 1 ? Infinity : Math.max(ink, range.getBoundingClientRect().width);
        if (ink === Infinity) break;
      }
      const pad = parseFloat(getComputedStyle(th).paddingLeft)
        + parseFloat(getComputedStyle(th).paddingRight);
      const extra = th.getBoundingClientRect().width - pad - ink;
      if (extra > 24) {
        const name = th.textContent.trim();
        found.push(`column "${name}" is ${Math.round(extra)} px wider than its content`);
      }
    });
  }
  return { tables: document.querySelectorAll('main table').length, found };
}

/** The folded menus of a docs page at a narrow width: what is wrong, empty when all works.
 * @param {any} p the page @param {number} docs how many docs pages there are */
async function narrowMenus(p, docs, railed) {
  const why = [];
  const menus = [await p.$('header.top details.docsmenu'), await p.$('.menus > details')];
  if (!menus[0]) return ['no Docs menu'];
  await menus[0].evaluate((d) => d.querySelector('summary').click());
  const listed = await menus[0].evaluate((d) => d.open
    && d.querySelector('.pop').getBoundingClientRect().right <= innerWidth + 0.5
    ? d.querySelectorAll('a').length : -1);
  if (listed !== docs) why.push(`Docs menu lists ${listed} of ${docs}`);
  await menus[0].evaluate((d) => { d.open = false; });
  const rail = await p.$$eval('nav.outline.rail a', (as) => as.map((a) => a.getAttribute('href')));
  // Beside the rail the "On this page" menu has no reason to show.
  if (railed) {
    const shown = await p.$$eval('.menus', (m) => m.some((x) => x.getClientRects().length > 0));
    return shown ? [...why, 'On this page menu shows beside the rail'] : why;
  }
  if (!rail.length) return why;
  const outline = menus[1];
  if (!outline) return [...why, 'no On this page menu'];
  await outline.evaluate((d) => d.querySelector('summary').click());
  const items = await outline.evaluate((d) => [...d.querySelectorAll('a')]
    .map((a) => a.getAttribute('href')));
  if (JSON.stringify(items) !== JSON.stringify(rail)) why.push('On this page menu is not the rail');
  const pick = items[items.length - 1];
  await outline.evaluate((d, h) => d.querySelector(`a[href="${h}"]`).click(), pick);
  await p.waitForTimeout(250);
  const after = await outline.evaluate((d, h) => ({
    open: d.open,
    top: document.getElementById(h.slice(1)).getBoundingClientRect().top,
    // At the page end a heading cannot reach the top, and that is fine.
    end: Math.ceil(scrollY + innerHeight) >= document.documentElement.scrollHeight,
  }), pick);
  if (after.open) why.push('On this page menu stays open after a pick');
  if (after.top > 200 && !after.end) why.push(`picked heading at ${Math.round(after.top)} px`);
  return why;
}
