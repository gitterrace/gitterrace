// Row parts never overlap: in each listed row on every list page (log, Graph, history, search,
// compare, refs, tag, insights), no two painted parts of a row (ref name, badges, avatar,
// subject, author and time, commit id, ahead and behind) share a pixel, and none runs past
// the row's edge, at 1440, 900, 600 and 360 px. Served on the crowded repository of
// browser-crowded-lib.mjs. Imported by scripts/browser-check.mjs, not run on its own.

import { serve } from './browser-lib.mjs';
import { LONG, LONG_TAG, crowdedRepo } from './browser-crowded-lib.mjs';

/** Pairs of overlapping parts, per row, read in the page. A part is what the row paints: each
 * run of text (by line, from a Range, so a box squeezed to 0 px still shows the text it spills)
 * and each chip with a border or background (commit id, badge, avatar). Parts of one element
 * never count against each other, and any part past the row's right edge is a finding too. */
function overlaps() {
  const out = [];
  const painted = (e) => {
    const s = getComputedStyle(e);
    const clear = s.backgroundColor === 'rgba(0, 0, 0, 0)' || s.backgroundColor === 'transparent';
    return parseFloat(s.borderTopWidth) > 0 || !clear;
  };
  const shown = (e) => {
    for (let n = e; n; n = n.parentElement) {
      const s = getComputedStyle(n);
      if (s.display === 'none' || s.visibility === 'hidden') return false;
    }
    return true;
  };
  const label = (e) => e.className && typeof e.className === 'string'
    ? `${e.tagName.toLowerCase()}.${e.className.split(' ')[0]}` : e.tagName.toLowerCase();
  // What of `r` shows: cut by every box between `from` and the row that hides its overflow (an
  // ellipsed subject paints only up to its box). Null when nothing is left.
  const clip = (r, from, row) => {
    let { left, top, right, bottom } = r;
    for (let e = from; e && e !== row.parentElement; e = e.parentElement) {
      if (getComputedStyle(e).overflowX === 'visible') continue;
      const b = e.getBoundingClientRect();
      left = Math.max(left, b.left); right = Math.min(right, b.right);
      top = Math.max(top, b.top); bottom = Math.min(bottom, b.bottom);
    }
    return right - left > 0.5 && bottom - top > 0.5 ? { left, top, right, bottom } : null;
  };
  const hit = (p, q) => p.left < q.right - 0.5 && q.left < p.right - 0.5
    && p.top < q.bottom - 0.5 && q.top < p.bottom - 0.5;
  for (const row of document.querySelectorAll('ul.list > li')) {
    /** @typedef {{left: number, top: number, right: number, bottom: number}} Box */
    /** @type {Array<{owner: Element, rect: Box, what: string}>} */
    const parts = [];
    const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
    for (let n = walker.currentNode; n; n = walker.nextNode()) {
      // The Graph view's cell is drawing, not a part: its lines run under the row by design.
      if (n.nodeType === 1 && /** @type {Element} */ (n).closest('.graph')) continue;
      if (n.nodeType === 3 && n.parentElement?.closest('.graph')) continue;
      if (n.nodeType === 1) {
        const e = /** @type {Element} */ (n);
        if (e === row || !painted(e) || !shown(e)) continue;
        const rect = clip(e.getBoundingClientRect(), e.parentElement, row);
        if (rect) parts.push({ owner: e, rect, what: label(e) });
      } else if (n.textContent.trim() && n.parentElement && shown(n.parentElement)) {
        const rg = document.createRange();
        rg.selectNodeContents(n);
        // The chip a text sits in owns it, so a chip and its own text are one part.
        const owner = n.parentElement.closest('.sha,.tag,.av') || n.parentElement;
        for (const raw of rg.getClientRects()) {
          const rect = clip(raw, n.parentElement, row);
          if (rect) parts.push({ owner, rect, what: `"${n.textContent.trim().slice(0, 16)}"` });
        }
      }
    }
    const edge = row.getBoundingClientRect();
    const name = row.textContent.trim().slice(0, 30);
    for (const p of parts) {
      if (p.rect.right > edge.right + 0.5) out.push(`${p.what} past the row edge in "${name}"`);
    }
    for (let i = 0; i < parts.length; i++) for (let j = i + 1; j < parts.length; j++) {
      const a = parts[i], b = parts[j];
      if (a.owner === b.owner || a.owner.contains(b.owner) || b.owner.contains(a.owner)) continue;
      if (hit(a.rect, b.rect)) out.push(`${a.what} over ${b.what} in "${name}"`);
    }
  }
  return { rows: document.querySelectorAll('ul.list > li').length, out };
}

/** @param {any} env what browser-check.mjs set up (bin, r, browsers, context, ...) */
export function checks(env) {
  const { bin, r, browsers, context } = env;
  async function rowOverlap() {
    const srv = await serve(bin, crowdedRepo());
    // Every page that lists rows. Each must show at least one, or the check says so.
    const pages = {
      log: '/log/HEAD', graph: '/log/HEAD?view=graph', history: '/history/f.txt',
      search: '/search?q=change&kind=commits',
      compare: `/compare?base=main&head=feature/${LONG}-too`,
      refs: '/refs', tag: `/tag/${LONG_TAG}`, types: '/stats',
    };
    try {
      for (const name of browsers) {
        const { b, ctx } = await context(name);
        const p = await ctx.newPage();
        const runs = [1440, 900, 600, 360]
          .flatMap((w) => Object.entries(pages).map((e) => [w, ...e]));
        for (const [width, key, path] of runs) {
          await p.setViewportSize({ width, height: 900 });
          await p.goto(srv.base + path, { waitUntil: 'load' });
          await p.evaluate(() => document.fonts.ready);
          const m = await p.evaluate(overlaps);
          const where = `row-overlap ${name} ${width} ${key}`;
          if (!m.rows) r.fail(`${where}: no rows listed at ${path}`);
          else if (m.out.length) r.fail(`${where}: ${m.out.slice(0, 3).join('; ')}`);
          else r.ok(`${where}: no two parts of ${m.rows} rows overlap`);
        }
        await b.close();
      }
    } finally {
      srv.stop();
    }
  }
  return { rowOverlap };
}
