// One button vocabulary, checked where readers meet it: on the log, a file, its blame, a

import { eachBrowser, lum, frames } from '../../lib/browser-lib.mjs';
// Markdown file, settings (with a list's Add pressed) and the Send to window, in light and dark.
// Every visible <button> looks at rest like a fresh button with the same variant classes
// (secondary, .primary, .ghost, .danger) in the same spot, so no component paints its own; one
// button of each variant per page changes its computed look on hover, on press and on keyboard
// focus; and text keeps 4.5:1 against what is behind it in every state. Imported by
// browser-check.mjs, not run on its own.

const VARIANTS = ['primary', 'ghost', 'danger'];

/** Pages, each with what to do before the buttons are read.
 * @type {Array<[string, string, string?, ((p: any) => Promise<void>)?]>} */
const PAGES = [
  ['log', '/log/HEAD'],
  ['file', '/blob/HEAD/a.txt'],
  ['markdown', '/blob/HEAD/README.md'],
  ['blame', '/blame/HEAD/a.txt'],
  ['settings', '/_settings', 'origin', async (p) => {
    // Something to save, so Save (the primary) is enabled.
    await p.fill('#f-watch\\.rescan_secs', '61');
  }],
  ['settings add', '/_settings?f=actions', 'origin', async (p) => {
    await p.click('[data-add]');
  }],
  ['send to', '/blob/HEAD/a.txt', '', async (p) => {
    await p.keyboard.press('.');
    await p.keyboard.press('s');
    await p.waitForSelector('.ov.send');
  }],
];

/** In the page: tag every visible button and report the ones that do not look like their
 * variant at rest. The reference is a fresh button with the same variant classes inserted
 * next to it, so inherited color and font match. */
function survey(variants) {
  const look = (e) => {
    const s = getComputedStyle(e);
    return [s.backgroundColor, s.borderTopColor, s.borderTopWidth, s.color].join(' | ');
  };
  const shown = (e) => {
    const q = e.getBoundingClientRect(), s = getComputedStyle(e);
    return q.width > 0 && q.height > 0 && s.visibility !== 'hidden' && s.opacity !== '0';
  };
  const out = { bad: [], picks: {} };
  let n = 0;
  // An open window covers the page: its buttons are the ones a reader can reach.
  const scope = document.querySelector('.ov') || document;
  for (const b of scope.querySelectorAll('button')) {
    if (!shown(b) || b.closest('.segs') || b.matches('.copied')) continue;
    const variant = variants.find((v) => b.classList.contains(v)) || 'secondary';
    const ref = document.createElement('button');
    for (const v of variants) if (b.classList.contains(v)) ref.classList.add(v);
    if (b.classList.contains('icon')) ref.classList.add('icon');
    ref.disabled = b.disabled;
    ref.type = 'button';
    b.after(ref);
    const want = look(ref), got = look(b);
    ref.remove();
    const text = (b.textContent || b.getAttribute('aria-label') || '').trim().slice(0, 20);
    const name = `${b.className || '(base)'} "${text}"`;
    if (got !== want) out.bad.push(`${name} as ${variant}: ${got} vs ${want}`);
    if (!b.disabled && !out.picks[variant]) {
      b.dataset.btnPick = String(++n);
      out.picks[variant] = { id: n, name };
    }
  }
  return out;
}

/** In the page: the button's look, its text color and the color behind it as sRGB 0-255. */
function read(id) {
  const b = /** @type {HTMLElement} */ (document.querySelector(`[data-btn-pick="${id}"]`));
  const cv = document.createElement('canvas');
  cv.width = cv.height = 1;
  const g = /** @type {CanvasRenderingContext2D} */ (
    cv.getContext('2d', { willReadFrequently: true }));
  const rgba = (c) => {
    g.clearRect(0, 0, 1, 1);
    g.fillStyle = '#000';
    g.fillStyle = c;
    g.fillRect(0, 0, 1, 1);
    return [...g.getImageData(0, 0, 1, 1).data];
  };
  // The fill behind the text: the button's own, laid over its ancestors' until one is opaque.
  const layers = [];
  for (let e = b; e; e = e.parentElement) {
    const c = rgba(getComputedStyle(e).backgroundColor);
    if (c[3] > 0) layers.push(c);
    if (c[3] === 255) break;
  }
  let bg = [255, 255, 255];
  for (const c of layers.reverse()) bg = bg.map((v, i) => v + (c[i] - v) * (c[3] / 255));
  const s = getComputedStyle(b);
  return {
    look: [s.backgroundColor, s.borderTopColor, s.color, s.outlineStyle, s.outlineWidth,
      s.boxShadow].join(' | '),
    fg: rgba(s.color), bg,
    ring: s.outlineStyle !== 'none' && parseFloat(s.outlineWidth) >= 2,
  };
}

