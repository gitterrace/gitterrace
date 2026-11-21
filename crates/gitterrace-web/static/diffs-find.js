// Find in the whole diff: the server searches every file of the commit or working tree tab, so
// folded and not yet loaded files count. Hits list under the box; a click opens that file.
// app.js loads this only on a commit or working tree page with diff files.
(function () {
  "use strict";
  var base = window.gtNav.B;
  var path = location.pathname.slice(base.length);
  var m = /^\/commit\/([^/]+)$/.exec(path), url;
  if (m) url = base + "/diff-find/" + m[1];
  else if (path === "/worktree") url = base + "/worktree/find" + location.search;
  if (!url) return;
  var el = window.gtNav.el;
  function build() {
    var first = document.querySelector("section.file[data-path]");
    if (!first || document.querySelector(".diff-find")) return;
    var box = el("form", "diff-find pad");
    var input = el("input");
    input.type = "search";
    input.placeholder = "Find in the whole diff";
    input.setAttribute("aria-label", "Find in the whole diff");
    var out = el("div", "diff-found");
    out.setAttribute("role", "status");
    box.appendChild(input);
    box.appendChild(out);
    first.parentNode.insertBefore(box, first);
    var seq = 0, timer = 0;
    function show(r) {
      out.textContent = "";
      if (!r.total) { out.appendChild(el("p", "muted", "No match in this diff.")); return; }
      var head = r.total + (r.total === 1 ? " line" : " lines") + " in " + r.files.length +
        (r.files.length === 1 ? " file" : " files") + (r.truncated ? " (list cut short)" : "");
      out.appendChild(el("p", "muted", head));
      var ul = el("ul", "files");
      r.files.forEach(function (f) {
        var li = el("li"), a = el("a", "", f.path);
        a.href = "#p/" + f.path.split("/").map(encodeURIComponent).join("/");
        li.appendChild(a);
        li.appendChild(el("span", "counts", " " + f.count));
        f.lines.forEach(function (l) {
          li.appendChild(el("div", "muted", (l.no === null ? "" : l.no + ": ") + l.text.trim()));
        });
        ul.appendChild(li);
      });
      out.appendChild(ul);
    }
    function run() {
      var q = input.value, mine = ++seq;
      if (!q) { out.textContent = ""; return; }
      var sep = url.indexOf("?") < 0 ? "?" : "&";
      fetch(url + sep + "q=" + encodeURIComponent(q)).then(function (r) {
        if (!r.ok) throw r;
        return r.json();
      }).then(function (r) { if (mine === seq) show(r); }, function () {
        if (mine === seq) out.textContent = "The search failed.";
      });
    }
    input.addEventListener("input", function () {
      clearTimeout(timer);
      timer = setTimeout(run, 250);
    });
    box.addEventListener("submit", function (e) { e.preventDefault(); run(); });
  }
  build();
  document.addEventListener("gt:live-refreshed", build);
})();
