// The Send window: opens prefilled from the menu target, the user adds a note, picks a
// target and sends. Targets come from GET {base}/_send/targets as
// [{id,label,parts:[{lit}|{ph}]}]. Nothing is stored server-side; the last target picked
// is remembered in localStorage. With no targets the only choice is "Copy as markdown".
(function () {
  "use strict";
  if (typeof document === "undefined") { module.exports = { enc: enc, fill: fill }; return; }
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
  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  function range(l) { return l ? "L" + l.start + (l.end !== l.start ? "-L" + l.end : "") : ""; }
  function titleOf(c) {
    if (c.kind === "commit" && c.subject) return c.subject;
    if (!c.path) return document.title;
    var l = c.lines;
    return c.path + (l ? ":L" + l.start + (l.end !== l.start ? "-" + l.end : "") : "");
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
    var back = el("div", "ov-back send-back");
    var box = el("form", "ov send");
    var title = el("input", "send-title");
    title.type = "text";
    title.value = titleOf(c);
    title.setAttribute("aria-label", "Title");
    var ctx = el("textarea", "send-ctx");
    ctx.value = "\n" + c.markdown;
    ctx.setAttribute("aria-label", "Context");
    ctx.spellcheck = false;
    // Grows with its content up to the CSS max-height. Browsers with field-sizing do this in CSS.
    if (!CSS.supports || !CSS.supports("field-sizing", "content")) {
      var grow = function () { ctx.style.height = "auto"; ctx.style.height = ctx.scrollHeight + "px"; };
      ctx.addEventListener("input", grow);
      requestAnimationFrame(grow);
    }
    var foot = el("div", "send-foot");
    var pick = el("select", "send-pick");
    pick.setAttribute("aria-label", "Target");
    var go = el("button", "send-go", "Send");
    go.type = "submit";
    var hint = el("span", "send-hint");
    hint.appendChild(el("kbd", "", (/Mac|iP/.test(navigator.platform) ? "\u2318" : "Ctrl") + "\u21B5"));
    foot.appendChild(pick);
    foot.appendChild(hint);
    foot.appendChild(go);
    box.appendChild(title);
    box.appendChild(ctx);
    box.appendChild(foot);
    back.appendChild(box);
    cur = { back: back, targets: [] };
    window.gtOverlay.open("send", back, box, { label: "Send", onClose: function () { if (cur && cur.back === back) cur = null; } });
    ctx.focus();
    ctx.setSelectionRange(0, 0);

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
    load().then(function (l) { if (cur && cur.back === back && Array.isArray(l)) fillPick(l); });

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
    back.addEventListener("mousedown", function (e) { if (e.target === back) close(); });
    box.addEventListener("keydown", function (e) {
      e.stopPropagation();
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); send(); }
    });
  }

  if (typeof module !== "undefined") module.exports = { enc: enc, fill: fill };
  window.gtSend = { open: open, enc: enc, fill: fill };
})();
