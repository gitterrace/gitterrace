#!/usr/bin/env node
// Takes the website and README screenshots of jq and ripgrep (public MIT projects, the same as
// the demo). Self-contained: clones both into ~/.cache/gitterrace-shots (reused when present),
// pins a `release` branch in each to the commit in PINS, and serves them with the release binary
// on a free port, with scripts/site-shots/config as the config folder (no tendir in Send to) and
// a throwaway state folder, so neither the user's config nor a running preview is touched. Light
// and dark images per shot, 1280x800 at 2x, into target/site-shots/ (ignored). Then copies the
// PNG files README.md links into docs/screenshots/ and deletes the rest there. `make site-shots`
// builds first and follows this with site-render (the WebPs) and `cargo xtask generate`. Not in
// check: it needs browsers and the network on the first run.
//
//   node scripts/site-shots-run.mjs [NAME...]   all shots by default
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync } from 'node:fs';
import { execFileSync, spawn } from 'node:child_process';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { root, loadPlaywright, launch, freePort } from './browser-lib.mjs';

// The demo's releases: jq 1.8.2 and ripgrep 15.2.0. The shot paths below fit these commits
// (ripgrep 068a63e is in 15.2.0, and the blame and send files below exist there).
const PINS = {
  jq: { url: 'https://github.com/jqlang/jq.git', commit: '34f7186b86743a083a589741b6cea95293524108' },
  ripgrep: { url: 'https://github.com/BurntSushi/ripgrep.git', commit: 'e89fff89ac9af12e8d4ce9d5fd07beb408ca730f' },
};
const corpus = join(homedir(), '.cache/gitterrace-shots');
const git = (cwd, ...a) => execFileSync('git', ['-C', cwd, ...a], { stdio: ['ignore', 'pipe', 'inherit'] }).toString().trim();
mkdirSync(corpus, { recursive: true });
for (const [name, { url, commit }] of Object.entries(PINS)) {
  const dir = join(corpus, name);
  if (!existsSync(join(dir, '.git'))) execFileSync('git', ['clone', '-q', url, dir], { stdio: 'inherit' });
  try { git(dir, 'cat-file', '-e', `${commit}^{commit}`); } catch { git(dir, 'fetch', '-q', 'origin', commit); }
  // HEAD on `release` at the pin, a clean worktree: every shot reads the same tree.
  git(dir, 'checkout', '-q', '-f', '-B', 'release', commit);
  git(dir, 'clean', '-qfdx', '-e', '.git');
}

const bin = join(root, 'target/release/git-terrace');
const port = await freePort();
const state = mkdtempSync(join(tmpdir(), 'gt-shots-state-'));
const server = spawn(bin, ['serve', '--addr', `127.0.0.1:${port}`, '--no-watch', '--config', join(root, 'scripts/site-shots/config'), ...Object.keys(PINS).map((n) => join(corpus, n))], {
  stdio: ['ignore', 'ignore', 'inherit'],
  env: { ...process.env, XDG_STATE_HOME: state },
});
const stop = () => { server.kill(); rmSync(state, { recursive: true, force: true }); };
process.on('exit', stop);
const base = `http://127.0.0.1:${port}`;
for (let i = 0, up = false; !up; i++) {
  try { up = (await fetch(base + '/r/jq/')).ok && (await fetch(base + '/r/ripgrep/')).ok; } catch {}
  if (!up && i > 600) throw new Error(`${bin} did not serve /r/jq and /r/ripgrep on ${base}`);
  if (!up) await new Promise((r) => setTimeout(r, 100));
}

const only = process.argv.slice(2);
const out = join(root, 'target/site-shots');
mkdirSync(out, { recursive: true });

// Stable frames: no animation, no caret, no live-update pulse.
const FREEZE = '*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}';

