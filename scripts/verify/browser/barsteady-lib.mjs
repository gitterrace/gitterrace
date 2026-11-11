// No button in the site's top bar moves. Imported by scripts/verify/browser/browser-check.mjs.
import { serve, pages } from './sitebar-lib.mjs';
import { launch } from '../../lib/browser-lib.mjs';

const WIDTHS = [390, 600, 768, 1024, 1280];
const SLACK = 0.5;
const ITEMS = ['logo', 'Docs', 'Demo', 'GitHub', 'theme'];

/** The box of each bar item in the page now, by name, in page coordinates. */
function boxes() {
  const bar = document.querySelector('header.top');
  const seen = (e) => e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden';
  const text = (t) => [...bar.querySelectorAll('a, summary')]
    .find((e) => seen(e) && e.textContent.trim() === t);
  const found = { logo: bar.querySelector('.brand'), Docs: text('Docs'), Demo: text('Demo'),
    GitHub: bar.querySelector('.gh'), theme: bar.querySelector('#theme') };
  const out = {};
  for (const [k, e] of Object.entries(found)) {
    if (!e) continue;
    const b = e.getBoundingClientRect();
    out[k] = [b.x + scrollX, b.y + scrollY, b.width, b.height];
  }
  return out;
}

/** @param {any} env what browser-check.mjs set up (pw, r, browsers, ...) */
export function checks(env) {
  const { pw, r, browsers } = env;

  // At 390, 600, 768, 1024 and 1280 px, light and dark, on the landing page and every docs
  // page: each bar item has the same box on every page, with the Docs menu open or closed,
  // under hover, under keyboard focus and after each press of the theme button.
  async function barSteady() {
    const { server, url } = await serve();
    const all = pages().filter((p) => p === '/' || (p.startsWith('/docs/') && p !== '/docs/'));
    try {
      for (const name of browsers) {
        const b = await launch(pw, name);
        for (const scheme of ['light', 'dark']) {
          for (const width of WIDTHS) {
            const viewport = { width, height: 900 };
            const ctx = await b.newContext({ colorScheme: scheme, viewport });
            const p = await ctx.newPage();
            const bad = [];
            let ref = null;
            let taken = 0;
            const take = async (page, state) => {
              const got = await p.evaluate(boxes);
              taken++;
              ref ??= got;
              for (const k of ITEMS) {
                const a = ref[k];
                const c = got[k];
                if (!a !== !c) bad.push(`${page} ${state}: ${k} ${c ? 'appears' : 'is gone'}`);
                else if (a && a.some((v, i) => Math.abs(v - c[i]) > SLACK)) {
                  bad.push(`${page} ${state}: ${k} ${a.map(Math.round)} -> ${c.map(Math.round)}`);
                }
              }
            };
            for (const path of all) {
              await p.goto(url + path, { waitUntil: 'load' });
              await take(path, 'closed');
              const menu = await p.$('header.top details.docsmenu > summary');
              const open = menu && await menu.isVisible();
              if (open) {
                await menu.click();
                await take(path, 'menu open');
                await menu.click();
                await take(path, 'menu closed again');
              }
              for (const sel of ['.brand', 'header.top a:not(.brand):not(.gh)', '.gh', '#theme']) {
                const e = await p.$(sel);
                if (e && await e.isVisible()) { await e.hover(); await take(path, `hover ${sel}`); }
              }
              await p.mouse.move(width - 1, 800);
              await p.evaluate(() => /** @type {any} */ (document.activeElement)?.blur());
              for (let i = 0; i < 6; i++) {
                await p.keyboard.press('Tab');
                const inBar = await p.evaluate(
                  () => !!document.activeElement?.closest('header.top'));
                if (inBar) await take(path, `focus ${i}`);
              }
              for (let i = 0; i < 3; i++) {
                await p.click('#theme');
                await take(path, `theme press ${i + 1}`);
              }
            }
            const where = `bar-steady ${name} ${scheme} ${width}`;
            if (bad.length) r.fail(`${where}: ${bad.length} moved: ${bad.slice(0, 4).join('; ')}`);
            else r.ok(`${where}: bar items stay put on ${all.length} pages, ${taken} states`);
            await ctx.close();
          }
        }
        await b.close();
      }
    } finally { server.close(); }
  }

  return { barSteady };
}
