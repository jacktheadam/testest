#!/usr/bin/env python3
"""First-divergence between an Aero execution trace and a QEMU one.

Backward-chaining a crash costs effort proportional to the length of the causal
chain; the Windows 7 bring-up record has a root cause that took six hops.
Forward first-divergence makes the chain length irrelevant: run both emulators on
the same guest, find the first place they disagree, and that *is* the bug.

This tool existed before as `/tmp/flowdiff.py`, `/tmp/firstvisit.py` and
`/tmp/ordercmp.py`, with a note to promote it into the repository "once the shape
settles". `/tmp` was cleared and all three were lost, so the technique had to be
rebuilt from memory each time it was wanted — which is most of the reason it was
only ever used ad hoc. It lives here now.

Input formats
-------------
Aero  : `AERO_TRACE_COMPACT=1` writes one linear address per line.
QEMU  : `-d exec` writes lines like
        `Trace 0: 0x7f3c40000100 [00000000/00000000000ffff0/0x00000000/...]`
        where the guest PC is the second field inside the brackets. Older builds
        emit `Trace 0x... [pc]`; both are accepted.

Comparison modes
----------------
raw          every executed block, in order. Strictest, and noisy when the two
             differ only in how they chunk basic blocks.
first-visit  the ordered sequence of addresses on their *first* execution.
             Robust to loops running a different number of times, which is the
             common benign difference between two emulators. This is the default.
page         first-visit reduced to 4 KiB pages. Coarsest; use when the traces
             diverge in block chunking but agree on control flow at page level.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# `Trace 0: 0xHOST [flags/PC/cs_base/...]` and the older `Trace 0xHOST [PC]`.
QEMU_BRACKET = re.compile(r"\[([0-9a-fA-F/x]+)\]")
HEX_LINE = re.compile(r"^\s*(?:0x)?([0-9a-fA-F]+)\s*$")


def parse_qemu(path: Path, limit: int) -> list[int]:
    out: list[int] = []
    with path.open("r", errors="replace") as fh:
        for line in fh:
            if "Trace" not in line:
                continue
            m = QEMU_BRACKET.search(line)
            if not m:
                continue
            fields = [f for f in m.group(1).split("/") if f]
            if not fields:
                continue
            # Second bracket field is the guest PC on modern builds; on older
            # single-field builds the only field is the PC.
            raw = fields[1] if len(fields) > 1 else fields[0]
            try:
                out.append(int(raw, 16))
            except ValueError:
                continue
            if len(out) >= limit:
                break
    return out


def parse_aero(path: Path, limit: int) -> list[int]:
    out: list[int] = []
    with path.open("r", errors="replace") as fh:
        for line in fh:
            m = HEX_LINE.match(line)
            if not m:
                continue
            try:
                out.append(int(m.group(1), 16))
            except ValueError:
                continue
            if len(out) >= limit:
                break
    return out


def first_visit(seq: list[int]) -> list[int]:
    seen: set[int] = set()
    out: list[int] = []
    for a in seq:
        if a not in seen:
            seen.add(a)
            out.append(a)
    return out


def to_pages(seq: list[int]) -> list[int]:
    return [a & ~0xFFF for a in seq]


def report(aero: list[int], qemu: list[int], mode: str, context: int) -> int:
    if not aero:
        print("tracediff: the Aero trace is empty — was AERO_TRACE_COMPACT=1 set?", file=sys.stderr)
        return 2
    if not qemu:
        print("tracediff: the QEMU trace is empty — was -d exec -D <file> used?", file=sys.stderr)
        return 2

    n = min(len(aero), len(qemu))
    idx = next((i for i in range(n) if aero[i] != qemu[i]), None)

    print(f"=== first divergence ({mode}) ===")
    print(f"  aero: {len(aero)} entries   qemu: {len(qemu)} entries   compared: {n}")

    if idx is None:
        if len(aero) == len(qemu):
            print("  no divergence: the traces agree over their whole length.")
            return 0
        longer = "aero" if len(aero) > len(qemu) else "qemu"
        print(f"  no divergence within the common prefix; {longer} simply ran longer.")
        print("  (Raise the budget on the shorter side before reading anything into this.)")
        return 0

    print(f"  DIVERGES at index {idx}")
    print(f"    aero: {aero[idx]:#018x}")
    print(f"    qemu: {qemu[idx]:#018x}")
    print()
    lo = max(0, idx - context)
    hi = min(n, idx + context + 1)
    print(f"  context [{lo}:{hi}]  (>>> marks the divergence)")
    print(f"  {'idx':>8}  {'aero':>18}  {'qemu':>18}")
    for i in range(lo, hi):
        mark = ">>>" if i == idx else "   "
        print(f"{mark} {i:>8}  {aero[i]:#018x}  {qemu[i]:#018x}")
    print()
    print("  The first address Aero reaches that QEMU does not (or vice versa) is where")
    print("  the guest's control flow first depended on something the two model")
    print("  differently. Disassemble both sides at the preceding address.")
    return 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("aero_trace", type=Path, help="Aero compact trace (AERO_TRACE_COMPACT=1)")
    ap.add_argument("qemu_trace", type=Path, help="QEMU exec log (-d exec -D <file>)")
    ap.add_argument("--mode", choices=("raw", "first-visit", "page"), default="first-visit")
    ap.add_argument("--limit", type=int, default=20_000_000, help="max entries to read per side")
    ap.add_argument("--context", type=int, default=8, help="entries of context to print around the divergence")
    args = ap.parse_args()

    for p in (args.aero_trace, args.qemu_trace):
        if not p.is_file():
            print(f"tracediff: no such file: {p}", file=sys.stderr)
            return 2

    aero = parse_aero(args.aero_trace, args.limit)
    qemu = parse_qemu(args.qemu_trace, args.limit)

    if args.mode == "first-visit":
        aero, qemu = first_visit(aero), first_visit(qemu)
    elif args.mode == "page":
        aero, qemu = first_visit(to_pages(aero)), first_visit(to_pages(qemu))

    return report(aero, qemu, args.mode, args.context)


if __name__ == "__main__":
    raise SystemExit(main())
