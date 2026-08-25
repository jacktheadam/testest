#!/usr/bin/env python3
"""
aero_diff — forward first-divergence diff for the Aero emulator.

Aligns two per-instruction execution-RIP streams (the QEMU ground truth captured
by tools/qemu-trace-plugin/aero_trace.so,exec=... and the Aero side captured by
AERO_TRACE_COMPACT=1) at a chosen entry RIP, then reports the first RIP where
the two executions part company. The first unreconciled divergence is the root
cause: a wrong branch (CPU flag bug) or a device read that returned a different
value. See wiki:notes/bring-up-findings-divergence-and-sight-audits for method.

Both streams are text files with one hex address per line, e.g. `0x7c00`.
(QEMU's aero_trace `exec=` and Aero's AERO_TRACE_COMPACT produce the same
format. Non-address lines are skipped.)

Usage:
    aero_diff.py <qemu_stream> <aero_stream> [--entry 0x7c00] [--context N]
                 [--max N] [--resync N]

  --entry ADDR   hex RIP to align both streams at (default 0x7c00 = El Torito
                 boot sector entry).
  --context N    lines of pre-divergence context to print (default 8)
  --max N        max instructions to compare after alignment (default 50_000_000)
  --resync N     reconvergence-aware mode: on divergence, search ahead up to N
                 instructions in each stream for the next common RIP and resync
                 there (recording a transient gap). BIOS-service-call structural
                 differences (Aero's Rust BIOS vs SeaBIOS) reconverge and are
                 reported as transient; a divergence that CANNOT be resynced is
                 a real bug. Recommended for the real-mode boot-sector phase.
                 Without --resync, the first divergence is reported directly.

Exit code 0 = no (unreconciled) divergence over the window; 1 = divergence found.
"""
import argparse
import sys
from collections import deque


def is_addr(line: str) -> bool:
    s = line.strip()
    return len(s) > 2 and s[:2] in ("0x", "0X") and all(c in "0123456789abcdefABCDEF" for c in s[2:])


def find_entry(path: str, entry: int) -> int:
    needle = f"0x{entry:x}"
    with open(path, "r", errors="replace") as f:
        for i, line in enumerate(f):
            if line.strip() == needle:
                return i
    return -1


def iter_addr_from(path: str, start_line: int):
    """Yield stripped address lines from `path` starting at line index
    `start_line`, skipping any non-address lines."""
    with open(path, "r", errors="replace") as f:
        for _ in range(start_line):
            if not f.readline():
                return
        for line in f:
            s = line.strip()
            if is_addr(s):
                yield s


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("qemu")
    ap.add_argument("aero")
    ap.add_argument("--entry", default="0x7c00")
    ap.add_argument("--context", type=int, default=8)
    ap.add_argument("--max", type=int, default=50_000_000)
    ap.add_argument("--resync", type=int, default=0,
                    help="reconvergence lookahead window (0 = report first divergence)")
    args = ap.parse_args()

    entry = int(args.entry, 16)

    q_start = find_entry(args.qemu, entry)
    a_start = find_entry(args.aero, entry)
    if q_start < 0:
        print(f"entry RIP 0x{entry:x} not found in QEMU stream {args.qemu}", file=sys.stderr)
        return 2
    if a_start < 0:
        print(f"entry RIP 0x{entry:x} not found in Aero stream {args.aero}", file=sys.stderr)
        return 2
    print(f"aligned at 0x{entry:x}: qemu line {q_start+1}, aero line {a_start+1}")

    if args.resync > 0:
        return diff_resync(args, q_start, a_start)
    return diff_first(args, q_start, a_start)


def diff_first(args, q_start, a_start):
    q_it = iter_addr_from(args.qemu, q_start)
    a_it = iter_addr_from(args.aero, a_start)
    context = deque(maxlen=args.context)
    compared = 0
    for q_line, a_line in zip(q_it, a_it):
        if compared >= args.max:
            print(f"no divergence within --max {args.max} instructions (window exhausted)")
            return 0
        if q_line != a_line:
            print(f"\n=== FIRST DIVERGENCE at instruction {compared} (0-indexed from entry) ===")
            print(f"QEMU RIP: {q_line}")
            print(f"Aero RIP: {a_line}")
            print(f"\n--- preceding {len(context)} matching instructions ---")
            base = compared - len(context)
            for i, rip in enumerate(context):
                print(f"  {base + i:>10}  {rip}")
            print(f"\n--- next 8 QEMU RIPs ---")
            for _ in range(8):
                try: print(f"  Q: {next(q_it)}")
                except StopIteration: break
            print(f"--- next 8 Aero RIPs ---")
            for _ in range(8):
                try: print(f"  A: {next(a_it)}")
                except StopIteration: break
            return 1
        context.append(q_line)
        compared += 1
    print(f"streams identical over {compared} instructions (end of one stream reached)")
    return 0


