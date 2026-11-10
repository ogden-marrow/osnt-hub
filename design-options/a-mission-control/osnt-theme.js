/* OSNT theme switch. Load in <head> (before paint) with: <script src="/static/osnt-theme.js"></script>
 * Applies a saved choice (system | light | dark) as <html data-theme>, and wires any
 * <button data-theme-toggle> to cycle it. Storage may be blocked: everything is guarded. */
(function () {
  var K = 'osnt.theme', root = document.documentElement, modes = ['system', 'light', 'dark'];
  function get() { try { var v = localStorage.getItem(K); return modes.indexOf(v) > 0 ? v : 'system'; } catch (e) { return 'system'; } }
  function apply(m) { if (m === 'system') root.removeAttribute('data-theme'); else root.setAttribute('data-theme', m); }
  function label(b, m) {
    var t = b.querySelector('[data-theme-label]'); if (t) t.textContent = m === 'system' ? 'Auto' : m;
    b.setAttribute('aria-label', 'Theme: ' + m + '. Activate to change.'); b.title = 'Theme: ' + m;
  }
  apply(get());
  document.addEventListener('DOMContentLoaded', function () {
    var bs = document.querySelectorAll('[data-theme-toggle]');
    for (var i = 0; i < bs.length; i++) (function (b) {
      label(b, get());
      b.addEventListener('click', function () {
        var m = modes[(modes.indexOf(get()) + 1) % 3];
        try { if (m === 'system') localStorage.removeItem(K); else localStorage.setItem(K, m); } catch (e) {}
        apply(m); for (var j = 0; j < bs.length; j++) label(bs[j], m);
      });
    })(bs[i]);
  });
})();
