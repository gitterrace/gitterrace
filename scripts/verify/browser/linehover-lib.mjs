// Line hover: pointing at a line paints the whole line, number and code, in the file view and
// in blame, in light and dark, in each browser. With display.has_line_hover = false neither
// paints. The check reads the computed background of the line (file) or of every cell of its
// row (blame), not the ::before of the number. Imported by scripts/verify/browser/
// browser-check.mjs, not run on its own.

import { mkdtempSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { eachBrowser, sh, serve, commitAll } from '../../lib/browser-lib.mjs';

/** A repository with a source file of a few lines, one of them far wider than the card. */
function hoverRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-hover-'));
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  const wide = `    let long = "${'x'.repeat(400)}";`;
  writeFileSync(join(dir, 'h.rs'), `fn main() {\n    let a = 1; // note\n${wide}\n}\n`);
  commitAll(dir, 'hover fixture');
  return dir;
}

/** Whether a computed style paints: a color with alpha or a background image. */
function paints(s) {
  const c = s.backgroundColor;
  return s.backgroundImage !== 'none' || (c !== 'rgba(0, 0, 0, 0)' && c !== 'transparent');
}

/** Hover the line `i` of the file view and report whether the whole line paints. */
export async function fileHover(p, i) {
  const sel = `.src .l >> nth=${i}`;
  await p.hover(sel);
  return p.evaluate((n) => {
    const l = document.querySelectorAll('.src .l')[n];
    const s = getComputedStyle(l);
    const c = s.backgroundColor;
    const on = s.backgroundImage !== 'none' || (c !== 'rgba(0, 0, 0, 0)' && c !== 'transparent');
    const spans = [...l.querySelectorAll('span')].filter((e) => {
      const t = getComputedStyle(e);
      return t.backgroundImage !== 'none' || (t.backgroundColor !== 'rgba(0, 0, 0, 0)');
    }).length;
    const ch = l.closest('.ch') || l.parentElement;
    const wide = Math.min(l.scrollWidth, ch.scrollWidth) - 1;
    return { on, spans, full: l.getBoundingClientRect().width >= wide };
  }, i);
}

/** Hover blame row `i` and report whether every cell of the row paints. */
export async function blameHover(p, i) {
  await p.hover(`.code tr >> nth=${i} >> td.who`);
  return p.evaluate((n) => {
    const cells = [...document.querySelectorAll('.code tr')[n].querySelectorAll('td')];
    const on = (e) => {
      const s = getComputedStyle(e), c = s.backgroundColor;
      return s.backgroundImage !== 'none' || (c !== 'rgba(0, 0, 0, 0)' && c !== 'transparent');
    };
    return { cells: cells.length, painted: cells.filter(on).length };
  }, i);
}

/** @param {any} env what browser-check.mjs set up (bin, r, browsers, context, ...) */
export function checks(env) {
  const { bin, r, browsers, context } = env;
  async function lineHover() {
    const dir = hoverRepo();
    const offCfg = mkdtempSync(join(tmpdir(), 'gt-cfg-'));
    writeFileSync(join(offCfg, 'config.toml'), '[display]\nhas_line_hover = false\n');
    const on = await serve(bin, dir);
    const off = await serve(bin, dir, offCfg);
    try {
      await eachBrowser(browsers, context, async (ctx, name) => {
        const p = await ctx.newPage();
        for (const scheme of ['light', 'dark']) {
          await p.emulateMedia({ colorScheme: scheme });
          await p.setViewportSize({ width: 900, height: 700 });
          for (const { label, srv, want } of [
            { label: 'on', srv: on, want: true }, { label: 'off', srv: off, want: false },
          ]) {
            const where = `line-hover ${name} ${scheme} ${label}`;
            await p.goto(srv.base + '/blob/HEAD/h.rs', { waitUntil: 'load' });
            await p.evaluate(() => document.fonts.ready);
            const lines = [1, 2];
            const got = [];
            for (const i of lines) got.push(await fileHover(p, i));
            const fileOk = got.every((g) => g.on === want && g.spans === 0 && (!want || g.full));
            if (fileOk) r.ok(`${where}: file line ${want ? 'paints whole' : 'does not paint'}`);
            else r.fail(`${where}: file ${JSON.stringify(got)}`);
            await p.goto(srv.base + '/blame/HEAD/h.rs', { waitUntil: 'load' });
            const bl = await blameHover(p, 1);
            const blameOk = want ? bl.cells > 0 && bl.painted === bl.cells : bl.painted === 0;
            const verb = want ? 'paints every cell' : 'does not paint';
            if (blameOk) r.ok(`${where}: blame row ${verb}`);
            else r.fail(`${where}: blame ${JSON.stringify(bl)}`);
          }
        }
      });
    } finally {
      on.stop();
      off.stop();
    }
  }
  return { lineHover };
}
