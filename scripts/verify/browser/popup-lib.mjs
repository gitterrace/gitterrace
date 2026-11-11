// Anchored popups stay usable when the page is scrolled. Imported by browser-check.mjs,
// not run alone.

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context } = env;

  // Scroll a long page to the bottom, so the header's branch button and the page's action
  // button are off screen, then press `b` and `.`: each popup lies fully inside the viewport,
  // its focus is in the popup and a typed letter reaches the filter field.
  async function popupPlace() {
    for (const name of browsers) {
      const { b, ctx } = await context(name);
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
      await b.close();
    }
  }
  return { popupPlace };
}
