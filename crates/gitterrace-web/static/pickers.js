// The branch and repository switchers. nav.js shares its overlay and list helpers as
// `gtNav` and loads first; this file offers `gtPickers.openRefs` for nav.js's "b" key.
(function () {
  "use strict";
  var n = window.gtNav;
  if (!n) return;
  var B = n.B, STATIC = n.STATIC, el = n.el, whenEl = n.whenEl, inRepo = n.inRepo, isOff = n.isOff;
  var overlay = n.overlay, listPicker = n.listPicker, closeOverlay = n.closeOverlay;

  // ---- ref switcher -------------------------------------------------------
  var refs = null;
  function swapRev(name) {
    var path = inRepo(location.pathname);
    var parts = path.split("/");
    if (["tree", "blob", "blame", "log", "raw"].indexOf(parts[1]) >= 0 && parts.length > 2) {
      parts[2] = encodeURIComponent(name);
      location.href = B + parts.join("/") + location.search;
    } else if (path === "/" || path === "/refs" || path === "/stats"
      || path.indexOf("/stats/") === 0 || path.indexOf("/history/") === 0) {
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
    if (!box) return;
    box.classList.add("anchored");
    var p = listPicker(box, "Filter branches and tags", function (q) {
      return (refs || []).filter(function (x) {
        return !q || x.name.toLowerCase().indexOf(q) >= 0;
      }).slice(0, 100);
    }, function (li, it) {
      li.appendChild(el("span", "name", it.name));
      li.appendChild(el("span", "tag " + it.kind, it.kind));
      li.appendChild(whenEl(it.when, "dir"));
    }, function (it) { swapRev(it.name); }, "Enter to switch");
    n.ov.place(box, btn);
    if (refs) p.refresh();
    else if (!STATIC) fetch(B + "/api/refs").then(function (r) { return r.json(); })
      .then(function (l) { refs = l; p.refresh(); });
  }

  // ---- repository switcher (shown while the server holds several repos) --
  // Repositories join and leave a running server, so the list is read on every page and again
  // on each live notice: the switcher shows, drops an entry or hides without a reload.
  var repos = null;
  // A select panel: title row with a close button, a filter that holds
  // focus, a check before the current repository and an empty slot before the others.
  var TICK = "M13.8 4.2a.75.75 0 0 1 0 1.06l-6.5 6.5a.75.75 0 0 1-1.06 0l-3-3a.75.75 0 1 1 "
    + "1.06-1.06L6.77 10.2l5.97-5.97a.75.75 0 0 1 1.06 0z";
  function icon(d) {
    var NS = "http://www.w3.org/2000/svg";
    var s = document.createElementNS(NS, "svg");
    s.setAttribute("viewBox", "0 0 16 16");
    s.setAttribute("aria-hidden", "true");
    var p = document.createElementNS(NS, "path");
    p.setAttribute("d", d);
    s.appendChild(p);
    return s;
  }
  function openRepos() {
    var btn = document.getElementById("repobtn");
    var box = overlay("refs repopanel", "Switch repository", btn);
    if (!box) return;
    box.classList.add("anchored");
    var head = el("div", "ov-title");
    head.appendChild(el("h2", "", "Switch repository"));
    var x = el("button", "ghost icon", null);
    x.type = "button";
    x.setAttribute("aria-label", "Close");
    x.appendChild(icon("M4 4l8 8M12 4l-8 8"));
    x.addEventListener("click", closeOverlay);
    head.appendChild(x);
    box.appendChild(head);
    var sw = document.getElementById("reposwitch");
    box.dataset.switch = sw ? sw.dataset.switch || "" : "";
    var here = STATIC ? location.href : B.replace(/\/$/, "") + "/";
    var p = listPicker(box, "Filter repositories", function (q) {
      return (repos || []).filter(function (x) {
        return !q || x.name.toLowerCase().indexOf(q) >= 0;
      });
    }, function (li, it) {
      var on = STATIC ? here.indexOf(it.href.replace(/index\.html$/, "")) === 0 : it.href === here;
      var slot = el("span", "tick");
      if (on) { slot.appendChild(icon(TICK)); li.setAttribute("aria-current", "true"); }
      li.appendChild(slot);
      var txt = el("span", "two");
      txt.appendChild(el("span", "name", it.name));
      txt.appendChild(whenEl(it.when, "dir"));
      li.appendChild(txt);
    }, function (it) {
      // The server computed where this view lives in another repository (data-switch).
      var to = STATIC ? "" : (box.dataset.switch || "");
      location.href = it.href + (to.charAt(0) === "/" ? to.slice(1) : "");
    }, "Enter to open", "No repositories match");
    n.ov.place(box, btn);
    if (repos) p.refresh();
    else load(function (l) { repos = l; p.refresh(); });
  }
  var repoBtn = document.getElementById("repobtn");
  // An export has no /api: it lists its repositories as hidden links on every page, already
  // relative to it, and the current one is the link this page sits under.
  function load(done) {
    var nav = document.getElementById("gt-repolist");
    if (!STATIC) {
      return fetch("/api/repos", { cache: "no-store" })
        .then(function (r) { return r.json(); }).then(done);
    }
    done(nav ? [].map.call(nav.querySelectorAll("a"), function (a) {
      return { name: a.textContent, href: a.href, when: { text: "" } };
    }) : []);
    return Promise.resolve();
  }
  function loadRepos() {
    load(function (l) {
      repos = l;
      var box = document.getElementById("reposwitch");
      if (box) box.hidden = l.length < 2;
    }).catch(function () {});
  }
  if (repoBtn && !isOff("picker")) {
    repoBtn.addEventListener("click", openRepos);
    loadRepos();
    // live.js: a repository joined or left, so the switcher follows without a reload.
    if (!STATIC) document.addEventListener("gt:repos-changed", loadRepos);
  }
  var rb = document.getElementById("refbtn");
  if (rb) rb.addEventListener("click", openRefs);
  window.gtPickers = { openRefs: openRefs };
  // Screenshot hook, ?open=refs.
  if (/[?&]open=refs/.test(location.search)) openRefs();
})();
