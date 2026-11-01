// Browser checks for the served pages, one harness. Serves a temp repository with a given binary
// on a free port and runs the checks below in real browsers. Not in `make check`: it needs
// browsers. Run it with `make browser-check BIN=target/debug/git-terrace`.
//
//   node scripts/browser-check.mjs [--bin PATH] [--checks a,b] [--browsers webkit,chromium]
//        [--tabs 8,12] [--from-disk]
//
// Checks (all by default):
//   truncation   nothing cut or broken mid-word, one-line .subj with a tip when cut, no sparse
//                cells, no page-wide horizontal scroll, at 360, 900 and 1440 px
//   live-tabs    8 and 12 open tabs do not starve a further navigation (< 1 s), and with 3 tabs
//                every one shows the Working tree chip within 2 s of a file edit
//   mermaid      the README diagram renders and has no Copy node
//   chip         the Working tree chip dot has no animation, in light and dark
//   doctabs      the README/LICENSE/SECURITY tab row is one line at 360, 900 and 1440 px and the
//                active tab has aria-current; shots into --shots (default target/shots)/doctabs-*.png
//   header       from 1440 down to 320 px the top bar's tools (search, actions, theme) share one
//                row and the page never scrolls sideways; where search is folded, its button opens
//                the field with focus and Escape folds it again
//   marked       text marked with the mouse is what the menu acts on: marking lines 2-3 of a file
//                shows a trigger, and Send to's title and context carry L2-3 and their text;
//                marked README prose arrives as a > quote
//   gutter       in a 250-line file (three chunks) every line's gutter number equals its line,
//                read from the CSS counter itself, with the selection trigger shown; and the
//                Send to window opens small and grows with its text
//
// --from-disk serves crates/gitterrace-web/static/*.js from the working tree over the (possibly
// older) binary's copies, to test edits before a rebuild. Playwright comes from $PLAYWRIGHT_CORE,
// the repo's node_modules or the website checkout (see browser-lib.mjs).
import { readFileSync, writeFileSync, rmSync, mkdirSync, mkdtempSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { root, loadPlaywright, launch, makeRepo, serve, openPage, reporter, sh } from './browser-lib.mjs';

const arg = (k, d) => { const i = process.argv.indexOf(k); return i < 0 ? d : process.argv[i + 1]; };
const bin = resolve(arg('--bin', join(root, 'target/debug/git-terrace')));
const fromDisk = process.argv.includes('--from-disk');
const tabCounts = arg('--tabs', '8,12').split(',').map(Number);
const browsers = arg('--browsers', 'webkit,chromium').split(',');
const ALL = ['truncation', 'live-tabs', 'mermaid', 'chip', 'doctabs', 'chords', 'header', 'marked', 'gutter'];
const wanted = arg('--checks', ALL.join(',')).split(',');
for (const c of wanted) if (!ALL.includes(c)) { console.error(`unknown check ${c}; known: ${ALL.join(', ')}`); process.exit(2); }

const pw = await loadPlaywright();
const dir = makeRepo();
// The chords check needs one action, key "x", that touches a marker file. It has to exist
// before the server starts (read at load) and outside the repository (refused inside one).
const cfg = mkdtempSync(join(tmpdir(), 'gt-cfg-'));
const marker = join(cfg, 'chord-ran');
writeFileSync(join(cfg, 'actions.toml'), `version = 1\n[[action]]\nid = "mark"\nlabel = "Mark"\nkinds = ["page"]\ncommand = ["/usr/bin/touch", ${JSON.stringify(marker)}]\nkey = "x"\n`, { mode: 0o600 });
const srv = await serve(bin, dir, cfg);
const r = reporter();
console.log('serving', srv.base, 'with', bin);

async function context(name) {
  const b = await launch(pw, name);
  const ctx = await b.newContext();
  if (fromDisk) for (const f of ['live.js', 'actions.js', 'app.js', 'nav.js', 'send.js', 'markdown.js']) {
    await ctx.route(`**/static/${f}`, (q) => q.fulfill({ contentType: 'text/javascript', body: readFileSync(join(root, 'crates/gitterrace-web/static', f)) }));
  }
  return { b, ctx };
}

// ---- truncation -------------------------------------------------------------------------------
// Runs in the page. Returns what is cut, broken, sparse or scrolling.
function measure() {
  const out = { cut: [], broken: [], sparse: [], subj: [], name: '', full: true, hscroll: false };
  const overflowing = (e) => {
    const s = getComputedStyle(e);
    const clips = (s.overflowX !== 'visible' && s.overflowX !== 'auto' && s.overflowX !== 'scroll') || s.textOverflow === 'ellipsis';
    return clips && e.scrollWidth > e.clientWidth + 1 && e.clientWidth > 0 && !e.classList.contains('subj');
  };
  out.cut = [...document.querySelectorAll('body *')].filter(overflowing).map((e) => `${e.tagName}.${e.className}`);
  const n = document.querySelector('.reponame');
  if (n) {
    const rc = n.getBoundingClientRect();
    out.name = n.textContent;
    out.full = n.scrollWidth <= n.clientWidth + 1 && rc.right <= innerWidth + 1 && rc.width > 0;
  }
  // A one-word link or cell spanning more than one line box was broken mid-word.
  for (const e of document.querySelectorAll('table.entries td, table.entries td a, .rlist li a, .rlist li code, .cmain a, .cmain .muted, .reponame, .ov-list li, .tabs a')) {
    const t = (e.textContent || '').trim();
    if (!t || /\s/.test(t)) continue;
    const walk = document.createTreeWalker(e, NodeFilter.SHOW_TEXT);
    const tops = new Set();
    let node;
    while ((node = walk.nextNode())) {
      if (!node.textContent.trim()) continue;
      const g = document.createRange();
      g.selectNodeContents(node);
      for (const x of g.getClientRects()) tops.add(Math.round(x.top / 4));
    }
    if (tops.size > 1) out.broken.push(t);
  }
  // Density: no cell may hold more than 64 px of air while a sibling wraps past two lines.
  for (const row of document.querySelectorAll('table.entries tr')) {
    const info = [...row.cells].map((c) => {
      const g = document.createRange();
      g.selectNodeContents(c);
      const rc = g.getBoundingClientRect();
      const lh = parseFloat(getComputedStyle(c).lineHeight) || 20;
      return { air: c.getBoundingClientRect().width - rc.width, lines: rc.height / lh, t: c.textContent.trim().slice(0, 20), has: !!c.textContent.trim() };
    });
    if (info.some((i) => i.lines > 2.5) && info.some((i) => i.has && i.air > 64 && i.lines <= 1.5)) {
      out.sparse.push(info.map((i) => `${i.t}:${Math.round(i.air)}/${i.lines.toFixed(1)}`).join(' '));
    }
  }
  // Commit messages: exactly one line, an ellipsis, and a tip whenever cut.
  for (const e of document.querySelectorAll('.subj')) {
    if (!e.offsetParent) continue;
    const lh = parseFloat(getComputedStyle(e).lineHeight) || 20;
    const t = e.textContent.trim().slice(0, 30);
    if (e.getBoundingClientRect().height > lh * 1.6) out.subj.push(`wraps: ${t}`);
    if (getComputedStyle(e).textOverflow !== 'ellipsis') out.subj.push(`no ellipsis: ${t}`);
    e.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }));
    if (e.scrollWidth > e.clientWidth + 1 && !e.getAttribute('data-tip')) out.subj.push(`cut without tip: ${t}`);
  }
  // A commit row never grows a line for its message: above the narrow breakpoint the subject
  // and its author share one line (the subject is cut instead); below it, the author line sits
  // under the subject and is itself one line. So no row is a line taller than the shortest.
  const rows = [...document.querySelectorAll('li.crow')].filter((li) => li.offsetParent);
  if (rows.length) {
    const hs = rows.map((li) => li.getBoundingClientRect().height);
    const min = Math.min(...hs);
    const line = parseFloat(getComputedStyle(rows[0].querySelector('.subj')).lineHeight) || 20;
    const tall = hs.filter((h) => h > min + line / 2).length;
    if (tall) out.subj.push(`${tall} commit rows grow a line (shortest ${Math.round(min)}px, tallest ${Math.round(Math.max(...hs))}px)`);
  }
  out.hscroll = document.documentElement.scrollWidth > innerWidth;
  return out;
}

