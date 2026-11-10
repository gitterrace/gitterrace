// The "On this page" filter on every page that has the menu. Imported by
// scripts/browser-check.mjs, not run on its own.
import { writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { sh } from './lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, dir, ...) */
export function checks(env) {
  const { srv, r, browsers, context, dir } = env;

  // The filter shows, narrows the list on typing, and Escape closes the menu and returns focus to
  // its button, on a Markdown page and on an insights page alike.
  async function outlineFilter() {
    writeFileSync(join(dir, 'filter-outline.md'), '# Doc\n\n' + Array.from({ length: 12 }, (_, i) => `## Head ${i}\n\ntext\n`).join('\n'));
    sh(dir, 'git', 'add', '-A');
    sh(dir, 'git', 'commit', '-q', '-m', 'docs: filter outline fixture');
    await new Promise((ok) => setTimeout(ok, 600));
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      const p = await ctx.newPage();
      await p.setViewportSize({ width: 900, height: 800 });
      for (const path of ['/blob/HEAD/filter-outline.md']) {
        const where = `outline-filter ${name} ${path}`;
        try {
          await p.goto(srv.base + path, { waitUntil: 'load' });
          await p.click('details.outline summary');
          await p.waitForTimeout(200);
          const v = await p.evaluate(() => {
            const d = document.querySelector('details.outline');
            const q = /** @type {HTMLInputElement} */ (d.querySelector('.outline-q'));
            return { shown: !q.hidden, focused: document.activeElement === q, total: d.querySelectorAll('li').length };
          });
          await p.keyboard.type('zzzz-no-such-heading');
          const after = await p.evaluate(() => [...document.querySelectorAll('details.outline li')].filter((l) => !(/** @type {HTMLElement} */ (l)).hidden).length);
          await p.keyboard.press('Escape');
          const esc = await p.evaluate(() => ({ closed: !(/** @type {HTMLDetailsElement} */ (document.querySelector('details.outline'))).open, onButton: document.activeElement === document.querySelector('details.outline summary') }));
          if (v.shown && v.focused && v.total > 0 && after === 0 && esc.closed && esc.onButton) r.ok(`${where}: filter shown and focused, hides items, Escape returns to the button`);
          else r.fail(`${where}: ${JSON.stringify({ ...v, after, ...esc })}`);
        } catch (e) { r.fail(`${where}: ${e.message.split('\n')[0]}`); }
      }
      await b.close();
    }
  }
  // A source file's symbols: at 1440 px a rail beside the code, below the pinned path bar; at
  // 390 px no rail, a menu in the path bar inside the viewport. A link selects its line and
  // shows it as any line link does: in view, clear of the pinned bar. No page-wide scroll.
  async function symbols() {
    const fn = (i) => `pub fn f${i}() -> u32 {\n${'    1 +\n'.repeat(4)}    0\n}\n`;
    const body = Array.from({ length: 40 }, (_, i) => fn(i)).join('\n');
    const head = 'pub struct Holder;\n\nimpl Holder {\n    pub fn new() -> Self { Holder }\n}\n\n';
    writeFileSync(join(dir, 'symbols.rs'), head + body);
    sh(dir, 'git', 'add', '-A');
    sh(dir, 'git', 'commit', '-q', '-m', 'feat: symbols fixture');
    await new Promise((ok) => setTimeout(ok, 600));
    // The followed line: selected, its box, and the bottom of the path bar when it is pinned.
    // f5 is on line 47.
    const landed = (p, id) => p.evaluate((i) => {
      const line = document.getElementById(i), bar = document.querySelector('main>.ph');
      const r = line.getBoundingClientRect(), pinned = getComputedStyle(bar).position === 'sticky';
      return {
        hash: location.hash, sel: line.classList.contains('sel'), top: r.top, bottom: r.bottom,
        bar: pinned ? bar.getBoundingClientRect().bottom : 0, vh: innerHeight,
      };
    }, id);
    const shown = (p, sel) => p.evaluate((s) => {
      const e = document.querySelector(s);
      return !!e && getComputedStyle(e).display !== 'none' && e.getClientRects().length > 0;
    }, sel);
    const wide = (p) => p.evaluate(() => {
      const d = document.documentElement;
      return d.scrollWidth - d.clientWidth;
    });
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      const p = await ctx.newPage();
      for (const width of [1440, 390]) {
        const where = `symbols ${name} ${width}`;
        try {
          await p.setViewportSize({ width, height: 800 });
          await p.goto(srv.base + '/blob/HEAD/symbols.rs', { waitUntil: 'load' });
          const v = {
            rail: await shown(p, '.outline.rail'),
            menu: await shown(p, 'main>.ph details.outline'),
            over: await wide(p),
          };
          v.items = await p.evaluate(() => document.querySelectorAll('[data-outline] a').length);
          if (width === 1440) {
            await p.click('.outline.rail a[href="#L47"]');
            await p.waitForTimeout(400);
            v.go = await landed(p, 'L47');
            v.railTop = await p.evaluate(
              () => document.querySelector('.outline.rail').getBoundingClientRect().top,
            );
            v.on = await p.evaluate(
              () => document.querySelector('.outline.rail a.on')?.getAttribute('href'),
            );
          } else {
            await p.click('main>.ph details.outline summary');
            await p.waitForTimeout(200);
            v.pop = await p.evaluate(() => {
              const r = document.querySelector('main>.ph .outline-pop').getBoundingClientRect();
              return r.left >= 0 && r.right <= innerWidth + 1;
            });
            await p.click('main>.ph .outline-pop a[href="#L47"]');
            await p.waitForTimeout(400);
            v.go = await landed(p, 'L47');
          }
          const shots = process.env.OUTLINE_SHOTS || tmpdir();
          await p.screenshot({ path: join(shots, `symbols-${name}-${width}.png`) });
          // 2 structure entries plus 40 functions, listed twice (rail and menu).
          const listed = v.items === 2 * 43;
          const g = v.go;
          const lands = g.hash === '#L47' && g.sel && g.top >= g.bar - 1 && g.bottom <= g.vh;
          const layout = width === 1440
            ? v.rail && !v.menu && v.railTop >= g.bar - 1 && v.on === '#L47'
            : !v.rail && v.menu && v.pop;
          const shape = width === 1440
            ? 'rail beside the code under the path bar, the followed item lit'
            : 'menu in the path bar';
          if (listed && lands && layout && v.over <= 0) {
            r.ok(`${where}: ${shape}, the link selects its line in view`);
          }
          else r.fail(`${where}: ${JSON.stringify(v)}`);
        } catch (e) { r.fail(`${where}: ${e.message.split('\n')[0]}`); }
      }
      await b.close();
    }
  }
  return { outlineFilter, symbols };
}
