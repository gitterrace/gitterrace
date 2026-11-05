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

  // "On this page" outline: scroll highlight, menu filter and keyboard. The lists are plain links
  // without this.
  (function () {
    var navs = Array.prototype.slice.call(document.querySelectorAll("[data-outline]"));
    if (!navs.length) return;
    var links = [];
    navs.forEach(function (n) { n.querySelectorAll("a[href^='#']").forEach(function (a) { links.push(a); }); });
    var heads = [];
    links.forEach(function (a) {
      var h = document.getElementById(decodeURIComponent(a.getAttribute("href").slice(1)));
      if (h && heads.indexOf(h) < 0) heads.push(h);
    });
    heads.sort(function (a, b) { return a.getBoundingClientRect().top - b.getBoundingClientRect().top; });
    function mark(id) {
      links.forEach(function (a) {
        var on = a.getAttribute("href") === "#" + id;
        a.classList.toggle("on", on);
        if (on) { a.setAttribute("aria-current", "location"); } else { a.removeAttribute("aria-current"); }
        if (on && a.closest(".rail")) {
          var box = a.closest(".rail"), r = a.getBoundingClientRect(), b = box.getBoundingClientRect();
          if (r.top < b.top || r.bottom > b.bottom) box.scrollTop += r.top - b.top - b.height / 2;
        }
      });
    }
    function current() {
      var line = window.innerHeight * 0.3, id = heads.length ? heads[0].id : "";
      heads.forEach(function (h) { if (h.getBoundingClientRect().top <= line) id = h.id; });
      if (id) mark(id);
    }
    if ("IntersectionObserver" in window && heads.length) {
      var io = new IntersectionObserver(current, { rootMargin: "0px 0px -70% 0px", threshold: [0, 1] });
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
    document.querySelectorAll("details[data-outline]").forEach(function (d) {
      var q = d.querySelector(".outline-q"), none = d.querySelector(".outline-none");
      var items = Array.prototype.slice.call(d.querySelectorAll("li"));
      function shown() { return items.filter(function (li) { return !li.hidden; }).map(function (li) { return li.firstChild; }); }
      q.hidden = false;
      q.addEventListener("input", function () {
        var t = q.value.trim().toLowerCase();
        items.forEach(function (li) { li.hidden = t !== "" && li.textContent.toLowerCase().indexOf(t) < 0; });
        none.hidden = shown().length > 0;
      });
      d.addEventListener("toggle", function () { if (d.open) q.focus(); });
      function close() { d.open = false; d.querySelector("summary").focus(); }
      d.addEventListener("keydown", function (e) {
        if (e.key === "Escape" && d.open) { e.preventDefault(); close(); return; }
        if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
        var list = shown(), i = list.indexOf(document.activeElement);
        if (!list.length) return;
        e.preventDefault();
        var n = e.key === "ArrowDown" ? i + 1 : i - 1;
        if (n < 0) { q.focus(); } else { list[Math.min(n, list.length - 1)].focus(); }
      });
      d.addEventListener("click", function (e) { if (e.target.closest && e.target.closest("a")) d.open = false; });
      document.addEventListener("click", function (e) { if (d.open && !d.contains(e.target)) d.open = false; });
    });
  })();

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
