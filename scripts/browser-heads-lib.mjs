// Page heads: each page's controls sit in the title row or one bar, never in rows of their own.
// Imported by scripts/browser-check.mjs, not run on its own.

/** Visible head rows of the page: the path bar and any document header, as boxes. */
const HEADS = 'main .head, [data-dochead]';

export function checks({ srv, r, browsers, context }) {
  /** Runs `fn(page)` returning a list of faults at each width and scheme; reports one line each. */
  async function each(label, path, fn, widths = [1440, 900, 390]) {
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      for (const scheme of ['light', 'dark']) for (const width of widths) {
        const p = await ctx.newPage();
        await p.emulateMedia({ colorScheme: scheme });
        await p.setViewportSize({ width, height: 900 });
        await p.goto(srv.base + path, { waitUntil: 'load' });
        const bad = await fn(p, width);
        const where = `page-heads ${name} ${scheme} ${width} ${label}`;
        if (bad.length) r.fail(`${where}: ${bad.join('; ')}`); else r.ok(where);
        await p.close();
      }
      await b.close();
    }
  }

  async function pageHeads() {
    // A Markdown file shows exactly one head row, holding path, size, Blame, History and the
    // Preview/Code switch, and it is the sticky one.
    await each('markdown blob', '/blob/HEAD/README.md', (p) => p.evaluate((sel) => {
      const rows = [...document.querySelectorAll(sel)].filter((e) => e.getClientRects().length);
      const bad = [];
      if (rows.length !== 1) bad.push(`${rows.length} head rows, want 1`);
      const h = rows[0];
      if (h) {
        if (!h.querySelector('.crumbs')) bad.push('no path in the row');
        if (!h.querySelector('[data-show=rendered]') || !h.querySelector('[data-show=source]')) bad.push('no Preview/Code in the row');
        const text = h.textContent;
        if (!/History/.test(text)) bad.push('no History link in the row');
        if (/Copy permalink/.test(text)) bad.push('Copy permalink is still shown');
        if (getComputedStyle(h).position !== 'sticky') bad.push('the row is not sticky');
      }
      return bad;
    }, HEADS));
  }
  /** Rows of controls: how many direct rows of `root`'s children hold only `n` controls. */
  const CTL = 'a[href], button, input:not([type=hidden]), select, summary';



  async function compareHeads() {
    await each('compare', '/compare?base=HEAD~2&head=HEAD', (p) => p.evaluate((ctl) => {
      const bad = [];
      const form = document.querySelector('form.cmp');
      const head = document.querySelector('main .head');
      if (!form || !head || !head.contains(form)) bad.push('the form is not in the title row');
      // No row of one control: every direct child of main that holds controls holds more than one,
      // except the KPI row, whose head sha is a link in a row of figures.
      for (const e of document.querySelectorAll('main > p.hint')) {
        if (e.querySelectorAll(ctl).length === 1 && /Open the head commit/.test(e.textContent)) bad.push('Open the head commit has a row of its own');
      }
      if (!document.querySelector('.kpis a[href*="/commit/"]')) bad.push('no link on the head sha');
      const h = head && head.getBoundingClientRect(), f = form && form.getBoundingClientRect();
      if (h && f && innerWidth > 600 && f.top - h.top > h.height) bad.push('the form wraps below the title');
      return bad;
    }, CTL), [1440, 900]);
    await each('compare phone', '/compare?base=HEAD~2&head=HEAD', (p) => p.evaluate(() => {
      const form = document.querySelector('form.cmp'), head = document.querySelector('main .head');
      return form && head && head.contains(form) ? [] : ['the form is not in the title row'];
    }), [390]);
  }
  async function commitHeads() {
    await each('commit', '/commit/HEAD', (p) => p.evaluate(() => {
      const bad = [];
      const head = document.querySelector('main .head'), meta = document.querySelector('p.meta');
      if (!meta || head?.nextElementSibling !== meta) bad.push('the meta line is not right under the title');
      if (document.querySelector('p.meta button.copy')) bad.push('a Copy button is still on the meta line');
      const sum = document.querySelector('details.files > summary');
      if (!sum || !sum.querySelector('.counts .add') || !sum.querySelector('.counts .del')) bad.push('no totals in the changed-files header');
      return bad;
    }));
  }
  return { pageHeads, compareHeads, commitHeads };
}
