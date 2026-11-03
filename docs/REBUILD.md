# Rebuild the Pi from nothing (the acceptance test)

Proves the whole setup is reproducible: a blank SD card to a working hub with every plugin, using only this repository,
the plugin repository and what lives on your computer. Takes about 45 minutes, most of it waiting.

## What survives, what does not

Stays on your computer (nothing to back up): the CA and certificate (`~/.config/osnt/ca/`), the gateway key
(`~/.config/osnt/gateway.key`), the Pi SSH key (`~/.ssh/osnt_pi_ed25519`), both repositories.

Lost with the card, so note it first: your API keys (re-enter them on Settings > API keys: LLM base URL, model, key;
Google Vision key or service-account file; SerpAPI; Planetary Computer), the GitHub connection (reconnect), the package
repository list (re-add), passkeys (create again), plugin data (the aircraft position history, cached slices, MP3 files).

## Steps

1. **Image.** Download Raspberry Pi OS Lite, 64-bit (arm64), from https://www.raspberrypi.com/software/operating-systems/
   (about 500 MB, an `.img.xz`). Do not unpack it.
2. **Power off and move the card.** `ssh -i ~/.ssh/osnt_pi_ed25519 osnt@osnt-pi.local 'sudo poweroff'`, wait for the
   green light to stop, put the card in your computer.
3. **Flash.** Find the card with `lsblk` (it is the removable one, for example `/dev/mmcblk0`), then:
   `sudo OSNT_IMG=~/Downloads/<image>.img.xz ./flash-pi.sh /dev/mmcblk0` and type `yes`. The script refuses a system
   disk. It writes the image, your SSH key, the hostname `osnt-pi`, wired DHCP and the passkey reset file.
4. **Boot.** Card in the Pi 5, Ethernet in, power on, wait 3 minutes (first boot runs cloud-init).
5. **SSH.** `ssh-keygen -R osnt-pi.local` (the host key changed), then
   `ssh -i ~/.ssh/osnt_pi_ed25519 osnt@osnt-pi.local true`.
6. **Bootstrap.** From this repository: `cargo deploy bootstrap`. It deploys the recovery service, the portal (HTTPS with your
   existing CA, gateway key), the install helper, and the Packages and Credentials plugins from `../osnt-plugins`
   (override with `OSNT_PLUGINS_REPO`). About 5 minutes of building.
7. **Passkey.** Open https://osnt-pi.local/setup and create your passkey (your browser already trusts the CA).
8. **Packages.** Settings > Packages: Connect GitHub (type the code at github.com/login/device), add
   `https://github.com/ogden-marrow/osnt-plugins`, then install what you want. Approve the system software dialog for
   YouTube to MP3.
9. **Keys.** Settings > API keys: re-enter the keys you noted.
10. **Reset test.** Power off, put the card in your computer, open `OSNT-RESET-PASSKEYS.txt` on the boot partition, follow its
    instructions, put the card back, boot. https://osnt-pi.local/setup must be open again. Create a passkey again.

## Pass criteria

- `cargo deploy status` shows the recovery, portal and helper units healthy.
- The launcher lists every plugin you installed, and each plugin page opens.
- No step needed anything not in the two repositories or on your computer.
- Step 10 reopens setup and the old passkey no longer signs in.
- Installing, updating and uninstalling a plugin from the Packages page works.
