#!/usr/bin/env bash
#
# Install Windows 7 unattended, under QEMU/KVM, onto the raw disk Aero boots.
#
# Aero runs the interpreter at roughly 6M instructions/second. A Win7 install is
# on the order of a trillion instructions, so running setup inside Aero would
# take days. Installing under KVM takes minutes, and the installed image is what
# actually matters: booting it in Aero is the milestone, not watching setup.
#
# The storage topology here is not a free choice. Aero attaches `--disk` to
# **AHCI port 0**, so the install must present AHCI too — Windows binds its boot
# storage driver at install time, and an IDE install booted against AHCI
# bugchecks with 0x7B INACCESSIBLE_BOOT_DEVICE. The CD-ROMs stay on IDE/ATAPI,
# which is what Aero presents for removable media.
#
# `autounattend.xml` comes from the config ISO, which setup finds by scanning
# removable media. It wipes disk 0, accepts the EULA, selects Professional and
# enables auto-logon, so no input is ever required.
#
# Usage: scripts/win7-install.sh [minutes]

set -euo pipefail

IMAGES_DIR="${AERO_IMAGES_DIR:-$HOME/aero-images}"
DISK="${AERO_WIN7_DISK:-$IMAGES_DIR/win7-hdd.raw}"
CONFIG_ISO="${AERO_WIN7_CONFIG_ISO:-$IMAGES_DIR/aero-config.iso}"
QMP_SOCK="${AERO_INSTALL_QMP:-/tmp/qmp-win7-install.sock}"
RAM_MIB="${AERO_INSTALL_RAM:-4096}"
SMP="${AERO_INSTALL_SMP:-4}"

ISO="${AERO_WIN7_ISO:-$(ls "$IMAGES_DIR"/*win7*.iso "$IMAGES_DIR"/*.iso 2>/dev/null | grep -iv 'aero-config\|win7-debug' | head -1 || true)}"

die() { echo "win7-install.sh: $*" >&2; exit 1; }

[[ -n "$ISO" && -f "$ISO" ]] || die "no Win7 ISO under $IMAGES_DIR"
[[ -f "$DISK" ]] || die "no disk image: $DISK"
[[ -f "$CONFIG_ISO" ]] || die "no config ISO (autounattend): $CONFIG_ISO"
[[ -e /dev/kvm ]] || die "/dev/kvm not available; this would take days without it"

rm -f "$QMP_SOCK"

echo "==> installing Windows 7"
echo "    disk:   $DISK (AHCI port 0, matching Aero)"
echo "    media:  $ISO"
echo "    config: $CONFIG_ISO"
echo "    qmp:    $QMP_SOCK"

# `-boot order=dc` boots the CD first. Setup reboots several times; on each one
# the CD prompts for a keypress, gets none, and falls through to the disk, which
# is exactly the behaviour an unattended install needs.
exec qemu-system-x86_64 \
  -accel kvm -m "$RAM_MIB" -smp "$SMP" \
  -device ich9-ahci,id=ahci \
  -drive file="$DISK",format=raw,if=none,id=hd0 \
  -device ide-hd,drive=hd0,bus=ahci.0 \
  -drive file="$ISO",media=cdrom,if=ide,index=0 \
  -drive file="$CONFIG_ISO",media=cdrom,if=ide,index=1 \
  -boot order=dc -display none -vga std \
  -qmp "unix:$QMP_SOCK,server,nowait"
