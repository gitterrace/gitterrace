// The diff find bar. Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
//
// A repository with five uncommitted files. On /worktree, in each browser:
// - the bar is in the server's HTML and the page shifts by nothing while the scripts load
// - f focuses the box, "/" still belongs to global search
// - a query shows "n of m" after a step, n and N and Enter and Shift+Enter move it, and the
//   match ranges are painted without adding one element to the page
// - an empty box shows no status, a query with no hit says so
// - Hide viewed hides the marked files, and the viewed count and bar follow
// - a diff body takes focus (it scrolls) and a file name is a level 2 heading
// - at 390 the bar wraps inside itself
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { eachBrowser, serve, sh, commitAll, ready } from '../../lib/browser-lib.mjs';

const BOX = '.difftools input[type=search]';

function makeRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-difffind-'));
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  for (let i = 0; i < 5; i++) writeFileSync(join(dir, `f${i}.txt`), 'alpha\nbeta\n');
  commitAll(dir, 'init');
  for (let i = 0; i < 5; i++) {
    const body = `alpha\nbeta\nNeedle ${i} one\nplain ${i}\nneedle ${i} two needle\n`;
    writeFileSync(join(dir, `f${i}.txt`), body);
  }
  return dir;
}

/** @param {any} env what browser-check.mjs set up (r, browsers, bin, cfg, context) */
export function checks(env) {
  const { r, browsers, bin, cfg, context } = env;
  const say = (ok, good, bad) => (ok ? r.ok(good) : r.fail(bad));
  const status = (p) => p.$eval('.find-status', (e) => e.textContent.trim());
  const painted = (p) => p.evaluate(() => ['find', 'find-current']
    .reduce((n, k) => n + (CSS.highlights.get(k)?.size ?? 0), 0));
  const onBox = (p) => p.evaluate((q) => document.activeElement.matches(q), BOX);
  const count = (p) => p.evaluate(() => document.getElementsByTagName('*').length);

  /** Layout, placement and the accessibility of the markup, at one width. */
  async function shape(p, w) {
    const cls = await p.evaluate(() => /** @type {any} */ (window).__cls);
    say(cls === 0, `${w}: layout shift 0`, `${w}: layout shift ${cls}`);
    const wide = await p.evaluate(() => document.documentElement.scrollWidth > innerWidth);
    say(!wide, `${w}: no sideways scroll`, `${w}: the page scrolls sideways`);
    const gap = await p.evaluate(() => {
      const h = document.querySelector('section.file>header');
      const k = [...h.children];
      return k.indexOf(h.querySelector('.viewed-mark')) - k.indexOf(h.querySelector('.counts'));
    });
    say(gap === 1, `${w}: Viewed sits right after the counts`, `${w}: Viewed is ${gap} after`);
    const a = await p.evaluate(() => {
      const d = document.querySelector('.diffwrap');
      const h = document.querySelector('section.file>header');
      return { tab: /** @type {any} */ (d).tabIndex, label: d.getAttribute('aria-label'),
        role: h.getAttribute('role'), level: h.getAttribute('aria-level') };
    });
    say(a.tab === 0 && a.label && a.role === 'heading' && a.level === '2',
      `${w}: diff bodies take focus, file names are level 2 headings`,
      `${w}: ${JSON.stringify(a)}`);
  }

  /** The keyboard flow and the states, at 1440. */
  async function flow(p, w) {
    const before = await count(p);
    await p.keyboard.press('f');
    say(await onBox(p), `${w}: f focuses the find box`, `${w}: f did not focus the box`);
    say((await status(p)) === '', `${w}: an empty box shows no status`, `${w}: status shown`);
    await p.keyboard.type('needle');
    await p.waitForFunction(() => /lines? in/.test(
      document.querySelector('.find-status').textContent), null, { timeout: 10000 });
    const first = await status(p);
    say(first === '10 lines in 5 files', `${w}: ${first}`, `${w}: count "${first}"`);
    for (const [key, want] of [['Enter', '1 of'], ['Enter', '2 of'], ['Shift+Enter', '1 of']]) {
      await p.keyboard.press(key);
      const s = await status(p);
      say(s.startsWith(want), `${w}: ${key} gives "${s}"`, `${w}: ${key} gave "${s}"`);
    }
    const n = await painted(p);
    say(n >= 3, `${w}: ${n} ranges painted with CSS.highlights`, `${w}: ${n} ranges painted`);
    const after = await count(p);
    say(after === before, `${w}: a query adds no element (${after})`, `${w}: ${before}->${after}`);
    await p.evaluate(() => /** @type {any} */ (document.activeElement).blur());
    for (const [key, want] of [['n', '2 of'], ['N', '1 of']]) {
      await p.keyboard.press(key);
      const s = await status(p);
      say(s.startsWith(want), `${w}: ${key} outside the box gives "${s}"`, `${w}: ${key} "${s}"`);
    }
    await p.keyboard.press('Escape');
    const cleared = await p.waitForFunction(() => document.querySelector('.find-status')
      .textContent === '', null, { timeout: 5000 }).then(() => true, () => false);
    const left = await painted(p);
    say(cleared && left === 0, `${w}: Esc clears the query and the highlights`,
      `${w}: Esc left ${left} ranges, cleared ${cleared}`);
    await p.keyboard.press('/');
    say(!(await onBox(p)), `${w}: / stays global search`, `${w}: / went to the diff box`);
    await p.keyboard.press('Escape');
    await p.keyboard.press('f');
    await p.keyboard.type('zzzzqqq');
    const none = await p.waitForFunction(() => document.querySelector('.find-status')
      .textContent === 'No match', null, { timeout: 10000 }).then(() => true, () => false);
    say(none, `${w}: no match says so`, `${w}: no "No match" state`);
    await p.keyboard.press('Escape');
  }

  async function hideViewed(p, w) {
    const all = await p.$$eval('section.file', (s) => s.length);
    await p.locator('section.file input.viewed').first().check();
    await p.locator('.dt-hide-box').check();
    const shown = await p.$$eval('section.file',
      (s) => s.filter((x) => x.getBoundingClientRect().height > 0).length);
    const txt = await p.$eval('.difftools .viewed-count', (e) => e.textContent);
    const val = await p.$eval('.dt-bar', (e) => e.value);
    say(shown === all - 1 && txt === `Viewed 1 of ${all}` && val === 1,
      `${w}: Hide viewed hides one of ${all}, "${txt}"`, `${w}: shown ${shown}, "${txt}", ${val}`);
  }

  async function diffFind() {
    const dir = makeRepo();
    const srv = await serve(bin, dir, cfg);
    try {
      await eachBrowser(browsers, context, async (ctx, name) => {
        // This server's owner cookie: the working tree page keeps viewed marks through it.
        await ctx.addCookies([await srv.owner()]);
        const where = `diff-find ${name}`;
        const raw = await (await fetch(`${srv.base}/worktree`)).text();
        say(/<form class="difftools"/.test(raw), `${where}: the bar is server rendered`,
          `${where}: no .difftools in the HTML`);
        for (const width of [1440, 390]) {
          const p = await ctx.newPage({ viewport: { width, height: 900 } });
          await p.addInitScript(() => {
            /** @type {any} */ (window).__cls = 0;
            try {
              new PerformanceObserver((l) => {
                for (const e of /** @type {any[]} */ (l.getEntries())) {
                  if (!e.hadRecentInput) /** @type {any} */ (window).__cls += e.value;
                }
              }).observe({ type: 'layout-shift', buffered: true });
            } catch { /* no layout-shift entries in this engine */ }
          });
          await p.goto(`${srv.base}/worktree`, { waitUntil: 'load' });
          await p.waitForSelector('section.file input.viewed', { timeout: 10000 });
          // The find keys (f, n, /) are bound by the lazy script, which loads apart from viewed.
          await ready(p, 'diffs-find');
          // Layout shifts land after fonts swap in, one frame past paint.
          await p.evaluate(() => document.fonts.ready.then(() => new Promise((r) =>
            requestAnimationFrame(() => requestAnimationFrame(r)))));
          const w = `${where} at ${width}`;
          await shape(p, w);
          if (width === 1440) {
            await flow(p, w);
            await hideViewed(p, w);
          } else {
            const inside = await p.evaluate(() => {
              const f = document.querySelector('.difftools').getBoundingClientRect();
              return [...document.querySelectorAll('.difftools *')].every((e) => {
                const b = e.getBoundingClientRect();
                return !b.width || (b.left >= f.left - 1 && b.right <= f.right + 1);
              });
            });
            say(inside, `${w}: the bar wraps inside itself`, `${w}: the bar overflows`);
          }
          await p.close();
        }
      });
    } catch (e) {
      r.fail(`diff-find: ${e && e.stack}`);
    } finally {
      srv.stop();
      rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    }
  }
  return { diffFind };
}
