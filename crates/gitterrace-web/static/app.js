// Copy-to-clipboard helpers, the toast that confirms a copy and the flash on a jumped-to line.
// Vanilla JS, no dependencies.
(function () {
  var root = document.documentElement;
  var btn = document.getElementById("theme");
  var order = ["system", "light", "dark"];
  function current() {
    return root.dataset.theme || "system";
  }
  function label() {
    if (btn) btn.setAttribute("data-tip", "Theme: " + current() + " (click to change)");
  }
  if (btn) {
    label();
    btn.addEventListener("click", function () {
      var next = order[(order.indexOf(current()) + 1) % 3];
      if (next === "system") delete root.dataset.theme;
      else root.dataset.theme = next;
      try {
        if (next === "system") localStorage.removeItem("gitterrace-theme");
        else localStorage.setItem("gitterrace-theme", next);
      } catch (e) {}
      label();
    });
  }
  // One rule for every segmented control: when its choices do not fit one row beside the
  // caption, the native select takes their place.
  var segBoxes = Array.prototype.slice.call(document.querySelectorAll("[data-seg-select]"));
  function fitSegs() {
    segBoxes.forEach(function (box) {
      box.classList.remove("narrow");
      // The row of choices, not its box: an inline box at full width lets the row spill.
      var segs = box.querySelector(".segs");
      var limit = Math.min(document.documentElement.clientWidth, box.parentElement.getBoundingClientRect().right);
      var over = box.scrollWidth > box.clientWidth + 1 || segs.scrollWidth > segs.clientWidth + 1;
      var out = segs.getBoundingClientRect().right > limit + 1;
      if (over || out) box.classList.add("narrow");
    });
  }
  segBoxes.forEach(function (box) {
    var sel = box.querySelector("select");
    sel.hidden = false;
    sel.addEventListener("change", function () {
      var btn = box.querySelector('button[data-show="' + sel.value + '"]');
      var radio = box.querySelector('input[value="' + sel.value + '"]');
      if (radio) {
        radio.checked = true;
        radio.dispatchEvent(new Event("change", { bubbles: true }));
      }
      else if (btn) btn.click();
      else location.href = sel.value;
    });
    box.addEventListener("change", function (e) {
      if (e.target.type === "radio") sel.value = e.target.value;
    });
    // A button switch can change from elsewhere (a keyboard shortcut): keep the select in step.
    box.querySelectorAll("button[data-show]").forEach(function (b) {
      b.addEventListener("click", function () { sel.value = b.getAttribute("data-show"); });
    });
    // A switch that stays on the page (data-seg-own on an ancestor) is marked here, the one
    // place a segmented control's state is written; its page listens for the click.
    if (box.closest("[data-seg-own]")) box.querySelectorAll("button[data-show]").forEach(function (b, _, all) {
      b.addEventListener("click", function () {
        all.forEach(function (o) { o.classList.toggle("on", o === b); o.setAttribute("aria-pressed", o === b ? "true" : "false"); });
      });
    });
  });
  fitSegs();
  addEventListener("resize", fitSegs);
  // A tab row too wide for the screen scrolls sideways on one line: fade its edge and bring the
  // open tab into view.
  var tabRows = Array.prototype.slice.call(document.querySelectorAll(".tabs"));
  function edge(row) {
    // Measured on the tabs themselves: WebKit's scrollWidth can run a few pixels past the end.
    var r = row.getBoundingClientRect();
    row.classList.toggle("over-l", row.firstElementChild.getBoundingClientRect().left < r.left - 1);
    row.classList.toggle("over-r", row.lastElementChild.getBoundingClientRect().right > r.right + 1);
  }
  function fitTabs() {
    tabRows.forEach(function (row) {
      var on = row.querySelector('[aria-current="page"]');
      if (on) {
        var r = row.getBoundingClientRect(), o = on.getBoundingClientRect();
        // Past the open tab by a fade's width, but never past the row's end.
        if (o.right > r.right) row.scrollLeft += o.right - r.right + 32;
        else if (o.left < r.left) row.scrollLeft -= r.left - o.left + 32;
      }
      edge(row);
    });
  }
  tabRows.forEach(function (row) { row.addEventListener("scroll", function () { edge(row); }, { passive: true }); });
  fitTabs();
  addEventListener("resize", fitTabs);
  // Web fonts change the widths once they arrive.
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(function () { fitSegs(); fitTabs(); });
  var boxes = document.querySelectorAll("[data-progress]");
  if (!boxes.length) return;
  var timer = setInterval(async function () {
    try {
      var p = await (await fetch(window.gtNav.B + "/api/progress")).json();
      var pct = p.total ? Math.floor((p.done / p.total) * 100) : p.ready ? 100 : 0;
      boxes.forEach(function (b) {
        var bar = b.querySelector("progress");
        // No value until the build reports a total: the bar keeps its indeterminate motion.
        if (bar && p.total) bar.value = pct;
        var t = b.querySelector(".pct");
        if (t && p.total) t.textContent = pct + "%";
        var s = b.querySelector(".badge.state");
        if (s && p.state) s.textContent = p.state;
      });
      if (p.ready) {
        clearInterval(timer);
        location.reload();
      }
    } catch (e) {}
  }, 1000);
})();

