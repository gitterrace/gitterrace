// The agent-trailers check. Imported by browser-check.mjs, not run on its own.
//
// A commit with an Agent-Session trailer shows the session and its resume command on the
// commit page, in each browser.
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { eachBrowser, serve, sh } from '../../lib/browser-lib.mjs';

/** @param {any} env what browser-check.mjs set up (r, browsers, bin, cfg, context) */
export function checks(env) {
  const { r, browsers, bin, cfg, context } = env;

  async function agentTrailers() {
    const dir = mkdtempSync(join(tmpdir(), 'gt-agent-'));
    sh(dir, 'git', 'init', '-q', '-b', 'main');
    writeFileSync(join(dir, 'a.txt'), 'one\n');
    sh(dir, 'git', 'add', '-A');
    sh(dir, 'git', 'commit', '-q', '-m', 'feat: x', '-m', 'Agent: claude\nAgent-Session: abc-123');
    const srv = await serve(bin, dir, cfg);
    try {
      const id = sh(dir, 'git', 'rev-parse', 'HEAD');
      await eachBrowser(browsers, context, async (ctx, name) => {
        const p = await ctx.newPage({ viewport: { width: 1440, height: 900 } });
        await p.goto(`${srv.base}/commit/${id}`, { waitUntil: 'load' });
        const t = await p.$eval('.agents', (e) => e.textContent).catch(() => '');
        if (t.includes('claude --resume abc-123')) r.ok(`agent-trailers ${name}: resume shown`);
        else r.fail(`agent-trailers ${name}: got "${t}"`);
        await p.close();
      });
    } catch (e) {
      r.fail(`agent-trailers: ${e && e.stack}`);
    } finally {
      srv.stop();
      rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    }
  }
  return { agentTrailers };
}
