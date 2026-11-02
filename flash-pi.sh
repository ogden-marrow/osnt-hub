#!/usr/bin/env bash
# Flash Raspberry Pi OS Lite (arm64) to the SD card and seed a bare-bones cloud-init.
# Usage: sudo OSNT_IMG=/path/to/raspios-lite-arm64.img.xz ./flash-pi.sh /dev/mmcblk0
set -euo pipefail

DEV="${1:?usage: sudo $0 /dev/mmcblk0}"
IMG="${OSNT_IMG:?set OSNT_IMG to the downloaded raspios_lite_arm64 .img.xz (https://www.raspberrypi.com/software/operating-systems/)}"
PUBKEY_FILE="/home/${SUDO_USER:-jpalenchar}/.ssh/osnt_pi_ed25519.pub"
HOSTNAME_PI=osnt-pi
USER_PI=osnt

[[ $EUID -eq 0 ]] || { echo "run with sudo"; exit 1; }
[[ -b $DEV ]] || { echo "$DEV is not a block device"; exit 1; }
[[ $DEV == /dev/mmcblk* || $DEV == /dev/sd* ]] || { echo "refusing $DEV"; exit 1; }
[[ $(lsblk -dno RM,SIZE "$DEV" | awk '{print $1}') == 1 || $DEV == /dev/mmcblk* ]] || { echo "$DEV not removable"; exit 1; }
if lsblk -no MOUNTPOINTS "$DEV" | grep -qxE '/|/boot/efi'; then echo "$DEV holds the system disk, aborting"; exit 1; fi

lsblk -o NAME,SIZE,LABEL "$DEV"
read -rp "ERASE $DEV and flash Raspberry Pi OS Lite? Type 'yes': " ans
[[ $ans == yes ]] || exit 1

umount "${DEV}"* 2>/dev/null || true
xzcat "$IMG" | dd of="$DEV" bs=4M conv=fsync status=progress
sync
partprobe "$DEV"; sleep 2

MNT=$(mktemp -d)
BOOTPART="${DEV}p1"; [[ $DEV == /dev/sd* ]] && BOOTPART="${DEV}1"
mount "$BOOTPART" "$MNT"

cat > "$MNT/user-data" <<EOF
#cloud-config
hostname: ${HOSTNAME_PI}
manage_etc_hosts: true
timezone: UTC
disable_root: true
ssh_pwauth: false
users:
  - name: ${USER_PI}
    shell: /bin/bash
    groups: [sudo, adm]
    lock_passwd: true
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
      - $(cat "$PUBKEY_FILE")
ssh_deletekeys: true
EOF

cat > "$MNT/network-config" <<'EOF'
network:
  version: 2
  ethernets:
    eth0:
      dhcp4: true
      optional: true
EOF

sync
umount "$MNT"; rmdir "$MNT"
echo "Done. Put the card in the Pi 5, connect Ethernet, power on, then: ssh -i ~/.ssh/osnt_pi_ed25519 ${USER_PI}@${HOSTNAME_PI}.local"
