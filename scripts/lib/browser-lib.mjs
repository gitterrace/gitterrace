// Shared launcher for the browser checks (scripts/verify/browser/browser-check.mjs). Not run
// on its own.
// - finds playwright-core: $PLAYWRIGHT_CORE, else the repo's pinned node_modules (make
//   browser-deps); no network is used, and missing deps or browsers stop it early
// - builds a temp repository and serves it with a given binary on a free port
// - opens pages at given widths and colour schemes
import { spawn, execFileSync } from 'node:child_process';
import {
  mkdtempSync,
  writeFileSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  existsSync,
} from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir, homedir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import net from 'node:net';

/** The shortest watch.rescan_secs the config allows: the checks serve with it. */
export const RESCAN_SECS = 5;
/** How long a working tree change may take to show: the documented rescan bound plus a margin. */
export const WATCH_BOUND_MS = (RESCAN_SECS + 5) * 1000;

/** The insights pages, one per tab of the /stats row. */
export const STATS = ['/stats', '/stats/hotspots', '/stats/contributors', '/stats/ownership',
  '/stats/languages'];

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

/** How to get the browser check dependencies, named in every early failure. */
const INSTALL = 'run `make browser-deps` (npm ci, then the Playwright browsers)';

function die(msg) {
  console.error(`browser-check: ${msg}; ${INSTALL}`);
  process.exit(2);
}

/**
 * playwright-core from $PLAYWRIGHT_CORE, else the repo's pinned node_modules. Its ESM entry is
 * index.mjs: importing the CommonJS index.js leaves the browser types undefined. Exits early,
 * naming the install step, when it or axe-core or one of the `need` browsers is missing.
 */
export async function loadPlaywright(need = ['chromium']) {
  const p = process.env.PLAYWRIGHT_CORE || join(root, 'node_modules/playwright-core/index.mjs');
  if (!existsSync(p)) die(`playwright-core not found at ${p}`);
  const pw = await import(pathToFileURL(p).href);
  for (const n of LAUNCHERS) if (!pw[n]?.launch) die(`${p} has no ${n} launcher`);
  try {
    createRequire(join(root, 'package.json')).resolve('axe-core');
  } catch {
    die('axe-core not found');
  }
  for (const name of need) {
    const exe = name === 'chromium'
      ? chromiumPath() ?? pw.chromium.executablePath()
      : pw[name]?.executablePath();
    if (!exe || !existsSync(exe)) die(`${name} is not installed (${exe})`);
  }
  return pw;
}

/** A chromium Playwright can start: $PLAYWRIGHT_CHROMIUM, else the newest cached, else default. */
function chromiumPath() {
  if (process.env.PLAYWRIGHT_CHROMIUM) return process.env.PLAYWRIGHT_CHROMIUM;
  const cache = join(homedir(), 'Library/Caches/ms-playwright');
  try {
    const dirs = readdirSync(cache).filter((d) => /^chromium-\d+$/.test(d)).sort().reverse();
    for (const d of dirs) {
      const p = join(cache, d, 'chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing');
      if (existsSync(p)) return p;
    }
  } catch {}
  return undefined;
}

/** Every browser `launch` can start, and so every one CI installs (browsers-check.mjs). */
export const LAUNCHERS = ['chromium', 'webkit', 'firefox'];

export async function launch(pw, name) {
  if (name === 'chromium') return pw.chromium.launch({ executablePath: chromiumPath() });
  if (!LAUNCHERS.includes(name)) die(`unknown browser ${name}; known: ${LAUNCHERS.join(', ')}`);
  return pw[name].launch();
}

/**
 * The stylesheet bundles the pages load (`base.css`, `late.css`, ...) as name to part list,
 * read from BUNDLES in routes/assets.rs so the list lives in one place.
 */
function bundles() {
  const src = readFileSync(join(root, 'crates/gitterrace-web/src/routes/assets.rs'), 'utf8');
  const block = /const BUNDLES[^=]*=\s*\[([\s\S]*?)\n\];/.exec(src)?.[1] ?? '';
  const out = new Map();
  for (const m of block.matchAll(/"([\w-]+\.css)",\s*&\[([^\]]*)\]/g)) {
    out.set(m[1], [...m[2].matchAll(/"([\w-]+\.css)"/g)].map((x) => x[1]));
  }
  return out;
}

/**
 * Serves every script and stylesheet of `staticDir` from disk in `ctx`, over the binary's copies.
 * A page asks for them as `/static/app.js?v=<build>`, so the match is on the path and takes any
 * query: a glob on the bare name never matches and the binary's copy is served instead. The
 * stylesheets a page loads are the bundles, so each is its parts read back to back.
 */
