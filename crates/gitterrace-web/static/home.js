// The repositories home renders before every working tree is scanned. Rows without a fresh
// answer start with a hidden chip; one request for /_dirty fills them in.
(function () {
  "use strict";
  var chips = document.querySelectorAll(".wtchip[data-repo]");
  if (!chips.length || !window.fetch) return;
  fetch("/_dirty", { headers: { Accept: "application/json" } })
    .then(function (r) { return r.ok ? r.json() : null; })
    .then(function (dirty) {
      if (!dirty) return;
      chips.forEach(function (c) { c.hidden = dirty[c.dataset.repo] !== true; });
    })
    .catch(function () {});
})();

// The filter above the list. Without this script the form submits ?q= and the server narrows
// the list; with it the same box narrows in place: "/" focuses it, Enter opens the first match,
// Escape clears it and the arrow keys walk the matches.
(function () {
  "use strict";
  var form = document.querySelector("[data-repofilter]");
  var q = document.getElementById("repoq");
  if (!form || !q || !window.gtFilter) return;
  var count = document.getElementById("repocount");
  var rows = Array.prototype.slice.call(document.querySelectorAll(".reporow"));
  var none = document.querySelector(".repos-none");
  var f = window.gtFilter(q, {
    items: rows,
    text: function (li) { return li.dataset.name; },
    link: function (li) { return li.querySelector(".rname"); },
    none: none,
    clear: true,
    count: function (n, all) {
      count.textContent = n + " of " + all;
      var t = q.value.trim();
      history.replaceState(null, "", t ? "?q=" + encodeURIComponent(t) : location.pathname);
    }
  });
  form.querySelector("button").hidden = true;
  f.apply();
  form.addEventListener("submit", function (e) {
    e.preventDefault();
    var first = f.shown()[0];
    if (first) location.href = first.querySelector(".rname").href;
  });
  document.addEventListener("keydown", function (e) {
    var t = e.target, tag = t && t.tagName;
    if (e.key !== "/" || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
    if (/^(INPUT|TEXTAREA|SELECT)$/.test(tag) || (t && t.isContentEditable)) return;
    e.preventDefault();
    q.focus();
    q.select();
  });
})();
