// Copy-to-clipboard helpers, the toast that confirms a copy and the flash on a jumped-to line.
// Vanilla JS, no dependencies.
(function () {
  var root = document.documentElement;
  function base() { var m = document.querySelector('meta[name="gt-base"]'); return m ? m.content : ""; }
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
      if (btn) btn.click();
      else location.href = sel.value;
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
      var p = await (await fetch(base() + "/api/progress")).json();
      var pct = p.total ? Math.floor((p.done / p.total) * 100) : p.ready ? 100 : 0;
      boxes.forEach(function (b) {
        var bar = b.querySelector("progress");
        if (bar) bar.value = pct;
        var t = b.querySelector(".pct");
        if (t) t.textContent = pct + "%";
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
  var heads = document.querySelectorAll(".dochead,section.file>header,main>.head,.entries tr.th:first-child td");
  function stuck() {
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
})();

