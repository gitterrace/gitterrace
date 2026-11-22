// Reading and keyboard checks: highlighted text, the skip link and tab stops, the shortcut
// sheet and the overview sidebar order. Each runs in every browser, light and dark, at 1400
// and 900 px. Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
import { createRequire } from 'node:module';
import { writeFileSync } from 'node:fs';
import { root, sh, sweep, reloadUntil, synced } from '../../lib/browser-lib.mjs';

const SCHEMES = ['light', 'dark'];
const WIDTHS = [1400, 900];

// axe-core rules still failing, keyed "page rule" (a page key from `golden()`, an axe rule id).
// The list can only shrink: an entry whose page passes that rule now fails the check, so
// remove it then. A new page or rule that fails is never added here, it is fixed.
const KNOWN_VIOLATIONS = new Set([]);
const AXE_TAGS = ['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'];
const AXE_WIDTHS = [360, 1440];

/** The page kinds the golden set renders, as paths in the test repository. */
async function golden(p, base) {
  const fixed = {
    overview: '/', tree: '/tree/HEAD/src', blob: '/blob/HEAD/a.txt',
    'blob-md': '/blob/HEAD/README.md', log: '/log/HEAD', commit: '/commit/HEAD',
    compare: '/compare', search: '/search?q=one', refs: '/refs', settings: '/settings',
  };
  await p.goto(base + '/stats', { waitUntil: 'load' });
  const tabs = await p.$$eval('main .tabs a', (as) => as.map((a) => a.getAttribute('href')));
  const out = { ...fixed };
  for (const h of new Set(tabs)) {
    const path = new URL(h, base + '/').pathname.replace(/^\/r\/[^/]+/, '');
    out[`insights${path.replace(/^\/stats/, '') || '/activity'}`] = path;
  }
  return out;
}

