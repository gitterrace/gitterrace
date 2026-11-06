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
  return { pageHeads };
}
