// The md-anchor check: a cross-file Markdown link with #heading lands on the heading.
// Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { launch } from '../../lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (dir, srv, r, browsers, context, cfg, ...) */
export function checks(env) {
  const { dir, srv, r, browsers, context, cfg } = env;
  // Heading ids are plain, as a user writes them, so the link resolves with JS on and off. A
  // heading named like a chrome id ("main") is prefixed and reached through app.js. The commit goes
  // on a side ref through a private index, so HEAD and the work tree the other checks use stay
  // put.
  async function mdAnchor() {
    const gitEnv = { ...process.env, GIT_INDEX_FILE: join(cfg, 'anchor-index') };
    const git = (input, ...a) => execFileSync('git', a, { cwd: dir, env: gitEnv, input, encoding: 'utf8' }).trim();
    git(undefined, 'read-tree', 'HEAD');
    const filler = Array.from({ length: 80 }, (_, i) => `Paragraph ${i + 1} of filler text.\n`).join('\n');
    const files = {
      'anchor/index.md': '# Index\n\n[main](guide.md#main) [syntax](guide.md#syntax-highlighting) [lines](guide.md#L2)\n',
      'anchor/guide.md': `# Guide\n\n${filler}\n## Syntax highlighting\n\nThe target.\n\n${filler}\n## main\n\nClobber bait.\n\n${filler}`,
    };
    for (const [path, body] of Object.entries(files)) {
      git(undefined, 'update-index', '--add', '--cacheinfo', `100644,${git(body, 'hash-object', '-w', '--stdin')},${path}`);
    }
    const sha = git(undefined, 'commit-tree', git(undefined, 'write-tree'), '-p', 'HEAD', '-m', 'Anchor fixtures');
    git(undefined, 'update-ref', 'refs/heads/anchor-check', sha);
    const seen = async (p, id = 'syntax-highlighting') => p.evaluate((id) => {
      const h = document.getElementById(id);
      if (!h) return null;
      const t = h.getBoundingClientRect().top;
      return { top: t, inView: t >= -2 && t < innerHeight, scrolled: scrollY > 0 };
    }, id);
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      const where = `md-anchor ${name}`;
      // Through the link on the index page.
      const p = await ctx.newPage();
      await p.setViewportSize({ width: 900, height: 600 });
      await p.goto(`${srv.base}/blob/${sha}/anchor/index.md`, { waitUntil: 'load' });
      await p.click('a[href$="guide.md#syntax-highlighting"]');
      await p.waitForURL(/guide\.md#syntax-highlighting$/);
      await p.waitForTimeout(400);
      let m = await seen(p);
      if (!m) r.fail(`${where}: link: no rendered heading on the target page`);
      else if (!m.scrolled || !m.inView) r.fail(`${where}: link: heading at ${Math.round(m.top)} px, page ${m.scrolled ? 'scrolled' : 'not scrolled'}`);
      else r.ok(`${where}: link lands on the heading (${Math.round(m.top)} px from the top)`);
      // A typed URL, then a hash change on the same page.
      const q = await ctx.newPage();
      await q.setViewportSize({ width: 900, height: 600 });
      await q.goto(`${srv.base}/blob/${sha}/anchor/guide.md#syntax-highlighting`, { waitUntil: 'load' });
      await q.waitForTimeout(400);
      m = await seen(q);
      m && m.scrolled && m.inView ? r.ok(`${where}: typed URL lands on the heading`) : r.fail(`${where}: typed URL: ${m ? `heading at ${Math.round(m.top)} px` : 'no heading'}`);
      await q.evaluate(() => { history.replaceState(null, '', location.pathname); scrollTo(0, 0); });
      await q.evaluate(() => { location.hash = '#syntax-highlighting'; });
      await q.waitForTimeout(400);
      m = await seen(q);
      m && m.scrolled && m.inView ? r.ok(`${where}: hashchange lands on the heading`) : r.fail(`${where}: hashchange: ${m ? `heading at ${Math.round(m.top)} px` : 'no heading'}`);
      // A heading named "main" must not clobber the page, and a typed #main still lands on it.
      const c = await ctx.newPage();
      await c.setViewportSize({ width: 900, height: 600 });
      await c.goto(`${srv.base}/blob/${sha}/anchor/guide.md#main`, { waitUntil: 'load' });
      await c.waitForTimeout(400);
      const own = await c.evaluate(() => document.getElementById('main') && document.getElementById('main').tagName);
      m = await seen(c, 'user-content-main');
      own === 'MAIN' && m && m.scrolled && m.inView ? r.ok(`${where}: #main lands on the heading, page <main> intact`) : r.fail(`${where}: #main: own=${own} ${m ? `heading at ${Math.round(m.top)} px` : 'no heading'}`);
      // A busy machine: the browser's own scroll to the page's <main> comes well after load.
      // Played here as a late scroll 150 ms after load; the jump must still win.
      const late = await ctx.newPage();
      await late.setViewportSize({ width: 900, height: 600 });
      await late.addInitScript(() => {
        addEventListener('load', () => setTimeout(() => {
          const el = document.querySelector('main');
          if (el) el.scrollIntoView();
        }, 150));
      });
      await late.goto(`${srv.base}/blob/${sha}/anchor/guide.md#main`, { waitUntil: 'load' });
      await late.waitForTimeout(1200);
      m = await seen(late, 'user-content-main');
      m && m.scrolled && m.inView ? r.ok(`${where}: #main still lands on the heading after a late load scroll`) : r.fail(`${where}: #main after a late load scroll: ${m ? `heading at ${Math.round(m.top)} px` : 'no heading'}`);
      await b.close();
      // JS off, as an export may be opened: the plain fragment resolves by the browser alone.
      const nb = await launch(env.pw, name);
      const nctx = await nb.newContext({ javaScriptEnabled: false });
      const n = await nctx.newPage();
      await n.setViewportSize({ width: 900, height: 600 });
      await n.goto(`${srv.base}/blob/${sha}/anchor/guide.md#syntax-highlighting`, { waitUntil: 'load' });
      await n.waitForTimeout(400);
      const pos = await n.evaluate(() => ({ y: scrollY }));
      const box = await n.locator('[id="syntax-highlighting"]').boundingBox();
      box && pos.y > 0 && box.y >= -2 && box.y < 600 ? r.ok(`${where}: JS off, plain #fragment lands on the heading`) : r.fail(`${where}: JS off: y=${pos.y} box=${box && Math.round(box.y)}`);
      await nb.close();
    }
  }
  return { mdAnchor };
}
