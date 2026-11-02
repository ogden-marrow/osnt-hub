# OSNT hub

The dashboard for a modular open source intelligence system, written in Rust and run straight on Raspberry Pi OS
(no containers). The hub is deliberately small: a launcher, a plugin registry and proxy, an installer and an agent API.
Everything else (maps, aircraft, lookups, image forensics, the credential store...) is a plugin, kept in a separate
repository: [osnt-plugins](https://github.com/ogden-marrow/osnt-plugins).

## Layout

- `crates/portal`: the core web server (launcher, plugin proxy, shared map kit, agent tool API). Assets are embedded in the binary.
- `crates/deploy`: the `osnt-deploy` tool: deploys the core to the Pi and installs/removes plugins (`cargo deploy ...`).
- `deploy/osnt-portal.service`: hardened systemd unit for the core.
- `docs/PLUGINS.md`: what the hub guarantees plugins.
- `flash-pi.sh`: one-off SD card flasher for the Pi 5 (Raspberry Pi OS Lite, key-only SSH).

## Use

```bash
rustup target add aarch64-unknown-linux-musl   # once
cargo run -p osnt-portal                       # local dev on :8080
cargo deploy                                   # build and deploy the core to the Pi
cargo deploy token                             # print the Credentials plugin admin token

cargo deploy repo add https://github.com/ogden-marrow/osnt-plugins
cargo deploy plugin search
cargo deploy plugin add map                    # by id from a registered repository
cargo deploy plugin add /path/to/osnt-plugins/plugins/map   # or from a local folder or a GitHub folder link
cargo deploy plugin list
cargo deploy plugin remove map [--purge]
```

Deploy target defaults to `osnt@osnt-pi.local` with key `~/.ssh/osnt_pi_ed25519`; override with `OSNT_DEPLOY_HOST` and
`OSNT_SSH_KEY`. Core config: `OSNT_BIND` (default `0.0.0.0:8080`), `OSNT_LAYERS`, `OSNT_TOOLS`, `OSNT_PLUGINS_DIR`.

## Agent API

```bash
curl http://osnt-pi.local:8080/api/agent/tools
curl -X POST http://osnt-pi.local:8080/api/agent/call -H 'content-type: application/json' \
  -d '{"tool":"lookup_overview","args":{"q":"example.com"}}'
```
