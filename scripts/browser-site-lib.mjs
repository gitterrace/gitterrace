// Checks of marks and the generated site: mark seam, about placement, site theme. Imported by scripts/browser-check.mjs, not run on its own.
import { readFileSync, readdirSync, existsSync, mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL } from 'node:url';
import { root, launch, sh } from './browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (pw, dir, srv, r, bin, cfg, arg, ...) */
export function checks(env) {
  const { pw, dir, srv, r, browsers, tabCounts, marker, context, arg, bin, cfg } = env;
async function markSeam() {
  const marks = readdirSync(join(root, 'docs/brand'))
    .filter((f) => /^mark-(light|dark)\.svg$/.test(f))
    .map((f) => ({ name: f, svg: readFileSync(join(root, 'docs/brand', f), 'utf8') }));
  const sizes = [16, 19, 21, 23, 27, 31, 37, 45, 53, 61, 77, 101, 133];
  for (const browser of browsers) {
    const b = await launch(pw, browser);
    for (const m of marks) {
      const bg = m.name.includes('dark') ? '#1e2023' : '#ffffff';
      const src = 'data:image/svg+xml,' + encodeURIComponent(m.svg);
      for (const dpr of [1, 2]) {
        const p = await b.newPage({ deviceScaleFactor: dpr, viewport: { width: 1900, height: 200 } });
        await p.setContent(`<body style="margin:0;background:${bg}">${sizes.map((s, i) =>
          `<img src="${src}" style="position:absolute;left:${10 + i * 140 + 0.3}px;top:20.4px;width:${s}px;height:${s}px">`).join('')}`);
        await p.waitForFunction(() => [...document.images].every((i) => i.complete));
        const png = (await p.screenshot()).toString('base64');
        const bad = await p.evaluate(async ({ png, sizes, dpr, dark }) => {
          const img = new Image(); img.src = 'data:image/png;base64,' + png; await img.decode();
          const c = document.createElement('canvas'); c.width = img.width; c.height = img.height;
          const g = c.getContext('2d'); g.drawImage(img, 0, 0);
          const d = g.getImageData(0, 0, c.width, c.height).data, W = c.width;
          const lum = (x, y) => { const i = (y * W + x) * 4; return d[i] + d[i + 1] + d[i + 2]; };
          const out = [];
          sizes.forEach((s, i) => {
            const left = (10 + i * 140 + 0.3) * dpr, top = 20.4 * dpr, S = s * dpr, ys = top + (24 / 32) * S;
            let worst = 0;
            for (let y = Math.floor(ys) - 1; y <= Math.ceil(ys) + 1; y++)
              for (let x = Math.ceil(left + (16 / 32) * S); x < Math.floor(left + (22 / 32) * S); x++) {
                const a = lum(x, y - 2 - Math.ceil(dpr)), b2 = lum(x, y + 2 + Math.ceil(dpr)), v = lum(x, y);
                worst = Math.max(worst, dark ? Math.min(a, b2) - v : v - Math.max(a, b2));
              }
            if (worst > 6) out.push(`${s}px:${worst}`);
          });
          return out;
        }, { png, sizes, dpr, dark: bg !== '#ffffff' });
        const what = `${browser} ${m.name} @${dpr}x`;
        if (bad.length) r.fail(`${what}: background shows on the colour change at ${bad.join(' ')}`);
        else r.ok(`${what}: no seam at ${sizes.length} sizes`);
        await p.close();
      }
    }
    await b.close();
  }
}

// The overview's About box: one place per layout, never moving between nearby widths. With two
// columns it is beside the files, with one it is above them, never below.
async function aboutPlace() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    for (const width of [1440, 1200, 1101, 1100, 1000, 900, 861, 860, 760, 600, 375, 320]) {
      await p.setViewportSize({ width, height: 900 });
      await p.goto(srv.base + '/', { waitUntil: 'load' });
      const m = await p.evaluate(() => {
        const a = document.querySelector('.cols>aside'), f = document.querySelector('.cols>article');
        if (!a || !f) return null;
        const ar = a.getBoundingClientRect(), fr = f.getBoundingClientRect();
        return { beside: ar.left >= fr.right - 1, above: ar.bottom <= fr.top + 1 };
      });
      const where = `${name} ${width}`;
      if (!m) r.fail(`about-place ${where}: no overview columns`);
      else if (m.beside || m.above) r.ok(`about-place ${where}: ${m.beside ? 'beside' : 'above'} the files`);
      else r.fail(`about-place ${where}: About is below the files`);
    }
    await b.close();
  }
}

