// Keyboard shortcuts, file finder, ref switcher, line selection and copy buttons.
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
    close: function () {
      if (!ov.back) return;
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
  function typing(t) {
    return t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable);
  }
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

  function listPicker(box, placeholder, getItems, render, choose, hint) {
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
      if (!shown.length) list.appendChild(el("li", "none", "No matches"));
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
      filesPending = fetch(B + "/api/files/" + revEnc).then(function (r) { return r.json(); }).then(function (l) { files = l; return l; });
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

  // ---- ref switcher -------------------------------------------------------
  var refs = null;
  function swapRev(name) {
    var path = local(location.pathname);
    var parts = path.split("/");
    if (["tree", "blob", "blame", "log", "raw"].indexOf(parts[1]) >= 0 && parts.length > 2) {
      parts[2] = encodeURIComponent(name);
      location.href = B + parts.join("/") + location.search;
    } else if (path === "/" || path === "/stats" || path.indexOf("/stats/") === 0 || path.indexOf("/history/") === 0) {
      // Pages computed from the selected ref keep their place and their filters.
      var q = new URLSearchParams(location.search);
      if (name === "HEAD") q.delete("rev"); else q.set("rev", name);
      var s = q.toString();
      location.href = B + path + (s ? "?" + s : "");
    } else {
      location.href = B + "/tree/" + encodeURIComponent(name);
    }
  }
  function openRefs() {
    var btn = document.getElementById("refbtn");
    var box = overlay("refs", "Switch branch or tag", btn);
    if (btn) {
      var r = btn.getBoundingClientRect();
      box.classList.add("anchored");
      box.style.left = Math.max(8, r.left) + "px";
      box.style.top = r.bottom + 6 + "px";
    }
    var p = listPicker(box, "Filter branches and tags", function (q) {
      return (refs || []).filter(function (x) { return !q || x.name.toLowerCase().indexOf(q) >= 0; }).slice(0, 100);
    }, function (li, it) {
      li.appendChild(el("span", "name", it.name));
      li.appendChild(el("span", "tag " + it.kind, it.kind));
      li.appendChild(el("span", "dir", it.when));
    }, function (it) { swapRev(it.name); }, "Enter to switch");
    if (refs) p.refresh();
    else fetch(B + "/api/refs").then(function (r) { return r.json(); }).then(function (l) { refs = l; p.refresh(); });
  }

  // ---- repository switcher (only rendered when several repos are served) --
  var repos = null;
  function openRepos() {
    var btn = document.getElementById("repobtn");
    var box = overlay("refs", "Switch repository", btn);
    var r = btn.getBoundingClientRect();
    box.classList.add("anchored");
    box.style.left = Math.max(8, r.left) + "px";
    box.style.top = r.bottom + 6 + "px";
    var p = listPicker(box, "Filter repositories", function (q) {
      return (repos || []).filter(function (x) { return !q || x.name.toLowerCase().indexOf(q) >= 0; });
    }, function (li, it) {
      li.appendChild(el("span", "name", it.name));
      li.appendChild(el("span", "dir", it.when));
    }, function (it) { location.href = it.href; }, "Enter to open");
    if (repos) p.refresh();
    else fetch("/api/repos").then(function (r) { return r.json(); }).then(function (l) { repos = l; p.refresh(); });
  }
  var repoBtn = document.getElementById("repobtn");
  if (repoBtn) repoBtn.addEventListener("click", openRepos);

  // ---- shortcut sheet -----------------------------------------------------
  var SHORTCUTS = [
    ["t", "Go to file"], ["/", "Focus search"], ["y", "Permalink to this commit"],
    ["g c", "Code"], ["g i", "Insights"], ["g b", "Branches and tags"], ["g l", "Commits"], ["g s", "Search"],
    ["b", "Switch branch or tag"], ["j / k", "Next / previous commit or result"], ["?", "This sheet"], ["Esc", "Close"]
  ];
  // Action chords: "a" then the key the user gave an action in actions.toml.
  function actionChords() {
    var m = document.querySelector('meta[name="gitterrace-actions"]');
    var list = [];
    try { list = m ? JSON.parse(m.dataset.menu || "[]") : []; } catch (e) { list = []; }
    return list.filter(function (a) { return a.key; }).map(function (a) { return ["a " + a.key, a.label]; });
  }
  function openHelp() {
    var box = overlay("help", "Keyboard shortcuts");
    box.appendChild(el("h2", "", "Keyboard shortcuts"));
    var dl = el("dl", "keys");
    SHORTCUTS.concat(actionChords()).forEach(function (s) {
      var dt = el("dt");
      s[0].split(" ").forEach(function (k) { if (k !== "/" || s[0] === "/") { dt.appendChild(el("kbd", "", k)); dt.appendChild(document.createTextNode(" ")); } });
      dl.appendChild(dt);
      dl.appendChild(el("dd", "", s[1]));
    });
    box.appendChild(dl);
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
  document.addEventListener("keydown", function (e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === "Escape") { if (ov.name) closeOverlay(); else if (typing(e.target)) e.target.blur(); return; }
    // actions.js handles its "a" chord in the capture phase and marks the key handled; the
    // action menu is its own overlay.
    if (typing(e.target) || ov.name || e.defaultPrevented) return;
    var k = e.key;
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
    else if (k === "b") { openRefs(); e.preventDefault(); }
    else if (k === "?") { openHelp(); e.preventDefault(); }
    else if (k === "j") move(1);
    else if (k === "k") move(-1);
    else if (k === "Enter" && cursor >= 0) openCurrent();
  });
  var rb = document.getElementById("refbtn");
  if (rb) rb.addEventListener("click", openRefs);

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
    var hlUrl = location.pathname.replace(/\/blob\//, "/hl/");
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
  document.addEventListener("click", function (e) {
    var b = e.target.closest && e.target.closest("[data-load-diff]");
    if (!b) return;
    b.disabled = true;
    fetch(b.dataset.loadDiff).then(function (r) { return r.text(); }).then(function (h) {
      b.parentNode.outerHTML = h;
    }, function () { b.disabled = false; });
  });

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
        var age = el("td", "ago", c.when);
        td.parentNode.insertBefore(age, td.nextSibling);
      });
      table.classList.add("has-last");
    }).catch(function () {});
  }

  // ---- screenshot hooks: ?open=finder|help|refs -------------------------------
  var want = /[?&]open=(finder|help|refs)/.exec(location.search);
  if (want) {
    if (want[1] === "finder") { openFinder(); var qm = /[?&]fq=([^&]*)/.exec(location.search); if (qm) setTimeout(function () { var i = document.querySelector(".ov-input"); if (i) { i.value = decodeURIComponent(qm[1]); i.dispatchEvent(new Event("input")); } }, 300); }
    else if (want[1] === "help") openHelp();
    else openRefs();
  }
})();
