// Shared launcher for the browser checks (scripts/browser-check.mjs). Not run on its own.
// - finds playwright-core: $PLAYWRIGHT_CORE, then node resolution from the repo, then the
//   website checkout; no network is used
// - builds a temp repository and serves it with a given binary on a free port
// - opens pages at given widths and colour schemes
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, mkdirSync, readdirSync, existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir, homedir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import net from 'node:net';

/** The shortest watch.rescan_secs the config allows: the checks serve with it. */
export const RESCAN_SECS = 5;
/** How long a working tree change may take to show: the documented rescan bound plus a margin. */
export const WATCH_BOUND_MS = (RESCAN_SECS + 5) * 1000;

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export async function loadPlaywright() {
  const tries = [
    process.env.PLAYWRIGHT_CORE,
    () => createRequire(join(root, 'package.json')).resolve('playwright-core'),
    join(homedir(), 'github/tendir/website/node_modules/playwright-core/index.mjs'),
  ];
  for (const t of tries) {
    try {
      const p = typeof t === 'function' ? t() : t;
      if (p && existsSync(p)) return await import(pathToFileURL(p).href);
    } catch {}
  }
  throw new Error('playwright-core not found: set PLAYWRIGHT_CORE to its index.mjs');
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

export async function launch(pw, name) {
  return name === 'webkit' ? pw.webkit.launch() : pw.chromium.launch({ executablePath: chromiumPath() });
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

/**
 * Serves `dir` with `bin` on a free port. Returns { origin, base, stop }, where `base` is the
 * repository's own root, wherever it is mounted.
 */
/**
 * Serves `dir` with `bin`. `cfg` is the config folder; it must sit outside `dir`, since
 * actions.toml inside a served repository is refused.
 */
export async function serve(bin, dir, cfg = mkdtempSync(join(tmpdir(), 'gt-cfg-'))) {
  const port = await freePort();
  const child = spawn(bin, ['serve', '--addr', `127.0.0.1:${port}`, '--config', cfg, dir], { stdio: 'ignore' });
  const origin = `http://127.0.0.1:${port}`;
  let up = false;
  for (let i = 0; i < 100 && !up; i++) {
    try { up = (await fetch(origin + '/')).ok; } catch {}
    if (!up) await new Promise((r) => setTimeout(r, 100));
  }
  if (!up) { child.kill(); throw new Error(`${bin} did not answer on ${origin}`); }
  const m = /href="(\/r\/[^"\/]+)\/?"/.exec(await (await fetch(origin + '/')).text());
  return { origin, base: m ? origin + m[1] : origin, stop: () => child.kill() };
}

/** Opens `url` at one width and colour scheme. */
export async function openPage(browser, url, { width = 900, height = 800, scheme = 'light', wait = 'load' } = {}) {
  const p = await browser.newPage({ colorScheme: scheme, viewport: { width, height } });
  await p.goto(url, { waitUntil: wait, timeout: 15000 });
  return p;
}

/** Collects pass/fail lines; `result()` prints the verdict and returns the exit code. */
export function reporter() {
  let failed = 0;
  return {
    ok: (m) => console.log('ok  ', m),
    fail: (m) => { failed++; console.log('FAIL', m); },
    result: () => { console.log(failed ? `RESULT: FAIL (${failed})` : 'RESULT: ok'); return failed ? 1 : 0; },
  };
}
