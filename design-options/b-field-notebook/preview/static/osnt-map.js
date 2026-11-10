// Shared map bootstrap for OSNT pages. Needs Leaflet loaded first.
//   const { map, cfg } = await OSNT.createMap('map', { readout: 'readout' });
// Layers come from /api/config (layers.toml) and are streamed from the internet on demand.
window.OSNT = (() => {
  // Elevation ramp (metres -> colour); Terrarium encodes height as R*256 + G + B/256 - 32768.
  const RAMP = [[-8000,[8,29,88]],[-200,[37,52,148]],[0,[65,182,196]],[1,[35,139,69]],[300,[161,217,155]],
                [800,[254,217,118]],[1500,[217,150,65]],[3000,[140,90,60]],[5000,[230,230,230]],[8800,[255,255,255]]];
  const decode = (r, g, b) => r * 256 + g + b / 256 - 32768;
  function colour(m) {
    for (let i = 1; i < RAMP.length; i++) {
      if (m <= RAMP[i][0]) {
        const [m0, c0] = RAMP[i - 1], [m1, c1] = RAMP[i], t = (m - m0) / (m1 - m0);
        return c0.map((v, k) => Math.round(v + (c1[k] - v) * t));
      }
    }
    return RAMP[RAMP.length - 1][1];
  }
  const fill = (u, subs, c) => u.replace('{s}', subs.length ? subs[(c.x + c.y) % subs.length] : '')
                                .replace('{z}', c.z).replace('{x}', c.x).replace('{y}', c.y);

  // Raw Terrarium tiles are cached so the elevation readout works even when the overlay is off.
  const rawTiles = new Map();
  function loadRaw(def, z, x, y) {
    const key = `${def.id}/${z}/${x}/${y}`;
    if (!rawTiles.has(key)) {
      rawTiles.set(key, new Promise((resolve, reject) => {
        const img = new Image(); img.crossOrigin = 'anonymous';
        img.onload = () => {
          const cv = document.createElement('canvas'); cv.width = cv.height = 256;
          const cx = cv.getContext('2d', { willReadFrequently: true }); cx.drawImage(img, 0, 0);
          resolve(cx.getImageData(0, 0, 256, 256));
        };
        img.onerror = reject;
        img.src = fill(def.url, def.subdomains || [], { x, y, z });
      }));
    }
    return rawTiles.get(key);
  }

  const Terrarium = L.GridLayer.extend({
    initialize(def) {
      this.def = def;
      L.GridLayer.prototype.initialize.call(this, {
        attribution: def.attribution, opacity: def.opacity ?? 1, maxNativeZoom: def.max_zoom, maxZoom: 19, tileSize: 256 });
    },
    createTile(c, done) {
      const cv = document.createElement('canvas'); cv.width = cv.height = 256;
      loadRaw(this.def, c.z, c.x, c.y).then(src => {
        const out = cv.getContext('2d').createImageData(256, 256);
        for (let i = 0; i < src.data.length; i += 4) {
          const [r, g, b] = colour(decode(src.data[i], src.data[i + 1], src.data[i + 2]));
          out.data[i] = r; out.data[i + 1] = g; out.data[i + 2] = b; out.data[i + 3] = 255;
        }
        cv.getContext('2d').putImageData(out, 0, 0); done(null, cv);
      }).catch(e => done(e, cv));
      return cv;
    },
  });

  function build(def) {
    const opts = { attribution: def.attribution, maxZoom: def.max_zoom ?? 19, maxNativeZoom: def.max_zoom ?? undefined,
                   opacity: def.opacity ?? 1, subdomains: def.subdomains?.length ? def.subdomains : 'abc' };
    switch (def.kind) {
      case 'xyz': return L.tileLayer(def.url, opts);
      case 'wms': return L.tileLayer.wms(def.url, { ...opts, layers: def.wms_layers, format: 'image/png', transparent: true });
      case 'terrarium': return new Terrarium(def);
      default: console.warn('unknown layer kind', def.kind); return null;
    }
  }

  async function createMap(el, { readout } = {}) {
    const cfg = await (await fetch('/api/config')).json();
    const map = L.map(el, { worldCopyJump: true }).setView(cfg.map.center, cfg.map.zoom);
    const bases = {}, overlays = {}, defs = {};
    let chosen = null;
    for (const def of cfg.layer) {
      const layer = build(def); if (!layer) continue;
      defs[def.id] = def;
      (def.group === 'base' ? bases : overlays)[def.name] = layer;
      if (def.group === 'base' && (def.default || !chosen)) chosen = layer;
      if (def.group !== 'base' && def.default) layer.addTo(map);
    }
    if (chosen) chosen.addTo(map);
    L.control.layers(bases, overlays, { collapsed: false }).addTo(map);
    L.control.scale().addTo(map);

    const elev = Object.values(defs).find(d => d.kind === 'terrarium');
    const out = readout && document.getElementById(readout);
    if (out) {
      let pending = 0;
      map.on('mousemove', async e => {
        const { lat, lng } = e.latlng;
        let text = `${lat.toFixed(4)}, ${((lng + 540) % 360 - 180).toFixed(4)}`;
        const ticket = ++pending;
        if (elev) {
          const z = Math.min(Math.max(Math.round(map.getZoom()), 0), elev.max_zoom ?? 13), n = 2 ** z;
          const fx = (((lng + 180) / 360) % 1 + 1) % 1 * n;
          const rad = lat * Math.PI / 180, fy = (1 - Math.log(Math.tan(rad) + 1 / Math.cos(rad)) / Math.PI) / 2 * n;
          if (fy >= 0 && fy < n) {
            try {
              const tile = await loadRaw(elev, z, Math.floor(fx), Math.floor(fy));
              const px = Math.floor((fx % 1) * 256), py = Math.floor((fy % 1) * 256), i = (py * 256 + px) * 4;
              text += `  ·  ${Math.round(decode(tile.data[i], tile.data[i + 1], tile.data[i + 2]))} m`;
            } catch { /* elevation unavailable here */ }
          }
        }
        if (ticket === pending) out.textContent = text;
      });
    }
    return { map, cfg };
  }

  return { createMap };
})();
