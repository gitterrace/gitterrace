// Checks of the page layout: truncation, live tabs, mermaid, chip, doctabs, chords, header.
// Imported by scripts/browser-check.mjs, not run on its own.
import { writeFileSync, rmSync, mkdirSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { root, WATCH_BOUND_MS, launch, openPage, sh } from './lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (pw, dir, srv, r, bin, cfg, arg, ...) */
export function checks(env) {
  const { pw, dir, srv, r, browsers, tabCounts, marker, context, arg, bin, cfg } = env;
// ---- truncation -------------------------------------------------------------------------------
// Runs in the page. Returns what is cut, broken, sparse or scrolling.
function measure() {
  const out = { cut: [], broken: [], sparse: [], subj: [], name: '', full: true, hscroll: false };
  // Visually hidden text (the .vh pattern: a 1 px box clipped to nothing) is for screen
  // readers by design, so it is hidden, not cut.
  const hidden = (e) => {
    const s = getComputedStyle(e), q = e.getBoundingClientRect();
    const clipped = s.clip === 'rect(0px, 0px, 0px, 0px)' || s.clipPath === 'inset(50%)';
    return q.width <= 1 && q.height <= 1 && clipped;
  };
  const overflowing = (e) => {
    if (hidden(e)) return false;
    const s = getComputedStyle(e);
    const clips = (s.overflowX !== 'visible' && s.overflowX !== 'auto' && s.overflowX !== 'scroll') || s.textOverflow === 'ellipsis';
    return clips && e.scrollWidth > e.clientWidth + 1 && e.clientWidth > 0
      && !e.classList.contains('subj');
  };
  out.cut = [...document.querySelectorAll('body *')].filter(overflowing).map((e) => `${e.tagName}.${e.className}`);
  // The repo switcher button is `hidden` while one repository is served, so take the one shown.
  const n = [...document.querySelectorAll('.reponame')].find((e) => /** @type {HTMLElement} */ (e).offsetParent);
  if (n) {
    const rc = n.getBoundingClientRect();
    out.name = n.textContent;
    out.full = n.scrollWidth <= n.clientWidth + 1 && rc.right <= innerWidth + 1 && rc.width > 0;
  }
  // A one-word link or cell spanning more than one line box was broken mid-word. A path drawn by
  // the path macro may break after a slash: there each part (.seg) must stay whole instead.
  const words = [...document.querySelectorAll('table.entries :is(td,th), table.entries :is(td,th) a, .rlist li a, .rlist li code, .cmain a, .cmain .muted, .reponame, .ov-list li, .tabs a')]
    .flatMap((e) => (e.querySelector('.seg') ? [...e.querySelectorAll('.seg')] : [e]));
  for (const e of new Set(words)) {
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
    const info = [.../** @type {HTMLTableRowElement} */ (row).cells].map((c) => {
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
    if (!(/** @type {HTMLElement} */ (e)).offsetParent) continue;
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
  const rows = [...document.querySelectorAll('li.crow')].filter((li) => /** @type {HTMLElement} */ (li).offsetParent);
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
  const paths = ['/', '/log', '/refs', '/tree/HEAD', '/tree/HEAD/src', '/stats', '/stats/hotspots', '/worktree'];
  for (const width of [360, 900, 1440]) for (const path of paths) {
    const p = await openPage(b, srv.base + path, { width, wait: 'networkidle' });
    const m = await p.evaluate(measure);
    const bad = m.cut.length || !m.full || m.broken.length || m.sparse.length || m.subj.length || m.hscroll;
    const line = `truncation ${width} ${path} cut=${JSON.stringify(m.cut)} subj=${JSON.stringify(m.subj.slice(0, 3))} sparse=${JSON.stringify(m.sparse.slice(0, 2))} broken=${JSON.stringify(m.broken.slice(0, 5))} hscroll=${m.hscroll}`;
    bad ? r.fail(line) : r.ok(`truncation ${width} ${path}`);
    await p.close();
  }
  // Red case: the visually hidden skip must not swallow real truncation. A subject cut with an
  // ellipsis at 360, and the same text clipped by overflow hidden, are still reported.
  const p = await openPage(b, srv.base + '/log', { width: 360, wait: 'networkidle' });
  await p.evaluate(() => {
    const cases = [['redcase-ellipsis', 'text-overflow:ellipsis'], ['redcase-hidden', '']];
    for (const [cls, css] of cases) {
      const e = document.createElement('div');
      e.className = cls;
      e.textContent = 'a commit subject far too long to fit in the narrow column of a phone';
      e.setAttribute('style', `width:120px;white-space:nowrap;overflow:hidden;${css}`);
      document.querySelector('main').prepend(e);
    }
  });
  const red = await p.evaluate(measure);
  for (const cls of ['redcase-ellipsis', 'redcase-hidden']) {
    const where = `truncation red case: ${cls}`;
    if (red.cut.some((c) => c.endsWith(`.${cls}`))) r.ok(`${where} is reported as cut`);
    else r.fail(`${where} was not reported, the hidden-text skip is too wide`);
  }
  await p.close();
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
    const ms = await Promise.all(pages.map((p) => p.waitForSelector('#wtchip:not([hidden])', { timeout: WATCH_BOUND_MS }).then(() => Date.now() - t, () => null)));
    ms.forEach((v, i) => { if (v == null) r.fail(`live-tabs ${name} tab ${i}: no working tree chip within the rescan bound of ${WATCH_BOUND_MS / 1000} s`); });
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
  // Mermaid styles its nodes from a <style> it writes at runtime. A page policy that refuses
  // inline styles leaves node shapes at the SVG default: a black fill.
  const fill = await p.$eval('pre.mermaid svg .node rect, pre.mermaid svg .node path',
    (n) => getComputedStyle(n).fill).catch(() => '');
  if (!fill || fill === 'rgb(0, 0, 0)') r.fail(`mermaid: diagram unstyled (node fill ${fill || 'none'})`);
  const labels = await p.$$eval('pre.mermaid .nodeLabel', (ns) => ns.map((n) => n.textContent.trim()));
  if (!labels.length) r.fail('mermaid: no diagram nodes rendered');
  else if (labels.includes('Copy')) r.fail(`mermaid: a Copy node leaked into the diagram: ${labels.join(', ')}`);
  else r.ok(`mermaid: ${labels.length} nodes, no Copy node`);
  await b.close();
}

// ---- chip -------------------------------------------------------------------------------------
async function chip() {
  writeFileSync(join(dir, 'a.txt'), 'one\ndirty\n');
  // The server reuses its "worktree differs" answer for DIRTY_TTL (2 s, app.rs), and an earlier
  // check may have just read a clean tree: wait it out so the page renders the new state.
  await new Promise((ok) => setTimeout(ok, 2600));
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
        // One way to each job in the bar: a tool that only repeats an action-menu entry is noise.
        const labels = tools.map((e) => (e.getAttribute('aria-label') || '').toLowerCase());
        const dup = labels.filter((l) => l && labels.indexOf(l) !== labels.lastIndexOf(l)).concat(labels.filter((l) => /^copy /.test(l)));
        const q = /** @type {HTMLInputElement | null} */ (document.getElementById('q'));
        const vague = !!q && !/\bcode\b/i.test(q.placeholder);
        return { rows, n: tools.length, dup, vague, hscroll: document.documentElement.scrollWidth > innerWidth + 1, folded: !!btn && getComputedStyle(btn).display !== 'none' };
      });
      const where = `${name} ${width}`;
      if (m.n < 2 || m.rows !== 1) r.fail(`header ${where}: tools split over ${m.rows} rows`);
      else if (m.hscroll) r.fail(`header ${where}: page scrolls sideways`);
      else if (m.dup.length) r.fail(`header ${where}: the bar repeats a menu action: ${m.dup.join(', ')}`);
      else if (m.vague) r.fail(`header ${where}: the search placeholder does not say what it searches`);
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
  return { truncation, liveTabs, mermaid, chip, doctabs, chords, header };
}
