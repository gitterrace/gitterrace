// The Send to window and image diffs, as a reader meets them. Imported by

import { eachBrowser } from '../../lib/browser-lib.mjs';
// scripts/verify/browser/browser-check.mjs, not run on its own.

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context } = env;

  // The window names itself and its fields, closes with Cancel, and on a phone sits along the
  // bottom edge with its footer in view.
  async function sendDialog() {
    await eachBrowser(browsers, context, async (ctx, name) => {
      const p = await ctx.newPage();
      for (const width of [1440, 390]) {
        await p.setViewportSize({ width, height: 800 });
        await p.goto(srv.base + '/blob/HEAD/a.txt', { waitUntil: 'load' });
        await p.waitForTimeout(300);
        await p.keyboard.press('.');
        await p.keyboard.press('s');
        await p.waitForSelector('.send-ctx');
        const v = await p.evaluate(() => {
          const box = document.querySelector('.ov.send');
          const named = box.closest('[aria-labelledby]');
          const head = document.getElementById(named?.getAttribute('aria-labelledby') || '');
          const lab = (id) => document.querySelector(`label[for="${id}"]`)?.textContent || '';
          const foot = box.querySelector('.send-foot').getBoundingClientRect();
          return {
            head: head ? head.textContent : '',
            title: lab(box.querySelector('.send-title').id),
            notes: lab(box.querySelector('.send-ctx').id),
            cancel: !!box.querySelector('button.send-cancel'),
            hint: box.querySelector('.send-hint')?.textContent || '',
            bottom: Math.round(box.getBoundingClientRect().bottom),
            left: Math.round(box.getBoundingClientRect().left),
            footIn: foot.bottom <= innerHeight + 1,
            h: innerHeight,
          };
        });
        const where = `send-dialog ${name} ${width}`;
        if (v.head === 'Send to' && v.title === 'Title' && v.notes === 'Notes' && v.cancel) r.ok(`${where}: head, labels and Cancel`);
        else r.fail(`${where}: ${JSON.stringify(v)}`);
        if (width === 390) {
          if (v.bottom >= v.h - 1 && v.left <= 0 && v.footIn) r.ok(`${where}: a bottom sheet, footer in view`);
          else r.fail(`${where}: not a bottom sheet ${JSON.stringify(v)}`);
        } else if (/Enter to send$/.test(v.hint)) r.ok(`${where}: the shortcut in words`);
        else r.fail(`${where}: hint ${JSON.stringify(v.hint)}`);
        await p.click('.send-cancel');
        if (await p.$('.ov.send')) r.fail(`${where}: Cancel left the window open`);
        else r.ok(`${where}: Cancel closes`);
      }
    });
  }
  return { sendDialog };
}
