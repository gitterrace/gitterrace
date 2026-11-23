// Phone layout (390px): the file list comes before the sidebar, the tab row and the header tools
// are 44px targets, the branch switcher keeps the ref's name, README text has room at the edge,
// and a commit row gives its subject the full width. Imported by browser-check.mjs.
import { sweep } from '../../lib/browser-lib.mjs';

export function checks({ srv, r, browsers, context }) {
  const each = (label, path, fn) =>
    sweep({ browsers, context, widths: [390], height: 900 }, async (p, { name, scheme }) => {
      await p.goto(srv.base + path, { waitUntil: 'load' });
      await p.evaluate(() => document.fonts.ready);
      r.verdict(`${label} ${name} ${scheme}`, await fn(p));
    });

  async function phoneLayout() {
    await each('phone-layout overview', '/', (p) => p.evaluate(() => {
      const bad = [];
      const list = document.querySelector('.cols>article');
      const side = document.querySelector('.cols>aside');
      if (list && side && side.getBoundingClientRect().top < list.getBoundingClientRect().top) {
        bad.push('the sidebar sits above the file list');
      }
      for (const a of document.querySelectorAll('.tabnav a')) {
        const h = a.getBoundingClientRect().height;
        if (h < 44) bad.push(`tab ${a.textContent.trim()} is ${Math.round(h)}px high`);
      }
      for (const e of document.querySelectorAll('.tools button')) {
        const q = e.getBoundingClientRect();
        if (q.width && (q.height < 44 || q.width < 44)) bad.push(`tool ${e.id} is ${Math.round(q.width)}x${Math.round(q.height)}`);
      }
      const ref = document.querySelector('#refbtn .name');
      if (!ref || getComputedStyle(ref).display === 'none' || !ref.getBoundingClientRect().width) {
        bad.push('the current ref is not shown');
      }
      const md = document.querySelector('.readme .markdown-body, .readme .doc-plain');
      if (md && md.getBoundingClientRect().left + parseFloat(getComputedStyle(md).paddingLeft) > 36) {
        bad.push('README text starts more than 36px from the edge');
      }
      return bad;
    }));
    await each('phone-layout log', '/log/HEAD', (p) => p.evaluate(() => {
      const bad = [];
      const row = document.querySelector('.list .crow');
      if (!row) return ['no commit row'];
      const subj = row.querySelector('.subj').getBoundingClientRect();
      const sha = row.querySelector('.sha').getBoundingClientRect();
      if (sha.left < subj.right - 1 && sha.top < subj.bottom - 1) bad.push('the sha shares the subject line');
      if (subj.width < row.getBoundingClientRect().width * 0.7) bad.push(`subject is ${Math.round(subj.width)}px wide`);
      return bad;
    }));
  }
  return { phoneLayout };
}