def diff_resync(args, q_start, a_start):
    """Reconvergence-aware diff. BIOS-service-call gaps (Aero Rust BIOS vs
    SeaBIOS) reconverge; a divergence that cannot be resynced within the
    lookahead bound is a real bug.

    Resync method: on divergence, snapshot Aero's next `look` RIPs into a set,
    then scan QEMU forward (up to `2*look` instructions) for any RIP in that set.
    This is O(n) and tolerates arbitrarily-asymmetric BIOS-call sizes (SeaBIOS
    servicing a disk read can run tens of thousands of instructions while Aero's
    Rust BIOS returns in 2)."""
    q_it = iter_addr_from(args.qemu, q_start)
    a_it = iter_addr_from(args.aero, a_start)
    look = max(args.resync, 4096)
    bound = 2 * look
    compared = 0
    gaps = 0
    total_gap_insns = 0
    last_common = None
    context = deque(maxlen=args.context)

    # Read everything we need from Aero into a list for random access during
    # resync scans; QEMU is streamed (it can be very large). Cap by args.max.
    a_all = []
    for s in a_it:
        a_all.append(s)
        if len(a_all) >= args.max + bound:
            break
    ai = 0  # index into a_all

    q_buf = []
    while compared < args.max:
        # Ensure q_buf has the current QEMU RIP.
        if not q_buf:
            try:
                q_buf.append(next(q_it))
            except StopIteration:
                break
        if ai >= len(a_all):
            break
        q_line = q_buf[0]
        a_line = a_all[ai]
        if q_line == a_line:
            last_common = q_line
            context.append(q_line)
            q_buf.pop(0)
            ai += 1
            compared += 1
            continue
        # Divergence: look for reconvergence. Snapshot Aero's next `look` RIPs.
        snap_end = min(ai + look, len(a_all))
        a_snap = set(a_all[ai:snap_end])
        # Scan QEMU forward (drain q_buf then stream) for a RIP in a_snap.
        found_q = None
        scan = q_buf[:]
        scan_idx = 0
        it_exhausted = False
        while scan_idx < bound:
            while scan_idx >= len(scan):
                try:
                    scan.append(next(q_it))
                except StopIteration:
                    it_exhausted = True
                    break
            if it_exhausted and scan_idx >= len(scan):
                break
            if scan[scan_idx] in a_snap:
                found_q = scan_idx
                break
            scan_idx += 1
        if found_q is None:
            print(f"\n=== REAL DIVERGENCE (no reconvergence within {bound}) at insn {compared} ===")
            print(f"last common RIP: {last_common}")
            print(f"QEMU RIP: {q_line}")
            print(f"Aero RIP: {a_line}")
            print(f"\n--- preceding {len(context)} matching instructions ---")
            base = compared - len(context)
            for k, rip in enumerate(context):
                print(f"  {base + k:>10}  {rip}")
            print(f"\n--- next 8 QEMU RIPs ---")
            for x in scan[1:9]:
                print(f"  Q: {x}")
            print(f"--- next 8 Aero RIPs ---")
            for x in a_all[ai + 1:ai + 9]:
                print(f"  A: {x}")
            print(f"\nsummary: {compared} matched instructions, {gaps} transient gaps resynced "
                  f"({total_gap_insns} gap insns)")
            return 1
        # Resync: find the Aero index for scan[found_q].
        a_idx = a_all.index(scan[found_q], ai, snap_end)
        q_gap = found_q
        a_gap = a_idx - ai
        gaps += 1
        total_gap_insns += q_gap + a_gap
        if gaps <= 5 or gaps % 50 == 0:
            print(f"[transient gap #{gaps} at insn {compared}: QEMU +{q_gap}, Aero +{a_gap}; "
                  f"resync at {scan[found_q]}]")
        # Advance: QEMU consumed up to and including found_q; Aero to a_idx.
        q_buf = scan[found_q + 1:]
        ai = a_idx + 1
        compared += 1
        last_common = scan[found_q]
        context.append(scan[found_q])

    print(f"\nno unreconciled divergence over {compared} instructions")
    print(f"summary: {gaps} transient gaps resynced ({total_gap_insns} gap insns); "
          f"last common RIP {last_common}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
