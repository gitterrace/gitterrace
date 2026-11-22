// The "On this page" menu. Imported by
// scripts/verify/browser/browser-check.mjs, not run on its own.
import { writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { eachBrowser, commitAll, synced, frames, scrollSettled } from '../../lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, dir, ...) */
export function checks(env) {
  const { srv, r, browsers, context, dir } = env;

  // Escape closes the menu and returns focus to its button.
  async function outlineMenu() {
    writeFileSync(join(dir, 'menu-outline.md'), '# Doc\n\n' + Array.from({ length: 12 }, (_, i) => `## Head ${i}\n\ntext\n`).join('\n'));
    commitAll(dir, 'docs: menu outline fixture');
    await synced(srv, dir);
    await eachBrowser(browsers, context, async (ctx, name) => {
      const p = await ctx.newPage();
      await p.setViewportSize({ width: 900, height: 800 });
      for (const path of ['/blob/HEAD/menu-outline.md']) {
        const where = `outline-menu ${name} ${path}`;
        try {
          await p.goto(srv.base + path, { waitUntil: 'load' });
          await p.click('details.outline summary');
          await frames(p);
          const v = await p.evaluate(() => {
            const d = document.querySelector('details.outline');
            return { open: /** @type {HTMLDetailsElement} */ (d).open, total: d.querySelectorAll('li').length };
          });
          await p.keyboard.press('Escape');
          const esc = await p.evaluate(() => ({ closed: !(/** @type {HTMLDetailsElement} */ (document.querySelector('details.outline'))).open, onButton: document.activeElement === document.querySelector('details.outline summary') }));
          if (v.open && v.total > 0 && esc.closed && esc.onButton) r.ok(`${where}: opens with its list, Escape returns to the button`);
          else r.fail(`${where}: ${JSON.stringify({ ...v, ...esc })}`);
        } catch (e) { r.fail(`${where}: ${e.message.split('\n')[0]}`); }
      }
    });
  }
  // A source file's symbols: at 1440 px a rail beside the code, below the pinned path bar; at
  // 390 px no rail, a menu in the path bar inside the viewport. A link selects its line and
  // shows it as any line link does: in view, clear of the pinned bar. No page-wide scroll.
  async function symbols() {
    const fn = (i) => `pub fn f${i}() -> u32 {\n${'    1 +\n'.repeat(4)}    0\n}\n`;
    const body = Array.from({ length: 40 }, (_, i) => fn(i)).join('\n');
    const head = 'pub struct Holder;\n\nimpl Holder {\n    pub fn new() -> Self { Holder }\n}\n\n';
    writeFileSync(join(dir, 'symbols.rs'), head + body);
    commitAll(dir, 'feat: symbols fixture');
    await synced(srv, dir);
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
    await eachBrowser(browsers, context, async (ctx, name) => {
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
            await scrollSettled(p);
            v.go = await landed(p, 'L47');
            v.railTop = await p.evaluate(
              () => document.querySelector('.outline.rail').getBoundingClientRect().top,
            );
            v.on = await p.evaluate(
              () => document.querySelector('.outline.rail a.on')?.getAttribute('href'),
            );
          } else {
            await p.click('main>.ph details.outline summary');
            await frames(p);
            v.pop = await p.evaluate(() => {
              const r = document.querySelector('main>.ph .outline-pop').getBoundingClientRect();
              return r.left >= 0 && r.right <= innerWidth + 1;
            });
            await p.click('main>.ph .outline-pop a[href="#L47"]');
            await scrollSettled(p);
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
    });
  }
  // A long source file opened at #L<n> in the middle lights the symbol that holds n, and still
  // does after scrolling up and down. The page swaps its code chunks once the highlighted copy
  // arrives, so a head element held from load goes stale (rect all zeros) and the last symbol
  // lit up. Each symbol is 60 lines (59 of code, a blank), taller than the reading line is deep, so
  // the holder is the same wherever the line lands. The file is over 32 KB, which is what makes
  // the page highlight its tail after first paint.
  async function outlineCurrent() {
    const fn = (i) => `pub fn g${i}() -> u32 {\n${"    1 +\n".repeat(56)}    0\n}\n`;
    writeFileSync(join(dir, 'current.rs'), Array.from({ length: 90 }, (_, i) => fn(i)).join("\n"));
    commitAll(dir, 'feat: current symbol fixture');
    await synced(srv, dir);
    // The symbol whose lines hold n: fn k starts on line 60k+1.
    const holding = (n) => `#L${60 * Math.floor((n - 1) / 60) + 1}`;
    const lit = (p) => p.evaluate(
      () => [...document.querySelectorAll('.outline.rail a.on')].map((a) => a.getAttribute('href')),
    );
    await eachBrowser(browsers, context, async (ctx, name) => {
      const p = await ctx.newPage();
      for (const [width, height] of [[1440, 800], [1440, 1200]]) {
        const where = `outline-current ${name} ${width}x${height}`;
        try {
          await p.setViewportSize({ width, height });
          const n = 1830;
          await p.goto(`${srv.base}/blob/HEAD/current.rs#L${n}`, { waitUntil: 'load' });
          // Past the first-screen swap of the code chunks.
          await p.waitForFunction(() => !document.querySelector('.src[data-hl]') ||
            document.querySelectorAll('.src .ch').length > 0);
          await scrollSettled(p);
          const seen = { landed: await lit(p) };
          await p.mouse.wheel(0, -2400);
          await scrollSettled(p);
          seen.up = await lit(p);
          seen.upLine = await p.evaluate(() => {
            const line = (innerHeight) * 0.55;
            const ids = [...document.querySelectorAll('.outline.rail a')]
              .filter((a) => a instanceof HTMLAnchorElement)
              .map((a) => a.hash.slice(1));
            let id = '';
            for (const i of ids) {
              if (document.getElementById(i).getBoundingClientRect().top <= line) id = i;
            }
            return '#' + id;
          });
          await p.mouse.wheel(0, 4800);
          await scrollSettled(p);
          seen.down = await lit(p);
          const want = holding(n);
          const ok = seen.landed.length === 1 && seen.landed[0] === want &&
            seen.up.length === 1 && seen.up[0] === seen.upLine &&
            seen.down.length === 1 && seen.down[0] !== want;
          if (ok) r.ok(`${where}: #L${n} lights ${want}, and it follows the scroll`);
          else r.fail(`${where}: want ${want}, got ${JSON.stringify(seen)}`);
        } catch (e) { r.fail(`${where}: ${e.message.split('\n')[0]}`); }
      }
    });
  }
  return { outlineMenu, symbols, outlineCurrent };
}
