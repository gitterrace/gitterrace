// The file card and the hot badges: fetched after first paint from the page's data-card URL.
// An empty answer (index still building, path not in history) leaves the page as it is.
(function () {
  "use strict";
  var host = document.querySelector("[data-card]");
  if (!host || !window.fetch) return;
  fetch(host.getAttribute("data-card"), { credentials: "same-origin" })
    .then(function (r) { return r.ok ? r.text() : ""; })
    .then(function (html) {
      if (!html.trim()) return;
      var box = document.createElement("div");
      box.innerHTML = html;
      if (host.tagName === "TABLE") {
        box.querySelectorAll("a.hot").forEach(function (a) {
          var cells = host.querySelectorAll("td.lc");
          for (var i = 0; i < cells.length; i++) {
            if (cells[i].getAttribute("data-name") === a.getAttribute("data-name")) {
              var th = cells[i].parentNode.querySelector("th");
              if (th) th.appendChild(a);
              break;
            }
          }
        });
      } else {
        host.innerHTML = html;
        host.hidden = false;
      }
    })
    .catch(function () {});
})();
