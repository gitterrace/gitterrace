// The date line of a row never breaks inside a word: on the log (plain and Graph), the
// overview, the refs page and the tag page, served with a 120 character branch name and a 200
// character subject, at 1440, 900, 600 and 390 px, every `.muted` author and time text keeps
// each word (a hyphen may end a line) on one line and takes at most 2 lines. It reads the
// text's own line boxes, so a
// box squeezed until it wraps one character per line shows. The stash table on the refs page
// holds the same kind of cell, so a table cell squeezed the same way fails too. Set
// DATEWRAP_SHOTS to a folder to also save a screenshot of each page. Imported by
// browser-check.mjs, not run on its own.

import { mkdtempSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { eachBrowser, serve } from '../../lib/browser-lib.mjs';

const BRANCH = `feature/${'long-branch-name-'.repeat(7)}end`.slice(0, 120);
const SUBJECT = `feat(core): ${'a subject that keeps going and going '.repeat(6)}`.slice(0, 200);
const AUTHOR = 'Alexandria Featherstonehaugh';

/** A repository with the long branch, the long subject, an annotated tag and a long stash. */
function longRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-datewrap-'));
  const env = { ...process.env, GIT_AUTHOR_NAME: AUTHOR, GIT_AUTHOR_EMAIL: 'a@a',
    GIT_COMMITTER_NAME: AUTHOR, GIT_COMMITTER_EMAIL: 'a@a' };
  const git = (...a) => execFileSync('git', a, { cwd: dir, env, stdio: 'pipe' }).toString();
  git('init', '-q', '-b', 'main');
  const commit = (i, subject) => {
    writeFileSync(join(dir, 'f.txt'), `${i}\n`);
    git('add', '-A');
    git('commit', '-q', '-m', subject);
  };
  commit(0, 'chore: first');
  git('checkout', '-q', '-b', BRANCH);
  for (let i = 1; i < 4; i++) commit(i, `${SUBJECT} ${i}`);
  git('checkout', '-q', 'main');
  commit(10, SUBJECT);
  git('tag', '-a', 'v1.0.0-with-a-very-long-release-name', '-m', SUBJECT);
  writeFileSync(join(dir, 'f.txt'), 'dirty\n');
  git('stash', 'push', '-q', '-m', SUBJECT);
  return dir;
}

/** Findings for the date lines of the page: a word cut across lines, or more than 2 lines. */
function dateLines() {
  const out = [];
  let seen = 0;
  const cells = document.querySelectorAll('ul.list .cmain > .muted, td.muted');
  for (const el of cells) {
    if (!el.getClientRects().length) continue;
    seen++;
    const w = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let n = w.nextNode(); n; n = w.nextNode()) {
      for (const m of n.textContent.matchAll(/[^\s-]+-?/g)) {
        const rg = document.createRange();
        rg.setStart(n, m.index);
        rg.setEnd(n, m.index + m[0].length);
        const lines = new Set([...rg.getClientRects()].map((q) => Math.round(q.top)));
        if (lines.size > 1) out.push(`"${m[0]}" broken across ${lines.size} lines`);
      }
    }
    // Lines are counted from the text's own boxes, and only those inside the element's box:
    // a line clamp hides the lines past its limit, and a table cell can be stretched taller
    // than its text by a neighbor.
    const rg = document.createRange();
    rg.selectNodeContents(el);
    const box = el.getBoundingClientRect();
    const tops = [...new Set([...rg.getClientRects()]
      .filter((q) => q.top < box.bottom - 1).map((q) => Math.round(q.top)))].sort((x, y) => x - y);
    const rows = tops.reduce((k, t, i) => (i && t - tops[i - 1] <= 4 ? k : k + 1), 0);
    if (rows > 2) out.push(`"${el.textContent.trim().slice(0, 24)}" takes ${rows} lines`);
  }
  // Graph rows keep a readable subject: about 16 characters, 8rem, whatever the labels hold.
  const min = 8 * parseFloat(getComputedStyle(document.documentElement).fontSize);
  for (const sj of document.querySelectorAll('.graphed .crow .subj')) {
    const w = sj.getBoundingClientRect().width;
    const name = sj.textContent.trim().slice(0, 12);
    if (w && w < min) out.push(`subject "${name}" is ${Math.round(w)}px, under ${min}px`);
  }
  // A Graph label shows at most 3 lines and never breaks inside a word (a name ends a
  // word at a slash, hyphen or underscore, where the template offers a break). At 390 the
  // gutter is the narrow one, so the same limits hold on the page's labels.
  for (const g of document.querySelectorAll('.glabels .gref')) {
    if (!g.getClientRects().length) continue;
    const cs = getComputedStyle(g);
    const line = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.25;
    const inner = g.getBoundingClientRect().height - parseFloat(cs.paddingTop) * 2 - 2;
    const label = g.textContent.slice(0, 12);
    if (Math.round(inner / line) > 3) out.push(`label "${label}" takes over 3 lines`);
    const tw = document.createTreeWalker(g, NodeFilter.SHOW_TEXT);
    for (let w = tw.nextNode(); w; w = tw.nextNode()) {
      for (const m of w.textContent.matchAll(/[^\s\/_-]+[\/_-]?/g)) {
        const rg = document.createRange();
        rg.setStart(w, m.index);
        rg.setEnd(w, m.index + m[0].length);
        const tops = new Set([...rg.getClientRects()].map((q) => Math.round(q.top)));
        if (tops.size > 1) out.push(`label word "${m[0]}" is broken`);
      }
    }
  }
  return { seen, out };
}

/** @param {any} env what browser-check.mjs set up (bin, r, browsers, context, ...) */
export function checks(env) {
  const { bin, r, browsers, context } = env;
  async function dateWrap() {
    const srv = await serve(bin, longRepo());
    const shots = process.env.DATEWRAP_SHOTS;
    const pages = {
      overview: '/', log: '/log/HEAD', graph: '/log/HEAD?view=graph', refs: '/refs',
      tag: '/tag/v1.0.0-with-a-very-long-release-name',
    };
    try {
      await eachBrowser(browsers, context, async (ctx, name) => {
        const p = await ctx.newPage();
        for (const width of [1440, 900, 600, 390]) {
          for (const [key, path] of Object.entries(pages)) {
            await p.setViewportSize({ width, height: 900 });
            await p.goto(srv.base + path, { waitUntil: 'load' });
            await p.evaluate(() => document.fonts.ready);
            const m = await p.evaluate(dateLines);
            const where = `date-wrap ${name} ${width} ${key}`;
            if (shots) await p.screenshot({ path: join(shots, `${name}-${width}-${key}.png`) });
            if (!m.seen) r.fail(`${where}: no date line on ${path}`);
            else if (m.out.length) {
              r.fail(`${where}: ${[...new Set(m.out)].slice(0, 3).join('; ')}`);
            }
            else r.ok(`${where}: ${m.seen} date lines whole, at most 2 lines`);
          }
        }
      });
    } finally {
      srv.stop();
    }
  }
  return { dateWrap };
}
