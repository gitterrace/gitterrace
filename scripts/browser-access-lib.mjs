// Reading and keyboard checks: highlighted text, the skip link and tab stops. Each runs in every
// browser, light and dark, at 1400 and 900 px.
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

  // The first Tab on any page lands on a skip link to the content, which moves focus into
  // <main>; a chart is one tab stop, never one per bar or dot.
  // macOS WebKit skips links on a plain Tab unless the user turned that on, so there Alt+Tab is
  // the keyboard move that reaches every control, as in Safari with the default setting.
  const tab = (p) => p.keyboard.press(p.context().browser()?.browserType().name() === 'webkit' && process.platform === 'darwin' ? 'Alt+Tab' : 'Tab');
  async function tabStops() {
    for (const path of ['/', '/stats', '/stats/hotspots', '/log']) {
      await each(`tab-stops ${path}`, path, async (p) => {
        for (let i = 0; i < 40 && path !== '/' && !(await p.$('svg.chart')); i++) { await p.waitForTimeout(250); await p.reload(); }
        const bad = [];
        await tab(p);
        const first = await p.evaluate(() => {
          const a = document.activeElement;
          return a && { text: a.textContent.trim(), href: a.getAttribute('href'), seen: a.getBoundingClientRect().height > 1 };
        });
        if (!first || first.href !== '#main') bad.push(`first stop is "${first?.text}", not a skip link`);
        else {
          if (!first.seen) bad.push('skip link not visible on focus');
          await p.keyboard.press('Enter');
          await tab(p);
          const inMain = await p.evaluate(() => !!document.activeElement?.closest('main'));
          if (!inMain) bad.push('Tab after the skip link leaves main');
        }
        // The chart itself may be the one stop; no mark inside it may add another.
        const stops = await p.evaluate(() => [...document.querySelectorAll('svg')].map((s) => {
          const f = s.querySelectorAll('a[href],[tabindex]');
          return 1 + [...f].filter((e) => /** @type {HTMLElement} */ (e).tabIndex >= 0).length - (s.tabIndex >= 0 ? 0 : 1);
        }).filter((n) => n > 1));
        if (stops.length) bad.push(`a chart with ${Math.max(...stops)} tab stops`);
        const charts = await p.$$eval('svg.chart', (l) => l.length);
        if (path !== '/' && path !== '/log' && !charts) bad.push('no chart on the page');
        return bad;
      });
    }
  }

  return { markFlat, tabStops };
}
