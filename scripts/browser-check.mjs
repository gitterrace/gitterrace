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
const ALL = ['truncation', 'live-tabs', 'mermaid', 'chip', 'doctabs', 'chords'];
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

const checks = { truncation, 'live-tabs': liveTabs, mermaid, chip, doctabs, chords };
try {
  for (const c of wanted) { console.log(`-- ${c}`); await checks[c](); }
} finally {
  srv.stop();
  rmSync(dir, { recursive: true, force: true });
  rmSync(cfg, { recursive: true, force: true });
}
process.exit(r.result());
