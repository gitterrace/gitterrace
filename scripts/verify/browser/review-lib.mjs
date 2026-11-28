// The review order panel of the working tree page. Imported by browser-check.mjs, not run alone.
//
// A repository with a small edit and a large one. In each browser:
// - the panel lists the files riskiest first and every row has a "Why" with its points
// - "]" walks the diff files in the panel's order, not the page's
// A screenshot goes to $TMPDIR/gt-review-order-<browser>.png.
import { mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { serve, sh, commitAll } from '../../lib/browser-lib.mjs';

function makeRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-review-'));
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  for (const n of ['a', 'b']) writeFileSync(join(dir, `${n}.txt`), 'one\ntwo\n');
  commitAll(dir, 'init');
  writeFileSync(join(dir, 'a.txt'), 'one\ntwo\nx\n');
  writeFileSync(join(dir, 'b.txt'), `one\ntwo\n${'y\n'.repeat(40)}`);
  return dir;
}

/** @param {any} env what browser-check.mjs set up (r, browsers, bin, cfg, context) */
export function checks(env) {
  const { r, browsers, bin, cfg, context } = env;
  const say = (ok, good, bad) => (ok ? r.ok(good) : r.fail(bad));
  const WAIT = { timeout: 15000 };

  async function reviewOrderWalk() {
    for (const name of browsers) {
      const dir = makeRepo();
      const srv = await serve(bin, dir, cfg);
      const w = `review-order-walk ${name}`;
      try {
        const { b, ctx } = await context(name);
        try {
          await ctx.addCookies([await srv.owner()]);
          const p = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
          await p.goto(`${srv.base}/worktree`, { waitUntil: 'load' });
          await p.waitForSelector('[data-rplan] [data-rplan-path]', WAIT);
          const order = await p.$$eval('[data-rplan-path]',
            (l) => l.map((x) => /** @type {HTMLElement} */ (x).dataset.rplanPath));
          say(order.length === 2, `${w}: the panel lists both files`, `${w}: rows ${order}`);
          const why = await p.$$eval('[data-rplan-path] .rplan-why li',
            (l) => l.map((x) => x.textContent));
          say(why.some((t) => /^Churn \d+ of 30$/.test(t || '')),
            `${w}: the points behind a place are inspectable`, `${w}: why was ${why}`);
          await p.keyboard.press(']');
          const first = await p.evaluate(() => /** @type {HTMLElement|null} */ (
            document.activeElement?.closest('section.file'))?.dataset.path);
          say(first === order[0], `${w}: "]" starts at the riskiest file`,
            `${w}: "]" focused ${first}, want ${order[0]}`);
          await p.keyboard.press(']');
          const second = await p.evaluate(() => /** @type {HTMLElement|null} */ (
            document.activeElement?.closest('section.file'))?.dataset.path);
          say(second === order[1], `${w}: "]" goes on in the panel order`,
            `${w}: second "]" focused ${second}, want ${order[1]}`);
          await p.evaluate(() => window.scrollTo(0, 0));
          await p.waitForFunction(() => document.getAnimations().length === 0, null, WAIT);
          await p.screenshot({ path: join(tmpdir(), `gt-review-order-${name}.png`) });
          await p.close();
        } finally { await b.close(); }
      } catch (e) {
        r.fail(`${w}: ${e && e.stack}`);
      } finally {
        srv.stop();
        rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
      }
    }
  }
  // Two folders and a changed manifest: the panel opens with chapters per folder and names
  // the config change in the Why list of the manifest.
  async function reviewChapters() {
    for (const name of browsers) {
      const dir = mkdtempSync(join(tmpdir(), 'gt-chapters-'));
      sh(dir, 'git', 'init', '-q', '-b', 'main');
      mkdirSync(join(dir, 'app'));
      mkdirSync(join(dir, 'lib'));
      writeFileSync(join(dir, 'app', 'a.txt'), 'one\n');
      writeFileSync(join(dir, 'lib', 'b.txt'), 'one\n');
      writeFileSync(join(dir, 'Cargo.toml'), '[package]\n');
      commitAll(dir, 'init');
      writeFileSync(join(dir, 'app', 'a.txt'), 'one\ntwo\n');
      writeFileSync(join(dir, 'lib', 'b.txt'), 'one\ntwo\n');
      writeFileSync(join(dir, 'Cargo.toml'), '[package]\nname = "x"\n');
      const srv = await serve(bin, dir, cfg);
      const w = `review-chapters ${name}`;
      try {
        const { b, ctx } = await context(name);
        try {
          await ctx.addCookies([await srv.owner()]);
          const p = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
          await p.goto(`${srv.base}/worktree`, { waitUntil: 'load' });
          await p.waitForSelector('[data-rplan-chapter]', WAIT);
          const titles = await p.$$eval('[data-rplan-chapter]',
            (l) => l.map((x) => /** @type {HTMLElement} */ (x).dataset.rplanChapter));
          say(['app', 'lib', 'Top level'].every((t) => titles.includes(t)),
            `${w}: one chapter per folder`, `${w}: chapters ${titles}`);
          const rows = await p.$$eval('[data-rplan-path]', (l) => l.length);
          say(rows === 3, `${w}: each file is listed once`, `${w}: ${rows} rows, want 3`);
          const why = await p.$$eval('[data-rplan-path="Cargo.toml"] .rplan-why li',
            (l) => l.map((x) => x.textContent));
          say(why.some((t) => /^Config change:/.test(t || '')),
            `${w}: the manifest carries its signal`, `${w}: why was ${why}`);
          await p.close();
        } finally { await b.close(); }
      } catch (e) {
        r.fail(`${w}: ${e && e.stack}`);
      } finally {
        srv.stop();
        rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
      }
    }
  }
  return { reviewOrderWalk, reviewChapters };
}
