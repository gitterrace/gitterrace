// A font blob's sample: the browser loads the file from /raw as a FontFace and the sample
// shows once it is usable. A file the browser refuses leaves the sample hidden. No style
// element or attribute is written, only the CSSOM, which the page policy allows.
(function () {
  if (!window.FontFace || !document.fonts) return;
  Array.prototype.forEach.call(document.querySelectorAll("[data-font-src]"), function (box, i) {
    var name = "gt-sample-" + i;
    var url = box.getAttribute("data-font-src").replace(/"/g, "%22");
    var face = new FontFace(name, 'url("' + url + '")');
    face.load().then(function () {
      document.fonts.add(face);
      box.style.fontFamily = '"' + name + '", sans-serif';
      box.hidden = false;
    }, function () {});
  });
})();
