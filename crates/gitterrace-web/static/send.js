// The Send window: opens prefilled from the menu target, the user adds a note, picks a
// target and sends. Targets come from GET {base}/_send/targets as
// [{id,label,parts:[{lit}|{ph}]}]. Nothing is stored server-side; the last target picked
// is remembered in localStorage. With no targets the only choice is "Copy as markdown".
(function () {
  "use strict";
  if (typeof document === "undefined") { module.exports = { enc: enc, fill: fill, sourceLine: sourceLine, titleOf: titleOf }; return; }
  var el = window.gtNav.el;
  var meta = document.querySelector('meta[name="gt-base"]');
  var B = meta ? meta.content : "";
  var KEY = "gitterrace-send-target";
  var COPY = "__copy";
  var cur = null;

  // Mirrors the Rust fill(): RFC 3986 unreserved kept, every other UTF-8 byte as %XX.
  function enc(s) {
    var out = "", bytes = new TextEncoder().encode(String(s == null ? "" : s));
    for (var i = 0; i < bytes.length; i++) {
      var b = bytes[i];
      if ((b >= 48 && b <= 57) || (b >= 65 && b <= 90) || (b >= 97 && b <= 122) || b === 45 || b === 46 || b === 95 || b === 126) out += String.fromCharCode(b);
      else out += "%" + (b < 16 ? "0" : "") + b.toString(16).toUpperCase();
    }
    return out;
  }
  function fill(parts, vals) {
    return parts.map(function (p) { return "lit" in p ? p.lit : enc(vals[p.ph] || ""); }).join("");
  }
  function range(l) { return l ? "L" + l.start + (l.end !== l.start ? "-L" + l.end : "") : ""; }
  // Mirrors the Rust source_line(): the last, machine-read line of every note.
  function sourceLine(repo, sha, path, lines) {
    var flat = function (s) { return String(s).replace(/[\u0000-\u001f\u007f-\u009f]/g, " "); };
    var out = "source: " + flat(repo) + "@" + sha;
    if (path) {
      // `%` and `#` encoded, so the only `#` on the line is the one before the range.
      out += ":" + flat(path).replace(/%/g, "%25").replace(/#/g, "%23");
      if (lines) out += "#L" + lines.start + "-L" + lines.end;
    }
    return out;
  }
  // The note's first line when there is one; else the file and the first words of what was
  // marked; never the bare path, which every send from one file would share.
  function titleOf(c, note) {
    // Inside the function: under node the file returns before any var is set.
    var TITLE_WORDS = 8;
    var first = String(note || "").split("\n")[0].trim();
    if (first) return first;
    if (c.kind === "commit" && c.subject) return c.subject;
    var words = String(c.marked || c.text || "").replace(/\s+/g, " ").trim().split(" ").filter(Boolean);
    var what = words.slice(0, TITLE_WORDS).join(" ") + (words.length > TITLE_WORDS ? "\u2026" : "");
    if (!c.path) return what || (typeof document === "undefined" ? "" : document.title);
    var file = c.path.slice(c.path.lastIndexOf("/") + 1);
    var l = c.lines;
    return file + ": " + (what || (l ? "lines " + l.start + (l.end !== l.start ? "-" + l.end : "") : "whole file"));
  }
  function lastUsed() { try { return localStorage.getItem(KEY); } catch (e) { return null; } }
  function remember(id) { try { localStorage.setItem(KEY, id); } catch (e) {} }

  function close() { if (cur) window.gtOverlay.close(); }
  function load() {
    return fetch(B + "/_send/targets", { cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : []; })
      .catch(function () { return []; });
  }

  function open(c) {
    var dlg = el("dialog", "ov-back send-back");
    var box = el("form", "ov send");
    dlg.appendChild(box);
    // Named fields under a named head: with targets set up the window is a small form, and a
    // bare input and text box gave no hint which was the title and which the notes.
    var head = el("h2", "send-head", "Send to");
    head.id = "send-head";
    dlg.setAttribute("aria-labelledby", "send-head");
    var title = el("input", "send-title");
    title.type = "text";
    title.id = "send-title";
    title.value = titleOf(c);
    var titleLab = el("label", "send-lab", "Title");
    titleLab.htmlFor = "send-title";
    var ctx = el("textarea", "send-ctx");
    ctx.id = "send-ctx";
    ctx.value = "\n" + c.markdown;
    var ctxLab = el("label", "send-lab", "Notes");
    ctxLab.htmlFor = "send-ctx";
    ctx.spellcheck = false;
    // Grows with its content up to the CSS max-height. Browsers with field-sizing do this in CSS.
    if (!CSS.supports || !CSS.supports("field-sizing", "content")) {
      var grow = function () { ctx.style.height = "auto"; ctx.style.height = ctx.scrollHeight + "px"; };
      ctx.addEventListener("input", grow);
      requestAnimationFrame(grow);
    }
    var foot = el("div", "send-foot");
    var pick = el("select", "send-pick sm");
    pick.setAttribute("aria-label", "Target");
    var go = el("button", "send-go primary sm", "Send");
    go.type = "submit";
    var cancel = el("button", "send-cancel sm", "Cancel");
    cancel.type = "button";
    cancel.addEventListener("click", function () { close(); });
    var hint = el("span", "send-hint", (/Mac|iP/.test(navigator.platform) ? "Cmd" : "Ctrl") + "+Enter to send");
    foot.appendChild(pick);
    foot.appendChild(hint);
    foot.appendChild(cancel);
    foot.appendChild(go);
    box.appendChild(head);
    box.appendChild(titleLab);
    box.appendChild(title);
    box.appendChild(ctxLab);
    box.appendChild(ctx);
    box.appendChild(foot);
    cur = { box: dlg, targets: [] };
    window.gtOverlay.open("send", dlg, box, { label: "Send", onClose: function () { if (cur && cur.box === dlg) cur = null; } });
    ctx.focus();
    ctx.setSelectionRange(0, 0);
    // The title follows the note's first line until the user edits the title itself.
    var auto = title.value;
    ctx.addEventListener("input", function () {
      if (title.value !== auto) return;
      var top = ctx.value.slice(0, ctx.value.indexOf("\n") < 0 ? ctx.value.length : ctx.value.indexOf("\n"));
      // The note is what sits above the prefilled context; that context is not a note.
      auto = title.value = titleOf(c, ctx.value.indexOf(c.markdown) > 0 ? top : "");
    });

    function fillPick(list) {
      cur.targets = list;
      pick.textContent = "";
      list.forEach(function (t) { var o = el("option", "", t.label || t.id); o.value = t.id; pick.appendChild(o); });
      var o = el("option", "", "Copy as markdown");
      o.value = COPY;
      pick.appendChild(o);
      var last = lastUsed();
      pick.value = list.some(function (t) { return t.id === last; }) ? last : (list.length ? list[0].id : COPY);
      if (pick.value === COPY && last !== COPY && list.length) pick.value = list[0].id;
      if (last === COPY) pick.value = COPY;
    }
    fillPick([]);
    load().then(function (l) { if (cur && cur.box === dlg && Array.isArray(l)) fillPick(l); });

    function send() {
      var id = pick.value;
      remember(id);
      if (id === COPY) {
        var text = title.value + "\n\n" + ctx.value;
        window.gtCopy(text, "Copied with context").then(function () { close(); });
        return;
      }
      var t = cur.targets.filter(function (x) { return x.id === id; })[0];
      if (!t) return;
      var l = c.lines;
      var rm = document.querySelector('meta[name="gt-repo"]');
      var url = fill(t.parts, {
        title: title.value, notes: ctx.value, permalink: c.link, path: c.path,
        lines: range(c.lines), sha: c.sha, repo: rm ? rm.content : ""
      });
      close();
      if (/^https?:/i.test(url)) window.open(url, "_blank", "noopener");
      else location.href = url;
    }
    box.addEventListener("submit", function (e) { e.preventDefault(); send(); });
    box.addEventListener("keydown", function (e) {
      e.stopPropagation();
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); send(); }
    });
  }

  if (typeof module !== "undefined") module.exports = { enc: enc, fill: fill, sourceLine: sourceLine, titleOf: titleOf };
  window.gtSend = { open: open, enc: enc, fill: fill, sourceLine: sourceLine };
})();
