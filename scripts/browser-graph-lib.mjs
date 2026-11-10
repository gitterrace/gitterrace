// The log's Graph view: its lines meet across row edges, a wide history folds into "+N" at a
// narrow width, a ref chip keeps its row's height, tip labels show in full and overlap
// nothing, and the List | Graph switch keeps the page at the top. Serves its own fixture with
// a wide merge history. Imported by scripts/browser-check.mjs, not run on its own.
import { mkdtempSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { sh, serve } from './lib/browser-lib.mjs';

/** Branches open at once in the fixture: more than a narrow screen shows. */
const OPEN = 5;

/** Branches in the fan-in fixture: far more than a wide screen shows. */
const FAN = 15;

/** Branch names on the tip: one must wrap at `/`, `-` and `_` instead of being clipped. */
const LONG = ['feature/graph-labels_wrap-inside-their-column-at-every-width', 'fix/x'];

/** A history where `open` branches fork from main and merge back one by one, newest on top. */
function wideRepo(open = OPEN) {
  const dir = mkdtempSync(join(tmpdir(), 'gt-graph-'));
  let t = 1_700_000_000;
  const at = (...a) => {
    t += 60;
    const env = { GIT_AUTHOR_DATE: `${t} +0000`, GIT_COMMITTER_DATE: `${t} +0000` };
    return sh(dir, 'env', ...Object.entries(env).map(([k, v]) => `${k}=${v}`), 'git', ...a);
  };
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  at('commit', '-q', '--allow-empty', '-m', 'base');
  for (let i = 0; i < open; i++) {
    at('checkout', '-q', '-b', `topic-${i}`, 'main');
    at('commit', '-q', '--allow-empty', '-m', `topic ${i}: work`);
  }
  at('checkout', '-q', 'main');
  for (let i = 0; i < open; i++) {
    at('commit', '-q', '--allow-empty', '-m', `main ${i}`);
    at('merge', '-q', '--no-ff', '-m', `Merge branch 'topic-${i}'`, `topic-${i}`);
  }
  at('tag', 'v1');
  // Several labels on the top tip, one far longer than a label column.
  for (const n of LONG) at('branch', n);
  return dir;
}

/** In the page: row edges where the cell above and the one below do not touch. */
function measure() {
  const rows = [...document.querySelectorAll('.graphed .crow')];
  const box = (el) => el.getBoundingClientRect();
  const gaps = [];
  rows.forEach((li, i) => {
    const g = box(li.querySelector('.graph'));
    const l = box(li);
    if (Math.abs(g.top - l.top) > 1 || Math.abs(g.bottom - l.bottom) > 1) gaps.push(`row ${i} cell off its row`);
    if (i && Math.abs(g.top - box(rows[i - 1].querySelector('.graph')).bottom) > 1) gaps.push(`gap above row ${i}`);
  });
  const plain = rows.filter((li) => !li.querySelector('.grefs')).map((li) => box(li).height);
  const tall = rows.filter((li) => li.querySelector('.grefs') && box(li).height > Math.max(...plain) + 1).length;
  const fold = [...document.querySelectorAll('.gfold')].some((f) => getComputedStyle(f).display !== 'none' && box(f).width > 0);
  const over = document.documentElement.scrollWidth > innerWidth + 1;
  return { rows: rows.length, gaps, tall, fold, over };
}

/**
 * In the page: tip labels that are clipped (scrollWidth past clientWidth) or that overlap
 * another label, the row's graph cell or its subject.
 */
function labels() {
  const bad = [];
  const box = (el) => el.getBoundingClientRect();
  const hit = (a, b) => a.left < b.right - 0.5 && b.left < a.right - 0.5
    && a.top < b.bottom - 0.5 && b.top < a.bottom - 0.5;
  const refs = [...document.querySelectorAll('.graphed .crow .gref')];
  refs.forEach((g, i) => {
    const name = g.textContent;
    if (g.scrollWidth > g.clientWidth + 0.5) bad.push(`${name} clipped`);
    const a = box(g);
    for (const o of refs.slice(i + 1)) {
      if (hit(a, box(o))) bad.push(`${name} on ${o.textContent}`);
    }
    const li = g.closest('.crow');
    for (const sel of ['.graph', '.subj']) {
      const o = li.querySelector(sel);
      if (o && hit(a, box(o))) bad.push(`${name} on ${sel}`);
    }
  });
  return { n: refs.length, bad };
}

/**
 * In the page: strokes drawn past the columns a cell shows, and "+N" counts that repeat the
 * row above. A shown piece of line must end inside the cell's visible box, so a line past the
 * cap is never cut off at the edge.
 */
function clipped() {
  const out = [], rep = [];
  let above = null;
  [...document.querySelectorAll('.graphed .crow .graph')].forEach((g, i) => {
    const w = g.getBoundingClientRect().width;
    for (const e of g.querySelectorAll('path, circle')) {
      if (getComputedStyle(e).display === 'none' || !e.getClientRects().length) continue;
      const b = /** @type {SVGGraphicsElement} */ (e).getBBox();
      const end = b.x + b.width;
      const at = `row ${i} ${e.tagName} to x ${end.toFixed(0)}`;
      if (end > w + 0.5) out.push(`${at} past ${w.toFixed(0)}`);
    }
    const shown = (e) => getComputedStyle(e).display !== 'none';
    const dot = [...g.querySelectorAll('circle')].some(shown);
    const edge = [...g.querySelectorAll('.gedge')].some((e) => e.getClientRects().length);
    if (!dot && !edge) out.push(`row ${i} has no dot and no fold-edge mark`);
    const f = [...g.querySelectorAll('.gfold')].find((e) => getComputedStyle(e).display !== 'none');
    const n = f ? f.textContent : null;
    if (n && n === above) rep.push(`row ${i} repeats ${n}`);
    above = n;
  });
  return { out, rep };
}

/** @param {any} env what browser-check.mjs set up (r, browsers, context, bin, ...) */
export function checks(env) {
  const { r, browsers, context, bin } = env;

  async function graphView() {
    const srv = await serve(bin, wideRepo());
    try {
      for (const name of browsers) {
        const { b, ctx } = await context(name);
        const p = await ctx.newPage();
        for (const width of [390, 1440]) {
          const where = `graph-view ${name} ${width}`;
          await p.setViewportSize({ width, height: 900 });
          await p.goto(srv.base + '/log/HEAD?view=graph', { waitUntil: 'load' });
          await p.evaluate(() => document.fonts.ready);
          const m = await p.evaluate(measure);
          if (!m.rows) { r.fail(`${where}: no graph rows`); continue; }
          if (m.gaps.length) r.fail(`${where}: lines break at row edges: ${m.gaps.slice(0, 3).join('; ')}`);
          else r.ok(`${where}: lines meet across all ${m.rows} row edges`);
          if (m.tall) r.fail(`${where}: ${m.tall} labelled row(s) taller than an unlabelled one`);
          else r.ok(`${where}: a ref chip keeps the row height`);
          if (m.over) r.fail(`${where}: page scrolls sideways`);
          const t = await p.evaluate(labels);
          const why = t.bad.slice(0, 3).join('; ');
          if (!t.n) r.fail(`${where}: no tip labels`);
          else if (t.bad.length) r.fail(`${where}: tip labels clipped or overlapping: ${why}`);
          else r.ok(`${where}: ${t.n} tip labels show in full and overlap nothing`);
          if (width === 390) {
            if (m.fold) r.ok(`${where}: ${OPEN} open branches fold into a "+N" count`);
            else r.fail(`${where}: no "+N" fold on a history wider than 3 columns`);
          }
          // The switch is a link: the page it loads starts at the top, both ways.
          for (const [to, url] of [['List', /\/log\/HEAD$/], ['Graph', /view=graph/]]) {
            await p.evaluate(() => scrollTo(0, 0));
            await Promise.all([p.waitForURL(url, { waitUntil: 'load' }), p.click(`nav.tabs a:has-text("${to}")`)]);
            const y = await p.evaluate(() => scrollY);
            if (y > 0) r.fail(`${where}: switching to ${to} left the page at ${y}px`);
            else r.ok(`${where}: switching to ${to} keeps the page at the top`);
          }
        }
        await b.close();
      }
    } finally {
      srv.stop();
    }
    const fan = await serve(bin, wideRepo(FAN));
    try {
      for (const name of browsers) {
        const { b, ctx } = await context(name);
        const p = await ctx.newPage();
        for (const width of [390, 1440]) {
          const where = `graph-view ${name} ${width} ${FAN}-branch fan-in`;
          await p.setViewportSize({ width, height: 900 });
          await p.goto(fan.base + '/log/HEAD?view=graph', { waitUntil: 'load' });
          const m = await p.evaluate(clipped);
          const cut = m.out.slice(0, 3).join('; ');
          if (m.out.length) r.fail(`${where}: ${m.out.length} stroke(s) cut at the edge: ${cut}`);
          else r.ok(`${where}: strokes end inside the visible columns, each row has an anchor`);
          const rep = m.rep.slice(0, 3).join('; ');
          if (m.rep.length) r.fail(`${where}: ${m.rep.length} "+N" repeat the row above: ${rep}`);
          else r.ok(`${where}: a "+N" count shows only where it changes`);
        }
        await b.close();
      }
    } finally {
      fan.stop();
    }
  }
  return { graphView };
}
