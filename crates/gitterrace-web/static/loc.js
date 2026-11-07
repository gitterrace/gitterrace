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
  // The crosshair tip shows the whole breakdown; the bands' own tips would cover it.
  svg.querySelectorAll("path[data-tip]").forEach(function (p) { p.removeAttribute("data-tip"); });
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
    var sum = d.l[j].reduce(function (a, b) { return a + Math.max(b, 0); }, 0);
    var total = sum || 1;
    var rows = d.names.map(function (n, k) {
      var v = Math.max(d.l[j][k], 0);
      var cls = k === d.names.length - 1 && d.names[k] === "Other" ? "c8" : "c" + Math.min(k + 1, 8);
      var pct = v > 0 && v / total < 0.005 ? "<1%" : Math.round((v / total) * 100) + "%";
      return '<div><i class="dot ' + cls + '"></i><span>' + esc(n) + "</span><b>" + num(v) +
        ' <span class="muted">' + pct + "</span></b></div>";
    }).reverse().join("");
    tip.innerHTML = '<p class="s">' + esc(d.s[j] || "(no subject)") + '</p><p class="muted">' +
      esc(d.d[j]) + " &middot; commit " + num(d.i[j] + 1) + " of " + num(d.n) + "</p>" + rows +
      '<div class="tot"><span>Total</span><b>' + num(sum) + (sum === 1 ? " line" : " lines") + "</b></div>";
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
  // The keyboard reads the same tip: the arrow keys step through the sampled commits, Home and
  // End jump to the ends, Escape and leaving the chart drop the tip. The tip is announced
  // politely while the keyboard drives it, never on hover.
  var at = -1;
  svg.addEventListener("keydown", function (ev) {
    var last = d.x.length - 1;
    var step = { ArrowRight: 1, ArrowUp: 1, ArrowLeft: -1, ArrowDown: -1 }[ev.key];
    if (ev.key === "Escape") { hide(); return; }
    if (!step && ev.key !== "Home" && ev.key !== "End") return;
    if (ev.key === "Home") at = 0;
    else if (ev.key === "End") at = last;
    else at = at < 0 ? (step > 0 ? 0 : last) : Math.max(0, Math.min(last, at + step));
    tip.setAttribute("aria-live", "polite");
    show(at);
    ev.preventDefault();
  });
  svg.addEventListener("blur", function () { tip.removeAttribute("aria-live"); hide(); });
  svg.addEventListener("pointerenter", function () { tip.removeAttribute("aria-live"); });
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
