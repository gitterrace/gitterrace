// The Checkpoint and Mark all viewed buttons of the working tree page. A checkpoint copies the
// changed text files into the cache folder and writes nothing else; Mark all viewed sends every
// file's patch id (data-p) and, with review.has_auto_checkpoint on, takes a checkpoint first.
// The marks themselves are read and shown by diffs-viewed.js. app.js loads this only where the
// server rendered the bar.
(function () {
  "use strict";
  var B = window.gtNav.B;
  function post(url, body) {
    return fetch(B + url, {
      method: "POST", cache: "no-store", headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body)
    }).then(function (r) {
      if (!r.ok) throw new Error(String(r.status));
      return r.json();
    });
  }
  function say(text) {
    var n = document.querySelector("[data-cpbar] .cpnote");
    if (n) n.textContent = text;
  }
  // The Since checkpoint tab opens the newest checkpoint, so a new one moves its link.
  function tab(id) {
    var nav = document.querySelector("header.ph nav.tabs");
    if (!nav) return;
    var a = Array.prototype.find.call(nav.children, function (x) {
      return x.textContent.indexOf("Since checkpoint") === 0;
    });
    if (!a) {
      a = document.createElement("a");
      a.textContent = "Since checkpoint";
      nav.appendChild(a);
    }
    a.href = B + "/worktree?since=" + encodeURIComponent(id);
  }
  // What a checkpoint write gave up, in words, for the status note.
  function gaveUp(r) {
    var m = "";
    if (r.dropped) {
      m += " " + r.dropped + " older checkpoint(s) were dropped to keep the log readable.";
    }
    if (r.copies_omitted) {
      m += " " + r.copies_omitted + " file copy(ies) were not kept: too much text.";
    }
    return m;
  }
  function busy(b, on) {
    b.disabled = on;
    if (on) b.setAttribute("aria-busy", "true"); else b.removeAttribute("aria-busy");
  }
  document.addEventListener("click", function (e) {
    var b = e.target.closest && e.target.closest("[data-checkpoint], [data-mark-all]");
    if (!b || b.disabled) return;
    busy(b, true);
    if (b.hasAttribute("data-checkpoint")) {
      post("/_checkpoint", { scope: "all", label: "" }).then(function (r) {
        tab(r.id);
        var m = r.created ? "Checkpoint saved." : "Nothing changed since the last checkpoint.";
        say(m + gaveUp(r));
      }).catch(function () {
        say("The checkpoint was not saved: checkpoints may be off or the cache folder full.");
      }).then(function () { busy(b, false); });
      return;
    }
    var items = Array.prototype.map.call(
      document.querySelectorAll("section.file[data-path][data-p]"),
      function (s) { return { path: s.dataset.path, patch: s.dataset.p }; });
    post("/_viewed/all", { items: items, checkpoint: true }).then(function (r) {
      if (r.checkpoint) tab(r.checkpoint);
      say((r.checkpoint ? "Checkpoint saved, marks kept: " + r.marked + "."
        : "Marks kept: " + r.marked + ".") + gaveUp(r));
      document.dispatchEvent(new CustomEvent("gt:viewed-changed"));
    }).catch(function () {
      say("The marks were not saved.");
    }).then(function () { busy(b, false); });
  });
})();
