/* Optional manual theme switch for OSNT pages. Load in <head>, before the stylesheet paints:
 *   <script src="/static/osnt-theme.js"></script>
 * Applies the saved choice (light | dark) as <html data-theme>; with no choice the OS decides.
 * Any element with [data-theme-toggle] becomes the switch. Storage is guarded: it may be blocked. */
(() => {
  const KEY = 'osnt.theme', root = document.documentElement;
  const get = () => { try { return localStorage.getItem(KEY); } catch { return null; } };
  const put = v => { try { localStorage.setItem(KEY, v); } catch {} };
  const os = () => matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  const apply = v => { if (v === 'light' || v === 'dark') root.setAttribute('data-theme', v); else root.removeAttribute('data-theme'); };
  apply(get());
  const current = () => root.getAttribute('data-theme') || os();
  function sync() {
    document.querySelectorAll('[data-theme-toggle]').forEach(b => {
      const dark = current() === 'dark';
      b.setAttribute('aria-pressed', String(dark));
      b.setAttribute('aria-label', dark ? 'Switch to light theme' : 'Switch to dark theme');
      b.title = dark ? 'Switch to light theme' : 'Switch to dark theme';
    });
  }
  document.addEventListener('click', e => {
    const b = e.target.closest && e.target.closest('[data-theme-toggle]'); if (!b) return;
    const next = current() === 'dark' ? 'light' : 'dark'; put(next); apply(next); sync();
  });
  addEventListener('storage', e => { if (e.key === KEY) { apply(e.newValue); sync(); } });
  document.addEventListener('DOMContentLoaded', sync);
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', sync);
})();
