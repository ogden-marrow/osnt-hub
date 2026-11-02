# OSNT

Open source intelligence portal, written in Rust, deployed straight onto Raspberry Pi OS (no containers).

## Layout

- `crates/portal` – axum web server. Static assets (including vendored Leaflet) are embedded in the binary.
- `crates/common` – shared helpers for plugins (HTTPS client with pure-Rust TLS).
- Plugins, each a separate service behind the core:
  - `crates/adsb` – live/military/emergency aircraft from adsb.lol, with a recorded position history (redb).
  - `crates/imagery` – NASA GIBS satellite imagery timeline with A/B date blend and VIIRS fire/thermal detections.
  - `crates/lookup` – domain and IP lookup: DNS over HTTPS, RDAP registration, reverse DNS, certificate-transparency names.
  - `crates/imgmeta` – image forensics: hashes, EXIF, GPS and editing flags, reverse-image-search links.
  - `crates/cameras` – grid of live still-image camera feeds from `cameras.toml`, with optional map placement.
  - `crates/deploy` – Rust deploy tool: builds, copies over SSH, installs the systemd unit, restarts.
- `deploy/osnt-portal.service` – hardened systemd unit (DynamicUser, read-only filesystem).
- `.claude/skills/osnt-plugin` – Claude Code skill: how to write, wire in and verify a plugin.
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

## Agent API

The core aggregates every plugin's `GET /tools` into one catalogue:

```bash
curl http://osnt-pi.local:8080/api/agent/tools                      # all tools with JSON-schema parameters
curl -X POST http://osnt-pi.local:8080/api/agent/call -H 'content-type: application/json' \
  -d '{"tool":"lookup_overview","args":{"q":"example.com"}}'        # call one
```

An LLM harness (for example DeepSeek's OpenAI-compatible API) can use the first as its tool list and the second to execute calls.
