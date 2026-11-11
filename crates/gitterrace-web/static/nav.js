// Keyboard shortcuts, file finder, line selection and copy buttons.
// Vanilla JS, no dependencies. Everything degrades to plain links without it.
(function () {
  "use strict";
  var body = document.body;
  var rev = body.dataset.rev || "HEAD";
  var commit = body.dataset.commit || "";
  var revEnc = encodeURIComponent(rev);
  // Repository prefix ("" or "/r/NAME"); every in-app URL starts with it.
  var meta = document.querySelector('meta[name="gt-base"]');
  var B = meta ? meta.content : "";
  function local(p) { return B && p.indexOf(B + "/") === 0 ? p.slice(B.length) : p; }
  // Features the config switched off (`gt-off`): no key, entry or fetch leads to them.
  var offMeta = document.querySelector('meta[name="gt-off"]'), OFF = offMeta ? offMeta.content.split(" ") : [];
  function isOff(f) { return OFF.indexOf(f) >= 0; }
  var STATIC = document.body.dataset.static !== undefined; // an export: no server, no /api
  // The one record of which overlay is open, shared by every script that opens one (this file
  // loads first); key handlers read it instead of their own copies. `name` labels it, `back` is
  // its backdrop. open() names the box, moves focus in, keeps Tab inside a dialog (a menu closes
  // on Tab instead, as WAI-ARIA menus do), closes on Escape and gives focus back to the opener
  // (`o.from`, else whatever had focus).
  var ov = window.gtOverlay = {
    name: null, back: null, from: null, onClose: null,
    open: function (name, back, box, o) {
      ov.close();
      o = o || {};
      box.setAttribute("role", o.menu ? "menu" : "dialog");
      if (!o.menu) box.setAttribute("aria-modal", "true");
      box.setAttribute("aria-label", o.label);
      if (!box.hasAttribute("tabindex")) box.tabIndex = -1;
      box.addEventListener("keydown", function (e) {
        if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); ov.close(); return; }
        if (e.key !== "Tab") return;
        e.preventDefault();
        if (o.menu) { ov.close(); return; }
        var f = Array.prototype.filter.call(box.querySelectorAll("a[href],button,input,textarea,select,[tabindex]:not([tabindex='-1'])"), function (x) { return !x.disabled && x.offsetParent !== null; });
        if (!f.length) { box.focus(); return; }
        var i = f.indexOf(document.activeElement);
        f[e.shiftKey ? (i <= 0 ? f.length - 1 : i - 1) : (i < 0 || i === f.length - 1 ? 0 : i + 1)].focus();
      });
      ov.from = o.from || document.activeElement;
      ov.onClose = o.onClose || null;
      ov.name = name; ov.back = back;
      document.body.appendChild(back);
      box.focus();
    },
    // Puts a position:fixed box beside `anchor`, using the box's measured size. An anchor that is
    // scrolled out of view is ignored: the box goes top-center, under a pinned header or at 12vh
    // like the centered overlays. It flips above the anchor when it would overflow below, and is
    // clamped inside the viewport. It re-places on resize and on scroll (rather than closing on
    // scroll) so a box stays glued to a visible trigger and long lists keep their place; a
    // ResizeObserver covers content that arrives late.
    place: function (box, anchor) {
      var M = 8;
      function go() {
        if (!box.isConnected) return;
        var vw = document.documentElement.clientWidth, vh = window.innerHeight;
        var w = box.offsetWidth, h = box.offsetHeight;
        var r = anchor && anchor.isConnected ? anchor.getBoundingClientRect() : null;
        var seen = r && r.width + r.height > 0 && r.bottom > 0 && r.top < vh;
        seen = seen && r.right > 0 && r.left < vw;
        var left, top;
        if (seen) {
          left = r.left;
          top = r.bottom + 6;
          if (top + h > vh - M && r.top - 6 - h >= M) top = r.top - 6 - h;
        } else {
          var hd = document.querySelector("header.top");
          var pinned = hd && /^(sticky|fixed)$/.test(getComputedStyle(hd).position);
          left = (vw - w) / 2;
          top = pinned ? hd.getBoundingClientRect().bottom + 8 : vh * 0.12;
        }
        box.style.left = Math.max(M, Math.min(left, vw - w - M)) + "px";
        box.style.top = Math.max(M, Math.min(top, vh - h - M)) + "px";
      }
      go();
      window.addEventListener("resize", go);
      window.addEventListener("scroll", go, { passive: true });
      var ro = window.ResizeObserver ? new ResizeObserver(go) : null;
      if (ro) ro.observe(box);
      ov.unplace = function () {
        window.removeEventListener("resize", go);
        window.removeEventListener("scroll", go);
        if (ro) ro.disconnect();
        ov.unplace = null;
      };
    },
    unplace: null,
    close: function () {
      if (!ov.back) return;
      if (ov.unplace) ov.unplace();
      var from = ov.from, done = ov.onClose;
      ov.back.remove();
      ov.name = ov.back = ov.from = ov.onClose = null;
      if (done) done();
      if (from && from.focus && document.contains(from)) from.focus();
    },
    // The "g" chord: nav.js arms it, and actions.js (capture phase, runs first) leaves the
    // armed second key to nav.js, so both read one timer.
    chord: false, chordTimer: null,
    armChord: function () { ov.chord = true; clearTimeout(ov.chordTimer); ov.chordTimer = setTimeout(function () { ov.chord = false; }, 1200); },
    takeChord: function () { var c = ov.chord; ov.chord = false; clearTimeout(ov.chordTimer); return c; }
  };

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  // The one place JS writes a date: the relative text, with the full date as the hover tip.
  // `w` is a When from the API: {text, datetime, tip}.
  function whenEl(w, cls) {
    var e = el(w.datetime ? "time" : "span", cls, w.text);
    if (w.datetime) {
      e.setAttribute("datetime", w.datetime);
      e.setAttribute("data-tip", w.tip);
      e.tabIndex = 0;
    }
    return e;
  }
  function typing(t) {
    return t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable);
  }
  // Single-key shortcuts can be turned off (WCAG 2.1.4). The choice lives in localStorage like
  // the theme; "?" stays so the sheet, and the switch in it, is always reachable.
  var KEYS_STORE = "gitterrace-keys";
  function keysOff() {
    try { return localStorage.getItem(KEYS_STORE) === "off"; } catch (e) { return false; }
  }
  function setKeysOff(off) {
    try {
      if (off) localStorage.setItem(KEYS_STORE, "off"); else localStorage.removeItem(KEYS_STORE);
    } catch (e) {}
  }
  window.gtKeysOff = keysOff;
  function toast(msg) { window.gtToast(msg); }
  function copy(text, done, btn) { return window.gtCopy(text, done, btn); }

  // Search folds into a button when the top bar is narrow (CSS decides). Opening it takes the
  // whole bar; Escape, or focus leaving it empty, folds it again. Returns false when there is
  // no search box, so "/" falls through.
  var bar = document.querySelector(".topbar");
  var q = document.getElementById("q");
  function openSearch() {
    if (!q) return false;
    if (bar) bar.classList.add("searching");
    q.focus();
    q.select();
    return true;
  }
  function foldSearch() { if (bar) bar.classList.remove("searching"); }
  var sbtn = document.getElementById("searchbtn");
  if (sbtn) sbtn.addEventListener("click", openSearch);
  if (q) {
    q.addEventListener("keydown", function (e) { if (e.key === "Escape") { q.blur(); foldSearch(); } });
    q.addEventListener("blur", function () { if (!q.value) foldSearch(); });
  }

  // ---- overlays -----------------------------------------------------------
  // `btn` is the toggle whose aria-expanded follows the overlay, if any.
  function overlay(cls, label, btn) {
    var back = el("div", "ov-back");
    var box = el("div", "ov " + cls);
    back.appendChild(box);
    back.addEventListener("mousedown", function (e) { if (e.target === back) closeOverlay(); });
    ov.open(cls, back, box, { label: label, onClose: btn ? function () { btn.setAttribute("aria-expanded", "false"); } : null });
    if (btn) btn.setAttribute("aria-expanded", "true");
    return box;
  }
  function closeOverlay() { ov.close(); }
  // What pickers.js builds on; the function declarations below are hoisted.
  window.gtNav = {
    B: B, STATIC: STATIC, ov: ov, el: el, whenEl: whenEl, inRepo: local, isOff: isOff,
    overlay: overlay, listPicker: listPicker, closeOverlay: closeOverlay
  };

  // Subsequence match with a score favoring file-name hits, word starts and short paths.
  function score(q, path) {
    var p = path.toLowerCase(), qi = 0, s = 0, last = -2, base = p.lastIndexOf("/") + 1;
    for (var i = 0; i < p.length && qi < q.length; i++) {
      if (p[i] === q[qi]) {
        s += 1;
        if (i === last + 1) s += 3;
        var prev = i ? p[i - 1] : "/";
        if (prev === "/" || prev === "_" || prev === "-" || prev === ".") s += 4;
        if (i >= base) s += 2;
        last = i;
        qi++;
      }
    }
    if (qi < q.length) return -1;
    if (p.indexOf(q, base) >= 0) s += 15;
    return s - path.length * 0.05;
  }

  function listPicker(box, placeholder, getItems, render, choose, hint, none) {
    var input = el("input", "ov-input");
    input.placeholder = placeholder;
    input.setAttribute("aria-label", placeholder);
    input.autocomplete = "off";
    input.spellcheck = false;
    var list = el("ul", "ov-list");
    var foot = el("div", "ov-foot", hint || "");
    box.appendChild(input);
    box.appendChild(list);
    box.appendChild(foot);
    var shown = [], sel = 0;
    function paint() {
      list.textContent = "";
      shown.forEach(function (it, i) {
        var li = el("li", i === sel ? "on" : "");
        render(li, it, input.value);
        li.addEventListener("mousedown", function (e) { e.preventDefault(); choose(it); });
        li.addEventListener("mousemove", function () { if (sel !== i) { sel = i; mark(); } });
        list.appendChild(li);
      });
      if (!shown.length) list.appendChild(el("li", "none", none || "No matches"));
    }
    function mark() {
      var kids = list.children;
      for (var i = 0; i < kids.length; i++) kids[i].classList.toggle("on", i === sel);
      var cur = kids[sel];
      if (cur && cur.scrollIntoView) cur.scrollIntoView({ block: "nearest" });
    }
    function refresh() {
      shown = getItems(input.value.trim().toLowerCase());
      sel = 0;
      paint();
    }
    input.addEventListener("input", refresh);
    input.addEventListener("keydown", function (e) {
      if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) { sel = Math.min(sel + 1, shown.length - 1); mark(); e.preventDefault(); }
      else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) { sel = Math.max(sel - 1, 0); mark(); e.preventDefault(); }
      else if (e.key === "Enter") { if (shown[sel]) choose(shown[sel], e); e.preventDefault(); }
    });
    input.focus();
    return { refresh: refresh, input: input };
  }

  // ---- file finder --------------------------------------------------------
  var files = null, filesPending = null;
  function loadFiles() {
    if (files) return Promise.resolve(files);
    if (!filesPending) {
      filesPending = STATIC ? Promise.resolve([]) : fetch(B + "/api/files/" + revEnc).then(function (r) { return r.json(); }).then(function (l) { files = l; return l; });
    }
    return filesPending;
  }
  function openFinder() {
    var box = overlay("finder", "Go to file");
    var p = listPicker(box, "Go to file", function (q) {
      if (!files) return [];
      if (!q) return files.slice(0, 50).map(function (f) { return { f: f, pos: [] }; });
      var parts = q.split(/\s+/).filter(Boolean), out = [];
      for (var i = 0; i < files.length && out.length < 5000; i++) {
        var sc = 0;
        for (var k = 0; k < parts.length && sc >= 0; k++) {
          var s = score(parts[k], files[i]);
          sc = s < 0 ? -1 : sc + s;
        }
        if (sc >= 0) out.push({ f: files[i], s: sc });
      }
      out.sort(function (a, b) { return b.s - a.s; });
      return out.slice(0, 50);
    }, function (li, it, q) {
      var i = it.f.lastIndexOf("/");
      li.appendChild(el("span", /\.(png|jpe?g|gif|webp|avif|bmp|ico|svg)$/i.test(it.f) ? "ico img" : "ico file", ""));
      li.appendChild(el("span", "name", it.f.slice(i + 1)));
      if (i >= 0) li.appendChild(el("span", "dir", it.f.slice(0, i)));
    }, function (it, e) {
      var url = B + "/blob/" + revEnc + "/" + it.f.split("/").map(encodeURIComponent).join("/");
      if (e && (e.metaKey || e.ctrlKey)) window.open(url, "_blank"); else location.href = url;
    }, "Enter to open, Esc to close");
    var status = el("div", "ov-status", "Loading files...");
    box.insertBefore(status, box.children[1]);
    loadFiles().then(function () { status.remove(); p.refresh(); }, function () { status.textContent = "Could not load the file list"; });
  }


  // ---- shortcut sheet -----------------------------------------------------
  var SHORTCUTS = [
    ["t", "Go to file"], ["/", "Focus search"], ["y", "Permalink to this commit"],
    ["g c", "Code"], ["g i", "Insights"], ["g b", "Branches and tags"], ["g l", "Commits"], ["g s", "Search"],
    ["b", "Switch branch or tag"], [".", "Action menu"], ["j / k", "Next / previous commit or result"], ["?", "This sheet"], ["Esc", "Close"]
  ];
  // Action chords: "a" then the key the user gave an action in actions.toml.
  function actionChords() {
    var m = document.querySelector('meta[name="gitterrace-actions"]'), list = [];
    try { list = m ? JSON.parse(m.dataset.menu || "[]") : []; } catch (e) { list = []; }
    return list.filter(function (a) { return a.key; }).map(function (a) { return ["a " + a.key, a.label]; });
  }
  function openHelp() {
    var box = overlay("help", "Keyboard shortcuts"), dl = el("dl", "keys");
    var sw = el("label", "keys-switch"), cb = el("input");
    cb.type = "checkbox";
    cb.id = "keys-off";
    cb.checked = keysOff();
    cb.addEventListener("change", function () { setKeysOff(cb.checked); });
    sw.append(cb, el("span", "", "Turn off single-key shortcuts (? still opens this sheet)"));
    box.append(el("h2", "", "Keyboard shortcuts"), sw, dl);
    // A chord is one unit: keys close together, joined by a muted "then" ("j / k" reads "or").
    SHORTCUTS.filter(function (x) { return !(isOff("insights") && x[0] === "g i") && !(isOff("search") && (x[0] === "g s" || x[0] === "/")); }).concat(actionChords()).forEach(function (s) {
      var alt = s[0].indexOf(" / ") > 0, dt = dl.appendChild(el("dt"));
      s[0].split(alt ? " / " : " ").forEach(function (k, i) {
        if (i > 0) dt.appendChild(el("span", "muted", alt ? "or" : "then"));
        dt.appendChild(el("kbd", "", k));
      });
      dl.appendChild(el("dd", "", s[1]));
    });
  }

  // ---- permalink ----------------------------------------------------------
  function permalink() {
    if (!commit) { toast("No commit for this page"); return; }
    var parts = local(location.pathname).split("/");
    if (["tree", "blob", "blame", "log", "raw"].indexOf(parts[1]) >= 0 && parts.length > 2) parts[2] = commit;
    var path = B + parts.join("/");
    history.replaceState(null, "", path + location.search + location.hash);
    copy(location.href, "Permalink copied");
  }

  // ---- list navigation ----------------------------------------------------
  var cursor = -1;
  function rows() { return Array.prototype.slice.call(document.querySelectorAll("ul.log > li, .hits > header, .hits tr.hit")); }
  function move(d) {
    var r = rows();
    if (!r.length) return;
    cursor = Math.max(0, Math.min(r.length - 1, cursor + d));
    r.forEach(function (x, i) { x.classList.toggle("cur", i === cursor); });
    r[cursor].scrollIntoView({ block: "center" });
  }
  function openCurrent() {
    var r = rows();
    var a = r[cursor] && r[cursor].querySelector("a");
    if (a) location.href = a.href;
  }

  // ---- keys ---------------------------------------------------------------
  // Single page keys: g, t, /, y, b, ?, j, k, Enter. RESERVED_KEYS in the actions config
  // refuses the letters among them as action keys; a test pins the two lists together.
  var goto = { c: B + "/", i: B + "/stats", b: B + "/refs", l: B + "/log/" + revEnc, s: B + "/search" };
  if (isOff("insights")) delete goto.i;
  if (isOff("search")) delete goto.s;
  document.addEventListener("keydown", function (e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === "Escape") { if (ov.name) closeOverlay(); else if (typing(e.target)) e.target.blur(); return; }
    // actions.js handles its "a" chord in the capture phase and marks the key handled; the
    // action menu is its own overlay.
    if (typing(e.target) || ov.name || e.defaultPrevented) return;
    var k = e.key;
    if (keysOff() && k !== "?") return;
    if (ov.takeChord()) {
      // The second key belongs to the chord even with no target, so "g a" never arms "a".
      e.preventDefault();
      if (goto[k]) location.href = goto[k];
      return;
    }
    if (k === "g") { ov.armChord(); }
    else if (k === "t") { openFinder(); e.preventDefault(); }
    else if (k === "/") { if (openSearch()) e.preventDefault(); }
    else if (k === "y") { permalink(); }
    else if (k === "b") { if (window.gtPickers) window.gtPickers.openRefs(); e.preventDefault(); }
    else if (k === "?") { openHelp(); e.preventDefault(); }
    else if (k === "j") move(1);
    else if (k === "k") move(-1);
    else if (k === "Enter" && cursor >= 0) openCurrent();
  });

  // ---- line selection on blob and blame pages -------------------------------
  var code = document.querySelector(".card .src");
  function paintLines() {
    var m = /^#L(\d+)(?:-L?(\d+))?$/.exec(location.hash);
    var prev = document.querySelectorAll(".src .sel");
    for (var i = 0; i < prev.length; i++) prev[i].classList.remove("sel");
    if (!m) return;
    var a = +m[1], b = m[2] ? +m[2] : a;
    if (b < a) { var t = a; a = b; b = t; }
    for (var n = a; n <= b && n - a < 5000; n++) {
      var row = document.getElementById("L" + n);
      if (row) row.classList.add("sel");
    }
    var first = document.getElementById("L" + a);
    if (first && first.scrollIntoView) first.scrollIntoView({ block: "center" });
  }
  if (code && code.querySelector("[id^=L]")) {
    var anchor = null;
    code.addEventListener("click", function (e) {
      var a = e.target.closest && e.target.closest(".l");
      if (!a || e.clientX - a.getBoundingClientRect().left > 8 * parseFloat(getComputedStyle(a).fontSize)) return;
      var n = +a.id.slice(1);
      e.preventDefault();
      var h;
      if (e.shiftKey && anchor) h = "#L" + Math.min(anchor, n) + "-L" + Math.max(anchor, n);
      else { anchor = n; h = "#L" + n; }
      history.replaceState(null, "", location.pathname + location.search + h);
      paintLines();
    });
    window.addEventListener("hashchange", paintLines);
    paintLines();
  }

  // ---- big files arrive with only the first screen highlighted ---------------
  var plain = document.querySelector(".src[data-hl]");
  if (plain) {
    var chunks = plain.querySelectorAll(".ch");
    var hlUrl = location.pathname.replace("/blob/", "/hl/");
    fetch(hlUrl).then(function (r) { return r.ok ? r.text() : null; }).then(function (html) {
      if (html === null || chunks.length < 3) return;
      var sel = plain.querySelectorAll(".sel");
      var ids = [];
      for (var i = 0; i < sel.length; i++) ids.push(sel[i].id);
      for (var j = chunks.length - 1; j >= 2; j--) chunks[j].remove();
      chunks[1].insertAdjacentHTML("afterend", html);
      for (var k = 0; k < ids.length; k++) { var el = document.getElementById(ids[k]); if (el) el.classList.add("sel"); }
    }).catch(function () {});
  }

  // ---- folded diffs -----------------------------------------------------------
  // A folded file is a link to its body. Clicking it (or Enter on it) swaps the body in; a file
  // folded only for the page budget (data-lazy) also loads when it scrolls near the viewport,
  // and an #fN anchor opens its file. Find in page sees loaded files only.
  function loadDiff(b, scroll) {
    if (b.dataset.busy) return;
    b.dataset.busy = "1";
    if (lazyObs) lazyObs.unobserve(b);
    var section = b.closest("section");
    fetch(b.dataset.loadDiff).then(function (r) { if (!r.ok) throw r; return r.text(); }).then(function (h) {
      var p = b.parentNode, focused = document.activeElement === b;
      p.insertAdjacentHTML("afterend", h);
      var body = p.nextElementSibling;
      p.remove();
      if (focused && body) { body.tabIndex = -1; body.focus({ preventScroll: true }); }
      // The body above the target grew or the target moved: put the file's top back in view.
      if (scroll && section) section.scrollIntoView();
    }, function () {
      delete b.dataset.busy;
      // Watched again, so a lazy file that failed once retries when it next scrolls near.
      if (lazyObs && b.hasAttribute("data-lazy")) lazyObs.observe(b);
    });
  }
  document.addEventListener("click", function (e) {
    var b = e.target.closest && e.target.closest("[data-load-diff]");
    if (!b || e.button || e.metaKey || e.ctrlKey || e.shiftKey) return;
    e.preventDefault();
    loadDiff(b);
  });
  var lazyObs = window.IntersectionObserver ? new IntersectionObserver(function (es) {
    es.forEach(function (x) { if (x.isIntersecting) loadDiff(x.target); });
  }, { rootMargin: "1500px 0px" }) : null;
  function watchLazy() {
    if (!lazyObs) return;
    Array.prototype.forEach.call(document.querySelectorAll("[data-load-diff][data-lazy]"), function (b) { lazyObs.observe(b); });
  }
  // `jump` brings the file into view once it loads; a live refresh opens it where it is.
  function openHashed(jump) {
    var m = /^#(f\d+)$/.exec(location.hash), s = m && document.getElementById(m[1]);
    var b = s && s.querySelector("[data-load-diff]");
    if (b) loadDiff(b, jump !== false);
  }
  watchLazy();
  openHashed();
  window.addEventListener("hashchange", function () { openHashed(true); });
  document.addEventListener("gt:live-refreshed", function () { watchLazy(); openHashed(false); });

  // ---- copy buttons ---------------------------------------------------------
  Array.prototype.forEach.call(document.querySelectorAll("[data-copyable]"), function (card) {
    var b = el("button", "copy float", "Copy");
    b.type = "button";
    b.setAttribute("data-tip", "Copy file contents");
    b.addEventListener("click", function () {
      var cells = card.querySelectorAll(".src .l, td.c");
      copy(Array.prototype.map.call(cells, function (c) { return c.textContent; }).join("\n") + "\n", "Copied file", b);
    });
    card.style.position = "relative";
    card.appendChild(b);
  });
  // Not on diagrams: mermaid reads the block's text, so a button inside it becomes a node.
  Array.prototype.forEach.call(document.querySelectorAll(".markdown-body pre:not(.mermaid)"), function (pre) {
    var b = el("button", "copy float", "Copy");
    b.type = "button";
    // The button sits inside the pre, so its own label would ride along in textContent.
    b.addEventListener("click", function () {
      var c = pre.cloneNode(true);
      Array.prototype.forEach.call(c.querySelectorAll(".copy"), function (x) { x.remove(); });
      copy(c.textContent, "Copied", b);
    });
    pre.style.position = "relative";
    pre.appendChild(b);
  });
  document.addEventListener("click", function (e) {
    var t = e.target.closest && e.target.closest("[data-copy],[data-copy-url]");
    if (!t) return;
    if (t.dataset.copy) copy(t.dataset.copy, "Copied sha", t);
    else {
      var parts = local(location.pathname).split("/");
      if (commit && parts.length > 2) parts[2] = commit;
      // links.style = "gitterrace": the same file as gitterrace://<id>/<sha>/<path>#L..
      var rm = document.querySelector('meta[name="gt-repo"]');
      if (rm && rm.dataset.link === "gitterrace" && rm.dataset.id && commit && parts[1] === "blob") {
        var hash = /^#L\d+(-L\d+)?$/.test(location.hash) ? location.hash : "";
        copy("gitterrace://" + rm.dataset.id + "/" + commit + "/" + parts.slice(3).join("/") + hash, "Permalink copied", t);
        return;
      }
      copy(location.origin + B + parts.join("/") + location.search + location.hash, "Permalink copied", t);
    }
  });

  // ---- last commit column -----------------------------------------------------
  var table = document.querySelector("table.entries[data-last]");
  // A static export has no API: leave the column empty rather than fetch a 404.
  if (table && document.body.dataset.static === undefined) {
    fetch(table.dataset.last.replace(/\/$/, "")).then(function (r) { return r.json(); }).then(function (list) {
      var by = {};
      list.forEach(function (c) { by[c.name] = c; });
      Array.prototype.forEach.call(table.querySelectorAll("td.lc"), function (td) {
        var c = by[td.dataset.name];
        if (!c) return;
        var a = el("a", "subj", c.subject);
        a.href = B + "/commit/" + c.id;
        td.appendChild(a);
        // The age gets its own right-aligned column, so the
        // message can use all the width between name and age.
        var age = el("td", "ago");
        age.appendChild(whenEl(c.when));
        td.parentNode.insertBefore(age, td.nextSibling);
      });
      table.classList.add("has-last");
    }).catch(function () {});
  }

  // ---- screenshot hooks: ?open=finder|help -------------------------------
  var want = /[?&]open=(finder|help)/.exec(location.search);
  if (want) {
    if (want[1] === "finder") { openFinder(); var qm = /[?&]fq=([^&]*)/.exec(location.search); if (qm) setTimeout(function () { var i = document.querySelector(".ov-input"); if (i) { i.value = decodeURIComponent(qm[1]); i.dispatchEvent(new Event("input")); } }, 300); }
    else openHelp();
  }
})();
