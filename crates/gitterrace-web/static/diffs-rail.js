// The file rail beside a diff: marks the file in view and the files marked viewed. The links are
// plain #fN anchors the server wrote, so without this script the rail still jumps. The rail has
// one row per file, and the scan below reads file sections only, never diff lines.
// app.js loads this only on a page with a rail.
(function () {
  "use strict";
  var current = null;
  var io = null;
  function rows() {
    return Array.prototype.slice.call(document.querySelectorAll("[data-rail] li[data-f]"));
  }
  function mark(id) {
    if (id === current) return;
    current = id;
    rows().forEach(function (li) {
      if (li.dataset.f === id) {
        li.setAttribute("aria-current", "true");
        // Keep the marked row inside the rail's own scroll, never the page's.
        var rail = li.closest("[data-rail]");
        if (rail && (li.offsetTop < rail.scrollTop ||
            li.offsetTop + li.offsetHeight > rail.scrollTop + rail.clientHeight)) {
          rail.scrollTop = li.offsetTop - rail.clientHeight / 2;
        }
      } else {
        li.removeAttribute("aria-current");
      }
    });
  }
  // The sections near the top of the window: the first one still in view is the current file.
  var seen = {};
  // A clicked row stays current until the reader scrolls by hand: a short page cannot bring the
  // target to the top, so the observer alone would name an earlier file and undo the click.
  var pinned = null;
  ["wheel", "touchstart", "keydown"].forEach(function (t) {
    window.addEventListener(t, function () { pinned = null; }, { passive: true });
  });
  function observe() {
    if (io) io.disconnect();
    seen = {};
    if (!("IntersectionObserver" in window)) return;
    io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) { seen[e.target.id] = e.isIntersecting; });
      var first = Array.prototype.slice.call(document.querySelectorAll("section.file[id]"))
        .filter(function (s) { return seen[s.id]; })[0];
      if (pinned) mark(pinned);
      else if (first) mark(first.id);
    }, { rootMargin: "0px 0px -60% 0px" });
    document.querySelectorAll("section.file[id]").forEach(function (s) { io.observe(s); });
  }
  // A click names the file at once: the observer catches up once the page has scrolled.
  document.addEventListener("click", function (e) {
    var a = e.target.closest && e.target.closest("[data-rail] a");
    if (a) {
      pinned = a.getAttribute("href").slice(1);
      mark(pinned);
    }
  });
  // Viewed state is read from the sections diffs-viewed.js marks, so the two never disagree.
  function viewed() {
    var done = {};
    document.querySelectorAll("section.file[data-viewed]").forEach(function (s) {
      done[s.id] = true;
    });
    rows().forEach(function (li) {
      if (done[li.dataset.f]) li.setAttribute("data-seen", "");
      else li.removeAttribute("data-seen");
    });
  }
  var watch = null;
  function start() {
    observe();
    viewed();
    if (watch) watch.disconnect();
    if (!window.MutationObserver) return;
    watch = new MutationObserver(viewed);
    watch.observe(document.body, { attributes: true, attributeFilter: ["data-viewed"],
      subtree: true });
  }
  start();
  document.addEventListener("gt:live-refreshed", start);
})();
