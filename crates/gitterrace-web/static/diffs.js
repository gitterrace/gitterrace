// Folded diff files: a click, a scroll near the viewport or an #fN anchor swaps the body in.
// app.js loads this only on a page with a folded file.
(function () {
  "use strict";
  // A folded file is a link to its body. Clicking it (or Enter on it) swaps the body in; a file
  // folded only for the page budget (data-lazy) also loads when it scrolls near the viewport,
  // and an #fN anchor opens its file. Find in page sees loaded files only.
  function loadDiff(b) {
    if (b.dataset.busy) return;
    b.dataset.busy = "1";
    if (lazyObs) lazyObs.unobserve(b);
    fetch(b.dataset.loadDiff).then(function (r) {
      if (!r.ok) throw r;
      return r.text();
    }).then(function (h) {
      var p = b.parentNode, focused = document.activeElement === b;
      p.insertAdjacentHTML("afterend", h);
      var body = p.nextElementSibling;
      p.remove();
      if (focused && body) { body.tabIndex = -1; body.focus({ preventScroll: true }); }
      keepAnchor();
    }, function () {
      delete b.dataset.busy;
      // Watched again, so a lazy file that failed once retries when it next scrolls near.
      if (lazyObs && b.hasAttribute("data-lazy")) lazyObs.observe(b);
    });
  }
  document.addEventListener("click", function (e) {
    var o = e.target.closest && e.target.closest("[data-open-all]");
    if (o && !e.button) {
      e.preventDefault();
      document.querySelectorAll("[data-load-diff]").forEach(loadDiff);
      o.disabled = true;
      return;
    }
    var g = e.target.closest && e.target.closest("[data-expand-generated]");
    var b = g ? null : e.target.closest && e.target.closest("[data-load-diff]");
    if (!(g || b) || e.button || e.metaKey || e.ctrlKey || e.shiftKey) return;
    e.preventDefault();
    if (b) loadDiff(b);
    else document.querySelectorAll("section[data-generated] [data-load-diff]").forEach(loadDiff);
  });
  var lazyObs = window.IntersectionObserver ? new IntersectionObserver(function (es) {
    es.forEach(function (x) { if (x.isIntersecting) loadDiff(x.target); });
  }, { rootMargin: "1500px 0px" }) : null;
  function watchLazy() {
    if (!lazyObs) return;
    var bs = document.querySelectorAll("[data-load-diff][data-lazy]");
    Array.prototype.forEach.call(bs, function (b) { lazyObs.observe(b); });
  }
  // WebKit has no scroll anchoring: files above an #fN target load and grow after the jump and
  // push the target away. While the reader has not scrolled or keyed, each load puts it back.
  var anchored = null;
  function keepAnchor() {
    var s = anchored && document.getElementById(anchored);
    s = s && s.closest("section");
    if (s) s.scrollIntoView({ block: "start" });
  }
  ["wheel", "touchstart", "mousedown", "keydown"].forEach(function (t) {
    addEventListener(t, function () { anchored = null; }, { capture: true, passive: true });
  });
  // The browser jumps to #fN itself. This opens a folded target and keeps it in place.
  function openHashed(jump) {
    var m = /^#(f\d+|p\/.+)$/.exec(location.hash), id = m && m[1], s;
    // #p/<path> names a file by path, so it survives a different file order.
    try { if (id && id[0] === "p") id = decodeURIComponent(id); } catch (x) { id = null; }
    s = id && document.getElementById(id);
    s = s && s.closest("section");
    var b = s && s.querySelector("[data-load-diff]");
    if (jump === true) anchored = s ? id : null;
    if (b) loadDiff(b);
  }
  watchLazy();
  openHashed(true);
  window.addEventListener("hashchange", function () { openHashed(true); });
  document.addEventListener("gt:live-refreshed", function () { watchLazy(); openHashed(false); });
})();
