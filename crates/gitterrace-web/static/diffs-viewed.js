// Viewed marks on a diff page: a checkbox per file, a count of the marked ones, and a badge on a
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
  function count(files, marks) {
    var n = 0, p = document.querySelector("p.viewed-count");
    files.forEach(function (s) { if (marks[s.dataset.path] === s.dataset.h) n++; });
    if (!files.length) return;
    if (!p) {
      p = el("p", "muted pad viewed-count");
      p.setAttribute("role", "status");
      files[0].parentNode.insertBefore(p, files[0]);
    }
    p.textContent = "Viewed " + n + " of " + files.length;
  }
  function decorate() {
    var files = Array.prototype.slice.call(document.querySelectorAll("section.file[data-path]"));
    var marks = read();
    files.forEach(function (s) {
      var head = s.querySelector("header"), path = s.dataset.path, h = s.dataset.h;
      var box = s.querySelector("input.viewed");
      if (!head) return;
      if (!box) {
        var lab = el("label", "viewed-mark");
        box = el("input", "viewed");
        box.type = "checkbox";
        lab.appendChild(box);
        lab.appendChild(document.createTextNode(" Viewed"));
        var counts = head.querySelector(".counts, .media-counts");
        head.insertBefore(lab, counts);
        box.addEventListener("change", function () {
          var m = read();
          if (box.checked) m[path] = h; else delete m[path];
          write(m);
          sync();
        });
      }
      var seen = path in marks;
      var stale = seen && marks[path] !== h;
      var badge = head.querySelector(".viewed-changed");
      box.checked = seen && !stale;
      if (stale && !badge) {
        badge = el("span", "tag viewed-changed", "Changed since viewed");
        head.insertBefore(badge, box.parentNode);
      } else if (!stale && badge) {
        badge.remove();
      }
    });
    count(files, marks);
  }
  function sync() { decorate(); }
  decorate();
  document.addEventListener("gt:live-refreshed", decorate);
})();
