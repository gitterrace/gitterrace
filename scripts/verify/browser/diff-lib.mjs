// The huge-diff check. Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
//
// A repository whose last commit changes 2,000 files: 1,960 pure renames (a folder moved) and
// 40 files rewritten with 300 lines each (12k lines). Checked on the commit page and, after a
// soft reset leaves the same change uncommitted, on the working tree page. In each browser:
// - the DOM after load stays under DOM_BOUND elements
// - a pure rename shows one "Renamed from" line, not a diff
// - scrolling to the end loads the bodies of the last files
// - an anchor to the last edited file (#fN) opens it
// - focus on a folded file and Enter opens it
// It prints load time, long tasks (> 50 ms, from navigation start) and scroll frame times, the
// numbers to compare before and after a change to how diffs render.
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { serve, sh, commitAll } from '../../lib/browser-lib.mjs';

const RENAMES = 1960;
const EDITS = 40;
// Headers of 2,000 files cost about 9 elements each; the rest is the chrome and the files the
// page opens in full, each changed line with its hidden "Added, "/"Removed, " span for screen
// readers (about 1,800 here). Every file and line in the DOM is several times this.
const DOM_BOUND = 33000;

function makeHugeRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-hugediff-'));
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  mkdirSync(join(dir, 'old'));
  mkdirSync(join(dir, 'big'));
  for (let i = 0; i < RENAMES; i++) writeFileSync(join(dir, 'old', `m${String(i).padStart(4, '0')}.txt`), `file ${i}\nsecond line ${i}\n`);
  for (let i = 0; i < EDITS; i++) writeFileSync(join(dir, 'big', `e${String(i).padStart(2, '0')}.txt`), `start ${i}\n`);
  commitAll(dir, 'base');
  // git mv of the whole folder keeps every blob: pure renames. Then the large edits.
  sh(dir, 'git', 'mv', 'old', 'new');
  for (let i = 0; i < EDITS; i++) {
    writeFileSync(join(dir, 'big', `e${String(i).padStart(2, '0')}.txt`), Array.from({ length: 300 }, (_, k) => `edited line ${k} of file ${i} with some words`).join('\n') + '\n');
  }
  commitAll(dir, 'move everything and edit some');
  return dir;
}

/** The id of the last file with added lines, or null. Runs in the page. */
function lastEditId() {
  const s = [...document.querySelectorAll('section.file')].reverse().find((x) => x.querySelector('.counts .add')?.textContent !== '+0');
  return s ? s.id : null;
}

