// The font-sample check: a committed font shows its facts and, once the browser has loaded it
// from /raw, a sample set in that face; a file that is not a font shows neither. The commit
// goes on a side ref through a private index, so HEAD and the work tree stay put.
// Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { root, launch } from '../../lib/browser-lib.mjs';

const FONT = join(
  root, 'vendor/schwarzwald/kit/fonts/jetbrains-mono/jetbrains-mono-latin-wght-normal.woff2',
);

/** @param {any} env what browser-check.mjs set up (dir, srv, r, browsers, cfg, ...) */
export function checks(env) {
  const { dir, srv, r, browsers, cfg } = env;
  async function fontSample() {
    const gitEnv = { ...process.env, GIT_INDEX_FILE: join(cfg, 'font-index') };
    const git = (input, ...a) =>
      execFileSync('git', a, { cwd: dir, env: gitEnv, input, encoding: 'utf8' }).trim();
    git(undefined, 'read-tree', 'HEAD');
    const add = (body, path) => {
      const blob = git(body, 'hash-object', '-w', '--stdin');
      git(undefined, 'update-index', '--add', '--cacheinfo', `100644,${blob},${path}`);
    };
    add(readFileSync(FONT), 'fonts/mono.woff2');
    add('\0\x01\0\0not a font', 'fonts/bad.ttf');
    const tree = git(undefined, 'write-tree');
    const sha = git(undefined, 'commit-tree', tree, '-p', 'HEAD', '-m', 'Fonts');
    git(undefined, 'update-ref', 'refs/heads/font-check', sha);
    for (const name of browsers) {
      const b = await launch(env.pw, name);
      const p = await (await b.newContext()).newPage();
      const csp = [];
      p.on('console', (m) => /Content Security Policy/i.test(m.text()) && csp.push(m.text()));
      await p.goto(`${srv.base}/blob/${sha}/fonts/mono.woff2`, { waitUntil: 'load' });
      const facts = await p.locator('.font-facts').innerText();
      /JetBrains Mono/.test(facts) && /glyphs/.test(facts)
        ? r.ok(`${name}: font facts shown`)
        : r.fail(`${name}: facts: ${facts}`);
      try {
        await p.waitForSelector('.font-sample:not([hidden])', { timeout: 5000 });
        const face = await p.evaluate(() => [...document.fonts]
          .some((f) => f.family.startsWith('gt-sample') && f.status === 'loaded'));
        face && !csp.length
          ? r.ok(`${name}: sample drawn from /raw, no CSP violation`)
          : r.fail(`${name}: loaded=${face} csp=${csp}`);
      } catch {
        r.fail(`${name}: the sample never showed (csp: ${csp})`);
      }
      await p.goto(`${srv.base}/blob/${sha}/fonts/bad.ttf`, { waitUntil: 'load' });
      (await p.locator('.font-preview').count()) === 0
        ? r.ok(`${name}: a broken font shows no sample`)
        : r.fail(`${name}: broken font got a preview`);
      await b.close();
    }
  }
  return { fontSample };
}
