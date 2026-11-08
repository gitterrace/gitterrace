// The action menu: "..." on a line selection, hunk, blame line, search hit, hotspot row,
// commit and page, or "." for the current selection. Two built-ins work everywhere (copy
// permalink, copy as markdown); user actions from actions.toml appear only when the server
// embedded a token, and run through POST /_actions/run from a real click or key press.
(function () {
  "use strict";
  var body = document.body;
  // A static export (`git terrace export`) has no server to run or list actions.
  if (body && body.dataset.static !== undefined) return;
  var meta = function (n) { return document.querySelector('meta[name="' + n + '"]'); };
  var B = (meta("gt-base") || {}).content || "";
  var repoMeta = meta("gt-repo");
  var repoKey = repoMeta ? repoMeta.content : "";
  var repoId = repoMeta ? repoMeta.dataset.id : "";
  // links.style: "gitterrace" hands out gitterrace://<id>/<sha>/..., opened by `git terrace
  // open` without a running server; "go" the /go URL of this server.
  var linkStyle = repoMeta ? repoMeta.dataset.link : "go";
  var offMeta = meta("gt-off");
  var OFF = offMeta ? offMeta.content.split(" ") : [];
  var act = meta("gitterrace-actions");
  var token = act ? act.content : "";
  var mine = [];
  try { mine = act ? JSON.parse(act.dataset.menu || "[]") : []; } catch (e) { mine = []; }
  var rev = body.dataset.rev || "HEAD";
  var commit = body.dataset.commit || "";
  var DOCS = "https://github.com/gitterrace/gitterrace/blob/main/docs/actions.md";
  var MD_LINES = 40;
  // A mark shorter than this many characters carries a line or two of context with it.
  var SHORT = 20;

  // The server renders the note into the page, so it is there at first paint. Later edits of
  // actions.toml are fetched when the tab is shown or focused, and touch the page only when the
  // text changes.
  var noteEl = document.getElementById("actnote");
  function showNote(text) {
    if (!noteEl) return;
    var line = noteEl.firstElementChild;
    if (!text) { noteEl.hidden = true; return; }
    if (line.textContent !== text) line.textContent = text;
    noteEl.hidden = false;
  }
  function pollNote() {
    if (document.hidden || OFF.indexOf("actions") >= 0) return;
    fetch("/_actions/status", { cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (j) { if (j) showNote(j.note); })
      .catch(function () {});
  }
  pollNote();
  // No timer: a poll per tab holds connections that WebKit's 6 per host cap needs for navigation.
  document.addEventListener("visibilitychange", pollNote);
  window.addEventListener("focus", pollNote);

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  function toast(msg, sticky) { window.gtToast(msg, sticky); }
  function copy(text, done) { return window.gtCopy(text, done, menuOpener); }
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
  // A word character in any script: the selection snaps to whole words of these.
  var WORD = /[\p{L}\p{N}_]/u;
  // The range widened so neither end cuts a word in half. Only an end that falls between two
  // word characters moves; one already at a space or punctuation stays.
  function snap(range) {
    var r = range.cloneRange();
    var a = r.startContainer, i = r.startOffset;
    if (a.nodeType === 3 && i > 0 && i < a.data.length && WORD.test(a.data[i - 1]) && WORD.test(a.data[i])) {
      while (i > 0 && WORD.test(a.data[i - 1])) i--;
      r.setStart(a, i);
    }
    var z = r.endContainer, j = r.endOffset;
    if (z.nodeType === 3 && j > 0 && j < z.data.length && WORD.test(z.data[j - 1]) && WORD.test(z.data[j])) {
      while (j < z.data.length && WORD.test(z.data[j])) j++;
      r.setEnd(z, j);
    }
    return r;
  }
  // The source lines of rendered Markdown a range touches, read from the blocks' data-line
  // spans. A block the range only grazes (its end at the block's very start) does not count.
  // Each marked piece of text counts with its innermost block, so a list item or quote that
  // holds the marked block does not widen the range to its own lines.
  function mdLines(r, root) {
    var lo = 0, hi = 0;
    var walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (var t = walk.nextNode(); t; t = walk.nextNode()) {
      if (!r.intersectsNode(t)) continue;
      var a = t === r.startContainer ? r.startOffset : 0;
      var z = t === r.endContainer ? r.endOffset : t.data.length;
      if (!t.data.slice(a, z).trim()) continue;
      var el = t.parentElement && t.parentElement.closest("[data-line]");
      if (!el || !root.contains(el)) continue;
      var m = /^(\d+)-(\d+)$/.exec(el.getAttribute("data-line"));
      if (!m) continue;
      if (!lo || +m[1] < lo) lo = +m[1];
      if (+m[2] > hi) hi = +m[2];
    }
    return lo ? { start: lo, end: hi } : null;
  }
  // The raw text of a file at this commit, fetched once when prose of it is first marked, so
  // the quote is the source and not the rendered text. Null until it arrives.
  var raw = {};
  function rawText(path) {
    var key = (commit || rev) + ":" + path;
    if (!(key in raw) && OFF.indexOf("raw") < 0) {
      raw[key] = null;
      fetch(B + "/raw/" + encodeURIComponent(commit || rev) + "/" + encPath(path), { credentials: "same-origin" })
        .then(function (r) { return r.ok ? r.text() : null; })
        .then(function (t) { raw[key] = t; }, function () {});
    }
    return raw[key];
  }
  // Lines start..end of the file exactly as stored, line ends left out, the end clamped to
  // the last line. Null while the raw text is still on its way.
  function sourceLines(path, l) {
    var t = rawText(path);
    if (t == null) return null;
    var all = t.replace(/\r?\n$/, "").split(/\r?\n/);
    return l.start <= all.length ? all.slice(l.start - 1, Math.min(l.end, all.length)).join("\n") : null;
  }
  // Fetched ahead, so the first mark already quotes the source.
  var mdSrc = document.querySelector(".markdown-body[data-src]");
  if (mdSrc) rawText(mdSrc.getAttribute("data-src"));
  // What the reader marked with the mouse, if anything: inside a code view the lines it spans
  // and the exact text; in rendered Markdown the source lines; elsewhere the text alone. The
  // ends snap to whole words. Null when nothing is marked.
  function marked() {
    var s = window.getSelection && window.getSelection();
    if (!s || s.isCollapsed || !s.rangeCount) return null;
    var r = snap(s.getRangeAt(0));
    var text = r.toString();
    if (!text.trim()) return null;
    var md = r.commonAncestorContainer;
    md = (md.nodeType === 1 ? md : md.parentElement);
    md = md && md.closest && md.closest(".markdown-body[data-src]");
    if (md) {
      var ml = mdLines(r, md);
      if (ml) return { md: md.getAttribute("data-src"), lines: ml, text: text };
    }
    var line = function (n) {
      var e = n && (n.nodeType === 1 ? n : n.parentElement);
      return e && e.closest && e.closest(".src .l, table.code tr[id^=L], table.blame tr[id^=L]");
    };
    var a = line(r.startContainer), b = line(r.endContainer);
    // A selection that ends at the very start of the next line does not include that line.
    if (b && b !== a && r.endOffset === 0 && b.previousElementSibling) b = b.previousElementSibling;
    if (a && b && /^L\d+$/.test(a.id) && /^L\d+$/.test(b.id)) {
      var x = +a.id.slice(1), y = +b.id.slice(1);
      return { lines: { start: Math.min(x, y), end: Math.max(x, y) }, text: text.replace(/\n$/, "") };
    }
    // Prose outside Markdown: a short mark brings its block along as context.
    var block = a || (r.startContainer.nodeType === 1 ? r.startContainer : r.startContainer.parentElement);
    block = block && block.closest && block.closest("p, li, pre, td, h1, h2, h3, h4, h5, h6, blockquote");
    var ctx = text.trim().length < SHORT && block ? block.textContent.trim() : "";
    return { text: text, context: ctx && ctx !== text.trim() ? ctx : "" };
  }
  // Lines around a short mark: one more on each side, clamped to the file's first line.
  function widen(l) { return { start: Math.max(1, l.start - 1), end: l.end + 1 }; }
  function current() {
    var path = filePath(), lines = selection(), m = marked();
    // Marked rendered Markdown: the link and the quote are the source lines it came from.
    if (m && m.md) {
      var short = m.text.trim().length < SHORT;
      var quote = sourceLines(m.md, short ? widen(m.lines) : m.lines);
      return { kind: "line", path: m.md, lines: m.lines, quote: quote || m.text, marked: m.text };
    }
    if (path && m && m.lines) {
      var wide = m.text.trim().length < SHORT ? lineText(widen(m.lines)) : "";
      // Across rows the selection's text has no line breaks, so the rows' own text names it.
      var named = m.lines.end > m.lines.start ? lineText(m.lines) : m.text;
      return { kind: "line", path: path, lines: m.lines, text: wide || m.text, marked: named };
    }
    if (path && lines) return { kind: "line", path: path, lines: lines, text: lineText(lines) };
    var t;
    if (path) t = { kind: "file", path: path };
    else if (local().indexOf("/commit/") === 0) t = commitTarget();
    else t = { kind: "page" };
    // Marked prose (a commit message) travels as the quote, not the whole page.
    if (m) { t.quote = m.context || m.text; t.marked = m.text; }
    return t;
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
  function shortSha() { return commit.slice(0, 8); }
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
  // The same string the payload's `markdown` field carries, ending in the machine source line.
  function markdown(t) {
    var link = permalink(t);
    var src = window.gtSend ? window.gtSend.sourceLine(repoKey, commit, t.path || null, t.lines || null) + "\n" : "";
    // Marked rendered Markdown is source text, fenced like code so its bytes arrive as stored.
    if (t.path && t.text == null && t.lines && t.quote != null && t.marked != null) t = { path: t.path, lines: t.lines, text: t.quote, subject: t.subject };
    // Other marked prose rides along as a quote under the link.
    var quote = t.quote ? "\n" + t.quote.split("\n").slice(0, MD_LINES).map(function (l) { return "> " + l; }).join("\n") + "\n" : "";
    if (!t.path) return "**" + (t.subject || document.title) + "** @ " + shortSha() + "\n" + (t.author || "") + "\n" + link + "\n" + quote + src;
    var md = "**" + ref(t) + "** @ " + shortSha() + "\n" + link + "\n" + quote;
    if (t.text != null && t.lines) {
      var snippet = t.text.split("\n").slice(0, MD_LINES).join("\n");
      var runs = snippet.match(/`+/g) || [];
      var longest = runs.reduce(function (m, r) { return Math.max(m, r.length); }, 2);
      var fence = new Array(longest + 2).join("`");
      var dot = t.path.lastIndexOf(".");
      var lang = dot > t.path.lastIndexOf("/") ? t.path.slice(dot + 1) : "";
      md += fence + lang + "\n" + snippet + "\n" + fence + "\n";
    }
    return md + src;
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

  function fits(a, t) { return a.kinds.indexOf(t.kind) >= 0; }

  // ---- the menu -------------------------------------------------------------
  var menu = null;
  var ov = window.gtOverlay;
  function closeMenu() { if (menu) ov.close(); }
  // The button that opened the action menu: the menu closes on a copy, so the opener is what
  // stays on screen to flash. Null when the menu was opened by keyboard with no anchor.
  var menuOpener = null;
  function openMenu(t, anchor) {
    closeMenu();
    menuOpener = anchor || null;
    var back = el("div", "ov-back actmenu-back");
    var box = el("div", "ov anchored actmenu");
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
    function head(text) { var h = el("li", "grp", text); h.setAttribute("role", "presentation"); list.appendChild(h); }
    head("Copy");
    add("Copy permalink", "p", function () { copy(permalink(t), "Permalink copied"); closeMenu(); });
    add("Copy as markdown", "m", function () { copy(markdown(t), "Markdown copied"); closeMenu(); });
    var keys = { p: 0, m: 1 };
    // Send to reads its targets from the actions routes: with actions off it is not offered.
    if (OFF.indexOf("actions") < 0) {
      head("Share");
      add("Send to\u2026", "s", function () {
        closeMenu();
        if (window.gtSend) window.gtSend.open({ kind: t.kind, path: t.path || "", lines: t.lines || null, subject: t.subject || "", sha: commit, link: permalink(t), markdown: markdown(t), text: t.text || "", marked: t.marked || t.quote || "" });
      });
      keys.s = 2;
    }
    // A page may name one place its menu opens (compare: the head commit).
    var open = document.querySelector('meta[name="gt-open"]');
    if (open) {
      head("Open");
      add(open.dataset.label || "Open", "", function () { closeMenu(); location.href = open.content; });
    }
    // Every view lists the same entries in the same order (the server's one list); an action
    // the selection does not fit stays, disabled, with the server's reason as its tip.
    if (mine.length) { list.appendChild(el("li", "sep")); head("Your actions"); }
    mine.forEach(function (a) {
      if (a.key && !(a.key in keys)) keys[a.key] = items.length;
      var on = fits(a, t);
      var li = add(a.label, a.key, function (e, li) { if (on) run(a, t, e, li); else toast(a.label + ": " + a.why); });
      if (!on) { li.classList.add("off"); li.setAttribute("aria-disabled", "true"); li.setAttribute("data-tip", a.why); }
    });
    // The server's own settings, offered where the page says the settings page exists.
    var settings = document.querySelector('meta[name="gt-settings"]');
    if (settings) {
      list.appendChild(el("li", "sep"));
      head("Server");
      add("Settings\u2026", ",", function () { closeMenu(); location.href = settings.content; });
      keys[","] = items.length - 1;
    }
    if (!mine.length) {
      var hint = el("li", "hint");
      var link = el("a", "", "Add your own actions");
      link.href = DOCS;
      link.target = "_blank";
      link.rel = "noopener";
      hint.appendChild(link);
      list.appendChild(hint);
    }
    // The letters beside the items work only while this menu is open, not as page shortcuts.
    var note = el("li", "keynote", "Keys work while this menu is open");
    note.setAttribute("role", "presentation");
    list.appendChild(note);
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
      if (e.key === "Escape" || e.key === "Tab") return;
      if (e.key === "ArrowDown") focus(sel + 1);
      else if (e.key === "ArrowUp") focus(sel - 1);
      else if (e.key === "Enter" || e.key === " ") fns[sel](e, items[sel]);
      else if (e.key in keys) { focus(keys[e.key]); fns[keys[e.key]](e, items[keys[e.key]]); }
      else return;
      e.preventDefault();
    });
    menu = { back: back };
    ov.open("actmenu", back, box, { menu: true, label: "Actions", from: anchor, onClose: function () { if (menu && menu.back === back) menu = null; } });
    var r = anchor.getBoundingClientRect();
    box.style.left = Math.max(8, Math.min(r.left, window.innerWidth - 300)) + "px";
    box.style.top = Math.min(r.bottom + 6, window.innerHeight - 200) + "px";
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
    // Copy permalink lives in this menu and on y: no separate icon competing for the bar.
    theme.parentNode.insertBefore(top, theme);
  }
  // Commit page: next to the sha.
  var sha = document.querySelector("p.meta > a.sha");
  if (sha && local().indexOf("/commit/") === 0) {
    var cb = button("Actions for this commit");
    cb.addEventListener("click", function () { openMenu(commitTarget(), cb); });
    sha.parentNode.insertBefore(cb, sha.nextSibling);
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

  // Marked text anywhere: a trigger at the end of the selection. It is fixed to the window
  // and lives outside the text, so it never becomes part of what is copied. The target is
  // read on mousedown, before the press would collapse the selection.
  var markBtn = button("Actions for the marked text (.)");
  markBtn.id = "actmark";
  markBtn.classList.add("mark");
  markBtn.hidden = true;
  document.body.appendChild(markBtn);
  var markTarget = null;
  markBtn.addEventListener("mousedown", function (e) { markTarget = current(); e.preventDefault(); });
  markBtn.addEventListener("click", function () { openMenu(markTarget || current(), markBtn); markTarget = null; });
  function placeMark() {
    var s = window.getSelection();
    var m = !menu && marked();
    if (!m || !s.rangeCount) { markBtn.hidden = true; return; }
    var rects = s.getRangeAt(0).getClientRects();
    var r = rects[rects.length - 1];
    if (!r) { markBtn.hidden = true; return; }
    markBtn.hidden = false;
    markBtn.style.left = Math.min(r.right + 4, window.innerWidth - 32) + "px";
    markBtn.style.top = Math.max(4, r.top + r.height / 2 - 11) + "px";
  }
  var markTimer = null;
  document.addEventListener("selectionchange", function () { clearTimeout(markTimer); markTimer = setTimeout(placeMark, 120); });
  window.addEventListener("scroll", function () { if (!markBtn.hidden) placeMark(); }, { passive: true });

  document.addEventListener("keydown", function (e) {
    if (e.key !== "." || e.metaKey || e.ctrlKey || e.altKey || ov.name) return;
    if (window.gtKeysOff && window.gtKeysOff()) return;
    var t = e.target;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
    var anchor = (!markBtn.hidden && markBtn) || document.getElementById("actsel") || document.querySelector(".actbtn.top");
    if (!anchor) return;
    openMenu(current(), anchor);
    e.preventDefault();
  });

  // Chords: "a" then an action's key runs it from the page, no menu. The loader refuses keys
  // that clash with the menu's, the page's or each other. This listener runs in the capture
  // phase and marks the armed second key handled, so nav.js never also acts on it; the arming
  // "a" is skipped when nav.js already took it as the second key of its own "g" chord.
  var armed = false, armTimer = null;
  function disarm() { armed = false; clearTimeout(armTimer); }
  document.addEventListener("keydown", function (e) {
    if (e.metaKey || e.ctrlKey || e.altKey || ov.name) return;
    if (window.gtKeysOff && window.gtKeysOff()) return;
    var t = e.target;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable)) return;
    if (armed) {
      disarm();
      e.preventDefault();
      var tg = current();
      var hit = mine.filter(function (a) { return a.key === e.key; })[0];
      if (hit && fits(hit, tg)) run(hit, tg, e, null);
      else if (hit) toast(hit.label + ": " + hit.why);
      return;
    }
    // nav.js's "g" chord takes the next key, whatever it is, and nav.js arms it on "g".
    if (ov.chord || e.key === "g") return;
    if (e.key === "a" && token && mine.some(function (a) { return a.key; })) {
      armed = true;
      armTimer = setTimeout(disarm, 1200);
      toast("a, then " + mine.filter(function (a) { return a.key; }).map(function (a) { return a.key + " " + a.label; }).join(", "));
    }
  }, true);
})();
