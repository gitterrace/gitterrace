// Live updates. One EventSource per tab on GET /events. The server says which parts of the
// repository moved (head, refs, stash, worktrees, index, worktree); the page decides what that means:
//   data-live="refresh"  re-fetch this page and swap <main> in place
//   data-live="notice"   show a quiet "Updated, refresh" note, never reload on its own
// The top bar chip and the HEAD banner always follow. Pages pinned to a commit or tag carry no
// data-live attribute and never open a stream.
(function () {
  "use strict";
  var body = document.body;
  if (!body || body.dataset.live === undefined || body.dataset.static !== undefined || !window.EventSource) return;
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
      if (wrap) wrap.innerHTML = c.banner || ""; // server-escaped HTML: the commit is a sha link
    }
  }

  function notice() {
    if (!note) return;
    note.textContent = "Updated, refresh";
    note.setAttribute("data-tip", "The repository or the settings changed since this page was loaded. Click to reload.");
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
    // A reload of config.toml can change any page, whatever kinds it follows.
    if ((c.kinds || []).indexOf("config") >= 0) { notice(); return; }
    // A repository joined the server: the switcher (nav.js) re-reads the list in place.
    if ((c.kinds || []).indexOf("repos") >= 0) {
      document.dispatchEvent(new CustomEvent("gt:repos-changed"));
      if (!shows(c.kinds)) return;
    }
    if (!shows(c.kinds || [])) return;
    if (how === "refresh") refresh();
    else if (how === "notice") notice();
  }

  // One stream per browser, not per tab: WebKit allows 6 HTTP/1.1 connections per host, and a
  // stream per tab starves navigation after a few tabs. The visible tab that holds the Web Lock
  // for this repository is the leader: it owns the EventSource and relays every event to the
  // other tabs over a BroadcastChannel. A hidden tab holds nothing (it drops the lock, the next
  // visible tab takes over), and catches up on return through the version.
  var wantWt = on.indexOf("worktree") >= 0;
  var id = Math.random().toString(36).slice(2);
  var shared = !!(window.BroadcastChannel && navigator.locks && navigator.locks.request);
  var chan = shared ? new BroadcastChannel("gt-live:" + B) : null;
  var leader = false;
  var latest = version; // newest version seen on the stream, leader only
  var peers = {};       // id -> needs the working tree scan, visible tabs only
  var held = null;      // { abort, release } of the lock request

  function streamWt() {
    if (wantWt) return true;
    for (var k in peers) if (peers[k]) return true;
    return false;
  }

  function open() {
    if (source) source.close();
    var wt = streamWt();
    source = new EventSource(B + "/events?v=" + encodeURIComponent(latest) + (wt ? "&wt=1" : ""));
    source.wt = wt;
    source.addEventListener("change", function (ev) {
      try { latest = JSON.parse(ev.data).v || latest; } catch (e) {}
      if (chan) chan.postMessage({ t: "ev", data: ev.data });
      handle(ev);
    });
    source.onopen = function () { delay = 1000; };
    source.onerror = function () {
      // The browser would retry on its own, but at a fixed short interval. Back off instead.
      // A refused stream (503, too many open pages) lands here too and retries slowly.
      source.close();
      source = null;
      clearTimeout(timer);
      timer = setTimeout(open, delay);
      delay = Math.min(delay * 2, 30000);
    };
  }

  function stop() {
    if (source) { source.close(); source = null; }
    clearTimeout(timer);
  }

  function lead(release) {
    leader = true;
    latest = version;
    peers = {};
    delay = 1000;
    held.release = release;
    open();
    chan.postMessage({ t: "who" });
  }

  function claim() {
    if (held || document.hidden) return;
    if (!shared) { delay = 1000; open(); return; }
    var ctl = new AbortController();
    held = { abort: function () { ctl.abort(); }, release: null };
    navigator.locks.request("gt-live:" + B, { signal: ctl.signal }, function () {
      return new Promise(function (release) { lead(release); });
    }).catch(function () {}).then(function () { held = null; });
  }

  function drop() {
    if (!shared) { stop(); return; }
    if (held) {
      if (held.release) held.release(); else held.abort();
    }
    if (leader) { leader = false; stop(); }
    held = null;
  }

  function hello() {
    if (chan) chan.postMessage({ t: "hello", id: id, wt: wantWt, v: version });
  }

  if (chan) {
    chan.onmessage = function (m) {
      var d = m.data || {};
      if (d.t === "ev") {
        if (!document.hidden) handle({ data: d.data });
      } else if (d.t === "who") {
        if (!document.hidden) hello();
      } else if (leader && d.t === "hello") {
        var before = streamWt();
        peers[d.id] = d.wt;
        // A tab that was hidden may have missed changes: tell it everything may have moved.
        if (d.v !== latest) {
          chan.postMessage({ t: "ev", data: JSON.stringify({ kinds: ["head", "refs", "stash", "worktrees", "index", "worktree", "repos"], v: latest }) });
        }
        if (!before && streamWt() && source) open();
      } else if (leader && d.t === "bye") {
        delete peers[d.id];
      }
    };
  }

  if (note) note.addEventListener("click", function () { location.reload(); });
  document.addEventListener("visibilitychange", function () {
    if (document.hidden) {
      if (chan) chan.postMessage({ t: "bye", id: id });
      drop();
    } else {
      delay = 1000;
      hello();
      claim();
    }
  });
  window.addEventListener("pagehide", function () {
    if (chan) chan.postMessage({ t: "bye", id: id });
    drop();
  });
  if (!document.hidden) { hello(); claim(); }
})();
