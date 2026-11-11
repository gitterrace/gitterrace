#!/usr/bin/env node
// Browser checks for the served pages, one harness. Serves a temp repository with a given binary
// on a free port and runs the checks below in real browsers. Not in `make check`: it needs
// browsers. Run it with `make browser-check BIN=target/debug/git-terrace`.
//
//   node scripts/verify/browser/browser-check.mjs [--bin PATH] [--checks a,b]
//        [--browsers webkit,chromium] [--tabs 8,12] [--from-disk]
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
//                saves actions.toml in place, and a variable added as a name and value row
//                saves, in each browser
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
//   site-theme   the built site, target/site (served with its _headers CSP): first load follows the
//                system scheme with the theme already set in the head, the button cycles system,
//                light, dark, the choice survives a reload, the shown logo and screenshots are
//                the theme's and the other scheme's are never fetched, no CSP violation, and each
//                theme icon is the source-link mark's size
//   site-bar     every page of the built site, 404 included, at 390 and 1440 px in light
//                and dark: one bar right after the skip link, dark with light text in both
//                schemes, Docs, the source link and the theme button in view, no sideways
//                scroll
//   site-rail    the docs pages: at 1440 px a page with three or more h2 shows the "On this
//                page" rail beside the text (11 px mono label with a hairline, 2 px rail) and
//                scrolling to a heading marks its item alone in the accent; at 390 px and on
//                shorter pages no rail
//   doc-menus    docs pages at 390 px: one Docs control, menus clear of the h1, keyboard
//   bar-steady   top bar items keep their boxes across pages, widths, menu, hover, focus, theme
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
//   period-click clicking Week on each insights page shows Week as the current period
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
//   outline-menu  the "On this page" menu opens with its list and Escape returns focus to
//                its button
//   repos-filter the Repositories home filter: typing narrows rows and count, Enter opens the
//                first match, Escape clears, nothing shifts, at 390 and 1440 px
//   log-filter   the log's one field, with and without script, at 390 and 1440 px: Enter sends
//                ?q= with a chip per term, a chip removes its term alone, a refused filter
//                keeps its text with the reason tied to the field, no Compare in the head
//   log-filter-help  the ? at the field's end opens a help listing every keyword, with and
//                without script; Escape closes it and gives focus back
//   symbols      a source file's symbol outline: at 1440 px a rail beside the code below the
//                pinned path bar, at 390 px a menu in the path bar; a link lands its line
//   outline-current  a long source file opened at #L<n> lights the one symbol that holds n in the
//                outline rail, and the one at the reading line after scrolling up and down
//   buttons      every button on six pages and in Send to has its variant's look; each variant
//                changes on hover, press and focus with text at 4.5:1, light and dark
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
//   line-hover   a hovered line paints whole in file view and blame, not with has_line_hover off
//   popup-place  scrolled to the bottom of a long page, b and . open popups fully inside the
//                viewport, focus inside, typing reaching the filter
//   blame-fit    every blame gutter cell (sha, author, age) ends inside its right hairline, with a
//                long author and a minutes-old age, at 1400 and 900 px, light and dark
//   huge-diff    a commit of 2,000 changed files (mostly pure renames) loads under a DOM bound,
//                shows a rename as one line, loads the last files on scroll, opens #fN, opens a
//                folded file with Enter, and prints scroll frame times and long tasks
//
// --from-disk serves crates/gitterrace-web/static/*.js from the working tree over the (possibly
// older) binary's copies, to test edits before a rebuild. Playwright comes from $PLAYWRIGHT_CORE
// or the repo's node_modules, installed by `make browser-deps` (see lib/browser-lib.mjs).
import { readFileSync, readdirSync, writeFileSync, rmSync, mkdirSync, mkdtempSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { checks as pageChecks } from './pages-lib.mjs';
import { checks as uiChecks } from './ui-lib.mjs';
import { checks as layoutChecks } from './layout-lib.mjs';
import { checks as siteChecks } from './site-lib.mjs';
import { checks as siteBarChecks } from './sitebar-lib.mjs';
import { checks as diffChecks } from './diff-lib.mjs';
import { root, RESCAN_SECS, loadPlaywright, launch, makeRepo, serve, openPage, reporter, sh } from '../../lib/browser-lib.mjs';

const arg = (k, d) => { const i = process.argv.indexOf(k); return i < 0 ? d : process.argv[i + 1]; };
const bin = resolve(arg('--bin', join(root, 'target/debug/git-terrace')));
const fromDisk = process.argv.includes('--from-disk');
const tabCounts = arg('--tabs', '8,12').split(',').map(Number);
const browsers = arg('--browsers', 'webkit,chromium').split(',');
const ALL = ['truncation', 'live-tabs', 'mermaid', 'chip', 'doctabs', 'chords', 'header', 'marked', 'menu-same', 'menu-keys', 'gutter', 'overlays', 'tips', 'date-tips', 'repo-switch', 'outline', 'mark-seam', 'about-place', 'site-theme', 'install-tabs', 'site-bar', 'site-rail', 'site-tables', 'doc-menus', 'bar-steady', 'side-width', 'calm-header', 'header-theme', 'tabs-one-line', 'tab-levels', 'huge-diff', 'sticky-heads', 'md-anchor', 'live-scroll', 'live-read', 'sticky-line', 'seg-one-row', 'period-steady', 'period-click', 'export-from-disk', 'inline-controls', 'section-gaps', 'path-cells', 'mark-flat', 'tab-stops', 'help-keys', 'side-order', 'settings', 'lone-rows', 'page-heads', 'crumb-flow', 'compare-head', 'commit-head', 'search-head', 'small-layout', 'send-dialog', 'keyboard-walk', 'reflow', 'text-spacing', 'targets', 'keys-off', 'axe-all', 'forced-colors', 'reduced-motion', 'outline-menu', 'avatar-fit', 'graph-view', 'layout-shift', 'head-align', 'symbols', 'outline-current', 'no-sideways-scroll', 'head-fits', 'repos-filter', 'log-filter', 'log-filter-help', 'home-live', 'row-overlap', 'blame-fit', 'anatomy', 'popup-place', 'buttons', 'line-hover'];
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
const { checks: anchorChecks } = await import('./anchor-lib.mjs');
const { mdAnchor } = anchorChecks(env);
const { checks: scrollChecks } = await import('./scroll-lib.mjs');
const { liveRead } = scrollChecks(env);
const { truncation, liveTabs, mermaid, chip, doctabs, chords, header } = pageChecks(env);
const { marked, menuKeys, menuSame, gutter, overlays, tips, dateTips, repoSwitch, liveScroll, outline } = uiChecks(env);
const { inlineControls, sectionGaps, pathCells } = layoutChecks(env);
const { markSeam, aboutPlace, siteTheme, exportFromDisk, installTabs } = siteChecks(env);
const { siteBar, siteRail, siteTables, headerTheme } = siteBarChecks(env);
const { docMenus, barSteady } = (await import('./docmenus-lib.mjs')).checks(env);
const { hugeDiff } = diffChecks(env);
const core = (await import('./core-lib.mjs')).checks(env);
const { periodClick } = (await import('./period-lib.mjs')).checks(env);
const { sideWidth, calmHeader, tabsOneLine, segOneRow, tabLevels, periodSteady } = core;
const { stickyHeads, settings } = core;
const { checks: stickyChecks } = await import('./sticky-lib.mjs');
const { stickyLine } = stickyChecks(env);
const { checks: headsChecks } = await import('./heads-lib.mjs');
const { pageHeads, crumbHeads, compareHeads, commitHeads, searchHeads } = headsChecks(env);
const { checks: a11yChecks } = await import('./access-lib.mjs');
const {
  markFlat, tabStops, helpKeys, sideOrder, axeAll, forcedColors, reducedMotion,
} = a11yChecks(env);
const { checks: rowChecks } = await import('./rows-lib.mjs');
const { loneRows, smallLayout, avatarFit } = rowChecks(env);
const { checks: sendChecks } = await import('./send-lib.mjs');
const { sendDialog } = sendChecks(env);
const { checks: outlineChecks } = await import('./outline-lib.mjs');
const { outlineMenu, symbols, outlineCurrent } = outlineChecks(env);
const { checks: graphChecks } = await import('./graph-lib.mjs');
const { graphView } = graphChecks(env);
const { checks: shiftChecks } = await import('./shift-lib.mjs');
const { layoutShift, headAlign } = shiftChecks(env);
const { checks: homeLiveChecks } = await import('./homelive-lib.mjs');
const { homeLive } = homeLiveChecks(env);
const { checks: reposChecks } = await import('./repos-lib.mjs');
const { reposFilter } = reposChecks(env);
const { checks: logFilterChecks } = await import('./logfilter-lib.mjs');
const { logFilter, logFilterHelp } = logFilterChecks(env);
const { checks: anatomyChecks } = await import('./anatomy-lib.mjs');
const { anatomy } = anatomyChecks(env);
const { checks: kbChecks } = await import('./keyboard-lib.mjs');
const { keyboardWalk, reflow, textSpacing, targets, keysOff } = kbChecks(env);
const { checks: fitChecks } = await import('./fit-lib.mjs');
const { noSidewaysScroll, headFits } = fitChecks(env);
const { checks: overlapChecks } = await import('./overlap-lib.mjs');
const { rowOverlap } = overlapChecks(env);
const { checks: popupChecks } = await import('./popup-lib.mjs');
const { popupPlace } = popupChecks(env);
const { checks: blameChecks } = await import('./blame-lib.mjs');
const { blameFit } = blameChecks(env);
const { buttons } = (await import('./buttons-lib.mjs')).checks(env);
const { lineHover } = (await import('./linehover-lib.mjs')).checks(env);

const checks = {'sticky-heads': stickyHeads, 'tabs-one-line': tabsOneLine, 'tab-levels': tabLevels, 'calm-header': calmHeader, 'side-width': sideWidth, truncation, 'live-tabs': liveTabs, mermaid, chip, doctabs, chords, header, marked, 'menu-same': menuSame, 'menu-keys': menuKeys, gutter, overlays, tips, 'date-tips': dateTips, 'repo-switch': repoSwitch, outline, 'mark-seam': markSeam, 'about-place': aboutPlace, 'site-theme': siteTheme, 'install-tabs': installTabs, 'site-bar': siteBar, 'site-rail': siteRail, 'site-tables': siteTables, 'doc-menus': docMenus, 'bar-steady': barSteady, 'header-theme': headerTheme, 'huge-diff': hugeDiff, 'md-anchor': mdAnchor, 'live-scroll': liveScroll, 'live-read': liveRead, 'sticky-line': stickyLine, 'seg-one-row': segOneRow, 'period-steady': periodSteady, 'period-click': periodClick, 'export-from-disk': exportFromDisk, 'inline-controls': inlineControls, 'section-gaps': sectionGaps, 'path-cells': pathCells, 'mark-flat': markFlat, 'tab-stops': tabStops, 'help-keys': helpKeys, 'side-order': sideOrder, settings, 'lone-rows': loneRows, 'page-heads': pageHeads, 'crumb-flow': crumbHeads, 'compare-head': compareHeads, 'commit-head': commitHeads, 'search-head': searchHeads, 'small-layout': smallLayout, 'send-dialog': sendDialog, 'keyboard-walk': keyboardWalk, reflow, 'text-spacing': textSpacing, targets, 'keys-off': keysOff, 'axe-all': axeAll, 'forced-colors': forcedColors, 'reduced-motion': reducedMotion, 'outline-menu': outlineMenu, 'graph-view': graphView, 'avatar-fit': avatarFit, 'layout-shift': layoutShift, 'head-align': headAlign, symbols, 'outline-current': outlineCurrent, 'no-sideways-scroll': noSidewaysScroll, 'head-fits': headFits, 'repos-filter': reposFilter, 'log-filter': logFilter, 'log-filter-help': logFilterHelp, 'home-live': homeLive, 'row-overlap': rowOverlap, 'blame-fit': blameFit, 'popup-place': popupPlace, anatomy, buttons, 'line-hover': lineHover};
try {
  for (const c of wanted) { console.log(`-- ${c}`); await checks[c](); }
} finally {
  srv.stop();
  rmSync(dir, { recursive: true, force: true });
  rmSync(cfg, { recursive: true, force: true });
}
process.exit(r.result());
