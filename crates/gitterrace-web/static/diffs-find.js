// Find in the whole diff. The server searches every file of the commit or working tree tab and
// returns, per matching line, the ranges of the match (diff_find.rs), so this only paints them
// with the CSS Custom Highlight API: no element is added to a line. The bar is server rendered
// (.difftools); this binds to it. Keys: f focuses the box, Enter or n steps on, Shift+Enter or
// N back, Esc clears. "/" stays the global search. app.js loads this only on a diff page.
(function () {
  "use strict";
  var base = window.gtNav.B;
  var path = location.pathname.slice(base.length);
  var m = /^\/commit\/([^/]+)$/.exec(path), url;
  if (m) url = base + "/diff-find/" + m[1];
  else if (path === "/worktree") url = base + "/worktree/find" + location.search;
  if (!url) return;
  var HL = window.CSS && CSS.highlights && window.Highlight;
  var form, input, status, prev, next;
  var query = "", seq = 0, timer = 0, slow = 0, watch = null;
  var matches = [], truncated = false, total = 0, nfiles = 0, cur = -1;

  function lines(section) {
    var map = {};
    Array.prototype.forEach.call(section.querySelectorAll(".diff div[data-o]"), function (d) {
      var k = d.className === "del" ? "del" + d.dataset.o : d.className + d.dataset.n;
      map[k] = d;
    });
    return map;
  }
  // The range of `text` offsets [a, b) inside a line, skipping the hidden "Added, " sign.
  function span(line, a, b) {
    var w = document.createTreeWalker(line, NodeFilter.SHOW_TEXT, {
      acceptNode: function (n) {
        return n.parentNode.closest(".sign") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT;
      }
    });
    var r = document.createRange(), at = 0, n, started = false;
    while ((n = w.nextNode())) {
      var len = n.nodeValue.length;
      if (!started && a < at + len) { r.setStart(n, a - at); started = true; }
      if (started && b <= at + len) { r.setEnd(n, b - at); return r; }
      at += len;
    }
    return null;
  }
  // Resolves each match to its line and ranges where the file is loaded.
  function resolve() {
    var cache = {};
    matches.forEach(function (x) {
      x.line = null; x.rs = [];
      var s = document.querySelector('section.file[data-path="' + CSS.escape(x.path) + '"]');
      if (!s) return;
      var c = cache[x.path] || (cache[x.path] = lines(s));
      x.line = c[x.kind + x.no] || null;
      if (!x.line) return;
      x.ranges.forEach(function (r) {
        var g = span(x.line, r[0], r[1]);
        if (g) x.rs.push(g);
      });
    });
  }
  function paint() {
    if (!HL) return;
    var all = [], one = [];
    matches.forEach(function (x, i) {
      var into = i === cur ? one : all;
      into.push.apply(into, x.rs);
    });
    CSS.highlights.set("find", new Highlight(...all));
    CSS.highlights.set("find-current", new Highlight(...one));
  }
  function unpaint() {
    if (HL) { CSS.highlights.delete("find"); CSS.highlights.delete("find-current"); }
  }
  function state(s, text) {
    form.dataset.state = s;
    status.textContent = text;
    status.setAttribute("role", s === "error" ? "alert" : "status");
    prev.disabled = next.disabled = !matches.length;
  }
  function label() {
    if (!matches.length) return;
    var of = truncated ? matches.length + "+" : String(total);
    var at = cur >= 0 ? cur + 1 + " of " : "";
    state("hits", at + of + (total === 1 ? " line" : " lines") + " in " +
      nfiles + (nfiles === 1 ? " file" : " files"));
  }
  function show(i, tries) {
    var x = matches[i];
    if (!x) return;
    cur = i;
    label();
    if (!x.line) {
      var sec = document.querySelector('section.file[data-path="' + CSS.escape(x.path) + '"]');
      var b = sec && sec.querySelector("[data-load-diff]");
      if (b && (tries || 0) < 30) {
        if (!tries) b.click();
        setTimeout(function () { resolve(); if (cur === i) show(i, (tries || 0) + 1); }, 100);
      }
      paint();
      return;
    }
    paint();
    x.line.scrollIntoView({ block: "center" });
  }
  function step(d) {
    if (!matches.length) return;
    show(cur < 0 ? (d > 0 ? 0 : matches.length - 1) : (cur + d + matches.length) % matches.length);
  }
  function reset() {
    matches = []; cur = -1; total = nfiles = 0; truncated = false;
    unpaint();
    if (watch) { watch.disconnect(); watch = null; }
  }
  function got(r) {
    matches = [];
    r.files.forEach(function (f) {
      f.matches.forEach(function (x) {
        matches.push({ path: f.path, kind: x.kind, no: x.no, ranges: x.ranges });
      });
    });
    total = r.total; nfiles = r.files.length; truncated = r.truncated; cur = -1;
    if (!r.total) { state("none", "No match"); unpaint(); return; }
    resolve();
    label();
    paint();
    observe();
  }
  // A file that loads later gets its ranges too.
  function observe() {
    var main = document.querySelector("main");
    if (watch || !main || !window.MutationObserver) return;
    var t = 0;
    watch = new MutationObserver(function () {
      clearTimeout(t);
      t = setTimeout(function () { resolve(); paint(); }, 150);
    });
    watch.observe(main, { childList: true, subtree: true });
  }
  function run() {
    var mine = ++seq;
    query = input.value;
    clearTimeout(slow);
    if (!query) { reset(); state("idle", ""); return; }
    slow = setTimeout(function () { if (mine === seq) state("loading", "Searching..."); }, 150);
    var sep = url.indexOf("?") < 0 ? "?" : "&";
    fetch(url + sep + "q=" + encodeURIComponent(query)).then(function (r) {
      if (!r.ok) throw r;
      return r.json();
    }).then(function (r) {
      if (mine !== seq) return;
      clearTimeout(slow);
      reset();
      got(r);
    }, function () {
      if (mine !== seq) return;
      clearTimeout(slow);
      reset();
      state("error", "Search failed. ");
      var b = window.gtNav.el("button", "ghost", "Retry");
      b.type = "button";
      b.addEventListener("click", run);
      status.appendChild(b);
    });
  }
  function clear() { input.value = ""; run(); }
  function bind() {
    var f = document.querySelector("form.difftools");
    if (!f || f === form) return;
    form = f;
    input = f.querySelector("input[type=search]");
    status = f.querySelector(".find-status");
    prev = f.querySelector(".dt-prev");
    next = f.querySelector(".dt-next");
    input.addEventListener("input", function () {
      clearTimeout(timer);
      timer = setTimeout(run, 250);
    });
    f.addEventListener("submit", function (e) { e.preventDefault(); });
    prev.addEventListener("click", function () { step(-1); });
    next.addEventListener("click", function () { step(1); });
    input.addEventListener("keydown", function (e) {
      if (e.key === "Enter") {
        e.preventDefault();
        if (input.value !== query) run(); else step(e.shiftKey ? -1 : 1);
      }
      else if (e.key === "Escape") {
        e.stopPropagation();
        if (input.value) clear(); else input.blur();
      }
    });
    if (query) { input.value = query; run(); }
    // The sticky file headers sit under the bar, whatever height it wraps to.
    if (window.ResizeObserver) {
      new ResizeObserver(function () {
        document.documentElement.style.setProperty("--dt-h", f.offsetHeight + "px");
      }).observe(f);
    }
  }
  document.addEventListener("keydown", function (e) {
    if (!form || e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented) return;
    if (window.gtTyping && window.gtTyping(e.target)) return;
    var shortcutsOff = window.gtKeysOff && window.gtKeysOff();
    if (shortcutsOff) return;
    if (window.gtNav.ov && window.gtNav.ov.name) return;
    if (e.key === "f") { e.preventDefault(); input.focus(); input.select(); }
    else if ((e.key === "n" || e.key === "N") && query) {
      e.preventDefault();
      step(e.key === "n" ? 1 : -1);
    }
    else if (e.key === "Escape" && query) clear();
  });
  bind();
  document.addEventListener("gt:live-refreshed", function () { form = null; bind(); });
})();
