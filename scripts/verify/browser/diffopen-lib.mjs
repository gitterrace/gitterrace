// The diff-open-all check. Imported by browser-check.mjs, not run on its own.
//
// A commit with one small file and one file over display.diff_open_lines. In each browser:
// - the small file is open, the big one is folded, and the bar says "1 of 2 files open"
// - Open all loads the folded file through the same loader as its own link
// - once nothing is folded the Open all button is disabled
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { eachBrowser, serve, sh, commitAll } from '../../lib/browser-lib.mjs';

function makeRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-openall-'));
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  writeFileSync(join(dir, 'small.txt'), 'one\ntwo\n');
  const rows = Array.from({ length: 1200 }, (_, i) => `row ${i}\n`);
  writeFileSync(join(dir, 'big.txt'), rows.join(''));
  commitAll(dir, 'init');
  return dir;
}

/** @param {any} env what browser-check.mjs set up (r, browsers, bin, cfg, context) */
export function checks(env) {
  const { r, browsers, bin, cfg, context } = env;
  const say = (ok, good, bad) => (ok ? r.ok(good) : r.fail(bad));

  async function diffOpenAll() {
    const dir = makeRepo();
    const srv = await serve(bin, dir, cfg);
    try {
      const id = sh(dir, 'git', 'rev-parse', 'HEAD');
      await eachBrowser(browsers, context, async (ctx, name) => {
        const where = `diff-open-all ${name}`;
        const p = await ctx.newPage({ viewport: { width: 1440, height: 900 } });
        await p.goto(`${srv.base}/commit/${id}`, { waitUntil: 'load' });
        const text = await p.$eval('.open-count', (e) => e.textContent.trim());
        say(/^1 of 2 files open, 1 folded/.test(text),
          `${where}: the bar says "${text}"`, `${where}: bar text "${text}"`);
        const folded = await p.$$eval('[data-load-diff]', (b) => b.length);
        say(folded === 1, `${where}: one file is folded`, `${where}: ${folded} folded files`);
        await p.click('[data-open-all]');
        const done = await p.waitForFunction(() => !document.querySelector('[data-load-diff]')
          && document.querySelectorAll('section.file .diffwrap').length === 2, null,
        { timeout: 10000 }).then(() => true, () => false);
        say(done, `${where}: Open all loads every folded file`, `${where}: a file stayed folded`);
        const off = await p.$eval('[data-open-all]', (b) => b.disabled);
        say(off, `${where}: Open all is spent`, `${where}: Open all is still enabled`);
        await p.close();
      });
    } catch (e) {
      r.fail(`diff-open-all: ${e && e.stack}`);
    } finally {
      srv.stop();
      rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    }
  }
  return { diffOpenAll };
}
