# OSNT

Open source intelligence portal, written in Rust, deployed straight onto Raspberry Pi OS (no containers).

## Layout

- `crates/portal` – axum web server. Static assets (including vendored Leaflet) are embedded in the binary.
- `crates/adsb` – ADS-B plugin: live/military/emergency aircraft from adsb.lol, position history in an embedded redb file.
- `crates/deploy` – Rust deploy tool: builds, copies over SSH, installs the systemd unit, restarts.
- `deploy/osnt-portal.service` – hardened systemd unit (DynamicUser, read-only filesystem).
- `flash-pi.sh` – one-off SD card flasher for the Pi 5 (Raspberry Pi OS Lite, key-only SSH).

## Use

```bash
rustup target add aarch64-unknown-linux-musl   # once
cargo run -p osnt-portal                       # local dev on :8080
cargo deploy                                   # build + deploy to the Pi
cargo deploy status | logs
```

Deploy target defaults to `osnt@osnt-pi.local` with key `~/.ssh/osnt_pi_ed25519`; override with
`OSNT_DEPLOY_HOST` and `OSNT_SSH_KEY`. Server config: `OSNT_BIND` (default `0.0.0.0:8080`),
`OSNT_LAYERS` / `OSNT_TOOLS` (paths to override the layer catalog and launcher grids; re-read per request).

Map layers (`crates/portal/layers.toml`) are streamed from public tile services on demand; nothing is stored locally.
Check each provider's usage policy before real traffic (the public OSM tile server in particular).

## Plugins

A plugin is a separate Rust service on loopback plus a manifest in `/etc/osnt/plugins.d/<id>.toml`
(`id`, `name`, `description`, `grid`, `port`; see `deploy/plugins.d/adsb.toml`). The core reverse-proxies
`/p/<id>/` to it and merges its card into the launcher grid (shown "offline" if the service is down).
Plugin pages must use relative URLs and may reuse `/static/osnt-map.js` for the shared map and layer catalog.
A plugin may also serve `GET /tools`: a JSON list of its operations (name, description, path, JSON-schema
parameters), which is the contract the planned agent layer will use to call plugins.

`cargo deploy [portal|adsb ...]` builds, installs the unit and manifest, and restarts. Everything is pure Rust
(TLS via rustls with the RustCrypto provider), so no C cross-toolchain is needed.
