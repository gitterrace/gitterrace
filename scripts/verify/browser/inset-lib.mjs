// Vertical balance: the one-line blame row and the Send to action row have as much space
// above their content as below it (1 px tolerance), and the label ink of the Send to buttons,
// its picker and a cap-only probe at 28 and 32 px sits centered in its box (read from
// screenshot pixels), light and dark. Imported by browser-check.mjs, not run on its own.

import { mkdtempSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { inflateSync } from 'node:zlib';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { serve } from '../../lib/browser-lib.mjs';

/** Device pixels per CSS px for the ink screenshots, sub-pixel positions averaged, and the
 * most the ink above may differ from the ink below, in CSS px: a label is centered within a
 * quarter pixel either way. */
const INK_SCALE = 4;
const SHIFTS = 8;
const INK_TOL = 0.5;

/** A repository whose one file has a single line. */
function oneLineRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'gt-inset-'));
  const git = (...a) => execFileSync('git', a, {
    cwd: dir,
    env: {
      ...process.env, GIT_AUTHOR_NAME: 'Al', GIT_AUTHOR_EMAIL: 'a@a',
      GIT_COMMITTER_NAME: 'Al', GIT_COMMITTER_EMAIL: 'a@a',
    },
  });
  git('init', '-q', '-b', 'main');
  writeFileSync(join(dir, 'one.txt'), 'only line\n');
  git('add', '-A');
  git('commit', '-q', '-m', 'one');
  return dir;
}

/** Space above and below the content of each cell of the row, inside the row's box. */
export function blameInsets() {
  const tr = /** @type {HTMLTableRowElement} */ (document.querySelector('table.blame tbody tr'));
  const box = tr.getBoundingClientRect();
  const out = [];
  for (const td of tr.cells) {
    const bl = td.querySelector('.bl');
    const rg = document.createRange();
    rg.selectNodeContents(td);
    const c = bl ? bl.getBoundingClientRect() : rg.getBoundingClientRect();
    out.push({ cell: td.className || 'code', top: c.top - box.top, bottom: box.bottom - c.bottom });
  }
  return out;
}

/** Space above and below the buttons inside the footer, its border not counted. */
export function footInsets(sel) {
  const foot = document.querySelector(sel);
  const box = foot.getBoundingClientRect();
  const cs = getComputedStyle(foot);
  const kids = [...foot.querySelectorAll('button')].map((b) => b.getBoundingClientRect());
  const top = Math.min(...kids.map((k) => k.top)), bottom = Math.max(...kids.map((k) => k.bottom));
  return [{
    cell: sel,
    top: top - box.top - parseFloat(cs.borderTopWidth),
    bottom: box.bottom - bottom - parseFloat(cs.borderBottomWidth),
  }];
}

/** The Send to footer's controls the ink check reads, with a cap-only probe at both heights:
 * capitals and an "HHHH" option have no ascender or descender to skew the ink. */
export function inkProbes() {
  const foot = document.querySelector('.send-foot');
  const add = (cls, id) => {
    const e = document.createElement('button');
    e.type = 'button';
    e.className = cls;
    e.id = id;
    e.textContent = 'HHHH';
    foot.prepend(e);
  };
  add('sm', 'ink-sm');
  add('', 'ink-32');
  const pick = foot.querySelector('select');
  pick.add(new Option('HHHH', 'ink', true, true));
  /** @type {HTMLElement} */ (document.activeElement)?.blur();
  return ['#ink-sm', '#ink-32', '.send-cancel', '.send-go', '.send-foot select'];
}

/** Each probe's box and the columns of its first glyph (a select's text starts at its padding),
 * in CSS px from the box. Only the first capital is read, so a lowercase ascender never counts. */
export function inkBoxes(sels) {
  return sels.map((s) => {
    const e = document.querySelector(s);
    const r = e.getBoundingClientRect();
    let x0 = parseFloat(getComputedStyle(e).paddingLeft), x1 = x0 + 6;
    if (e.tagName !== 'SELECT') {
      const rg = document.createRange();
      rg.setStart(e.firstChild, 0);
      rg.setEnd(e.firstChild, 1);
      const g = rg.getBoundingClientRect();
      x0 = g.left - r.left;
      x1 = g.right - r.left;
    }
    return { s, x: r.x, y: r.y, w: r.width, h: r.height, x0, x1 };
  });
}

/** RGBA pixels of an 8-bit RGB or RGBA PNG (what Playwright writes), unfiltered by hand. */
function decodePng(buf) {
  const w = buf.readUInt32BE(16), h = buf.readUInt32BE(20), n = buf[25] === 6 ? 4 : 3;
  const parts = [];
  for (let o = 8; o < buf.length;) {
    const len = buf.readUInt32BE(o), type = buf.toString('ascii', o + 4, o + 8);
    if (type === 'IDAT') parts.push(buf.subarray(o + 8, o + 8 + len));
    o += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(parts));
  const st = w * n, un = Buffer.alloc(st * h);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (st + 1)];
    for (let i = 0; i < st; i++) {
      const v = raw[y * (st + 1) + 1 + i];
      const a = i >= n ? un[y * st + i - n] : 0, b = y ? un[(y - 1) * st + i] : 0;
      const c = i >= n && y ? un[(y - 1) * st + i - n] : 0;
      const pa = Math.abs(b - c), pb = Math.abs(a - c), pc = Math.abs(a + b - 2 * c);
      const pred = [0, a, b, (a + b) >> 1, pa <= pb && pa <= pc ? a : pb <= pc ? b : c][f];
      un[y * st + i] = (v + pred) & 255;
    }
  }
  if (n === 4) return { w, h, px: un };
  const px = Buffer.alloc(w * h * 4, 255);
  for (let i = 0; i < w * h; i++) un.copy(px, i * 4, i * 3, i * 3 + 3);
  return { w, h, px };
}

