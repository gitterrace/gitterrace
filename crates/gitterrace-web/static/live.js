// Live updates. One EventSource per tab on GET /events. The server says which parts of the
// repository moved (head, refs, stash, worktrees, index, worktree); the page decides what that means:
//   data-live="refresh"  re-fetch this page and swap <main> in place
//   data-live="notice"   show a quiet "Updated, refresh" note, never reload on its own
// The top bar chip and the HEAD banner always follow. Pages pinned to a commit or tag carry no
// data-live attribute and never open a stream.
(function () {
  "use strict";
  var body = document.body;
  if (!body || body.dataset.live === undefined || !window.EventSource) return;
  var meta = document.querySelector('meta[name="gt-base"]');
  var B = meta ? meta.content : "";
  var how = body.dataset.live;
  var on = (body.dataset.liveOn || "").split(" ").filter(Boolean);
  var version = body.dataset.liveV || "";
  var chip = document.getElementById("wtchip");
  var banner = document.getElementById("headstate");
  var note = document.getElementById("livenote");
  var source = null;
  var delay = 1000;
  var timer = null;
  var fetching = false;
  var again = false;

  function shows(kinds) {
    return kinds.some(function (k) { return on.indexOf(k) >= 0; });
  }

  function chrome(c) {
    if (chip && typeof c.dirty === "boolean") chip.hidden = !c.dirty;
    if (banner && "banner" in c) {
      banner.hidden = !c.banner;
      var wrap = banner.querySelector(".wrap");
      if (wrap) wrap.textContent = c.banner || "";
    }
  }

  function notice() {
    if (!note) return;
    note.textContent = "Updated, refresh";
    note.title = "The repository changed since this page was loaded. Click to reload.";
    note.hidden = false;
  }

  // Swaps <main> for the fresh render, keeping scroll and open <details>.
  async function refresh() {
    // A change that lands mid-fetch may postdate the render being fetched: go round again.
    if (fetching) { again = true; return; }
    fetching = true;
    try {
      var res = await fetch(location.href, { headers: { "X-Live": "1" }, cache: "no-store" });
      if (!res.ok) return;
      var doc = new DOMParser().parseFromString(await res.text(), "text/html");
      var next = doc.getElementById("main");
      var cur = document.getElementById("main");
      if (!next || !cur) return;
      var y = window.scrollY;
      cur.replaceWith(document.importNode(next, true));
      window.scrollTo(0, y);
      document.dispatchEvent(new CustomEvent("gt:live-refreshed"));
    } catch (e) {
      notice();
    } finally {
      fetching = false;
      if (again) { again = false; refresh(); }
    }
  }

  function handle(ev) {
    var c;
    try { c = JSON.parse(ev.data); } catch (e) { return; }
    version = c.v || version;
    chrome(c);
    if (!shows(c.kinds || [])) return;
    if (how === "refresh") refresh();
    else if (how === "notice") notice();
  }

  function connect() {
    if (source) source.close();
    source = new EventSource(B + "/events?v=" + encodeURIComponent(version) +
      (on.indexOf("worktree") >= 0 ? "&wt=1" : ""));
    source.addEventListener("change", handle);
    source.onopen = function () { delay = 1000; };
    source.onerror = function () {
      // The browser would retry on its own, but at a fixed short interval. Back off instead,
      // and stop for good when the server refuses the stream (too many open pages).
      source.close();
      source = null;
      clearTimeout(timer);
      timer = setTimeout(connect, delay);
      delay = Math.min(delay * 2, 30000);
    };
  }

  if (note) note.addEventListener("click", function () { location.reload(); });
  // A background tab needs no stream: close it, and catch up on return via the version.
  document.addEventListener("visibilitychange", function () {
    if (document.hidden) {
      if (source) { source.close(); source = null; }
      clearTimeout(timer);
    } else if (!source) {
      delay = 1000;
      connect();
    }
  });
  window.addEventListener("pagehide", function () { if (source) source.close(); });
  if (!document.hidden) connect();
})();
