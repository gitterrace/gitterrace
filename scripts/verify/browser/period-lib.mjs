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

  // Dragging over the lines-of-code chart sets the range and redraws the chart on just that
  // span: the plotted commits run from the first dragged position to the last.
  async function rangeGraph() {
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      const where = `range-graph ${name}`;
      const p = await ctx.newPage();
      await p.setViewportSize({ width: 1400, height: 700 });
      await p.goto(srv.base + '/stats/languages', { waitUntil: 'load' });
      const data = () => JSON.parse(document.getElementById('loc-data').textContent);
      const before = await p.evaluate(data);
      const box = await p.locator('#loc').boundingBox();
      if (!box || before.i.length < 3) {
        r.fail(`${where}: no chart with enough commits (${before.i.length})`);
        await b.close();
        continue;
      }
      const y = box.y + box.height / 2;
      await p.mouse.move(box.x + box.width * 0.5, y);
      await p.mouse.down();
      await p.mouse.move(box.x + box.width * 0.95, y, { steps: 5 });
      await Promise.all([p.waitForNavigation({ waitUntil: 'load' }), p.mouse.up()]);
      const got = await p.evaluate(() => {
        const q = new URL(location.href).searchParams;
        const d = JSON.parse(document.getElementById('loc-data').textContent);
        const at = d.i.length - 1;
        return {
          from: +q.get('from'), to: +q.get('to'), first: d.i[0], last: d.i[at],
          x0: d.x[0], x1: d.x[at],
        };
      });
      const shorter = got.last - got.first < before.i[before.i.length - 1];
      if (got.first === got.from && got.last === got.to && shorter) {
        r.ok(`${where}: the chart draws positions ${got.from} to ${got.to}`);
      } else {
        r.fail(`${where}: ${JSON.stringify(got)} does not fit the dragged range`);
      }
      // The axis spans the plot again, not a slice of the old one.
      if (got.x1 - got.x0 > 800) r.ok(`${where}: the range fills the plot`);
      else r.fail(`${where}: the range spans ${got.x1 - got.x0}px of the plot`);
      await p.close();
      await b.close();
    }
  }

  return { periodClick, rangeGraph };
}