// One copy path for every script, so every copy shows the same confirmation: the button that
// asked flips to a check for a moment, and a toast names what landed on the clipboard. A copy
// that fails says so; a silent clipboard is the bug this exists to prevent.
(function () {
  var FLASH_MS = 1200;
  function toast(msg, sticky) {
    var old = document.querySelector(".toast");
    if (old) old.remove();
    var t = document.createElement("div");
    t.className = "toast";
    t.setAttribute("role", "status");
    t.textContent = msg;
    document.body.appendChild(t);
    setTimeout(function () { t.classList.add("out"); }, sticky ? 4000 : 1400);
    setTimeout(function () { t.remove(); }, sticky ? 4400 : 1800);
  }
  function flash(btn) {
    if (!btn || !btn.classList) return;
    clearTimeout(btn._gtCopied);
    btn.classList.add("copied");
    btn._gtCopied = setTimeout(function () { btn.classList.remove("copied"); }, FLASH_MS);
  }
  function fallback(text) {
    var ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    var ok = false;
    try { ok = document.execCommand("copy"); } catch (e) { ok = false; }
    ta.remove();
    return ok;
  }
  // copy(text, label, button): label is the toast ("Permalink copied"), button the element to
  // flash; it defaults to the focused button, which is the one clicked or keyed.
  function copy(text, label, btn) {
    var target = btn || (document.activeElement && document.activeElement.closest &&
      document.activeElement.closest("button,[role=menuitem]"));
    var done = function () { flash(target); toast(label || "Copied"); return true; };
    var fail = function () { toast("Copy failed"); return false; };
    if (navigator.clipboard && window.isSecureContext !== false) {
      return navigator.clipboard.writeText(text).then(done, function () { return fallback(text) ? done() : fail(); });
    }
    return Promise.resolve(fallback(text) ? done() : fail());
  }
  window.gtCopy = copy;

  // Pinned heads: a head is stuck while it sits below the top of its box's content, which only
  // happens when sticky moved it. The CSS draws the line under a pinned page head from it.
  var heads = document.querySelectorAll(
    ".dochead,section.file>header,main>.ph.pin,.entries tr.th:first-child th");
  // A pinned page head's height, so anchors and the symbol rail land below it.
  var pin = document.querySelector("main>.ph.pin");
  function stuck() {
    if (pin) {
      var h = pin.getBoundingClientRect().height;
      document.documentElement.style.setProperty("--ph-pin-h", h + "px");
    }
    heads.forEach(function (h) {
      if (getComputedStyle(h).position !== "sticky") { h.removeAttribute("data-stuck"); return; }
      var box = h.closest(".card,section.file") || h.parentElement;
      var natural = box.getBoundingClientRect().top + box.clientTop + parseFloat(getComputedStyle(box).paddingTop);
      h.toggleAttribute("data-stuck", h.getBoundingClientRect().top > natural + 0.5);
    });
  }
  if (heads.length) {
    var queued = false;
    var later = function () { if (!queued) { queued = true; requestAnimationFrame(function () { queued = false; stuck(); }); } };
    window.addEventListener("scroll", later, { passive: true });
    window.addEventListener("resize", later);
    stuck();
  }
  // A chart of links is one tab stop: the arrow keys move between its marks, Home and End jump
  // to the ends, Enter follows the focused mark and Tab leaves the chart.
  document.addEventListener("keydown", function (e) {
    var svg = e.target.closest && e.target.closest("svg[data-marks]");
    var step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
    if (!svg || (!step && e.key !== "Home" && e.key !== "End")) return;
    var marks = Array.prototype.slice.call(svg.querySelectorAll("a[href]"));
    if (!marks.length) return;
    var at = marks.indexOf(e.target);
    var last = marks.length - 1, next;
    if (e.key === "Home") next = 0;
    else if (e.key === "End" || (at < 0 && step < 0)) next = last;
    else next = at < 0 ? 0 : Math.max(0, Math.min(last, at + step));
    marks[next].focus();
    e.preventDefault();
  });
  window.gtToast = toast;
  // Image diffs: CSS shows the checked view. The choice is kept per browser; on a phone,
  // where the sides stack anyway, the overlay is the default.
  var mediaViews = document.querySelectorAll("[data-media-view]");
  if (mediaViews.length) {
    var KEY = "gitterrace-media-view", saved = null;
    try { saved = localStorage.getItem(KEY); } catch (e) {}
    var first = saved || (matchMedia("(max-width:600px)").matches ? "overlay" : "sides");
    mediaViews.forEach(function (box) {
      var r = box.querySelector('input[value="' + first + '"]');
      if (r) r.checked = true;
      box.addEventListener("change", function (e) {
        if (e.target.type !== "radio") return;
        try { localStorage.setItem(KEY, e.target.value); } catch (x) {}
      });
      var cut = box.querySelector("[data-media-cut]");
      if (cut) cut.addEventListener("input", function () { box.style.setProperty("--cut", cut.value + "%"); });
    });
  }
})();


