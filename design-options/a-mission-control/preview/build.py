#!/usr/bin/env python3
"""Assemble the preview: copy the design files, the shared map assets and four real plugin pages next to a
mock-API shim, rewriting absolute /static/ links to relative ones. Run by serve.py; safe to re-run."""
import pathlib, re, shutil
HERE = pathlib.Path(__file__).resolve().parent
DESIGN = HERE.parent
REPO = DESIGN.parent.parent                       # OSNT_Tool
PORTAL = REPO / 'crates' / 'portal' / 'assets'
PLUGINS = REPO.parent / 'osnt-plugins' / 'plugins'
SHIM = '<script src="{p}mock.js"></script>'

def page(src, dst, prefix, extra_head=''):
    s = pathlib.Path(src).read_text()
    s = s.replace('"/static/', '"' + prefix + 'static/')
    s = re.sub(r'(<head[^>]*>)', r'\1\n  ' + SHIM.format(p=prefix) + extra_head, s, count=1)
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(s)

(HERE / 'static').mkdir(exist_ok=True)
for f in ('osnt-ui.css', 'osnt-theme.js'):
    shutil.copy(DESIGN / f, HERE / 'static' / f)
shutil.copy(PORTAL / 'osnt-map.js', HERE / 'static' / 'osnt-map.js')
shutil.copytree(PORTAL / 'leaflet', HERE / 'static' / 'leaflet', dirs_exist_ok=True)
for f in ('index.html', 'login.html', 'settings.html'):
    page(DESIGN / f, HERE / f, '')
THEME = '\n  <script src="{p}static/osnt-theme.js"></script>'
for name in ('packages', 'adsb', 'link-graph', 'map'):
    page(PLUGINS / name / 'assets' / 'index.html', HERE / 'p' / name / 'index.html', '../../', THEME.format(p='../../'))
# Plugin pages that the launcher links to but the harness does not ship answer with a plain note.
print('preview built in', HERE)
