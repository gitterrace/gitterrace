// The coupled-files section of the hotspots page. Imported by browser-check.mjs, not run alone.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { eachBrowser, commitAll, poll } from '../../lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (dir, srv, r, browsers, context, ...) */
export function checks(env) {
  const { dir, srv, r, browsers, context } = env;

  // Files that change together: a sibling pair and a pair across directories, three commits
  // each. The same-directory list shows the sibling only, the other list adds the cross pair,
  // and the partner link of a row opens the file's history card page.
  async function couplingToggle() {
    for (const d of ['pair', 'other']) mkdirSync(join(dir, d), { recursive: true });
    for (let n = 0; n < 3; n++) {
      for (const f of ['pair/x.txt', 'pair/z.txt', 'other/y.txt']) {
        writeFileSync(join(dir, f), `${n}\n`);
      }
      commitAll(dir, `fix(pair): together ${n}`);
      writeFileSync(join(dir, 'pair/x.txt'), `again ${n}\n`);
      writeFileSync(join(dir, 'other/y.txt'), `again ${n}\n`);
      commitAll(dir, `fix(other): across ${n}`);
    }
    await eachBrowser(browsers, context, async (ctx, name) => {
      const where = `coupling-toggle ${name}`;
      const p = await ctx.newPage();
      await p.setViewportSize({ width: 1400, height: 800 });
      const sel = '#coupled ~ .card table.pair tr[data-path]';
      const rows = () => p.$$eval(sel, (t) => t.map((x) => x.textContent));
      let same = [];
      await poll(async () => {
        await p.goto(`${srv.base}/stats/hotspots?period=all`, { waitUntil: 'load' });
        same = await rows();
        return same.some((t) => t.includes('z.txt'));
      }, 'the sibling pair');
      if (same.some((t) => t.includes('y.txt'))) r.fail(`${where}: sibling list has a cross pair`);
      const across = p.locator('.segctl[aria-label="Pairs"] a', { hasText: 'Across' });
      if (!(await across.count())) {
        r.fail(`${where}: no cross-directory switch`);
        await p.close();
        return;
      }
      await Promise.all([
        p.waitForNavigation({ waitUntil: 'load' }),
        across.first().click(),
      ]);
      const all = await rows();
      const bad = [];
      if (!all.some((t) => t.includes('y.txt'))) bad.push('no cross-directory pair');
      if (all.length <= same.length) bad.push(`rows ${same.length} then ${all.length}`);
      const link = p.locator(`${sel} td a`).first();
      const href = await link.getAttribute('href');
      const opens = href && (href.includes('/history/') || href.includes('/blob/'));
      if (!opens) bad.push(`partner link ${href}`);
      r.verdict(where, bad, `${same.length} rows, then ${all.length} across, partner link opens`);
      await p.close();
    });
  }

  return { couplingToggle };
}
