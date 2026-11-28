#!/usr/bin/env node
// Guard: a same-origin 4xx or 5xx fails a browser case instead of passing as a silent no-op
// (lib/browser-lib.mjs `guard`). Red: a page that plants a real 404 and a fulfilled 403 must be
// reported. Green: the same responses listed in advance, a 200 page and a foreign-origin
// error must not be.
//
//   node scripts/verify/browser/err-check.mjs [--bin target/debug/git-terrace]
import { join, resolve } from 'node:path';
import { root, loadPlaywright, launch, makeRepo, serve, guard } from '../../lib/browser-lib.mjs';

const arg = (k, d) => {
  const i = process.argv.indexOf(k);
  return i < 0 ? d : process.argv[i + 1];
};
const bin = resolve(arg('--bin', join(root, 'target/debug/git-terrace')));
const name = arg('--browser', 'chromium');

const pw = await loadPlaywright([name]);
const srv = await serve(bin, makeRepo());
const b = await launch(pw, name);
let bad = 0;
const say = (ok, m) => { if (!ok) bad++; console.log(ok ? 'ok  ' : 'FAIL', m); };

/** Loads the repository page, then asks for a missing path and a planted 403 from the page. */
async function run(allow) {
  guard.begin(allow);
  const p = await b.newPage();
  await p.route('**/planted-403', (q) => q.fulfill({ status: 403, body: 'no' }));
  await p.goto(srv.base + '/', { waitUntil: 'load' });
  await p.evaluate(async (base) => {
    await fetch(`${base}/missing-for-guard`);
    await fetch(`${base}/planted-403`);
  }, srv.base);
  await p.close();
  return guard.take();
}

try {
  const red = await run([]);
  say(red.length === 2 && red.some((f) => f.startsWith('404 '))
    && red.some((f) => f.startsWith('403 ')), `err-guard red: unexpected errors reported (${red})`);
  const green = await run([{ status: 404, path: /missing-for-guard$/ },
    { status: 403, path: /planted-403$/ }]);
  say(green.length === 0, `err-guard green: expected errors pass (${green})`);
  const part = await run([{ status: 404, path: /missing-for-guard$/ }]);
  say(part.length === 1 && part[0].startsWith('403 '),
    `err-guard narrow: an allowance covers only its own status and path (${part})`);
} finally {
  await b.close();
  srv.stop();
}
process.exit(bad ? 1 : 0);
