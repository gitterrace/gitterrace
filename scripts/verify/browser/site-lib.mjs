// Checks of marks and the generated site: mark seam, about placement, site theme.
// Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
import { readFileSync, readdirSync, mkdtempSync, renameSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL } from 'node:url';
import { eachLaunch, eachBrowser, root, launch, makeRepo, sh, scrollSettled, frames } from '../../lib/browser-lib.mjs';
import { serve } from './sitebar-lib.mjs';

/** @param {any} env what browser-check.mjs set up (pw, dir, srv, r, bin, cfg, arg, ...) */
export function checks(env) {
  const { pw, dir, srv, r, browsers, tabCounts, marker, context, arg, bin, cfg } = env;
async function markSeam() {
  const marks = readdirSync(join(root, 'docs/brand'))
    .filter((f) => /^mark-(light|dark)\.svg$/.test(f))
    .map((f) => ({ name: f, svg: readFileSync(join(root, 'docs/brand', f), 'utf8') }));
  const sizes = [16, 19, 21, 23, 27, 31, 37, 45, 53, 61, 77, 101, 133];
  await eachLaunch(pw, browsers, async (b, browser) => {
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
            // The ink's tread ends at y=23.5 and the accent bar starts at y=25, so the gap is
            // open air. Where it spans two device pixels or more, its middle row must stay
            // within 30% of the background, not blend toward either colour.
            const left = (10 + i * 140 + 0.3) * dpr, top = 20.4 * dpr, S = s * dpr;
            const y0 = Math.ceil(top + (23.5 / 32) * S), y1 = Math.floor(top + (25 / 32) * S);
            if (y1 <= y0) return;
            const bgv = dark ? 97 : 765;
            const inkv = dark ? 3 * 0xe8 : 3 * 0x25;
            let worst = Infinity;
            for (let y = y0; y < y1; y++) {
              let row = 0;
              for (let x = Math.ceil(left + (16 / 32) * S); x < Math.floor(left + (22 / 32) * S); x++)
                row = Math.max(row, Math.abs(lum(x, y) - bgv) - 0.3 * Math.abs(inkv - bgv));
              worst = Math.min(worst, row);
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
  });
}

// The overview's About box: one place per layout, never moving between nearby widths. With two
// columns it is beside the files, with one it is above them, never below.
async function aboutPlace() {
  await eachBrowser(browsers, context, async (ctx, name) => {
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
  });
}

async function siteTheme() {
  const { server, url: root_ } = await serve();
  const url = `${root_}/`;
  // What the page shows: data-theme, the visible logo, the visible screenshot images, each by file.
  const state = (p) => p.evaluate(() => {
    const shown = (sel) => [...document.querySelectorAll(sel)].filter((i) => i.offsetParent && getComputedStyle(i).display !== 'none').map((i) => i.getAttribute('src'));
    const pic = document.querySelector('.hero picture');
    const media = [...(pic?.querySelectorAll('source') || [])]
      .map((s) => `${s.media} ${s.srcset}`);
    const img = pic?.querySelector('img')?.currentSrc || '';
    return { theme: document.documentElement.dataset.theme || 'system', logo: shown('.hero h1 img'), pic: img, media, shots: shown('.shots a:not(.lightbox) img'), label: document.getElementById('theme')?.getAttribute('aria-label') };
  });
  try {
    await eachLaunch(pw, browsers, async (b, browser) => {
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
          const bad = [...s.logo, ...s.shots, s.pic].filter((x) => !x.includes(`-${t}`));
          // The landing picture: a source per scheme, the dark one under the system query.
          const dark = s.media
            .some((m) => m.includes('-dark') && /prefers-color-scheme: ?dark|^all|not all/.test(m));
          if (s.logo.length !== 1 || !s.shots.length || bad.length || !dark) r.fail(`site-theme ${where} ${what}: shows ${JSON.stringify(s)} for ${t}`);
          else r.ok(`site-theme ${where} ${what}: logo, picture and ${s.shots.length} shots`
            + ` are ${t}`);
          return other;
        };
        await p.goto(url, { waitUntil: 'load' });
        await p.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
        await scrollSettled(p);
        let s = await state(p);
        if (s.theme !== 'system' || s.label !== 'Theme: system, click for light') r.fail(`site-theme ${where}: first load is ${s.theme} (${s.label})`);
        const other = want(scheme, s, 'first load');
        const leaked = fetched.filter((u) => u.includes(`-${other}`) && /screenshots|logo/.test(u));
        if (leaked.length) r.fail(`site-theme ${where}: fetched the ${other} scheme's images: ${leaked.join(' ')}`);
        else r.ok(`site-theme ${where}: nothing of the ${other} scheme fetched`);
        // The cycle: system, light, dark, system. The label names the next pick too.
        const next = { system: 'light', light: 'dark', dark: 'system' };
        for (const t of ['light', 'dark', 'system']) {
          await p.click('#theme');
          // A swapped source sets currentSrc only once it has loaded.
          const done = () => document.querySelector('.hero picture')?.querySelector('img')
            ?.complete !== false;
          await p.waitForFunction(done, null, { timeout: 5000 }).catch(() => {});
          await frames(p);
          s = await state(p);
          if (s.theme !== t || s.label !== `Theme: ${t}, click for ${next[t]}`) r.fail(`site-theme ${where}: click gave ${s.theme} (${s.label}), wanted ${t}`);
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
    });
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
    const rel = (u) => u.replace(new RegExp('^.*/site/'), '');
    p.on('console', (m) => {
      if (m.type() === 'error') errors.push(`${rel(p.url())}${': '}${m.text()}`);
    });
    p.on('pageerror', (e) => errors.push(`${rel(p.url())}${': '}${String(e)}`));
    p.on('requestfailed', (q) => errors.push(`${rel(p.url())}${': no '}${rel(q.url())}`));
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
      await p.waitForLoadState('networkidle');
      for (const f of pages) {
        await p.goto(pathToFileURL(f).href, { waitUntil: 'networkidle' });
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
      r.verdict(where, bad, `forwards, styled, a link opens a second page, no server form shows, no script error (${pages.length} form pages)`, ', ');
    } catch (e) { r.fail(`${where}: ${String(e.message).split('\n')[0]}`); } finally { await b.close(); }
  }
  rmSync(join(out, '..'), { recursive: true, force: true });
}
// The landing page's install tabs: the selected tab merges into its panel (no line at the seam,
// the panel's colour), an unselected tab keeps the line so it reads as behind. Pixels at the seam.
async function installTabs() {
  const { server, url: root_ } = await serve(false);
  const url = `${root_}/`;
  try {
    await eachLaunch(pw, browsers, async (b, browser) => {
      for (const scheme of ['light', 'dark']) for (const width of [390, 1440]) {
        const where = `install-tabs ${browser} ${scheme} ${width}`;
        const ctx = await b.newContext({ colorScheme: scheme, viewport: { width, height: 900 } });
        const p = await ctx.newPage();
        try {
          await p.goto(url, { waitUntil: 'load' });
          const n = await p.evaluate(() => document.querySelectorAll('.tabs .tab-label').length);
          const bad = [];
          for (let sel = 0; sel < n; sel++) {
            await p.evaluate((i) => {
              const radios = document.querySelectorAll('.tabs .tab-radio');
              const r = /** @type {HTMLInputElement} */ (radios[i]);
              r.checked = true;
              r.dispatchEvent(new Event('change', { bubbles: true }));
            }, sel);
            const g = await p.evaluate(() => {
              const all = document.querySelectorAll('.tabs .tab-panel pre');
              const pres = /** @type {HTMLElement[]} */ ([...all]);
              const pre = /** @type {HTMLElement} */ (pres.find((e) => e.offsetParent));
              const cs = getComputedStyle(pre);
              const pr = pre.getBoundingClientRect();
              const labels = [...document.querySelectorAll('.tabs .tab-label')].map((l) => {
                const r = l.getBoundingClientRect();
                return { x: r.left + r.width / 2 + window.scrollX };
              });
              const bw = parseFloat(cs.borderTopWidth);
              return { y: pr.top + window.scrollY, bg: cs.backgroundColor, bw, labels };
            });
            const shot = (await p.screenshot({ fullPage: true })).toString('base64');
            const px = await p.evaluate(async ({ shot, g }) => {
              const img = new Image();
              img.src = 'data:image/png;base64,' + shot;
              await img.decode();
              const c = document.createElement('canvas');
              c.width = img.width;
              c.height = img.height;
              const k = /** @type {CanvasRenderingContext2D} */ (c.getContext('2d'));
              k.drawImage(img, 0, 0);
              const f = img.width / document.documentElement.scrollWidth;
              const y = Math.floor((g.y + g.bw / 2) * f);
              return g.labels.map((l) => {
                const d = k.getImageData(Math.floor(l.x * f), y, 1, 1).data;
                return `rgb(${d[0]}, ${d[1]}, ${d[2]})`;
              });
            }, { shot, g });
            px.forEach((c, i) => {
              const same = c === g.bg;
              const line = `tab ${i} (selected) has a line: ${c}, panel ${g.bg}`;
              if (i === sel && !same) bad.push(line);
              if (i !== sel && same) bad.push(`tab ${i} lost its line, tab ${sel} selected`);
            });
          }
          if (bad.length) r.fail(`${where}: ${bad.join('; ')}`);
          else r.ok(`${where}: selected tab merges into its panel, others keep the line`);
        } catch (e) {
          r.fail(`${where}: ${String(e.message).split('\n')[0]}`);
        } finally { await ctx.close(); }
      }
    });
  } finally { server.close(); }
}
// The repository switcher of an export of two repositories, opened from disk: it shows, ticks
// the repository the page is in, and choosing the other opens that repository's pages. An
// export has no /api, so the list comes from the page itself.
async function exportSwitch() {
  const out = join(mkdtempSync(join(tmpdir(), 'gt-export-')), 'site');
  // A second repository under another folder name, so the export names two sites.
  const other = join(mkdtempSync(join(tmpdir(), 'gt-other-')), 'other');
  renameSync(makeRepo(), other);
  sh(root, bin, 'export', out, dir, other);
  const sites = readdirSync(join(out, 'r'));
  for (const name of browsers) {
    const b = await launch(pw, name);
    const p = await b.newPage();
    const where = `export-switch ${name}`;
    const errors = [];
    p.on('pageerror', (e) => errors.push(String(e)));
    try {
      await p.goto(pathToFileURL(join(out, 'r', sites[0], 'log', 'HEAD', 'index.html')).href);
      await p.waitForLoadState('networkidle');
      const bad = [];
      if (await p.$eval('#reposwitch', (e) => e.hidden)) bad.push('the switcher is hidden');
      await p.click('#repobtn', { timeout: 3000 }).catch(() => bad.push('no switcher button'));
      await p.waitForSelector('.repopanel li', { timeout: 3000 }).catch(() => {});
      const items = await p.$$eval('.repopanel li', (l) => l.map((x) => ({
        name: x.querySelector('.name')?.textContent, here: x.hasAttribute('aria-current'),
      })));
      if (items.length !== 2) bad.push(`${items.length} repositories listed`);
      const ticked = items.some((x) => x.name === sites[0] && x.here);
      if (!ticked) bad.push('the current one is not ticked');
      const other = items.find((x) => !x.here);
      if (other) {
        await p.click(`.repopanel li:has(.name:text-is("${other.name}"))`);
        await p.waitForLoadState('load');
        const want = `/r/${other.name}/index.html`;
        if (!p.url().endsWith(want)) bad.push(`choosing ${other.name} opens ${p.url()}`);
      }
      if (errors.length) bad.push(`script errors: ${errors.join('; ')}`);
      if (bad.length) r.fail(`${where}: ${bad.join(', ')}`);
      else r.ok(`${where}: two repositories, the current ticked, the other opens from disk`);
    } catch (e) {
      r.fail(`${where}: ${String(e.message).split('\n')[0]}`);
    } finally { await b.close(); }
  }
  rmSync(join(out, '..'), { recursive: true, force: true });
  rmSync(join(other, '..'), { recursive: true, force: true });
}

  return { markSeam, aboutPlace, siteTheme, exportFromDisk, exportSwitch, installTabs };
}
