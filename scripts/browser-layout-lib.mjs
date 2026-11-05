// Layout checks of the insights and commit pages: inline controls share one size, section heads
// and keep room above them. Imported by
// scripts/browser-check.mjs, not run on its own.
/** @param {any} env what browser-check.mjs set up (dir, srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context } = env;
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
  return { inlineControls, sectionGaps };
}
