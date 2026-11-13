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

// gtFilter, the filter behind every "narrow this list" box: typing hides the items that do not
// contain the text, an empty result shows a note, and the arrow keys walk the links that are
// left. Used by the filter below.
// gtFilter(input, { items, text, link, none, root, clear, count }) returns { shown, apply }.
//   items  the elements to hide or show   text  item -> the string searched (default textContent)
//   link   item -> its focusable link     none  the note shown when nothing matches
//   root   where arrow keys are heard     clear Escape empties the box
//   count  called with (shown, total) after every change
(function () {
  "use strict";
  window.gtFilter = function (q, o) {
    var items = o.items;
    var text = o.text || function (el) { return el.textContent; };
    var link = o.link || function (el) { return el.firstChild; };
    function shown() { return items.filter(function (el) { return !el.hidden; }); }
    function apply() {
      var t = q.value.trim().toLowerCase();
      items.forEach(function (el) {
        el.hidden = t !== "" && text(el).toLowerCase().indexOf(t) < 0;
      });
      var n = shown().length;
      if (o.none) o.none.hidden = n > 0;
      if (o.count) o.count(n, items.length);
    }
    q.addEventListener("input", apply);
    (o.root || q).addEventListener("keydown", function (e) {
      if (e.key === "Escape") {
        if (o.clear && q.value) { e.preventDefault(); q.value = ""; apply(); }
        return;
      }
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      var list = shown().map(link), i = list.indexOf(document.activeElement);
      if (!list.length) return;
      e.preventDefault();
      var n = e.key === "ArrowDown" ? i + 1 : i - 1;
      if (n < 0) { q.focus(); } else { list[Math.min(n, list.length - 1)].focus(); }
    });
    return { shown: shown, apply: apply };
  };
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
