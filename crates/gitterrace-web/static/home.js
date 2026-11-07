// The repositories home renders before every working tree is scanned. Rows without a fresh
// answer start with a hidden chip; one request for /_dirty fills them in.
(function () {
  "use strict";
  var chips = document.querySelectorAll(".wtchip[data-repo]");
  if (!chips.length || !window.fetch) return;
  fetch("/_dirty", { headers: { Accept: "application/json" } })
    .then(function (r) { return r.ok ? r.json() : null; })
    .then(function (dirty) {
      if (!dirty) return;
      chips.forEach(function (c) { c.hidden = dirty[c.dataset.repo] !== true; });
    })
    .catch(function () {});
})();
