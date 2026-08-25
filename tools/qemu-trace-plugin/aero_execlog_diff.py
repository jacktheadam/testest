#!/usr/bin/env python3
"""
aero_execlog_diff — fast QEMU `-d exec` → Aero AERO_TRACE_COMPACT diff.

QEMU's built-in `-d exec` logging is ~30× faster than the TCG plugin approach
(no per-instruction callback overhead). It logs at translation-block (TB)
granularity rather than per-instruction, but for first-divergence detection this
is equivalent: if QEMU executes a TB at address X and Aero doesn't (or vice
versa), that's a divergence. Since both decode the same x86 bytes, a matching
TB-start RIP implies the entire TB matches.

QEMU `-d exec` output format (one line per TB execution):
    Trace 0: 0xHOST [CS_BASE/GUEST_RIP/FLAGS/HFLAGS]

This tool extracts the GUEST_RIP field from each Trace line, aligns at an entry
RIP, and diffs against Aero's AERO_TRACE_COMPACT output.

Usage:
    # Capture QEMU (fast, no plugin):
    qemu-system-x86_64 -nographic ... -d exec -D qemu_execlog.txt

    # Capture Aero:
    AERO_TRACE_COMPACT=1 ./aero-machine ... > aero_exec.txt

    # Diff:
    python3 aero_execlog_diff.py qemu_execlog.txt aero_exec.txt --entry 0x7c00
"""
import argparse
import re
import sys

TRACE_RE = re.compile(r'Trace\s+\d+:\s+0x[0-9a-f]+\s+\[([0-9a-f]+)/([0-9a-f]+)/')


def parse_execlog(path: str):
    """Yield guest RIPs from a QEMU -d exec log, one per TB execution."""
    with open(path, "r", errors="replace") as f:
        for line in f:
            m = TRACE_RE.search(line)
            if m:
                rip = int(m.group(2), 16)
                yield f"0x{rip:x}"


def parse_compact(path: str):
    """Yield stripped address lines from Aero AERO_TRACE_COMPACT output."""
    with open(path, "r", errors="replace") as f:
        for line in f:
            s = line.strip()
            if s.startswith("0x") and len(s) > 2:
                yield s


def find_entry(stream, entry_hex: str):
    """Return the first matching address, or None."""
    for addr in stream:
        if addr == entry_hex:
            return addr
    return None


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("qemu_execlog", help="QEMU -d exec output file")
    ap.add_argument("aero_stream", help="Aero AERO_TRACE_COMPACT output file")
    ap.add_argument("--entry", default="0x7c00",
                    help="hex RIP to align at (default 0x7c00 = El Torito)")
    ap.add_argument("--context", type=int, default=8)
    ap.add_argument("--max", type=int, default=10_000_000)
    args = ap.parse_args()

    entry_hex = f"0x{int(args.entry, 16):x}"

    # Parse both streams fully into lists (TB granularity for QEMU is much
    # smaller than per-instruction Aero, so this is manageable).
    print(f"parsing QEMU execlog...", file=sys.stderr)
    q_all = list(parse_execlog(args.qemu_execlog))
    print(f"  {len(q_all)} TB executions", file=sys.stderr)
    print(f"parsing Aero compact trace...", file=sys.stderr)
    a_all = list(parse_compact(args.aero_stream))
    print(f"  {len(a_all)} instructions", file=sys.stderr)

    q_idx = next((i for i, x in enumerate(q_all) if x == entry_hex), -1)
    a_idx = next((i for i, x in enumerate(a_all) if x == entry_hex), -1)
    if q_idx < 0:
        print(f"entry {entry_hex} not found in QEMU stream", file=sys.stderr)
        return 2
    if a_idx < 0:
        print(f"entry {entry_hex} not found in Aero stream", file=sys.stderr)
        return 2
    print(f"aligned at {entry_hex}: qemu TB #{q_idx}, aero insn #{a_idx}")

    # Compare. QEMU is at TB granularity; each QEMU entry is a TB-start RIP.
    # Aero is per-instruction. A TB-start RIP in QEMU should appear in the Aero
    # stream (it's the first instruction of that TB). If QEMU's next TB-start
    # doesn't appear in the Aero stream before Aero's addresses diverge, that's
    # the divergence point.
    compared = 0
    context = []
    qi = q_idx
    ai = a_idx
    while qi < len(q_all) and ai < len(a_all) and compared < args.max:
        q_rip = q_all[qi]
        # Scan Aero forward for the QEMU TB-start RIP.
        found = False
        for j in range(ai, min(ai + 50000, len(a_all))):
            if a_all[j] == q_rip:
                context.append(q_rip)
                if len(context) > args.context:
                    context.pop(0)
                ai = j + 1
                qi += 1
                compared += 1
                found = True
                break
        if not found:
            print(f"\n=== DIVERGENCE at TB #{qi} (Aero insn ~{ai}) ===")
            print(f"QEMU next TB starts at: {q_rip}")
            print(f"Aero current insn:       {a_all[ai]}")
            print(f"\n--- preceding {len(context)} matching TB-starts ---")
            for i, rip in enumerate(context):
                print(f"  {i:>6}  {rip}")
            print(f"\n--- next 5 QEMU TB-starts ---")
            for k in range(qi, min(qi + 5, len(q_all))):
                print(f"  Q: {q_all[k]}")
            print(f"--- next 5 Aero insns ---")
            for k in range(ai, min(ai + 5, len(a_all))):
                print(f"  A: {a_all[k]}")
            return 1

    print(f"\nno divergence over {compared} TB-start matches")
    return 0


if __name__ == "__main__":
    sys.exit(main())
