// "On this page" outline: scroll highlight and keyboard (menu and rail). The lists are
// plain links without this. app.js loads it only on a page with an outline or a filter help.
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
  // A heading is the last one above 30% of the window. A followed code line (a source file's
  // symbol) lands just under the pinned head by its scroll margin, and symbols sit a few lines
  // apart, so it is measured near the top, or the item below the followed one would light up.
  // Both are measured in the part of the window below a pinned page head.
  var mid = heads.length > 0 && !!heads[0].closest(".src");
  var pin = document.querySelector("main>.ph.pin");
  function current() {
    var pinned = pin && getComputedStyle(pin).position === "sticky";
    var top = pinned ? pin.getBoundingClientRect().height : 0;
    var line = top + (window.innerHeight - top) * (mid ? 0.1 : 0.3);
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

// The filter's "?" help is a native details, so it opens without script. Escape and a click
// outside close it, and Escape gives focus back to its button.
(function () {
  document.addEventListener("keydown", function (e) {
    var d = e.target.closest && e.target.closest("details[data-filter-help][open]");
    if (e.key !== "Escape" || !d) return;
    d.open = false;
    d.querySelector("summary").focus();
  });
  document.addEventListener("click", function (e) {
    document.querySelectorAll("details[data-filter-help][open]").forEach(function (d) {
      if (!d.contains(e.target)) d.open = false;
    });
  });
})();
