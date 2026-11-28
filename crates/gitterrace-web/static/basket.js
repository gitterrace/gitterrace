// The review basket: the count on the header button, the panel that lists it, the note window
// of "Add to review" and Send all. The notes and where the code is now come from the server
// (GET /_findings); the page only shows them, as text. Send all names the entries by id: the
// server reads the notes from its own basket, so this page cannot send words of its own.
// app.js loads this only where the server rendered the button.
(function () {
  "use strict";
  var el = window.gtNav.el, B = window.gtNav.B;
  var ov = window.gtOverlay;
  var btn = document.getElementById("basketbtn");
  // The server renders the button with a short accessible name, so it is named before this file
  // loads (axe can run first). It sits in no form, so it needs no type. The icon, the tooltip and
  // the full label (with the entry count) are set here.
  if (btn && !btn.firstChild) {
    btn.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true">' +
      '<path d="M5 5h14v14H5zM9 10h6M9 14h6"/></svg>';
    btn.setAttribute("aria-haspopup", "dialog");
    btn.setAttribute("data-tip", "Review basket");
  }
  // Most findings one review payload carries (gitterrace-actions MAX_REVIEW_FINDINGS, held equal
  // by a test). A larger basket is sent in chosen parts, never trimmed.
  var SEND_MAX = 500;
  // Longest note, in UTF-8 bytes (gitterrace-index MAX_NOTE, held equal by a test). The server
  // refuses a longer one, so the box counts bytes the way it does and never lets one through.
  var NOTE_MAX = 2000;
  var badge = null;
  var cur = null;

  function json(method, url, body) {
    var o = { method: method, cache: "no-store", credentials: "same-origin", headers: {} };
    if (body !== undefined) {
      o.headers["Content-Type"] = "application/json";
      o.body = JSON.stringify(body);
    }
    return fetch(url, o).then(function (r) {
      return r.json().catch(function () { return {}; }).then(function (j) {
        if (!r.ok) throw new Error(j.first_line || String(r.status));
        return j;
      });
    });
  }
  function count(n) {
    if (!btn) return;
    if (!badge) {
      badge = el("span", "basketn");
      badge.setAttribute("aria-hidden", "true");
      btn.appendChild(badge);
    }
    badge.textContent = n > 99 ? "99+" : String(n);
    badge.hidden = n === 0;
    btn.setAttribute("aria-label", n ? "Review basket, " + n + " entries" : "Review basket");
  }
  // Bytes the server would store for `text`: control characters other than a line break or tab
  // are dropped there, and the rest is counted as UTF-8.
  function noteBytes(text) {
    var kept = text.replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/g, "");
    return new TextEncoder().encode(kept).length;
  }
  function refresh() {
    return json("GET", B + "/_findings?count=1").then(function (j) { count(j.count || 0); },
      function () {});
  }
  function range(l) {
    return l ? ":" + l.start + (l.end !== l.start ? "-" + l.end : "") : "";
  }
  function where(f) {
    var a = f.anchor || {};
    var now = a.now_path && a.now_path !== f.path ? " (now " + a.now_path + ")" : "";
    var moved = a.now_lines ? ", now at " + a.now_lines.start + "-" + a.now_lines.end : "";
    return (f.path || f.query || f.kind) + range(f.lines) + now + moved;
  }
  function open(name, label, dlg, box) {
    return ov.open(name, dlg, box, {
      label: label, anchor: btn,
      onClose: function () { if (cur === dlg) cur = null; }
    });
  }
  function shell(id, title) {
    var dlg = el("dialog", "ov-back basket-back");
    var box = el("div", "ov basket");
    var h = el("h2", "basket-head", title);
    h.id = id;
    dlg.setAttribute("aria-labelledby", id);
    dlg.appendChild(box);
    box.appendChild(h);
    return { dlg: dlg, box: box };
  }
  function button(text, cls, fn) {
    var b = el("button", cls || "sm", text);
    b.type = "button";
    b.addEventListener("click", fn);
    return b;
  }
  function reviewActions() {
    var m = document.querySelector('meta[name="gitterrace-actions"]');
    var list = [];
    try { list = m ? JSON.parse(m.dataset.menu || "[]") : []; } catch (e) { list = []; }
    return list.filter(function (a) { return a.kinds.indexOf("review") >= 0; });
  }

  // ---- add -----------------------------------------------------------------
  // c: {kind, path?, lines?, query?, rev, sha?, label}. Asks for the note, then saves.
  function add(c) {
    var s = shell("basket-add-head", "Add to review");
    var what = el("p", "basket-what", c.label);
    var lab = el("label", "basket-lab", "Note: why this matters");
    lab.htmlFor = "basket-note";
    var note = el("textarea", "basket-note");
    note.id = "basket-note";
    note.spellcheck = false;
    var msg = el("span", "basket-msg muted");
    msg.setAttribute("role", "status");
    var foot = el("div", "basket-foot");
    var go = el("button", "primary sm", "Add");
    go.type = "submit";
    function size() {
      var n = noteBytes(note.value);
      var over = n > NOTE_MAX;
      go.disabled = over;
      msg.textContent = over || n > NOTE_MAX * 0.8 ? n + " of " + NOTE_MAX + " bytes"
        + (over ? ": too long, shorten the note" : "") : "";
    }
    note.addEventListener("input", size);
    foot.appendChild(msg);
    foot.appendChild(button("Cancel", "sm", function () { ov.close(); }));
    foot.appendChild(go);
    var form = el("form", "basket-form");
    [what, lab, note, foot].forEach(function (n) { form.appendChild(n); });
    s.box.appendChild(form);
    form.addEventListener("submit", function (e) {
      e.preventDefault();
      if (noteBytes(note.value) > NOTE_MAX) { size(); return; }
      go.disabled = true;
      var body = { kind: c.kind, rev: c.rev, note: note.value };
      ["path", "lines", "query", "sha"].forEach(function (k) { if (c[k]) body[k] = c[k]; });
      json("POST", B + "/_findings", body).then(function () {
        ov.close();
        window.gtToast("Added to the review basket");
        refresh();
      }, function (err) {
        go.disabled = false;
        msg.textContent = "Not added: " + err.message;
      });
    });
    s.box.addEventListener("keydown", function (e) {
      e.stopPropagation();
      if (e.key !== "Enter" || !(e.metaKey || e.ctrlKey)) return;
      e.preventDefault();
      form.requestSubmit();
    });
    cur = s.dlg;
    if (!open("basket", "Add to review", s.dlg, s.box)) return;
    note.focus();
  }

  // ---- the panel -------------------------------------------------------------
  function row(f, again, picked, changed) {
    var li = el("li", "basket-row");
    li.dataset.id = f.id;
    var top = el("div", "basket-top");
    var tick = el("input", "basket-tick");
    tick.type = "checkbox";
    tick.checked = picked.has(f.id);
    tick.setAttribute("aria-label", "Send " + where(f));
    tick.addEventListener("change", function () {
      if (tick.checked) picked.add(f.id); else picked.delete(f.id);
      changed();
    });
    top.appendChild(tick);
    top.appendChild(el("span", "basket-where", where(f)));
    var chip = el("span", "basket-chip", f.anchor.status);
    chip.dataset.status = f.anchor.status;
    top.appendChild(chip);
    var rm = button("Remove", "ghost sm", function () {
      rm.disabled = true;
      json("DELETE", B + "/_findings", { ids: [f.id] }).then(again, function () {
        rm.disabled = false;
      });
    });
    rm.setAttribute("aria-label", "Remove " + where(f));
    top.appendChild(rm);
    li.appendChild(top);
    if (f.note) li.appendChild(el("p", "basket-text", f.note));
    return li;
  }
  function sendAll(ids, pick, msg, go) {
    var chosen = pick.value;
    go.disabled = true;
    var rm = document.querySelector('meta[name="gt-repo"]');
    var loc = {
      repo: rm ? rm.content : "", rev: document.body.dataset.rev || "HEAD", findings: ids
    };
    fetch("/_actions/run", {
      method: "POST", credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ action: chosen, kind: "review", locator: loc })
    }).then(function (r) {
      return r.json().catch(function () { return { ok: false, first_line: "HTTP " + r.status }; });
    })
      .then(function (res) {
        var said = res.first_line || (res.ok ? "Sent." : "failed");
        msg.textContent = res.ok ? said : "Not sent: " + said;
      }, function () { msg.textContent = "Not sent: no response"; })
      .then(function () { go.disabled = false; });
  }
  function panel() {
    if (cur) { ov.close(); return; }
    var s = shell("basket-head", "Review basket");
    var list = el("ul", "basket-list");
    var note = el("p", "basket-empty muted");
    var msg = el("span", "basket-msg muted");
    msg.setAttribute("role", "status");
    msg.setAttribute("aria-live", "polite");
    var foot = el("div", "basket-foot");
    var acts = reviewActions();
    var pick = el("select", "basket-pick sm");
    pick.setAttribute("aria-label", "Action");
    acts.forEach(function (a) {
      var o = el("option", "", a.label);
      o.value = a.id;
      pick.appendChild(o);
    });
    var go = el("button", "primary sm", "Send all");
    var first = button("Select first " + SEND_MAX, "sm basket-first", function () {
      picked.clear();
      ids.slice(0, SEND_MAX).forEach(function (id) { picked.add(id); });
      // Only the ticks change: no server data did, so no GET and no anchors to recompute.
      Array.prototype.forEach.call(list.querySelectorAll(".basket-tick"), function (t) {
        t.checked = picked.has(t.closest("li").dataset.id);
      });
      sync();
    });
    first.hidden = true;
    var picked = new Set();
    var known = new Set();
    go.type = "button";
    var clear = button("Clear", "sm", function () {
      json("DELETE", B + "/_findings", { ids: [] }).then(load, function () {});
    });
    var hint = el("span", "basket-msg muted",
      "Add an action with kinds = [\"review\"] to send the basket.");
    if (acts.length) { foot.appendChild(pick); foot.appendChild(msg); } else foot.appendChild(hint);
    foot.appendChild(clear);
    if (acts.length) { foot.appendChild(first); foot.appendChild(go); }
    foot.appendChild(button("Close", "sm", function () { ov.close(); }));
    [note, list, foot].forEach(function (n) { s.box.appendChild(n); });
    var ids = [];
    go.addEventListener("click", function (e) {
      var chosen = ids.filter(function (id) { return picked.has(id); });
      if (e.isTrusted && chosen.length && chosen.length <= SEND_MAX) sendAll(chosen, pick, msg, go);
    });
    // Send all, or Send n of m once some are unticked; held back above what one send takes.
    function sync() {
      var n = ids.filter(function (id) { return picked.has(id); }).length;
      go.textContent = n === ids.length ? "Send all" : "Send " + n + " of " + ids.length;
      var over = n > SEND_MAX;
      go.disabled = n === 0 || over;
      first.hidden = ids.length <= SEND_MAX;
      if (over) {
        msg.textContent = n + " chosen: one send takes at most " + SEND_MAX
          + ". Untick some or select the first " + SEND_MAX + ".";
      }
      else if (msg.dataset.over) msg.textContent = "";
      if (over) msg.dataset.over = "1"; else delete msg.dataset.over;
    }
    function load() {
      return json("GET", B + "/_findings").then(function (j) {
        ids = j.findings.map(function (f) { return f.id; });
        // An entry new to the panel starts ticked; a choice the user made stays.
        ids.forEach(function (id) { if (!known.has(id)) { known.add(id); picked.add(id); } });
        picked.forEach(function (id) { if (ids.indexOf(id) < 0) picked.delete(id); });
        list.textContent = "";
        j.findings.forEach(function (f) { list.appendChild(row(f, load, picked, sync)); });
        note.hidden = ids.length > 0 && j.persisted;
        note.textContent = !j.persisted
          ? "The basket cannot be kept: there is no writable cache folder."
          : "Nothing here yet. Use Add to review in the . menu on a line, file or commit.";
        clear.disabled = !ids.length;
        sync();
        count(ids.length);
      }, function () { note.hidden = false; note.textContent = "The basket could not be read."; });
    }
    s.box.addEventListener("keydown", function (e) { e.stopPropagation(); });
    cur = s.dlg;
    if (!open("basket", "Review basket", s.dlg, s.box)) return;
    load();
  }

  if (btn) {
    btn.addEventListener("click", panel);
    // A gutter marker on a file row (blob_marks.rs) opens the panel; without script it is a link
    // to its line.
    document.addEventListener("click", function (e) {
      if (!e.target.closest || !e.target.closest(".fm-link")) return;
      e.preventDefault();
      if (!cur) panel();
    });
    refresh();
  }
  window.gtBasket = { add: add, refresh: refresh };
})();
