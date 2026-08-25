#!/usr/bin/env python3
"""
aero_ram_diff — value-level divergence via periodic RAM snapshots.

The exec-stream diff (aero_diff.py) finds control-flow divergence (wrong
branch). This complementary tool finds *data* divergence: it hashes regions of
guest RAM at aligned checkpoints and reports the first region where the two
emulators' memory contents diverge. This catches the class of bug where
control flow is identical but a store wrote a different value (e.g. a device
read returned different data, or an ALU flag/state computation differs).

Inputs are two directory trees of region snapshots, each file named
`<base_addr_hex>.bin` containing the raw bytes of a RAM region captured at a
common execution checkpoint (e.g. right after the boot sector loads, or at
kernel entry). Capture:

  QEMU:  (monitor)   pmemsave 0 <len> /tmp/qemu/<base>.bin
  Aero:  AERO_DUMP_MEM=<addr>:<len>[,...]   (on stop/fault)

or via the snapshot format's RAM section (see `cargo xtask snapshot inspect`).

For a coarse first pass, hash the whole region and compare; for a precise
location, find the first differing byte and print a hexdump window around it.

Usage:
    aero_ram_diff.py <qemu_dir> <aero_dir> [--context N] [--full]
"""
import argparse
import hashlib
import os
import sys


def list_regions(d: str):
    out = {}
    for name in os.listdir(d):
        if not name.endswith(".bin"):
            continue
        try:
            base = int(name[:-4], 16)
        except ValueError:
            continue
        out[base] = os.path.join(d, name)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("qemu_dir")
    ap.add_argument("aero_dir")
    ap.add_argument("--context", type=int, default=32,
                    help="bytes of hexdump context around the first diff (default 32)")
    ap.add_argument("--full", action="store_true",
                    help="report every differing region, not just the first")
    args = ap.parse_args()

    q = list_regions(args.qemu_dir)
    a = list_regions(args.aero_dir)
    common = sorted(set(q) & set(a))
    if not common:
        print(f"no common regions between {args.qemu_dir} and {args.aero_dir}", file=sys.stderr)
        print(f"  qemu regions: {sorted(hex(b) for b in q)[:10]}", file=sys.stderr)
        print(f"  aero regions: {sorted(hex(b) for b in a)[:10]}", file=sys.stderr)
        return 2

    print(f"{len(common)} common regions to compare")
    first = True
    for base in common:
        with open(q[base], "rb") as f: qb = f.read()
        with open(a[base], "rb") as f: ab = f.read()
        if len(qb) != len(ab):
            print(f"[size mismatch] region {base:#x}: qemu={len(qb)} aero={len(ab)}")
        qh = hashlib.sha256(qb).hexdigest()[:16]
        ah = hashlib.sha256(ab).hexdigest()[:16]
        if qh == ah and len(qb) == len(ab):
            continue
        # Find first differing byte.
        n = min(len(qb), len(ab))
        first_diff = -1
        for i in range(n):
            if qb[i] != ab[i]:
                first_diff = i
                break
        if first_diff < 0:
            print(f"[hash differs but bytes match up to min len {n}] region {base:#x}")
            continue
        print(f"\n=== {'FIRST' if first else 'NEXT'} RAM DIVERGENCE: region {base:#x} +{first_diff:#x} "
              f"(linear {base + first_diff:#x}) ===")
        s = max(0, first_diff - args.context)
        e = min(n, first_diff + args.context)
        print(f"  qemu sha256[:16]={qh}  aero sha256[:16]={ah}")
        print(f"  qemu[{s:#x}..{e:#x}]: {qb[s:e].hex(' ')}")
        print(f"  aero[{s:#x}..{e:#x}]: {ab[s:e].hex(' ')}")
        first = False
        if not args.full:
            return 1
    if first:
        print(f"all {len(common)} regions identical")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