export async function routeFromDisk(ctx, staticDir = join(root, 'crates/gitterrace-web/static')) {
  const parts = bundles();
  const assets = readdirSync(staticDir).filter((n) => /\.(js|css)$/.test(n));
  const files = new Map(assets
    .map((n) => [n, n.endsWith('.css') ? 'text/css' : 'text/javascript']));
  for (const name of parts.keys()) files.set(name, 'text/css');
  const asset = (u) => /\/static\/([^/?#]+)$/.exec(new URL(u).pathname)?.[1];
  const body = (f) => parts.has(f)
    ? parts.get(f).map((p) => readFileSync(join(staticDir, p), 'utf8')).join('\n')
    : readFileSync(join(staticDir, f));
  await ctx.route((u) => files.has(asset(u.href)), (q) => {
    const f = asset(q.request().url());
    return q.fulfill({ contentType: files.get(f), body: body(f) });
  });
}

/** Polls `fn` until it returns something truthy, or throws after `ms`. Returns that value. */
export async function poll(fn, what = 'condition', ms = WATCH_BOUND_MS) {
  const end = Date.now() + ms;
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out after ${ms} ms waiting for ${what}`);
    await new Promise((ok) => setTimeout(ok, 50));
  }
}

/**
 * Loads `url` (or reloads the page when none) until `ready(page)` holds: parts of a page appear
 * once the server has built its index or seen a working tree edit. Throws after `ms`.
 */
export async function reloadUntil(p, ready, url = null, what = 'the page to be ready',
  ms = WATCH_BOUND_MS) {
  let first = true;
  await poll(async () => {
    if (url) await p.goto(url, { waitUntil: 'load' });
    else if (!first) await p.reload({ waitUntil: 'load' });
    first = false;
    return ready(p);
  }, what, ms);
}

/**
 * Waits until the server shows what git now holds in `dir`: the head commit, and as many changed
 * files in the working tree as `git status` lists. A check that commits or edits a file then
 * reads the served page calls this instead of sleeping for the rescan.
 */
export async function synced(srv, dir) {
  const head = sh(dir, 'git', 'rev-parse', 'HEAD');
  const status = sh(dir, 'git', 'status', '--porcelain', '-uall');
  const changed = status.split('\n').filter(Boolean).length;
  await poll(async () => {
    const repo = await (await fetch(`${srv.base}/api/v1/repo`)).json();
    if (repo.head !== head) return false;
    const wt = await (await fetch(`${srv.base}/api/v1/worktree`)).json();
    return wt.files.length === changed;
  }, `the server to show ${head.slice(0, 8)} and ${changed} changed file(s)`);
}

/**
 * Runs `fn` in the page once it is the front tab: a background tab runs no animation frames, so
 * a frame wait on it never ends. Bounded, so a page that never paints fails instead of hanging.
 */
async function inFront(p, fn, arg) {
  await p.bringToFront();
  return Promise.race([
    p.evaluate(fn, arg),
    new Promise((_, no) => setTimeout(
      () => no(new Error('no animation frames within the bound')), WATCH_BOUND_MS)),
  ]);
}

/** Waits `n` animation frames in the page: layout and the scroll handlers have run. */
export const frames = (p, n = 2) => inFront(p, (k) => new Promise((ok) => {
  const tick = (i) => (i ? requestAnimationFrame(() => tick(i - 1)) : ok());
  tick(k);
}), n);

/** Waits until the DOM has not changed for `n` frames: a swap or a lazy load is done. */
export const domQuiet = (p, n = 30) => inFront(p, (k) => new Promise((ok) => {
  let still = 0;
  const seen = new MutationObserver(() => { still = 0; });
  seen.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  const tick = () => {
    if (++still >= k) { seen.disconnect(); ok(); } else requestAnimationFrame(tick);
  };
  tick();
}), n);

/**
 * Waits until the page's scroll position is the same on `n` reads in a row. Polls from here, so
 * it also works on a page with scripts off, where no animation frame callback runs.
 */
export async function scrollSettled(p, n = 5) {
  let last = NaN, same = 0;
  await poll(async () => {
    const y = await p.evaluate(() => scrollY);
    same = y === last ? same + 1 : 0;
    last = y;
    return same >= n;
  }, 'the scroll position to settle');
}

export async function freePort() {
  return new Promise((ok) => {
    const s = net.createServer().listen(0, '127.0.0.1', () => { const p = /** @type {net.AddressInfo} */ (s.address()).port; s.close(() => ok(p)); });
  });
}

const ident = { GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t' };

/** Runs a command in `cwd`; returns trimmed stdout. */
export function sh(cwd, ...a) {
  return execFileSync(a[0], a.slice(1), { cwd, stdio: 'pipe', env: { ...process.env, ...ident } }).toString().trim();
}

/**
 * Commits `files` (path to string or Buffer) on top of `parent` onto the side ref `ref`, through
 * the private index file `indexFile`, so HEAD and the work tree stay put. Returns the commit.
 */
export function commitFiles(dir, indexFile, files, ref, subject, parent = 'HEAD') {
  const env = { ...process.env, ...ident, GIT_INDEX_FILE: indexFile };
  const git = (input, ...a) =>
    execFileSync('git', a, { cwd: dir, env, input, encoding: 'utf8' }).trim();
  git(undefined, 'read-tree', parent);
  for (const [path, body] of Object.entries(files)) {
    const blob = git(body, 'hash-object', '-w', '--stdin');
    git(undefined, 'update-index', '--add', '--cacheinfo', `100644,${blob},${path}`);
  }
  const tree = git(undefined, 'write-tree');
  const sha = git(undefined, 'commit-tree', tree, '-p', parent, '-m', subject);
  git(undefined, 'update-ref', ref, sha);
  return sha;
}

/** Stages everything in `dir` and commits it with `subject`. */
export function commitAll(dir, subject) {
  sh(dir, 'git', 'add', '-A');
  sh(dir, 'git', 'commit', '-q', '-m', subject);
}

const SUBJECTS = [
  'feat(core): a deliberately long commit subject that must stay on one line and carry a tip when the column cuts it short',
  'fix: short',
  'chore(deps): bump everything-at-once-with-a-very-long-hyphenated-name from 1.2.3 to 4.5.6',
];

/** A repository with a mermaid README, long names and subjects, and a tracked file to edit. */
export function makeRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-browser-'));
  sh(dir, 'git', 'init', '-q', '-b', 'main');
  mkdirSync(join(dir, 'src/a_rather_long_directory_name'), { recursive: true });
  writeFileSync(join(dir, 'README.md'), '# Fixture\n\n```mermaid\ngraph TD; A[Start]-->B[Build]; B-->C[Ship]\n```\n');
  writeFileSync(join(dir, 'a.txt'), 'one\n');
  writeFileSync(join(dir, 'src/a_rather_long_directory_name/a_file_with_a_really_long_name_for_wrapping.rs'), 'fn main() {}\n');
  SUBJECTS.forEach((s, i) => {
    writeFileSync(join(dir, `f${i}.txt`), `${i}\n`);
    sh(dir, 'git', 'add', '-A');
    sh(dir, 'git', 'commit', '-q', '-m', s);
  });
  return dir;
}

/** Every server still running. A `finally` never runs on a signal, an exit or a crash. */
const servers = new Set();

/** Stops every server this process started; the exit hook below and its signals call it. */
export function stopAll() {
  for (const c of servers) c.kill();
  servers.clear();
}

process.on('exit', stopAll);
const SIGNALS = { SIGINT: 130, SIGTERM: 143, SIGHUP: 129 };
for (const [sig, code] of Object.entries(SIGNALS)) {
  process.on(sig, () => { stopAll(); process.exit(code); });
}
process.on('uncaughtException', (e) => { stopAll(); console.error(e); process.exit(1); });
process.on('unhandledRejection', (e) => { stopAll(); console.error(e); process.exit(1); });

/**
 * Serves `dir` (one folder, or a list of them) with `bin` on a free port. Returns
 * { origin, base, stop }, where `base` is the
 * repository's own root, wherever it is mounted. `cfg` is the config folder; it must sit outside
 * `dir`, since actions.toml inside a served repository is refused. The server is stopped when
 * this process ends, however it ends.
 */
export async function serve(bin, dir, cfg = mkdtempSync(join(tmpdir(), 'gt-cfg-'))) {
  const dirs = [].concat(dir);
  const port = await freePort();
  // Its own state folder: the owner secret the launch link needs is written there.
  const state = mkdtempSync(join(tmpdir(), 'gt-state-'));
  const env = { ...process.env, XDG_STATE_HOME: state };
  const args = ['serve', '--addr', `127.0.0.1:${port}`, '--config', cfg, ...dirs];
  const child = spawn(bin, args, { stdio: 'ignore', env });
  servers.add(child);
  child.on('exit', () => servers.delete(child));
  const origin = `http://127.0.0.1:${port}`;
  let up = false;
  for (let i = 0; i < 100 && !up; i++) {
    try { up = (await fetch(origin + '/')).ok; } catch {}
    if (!up) await new Promise((r) => setTimeout(r, 100));
  }
  if (!up) { child.kill(); throw new Error(`${bin} did not answer on ${origin}`); }
  const m = /href="(\/r\/[^"\/]+)\/?"/.exec(await (await fetch(origin + '/')).text());
  const secret = join(state, 'gitterrace', `add-token-${port}`);
  const owner = () => ownerCookie(origin, secret);
  // The one way a case runs the binary against this server. The join and remove commands read
  // the add secret from this server's own state folder (a caller's XDG_STATE_HOME is not it: with
  // the wrong one the command finds no secret and tries to bind the port itself, EADDRINUSE).
  // The address is the real port the server bound, so no case names a port.
  const cli = (cwd, ...rest) => execFileSync(
    bin, ['--addr', `127.0.0.1:${port}`, '--config', cfg, ...rest],
    { cwd, stdio: 'pipe', env: { ...process.env, ...ident, XDG_STATE_HOME: state } },
  ).toString().trim();
  return { origin, base: m ? origin + m[1] : origin, stop: () => child.kill(), owner, cli, state };
}