export function checks({ dir, srv, r, browsers, context }) {
  /** Runs `fn(page)`, which returns its failures, on `path` in every browser, scheme, width. */
  async function each(label, path, fn) {
    const sw = { browsers, context, widths: WIDTHS, height: 800 };
    await sweep(sw, async (p, { name, scheme, width }) => {
      await p.goto(srv.base + path, { waitUntil: 'load' });
      r.verdict(`${label} ${name} ${scheme} ${width}`, await fn(p));
    });
  }

  /** Reloads `p` until `sel` is on the page: insights parts appear once the index is built. */
  async function until(p, sel) {
    // Some pages have no such part at all: the checks that follow report what they find.
    const there = async () => !!(await p.$(sel));
    await reloadUntil(p, there, null, `${sel} on the page`).catch(() => {});
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
        if (!first || first.href !== '#gt-main') {
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

  // axe-core over every golden page in every browser, at 360 and 1440, light and dark, with the
  // WCAG 2.0 to 2.2 A and AA rules. Any violation outside KNOWN_VIOLATIONS fails. With
  // AXE_REPORT=1 it prints the counts per page and rule and fails nothing, to size the work.
  // The shipped theme is the one the stylesheet carries; its light and dark are both walked.
  async function axeAll() {
    const axePath = createRequire(`${root}/package.json`).resolve('axe-core');
    const report = process.env.AXE_REPORT === '1';
    // The pages walked carry their crowded state whatever ran before: a search with numbered
    // hits and a branch ahead of main, so /refs shows its compare-count links.
    writeFileSync(`${dir}/axe-hits.txt`, 'one\ntwo one\n');
    sh(dir, 'git', 'add', 'axe-hits.txt');
    sh(dir, 'git', 'commit', '-q', '--allow-empty', '-m', 'test: lines that match a search');
    sh(dir, 'git', 'checkout', '-q', '-B', 'axe-ahead');
    sh(dir, 'git', 'commit', '-q', '--allow-empty', '-m', 'Work ahead of main');
    sh(dir, 'git', 'checkout', '-q', 'main');
    await synced(srv, dir);
    const seen = new Set();
    let walked = new Set();
    for (const name of browsers) {
      // axe goes in as a script the page policy does not list, so this context alone skips the
      // policy. The pages' own scripts run as they always do.
      const { b, ctx } = await context(name, { bypassCSP: true });
      const p = await ctx.newPage();
      const all = await golden(p, srv.base);
      walked = new Set(Object.keys(all));
      for (const scheme of SCHEMES) for (const width of AXE_WIDTHS) {
        await p.emulateMedia({ colorScheme: scheme });
        await p.setViewportSize({ width, height: 900 });
        for (const [key, path] of Object.entries(all)) {
          await p.goto(srv.base + path, { waitUntil: 'load' });
          if (key.startsWith('insights')) await until(p, 'svg.chart');
          await p.evaluate(() => document.fonts.ready);
          await p.addScriptTag({ path: axePath });
          /** @type {Array<[string, number, string]>} */
          const found = await p.evaluate(async (tags) => {
            // @ts-ignore injected by addScriptTag
            const res = await axe.run(document, { runOnly: tags });
            return res.violations.map((v) => [v.id, v.nodes.length,
              v.nodes.slice(0, 2).map((n) => n.target.join(' ')).join(' | ')]);
          }, AXE_TAGS);
          const where = `axe-all ${name} ${scheme} ${width} ${key}`;
          const bad = found.filter(([id]) => {
            seen.add(`${key} ${id}`);
            return !KNOWN_VIOLATIONS.has(`${key} ${id}`);
          });
          const counts = found.map(([i, n]) => `${i}=${n}`).join(' ') || 'clean';
          if (report) console.log(`${where}: ${counts}`);
          else if (bad.length) {
            r.fail(`${where}: ${bad.map(([i, n, at]) => `${i} (${n}: ${at})`).join(', ')}`);
          }
          else r.ok(where);
        }
      }
      await b.close();
    }
    if (report) return;
    for (const k of KNOWN_VIOLATIONS) {
      const [page] = k.split(' ');
      if (!walked.has(page)) {
        r.fail(`axe-all: KNOWN_VIOLATIONS names ${page}, which is not walked`);
      }
      else if (!seen.has(k)) r.fail(`axe-all: ${k} is clean now, drop it from KNOWN_VIOLATIONS`);
    }
  }

  // Under forced colors (Chromium emulation) the focus ring, the selected tab and the selected
  // segment stay visible without our colors: every Tab stop has a real outline (or, in a chart,
  // a stroked mark), and the open tab differs from its siblings by border, underline or weight.
  async function forcedColors() {
    const name = browsers.includes('chromium') ? 'chromium' : null;
    if (!name) return r.ok('forced-colors: skipped, needs chromium');
    const { b, ctx } = await context(name);
    for (const path of ['/', '/log/HEAD', '/stats', '/settings']) {
      for (const width of [1400, 360]) {
        const p = await ctx.newPage();
        await p.emulateMedia({ forcedColors: 'active', colorScheme: 'light' });
        await p.setViewportSize({ width, height: 800 });
        await p.goto(srv.base + path, { waitUntil: 'load' });
        if (path === '/stats') await until(p, 'svg.chart');
        const where = `forced-colors ${path} ${width}`;
        const bad = [];
        const active = await p.evaluate(() => matchMedia('(forced-colors: active)').matches);
        if (!active) bad.push('forced colors did not engage');
        for (let i = 0; i < 12; i++) {
          await p.keyboard.press('Tab');
          const ring = await p.evaluate(() => {
            const a = document.activeElement;
            if (!a || a === document.body) return null;
            const s = getComputedStyle(a);
            const line = parseFloat(s.outlineWidth) > 0 && s.outlineStyle !== 'none';
            const mark = a.querySelector('.hit');
            const stroke = mark && parseFloat(getComputedStyle(mark).strokeWidth) > 1;
            const edge = ['Top', 'Right', 'Bottom', 'Left']
              .some((k) => parseFloat(s[`border${k}Width`]) > 0
                && s[`border${k}Style`] !== 'none');
            const name = `${a.tagName.toLowerCase()}.${a.className}`;
            return { line, stroke: !!stroke, edge, name };
          });
          if (ring && !ring.line && !ring.stroke) bad.push(`no focus ring on ${ring.name}`);
        }
        const sel = await p.evaluate(() => {
          const on = document.querySelector('.tabnav a.on, .tabs > a.on');
          if (!on) return null;
          const off = [...on.parentElement.querySelectorAll('a')].find((a) => a !== on);
          const look = (e) => {
            const s = getComputedStyle(e);
            return [s.borderBottomWidth, s.borderBottomStyle, s.textDecorationLine, s.fontWeight]
              .join(' ');
          };
          return { on: look(on), off: off ? look(off) : null };
        });
        if (path !== '/settings' && (!sel || sel.on === sel.off)) {
          bad.push(`selected tab looks like the others (${sel?.on})`);
        }
        bad.length ? r.fail(`${where}: ${[...new Set(bad)].join('; ')}`) : r.ok(where);
        await p.close();
      }
    }
    await b.close();
  }

  // With reduced motion every element, and its before and after, has a 0s transition and
  // animation duration, and no animation is running, also with the finder and the shortcut sheet
  // open.
  async function reducedMotion() {
    await each('reduced-motion', '/', async (p) => {
      await p.emulateMedia({ reducedMotion: 'reduce', colorScheme: 'light' });
      await p.reload({ waitUntil: 'load' });
      const probe = () => p.evaluate(() => {
        const zero = (v) => v.split(',').every((x) => parseFloat(x) === 0);
        const bad = new Set();
        for (const e of document.querySelectorAll('*')) {
          for (const pseudo of [null, '::before', '::after']) {
            const s = getComputedStyle(e, pseudo);
            const what = `${e.tagName.toLowerCase()}.${e.className}`;
            const moves = s.transitionDuration;
            if (!zero(moves)) bad.add(`transition ${moves} on ${what}`);
            if (!zero(s.animationDuration)) bad.add(`animation ${s.animationDuration} on ${what}`);
          }
        }
        const running = document.getAnimations().filter((a) => a.playState === 'running'
          && Number(a.effect?.getComputedTiming().duration ?? 0) > 0);
        if (running.length) bad.add(`${running.length} animations running`);
        return [...bad].slice(0, 6);
      });
      const bad = await probe();
      await p.keyboard.press('?');
      await p.waitForSelector('.keys dt');
      bad.push(...(await probe()));
      return [...new Set(bad)];
    });
  }

  return { markFlat, tabStops, helpKeys, sideOrder, axeAll, forcedColors, reducedMotion };
}
