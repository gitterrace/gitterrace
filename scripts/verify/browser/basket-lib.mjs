// The review basket. Imported by browser-check.mjs, not run alone.
//
// A repository with a file of ten lines and one `review` action that saves what it is sent.
// In each browser, with the owner cookie:
// - three notes are added from the "." menu of a line selection, the header count reads 3
// - the panel lists them with the status chip "untouched"
// - Send all runs the action once with the three ids and the server's own notes
// - after an edit to the noted lines the chip reads "lines changed"
// - a note with markup shows as text, never as an element
// - with no owner cookie a write answers 403 and nothing is kept
// - a basket of 501 entries sends a chosen subset of 500 and keeps every entry
// - blob-finding-marker: a finding on lines 4-5 puts a gutter dot on those rows, its link opens
//   the panel, and edited lines lose it
// A screenshot of the panel goes to $TMPDIR/gt-basket-<browser>.png.
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { serve, sh, commitAll } from '../../lib/browser-lib.mjs';

const BODY = Array.from({ length: 10 }, (_, i) => `line ${i + 1}`).join('\n') + '\n';

export function checks(env) {
  const { r, browsers, bin, context } = env;
  const say = (ok, good, bad) => (ok ? r.ok(good) : r.fail(bad));
  const WAIT = { timeout: 15000 };

  async function basketSendAll() {
    for (const name of browsers) {
      const dir = mkdtempSync(join(tmpdir(), 'gt-basket-'));
      const cfg = mkdtempSync(join(tmpdir(), 'gt-basket-cfg-'));
      const sent = join(cfg, 'sent.json');
      sh(dir, 'git', 'init', '-q', '-b', 'main');
      writeFileSync(join(dir, 'a.txt'), BODY);
      commitAll(dir, 'init');
      writeFileSync(join(cfg, 'actions.toml'), `version = 1\n[[action]]\nid = "keep"\n`
        + `label = "Keep"\nkinds = ["review"]\nshell = true\n`
        + `command = 'cat > "${sent}"; echo kept'\n`, { mode: 0o600 });
      const srv = await serve(bin, dir, cfg);
      const w = `basket-send-all ${name}`;
      try {
        const { b, ctx } = await context(name);
        try {
          await ctx.addCookies([await srv.owner()]);
          const p = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
          const notes = [['L2', 'first <img src=x onerror=alert(1)>'], ['L4-L5', 'second'],
            ['L7', 'third']];
          for (const [lines, text] of notes) {
            await p.goto(`${srv.base}/blob/main/a.txt#${lines}`, { waitUntil: 'load' });
            await p.waitForSelector('#basketbtn', WAIT);
            await p.keyboard.press('.');
            await p.locator('.actmenu li:has-text("Add to review")').click();
            await p.locator('textarea.basket-note').fill(text);
            await p.locator('.basket form button[type=submit]').click();
            await p.waitForSelector('.basket', { state: 'detached', ...WAIT });
          }
          await p.goto(`${srv.base}/`, { waitUntil: 'load' });
          await p.waitForSelector('.basketn:text-is("3")', WAIT).then(
            () => r.ok(`${w}: the header counts three`),
            () => r.fail(`${w}: the header count is not 3`));
          await p.click('#basketbtn');
          await p.waitForSelector('.basket-row', WAIT);
          const chips = await p.$$eval('.basket-chip', (c) => c.map((x) => x.textContent));
          say(chips.length === 3 && chips.every((c) => c === 'untouched'),
            `${w}: three rows, each untouched`, `${w}: chips ${chips}`);
          say((await p.$$('.basket-text img')).length === 0
            && (await p.textContent('.basket-text')).includes('<img'),
          `${w}: markup in a note shows as text`, `${w}: a note was parsed as markup`);
          await p.screenshot({ path: join(tmpdir(), `gt-basket-${name}.png`) });
          await p.click('.basket-foot .primary');
          await p.waitForSelector('.basket-msg:text("kept")', WAIT).then(
            () => r.ok(`${w}: Send all ran the action`), () => r.fail(`${w}: no sent status`));
          const got = existsSync(sent) ? JSON.parse(readFileSync(sent, 'utf8')) : {};
          const f = got.findings || [];
          say(got.kind === 'review' && f.length === 3 && f[0].note.includes('first'),
            `${w}: one payload with three findings and the server's notes`,
            `${w}: payload ${JSON.stringify(got).slice(0, 200)}`);
          await p.keyboard.press('Escape');
          writeFileSync(join(dir, 'a.txt'), BODY.replace('line 4', 'changed 4'));
          await p.reload({ waitUntil: 'load' });
          await p.click('#basketbtn');
          await p.waitForSelector('.basket-chip:text-is("lines changed")', WAIT).then(
            () => r.ok(`${w}: an edited range reads lines changed`),
            () => r.fail(`${w}: no lines changed chip`));
          const plain = await fetch(`${srv.base}/_findings`, {
            method: 'DELETE',
            headers: { 'content-type': 'application/json', 'sec-fetch-site': 'same-origin',
              origin: srv.origin },
            body: '{"ids":[]}',
          });
          say(plain.status === 403, `${w}: a write without the owner cookie is refused`,
            `${w}: answered ${plain.status} without the cookie`);
          await p.close();
        } finally { await b.close(); }
      } catch (e) {
        r.fail(`${w}: ${e && e.stack}`);
      } finally {
        srv.stop();
        const gone = { recursive: true, force: true, maxRetries: 10, retryDelay: 200 };
        for (const d of [dir, cfg]) rmSync(d, gone);
      }
    }
  }
  // 501 entries are above what one review payload carries: the panel must let the user pick
  // 500 of them, send those, and delete nothing.
  async function basketSendSubset() {
    for (const name of browsers) {
      const dir = mkdtempSync(join(tmpdir(), 'gt-basket-'));
      const cfg = mkdtempSync(join(tmpdir(), 'gt-basket-cfg-'));
      const sent = join(cfg, 'sent.json');
      sh(dir, 'git', 'init', '-q', '-b', 'main');
      writeFileSync(join(dir, 'a.txt'), BODY);
      commitAll(dir, 'init');
      writeFileSync(join(cfg, 'config.toml'), '[review]\nfindings_entries = 600\n');
      writeFileSync(join(cfg, 'actions.toml'), `version = 1\n[[action]]\nid = "keep"\n`
        + `label = "Keep"\nkinds = ["review"]\nshell = true\n`
        + `command = 'cat > "${sent}"; echo kept'\n`, { mode: 0o600 });
      const srv = await serve(bin, dir, cfg);
      const w = `basket-send-subset ${name}`;
      try {
        const { b, ctx } = await context(name);
        try {
          await ctx.addCookies([await srv.owner()]);
          const p = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
          await p.addInitScript(() => {
            window['baseOf'] = () => document.querySelector('meta[name="gt-base"]')
              .getAttribute('content').replace(/\/$/, '');
          });
          await p.goto(`${srv.base}/blob/main/a.txt`, { waitUntil: 'load' });
          await p.waitForSelector('#basketbtn', WAIT);
          const added = await p.evaluate(async (n) => {
            let ok = 0;
            const base = window['baseOf']();
            for (let i = 0; i < n; i++) {
              const r = await fetch(`${base}/_findings`, {
                method: 'POST', credentials: 'same-origin',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ kind: 'line', rev: 'HEAD', path: 'a.txt',
                  lines: { start: 1 + (i % 10), end: 1 + (i % 10) }, note: `n${i}` }),
              });
              if (r.ok) ok++;
              else if (i === 0) throw new Error(`POST ${r.status} ${await r.text()}`);
            }
            return ok;
          }, 501);
          say(added === 501, `${w}: 501 entries added`, `${w}: only ${added} added`);
          await p.goto(`${srv.base}/`, { waitUntil: 'load' });
          await p.click('#basketbtn');
          await p.waitForSelector('.basket-row', WAIT);
          const go = p.locator('.basket-foot .primary');
          say(await go.isDisabled(), `${w}: sending 501 at once is held back`,
            `${w}: Send is enabled with 501 chosen`);
          await p.click('.basket-foot .basket-first');
          await p.waitForFunction(
            () => !document.querySelector('.basket-foot .primary').hasAttribute('disabled'),
            null, WAIT);
          await go.click();
          await p.waitForSelector('.basket-msg:text("kept")', WAIT).then(
            () => r.ok(`${w}: the chosen 500 were sent`), () => r.fail(`${w}: no sent status`));
          const got = existsSync(sent) ? JSON.parse(readFileSync(sent, 'utf8')) : {};
          say((got.findings || []).length === 500,
            `${w}: one payload with 500 findings`, `${w}: ${(got.findings || []).length} sent`);
          const left = await p.evaluate(async () => {
            const m = window['baseOf']();
            return (await (await fetch(`${m}/_findings?count=1`)).json()).count;
          });
          say(left === 501, `${w}: every entry is still in the basket`, `${w}: ${left} left`);
          await p.close();
        } finally { await b.close(); }
      } catch (e) {
        r.fail(`${w}: ${e && e.stack}`);
      } finally {
        srv.stop();
        const gone = { recursive: true, force: true, maxRetries: 10, retryDelay: 200 };
        for (const d of [dir, cfg]) rmSync(d, gone);
      }
    }
  }
  // The file view marks the lines a finding points at, and the marker opens the basket.
  async function blobFindingMarker() {
    for (const name of browsers) {
      const dir = mkdtempSync(join(tmpdir(), 'gt-basket-'));
      const cfg = mkdtempSync(join(tmpdir(), 'gt-basket-cfg-'));
      sh(dir, 'git', 'init', '-q', '-b', 'main');
      writeFileSync(join(dir, 'a.txt'), BODY);
      commitAll(dir, 'init');
      const srv = await serve(bin, dir, cfg);
      const w = `blob-finding-marker ${name}`;
      try {
        const { b, ctx } = await context(name);
        try {
          await ctx.addCookies([await srv.owner()]);
          const p = await ctx.newPage({ viewport: { width: 1200, height: 900 } });
          await p.goto(`${srv.base}/blob/main/a.txt#L4-L5`, { waitUntil: 'load' });
          await p.waitForSelector('#basketbtn', WAIT);
          await p.keyboard.press('.');
          await p.locator('.actmenu li:has-text("Add to review")').click();
          await p.locator('textarea.basket-note').fill('mind this');
          await p.locator('.basket form button[type=submit]').click();
          await p.waitForSelector('.basket', { state: 'detached', ...WAIT });
          await p.goto(`${srv.base}/blob/main/a.txt`, { waitUntil: 'load' });
          const marked = await p.$$eval('.src .l.fm', (rows) => rows.map((x) => x.dataset.n));
          say(marked.join() === '4,5', `${w}: lines 4 and 5 are marked`, `${w}: marked ${marked}`);
          const links = await p.$$eval('.fm-link', (a) => a.map((x) => x.getAttribute('title')));
          say(links.length === 1 && links[0] === 'mind this', `${w}: one link carries the note`,
            `${w}: links ${links}`);
          await p.locator('.fm-link').click({ force: true });
          await p.waitForSelector('.basket-row', WAIT).then(
            () => r.ok(`${w}: the marker opens the basket`),
            () => r.fail(`${w}: the marker did not open the basket`));
          await p.keyboard.press('Escape');
          writeFileSync(join(dir, 'a.txt'), BODY.replace('line 4', 'changed 4'));
          await p.goto(`${srv.base}/worktree/blob/a.txt`, { waitUntil: 'load' });
          say((await p.$$('.fm-link')).length === 0, `${w}: edited lines lose the marker`,
            `${w}: a marker stayed on edited lines`);
          await p.close();
        } finally { await b.close(); }
      } catch (e) {
        r.fail(`${w}: ${e && e.stack}`);
      } finally {
        srv.stop();
        const gone = { recursive: true, force: true, maxRetries: 10, retryDelay: 200 };
        for (const d of [dir, cfg]) rmSync(d, gone);
      }
    }
  }
  return { basketSendAll, basketSendSubset, blobFindingMarker };
}