const SHOTS = [
  { name: 'overview', path: '/r/ripgrep/' },
  // Hotspots has the page header, tab bar, key numbers and the scatter above the fold.
  { name: 'insights', path: '/r/jq/stats/hotspots' },
  // A commit page: message, changed files and a highlighted diff that fits one screen.
  { name: 'commit', path: '/r/ripgrep/commit/068a63e' },
  // configure.ac from the top: 13 blocks and 8 authors in the first 28 lines (scanned over jq and
  // ripgrep).
  { name: 'blame', path: '/r/jq/blame/release/configure.ac' },
  // Bus factor, solo and stale lines, and the owners of each directory.
  { name: 'ownership', path: '/r/jq/stats/ownership' },
  {
    name: 'send',
    // L14-17 sit below the window at the top of the page, so the marked band shows behind it.
    path: '/r/ripgrep/blob/release/crates/core/main.rs#L14-L17',
    async prepare(p) {
      await p.waitForSelector('#actsel');
      await p.click('#actsel');
      await p.waitForSelector('.actmenu');
      await p.click('.actmenu li:has-text("Send to…") >> nth=0');
      await p.waitForSelector('.send, dialog, [role=dialog]', { timeout: 5000 });
      // The site shows kinds of targets, not one app. Serve with an actions.toml without tendir.
      const text = await p.evaluate(() => document.body.innerText);
      if (/tendir/i.test(text)) throw new Error('send shot shows tendir');
      // The fixture's three targets, so a config that did not load fails here, not on the site.
      // Targets arrive after the window opens.
      await p.waitForFunction(() => document.querySelectorAll('select option').length > 1, null, { timeout: 5000 }).catch(() => {});
      const opts = await p.$$eval('select option', (os) => os.map((o) => o.textContent.trim()));
      for (const want of ['Mail draft', 'New issue', 'New merge request']) {
        if (!opts.includes(want)) throw new Error(`send shot: no "${want}" target, got ${JSON.stringify(opts)}`);
      }
      // Same frame as the other shots: the app header at the top, the marked lines in view.
      await p.evaluate(() => window.scrollTo(0, 0));
      // No focus ring on the text field: the frame shows the window at rest.
      await p.evaluate(() => /** @type {HTMLElement} */ (document.activeElement)?.blur());
    },
  },
];

const todo = SHOTS.filter((s) => !only.length || only.includes(s.name));
const pw = await loadPlaywright();
const browser = await launch(pw, 'chromium');
for (const scheme of ['light', 'dark']) {
  const ctx = await browser.newContext({ colorScheme: scheme, viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2 });
  for (const s of todo) {
    const p = await ctx.newPage();
    await p.goto(base + s.path, { waitUntil: 'networkidle', timeout: 60000 });
    await p.addStyleTag({ content: FREEZE });
    await p.evaluate(() => document.fonts.ready);
    if (s.prepare) await s.prepare(p);
    await p.waitForTimeout(400);
    const file = join(out, `${s.name}-${scheme}.png`);
    await p.screenshot({ path: file });
    await p.close();
    console.log(`wrote target/site-shots/${s.name}-${scheme}.png`);
  }
  await ctx.close();
}
await browser.close();
stop();

// Smaller files when a PNG optimizer is installed; the shots are the same either way.
const files = todo.flatMap((s) => ['light', 'dark'].map((c) => join(out, `${s.name}-${c}.png`)));
/** @type {[string, string[]][]} */
const optimizers = [['oxipng', ['-q', '-o', '4', '--strip', 'safe']], ['pngquant', ['--force', '--skip-if-larger', '--ext', '.png', '--quality', '70-95']]];
for (const [tool, args] of optimizers) {
  try {
    execFileSync(tool, [...args, ...files], { stdio: 'ignore' });
    console.log(`optimized with ${tool}`);
    break;
  } catch {}
}
for (const f of files) console.log(`${f.slice(root.length + 1)} ${(statSync(f).size / 1024).toFixed(0)} KiB`);

// The README's PNG files, by the same rule as readme_shots in xtask/src/site.rs: every
// docs/screenshots/*.png path README.md names. Only those are tracked, beside the site's WebPs.
const docs = join(root, 'docs/screenshots');
const readme = new Set([...readFileSync(join(root, 'README.md'), 'utf8').matchAll(/docs\/screenshots\/([\w.-]+?\.png)/g)].map((m) => m[1]));
mkdirSync(docs, { recursive: true });
for (const f of readdirSync(docs)) if (f.endsWith('.png') && !readme.has(f)) rmSync(join(docs, f));
for (const f of readme) {
  try {
    copyFileSync(join(out, f), join(docs, f));
    console.log(`copied docs/screenshots/${f}`);
  } catch {
    console.error(`site-shots: README.md links docs/screenshots/${f}, but no shot made it`);
    process.exit(1);
  }
}
