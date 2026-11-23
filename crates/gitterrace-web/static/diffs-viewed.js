// Viewed marks on a diff page: a checkbox per file, a count, a hide switch and a badge on a
// marked file whose content changed since. Kept in localStorage under the page's own address
// (a commit, a compare range or a working tree tab), so nothing leaves the machine.
// app.js loads this only on a page with diff files.
(function () {
  "use strict";
  var KEY = "gt-viewed:" + location.pathname + location.search;
  function read() {
    try { return JSON.parse(localStorage.getItem(KEY)) || {}; } catch (e) { return {}; }
  }
  function write(m) {
    try {
      if (Object.keys(m).length) localStorage.setItem(KEY, JSON.stringify(m));
      else localStorage.removeItem(KEY);
    } catch (e) { /* storage refused: the marks last for this page view only */ }
  }
  var el = window.gtNav.el;
  // The bar is server rendered: this only fills in its numbers, so nothing moves when it runs.
  function count(files, marks) {
    var n = 0, p = document.querySelector(".difftools .viewed-count");
    var bar = document.querySelector(".difftools .dt-bar");
    files.forEach(function (s) {
      var on = marks[s.dataset.path] === s.dataset.h;
      if (on) n++;
      if (on) s.setAttribute("data-viewed", ""); else s.removeAttribute("data-viewed");
    });
    if (p) p.textContent = "Viewed " + n + " of " + files.length;
    if (bar) bar.value = n;
    var hide = document.querySelector(".difftools .dt-hide-box");
    if (hide) hide.checked = document.body.classList.contains("hide-viewed");
  }
  // One handler for every box, so a file a live refresh brings in needs no binding of its own.
  document.addEventListener("change", function (e) {
    var t = e.target, s = t.classList.contains("viewed") && t.closest("section.file");
    if (s) {
      var m = read();
      if (t.checked) m[s.dataset.path] = s.dataset.h; else delete m[s.dataset.path];
      write(m);
      decorate();
    }
  });
  document.addEventListener("change", function (e) {
    if (e.target.classList && e.target.classList.contains("dt-hide-box")) {
      document.body.classList.toggle("hide-viewed", e.target.checked);
    }
  });
  function decorate() {
    var files = Array.prototype.slice.call(document.querySelectorAll("section.file[data-path]"));
    var marks = read();
    files.forEach(function (s) {
      var head = s.querySelector("header"), path = s.dataset.path, h = s.dataset.h;
      var box = s.querySelector("input.viewed");
      if (!box) {
        var lab = el("label", "viewed-mark");
        box = el("input", "viewed");
        box.type = "checkbox";
        lab.appendChild(box);
        lab.appendChild(document.createTextNode(" Viewed"));
        // Right after the +/- counts, so the toggle reads with the numbers it marks.
        var counts = head.querySelector(".counts, .media-counts");
        if (counts) counts.after(lab); else head.appendChild(lab);
      }
      var seen = path in marks;
      var stale = seen && marks[path] !== h;
      var badge = head.querySelector(".viewed-changed");
      box.checked = seen && !stale;
      if (stale && !badge) {
        badge = el("span", "tag viewed-changed", "Changed since viewed");
        head.insertBefore(badge, head.querySelector(".counts, .media-counts"));
      } else if (!stale && badge) {
        badge.remove();
      }
    });
    count(files, marks);
  }
  decorate();
  document.addEventListener("gt:live-refreshed", decorate);
})();
