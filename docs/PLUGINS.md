# Plugin contract (host side)

Plugins live in a separate repository (see the `osnt-plugins` repository); this document is what the hub guarantees them.

- **Discovery.** The installer writes `/etc/osnt/plugins.d/<id>.toml` (id, name, description, grid, port, `[[secret]]`,
  `token_sha256`). The portal re-reads that directory on every request, so installs show up without a restart.
- **Mounting.** `GET /p/<id>/...` is reverse-proxied to `127.0.0.1:<port>/...`. Paths starting with `internal/` are never proxied.
- **Launcher.** Each plugin becomes a card in its `grid`, shown as `live` when its port accepts connections and `offline` otherwise.
- **Agent API.** `GET /api/agent/tools` merges each plugin's `GET /tools`; `POST /api/agent/call {tool, args}` runs a GET tool.
- **Signed-in user.** The portal is the only place users sign in (passkeys). It adds `X-OSNT-Gateway: v1.<time>.<hmac>` to every
  proxied request, signed with a key derived for that plugin only (`OSNT_GATEWAY_KEY`, from a master key that lives on
  your computer and the portal). Plugins that need to know a user is signed in verify it (`osnt_common::gateway_authorized`)
  instead of running their own login; they cannot forge requests to other plugins.
- **Secrets.** The Credentials plugin stores keys. A plugin reads one with its own token (`OSNT_PLUGIN_TOKEN`, loaded from a
  root-only file by its systemd unit) and only for names declared in its manifest.
- **Shared UI kit.** `/static/osnt-ui.css` (design system; add class `embedded` to `<html>` inside iframes), `/static/leaflet/*`
  and `/static/osnt-map.js` (layer catalog from `crates/portal/layers.toml`).
- **Deep links.** Pages read `?q=` (or `?lat&lon&zoom`, `?id=`) to pre-fill and run. `?pin=` on the case-notebook plugin opens a
  confirmation to save the item to the case.
- **Sandbox.** The installer generates each plugin's systemd unit: dynamic user, read-only filesystem, private /tmp,
  optional `StateDirectory=osnt-<id>`, loopback binding by convention. Plugins cannot set their own command line or privileges.
- **System software.** A plugin may declare `[system]` with `apt = [...]` and `[[system.download]]` (`name`, `url`, optional
  `sha256_url`; GitHub release URLs only). The Packages page shows exactly what will be installed and only installs what
  the user approves; the root helper skips anything already present.
- **Dependencies.** `requires` (plugin ids that must be installed first) and `provides` (API names) in `plugin.toml`.

Installing and removing: Settings, Packages in the web UI (install, update, remove), or `cargo deploy plugin add|remove|list|search`, `cargo deploy repo add|list|remove`.
