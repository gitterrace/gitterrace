// Reading and keyboard checks: highlighted text, the skip link and tab stops, the shortcut
// sheet and the overview sidebar order. Each runs in every browser, light and dark, at 1400
// and 900 px. Imported by scripts/browser-check.mjs, not run on its own.

const SCHEMES = ['light', 'dark'];
const WIDTHS = [1400, 900];

export function checks({ dir, srv, r, browsers, context }) {
  /** Runs `fn(page)`, which returns its failures, on `path` in every browser, scheme, width. */
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

  /** Reloads `p` until `sel` is on the page: insights parts appear once the index is built. */
  async function until(p, sel) {
    for (let i = 0; i < 40 && !(await p.$(sel)); i++) {
      await p.waitForTimeout(250);
      await p.reload();
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
          const border = parseFloat(s[`border${k}Width`]) > 0 && s[`border${k}Style`] !== 'none';
          if (border) bad.add(`border ${k}`);
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

  // The first Tab on any page lands on a visible skip link whose Enter moves the next Tab into
  // <main>; a chart is one tab stop, never one per bar or dot.
  // macOS WebKit skips links on a plain Tab unless the user turned that on, so there Alt+Tab is
  // the keyboard move that reaches every control, as in Safari with the default setting.
  const tab = (p) => p.keyboard.press(p.context().browser()?.browserType().name() === 'webkit' && process.platform === 'darwin' ? 'Alt+Tab' : 'Tab');
  async function tabStops() {
    for (const path of ['/', '/stats', '/stats/hotspots', '/log']) {
      const charted = path.startsWith('/stats');
      await each(`tab-stops ${path}`, path, async (p) => {
        if (charted) await until(p, 'svg.chart');
        const bad = [];
        await tab(p);
        const first = await p.evaluate(() => {
          const a = document.activeElement;
          if (!a) return null;
          const seen = a.getBoundingClientRect().bottom > 0;
          return { text: a.textContent.trim(), href: a.getAttribute('href'), seen };
        });
        if (!first || first.href !== '#main') {
          bad.push(`first stop is "${first?.text}", not a skip link`);
        } else {
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
        if (charted && !(await p.$('svg.chart'))) bad.push('no chart on the page');
        return bad;
      });
    }
  }

  // In the shortcut sheet the keys all start at one x, in a column at least 64 px wide, and the
  // descriptions all start at one x. A chord of several keys reads as one unit: no two keys
  // stand bare side by side (a muted word such as "then" joins them), and its parts sit closer
  // to each other than the key column sits to its description.
  async function helpKeys() {
    await each('help-keys', '/', async (p) => {
      await p.keyboard.press('?');
      await p.waitForSelector('.keys dt');
      return p.evaluate(() => {
        const dts = [...document.querySelectorAll('.keys dt')];
        const dds = [...document.querySelectorAll('.keys dd')];
        const box = (e) => e.getBoundingClientRect();
        const xs = (els) => [...new Set(els.map((e) => Math.round(box(e).left)))];
        const bad = [];
        if (xs(dds).length !== 1) bad.push(`descriptions start at ${xs(dds).join(', ')} px`);
        if (xs(dts).length !== 1) bad.push(`keys start at ${xs(dts).join(', ')} px`);
        const colW = box(dds[0]).left - box(dts[0]).left;
        if (colW < 64) bad.push(`key column ${colW.toFixed(0)} px`);
        const chords = dts.filter((d) => d.querySelectorAll('kbd').length > 1);
        if (!chords.length) bad.push('no chord of several keys');
        const col = box(dds[0]).left - Math.max(...dts.map((d) => box(d).right));
        for (const d of chords) {
          const parts = [...d.children], name = `"${d.textContent.trim()}"`;
          for (let i = 1; i < parts.length; i++) {
            const bare = parts[i].tagName === 'KBD' && parts[i - 1].tagName === 'KBD';
            if (bare) bad.push(`${name}: two bare keys side by side`);
            const gap = box(parts[i]).left - box(parts[i - 1]).right;
            if (gap >= col) bad.push(`${name}: parts ${gap.toFixed(0)} px apart, column ${col}`);
          }
        }
        return [...new Set(bad)];
      });
    });
  }

  // On the overview the sidebar reads Now, Loose ends, Recent: what is left open comes before
  // the activity numbers.
  async function sideOrder() {
    const { sh } = await import('./browser-lib.mjs');
    sh(dir, 'git', 'checkout', '-q', '-b', 'side-order');
    sh(dir, 'git', 'commit', '-q', '--allow-empty', '-m', 'Work left on a branch');
    sh(dir, 'git', 'checkout', '-q', 'main');
    await each('side-order', '/', async (p) => {
      await until(p, '.side .sidelist');
      return p.evaluate(() => {
        const hs = [...document.querySelectorAll('aside .side h3')]
          .map((h) => h.firstChild.textContent.trim());
        const at = (t) => hs.indexOf(t);
        if (at('Loose ends') < 0) return [`no Loose ends in ${hs.join(', ')}`];
        const ok = at('Now') < at('Loose ends') && at('Loose ends') < at('Recent');
        return ok ? [] : [`order ${hs.join(', ')}`];
      });
    });
  }

  return { markFlat, tabStops, helpKeys, sideOrder };
}
