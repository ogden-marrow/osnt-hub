# Option B: Field notebook

## Rationale
An investigator's notebook: warm paper ground, blue-black ink, one restrained accent (fountain-pen blue). A serif carries titles and group headings; the system sans carries working text. Surfaces are sheets separated by tone and quiet ruled lines, corners barely rounded, no shadows at rest. The launcher is an index, not a card wall: a contents list, dotted-leader group headings, ruled two-column rows; only pinned favourites become index cards. Dark theme is a lamp-lit desk. System font stacks only; no external requests. Every class name and `--variable` of the old system is kept.

## Run the preview
    python3 design-options/b-field-notebook/preview/serve.py      # http://localhost:8765/

Launcher, login, settings, plus real packages, adsb, link-graph and map pages with mocked `/api`. Add `?theme=dark` to any URL to force a theme. Map tiles need internet.

## Adopt
Copy into `crates/portal/assets/`: `osnt-ui.css`, `osnt-theme.js` (new, optional manual theme switch), `index.html`, `login.html`, `settings.html`. Plugins need no change; to let them follow the manual theme, add `<script src="/static/osnt-theme.js"></script>` in their `<head>` (settings already mirrors the theme into its frame). `setup.html` is not restyled but inherits the CSS. Check `osnt-theme.js` is served under `/static/`.