async function siteTheme() {
  const { createServer } = await import('node:http');
  const siteDir = join(root, 'site');
  const csp = /Content-Security-Policy:\s*(.+)/.exec(readFileSync(join(siteDir, '_headers'), 'utf8'))[1].trim();
  const types = { html: 'text/html', css: 'text/css', svg: 'image/svg+xml', webp: 'image/webp', png: 'image/png' };
  const server = createServer((q, s) => {
    const p = join(siteDir, decodeURIComponent(q.url.split(/[?#]/)[0]).replace(/\/$/, '/index.html'));
    if (!p.startsWith(siteDir) || !existsSync(p)) { s.writeHead(404); s.end(); return; }
    s.writeHead(200, { 'content-type': types[p.split('.').pop()] || 'application/octet-stream', 'content-security-policy': csp });
    s.end(readFileSync(p));
  });
  await new Promise((ok) => server.listen(0, '127.0.0.1', () => ok(undefined)));
  const url = `http://127.0.0.1:${(/** @type {any} */ (server.address())).port}/`;
  // What the page shows: data-theme, the visible logo, the visible screenshot images, each by file.
  const state = (p) => p.evaluate(() => {
    const shown = (sel) => [...document.querySelectorAll(sel)].filter((i) => i.offsetParent && getComputedStyle(i).display !== 'none').map((i) => i.getAttribute('src'));
    return { theme: document.documentElement.dataset.theme || 'system', logo: shown('header img'), shots: shown('.shots a:not(.lightbox) img'), label: document.getElementById('theme')?.getAttribute('aria-label') };
  });
  try {
    for (const browser of browsers) {
      const b = await launch(pw, browser);
      for (const scheme of ['light', 'dark']) {
        const where = `${browser} ${scheme} system`;
        const ctx = await b.newContext({ colorScheme: scheme, viewport: { width: 1100, height: 900 } });
        const p = await ctx.newPage();
        const fetched = [], errors = [];
        p.on('request', (q) => fetched.push(q.url()));
        p.on('console', (m) => { if (/Content.Security.Policy|Refused to/i.test(m.text())) errors.push(m.text()); });
        p.on('pageerror', (e) => errors.push(String(e)));
        const want = (t, s, what) => {
          const other = t === 'light' ? 'dark' : 'light';
          const bad = [...s.logo, ...s.shots].filter((x) => !x.includes(`-${t}`));
          if (s.logo.length !== 1 || !s.shots.length || bad.length) r.fail(`site-theme ${where} ${what}: shows ${JSON.stringify(s)} for ${t}`);
          else r.ok(`site-theme ${where} ${what}: logo and ${s.shots.length} shots are ${t}`);
          return other;
        };
        await p.goto(url, { waitUntil: 'load' });
        await p.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
        await p.waitForTimeout(400);
        let s = await state(p);
        if (s.theme !== 'system' || s.label !== 'Theme: system') r.fail(`site-theme ${where}: first load is ${s.theme} (${s.label})`);
        const other = want(scheme, s, 'first load');
        const leaked = fetched.filter((u) => u.includes(`-${other}`) && /screenshots|logo/.test(u));
        if (leaked.length) r.fail(`site-theme ${where}: fetched the ${other} scheme's images: ${leaked.join(' ')}`);
        else r.ok(`site-theme ${where}: nothing of the ${other} scheme fetched`);
        // The cycle: system, light, dark, system.
        for (const t of ['light', 'dark', 'system']) {
          await p.click('#theme');
          s = await state(p);
          if (s.theme !== t || s.label !== `Theme: ${t}`) r.fail(`site-theme ${where}: click gave ${s.theme} (${s.label}), wanted ${t}`);
          else want(t === 'system' ? scheme : t, s, `after click to ${t}`);
        }
        // The choice survives a reload, and is in place before first paint (set in the head).
        await p.click('#theme'); await p.click('#theme');
        const pick = 'dark';
        await p.reload({ waitUntil: 'commit' });
        const early = await p.evaluate(() => new Promise((ok) => {
          const go = () => ok({ theme: document.documentElement.dataset.theme || 'system', body: !!document.body && document.body.children.length > 0 });
          if (document.readyState === 'loading') document.addEventListener('readystatechange', go, { once: true }); else go();
        }));
        await p.waitForLoadState('load');
        s = await state(p);
        if (early.theme !== pick || s.theme !== pick) r.fail(`site-theme ${where}: after reload ${early.theme}/${s.theme}, wanted ${pick}`);
        else r.ok(`site-theme ${where}: ${pick} kept over a reload, set before the body rendered`);
        if (errors.length) r.fail(`site-theme ${where}: ${errors.join(' | ')}`);
        else r.ok(`site-theme ${where}: no CSP violation or script error`);
        // Every theme icon is as large as the source-link mark beside it: same box, and its
        // painted extent (stroke included) within 1.5 px of the mark's on the longer side.
        for (const t of ['system', 'light', 'dark']) {
          const m = await p.evaluate((t) => {
            const r = document.documentElement;
            if (t === 'system') delete r.dataset.theme; else r.dataset.theme = t;
            const ink = (s) => {
              const box = s.getBoundingClientRect(), vb = s.viewBox.baseVal, k = box.width / vb.width;
              let l = 1e9, tp = 1e9, rt = -1e9, bt = -1e9;
              for (const e of s.querySelectorAll('path,circle')) {
                const g = e.getBBox(), h = (getComputedStyle(e).stroke !== 'none' ? parseFloat(getComputedStyle(e).strokeWidth) : 0) / 2;
                l = Math.min(l, (g.x - vb.x) * k - h); tp = Math.min(tp, (g.y - vb.y) * k - h);
                rt = Math.max(rt, (g.x + g.width - vb.x) * k + h); bt = Math.max(bt, (g.y + g.height - vb.y) * k + h);
              }
              return { box: [box.width, box.height], size: Math.max(rt - l, bt - tp) };
            };
            const th = [...document.querySelectorAll('.theme svg')].find((s) => getComputedStyle(s).display !== 'none');
            return { gh: ink(document.querySelector('.gh svg')), theme: ink(th) };
          }, t);
          const same = m.gh.box.join() === m.theme.box.join() && Math.abs(m.gh.size - m.theme.size) <= 1.5;
          if (same) r.ok(`site-theme ${where} ${t} icon: ${m.theme.size.toFixed(1)} px, GitHub ${m.gh.size.toFixed(1)} px`);
          else r.fail(`site-theme ${where} ${t} icon: ${JSON.stringify(m.theme)} vs GitHub ${JSON.stringify(m.gh)}`);
        }
        await ctx.close();
      }
      await b.close();
    }
  } finally { server.close(); }
}
// An export opened from disk (a file:// URL, no server): the forwarding index lands on the
// repository, the page shows its content, the stylesheets load, and a link reaches a
// second page. On every exported page with a form or a switcher (the overview, the log with
// its path filter, compare and search when exported), no visible form submits to a server path
// and no script logs an error or throws: a static copy cannot filter, search or list repos.
async function exportFromDisk() {
  const out = join(mkdtempSync(join(tmpdir(), 'gt-export-')), 'site');
  sh(root, bin, 'export', out, dir);
  // Every exported page under the repository folder whose name says it can hold a form.
  const pages = [];
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const f = join(d, e.name);
      if (e.isDirectory()) walk(f);
      else if (e.name === 'index.html' && /\/(log|compare|search)\//.test(f) && pages.length < 12) pages.push(f);
    }
  };
  walk(out);
  for (const name of browsers) {
    const b = await launch(pw, name);
    const p = await b.newPage();
    const where = `export-from-disk ${name}`;
    const errors = [];
    p.on('console', (m) => { if (m.type() === 'error') errors.push(`${p.url().replace(/^.*\/site\//, '')}: ${m.text()}`); });
    p.on('pageerror', (e) => errors.push(`${p.url().replace(/^.*\/site\//, '')}: ${String(e)}`));
    p.on('requestfailed', (q) => errors.push(`${p.url().replace(/^.*\/site\//, '')}: no ${q.url().replace(/^.*\/site\//, '')}`));
    try {
      await p.goto(pathToFileURL(join(out, 'index.html')).href, { waitUntil: 'load' });
      const bad = [];
      await p.waitForURL(/\/r\/[^/]+\/index\.html$/, { timeout: 3000 }).catch(() => bad.push(`the forwarding page stays at ${p.url()}`));
      const m = await p.evaluate(() => ({
        text: document.body.innerText.length,
        banner: getComputedStyle(document.querySelector('.snapshot') || document.body).backgroundColor,
        sheets: [...document.styleSheets].every((s) => { try { return s.cssRules.length > 0; } catch { return true; } }),
        links: [...document.querySelectorAll('a[href]')].filter((a) => /** @type {HTMLAnchorElement} */ (a).href.startsWith('file:')).length,
      }));
      if (m.text < 100) bad.push(`page shows ${m.text} characters`);
      if (/rgba\(0, 0, 0, 0\)|transparent/.test(m.banner)) bad.push('snapshot style not applied');
      if (!m.links) bad.push('no link stays on disk');
      const forms = async () => p.evaluate(() => [...document.querySelectorAll('form')]
        .filter((f) => f.getClientRects().length > 0 && getComputedStyle(f).visibility !== 'hidden')
        .map((f) => `${f.className || 'form'} -> ${f.getAttribute('action') || '(none)'}`));
      const shown = [...await forms()];
      // The repository switcher reads its list when the page loads: give it the time a
      // failed fetch takes to report.
      await p.waitForTimeout(300);
      for (const f of pages) {
        await p.goto(pathToFileURL(f).href, { waitUntil: 'load' });
        await p.waitForTimeout(300);
        for (const x of await forms()) shown.push(`${f.replace(out, '')}: ${x}`);
      }
      if (!pages.some((f) => /\/log\//.test(f))) bad.push('the export has no log page to check');
      if (shown.length) bad.push(`forms that need the server show: ${shown.join('; ')}`);
      if (errors.length) bad.push(`script errors: ${errors.join('; ')}`);
      const link = await p.$('a[href$="/refs/index.html"], a[href*="/log/"]');
      if (!link) bad.push('no link to a second page');
      else {
        await link.click({ timeout: 3000 }).catch(() => {});
        await p.waitForLoadState('load');
        const n = await p.evaluate(() => document.body.innerText.length);
        if (n < 100 || !p.url().startsWith('file:')) bad.push(`a followed link shows ${n} characters at ${p.url()}`);
      }
      bad.length ? r.fail(`${where}: ${bad.join(', ')}`) : r.ok(`${where}: forwards, styled, a link opens a second page, no server form shows, no script error (${pages.length} form pages)`);
    } catch (e) { r.fail(`${where}: ${String(e.message).split('\n')[0]}`); } finally { await b.close(); }
  }
  rmSync(join(out, '..'), { recursive: true, force: true });
}
  return { markSeam, aboutPlace, siteTheme, exportFromDisk };
}
