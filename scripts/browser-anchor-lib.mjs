// The md-anchor check: a cross-file Markdown link with #heading lands on the heading. Imported by scripts/browser-check.mjs, not run on its own.
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';

/** @param {any} env what browser-check.mjs set up (dir, srv, r, browsers, context, cfg, ...) */
export function checks(env) {
  const { dir, srv, r, browsers, context, cfg } = env;
  // The rendered heading ids carry a "user-content-" prefix, the link keeps the plain
  // fragment. The commit goes on a side ref through a private index, so HEAD and the work tree
  // the other checks use stay put.
  async function mdAnchor() {
    const gitEnv = { ...process.env, GIT_INDEX_FILE: join(cfg, 'anchor-index') };
    const git = (input, ...a) => execFileSync('git', a, { cwd: dir, env: gitEnv, input, encoding: 'utf8' }).trim();
    git(undefined, 'read-tree', 'HEAD');
    const filler = Array.from({ length: 80 }, (_, i) => `Paragraph ${i + 1} of filler text.\n`).join('\n');
    const files = {
      'anchor/index.md': '# Index\n\n[syntax](guide.md#syntax-highlighting) [lines](guide.md#L2)\n',
      'anchor/guide.md': `# Guide\n\n${filler}\n## Syntax highlighting\n\nThe target.\n\n${filler}`,
    };
    for (const [path, body] of Object.entries(files)) {
      git(undefined, 'update-index', '--add', '--cacheinfo', `100644,${git(body, 'hash-object', '-w', '--stdin')},${path}`);
    }
    const sha = git(undefined, 'commit-tree', git(undefined, 'write-tree'), '-p', 'HEAD', '-m', 'Anchor fixtures');
    git(undefined, 'update-ref', 'refs/heads/anchor-check', sha);
    const seen = async (p) => p.evaluate(() => {
      const h = document.getElementById('user-content-syntax-highlighting');
      if (!h) return null;
      const t = h.getBoundingClientRect().top;
      return { top: t, inView: t >= -2 && t < innerHeight, scrolled: scrollY > 0 };
    });
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
      await b.close();
    }
  }
  return { mdAnchor };
}
