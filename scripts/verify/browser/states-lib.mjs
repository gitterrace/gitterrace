// Empty and error states: the not-found page is the app shell with the requested path and next
// steps, a bare error page keeps the header, a clean working tree and an empty ref group use the
// one empty state, and a repository whose build failed says why. Imported by browser-check.mjs.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { sweep, makeRepo, serve, reloadUntil } from '../../lib/browser-lib.mjs';

export function checks({ srv, r, browsers, context, bin }) {
  async function notFoundStates() {
    const sw = { browsers, context, widths: [1440, 390], height: 800 };
    await sweep(sw, async (p, { name, scheme, width }) => {
      const where = `not-found ${name} ${scheme} ${width}`;
      const res = await p.goto(srv.base + '/nothing-here', { waitUntil: 'load' });
      const bad = [];
      if (res.status() !== 404) bad.push(`status ${res.status()}`);
      const m = await p.evaluate(() => ({
        header: !!document.querySelector('header.top .brand[aria-label]'),
        h1: document.querySelector('h1')?.textContent.trim(),
        tag: document.querySelector('.ph-tag')?.textContent.trim(),
        path: document.querySelector('.lost-path code')?.textContent,
        steps: [...document.querySelectorAll('.lost .steps a.btn')]
          .map((a) => a.textContent.trim()),
        align: getComputedStyle(document.querySelector('.lost')).textAlign,
      }));
      if (!m.header) bad.push('no app header');
      if (!m.path || !m.path.includes('nothing-here')) bad.push('the requested path is not shown');
      if (m.steps.join() !== 'Code,Commits,Search') bad.push(`steps ${m.steps.join()}`);
      if (/^nothing here\.?$/i.test(m.tag || '')) bad.push('the tagline repeats the title');
      if (m.align === 'center') bad.push('the card is centred');
      r.verdict(where, bad);
      // A bare error page (an address outside any repository) keeps the shell too.
      const o = await p.goto(srv.origin + '/r/nope-not-served/x', { waitUntil: 'load' });
      const e = await p.evaluate(() => ({
        header: !!document.querySelector('header.top .brand'),
        styled: getComputedStyle(document.querySelector('h1')).fontFamily.includes('Fraunces'),
      }));
      const bad2 = [];
      if (o.status() !== 404) bad2.push(`status ${o.status()}`);
      if (!e.header) bad2.push('no header on the bare error page');
      if (!e.styled) bad2.push('the title is unstyled');
      r.verdict(`error-page ${name} ${scheme} ${width}`, bad2);
    });
  }
  // A repository whose cache a newer build owns fails its index build: the list says why.
  async function failedRepo() {
    const a = makeRepo(), b = makeRepo();
    mkdirSync(join(b, '.git/gitterrace'), { recursive: true });
    writeFileSync(join(b, '.git/gitterrace/VERSION'), 'v99\n');
    const srv = await serve(bin, [a, b]);
    const origin = srv.origin;
    try {
      for (const name of browsers) {
        const { b: br, ctx } = await context(name);
        const p = await ctx.newPage();
        let why = '';
        await reloadUntil(p, async () => {
          why = await p.evaluate(() => document.querySelector('.reporow .why')?.textContent || '');
          return why;
        }, origin + '/', 'the repository list to say why a build failed');
        const ok = /failed/i.test(why) && why.length > 20;
        const left = ok ? [] : [`no reason on the list (${JSON.stringify(why)})`];
        r.verdict(`failed-repo ${name}`, left);
        await br.close();
      }
    } finally {
      srv.stop();
    }
  }
  return { notFoundStates, failedRepo };
}