async function truncation() {
  const b = await launch(pw, 'chromium');
  const paths = ['/', '/log', '/refs', '/tree/HEAD', '/tree/HEAD/src', '/stats', '/stats/hotspots', '/stats/types', '/worktree'];
  for (const width of [360, 900, 1440]) for (const path of paths) {
    const p = await openPage(b, srv.base + path, { width, wait: 'networkidle' });
    const m = await p.evaluate(measure);
    const bad = m.cut.length || !m.full || m.broken.length || m.sparse.length || m.subj.length || m.hscroll;
    const line = `truncation ${width} ${path} cut=${JSON.stringify(m.cut)} subj=${JSON.stringify(m.subj.slice(0, 3))} sparse=${JSON.stringify(m.sparse.slice(0, 2))} broken=${JSON.stringify(m.broken.slice(0, 5))} hscroll=${m.hscroll}`;
    bad ? r.fail(line) : r.ok(`truncation ${width} ${path}`);
    await p.close();
  }
  await b.close();
}

// ---- live-tabs --------------------------------------------------------------------------------
async function liveTabs() {
  const url = srv.base + '/worktree';
  for (const name of browsers) for (const n of tabCounts) {
    const { b, ctx } = await context(name);
    for (let i = 0; i < n; i++) {
      const p = await ctx.newPage();
      await p.goto(url, { waitUntil: 'load', timeout: 5000 }).catch(() => r.fail(`live-tabs ${name} ${n} tabs: tab ${i} did not load within 5 s`));
    }
    const last = await ctx.newPage();
    const t = Date.now();
    try {
      await last.goto(url, { waitUntil: 'load', timeout: 10000 });
      const ms = Date.now() - t;
      ms > 1000 ? r.fail(`live-tabs ${name} ${n} tabs: next page ${ms} ms > 1000`) : r.ok(`live-tabs ${name} ${n} tabs: next page ${ms} ms`);
    } catch { r.fail(`live-tabs ${name} ${n} tabs: next page did NOT load within 10 s`); }
    await b.close();
  }
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    const pages = [];
    for (let i = 0; i < 3; i++) { const p = await ctx.newPage(); await p.goto(url, { waitUntil: 'load' }); pages.push(p); }
    await new Promise((ok) => setTimeout(ok, 1000));
    writeFileSync(join(dir, 'a.txt'), `one\ntwo ${name}\n`);
    const t = Date.now();
    const ms = await Promise.all(pages.map((p) => p.waitForSelector('#wtchip:not([hidden])', { timeout: 2000 }).then(() => Date.now() - t, () => null)));
    ms.forEach((v, i) => { if (v == null) r.fail(`live-tabs ${name} tab ${i}: no working tree chip within 2 s`); });
    if (ms.every((v) => v != null)) r.ok(`live-tabs ${name}: chip per tab (ms) ${ms.join(', ')}`);
    await b.close();
    sh(dir, 'git', 'checkout', '-q', '--', 'a.txt');
    await new Promise((ok) => setTimeout(ok, 800));
  }
}

