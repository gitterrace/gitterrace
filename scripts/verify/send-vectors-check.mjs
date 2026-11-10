#!/usr/bin/env node
// Runs every shared send vector through the JS encoder in send.js, every source-line vector
// through its sourceLine(), and the title rule through titleOf().
//
//   node scripts/verify/send-vectors-check.mjs
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { runInNewContext } from 'node:vm';
import { root } from '../lib/browser-lib.mjs';

// send.js is a classic browser script that exports through `module` when no document exists.
// A sandbox gives it that `module`, whatever module type the package declares.
const sandbox = { module: { exports: {} }, TextEncoder };
runInNewContext(readFileSync(join(root, 'crates/gitterrace-web/static/send.js'), 'utf8'), sandbox);
const { fill, sourceLine, titleOf } = sandbox.module.exports;
const data = (f) => JSON.parse(readFileSync(join(root, 'crates/gitterrace-actions/testdata', f), 'utf8'));
let bad = 0;
const fail = (msg) => { bad++; console.error(`FAIL ${msg}`); };
const v = data('send-vectors.json');
for (const t of v) {
  const got = fill(t.parts, t.values);
  if (got !== t.expected) fail(`${t.name}: ${got} != ${t.expected}`);
}
const s = data('source-vectors.json');
for (const t of s) {
  const got = sourceLine(t.repo, t.sha, t.path, t.lines);
  if (got !== t.expected) fail(`source ${t.name}: ${got} != ${t.expected}`);
}
// The title is never the bare path: the note's first line, else file and first marked words.
const c = { kind: 'line', path: 'notes/guide/setup.md', lines: { start: 4, end: 6 }, marked: 'Install the binary with cargo and then run it' };
const titles = [
  [titleOf(c, 'Fix the install step\nmore'), 'Fix the install step'],
  [titleOf(c, ''), 'setup.md: Install the binary with cargo and then run\u2026'],
  [titleOf({ kind: 'file', path: 'a/b.rs' }, ''), 'b.rs: whole file'],
];
for (const [got, want] of titles) if (got !== want) fail(`title: ${JSON.stringify(got)} != ${JSON.stringify(want)}`);
for (const p of ['notes/guide/setup.md', 'a/b.rs']) {
  const t = titleOf({ kind: 'line', path: p, lines: { start: 1, end: 1 } }, '');
  if (t === p || t.startsWith(p + ':L')) fail(`title is the bare path: ${t}`);
}
console.log(`${v.length} send vectors, ${s.length} source vectors, ${titles.length} titles, ${bad} failed`);
process.exit(bad ? 1 : 0);
