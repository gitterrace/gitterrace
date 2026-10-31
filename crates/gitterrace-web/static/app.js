(function () {
  var root = document.documentElement;
  function base() { var m = document.querySelector('meta[name="gt-base"]'); return m ? m.content : ""; }
  var btn = document.getElementById("theme");
  var order = ["system", "light", "dark"];
  function current() {
    return root.dataset.theme || "system";
  }
  function label() {
    if (btn) btn.title = "Theme: " + current() + " (click to change)";
  }
  if (btn) {
    label();
    btn.addEventListener("click", function () {
      var next = order[(order.indexOf(current()) + 1) % 3];
      if (next === "system") delete root.dataset.theme;
      else root.dataset.theme = next;
      try {
        if (next === "system") localStorage.removeItem("gitterrace-theme");
        else localStorage.setItem("gitterrace-theme", next);
      } catch (e) {}
      label();
    });
  }
  var boxes = document.querySelectorAll("[data-progress]");
  if (!boxes.length) return;
  var timer = setInterval(async function () {
    try {
      var p = await (await fetch(base() + "/api/progress")).json();
      var pct = p.total ? Math.floor((p.done / p.total) * 100) : p.ready ? 100 : 0;
      boxes.forEach(function (b) {
        var bar = b.querySelector("progress");
        if (bar) bar.value = pct;
        var t = b.querySelector(".pct");
        if (t) t.textContent = pct + "%";
      });
      if (p.ready) {
        clearInterval(timer);
        location.reload();
      }
    } catch (e) {}
  }, 1000);
})();
