// The checks of the core pages that need no fixture of their own: the sidebar, the header,
// the tab rows, the period control, the sticky heads and the settings page.

import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { eachBrowser, STATS, sh, sweep, commitFiles, synced, reloadUntil, scrollSettled } from '../../lib/browser-lib.mjs';
import { varsStep } from './settings-vars-lib.mjs';

let dir, srv, r, browsers, context, arg, cfg;

export function checks(env) {
  ({ dir, srv, r, browsers, context, arg, cfg } = env);
  return {
    sideWidth,
    calmHeader,
    tabsOneLine,
    segOneRow,
    tabLevels,
    periodSteady,
    stickyHeads,
    settings,
  };
}

async function sideWidth() {
  const sw = { browsers, context, widths: [1440, 1100, 360], height: 900 };
  await sweep(sw, async (p, { name, scheme, width }) => {
    // The sidebar appears once the index is built: reload until it does.
    await reloadUntil(p, async () => !!(await p.$('.side .langbar')), srv.base + '/', 'the sidebar');
    const m = await p.evaluate(() => {
      const s = document.querySelector('.side svg.spark'),
        l = document.querySelector('.side .langbar');
      if (!s || !l) return null;
      const a = s.getBoundingClientRect(), c = l.getBoundingClientRect();
      return { sw: a.width, lw: c.width, sh: a.height, lh: c.height };
    });
    const where = `side-width ${name} ${scheme} ${width}`;
    if (!m) r.fail(`${where}: no sparkline or language bar`);
    else if (Math.abs(m.sw - m.lw) > 1) {
      r.fail(`${where}: sparkline ${m.sw} px wide, language bar ${m.lw} px`);
    } else r.ok(`${where}: both ${Math.round(m.lw)} px wide`);
  });
}
async function calmHeader() {
  writeFileSync(join(dir, 'a.txt'), 'one\ndirty\n');
  await synced(srv, dir);
  const sw = { browsers, context, widths: [1440, 900, 360], height: 400 };
  await sweep(sw, async (p, { name, scheme, width }) => {
    await p.goto(srv.base + '/', { waitUntil: 'load' });
    await p.mouse.move(width - 1, 399);
    const m = await p.evaluate(() => {
      const look = (el) => {
        if (!el) return null;
        const s = getComputedStyle(el);
        const bw = ['Top', 'Right', 'Bottom', 'Left'].some((k) =>
          parseFloat(s[`border${k}Width`]) > 0 && s[`border${k}Style`] !== 'none' &&
          !/rgba\(.*, 0\)$|transparent/.test(s[`border${k}Color`])
        );
        return { border: bw, weight: Number(s.fontWeight) };
      };
      const chip = document.getElementById('wtchip'),
        q = /** @type {HTMLElement | null} */ (document.getElementById('searchbtn'));
      const x = () => (q && q.offsetParent ? q.getBoundingClientRect().x : null);
      const out = {
        branch: look(document.getElementById('refbtn')),
        chip: look(chip),
        shown: chip && !chip.hidden,
      };
      if (chip) {
        const a = x();
        chip.hidden = !chip.hidden;
        const c = x();
        chip.hidden = !chip.hidden;
        out.dx = a === null ? 0 : Math.abs(a - c);
      }
      return out;
    });
    const where = `calm-header ${name} ${scheme} ${width}`;
    const bad = [];
    if (!m.branch) bad.push('no branch switcher');
    else {
      if (m.branch.border) bad.push('switcher has a border at rest');
      if (m.branch.weight >= 600) bad.push(`switcher weight ${m.branch.weight}`);
    }
    if (!m.shown) bad.push('chip not shown on a dirty tree');
    else {
      if (m.chip.border) bad.push('chip has a border at rest');
      if (m.chip.weight >= 600) bad.push(`chip weight ${m.chip.weight}`);
    }
    if (m.dx > 0.5) bad.push(`search moves ${m.dx.toFixed(1)} px when the chip toggles`);
    r.verdict(where, bad, `quiet, search steady`, ', ');
  });
  sh(dir, 'git', 'checkout', '-q', '--', 'a.txt');
}
// The section tabs stay one row at every width: on a narrow screen they scroll sideways instead
// of wrapping, the tab of the open section is fully in view, and an edge fade shows that more
// tabs are off screen (the scroll bar is hidden).
async function tabsOneLine() {
  const rows = [['.tabnav', '/stats'], ['.tabs', '/stats/ownership']];
  const sw = { browsers, context, widths: [1440, 900, 360, 320], height: 600, items: rows };
  await sweep(sw, async (p, { name, scheme, width, item: [row, path] }) => {
    await p.goto(srv.base + path, { waitUntil: 'load' });
    await p.evaluate(() =>
      document.fonts.ready.then(() =>
        new Promise((ok) => requestAnimationFrame(() => requestAnimationFrame(ok)))
      )
    );
    const m = await p.evaluate((sel) => {
      const nav = document.querySelector(sel);
      if (!nav) {
        return null;
      }
      const tabs = [...nav.querySelectorAll('a')];
      const tops = new Set(tabs.map((a) => Math.round(a.getBoundingClientRect().top)));
      const on = nav.querySelector('a[aria-current="page"]');
      const n = nav.getBoundingClientRect(), o = on && on.getBoundingClientRect();
      const over = nav.scrollWidth > nav.clientWidth + 1;
      const mask = getComputedStyle(nav).maskImage || getComputedStyle(nav).webkitMaskImage ||
        'none';
      return {
        rows: tops.size,
        over,
        fade: mask !== 'none',
        onIn: !!o && o.left >= n.left - 0.5 && o.right <= n.right + 0.5,
        page: document.documentElement.scrollWidth > innerWidth + 1,
      };
    }, row);
    const where = `tabs-one-line ${name} ${scheme} ${width} ${row}`;
    const bad = [];
    if (!m) bad.push('no section tabs');
    else {
      if (m.rows !== 1) bad.push(`tabs in ${m.rows} rows`);
      if (!m.onIn) bad.push('the open tab is not fully visible');
      if (m.over && !m.fade) bad.push('tabs overflow with no edge fade');
      if (m.page) bad.push('the page scrolls sideways');
    }
    r.verdict(where, bad, `one row${m.over ? ', scrolls with a fade' : ''}`, ', ');
  });
}
// Segmented controls never break across lines: on every insights page and a document, at 360
// and 1440 px, each control shows either one row of choices or its native select, never a
// second row and never wider than the page.
async function segOneRow() {
  const paths = [...STATS, '/blob/HEAD/README.md'];
  const sw = { browsers, context, widths: [1440, 360], height: 700, items: paths };
  await sweep(sw, async (p, { name, scheme, width, item: path }) => {
    await p.goto(srv.base + path, { waitUntil: 'load' });
    // Measure the settled page: after the web fonts and the frame that refits the controls.
    await p.evaluate(() =>
      document.fonts.ready.then(() =>
        new Promise((ok) => requestAnimationFrame(() => requestAnimationFrame(ok)))
      )
    );
    const m = await p.evaluate(() =>
      [...document.querySelectorAll('.segctl')].map((box) => {
        const segs = box.querySelector('.segs'), sel = box.querySelector('select');
        const shown = (e) =>
          !!e && getComputedStyle(e).display !== 'none' && e.getClientRects().length > 0;
        const tops = new Set(
          [...segs.children].map((c) => Math.round(c.getBoundingClientRect().top)),
        );
        const r = box.getBoundingClientRect();
        return {
          name: box.getAttribute('aria-label'),
          segs: shown(segs),
          sel: shown(sel),
          rows: shown(segs) ? tops.size : 0,
          cut: segs.scrollWidth > segs.clientWidth + 1 && shown(segs),
          out: r.right > document.documentElement.clientWidth + 1,
        };
      })
    );
    const where = `seg-one-row ${name} ${scheme} ${width} ${path}`;
    const bad = [];
    for (const c of m) {
      if (c.segs === c.sel) {
        bad.push(`${c.name}: ${c.segs ? 'both' : 'neither'} row and select shown`);
      }
      if (c.rows > 1) bad.push(`${c.name}: ${c.rows} rows`);
      if (c.cut) bad.push(`${c.name}: choices cut off`);
      if (c.out) bad.push(`${c.name}: wider than the page`);
    }
    if (path.startsWith('/stats') && !m.length) bad.push('no segmented control');
    r.verdict(where, bad, `${m.length} controls, one row or a select`, ', ');
  });
}
// Tab levels: a page's own tabs never share the header nav's type, and every insights page is
// one tab of the row: no More, and the open tab is marked in place.
async function tabLevels() {
  await eachBrowser(browsers, context, async (ctx, name) => {
    for (const width of [1440, 900]) {
      const p = await ctx.newPage();
      await p.setViewportSize({ width, height: 600 });
      await p.goto(srv.base + '/stats/ownership', { waitUntil: 'load' });
      const where = `tab-levels ${name} ${width}`;
      const look = await p.evaluate(() => {
        const f = (s) => {
          const e = document.querySelector(s);
          if (!e) return null;
          const c = getComputedStyle(e);
          return c.fontFamily + '|' + c.textTransform;
        };
        const on = document.querySelector('.tabs>a[aria-current="page"]');
        return {
          nav: f('.tabnav a'),
          page: f('.tabs>a'),
          more: !!document.querySelector('.tabs details, .tabs summary'),
          on: on ? on.textContent.trim() : '',
        };
      });
      const bad = [];
      if (!look.nav || !look.page) bad.push('missing tab rows');
      else if (look.nav === look.page) {
        bad.push(`page tabs look like the header nav (${look.page})`);
      }
      if (look.more) bad.push('the tab row still hides tabs behind More');
      if (look.on !== 'Ownership') bad.push(`open tab reads "${look.on}" on Ownership`);
      r.verdict(where, bad, `two tab levels, every tab in the row`, ', ');
      await p.close();
    }
  });
}
// Period steady: the period control sits in one place whatever the filters are. With no
// filter, a period, and a period plus a path chip, its box starts at the same x and y, at 360
// and 1400 px, so choosing a period never moves the control under the pointer; and the chips
// row comes last, below the counting line, at both widths.
async function periodSteady() {
  await eachBrowser(browsers, context, async (ctx, name) => {
    for (const width of [1400, 360]) {
      const at = [];
      for (const q of ['', '?period=1w', '?period=1w&q=path%3Asrc']) {
        const p = await ctx.newPage();
        await p.setViewportSize({ width, height: 700 });
        await p.goto(srv.base + '/stats' + q, { waitUntil: 'load' });
        await p.evaluate(() =>
          document.fonts.ready.then(() =>
            new Promise((ok) => requestAnimationFrame(() => requestAnimationFrame(ok)))
          )
        );
        at.push(
          await p.evaluate(() => {
            const c = document.querySelector('.segctl[aria-label="Period"]');
            if (!c) return null;
            const r = c.getBoundingClientRect();
            const opt = document.querySelector('.optline'),
              chips = document.querySelector('.chips');
            const below = !chips || !opt ||
              chips.getBoundingClientRect().top >= opt.getBoundingClientRect().bottom - 0.5;
            return {
              q: location.search,
              x: Math.round(r.left),
              y: Math.round(r.top + scrollY),
              chip: !!document.querySelector('.chip'),
              below,
            };
          }),
        );
        await p.close();
      }
      const where = `period-steady ${name} ${width}`;
      const bad = [];
      if (at.some((a) => !a)) bad.push('no period control');
      else {
        for (const a of at.slice(1)) {
          if (a.x !== at[0].x || a.y !== at[0].y) {
            bad.push(`${a.q || 'none'} at ${a.x},${a.y}, not ${at[0].x},${at[0].y}`);
          }
        }
        if (!at[2].chip) bad.push('the path filter shows no chip');
        if (at.some((a) => !a.below)) bad.push('the chips row sits above the counting line');
      }
      r.verdict(where, bad, `the control stays at ${at[0].x},${at[0].y}`, ', ');
    }
  });
}
// Sticky heads: on a commit with three 300-line files, scrolled halfway into the second, its
// header is pinned at the top and the first's is not; a #L120 link on a source page lands
// fully below the pinned path bar; at 375 px the site header never pins. The commit is made
// with a private index on a side ref, so HEAD and the work tree the other checks use stay put.
async function stickyHeads() {
  const files = {};
  for (const n of ['one', 'two', 'three']) {
    const lines = Array.from({ length: 300 }, (_, i) => `${n} line ${i + 1}`);
    files[`sticky/${n}.txt`] = lines.join('\n') + '\n';
  }
  const index = join(cfg, 'sticky-index');
  const sha = commitFiles(dir, index, files, 'refs/heads/sticky-check', 'Three long files');
  const sw = { browsers, context, widths: [1440, 375], height: 700 };
  await sweep(sw, async (p, { name, scheme, width }) => {
    await p.goto(`${srv.base}/commit/${sha}`, { waitUntil: 'load' });
    const m = await p.evaluate(() => {
      const s = [...document.querySelectorAll('section.file')];
      if (s.length < 2) return null;
      const r = s[1].getBoundingClientRect();
      scrollTo(0, scrollY + r.top + r.height / 2);
      const top = (el) => el.getBoundingClientRect().top;
      const site = document.querySelector('header.top');
      return {
        second: top(s[1].querySelector('header')),
        first: top(s[0].querySelector('header')),
        site: site ? top(site) : 0,
      };
    });
    const where = `sticky-heads ${name} ${scheme} ${width}`;
    const bad = [];
    if (!m) bad.push('fewer than two files on the commit page');
    else {
      if (Math.abs(m.second) > 1) {
        bad.push(`second file header at ${m.second.toFixed(1)} px, not pinned`);
      }
      if (Math.abs(m.first) <= 1) bad.push('first file header still pinned');
      if (width <= 600 && Math.abs(m.site) <= 1) bad.push('site header pinned at 375');
    }
    await p.goto(`${srv.base}/blob/${sha}/sticky/two.txt#L120`, { waitUntil: 'load' });
    await scrollSettled(p);
    const a = await p.evaluate(() => {
      const l = document.getElementById('L120');
      if (!l) return null;
      const t = l.getBoundingClientRect();
      let cover = 0;
      for (const el of document.querySelectorAll('body *')) {
        const st = getComputedStyle(el);
        if (st.position !== 'sticky' && st.position !== 'fixed') continue;
        const r = el.getBoundingClientRect();
        if (r.height && r.top <= 1 && r.left < t.right && r.right > t.left) {
          cover = Math.max(cover, r.bottom);
        }
      }
      return { top: t.top, bottom: t.bottom, cover, h: innerHeight };
    });
    if (!a) bad.push('no #L120 on the source page');
    else if (a.top < a.cover - 0.5 || a.bottom > a.h) {
      bad.push(
        `#L120 at ${a.top.toFixed(1)} px, under a pinned bar ending at ${
          a.cover.toFixed(1)
        } px`,
      );
    }
    if (width > 600 && a && a.cover < 1) bad.push('source path bar not pinned');
    // The pinned page head holds the path and the file's tabs, so the reader keeps both.
    if (width > 600) {
      const held = await p.evaluate(() => {
        const h = document.querySelector('main>.ph.pin');
        const path = h && h.querySelector('h1.crumbs a:last-child');
        return !!(path && h.querySelector('.tabs a[aria-current="page"]'));
      });
      if (!held) bad.push('the pinned head lacks the path or the File tab');
    }
    const held =
      'file header pinned and handed over, #L120 clear of the head, path and tabs held';
    r.verdict(where, bad, `${held}`, ', ');
    if (arg('--shots')) {
      mkdirSync(arg('--shots'), { recursive: true });
      await p.screenshot({
        path: join(arg('--shots'), `sticky-blob-${name}-${scheme}-${width}.png`),
      });
      await p.goto(`${srv.base}/commit/${sha}`);
      await p.evaluate(() => {
        const s = document.querySelectorAll('section.file')[1].getBoundingClientRect();
        scrollTo(0, scrollY + s.top + s.height / 2);
      });
      await p.screenshot({
        path: join(arg('--shots'), `sticky-commit-${name}-${scheme}-${width}.png`),
      });
    }
  });
}
// ---- settings --------------------------------------------------------------------------------
async function settings() {
  for (const name of browsers) {
    // Each browser starts from the same file, never the one the last browser saved.
    rmSync(join(cfg, 'config.toml'), { force: true });
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    try {
      await p.goto(srv.base + '/', { waitUntil: 'load' });
      const gear = await p.$('#settingsbtn');
      await p.click('.actbtn.top');
      await p.waitForSelector('.actmenu li >> text=Settings…');
      await p.click('.actmenu li >> text=Settings…');
      await p.waitForSelector('#setform');
      if (gear) r.fail(`settings ${name}: a header gear duplicates the menu entry`);
      await p.fill('#f-watch\\.rescan_secs', '75');
      await p.click('.segs [data-show="plain"]');
      await p.waitForFunction(
        () =>
          /rescan_secs = 75/.test(
            /** @type {HTMLTextAreaElement} */ (document.getElementById('settext')).value,
          ),
        null,
        { timeout: 3000 },
      );
      await p.fill('#settext', '[watch]\nrescan_secs = 3\n');
      await p.waitForSelector('#setgut .bad', { timeout: 3000 });
      const off = await p.$eval('#setsave', (x) => /** @type {HTMLButtonElement} */ (x).disabled);
      await p.fill('#settext', '[watch]\nrescan_secs = 80\n');
      await p.waitForFunction(
        () => !/** @type {HTMLButtonElement} */ (document.getElementById('setsave')).disabled,
        null,
        { timeout: 3000 },
      );
      await Promise.all([p.waitForNavigation(), p.click('#setsave')]);
      const saved = readFileSync(join(cfg, 'config.toml'), 'utf8');
      const shown = await p.$eval(
        '#f-watch\\.rescan_secs',
        (x) => /** @type {HTMLInputElement} */ (x).value,
      );
      if (off && saved === '[watch]\nrescan_secs = 80\n' && shown === '80') {
        r.ok(`settings ${name}: form, plain, error line and save`);
      } else {r.fail(
          `settings ${name}: save off on error ${off}, file ${
            JSON.stringify(saved)
          }, form shows ${shown}`,
        );}
      // actions.toml: rename the "Mark" action in its card; the comment-free rest stays.
      const before = readFileSync(join(cfg, 'actions.toml'), 'utf8');
      await p.goto(srv.origin + '/_settings?f=actions', { waitUntil: 'load' });
      await p.fill('#f-action\\.0\\.label', 'Mark it');
      await p.waitForFunction(
        () => !/** @type {HTMLButtonElement} */ (document.getElementById('setsave')).disabled,
        null,
        { timeout: 3000 },
      );
      await Promise.all([p.waitForNavigation(), p.click('#setsave')]);
      const after = readFileSync(join(cfg, 'actions.toml'), 'utf8');
      if (after === before.replace('label = "Mark"', 'label = "Mark it"')) {
        r.ok(`settings ${name}: an action card edit saves in place`);
      } else r.fail(`settings ${name}: actions.toml became ${JSON.stringify(after)}`);
      await varsStep(p, srv, cfg, name, r);
      writeFileSync(join(cfg, 'actions.toml'), before, { mode: 0o600 });
    } catch (e) {
      r.fail(`settings ${name}: ${e.message.split('\n')[0]}`);
    }
    await b.close();
  }
}
