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
//                active tab has aria-current; shots into --shots (default target/shots)/doctabs-*.png
//   header       from 1440 down to 320 px the top bar's tools (search, actions, theme) share one
//                row and the page never scrolls sideways; where search is folded, its button opens
//                the field with focus and Escape folds it again
//   marked       text marked with the mouse is what the menu acts on: marking lines 2-3 of a file
//                shows a trigger, and Send to's title and context carry L2-3 and their text;
//                marked README prose arrives as a > quote
//   overlays     t, ? and b each open a named dialog that takes focus, keeps it through 5 Tabs
//                (and Shift+Tab), closes on Escape and gives focus back to what had it
//   gutter       in a 250-line file (three chunks) every line's gutter number equals its line,
//                read from the CSS counter itself, with the selection trigger shown; and the
//                Send to window opens small and grows with its text
//   tips         closing ., .+s, t, ? or b with Escape leaves no tip on the opener they hand
//                focus back to, while Tab onto a tipped button still shows its tip
//   repo-switch  a server on one repository hides the switcher; when a second `git terrace`
//                joins, the open page shows it without a reload, it lists both, and picking
//                each lands on its /r/NAME/ page; `git terrace remove` hides it again and
//                the removed repository's page answers 404
//   about-place  on the overview, About sits beside the files when there are two columns and
//                above them when there is one, at every width from 1440 to 320 px
//   mark-seam    every two-colour mark (docs/brand/mark-*.svg, the in-app header logo) at 16 to
//                133 px, 1x and 2x, on light and dark: no pixel on the colour change is closer
//                to the background than both colours are, in every browser
//   site-theme   the generated site/ (served with its _headers CSP): first load follows the
//                system scheme with the theme already set in the head, the button cycles system,
//                light, dark, the choice survives a reload, the shown logo and screenshots are
//                the theme's and the other scheme's are never fetched, no CSP violation, and each
//                theme icon is the GitHub mark's size
//
// --from-disk serves crates/gitterrace-web/static/*.js from the working tree over the (possibly
// older) binary's copies, to test edits before a rebuild. Playwright comes from $PLAYWRIGHT_CORE,
// the repo's node_modules or the website checkout (see browser-lib.mjs).
import { readFileSync, readdirSync, writeFileSync, rmSync, mkdirSync, mkdtempSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { checks as pageChecks } from './browser-pages-lib.mjs';
import { checks as uiChecks } from './browser-ui-lib.mjs';
import { checks as siteChecks } from './browser-site-lib.mjs';
import { root, loadPlaywright, launch, makeRepo, serve, openPage, reporter, sh } from './browser-lib.mjs';

const arg = (k, d) => { const i = process.argv.indexOf(k); return i < 0 ? d : process.argv[i + 1]; };
const bin = resolve(arg('--bin', join(root, 'target/debug/git-terrace')));
const fromDisk = process.argv.includes('--from-disk');
const tabCounts = arg('--tabs', '8,12').split(',').map(Number);
const browsers = arg('--browsers', 'webkit,chromium').split(',');
const ALL = ['truncation', 'live-tabs', 'mermaid', 'chip', 'doctabs', 'chords', 'header', 'marked', 'gutter', 'overlays', 'tips', 'repo-switch', 'mark-seam', 'about-place', 'site-theme'];
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
  // Every script and stylesheet in the folder, read at run time, so a new or forgotten file is
  // never served stale from the binary while the check claims to test the working tree.
  if (fromDisk) for (const f of readdirSync(join(root, 'crates/gitterrace-web/static')).filter((n) => n.endsWith('.js') || n.endsWith('.css'))) {
    const type = f.endsWith('.css') ? 'text/css' : 'text/javascript';
    await ctx.route(`**/static/${f}`, (q) => q.fulfill({ contentType: type, body: readFileSync(join(root, 'crates/gitterrace-web/static', f)) }));
  }
  return { b, ctx };
}


const env = { pw, dir, srv, r, browsers, tabCounts, marker, context, arg, bin, cfg };
const { truncation, liveTabs, mermaid, chip, doctabs, chords, header } = pageChecks(env);
const { marked, gutter, overlays, tips, repoSwitch } = uiChecks(env);
const { markSeam, aboutPlace, siteTheme } = siteChecks(env);
const checks = { truncation, 'live-tabs': liveTabs, mermaid, chip, doctabs, chords, header, marked, gutter, overlays, tips, 'repo-switch': repoSwitch, 'mark-seam': markSeam, 'about-place': aboutPlace, 'site-theme': siteTheme };
try {
  for (const c of wanted) { console.log(`-- ${c}`); await checks[c](); }
} finally {
  srv.stop();
  rmSync(dir, { recursive: true, force: true });
  rmSync(cfg, { recursive: true, force: true });
}
process.exit(r.result());
