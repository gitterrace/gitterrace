#!/usr/bin/env python3
"""Run every shared send vector through the JS encoder in send.js (via node)."""
import json, pathlib, subprocess, sys

root = pathlib.Path(__file__).resolve().parent.parent
vectors = root / "crates/gitterrace-actions/testdata/send-vectors.json"
js = root / "crates/gitterrace-web/static/send.js"
if not vectors.exists():
    sys.exit(f"missing {vectors}")
runner = """
const {fill} = require(process.argv[1]);
const v = JSON.parse(require('fs').readFileSync(process.argv[2], 'utf8'));
let bad = 0;
for (const t of v) {
  const got = fill(t.parts, t.values);
  if (got !== t.expected) { bad++; console.error('FAIL ' + t.name + ': ' + got + ' != ' + t.expected); }
}
console.log(v.length + ' vectors, ' + bad + ' failed');
process.exit(bad ? 1 : 0);
"""
sys.exit(subprocess.run(["node", "-e", runner, str(js), str(vectors)]).returncode)