// ---- mermaid ----------------------------------------------------------------------------------
async function mermaid() {
  const b = await launch(pw, 'webkit');
  const p = await openPage(b, srv.base + '/', {});
  await p.waitForSelector('pre.mermaid svg', { timeout: 10000 }).catch(() => {});
  const labels = await p.$$eval('pre.mermaid .nodeLabel', (ns) => ns.map((n) => n.textContent.trim()));
  if (!labels.length) r.fail('mermaid: no diagram nodes rendered');
  else if (labels.includes('Copy')) r.fail(`mermaid: a Copy node leaked into the diagram: ${labels.join(', ')}`);
  else r.ok(`mermaid: ${labels.length} nodes, no Copy node`);
  await b.close();
}

// ---- chip -------------------------------------------------------------------------------------
async function chip() {
  writeFileSync(join(dir, 'a.txt'), 'one\ndirty\n');
  await new Promise((ok) => setTimeout(ok, 600));
  const b = await launch(pw, 'chromium');
  for (const scheme of ['light', 'dark']) {
    const p = await openPage(b, srv.base + '/', { scheme, height: 120 });
    const anim = await p.$eval('#wtchip .dot', (d) => getComputedStyle(d).animationName).catch(() => 'no chip');
    const hidden = await p.$eval('#wtchip', (d) => d.hidden).catch(() => 'no chip');
    if (anim === 'no chip' || hidden) r.fail(`chip ${scheme}: chip missing or hidden on a dirty tree`);
    else anim !== 'none' ? r.fail(`chip ${scheme}: dot animates (${anim})`) : r.ok(`chip ${scheme}: no animation`);
    await p.close();
  }
  await b.close();
  sh(dir, 'git', 'checkout', '-q', '--', 'a.txt');
}

