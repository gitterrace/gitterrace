// Blame gutter fit: in each run's gutter cell (sha, author, age) nothing paints past the
// cell's right edge, the hairline that closes the column, at 1400 and 900 px in light and
// dark. Served on a file whose lines come from a long author name a few minutes ago.
// Imported by browser-check.mjs, not run on its own.

import { mkdtempSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { eachBrowser, serve } from '../../lib/browser-lib.mjs';

const WHO = 'Maximilian Featherstonehaugh-Runge';

/** A repository with two runs: a long author 14 minutes ago and a short one an hour ago. */
export function blameRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-blame-'));
  const git = (who, when, ...a) => execFileSync('git', a, {
    cwd: dir,
    env: {
      ...process.env, GIT_AUTHOR_NAME: who, GIT_AUTHOR_EMAIL: 'w@w', GIT_COMMITTER_NAME: who,
      GIT_COMMITTER_EMAIL: 'w@w', GIT_AUTHOR_DATE: when, GIT_COMMITTER_DATE: when,
    },
  });
  const at = (min) => `${Math.floor(Date.now() / 1000) - min * 60} +0000`;
  git('Al', at(60), 'init', '-q', '-b', 'main');
  writeFileSync(join(dir, 'b.txt'), 'one\ntwo\n');
  git('Al', at(60), 'add', '-A');
  git('Al', at(60), 'commit', '-q', '-m', 'first');
  writeFileSync(join(dir, 'b.txt'), 'one\ntwo\nthree\nfour\n');
  git(WHO, at(14), 'commit', '-q', '-a', '-m', 'second');
  return dir;
}

/** Findings: any gutter cell whose content or text reaches past the cell's right edge, or
 * whose text is cut inside its own box (a clipped first glyph is as wrong as a spill). */
export function spill() {
  const out = [];
  for (const td of document.querySelectorAll('.blame td.who')) {
    const bl = td.querySelector('.bl');
    if (!bl) continue;
    const edge = td.getBoundingClientRect().right;
    const pad = parseFloat(getComputedStyle(td).paddingRight);
    for (const e of [bl, ...bl.children]) {
      if (!e.getClientRects().length) continue;
      const rg = document.createRange();
      rg.selectNodeContents(e);
      const box = e.getBoundingClientRect(), text = rg.getBoundingClientRect();
      const right = Math.max(box.right, text.right);
      if (right > edge - pad + 0.5) {
        out.push(`${e.className || e.tagName} ends ${right.toFixed(1)} past ${edge - pad}`);
      }
      // `.vh` is clipped on purpose: it is read by screen readers, never seen.
      const cut = text.left < box.left - 0.5 || e.scrollWidth > e.clientWidth + 1;
      if (e !== bl && !e.classList.contains('vh') && cut) {
        out.push(`${e.className || e.tagName} text is cut inside its box`);
      }
    }
  }
  return { cells: document.querySelectorAll('.blame td.who .bl').length, out };
}

/** @param {any} env what browser-check.mjs set up (bin, r, browsers, context, ...) */
export function checks(env) {
  const { bin, r, browsers, context } = env;
  async function blameFit() {
    const srv = await serve(bin, blameRepo());
    try {
      await eachBrowser(browsers, context, async (ctx, name) => {
        const p = await ctx.newPage();
        for (const scheme of ['light', 'dark']) for (const width of [1400, 900]) {
          await p.emulateMedia({ colorScheme: scheme });
          await p.setViewportSize({ width, height: 800 });
          await p.goto(srv.base + '/blame/HEAD/b.txt', { waitUntil: 'load' });
          await p.evaluate(() => document.fonts.ready);
          const m = await p.evaluate(spill);
          const where = `blame-fit ${name} ${scheme} ${width}`;
          if (!m.cells) r.fail(`${where}: no gutter cells`);
          else if (m.out.length) r.fail(`${where}: ${m.out.slice(0, 3).join('; ')}`);
          else r.ok(`${where}: ${m.cells} gutter cells fit`);
        }
      });
    } finally {
      srv.stop();
    }
  }
  return { blameFit };
}
