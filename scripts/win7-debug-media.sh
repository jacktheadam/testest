#!/usr/bin/env bash
#
# Build a Win7 install ISO with kernel debugging enabled over COM1.
#
# A stuck boot that cannot describe itself is the expensive kind. Windows will
# describe itself — which driver it is initialising, and on a bugcheck the stop
# code and its parameters — if the BCD asks it to, and Aero already captures
# COM1 via `--serial-out`. This script produces media that asks.
#
# It works by splicing the patched BCD hives back into a *copy* of the ISO at
# their original byte offsets, leaving every other byte of the image untouched.
#
# The obvious alternative — extract, patch, rebuild with xorriso — was tried and
# does not work: Win7's El Torito loader depends on the exact ISO9660 layout of
# the original media, and a rebuilt image boots only as far as
#
#     CDBOOT: Couldn't find BOOTMGR
#
# Splicing avoids the question entirely. The BCD hives are found by scanning for
# the REGF signature on sector boundaries rather than by walking the filesystem,
# which sidesteps ISO9660 name-mapping (the stores are `/BOOT/BCD;1`-style names
# that neither xorriso's `-find` nor a naive path lookup matches reliably).
#
# The patched hive is smaller than the original — `bcd_patch` rewrites compactly
# — so it is zero-padded back to the original extent. A REGF hive declares its
# own length in its header and ignores trailing bytes, and the padded result is
# verified to re-read before it is used.
#
# Usage: scripts/win7-debug-media.sh [output.iso]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGES_DIR="${AERO_IMAGES_DIR:-/root/aero-images}"

SRC_ISO="${AERO_WIN7_SOURCE_ISO:-$(ls "$IMAGES_DIR"/*win7*.iso "$IMAGES_DIR"/*.iso 2>/dev/null | grep -iv 'aero-config\|win7-debug' | head -1 || true)}"
OUT_ISO="${1:-$IMAGES_DIR/win7-debug.iso}"
WORK="${AERO_DEBUG_MEDIA_WORK:-$IMAGES_DIR/.debug-media-work}"

die() { echo "win7-debug-media.sh: $*" >&2; exit 1; }

[[ -n "$SRC_ISO" && -f "$SRC_ISO" ]] || die "no source ISO found under $IMAGES_DIR"

BCD_PATCH="$REPO_ROOT/target/debug/bcd_patch"
if [[ ! -x "$BCD_PATCH" ]]; then
  echo "==> building bcd_patch"
  (cd "$REPO_ROOT" && cargo build -p bcd-patch)
fi
[[ -x "$BCD_PATCH" ]] || die "bcd_patch not built"

echo "==> source: $SRC_ISO"
echo "==> output: $OUT_ISO"

rm -rf "$WORK"; mkdir -p "$WORK"

echo "==> locating BCD hives (REGF scan on sector boundaries)"
mapfile -t OFFSETS < <(python3 - "$SRC_ISO" <<'PY'
import sys, pathlib
iso = pathlib.Path(sys.argv[1])
SECTOR, CHUNK = 2048, 64 << 20
off = 0
with iso.open("rb") as fh:
    while (buf := fh.read(CHUNK)):
        for i in range(0, len(buf) - 4, SECTOR):
            if buf[i:i + 4] == b"regf":
                print(off + i)
        off += len(buf)
PY
)
(( ${#OFFSETS[@]} > 0 )) || die "no REGF hives found in $SRC_ISO"

echo "==> copying ISO (the original is never modified)"
cp --reflink=auto "$SRC_ISO" "$OUT_ISO"

EXTENT=262144   # every BCD store on Win7 media occupies this many bytes
# Offsets are ISO9660 sector boundaries, which are 2048 bytes. Using a larger dd
# block size silently truncates the offset for any store that is not also
# aligned to it — one of the two stores on this media is not.
BS=2048
patched=0
for off in "${OFFSETS[@]}"; do
  hive="$WORK/hive-$off.bin"
  dd if="$SRC_ISO" of="$hive" bs="$BS" skip=$((off / BS)) count=$((EXTENT / BS)) status=none

  # Not every REGF hive on the media is a BCD store; the WIMs carry others.
  if ! "$BCD_PATCH" --store "$hive" --kernel-debug on >/dev/null 2>&1; then
    continue
  fi

  size=$(stat -c%s "$hive")
  (( size <= EXTENT )) || die "patched hive at $off grew to $size (> $EXTENT); cannot splice"
  truncate -s "$EXTENT" "$hive"

  # A padded hive that no longer parses would boot into an unexplained failure,
  # which is exactly what this script exists to avoid. Check before splicing.
  "$BCD_PATCH" --store "$hive" --kernel-debug on >/dev/null 2>&1 ||
    die "padded hive at offset $off no longer parses; refusing to splice"

  dd if="$hive" of="$OUT_ISO" bs="$BS" seek=$((off / BS)) count=$((EXTENT / BS)) conv=notrunc status=none
  echo "    spliced BCD at offset $off"
  patched=$((patched + 1))
done

(( patched > 0 )) || die "no BCD store could be patched"
rm -rf "$WORK"

echo "==> patched $patched BCD store(s): kernel debugging on COM1 @ 115200"
echo "==> done: $OUT_ISO ($(du -h "$OUT_ISO" | cut -f1))"
echo
echo "Boot it with:"
echo "  AERO_WIN7_ISO=$OUT_ISO just boot-cold"
echo "and read the guest's own account of the boot from the run's serial.log."
