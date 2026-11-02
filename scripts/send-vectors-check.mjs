#!/usr/bin/env node
// Runs every shared send vector through the JS encoder in send.js.
//
//   node scripts/send-vectors-check.mjs
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { root } from './browser-lib.mjs';

const { fill } = createRequire(import.meta.url)(join(root, 'crates/gitterrace-web/static/send.js'));
const v = JSON.parse(readFileSync(join(root, 'crates/gitterrace-actions/testdata/send-vectors.json'), 'utf8'));
let bad = 0;
for (const t of v) {
  const got = fill(t.parts, t.values);
  if (got !== t.expected) {
    bad++;
    console.error(`FAIL ${t.name}: ${got} != ${t.expected}`);
  }
}
console.log(`${v.length} vectors, ${bad} failed`);
process.exit(bad ? 1 : 0);
