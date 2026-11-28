// Moving old localStorage viewed marks to the server. Imported by browser-check.mjs, not run alone.
//
// A repository with two uncommitted files. Marks kept in localStorage under the page address are
// seeded, then the page is loaded again so it migrates them. In each browser:
// - a rejected POST keeps every entry and says the marks could not be moved
// - a saved POST drops the confirmed entries and the marks show after a reload
// - an entry for a file not on the page stays, and one whose content changed stays unmarked
// - two tabs migrating at once end with the marks on the server and nothing lost
// - a click made while the migration request is held (then released) is still there after a
//   reload: tick and untick a file, and the server must not end up holding the older mark
// - a migration held in one tab while a second tab ticks and unticks the same file ends with the
//   file unmarked on the server after a reload (an unmark is kept, not forgotten)
// A second defect the same checks catch: a retry that drops the entries of a different page.
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { serve, sh, commitAll } from '../../lib/browser-lib.mjs';

function makeRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-viewedmig-'));
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  for (const n of ['a', 'b']) writeFileSync(join(dir, `${n}.txt`), 'one\ntwo\n');
  commitAll(dir, 'init');
  writeFileSync(join(dir, 'a.txt'), 'one\ntwo\nfirst edit\n');
  writeFileSync(join(dir, 'b.txt'), 'one\ntwo\nsecond edit\n');
  return dir;
}

