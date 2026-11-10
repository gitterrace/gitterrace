// "On this page" outline: scroll highlight and keyboard (menu and rail). The lists are
// plain links without this. Loaded on every page: with no [data-outline] it does nothing.
// gtFilter, the filter behind every "narrow this list" box: typing hides the items that do not
// contain the text, an empty result shows a note, and the arrow keys walk the links that are
// left. Used by the repositories home.
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
  // there, or the item above the followed one would light up. Both are measured in the part of
  // the window below a pinned page head.
  var mid = heads.length > 0 && !!heads[0].closest(".src");
  var pin = document.querySelector("main>.ph.pin");
  function current() {
    var pinned = pin && getComputedStyle(pin).position === "sticky";
    var top = pinned ? pin.getBoundingClientRect().height : 0;
    var line = top + (window.innerHeight - top) * (mid ? 0.55 : 0.3);
    var id = heads.length ? heads[0].id : "";
    // Looked up by id each time: a live refresh swaps <main> and the first-screen highlight swaps
    // the code chunks, so an element held from load is detached, and its rect is all zeros,
    // which would put every head above the line and light the last one.
    heads.forEach(function (k) {
      var h = document.getElementById(k.id);
      if (h && h.getBoundingClientRect().top <= line) id = h.id;
    });
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
    var menu = d.tagName === "DETAILS";
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
