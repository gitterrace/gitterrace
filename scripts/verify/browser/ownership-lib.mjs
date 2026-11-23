// The knowledge risk section of the ownership page and a person's page. Imported by
// browser-check.mjs, not run alone.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { eachBrowser, poll, sh } from '../../lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (dir, srv, r, browsers, context, ...) */
export function checks(env) {
  const { dir, srv, r, browsers, context } = env;

  // Two people: Ann wrote `old/` long ago and has been quiet since, Bob keeps committing. The
  // section marks `old` as at risk, a click on its owner opens Ann's page with her file, a click
  // on a directory drills one level and "Up one level" goes back.
  async function ownershipPersonDrill() {
    mkdirSync(join(dir, 'old/deep'), { recursive: true });
    mkdirSync(join(dir, 'live'), { recursive: true });
    const as = (who, date) => ({
      GIT_AUTHOR_NAME: who, GIT_AUTHOR_EMAIL: `${who.toLowerCase()}@t`, GIT_AUTHOR_DATE: date,
      GIT_COMMITTER_NAME: who, GIT_COMMITTER_EMAIL: `${who.toLowerCase()}@t`,
      GIT_COMMITTER_DATE: date,
    });
    const commit = (who, date, subject) => {
      sh(dir, 'git', 'add', '-A');
      sh(dir, 'env', ...Object.entries(as(who, date)).map(([k, v]) => `${k}=${v}`),
        'git', 'commit', '-q', '-m', subject);
    };
    writeFileSync(join(dir, 'old/deep/x.txt'), 'x\n'.repeat(80));
    writeFileSync(join(dir, 'old/y.txt'), 'y\n'.repeat(40));
    commit('Ann', '2015-03-01T12:00:00Z', 'feat(old): the old code');
    writeFileSync(join(dir, 'live/z.txt'), 'z\n'.repeat(60));
    commit('Bob', '2026-11-01T12:00:00Z', 'feat(live): the live code');
    await eachBrowser(browsers, context, async (ctx, name) => {
      const where = `ownership-person-drill ${name}`;
      const p = await ctx.newPage();
      await p.setViewportSize({ width: 1400, height: 900 });
      let row = '';
      await poll(async () => {
        await p.goto(`${srv.base}/stats/ownership?period=all`, { waitUntil: 'load' });
        row = (await p.$$eval('tr[data-risk-row]', (t) => t.map((x) => x.textContent)))
          .find((t) => t.includes('old')) || '';
        return row !== '';
      }, 'the knowledge risk rows');
      const bad = [];
      if (!row.includes('At risk')) bad.push(`old is not marked at risk: ${row}`);
      await Promise.all([
        p.waitForNavigation({ waitUntil: 'load' }),
        p.locator('tr[data-risk-row] a[data-drill]', { hasText: 'old' }).first().click(),
      ]);
      if (!p.url().includes('drill=old')) bad.push(`drill went to ${p.url()}`);
      const kids = await p.$$eval('tr[data-risk-row]', (t) => t.map((x) => x.textContent));
      if (!kids.some((t) => t.includes('deep'))) bad.push(`no child directory: ${kids}`);
      await Promise.all([
        p.waitForNavigation({ waitUntil: 'load' }),
        p.locator('[data-risk-up]').first().click(),
      ]);
      if (p.url().includes('drill=')) bad.push(`up stayed in ${p.url()}`);
      await Promise.all([
        p.waitForNavigation({ waitUntil: 'load' }),
        p.locator('a[data-person]', { hasText: 'Ann' }).first().click(),
      ]);
      if (!p.url().includes('/stats/ownership/Ann')) bad.push(`owner link went to ${p.url()}`);
      const owned = await p.locator('#owned ~ .card').first().textContent();
      if (!owned || !owned.includes('x.txt')) bad.push(`Ann's page lacks her file: ${owned}`);
      // The page has settled once its animations have run: look, then keep a picture.
      await poll(() => p.evaluate(() => document.getAnimations().length === 0), 'animations');
      if (process.env.SHOT_DIR) {
        await p.screenshot({ path: join(process.env.SHOT_DIR, `ownership-person-${name}.png`) });
      }
      r.verdict(where, bad, 'at risk, drill, up and the owner page');
      await p.close();
    });
  }

  return { ownershipPersonDrill };
}
