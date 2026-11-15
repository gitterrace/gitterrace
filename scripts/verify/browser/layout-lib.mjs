// Layout checks of the insights and commit pages: inline controls share one size, section heads
// keep room above them, and file paths break only between folders. Imported by
// scripts/verify/browser/browser-check.mjs, not run on its own.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { sh } from '../../lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (dir, srv, r, browsers, context, ...) */
export function checks(env) {
  const { dir, srv, r, browsers, context } = env;
// A metadata line's controls (the sha chip, Copy, the actions trigger) are one size on one
// line: a trigger that is a pixel shorter or lower than its neighbours reads as a different
// control. The commit page in light and dark, after the actions script added its trigger.
async function inlineControls() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const scheme of ['light', 'dark']) {
      const p = await ctx.newPage();
      await p.emulateMedia({ colorScheme: scheme });
      await p.setViewportSize({ width: 1400, height: 700 });
      await p.goto(srv.base + '/commit/HEAD', { waitUntil: 'load' });
      await p.waitForSelector('p.meta .actbtn', { timeout: 5000 }).catch(() => {});
      const m = await p.$$eval('p.meta > :is(a.sha, button)', (els) => els.map((e) => {
        const r = e.getBoundingClientRect(), s = getComputedStyle(e);
        return { what: e.className || e.tagName, h: Math.round(r.height), top: Math.round(r.top), bg: s.backgroundColor };
      }));
      const where = `inline-controls ${name} ${scheme}`;
      const bad = [];
      if (!m.some((c) => c.what.includes('actbtn'))) bad.push('no actions trigger on the commit line');
      const heights = new Set(m.map((c) => c.h)), tops = new Set(m.map((c) => c.top));
      if (heights.size > 1) bad.push(`heights ${m.map((c) => `${c.what} ${c.h}`).join(', ')}`);
      if (tops.size > 1) bad.push(`tops ${m.map((c) => `${c.what} ${c.top}`).join(', ')}`);
      const buttons = m.filter((c) => c.what !== 'sha');
      if (new Set(buttons.map((c) => c.bg)).size > 1) bad.push(`button fills ${buttons.map((c) => `${c.what} ${c.bg}`).join(', ')}`);
      bad.length ? r.fail(`${where}: ${bad.join('; ')}`) : r.ok(`${where}: ${m.length} controls, one size on one line`);
      await p.close();
    }
    await b.close();
  }
}
// Every section head on the insights pages keeps space above it: a heading flush against the
// table before it reads as that table's footer. Sections folded into a page (types and
// frequency in Activity, patterns in Hotspots) follow the same rule as the page's own.
async function sectionGaps() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const path of ['/stats', '/stats/hotspots', '/stats/contributors', '/stats/ownership', '/stats/languages']) {
      const p = await ctx.newPage();
      await p.setViewportSize({ width: 1400, height: 800 });
      await p.goto(srv.base + path, { waitUntil: 'load' });
      const tight = await p.$$eval('main .chart-head', (els) => els.flatMap((e) => {
        const prev = e.previousElementSibling;
        if (!prev) return [];
        const gap = Math.round(e.getBoundingClientRect().top - prev.getBoundingClientRect().bottom);
        return gap < 12 ? [`${e.querySelector('h2')?.textContent} ${gap}px under ${prev.className.split(' ')[0]}`] : [];
      }));
      const where = `section-gaps ${name} ${path}`;
      tight.length ? r.fail(`${where}: ${tight.slice(0, 4).join('; ')}`) : r.ok(`${where}: every section head has room above it`);
      await p.close();
    }
    await b.close();
  }
}
// A file path in a table cell breaks only after a slash, never inside a name: gitterrace-web
// split as "gitterrace-" and "web" reads as two files. Every path cell on the insights pages
// with path tables, at a wide, a middle and a narrow width.
async function pathCells() {
  // Two deep, hyphenated paths that change together, so they show up as a coupled pair, in the
  // hotspots table and as owned directories. Built from parts: they live in the fixture only.
  const top = ['pkgs', 'long-package-name'].join('/');
  const deep = [`${top}/src/coupled-module.rs`, `${top}/views/coupled-page.html`];
  mkdirSync(join(dir, top, 'src'), { recursive: true });
  mkdirSync(join(dir, top, 'views'), { recursive: true });
  for (let i = 0; i < 4; i++) {
    for (const f of deep) writeFileSync(join(dir, f), `${i}\n`);
    sh(dir, 'git', 'add', '-A');
    sh(dir, 'git', 'commit', '-q', '-m', `feat(web): coupled change ${i}`);
  }
  await new Promise((ok) => setTimeout(ok, 1500));
  const pages = ['/stats/hotspots', '/stats/ownership'];
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const width of [1440, 900, 360]) for (const path of pages) {
      const p = await ctx.newPage();
      await p.setViewportSize({ width, height: 700 });
      await p.goto(srv.base + path, { waitUntil: 'load' });
      await p.evaluate(() => document.fonts.ready);
      const bad = await p.evaluate(() => {
        const out = [];
        for (const a of document.querySelectorAll('table.entries :is(td,th) a')) {
          if (!a.textContent.includes('/')) continue;
          const range = document.createRange();
          const walk = document.createTreeWalker(a, NodeFilter.SHOW_TEXT);
          let top = null, prev = '';
          for (let node = walk.nextNode(); node; node = walk.nextNode()) {
            const t = node.textContent;
            for (let i = 0; i < t.length; i++) {
              range.setStart(node, i); range.setEnd(node, i + 1);
              const y = Math.round(range.getBoundingClientRect().top);
              if (top !== null && y > top && prev !== '/') out.push(`${a.textContent} breaks before "${t.slice(i, i + 8)}"`);
              top = y; prev = t[i];
            }
          }
        }
        return out.slice(0, 3);
      });
      const where = `path-cells ${name} ${width} ${path}`;
      bad.length ? r.fail(`${where}: ${bad.join('; ')}`) : r.ok(`${where}: paths break only after a slash`);
      await p.close();
    }
    await b.close();
  }
}