/** @param {any} env what browser-check.mjs set up (r, browsers, bin, cfg, context) */
export function checks(env) {
  const { r, browsers, bin, cfg, context } = env;

  async function onePage(ctx, where, url) {
    const p = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
    // Long tasks from navigation start: the parse and first layout are where a big DOM costs.
    await p.addInitScript(() => { /** @type {any} */ (window).__long = []; try { new PerformanceObserver((l) => { for (const e of l.getEntries()) /** @type {any} */ (window).__long.push(e.duration); }).observe({ type: 'longtask', buffered: true }); } catch {} });
    const t0 = Date.now();
    await p.goto(url, { waitUntil: 'load', timeout: 180000 });
    const loadMs = Date.now() - t0;
    const sections = await p.evaluate(() => document.querySelectorAll('section.file').length);
    const els = await p.evaluate(() => document.getElementsByTagName('*').length);
    if (sections !== RENAMES + EDITS) r.fail(`${where}: ${sections} file sections, want ${RENAMES + EDITS}`);
    els > DOM_BOUND ? r.fail(`${where}: ${els} elements after load, bound ${DOM_BOUND}`) : r.ok(`${where}: ${els} elements after load (bound ${DOM_BOUND})`);
    const moved = await p.evaluate(() => {
      const s = [...document.querySelectorAll('section.file')].find((x) => x.querySelector('.badge.renamed'));
      return s ? { line: (s.querySelector('.moved') || {}).textContent || '', diff: !!s.querySelector('.diff') } : null;
    });
    if (!moved || !/^Renamed from old\//.test(moved.line) || moved.diff) r.fail(`${where}: a pure rename shows ${JSON.stringify(moved)}`);
    else r.ok(`${where}: a pure rename is one line: ${moved.line}`);
    const atLoad = await p.evaluate(() => /** @type {any} */ (window).__long.slice());

    // One scroll from top to bottom, a screen per frame.
    const frames = await p.evaluate(async () => {
      /** @type {any} */ (window).__long.length = 0;
      const out = [];
      let last = performance.now();
      while (scrollY + innerHeight < document.documentElement.scrollHeight - 2) {
        scrollBy(0, innerHeight);
        await new Promise((ok) => requestAnimationFrame(() => ok()));
        const now = performance.now();
        out.push(now - last);
        last = now;
      }
      return out;
    });
    await p.waitForTimeout(2000);
    const scrollLong = await p.evaluate(() => /** @type {any} */ (window).__long.slice());
    frames.sort((x, y) => x - y);
    const pct = (q) => (frames.length ? frames[Math.min(frames.length - 1, Math.floor(frames.length * q))].toFixed(1) : 'n/a');
    const sum = (l) => Math.round(l.reduce((x, y) => x + y, 0));
    console.log(`     ${where}: load ${loadMs} ms, ${els} elements, long tasks at load ${atLoad.length} (${sum(atLoad)} ms, max ${Math.round(Math.max(0, ...atLoad))} ms); scroll ${frames.length} frames p50 ${pct(0.5)} ms p95 ${pct(0.95)} ms max ${pct(1)} ms, long tasks ${scrollLong.length} (${sum(scrollLong)} ms)`);

    // The last file's fragment loads after the scroll ends, and how long that takes depends on
    // the machine, so wait for the condition (keeping the end in view) instead of a fixed time.
    const tailOpen = await p.waitForFunction(() => {
      const s = [...document.querySelectorAll('section.file')].reverse().find((x) => x.querySelector('.counts .add')?.textContent !== '+0');
      if (s && s.querySelector('.diff .add')) return true;
      // Bring the file in view; every other poll step away first, because a fragment that
      // failed to load is watched again only once it leaves the viewport and comes back.
      const w = /** @type {any} */ (window);
      w.__tailPoll = (w.__tailPoll || 0) + 1;
      if (w.__tailPoll % 10 === 0) scrollTo(0, 0);
      else if (s) s.scrollIntoView();
      else scrollTo(0, document.documentElement.scrollHeight);
      return false;
    }, null, { timeout: 30000, polling: 100 }).then(() => true, () => false);
    tailOpen ? r.ok(`${where}: scrolling to the end loaded the last edited file`) : r.fail(`${where}: after scrolling to the end the last edited file is not open`);

    const last = await p.evaluate(lastEditId);
    await p.close();
    const q = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
    await q.goto(`${url}#${last}`, { waitUntil: 'load', timeout: 180000 });
    const opened = await q.waitForFunction((x) => !!document.querySelector(`#${x} .diff .add`), last, { timeout: 15000 }).then(() => true, () => false);
    opened ? r.ok(`${where}: #${last} opens the file`) : r.fail(`${where}: #${last} left the file folded`);
    const fid = await q.evaluate(() => {
      const a = /** @type {HTMLElement | null} */ (document.querySelector('[data-load-diff]'));
      if (!a) return null;
      a.focus();
      return document.activeElement === a ? a.closest('section').id : null;
    });
    if (!fid) r.fail(`${where}: no folded file takes focus`);
    else {
      await q.keyboard.press('Enter');
      const ok = await q.waitForFunction((x) => !!document.querySelector(`#${x} .diff`) && !document.querySelector(`#${x} [data-load-diff]`), fid, { timeout: 15000 }).then(() => true, () => false);
      ok ? r.ok(`${where}: Enter on #${fid} opens it`) : r.fail(`${where}: Enter on #${fid} did not open it`);
    }
    await q.close();

    // A hashchange to a folded file far down opens it and leaves its header in view, though
    // the files above it load and grow meanwhile.
    const h = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
    await h.goto(url, { waitUntil: 'load', timeout: 180000 });
    const far = await h.evaluate(() => { const l = [...document.querySelectorAll('section.file [data-load-diff][data-lazy]')]; return l.length ? l[l.length - 1].closest('section').id : null; });
    if (!far) r.fail(`${where}: no lazy file to jump to`);
    else {
      await h.evaluate((x) => { location.hash = x; }, far);
      await h.waitForFunction((x) => !document.querySelector(`#${x} [data-load-diff]`), far, { timeout: 15000 }).catch(() => {});
      await h.waitForTimeout(1500);
      const top = await h.evaluate((x) => document.getElementById(x).getBoundingClientRect().top, far);
      top > -2 && top < 900 ? r.ok(`${where}: hashchange to #${far} opens it in view (top ${Math.round(top)})`) : r.fail(`${where}: hashchange to #${far} left it at top ${Math.round(top)}`);
    }
    await h.close();

    // Opening the page at #fN scrolls that file to the top, its header where it pins and its
    // top border just above the window. Nothing of the file above shows. Off-screen line
    // blocks are only measured once seen, so their reserved height must be their real one.
    const g = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
    await g.goto(url, { waitUntil: 'load', timeout: 180000 });
    const open = await g.evaluate(() => [...document.querySelectorAll('section.file')]
      .filter((s) => s.querySelector('.diff') && !s.querySelector('[data-load-diff]'))
      .map((s) => s.id));
    const pick = [open[Math.floor(open.length / 2)], open[open.length - 1]].filter(Boolean);
    for (const id of pick) {
      await g.goto(`${url}#${id}`, { waitUntil: 'load', timeout: 180000 });
      await g.waitForTimeout(1500);
      const at = await g.evaluate((x) => {
        const s = document.getElementById(x);
        const hdr = s.querySelector('header').getBoundingClientRect();
        const under = document.elementFromPoint(innerWidth / 2, hdr.bottom + 20)
          ?.closest('section.file');
        // The file above may not show: a sliver of it reads as being in the wrong file.
        const above = document.elementFromPoint(innerWidth / 2, 4)?.closest('section.file');
        return { top: Math.round(s.getBoundingClientRect().top), hdr: Math.round(hdr.top),
          under: under && under.id, above: above && above.id };
      }, id);
      at.top === -1 && at.hdr === 0 && at.under === id && (!at.above || at.above === id)
        ? r.ok(`${where}: opening at #${id} pins its header at the top`)
        : r.fail(`${where}: opening at #${id} shows ${at.above} on top, file ${at.top},`
          + ` header ${at.hdr}`);
    }
    await g.close();

    // The first fetch of every fragment fails: a lazy file must load on a later scroll.
    const f = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
    const failed = new Set();
    await f.route(new RegExp('/(worktree/)?diff/'), (rt) => {
      const u = rt.request().url();
      if (failed.has(u)) return rt.continue();
      failed.add(u);
      return rt.fulfill({ status: 503, body: 'busy' });
    });
    await f.goto(url, { waitUntil: 'load', timeout: 180000 });
    const target = await f.evaluate(() => { const a = document.querySelector('[data-load-diff][data-lazy]'); return a ? a.closest('section').id : null; });
    if (!target) r.fail(`${where}: no lazy file for the retry check`);
    else {
      await f.evaluate((x) => document.getElementById(x).scrollIntoView(), target);
      await f.waitForTimeout(1500);
      await f.evaluate(() => scrollTo(0, 0));
      await f.waitForTimeout(800);
      await f.evaluate((x) => document.getElementById(x).scrollIntoView(), target);
      const retried = await f.waitForFunction((x) => !document.querySelector(`#${x} [data-load-diff]`), target, { timeout: 15000 }).then(() => true, () => false);
      retried ? r.ok(`${where}: a lazy file that failed once loads on the next scroll`) : r.fail(`${where}: #${target} failed once and never retried on scroll`);
    }
    await f.close();
  }

  async function hugeDiff() {
    const dir = makeHugeRepo();
    const srv = await serve(bin, dir, cfg);
    const id = sh(dir, 'git', 'rev-parse', 'HEAD');
    try {
      for (const name of browsers) {
        const { b, ctx } = await context(name);
        await onePage(ctx, `huge-diff ${name} commit`, `${srv.base}/commit/${id}`);
        await b.close();
      }
      // The same change, uncommitted: the working tree page of a big uncommitted rename.
      sh(dir, 'git', 'reset', '-q', '--soft', 'HEAD~1');
      await new Promise((ok) => setTimeout(ok, 1500));
      for (const name of browsers) {
        const { b, ctx } = await context(name);
        await onePage(ctx, `huge-diff ${name} worktree`, `${srv.base}/worktree`);
        await b.close();
      }
    } catch (e) {
      r.fail(`huge-diff: ${e && e.stack}`);
    } finally {
      srv.stop();
      // The server may still be writing its cache while it exits.
      rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    }
  }
  return { hugeDiff };
}
