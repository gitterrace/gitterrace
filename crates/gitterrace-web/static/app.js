(function () {
  var boxes = document.querySelectorAll("[data-progress]");
  if (!boxes.length) return;
  var timer = setInterval(async function () {
    try {
      var p = await (await fetch("/api/progress")).json();
      var pct = p.total ? Math.floor((p.done / p.total) * 100) : p.ready ? 100 : 0;
      boxes.forEach(function (b) {
        var bar = b.querySelector("progress");
        if (bar) bar.value = pct;
      });
      if (p.ready) {
        clearInterval(timer);
        location.reload();
      }
    } catch (e) {}
  }, 1000);
})();
