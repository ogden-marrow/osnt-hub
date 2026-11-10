#!/usr/bin/env python3
"""Static preview harness for the OSNT hub design option.

  python3 preview/serve.py            # then open http://localhost:8765/

Serves the launcher, login and settings pages from the parent folder, four real plugin pages
(copied unchanged into preview/plugins/) and mocked /api responses. Nothing here touches a hub.
Standard library only.
"""
import json, math, os, random, re, sys, tomllib
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)  # the option folder: osnt-ui.css, index.html, ...
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8765

def read(path, mode="rb"):
    with open(path, mode) as f:
        return f.read()

# --- mock data -------------------------------------------------------------------------------------
TOOLS = json.loads(read(os.path.join(HERE, "mock", "tools.json"), "r"))
LAYERS = tomllib.loads(read(os.path.join(HERE, "mock", "layers.toml"), "r"))

def aircraft(lat, lon, radius_nm):
    rnd = random.Random(7)
    kinds = [("B738", "N"), ("A320", "N"), ("E75L", "N"), ("C172", "N"), ("B763", "N"), ("GLF6", "N"), ("H60", "")]
    out = []
    for i in range(34):
        ang, dist = rnd.uniform(0, 2 * math.pi), radius_nm * math.sqrt(rnd.random()) * 0.9
        dlat, dlon = dist * math.cos(ang) / 60, dist * math.sin(ang) / (60 * max(0.2, math.cos(math.radians(lat))))
        t, p = rnd.choice(kinds)
        mil = i % 11 == 3
        ground = i % 13 == 5
        alt = None if ground else rnd.choice([1800, 3400, 6200, 11000, 18500, 24000, 33000, 36000, 39000])
        out.append({
            "hex": f"a{rnd.randrange(0x10000, 0xfffff):05x}", "flight": rnd.choice(["DAL", "SWA", "AAL", "UAL", "FDX", "RCH"]) + str(rnd.randrange(100, 2999)),
            "reg": f"N{rnd.randrange(100, 999)}{rnd.choice('ABCDEFGH')}{rnd.choice('JKLMNPQ')}", "type": t, "lat": lat + dlat, "lon": lon + dlon,
            "alt": alt, "on_ground": ground, "gs": 0 if ground else rnd.randrange(110, 520), "track": rnd.randrange(0, 360),
            "squawk": f"{rnd.randrange(1000, 7000)}", "mil": mil, "seen_pos": rnd.choice([1, 2, 3, 40]),
        })
    return out

NODES = [
    ("n1", "organisation", "Meridian Holdings Ltd", "core", "Registered 2019; nominee directors."),
    ("n2", "domain", "meridian-holdings.example", None, ""), ("n3", "ip", "203.0.113.24", None, ""), ("n4", "asn", "AS64500", None, ""),
    ("n5", "vessel", "MV Corvina (IMO 9321483)", "core", ""), ("n6", "organisation", "Corvina Shipping SA", None, ""), ("n7", "place", "Port of Rotterdam", None, ""),
    ("n8", "aircraft", "N512MH (a6f2d1)", None, ""), ("n9", "domain", "mail.meridian-holdings.example", None, ""), ("n10", "certificate", "CN=*.meridian-holdings.example", None, ""),
    ("n11", "url", "meridian-holdings.example/about", None, ""), ("n12", "organisation", "Harlow Trust", None, ""), ("n13", "event", "Dry dock, Gdansk, Mar 2024", None, ""),
    ("n14", "document", "Annual return 2023", None, ""),
]
EDGES = [("n1", "n2", "operates"), ("n2", "n3", "resolves to"), ("n3", "n4", "announced by"), ("n1", "n5", "owns"), ("n5", "n6", "managed by"), ("n5", "n7", "last port"),
         ("n1", "n8", "operates"), ("n2", "n9", "subdomain"), ("n9", "n10", "presents"), ("n2", "n11", ""), ("n1", "n12", "held by"), ("n5", "n13", "involved in"), ("n1", "n14", "filed")]
