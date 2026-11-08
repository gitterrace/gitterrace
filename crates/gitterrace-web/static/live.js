// Live updates. One EventSource per tab on GET /events. The server says which parts of the
// repository moved (head, refs, stash, worktrees, index, worktree); the page decides what that
// means:
//   data-live="refresh"  re-fetch this page and swap <main> in place
//   data-live="notice"   show a quiet "Updated, refresh" note, never reload on its own
//   data-live="home"     the repositories home: re-fetch it and add or drop rows in place
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

  // ---- scroll anchoring across a swap -----------------------------------------------------------
  // The fresh render folds every diff again, so it is shorter than the page the reader scrolled.
  // Restoring window.scrollY would clamp upward and every lazy load would shift the page again.
  // Instead the topmost visible element with a stable key (a file by its path, else an id) is
  // remembered with its offset from the viewport top, and put back at that offset after the swap
  // and after every body that loads, until nothing has moved for a while. The reader may scroll
  // on meanwhile: their scroll moves the anchor to wherever they are, so a body that loads above
  // them later never pushes the page back.
  var hold = null;      // { key, top } being kept in place
  var watch = null;     // MutationObserver re-applying the anchor
  var sizes = null;     // ResizeObserver re-applying it when only layout moved
  var quiet = null;     // timer that ends the hold once nothing moves for a while
  var gen = 0;         // which hold is current, so a stale observer never moves the page
  var ours = null;      // scrollY right after our own scroll, so it is not taken for the reader's

  function keyOf(el) {
    return el.dataset && el.dataset.path ? "p:" + el.dataset.path : "i:" + el.id;
  }

  function find(root, key) {
    var list = root.querySelectorAll("section[data-path], [id]");
    for (var i = 0; i < list.length; i++) if (keyOf(list[i]) === key) return list[i];
    return null;
  }

  // The file at the middle of the viewport, the one being read, by the offset of its top (also
  // when that top is far above, inside a long file). The topmost file reaching into view is often
  // one that started far above: it folds again or loads, and holding it moved the page by whole
  // files. An enclosing element with an id spans the viewport too, so files come first.
  function anchor(root) {
    var files = root.querySelectorAll("section[data-path]"), mid = window.innerHeight / 2, near = null;
    for (var j = 0; j < files.length; j++) {
      var f = files[j].getBoundingClientRect();
      if (f.bottom <= 0) continue;
      if (f.top <= mid && f.bottom > mid) return { key: keyOf(files[j]), top: f.top };
      if (!near) near = { key: keyOf(files[j]), top: f.top };
      if (f.top > mid) break;
    }
    if (near) return near;
    var list = root.querySelectorAll("[id]");
    for (var i = 0; i < list.length; i++) {
      var el = list[i];
      if (el === root) continue;
      var r = el.getBoundingClientRect();
      if (r.bottom > 0) return { key: keyOf(el), top: r.top };
    }
    return null;
  }

  function place() {
    if (!hold) return;
    var el = find(document, hold.key);
    if (!el) return;
    var d = el.getBoundingClientRect().top - hold.top;
    if (Math.abs(d) >= 1) { window.scrollBy(0, d); ours = window.scrollY; }
  }

  // A scroll the code did not make is the reader's: hold the place they scrolled to instead.
  function moved() {
    if (!hold || window.scrollY === ours) return;
    ours = null;
    var main = document.getElementById("main");
    var a = main && anchor(main);
    if (a) hold = a;
  }

  function release() {
    hold = null;
    ours = null;
    clearTimeout(quiet);
    if (watch) { watch.disconnect(); watch = null; }
    if (sizes) { sizes.disconnect(); sizes = null; }
    window.removeEventListener("scroll", moved);
  }

  function keep(a) {
    release();
    hold = a;
    var mine = ++gen;
    window.addEventListener("scroll", moved, { passive: true });
    var main = document.getElementById("main");
    var bump = function () { clearTimeout(quiet); quiet = setTimeout(release, 3000); };
    if (window.MutationObserver && main) {
      watch = new MutationObserver(function () { if (mine !== gen) return; place(); bump(); });
      watch.observe(main, { childList: true, subtree: true });
    }
    // A diff chunk skipped by content-visibility gets its real height later, with no DOM change,
    // and the files below it move: re-apply the anchor when any file changes size.
    if (window.ResizeObserver && main) {
      sizes = new ResizeObserver(function () { if (mine !== gen) return; place(); });
      Array.prototype.forEach.call(main.querySelectorAll("section[data-path]"), function (s) { sizes.observe(s); });
    }
    bump();
  }

  // Files whose body was loaded and is unchanged (same data-h) move into the fresh render as
  // they are. A file whose content changed is loaded again through its own fold link.
  function carry(cur, next) {
    var loaded = {};
    Array.prototype.forEach.call(cur.querySelectorAll("section[data-path]"), function (s) {
      if (!s.querySelector("[data-load-diff]")) loaded[s.dataset.path] = s;
    });
    Array.prototype.forEach.call(next.querySelectorAll("section[data-path]"), function (s) {
      var old = loaded[s.dataset.path];
      var fold = s.querySelector("[data-load-diff]");
      if (!old || !fold) return;
      var pa = fold.closest(".fold") || fold.parentNode;
      if (old.dataset.h && old.dataset.h === s.dataset.h) {
        var head = old.querySelector("header");
        var rest = [];
        for (var n = head ? head.nextSibling : null; n; n = n.nextSibling) rest.push(n);
        rest.forEach(function (n) { pa.parentNode.insertBefore(n, pa); });
        pa.remove();
      } else {
        fold.setAttribute("data-reload", "");
      }
    });
  }

  // Swaps <main> for the fresh render, keeping the reader's place.
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
      var a = anchor(cur);
      var y = window.scrollY;
      var fresh = document.importNode(next, true);
      carry(cur, fresh);
      cur.replaceWith(fresh);
      if (a) { keep(a); place(); } else window.scrollTo(0, y);
      document.dispatchEvent(new CustomEvent("gt:live-refreshed"));
      // Changed files that were open load again; each load re-applies the anchor.
      Array.prototype.forEach.call(fresh.querySelectorAll("[data-reload]"), function (b) { b.removeAttribute("data-reload"); b.click(); });
    } catch (e) {
      notice();
    } finally {
      fetching = false;
      if (again) { again = false; refresh(); }
    }
  }

  // The home: rows are matched by data-name. A row still listed stays the same node (a filter's
  // hidden flag and focus survive), a gone one is removed, a new one is put where the server
  // lists it. Anything that does not fit that shape falls back to the note.
  async function relist() {
    if (fetching) { again = true; return; }
    fetching = true;
    try {
      // The bare home, not a filtered ?q= render: the filter is the page's to apply.
      var res = await fetch("/", { headers: { "X-Live": "1" }, cache: "no-store" });
      // One repository left: the home forwards to it, there is no list to patch.
      if (!res.ok || res.redirected) { notice(); return; }
      var doc = new DOMParser().parseFromString(await res.text(), "text/html");
      var next = doc.querySelector("ul.repos");
      var cur = document.querySelector("ul.repos");
      if (!next || !cur) { notice(); return; }
      var have = {};
      Array.prototype.forEach.call(cur.children, function (li) {
        if (li.dataset.name) have[li.dataset.name] = li;
      });
      var want = Array.prototype.slice.call(next.children).filter(function (li) {
        return li.dataset.name;
      });
      if (want.length !== next.children.length) { notice(); return; }
      var keep = {}, added = [];
      want.forEach(function (li) { keep[li.dataset.name] = true; });
      Object.keys(have).forEach(function (n) { if (!keep[n]) have[n].remove(); });
      var at = cur.firstChild;
      want.forEach(function (li) {
        var el = have[li.dataset.name];
        if (!el) { el = document.importNode(li, true); added.push(el); }
        if (el !== at) cur.insertBefore(el, at); else at = at.nextSibling;
      });
      // A row that joins while a filter is typed follows that filter, as its neighbours do.
      var q = document.getElementById("repoq");
      var t = q ? q.value.trim().toLowerCase() : "";
      if (t) added.forEach(function (li) {
        li.hidden = li.dataset.name.toLowerCase().indexOf(t) < 0;
      });
      var meta = doc.querySelector(".repolist .doc-tools .muted");
      var mine = document.querySelector(".repolist .doc-tools .muted");
      if (meta && mine) mine.textContent = meta.textContent;
      document.dispatchEvent(new CustomEvent("gt:home-rows", { detail: { added: added } }));
      document.dispatchEvent(new CustomEvent("gt:live-refreshed"));
    } catch (e) {
      notice();
    } finally {
      fetching = false;
      if (again) { again = false; relist(); }
    }
  }

  function handle(ev) {
    var c;
    try { c = JSON.parse(ev.data); } catch (e) { return; }
    version = c.v || version;
    chrome(c);
    // A reload of config.toml can change any page, whatever kinds it follows.
    if ((c.kinds || []).indexOf("config") >= 0) { notice(); return; }
    // A repository joined or left: the switcher (nav.js) re-reads the list in place.
    if ((c.kinds || []).indexOf("repos") >= 0) {
      document.dispatchEvent(new CustomEvent("gt:repos-changed"));
      if (!shows(c.kinds)) return;
    }
    if (!shows(c.kinds || [])) return;
    if (how === "refresh") refresh();
    else if (how === "home") relist();
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
    source.onopen = function () {
      delay = 1000;
      // A repository may have joined or left before this stream was open (the version does not
      // move for it, so no catch-up event comes): the switcher reads the list again.
      document.dispatchEvent(new CustomEvent("gt:repos-changed"));
    };
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
