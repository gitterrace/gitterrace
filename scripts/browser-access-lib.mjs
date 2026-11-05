// Reading checks: how highlighted text looks. Each runs in every browser, light and dark, at 1400 and 900 px.
// Imported by scripts/browser-check.mjs, not run on its own.

const SCHEMES = ['light', 'dark'];
const WIDTHS = [1400, 900];

export function checks({ srv, r, browsers, context }) {
  /** Runs `fn(page, where)` on `path` for every browser, scheme and width. */
  async function each(label, path, fn) {
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      for (const scheme of SCHEMES) for (const width of WIDTHS) {
        const p = await ctx.newPage();
        await p.emulateMedia({ colorScheme: scheme });
        await p.setViewportSize({ width, height: 800 });
        await p.goto(srv.base + path, { waitUntil: 'load' });
        const where = `${label} ${name} ${scheme} ${width}`;
        const bad = await fn(p);
        bad.length ? r.fail(`${where}: ${bad.join('; ')}`) : r.ok(where);
        await p.close();
      }
      await b.close();
    }
  }

  // Every highlight of text (a search match, any <mark>) is one background: no outline, border,
  // ring shadow or padding, so it takes exactly the width of its letters and never looks like a
  // box cut out of the word around it.
  async function markFlat() {
    await each('mark-flat', '/search?q=fn+ma', async (p) => p.evaluate(() => {
      const ms = [...document.querySelectorAll('mark')];
      if (!ms.length) return ['no highlighted match'];
      const bad = new Set();
      for (const m of ms) {
        const s = getComputedStyle(m);
        if (s.boxShadow !== 'none') bad.add(`ring shadow ${s.boxShadow}`);
        if (s.outlineStyle !== 'none' && parseFloat(s.outlineWidth) > 0) bad.add('outline');
        for (const k of ['Top', 'Right', 'Bottom', 'Left']) {
          if (parseFloat(s[`border${k}Width`]) > 0 && s[`border${k}Style`] !== 'none') bad.add(`border ${k}`);
          if (parseFloat(s[`padding${k}`]) > 0) bad.add(`padding ${k}`);
        }
        const plain = document.createElement('span');
        plain.textContent = m.textContent;
        m.after(plain);
        const dw = Math.abs(m.getBoundingClientRect().width - plain.getBoundingClientRect().width);
        plain.remove();
        if (dw > 0.5) bad.add(`${dw.toFixed(1)} px wider than its letters`);
        if (s.backgroundColor === 'rgba(0, 0, 0, 0)') bad.add('no background');
      }
      return [...bad];
    }));
  }

  return { markFlat };
}
