#!/usr/bin/env bash
# Marginal host cost per guest instruction, measured with retired-instruction counters.
#
# Wall-clock on a shared box is too noisy to compare interpreter changes: the same
# fixed slice varies by roughly +/-15% between runs depending on what else is
# running. Retired host instructions do not: they are stable to three significant
# figures. Each binary is run at two guest-instruction budgets and the difference
# is taken, which cancels the fixed snapshot-load cost and leaves only the
# steady-state loop.
#
# Cycles are reported alongside because they capture stalls and branch
# mispredictions that instruction counts miss — but they inherit some of wall
# clock's noise, so treat a cycle win that contradicts the instruction count as
# unproven.
#
# Every run gets a **fresh** copy-on-write overlay. That matters: the overlay is
# persistent, `--snapshot-load` restores RAM but not the disk, and a reused
# overlay would let one run's guest writes leak into the next — which would
# invalidate both the differencing and any comparison between binaries.
#
# Usage:
#   AERO_BENCH_SNAPSHOT=/path/to/snap.bin ./bench-insns.sh ./target/release/aero-machine [more-binaries...]
#
# Environment:
#   AERO_BENCH_SNAPSHOT  required; a machine snapshot to resume from
#   AERO_BENCH_DISK      guest disk image      (default: /root/aero-images/win7-hdd.raw)
#   AERO_BENCH_LO/HI     guest instruction budgets (default: 20M / 120M)
#   AERO_BENCH_CORES     taskset CPU list, to keep runs off contended cores
#
# See the wiki: performance, "Measure retired host instructions, not wall clock".
set -euo pipefail

SNAP="${AERO_BENCH_SNAPSHOT:-}"
DISK="${AERO_BENCH_DISK:-/root/aero-images/win7-hdd.raw}"
LO="${AERO_BENCH_LO:-20000000}"
HI="${AERO_BENCH_HI:-120000000}"
CORES="${AERO_BENCH_CORES:-}"

die() { echo "bench-insns: $*" >&2; exit 1; }

[[ -n "$SNAP" ]] || die "set AERO_BENCH_SNAPSHOT to a snapshot to resume from"
[[ -f "$SNAP" ]] || die "snapshot not found: $SNAP"
[[ -f "$DISK" ]] || die "disk image not found: $DISK (set AERO_BENCH_DISK)"
[[ $# -ge 1 ]]   || die "usage: $0 <aero-machine binary> [more binaries...]"
command -v perf >/dev/null || die "perf not found; it is what makes this measurement stable"
[[ "$HI" -gt "$LO" ]] || die "AERO_BENCH_HI ($HI) must exceed AERO_BENCH_LO ($LO)"

WORK="$(mktemp -d -t aero-bench-XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

# Run one (binary, budget) pair. Emits "<instructions> <cycles>" on stdout.
# The overlay is removed first so the CLI creates a fresh one; perf's counters go
# to their own file so the guest's stderr cannot be mistaken for a counter line.
run() {
  local bin="$1" budget="$2"
  local overlay="$WORK/overlay.aerospar"
  local counters="$WORK/perf.csv"
  local log="$WORK/run.log"
  local -a pin=()
  [[ -n "$CORES" ]] && pin=(taskset -c "$CORES")

  rm -f "$overlay"
  if ! timeout -k 30 900 perf stat -e instructions,cycles -x, -o "$counters" \
      "${pin[@]}" "$bin" \
      --disk "$DISK" --disk-overlay "$overlay" \
      --ram 2048 --boot hdd --snapshot-load "$SNAP" --max-insts "$budget" \
      >"$log" 2>&1; then
    echo "bench-insns: run failed ($bin at $budget insts):" >&2
    tail -5 "$log" >&2
    return 1
  fi

  local insns cycles
  insns="$(awk -F, '$3=="instructions"{print $1}' "$counters" | head -1)"
  cycles="$(awk -F, '$3=="cycles"{print $1}' "$counters" | head -1)"
  [[ "$insns" =~ ^[0-9]+$ ]] || { echo "bench-insns: no instruction count (perf_event_paranoid?)" >&2; return 1; }
  [[ "$cycles" =~ ^[0-9]+$ ]] || { echo "bench-insns: no cycle count" >&2; return 1; }
  echo "$insns $cycles"
}

for bin in "$@"; do
  [[ -x "$bin" ]] || die "not executable: $bin"
  read -r lo_i lo_c < <(run "$bin" "$LO") || die "LO run failed for $bin"
  read -r hi_i hi_c < <(run "$bin" "$HI") || die "HI run failed for $bin"
  python3 - "$(basename "$bin")" "$lo_i" "$hi_i" "$lo_c" "$hi_c" "$((HI - LO))" <<'PY'
import sys
name, lo_i, hi_i, lo_c, hi_c, guest = sys.argv[1], *map(int, sys.argv[2:])
di, dc = hi_i - lo_i, hi_c - lo_c
print(f"{name}: {di/guest:6.1f} host insns/guest  {dc/guest:6.1f} cycles/guest")
if di <= 0 or dc <= 0:
    print(
        "  WARNING: a non-positive difference means run-to-run noise swamped the\n"
        "  budget gap. Raise AERO_BENCH_HI/LO (defaults 20M/120M) — this number is\n"
        "  not meaningful.",
        file=sys.stderr,
    )
PY
done
