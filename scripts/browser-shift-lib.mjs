// Layout shift between page kinds: moving from one page to another must not move the header,
// its nav, the content's left edge, the top of the first block or the title inside a head row.
// Every page kind is measured in the same boxes and compared with the first page walked. The
// walk reads the nav and the insights tab row, so a new tab is covered without touching the
// list; a new kind of page goes into `fixed`. Imported by scripts/browser-check.mjs.

/** How far (px) a box may differ between two page kinds. */
const TOL = 0.5;

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context } = env;
  const fixed = {
    overview: '/', tree: '/tree/HEAD/src', 'tree-root': '/tree/HEAD', blob: '/blob/HEAD/a.txt',
    docs: '/blob/HEAD/README.md', log: '/log/HEAD', commit: '/commit/HEAD',
    blame: '/blame/HEAD/a.txt', history: '/history/a.txt', compare: '/compare',
    search: '/search?q=one', refs: '/refs',
    // Served at the origin, outside the repository's own root.
    settings: srv.origin + '/_settings',
  };
  async function pages(p) {
    await p.goto(srv.base + '/stats', { waitUntil: 'load' });
    const tabs = await p.$$eval('main .tabs a', (as) => as.map((a) => a.getAttribute('href')));
    const out = { ...fixed };
    for (const h of new Set(tabs)) {
      const path = new URL(h, srv.base + '/').pathname.replace(/^\/r\/[^/]+/, '');
      out[`insights${path.replace(/^\/stats/, '') || '/activity'}`] = path;
    }
    return out;
  }
  /** The boxes that must hold still, in page coordinates, rounded to a tenth of a pixel. */
  function boxes() {
    const box = (e) => {
      if (!e) return null;
      const b = e.getBoundingClientRect();
      return { x: b.x + scrollX, y: b.y + scrollY, w: b.width, h: b.height };
    };
    const main = document.querySelector('main');
    const shown = [...main.children].filter(
      (e) => !e.classList.contains('vh') && e.getBoundingClientRect().height > 0,
    );
    const head = shown[0] && shown[0].classList.contains('head') ? shown[0] : null;
    const title = head && head.firstElementChild;
    // The first line of the title, so a subject that wraps does not read as a shift.
    let t = null;
    if (title) {
      const rg = document.createRange();
      rg.selectNodeContents(title);
      const q = rg.getClientRects()[0] || title.getBoundingClientRect();
      t = { x: q.x + scrollX, y: q.y + scrollY, h: q.height };
    }
    return {
      header: box(document.querySelector('header.top')),
      bar: box(document.querySelector('.topbar')),
      nav: box(document.querySelector('.tabnav')),
      main: box(main),
      first: box(shown[0]),
      title: t && { x: t.x, y: t.y + t.h / 2 },
    };
  }
  async function layoutShift() {
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      const p = await ctx.newPage();
      const all = await pages(p);
      for (const scheme of ['light', 'dark']) for (const width of [1440, 390]) {
        await p.emulateMedia({ colorScheme: scheme });
        await p.setViewportSize({ width, height: 900 });
        let ref = null, refKey = '', refTitleBox = null, refTitleBoxKey = '';
        const bad = [];
        for (const [key, path] of Object.entries(all)) {
          await p.goto(path.startsWith('http') ? path : srv.base + path, { waitUntil: 'load' });
          await p.evaluate(() => document.fonts.ready);
          // Two frames after the fonts, so layout has settled before the boxes are read.
          await p.evaluate(() => {
            const frame = () => new Promise((ok) => requestAnimationFrame(ok));
            return frame().then(frame);
          });
          const m = await p.evaluate(boxes);
          if (!ref) { ref = m; refKey = key; }
          const d = (what, a, c, ...ks) => {
            for (const k of ks) {
              if (a && c && Math.abs(a[k] - c[k]) > TOL) {
                bad.push(`${key} ${what}.${k} ${(a[k] - c[k]).toFixed(1)}`);
              }
            }
          };
          for (const f of ['header', 'bar', 'nav']) d(f, m[f], ref[f], 'x', 'y', 'w', 'h');
          d('main', m.main, ref.main, 'x', 'w');
          d('first', m.first, ref.first, 'x', 'y', 'w');
          if (m.title) {
            if (!refTitleBox) { refTitleBox = m.title; refTitleBoxKey = key; }
            d('title', m.title, refTitleBox, 'x', 'y');
          }
        }
        const where = `layout-shift ${name} ${scheme} ${width}`;
        const against = `against ${refKey}, title against ${refTitleBoxKey}`;
        const n = Object.keys(all).length;
        bad.length ? r.fail(`${where} (${against}): ${bad.join('; ')}`)
          : r.ok(`${where}: ${n} page kinds, header, nav, edge, first block and title hold still`);
      }
      await b.close();
    }
  }
  /**
   * In the page: head-row faults. Every visible child of a head row that is or holds a control
   * shares one vertical centre with the others on its flex line, within 1 px.
   */
  function headAlign(ctl) {
    const bad = [];
    const rows = 'main .head, main>.ph>.ph-row, main>.ph>.fbar, [data-dochead]';
    for (const h of document.querySelectorAll(rows)) {
      if (!h.getClientRects().length) continue;
      const kids = [...h.children].filter((e) => e.getClientRects().length
        && (e.matches(ctl) || e.querySelector(ctl)));
      const lines = [];
      for (const e of kids) {
        const b = e.getBoundingClientRect();
        const line = lines.find((l) => b.top < l.bottom && b.bottom > l.top);
        const it = { e, mid: b.top + b.height / 2 };
        if (!line) { lines.push({ top: b.top, bottom: b.bottom, items: [it] }); continue; }
        line.items.push(it);
        line.top = Math.min(line.top, b.top);
        line.bottom = Math.max(line.bottom, b.bottom);
      }
      const name = (e) => {
        const c = String(e.className).split(' ')[0];
        return e.tagName.toLowerCase() + (c ? '.' + c : '');
      };
      for (const l of lines) {
        const mids = l.items.map((i) => i.mid);
        if (Math.max(...mids) - Math.min(...mids) > 1) {
          bad.push(l.items.map((i) => `${name(i.e)}@${i.mid.toFixed(1)}`).join(' '));
        }
      }
    }
    return bad;
  }
  async function headAlignCheck() {
    const CTL = 'a[href], button, input:not([type=hidden]), select, summary';
    for (const name of browsers) {
      const { b, ctx } = await context(name);
      const p = await ctx.newPage();
      const all = await pages(p);
      // The log's switch and Compare link only show with a rev and the graph view.
      Object.assign(all, { 'log-graph': '/log/HEAD?view=graph' });
      for (const width of [1440, 390]) {
        await p.setViewportSize({ width, height: 900 });
        const bad = [];
        for (const [key, path] of Object.entries(all)) {
          await p.goto(path.startsWith('http') ? path : srv.base + path, { waitUntil: 'load' });
          await p.evaluate(() => document.fonts.ready);
          for (const f of await p.evaluate(headAlign, CTL)) bad.push(`${key}: ${f}`);
        }
        const where = `head-align ${name} ${width}`;
        bad.length ? r.fail(`${where}: ${bad.join('; ')}`)
          : r.ok(`${where}: every head row's controls share one centre`);
      }
      await b.close();
    }
  }
  return { layoutShift, headAlign: headAlignCheck };
}
