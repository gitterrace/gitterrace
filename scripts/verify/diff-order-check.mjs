#!/usr/bin/env node
// The commit page gets its Find box from diffs-find.js and its Viewed count from
// diffs-viewed.js. app.js loads both on demand, so either can run first. This runs them in both
// orders on a tiny DOM and fails unless the page reads the same: box, count, then the files.
//
//   node scripts/verify/diff-order-check.mjs
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { runInNewContext } from 'node:vm';
import { root } from '../lib/browser-lib.mjs';

class Node {
  constructor(tag) {
    this.tag = tag;
    this.cls = [];
    this.dataset = {};
    this.kids = [];
    this.parentNode = null;
    this.attrs = {};
  }
  set className(v) { this.cls = v.split(/\s+/).filter(Boolean); }
  get className() { return this.cls.join(' '); }
  set textContent(v) { this.kids = []; this.text = v; }
  setAttribute(k, v) { this.attrs[k] = v; }
  addEventListener() {}
  appendChild(c) { return this.insertBefore(c, null); }
  insertBefore(c, ref) {
    if (c.parentNode) c.remove();
    const i = ref ? this.kids.indexOf(ref) : this.kids.length;
    this.kids.splice(i, 0, c);
    c.parentNode = this;
    return c;
  }
  remove() {
    this.parentNode.kids.splice(this.parentNode.kids.indexOf(this), 1);
    this.parentNode = null;
  }
  get nextSibling() { const k = this.parentNode.kids; return k[k.indexOf(this) + 1] || null; }
  is(sel) {
    return sel.split(',').some((s) => {
      const m = /^\s*(\w+)?((?:\.[\w-]+)*)(\[data-path\])?\s*$/.exec(s);
      const need = m ? m[2].split('.').filter(Boolean) : [];
      return m && (!m[1] || m[1] === this.tag) && need.every((c) => this.cls.includes(c)) &&
        (!m[3] || this.dataset.path !== undefined);
    });
  }
  all(sel) {
    return this.kids.flatMap((k) => [
      ...(k.is && k.is(sel) ? [k] : []),
      ...(k.all ? k.all(sel) : []),
    ]);
  }
  querySelector(sel) { return this.all(sel)[0] || null; }
  querySelectorAll(sel) { return this.all(sel); }
}

function page(order) {
  const body = new Node('body');
  const files = ['a', 'b'].map((p) => {
    const s = new Node('section');
    s.className = 'file';
    s.dataset.path = p;
    s.dataset.h = 'h';
    s.appendChild(new Node('header'));
    body.appendChild(s);
    return s;
  });
  const el = (tag, cls, text) => {
    const n = new Node(tag);
    n.className = cls || '';
    if (text) n.text = text;
    return n;
  };
  const sandbox = {
    window: { gtNav: { B: '', el } },
    location: { pathname: '/commit/abc', search: '' },
    localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    document: {
      querySelector: (s) => body.querySelector(s),
      querySelectorAll: (s) => body.querySelectorAll(s),
      addEventListener() {},
      createTextNode: (t) => ({ text: t }),
    },
  };
  for (const name of order) {
    const file = join(root, `crates/gitterrace-web/static/${name}.js`);
    runInNewContext(readFileSync(file, 'utf8'), sandbox);
  }
  const mark = (c) => c === 'diff-find' || c === 'viewed-count';
  const name = (k) => (k.cls.includes('file') ? `file:${k.dataset.path}` : k.cls.find(mark));
  return body.kids.map(name);
}

const want = ['diff-find', 'viewed-count', 'file:a', 'file:b'].join();
let bad = 0;
for (const order of [['diffs-find', 'diffs-viewed'], ['diffs-viewed', 'diffs-find']]) {
  const got = page(order).join();
  if (got !== want) { bad++; console.error(`FAIL ${order.join(' then ')}: ${got}`); }
}
if (bad) process.exit(1);
console.log('diff page order ok in both script orders');
