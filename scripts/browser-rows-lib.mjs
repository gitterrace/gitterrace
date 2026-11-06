// Control rows above a page's content: a row that holds one control and leaves most of its
// width empty reads as a stray and pushes the content down. Imported by
// scripts/browser-check.mjs, not run on its own.

// Pages red today, each with the change that fixes it, in words. A page listed here that is
// green at every width and in every browser fails the check, so the list can only shrink.
const KNOWN = {
  compare: 'one branch link alone in the Against a branch hint, a hint row that wraps its links',
  search: 'the Regex and Match case boxes each alone at 390, one options row that wraps as a unit',
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
  // Small layout: the tree's '..' row shows only at narrow widths (the breadcrumb does its job on
  // wide ones), the sidebar's insights link sits in the Languages heading, and the nav names
  // the refs tab 'Branches' at 390.
  async function smallLayout() {
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      const p = await ctx.newPage();
      /** @type {Array<[number, boolean, string]>} */
      const cases = [[1440, false, 'Branches & Tags'], [390, true, 'Branches']];
      for (const [width, up, label] of cases) {
        await p.setViewportSize({ width, height: 900 });
        await p.goto(srv.base + '/tree/HEAD/src', { waitUntil: 'load' });
        const row = await p.$('tr.up a');
        const seen = row ? await row.isVisible() : false;
        const where = `small-layout ${name} ${width}`;
        if (seen === up) r.ok(`${where}: '..' row ${up ? 'shown' : 'hidden'}`);
        else r.fail(`${where}: '..' row visible=${seen}, want ${up}`);
        // The visible label: text of a part clipped to 1px (kept for the accessible name) is not
        // shown, so it is left out here and checked through the accessible name below.
        const nav = await p.$eval('.tabnav a[href$="/refs"]', (a) => {
          const c = /** @type {HTMLElement} */ (a.cloneNode(true));
          for (const m of Array.from(a.querySelectorAll('*'))) {
            const s = getComputedStyle(m);
            const hidden = s.display === 'none' || (s.position === 'absolute' && m.getBoundingClientRect().width <= 1);
            if (hidden) c.querySelector(`.${Array.from(m.classList).join('.')}`)?.remove();
          }
          return (c.textContent || '').replace(/\s+/g, ' ').trim().toLowerCase();
        });
        if (nav === label.toLowerCase()) r.ok(`${where}: nav says ${label}`);
        else r.fail(`${where}: nav says '${nav}', want '${label}'`);
        // Hidden text stays in the accessible name: a narrow nav still reads 'Branches & Tags'.
        const named = await p.getByRole('link', { name: 'Branches & Tags', exact: true }).count();
        if (named === 1) r.ok(`${where}: refs tab is named Branches & Tags`);
        else r.fail(`${where}: refs tab accessible name lost '& Tags' (${named} matches)`);
        await p.goto(srv.base + '/', { waitUntil: 'load' });
        const text = await p.evaluate(() => document.body.innerText);
        const inHead = await p.$('aside h3 a.more');
        if (/View insights/.test(text)) r.fail(`${where}: a lone 'View insights' link remains`);
        else if (!inHead) r.fail(`${where}: no insights link in the Languages heading`);
        else r.ok(`${where}: insights link sits in the Languages heading`);
      }
      await b.close();
    }
  }
  return { loneRows, smallLayout };
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
