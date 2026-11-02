// One hover for the whole page: any element with data-tip="..." shows it after the pointer
// rests 300 ms, at once when Tab moves focus onto it, and drops it on leave, mouse-down, scroll
// and Esc. Focus a script puts back (a menu or dialog closing returns it to its opener) shows
// nothing, or every closed overlay would leave its opener's tip on screen.
(function () {
  var DELAY = 300;
  var GAP = 8;
  var tip = null;
  var current = null;
  var timer = 0;
  var mx = null;
  var tabbed = false;

  function box() {
    if (tip) return tip;
    tip = document.createElement("div");
    tip.className = "tip";
    tip.id = "gt-tip";
    tip.setAttribute("role", "tooltip");
    document.body.appendChild(tip);
    return tip;
  }

  // A commit message (.subj) is cut with an ellipsis when it runs out of room; only then does
  // it carry its full text as a tip.
  function cut(node) {
    var s = node && node.closest ? node.closest(".subj") : null;
    if (!s) return;
    if (s.scrollWidth > s.clientWidth) s.setAttribute("data-tip", s.textContent.trim());
    else s.removeAttribute("data-tip");
  }

  function target(node) {
    cut(node);
    return node && node.closest ? node.closest("[data-tip]") : null;
  }

  function place(el) {
    var t = box();
    var r = el.getBoundingClientRect();
    var vw = document.documentElement.clientWidth;
    var vh = document.documentElement.clientHeight;
    var w = t.offsetWidth;
    var h = t.offsetHeight;
    // A wide element (a heading) anchors at the pointer, a small one at its centre.
    var cx = mx !== null && r.width > 240 && mx >= r.left && mx <= r.right ? mx : r.left + r.width / 2;
    var left = Math.max(GAP, Math.min(cx - w / 2, vw - w - GAP));
    var top = r.top - h - GAP;
    if (top < GAP) top = r.bottom + GAP;
    top = Math.max(GAP, Math.min(top, vh - h - GAP));
    t.style.left = left + "px";
    t.style.top = top + "px";
  }

  function show(el) {
    var text = el.getAttribute("data-tip");
    if (!text) return;
    var t = box();
    t.textContent = text;
    current = el;
    el.setAttribute("aria-describedby", "gt-tip");
    place(el);
    t.classList.add("on");
  }

  function hide() {
    clearTimeout(timer);
    timer = 0;
    if (current) current.removeAttribute("aria-describedby");
    current = null;
    if (tip) tip.classList.remove("on");
  }

  function arm(el) {
    if (el === current) return;
    hide();
    timer = setTimeout(function () { show(el); }, DELAY);
    pending = el;
  }
  var pending = null;

  document.addEventListener("mouseover", function (e) {
    mx = e.clientX;
    var el = target(e.target);
    if (!el) return;
    if (el === current || el === pending && timer) return;
    arm(el);
  });
  document.addEventListener("mouseout", function (e) {
    var el = target(e.target);
    if (!el) return;
    var to = target(e.relatedTarget);
    if (to === el) return;
    hide();
    pending = null;
  });
  document.addEventListener("focusin", function (e) {
    var el = target(e.target);
    if (!el || !tabbed) return;
    mx = null;
    hide();
    show(el);
  });
  document.addEventListener("focusout", function () { hide(); pending = null; });
  document.addEventListener("mousedown", function () { tabbed = false; hide(); pending = null; }, true);
  document.addEventListener("scroll", function () { hide(); pending = null; }, true);
  // Capture phase, so the flag is set before any handler moves focus for this key.
  document.addEventListener("keydown", function (e) {
    tabbed = e.key === "Tab";
    if (e.key === "Escape") { hide(); pending = null; }
  }, true);
})();
