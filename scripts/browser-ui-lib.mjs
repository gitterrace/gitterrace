// Checks of the interactive UI: marked text, gutter, overlays, tips, repo switcher. Imported by scripts/browser-check.mjs, not run on its own.
import { writeFileSync, rmSync, mkdtempSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { makeRepo, serve, sh } from './browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (pw, dir, srv, r, bin, cfg, arg, ...) */
export function checks(env) {
  const { pw, dir, srv, r, browsers, tabCounts, marker, context, arg, bin, cfg } = env;
// ---- marked -----------------------------------------------------------------------------------
async function marked() {
  writeFileSync(join(dir, 'marked.txt'), 'first line\nsecond line\nthird line\nfourth line\n');
  writeFileSync(join(dir, 'README.md'), '# Fixture\n\nA sentence worth quoting.\n');
  sh(dir, 'git', 'add', '-A');
  sh(dir, 'git', 'commit', '-q', '-m', 'docs: text to mark');
  await new Promise((ok) => setTimeout(ok, 600));
  const mark = (sel) => (s) => {
    const r = document.createRange();
    if (s === 'lines') {
      const a = document.getElementById('L2'), z = document.getElementById('L3');
      r.setStart(a, 0); r.setEnd(z, z.childNodes.length);
    } else {
      const w = document.createTreeWalker(document.querySelector('.markdown-body'), NodeFilter.SHOW_TEXT);
      let t; while ((t = w.nextNode())) if (t.textContent.trim().length > 3) break;
      r.setStart(t, 0); r.setEnd(t, t.textContent.length);
    }
    const g = getSelection(); g.removeAllRanges(); g.addRange(r);
    return r.toString().trim();
  };
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    await p.goto(srv.base + '/blob/HEAD/marked.txt', { waitUntil: 'load' });
    const isCode = await p.$('#L3');
    if (!isCode) { r.fail(`marked ${name}: no code lines to mark`); await b.close(); continue; }
    await p.evaluate(mark(), 'lines');
    await p.waitForTimeout(300);
    const shown = await p.evaluate(() => { const m = document.getElementById('actmark'); return !!m && !m.hidden; });
    if (!shown) { r.fail(`marked ${name}: no trigger appears for marked lines`); await b.close(); continue; }
    await p.click('#actmark');
    await p.keyboard.press('s');
    await p.waitForSelector('.send-ctx');
    const send = await p.evaluate(() => ({ title: /** @type {HTMLInputElement} */ (document.querySelector('.send-title')).value, ctx: /** @type {HTMLTextAreaElement} */ (document.querySelector('.send-ctx')).value }));
    const line3 = await p.$eval('#L3', (e) => e.textContent.trim());
    if (shown && /:L2-3$/.test(send.title) && send.ctx.includes('#L2-L3') && send.ctx.includes(line3)) r.ok(`marked ${name}: lines 2-3 reach Send to`);
    else r.fail(`marked ${name}: trigger=${shown} title=${send.title}`);
    await p.keyboard.press('Escape');
    await p.goto(srv.base + '/', { waitUntil: 'load' });
    const phrase = await p.evaluate(mark(), 'prose');
    await p.waitForTimeout(300);
    await p.keyboard.press('.');
    await p.keyboard.press('s');
    await p.waitForSelector('.send-ctx');
    const ctx2 = await p.$eval('.send-ctx', (e) => e.value);
    if (phrase && ctx2.includes('> ' + phrase)) r.ok(`marked ${name}: README prose arrives as a quote`);
    else r.fail(`marked ${name}: quote missing for ${JSON.stringify(phrase)}`);
    await b.close();
  }
}

// ---- gutter -----------------------------------------------------------------------------------
async function gutter() {
  writeFileSync(join(dir, 'long.txt'), Array.from({ length: 250 }, (_, i) => `line ${i + 1}`).join('\n') + '\n');
  sh(dir, 'git', 'add', '-A');
  sh(dir, 'git', 'commit', '-q', '-m', 'docs: a file across chunks');
  await new Promise((ok) => setTimeout(ok, 600));
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    await p.setViewportSize({ width: 1200, height: 900 });
    await p.goto(srv.base + '/blob/HEAD/long.txt#L99-L103', { waitUntil: 'load' });
    await p.waitForTimeout(300);
    // The number shown before a line is its ::before content. Every number must equal its line.
    const nums = await p.evaluate(() =>
      [...document.querySelectorAll('.src .l')].map((l) =>
        getComputedStyle(l, '::before').content.replace(/"/g, '')));
    const wrong = nums.map((n, i) => [i + 1, n]).filter(([want, n]) => String(want) !== n);
    const shown = await p.evaluate(() => !!document.getElementById('actsel'));
    if (!wrong.length && nums.length === 250) r.ok(`gutter ${name}: 250 lines numbered 1..250 with the trigger shown=${shown}`);
    else r.fail(`gutter ${name}: ${wrong.length} wrong, first ${JSON.stringify(wrong.slice(0, 3))}`);
    // Send to opens small and grows with its text. Measured in Chromium: headless WebKit
    // stalls on this menu-then-dialog sequence (the marked check covers its dialog).
    if (name !== 'chromium') { await b.close(); continue; }
    await p.click('#actsel');
    await p.keyboard.press('s');
    await p.waitForSelector('.send-ctx');
    const size = await p.evaluate(async () => {
      const box = document.querySelector('.ov.send'), t = /** @type {HTMLTextAreaElement} */ (document.querySelector('.send-ctx'));
      const before = { w: box.getBoundingClientRect().width, h: t.getBoundingClientRect().height };
      t.value += '\n'.repeat(30) + 'end'; t.dispatchEvent(new Event('input'));
      await new Promise((ok) => requestAnimationFrame(() => requestAnimationFrame(ok)));
      return { ...before, grown: t.getBoundingClientRect().height, cap: innerHeight * 0.6 + 1 };
    });
    if (size.w <= 521 && size.h < 300 && size.grown > size.h && size.grown <= size.cap) r.ok(`gutter ${name}: Send to ${Math.round(size.w)}x${Math.round(size.h)} grows to ${Math.round(size.grown)}`);
    else r.fail(`gutter ${name}: Send to size ${JSON.stringify(size)}`);
    await b.close();
  }
}

// ---- overlays ---------------------------------------------------------------------------------
async function overlays() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    await p.goto(srv.base + '/', { waitUntil: 'load' });
    for (const key of ['t', '?', 'b']) {
      // A page link holds focus first, so "returns" means back to it, not to <body>.
      const opener = await p.evaluate(() => { const a = /** @type {HTMLElement} */ (document.querySelector('main a[href], a[href]')); a.focus(); a.dataset.opener = '1'; return !!a; });
      await p.keyboard.press(key);
      await p.waitForSelector('.ov-back [role=dialog]');
      const inside = () => p.evaluate(() => { const d = document.querySelector('.ov-back [role=dialog]'); return !!d && d.contains(document.activeElement); });
      const named = await p.evaluate(() => !!document.querySelector('.ov-back [role=dialog]').getAttribute('aria-label'));
      let kept = await inside();
      for (let i = 0; i < 5; i++) { await p.keyboard.press('Tab'); kept = kept && await inside(); }
      await p.keyboard.press('Shift+Tab'); kept = kept && await inside();
      await p.keyboard.press('Escape');
      const after = await p.evaluate(() => ({ open: !!document.querySelector('.ov-back'), back: document.activeElement instanceof HTMLElement && document.activeElement.dataset.opener === '1' }));
      if (opener && named && kept && !after.open && after.back) r.ok(`overlays ${name} ${key}: named, focus kept inside, Escape closes and returns focus`);
      else r.fail(`overlays ${name} ${key}: ${JSON.stringify({ named, kept, ...after })}`);
    }
    await b.close();
  }
}

// ---- tips -------------------------------------------------------------------------------------
// A closing overlay hands focus back to its opener. That focus is the script's, not the user's,
// so no tip may follow it: for every key-opened overlay, closed by Escape, no tip is on screen.
// Tab onto a tipped control still shows its tip at once.
async function tips() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    await p.goto(srv.base + '/', { waitUntil: 'load' });
    const tipOn = () => p.evaluate(() => { const t = document.getElementById('gt-tip'); return !!t && t.classList.contains('on') ? t.textContent : ''; });
    const bad = [];
    for (const keys of [['.'], ['.', 's'], ['t'], ['?'], ['b']]) {
      // The page-actions button holds focus first, as it does after a click on it.
      await p.evaluate(() => { const t = /** @type {HTMLElement} */ (document.querySelector('.actbtn.top')); if (t) t.focus(); });
      for (const k of keys) await p.keyboard.press(k);
      await p.waitForSelector('.ov-back');
      await p.keyboard.press('Escape');
      await p.waitForTimeout(50);
      const on = await tipOn();
      if (on) bad.push(`${keys.join('+')}: "${on}"`);
    }
    await p.evaluate(() => { if (document.activeElement instanceof HTMLElement) document.activeElement.blur(); const t = /** @type {HTMLElement} */ (document.querySelector('.actbtn.top')); const all = /** @type {HTMLElement[]} */ ([...document.querySelectorAll('a[href],button')]); const i = all.indexOf(t); if (i > 0) all[i - 1].focus(); });
    await p.keyboard.press('Tab');
    const tabbed = await tipOn();
    if (!bad.length && tabbed) r.ok(`tips ${name}: no tip after an overlay closes, Tab still shows "${tabbed}"`);
    else r.fail(`tips ${name}: ${JSON.stringify({ bad, tabbed })}`);
    await b.close();
  }
}

