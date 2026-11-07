// "On this page" outline: scroll highlight, filter and keyboard (menu and rail). The lists are
// plain links without this. Loaded on every page: with no [data-outline] it does nothing.
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
  navs.forEach(function (d) {
    var q = d.querySelector(".outline-q"), none = d.querySelector(".outline-none");
    if (!q) return;
    var menu = d.tagName === "DETAILS";
    var items = Array.prototype.slice.call(d.querySelectorAll("li"));
    function shown() { return items.filter(function (li) { return !li.hidden; }).map(function (li) { return li.firstChild; }); }
    q.hidden = false;
    q.addEventListener("input", function () {
      var t = q.value.trim().toLowerCase();
      items.forEach(function (li) { li.hidden = t !== "" && li.textContent.toLowerCase().indexOf(t) < 0; });
      none.hidden = shown().length > 0;
    });
    if (menu) d.addEventListener("toggle", function () { if (d.open) q.focus(); });
    function close() { d.open = false; d.querySelector("summary").focus(); }
    d.addEventListener("keydown", function (e) {
      if (e.key === "Escape") {
        if (menu && d.open) { e.preventDefault(); close(); }
        else if (!menu && q.value) { e.preventDefault(); q.value = ""; q.dispatchEvent(new Event("input")); }
        return;
      }
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      var list = shown(), i = list.indexOf(document.activeElement);
      if (!list.length) return;
      e.preventDefault();
      var n = e.key === "ArrowDown" ? i + 1 : i - 1;
      if (n < 0) { q.focus(); } else { list[Math.min(n, list.length - 1)].focus(); }
    });
    if (!menu) return;
    d.addEventListener("click", function (e) { if (e.target.closest && e.target.closest("a")) d.open = false; });
    document.addEventListener("click", function (e) { if (d.open && !d.contains(e.target)) d.open = false; });
  });
})();
