// Apply cached prefs before first paint (mirrored by PrefsProvider); defaults: dark, green, system language.
// Kept as an external file so the strict CSP (script-src 'self') needs no inline-script hash.
(function () {
  var d = document.documentElement;
  try {
    var t = localStorage.getItem("memdock.theme") || "dark";
    var dark = t === "system" ? !window.matchMedia || window.matchMedia("(prefers-color-scheme: dark)").matches : t !== "light";
    d.classList.toggle("dark", dark);
    d.setAttribute("data-accent", localStorage.getItem("memdock.accent") || "green");
    var l = localStorage.getItem("memdock.lang") || "system";
    if (l === "system") l = navigator.language;
    d.lang = String(l).toLowerCase().indexOf("zh") === 0 ? "zh-CN" : "en";
  } catch (e) {}
})();
