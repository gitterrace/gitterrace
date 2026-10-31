// Lines-of-code chart: hover crosshair and drag-to-select range. Without this script the
// range card still works through ?from=&to= links.
(function () {
  "use strict";
  var svg = document.getElementById("loc");
  var raw = document.getElementById("loc-data");
  if (!svg || !raw) return;
  var d = JSON.parse(raw.textContent);
  var tip = svg.parentNode.querySelector(".loc-tip");
  var xh = svg.querySelector(".xh");
  var sel = svg.querySelector(".sel");
  var vb = svg.viewBox.baseVal;
  var drag = null;

  function fx(ev) {
    var r = svg.getBoundingClientRect();
    return ((ev.clientX - r.left) / r.width) * vb.width;
  }
  function nearest(x) {
    var lo = 0, hi = d.x.length - 1;
    while (lo < hi) {
      var mid = (lo + hi) >> 1;
      if (d.x[mid] < x) lo = mid + 1; else hi = mid;
    }
    if (lo > 0 && Math.abs(d.x[lo - 1] - x) <= Math.abs(d.x[lo] - x)) lo--;
    return lo;
  }
  function num(n) {
    return String(Math.round(n)).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  }
  function esc(s) {
    return s.replace(/[&<>"]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c];
    });
  }
  function show(j) {
    var total = d.l[j].reduce(function (a, b) { return a + Math.max(b, 0); }, 0) || 1;
    var rows = d.names.map(function (n, k) {
      var v = Math.max(d.l[j][k], 0);
      var cls = k === d.names.length - 1 && d.names[k] === "Other" ? "c8" : "c" + Math.min(k + 1, 8);
      return '<div><i class="dot ' + cls + '"></i><span>' + esc(n) + "</span><b>" +
        (d.share ? ((v / total) * 100).toFixed(1) + "%" : num(v)) + "</b></div>";
    }).reverse().join("");
    tip.innerHTML = '<p class="s">' + esc(d.s[j] || "(no subject)") + '</p><p class="muted">' +
      esc(d.d[j]) + " &middot; commit " + num(d.i[j] + 1) + " of " + num(d.n) + "</p>" + rows;
    tip.hidden = false;
    var r = svg.getBoundingClientRect();
    var px = (d.x[j] / vb.width) * r.width;
    var w = tip.offsetWidth;
    tip.style.left = (px + 14 + w > r.width ? px - 14 - w : px + 14) + "px";
    xh.setAttribute("x1", d.x[j]);
    xh.setAttribute("x2", d.x[j]);
    xh.style.display = "block";
  }
  function hide() {
    tip.hidden = true;
    xh.style.display = "none";
  }
  function paint(a, b) {
    var lo = Math.min(a, b), hi = Math.max(a, b);
    sel.setAttribute("x", d.x[lo]);
    sel.setAttribute("width", Math.max(d.x[hi] - d.x[lo], 2));
  }
  svg.style.touchAction = "pan-y";
  svg.addEventListener("pointermove", function (ev) {
    var j = nearest(fx(ev));
    if (drag) { drag.to = j; paint(drag.from, j); }
    show(j);
  });
  svg.addEventListener("pointerleave", function () { if (!drag) hide(); });
  svg.addEventListener("pointerdown", function (ev) {
    if (ev.button !== 0) return;
    var j = nearest(fx(ev));
    drag = { from: j, to: j };
    svg.setPointerCapture(ev.pointerId);
    paint(j, j);
  });
  svg.addEventListener("pointerup", function (ev) {
    if (!drag) return;
    var a = Math.min(drag.from, drag.to), b = Math.max(drag.from, drag.to);
    drag = null;
    svg.releasePointerCapture(ev.pointerId);
    if (b - a < 1) return;
    var u = new URL(d.view, location.href);
    u.searchParams.set("from", d.i[a]);
    u.searchParams.set("to", d.i[b]);
    location.href = u.pathname + u.search + "#range";
  });
})();
