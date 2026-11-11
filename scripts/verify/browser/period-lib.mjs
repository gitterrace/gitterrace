// The period picker of the insights pages. Imported by browser-check.mjs, not run alone.

const PAGES = [
  '/stats',
  '/stats/hotspots',
  '/stats/ownership',
  '/stats/contributors',
  '/stats/languages',
];

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context } = env;

  // Each insights page, opened unfiltered first as the warm pass serves it: clicking Week
  // lands with Week as the current period. A link form the page or its memo ignores lands
  // on All.
  async function periodClick() {
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      for (const page of PAGES) {
        const where = `period-click ${name} ${page}`;
        const p = await ctx.newPage();
        await p.setViewportSize({ width: 1400, height: 700 });
        await p.goto(srv.base + page, { waitUntil: 'load' });
        const week = p.locator('.segctl[aria-label="Period"] a', { hasText: 'Week' });
        if (!(await week.count())) {
          r.fail(`${where}: no Week link`);
        } else {
          await Promise.all([
            p.waitForNavigation({ waitUntil: 'load' }),
            week.first().click(),
          ]);
          const on = await p.evaluate(() => document
            .querySelector('.segctl[aria-label="Period"] a[aria-current]')?.textContent);
          if (on === 'Week') r.ok(`${where}: Week is selected`);
          else r.fail(`${where}: ${on || 'nothing'} is selected after clicking Week`);
        }
        await p.close();
      }
      await b.close();
    }
  }

  return { periodClick };
}
