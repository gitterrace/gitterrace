// "On this page" outline: scroll highlight, filter and keyboard (menu and rail). The lists are
// plain links without this. Loaded on every page: with no [data-outline] it does nothing.
// gtFilter, the filter behind every "narrow this list" box: typing hides the items that do not
// contain the text, an empty result shows a note, and the arrow keys walk the links that are
// left. Used by the outline and the repositories home, so the two cannot drift.
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
(function () {
  var navs = Array.prototype.slice.call(document.querySelectorAll("[data-outline]"));
  if (!navs.length) return;
  var links = [];
  navs.forEach(function (n) {
    n.querySelectorAll("a[href^='#']").forEach(function (a) { links.push(a); });
  });
  var heads = [];
  links.forEach(function (a) {
    var h = document.getElementById(decodeURIComponent(a.getAttribute("href").slice(1)));
    if (h && heads.indexOf(h) < 0) heads.push(h);
  });
  heads.sort(function (a, b) {
    return a.getBoundingClientRect().top - b.getBoundingClientRect().top;
  });
  function mark(id) {
    links.forEach(function (a) {
      var on = a.getAttribute("href") === "#" + id;
      a.classList.toggle("on", on);
      if (on) { a.setAttribute("aria-current", "location"); }
      else { a.removeAttribute("aria-current"); }
      if (on && a.closest(".rail")) {
        var box = a.closest(".rail"), r = a.getBoundingClientRect();
        var b = box.getBoundingClientRect();
        if (r.top < b.top || r.bottom > b.bottom) box.scrollTop += r.top - b.top - b.height / 2;
      }
    });
  }
  // A heading lands at the top, so the item is the last one above 30% of the window. A code
  // line (a source file's symbol) is scrolled to the middle by the line link, so it is measured
  // there, or the item above the followed one would light up.
  var mid = heads.length > 0 && !!heads[0].closest(".src");
  function current() {
    var line = window.innerHeight * (mid ? 0.55 : 0.3), id = heads.length ? heads[0].id : "";
    heads.forEach(function (h) { if (h.getBoundingClientRect().top <= line) id = h.id; });
    if (id) mark(id);
  }
  if ("IntersectionObserver" in window && heads.length) {
    var io = new IntersectionObserver(current, {
      rootMargin: "0px 0px -70% 0px", threshold: [0, 1]
    });
    heads.forEach(function (h) { io.observe(h); });
    window.addEventListener("scroll", current, { passive: true });
    current();
  }
  // The Code tab shows the source, which has no headings: hide the outline with the preview.
  document.querySelectorAll("[data-md-view] [data-show]").forEach(function (b) {
    b.addEventListener("click", function () {
      var rendered = b.dataset.show === "rendered";
      document.querySelectorAll("[data-outline]").forEach(function (n) { n.hidden = !rendered; });
    });
  });
  navs.forEach(function (d) {
    var q = d.querySelector(".outline-q"), none = d.querySelector(".outline-none");
    var menu = d.tagName === "DETAILS";
    if (q) {
      var items = Array.prototype.slice.call(d.querySelectorAll("li"));
      q.hidden = false;
      window.gtFilter(q, { items: items, root: d, clear: !menu, none: none });
      if (menu) d.addEventListener("toggle", function () { if (d.open) q.focus(); });
    }
    function close() { d.open = false; d.querySelector("summary").focus(); }
    d.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && menu && d.open) { e.preventDefault(); close(); }
    });
    if (!menu) return;
    d.addEventListener("click", function (e) {
      if (e.target.closest && e.target.closest("a")) d.open = false;
    });
    document.addEventListener("click", function (e) {
      if (d.open && !d.contains(e.target)) d.open = false;
    });
  });
})();
