# Option A: Mission control

## Rationale
A dark-first ops console. Graphite ground, one instrument-amber accent (actions and selection only); green, yellow and red are reserved for state. Dense 13px type, 4px corners, hairline panel edges, mono for numbers, paths and keys. The launcher gains a telemetry strip (running, down, planned, pinned, UTC clock) built from real tool data, a group rail, per-group status pips, grid or list layout, and keyboard control (`/`, Ctrl/Cmd+K, arrows, Enter, F to pin, V to switch layout). Light theme keeps the same structure. Every existing class and variable is kept, so plugins need no edits.

## Preview
```
python3 design-options/a-mission-control/preview/serve.py
```
Open http://localhost:8099/ . It copies the design and four real plugin pages (packages, adsb, link-graph, map) beside a fetch mock (`preview/mock.js`). Map tiles need internet. Add `?theme=light|dark` to force a theme.

## Adopt
Copy `osnt-ui.css`, `osnt-theme.js`, `index.html`, `login.html`, `settings.html` into `crates/portal/assets/` (the theme file is served as `/static/osnt-theme.js`; without it everything works, minus the toggle). To make plugin pages honour the toggle, add `<script src="/static/osnt-theme.js"></script>` to their `<head>`; otherwise they follow the OS setting. `setup.html` was not restyled but inherits the new CSS.
