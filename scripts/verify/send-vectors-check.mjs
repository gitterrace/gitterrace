#!/usr/bin/env node
// Runs every shared send vector through the JS encoder in send.js, every source-line vector
// through its sourceLine(), and the title rule through titleOf(). The review vectors must hold
// the same source lines the JS builds, and no status says a finding was dealt with.
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
// Review vectors: one `source:` line per finding, equal to sourceLine(), with a closed set of
// status words that say what became of the code and never that a finding was dealt with.
const STATUS = ['untouched', 'lines changed', 'lines edited', 'file changed', 'file gone'];
const r = data('review-vectors.json');
const asLines = (l) => (l ? { start: l[0], end: l[1] } : null);
for (const t of r) {
  const src = t.markdown.split('\n').map((l) => l.trim()).filter((l) => l.startsWith('source: '));
  const tag = `review ${t.name}`;
  if (src.length !== t.findings.length) fail(`${tag}: ${src.length} source lines`);
  t.findings.forEach((f, i) => {
    const want = sourceLine(t.repo, f.sha, f.path, asLines(f.lines));
    if (src[i] !== want) fail(`${tag} #${f.n}: ${src[i]} != ${want}`);
    if (!STATUS.includes(f.status)) fail(`${tag} #${f.n}: status ${f.status}`);
    if (/resolved|fixed/i.test(f.status)) fail(`${tag} #${f.n}: status claims it was dealt with`);
  });
}
console.log(`${v.length} send, ${s.length} source, ${r.length} review, ${titles.length} titles, ${bad} failed`);
process.exit(bad ? 1 : 0);
