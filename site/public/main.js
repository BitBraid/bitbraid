// Two things, neither needed for the page to work: the braid that runs
// behind the screenshot, and the first download button pointed at the
// visitor's system.
(function () {
  "use strict";

  var SVG = "http://www.w3.org/2000/svg";

  function make(name, attributes, parent) {
    var node = document.createElementNS(SVG, name);
    for (var key in attributes) node.setAttribute(key, attributes[key]);
    if (parent) parent.appendChild(node);
    return node;
  }

  // Three strands plaited without end, as the mark in brand/ is plaited
  // once: strand i sits at A sin(t + 2 pi i / 3) across
  // the band and lies over its neighbour where sin 2(t + 2 pi i / 3) is the
  // greater. Each strand is one path; where it lies under another, a mask
  // cuts it back a gap beyond the edge of the one above. The picture
  // repeats every period, so moving it by one period loops without a seam.
  function drawBraid(host) {
    var scale = Math.min(1, Math.max(0.5, window.innerWidth / 1440));
    var amplitude = 52 * scale;
    var width = 34 * scale;
    var gap = 8 * scale;
    var period = Math.round(370 * scale);
    var blur = 22 * scale;
    var height = Math.ceil(2 * (amplitude + width / 2 + 3 * blur));
    var middle = height / 2;
    var span = Math.max(window.innerWidth, (window.screen && window.screen.width) || 0);
    var periods = Math.ceil(span / period) + 2;
    var total = periods * period;
    var step = period / 64;
    var turn = 2 * Math.PI;

    function y(strand, x) {
      return middle + amplitude * Math.sin((turn * x) / period + (turn * strand) / 3);
    }
    function depth(strand, x) {
      return Math.sin(2 * ((turn * x) / period + (turn * strand) / 3));
    }
    function line(strand, from, to) {
      var d = "";
      for (var x = from; x <= to + 0.001; x += step) {
        d += (d ? "L" : "M") + x.toFixed(1) + " " + y(strand, x).toFixed(1);
      }
      return d;
    }

    // Two strands cross where t = pi/2 - pi (i + j) / 3 + k pi; around each
    // crossing, the one beneath is cut along the stretch of the one above.
    var cuts = ["", "", ""];
    var pairs = [[0, 1], [0, 2], [1, 2]];
    for (var p = 0; p < pairs.length; p++) {
      var i = pairs[p][0];
      var j = pairs[p][1];
      for (var k = -1; k <= 2 * periods + 1; k++) {
        var t = Math.PI / 2 - (Math.PI * (i + j)) / 3 + k * Math.PI;
        var x = (t / turn) * period;
        if (x < -period / 4 || x > total + period / 4) continue;
        var over = depth(i, x) > depth(j, x) ? i : j;
        var under = over === i ? j : i;
        cuts[under] += line(over, x - period / 6, x + period / 6);
      }
    }

    var svg = make("svg", {
      width: total,
      height: height,
      viewBox: "0 0 " + total + " " + height,
      focusable: "false",
    });
    var defs = make("defs", {}, svg);
    var box = { x: 0, y: 0, width: total, height: height };
    var filter = make("filter", Object.assign({ id: "braid-glow", filterUnits: "userSpaceOnUse" }, box), defs);
    make("feGaussianBlur", { stdDeviation: blur.toFixed(1) }, filter);

    var glow = make("g", { class: "glow", filter: "url(#braid-glow)" }, svg);
    var strands = make("g", { class: "strands" }, svg);
    for (var s = 0; s < 3; s++) {
      var mask = make("mask", Object.assign({ id: "braid-cut-" + s, maskUnits: "userSpaceOnUse" }, box), defs);
      make("rect", { width: total, height: height, fill: "#fff" }, mask);
      make("path", { d: cuts[s], fill: "none", stroke: "#000", "stroke-width": (width + 2 * gap).toFixed(1) }, mask);
      var d = line(s, 0, total);
      make("path", { d: d, class: "l" + s, "stroke-width": (width * 1.3).toFixed(1) }, glow);
      make("path", { d: d, class: "l" + s, "stroke-width": width.toFixed(1), mask: "url(#braid-cut-" + s + ")" }, strands);
    }

    svg.style.setProperty("--period", -period + "px");
    svg.style.marginLeft = -total / 2 + "px";
    host.style.height = height + "px";
    host.replaceChildren(svg);
    return span;
  }

  var host = document.querySelector(".braid");
  if (host && document.createElementNS) {
    var covered = drawBraid(host);
    var drawnAt = window.innerWidth;
    requestAnimationFrame(function () {
      host.classList.add("is-drawn");
    });

    var waiting = 0;
    window.addEventListener("resize", function () {
      clearTimeout(waiting);
      waiting = setTimeout(function () {
        var now = window.innerWidth;
        if (now > covered || Math.abs(now - drawnAt) > drawnAt / 8) {
          covered = drawBraid(host);
          drawnAt = now;
        }
      }, 200);
    });

    // It stands still while it cannot be seen.
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (entries) {
        host.classList.toggle("is-still", !entries[0].isIntersecting);
      }).observe(host);
    }
  }

  // The visitor's system ------------------------------------------------

  var agent = navigator.userAgent || "";
  // Phones and tablets get the list: BitBraid is for desktops.
  if (/Android|iPhone|iPad|iPod/i.test(agent)) return;

  var platform =
    (navigator.userAgentData && navigator.userAgentData.platform) ||
    navigator.platform ||
    agent;
  var system = /mac/i.test(platform)
    ? "macos"
    : /win/i.test(platform)
      ? "windows"
      : /linux|x11|bsd/i.test(platform)
        ? "linux"
        : null;
  if (!system) return;

  var row = document.querySelector('.system[data-system="' + system + '"]');
  var link = row && row.querySelector("a.button");
  var button = document.getElementById("hero-download");
  if (!row || !link || !button) return;

  row.classList.add("yours");
  button.href = link.href;
  button.textContent = link.textContent;
  var others = document.getElementById("hero-others");
  if (others) others.textContent = "Other systems";
})();
