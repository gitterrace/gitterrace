// The action menu: "..." on a line selection, hunk, blame line, search hit, hotspot row,
// commit and page, or "." for the current selection. Two built-ins work everywhere (copy
// permalink, copy as markdown); user actions from actions.toml appear only when the server
// embedded a token, and run through POST /_actions/run from a real click or key press.
(function () {
  "use strict";
  var body = document.body;
  var meta = function (n) { return document.querySelector('meta[name="' + n + '"]'); };
  var B = (meta("gt-base") || {}).content || "";
  var repoMeta = meta("gt-repo");
  var repoKey = repoMeta ? repoMeta.content : "";
  var repoId = repoMeta ? repoMeta.dataset.id : "";
  // links.style: "gitterrace" hands out gitterrace://<id>/<sha>/..., opened by `git terrace
  // open` without a running server; "go" the /go URL of this server.
  var linkStyle = repoMeta ? repoMeta.dataset.link : "go";
  var act = meta("gitterrace-actions");
  var token = act ? act.content : "";
  var mine = [];
  try { mine = act ? JSON.parse(act.dataset.menu || "[]") : []; } catch (e) { mine = []; }
  var rev = body.dataset.rev || "HEAD";
  var commit = body.dataset.commit || "";
  var DOCS = "https://github.com/gitterrace/gitterrace/blob/main/docs/actions.md";
  var MD_LINES = 40;

  // The server renders the note into the page, so it is there at first paint. The poll only
  // follows later edits of actions.toml, and touches the page only when the text changes.
  var noteEl = document.getElementById("actnote");
  function showNote(text) {
    if (!noteEl) return;
    var line = noteEl.firstElementChild;
    if (!text) { noteEl.hidden = true; return; }
    if (line.textContent !== text) line.textContent = text;
    noteEl.hidden = false;
  }
  function pollNote() {
    if (document.hidden) return;
    fetch("/_actions/status", { cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (j) { if (j) showNote(j.note); })
      .catch(function () {});
  }
  pollNote();
  setInterval(pollNote, 3000);

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  function toast(msg, sticky) {
    var t = el("div", "toast", msg);
    document.body.appendChild(t);
    setTimeout(function () { t.classList.add("out"); }, sticky ? 4000 : 1400);
    setTimeout(function () { t.remove(); }, sticky ? 4400 : 1800);
  }
  function copy(text, done) {
    var ok = function () { toast(done); };
    if (navigator.clipboard) navigator.clipboard.writeText(text).then(ok, function () { toast("Copy failed"); });
    else toast("Copy failed");
  }
  function local() { var p = location.pathname; return B && p.indexOf(B + "/") === 0 ? p.slice(B.length) : p; }
  function encPath(p) { return p.split("/").map(encodeURIComponent).join("/"); }

  // ---- what the menu acts on ------------------------------------------------
  // A target is {kind, path?, lines?, title, text?}; the server rebuilds the payload from
  // git, so only the locator (repo, rev, sha, path, lines) is ever sent.
  function filePath() {
    var parts = local().split("/");
    if ((parts[1] === "blob" || parts[1] === "blame") && parts.length > 3) {
      return parts.slice(3).map(decodeURIComponent).join("/");
    }
    return null;
  }
  function selection() {
    var m = /^#L(\d+)(?:-L?(\d+))?$/.exec(location.hash);
    if (!m) return null;
    var a = +m[1], b = m[2] ? +m[2] : a;
    return { start: Math.min(a, b), end: Math.max(a, b) };
  }
  function lineText(lines) {
    var out = [];
    for (var n = lines.start; n <= lines.end && out.length < MD_LINES; n++) {
      var row = document.getElementById("L" + n);
      if (!row) break;
      out.push((row.querySelector("td.c") || row).textContent);
    }
    return out.join("\n");
  }
  function current() {
    var path = filePath(), lines = selection();
    if (path && lines) return { kind: "line", path: path, lines: lines, text: lineText(lines) };
    if (path) return { kind: "file", path: path };
    if (local().indexOf("/commit/") === 0) return commitTarget();
    return { kind: "page" };
  }
  function commitTarget() {
    var h = document.querySelector(".head h1");
    var who = document.querySelector(".meta .who b");
    return { kind: "commit", subject: h ? h.textContent : "", author: who ? who.textContent : "" };
  }
  function hunkTarget(hh) {
    var sec = hh.closest("section.file");
    var m = /\+(\d+)(?:,(\d+))?/.exec(hh.textContent);
    var t = { kind: "hunk", path: sec ? sec.dataset.path : null };
    if (m) {
      var start = Math.max(1, +m[1]), len = m[2] == null ? 1 : +m[2];
      t.lines = { start: start, end: Math.max(start, start + len - 1) };
    }
    return t;
  }
  function rowTarget(tr) {
    if (tr.closest("table.blame")) {
      var n = +tr.id.slice(1);
      return { kind: "blame", path: filePath(), lines: { start: n, end: n }, text: lineText({ start: n, end: n }) };
    }
    var t = { kind: tr.dataset.act };
    var holder = tr.closest("[data-path]");
    if (holder) t.path = holder.dataset.path;
    if (tr.dataset.line) { var l = +tr.dataset.line; t.lines = { start: l, end: l }; }
    return t;
  }

  // ---- built-ins ------------------------------------------------------------
  function shortSha() { return commit.slice(0, 7); }
  function ref(t) {
    var base = t.path ? t.path.slice(t.path.lastIndexOf("/") + 1) : "";
    if (!t.lines) return base;
    return base + ":" + t.lines.start + (t.lines.end !== t.lines.start ? "-" + t.lines.end : "");
  }
  function permalink(t) {
    if (!repoId || !commit) return location.href;
    var url = (linkStyle === "gitterrace" ? "gitterrace://" : location.origin + "/go/") + repoId + "/" + commit;
    if (t.path) url += "/" + encPath(t.path);
    if (t.lines) url += "#L" + t.lines.start + (t.lines.end !== t.lines.start ? "-L" + t.lines.end : "");
    return url;
  }
  // The same string the payload's `markdown` field carries.
  function markdown(t) {
    var link = permalink(t);
    if (!t.path) return "**" + (t.subject || document.title) + "** @ " + shortSha() + "\n" + (t.author || "") + "\n" + link + "\n";
    var md = "**" + ref(t) + "** @ " + shortSha() + "\n" + link + "\n";
    if (t.text != null && t.lines) {
      var snippet = t.text.split("\n").slice(0, MD_LINES).join("\n");
      var runs = snippet.match(/`+/g) || [];
      var longest = runs.reduce(function (m, r) { return Math.max(m, r.length); }, 2);
      var fence = new Array(longest + 2).join("`");
      var dot = t.path.lastIndexOf(".");
      var lang = dot > t.path.lastIndexOf("/") ? t.path.slice(dot + 1) : "";
      md += fence + lang + "\n" + snippet + "\n" + fence + "\n";
    }
    return md;
  }

  // ---- running a user action ------------------------------------------------
  var running = {};
  function run(a, t, e, item) {
    // A run starts only from a real click or key press; scripts on this page cannot fake one.
    if (!e || !e.isTrusted || running[a.id]) return;
    running[a.id] = true;
    if (item) { item.classList.add("busy"); item.setAttribute("aria-disabled", "true"); }
    var loc = { repo: repoKey, rev: rev };
    if (commit) loc.sha = commit;
    if (t.path) loc.path = t.path;
    if (t.lines) loc.lines = t.lines;
    fetch("/_actions/run", {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json", "X-Gitterrace-Token": token },
      body: JSON.stringify({ action: a.id, kind: t.kind, locator: loc })
    }).then(function (r) { return r.json().catch(function () { return { ok: false, first_line: "HTTP " + r.status }; }); })
      .then(function (res) {
        if (res.ok) { if (res.first_line) toast(a.label + ": " + res.first_line, true); }
        else toast(a.label + " failed" + (res.exit != null ? " (exit " + res.exit + ")" : "") + (res.first_line ? ": " + res.first_line : ""), true);
      }, function () { toast(a.label + " failed: no response", true); })
      .then(function () { delete running[a.id]; closeMenu(); });
  }

  // ---- the menu -------------------------------------------------------------
  var menu = null;
  function closeMenu() {
    if (menu) { menu.back.remove(); if (menu.from) menu.from.focus(); menu = null; }
  }
  function openMenu(t, anchor) {
    closeMenu();
    var back = el("div", "ov-back actmenu-back");
    var box = el("div", "ov anchored actmenu");
    box.setAttribute("role", "menu");
    var list = el("ul", "ov-list");
    box.appendChild(list);
    back.appendChild(box);
    var items = [], fns = [];
    function add(label, key, fn, cls) {
      var li = el("li", cls || "", null);
      li.setAttribute("role", "menuitem");
      li.tabIndex = -1;
      li.appendChild(el("span", "name", label));
      if (key) li.appendChild(el("kbd", "", key));
      li.addEventListener("click", function (e) { fn(e, li); });
      li.addEventListener("mousemove", function () { focus(items.indexOf(li)); });
      list.appendChild(li);
      items.push(li);
      fns.push(fn);
      return li;
    }
    add("Copy permalink", "p", function () { copy(permalink(t), "Permalink copied"); closeMenu(); });
    add("Copy as markdown", "m", function () { copy(markdown(t), "Markdown copied"); closeMenu(); });
    add("Send to\u2026", "s", function () {
      closeMenu();
      if (window.gtSend) window.gtSend.open({ kind: t.kind, path: t.path || "", lines: t.lines || null, subject: t.subject || "", sha: commit, link: permalink(t), markdown: markdown(t), text: t.text || "" });
    });
    var keys = { p: 0, m: 1, s: 2 };
    var usable = mine.filter(function (a) { return a.kinds.indexOf(t.kind) >= 0; });
    if (usable.length) list.appendChild(el("li", "sep"));
    usable.forEach(function (a) {
      if (a.key && !(a.key in keys)) keys[a.key] = items.length;
      add(a.label, a.key, function (e, li) { run(a, t, e, li); });
    });
    if (!mine.length) {
      var hint = el("li", "hint");
      var link = el("a", "", "Add your own actions");
      link.href = DOCS;
      link.target = "_blank";
      link.rel = "noopener";
      hint.appendChild(link);
      list.appendChild(hint);
    }
    var sel = 0;
    function focus(i) {
      sel = Math.max(0, Math.min(items.length - 1, i));
      items.forEach(function (x, j) { x.classList.toggle("on", j === sel); });
      items[sel].focus();
    }
    back.addEventListener("mousedown", function (e) { if (e.target === back) closeMenu(); });
    // The key press itself is handed to the item, so a user action sees a trusted event.
    box.addEventListener("keydown", function (e) {
      e.stopPropagation();
      if (e.key === "Escape") closeMenu();
      else if (e.key === "ArrowDown") focus(sel + 1);
      else if (e.key === "ArrowUp") focus(sel - 1);
      else if (e.key === "Enter" || e.key === " ") fns[sel](e, items[sel]);
      else if (e.key in keys) { focus(keys[e.key]); fns[keys[e.key]](e, items[keys[e.key]]); }
      else return;
      e.preventDefault();
    });
    document.body.appendChild(back);
    var r = anchor.getBoundingClientRect();
    box.style.left = Math.max(8, Math.min(r.left, window.innerWidth - 300)) + "px";
    box.style.top = Math.min(r.bottom + 6, window.innerHeight - 200) + "px";
    menu = { back: back, from: anchor };
    focus(0);
  }

  // ---- triggers -------------------------------------------------------------
  // The glyph is drawn by CSS, so a trigger inside a code line adds nothing to copied text.
  function button(title) {
    var b = el("button", "actbtn", null);
    b.type = "button";
    b.setAttribute("data-tip", title);
    b.setAttribute("aria-haspopup", "menu");
    b.setAttribute("aria-label", title);
    return b;
  }
  // Page-level trigger in the top bar.
  var theme = document.getElementById("theme");
  if (theme) {
    var top = button("Actions for this page (.)");
    top.classList.add("top");
    top.addEventListener("click", function () { openMenu(current(), top); });
    theme.parentNode.insertBefore(top, theme);
  }
  // Commit page: next to the sha.
  var shaCopy = document.querySelector(".meta [data-copy]");
  if (shaCopy && local().indexOf("/commit/") === 0) {
    var cb = button("Actions for this commit");
    cb.addEventListener("click", function () { openMenu(commitTarget(), cb); });
    shaCopy.parentNode.insertBefore(cb, shaCopy.nextSibling);
  }
  // One floating trigger follows the hovered hunk header, blame line, search hit or
  // hotspot row, so a long page does not carry thousands of buttons.
  var hover = button("Actions");
  hover.classList.add("float");
  var hoverTarget = null;
  hover.addEventListener("click", function () {
    if (!hoverTarget) return;
    openMenu(hoverTarget.classList.contains("hh") ? hunkTarget(hoverTarget) : rowTarget(hoverTarget), hover);
  });
  function slot(h) { return h.tagName === "TR" ? h.lastElementChild : h; }
  document.addEventListener("mouseover", function (e) {
    var h = e.target.closest && e.target.closest(".diff .hh, table.blame tr[id^=L], tr[data-act]");
    if (!h || h === hoverTarget) return;
    hoverTarget = h;
    slot(h).appendChild(hover);
  });
  // Line selection: the trigger sits on the first selected line.
  function placeSel() {
    var first = document.querySelector(".src .l.sel, table.code tr.sel");
    var b = document.getElementById("actsel");
    if (!first) { if (b) b.remove(); return; }
    if (!b) {
      b = button("Actions for the selected lines (.)");
      b.id = "actsel";
      b.classList.add("float", "sel");
      b.addEventListener("click", function () { openMenu(current(), b); });
    }
    slot(first).appendChild(b);
  }
  window.addEventListener("hashchange", function () { setTimeout(placeSel, 0); });
  document.addEventListener("click", function (e) { if (e.target.closest && e.target.closest(".src, table.code")) setTimeout(placeSel, 0); });
  setTimeout(placeSel, 0);

  document.addEventListener("keydown", function (e) {
    if (e.key !== "." || e.metaKey || e.ctrlKey || e.altKey || menu) return;
    var t = e.target;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
    var anchor = document.getElementById("actsel") || document.querySelector(".actbtn.top");
    if (!anchor) return;
    openMenu(current(), anchor);
    e.preventDefault();
  });
})();