/**
 * The owner cookie a browser gets from the launch link, as `git terrace open` hands it: the
 * secret from the 0600 file buys a one-time ticket, the ticket sets the HttpOnly cookie. Pass it
 * to `ctx.addCookies` so a page can run an action or save a setting.
 */
export async function ownerCookie(origin, secretFile) {
  const secret = readFileSync(secretFile, 'utf8').trim();
  const headers = { 'x-gitterrace-add-token': secret };
  const t = await (await fetch(`${origin}/_launch`, { method: 'POST', headers })).json();
  const res = await fetch(`${origin}/_launch?t=${t.ticket}&to=/`, { redirect: 'manual' });
  const [pair] = res.headers.get('set-cookie').split(';');
  const i = pair.indexOf('=');
  const [name, value] = [pair.slice(0, i), pair.slice(i + 1)];
  return { name, value, url: origin, httpOnly: true, sameSite: 'Strict' };
}

/** Opens `url` at one width and colour scheme. */
export async function openPage(browser, url, { width = 900, height = 800, scheme = 'light', wait = 'load' } = {}) {
  const p = await browser.newPage({ colorScheme: scheme, viewport: { width, height } });
  await p.goto(url, { waitUntil: wait, timeout: 15000 });
  return p;
}

/**
 * Runs `fn(page, { name, scheme, width, item })` on a fresh page at every browser, colour scheme,
 * width and `items` entry, closing each page and browser after it.
 */