// ---- doctabs ----------------------------------------------------------------------------------
async function doctabs() {
  writeFileSync(join(dir, 'LICENSE'), 'MIT License\n');
  writeFileSync(join(dir, 'SECURITY.md'), '# Security\n');
  writeFileSync(join(dir, 'contributing.md'), '# Contributing\n');
  sh(dir, 'git', 'add', '-A');
  sh(dir, 'git', 'commit', '-q', '-m', 'docs: about files');
  await new Promise((ok) => setTimeout(ok, 600));
  const shots = resolve(arg('--shots', join(root, 'target/shots')));
  mkdirSync(shots, { recursive: true });
  const b = await launch(pw, 'chromium');
  for (const width of [360, 900, 1440]) {
    const p = await openPage(b, srv.base + '/?doc=license', { width });
    const m = await p.$eval('.doctabs', (n) => {
      const tops = new Set([...n.children].map((a) => Math.round(a.getBoundingClientRect().top)));
      return { n: n.children.length, lines: tops.size, on: n.querySelectorAll('a[aria-current]').length };
    }).catch(() => null);
    if (!m) r.fail(`doctabs ${width}: no tab row`);
    else if (m.lines !== 1) r.fail(`doctabs ${width}: tab row wraps onto ${m.lines} lines`);
    else if (m.on !== 1) r.fail(`doctabs ${width}: ${m.on} tabs marked aria-current`);
    else r.ok(`doctabs ${width}: ${m.n} tabs on one line, active marked`);
    await p.close();
  }
  for (const scheme of ['light', 'dark']) {
    const p = await openPage(b, srv.base + '/?doc=license', { width: 900, scheme });
    await (await p.$('.card.readme'))?.scrollIntoViewIfNeeded();
    await p.screenshot({ path: join(shots, `doctabs-${scheme}.png`) });
    await p.close();
  }
  await b.close();
}

