// The log's one filter field. Imported by browser-check.mjs, not run alone.

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context } = env;

  // With and without script, at 1440 and 390 px: Enter on the field sends `?q=` and the log
  // shows one chip per term; a chip's cross removes its term alone; a refused filter keeps its
  // text with the reason under the field; the head has no Compare and nothing scrolls sideways.
  async function logFilter() {
    for (const name of browsers) {
      for (const js of [true, false]) {
        const { b, ctx } = await context(name, { javaScriptEnabled: js });
        const p = await ctx.newPage();
        for (const width of [1440, 390]) {
          const where = `log-filter ${name} ${js ? 'js' : 'no-js'} ${width}`;
          try {
            await p.setViewportSize({ width, height: 800 });
            await p.goto(srv.base + '/log/HEAD', { waitUntil: 'load' });
            const head = await p.evaluate(() => ({
              fields: document.querySelectorAll('.ph form.filter input[name=q]').length,
              compare: [...document.querySelectorAll('.ph a')]
                .some((a) => a.textContent.trim() === 'Compare'),
              path: !!document.querySelector('input[name=path]'),
              named: document.querySelector('#logq')?.getAttribute('aria-label'),
            }));
            await p.fill('#logq', 'path:src !merge');
            await Promise.all([p.waitForURL(/[?&]q=/), p.press('#logq', 'Enter')]);
            const url = new URL(p.url());
            const chips = () => p.$$eval('.chips .chip',
              (cs) => cs.map((c) => c.textContent.replace('×', '').trim()));
            const two = await chips();
            const value = await p.inputValue('#logq');
            await Promise.all([p.waitForNavigation(), p.click('.chips .chip:first-child a')]);
            const one = await chips();
            const left = await p.inputValue('#logq');
            await p.fill('#logq', 'a | b');
            await Promise.all([p.waitForNavigation(), p.press('#logq', 'Enter')]);
            const refused = await p.evaluate(() => {
              const e = document.querySelector('.filter-error');
              const q = /** @type {HTMLInputElement | null} */ (document.querySelector('#logq'));
              return {
                shown: !!e && e.getClientRects().length > 0,
                says: e?.textContent.includes('Supported:'),
                kept: q?.value,
                tied: q?.getAttribute('aria-describedby') === e?.id,
                sideways: document.documentElement.scrollWidth - innerWidth,
              };
            });
            const ok = head.fields === 1 && !head.compare && !head.path && head.named
              && url.searchParams.get('q') === 'path:src !merge'
              && value === 'path:src !merge'
              && two.length === 2 && two[0] === 'path src' && two[1] === 'merge not merge'
              && one.length === 1 && left === '!merge'
              && refused.shown && refused.says && refused.kept === 'a | b' && refused.tied
              && refused.sideways <= 0;
            const got = { head, url: url.search, two, value, one, left, refused };
            if (ok) r.ok(`${where}: one field, Enter sends q, a chip removes one term, refused`);
            else r.fail(`${where}: ${JSON.stringify(got)}`);
          } catch (e) { r.fail(`${where}: ${e.message.split('\n')[0]}`); }
        }
        await b.close();
      }
    }
  }
  // The "?" at the field's right end opens a short help that lists every keyword, with and
  // without script, at both widths. Escape closes it and gives focus back; a click outside
  // closes it (script only: without script a details only toggles).
  async function logFilterHelp() {
    const names = ['type', 'scope', 'author', 'path', 'after', 'before', 'merge'];
    for (const name of browsers) {
      for (const js of [true, false]) {
        const { b, ctx } = await context(name, { javaScriptEnabled: js });
        const p = await ctx.newPage();
        for (const width of [1440, 390]) {
          const where = `log-filter-help ${name} ${js ? 'js' : 'no-js'} ${width}`;
          try {
            await p.setViewportSize({ width, height: 800 });
            await p.goto(srv.base + '/log/HEAD', { waitUntil: 'load' });
            const btn = p.locator('.filter-help > .helpbtn');
            const has = await btn.count() === 1;
            if (!has) { r.fail(`${where}: no ? button`); continue; }
            const label = await btn.getAttribute('aria-label');
            const box = await p.evaluate(() => {
              const f = document.querySelector('.filter').getBoundingClientRect();
              const q = document.querySelector('#logq').getBoundingClientRect();
              const s = document.querySelector('.filter-help').getBoundingClientRect();
              return { sameRow: Math.abs(s.top - q.top) < 20, right: s.left >= q.right - 1,
                inside: s.right <= f.right + 1 };
            });
            await btn.click();
            const shown = await p.evaluate(() => {
              const h = document.querySelector('.filter-pop');
              const t = h.textContent;
              const rc = h.getBoundingClientRect();
              return { vis: h.getClientRects().length > 0, text: t,
                more: !!h.querySelector('a[href*="#find-commits"]'),
                fits: rc.left >= 0 && rc.right <= innerWidth,
                sideways: document.documentElement.scrollWidth - innerWidth };
            });
            const lists = names.every((n) => shown.text.includes(n + (n === 'merge' ? '' : ':')));
            let closed = true;
            if (js) {
              await p.keyboard.press('Escape');
              const esc = await p.evaluate(() => {
                const d = /** @type {HTMLDetailsElement} */ (
                  document.querySelector('.filter-help'));
                return { open: d.open, focus: document.activeElement?.closest('summary') !== null };
              });
              await btn.click();
              await p.mouse.click(5, 700);
              const out = await p.evaluate(() =>
                /** @type {HTMLDetailsElement} */ (document.querySelector('.filter-help')).open);
              closed = !esc.open && esc.focus && !out;
            }
            const ok = label === 'Filter help' && box.sameRow && box.right && box.inside
              && shown.vis && lists && shown.more && shown.fits && shown.sideways <= 0 && closed;
            if (ok) r.ok(`${where}: ? opens the help with every keyword`);
            else {
              const got = { label, box, shown: { ...shown, text: '' }, lists, closed };
              r.fail(`${where}: ${JSON.stringify(got)}`);
            }
          } catch (e) { r.fail(`${where}: ${e.message.split('\n')[0]}`); }
        }
        await b.close();
      }
    }
  }
  return { logFilter, logFilterHelp };
}
