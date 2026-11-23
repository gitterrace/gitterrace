// Page heads: each page's controls sit in the title row or one bar, never in rows of their own.
// Imported by scripts/verify/browser/browser-check.mjs, not run on its own.
import { sweep } from '../../lib/browser-lib.mjs';

/** Visible head rows of the page: the path bar and any document header, as boxes. */
const HEADS = 'main .head, main>.ph>.ph-row, main>.ph>.fbar, [data-dochead]';

export function checks({ srv, r, browsers, context }) {
  /** Runs `fn(page)` returning a list of faults at each width and scheme; reports one line each. */
  async function each(label, path, fn, widths = [1440, 900, 390]) {
    await sweep({ browsers, context, widths }, async (p, { name, scheme, width }) => {
      await p.goto(srv.base + path, { waitUntil: 'load' });
      r.verdict(`page-heads ${name} ${scheme} ${width} ${label}`, await fn(p, width));
    });
  }

  async function pageHeads() {
    // A Markdown file: the same pinned page head as any file holds the path, the File, Blame
    // and History tabs and, in its toolbar, the Preview/Code switch. No
    // Copy permalink anywhere in either (it is in the page menu).
    await each('markdown blob', '/blob/HEAD/README.md', (p, width) => p.evaluate((w) => {
      const bad = [];
      const ph = document.querySelector('main>.ph');
      if (!ph || !ph.querySelector('h1.crumbs')) bad.push('no path in the page head');
      const tabs = ph ? [...ph.querySelectorAll('.tabs a')].map((a) => a.textContent.trim()) : [];
      if (!tabs.includes('File') || !tabs.includes('History')) {
        bad.push(`file tabs ${tabs.join(',') || 'missing'}`);
      }
      if (document.querySelector('[data-md-view] [data-dochead]')) {
        bad.push('a card head row under the page head');
      }
      if (ph) {
        const both = ph.querySelector('[data-show=rendered]')
          && ph.querySelector('[data-show=source]');
        if (!both) bad.push('no Preview/Code in the page head');
        // The pin is for wide screens only (pagehead.css), as for every file.
        if (w > 600 && getComputedStyle(ph).position !== 'sticky') {
          bad.push('the page head is not sticky');
        }
      }
      const text = ph?.textContent || '';
      if (/Copy permalink/.test(text)) bad.push('Copy permalink is still shown');
      return bad;
    }, width));
  }
  /** Rows of controls: how many direct rows of `root`'s children hold only `n` controls. */
  const CTL = 'a[href], button, input:not([type=hidden]), select, summary';



  async function compareHeads() {
    await each('compare', '/compare?base=HEAD~2&head=HEAD', (p) => p.evaluate((ctl) => {
      const bad = [];
      const form = document.querySelector('form.cmp');
      const head = document.querySelector('main>.ph>.fbar');
      if (!form || !head || !head.contains(form)) bad.push('the form is not in the head toolbar');
      // No row of one control: every direct child of main that holds controls holds more than one,
      // except the KPI row, whose head sha is a link in a row of figures.
      for (const e of document.querySelectorAll('main > p.hint')) {
        if (e.querySelectorAll(ctl).length === 1 && /Open the head commit/.test(e.textContent)) bad.push('Open the head commit has a row of its own');
      }
      if (!document.querySelector('.kpis a[href*="/commit/"]')) bad.push('no link on the head sha');
      const h = head && head.getBoundingClientRect(), f = form && form.getBoundingClientRect();
      const wraps = h && f && innerWidth > 600 && f.top - h.top > h.height;
      if (wraps) bad.push('the form wraps in the toolbar');
      return bad;
    }, CTL), [1440, 900]);
    await each('compare phone', '/compare?base=HEAD~2&head=HEAD', (p) => p.evaluate(() => {
      const form = document.querySelector('form.cmp');
      const head = document.querySelector('main>.ph>.fbar');
      return form && head && head.contains(form) ? [] : ['the form is not in the head toolbar'];
    }), [390]);
  }
  async function crumbHeads() {
    // The path flows as one line of text: no crumb starts a new line while the "/" before it ends
    // the previous one, unless that crumb alone is wider than the heading.
    const check = (p) => p.evaluate(() => {
      const bad = [];
      const h = document.querySelector('main>.ph h1.crumbs');
      if (!h) return ['no path in the page head'];
      const hw = h.getBoundingClientRect().width;
      for (const a of h.querySelectorAll('a')) {
        const sep = a.previousElementSibling;
        if (!sep || sep.tagName !== 'I') continue;
        const ar = a.getClientRects()[0], sr = sep.getClientRects()[0];
        if (ar && sr && ar.top >= sr.bottom - 1 && a.getBoundingClientRect().width <= hw) {
          bad.push(`"${a.textContent}" drops below its slash`);
        }
      }
      return bad;
    });
    // The deepest fixture path, so the last name has to share a line with the actions.
    const deep = 'src/a_rather_long_directory_name';
    await each('crumbs folder', `/tree/HEAD/${deep}`, check, [1440, 390]);
    const file = `${deep}/a_file_with_a_really_long_name_for_wrapping.rs`;
    await each('crumbs file', `/blob/HEAD/${file}`, check, [1440, 390]);
  }
  async function commitHeads() {
    await each('commit', '/commit/HEAD', (p) => p.evaluate(() => {
      const bad = [];
      const head = document.querySelector('main>.ph'), meta = document.querySelector('p.meta');
      const under = meta && head?.nextElementSibling === meta;
      if (!under) bad.push('the meta line is not right under the head');
      if (document.querySelector('p.meta button.copy')) bad.push('a Copy button is still on the meta line');
      const sum = document.querySelector('details.files > summary');
      if (!sum || !sum.querySelector('.counts .add') || !sum.querySelector('.counts .del')) bad.push('no totals in the changed-files header');
      // Identity above, ids below: the sha, each parent and Browse files wrap as units with no
      // dot separators, and the actions button is a labelled control at the row's end.
      const ids = document.querySelector('.meta.ids');
      if (!ids) bad.push('no ids row');
      else {
        if (/\u00b7/.test(ids.textContent)) bad.push('dot separators in the ids row');
        const b = ids.querySelector('#actcommit');
        const q = b?.getBoundingClientRect();
        if (!b || !b.getAttribute('aria-label')) bad.push('the actions button has no label');
        else if (Math.abs(q.right - ids.getBoundingClientRect().right) > 2) bad.push('the actions button is not at the row end');
      }
      const c = sum?.querySelector('.counts');
      if (c && getComputedStyle(c).textTransform !== 'none') bad.push('the counts are uppercase');
      if (c && getComputedStyle(c).letterSpacing !== 'normal' && parseFloat(getComputedStyle(c).letterSpacing) !== 0) bad.push('the counts are tracked');
      if (sum && getComputedStyle(sum, '::before').content === 'none') bad.push('no caret on the files summary');
      const body = document.querySelector('pre.body');
      if (body) {
        if (/\n\n\n/.test(body.textContent)) bad.push('three newlines in the message body');
        if (!/mono|Mono/.test(getComputedStyle(body).fontFamily)) bad.push('the body is not set in mono');
      }
      return bad;
    }));
  }
  async function searchHeads() {
    await each('search', '/search?q=one', (p) => p.evaluate(() => {
      const bad = [];
      const form = document.querySelector('main form[action$="/search"]');
      if (!form) return ['no search form'];
      if (form.classList.contains('card') || getComputedStyle(form).borderTopWidth !== '0px') bad.push('the bar is a card');
      const rows = [...form.querySelectorAll('.srow')];
      if (rows.length !== 2) bad.push(`${rows.length} rows, want 2`);
      const r1 = rows[0];
      if (r1 && (!r1.querySelector('input[name=q]') || !r1.querySelector('.segctl'))) bad.push('row 1 lacks the query or the Code/Commits switch');
      if (form.querySelector('input[type=radio]')) bad.push('radio buttons for the kind');
      const tag = document.querySelector('main>.ph>.ph-tag')?.textContent || '';
      if (!/match/.test(tag)) bad.push('the summary is not the tagline');
      if (form.nextElementSibling?.classList.contains('sum')) bad.push('a separate summary line');
      return bad;
    }));
  }
  return { pageHeads, crumbHeads, compareHeads, commitHeads, searchHeads };
}
