#!/usr/bin/env bash
#
# The Win7 bring-up loop.
#
# The boot dominates the cost of one experiment (see the bring-up loop in the
# wiki's debugging area),
# so the point of this script is that the slow part happens once: `snapshot`
# boots from cold and saves the machine just before the wall, and `resume`
# restarts from there in seconds. Everything else is instrumentation that used to
# have to be remembered on the command line, and therefore got forgotten.
#
# Usage:
#   scripts/win7.sh snapshot [budget_ms]   boot cold, save a snapshot
#   scripts/win7.sh resume   [budget_ms]   resume from the snapshot (the fast loop)
#   scripts/win7.sh cold     [budget_ms]   boot cold without saving
#   scripts/win7.sh ladder   [run_dir]     report the furthest milestone reached
#
# Every mode writes a run directory containing run.log (progress samples),
# serial.log, debugcon.log and fb.png.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGES_DIR="${AERO_IMAGES_DIR:-/root/aero-images}"
OUT_ROOT="${AERO_BOOT_OUT:-/root/aero-boot-shots}"
SNAPSHOT="${AERO_BOOT_SNAPSHOT:-$OUT_ROOT/snap/at-wall.bin}"
BIN="${AERO_MACHINE_BIN:-$REPO_ROOT/target/release/aero-machine}"

# The install ISO is whichever Win7 ISO is present; there is normally exactly one.
ISO="${AERO_WIN7_ISO:-$(ls "$IMAGES_DIR"/*win7*.iso "$IMAGES_DIR"/*.iso 2>/dev/null | grep -iv aero-config | head -1 || true)}"
DISK="${AERO_WIN7_DISK:-$IMAGES_DIR/win7-hdd.raw}"
RAM_MIB="${AERO_WIN7_RAM:-2048}"
PROGRESS_MS="${AERO_PROGRESS_MS:-15000}"

die() { echo "win7.sh: $*" >&2; exit 1; }

require_inputs() {
  [[ -x "$BIN" ]] || die "runner not built: $BIN (cargo build --release -p aero-machine-cli)"
  [[ -n "$ISO" && -f "$ISO" ]] || die "no Win7 ISO found under $IMAGES_DIR (set AERO_WIN7_ISO)"
  [[ -f "$DISK" ]] || die "disk image not found: $DISK (set AERO_WIN7_DISK)"
}

# All instrumentation on, always. The reason this script exists is that these
# flags are the difference between a run that explains itself and a run that
# produces one PNG, and they are too easy to leave off by hand.
run_machine() {
  local run_dir="$1"; shift
  mkdir -p "$run_dir"
  "$BIN" \
    --install-iso "$ISO" \
    --disk "$DISK" \
    --ram "$RAM_MIB" \
    --progress-interval "$PROGRESS_MS" \
    --vga-png "$run_dir/fb.png" \
    --vga-png-interval "$PROGRESS_MS" \
    --serial-out "$run_dir/serial.log" \
    --debugcon-out "$run_dir/debugcon.log" \
    "$@" 2>&1 | tee "$run_dir/run.log"
}

