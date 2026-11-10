# Option C: Workbench

## Rationale
The hub stops being a menu you leave and becomes a bench you stay on. A rail holds open tools, pinned, recent and the 11 groups; every tool opens as a tab in its own iframe and stays alive, so a map keeps its view while you pivot. Two tabs can sit side by side with a draggable divider, and the address bar is a shareable workspace (`/#/p/adsb/?q=N123AB|/p/link-graph/`). Ctrl/Cmd+K is a palette: it searches tools, jumps to deep links (`adsb N123AB`) and recognises pasted leads (IP, domain, ASN, coordinates, CVE) and offers every tool that takes them. Plugin content stays flat and quiet; glass is only on the chrome.

## Preview
    cd design-options/c-workbench/preview && python3 -m http.server 8077
then open http://localhost:8077/ (also /settings/ and /login/). `/api/*` and the plugins' APIs are static mocks (4 real plugin pages with data: packages, adsb, link-graph, map; the other plugin pages load but their APIs 404, as designed). Map tiles need internet.

## Adopt
Copy `osnt-ui.css`, `index.html`, `login.html`, `settings.html` into `crates/portal/assets/`. `setup.html` and all plugins work unchanged (every class and variable is kept; grep-verified).
Portal changes: none required. It must keep allowing same-origin framing of `/p/*` (settings.html already relies on it). Optional: keep `/` serving `index.html` for hash deep links.
Plugins pick up the manual theme only inside the workbench (the shell sets `data-theme` on frames); standalone plugin pages follow the OS.

## Keys
Ctrl/Cmd+K palette, `/` palette, Shift+Enter open to the side, Ctrl+\ split, Alt+1..9 tab, Alt+0 overview, Alt+W close, Alt+[ ] cycle, Ctrl/Cmd+B rail.
