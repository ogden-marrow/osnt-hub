# Plugin contract (host side)

Plugins live in a separate repository (see the `osnt-plugins` repository); this document is what the hub guarantees them.

- **Discovery.** The installer writes `/etc/osnt/plugins.d/<id>.toml` (id, name, description, grid, port, `[[secret]]`,
  `token_sha256`). The portal re-reads that directory on every request, so installs show up without a restart.
- **Mounting.** `GET /p/<id>/...` is reverse-proxied to `127.0.0.1:<port>/...`. Paths starting with `internal/` are never proxied.
- **Launcher.** Each plugin becomes a card in its `grid`, shown as `live` when its port accepts connections and `offline` otherwise.
- **Agent API.** `GET /api/agent/tools` merges each plugin's `GET /tools`; `POST /api/agent/call {tool, args}` runs a GET tool.
- **Secrets.** The Credentials plugin stores keys. A plugin reads one with its own token (`OSNT_PLUGIN_TOKEN`, loaded from a
  root-only file by its systemd unit) and only for names declared in its manifest.
- **Shared UI kit.** `/static/leaflet/*` and `/static/osnt-map.js` (layer catalog from `crates/portal/layers.toml`).
- **Sandbox.** The installer generates each plugin's systemd unit: dynamic user, read-only filesystem, private /tmp,
  optional `StateDirectory=osnt-<id>`, loopback binding by convention. Plugins cannot set their own command line or privileges.
- **Dependencies.** `requires` (plugin ids that must be installed first) and `provides` (API names) in `plugin.toml`.

Installing and removing: `cargo deploy plugin add|remove|list|search`, `cargo deploy repo add|list|remove`.
