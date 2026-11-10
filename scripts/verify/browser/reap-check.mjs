#!/usr/bin/env node
// Guard: a browser check killed mid-run, or one that throws, leaves no test server running. A
// `finally` around the checks never runs on a signal, so the server is stopped by the exit hook
// in lib/browser-lib.mjs. This starts a child that serves and then hangs, ends it each way and
// counts the servers left on its port.
//
//   node scripts/verify/browser/reap-check.mjs [--bin target/debug/git-terrace]
import { spawn, execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { root } from '../../lib/browser-lib.mjs';

const at = process.argv.indexOf('--bin');
const bin = at > 0 ? process.argv[at + 1] : join(root, 'target/debug/git-terrace');
const lib = join(root, 'scripts/lib/browser-lib.mjs');

/** A child that serves a fresh repository, prints its origin, then hangs or throws. */
const child = (end) => `
  import { serve, makeRepo } from ${JSON.stringify(lib)};
  const srv = await serve(${JSON.stringify(bin)}, makeRepo());
  console.log('UP ' + srv.origin);
  try {
    const boom = () => { throw new Error('a check threw'); };
    if (${JSON.stringify(end)} === 'throw') setTimeout(boom, 50);
    await new Promise(() => {});
  } finally { srv.stop(); }
`;

/** Servers still answering to `origin`'s port, by their command line. */
function left(origin) {
  const port = origin.split(':').pop();
  try {
    const out = execFileSync('pgrep', ['-f', `serve --addr 127.0.0.1:${port} `]);
    return out.toString().trim().split('\n').length;
  } catch { return 0; }
}

async function run(end) {
  const args = ['--input-type=module', '-e', child(end)];
  const c = spawn(process.execPath, args, { stdio: ['ignore', 'pipe', 'ignore'] });
  const exited = new Promise((ok) => c.on('exit', ok));
  const origin = await new Promise((ok, no) => {
    c.stdout.on('data', (d) => { const m = /UP (\S+)/.exec(String(d)); if (m) ok(m[1]); });
    exited.then(() => no(new Error(`${end}: the child ended before serving`)));
  });
  if (end !== 'throw') c.kill(end);
  await exited;
  // The server gets its signal as the child exits; the OS may take a moment to reap it. A server
  // nobody signals is still there after this.
  for (let i = 0; i < 50 && left(origin); i++) await new Promise((ok) => setTimeout(ok, 100));
  const n = left(origin);
  if (n) execFileSync('pkill', ['-f', `serve --addr ${origin.replace('http://', '')} `]);
  return n;
}

let bad = 0;
for (const end of ['SIGTERM', 'SIGINT', 'SIGHUP', 'throw']) {
  const n = await run(end);
  if (n) { bad++; console.log(`FAIL ${end}: ${n} test server(s) left running`); }
  else console.log(`ok   ${end}: no test server left`);
}
process.exit(bad ? 1 : 0);
