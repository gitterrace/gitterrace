// Rendered Markdown extras: mermaid diagrams drawn on demand and picture sources kept in sync
// with the colour theme. Without it the Markdown still reads as plain text and images.
(function () {
  // This script's own URL, read while it runs: the diagram library lives beside it.
  var HERE = document.currentScript ? document.currentScript.src : location.href;
  var root = document.documentElement;
  function base() { var m = document.querySelector('meta[name="gt-base"]'); return m ? m.content : ""; }
  document.querySelectorAll("[data-md-view]").forEach(function (box) {
    var btns = box.querySelectorAll("[data-show]");
    btns.forEach(function (b) {
      b.addEventListener("click", function () {
        btns.forEach(function (o) { o.classList.toggle("on", o === b); o.setAttribute("aria-pressed", o === b ? "true" : "false"); });
        box.querySelectorAll("[data-pane]").forEach(function (p) {
          p.hidden = p.dataset.pane !== b.dataset.show;
        });
      });
    });
  });

  // A README <picture> picks its source by the OS colour scheme. When the page theme is
  // forced, rewrite each source's query so the image follows the page instead.
  var sources = Array.prototype.slice.call(document.querySelectorAll(".markdown-body picture source[media]"));
  sources.forEach(function (s) { s.dataset.media = s.media; });
  function syncPictures() {
    var t = root.dataset.theme;
    sources.forEach(function (s) {
      var wants = /dark/.test(s.dataset.media) ? "dark" : "light";
      s.media = !t ? s.dataset.media : (t === wants ? "all" : "not all");
    });
  }
  if (sources.length) {
    syncPictures();
    new MutationObserver(syncPictures).observe(root, { attributes: true, attributeFilter: ["data-theme"] });
  }

  var nodes = Array.prototype.slice.call(document.querySelectorAll("pre.mermaid"));
  if (!nodes.length) return;
  nodes.forEach(function (n) { n.dataset.src = n.textContent; });

  function dark() {
    var t = root.dataset.theme;
    if (t) return t === "dark";
    return window.matchMedia && matchMedia("(prefers-color-scheme: dark)").matches;
  }
  var s = document.createElement("script");
  // Beside this script, not at the root: an export opened from disk has its static folder
  // under the site, written with relative links, so `/static/` would leave the copy.
  s.src = new URL("vendor/mermaid-11.17.2.min.js", HERE).href;
  s.onload = function () {
    var n = 0;
    function draw() {
      window.mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: dark() ? "dark" : "default" });
      nodes.forEach(function (el) {
        el.removeAttribute("data-processed");
        el.textContent = el.dataset.src;
      });
      window.mermaid.run({ nodes: nodes }).catch(function () {});
    }
    draw();
    new MutationObserver(draw).observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    if (window.matchMedia) matchMedia("(prefers-color-scheme: dark)").addEventListener("change", draw);
  };
  document.head.appendChild(s);
})();