/** Space above and below the glyph's ink in a screenshot at `k` device pixels per CSS px: the
 * first and last rows whose color differs from the control's fill, in CSS px. */
export function inkInsets(png, boxes, k) {
  const { w: PW, px } = decodePng(png);
  return boxes.map(({ s, x, y, w, h, x0, x1 }) => {
    const X = Math.round(x * k), Y = Math.round(y * k), H = Math.round(h * k);
    const at = (r, col) => ((Y + r) * PW + X + col) * 4;
    const m = at(H >> 1, 3 * k);
    let top = -1, bot = -1;
    for (let r = 2 * k; r < H - 2 * k; r++) {
      for (let col = Math.ceil(x0 * k) + 1; col < Math.floor(x1 * k) - 1; col++) {
        const i = at(r, col);
        const diff = Math.abs(px[i] - px[m]) + Math.abs(px[i + 1] - px[m + 1])
          + Math.abs(px[i + 2] - px[m + 2]);
        if (diff > 120) {
          if (top < 0) top = r;
          bot = r;
          break;
        }
      }
    }
    if (px[m + 3] === 0) return { cell: s, blank: true };
    return top < 0 ? { cell: s, top: NaN, bottom: NaN }
      : { cell: s, top: top / k, bottom: (H - 1 - bot) / k };
  });
}

/** Left and right gap between the popup and its title field, against the Notes label's. */
export function titleInsets() {
  const box = document.querySelector('.ov.send').getBoundingClientRect();
  const t = document.querySelector('.send-title').getBoundingClientRect();
  const l = document.querySelector('.send-lab').getBoundingClientRect();
  const pad = parseFloat(getComputedStyle(document.querySelector('.send-lab')).paddingLeft);
  const want = pad + (l.left - box.left);
  return [
    { cell: '.send-title left', top: t.left - box.left, bottom: want },
    { cell: '.send-title right', top: box.right - t.right, bottom: want },
  ];
}

/** @param {any} env what browser-check.mjs set up (bin, r, browsers, context, ...) */
export function checks(env) {
  const { bin, r, browsers, context } = env;
  const judge = (where, rows) => {
    const bad = rows.filter((x) => !(Math.abs(x.top - x.bottom) <= (x.tol ?? 1)));
    if (!rows.length) r.fail(`${where}: nothing measured`);
    else if (bad.length) {
      const say = (x) => `${x.cell} top ${x.top.toFixed(2)} bottom ${x.bottom.toFixed(2)}`;
      r.fail(`${where}: ${bad.map(say).join('; ')}`);
    }
    else if (rows.some((x) => x.tol !== undefined)) {
      const say = (x) => `${x.cell} ${x.top.toFixed(2)}/${x.bottom.toFixed(2)}`;
      r.ok(`${where}: centered (${rows.map(say).join(', ')})`);
    }
    else r.ok(`${where}: top and bottom space equal`);
  };
  /** The label ink of each footer control, as the mean of SHIFTS screenshots with the popup
   * moved by a fraction of a pixel each time: a browser snaps the text line to whole pixels, so
   * one position can hide or invent a 1 px offset. 4x, so a quarter pixel is one row. */
  async function labelInk(p) {
    const sels = await p.evaluate(inkProbes);
    const sum = new Map();
    for (let i = 0; i < SHIFTS; i++) {
      await p.evaluate((dy) => {
        /** @type {HTMLElement} */ (document.querySelector('.ov.send')).style.marginTop = `${dy}px`;
      }, i / SHIFTS);
      const boxes = await p.evaluate(inkBoxes, sels);
      for (const x of inkInsets(await p.screenshot({ timeout: 60000 }), boxes, INK_SCALE)) {
        if (x.blank) return [{ cell: `${x.cell} screenshot is blank`, top: 0, bottom: 9 }];
        const t = sum.get(x.cell) || { cell: x.cell, top: 0, bottom: 0 };
        t.tol = INK_TOL;
        t.top += x.top / SHIFTS;
        t.bottom += x.bottom / SHIFTS;
        sum.set(x.cell, t);
      }
    }
    return [...sum.values()];
  }
  async function insetBalance() {
    const srv = await serve(bin, oneLineRepo());
    try {
      for (const name of browsers) {
        const { b, ctx } = await context(name, { deviceScaleFactor: INK_SCALE });
        const p = await ctx.newPage();
        for (const scheme of ['light', 'dark']) {
          await p.emulateMedia({ colorScheme: scheme });
          await p.setViewportSize({ width: 1200, height: 800 });
          await p.goto(srv.base + '/blame/HEAD/one.txt', { waitUntil: 'load' });
          await p.evaluate(() => document.fonts.ready);
          judge(`inset-balance ${name} ${scheme} blame row`, await p.evaluate(blameInsets));
          await p.goto(srv.base + '/blob/HEAD/one.txt', { waitUntil: 'load' });
          await p.waitForTimeout(300);
          await p.keyboard.press('.');
          await p.keyboard.press('s');
          await p.waitForSelector('.send-foot');
          await p.waitForTimeout(300);
          const foot = await p.evaluate(footInsets, '.send-foot');
          judge(`inset-balance ${name} ${scheme} send actions`, foot);
          judge(`inset-balance ${name} ${scheme} title inset`, await p.evaluate(titleInsets));
          judge(`inset-balance ${name} ${scheme} label ink`, await labelInk(p));
        }
        await b.close();
      }
    } finally {
      srv.stop();
    }
  }
  return { insetBalance };
}
