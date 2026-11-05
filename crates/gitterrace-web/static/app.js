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
  });
  fitSegs();
  addEventListener("resize", fitSegs);
  // Web fonts change the widths once they arrive.
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(fitSegs);
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
  window.gtToast = toast;
})();

