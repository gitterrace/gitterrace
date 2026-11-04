// Checks of the interactive UI: marked text, the menu across views, gutter, overlays, tips, repo switcher. Imported by scripts/browser-check.mjs, not run on its own.
import { readFileSync, writeFileSync, rmSync, mkdtempSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { makeRepo, serve, sh } from './browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (pw, dir, srv, r, bin, cfg, arg, ...) */
export function checks(env) {
  const { pw, dir, srv, r, browsers, tabCounts, marker, context, arg, bin, cfg } = env;
// ---- marked -----------------------------------------------------------------------------------
// Marked text is what the menu acts on. Code lines 2-3 reach Send to with their text; a
// mid-word mark snaps to whole words; marked rendered Markdown (backticks, a link, a wrapped
// paragraph) sends the raw file lines byte for byte with their range; the title is never the
// bare path; the note ends in a source line that names the same lines.
const GUIDE = '# Guide\n\nIntro line.\n\nRun `git terrace` with the [config](config.md) file\nand it reads ``a ` b`` from the\nwrapped *paragraph* here.\n\nLast.\n';
const LIST = '- alpha\n  - bravo item\n    charlie line\n- delta\n';
async function marked() {
  // The shared README holds the mermaid diagram other checks read: put it back afterwards, so
  // the checks pass in any order.
  const readme = readFileSync(join(dir, 'README.md'));
  try { await markedIn(); } finally {
    writeFileSync(join(dir, 'README.md'), readme);
    sh(dir, 'git', 'add', '-A');
    sh(dir, 'git', 'commit', '-q', '-m', 'docs: restore the README');
    await new Promise((ok) => setTimeout(ok, 600));
  }
}
async function markedIn() {
  writeFileSync(join(dir, 'marked.txt'), 'first line\nsecond line\nthird line\nfourth line\n');
  writeFileSync(join(dir, 'README.md'), '# Fixture\n\nA sentence worth quoting.\n');
  mkdirSync(join(dir, 'notes'), { recursive: true });
  writeFileSync(join(dir, 'notes/guide.md'), GUIDE);
  writeFileSync(join(dir, 'notes/list.md'), LIST);
  sh(dir, 'git', 'add', '-A');
  sh(dir, 'git', 'commit', '-q', '-m', 'docs: text to mark');
  await new Promise((ok) => setTimeout(ok, 600));
  const raw = (a, b) => GUIDE.split('\n').slice(a - 1, b).join('\n');
  // Marks text in the page: whole code lines, a cut through a word, or a rendered block.
  const mark = ([how, arg]) => {
    const r = document.createRange();
    const text = (root, has) => { const w = document.createTreeWalker(root, NodeFilter.SHOW_TEXT); let t; while ((t = w.nextNode())) if (t.textContent.includes(has)) return t; return null; };
    if (how === 'lines') {
      const a = document.getElementById('L2'), z = document.getElementById('L3');
      r.setStart(a, 0); r.setEnd(z, z.childNodes.length);
    } else if (how === 'midword') {
      const t = text(document.getElementById('L2'), 'second');
      const i = t.textContent.indexOf('second');
      r.setStart(t, i + 3); r.setEnd(t, i + 8);
    } else if (how === 'word') {
      // From one word to the end of its text node, inside whatever blocks hold it.
      const t = text(document.querySelector('.markdown-body'), arg);
      if (!t) return '';
      r.setStart(t, t.textContent.indexOf(arg)); r.setEnd(t, t.textContent.length);
    } else {
      const blocks = [...document.querySelectorAll('.markdown-body [data-line]')].filter((e) => e.textContent.includes(arg));
      const el = blocks[blocks.length - 1];
      if (!el) return '';
      r.selectNodeContents(el);
    }
    const g = getSelection(); g.removeAllRanges(); g.addRange(r);
    return r.toString().trim();
  };
  const sendOpen = async (p, via) => {
    await p.waitForTimeout(300);
    if (via === 'button') await p.click('#actmark'); else await p.keyboard.press('.');
    await p.keyboard.press('s');
    await p.waitForSelector('.send-ctx');
    const v = await p.evaluate(() => ({ title: /** @type {HTMLInputElement} */ (document.querySelector('.send-title')).value, ctx: /** @type {HTMLTextAreaElement} */ (document.querySelector('.send-ctx')).value }));
    await p.keyboard.press('Escape');
    return v;
  };
  const lastLine = (s) => s.replace(/\n+$/, '').split('\n').pop();
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    await p.goto(srv.base + '/blob/HEAD/marked.txt', { waitUntil: 'load' });
    if (!(await p.$('#L3'))) { r.fail(`marked ${name}: no code lines to mark`); await b.close(); continue; }
    await p.evaluate(mark, ['lines']);
    await p.waitForTimeout(300);
    const shown = await p.evaluate(() => { const m = document.getElementById('actmark'); return !!m && !m.hidden; });
    if (!shown) { r.fail(`marked ${name}: no trigger appears for marked lines`); await b.close(); continue; }
    const send = await sendOpen(p, 'button');
    const line3 = await p.$eval('#L3', (e) => e.textContent.trim());
    if (send.ctx.includes('#L2-L3') && send.ctx.includes(line3) && send.title === 'marked.txt: second line third line') r.ok(`marked ${name}: lines 2-3 reach Send to, titled ${JSON.stringify(send.title)}`);
    else r.fail(`marked ${name}: lines 2-3 title=${JSON.stringify(send.title)}`);
    // A cut through "second line" snaps to the whole words.
    await p.evaluate(mark, ['midword']);
    const mid = await sendOpen(p);
    if (mid.title === 'marked.txt: second line' && mid.ctx.includes('first line\nsecond line\nthird line')) r.ok(`marked ${name}: a mid-word mark snaps to "second line" with a line of context each side`);
    else r.fail(`marked ${name}: mid-word mark gave title ${JSON.stringify(mid.title)}`);
    // Rendered Markdown: the wrapped paragraph is lines 5-7 of the file.
    await p.goto(srv.base + '/blob/HEAD/notes/guide.md', { waitUntil: 'networkidle' });
    if (!(await p.evaluate(mark, ['prose', 'wrapped']))) { r.fail(`marked ${name}: rendered Markdown has no data-line blocks`); await b.close(); continue; }
    const md = await sendOpen(p);
    const want = raw(5, 7);
    const src = lastLine(md.ctx);
    const bad = [];
    if (!md.ctx.includes('\n' + want + '\n')) bad.push('quote is not the raw lines 5-7');
    if (!md.ctx.includes('notes/guide.md#L5-L7')) bad.push('link is not #L5-L7');
    if (!/^source: [^\n]+@[0-9a-f]{40}:notes\/guide\.md#L5-L7$/.test(src)) bad.push(`source line ${JSON.stringify(src)}`);
    if (md.title === 'notes/guide.md' || !md.title.startsWith('guide.md: Run git terrace')) bad.push(`title ${JSON.stringify(md.title)}`);
    bad.length ? r.fail(`marked ${name}: rendered Markdown: ${bad.join(', ')}`) : r.ok(`marked ${name}: rendered Markdown sends raw lines 5-7 byte for byte, its range and source line`);
    // A short mark in prose carries the line around it.
    await p.evaluate(mark, ['prose', 'Last']);
    const short = await sendOpen(p);
    if (short.ctx.includes('#L9-L9') && short.ctx.includes('```md\n' + raw(8, 9) + '\n```')) r.ok(`marked ${name}: a short Markdown mark brings its neighbour lines`);
    else r.fail(`marked ${name}: short mark context ${JSON.stringify(short.ctx)}`);
    // A nested list item names its own lines, not those of the item that holds it.
    await p.goto(srv.base + '/blob/HEAD/notes/list.md', { waitUntil: 'networkidle' });
    await p.evaluate(mark, ['word', 'bravo']);
    const nest = await sendOpen(p);
    if (nest.ctx.includes('notes/list.md#L2-L3') && nest.ctx.includes(LIST.split('\n').slice(1, 3).join('\n'))) r.ok(`marked ${name}: a nested list item maps to its own lines 2-3`);
    else r.fail(`marked ${name}: nested list item: ${JSON.stringify(nest.ctx)}`);
    // README on the overview: the quote is its source line, the link names the file.
    await p.goto(srv.base + '/', { waitUntil: 'networkidle' });
    await p.evaluate(mark, ['prose', 'worth quoting']);
    const ctx2 = (await sendOpen(p)).ctx;
    if (ctx2.includes('\nA sentence worth quoting.\n') && ctx2.includes('README.md#L3-L3')) r.ok(`marked ${name}: README prose arrives as its source line`);
    else r.fail(`marked ${name}: README quote missing in ${JSON.stringify(ctx2)}`);
    await b.close();
  }
}

// ---- menu-same --------------------------------------------------------------------------------
// The action menu lists the same entries, in the same order, with the same labels in every view:
// the rendered README, its code, a line selection in it, a blame line, a commit. An action the
// selection does not fit stays in place, disabled, with a tip that says why.
async function menuSame() {
  const read = (p) => p.evaluate(() => [...document.querySelectorAll('.actmenu li[role=menuitem]')].map((li) => ({
    label: li.querySelector('.name').textContent,
    why: li.getAttribute('aria-disabled') === 'true' ? li.getAttribute('data-tip') || '(none)' : '',
  })));
  const open = async (p) => { await p.keyboard.press('.'); await p.waitForSelector('.actmenu li'); const e = await read(p); await p.keyboard.press('Escape'); return e; };
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    const views = {};
    await p.goto(srv.base + '/blob/HEAD/README.md', { waitUntil: 'load' });
    views.preview = await open(p);
    await p.click('[data-show=source]');
    views.code = await open(p);
    await p.goto(srv.base + '/blob/HEAD/README.md#L1', { waitUntil: 'load' });
    await p.click('[data-show=source]');
    await p.waitForSelector('#actsel');
    await p.click('#actsel'); await p.waitForSelector('.actmenu li');
    views.lines = await read(p); await p.keyboard.press('Escape');
    await p.goto(srv.base + '/log', { waitUntil: 'load' });
    views.log = await open(p);
    const labels = (v) => v.map((e) => e.label).join(' | ');
    const want = labels(views.preview);
    const off = Object.entries(views).filter(([, v]) => labels(v) !== want).map(([k, v]) => `${k}: ${labels(v)}`);
    const silent = Object.entries(views).flatMap(([k, v]) => v.filter((e) => e.why === '(none)').map((e) => `${k}: ${e.label}`));
    const anyOff = Object.values(views).some((v) => v.some((e) => e.why));
    if (!off.length && !silent.length && anyOff) r.ok(`menu-same ${name}: preview, code, lines and log list ${views.preview.length} entries alike`);
    else r.fail(`menu-same ${name}: want ${want}; differ ${JSON.stringify(off)}; no reason ${JSON.stringify(silent)}; disabled seen ${anyOff}`);
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
    if (size.w <= 521 && size.h < 340 && size.grown > size.h && size.grown <= size.cap) r.ok(`gutter ${name}: Send to ${Math.round(size.w)}x${Math.round(size.h)} grows to ${Math.round(size.grown)}`);
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

// ---- live-scroll ------------------------------------------------------------------------------
// A live refresh swaps <main> for a fresh render that folds every diff again. The reader's place
// must survive it: on the working tree page with 90 changed files, open at #f3 and scrolled to a
// file far down, an edit to another file refreshes the page, the file in view stays within 4 px
// of where it was and the bodies that were loaded are still there.
async function liveScroll() {
  const FILES = 90;
  const body = (tag) => Array.from({ length: 30 }, (_, k) => `line ${k} ${tag}`).join('\n') + '\n';
  const repo = mkdtempSync(join(tmpdir(), 'gt-livescroll-'));
  const state = mkdtempSync(join(tmpdir(), 'gt-state-'));
  const saved = process.env.XDG_STATE_HOME;
  process.env.XDG_STATE_HOME = state;
  let one;
  try {
    sh(repo, 'git', 'init', '-q', '-b', 'main');
    for (let i = 0; i < FILES; i++) writeFileSync(join(repo, `s${String(i).padStart(2, '0')}.txt`), body('base'));
    sh(repo, 'git', 'add', '-A');
    sh(repo, 'git', 'commit', '-q', '-m', 'base');
    for (let i = 0; i < FILES; i++) writeFileSync(join(repo, `s${String(i).padStart(2, '0')}.txt`), body('edited'));
    one = await serve(bin, repo, cfg);
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      try {
        const p = await ctx.newPage();
        await p.setViewportSize({ width: 1100, height: 800 });
        let ready = false;
        for (let i = 0; i < 40 && !ready; i++) {
          await p.goto(`${one.base}/worktree#f3`, { waitUntil: 'load' });
          ready = (await p.$$('section[data-path]')).length >= FILES;
          if (!ready) await p.waitForTimeout(250);
        }
        await p.evaluate(() => { window.__refreshed = 0; document.addEventListener('gt:live-refreshed', () => { window.__refreshed++; }); });
        // Far down: a file the page budget folded, which loads as it comes near.
        await p.evaluate(() => document.getElementById('f70').scrollIntoView());
        await p.waitForFunction(() => !document.querySelector('#f70 [data-load-diff]'), null, { timeout: 8000 }).catch(() => {});
        await p.waitForTimeout(500);
        await p.evaluate(() => window.scrollBy(0, -37));
        await p.waitForTimeout(300);
        const read = () => p.evaluate(() => ({ top: document.getElementById('f70').getBoundingClientRect().top, bodies: document.querySelectorAll('#main .diffwrap').length, f3: !document.querySelector('#f3 [data-load-diff]'), y: scrollY }));
        const before = await read();
        writeFileSync(join(repo, 's01.txt'), body('edited again'));
        await p.waitForFunction(() => window.__refreshed > 0, null, { timeout: 8000 }).catch(() => {});
        await p.waitForTimeout(2500);
        const after = await read();
        const bad = [];
        if (!before.bodies) bad.push('no loaded bodies before the refresh');
        if (!before.f3) bad.push('#f3 was not open before the refresh');
        if (!(await p.evaluate(() => window.__refreshed))) bad.push('no live refresh happened');
        if (Math.abs(after.top - before.top) > 4) bad.push(`file moved ${Math.round(after.top - before.top)} px (scrollY ${Math.round(before.y)} to ${Math.round(after.y)})`);
        if (after.bodies < before.bodies) bad.push(`${before.bodies} loaded bodies before, ${after.bodies} after`);
        if (!after.f3) bad.push('#f3 folded again');
        bad.length ? r.fail(`live-scroll ${name}: ${bad.join(', ')}`) : r.ok(`live-scroll ${name}: file held within 4 px, ${after.bodies} bodies kept, #f3 open`);
      } finally {
        await b.close();
      }
    }
  } catch (e) {
    r.fail(`live-scroll: ${e && e.stack}`);
  } finally {
    if (one) one.stop();
    if (saved === undefined) delete process.env.XDG_STATE_HOME; else process.env.XDG_STATE_HOME = saved;
    rmSync(repo, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    rmSync(state, { recursive: true, force: true });
  }
}

// The mark is two paths, ink and accent, meeting on one line. A renderer that antialiases each
// path alone lets the background through there as a hairline (WebKit did). Paint every mark at
// many sizes and offsets and compare the split rows with the solid rows just above and below.
  return { marked, menuSame, gutter, overlays, tips, repoSwitch, liveScroll };
}
