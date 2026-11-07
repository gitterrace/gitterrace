// The "On this page" filter on every page that has the menu. Imported by
// scripts/browser-check.mjs, not run on its own.
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { sh } from './browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, dir, ...) */
export function checks(env) {
  const { srv, r, browsers, context, dir } = env;

  // The filter shows, narrows the list on typing, and Escape closes the menu and returns focus to
  // its button, on a Markdown page and on an insights page alike.
  async function outlineFilter() {
    writeFileSync(join(dir, 'filter-outline.md'), '# Doc\n\n## Alpha\n\ntext\n\n## Beta\n\ntext\n\n## Gamma\n\ntext\n');
    sh(dir, 'git', 'add', '-A');
    sh(dir, 'git', 'commit', '-q', '-m', 'docs: filter outline fixture');
    await new Promise((ok) => setTimeout(ok, 600));
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      const p = await ctx.newPage();
      await p.setViewportSize({ width: 900, height: 800 });
      for (const path of ['/blob/HEAD/filter-outline.md', '/stats/hotspots']) {
        const where = `outline-filter ${name} ${path}`;
        try {
          await p.goto(srv.base + path, { waitUntil: 'load' });
          await p.click('details.outline summary');
          await p.waitForTimeout(200);
          const v = await p.evaluate(() => {
            const d = document.querySelector('details.outline');
            const q = /** @type {HTMLInputElement} */ (d.querySelector('.outline-q'));
            return { shown: !q.hidden, focused: document.activeElement === q, total: d.querySelectorAll('li').length };
          });
          await p.keyboard.type('zzzz-no-such-heading');
          const after = await p.evaluate(() => [...document.querySelectorAll('details.outline li')].filter((l) => !(/** @type {HTMLElement} */ (l)).hidden).length);
          await p.keyboard.press('Escape');
          const esc = await p.evaluate(() => ({ closed: !(/** @type {HTMLDetailsElement} */ (document.querySelector('details.outline'))).open, onButton: document.activeElement === document.querySelector('details.outline summary') }));
          if (v.shown && v.focused && v.total > 0 && after === 0 && esc.closed && esc.onButton) r.ok(`${where}: filter shown and focused, hides items, Escape returns to the button`);
          else r.fail(`${where}: ${JSON.stringify({ ...v, after, ...esc })}`);
        } catch (e) { r.fail(`${where}: ${e.message.split('\n')[0]}`); }
      }
      await b.close();
    }
  }
  return { outlineFilter };
}
