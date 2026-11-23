// The file card: the blob page shows a card, the tree page a hot badge on a row, neither moves
// the layout of what was already there, and a server whose index is not built shows nothing.
// Imported by browser-check.mjs, not run on its own.
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { serve, makeRepo } from '../../lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, dir, bin, ...) */
export function checks(env) {
  const { srv, r, browsers, context, bin, dir } = env;
  const WAIT = { timeout: 15000 };

  /** The top of the content below the page head, to compare before and after the fetch. */
  const tops = () => [...document.querySelectorAll('main > .card, main > .head')]
    .filter((e) => !e.classList.contains('fcard'))
    .map((e) => Math.round(e.getBoundingClientRect().top));

  async function fileCardFragment() {
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      try {
        const p = await ctx.newPage();
        await p.goto(srv.base + '/blob/HEAD/a.txt', { waitUntil: 'load' });
        const before = await p.evaluate(tops);
        const card = await p.waitForSelector('.fcard:not([hidden]) .fcard-body', WAIT)
          .then(() => true, () => false);
        const after = await p.evaluate(tops);
        const moved = JSON.stringify(before) !== JSON.stringify(after);
        r.verdict(`file-card-fragment ${name} blob`,
          [card ? '' : 'no card', moved ? 'content moved' : ''].filter(Boolean),
          'card shown, nothing moved');
        await p.screenshot({ path: join(tmpdir(), `gt-file-card-${name}.png`) });
        await p.goto(srv.base + '/tree/HEAD', { waitUntil: 'load' });
        const rows = await p.evaluate(() => document.querySelectorAll('table.entries tr').length);
        const badge = await p.waitForSelector('table.entries a.hot', WAIT)
          .then(() => true, () => false);
        const rows2 = await p.evaluate(() => document.querySelectorAll('table.entries tr').length);
        r.verdict(`file-card-fragment ${name} tree`,
          [badge ? '' : 'no badge', rows === rows2 ? '' : 'rows changed'].filter(Boolean),
          'badge shown');
      } finally { await b.close(); }
    }
    // A cold index: the build waits for a page that needs it, so the fragment asked first of
    // all answers an empty body, and the card stays out of the page it was asked for.
    const cfg = mkdtempSync(join(tmpdir(), 'gt-cold-'));
    writeFileSync(join(cfg, 'config.toml'), '[index]\nbuild_on_start = false\n');
    const fresh = makeRepo();
    const cold = await serve(bin, fresh, cfg);
    try {
      const res = await fetch(cold.base + '/api/insight/HEAD/a.txt', {
        headers: { cookie: `${(await cold.owner()).name}=${(await cold.owner()).value}` },
      });
      const body = await res.text();
      r.verdict('file-card-fragment cold', body.trim() ? ['a card on a cold index'] : [],
        'nothing shown');
    } finally {
      cold.stop();
      rmSync(cfg, { recursive: true, force: true });
      rmSync(fresh, { recursive: true, force: true });
    }
  }

  return { fileCardFragment };
}
