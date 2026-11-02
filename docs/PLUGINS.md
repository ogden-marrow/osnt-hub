# Writing an OSNT plugin

A plugin is a separate Rust service. The portal core finds it through a manifest, proxies `/p/<id>/` to it,
and adds its card to the launcher. `crates/example` is a working starting point.

## 1. The service

- Listen on **loopback only** (`127.0.0.1:<port>`); pick an unused port (adsb uses 8101, example 8190).
- Serve your page at `/` and your API under `/api/...`. The core strips the `/p/<id>` prefix.
- In pages use **relative URLs** (`fetch('api/x')`). Shared assets are absolute: `/static/leaflet/leaflet.js`,
  `/static/osnt-map.js` (`await OSNT.createMap('map')` gives the shared map with the layer catalog from `layers.toml`).
- Optional but recommended: `GET /tools` returns a JSON array of operations
  `{ name, description, method, path, parameters (JSON schema) }` so the planned agent layer can call you.
- Prefer pure-Rust dependencies (the Pi build uses `rust-lld`, no C toolchain). For HTTPS use `ureq` with
  `rustls-no-provider` + `rustls-webpki-roots` + `rustls-rustcrypto` (see `crates/adsb/src/main.rs`, `make_agent`);
  for storage use `redb`.
- Persistent data: write under the systemd `StateDirectory` (see below). Secrets (API keys) go in environment
  variables set on the Pi, never in the repo.

## 2. Wiring it in

1. Add the crate to `members` in the root `Cargo.toml`.
2. `deploy/plugins.d/<id>.toml`:
   ```toml
   id = "example"
   name = "Example"
   description = "Shown on the launcher card."
   grid = "Tools"          # existing grid name, or a new one is created
   port = 8190
   ```
3. `deploy/osnt-<id>.service`: copy `deploy/osnt-adsb.service`, change binary, bind env, and `StateDirectory`.
4. Add a `Component { .. }` entry to `COMPONENTS` in `crates/deploy/src/main.rs`.
5. `cargo deploy <id>` builds for the Pi, installs binary, unit and manifest, and restarts. The card appears on
   the launcher immediately (it shows "offline" while the service is down).

## 3. Run locally

```bash
cargo run -p osnt-example                                   # the plugin on :8190
OSNT_PLUGINS_DIR=deploy/plugins.d cargo run -p osnt-portal  # the core on :8080 will proxy it
```