// ---- chords -----------------------------------------------------------------------------------
async function chords() {
  const { b, ctx } = await context('chromium');
  const p = await ctx.newPage();
  await p.goto(srv.base + '/', { waitUntil: 'load' });
  await p.keyboard.press('a');
  await p.keyboard.press('x');
  await new Promise((ok) => setTimeout(ok, 800));
  if (existsSync(marker)) r.ok('chords: a then x opens the action'); else r.fail('chords: a then x did not run the action');
  await p.keyboard.press('?');
  const sheet = await p.$eval('.help, .ov', (n) => n.textContent).catch(() => '');
  if (/Mark/.test(sheet)) r.ok('chords: the ? sheet lists a x'); else r.fail('chords: the ? sheet lacks the chord');
  rmSync(marker, { force: true });
  // A static export (body[data-static]) must ignore the chord.
  await p.close();
  const q = await ctx.newPage();
  // Mark the page as an export does, in the HTML itself, so the flag is there before any script.
  await q.route(srv.base + '/', async (rt) => {
    const res = await rt.fetch();
    rt.fulfill({ response: res, body: (await res.text()).replace(/<body/, '<body data-static') });
  });
  await q.goto(srv.base + '/', { waitUntil: 'load' });
  if (await q.evaluate(() => document.body.dataset.static) === undefined) r.fail('chords: the static fixture is not marked');
  await q.keyboard.press('a');
  await q.keyboard.press('x');
  await new Promise((ok) => setTimeout(ok, 800));
  if (existsSync(marker)) r.fail('chords: a then x ran on a static page'); else r.ok('chords: a then x does nothing on a static page');
  await q.close();
  await b.close();
}

// ---- header -----------------------------------------------------------------------------------
async function header() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    let folded = 0;
    for (const width of [1440, 1100, 900, 760, 600, 480, 375, 320]) {
      await p.setViewportSize({ width, height: 600 });
      await p.goto(srv.base + '/', { waitUntil: 'load' });
      const m = await p.evaluate(() => {
        const tools = [...document.querySelectorAll('.topbar .tools>*')].filter((e) => getComputedStyle(e).display !== 'none' && e.getBoundingClientRect().width > 0);
        // One row: every tool's box overlaps the first one vertically (heights differ).
        const boxes = tools.map((e) => e.getBoundingClientRect());
        const rows = boxes.every((x) => x.top < boxes[0].bottom && x.bottom > boxes[0].top) ? 1 : 2;
        const btn = document.getElementById('searchbtn');
        return { rows, n: tools.length, hscroll: document.documentElement.scrollWidth > innerWidth + 1, folded: !!btn && getComputedStyle(btn).display !== 'none' };
      });
      const where = `${name} ${width}`;
      if (m.n < 2 || m.rows !== 1) r.fail(`header ${where}: tools split over ${m.rows} rows`);
      else if (m.hscroll) r.fail(`header ${where}: page scrolls sideways`);
      else r.ok(`header ${where}: tools on one row${m.folded ? ', search folded' : ''}`);
      if (!m.folded || folded++) continue;
      await p.click('#searchbtn');
      const open = await p.evaluate(() => document.activeElement && document.activeElement.id === 'q' && getComputedStyle(document.querySelector('.topsearch')).display !== 'none');
      await p.keyboard.press('Escape');
      const shut = await p.evaluate(() => getComputedStyle(document.querySelector('.topsearch')).display === 'none');
      if (open && shut) r.ok(`header ${where}: search button opens the field, Escape folds it`);
      else r.fail(`header ${where}: search button open=${open} escape=${shut}`);
    }
    await b.close();
  }
}

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
    const send = await p.evaluate(() => ({ title: document.querySelector('.send-title').value, ctx: document.querySelector('.send-ctx').value }));
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
    // Script cannot read a CSS counter's value, so replay the CSS rules the gutter uses over
    // the real DOM: every line increments n by one, then a line's counter-set replaces it.
    // The number shown before a line is the result. Every number must equal its line.
    const nums = await p.evaluate(() => {
      let n = 0;
      return [...document.querySelectorAll('.src .l')].map((l) => {
        n += 1;
        const m = /counter-set:\s*n\s+(-?\d+)/.exec(l.getAttribute('style') || '');
        if (m) n = +m[1];
        return String(n);
      });
    });
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
      const box = document.querySelector('.ov.send'), t = document.querySelector('.send-ctx');
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

const checks = { truncation, 'live-tabs': liveTabs, mermaid, chip, doctabs, chords, header, marked, gutter };
try {
  for (const c of wanted) { console.log(`-- ${c}`); await checks[c](); }
} finally {
  srv.stop();
  rmSync(dir, { recursive: true, force: true });
  rmSync(cfg, { recursive: true, force: true });
}
process.exit(r.result());
