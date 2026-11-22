// Keyboard and reflow checks over every page the goldens cover: a Tab walk with a visible ring

import { eachBrowser, frames } from '../../lib/browser-lib.mjs';
// on every stop, 320 px reflow, WCAG 1.4.12 text spacing, 24 px targets, and the shortcut off
// switch. Each list of known pages can only shrink: a page green everywhere fails until it is
// dropped. Imported by scripts/verify/browser/browser-check.mjs, not run on its own.

// Pages red today, each with the change that fixes it, in words.
const KNOWN_WALK = {};
const KNOWN_REFLOW = {};
const KNOWN_SPACING = {};
const KNOWN_TARGETS = {};

const FIXED = {
  overview: '/',
  tree: '/tree/HEAD/src',
  blob: '/blob/HEAD/a.txt',
  'blob-md': '/blob/HEAD/README.md',
  log: '/log/HEAD',
  commit: '/commit/HEAD',
  compare: '/compare',
  search: '/search?q=one',
  refs: '/refs',
};

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context } = env;

  async function pages(p) {
    await p.goto(srv.base + '/stats', { waitUntil: 'load' });
    const tabs = await p.$$eval('main .tabs a', (as) => as.map((a) => a.getAttribute('href')));
    const out = { ...FIXED };
    for (const h of new Set(tabs)) {
      const path = new URL(h, srv.base + '/').pathname.replace(/^\/r\/[^/]+/, '');
      out[`insights${path.replace(/^\/stats/, '') || '/activity'}`] = path;
    }
    return out;
  }

  /** Runs `probe` on every page in every browser. Findings per page key, against `known`. */
  async function sweep(name, known, setup, probe, arg) {
    const red = new Map();
    await eachBrowser(browsers, context, async (ctx, bname) => {
      const p = await ctx.newPage();
      const all = await pages(p);
      await setup(p);
      for (const [key, path] of Object.entries(all)) {
        await p.goto(srv.base + path, { waitUntil: 'load' });
        await p.evaluate(() => document.fonts.ready);
        const found = await probe(p, arg);
        if (!red.has(key)) red.set(key, []);
        red.get(key).push(...found);
        const where = `${name} ${bname} ${key}`;
        if (!found.length) r.ok(`${where}: clean`);
        else if (known[key]) r.ok(`${where}: known, ${known[key]} fixes it (${found[0]})`);
        else r.fail(`${where}: ${found.slice(0, 5).join('; ')}`);
      }
    });
    for (const [key, row] of Object.entries(known)) {
      if (!red.has(key)) r.fail(`${name}: known list names ${key}, which is not walked`);
      else if (!red.get(key).length) r.fail(`${name} ${key}: green now, drop it (${row})`);
    }
  }

  // Tab through the whole page. At every stop: a ring of at least 2 px (outline, or a box
  // shadow, or a changed fill on svg), a 3:1 contrast against the page, not under the sticky
  // header, and the walk comes back to the start without a trap.
  async function keyboardWalk() {
    await sweep('keyboard-walk', KNOWN_WALK, async (p) => {
      await p.setViewportSize({ width: 1280, height: 800 });
    }, async (p) => {
      const found = [];
      // Both schemes: the ring and the surfaces under it change color with it.
      for (const colorScheme of ['light', 'dark']) {
        await p.emulateMedia({ colorScheme });
        found.push(...(await walk(p)).map((f) => `${colorScheme} ${f}`));
      }
      await p.emulateMedia({ colorScheme: null });
      return found;
    });
  }

  /** One Tab walk from the top of the page: what is wrong at each stop. */
  async function walk(p) {
    const found = [];
    await p.evaluate(() => {
      /** @type {HTMLElement} */ (document.activeElement)?.blur();
      scrollTo(0, 0);
      // Fades the page load started are held at their start, so every stop sees them whatever
      // the timing; ringProbe measures both ends of each.
      for (const an of document.getAnimations()) {
        an.pause();
        an.currentTime = 0;
      }
    });
    const seen = new Set();
    for (let i = 0; i < 400; i++) {
      await p.keyboard.press('Tab');
      const m = await p.evaluate(ringProbe);
      if (!m) break;
      if (seen.has(m.id) && m.id !== '') { if (m.id === [...seen][0]) break; }
      seen.add(m.id);
      if (m.problem) found.push(`${m.what}: ${m.problem}`);
      if (i === 399) found.push('Tab never leaves the page: a trap or too many stops');
    }
    return found;
  }

  async function reflow() {
    await sweep('reflow', KNOWN_REFLOW, async (p) => {
      await p.setViewportSize({ width: 320, height: 800 });
    }, async (p) =>
      p.evaluate(() => {
        const de = document.documentElement;
        if (de.scrollWidth <= de.clientWidth + 1) return [];
        const out = [];
        for (const e of document.body.querySelectorAll('*')) {
          const q = e.getBoundingClientRect();
          if (q.right <= de.clientWidth + 1 || q.width === 0) continue;
          if (
            e.closest('pre, code, table, .src, svg, .scroll, [tabindex="0"][role=region]')
          ) continue;
          out.push(
            `${e.tagName.toLowerCase()}.${String(e.className || '').split(' ')[0]} ends at ${
              Math.round(q.right)
            } of ${de.clientWidth}`,
          );
          if (out.length > 3) break;
        }
        return out.length ? out : [`page scrolls sideways (${de.scrollWidth} > ${de.clientWidth})`];
      }));
  }

  async function textSpacing() {
    await sweep('text-spacing', KNOWN_SPACING, async (p) => {
      await p.setViewportSize({ width: 1280, height: 800 });
    }, async (p) => {
      await p.addStyleTag({
        content:
          '*{line-height:1.5 !important;letter-spacing:0.12em !important;'
            + 'word-spacing:0.16em !important}p{margin-bottom:2em !important}',
      });
      await frames(p);
      return p.evaluate(() => {
        const out = [];
        for (const e of document.body.querySelectorAll('*')) {
          if (
            !e.childNodes.length
            || ![...e.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim())
          ) continue;
          const cs = getComputedStyle(e);
          if (cs.display === 'inline' || cs.visibility === 'hidden') continue;
          // Visually hidden text (the .vh pattern) is clipped to nothing on purpose.
          if (cs.clip === 'rect(0px, 0px, 0px, 0px)' || cs.clipPath === 'inset(50%)') continue;
          const clips = cs.overflow !== 'visible' || cs.overflowX !== 'visible'
            || cs.textOverflow === 'ellipsis';
          if (!clips || e.closest('pre, .src, table, [tabindex="0"]')) continue;
          const q = e.getBoundingClientRect();
          if (q.width === 0 || q.height === 0) continue;
          if (
            e.scrollHeight > e.clientHeight + 1 && cs.overflowY !== 'auto'
            && cs.overflowY !== 'scroll'
          ) {
            out.push(
              `${e.tagName.toLowerCase()}.${String(e.className || '').split(' ')[0]} clips ${
                e.scrollHeight - e.clientHeight
              } px of text`,
            );
          }
          if (out.length > 4) break;
        }
        return out;
      });
    });
  }

  // Every target 24 by 24 px, or no other target within 24 px of its center circle (WCAG 2.5.8).
  async function targets() {
    await sweep('targets', KNOWN_TARGETS, async (p) => {
      await p.setViewportSize({ width: 1280, height: 800 });
    }, async (p) =>
      p.evaluate(() => {
        const sel =
          'a[href], button, input:not([type=hidden]), select, textarea, summary, '
            + '[tabindex="0"], [role=button], [role=tab]';
        const box = [];
        const all = /** @type {NodeListOf<HTMLButtonElement>} */ (document.querySelectorAll(sel));
        for (const e of all) {
          const cs = getComputedStyle(e), q = e.getBoundingClientRect();
          if (cs.visibility === 'hidden' || q.width === 0 || q.height === 0 || e.disabled) continue;
          // An inline link inside a sentence is exempt.
          if (cs.display === 'inline' && e.closest('p, li, dd, td, th')) continue;
          // A chart's marks are one tab stop (the chart) and sit where their values put them:
          // the essential exception of 2.5.8.
          if (e.closest('svg.chart[data-marks]')) continue;
          box.push({ e, q });
        }
        const out = [];
        for (const { e, q } of box) {
          if (q.width >= 24 && q.height >= 24) continue;
          const cx = q.x + q.width / 2, cy = q.y + q.height / 2;
          const clash = box.some((o) =>
            o.e !== e && !e.contains(o.e) && !o.e.contains(e)
            && Math.hypot(
                Math.max(o.q.left - cx, 0, cx - o.q.right),
                Math.max(o.q.top - cy, 0, cy - o.q.bottom),
              ) < 12
          );
          if (clash) {
            out.push(
              `${e.tagName.toLowerCase()} "${
                (e.innerText || e.getAttribute('aria-label') || '').trim().slice(0, 24)
              }" is ${Math.round(q.width)}x${Math.round(q.height)} with a neighbor too close`,
            );
          }
          if (out.length > 4) break;
        }
        return out;
      }));
  }

  // With the setting off, g/t/y/b/j/k/slash do nothing; ? still opens the sheet; in an input no
  // shortcut ever fires; the switch in the sheet turns them back on.
  async function keysOff() {
    await eachBrowser(browsers, context, async (ctx, bname) => {
      const p = await ctx.newPage();
      await p.goto(srv.base + '/', { waitUntil: 'load' });
      await p.keyboard.press('?');
      const sw = await p.$('#keys-off');
      if (!sw) {
        r.fail(`keys-off ${bname}: no switch in the shortcut sheet`);
        return;
      }
      await sw.check();
      await p.keyboard.press('Escape');
      await p.keyboard.press('t');
      const finder = await p.$('.overlay, dialog[open], [role=dialog]');
      const open = finder ? await finder.isVisible() : false;
      if (open) r.fail(`keys-off ${bname}: t opened the finder with shortcuts off`);
      else r.ok(`keys-off ${bname}: t does nothing with shortcuts off`);
      await p.keyboard.press('?');
      const sheet = await p.$('#keys-off');
      if (sheet) r.ok(`keys-off ${bname}: ? still opens the sheet`);
      else r.fail(`keys-off ${bname}: ? does not open the sheet with keys off`);
      await (await p.$('#keys-off'))?.uncheck();
      await p.keyboard.press('Escape');
      await p.keyboard.press('t');
      const back = await p.$('.overlay, dialog[open], [role=dialog]');
      if (back && await back.isVisible()) r.ok(`keys-off ${bname}: back on after unchecking`);
      else r.fail(`keys-off ${bname}: t does nothing after turning shortcuts back on`);
      await p.keyboard.press('Escape');
      // The field is folded behind the search button until it is opened.
      await p.click('#searchbtn');
      await p.focus('#q');
      await p.keyboard.type('gtby');
      const typed = await p.$eval('#q', (i) => i.value);
      if (typed === 'gtby') {
        r.ok(`keys-off ${bname}: typing in the search box never fires a shortcut`);
      } else r.fail(`keys-off ${bname}: typed 'gtby', field holds '${typed}'`);
    });
  }
  return { keyboardWalk, reflow, textSpacing, targets, keysOff };
}

