---
name: osnt-plugin
description: Write, wire in and deploy a plugin for the OSNT hub (Rust services behind the portal core, deployed to a Raspberry Pi). Use when asked to add, build or change an OSNT plugin, a launcher card, an OSINT data source, or an agent-callable tool in this repo.
---

# Writing an OSNT plugin

A plugin is a separate pure-Rust service on loopback, described by a manifest. The portal core
(`crates/portal`) reads manifests, reverse-proxies `/p/<id>/` to the plugin and adds its card to the launcher.
Full contract: `docs/PLUGINS.md`. Prompt template for users: `docs/PLUGIN_PROMPT.md`.

## Start from the right reference
- `crates/example`: minimal skeleton (page, one API route, `/tools`). Copy this first.
- `crates/adsb`: poller, caching, redb storage, map page, alerts.
- `crates/lookup`, `crates/imgmeta`, `crates/cameras`, `crates/imagery`: smaller real plugins (HTTP JSON lookups, file upload analysis, config-driven feeds, frontend-only).
- `crates/common`: shared `http_agent()` for HTTPS calls.

## Rules
1. Listen on `127.0.0.1:<port>` only. Pick an unused port (8101 adsb, 8102 imagery, 8103 lookup, 8104 imgmeta, 8105 cameras, 8190 example); take the next free one.
2. Pure Rust only. The Pi build uses `rust-lld` and there is no C toolchain: no `ring`, `aws-lc`, bundled SQLite, `openssl`. HTTPS goes through `osnt_common::http_agent()`; storage uses `redb`.
3. Pages use relative URLs (`fetch('api/x')`) because the core mounts them at `/p/<id>/`. Shared assets are absolute: `/static/leaflet/leaflet.js`, `/static/osnt-map.js` (`await OSNT.createMap('map')` returns `{ map, cfg }`).
4. Serve `GET /tools`: JSON array of `{ name, description, method, path, parameters }` (JSON Schema parameters; `{param}` in `path` is substituted from args). The core aggregates these for agents at `/api/agent/tools`.
5. Never let callers supply arbitrary upstream URLs or hosts (SSRF): validate inputs strictly, and fetch only from a fixed host or a config file.
6. Cache upstream calls and set a User-Agent; these are free community services.
7. Secrets come from environment variables set on the Pi, never from the repo.
8. Do not build features that identify people from images or video (face recognition, matching individuals). Counting, detection of non-identifying events, metadata and provenance checks are fine.

## Wiring checklist
1. Add the crate to `members` in the root `Cargo.toml`.
2. `deploy/plugins.d/<id>.toml`: `id`, `name`, `description`, `grid`, `port`. If `name` equals a "planned" card in `crates/portal/tools.toml`, the plugin replaces it; delete that planned card when you add the real one.
3. `deploy/osnt-<id>.service`: copy `deploy/osnt-adsb.service`. Add `StateDirectory=osnt-<id>` only if it writes files.
4. Add a `plugin!("<id>", "osnt-<id>")` line to `COMPONENTS` in `crates/deploy/src/main.rs` (binary name is the crate name).
5. Update the README layout list.

## Verify (do not skip)
1. `cargo build --release -p osnt-<id> --target aarch64-unknown-linux-musl` must pass with no C compiler.
2. Run it locally and call every endpoint against real data; show the output.
3. `cargo deploy <id>`; confirm the launcher card says "open", the page works at `/p/<id>/`, and `/api/agent/tools` lists the plugin's tools.
4. Look at the page in a browser (screenshot) before saying it works.
5. Commit with a message that says what the plugin does.
