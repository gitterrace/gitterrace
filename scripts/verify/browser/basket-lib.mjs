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
  return { basketSendAll };
}
