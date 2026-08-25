# aero_trace — QEMU TCG plugin for ground-truth execution capture

A self-contained QEMU TCG plugin that captures per-instruction execution and
per-write memory streams from QEMU, to serve as the **reference side** of a
forward first-divergence diff against the Aero emulator.

The matching **Aero side** already exists in-tree:
`AERO_TRACE_COMPACT` (per-instruction linear address) and `AERO_WRITE_STREAM`
(every guest write as `rip addr len value`). The QEMU half — this plugin — is
what was previously the unbuilt gap on the reference side of that comparison.

## Why

Aero's Win7 bring-up debugged ~20 boot bugs in *crash order*: boot 20 minutes,
hit a fault, fix, repeat. Forward first-divergence diffing instead boots both
QEMU (ground truth — it boots Win7 to desktop in ~75 s) and Aero from the same
ISO, then diffs their execution streams. **The first RIP where the two part
company is the root cause**, regardless of where the eventual crash surfaces —
finding many bugs per diff run instead of one per boot cycle.

## Streams

| Stream | Plugin arg | Format (one record/line, hex) | Aero counterpart |
|---|---|---|---|
| Execution RIP | `exec=<file>` | `0x<linear_addr>` per instruction | `AERO_TRACE_COMPACT` |
| RAM writes | `writes=<file>` | `<rip> <paddr> <size>` per store | `AERO_WRITE_STREAM` (Aero also has value) |

Optional controls: `from=<n>`, `to=<n>` (retired-instruction window),
`maxexec=<n>`, `maxwrites=<n>` (record caps).

## Quick start (one command)

```
tools/qemu-trace-plugin/aero_diverge.sh --max-insts 12000000
```

This captures QEMU ground truth, captures the Aero trace, aligns at the boot
sector entry (`0x7c00`), and reports the first divergence. Traces are cached in
`/tmp/aero-diff/` so re-runs with a larger `--max-insts` only extend the
capture, not redo it. Delete `/tmp/aero-diff/` to force a full re-capture.

## Build

```
make
```

No `libglib2.0-dev` required. `glib-shim.h` provides the handful of glib *types*
that the vendored `qemu-plugin.h` references at compile time; this plugin never
calls any glib-returning API function (notably it avoids `qemu_plugin_insn_data`,
which segfaults inside `translator_st()` for some TB configs in QEMU 10.x —
QEMU's own example plugins avoid it too). CPUID/MSR ground truth is therefore
derived from QEMU source + SDM analysis rather than captured here.

## Capture QEMU ground truth (Win7 ISO boot)

```
timeout -k 3 60 qemu-system-x86_64 -nographic -machine pc -m 2048 \
    -cdrom <win7.iso> -boot d -serial file:/tmp/qemu_serial.txt \
    -plugin ./aero_trace.so,exec=/tmp/qemu_exec.txt
```

The El Torito boot sector entry (`0x7c00`) lands at ~instruction 6.6 M for
SeaBIOS 1.17; everything before that is SeaBIOS (not comparable — Aero uses its
own Rust BIOS), everything after is ISO-loaded code identical to what Aero runs.

## Capture the Aero side

```
AERO_TRACE_FROM=0 AERO_TRACE_TO=12000000 AERO_TRACE_COMPACT=1 \
    ./target/release/aero-machine --install-iso <win7.iso> --ram 2048 \
    --max-insts 12000000 --serial-out none --debugcon-out none \
    >/tmp/aero_exec.txt 2>/dev/null
```

## Diff (find the first divergence)

```
python3 ./aero_diff.py /tmp/qemu_exec.txt /tmp/aero_exec.txt --entry 0x7c00
```

Aligns both streams at the El Torito entry `0x7c00` and reports the first RIP
where execution diverges, with context. A clean split that never reconverges is
almost certainly a real bug; scattered single-line divergences are usually timer
nondeterminism (filter with `from=/to=` windows around interrupt-free regions).

## Notes / limitations

- **Determinism:** timer interrupts can fire at different instruction counts
  between QEMU and Aero, producing non-bug divergences. The first divergence in
  an interrupt-disabled window (boot sector / early bootmgr) is the strong
  signal. QEMU `-icount` + Aero deterministic TSC extends the aligned run.
- **Write values:** the QEMU mem callback does not expose the store value, so
  value-level divergence is detected via periodic RAM-snapshot diffs
  (`monitor pmemsave` on QEMU vs Aero's `AERO_DUMP_MEM`) hashed at checkpoints,
  not via this plugin's write stream.
- **Per-instruction overhead:** expect QEMU to run roughly an order of magnitude
  slower under the plugin. For long captures use a focused `from=/to=` window
  past a known point.
