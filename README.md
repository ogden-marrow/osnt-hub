# OSNT hub

The dashboard for a modular open source intelligence system, written in Rust and run straight on Raspberry Pi OS
(no containers). The hub is deliberately small: a launcher, a plugin registry and proxy, an installer and an agent API.
Everything else (maps, aircraft, lookups, image forensics, the credential store...) is a plugin, kept in a separate
repository: [osnt-plugins](https://github.com/ogden-marrow/osnt-plugins).

## Layout

- `crates/portal`: the core web server (launcher, plugin proxy, shared map kit, agent tool API). Assets are embedded in the binary.
- `crates/deploy`: the `osnt-deploy` tool: deploys the core to the Pi and installs/removes plugins (`cargo deploy ...`).
- `crates/recover`: `osnt-recover`, the boot-time root service that reads the SD-card reset file.
- `deploy/osnt-portal.service`, `deploy/osnt-recover.service`: hardened systemd units; `deploy/OSNT-RESET-PASSKEYS.txt`: the reset file template.
- `docs/PLUGINS.md`: what the hub guarantees plugins.
- `flash-pi.sh`: one-off SD card flasher for the Pi 5 (Raspberry Pi OS Lite, key-only SSH).

## Use

```bash
rustup target add aarch64-unknown-linux-musl   # once
cargo run -p osnt-portal                       # local dev on :8080
cargo deploy                                   # build and deploy the core to the Pi
cargo deploy token                             # print the Credentials plugin admin token
cargo deploy auth code|template                # one-time setup code / SD-card reset file
cargo deploy https init|renew|ca               # your own CA and the Pi's HTTPS certificate (see below)

cargo deploy repo add https://github.com/ogden-marrow/osnt-plugins
cargo deploy plugin search
cargo deploy plugin add map                    # by id from a registered repository
cargo deploy plugin add /path/to/osnt-plugins/plugins/map   # or from a local folder or a GitHub folder link
cargo deploy plugin list
cargo deploy plugin remove map [--purge]
```

## Sign-in: passkeys only

There are no passwords. A new hub has no passkey, so `cargo deploy` opens setup and prints a one-time code
(`cargo deploy auth code` prints a fresh one any time; it is valid for 30 minutes). Open `https://osnt-pi.local/setup`,
enter the code and create a passkey; everything else requires a signed-in session (14 days, HttpOnly cookie).

Lost your passkey: take the SD card out, open `OSNT-RESET-PASSKEYS.txt` on the boot partition (a plain FAT file any
computer can read), write a one-time code of 12+ characters on the line below the comments, put the card back and power
on. The tiny root service `osnt-recover` turns that into a reset request (storing only a hash of the code and blanking the
file), the portal removes all passkeys and sessions, and setup opens for 30 minutes. Physical access to the card is the
only way to trigger it.

## HTTPS (your own CA)

`cargo deploy` creates a private CA on first use (`~/.config/osnt/ca/`), issues a certificate for `osnt-pi.local`,
pushes it to the Pi and serves the hub on `https://osnt-pi.local/` (TLS 1.3, HTTP/2; port 80 redirects; the plain
listener is loopback-only). Install `~/.config/osnt/ca/osnt-ca.crt` once on each device (`cargo deploy https init`
prints per-platform steps). The CA is name-constrained to `.local`, its key never leaves your machine, and the Pi's
certificate is valid for 800 days (`cargo deploy https renew` to re-issue). Use `OSNT_HOST` for a different name.

Deploy target defaults to `osnt@osnt-pi.local` with key `~/.ssh/osnt_pi_ed25519`; override with `OSNT_DEPLOY_HOST` and
`OSNT_SSH_KEY`. Core config: `OSNT_BIND` (default `0.0.0.0:8080`, the unit sets loopback), `OSNT_TLS_BIND`, `OSNT_HTTP_BIND`, `OSNT_PUBLIC_HOST`, `OSNT_LAYERS`, `OSNT_TOOLS`, `OSNT_PLUGINS_DIR`.

## Agent API

```bash
curl --cacert ~/.config/osnt/ca/osnt-ca.crt https://osnt-pi.local/api/agent/tools
curl --cacert ~/.config/osnt/ca/osnt-ca.crt -X POST https://osnt-pi.local/api/agent/call -H 'content-type: application/json' \
  -d '{"tool":"lookup_overview","args":{"q":"example.com"}}'
```