# Milestones are read from the progress samples rather than from the screen, so
# the ladder is mechanical and does not depend on reading pixels. Each is a
# signal the guest can only produce by having got that far.
ladder() {
  local run_dir="${1:-$OUT_ROOT/latest}"
  local log="$run_dir/run.log"
  [[ -f "$log" ]] || die "no run log at $log"

  local reached=()
  grep -q 'inst=[1-9]' "$log" && reached+=("1 POST (guest instructions retired)")
  grep -q 'mode=Protected' "$log" && reached+=("2 protected mode")
  grep -q 'display=1024x768\|display=[89][0-9][0-9]x' "$log" && reached+=("3 VBE graphics mode programmed")
  grep -q 'mode=Long' "$log" && reached+=("4 long mode (winload transition)")
  grep -qE 'non_zero=[1-9][0-9]{4,}' "$log" && reached+=("5 substantial framebuffer content drawn")
  # A high-half rip (0xfffff8xxxxxxxxxx) can only be the kernel: winload runs from
  # low addresses, so this is the OslArchTransferToKernel handoff having happened.
  grep -qE 'rip=0xfffff[0-9a-f]{11}' "$log" && reached+=("6 kernel entered (high-half rip — past OslArchTransferToKernel)")
  # cr8 tracks IRQL, which only the kernel raises.
  grep -qE 'cr8=0x[1-9a-f]' "$log" && reached+=("7 kernel raised IRQL (cr8 non-zero)")
  [[ -s "$run_dir/serial.log" ]] && reached+=("8 guest serial output (kernel debug is talking)")

  local insts rate
  insts="$(grep -oE 'inst=[0-9]+' "$log" | tail -1 | cut -d= -f2 || echo 0)"
  rate="$(grep -oE '\([0-9.]+ Minst/s\)' "$log" | tail -1 || echo '(n/a)')"

  echo "=== Win7 milestone ladder: $run_dir ==="
  if ((${#reached[@]} == 0)); then
    echo "  (no milestones — did the run start?)"
  else
    printf '  reached: %s\n' "${reached[@]}"
  fi
  echo "  depth:   ${insts:-0} instructions ${rate}"

  # Forward progress is the question a stalled boot actually poses: a frozen
  # framebuffer says nothing about whether the CPU is still retiring work.
  local first_i last_i
  first_i="$(grep -oE 'inst=[0-9]+' "$log" | head -1 | cut -d= -f2 || echo 0)"
  last_i="$insts"
  if [[ -n "$first_i" && -n "$last_i" && "$last_i" != "$first_i" ]]; then
    echo "  status:  advancing (inst grew $first_i -> $last_i across the run)"
  else
    echo "  status:  NOT advancing (instruction count flat — genuinely stuck, not slow)"
  fi

  # A boot that advances without the screen changing is looping. Where it loops
  # is the next question, and the progress samples already carry rip.
  echo
  echo "  rip distribution over the last 40 samples (a narrow spread means a loop):"
  grep -oE 'rip=0x[0-9a-f]+' "$log" | tail -40 | sort | uniq -c | sort -rn | head -8 |
    while read -r count rip; do printf '    %5sx  %s\n' "$count" "$rip"; done

  # Mode transitions are the cheapest possible milestone signal: each one is a
  # boot stage the guest could only reach by having completed the previous.
  echo
  echo "  mode transitions observed:"
  grep -oE 'mode=[A-Za-z]+' "$log" | uniq | sed 's/^/    /' | uniq | head -12
}

mode="${1:-}"; shift || true
budget_ms="${1:-1500000}"

case "$mode" in
  snapshot)
    require_inputs
    run_dir="$OUT_ROOT/snap"
    mkdir -p "$(dirname "$SNAPSHOT")"
    echo "win7.sh: cold boot, saving snapshot to $SNAPSHOT (budget ${budget_ms}ms)"
    run_machine "$run_dir" --max-ms "$budget_ms" --snapshot-save "$SNAPSHOT"
    ln -sfn "$run_dir" "$OUT_ROOT/latest"
    ladder "$run_dir"
    ;;
  resume)
    require_inputs
    [[ -f "$SNAPSHOT" ]] || die "no snapshot at $SNAPSHOT — run: scripts/win7.sh snapshot"
    run_dir="$OUT_ROOT/resume-$(date +%H%M%S)"
    echo "win7.sh: resuming from $SNAPSHOT (budget ${budget_ms}ms)"
    run_machine "$run_dir" --max-ms "$budget_ms" --snapshot-load "$SNAPSHOT"
    ln -sfn "$run_dir" "$OUT_ROOT/latest"
    ladder "$run_dir"
    ;;
  cold)
    require_inputs
    run_dir="$OUT_ROOT/cold-$(date +%H%M%S)"
    run_machine "$run_dir" --max-ms "$budget_ms"
    ln -sfn "$run_dir" "$OUT_ROOT/latest"
    ladder "$run_dir"
    ;;
  ladder)
    ladder "${1:-$OUT_ROOT/latest}"
    ;;
  *)
    sed -n '3,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 1
    ;;
esac
