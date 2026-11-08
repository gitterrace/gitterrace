// The repositories home following the folder it serves. Imported by scripts/browser-check.mjs,
// not run alone.
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { sh, serve, WATCH_BOUND_MS } from './browser-lib.mjs';

/** A one-commit repository in `dir`. */
function small(dir) {
  mkdirSync(dir, { recursive: true });
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  writeFileSync(join(dir, 'a.txt'), 'a\n');
  sh(dir, 'git', 'add', '-A');
  sh(dir, 'git', 'commit', '-q', '-m', 'feat: first');
}

/** @param {any} env what browser-check.mjs set up (r, browsers, context, bin, cfg, ...) */
export function checks(env) {
  const { r, browsers, context, bin, cfg } = env;

  // A server on a folder of repositories, its home open: a repository folder made under the
  // folder shows as a row without a reload, and goes again once deleted. The page is the same
  // document throughout (a marker set on it survives), and typed filter text stays.
  async function homeLive() {
    for (const name of browsers) {
      const where = `home-live ${name}`;
      const top = mkdtempSync(join(tmpdir(), 'gt-home-'));
      small(join(top, 'alpha'));
      small(join(top, 'beta'));
      const srv = await serve(bin, top, cfg);
      const { b, ctx } = await context(name);
      try {
        const p = await ctx.newPage();
        await p.goto(srv.origin + '/', { waitUntil: 'load' });
        // A mark a reload would wipe: still there means the page never reloaded.
        await p.evaluate(() => { document.documentElement.dataset.gtSame = '1'; });
        const names = () => p.$$eval('.reporow',
          // A served name may carry a suffix that keeps it unique: compare the folder part.
          (l) => l.map((e) => String(/** @type {HTMLElement} */ (e).dataset.name)
            .replace(/-[0-9a-f]{6}$/, '')));
        const before = await names();
        const q = await p.$('#repoq');
        if (q) await q.fill('a');
        small(join(top, 'gamma'));
        const wait = { timeout: WATCH_BOUND_MS };
        const gamma = '.reporow[data-name^="gamma"]';
        const shown = await p.waitForFunction((s) => !!document.querySelector(s), gamma, wait)
          .then(() => true, () => false);
        const added = await names();
        // The server may still be writing under the folder: retry the removal.
        rmSync(join(top, 'gamma'), { recursive: true, force: true, maxRetries: 10 });
        const left = await p.waitForFunction((s) => !document.querySelector(s), gamma, wait)
          .then(() => true, () => false);
        const after = await names();
        const same = await p.evaluate(() => document.documentElement.dataset.gtSame === '1');
        const typed = q ? await p.inputValue('#repoq') : 'a';
        const note = await p.evaluate(() => {
          const n = document.getElementById('livenote');
          return !!n && !n.hidden;
        });
        const two = 'alpha,beta';
        const ok = shown && left && same && typed === 'a' && !note
          && before.join() === two && added.includes('gamma') && after.join() === two;
        const facts = { before, shown, added, left, after, same, typed, note };
        if (ok) r.ok(`${where}: a new folder shows, a deleted one goes, no reload, filter kept`);
        else r.fail(`${where}: ${JSON.stringify(facts)}`);
      } catch (e) {
        r.fail(`${where}: ${e.message.split('\n')[0]}`);
      } finally {
        await b.close();
        srv.stop();
        rmSync(top, { recursive: true, force: true });
      }
    }
  }
  return { homeLive };
}
