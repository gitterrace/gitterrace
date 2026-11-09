#!/usr/bin/env node
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
//                active tab has aria-current; shots into --shots (default
//                target/shots)/doctabs-*.png
//   header       from 1440 down to 320 px the top bar's tools (search, actions, theme) share one
//                row and the page never scrolls sideways; where search is folded, its button opens
//                the field with focus and Escape folds it again; no tool repeats a menu action
//                (a Copy ... button) and the search placeholder names what it searches
//   marked       text marked with the mouse is what the menu acts on: marking lines 2-3 of a file
//                shows a trigger and Send to carries L2-3, their text and a title that is not the
//                bare path; a mid-word mark snaps to whole words with a line of context; marked
//                rendered Markdown (backticks, a link, a wrapped paragraph) sends the raw lines
//                5-7 byte for byte with #L5-L7 and a matching source: line; README prose arrives
//                as its source line
//   md-anchor    a link to another file's #heading, a typed URL and a hash change land on the
//                heading, whose rendered id is the plain slug (JS on and off)
//   overlays     t, ? and b each open a named dialog that takes focus, keeps it through 5 Tabs
//                (and Shift+Tab), closes on Escape and gives focus back to what had it
//   menu-keys    the action menu says its letter keys work only while it is open; closed, they
//                do nothing and the ? sheet lists none of them
//   menu-same    the action menu lists the same entries in the same order in the README preview,
//                its code, a line selection and the log; an entry the selection does not fit is
//                disabled with a tip saying why, never dropped
//   gutter       in a 250-line file (three chunks) every line's gutter number equals its line,
//                read from the CSS counter itself, with the selection trigger shown; and the
//                Send to window opens small and grows with its text
//   tips         closing ., .+s, t, ? or b with Escape leaves no tip on the opener they hand
//                focus back to, while Tab onto a tipped button still shows its tip
//   sticky-line  scrolled past its top, each view's head (overview README, Markdown preview and
//                code, blob, blame, diff file, table) is pinned with a 1 px line under it, never
//                over it, as wide as its content box, the outline button inside the bar, the
//                Code view full width
//   date-tips    hovering or focusing a relative date, in the tree listing and on the
//                overview, shows its full date in the tip
//   outline      "On this page": a rail at 1400 px, a menu at 900 px, a click scrolls and sets
//                the hash, the active item follows the scroll, filter and Escape work, the
//                Code tab hides it
//   settings     the gear opens the settings page; a form edit shows in Plain as the server's
//                text, an invalid plain edit marks its line in the gutter and keeps Save off, and
//                Save writes the file and reloads with the new value, and an action card edit
//                saves actions.toml in place, in each browser
//   repo-switch  a server on one repository hides the switcher (a fresh load paints it in
//                place); when a second `git terrace` joins, the open page shows it without a
//                reload, it lists both, and picking
//                each lands on its /r/NAME/ page; `git terrace remove` hides it again and
//                the removed repository's page answers 404
//   home-live    a server on a folder of repositories: a repository folder made under it shows
//                on the open home without a reload, and goes once deleted, filter text kept
//   about-place  on the overview, About sits beside the files when there are two columns and
//                above them when there is one, at every width from 1440 to 320 px
//   mark-seam    every two-colour mark (docs/brand/mark-*.svg, the in-app header logo) at 16 to
//                133 px, 1x and 2x, on light and dark: no pixel on the colour change is closer
//                to the background than both colours are, in every browser
//   site-theme   the generated site/ (served with its _headers CSP): first load follows the
//                system scheme with the theme already set in the head, the button cycles system,
//                light, dark, the choice survives a reload, the shown logo and screenshots are
//                the theme's and the other scheme's are never fetched, no CSP violation, and each
//                theme icon is the source-link mark's size
//   site-bar     every page of the generated site/, 404 included, at 390 and 1440 px in light
//                and dark: one bar right after the skip link, dark with light text in both
//                schemes, Docs, the source link and the theme button in view, no sideways
//                scroll
//   site-rail    the docs pages: at 1440 px a page with three or more h2 shows the "On this
//                page" rail beside the text (11 px mono label with a hairline, 2 px rail) and
//                scrolling to a heading marks its item alone in the accent; at 390 px and on
//                shorter pages no rail
//   site-tables  every table on every site page, at 390 and 1440 px: no cell text wraps inside a
//                token (so no inline code split mid-name), and no cell narrower than two em
//   side-width   on the overview the Recent sparkline is exactly as wide as the language bar
//                (within 1 px) and as tall, at 1440, 1100 and 360 px, light and dark
//   calm-header  at rest the branch switcher and the Working tree chip have no visible border
//                and are not bold, and toggling the chip does not move the search field, at
//                1440, 900 and 360 px, light and dark
//   live-scroll  on the working tree page with 90 changed files, open at #f3 and scrolled to
//                a file far down, a live refresh keeps that file within 4 px of where it was,
//                keeps the loaded
//                bodies and keeps #f3 open
//   live-read    the same page while the reader scrolls on (wheel steps, PageDown, Space) and
//                refreshes keep landing: the file in view never jumps back more than 4 px, the
//                reader ends where they scrolled to and #f3 stays in the URL and open
//   tab-levels   at 1440 and 900 px a page's tabs never share the header nav's font and case,
//                every insights page is a tab of the row (no More) and the open one is marked
//   anatomy      every page opens with the one page head: slots in one order, one h1 in it,
//                one tab row of two or more, the same top, title size and gaps (1440 and 390
//                px, both schemes)
//   page-heads   a Markdown file: the page head holds the path and File/Blame/History tabs, the
//                card one sticky head row with Preview/Code, at 1440, 900 and 390 px, light
//                and dark
//   compare-head the compare form sits in the head's toolbar, 'Open the head commit' has no row
//                and the head sha is a link, at 1440 and 900 px (390: form still in the toolbar)
//   commit-head  the meta line sits right under the page head with no Copy button and the
//                changed-files header carries the totals, at 1440, 900 and 390 px
//   search-head  the search bar is no card: query and Code/Commits on row 1, toggles on row 2,
//                the summary is the head's tagline, at 1440, 900 and 390 px
//   seg-one-row  every segmented control on the insights pages and a document shows one row of
//                choices or its native select, never two rows, at 360 and 1440 px, light and dark
//   period-steady the period control keeps its x and y with no filter, a period, and a period
//                plus a path chip, and the chips row sits below the counting line, at 360 and
//                1400 px
//   export-from-disk  `export` output opened with a file:// URL: the forwarding index lands on the
//                repository, content and styles show, a link opens a second page, no form that
//                needs the server shows on the overview, log, compare or search pages, and no
//                script logs an error
//   mark-flat    every highlighted match on the search page is one background with no outline,
//                border, ring or padding, exactly as wide as its letters
//   tab-stops    on the overview, activity, hotspots and log the first Tab is a visible skip link
//                whose Enter moves the next Tab into main, and no chart is more than one tab stop
//   help-keys    in the ? sheet keys and descriptions each start at one x, the key column is at
//                least 64 px, and a chord joins its keys with a word, closer than the column gap
//   side-order   with a branch left unmerged, the overview sidebar reads Now, Loose ends, Recent
//   axe-all      axe-core (WCAG 2 A and AA to 2.2) over every golden page, both browsers, 360 and
//                1440 px, light and dark; only the violations in KNOWN_VIOLATIONS pass, and
//                that list can only shrink. AXE_REPORT=1 prints counts and fails nothing
//   forced-colors  Chromium with forced colors on: every Tab stop has an outline, the open tab
//                differs from its siblings by more than color
//   reduced-motion with reduce on, every transition and animation duration is 0s and none runs
//   small-layout the tree '..' row only at narrow widths, the insights link in the Languages
//                heading, and 'Branches' in the nav at 390 px.
//   lone-rows    on every page the goldens cover, at 1440 and 390 px, no control row above the
//                first content block holds a single control using under 40% of its width;
//                pages red today are listed as known with the change that fixes them
//   graph-view   the log's Graph view on a wide merge history, at 390 and 1440 px: lines meet
//                across every row edge, the "+N" fold shows at 390, a ref chip keeps the row
//                height, nothing scrolls sideways, and the List | Graph switch keeps the page top;
//                tip labels are never clipped and overlap no label, graph line or subject
//   outline-filter  the "On this page" filter shows, narrows the list and Escape returns
//                focus, on a Markdown page and an insights page
//   repos-filter the Repositories home filter: typing narrows rows and count, Enter opens the
//                first match, Escape clears, nothing shifts, at 390 and 1440 px
//   symbols      a source file's symbol outline: at 1440 px a rail beside the code below the
//                pinned path bar, at 390 px a menu in the path bar; a link lands its line
//   send-dialog  the Send to window has a "Send to" head, Title and Notes labels, a Cancel that
//                closes it and the shortcut in words; at 390 px it is a bottom sheet with its
//                footer in view
//   layout-shift every page kind (tree, blob, docs, log, commit, blame, history, compare, search,
//                refs, overview, every insights tab) puts the header, nav, content edge, first
//                block and head title where the first one does, at 1440 and 390 px, light and dark
//   keyboard-walk  Tab through every golden page: each stop has a 2 px ring at 3:1, none sits under
//                the sticky header, no trap
//   reflow       at 320 px no page scrolls sideways except code and tables
//   text-spacing with the WCAG 1.4.12 overrides injected, no text is clipped
//   targets      every target is 24 px square or has no neighbor within 24 px
//   keys-off     the ? sheet's switch turns single-key shortcuts off and on
//   no-sideways-scroll  no region meant to scroll down (the outline rail and menu, any box with
//                a capped height) scrolls sideways, on every page kind and the site docs, at
//                1440, 900 and 360 px
//   head-fits    every child of a head row is inside the viewport and not cut by an ancestor,
//                on the same pages and widths
//   head-align   on every page kind of layout-shift plus the Graph log, at 1440 and 390 px, the
//                children of a head row that are or hold a control share one vertical centre
//                on each flex line, within 1 px
//   row-overlap  no two parts of a listed row (ref, badges, avatar, subject, author and time, id,
//                ahead and behind) overlap, on every list page, at 1440, 900, 600 and 360 px
//   huge-diff    a commit of 2,000 changed files (mostly pure renames) loads under a DOM bound,
//                shows a rename as one line, loads the last files on scroll, opens #fN, opens a
//                folded file with Enter, and prints scroll frame times and long tasks
//
// --from-disk serves crates/gitterrace-web/static/*.js from the working tree over the (possibly
// older) binary's copies, to test edits before a rebuild. Playwright comes from $PLAYWRIGHT_CORE
// or the repo's node_modules, installed by `make browser-deps` (see browser-lib.mjs).
import { readFileSync, readdirSync, writeFileSync, rmSync, mkdirSync, mkdtempSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { checks as pageChecks } from './browser-pages-lib.mjs';
import { checks as uiChecks } from './browser-ui-lib.mjs';
import { checks as layoutChecks } from './browser-layout-lib.mjs';
import { checks as siteChecks } from './browser-site-lib.mjs';
import { checks as siteBarChecks } from './browser-sitebar-lib.mjs';
import { checks as diffChecks } from './browser-diff-lib.mjs';
import { root, RESCAN_SECS, loadPlaywright, launch, makeRepo, serve, openPage, reporter, sh } from './browser-lib.mjs';

const arg = (k, d) => { const i = process.argv.indexOf(k); return i < 0 ? d : process.argv[i + 1]; };
const bin = resolve(arg('--bin', join(root, 'target/debug/git-terrace')));
const fromDisk = process.argv.includes('--from-disk');
const tabCounts = arg('--tabs', '8,12').split(',').map(Number);
const browsers = arg('--browsers', 'webkit,chromium').split(',');
const ALL = ['truncation', 'live-tabs', 'mermaid', 'chip', 'doctabs', 'chords', 'header', 'marked', 'menu-same', 'menu-keys', 'gutter', 'overlays', 'tips', 'date-tips', 'repo-switch', 'outline', 'mark-seam', 'about-place', 'site-theme', 'install-tabs', 'site-bar', 'site-rail', 'site-tables', 'side-width', 'calm-header', 'tabs-one-line', 'tab-levels', 'huge-diff', 'sticky-heads', 'md-anchor', 'live-scroll', 'live-read', 'sticky-line', 'seg-one-row', 'period-steady', 'export-from-disk', 'inline-controls', 'section-gaps', 'path-cells', 'mark-flat', 'tab-stops', 'help-keys', 'side-order', 'settings', 'lone-rows', 'page-heads', 'crumb-flow', 'compare-head', 'commit-head', 'search-head', 'small-layout', 'send-dialog', 'keyboard-walk', 'reflow', 'text-spacing', 'targets', 'keys-off', 'axe-all', 'forced-colors', 'reduced-motion', 'outline-filter', 'avatar-fit', 'graph-view', 'layout-shift', 'head-align', 'symbols', 'no-sideways-scroll', 'head-fits', 'repos-filter', 'home-live', 'row-overlap', 'anatomy'];
const wanted = arg('--checks', ALL.join(',')).split(',');
for (const c of wanted) if (!ALL.includes(c)) { console.error(`unknown check ${c}; known: ${ALL.join(', ')}`); process.exit(2); }

const pw = await loadPlaywright(browsers);
const dir = makeRepo();
// The chords check needs one action, key "x", that touches a marker file. It has to exist
// before the server starts (read at load) and outside the repository (refused inside one).
const cfg = mkdtempSync(join(tmpdir(), 'gt-cfg-'));
const marker = join(cfg, 'chord-ran');
writeFileSync(join(cfg, 'actions.toml'), `version = 1\n[[action]]\nid = "mark"\nlabel = "Mark"\nkinds = ["page"]\ncommand = ["/usr/bin/touch", ${JSON.stringify(marker)}]\nkey = "x"\n[[action]]\nid = "lines"\nlabel = "Lines only"\nkinds = ["line"]\ncommand = ["/usr/bin/true"]\n`, { mode: 0o600 });
writeFileSync(join(cfg, 'config.toml'), `[watch]\nrescan_secs = ${RESCAN_SECS}\n`);
const srv = await serve(bin, dir, cfg);
const r = reporter();
console.log('serving', srv.base, 'with', bin);

async function context(name, opts = {}) {
  const b = await launch(pw, name);
  const ctx = await b.newContext(opts);
  // Every script and stylesheet in the folder, read at run time, so a new or forgotten file is
  // never served stale from the binary while the check claims to test the working tree.
  if (fromDisk) for (const f of readdirSync(join(root, 'crates/gitterrace-web/static')).filter((n) => n.endsWith('.js') || n.endsWith('.css'))) {
    const type = f.endsWith('.css') ? 'text/css' : 'text/javascript';
    await ctx.route(`**/static/${f}`, (q) => q.fulfill({ contentType: type, body: readFileSync(join(root, 'crates/gitterrace-web/static', f)) }));
  }
  return { b, ctx };
}


const env = { pw, dir, srv, r, browsers, tabCounts, marker, context, arg, bin, cfg };
const { checks: anchorChecks } = await import('./browser-anchor-lib.mjs');
const { mdAnchor } = anchorChecks(env);
const { checks: scrollChecks } = await import('./browser-scroll-lib.mjs');
const { liveRead } = scrollChecks(env);
const { truncation, liveTabs, mermaid, chip, doctabs, chords, header } = pageChecks(env);
const { marked, menuKeys, menuSame, gutter, overlays, tips, dateTips, repoSwitch, liveScroll, outline } = uiChecks(env);
const { inlineControls, sectionGaps, pathCells } = layoutChecks(env);
const { markSeam, aboutPlace, siteTheme, exportFromDisk, installTabs } = siteChecks(env);
const { siteBar, siteRail, siteTables } = siteBarChecks(env);
const { hugeDiff } = diffChecks(env);
const { checks: stickyChecks } = await import('./browser-sticky-lib.mjs');
const { stickyLine } = stickyChecks(env);
const { checks: headsChecks } = await import('./browser-heads-lib.mjs');
const { pageHeads, crumbHeads, compareHeads, commitHeads, searchHeads } = headsChecks(env);
const { checks: a11yChecks } = await import('./browser-access-lib.mjs');
const {
  markFlat, tabStops, helpKeys, sideOrder, axeAll, forcedColors, reducedMotion,
} = a11yChecks(env);
const { checks: rowChecks } = await import('./browser-rows-lib.mjs');
const { loneRows, smallLayout, avatarFit } = rowChecks(env);
const { checks: sendChecks } = await import('./browser-send-lib.mjs');
const { sendDialog } = sendChecks(env);
const { checks: outlineChecks } = await import('./browser-outline-lib.mjs');
const { outlineFilter, symbols } = outlineChecks(env);
const { checks: graphChecks } = await import('./browser-graph-lib.mjs');
const { graphView } = graphChecks(env);
const { checks: shiftChecks } = await import('./browser-shift-lib.mjs');
const { layoutShift, headAlign } = shiftChecks(env);
const { checks: homeLiveChecks } = await import('./browser-homelive-lib.mjs');
const { homeLive } = homeLiveChecks(env);
const { checks: reposChecks } = await import('./browser-repos-lib.mjs');
const { reposFilter } = reposChecks(env);
const { checks: anatomyChecks } = await import('./browser-anatomy-lib.mjs');
const { anatomy } = anatomyChecks(env);
const { checks: kbChecks } = await import('./browser-keyboard-lib.mjs');
const { keyboardWalk, reflow, textSpacing, targets, keysOff } = kbChecks(env);
const { checks: fitChecks } = await import('./browser-fit-lib.mjs');
const { noSidewaysScroll, headFits } = fitChecks(env);
const { checks: overlapChecks } = await import('./browser-overlap-lib.mjs');
const { rowOverlap } = overlapChecks(env);
async function sideWidth() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const scheme of ['light', 'dark']) for (const width of [1440, 1100, 360]) {
      const p = await ctx.newPage();
      await p.emulateMedia({ colorScheme: scheme });
      await p.setViewportSize({ width, height: 900 });
      // The sidebar appears once the index is built: reload until it does.
      for (let i = 0; i < 40; i++) {
        await p.goto(srv.base + '/', { waitUntil: 'load' });
        if (await p.$('.side .langbar')) break;
        await p.waitForTimeout(250);
      }
      const m = await p.evaluate(() => {
        const s = document.querySelector('.side svg.spark'), l = document.querySelector('.side .langbar');
        if (!s || !l) return null;
        const a = s.getBoundingClientRect(), c = l.getBoundingClientRect();
        return { sw: a.width, lw: c.width, sh: a.height, lh: c.height };
      });
      const where = `side-width ${name} ${scheme} ${width}`;
      if (!m) r.fail(`${where}: no sparkline or language bar`);
      else if (Math.abs(m.sw - m.lw) > 1) r.fail(`${where}: sparkline ${m.sw} px wide, language bar ${m.lw} px`);
      else r.ok(`${where}: both ${Math.round(m.lw)} px wide`);
      await p.close();
    }
    await b.close();
  }
}
async function calmHeader() {
  writeFileSync(join(dir, 'a.txt'), 'one\ndirty\n');
  await new Promise((ok) => setTimeout(ok, 2600));
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const scheme of ['light', 'dark']) for (const width of [1440, 900, 360]) {
      const p = await ctx.newPage();
      await p.emulateMedia({ colorScheme: scheme });
      await p.setViewportSize({ width, height: 400 });
      await p.goto(srv.base + '/', { waitUntil: 'load' });
      await p.mouse.move(width - 1, 399);
      const m = await p.evaluate(() => {
        const look = (el) => { if (!el) return null; const s = getComputedStyle(el); const bw = ['Top', 'Right', 'Bottom', 'Left'].some((k) => parseFloat(s[`border${k}Width`]) > 0 && s[`border${k}Style`] !== 'none' && !/rgba\(.*, 0\)$|transparent/.test(s[`border${k}Color`])); return { border: bw, weight: Number(s.fontWeight) }; };
        const chip = document.getElementById('wtchip'), q = /** @type {HTMLElement | null} */ (document.querySelector('.topsearch'));
        const x = () => (q && q.offsetParent ? q.getBoundingClientRect().x : null);
        const out = { branch: look(document.getElementById('refbtn')), chip: look(chip), shown: chip && !chip.hidden };
        if (chip) { const a = x(); chip.hidden = !chip.hidden; const c = x(); chip.hidden = !chip.hidden; out.dx = a === null ? 0 : Math.abs(a - c); }
        return out;
      });
      const where = `calm-header ${name} ${scheme} ${width}`;
      const bad = [];
      if (!m.branch) bad.push('no branch switcher');
      else { if (m.branch.border) bad.push('switcher has a border at rest'); if (m.branch.weight >= 600) bad.push(`switcher weight ${m.branch.weight}`); }
      if (!m.shown) bad.push('chip not shown on a dirty tree');
      else { if (m.chip.border) bad.push('chip has a border at rest'); if (m.chip.weight >= 600) bad.push(`chip weight ${m.chip.weight}`); }
      if (m.dx > 0.5) bad.push(`search moves ${m.dx.toFixed(1)} px when the chip toggles`);
      bad.length ? r.fail(`${where}: ${bad.join(', ')}`) : r.ok(`${where}: quiet, search steady`);
      await p.close();
    }
    await b.close();
  }
  sh(dir, 'git', 'checkout', '-q', '--', 'a.txt');
}
// The section tabs stay one row at every width: on a narrow screen they scroll sideways instead
// of wrapping, the tab of the open section is fully in view, and an edge fade shows that more
// tabs are off screen (the scroll bar is hidden).
async function tabsOneLine() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const scheme of ['light', 'dark']) for (const width of [1440, 900, 360, 320]) for (const [row, path] of [['.tabnav', '/stats'], ['.tabs', '/stats/ownership']]) {
      const p = await ctx.newPage();
      await p.emulateMedia({ colorScheme: scheme });
      await p.setViewportSize({ width, height: 600 });
      await p.goto(srv.base + path, { waitUntil: 'load' });
      await p.evaluate(() => document.fonts.ready.then(() => new Promise((ok) => requestAnimationFrame(() => requestAnimationFrame(ok)))));
      const m = await p.evaluate((sel) => {
        const nav = document.querySelector(sel);
        if (!nav) return null;
        const tabs = [...nav.querySelectorAll('a')];
        const tops = new Set(tabs.map((a) => Math.round(a.getBoundingClientRect().top)));
        const on = nav.querySelector('a[aria-current="page"]');
        const n = nav.getBoundingClientRect(), o = on && on.getBoundingClientRect();
        const over = nav.scrollWidth > nav.clientWidth + 1;
        const mask = getComputedStyle(nav).maskImage || getComputedStyle(nav).webkitMaskImage || 'none';
        return { rows: tops.size, over, fade: mask !== 'none', onIn: !!o && o.left >= n.left - 0.5 && o.right <= n.right + 0.5, page: document.documentElement.scrollWidth > innerWidth + 1 };
      }, row);
      const where = `tabs-one-line ${name} ${scheme} ${width} ${row}`;
      const bad = [];
      if (!m) bad.push('no section tabs');
      else {
        if (m.rows !== 1) bad.push(`tabs in ${m.rows} rows`);
        if (!m.onIn) bad.push('the open tab is not fully visible');
        if (m.over && !m.fade) bad.push('tabs overflow with no edge fade');
        if (m.page) bad.push('the page scrolls sideways');
      }
      bad.length ? r.fail(`${where}: ${bad.join(', ')}`) : r.ok(`${where}: one row${m.over ? ', scrolls with a fade' : ''}`);
      await p.close();
    }
    await b.close();
  }
}
// Segmented controls never break across lines: on every insights page and a document, at 360
// and 1440 px, each control shows either one row of choices or its native select, never a
// second row and never wider than the page.
async function segOneRow() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const scheme of ['light', 'dark']) for (const width of [1440, 360]) {
      for (const path of ['/stats', '/stats/contributors', '/stats/hotspots', '/stats/ownership', '/stats/languages', '/blob/HEAD/README.md']) {
        const p = await ctx.newPage();
        await p.emulateMedia({ colorScheme: scheme });
        await p.setViewportSize({ width, height: 700 });
        await p.goto(srv.base + path, { waitUntil: 'load' });
        // Measure the settled page: after the web fonts and the frame that refits the controls.
        await p.evaluate(() => document.fonts.ready.then(() => new Promise((ok) => requestAnimationFrame(() => requestAnimationFrame(ok)))));
        const m = await p.evaluate(() => [...document.querySelectorAll('.segctl')].map((box) => {
          const segs = box.querySelector('.segs'), sel = box.querySelector('select');
          const shown = (e) => !!e && getComputedStyle(e).display !== 'none' && e.getClientRects().length > 0;
          const tops = new Set([...segs.children].map((c) => Math.round(c.getBoundingClientRect().top)));
          const r = box.getBoundingClientRect();
          return { name: box.getAttribute('aria-label'), segs: shown(segs), sel: shown(sel), rows: shown(segs) ? tops.size : 0, cut: segs.scrollWidth > segs.clientWidth + 1 && shown(segs), out: r.right > document.documentElement.clientWidth + 1 };
        }));
        const where = `seg-one-row ${name} ${scheme} ${width} ${path}`;
        const bad = [];
        for (const c of m) {
          if (c.segs === c.sel) bad.push(`${c.name}: ${c.segs ? 'both' : 'neither'} row and select shown`);
          if (c.rows > 1) bad.push(`${c.name}: ${c.rows} rows`);
          if (c.cut) bad.push(`${c.name}: choices cut off`);
          if (c.out) bad.push(`${c.name}: wider than the page`);
        }
        if (path.startsWith('/stats') && !m.length) bad.push('no segmented control');
        bad.length ? r.fail(`${where}: ${bad.join(', ')}`) : r.ok(`${where}: ${m.length} controls, one row or a select`);
        await p.close();
      }
    }
    await b.close();
  }
}
// Tab levels: a page's own tabs never share the header nav's type, and every insights page is
// one tab of the row: no More, and the open tab is marked in place.
async function tabLevels() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const width of [1440, 900]) {
      const p = await ctx.newPage();
      await p.setViewportSize({ width, height: 600 });
      await p.goto(srv.base + '/stats/ownership', { waitUntil: 'load' });
      const where = `tab-levels ${name} ${width}`;
      const look = await p.evaluate(() => {
        const f = (s) => { const e = document.querySelector(s); if (!e) return null; const c = getComputedStyle(e); return c.fontFamily + '|' + c.textTransform; };
        const on = document.querySelector('.tabs>a[aria-current="page"]');
        return { nav: f('.tabnav a'), page: f('.tabs>a'), more: !!document.querySelector('.tabs details, .tabs summary'), on: on ? on.textContent.trim() : '' };
      });
      const bad = [];
      if (!look.nav || !look.page) bad.push('missing tab rows');
      else if (look.nav === look.page) bad.push(`page tabs look like the header nav (${look.page})`);
      if (look.more) bad.push('the tab row still hides tabs behind More');
      if (look.on !== 'Ownership') bad.push(`open tab reads "${look.on}" on Ownership`);
      bad.length ? r.fail(`${where}: ${bad.join(', ')}`) : r.ok(`${where}: two tab levels, every tab in the row`);
      await p.close();
    }
    await b.close();
  }
}
// Period steady: the period control sits in one place whatever the filters are. With no
// filter, a period, and a period plus a path chip, its box starts at the same x and y, at 360
// and 1400 px, so choosing a period never moves the control under the pointer; and the chips
// row comes last, below the counting line, at both widths.
async function periodSteady() {
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const width of [1400, 360]) {
      const at = [];
      for (const q of ['', '?period=1w', '?period=1w&path=src']) {
        const p = await ctx.newPage();
        await p.setViewportSize({ width, height: 700 });
        await p.goto(srv.base + '/stats' + q, { waitUntil: 'load' });
        await p.evaluate(() => document.fonts.ready.then(() => new Promise((ok) => requestAnimationFrame(() => requestAnimationFrame(ok)))));
        at.push(await p.evaluate(() => {
          const c = document.querySelector('.segctl[aria-label="Period"]');
          if (!c) return null;
          const r = c.getBoundingClientRect();
          const opt = document.querySelector('.optline'), chips = document.querySelector('.chips');
          const below = !chips || !opt || chips.getBoundingClientRect().top >= opt.getBoundingClientRect().bottom - 0.5;
          return { q: location.search, x: Math.round(r.left), y: Math.round(r.top + scrollY), chip: !!document.querySelector('.chip'), below };
        }));
        await p.close();
      }
      const where = `period-steady ${name} ${width}`;
      const bad = [];
      if (at.some((a) => !a)) bad.push('no period control');
      else {
        for (const a of at.slice(1)) if (a.x !== at[0].x || a.y !== at[0].y) bad.push(`${a.q || 'none'} at ${a.x},${a.y}, not ${at[0].x},${at[0].y}`);
        if (!at[2].chip) bad.push('the path filter shows no chip');
        if (at.some((a) => !a.below)) bad.push('the chips row sits above the counting line');
      }
      bad.length ? r.fail(`${where}: ${bad.join(', ')}`) : r.ok(`${where}: the control stays at ${at[0].x},${at[0].y}`);
    }
    await b.close();
  }
}
// Sticky heads: on a commit with three 300-line files, scrolled halfway into the second, its
// header is pinned at the top and the first's is not; a #L120 link on a source page lands
// fully below the pinned path bar; at 375 px the site header never pins. The commit is made
// with a private index on a side ref, so HEAD and the work tree the other checks use stay put.
async function stickyHeads() {
  const { execFileSync } = await import('node:child_process');
  const env = { ...process.env, GIT_INDEX_FILE: join(cfg, 'sticky-index') };
  const git = (...a) => execFileSync('git', a, { cwd: dir, env, encoding: 'utf8' }).trim();
  git('read-tree', 'HEAD');
  for (const n of ['one', 'two', 'three']) {
    const body = Array.from({ length: 300 }, (_, i) => `${n} line ${i + 1}`).join('\n') + '\n';
    const blob = execFileSync('git', ['hash-object', '-w', '--stdin'], { cwd: dir, input: body, encoding: 'utf8' }).trim();
    git('update-index', '--add', '--cacheinfo', `100644,${blob},sticky/${n}.txt`);
  }
  const sha = git('commit-tree', git('write-tree'), '-p', 'HEAD', '-m', 'Three long files');
  git('update-ref', 'refs/heads/sticky-check', sha);
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const scheme of ['light', 'dark']) for (const width of [1440, 375]) {
      const p = await ctx.newPage();
      await p.emulateMedia({ colorScheme: scheme });
      await p.setViewportSize({ width, height: 700 });
      await p.goto(`${srv.base}/commit/${sha}`, { waitUntil: 'load' });
      const m = await p.evaluate(() => {
        const s = [...document.querySelectorAll('section.file')];
        if (s.length < 2) return null;
        const r = s[1].getBoundingClientRect();
        scrollTo(0, scrollY + r.top + r.height / 2);
        const top = (el) => el.getBoundingClientRect().top;
        const site = document.querySelector('header.top');
        return { second: top(s[1].querySelector('header')), first: top(s[0].querySelector('header')), site: site ? top(site) : 0 };
      });
      const where = `sticky-heads ${name} ${scheme} ${width}`;
      const bad = [];
      if (!m) bad.push('fewer than two files on the commit page');
      else {
        if (Math.abs(m.second) > 1) bad.push(`second file header at ${m.second.toFixed(1)} px, not pinned`);
        if (Math.abs(m.first) <= 1) bad.push('first file header still pinned');
        if (width <= 600 && Math.abs(m.site) <= 1) bad.push('site header pinned at 375');
      }
      await p.goto(`${srv.base}/blob/${sha}/sticky/two.txt#L120`, { waitUntil: 'load' });
      await p.waitForTimeout(100);
      const a = await p.evaluate(() => {
        const l = document.getElementById('L120');
        if (!l) return null;
        const t = l.getBoundingClientRect();
        let cover = 0;
        for (const el of document.querySelectorAll('body *')) {
          const st = getComputedStyle(el);
          if (st.position !== 'sticky' && st.position !== 'fixed') continue;
          const r = el.getBoundingClientRect();
          if (r.height && r.top <= 1 && r.left < t.right && r.right > t.left) cover = Math.max(cover, r.bottom);
        }
        return { top: t.top, bottom: t.bottom, cover, h: innerHeight };
      });
      if (!a) bad.push('no #L120 on the source page');
      else if (a.top < a.cover - 0.5 || a.bottom > a.h) bad.push(`#L120 at ${a.top.toFixed(1)} px, under a pinned bar ending at ${a.cover.toFixed(1)} px`);
      if (width > 600 && a && a.cover < 1) bad.push('source path bar not pinned');
      // The pinned page head holds the path and the file's tabs, so the reader keeps both.
      if (width > 600) {
        const held = await p.evaluate(() => {
          const h = document.querySelector('main>.ph.pin');
          const path = h && h.querySelector('h1.crumbs a:last-child');
          return !!(path && h.querySelector('.tabs a[aria-current="page"]'));
        });
        if (!held) bad.push('the pinned head lacks the path or the File tab');
      }
      const held = 'file header pinned and handed over, #L120 clear of the head, path and tabs held';
      bad.length ? r.fail(`${where}: ${bad.join(', ')}`) : r.ok(`${where}: ${held}`);
      if (arg('--shots')) { mkdirSync(arg('--shots'), { recursive: true }); await p.screenshot({ path: join(arg('--shots'), `sticky-blob-${name}-${scheme}-${width}.png`) }); await p.goto(`${srv.base}/commit/${sha}`); await p.evaluate(() => { const s = document.querySelectorAll('section.file')[1].getBoundingClientRect(); scrollTo(0, scrollY + s.top + s.height / 2); }); await p.screenshot({ path: join(arg('--shots'), `sticky-commit-${name}-${scheme}-${width}.png`) }); }
      await p.close();
    }
    await b.close();
  }
}
// ---- settings --------------------------------------------------------------------------------
async function settings() {
  for (const name of browsers) {
    // Each browser starts from the same file, never the one the last browser saved.
    rmSync(join(cfg, 'config.toml'), { force: true });
    const { b, ctx } = await context(name);
    const p = await ctx.newPage();
    try {
      await p.goto(srv.base + '/', { waitUntil: 'load' });
      const gear = await p.$('#settingsbtn');
      await p.click('.actbtn.top');
      await p.waitForSelector('.actmenu li >> text=Settings…');
      await p.click('.actmenu li >> text=Settings…');
      await p.waitForSelector('#setform');
      if (gear) r.fail(`settings ${name}: a header gear duplicates the menu entry`);
      await p.fill('#f-watch\\.rescan_secs', '75');
      await p.click('.segs [data-show="plain"]');
      await p.waitForFunction(() => /rescan_secs = 75/.test(/** @type {HTMLTextAreaElement} */ (document.getElementById('settext')).value), null, { timeout: 3000 });
      await p.fill('#settext', '[watch]\nrescan_secs = 3\n');
      await p.waitForSelector('#setgut .bad', { timeout: 3000 });
      const off = await p.$eval('#setsave', (x) => /** @type {HTMLButtonElement} */ (x).disabled);
      await p.fill('#settext', '[watch]\nrescan_secs = 80\n');
      await p.waitForFunction(() => !/** @type {HTMLButtonElement} */ (document.getElementById('setsave')).disabled, null, { timeout: 3000 });
      await Promise.all([p.waitForNavigation(), p.click('#setsave')]);
      const saved = readFileSync(join(cfg, 'config.toml'), 'utf8');
      const shown = await p.$eval('#f-watch\\.rescan_secs', (x) => /** @type {HTMLInputElement} */ (x).value);
      if (off && saved === '[watch]\nrescan_secs = 80\n' && shown === '80') r.ok(`settings ${name}: form, plain, error line and save`);
      else r.fail(`settings ${name}: save off on error ${off}, file ${JSON.stringify(saved)}, form shows ${shown}`);
      // actions.toml: rename the "Mark" action in its card; the comment-free rest stays.
      const before = readFileSync(join(cfg, 'actions.toml'), 'utf8');
      await p.goto(srv.origin + '/_settings?f=actions', { waitUntil: 'load' });
      await p.fill('#f-action\\.0\\.label', 'Mark it');
      await p.waitForFunction(() => !/** @type {HTMLButtonElement} */ (document.getElementById('setsave')).disabled, null, { timeout: 3000 });
      await Promise.all([p.waitForNavigation(), p.click('#setsave')]);
      const after = readFileSync(join(cfg, 'actions.toml'), 'utf8');
      if (after === before.replace('label = "Mark"', 'label = "Mark it"')) r.ok(`settings ${name}: an action card edit saves in place`);
      else r.fail(`settings ${name}: actions.toml became ${JSON.stringify(after)}`);
      writeFileSync(join(cfg, 'actions.toml'), before, { mode: 0o600 });
    } catch (e) { r.fail(`settings ${name}: ${e.message.split('\n')[0]}`); }
    await b.close();
  }
}

