// Control rows above a page's content: a row that holds one control and leaves most of its
// width empty reads as a stray and pushes the content down. Imported by
// scripts/browser-check.mjs, not run on its own.

// Pages red today, each with the change that fixes it, in words. A page listed here that is
// green at every width and in every browser fails the check, so the list can only shrink.
const KNOWN = {
  'insights/activity': 'the insights toolbar row',
  'insights/contributors': 'the insights toolbar row',
  'insights/hotspots': 'the insights toolbar row',
  'insights/ownership': 'the insights toolbar row',
  compare: 'the compare form in the title row',
  refs: 'the branches and tags page on one page',
};

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context } = env;
  // Every kind of page the golden set renders. The insights tabs are read from the tab row, so
  // a new tab is walked without touching this list.
  const fixed = {
    overview: '/', tree: '/tree/HEAD/src', blob: '/blob/HEAD/a.txt',
    'blob-md': '/blob/HEAD/README.md', log: '/log/HEAD', commit: '/commit/HEAD',
    compare: '/compare', search: '/search?q=one', refs: '/refs',
  };
  async function pages(p) {
    await p.goto(srv.base + '/stats', { waitUntil: 'load' });
    const tabs = await p.$$eval('main .tabs a', (as) => as.map((a) => a.getAttribute('href')));
    const out = { ...fixed };
    for (const h of new Set(tabs)) {
      const path = new URL(h, srv.base + '/').pathname.replace(/^\/r\/[^/]+/, '');
      out[`insights${path.replace(/^\/stats/, '') || '/activity'}`] = path;
    }
    return out;
  }
  async function loneRows() {
    /** @type {Map<string, string[]>} page key to its findings, over every browser and width */
    const red = new Map();
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      const p = await ctx.newPage();
      const all = await pages(p);
      for (const width of [1440, 390]) for (const [key, path] of Object.entries(all)) {
        await p.setViewportSize({ width, height: 900 });
        await p.goto(srv.base + path, { waitUntil: 'load' });
        await p.evaluate(() => document.fonts.ready);
        const lone = await p.evaluate(rowsAboveContent);
        if (!red.has(key)) red.set(key, []);
        const where = `lone-rows ${name} ${width} ${key}`;
        if (!lone.length) r.ok(`${where}: no lone control row above the content`);
        else if (KNOWN[key]) r.ok(`${where}: known, ${KNOWN[key]} fixes it (${lone[0]})`);
        else r.fail(`${where}: ${lone.join('; ')}`);
        red.get(key).push(...lone);
      }
      await b.close();
    }
    for (const [key, row] of Object.entries(KNOWN)) {
      if (!red.has(key)) r.fail(`lone-rows: KNOWN names ${key}, which is not walked`);
      else if (!red.get(key).length) r.fail(`lone-rows ${key}: green now, drop it (${row})`);
    }
  }
  return { loneRows };
}

// Runs in the page. Walks main from the top down to the first content block. A row is the
// first box of at most 72 px that holds a control; its controls count as units (a segmented
// control, a menu, a link or a field is one). A row with one unit using under 40% of its width
// is reported.
function rowsAboveContent() {
  const CONTENT = '.card, table, pre, .chart-head, .kpis, .markdown-body, .empty, .diff, '
    + 'article, svg';
  const UNIT = '[role=group], details, a, button, input, select, summary, [role=tab]';
  const shown = (e) => {
    const q = e.getBoundingClientRect();
    return q.width > 0 && q.height > 0 && getComputedStyle(e).visibility !== 'hidden';
  };
  const units = (row) => [...row.querySelectorAll(UNIT)]
    .filter((e) => shown(e) && !e.parentElement.closest(UNIT));
  const label = (e) => (e.innerText || e.getAttribute('aria-label') || '')
    .trim().replace(/\s+/g, ' ').slice(0, 40);
  const out = [];
  let done = false;
  const walk = (el) => {
    for (const c of el.children) {
      if (done) return;
      if (!shown(c)) continue;
      if (c.matches(CONTENT)) { done = true; return; }
      const u = units(c);
      if (!u.length) { if (c.querySelector(CONTENT)) walk(c); continue; }
      if (c.getBoundingClientRect().height > 72) { walk(c); continue; }
      if (u.length !== 1) continue;
      const share = u[0].getBoundingClientRect().width / c.getBoundingClientRect().width;
      const box = `${c.tagName.toLowerCase()}.${String(c.className || '').split(' ')[0]}`;
      const pct = Math.round(share * 100);
      if (share < 0.4) out.push(`"${label(u[0])}" alone in ${box}, ${pct}% of the row`);
    }
  };
  const m = document.querySelector('main');
  if (m) walk(m);
  return out;
}
