// The live-read check: the reader scrolls on while live refreshes land. Imported by
// scripts/browser-check.mjs, not run on its own.
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { serve, sh } from './lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (r, browsers, context, bin, cfg, ...) */
export function checks(env) {
  const { r, browsers, context, bin, cfg } = env;
  // On the working tree page with 90 changed files, open at #f3, the reader scrolls down with the
  // wheel in small steps (a trackpad) and then with PageDown and Space, while files above and below
  // them change every 300 to 800 ms, so refreshes overlap, changed bodies fold and load again, and
  // bodies load late (each held 300 ms) with a refresh landing mid-load. Every frame the file at
  // the middle of the viewport is looked up again by its path: it must never move down the screen
  // (the page jumping up) by more than 4 px, the reader ends where their scrolling took them and
  // stays there, and #f3 stays in the URL. Then inside one file longer than the viewport, so no
  // file top is in view, a refresh keeps that file within 4 px.
  async function liveRead() {
    const FILES = 90;
    const body = (tag) =>
      Array.from({ length: 30 }, (_, k) => `line ${k} ${tag}`).join('\n') + '\n';
    const name2 = (i) => `s${String(i).padStart(2, '0')}.txt`;
    const repo = mkdtempSync(join(tmpdir(), 'gt-liveread-'));
    const state = mkdtempSync(join(tmpdir(), 'gt-state-'));
    const saved = process.env.XDG_STATE_HOME;
    process.env.XDG_STATE_HOME = state;
    let one;
    try {
      sh(repo, 'git', 'init', '-q', '-b', 'main');
      for (let i = 0; i < FILES; i++) writeFileSync(join(repo, name2(i)), body('base'));
      sh(repo, 'git', 'add', '-A');
      sh(repo, 'git', 'commit', '-q', '-m', 'base');
      for (let i = 0; i < FILES; i++) writeFileSync(join(repo, name2(i)), body('edited'));
      writeFileSync(
        join(repo, 's45big.txt'),
        Array.from({ length: 3000 }, (_, k) => `big ${k}`).join('\n') + '\n',
      );
      one = await serve(bin, repo, cfg);
      for (const name of browsers) {
        const { b, ctx } = await context(name);
        let editing = false;
        try {
          const p = await ctx.newPage();
          await p.setViewportSize({ width: 1100, height: 800 });
          let ready = false;
          for (let i = 0; i < 40 && !ready; i++) {
            await p.goto(`${one.base}/worktree#f3`, { waitUntil: 'load' });
            ready = (await p.$$('section[data-path]')).length >= FILES;
            if (!ready) await p.waitForTimeout(250);
          }
          // Bodies arrive late, so refreshes land while loads are in flight.
          const diffs = await p.$$eval(
            '[data-load-diff]',
            (l) => l.map((e) => /** @type {HTMLElement} */ (e).dataset.loadDiff),
          );
          const urls = new Set(diffs.map((u) => new URL(u, one.base).pathname));
          await ctx.route((u) => urls.has(u.pathname), async (q) => {
            await new Promise((ok) => setTimeout(ok, 300));
            await q.continue();
          });
          await p.waitForTimeout(800);
          await p.evaluate(() => {
            const w = /** @type {any} */ (window);
            w.__refreshed = 0;
            w.__worst = { d: 0 };
            w.__phase = 'wait';
            document.addEventListener('gt:live-refreshed', () => {
              w.__refreshed++;
            });
            let prev = null;
            const look = () => {
              if (prev) {
                const s = document.querySelector(`section[data-path="${CSS.escape(prev.path)}"]`);
                if (s && w.__phase !== 'wait') {
                  const d = s.getBoundingClientRect().top - prev.top;
                  if (d > w.__worst.d) {
                    w.__worst = {
                      d,
                      path: prev.path,
                      phase: w.__phase,
                      y: scrollY,
                      at: performance.now(),
                    };
                  }
                }
              }
              const mid = document.elementFromPoint(550, 400);
              const s = mid && mid.closest('section[data-path]');
              prev = s
                ? {
                  path: /** @type {HTMLElement} */ (s).dataset.path,
                  top: s.getBoundingClientRect().top,
                }
                : null;
              requestAnimationFrame(look);
            };
            requestAnimationFrame(look);
          });
          // Edits: alternate a file near the top (above the reader), a far one and a random one.
          editing = true;
          let n = 0;
          const edits = (async () => {
            while (editing) {
              const i = [1, 2, 3, 5][n % 4], j = 10 + Math.floor(Math.random() * 80);
              writeFileSync(join(repo, name2(n % 2 ? i : j)), body(`edit ${n}`));
              n++;
              await new Promise((ok) => setTimeout(ok, 300 + Math.random() * 500));
            }
          })();
          await p.mouse.move(550, 400);
          await p.evaluate(() => {
            /** @type {any} */ (window).__phase = 'wheel';
          });
          const t0 = Date.now();
          while (Date.now() - t0 < 10000) {
            await p.mouse.wheel(0, 40);
            await p.waitForTimeout(16);
          }
          await p.evaluate(() => {
            /** @type {any} */ (window).__phase = 'keys';
          });
          await p.evaluate(() => /** @type {HTMLElement} */ (document.activeElement)?.blur());
          for (let k = 0; k < 16; k++) {
            await p.keyboard.press(k % 2 ? 'Space' : 'PageDown');
            await p.waitForTimeout(200);
          }
          editing = false;
          await edits;
          // The reader stops: one more refresh, then they must still be where they stopped.
          await p.waitForFunction(
            () =>
              new Promise((ok) => {
                const y = scrollY;
                setTimeout(() => ok(scrollY === y), 300);
              }),
            null,
            { timeout: 5000 },
          ).catch(() => {});
          await p.evaluate(() => {
            /** @type {any} */ (window).__phase = 'still';
          });
          const where = () =>
            p.evaluate(() => {
              const s = document.elementFromPoint(550, 400)?.closest('section[data-path]');
              return s
                ? {
                  path: /** @type {HTMLElement} */ (s).dataset.path,
                  y: scrollY,
                  top: s.getBoundingClientRect().top,
                }
                : null;
            });
          const topOf = (path) =>
            p.evaluate(
              (q) =>
                document.querySelector(`section[data-path="${q}"]`)?.getBoundingClientRect().top,
              path,
            );
          const stop = await where();
          const seen = await p.evaluate(() => /** @type {any} */ (window).__refreshed);
          writeFileSync(join(repo, name2(2)), body('last'));
          await p.waitForFunction((k) => /** @type {any} */ (window).__refreshed > k, seen, {
            timeout: 8000,
          }).catch(() => {});
          await p.waitForTimeout(2500);
          const end = await where();
          const endTop = stop ? await topOf(stop.path) : null;
          // Inside the long file: its top is above the viewport and its bottom below.
          await p.evaluate(() => {
            /** @type {any} */ (window).__phase = 'wait';
          });
          await p.evaluate(() => {
            const s = document.querySelector('section[data-path="s45big.txt"]');
            s.scrollIntoView();
          });
          await p.waitForFunction(
            () => !document.querySelector('section[data-path="s45big.txt"] [data-load-diff]'),
            null,
            { timeout: 8000 },
          ).catch(() => {});
          await p.waitForTimeout(500);
          await p.evaluate(() => window.scrollBy(0, 3000));
          await p.waitForTimeout(500);
          const bigTop = () =>
            p.evaluate(() =>
              document.querySelector('section[data-path="s45big.txt"]').getBoundingClientRect().top
            );
          const big0 = await bigTop();
          const seen2 = await p.evaluate(() => /** @type {any} */ (window).__refreshed);
          writeFileSync(join(repo, name2(1)), body('big'));
          await p.waitForFunction((k) => /** @type {any} */ (window).__refreshed > k, seen2, {
            timeout: 8000,
          }).catch(() => {});
          await p.waitForTimeout(2500);
          const big1 = await bigTop();
          const res = await p.evaluate(() => ({
            worst: /** @type {any} */ (window).__worst,
            refreshed: /** @type {any} */ (window).__refreshed,
            hash: location.hash,
            f3: !document.querySelector('#f3 [data-load-diff]'),
          }));
          const bad = [];
          if (res.refreshed < 3) bad.push(`only ${res.refreshed} refreshes`);
          if (res.worst.d > 4) {
            bad.push(
              `page jumped up ${
                Math.round(res.worst.d)
              } px under ${res.worst.path} while ${res.worst.phase} (scrollY ${
                Math.round(res.worst.y)
              })`,
            );
          }
          if (!stop || endTop == null || Math.abs(endTop - stop.top) > 4) {
            bad.push(
              `stopped at ${stop?.path} (top ${
                Math.round(stop?.top)
              }), after a refresh its top is ${Math.round(endTop)}`,
            );
          }
          if (!stop || parseInt(stop.path.slice(1), 10) < 10) {
            bad.push(`scrolled only to ${stop?.path}`);
          }
          if (res.hash !== '#f3') bad.push(`hash ${res.hash}`);
          if (!res.f3) bad.push('#f3 folded again');
          if (!(big0 < -2000) || Math.abs(big1 - big0) > 4) {
            bad.push(
              `inside the long file its top moved from ${Math.round(big0)} to ${Math.round(big1)}`,
            );
          }
          bad.length
            ? r.fail(`live-read ${name}: ${bad.join(', ')}`)
            : r.ok(
              `live-read ${name}: ${res.refreshed} refreshes while scrolling, never pushed back `
                + `more than 4 px, ends at ${end.path}, #f3 kept, held inside a long file`,
            );
        } finally {
          editing = false;
          await b.close();
        }
      }
    } catch (e) {
      r.fail(`live-read: ${e && e.stack}`);
    } finally {
      if (one) one.stop();
      if (saved === undefined) delete process.env.XDG_STATE_HOME;
      else process.env.XDG_STATE_HOME = saved;
      rmSync(repo, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
      rmSync(state, { recursive: true, force: true });
    }
  }
  return { liveRead };
}
