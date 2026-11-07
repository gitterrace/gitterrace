// The settings page. The server renders the form and owns TOML: this file collects the form's
// changes, asks the server to check them, switches between Form and Plain through the server's
// text, and saves. The browser never serialises TOML, so the two views cannot disagree.
(function () {
  "use strict";
  var meta = document.querySelector('meta[name="gitterrace-settings"]');
  if (!meta) return;
  var token = meta.content;
  var file = meta.dataset.file;
  var stamp = meta.dataset.stamp;
  var form = document.getElementById("setform");
  var plain = document.getElementById("setplain");
  var text = document.getElementById("settext");
  var gut = document.getElementById("setgut");
  var errs = document.getElementById("seterrs");
  var note = document.getElementById("setnote");
  var save = document.getElementById("setsave");
  var summary = document.getElementById("seterrsum");
  var saving = false;
  var view = form && !form.hidden ? "form" : "plain";
  var saved = text ? text.value : "";
  var timer = 0;

  function call(method, url, body) {
    return fetch(url, {
      method: method,
      credentials: "same-origin",
      headers: { "content-type": "application/json", "x-gitterrace-token": token },
      body: body ? JSON.stringify(body) : undefined
    }).then(function (r) {
      return r.text().then(function (t) {
        var j = null;
        try { j = JSON.parse(t); } catch (e) { j = null; }
        return { status: r.status, json: j, text: t };
      });
    });
  }

  // A TOML literal for one form input. Strings and lists are quoted by JSON, whose string
  // escapes TOML basic strings share.
  function literal(row) {
    var kind = row.dataset.kind;
    var el = row.querySelector("input,select,textarea");
    if (kind === "checkbox") return el.checked ? "true" : "false";
    if (kind === "tri") return el.value === "" ? null : el.value;
    if (kind === "number") return el.value.trim() === "" ? null : el.value.trim();
    if (kind === "list") {
      var items = el.value.split("\n").map(function (s) { return s.trim(); }).filter(Boolean);
      return JSON.stringify(items);
    }
    return el.value === "" && row.dataset.src === "default" ? null : JSON.stringify(el.value);
  }

  // An actions row sends what was typed and the server makes the literal from the schema's
  // type; a removed row sends its id with no value. Removals go last, highest row first, so
  // the row numbers of the other edits still hold.
  function rowInput(row) {
    var el = row.querySelector("input,select,textarea");
    if (row.dataset.kind === "checkbox") return el.checked ? "true" : "false";
    return el.value;
  }

  function changes() {
    var out = [];
    if (!form) return out;
    form.querySelectorAll(".setrow").forEach(function (row) {
      if (!row.classList.contains("changed")) return;
      var card = row.closest(".setcard");
      if (card && card.dataset.removed) return;
      if (row.hasAttribute("data-row-key")) out.push({ id: row.dataset.id, input: rowInput(row) });
      else out.push({ id: row.dataset.id, value: literal(row) });
    });
    var gone = [];
    form.querySelectorAll(".setcard[data-removed]").forEach(function (c) { gone.push(c.dataset.row); });
    gone.sort(function (a, b) { return Number(b.split(".")[1]) - Number(a.split(".")[1]); });
    gone.forEach(function (id) { out.push({ id: id, value: null }); });
    return out;
  }

  function edit() {
    if (view === "form") return { f: file, changes: changes(), stamp: stamp };
    return { f: file, text: text.value, stamp: stamp };
  }

  function lines() {
    if (!gut) return;
    var n = text.value.split("\n").length;
    var bad = {};
    errs.querySelectorAll("li[data-line]").forEach(function (li) { bad[li.dataset.line] = true; });
    var out = [];
    for (var i = 1; i <= n; i++) out.push(bad[i] ? '<span class="bad">' + i + "</span>" : String(i));
    gut.innerHTML = out.join("\n");
    gut.scrollTop = text.scrollTop;
  }

  function show(reply) {
    errs.textContent = "";
    if (form) {
      form.querySelectorAll(".seterr").forEach(function (p) { p.hidden = true; p.textContent = ""; });
      form.querySelectorAll("[aria-invalid]").forEach(function (el) { el.removeAttribute("aria-invalid"); });
    }
    var first = null;
    (reply.errors || []).forEach(function (e) {
      var li = document.createElement("li");
      li.textContent = (e.line ? "Line " + e.line + ": " : "") + e.msg;
      if (e.line) li.dataset.line = String(e.line);
      errs.appendChild(li);
      var row = e.key && form && form.querySelector('.setrow[data-id="' + e.key + '"]');
      var msg = row && row.querySelector(".seterr");
      if (msg) {
        msg.textContent = e.msg;
        msg.hidden = false;
        var field = row.querySelector("input,select,textarea");
        if (field) {
          field.setAttribute("aria-invalid", "true");
          if (!first) first = field;
        }
      }
    });
    (reply.warnings || []).forEach(function (w) {
      var li = document.createElement("li");
      li.className = "muted";
      li.textContent = w;
      errs.appendChild(li);
    });
    lines();
    summary.textContent = reply.ok ? "" : "Not saved: " + (reply.errors || []).length + " error(s) to fix.";
    if (!reply.ok && saving && first) first.focus();
    var restart = reply.restart && reply.restart.length ? " Needs a restart: " + reply.restart.join(", ") + "." : "";
    note.textContent = reply.ok ? (dirty() ? "Unsaved changes." + restart : "") : "Not saved: fix the errors.";
    save.disabled = !reply.ok || !dirty();
  }

  function dirty() {
    return view === "form" ? changes().length > 0 : text.value !== saved;
  }

  function check() {
    clearTimeout(timer);
    timer = setTimeout(function () {
      call("POST", "/_settings/check", edit()).then(function (r) { if (r.json) show(r.json); });
    }, 250);
  }

  // The control marks the pressed button itself (app.js); a refused switch puts it back.
  function mark(to) {
    var b = document.querySelector('.segs [data-show="' + to + '"]');
    if (b && !b.classList.contains("on")) b.click();
  }

  function setView(to) {
    if (to === view) return;
    if (to === "plain") {
      // The form's edit as text, from the server, so Plain shows what Save would write.
      call("POST", "/_settings/check", edit()).then(function (r) {
        if (r.json && typeof r.json.text === "string") text.value = r.json.text;
        swap(to);
      });
      return;
    }
    // Back to the form: the form is rendered from the file, so a plain edit is saved first or
    // dropped; reloading with the saved text keeps one source of truth.
    if (text.value !== saved) {
      note.textContent = "Save or undo the plain edit to use the form.";
      setTimeout(function () { mark(view); }, 0);
      return;
    }
    swap(to);
  }

  function swap(to) {
    view = to;
    form.hidden = to !== "form";
    plain.hidden = to !== "plain";
    lines();
    check();
  }

  document.querySelectorAll(".segs [data-show]").forEach(function (b) {
    b.addEventListener("click", function () { setView(b.dataset.show); });
  });
  if (form) {
    form.addEventListener("click", function (e) {
      var add = e.target.closest("[data-add]");
      var rm = e.target.closest("[data-remove]");
      if (add) {
        var blank = form.querySelector('.setlist[data-list="' + add.dataset.add + '"] .setcard[data-blank]');
        if (blank) { blank.hidden = false; add.hidden = true; blank.querySelector("input,textarea").focus(); }
      } else if (rm) {
        var card = rm.closest(".setcard");
        card.dataset.removed = "1";
        card.hidden = true;
        check();
      }
    });
    form.addEventListener("input", function (e) {
      var row = e.target.closest(".setrow");
      if (row) row.classList.add("changed");
      check();
    });
    form.addEventListener("change", function (e) {
      var row = e.target.closest(".setrow");
      if (row) row.classList.add("changed");
      check();
    });
  }
  if (text) {
    text.addEventListener("input", check);
    text.addEventListener("scroll", function () { gut.scrollTop = text.scrollTop; });
  }
  save.addEventListener("click", function () {
    save.disabled = true;
    note.textContent = "Saving\u2026";
    saving = true;
    call("PUT", "/_settings/file", edit()).then(function (r) {
      saving = false;
      if (r.status === 200 && r.json) {
        // Reload so the form shows the saved file and its new stamp.
        var restart = r.json.restart && r.json.restart.length > 0;
        location.replace("/_settings?f=" + file + (restart ? "&restart=1" : "&saved=1"));
        return;
      }
      if (r.json) show(r.json);
      else note.textContent = r.text.trim() || "Not saved.";
      save.disabled = false;
    });
  });
  document.addEventListener("keydown", function (e) {
    if ((e.metaKey || e.ctrlKey) && e.key === "s") {
      e.preventDefault();
      if (!save.disabled) save.click();
    }
  });
  if (/[?&]restart=1/.test(location.search)) note.textContent = "Saved. Some changes need a restart.";
  else if (/[?&]saved/.test(location.search)) note.textContent = "Saved.";
  lines();
})();
