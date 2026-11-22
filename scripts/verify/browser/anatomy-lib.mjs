// Page anatomy: every page opens with the one page head (pagehead.rs) and keeps its rhythm.

import { eachBrowser } from '../../lib/browser-lib.mjs';
// The head is main's first child; its slots come in one order (eyebrow, title row, tagline, tab
// row or divider, toolbar); the page has one h1 and it is in the head; a tab row has two tabs
// or more and is the page's only one; the tagline says something. Across every page the head
// starts at the same height, the title has one size and font, and the gaps between the slots
// are the same, at 1440 and 390 px in light and dark. A new page, or a template that writes its
// own head, fails here. tests/conventions.rs fails when a page route is missing from PAGES.
// A view of figures (every insights tab and a file's history) has exactly one row of big
// numbers (.kpis) at the page level, and it comes first after the head, before any chart,
// table or list. A section's own figures (commit types, code frequency) do not count.
// Imported by scripts/verify/browser/browser-check.mjs, not run on its own.

/** Every kind of page under the top bar, by path under the repository (or the server). */
export const PAGES = [
  ['overview', '/'],
  ['tree', '/tree/HEAD/src'],
  ['file', '/blob/HEAD/a.txt'],
  ['markdown file', '/blob/HEAD/README.md'],
  ['blame', '/blame/HEAD/a.txt'],
  ['file history', '/history/a.txt', false, true],
  ['commits', '/log/HEAD'],
  ['commits graph', '/log/HEAD?view=graph'],
  ['commit', '/commit/HEAD'],
  ['compare', '/compare?base=HEAD~2&head=HEAD'],
  ['branches', '/refs'],
  ['activity', '/stats', false, true],
  ['contributors', '/stats/contributors', false, true],
  ['hotspots', '/stats/hotspots', false, true],
  ['languages', '/stats/languages', false, true],
  ['ownership', '/stats/ownership', false, true],
  ['search', '/search?q=one'],
  ['worktree', '/worktree'],
  ['not found', '/blob/HEAD/no-such-file'],
  ['settings', '/_settings', true],
  ['settings actions', '/_settings?f=actions', true],
];

/** The rhythm, from the tokens: title size per width, head top, gaps in px between slots. */
const TITLE = { 1440: '26px', 390: '20px' };
const GAPS = { tag: 4, close: 16, bar: 12 };

/** Slot classes in their one order; a `?` slot may be left out. */
const ORDER = ['ph-eyebrow?', 'ph-row', 'ph-tag', 'tabs|ph-sep', 'fbar?'];

/** In the page: the head's facts, or `{ err }` when there is no head where it belongs. */
function measure() {
  const main = document.querySelector('main');
  const h = main && main.firstElementChild;
  if (!h || !h.matches('header.ph[data-pagehead]')) {
    const what = h ? h.tagName.toLowerCase() + '.' + h.className : 'nothing';
    return { err: `main opens with ${what}, not the page head` };
  }
  const known = ['ph-eyebrow', 'ph-row', 'ph-tag', 'tabs', 'ph-sep', 'fbar'];
  const slot = (e) => known.find((c) => e.classList.contains(c))
    || e.tagName.toLowerCase() + '.' + e.className;
  const kids = [...h.children];
  const box = (e) => e && e.getBoundingClientRect();
  const row = h.querySelector(':scope>.ph-row'), tag = h.querySelector(':scope>.ph-tag');
  const close = h.querySelector(':scope>.tabs, :scope>.ph-sep');
  const bar = h.querySelector(':scope>.fbar');
  const h1s = [...document.querySelectorAll('h1')];
  const t = h.querySelector('h1');
  const cs = t && getComputedStyle(t);
  const pad = parseFloat(getComputedStyle(main).paddingTop);
  return {
    slots: kids.map(slot),
    h1s: h1s.length,
    h1In: !!t && h1s.length === 1,
    tabs: h.querySelectorAll(':scope>.tabs>a').length,
    otherTabs: [...main.querySelectorAll('.tabs')]
      .filter((n) => !h.contains(n) && !n.classList.contains('doctabs')).length,
    tagText: tag ? tag.textContent.trim().length : 0,
    kpis: main.querySelectorAll(':scope > .kpis').length,
    // The first block after the head (filter chips and a muted hint line aside) is the row.
    kpisFirst: (() => {
      let e = h.nextElementSibling;
      while (e && (e.matches('.chips, p.hint, [data-solo]') || !e.getClientRects().length)) {
        e = e.nextElementSibling;
      }
      return !!e && e.matches('.kpis');
    })(),
    top: Math.round(box(h).top - box(main).top - pad),
    font: cs ? `${cs.fontSize} ${cs.fontFamily.split(',')[0]} ${cs.fontWeight}` : '',
    gaps: row && tag && close ? {
      tag: Math.round(box(tag).top - box(row).bottom),
      close: Math.round(box(close).top - box(tag).bottom),
      bar: bar ? Math.round(box(bar).top - box(close).bottom) : null,
    } : null,
  };
}

/** Faults of one page's facts against the slot order. */
function order(slots) {
  let i = 0;
  for (const want of ORDER) {
    const opt = want.endsWith('?');
    const names = want.replace('?', '').split('|');
    if (names.includes(slots[i])) i++;
    else if (!opt) return [`slot ${want} missing or out of order: ${slots.join(' > ')}`];
  }
  return i < slots.length ? [`unknown or extra slot ${slots[i]}: ${slots.join(' > ')}`] : [];
}

export function checks({ srv, r, browsers, context }) {
  async function anatomy() {
    await eachBrowser(browsers, context, async (ctx, name) => {
      for (const scheme of ['light', 'dark']) for (const width of [1440, 390]) {
        // Every page keeps one title font; the first page's is the one the others must match.
        let first = null;
        for (const [label, path, outside, figures] of PAGES) {
          const p = await ctx.newPage();
          await p.emulateMedia({ colorScheme: scheme });
          await p.setViewportSize({ width, height: 900 });
          await p.goto((outside ? srv.origin : srv.base) + path, { waitUntil: 'load' });
          const m = await p.evaluate(measure);
          await p.close();
          const where = `anatomy ${name} ${scheme} ${width} ${label}`;
          if (m.err) { r.fail(`${where}: ${m.err}`); continue; }
          const bad = order(m.slots);
          if (!m.h1In) bad.push(`${m.h1s} h1 on the page, want one, in the head`);
          if (m.tabs === 1) bad.push('a tab row with one tab');
          if (m.otherTabs) bad.push('a second tab row outside the head');
          if (!m.tagText) bad.push('an empty tagline');
          if (!m.gaps) bad.push('no title row, tagline or closing line to measure');
          if (figures && m.kpis !== 1) bad.push(`${m.kpis} rows of big numbers, want 1`);
          else if (figures && !m.kpisFirst) bad.push('the big numbers do not come first');
          if (Math.abs(m.top) > 1) bad.push(`head starts ${m.top}px below main's top, want 0`);
          const size = TITLE[width];
          if (!m.font.startsWith(size + ' ')) bad.push(`title ${m.font}, want ${size}`);
          if (!first) first = { label, ...m };
          else if (m.font !== first.font) bad.push(`title ${m.font}, ${first.label} ${first.font}`);
          for (const [k, want] of Object.entries(GAPS)) {
            const got = m.gaps?.[k];
            const off = got != null && Math.abs(got - want) > 1;
            if (off) bad.push(`${k} gap ${got}px, want ${want}`);
          }
          r.verdict(where, bad);
        }
      }
    });
  }
  return { anatomy };
}