def graph():
    rnd = random.Random(3)
    return {"id": "g1", "name": "Meridian Holdings", "nodes": [dict(id=i, type=t, label=l, x=0, y=0, pinned=False, group=g, note=n, source="") for i, t, l, g, n in NODES],
            "edges": [dict(id=f"e{k}", **{"from": a, "to": b, "label": l}) for k, (a, b, l) in enumerate(EDGES)]}

PKGS = {
    "packages": [
        dict(id="adsb", name="Aircraft (ADS-B)", description="Live aircraft, military traffic and emergency squawks, with a recorded position history.", version="0.4.1", installed_version="0.4.1", status="installed", installable=True, core=False, requires=[], missing_requires=[], dependents=["adsb-history"], repo="https://github.com/osnt/plugins"),
        dict(id="link-graph", name="Link graph", description="Maltego-style entity graph for domains, IPs, ASNs, organisations, vessels, aircraft and places.", version="0.3.0", installed_version="0.2.4", status="update", installable=True, core=False, requires=[], missing_requires=[], dependents=[], repo="https://github.com/osnt/plugins"),
        dict(id="ais-vessels", name="Vessels (AIS)", description="Ship positions, tracks and registry details from open AIS feeds.", version="0.2.0", installed_version="0.2.0", status="installed", installable=True, core=False, requires=[], missing_requires=[], dependents=[], repo="https://github.com/osnt/plugins"),
        dict(id="packages", name="Packages", description="Package repositories, and which of their plugins are installed on this hub.", version="0.2.2", installed_version="0.2.2", status="installed", installable=True, core=True, requires=[], missing_requires=[], dependents=[], repo="https://github.com/osnt/plugins"),
        dict(id="yt-mp3", name="Audio extractor", description="Pull the audio track from a public video for transcription. Needs ffmpeg on the Pi.", version="0.1.3", status="available", installable=True, core=False, requires=["media-verify"], missing_requires=["media-verify"], dependents=[], repo="https://github.com/osnt/plugins"),
        dict(id="media-verify", name="Media verification", description="Reverse image search links, EXIF and error-level checks for a photo or clip.", version="0.5.0", status="available", installable=True, core=False, requires=[], missing_requires=[], dependents=[], repo="https://github.com/osnt/plugins"),
        dict(id="rail-live", name="Rail (live)", description="Live train positions and delays for supported networks.", version="0.1.0", status="available", installable=False, core=False, requires=[], missing_requires=[], dependents=[], repo="https://github.com/osnt/plugins-community"),
    ],
    "other_installed": [], "github": False,
    "repos": [dict(url="https://github.com/osnt/plugins", ok=True, count=51, published=True), dict(url="https://github.com/osnt/plugins-community", ok=False, error="No published build yet for this repository.")],
}

def mock(path, q):
    """Return (status, obj) for an API path, or None."""
    if path == "/api/tools": return 200, TOOLS
    if path == "/api/config": return 200, LAYERS
    if path == "/api/health": return 200, {"status": "ok", "version": "0.9.0"}
    if path == "/auth/status": return 200, {"logged_in": False, "configured": True}
    if path == "/auth/logout": return 200, {}
    if path == "/auth/login/begin": return 400, {"error": "Preview only: no passkeys here."}
    m = re.match(r"^/p/([\w-]+)/(api/.*)$", path)
    if not m: return None
    pid, rest = m.groups()
    if pid == "packages":
        if rest == "api/packages": return 200, PKGS
        if rest == "api/github": return 200, {"connected": False}
        if rest == "api/details": return 200, {"system": {}, "secrets": []}
    if pid == "adsb":
        lat, lon = float(q.get("lat", [29.65])[0]), float(q.get("lon", [-82.34])[0])
        radius = float(q.get("radius", [150])[0])
        if rest == "api/aircraft": return 200, aircraft(lat, lon, radius)
        if rest == "api/military": return 200, [a for a in aircraft(35, -90, 900) if a["mil"]]
        if rest == "api/emergencies":
            a = dict(aircraft(29.65, -82.34, 100)[2], squawk="7700", meaning="General emergency"); return 200, [a]
        if rest.startswith("api/trail/"): return 200, [{"lat": 29.0 + i * .03, "lon": -83.2 + i * .045 + math.sin(i / 5) * .04} for i in range(40)]
        if rest == "api/search": return 200, aircraft(29.65, -82.34, 80)[:1]
    if pid == "link-graph":
        if rest == "api/graphs": return 200, {"graphs": [{"id": "g1", "name": "Meridian Holdings", "nodes": len(NODES)}]}
        if rest == "api/graphs/g1": return 200, graph()
    return 404, {"error": "Not part of the preview"}

