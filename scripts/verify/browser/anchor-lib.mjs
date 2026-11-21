// The md-anchor check: a cross-file Markdown link with #heading lands on the heading.
// Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
import { deflateSync } from 'node:zlib';
import { join } from 'node:path';
import { launch, commitFiles } from '../../lib/browser-lib.mjs';

/** A valid w x h grey PNG, as bytes. */
function png(w, h) {
  const crcTable = Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc = (b) => {
    let c = 0xffffffff;
    for (const x of b) c = crcTable[(c ^ x) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const chunk = (type, data) => {
    const t = Buffer.concat([Buffer.from(type), data]);
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const c = Buffer.alloc(4); c.writeUInt32BE(crc(t));
    return Buffer.concat([len, t, c]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 0;
  const raw = Buffer.alloc((w + 1) * h, 0x80);
  for (let y = 0; y < h; y++) raw[y * (w + 1)] = 0;
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const end = chunk('IEND', Buffer.alloc(0));
  return Buffer.concat([sig, chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), end]);
}

/** @param {any} env what browser-check.mjs set up (dir, srv, r, browsers, context, cfg, ...) */
export function checks(env) {
  const { dir, srv, r, browsers, context, cfg } = env;
  // Heading ids are plain, as a user writes them, so the link resolves with JS on and off. A
  // heading named like a chrome id ("theme") is prefixed, and its pilcrow and outline links say
  // so; a typed #main still lands on the heading, not the page's <main>. The
  // commit goes on a side ref through a private index, so HEAD and the work tree the other checks
  // use stay put.
  async function mdAnchor() {
    const filler = Array.from({ length: 80 }, (_, i) => `Paragraph ${i + 1} of filler text.\n`).join('\n');
    const files = {
      'anchor/index.md': '# Index\n\n[main](guide.md#main) [syntax](guide.md#syntax-highlighting) [lines](guide.md#L2)\n',
      'anchor/guide.md': [
        `# Guide\n\n${filler}`, `## Syntax highlighting\n\nThe target.\n\n${filler}`,
        `## main\n\nClobber bait.\n\n${filler}`, `## theme\n\nChrome id.\n\n${filler}`,
      ].join('\n'),
      'anchor/lines.txt': Array.from({ length: 200 }, (_, i) => `line ${i + 1}`).join('\n') + '\n',
      // The folder's own README, long enough that its card can always scroll to the top.
      'anchor/README.md': `# Anchor folder\n\n${filler}`,
    };
    const index = join(cfg, 'anchor-index');
    const sha = commitFiles(dir, index, files, 'refs/heads/anchor-check', 'Anchor fixtures');
    // A commit of tall pictures above one text file: lazy images with no reserved box would
    // grow above #fN after the jump and push it down the screen.
    const pictures = {};
    for (let i = 0; i < 12; i++) {
      pictures[`anchor/img${String(i).padStart(2, '0')}.png`] = png(300, 400 + i);
    }
    const tail = Array.from({ length: 300 }, (_, i) => `after the pictures ${i}`);
    pictures['anchor/zz.txt'] = tail.join('\n') + '\n';
    const pics = commitFiles(dir, index, pictures, 'refs/heads/anchor-pics', 'Pictures', sha);
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
      // A heading named "main" is plain now that the page's own <main> is gt-main.
      const c = await ctx.newPage();
      await c.setViewportSize({ width: 900, height: 600 });
      await c.goto(`${srv.base}/blob/${sha}/anchor/guide.md#main`, { waitUntil: 'load' });
      await c.waitForTimeout(400);
      const own = await c.evaluate(() => document.getElementById('gt-main')?.tagName);
      m = await seen(c, 'main');
      own === 'MAIN' && m && m.scrolled && m.inView ? r.ok(`${where}: #main lands on the heading, page <main> intact`) : r.fail(`${where}: #main: own=${own} ${m ? `heading at ${Math.round(m.top)} px` : 'no heading'}`);
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
      // Each anchor lands the same with JS on and off: the browser does the jump, no script
      // re-scrolls. Pictures are held back so they arrive after the jump.
      for (const js of [true, false]) {
        const jb = await launch(env.pw, name);
        const jctx = await jb.newContext({ javaScriptEnabled: js });
        await jctx.route('**/raw/**', async (q) => {
          await new Promise((ok) => setTimeout(ok, 400));
          await q.continue();
        });
        const how = `${where} JS ${js ? 'on' : 'off'}`;
        const land = async (path, id, lo, hi) => {
          const pg = await jctx.newPage();
          await pg.setViewportSize({ width: 1100, height: 700 });
          await pg.goto(`${srv.base}${path}#${id}`, { waitUntil: 'load' });
          await pg.waitForTimeout(1500);
          const at = await pg.evaluate((x) => {
            const e = document.getElementById(x);
            if (!e) return null;
            return { top: Math.round(e.getBoundingClientRect().top), y: scrollY, tag: e.tagName };
          }, id);
          await pg.close();
          at && at.y > 0 && at.top >= lo && at.top <= hi
            ? r.ok(`${how}: #${id} lands at ${at.top} px`)
            : r.fail(`${how}: #${id} on ${path}: ${at ? `${at.top} px, y ${at.y}` : 'missing'}`);
          return at;
        };
        const files = await (async () => {
          const pg = await jctx.newPage();
          await pg.goto(`${srv.base}/commit/${pics}`, { waitUntil: 'load' });
          const ids = await pg.evaluate(() => [...document.querySelectorAll('section.file')]
            .map((x) => [x.id, x.getAttribute('data-path')]));
          await pg.close();
          return ids;
        })();
        const last = files.find(([, p]) => p === 'anchor/zz.txt');
        if (!last) r.fail(`${how}: no section for anchor/zz.txt`);
        else await land(`/commit/${pics}`, last[0], -2, 200);
        await land(`/blob/${sha}/anchor/lines.txt`, 'L120', -2, 350);
        const typed = await land(`/blob/${sha}/anchor/guide.md`, 'main', -2, 200);
        if (typed && typed.tag === 'MAIN') r.fail(`${how}: #main resolved to the page's <main>`);
        await land(`/blob/${sha}/anchor/guide.md`, 'user-content-theme', -2, 200);
        // The about card lands flush at the top, its border just above the window. The folder's
        // own long README keeps the page tall, whatever the other checks did to the repo's root.
        {
          const pg = await jctx.newPage();
          await pg.setViewportSize({ width: 1100, height: 600 });
          await pg.goto(`${srv.base}/tree/${sha}/anchor#about`, { waitUntil: 'load' });
          await pg.waitForTimeout(1500);
          const at = await pg.evaluate(() => {
            const t = document.getElementById('about').getBoundingClientRect().top;
            const end = scrollY >= document.documentElement.scrollHeight - innerHeight - 1;
            return { top: Math.round(t * 10) / 10, end };
          });
          await pg.close();
          at.top > -1.5 && at.top <= 0 && !at.end
            ? r.ok(`${how}: #about lands flush at ${at.top} px`)
            : r.fail(`${how}: #about at ${at.top} px${at.end ? ', page at its end' : ''}`);
        }
        await jb.close();
      }
    }
  }
  return { mdAnchor };
}
