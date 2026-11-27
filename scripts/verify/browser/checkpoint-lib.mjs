// The checkpoint flow of the working tree page. Imported by browser-check.mjs, not run alone.
//
// A repository with two uncommitted files. In each browser, with the owner cookie:
// - the Checkpoint button saves one and the Since checkpoint tab appears, wording that it
//   writes only the cache
// - after a further edit, the tab lists only that file and folds the other into a count
// - a viewed mark survives a reload and shows in a second tab
// - an edit to a marked file turns the mark into "Changed since viewed"
// - Mark all viewed marks every file
// - with no owner cookie the buttons answer 403 and nothing is saved
// checkpoint-between: two checkpoints with an edit between them. The Since and Until lists of
// the checkpoint page pick the older one and then the newer one, and the page then lists only
// the file edited between the two, not the one edited after the second.
// A screenshot of the checkpoint page goes to $TMPDIR/gt-checkpoint-<browser>.png.
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { serve, sh, commitAll, poll } from '../../lib/browser-lib.mjs';

function makeRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-checkpoint-'));
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  for (const n of ['a', 'b', 'c']) writeFileSync(join(dir, `${n}.txt`), 'one\ntwo\n');
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
  const paths = (p) => p.$$eval('section.file[data-path]',
    (s) => s.map((x) => /** @type {HTMLElement} */ (x).dataset.path));
  /** The paths of the files whose Viewed box is ticked, sorted. Runs in the page. */
  const ticked = () => [...document.querySelectorAll('section.file[data-path]')]
    .filter((x) => /** @type {HTMLInputElement|null} */ (x.querySelector('input.viewed'))?.checked)
    .map((x) => /** @type {HTMLElement} */ (x).dataset.path).sort();
  const marked = (p) => p.evaluate(ticked);
  const settled = (p, want) => p.waitForFunction((w) => [...document.querySelectorAll(
    'section.file[data-path]')]
    .filter((x) => /** @type {HTMLInputElement|null} */ (x.querySelector('input.viewed'))?.checked)
    .map((x) => /** @type {HTMLElement} */ (x).dataset.path).sort().join() === w,
    want.join(), WAIT).then(() => true, () => false);

  async function checkpointSinceTab() {
    // One repository and server per browser: marks and checkpoints persist in the repository.
    for (const name of browsers) {
      const dir = makeRepo();
      const srv = await serve(bin, dir, cfg);
      const w = `checkpoint-since-tab ${name}`;
      try {
        const { b, ctx } = await context(name);
        try {
          await ctx.addCookies([await srv.owner()]);
          const p = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
          await p.goto(`${srv.base}/worktree`, { waitUntil: 'load' });
          await p.waitForSelector('[data-checkpoint]', WAIT);
          say(!(await p.$('.tabs a:text-is("Since checkpoint")')),
            `${w}: no tab before a checkpoint`,
            `${w}: a Since checkpoint tab with no checkpoint`);
          // Viewed first, so the mark has to outlive the checkpoint and a reload.
          await p.waitForSelector('section.file input.viewed', WAIT);
          await p.locator('section.file[data-path="a.txt"] input.viewed').check();
          await p.reload({ waitUntil: 'load' });
          await p.waitForSelector('section.file input.viewed', WAIT);
          say(await settled(p, ['a.txt']), `${w}: a viewed mark survives a reload`,
            `${w}: marks after reload: ${await marked(p)}`);
          const tab2 = await ctx.newPage();
          await tab2.goto(`${srv.base}/worktree`, { waitUntil: 'load' });
          await tab2.waitForSelector('section.file input.viewed', WAIT);
          say(await settled(tab2, ['a.txt']), `${w}: a second tab shows the same mark`,
            `${w}: second tab marks: ${await marked(tab2)}`);
          await tab2.close();
          await p.click('[data-checkpoint]');
          await p.waitForSelector('.cpnote:text("Checkpoint saved")', WAIT);
          const link = p.locator('.tabs a:text-is("Since checkpoint")');
          say((await link.count()) === 1, `${w}: the Since checkpoint tab appears`,
            `${w}: no Since checkpoint tab after saving`);
          writeFileSync(join(dir, 'c.txt'), 'one\ntwo\nthird edit\n');
          await p.goto(`${srv.base}/worktree`, { waitUntil: 'load' });
          await p.locator('.tabs a:text-is("Since checkpoint")').click();
          await p.waitForSelector('[data-cp-since]', WAIT);
          const shown = await paths(p);
          say(shown.join() === 'c.txt', `${w}: the tab shows only the file edited since`,
            `${w}: tab shows ${shown}`);
          const note = await p.$eval('[data-cp-since]', (e) => e.textContent);
          say(/unchanged since/.test(note) && /writes nothing else/.test(note),
            `${w}: unchanged files fold into a count and the cache-only wording is there`,
            `${w}: note was ${note}`);
          await p.screenshot({ path: join(tmpdir(), `gt-checkpoint-${name}.png`) });
          // A marked file edited again is stale, and Mark all viewed marks the rest.
          writeFileSync(join(dir, 'a.txt'), 'one\ntwo\nfirst edit\nmore\n');
          await p.goto(`${srv.base}/worktree`, { waitUntil: 'load' });
          await p.waitForSelector('section.file[data-path="a.txt"] .viewed-changed', WAIT)
            .then(() => r.ok(`${w}: an edited marked file reads Changed since viewed`),
              () => r.fail(`${w}: no Changed since viewed badge`));
          await p.click('[data-mark-all]');
          say(await settled(p, ['a.txt', 'b.txt', 'c.txt']),
            `${w}: Mark all viewed marks every file`,
            `${w}: marks after Mark all: ${await marked(p)}`);
          // No owner cookie: the write is refused and nothing is saved.
          const plain = await fetch(`${srv.base}/_checkpoint`, {
            method: 'POST',
            headers: { 'content-type': 'application/json', 'sec-fetch-site': 'same-origin',
              origin: srv.origin },
            body: '{"scope":"all"}',
          });
          say(plain.status === 403, `${w}: a write without the owner cookie is refused`,
            `${w}: answered ${plain.status} without the cookie`);
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
  async function checkpointBetween() {
    for (const name of browsers) {
      const dir = makeRepo();
      const srv = await serve(bin, dir, cfg);
      const w = `checkpoint-between ${name}`;
      try {
        const { b, ctx } = await context(name);
        try {
          await ctx.addCookies([await srv.owner()]);
          const p = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
          await p.goto(`${srv.base}/worktree`, { waitUntil: 'load' });
          await p.waitForSelector('[data-checkpoint]', WAIT);
          await p.click('[data-checkpoint]');
          await p.waitForSelector('.cpnote:text("Checkpoint saved")', WAIT);
          writeFileSync(join(dir, 'c.txt'), 'one\ntwo\nbetween\n');
          await p.click('[data-checkpoint]');
          // poll, not waitForFunction with an async predicate: in webkit that returned before
          // the second checkpoint existed, and the edit below went into it.
          await poll(() => p.evaluate(async () => {
            const m = document.querySelector('meta[name="gt-base"]');
            const r = await fetch(`${m?.getAttribute('content') ?? ''}/api/v1/checkpoints`);
            return (await r.json()).checkpoints.length === 2;
          }), 'the second checkpoint');
          writeFileSync(join(dir, 'b.txt'), 'one\ntwo\nafter\n');
          await p.goto(`${srv.base}/worktree`, { waitUntil: 'load' });
          await p.locator('.tabs a:text-is("Since checkpoint")').click();
          await p.waitForSelector('[data-cp-pick]', WAIT);
          const links = (sel) => p.$$eval(`${sel} a`, (a) => a.map((x) => x.getAttribute('href')));
          const since = await links('[data-cp-since-pick]');
          const root = `${new URL(srv.base).pathname.replace(/\/$/, '')}/worktree?since=`;
          say(since.length === 2 && since.every((h) => h.startsWith(root)),
            `${w}: the Since list holds both checkpoints behind the base path`,
            `${w}: Since links ${since}`);
          await p.goto(new URL(since[1], srv.base).href, { waitUntil: 'load' });
          const until = await links('[data-cp-until-pick]');
          await p.goto(new URL(until[0], srv.base).href, { waitUntil: 'load' });
          await p.waitForSelector('[data-cp-since]', WAIT);
          const shown = await paths(p);
          say(shown.join() === 'c.txt', `${w}: since the first up to the second lists only c.txt`,
            `${w}: shown ${shown}`);
          const note = await p.$eval('[data-cp-since]', (e) => e.textContent);
          say(/up to the one taken/.test(note), `${w}: the note says it is not the working tree`,
            `${w}: note was ${note}`);
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
  return { checkpointSinceTab, checkpointBetween };
}