// Runs in the page after each Tab. Describes the focused element and what is wrong with its ring.
function ringProbe() {
  const e = /** @type {HTMLElement} */ (document.activeElement);
  if (!e || e === document.body || e === document.documentElement) return null;
  const q = e.getBoundingClientRect();
  const what = `${e.tagName.toLowerCase()}${e.id ? '#' + e.id : ''} "${
    (e.innerText || e.getAttribute('aria-label') || '').trim().replace(/\s+/g, ' ').slice(0, 24)
  }"`;
  const id = what + '@' + Math.round(q.x) + ',' + Math.round(q.y + scrollY);
  const cs = getComputedStyle(e);
  const ow = cs.outlineStyle !== 'none' ? parseFloat(cs.outlineWidth) : 0;
  const sh = /\d+px/.test(cs.boxShadow) && cs.boxShadow !== 'none'
    ? (cs.boxShadow.match(/(\d+(?:\.\d+)?)px/g) || []).map(parseFloat).slice(-1)[0]
    : 0;
  let problem = '';
  const isSvg = e instanceof SVGElement;
  const inner = isSvg ? e.querySelector('.hit, .bar') : null;
  const innerChange = inner && getComputedStyle(inner).strokeWidth
    && parseFloat(getComputedStyle(inner).strokeWidth) >= 2;
  if (ow < 2 && sh < 2 && !innerChange) problem = `no ring of 2 px (outline ${ow}, shadow ${sh})`;
  if (!problem && ow >= 2) {
    // A fill still fading (a transition, such as one a page load starts) is measured at both
    // ends, the worse counting: the result does not hang on where the fade was at the Tab, and a
    // fade out of a bad color is still caught.
    const fades = document.getAnimations().filter((an) => an.playState !== 'finished');
    const ends = [];
    if (fades.length) {
      for (const an of fades) {
        an.pause();
        an.currentTime = 0;
      }
      ends.push(ringContrast(e));
      for (const an of fades) an.finish();
    }
    ends.push(ringContrast(e));
    const [ratio, bg] = ends.sort((x, y) => x[0] - y[0])[0];
    if (ratio < 3) problem = `ring contrast ${ratio.toFixed(2)}:1 against ${bg}`;
  }
  if (!problem) {
    // A sticky header covers only while it is pinned at its top; one still in the flow lower
    // down covers nothing above it. Its own controls cannot sit under it either, so a stop
    // inside a pinned header is measured against none.
    const heads = [
      ...document.querySelectorAll('header, .topbar, [data-dochead], section.file>header'),
    ]
      .filter((h) => {
        const cs = getComputedStyle(h);
        if (cs.position === 'fixed') return true;
        return cs.position === 'sticky'
          && h.getBoundingClientRect().top <= (parseFloat(cs.top) || 0) + 1;
      });
    // A header covers only the column it spans: the outline rail beside a pinned document head
    // is not under it.
    const r2 = e.getBoundingClientRect();
    const top = heads.some((h) => h.contains(e)) ? 0 : heads
      .map((h) => h.getBoundingClientRect())
      .filter((q) => q.left < r2.right && q.right > r2.left)
      .reduce((m, q) => Math.max(m, q.bottom), 0);
    if (top > 0 && r2.top >= 0 && r2.top < top - 1 && r2.bottom > 0) {
      problem = `covered by the sticky header (top ${Math.round(r2.top)} < ${Math.round(top)})`;
    }
  }
  return { id, what, problem };

  // The ring's contrast against the lower of the colors it touches, each composited down to the
  // canvas so it is opaque. An offset above 0 leaves a gap of the parent between ring and element,
  // so it touches only the backdrop; at 0 it touches the backdrop and the element's fill; below 0
  // it is drawn on the element's fill.
  /** @param {Element} e @returns {[number, string]} */
  function ringContrast(e) {
    const cs = getComputedStyle(e);
    const lum = (v) => {
      const f = (x) => {
        x /= 255;
        return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
      };
      return 0.2126 * f(v[0]) + 0.7152 * f(v[1]) + 0.0722 * f(v[2]);
    };
    const rgba = (c) => {
      const v = (c.match(/[\d.]+/g) || []).map(Number);
      return v.length < 3 ? [0, 0, 0, 0] : [v[0], v[1], v[2], v.length > 3 ? v[3] : 1];
    };
    const over = (c, u) => [0, 1, 2].map((i) => c[i] * c[3] + u[i] * (1 - c[3]));
    const probe = document.createElement('div');
    probe.style.cssText = 'position:absolute;width:0;height:0;background:Canvas';
    document.body.append(probe);
    let under = rgba(getComputedStyle(probe).backgroundColor);
    probe.remove();
    const offset = parseFloat(cs.outlineOffset) || 0;
    const start = offset >= 0 ? e.parentElement : e;
    const layers = [];
    for (let n = start; n; n = n.parentElement) {
      const c = rgba(getComputedStyle(n).backgroundColor);
      if (c[3] > 0) layers.push(c);
      if (c[3] >= 1) break;
    }
    for (const c of layers.reverse()) under = over(c, under);
    const sides = [under];
    if (offset === 0) sides.push(over(rgba(cs.backgroundColor), under));
    const a = lum(rgba(cs.outlineColor));
    return sides
      .map((s) => {
        const b = lum(s);
        const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
        return /** @type {[number, string]} */ ([ratio, `rgb(${s.map(Math.round)})`]);
      })
      .sort((x, y) => x[0] - y[0])[0];
  }
}
