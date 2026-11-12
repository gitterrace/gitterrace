// Vertical balance: the one-line blame row and the Send to action row have as much space
// above their content as below it (1 px tolerance), light and dark. Imported by
// browser-check.mjs, not run on its own.

import { mkdtempSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { serve } from '../../lib/browser-lib.mjs';

/** A repository whose one file has a single line. */
function oneLineRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-inset-'));
  const git = (...a) => execFileSync('git', a, {
    cwd: dir,
    env: {
      ...process.env, GIT_AUTHOR_NAME: 'Al', GIT_AUTHOR_EMAIL: 'a@a',
      GIT_COMMITTER_NAME: 'Al', GIT_COMMITTER_EMAIL: 'a@a',
    },
  });
  git('init', '-q', '-b', 'main');
  writeFileSync(join(dir, 'one.txt'), 'only line\n');
  git('add', '-A');
  git('commit', '-q', '-m', 'one');
  return dir;
}

/** Space above and below the content of each cell of the row, inside the row's box. */
export function blameInsets() {
  const tr = /** @type {HTMLTableRowElement} */ (document.querySelector('table.blame tbody tr'));
  const box = tr.getBoundingClientRect();
  const out = [];
  for (const td of tr.cells) {
    const bl = td.querySelector('.bl');
    const rg = document.createRange();
    rg.selectNodeContents(td);
    const c = bl ? bl.getBoundingClientRect() : rg.getBoundingClientRect();
    out.push({ cell: td.className || 'code', top: c.top - box.top, bottom: box.bottom - c.bottom });
  }
  return out;
}

/** Space above and below the buttons inside the footer, its border not counted. */
export function footInsets(sel) {
  const foot = document.querySelector(sel);
  const box = foot.getBoundingClientRect();
  const cs = getComputedStyle(foot);
  const kids = [...foot.querySelectorAll('button')].map((b) => b.getBoundingClientRect());
  const top = Math.min(...kids.map((k) => k.top)), bottom = Math.max(...kids.map((k) => k.bottom));
  return [{
    cell: sel,
    top: top - box.top - parseFloat(cs.borderTopWidth),
    bottom: box.bottom - bottom - parseFloat(cs.borderBottomWidth),
  }];
}

/** @param {any} env what browser-check.mjs set up (bin, r, browsers, context, ...) */
export function checks(env) {
  const { bin, r, browsers, context } = env;
  const judge = (where, rows) => {
    const bad = rows.filter((x) => Math.abs(x.top - x.bottom) > 1);
    if (!rows.length) r.fail(`${where}: nothing measured`);
    else if (bad.length) {
      const say = (x) => `${x.cell} top ${x.top.toFixed(1)} bottom ${x.bottom.toFixed(1)}`;
      r.fail(`${where}: ${bad.map(say).join('; ')}`);
    }
    else r.ok(`${where}: top and bottom space equal`);
  };
  async function insetBalance() {
    const srv = await serve(bin, oneLineRepo());
    try {
      for (const name of browsers) {
        const { b, ctx } = await context(name);
        const p = await ctx.newPage();
        for (const scheme of ['light', 'dark']) {
          await p.emulateMedia({ colorScheme: scheme });
          await p.setViewportSize({ width: 1200, height: 800 });
          await p.goto(srv.base + '/blame/HEAD/one.txt', { waitUntil: 'load' });
          await p.evaluate(() => document.fonts.ready);
          judge(`inset-balance ${name} ${scheme} blame row`, await p.evaluate(blameInsets));
          await p.goto(srv.base + '/blob/HEAD/one.txt', { waitUntil: 'load' });
          await p.waitForTimeout(300);
          await p.keyboard.press('.');
          await p.keyboard.press('s');
          await p.waitForSelector('.send-foot');
          const foot = await p.evaluate(footInsets, '.send-foot');
          judge(`inset-balance ${name} ${scheme} send actions`, foot);
        }
        await b.close();
      }
    } finally {
      srv.stop();
    }
  }
  return { insetBalance };
}