export async function sweep(opts, fn) {
  const { browsers, context, widths, height = 900, items = [null] } = opts;
  const schemes = opts.schemes ?? ['light', 'dark'];
  for (const name of browsers) {
    const { b, ctx } = await context(name);
    for (const scheme of schemes) {
      for (const width of widths) {
        for (const item of items) {
          const p = await ctx.newPage();
          await p.emulateMedia({ colorScheme: scheme });
          await p.setViewportSize({ width, height });
          await fn(p, { name, scheme, width, item });
          await p.close();
        }
      }
    }
    await b.close();
  }
}

/**
 * Relative luminance of a CSS color: an [r, g, b] array in 0-255, or an `rgb()` or
 * `color(srgb ...)` string as getComputedStyle prints it.
 */
export function lum(c) {
  const k = typeof c === 'string' && c.startsWith('color(') ? 255 : 1;
  const v = typeof c === 'string' ? (c.match(/[\d.]+/g) || []).map((x) => Number(x) * k) : c;
  const f = (x) => ((x /= 255) <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4);
  return 0.2126 * f(v[0]) + 0.7152 * f(v[1]) + 0.0722 * f(v[2]);
}

/** Runs `fn(ctx, name, opts)` in a fresh context of each browser, closing the browser after it. */
export async function eachBrowser(browsers, context, fn, opts) {
  for (const name of browsers) {
    const { b, ctx } = await context(name, opts);
    await fn(ctx, name);
    await b.close();
  }
}

/** Runs `fn(browser, name)` in a freshly launched browser of each name, closing it after. */
export async function eachLaunch(pw, browsers, fn) {
  for (const name of browsers) {
    const b = await launch(pw, name);
    await fn(b, name);
    await b.close();
  }
}

/** Collects pass/fail lines; `result()` prints the verdict and returns the exit code. */
export function reporter() {
  let failed = 0;
  return {
    ok: (m) => console.log('ok  ', m),
    fail: (m) => { failed++; console.log('FAIL', m); },
    // One verdict line: a fault list fails as `where: a; b`, an empty one passes as `where: good`.
    verdict(where, bad, good = '', sep = '; ') {
      if (bad.length) this.fail(`${where}: ${bad.join(sep)}`);
      else this.ok(good ? `${where}: ${good}` : where);
    },
    result: () => { console.log(failed ? `RESULT: FAIL (${failed})` : 'RESULT: ok'); return failed ? 1 : 0; },
  };
}
