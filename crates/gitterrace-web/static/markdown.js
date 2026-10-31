(function () {
  var root = document.documentElement;
  function base() { var m = document.querySelector('meta[name="gt-base"]'); return m ? m.content : ""; }
  document.querySelectorAll("[data-md-view]").forEach(function (box) {
    var btns = box.querySelectorAll("[data-show]");
    btns.forEach(function (b) {
      b.addEventListener("click", function () {
        btns.forEach(function (o) { o.classList.toggle("on", o === b); });
        box.querySelectorAll("[data-pane]").forEach(function (p) {
          p.hidden = p.dataset.pane !== b.dataset.show;
        });
      });
    });
  });

  var nodes = Array.prototype.slice.call(document.querySelectorAll("pre.mermaid"));
  if (!nodes.length) return;
  nodes.forEach(function (n) { n.dataset.src = n.textContent; });

  function dark() {
    var t = root.dataset.theme;
    if (t) return t === "dark";
    return window.matchMedia && matchMedia("(prefers-color-scheme: dark)").matches;
  }
  var s = document.createElement("script");
  s.src = base() + "/static/vendor/mermaid-11.17.2.min.js";
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
