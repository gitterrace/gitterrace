// The file rail check. Imported by browser-check.mjs, not run on its own.
//
// A commit with three files. At 1440px wide:
// - the rail lists the three files and stays in the window after the page scrolls
// - a click on a row lands on that file's section and marks the row current
// - ticking Viewed on a file marks its rail row
// At 900px wide the rail is not shown and the files card is.
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { eachBrowser, serve, sh, commitAll, ready } from '../../lib/browser-lib.mjs';

function makeRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-rail-'));
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  for (const n of ['a', 'b', 'c']) {
    const rows = Array.from({ length: 120 }, (_, i) => `${n} ${i}\n`);
    writeFileSync(join(dir, `${n}.txt`), rows.join(''));
  }
  commitAll(dir, 'init');
  return dir;
}

/** @param {any} env what browser-check.mjs set up (r, browsers, bin, cfg, context) */
export function checks(env) {
  const { r, browsers, bin, cfg, context } = env;
  const say = (ok, good, bad) => (ok ? r.ok(good) : r.fail(bad));

  async function fileRail() {
    const dir = makeRepo();
    const srv = await serve(bin, dir, cfg);
    try {
      const id = sh(dir, 'git', 'rev-parse', 'HEAD');
      await eachBrowser(browsers, context, async (ctx, name) => {
        const where = `file-rail ${name}`;
        const p = await ctx.newPage({ viewport: { width: 1440, height: 900 } });
        await p.goto(`${srv.base}/commit/${id}`, { waitUntil: 'load' });
        await ready(p, 'diffs-rail');
        const n = await p.$$eval('[data-rail] li', (l) => l.length);
        say(n === 3, `${where}: one row per file`, `${where}: ${n} rows`);
        await p.evaluate(() => window.scrollTo(0, 600));
        const top = await p.$eval('[data-rail]', (e) => e.getBoundingClientRect().top);
        say(top >= 0 && top < 200, `${where}: the rail stays in view`, `${where}: rail at ${top}`);
        await p.click('[data-rail] li[data-f="f2"] a');
        const cur = await p.waitForFunction(() =>
          document.querySelector('[data-rail] li[aria-current]')
            ?.getAttribute('data-f') === 'f2', null,
        { timeout: 5000 }).then(() => true, () => false);
        say(cur, `${where}: a click marks the file current`, `${where}: no current row`);
        await p.click('#f2 input.viewed');
        const seen = await p.waitForFunction(() =>
          document.querySelector('[data-rail] li[data-f="f2"]')?.hasAttribute('data-seen'),
        null, { timeout: 5000 }).then(() => true, () => false);
        say(seen, `${where}: a viewed file is marked in the rail`, `${where}: row not marked`);
        await p.setViewportSize({ width: 900, height: 900 });
        const shown = await p.$eval('[data-rail]', (e) => getComputedStyle(e).display !== 'none');
        say(!shown, `${where}: the rail is hidden when narrow`, `${where}: rail shown at 900px`);
        await p.close();
      });
    } catch (e) {
      r.fail(`file-rail: ${e && e.stack}`);
    } finally {
      srv.stop();
      rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    }
  }
  return { fileRail };
}
