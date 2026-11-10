/* Preview harness: answers the hub's and the four plugins' /api calls with believable fixtures so the pages
 * render with real-looking content. Loaded before any page script. Not part of the design: delete when adopting. */
(() => {
  // ?theme=light|dark forces a theme for screenshots (the real pages use osnt-theme.js and the toggle).
  try { const t = new URLSearchParams(location.search).get('theme'); if (t) { const a = () => document.documentElement.setAttribute('data-theme', t); a(); document.addEventListener('DOMContentLoaded', a); } } catch {}
  // Seed a few pins and recents the first time, so the launcher shows its full shape.
  try { if (localStorage.getItem('osnt.favs') === null) { localStorage.setItem('osnt.favs', JSON.stringify(['/p/adsb/', '/p/map/', '/p/link-graph/'])); localStorage.setItem('osnt.recent', JSON.stringify(['/p/packages/', '/p/imagery/', '/p/cve-watch/'])); } } catch {}
  try { const v = new URLSearchParams(location.search).get('view'); if (v) localStorage.setItem('osnt.view', JSON.stringify(v)); } catch {}
  const real = window.fetch.bind(window);
  const here = document.currentScript.src;
  const json = (o, status = 200) => new Response(JSON.stringify(o), { status, headers: { 'Content-Type': 'application/json' } });
  const rnd = (() => { let s = 7; return () => (s = (s * 16807) % 2147483647) / 2147483647; })();

  // ---- adsb: ~70 aircraft over Florida ----
  const types = ['B738', 'A320', 'E75L', 'C172', 'B77W', 'A21N', 'CRJ9', 'PC12', 'B752', 'H60'];
  const air = Array.from({ length: 72 }, (_, i) => {
    const hex = (0xa00000 + Math.floor(rnd() * 0x5fffff)).toString(16);
    const ground = rnd() < .08, mil = i % 17 === 3;
    return { hex, flight: mil ? 'RCH' + (200 + i) : ['DAL', 'AAL', 'SWA', 'UAL', 'JBU', 'N'][i % 6] + (100 + Math.floor(rnd() * 8900)),
      reg: 'N' + Math.floor(100 + rnd() * 899) + 'AB', type: mil ? 'C17' : types[i % types.length],
      lat: 25.2 + rnd() * 6.5, lon: -86 + rnd() * 7.5, alt: ground ? null : Math.round(1000 + rnd() * 38000), on_ground: ground,
      gs: ground ? 8 : Math.round(120 + rnd() * 380), track: Math.round(rnd() * 359), squawk: '1200', mil, seen_pos: 1 };
  });
  const emerg = [{ ...air[11], squawk: '7700', meaning: 'General emergency', flight: 'N482PD', type: 'C172' }];

  // ---- link-graph: a small infrastructure graph ----
  const N = (id, type, label, x, y, group) => ({ id, type, label, x, y, pinned: false, group: group || '', note: '', source: 'user' });
  const graph = { id: 'g1', name: 'Harbourline Logistics infrastructure', nodes: [
    N('n1', 'organisation', 'Harbourline Logistics Ltd', 0, 0), N('n2', 'domain', 'harbourline.example', 130, -70, 'core'), N('n3', 'ip', '203.0.113.24', 250, -120, 'core'),
    N('n4', 'asn', 'AS64500', 370, -60, 'core'), N('n5', 'certificate', 'CN=*.harbourline.example', 250, -10), N('n6', 'domain', 'tracking.harbourline.example', 140, 60),
    N('n7', 'ip', '198.51.100.7', 270, 100), N('n8', 'vessel', 'MV Calla Star (IMO 9387421)', -140, 70), N('n9', 'place', 'Port of Rotterdam', -250, 10),
    N('n10', 'organisation', 'Calla Shipping SA', -130, -80), N('n11', 'document', 'Charter party 2023-114', -260, -90), N('n12', 'aircraft', 'N482PD', -120, 170),
    N('n13', 'domain', 'hl-mail.example', 20, -160), N('n14', 'url', 'https://harbourline.example/portal', 150, -190), N('n15', 'hash', 'a3f1…9c2e', 400, 60) ],
    edges: [['n1','n2','owns'],['n2','n3','resolves to'],['n3','n4','announced by'],['n2','n5','serves'],['n2','n6','subdomain'],['n6','n7','resolves to'],['n1','n10','charters from'],['n10','n8','operates'],['n8','n9','last port'],['n10','n11','party to'],['n1','n12','shared contact'],['n2','n13','mail'],['n2','n14','page'],['n4','n15','sample']].map(([from, to, label], i) => ({ id: 'e' + i, from, to, label })) };

  // ---- packages ----
  const P = (id, name, version, status, d, extra) => ({ id, name, version, installed_version: status === 'available' ? null : (status === 'update' ? '0.1.1' : version), status, installable: true, core: false, description: d, requires: [], missing_requires: [], dependents: [], repo: 'osnt/plugins', ...extra });
  const pkgs = { github: false, other_installed: [{ id: 'example', name: 'Example plugin', version: '0.0.1' }], repos: [
      { url: 'https://github.com/osnt/plugins', ok: true, count: 51, published: true }, { url: 'https://github.com/jpalenchar/osnt-extras', ok: true, count: 3, published: false } ],
    packages: [
      P('adsb', 'Aircraft (ADS-B)', '0.1.2', 'installed', 'Live aircraft, military traffic and emergency squawks, with a recorded position history.', { dependents: ['adsb-history'] }),
      P('adsb-history', 'Aircraft history', '0.1.0', 'installed', 'Search recorded ADS-B flights by hex, registration or area.', { requires: ['adsb'] }),
      P('link-graph', 'Link graph', '0.2.4', 'update', 'Draw entities and relationships, expand nodes with other plugins and export the graph.'),
      P('map', 'Map', '0.3.0', 'installed', 'Layered world map with elevation readout and place search.', { core: true }),
      P('packages', 'Packages', '0.1.0', 'installed', 'Install, update and remove plugins from GitHub repositories.', { core: true }),
      P('ais-vessels', 'Vessels (AIS)', '0.1.5', 'available', 'Live ship positions and port calls from public AIS feeds.'),
      P('cve-watch', 'CVE watch', '0.1.1', 'available', 'Track new vulnerabilities for the products you list.', { requires: ['credentials'], missing_requires: ['credentials'] }),
      P('sanctions-screen', 'Sanctions screening', '0.1.0', 'available', 'Check names against consolidated sanctions lists.', { installable: false }),
    ] };

  const routes = [
    [/\/api\/tools$/, () => real(new URL('tools.json', here))],
    [/\/api\/config$/, () => json({ map: { center: [29.65, -82.34], zoom: 7 }, layer: [
      { id: 'osm', name: 'OpenStreetMap', kind: 'xyz', group: 'base', default: true, url: 'https://tile.openstreetmap.org/{z}/{x}/{y}.png', max_zoom: 19, attribution: '&copy; OpenStreetMap contributors' },
      { id: 'topo', name: 'OpenTopoMap', kind: 'xyz', group: 'base', url: 'https://{s}.tile.opentopomap.org/{z}/{x}/{y}.png', subdomains: ['a', 'b', 'c'], max_zoom: 17, attribution: '&copy; OpenTopoMap' } ] })],
    [/\/api\/health$/, () => json({ status: 'ok', version: '0.4.2' })],
    [/\/auth\/status$/, () => json({ logged_in: false, configured: true })],
    [/api\/aircraft$/, () => json(air)], [/api\/military$/, () => json(air.filter(a => a.mil))], [/api\/emergencies$/, () => json(emerg)],
    [/api\/search$/, () => json([air[2]])],
    [/api\/trail\//, () => json(Array.from({ length: 40 }, (_, i) => ({ lat: air[0].lat - i * .02, lon: air[0].lon - i * .03 })))],
    [/api\/graphs$/, (u, m) => m === 'GET' ? json({ graphs: [{ id: 'g1', name: graph.name, nodes: graph.nodes.length }] }) : json({ id: 'g2' })],
    [/api\/graphs\/g1$/, (u, m) => json(m === 'GET' ? JSON.parse(JSON.stringify(graph)) : {})],
    [/\/p\/[^/]+\/tools$/, () => json({}, 200)],
    [/\/p\/case-notebook\/api\/cases/, () => json({ cases: [] })],
    [/api\/packages$/, () => json(pkgs)], [/api\/github$/, () => json({ connected: false })],
  ];
  window.fetch = (input, init) => {
    const req = typeof input === 'string' || input instanceof URL ? new URL(input, location.href) : new URL(input.url);
    if (req.origin === location.origin) {
      const method = (init && init.method) || 'GET';
      for (const [re, fn] of routes) if (re.test(req.pathname)) return Promise.resolve(fn(req, method));
      if (/\/(api|auth)\//.test(req.pathname)) return Promise.resolve(json({}));
    }
    return real(input, init);
  };
})();