// Scripts behind a gesture load on first use. gtLazy(name, done) adds <name>.js once, beside
// this script, and calls done when it has run. The server renders the triggers.
(function () {
  "use strict";
  var HERE = document.currentScript ? document.currentScript.src : location.href;
  // This script's own ?v=, so a file it adds is the same build's, cached as long.
  var V = new URL(HERE).search;
  var body = document.body;
  var STATIC = body.dataset.static !== undefined;
  var CSS = { send: "send.css", card: "card.css" };
  var state = {};
  window.gtLazy = function (name, done) {
    var s = state[name];
    if (!s) {
      s = state[name] = { ready: false, wait: [] };
      // Wait for the stylesheet too, or the window opens unstyled.
      var left = CSS[name] ? 2 : 1;
      var one = function () {
        if (--left > 0) return;
        s.ready = true;
        s.wait.splice(0).forEach(function (f) { f(); });
      };
      if (CSS[name]) {
        var l = document.createElement("link");
        l.rel = "stylesheet";
        l.href = new URL(CSS[name] + V, HERE).href;
        l.onload = l.onerror = one;
        document.head.appendChild(l);
      }
      var t = document.createElement("script");
      t.src = new URL(name + ".js" + V, HERE).href;
      t.onload = one;
      document.head.appendChild(t);
    }
    if (!done) return;
    if (s.ready) done(); else s.wait.push(done);
  };
  function meta(n) { return document.querySelector('meta[name="' + n + '"]'); }
  var off = meta("gt-off");
  var OFF = off ? off.content.split(" ") : [];
  var act = meta("gitterrace-actions");
  var chords = false;
  try {
    chords = !!act && JSON.parse(act.dataset.menu || "[]").some(function (a) { return a.key; });
  } catch (e) { chords = false; }
  function typing(t) {
    return t && (/^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName) || t.isContentEditable);
  }
  function free(e) {
    var ov = window.gtOverlay;
    if (e.metaKey || e.ctrlKey || e.altKey || typing(e.target)) return false;
    return !(ov && (ov.name || ov.chord)) && !(window.gtKeysOff && window.gtKeysOff());
  }

  // Outline and filter help: only where the page has one.
  if (document.querySelector("[data-outline], [data-filter-help]")) window.gtLazy("outline");

  // The file card and the hot badges: only where the page names a fragment.
  if (document.querySelector("[data-card]")) window.gtLazy("card");

  // Folded diff files: only where the page has one, or once a live refresh brings one.
  function withDiffs() {
    if (document.querySelector("[data-load-diff]")) window.gtLazy("diffs");
    if (!document.querySelector("section.file[data-path]")) return;
    window.gtLazy("diffs-viewed");
    window.gtLazy("diffs-find");
  }
  withDiffs();
  document.addEventListener("gt:live-refreshed", withDiffs);

  // Pickers: "b", a click on a switcher, a live repository change or ?open=refs.
  var pickers = false;
  function withPickers(done) { pickers = true; window.gtLazy("pickers", done); }
  document.addEventListener("keydown", function (e) {
    var shortcutsOff = window.gtKeysOff && window.gtKeysOff();
    if (shortcutsOff) return;
    // nav.js handles "b" once the pickers are here.
    if (window.gtKeysOff && window.gtKeysOff()) return;
    if (pickers || e.key !== "b" || !free(e)) return;
    e.preventDefault();
    withPickers(function () { window.gtPickers.openRefs(); });
  });
  ["refbtn", "repobtn"].forEach(function (id) {
    var b = document.getElementById(id);
    if (!b) return;
    b.addEventListener("click", function first() {
      b.removeEventListener("click", first);
      if (!pickers) withPickers(function () { b.click(); });
    });
  });
  document.addEventListener("gt:repos-changed", function () { if (!pickers) withPickers(); });
  if (/[?&]open=refs/.test(location.search)) withPickers();

  // Action menu: none in an export, which has no server.
  if (STATIC) return;
  // The actions note is rendered by the server and refreshed on show and focus. No timer: WebKit
  // caps connections per host.
  var noteEl = document.getElementById("actnote");
  function pollNote() {
    if (!noteEl || document.hidden || OFF.indexOf("actions") >= 0) return;
    fetch("/_actions/status", { cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (j) {
        if (!j) return;
        var line = noteEl.firstElementChild;
        if (!j.note) { noteEl.hidden = true; return; }
        if (line.textContent !== j.note) line.textContent = j.note;
        noteEl.hidden = false;
      })
      .catch(function () {});
  }
  pollNote();
  document.addEventListener("visibilitychange", pollNote);
  window.addEventListener("focus", pollNote);

  var actions = false;
  function withActions(done) { actions = true; window.gtLazy("actions", done); }
  // A chord runs a user action, which needs a real key press: load at once.
  if (chords) withActions();
  // ".": keys typed while loading (". s") are held, then replayed in order.
  var held = null;
  document.addEventListener("keydown", function (e) {
    var shortcutsOff = window.gtKeysOff && window.gtKeysOff();
    if (shortcutsOff) return;
    if (held) { e.preventDefault(); e.stopImmediatePropagation(); held.push(e.key); return; }
    if (window.gtKeysOff && window.gtKeysOff()) return;
    if (actions || e.key !== "." || !free(e)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    held = [e.key];
    withActions(function () {
      var keys = held;
      held = null;
      keys.forEach(function (key) {
        var ev = new KeyboardEvent("keydown", { key: key, bubbles: true, cancelable: true });
        (document.activeElement || document.body).dispatchEvent(ev);
      });
    });
  }, true);
  ["actpage", "actcommit"].forEach(function (id) {
    var b = document.getElementById(id);
    if (!b) return;
    b.addEventListener("click", function first() {
      b.removeEventListener("click", first);
      if (!actions) withActions(function () { b.click(); });
    });
  });
  // Floating triggers come from the menu's script: load it on the first sign of one.
  var HOVER = ".diff .hh, table.blame tr[id^=L], tr[data-act]";
  document.addEventListener("mouseover", function (e) {
    var h = !actions && e.target.closest && e.target.closest(HOVER);
    if (!h) return;
    withActions(function () { h.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })); });
  });
  document.addEventListener("selectionchange", function () {
    var s = window.getSelection();
    if (!actions && s && !s.isCollapsed) {
      withActions(function () { document.dispatchEvent(new Event("selectionchange")); });
    }
  });
  function lines() { return /^#L\d/.test(location.hash); }
  if (lines() || document.querySelector(".src .l.sel, table.code tr.sel")) withActions();
  window.addEventListener("hashchange", function () { if (!actions && lines()) withActions(); });
})();