// Every code row is as tall as its gutter number, an empty row included: a row of 24px text
// beside a 20px empty one leaves the gutter band stepping by the difference. And a file page
// and a diff set code at one size, so a shared rule cannot enlarge one of them.
async function codeRows() {
  writeFileSync(join(dir, 'rows.txt'), 'one\n\nthree\n');
  sh(dir, 'git', 'add', '-A');
  sh(dir, 'git', 'commit', '-q', '-m', 'docs: rows with an empty line');
  await new Promise((ok) => setTimeout(ok, 600));
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    const seen = {};
    for (const [path, rows, num] of [['/blob/HEAD/rows.txt', '.src .l', '::before'],
      ['/commit/HEAD', 'div.diff :is(.add,.del,.ctx)', '::after']]) {
      await p.goto(srv.base + path, { waitUntil: 'load' });
      seen[path] = await p.$$eval(rows, (els, num) => els.map((e) => {
        const s = getComputedStyle(e);
        const n = parseFloat(getComputedStyle(e, num).height);
        return { h: e.getBoundingClientRect().height, n, fs: s.fontSize };
      }), num);
    }
    const all = Object.values(seen).flat();
    const bad = all.filter((x) => Math.abs(x.h - x.n) > 0.5)
      .map((x) => `row ${x.h}px, number ${x.n}px`);
    const sizes = new Set(all.map((x) => x.fs));
    if (!seen['/commit/HEAD'].length) bad.push('no diff rows');
    if (sizes.size > 1) bad.push(`code sizes ${[...sizes].join(', ')}`);
    const where = `code-rows ${name}`;
    if (bad.length) r.fail(`${where}: ${bad.slice(0, 3).join('; ')}`);
    else r.ok(`${where}: ${all.length} rows as tall as their numbers, one code size`);
    await b.close();
  }
}

  return { inlineControls, sectionGaps, pathCells, codeRows };
}