/** @param {any} env what browser-check.mjs set up (r, browsers, bin, cfg, context) */
export function checks(env) {
  const { r, browsers, bin, cfg, context } = env;
  const say = (ok, good, bad) => (ok ? r.ok(good) : r.fail(bad));
  const WAIT = { timeout: 15000 };
  const KEY = '/worktree';

  /** The local content tag of each file on the page. Runs in the page. */
  const tags = () => Object.fromEntries([...document.querySelectorAll('section.file[data-path]')]
    .map((s) => {
      const d = /** @type {HTMLElement} */ (s).dataset;
      return [d.path, d.h];
    }));
  const stored = (p) => p.evaluate(() => {
    const k = Object.keys(localStorage).find((x) => x.startsWith('gt-viewed:'));
    return k ? JSON.parse(localStorage.getItem(k)) : null;
  });
  const count = (pg, n) => pg.waitForFunction(
    (k) => document.querySelectorAll('input.viewed:checked').length === k, n, WAIT);
  const ticked = (p) => p.evaluate(() => [...document.querySelectorAll('section.file[data-path]')]
    .filter((x) => /** @type {HTMLInputElement|null} */ (x.querySelector('input.viewed'))?.checked)
    .map((x) => /** @type {HTMLElement} */ (x).dataset.path).sort());

  /** Runs body(p, srv, w, ctx) on a fresh repository and server in each browser. */
  async function each(label, body) {
    for (const name of browsers) {
      const dir = makeRepo();
      const srv = await serve(bin, dir, cfg);
      const w = `viewed-migrate ${label} ${name}`;
      try {
        const { b, ctx } = await context(name);
        try {
          await ctx.addCookies([await srv.owner()]);
          const p = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
          await p.goto(`${srv.base}${KEY}`, { waitUntil: 'load' });
          await p.waitForSelector('[data-cpbar] .cpnote', { ...WAIT, state: 'attached' });
          await p.waitForSelector('section.file input.viewed', WAIT);
          await body(p, srv, w, ctx);
        } finally { await b.close(); }
      } catch (e) {
        r.fail(`${w}: ${e && e.stack}`);
      } finally {
        srv.stop();
        rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
      }
    }
  }

  async function seed(p, extra = {}) {
    const h = await p.evaluate(tags);
    await p.evaluate(
      ([m]) => localStorage.setItem(
        `gt-viewed:${location.pathname}${location.search}`, JSON.stringify(m)),
      [{ 'a.txt': h['a.txt'], 'b.txt': h['b.txt'], ...extra }]);
    return h;
  }
  const reload = async (p) => {
    await p.reload({ waitUntil: 'load' });
    await p.waitForSelector('section.file input.viewed', WAIT);
  };

  async function viewedMigrateRejected() {
    await each('rejected', async (p, srv, w) => {
      const h = await seed(p);
      await p.route('**/_viewed/all', (route) => route.fulfill({ status: 500, body: 'no' }));
      await reload(p);
      await p.waitForSelector('.cpnote:text("could not be moved")', WAIT);
      const left = await stored(p);
      say(left && left['a.txt'] === h['a.txt'] && left['b.txt'] === h['b.txt'],
        `${w}: a refused save keeps every local mark`,
        `${w}: storage after a refused save: ${JSON.stringify(left)}`);
      say(/again|retry|kept/i.test(await p.$eval('.cpnote', (e) => e.textContent)),
        `${w}: the note says the marks are kept for another try`,
        `${w}: the note has no retry wording`);
      await p.unroute('**/_viewed/all');
      await reload(p);
      await count(p, 2);
      say((await stored(p)) === null, `${w}: the next load moves them and drops the copies`,
        `${w}: storage after the retry: ${JSON.stringify(await stored(p))}`);
    });
  }

  async function viewedMigrateSaved() {
    await each('saved', async (p, srv, w) => {
      await seed(p);
      await reload(p);
      await count(p, 2);
      say((await stored(p)) === null, `${w}: a confirmed save drops the local entries`,
        `${w}: storage after a save: ${JSON.stringify(await stored(p))}`);
      await reload(p);
      await count(p, 2);
      say((await ticked(p)).join() === 'a.txt,b.txt',
        `${w}: the marks come from the server after a reload`,
        `${w}: ticked ${await ticked(p)}`);
    });
  }

  async function viewedMigrateKept() {
    await each('absent-and-changed', async (p, srv, w) => {
      const h = await seed(p, { 'gone.txt': 'abc' });
      await p.evaluate(([x]) => {
        const k = `gt-viewed:${location.pathname}${location.search}`;
        const m = JSON.parse(localStorage.getItem(k));
        m['b.txt'] = x;
        localStorage.setItem(k, JSON.stringify(m));
      }, ['changed-content-tag']);
      await reload(p);
      await count(p, 1);
      const left = await stored(p);
      say(left && left['gone.txt'] === 'abc' &&
        left['b.txt'] === 'changed-content-tag' && !('a.txt' in left),
        `${w}: an absent file and a changed file stay, the confirmed one goes`,
        `${w}: storage ${JSON.stringify(left)} (a.txt tag ${h['a.txt']})`);
      say((await ticked(p)).join() === 'a.txt', `${w}: only the matching file is marked`,
        `${w}: ticked ${await ticked(p)}`);
    });
  }

  async function viewedMigrateTwoTabs() {
    await each('two-tabs', async (p, srv, w, ctx) => {
      await seed(p);
      const q = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
      await Promise.all([reload(p), q.goto(`${srv.base}${KEY}`, { waitUntil: 'load' })]);
      await q.waitForSelector('section.file input.viewed', WAIT);
      await count(p, 2);
      await count(q, 2);
      say((await stored(p)) === null, `${w}: both tabs ended with the local copies gone`,
        `${w}: storage ${JSON.stringify(await stored(p))}`);
      await reload(p);
      await count(p, 2);
      say((await ticked(p)).join() === 'a.txt,b.txt', `${w}: the server holds both marks`,
        `${w}: ticked ${await ticked(p)}`);
      await q.close();
    });
  }

  async function viewedMigrateLaterClickWins() {
    await each('click-while-held', async (p, srv, w) => {
      await seed(p);
      /** @type {(v?: unknown) => void} */
      let release = () => {};
      const gate = new Promise((res) => { release = res; });
      /** @type {(v?: unknown) => void} */
      let held = () => {};
      const arrived = new Promise((res) => { held = res; });
      await p.route('**/_viewed/all', async (route) => {
        held(true);
        await gate;
        await route.continue();
      });
      let saved = 0;
      /** @type {(v?: unknown) => void} */
      let allSaved = () => {};
      const settled = new Promise((res) => { allSaved = res; });
      p.on('response', (x) => {
        const save = x.request().method() === 'POST' && /\/_viewed(\/all)?$/.test(x.url());
        if (save && ++saved === 3) allSaved(true);
      });
      await p.reload({ waitUntil: 'load' });
      await arrived;
      // The reader ticks a.txt and unticks it again while the migration is still in flight.
      const box = 'section.file[data-path="a.txt"] input.viewed';
      await p.click(box);
      await p.click(box);
      release();
      // The migration and both clicks have been answered: three saves, in whatever order.
      await settled;
      say(saved === 3, `${w}: the migration and both clicks were saved`, `${w}: ${saved} saves`);
      await p.unroute('**/_viewed/all');
      await reload(p);
      await count(p, 1);
      say((await ticked(p)).join() === 'b.txt',
        `${w}: the later untick survives a reload, the migrated b.txt stays`,
        `${w}: ticked after reload ${await ticked(p)}`);
    });
  }

  async function viewedMigrateOtherTabUnmark() {
    await each('other-tab-unmark', async (p, srv, w, ctx) => {
      await seed(p);
      /** @type {(v?: unknown) => void} */
      let release = () => {};
      const gate = new Promise((res) => { release = res; });
      /** @type {(v?: unknown) => void} */
      let held = () => {};
      const arrived = new Promise((res) => { held = res; });
      let moved = false;
      /** @type {(v?: unknown) => void} */
      let movedDone = () => {};
      const answered = new Promise((res) => { movedDone = res; });
      await p.route('**/_viewed/all', async (route) => {
        held(true);
        await gate;
        await route.continue();
        moved = true;
        movedDone(true);
      });
      await p.reload({ waitUntil: 'load' });
      await arrived;
      // Tab q moves the same old marks first (it shares p's storage, its own route is not held),
      // then draws them: a.txt shows ticked only once q's move is answered and decorated. The
      // response alone is not that signal, since the box is ticked a step after it, so a read
      // taken in between sees it unticked. Wait for the tick, then unmark with one saved click,
      // which is the newer choice.
      const q = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
      await q.goto(`${srv.base}${KEY}`, { waitUntil: 'load' });
      const box = 'section.file[data-path="a.txt"] input.viewed';
      await q.waitForSelector(`${box}:checked`, WAIT);
      const saved = q.waitForResponse(
        (x) => x.request().method() === 'POST' && /\/_viewed$/.test(x.url()), WAIT);
      await q.click(box);
      await saved;
      say(!(await q.isChecked(box)), `${w}: tab q unmarked a.txt`, `${w}: tab q left a.txt ticked`);
      release();
      await answered;
      say(moved, `${w}: the held move was answered last`, `${w}: the move never answered`);
      await p.unroute('**/_viewed/all');
      await reload(q);
      say(!(await ticked(q)).includes('a.txt'),
        `${w}: the unmark of the other tab survives the late move`,
        `${w}: ticked after reload ${await ticked(q)}`);
      await q.close();
    });
  }

  return {
    viewedMigrateOtherTabUnmark, viewedMigrateLaterClickWins, viewedMigrateRejected,
    viewedMigrateSaved, viewedMigrateKept,
    viewedMigrateTwoTabs,
  };
}
