#!/usr/bin/env bash
# aero_diverge.sh — one-command forward first-divergence diff between QEMU and Aero.
#
# Captures a QEMU Win7 boot execution trace (via the TCG plugin or -d exec), captures
# the matching Aero trace, aligns them at the El Torito boot sector entry (0x7c00),
# and reports the first divergence. BIOS-service-call gaps are automatically resynced.
#
# Usage:
#   tools/qemu-trace-plugin/aero_diverge.sh [--iso <path>] [--max-insts <N>] [--mode exec|plugin]
#
# Options:
#   --iso <path>        Win7 ISO path (default: /root/aero-images/7601.24214...iso)
#   --max-insts <N>     Max Aero instructions (default: 12000000)
#   --mode exec         Use QEMU -d exec (fast, TB-level; default)
#   --mode plugin       Use QEMU TCG plugin (slower, instruction-level)
#   --resync <N>        Reconvergence lookahead window (default: 10000)
#
# Requirements:
#   - QEMU 10.x with TCG plugin support (built: tools/qemu-trace-plugin/aero_trace.so)
#   - Aero release binary: target/release/aero-machine
#   - Python 3 for the diff scripts
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORK_DIR="${AERO_DIFF_WORK_DIR:-/tmp/aero-diff}"

# Defaults
ISO="/root/aero-images/7601.24214.180801-1700.win7sp1_ldr_escrow_CLIENT_PROFESSIONAL_x64FRE_en-us.iso"
MAX_INSTS=12000000
MODE="exec"
RESYNC=10000
QEMU_TIMEOUT=60
AERO_TIMEOUT=600

# Parse args
while [[ $# -gt 0 ]]; do
    case "$1" in
        --iso) ISO="$2"; shift 2 ;;
        --max-insts) MAX_INSTS="$2"; shift 2 ;;
        --mode) MODE="$2"; shift 2 ;;
        --resync) RESYNC="$2"; shift 2 ;;
        --help|-h)
            head -25 "$0" | tail -22
            exit 0 ;;
        *) echo "unknown arg: $1"; exit 1 ;;
    esac
done

mkdir -p "$WORK_DIR"
cd "$WORK_DIR"

echo "=== Aero Divergence Harness ==="
echo "ISO:     $ISO"
echo "Max insns: $MAX_INSTS"
echo "Mode:    $MODE"
echo ""

# --- 1. Capture QEMU ground truth ---
QEMU_EXEC="$WORK_DIR/qemu_exec.txt"
QEMU_SERIAL="$WORK_DIR/qemu_serial.txt"

if [[ ! -f "$QEMU_EXEC" ]]; then
    echo "[1/3] Capturing QEMU ground truth (${QEMU_TIMEOUT}s timeout)..."
    case "$MODE" in
        plugin)
            PLUGIN="$SCRIPT_DIR/aero_trace.so"
            if [[ ! -f "$PLUGIN" ]]; then
                echo "  Building plugin..."
                (cd "$SCRIPT_DIR" && make) || { echo "  ERROR: plugin build failed"; exit 1; }
            fi
            timeout -k 3 "$QEMU_TIMEOUT" qemu-system-x86_64 \
                -nographic -machine pc -m 2048 -nic none \
                -cdrom "$ISO" -boot d \
                -serial "file:$QEMU_SERIAL" \
                -plugin "$PLUGIN",exec="$QEMU_EXEC",maxexec=200000000 \
                >/dev/null 2>&1 || true
            ;;
        exec)
            timeout -k 3 "$QEMU_TIMEOUT" qemu-system-x86_64 \
                -nographic -machine pc -m 2048 -nic none \
                -cdrom "$ISO" -boot d \
                -serial "file:$QEMU_SERIAL" \
                -d exec -D "$QEMU_EXEC" \
                >/dev/null 2>&1 || true
            # Convert -d exec format to one-RIP-per-line
            EXECLOG="$QEMU_EXEC"
            QEMU_EXEC="${QEMU_EXEC%.txt}_parsed.txt"
            grep -oP '(?<=/)[0-9a-f]{16}' "$EXECLOG" | sed 's/^/0x/' > "$QEMU_EXEC" || true
            ;;
    esac
    echo "  QEMU trace: $(wc -l < "$QEMU_EXEC") lines"
else
    echo "[1/3] QEMU trace cached ($QEMU_EXEC)"
fi

# --- 2. Capture Aero side ---
AERO_EXEC="$WORK_DIR/aero_exec.txt"
AERO_BIN="$REPO_ROOT/target/release/aero-machine"

if [[ ! -f "$AERO_EXEC" ]]; then
    echo "[2/3] Capturing Aero trace (${AERO_TIMEOUT}s timeout)..."
    if [[ ! -x "$AERO_BIN" ]]; then
        echo "  Building aero-machine (release)..."
        (cd "$REPO_ROOT" && cargo build --release -p aero-machine-cli --locked) || {
            echo "  ERROR: build failed"; exit 1; }
    fi
    AERO_TRACE_FROM=0 AERO_TRACE_TO="$MAX_INSTS" AERO_TRACE_COMPACT=1 \
        timeout -k 5 "$AERO_TIMEOUT" "$AERO_BIN" \
        --install-iso "$ISO" --ram 2048 \
        --max-insts "$MAX_INSTS" \
        --serial-out none --debugcon-out none \
        > "$AERO_EXEC" 2> "$WORK_DIR/aero_trace.err" || true
    # Aero writes the compact trace to stdout
    if [[ ! -s "$AERO_EXEC" ]]; then
        # Trace might be on stderr
        cp "$WORK_DIR/aero_trace.err" "$AERO_EXEC"
    fi
    echo "  Aero trace: $(wc -l < "$AERO_EXEC") lines"
else
    echo "[2/3] Aero trace cached ($AERO_EXEC)"
fi

# --- 3. Diff ---
echo "[3/3] Diffing (aligned at 0x7c00, resync window $RESYNC)..."
case "$MODE" in
    plugin)
        python3 "$SCRIPT_DIR/aero_diff.py" \
            "$QEMU_EXEC" "$AERO_EXEC" \
            --entry 0x7c00 --resync "$RESYNC" --max "$MAX_INSTS" \
            || true
        ;;
    exec)
        python3 "$SCRIPT_DIR/aero_execlog_diff.py" \
            "$QEMU_EXEC" "$AERO_EXEC" \
            --entry 0x7c00 --max "$MAX_INSTS" \
            || true
        ;;
esac

echo ""
echo "=== Done. Traces cached in $WORK_DIR ==="
echo "Re-run with --max-insts to extend, or delete $WORK_DIR to re-capture."
