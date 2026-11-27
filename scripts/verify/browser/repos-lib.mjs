// The filter on the repositories home. Imported by browser-check.mjs, not run alone.
import { basename } from 'node:path';
import { eachBrowser, sh, makeRepo, serve, poll } from '../../lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, dir, ...) */
export function checks(env) {
  const { r, browsers, context, dir, bin, cfg } = env;

  // Three repositories on one server: typing narrows the list and the count, Enter opens the
  // first match, Escape clears, and the card keeps its place while all of it happens.
  async function reposFilter() {
    const others = [makeRepo(), makeRepo()];
    const one = await serve(bin, dir, cfg);
    for (const o of others) {
      one.cli(o, o);
    }
    const count = async () => (await (await fetch(one.origin + '/api/repos')).json()).length;
    await poll(async () => (await count()) >= 3, 'three repositories');
    const target = basename(others[0]);
    try {
      await eachBrowser(browsers, context, async (ctx, name) => {
        const p = await ctx.newPage();
        for (const width of [1440, 390]) {
          const where = `repos-filter ${name} ${width}`;
          try {
            await p.setViewportSize({ width, height: 800 });
            await p.goto(one.origin + '/', { waitUntil: 'load' });
            await p.evaluate(() => document.fonts.ready);
            const box = () => p.evaluate(() => {
              const c = document.querySelector('.repolist').getBoundingClientRect();
              const f = document.querySelector('.repofilter').getBoundingClientRect();
              const de = document.documentElement;
              return {
                x: c.x, y: c.y, w: c.width, fy: f.y, fh: f.height,
                sw: de.scrollWidth - innerWidth,
              };
            });
            const count = () => p.textContent('#repocount');
            const visible = () => p.evaluate(() => {
              const rows = [...document.querySelectorAll('.reporow')];
              return rows.filter((l) => l.getClientRects().length).length;
            });
            const before = await box();
            const total = await visible();
            await p.keyboard.press('/');
            const focused = await p.evaluate(() => document.activeElement?.id === 'repoq');
            await p.keyboard.type(target);
            const typed = { count: await count(), rows: await visible(), shot: await box() };
            const label = await p.evaluate(() => {
              const q = document.getElementById('repoq');
              const c = document.getElementById('repocount');
              const l = document.querySelector('label[for=repoq]');
              return { named: l?.textContent, live: c.getAttribute('aria-live') };
            });
            await p.keyboard.press('Escape');
            const cleared = {
              count: await count(), rows: await visible(),
              value: await p.inputValue('#repoq'), shot: await box(),
            };
            await p.keyboard.type('zzzz-none');
            const none = await p.evaluate(() => {
              const e = document.querySelector('.repos-none');
              return !!e && e.getClientRects().length > 0;
            });
            await p.fill('#repoq', target);
            await Promise.all([p.waitForURL(/\/r\//), p.keyboard.press('Enter')]);
            const opened = p.url().toLowerCase().includes('/r/' + target.toLowerCase());
            const near = (a, c) => Math.abs(a - c) < 1;
            const still = (a, c) => near(a.x, c.x) && near(a.w, c.w) && near(a.fy, c.fy)
              && near(a.fh, c.fh) && c.sw <= 0;
            const ok = total === 3 && focused && typed.count === '1 of 3' && typed.rows === 1
              && cleared.count === '3 of 3' && cleared.rows === 3 && cleared.value === ''
              && none && opened && label.named && label.live === 'polite'
              && still(before, typed.shot) && still(before, cleared.shot);
            if (ok) r.ok(`${where}: types, counts, Escape clears, Enter opens, no shift`);
            else {
              const got = { total, focused, typed, cleared, none, opened, label, before };
              r.fail(`${where}: ${JSON.stringify(got)}`);
            }
          } catch (e) { r.fail(`${where}: ${e.message.split('\n')[0]}`); }
        }
      });
    } finally { one.stop(); }
  }
  return { reposFilter };
}