// ---- repo-switch ------------------------------------------------------------------------------
// A server and state folder per browser: the join reads the add secret from there, it must
// start on one repository, and the shared server above must keep one for the other checks.
async function repoSwitch() {
  const two = makeRepo();
  const saved = process.env.XDG_STATE_HOME;
  const shown = (p) => p.evaluate(() => { const s = document.getElementById('reposwitch'); return !!s && !s.hidden; });
  try {
    for (const name of browsers) {
      const state = mkdtempSync(join(tmpdir(), 'gt-state-'));
      process.env.XDG_STATE_HOME = state;
      const one = await serve(bin, dir, cfg);
      const { b, ctx } = await context(name);
      try {
        const p = await ctx.newPage();
        await p.goto(one.base + '/', { waitUntil: 'load' });
        await p.waitForTimeout(300);
        const alone = await shown(p);
        sh(two, bin, '--addr', one.origin.replace('http://', ''), '--config', cfg, two);
        // No reload: the open page hears the join over its live stream and shows the switcher.
        await p.waitForFunction(() => { const s = document.getElementById('reposwitch'); return !!s && !s.hidden; }, null, { timeout: 5000 }).catch(() => {});
        const joined = await shown(p);
        const visits = [];
        // The panel: filter focused, the current repository ticked, Escape hands focus back.
        let panel = null;
        if (joined) {
          await p.focus('#repobtn');
          await p.keyboard.press('Enter');
          await p.waitForSelector('.repopanel li');
          panel = await p.evaluate(() => ({ focus: document.activeElement?.className, cur: document.querySelectorAll('.repopanel li[aria-current] svg').length }));
          await p.keyboard.type('zzzz');
          panel.none = await p.textContent('.repopanel li.none');
          await p.keyboard.press('Escape');
          panel.back = await p.evaluate(() => document.activeElement?.id);
        }
        const panelOk = !!panel && panel.focus === 'ov-input' && panel.cur === 1 && panel.none === 'No repositories match' && panel.back === 'repobtn';
        for (const target of joined ? [two, dir] : []) {
          const want = target.split('/').pop().toLowerCase();
          await p.click('#repobtn');
          await p.waitForSelector('.ov-back li');
          const names = await p.$$eval('.ov-back li .name', (l) => l.map((e) => e.textContent));
          const pick = names.find((n) => n === want);
          if (pick) await Promise.all([p.waitForURL(`**/r/${pick}/`), p.click(`.ov-back li:has(.name:text-is("${pick}"))`)]);
          visits.push({ names, pick, at: new URL(p.url()).pathname });
        }
        // `git terrace remove` takes the second one off again: the open page hears it and the
        // switcher hides (one repository left), and the removed one's pages answer 404.
        sh(two, bin, '--addr', one.origin.replace('http://', ''), '--config', cfg, 'remove', two);
        await p.waitForFunction(() => { const s = document.getElementById('reposwitch'); return !s || s.hidden; }, null, { timeout: 5000 }).catch(() => {});
        const left = !(await shown(p));
        const gone = visits[0]?.pick ? (await fetch(`${one.origin}/r/${visits[0].pick}/`)).status : 0;
        const ok = !alone && joined && panelOk && visits.length === 2 && visits.every((v) => v.pick && v.at === `/r/${v.pick}/`) && left && gone === 404;
        if (ok) r.ok(`repo-switch ${name}: hidden alone, shown after the join without a reload, panel keyboard ok, lands on ${visits.map((v) => v.at).join(' then ')}, hidden again after remove`);
        else r.fail(`repo-switch ${name}: ${JSON.stringify({ alone, joined, panel, visits, left, gone })}`);
      } finally {
        await b.close();
        one.stop();
        rmSync(state, { recursive: true, force: true });
      }
    }
  } finally {
    if (saved === undefined) delete process.env.XDG_STATE_HOME; else process.env.XDG_STATE_HOME = saved;
    rmSync(two, { recursive: true, force: true });
  }
}

// The mark is two paths, ink and accent, meeting on one line. A renderer that antialiases each
// path alone lets the background through there as a hairline (WebKit did). Paint every mark at
// many sizes and offsets and compare the split rows with the solid rows just above and below.
  return { marked, gutter, overlays, tips, repoSwitch };
}
