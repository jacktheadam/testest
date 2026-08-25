#!/usr/bin/env bash
#
# First-divergence between Aero and QEMU on the same guest.
#
# The wiki's debugging topic calls this "the strategic tool" and explains why:
# backward-chaining a crash costs effort proportional to the chain length, and
# one root cause needed six hops. The first place two emulators disagree while
# running the same guest is the bug, found in one query rather than six.
#
# The reference side was previously blocked on building a QEMU TCG plugin to get
# a memory-write stream. That is the better long-term signal, but it is not
# needed to start: `-d exec` gives the executed-block stream from the stock
# binary, which is enough to find control-flow divergence, and control-flow
# divergence is what a wrong instruction produces.
#
# Usage: scripts/win7-diverge.sh [max_insts] [mode]
#   max_insts  instruction budget for the Aero side (default 20,000,000)
#   mode       raw | first-visit | page   (default first-visit)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGES_DIR="${AERO_IMAGES_DIR:-/root/aero-images}"
OUT_DIR="${AERO_DIVERGE_OUT:-/root/aero-boot-shots/diverge}"
BIN="${AERO_MACHINE_BIN:-$REPO_ROOT/target/release/aero-machine}"
QEMU="${QEMU_BIN:-qemu-system-x86_64}"

ISO="${AERO_WIN7_ISO:-$(ls "$IMAGES_DIR"/*win7*.iso "$IMAGES_DIR"/*.iso 2>/dev/null | grep -iv aero-config | head -1 || true)}"
DISK="${AERO_WIN7_DISK:-$IMAGES_DIR/win7-hdd.raw}"
RAM_MIB="${AERO_WIN7_RAM:-2048}"

MAX_INSTS="${1:-20000000}"
MODE="${2:-first-visit}"

die() { echo "win7-diverge.sh: $*" >&2; exit 1; }

[[ -x "$BIN" ]] || die "runner not built: $BIN"
command -v "$QEMU" >/dev/null || die "$QEMU not found"
[[ -n "$ISO" && -f "$ISO" ]] || die "no Win7 ISO under $IMAGES_DIR"
[[ -f "$DISK" ]] || die "no disk image: $DISK"

mkdir -p "$OUT_DIR"
AERO_TRACE="$OUT_DIR/aero-exec.txt"
QEMU_TRACE="$OUT_DIR/qemu-exec.txt"

echo "==> Aero: tracing $MAX_INSTS instructions"
AERO_TRACE_COMPACT=1 AERO_TRACE_FROM=0 AERO_TRACE_TO="$MAX_INSTS" \
  "$BIN" --install-iso "$ISO" --disk "$DISK" --ram "$RAM_MIB" \
         --max-insts "$MAX_INSTS" \
  2> "$AERO_TRACE" >/dev/null || true

echo "==> QEMU: tracing the same boot (TCG; KVM cannot emit an exec trace)"
# QEMU has no instruction budget, so bound it by wall clock and rely on
# tracediff comparing the common prefix. It is given the same media, the same
# RAM, and no acceleration so that every block is traced.
timeout --foreground "${AERO_DIVERGE_QEMU_SECS:-120}" \
  "$QEMU" -accel tcg -m "$RAM_MIB" \
    -cdrom "$ISO" -drive file="$DISK",format=raw,if=ide \
    -boot d -display none -serial null -no-reboot \
    -d exec -D "$QEMU_TRACE" >/dev/null 2>&1 || true

for f in "$AERO_TRACE" "$QEMU_TRACE"; do
  [[ -s "$f" ]] || die "empty trace: $f"
done

echo "==> comparing (mode=$MODE)"
exec python3 "$REPO_ROOT/tools/tracediff/tracediff.py" \
  "$AERO_TRACE" "$QEMU_TRACE" --mode "$MODE"
