// The docs menus on a narrow screen, and the bar that must not move. Imported by browser-check.mjs.
import { serve, pages } from './sitebar-lib.mjs';
import { launch } from '../../lib/browser-lib.mjs';
import { checks as steadyChecks } from './barsteady-lib.mjs';

/** @param {any} env what browser-check.mjs set up (pw, r, browsers, ...) */
export function checks(env) {
  const { pw, r, browsers } = env;

  // Every docs page at 390 px: exactly one docs navigation control in view (the bar's Docs
  // menu, no second "Docs" in the page or the bar; the footer's links are not menus), "On
  // this page" at least 16 px above the h1, and both menus open with Enter on their focused
  // button and close with Escape, focus back on the button. Each control is at least 44 px tall.
  async function docMenus() {
    const { server, url } = await serve();
    const all = pages().filter((p) => p.startsWith('/docs/') && p !== '/docs/');
    try {
      for (const name of browsers) {
        const b = await launch(pw, name);
        const ctx = await b.newContext({ viewport: { width: 390, height: 900 } });
        const p = await ctx.newPage();
        const bad = [];
        let outlined = 0;
        for (const path of all) {
          await p.goto(url + path, { waitUntil: 'load' });
          const why = [];
          const sel = 'header a, header summary, main a, main summary';
          const docs = await p.evaluate((q) => [...document.querySelectorAll(q)]
            .filter((e) => e.textContent.trim() === 'Docs' && e.getClientRects().length > 0
              && e.getBoundingClientRect().width > 0 && getComputedStyle(e).visibility !== 'hidden')
            .map((e) => e.tagName), sel);
          if (docs.length !== 1 || docs[0] !== 'SUMMARY') {
            why.push(`docs controls in view: ${JSON.stringify(docs)}`);
          }
          const gap = await p.evaluate(() => {
            const s = [...document.querySelectorAll('main summary')]
              .find((e) => e.textContent.trim() === 'On this page');
            const h = document.querySelector('main h1');
            if (!s || !h) return null;
            return { gap: h.getBoundingClientRect().top - s.getBoundingClientRect().bottom,
              tall: s.getBoundingClientRect().height };
          });
          if (gap) {
            outlined++;
            if (gap.gap < 16) why.push(`On this page ${Math.round(gap.gap)} px above the h1`);
            if (gap.tall < 44) why.push(`On this page ${Math.round(gap.tall)} px tall`);
          }
          const sums = ['header.top summary', 'main summary'];
          for (const sel of gap ? sums : sums.slice(0, 1)) {
            const s = await p.$(sel);
            if (!s) { why.push(`no ${sel}`); continue; }
            const h = await s.evaluate((e) => e.getBoundingClientRect().height);
            if (h < 44) why.push(`${sel} ${Math.round(h)} px tall`);
            await s.focus();
            await p.keyboard.press('Enter');
            const open = await s.evaluate((e) => e.parentElement.open);
            await p.keyboard.press('Escape');
            const after = await s.evaluate((e) => ({ open: e.parentElement.open,
              focus: document.activeElement === e }));
            if (!open) why.push(`${sel} does not open by Enter`);
            if (after.open || !after.focus) why.push(`${sel} Escape: ${JSON.stringify(after)}`);
          }
          if (why.length) bad.push(`${path}: ${why.join(', ')}`);
        }
        const where = `doc-menus ${name} 390`;
        if (!outlined) bad.push('no page has an On this page menu');
        if (bad.length) r.fail(`${where}: ${bad.slice(0, 6).join('; ')}`);
        else r.ok(`${where}: one Docs control on ${all.length} pages, ${outlined} outlines clear`
          + ' of the h1, both menus open and close by keyboard');
        await ctx.close();
        await b.close();
      }
    } finally { server.close(); }
  }

  return { docMenus, ...steadyChecks(env) };
}
