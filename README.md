# OSNT

Open source intelligence portal, written in Rust, deployed straight onto Raspberry Pi OS (no containers).

## Layout

- `crates/portal` – axum web server. Static assets (including vendored Leaflet) are embedded in the binary.
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
before any real traffic, then self-host tiles).