# --- serving -----------------------------------------------------------------------------------------
THEME_SNIPPET = ("<script>try{var t=new URLSearchParams(location.search).get('theme');if(t)localStorage.setItem('osnt.theme',t)}catch(e){}</script>"
                 "<script src=\"/static/osnt-theme.js\"></script>")
PARAM_ONLY = ("<script>try{var t=new URLSearchParams(location.search).get('theme');if(t)localStorage.setItem('osnt.theme',t)}catch(e){}</script>")

def page(path):
    if path in ("/", "/index.html"): return os.path.join(ROOT, "index.html")
    if path == "/login": return os.path.join(ROOT, "login.html")
    if path == "/settings": return os.path.join(ROOT, "settings.html")
    m = re.match(r"^/p/([\w-]+)/?$", path)
    if m:
        f = os.path.join(HERE, "plugins", m.group(1) + ".html")
        return f if os.path.exists(f) else "stub:" + m.group(1)
    return None

class H(SimpleHTTPRequestHandler):
    def log_message(self, *a): pass

    def send(self, status, body, ctype):
        self.send_response(status); self.send_header("Content-Type", ctype); self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store"); self.end_headers(); self.wfile.write(body)

    def do_POST(self): self.do_GET()
    do_PUT = do_DELETE = do_POST

    def do_GET(self):
        u = urlparse(self.path); path, q = u.path, parse_qs(u.query)
        static = {"/static/osnt-ui.css": (os.path.join(ROOT, "osnt-ui.css"), "text/css"), "/static/osnt-theme.js": (os.path.join(ROOT, "osnt-theme.js"), "text/javascript"),
                  "/static/osnt-map.js": (os.path.join(HERE, "static", "osnt-map.js"), "text/javascript")}
        if path in static:
            f, t = static[path]; return self.send(200, read(f), t + "; charset=utf-8")
        if path.startswith("/static/leaflet/"):
            f = os.path.normpath(os.path.join(HERE, "static", path[len("/static/"):]))
            if f.startswith(os.path.join(HERE, "static")) and os.path.isfile(f):
                ct = {"css": "text/css", "js": "text/javascript", "png": "image/png", "svg": "image/svg+xml"}.get(f.rsplit(".", 1)[-1], "application/octet-stream")
                return self.send(200, read(f), ct)
        r = mock(path, q)
        if r: return self.send(r[0], json.dumps(r[1]).encode(), "application/json")
        f = page(path)
        if f and f.startswith("stub:"):
            body = (f"<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><link rel=stylesheet href=/static/osnt-ui.css><main class='page narrow'><div class=state><div class=h3>{f[5:]} is not part of this preview</div><p>The preview serves the launcher, login, settings and four plugin pages.</p><a class='btn' href=/>Back to the hub</a></div></main>").encode()
            return self.send(200, body, "text/html; charset=utf-8")
        if f:
            html = open(f, encoding="utf-8").read()
            snippet = PARAM_ONLY if "osnt-theme.js" in html else THEME_SNIPPET
            html = html.replace("<head>", "<head>" + snippet, 1) if "<head>" in html else snippet + html
            return self.send(200, html.encode(), "text/html; charset=utf-8")
        self.send(404, b"not found", "text/plain")

if __name__ == "__main__":
    print(f"OSNT field-notebook preview: http://localhost:{PORT}/   (Ctrl+C to stop)")
    ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
