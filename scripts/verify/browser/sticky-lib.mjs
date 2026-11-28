// Pinned heads, one check per view: scrolled past its top, each document or code card keeps its
// head at the top of the window, closed by a line under it as wide as the card's content box,
// the outline button sits inside the bar, and the Markdown Code view spans the content width.
// Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
import { execFileSync } from 'node:child_process';
import { writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { eachBrowser, synced, frames } from '../../lib/browser-lib.mjs';

/** The views: a page, the head and the box it closes, and whether to switch to Code first. */
const VIEWS = [
  { name: 'overview README', path: '/', head: '#about [data-dochead]', box: '#about' },
  { name: 'Markdown preview', path: '/blob/HEAD/guide/GUIDE.md', head: 'main>.ph.pin', box: 'main',
    md: true },
  { name: 'Markdown code', path: '/blob/HEAD/guide/GUIDE.md', head: 'main>.ph.pin', box: 'main',
    md: true, code: true },
  { name: 'blob', path: '/blob/HEAD/guide/long.txt', head: 'main>.ph.pin', box: 'main' },
  { name: 'blame', path: '/blame/HEAD/guide/long.txt', head: 'main>.ph.pin', box: 'main' },
  { name: 'diff file', path: '/commit/HEAD', head: 'section.file>header', box: 'section.file' },
  { name: 'table', path: '/stats/hotspots', head: '.entries tr.th:first-child th',
    box: '.card:has(>table.entries tr.th:first-child)' },
];

/** Adds the files the views need as one commit on the served branch. */
function addFiles(dir) {
  const sh = (...a) => execFileSync('git', a, { cwd: dir, encoding: 'utf8' }).trim();
  const md = '# Guide\n\n' + Array.from({ length: 10 }, (_, i) => `## Part ${i}\n\n` + 'Words in a paragraph. '.repeat(30) + '\n\n| a | b |\n| --- | --- |\n| ' + 'wide cell '.repeat(40) + ' | x |\n\n```\n' + 'wide_code_'.repeat(40) + '\n```\n').join('\n');
  const long = Array.from({ length: 300 }, (_, i) => `line ${i + 1} ` + 'x'.repeat(i % 7 === 0 ? 400 : 10)).join('\n') + '\n';
  return { md, long, sh };
}

export function checks({ pw, dir, srv, r, browsers, context }) {
  async function seed() {
    const { md, long, sh } = addFiles(dir);
    mkdirSync(join(dir, 'guide'), { recursive: true });
    writeFileSync(join(dir, 'guide/GUIDE.md'), md);
    writeFileSync(join(dir, 'guide/long.txt'), long);
    // A README long enough to scroll past, and files enough for the hotspots table to scroll.
    writeFileSync(join(dir, 'README.md'), md);
    mkdirSync(join(dir, 'many'), { recursive: true });
    for (let i = 0; i < 60; i++) writeFileSync(join(dir, `many/f${i}.txt`), `${i}\n`);
    sh('add', 'guide', 'README.md', 'many');
    sh('-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '-m', 'Docs and long files');
    await synced(srv, dir);
  }

  async function stickyLine() {
    await seed();
    await eachBrowser(browsers, context, async (ctx, name) => {
      for (const scheme of ['light', 'dark']) for (const width of [1400, 900]) for (const v of VIEWS) {
        if (v.name === 'table' && width < 1101) continue;
        const p = await ctx.newPage();
        await p.emulateMedia({ colorScheme: scheme });
        await p.setViewportSize({ width, height: 700 });
        await p.goto(srv.base + v.path, { waitUntil: 'load' });
        if (v.code) await p.click('[data-show=source]');
        // Scroll the box's top 300 px above the window, then let the scroll handler run.
        await p.evaluate((s) => { const box = document.querySelector(s); if (box) scrollTo(0, box.getBoundingClientRect().top + scrollY + 300); }, v.box);
        await frames(p);
        const m = await p.evaluate(({ head, box }) => {
          const hs = [...document.querySelectorAll(head)];
          const bx = document.querySelector(box);
          if (!hs.length || !bx) return null;
          const h = hs[0], hr = h.getBoundingClientRect(), br = bx.getBoundingClientRect();
          // The closing line sits on the head's bottom edge: its border, an inset shadow or a
          // 1 px ::after there. A line on the top edge (a ::before or a border) reads as over it.
          let lineW = 0, lineH = 0, topLine = false;
          // A table's head cells together make one line; any other view has one head per box.
          for (const e of h.tagName === 'TD' || h.tagName === 'TH' ? [...h.parentElement.children] : [h]) {
            const st = getComputedStyle(e), af = getComputedStyle(e, '::after');
            const w = e.getBoundingClientRect().width;
            const be = getComputedStyle(e, '::before');
            const drawn = (x) => x.content !== 'none' && x.content !== 'normal';
            if ((drawn(be) && parseFloat(be.height) <= 2) || parseFloat(st.borderTopWidth) > 0) {
              topLine = true;
            }
            const inset = /inset 0px -1px|0px -1px 0px 0px .*inset/.test(st.boxShadow);
            if (parseFloat(st.borderBottomWidth) === 1 || inset) {
              lineW += w;
              lineH = 1;
            } else if (drawn(af) && parseFloat(af.height) === 1 && af.bottom === '0px') {
              lineW += parseFloat(af.width) || w;
              lineH = 1;
            }
          }
          const content = bx.clientWidth - (box === 'main' ? parseFloat(getComputedStyle(bx).paddingLeft) + parseFloat(getComputedStyle(bx).paddingRight) : 0);
          const btn = h.querySelector('summary.outlinebtn');
          let inBar = true;
          if (btn) { const s = btn.getBoundingClientRect(); inBar = s.top >= hr.top && s.bottom <= hr.bottom - 1 && s.right <= hr.right; }
          const code = [...document.querySelectorAll('[data-pane=source]')]
            .find((e) => e.checkVisibility());
          const mainBox = document.querySelector('main').getBoundingClientRect();
          const mainPad = parseFloat(getComputedStyle(document.querySelector('main')).paddingLeft) * 2;
          // A diff's heads pin under the sticky diff toolbar, so the toolbar's bottom is their top.
          const bar = document.querySelector('.difftools');
          const sticky = bar && getComputedStyle(bar).position === 'sticky';
          const pin = sticky ? bar.getBoundingClientRect().bottom : 0;
          return { top: hr.top - pin, pin,
            topLine, stuck: h.hasAttribute('data-stuck'), lineW, lineH, content, inBar, codeW: code ? code.closest('.card').getBoundingClientRect().width : null, mainW: mainBox.width - mainPad, scrolled: br.top < 0 };
        }, v);
        const where = `sticky-line ${name} ${scheme} ${width} ${v.name}`;
        const bad = [];
        if (!m) bad.push('head or box missing');
        else {
          if (!m.scrolled) bad.push('could not scroll past the box top');
          if (Math.abs(m.top) > 1) {
            bad.push(`head ${m.top.toFixed(1)} px off its pin line, not pinned`);
          }
          if (!m.stuck || m.lineH !== 1) {
            bad.push(`no closing line under the head (stuck ${m.stuck}, ${m.lineH}px)`);
          }
          if (m.topLine) bad.push('a line on the top edge, over the head');
          else if (Math.abs(m.lineW - m.content) > 1) bad.push(`line ${m.lineW.toFixed(1)} px, content box ${m.content.toFixed(1)} px`);
          if (!m.inBar) bad.push('outline button leaves the bar');
          if (v.code && Math.abs(m.codeW - m.mainW) > 1) bad.push(`Code ${m.codeW} px wide, content ${m.mainW} px`);
        }
        if (v.md) {
          // Clipping the card must not stop the wide parts from scrolling sideways in their pane.
          const sideways = await p.evaluate((code) => {
            const el = document.querySelector(code ? '[data-pane=source]' : '.markdown-body pre');
            if (!el || el.scrollWidth <= el.clientWidth) return 'nothing wider than its pane';
            el.scrollLeft = 50;
            return el.scrollLeft > 0 ? '' : 'the wide part does not scroll sideways';
          }, v.code);
          if (sideways) bad.push(sideways);
          // A heading anchor or a focused line lands below the pinned head, not under it.
          const target = v.code ? '#L40' : '#part-6';
          await p.goto(srv.base + v.path + target, { waitUntil: 'load' });
          if (v.code) { await p.click('[data-show=source]'); await p.evaluate(() => document.querySelector('#L40')?.scrollIntoView()); }
          await frames(p);
          const under = await p.evaluate((t) => {
            const h = document.querySelector('main>.ph.pin'), el = document.querySelector(t);
            if (!h || !el) return 'no target';
            return el.getBoundingClientRect().top >= h.getBoundingClientRect().bottom - 1 ? '' : `${t} lands under the head`;
          }, target);
          if (under) bad.push(under);
          await p.goto(srv.base + v.path, { waitUntil: 'load' });
          if (v.code) await p.click('[data-show=source]');
        }
        await p.evaluate(() => scrollTo(0, 0));
        await frames(p);
        const top = await p.evaluate((s) => document.querySelector(s)?.hasAttribute('data-stuck'), v.head);
        if (top) bad.push('closing line shown before any scroll');
        r.verdict(where, bad);
        await p.close();
      }
    });
  }
  return { stickyLine };
}