const checks = {'sticky-heads': stickyHeads, 'tabs-one-line': tabsOneLine, 'tab-levels': tabLevels, 'calm-header': calmHeader, 'side-width': sideWidth, truncation, 'live-tabs': liveTabs, mermaid, chip, doctabs, chords, header, marked, 'menu-same': menuSame, 'menu-keys': menuKeys, gutter, overlays, tips, 'date-tips': dateTips, 'repo-switch': repoSwitch, outline, 'mark-seam': markSeam, 'about-place': aboutPlace, 'site-theme': siteTheme, 'install-tabs': installTabs, 'site-bar': siteBar, 'site-rail': siteRail, 'site-tables': siteTables, 'huge-diff': hugeDiff, 'md-anchor': mdAnchor, 'live-scroll': liveScroll, 'live-read': liveRead, 'sticky-line': stickyLine, 'seg-one-row': segOneRow, 'period-steady': periodSteady, 'export-from-disk': exportFromDisk, 'inline-controls': inlineControls, 'section-gaps': sectionGaps, 'path-cells': pathCells, 'mark-flat': markFlat, 'tab-stops': tabStops, 'help-keys': helpKeys, 'side-order': sideOrder, settings, 'lone-rows': loneRows, 'page-heads': pageHeads, 'crumb-flow': crumbHeads, 'compare-head': compareHeads, 'commit-head': commitHeads, 'search-head': searchHeads, 'small-layout': smallLayout, 'send-dialog': sendDialog, 'keyboard-walk': keyboardWalk, reflow, 'text-spacing': textSpacing, targets, 'keys-off': keysOff, 'axe-all': axeAll, 'forced-colors': forcedColors, 'reduced-motion': reducedMotion, 'outline-filter': outlineFilter, 'graph-view': graphView, 'avatar-fit': avatarFit, 'layout-shift': layoutShift, 'head-align': headAlign, symbols, 'no-sideways-scroll': noSidewaysScroll, 'head-fits': headFits, 'repos-filter': reposFilter, 'home-live': homeLive, 'row-overlap': rowOverlap, anatomy};
try {
  for (const c of wanted) { console.log(`-- ${c}`); await checks[c](); }
} finally {
  srv.stop();
  rmSync(dir, { recursive: true, force: true });
  rmSync(cfg, { recursive: true, force: true });
}
process.exit(r.result());
