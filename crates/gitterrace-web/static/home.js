// The repositories home renders before every working tree is scanned. Rows without a fresh
// answer start with a hidden chip; one request for /_dirty fills them in.
// A row live.js adds later starts hidden too, and is filled the same way.
(function () {
  "use strict";
  if (!window.fetch) return;
  function fill(chips) {
    if (!chips.length) return;
    fetch("/_dirty", { headers: { Accept: "application/json" } })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (dirty) {
        if (!dirty) return;
        chips.forEach(function (c) { c.hidden = dirty[c.dataset.repo] !== true; });
      })
      .catch(function () {});
  }
  fill(Array.prototype.slice.call(document.querySelectorAll(".wtchip[data-repo]")));
  document.addEventListener("gt:home-rows", function (e) {
    var chips = [];
    (e.detail.added || []).forEach(function (li) {
      Array.prototype.push.apply(chips, li.querySelectorAll(".wtchip[data-repo]"));
    });
    fill(chips);
  });
})();

// The filter above the list. Without this script the form submits ?q= and the server narrows
// the list; with it the same box narrows in place: "/" focuses it, Enter opens the first match,
// Escape clears it and the arrow keys walk the matches.
(function () {
  "use strict";
  var form = document.querySelector("[data-repofilter]");
  var q = document.getElementById("repoq");
  if (!form || !q) return;
  var count = document.getElementById("repocount");
  var rows = Array.prototype.slice.call(document.querySelectorAll(".reporow"));
  var none = document.querySelector(".repos-none");
  function shown() { return rows.filter(function (li) { return !li.hidden; }); }
  function apply() {
    var t = q.value.trim().toLowerCase();
    rows.forEach(function (li) {
      li.hidden = t !== "" && li.dataset.name.toLowerCase().indexOf(t) < 0;
    });
    var n = shown().length;
    if (none) none.hidden = n > 0;
    count.textContent = n + " of " + rows.length;
    var url = t ? "?q=" + encodeURIComponent(q.value.trim()) : location.pathname;
    history.replaceState(null, "", url);
  }
  q.addEventListener("input", apply);
  q.addEventListener("keydown", function (e) {
    if (e.key === "Escape") {
      if (q.value) { e.preventDefault(); q.value = ""; apply(); }
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    var list = shown().map(function (li) { return li.querySelector(".rname"); });
    var i = list.indexOf(document.activeElement);
    if (!list.length) return;
    e.preventDefault();
    var n = e.key === "ArrowDown" ? i + 1 : i - 1;
    if (n < 0) { q.focus(); } else { list[Math.min(n, list.length - 1)].focus(); }
  });
  form.querySelector("button").hidden = true;
  apply();
  form.addEventListener("submit", function (e) {
    e.preventDefault();
    var first = shown()[0];
    if (first) location.href = first.querySelector(".rname").href;
  });
  document.addEventListener("keydown", function (e) {
    var t = e.target, tag = t && t.tagName;
    if (window.gtKeysOff && window.gtKeysOff()) return;
    if (e.key !== "/" || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
    // Single-key shortcuts can be switched off (WCAG 2.1.4). nav.js stores the choice, and this
    // page does not load it, so the same key is read here.
    try { if (localStorage.getItem("gitterrace-keys") === "off") return; } catch (x) {}
    if (/^(INPUT|TEXTAREA|SELECT)$/.test(tag) || (t && t.isContentEditable)) return;
    e.preventDefault();
    q.focus();
    q.select();
  });
})();
