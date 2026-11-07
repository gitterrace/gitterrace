// The log's Graph view: its lines meet across row edges, a wide history folds into "+N" at a
// narrow width, a ref chip keeps its row's height, and the List | Graph switch keeps the page
// at the top. Serves its own fixture with a wide merge history. Imported by
// scripts/browser-check.mjs, not run on its own.
import { mkdtempSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { sh, serve } from './browser-lib.mjs';

/** Branches open at once in the fixture: more than a narrow screen shows. */
const OPEN = 5;

/** A history where OPEN branches fork from main and merge back one by one, newest on top. */
function wideRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-graph-'));
  let t = 1_700_000_000;
  const at = (...a) => {
    t += 60;
    const env = { GIT_AUTHOR_DATE: `${t} +0000`, GIT_COMMITTER_DATE: `${t} +0000` };
    return sh(dir, 'env', ...Object.entries(env).map(([k, v]) => `${k}=${v}`), 'git', ...a);
  };
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  at('commit', '-q', '--allow-empty', '-m', 'base');
  for (let i = 0; i < OPEN; i++) {
    at('checkout', '-q', '-b', `topic-${i}`, 'main');
    at('commit', '-q', '--allow-empty', '-m', `topic ${i}: work`);
  }
  at('checkout', '-q', 'main');
  for (let i = 0; i < OPEN; i++) {
    at('commit', '-q', '--allow-empty', '-m', `main ${i}`);
    at('merge', '-q', '--no-ff', '-m', `Merge branch 'topic-${i}'`, `topic-${i}`);
  }
  at('tag', 'v1');
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
          if (width === 390) {
            if (m.fold) r.ok(`${where}: ${OPEN} open branches fold into a "+N" count`);
            else r.fail(`${where}: no "+N" fold on a history wider than 3 columns`);
          }
          // The switch is a link: the page it loads starts at the top, both ways.
          for (const [to, url] of [['List', /\/log\/HEAD$/], ['Graph', /view=graph/]]) {
            await p.evaluate(() => scrollTo(0, 0));
            await Promise.all([p.waitForURL(url, { waitUntil: 'load' }), p.click(`.segs a:has-text("${to}")`)]);
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
  }
  return { graphView };
}
