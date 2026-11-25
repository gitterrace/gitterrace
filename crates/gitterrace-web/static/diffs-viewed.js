// Viewed marks on a diff page: a checkbox per file, a count, a hide switch and a badge on a
// marked file whose content changed since. Two stores. The working tree page keeps them in the
// cache folder through the server (its Checkpoint bar is on the page, and a mark holds while the
// file's patch id, data-p, is the same). Every other page, and a working tree page with no cache
// folder, keeps them in localStorage under the page's own address, so nothing leaves the machine.
// app.js loads this only on a page with diff files.
(function () {
  "use strict";
  var KEY = "gt-viewed:" + location.pathname + location.search;
  var B = window.gtNav.B;
  var el = window.gtNav.el;
  // Server marks by path, the patch id of each. Null until the first read has come back.
  var served = null;
  function server() { return !!document.querySelector("[data-cpbar]"); }
  // The id a mark is compared with on a file: the patch id on the server, the content tag here.
  function tag(s) { return server() ? s.dataset.p : s.dataset.h; }
  function read() {
    if (server()) return served || {};
    try { return JSON.parse(localStorage.getItem(KEY)) || {}; } catch (e) { return {}; }
  }
  function write(m) {
    try {
      if (Object.keys(m).length) localStorage.setItem(KEY, JSON.stringify(m));
      else localStorage.removeItem(KEY);
    } catch (e) { /* storage refused: the marks last for this page view only */ }
  }
  function post(url, body) {
    return fetch(B + url, {
      method: "POST", cache: "no-store", headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body)
    }).then(function (r) { if (!r.ok) throw new Error(String(r.status)); return r.json(); });
  }
  function note(text) {
    var n = document.querySelector("[data-cpbar] .cpnote");
    if (n) n.textContent = text;
  }
  // The marks the server holds, then, once, the ones this browser kept for the same address: a
  // local mark whose content tag still matches the file is sent up and the local copy dropped.
  var reads = 0;
  function load() {
    // Only the newest read counts: an older one that lands late holds marks from before a save.
    var mine = ++reads;
    return fetch(B + "/api/v1/viewed", { cache: "no-store" })
      .then(function (r) { if (!r.ok) throw new Error(String(r.status)); return r.json(); })
      .then(function (j) {
        if (mine !== reads) return null;
        served = {};
        j.viewed.forEach(function (v) { served[v.path] = v.patch; });
        return migrate();
      })
      .catch(function () { served = served || {}; note("Viewed marks could not be read."); });
  }
  function migrate() {
    var old = null;
    try { old = JSON.parse(localStorage.getItem(KEY)); } catch (e) { old = null; }
    if (!old) return null;
    var items = [];
    files().forEach(function (s) {
      var p = s.dataset.path;
      if (old[p] && old[p] === s.dataset.h && s.dataset.p && !(p in served)) {
        items.push({ path: p, patch: s.dataset.p });
      }
    });
    if (!items.length) return null;
    var sent = {};
    items.forEach(function (i) { sent[i.path] = old[i.path]; });
    touched = {};
    return post("/_viewed/all", { items: items }).then(function () {
      items.forEach(function (i) {
        // A mark the reader changed while the save was in flight is newer: leave it be.
        if (!touched[i.path] && !(i.path in served)) served[i.path] = i.patch;
      });
      forget(sent);
    }).catch(function () {
      note("Earlier viewed marks could not be moved. " +
        "They are kept here and tried again on the next load.");
    });
  }
  // Marks changed by a click since a migration began, by path.
  var touched = {};
  // Drops only the confirmed entries, and only while the stored value is still the one sent, so
  // an entry another tab rewrote or added meanwhile stays.
  function forget(sent) {
    try {
      var now = JSON.parse(localStorage.getItem(KEY)) || {};
      Object.keys(sent).forEach(function (p) { if (now[p] === sent[p]) delete now[p]; });
      if (Object.keys(now).length) localStorage.setItem(KEY, JSON.stringify(now));
      else localStorage.removeItem(KEY);
    } catch (e) { /* storage refused: the entries stay for the next load */ }
  }
  function files() {
    return Array.prototype.slice.call(document.querySelectorAll("section.file[data-path]"));
  }
  // The bar is server rendered: this only fills in its numbers, so nothing moves when it runs.
  function count(list, marks) {
    var n = 0, p = document.querySelector(".difftools .viewed-count");
    var bar = document.querySelector(".difftools .dt-bar");
    list.forEach(function (s) {
      var on = marks[s.dataset.path] === tag(s);
      if (on) n++;
      if (on) s.setAttribute("data-viewed", ""); else s.removeAttribute("data-viewed");
    });
    if (p) p.textContent = "Viewed " + n + " of " + list.length;
    if (bar) bar.value = n;
    var hide = document.querySelector(".difftools .dt-hide-box");
    if (hide) hide.checked = document.body.classList.contains("hide-viewed");
  }
  // One handler for every box, so a file a live refresh brings in needs no binding of its own.
  document.addEventListener("change", function (e) {
    var t = e.target, s = t.classList.contains("viewed") && t.closest("section.file");
    if (!s) return;
    if (server()) {
      // Shown at once and saved after: a refused save puts the old mark back.
      var on = t.checked, patch = s.dataset.p, path = s.dataset.path;
      served = served || {};
      touched[path] = true;
      var before = served[path];
      if (on) served[path] = patch; else delete served[path];
      decorate();
      post("/_viewed", { path: path, patch: patch, viewed: on }).catch(function () {
        if (before === undefined) delete served[path]; else served[path] = before;
        note("The viewed mark was not saved.");
        decorate();
      });
      return;
    }
    var m = read();
    if (t.checked) m[s.dataset.path] = s.dataset.h; else delete m[s.dataset.path];
    write(m);
    decorate();
  });
  document.addEventListener("change", function (e) {
    if (e.target.classList && e.target.classList.contains("dt-hide-box")) {
      document.body.classList.toggle("hide-viewed", e.target.checked);
    }
  });
  function decorate() {
    var list = files();
    var marks = read();
    list.forEach(function (s) {
      var head = s.querySelector("header"), path = s.dataset.path, h = tag(s);
      var box = s.querySelector("input.viewed");
      if (!box) {
        var lab = el("label", "viewed-mark");
        box = el("input", "viewed");
        box.type = "checkbox";
        lab.appendChild(box);
        lab.appendChild(document.createTextNode(" Viewed"));
        // Right after the +/- counts, so the toggle reads with the numbers it marks.
        var counts = head.querySelector(".counts, .media-counts");
        if (counts) counts.after(lab); else head.appendChild(lab);
      }
      var seen = path in marks;
      var stale = seen && marks[path] !== h;
      var badge = head.querySelector(".viewed-changed");
      box.checked = seen && !stale;
      if (stale && !badge) {
        badge = el("span", "tag viewed-changed", "Changed since viewed");
        head.insertBefore(badge, head.querySelector(".counts, .media-counts"));
      } else if (!stale && badge) {
        badge.remove();
      }
    });
    count(list, marks);
  }
  // The boxes at once, so nothing shifts, then the server's marks fill them in.
  function start() {
    decorate();
    // load() settles every failure itself, so nothing here can reject.
    if (server() && served === null) void load().then(decorate);
  }
  start();
  document.addEventListener("gt:live-refreshed", start);
  // The Checkpoint bar's Mark all viewed wrote marks: read them again.
  document.addEventListener("gt:viewed-changed", function () {
    served = null;
    start();
  });
})();