/** WCAG contrast of text `fg` (alpha blended over `bg`) on `bg`. */
export function contrast(fg, bg) {
  const a = (fg[3] ?? 255) / 255;
  const t = bg.map((v, i) => v + (fg[i] - v) * a);
  const [x, y] = [lum(t), lum(bg)].sort((m, n) => n - m);
  return (x + 0.05) / (y + 0.05);
}

/** @param {any} env what browser-check.mjs set up (srv, r, browsers, context, ...) */
export function checks(env) {
  const { srv, r, browsers, context } = env;

  async function buttons() {
    await eachBrowser(browsers, context, async (ctx, name) => {
      for (const scheme of ['light', 'dark']) for (const [label, path, where, before] of PAGES) {
        const p = await ctx.newPage();
        const at = `buttons ${name} ${scheme} ${label}`;
        try {
          // Reduced motion turns the transitions off, so a state reads at once.
          await p.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
          await p.setViewportSize({ width: 1280, height: 800 });
          await p.goto((where === 'origin' ? srv.origin : srv.base) + path, { waitUntil: 'load' });
          await p.evaluate(() => document.fonts.ready);
          await frames(p);
          if (before) await before(p);
          await p.mouse.move(1, 799);
          const s = await p.evaluate(survey, VARIANTS);
          if (s.bad.length) r.fail(`${at}: not the shared look: ${s.bad.join('; ')}`);
          else r.ok(`${at}: every button has its variant's look`);
          for (const [variant, pick] of Object.entries(s.picks)) {
            const sel = `[data-btn-pick="${pick.id}"]`;
            const bad = [];
            await p.mouse.move(1, 799);
            await p.evaluate(() => {
              if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
            });
            const rest = await p.evaluate(read, pick.id);
            await p.hover(sel, { timeout: 5000 });
            const hover = await p.evaluate(read, pick.id);
            await p.mouse.down();
            const press = await p.evaluate(read, pick.id);
            // Off the button before letting go, so the press never clicks (Send, Remove).
            await p.mouse.move(1, 799);
            await p.mouse.up();
            // By keyboard, so every engine treats it as focus-visible: focus the button, step
            // back one stop and Tab onto it again.
            await p.focus(sel);
            await p.keyboard.press('Shift+Tab');
            await p.keyboard.press('Tab');
            const on = await p.evaluate(
              (s) => document.activeElement === document.querySelector(s), sel);
            if (!on) bad.push('Tab does not reach it');
            const focus = await p.evaluate(read, pick.id);
            if (hover.look === rest.look) bad.push('hover looks like rest');
            if (press.look === rest.look || press.look === hover.look) {
              bad.push('press looks like rest or hover');
            }
            if (!focus.ring) bad.push('no focus ring');
            for (const [state, v] of Object.entries({ rest, hover, press, focus })) {
              const c = contrast(v.fg, v.bg);
              if (c < 4.5) bad.push(`${state} text ${c.toFixed(2)}:1`);
            }
            if (bad.length) r.fail(`${at}: ${variant} ${pick.name}: ${bad.join(', ')}`);
            else r.ok(`${at}: ${variant} states differ, text 4.5:1 in each`);
          }
        } catch (e) { r.fail(`${at}: ${e.message.split('\n')[0]}`); }
        await p.close();
      }
    });
  }
  return { buttons };
}
