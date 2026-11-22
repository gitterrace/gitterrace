// Anchored popups stay usable when the page is scrolled. Imported by browser-check.mjs,

import { eachBrowser } from '../../lib/browser-lib.mjs';
// not run alone.

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context } = env;

  // Scroll a long page to the bottom, so the header's branch button and the page's action
  // button are off screen, then press `b` and `.`: each popup lies fully inside the viewport,
  // its focus is in the popup and a typed letter reaches the filter field.
  async function popupPlace() {
    await eachBrowser(browsers, context, async (ctx, name) => {
      const p = await ctx.newPage();
      const where = `popup-place ${name}`;
      try {
        await p.setViewportSize({ width: 1200, height: 700 });
        await p.goto(`${srv.base}/log/HEAD`, { waitUntil: 'load' });
        await p.evaluate(() => {
          const f = document.createElement('div');
          f.style.height = '3000px';
          document.querySelector('main, .wrap, body').appendChild(f);
          scrollTo(0, document.documentElement.scrollHeight);
        });
        await p.waitForTimeout(200);
        const inside = () => p.evaluate(() => {
          const o = document.querySelector('.ov');
          if (!o) return null;
          const q = o.getBoundingClientRect();
          const a = document.activeElement;
          return {
            ok: q.top >= 0 && q.left >= 0 && q.height > 0
              && q.bottom <= innerHeight && q.right <= innerWidth,
            rect: [q.left, q.top, q.right, q.bottom].map(Math.round).join(','),
            inBox: o.contains(a), tag: a && a.tagName,
          };
        });
        for (const [key, input] of [['b', true], ['.', false]]) {
          await p.keyboard.press(key);
          await p.waitForTimeout(150);
          const m = await inside();
          if (!m) { r.fail(`${where}: ${key}: no popup opened`); continue; }
          if (!m.ok) {
            const vp = await p.evaluate(() => `${innerWidth}x${innerHeight}`);
            r.fail(`${where}: ${key}: popup at ${m.rect} lies outside the ${vp} viewport`);
          }
          else if (!m.inBox) r.fail(`${where}: ${key}: focus is not inside the popup`);
          else if (input) {
            await p.keyboard.type('ma');
            const v = await p.evaluate(() =>
              /** @type {HTMLInputElement | null} */ (document.querySelector('.ov input'))?.value);
            if (v === 'ma') r.ok(`${where}: ${key}: popup inside the viewport, filter types`);
            else r.fail(`${where}: ${key}: filter holds ${JSON.stringify(v)}`);
          } else r.ok(`${where}: ${key}: menu inside the viewport with focus`);
          await p.keyboard.press('Escape');
          await p.waitForTimeout(100);
        }
      } catch (e) {
        r.fail(`${where}: ${e.message.split('\n')[0]}`);
      }
    });
  }
  // The finder, the sheet and Send to are a modal <dialog>; the pickers and the action menu
  // are a [popover]. A box in a plain backdrop div fails, whichever overlay it is.
  async function overlayNative() {
    /** @type {[string[], string][]} */
    const want = [[['t'], 'dialog'], [['?'], 'dialog'], [['.', 's'], 'dialog'],
      [['b'], 'popover'], [['.'], 'popover']];
    await eachBrowser(browsers, context, async (ctx, name) => {
      const p = await ctx.newPage();
      await p.goto(srv.base + '/', { waitUntil: 'load' });
      const bad = [];
      for (const [keys, kind] of want) {
        for (const k of keys) await p.keyboard.press(k);
        await p.waitForTimeout(150);
        const got = await p.evaluate(() => {
          const box = document.activeElement?.closest('.ov');
          if (!box) return 'none';
          if (box.closest('dialog:modal')) return 'dialog';
          if (box.matches(':popover-open')) return 'popover';
          return box.tagName.toLowerCase();
        });
        if (got !== kind) bad.push(`${keys.join('+')}: ${got}, want ${kind}`);
        await p.keyboard.press('Escape');
        await p.waitForTimeout(100);
      }
      if (!bad.length) r.ok(`overlay-native ${name}: dialogs are modal, menus are popovers`);
      else r.fail(`overlay-native ${name}: ${bad.join('; ')}`);
    });
  }

  // With scripts off, Code and Preview still switch the README panes: radios and CSS only.
  async function panedNoJs() {
    for (const name of browsers) {
      const { b } = await context(name);
      const ctx = await b.newContext({ javaScriptEnabled: false });
      const p = await ctx.newPage();
      await p.goto(`${srv.base}/blob/HEAD/README.md`, { waitUntil: 'load' });
      const shown = () => p.evaluate(() => [...document.querySelectorAll('[data-pane]')]
        .filter((e) => e.checkVisibility()).map((e) => e.getAttribute('data-pane')).join(','));
      const first = await shown();
      await p.click('[data-show=source]');
      const code = await shown();
      await p.click('[data-show=rendered]');
      const back = await shown();
      if (first === 'rendered' && code === 'source' && back === 'rendered') {
        r.ok(`paned-nojs ${name}: Code and Preview switch without scripts`);
      } else r.fail(`paned-nojs ${name}: ${JSON.stringify({ first, code, back })}`);
      await b.close();
    }
  }
  return { popupPlace, overlayNative, panedNoJs };
}
