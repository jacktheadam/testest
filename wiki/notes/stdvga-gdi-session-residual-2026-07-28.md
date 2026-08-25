# STDVGA / GDI session residual (2026-07-28)

Non-normative investigation record from the cold VBE=BAR0 → framebuf /
setup-GDI bring-up segment. Normative resume state lives in
`wiki/state/repo-state-and-structure.md`; milestone history in
`wiki/history/windows-7-bring-up.md`.

Scratch proofs (ephemeral): `/tmp/grok-goal-2d3f1e2df180/implementer/native-gui/PROOF-*`.

## Summary

| Claim | Status |
|--|--|
| Natural bootvid/mid-kernel LFB COUNT under STDVGA | **Yes** (multi‑M `lfb_writes`, `lfb=0xe1000000`) |
| Hybrid desktop COUNT as “guest GDI” | **No** — synthetic fill demoted |
| Pure cold **PhysBasePtr=BAR0** through **setup.exe** | **Works with BAR1 absent** — setup PID 720 natural, `KiBugCheckData=0` |
| Pre-PhysBasePtr=BAR0 lineage (`bar0-c2`→setup) under current binary | **Works** — csrss/setup/raw `framebuf.pdb` string |
| Natural `STYPE_DEVICE` primary + setup GDI present | **Open** |
| Host REDIR + FORCE_DEVICE_SURF | Forces type=0 + bootvid `pvBits`; **paint still 0** |

## Resolution update (2026-07-31)

The failure below is retained as historical diagnosis of the old dual-BAR
topology. A fresh POST with STDVGA BAR1 actually absent, RC#40's architectural
same-value CR3 flush, and RC#41's pure shared-clock PM timer now reaches:

- `smss.exe`;
- two `csrss.exe` instances;
- `wininit.exe`;
- `winlogon.exe`;
- `winpeshl.exe`;
- `setup.exe`;
- all-zero `KiBugCheckData`.

No clock-cell poke, forced IF/CR8, COW override, display redirect, or synthetic
fill was used. Durable proof:
`<boot-out>/stdvga-clean/rc41-accelerated-plus9b.{bin,log,png}`.
The BAR1-absent session wall is closed; natural framebuf primary-surface
creation and setup GDI present remain open.

At the first clean winlogon checkpoint, a raw `framebuf.pdb` string is
present at physical `0x2b77c228`. The credible screen-sized 32-bpp SURFOBJ copies
(`0x0df6c428` and `0x2b4a8028`) are both `STYPE_BITMAP`, `cjBits=0x300000`,
with null `pvBits`/`pvScan0` and zero `lDelta`.

The string hit did **not** verify resident driver code. The exact framebuf
first-page `.text` signature is absent, and the CodeView-string physical page
has no mapping in System or the live process DTBs checked. A complete replay
from the last checkpoint without the string through the first checkpoint
with it records zero `0xaa55aa55` probe hits. The failure is upstream of
`framebuf!bInitSURF`'s framebuffer validation, or the attempted module is
unloaded before reaching it.

Six billion more instructions start services, LSASS, three svchost instances,
winpeshl, and finally **setup.exe naturally**, but leave these exact surface
fields unchanged and produce zero post-boot BAR0 writes. That forward control
separates slow process startup from the display defect.

QEMU 10.2.1 ground truth reaches the real Setup language UI and exposes
`1234:1111` with command `0x0103` (`IO|MEM`), revision 2, BAR0 16 MiB, BAR1
absent, and BAR2 4 KiB. Aero exposed command `0x0002`, revision 0, BAR0
16 MiB, BAR1 absent, and no BAR2. QEMU also performs a late 800×600×32 DISPI
transition that Aero never reaches. Win7's `display.inf` binds class
`PCI\CC_0300/0301` to `vgapnp.sys`; it does not require vendor/device
`1234:1111`. The exact `vgapnp.sys` access-range table requests fixed VBE
DISPI ports `0x1ce..0x1cf`, so legacy PCI I/O decode was the first difference
isolated. Its +2B late replay is unchanged despite guest-visible command `0x0003`:
no late VBE access, no `0xaa55aa55` probe, unchanged bitmap surfaces, and a
byte-identical final PNG. This does not reject the cold rule:
`vgapnp!VgaFindAdapter` calls `VideoPortGetVgaStatus`, and the exact
`videoprt.sys` PCI path requires `COMMAND.IO=1`; the +1B override may simply
postdate that one-shot check. Revision 2 alone is now also an exact
late-replay negative and is not consulted by the decoded videoprt VGA-status
branch. BAR2 remains a separate follow-up.

## AC1 honesty

| Era | `lfb_writes` | Natural? |
|--|--|--|
| Cold bootvid (VBE BAR0) | 6.7–10.5M | **Yes** |
| Mid-kernel Eng (`bar0-c2` / recount) | ~3.3M | **Yes** |
| Session progress slice (`c2-plus-12b`) | 180k | **Yes** (bootvid/session class, not setup UI chrome) |
| Setup/LogonUI GDI present | **0** | residual |
| Hybrid stable-gui COUNT 1.6M | — | **synthetic fill** (demoted) |

## Bugcheck wall: `0xC000021A` / `STATUS_NO_MEMORY`

### Capture method

`AERO_WATCH_WRITE=fffff8000d483a00` (this-boot **KiBugCheckData**, not a clock cell).

```
rip=…d379736 val=0x4c
rip=…d379940 val=0xc000021a
```

Final dump (s4, no COW):

| Field | Value |
|--|--|
| Code | `0xC000021A` STATUS_SYSTEM_PROCESS_TERMINATED |
| Arg1 | string **"initial session process or…"** (or “Session Manager Core Session failure”) |
| Arg3 low | **`0xC0000017` STATUS_NO_MEMORY** |
| Procs | smss alive; **no csrss** |

### Reproducibility

- From `cold-bar0-session-5b` +10B, **with and without** `AERO_COW_FORCE_RW`
- From `cold-vbe-bar0-800m` +15B **with early clock pokes** (pokes are not the sole cause)
- **Do not** `AERO_POKE_LINEAR` `fffff8000d483a00` — that is KiBugCheckData on this layout

### Contrast that still works

`bar0-c2-8b` → `c2-plus-12b` (booted when PhysBasePtr was still BAR1+`VBE_LFB_OFFSET` / `e4040000`):

- **csrss×2, winlogon, winpeshl, setup.exe** (14 processes)
- `framebuf.pdb` @ phys `0x2b912228`
- SCE wedge applied
- Host `DUMP_VGA` reports `lfb=0xe1000000` after snapshot-load `sync_bios_vbe_lfb_base_to_display_wiring`
- Natural SURFOBJ still **STYPE_BITMAP + null `pvBits`** (EnableSurface incomplete)

So: current STDVGA host is not categorically broken for session; **pure cold PhysBasePtr=BAR0 lineage** is.

### 4 GiB re-POST

Stuck earlier at `rip=0xfffff8000d7fae82` `cr8=0xf` (~11–15B), no KiBugCheck write; natural `lfb_writes≈10.5M`. Did not reach session. Snap RAM must match (`--ram 4096` cannot load a 2 GiB snap).

## framebuf EnableSurface (PE disasm)

`framebuf.dll` rva `0x1848` / map helper `0x229c`:

1. VIDEO IOCTL `0x23040c` prep, `0x230458` → LFB VA → **PDEV+0x20**
2. **Probe** write/read `0xaa55aa55` at that VA — fail aborts before CreateDeviceSurface
3. On success: `EngAllocMem` tag **`DDfb`** (`0x62664444`)
4. `EngCreateDeviceSurface` + `EngModifySurface(pvScan0=PDEV+0x20)`

If the VIDEO map VA has NP PTEs, probe fails → no durable `STYPE_DEVICE`.

## Host wedges (bring-up only)

| Env | Role | Honest limit |
|--|--|--|
| `AERO_AEROGPU_STDVGA_IDS=1` | IDs `1234:1111`, BAR0 16 MiB LFB alias, VBE PhysBasePtr=BAR0 | Verified through natural setup.exe with BAR1 absent |
| `AERO_STDVGA_ENABLE_IO=1` | Restored-checkpoint-only PCI command bit-0 override | +1B→+3B replay unchanged; timing vs `VgaFindAdapter` unresolved |
| `AERO_STDVGA_REVISION=<u8>` | Restored-checkpoint-only PCI revision override | `2` is a byte-identical +1B→+3B negative; not used by decoded VGA-status branch |
| `AERO_REDIR_ENG_LFB=1` | Eng + SURFOBJ → bootvid KVA | Not natural |
| `AERO_FORCE_DEVICE_SURF=1` | BITMAP→`STYPE_DEVICE`, cjBits=0 | Still `lfb_writes=0` |
| `AERO_COW_FORCE_RW=1` | User RO leaf → RW on write | Fixes setup soft-fault; not session wall |
| `AERO_FORCE_CR8=0` | **Avoid** after multi-thread session | Causes `0xB8` → d804e debugger |

## Code change in tree (2026-07-28, verified on full cold re-POST 2026-07-31)

Under STDVGA, **clear BAR1** (canonical 64 MiB VRAM aperture) so the guest sees
no second large framebuffer aperture:

- `crates/aero-machine/src/lib.rs` — `apply_aerogpu_stdvga_pci_ids` calls `clear_bar_definition(BAR1)`
- `crates/devices/src/pci/config.rs` — new `PciConfigSpace::clear_bar_definition`
- Tests (`aerogpu_stdvga_pci_ids` + `aero-devices` PCI config):
  - `clear_bar_survives_guest_sizing_probe_and_does_not_decode` — Win7
    `0xFFFFFFFF` size probe after hide must stay 0 (phantom second FB)
  - `restore_of_probed_bar_onto_cleared_slot_stays_unused` — snapshot of a
    mid-probe 64 MiB BAR1 restored onto STDVGA must not leak the old mask
  - `clear_64bit_bar_while_high_dword_is_probed` — clear 64-bit BAR mid-probe
    then reuse BAR1 as a 32-bit slot
  - `cannot_clear_high_dword_of_64bit_bar_independently` — refuse splitting
    a 64-bit BAR
  - `stdvga_bar1_stays_unimplemented_through_sizing_probe_and_reset` — POST
    + platform reset must not re-hand BAR1 a window
  - `stdvga_snapshot_restore_does_not_resurrect_bar1_after_probe` — machine
    snap taken during BAR1 probe restores unused
  - `last_hit_does_not_dispatch_after_bar_definition_cleared` — MMIO
    `last_hit` cursor + BAR1 hide
  - `last_hit_respects_mem_decode_disabled_mid_stream` — decode bit flipped
    mid-blit
  - `last_hit_does_not_clip_straddle_past_bar_end` — stale hit + size
    boundary
  - `stdvga_bar0_lfb_hits_last_dword_and_not_the_byte_past_the_aperture` —
    last in-BAR dword, one-past, MEM decode off then on

Modern QEMU standard VGA also exposes a small 4 KiB BAR2 control window, so
“single-BAR Bochs” was imprecise. The fresh BAR1-absent lineage verifies that
this topology passes session
creation. It does not isolate BAR1 as the only historical cause because the
same lineage also includes subsequent CPU and timer correctness fixes; the
observable contract is nonetheless now green.

A partial cold re-POST with the BAR1-cleared binary was started (`stdvga-nobar1-18b`) and reached ~1.8B Protected with natural `lfb_writes≈7.2M`, then was **killed during session wrap-up** — not a negative or positive session result.

## Resume recipe (next agent)

1. For process continuation, resume
   `<boot-out>/stdvga-clean/rc41-accelerated-plus9b.bin` with its
   `std-disk.aerospar`; setup.exe is already naturally alive.
2. For the display divergence, resume
   `rc41-accelerated-plus1b.bin`. The late PCI I/O replay is unchanged, but
   do not eliminate it until proving whether `VgaFindAdapter` ran earlier.
   Use the vgapnp code/string timeline and `AERO_LOG_STDVGA_PCI=1`, then
   replay from an earlier checkpoint or cold POST as required. Isolate
   revision 2 and BAR2 only afterward.
3. Scan SURFOBJ for **type=0 + non-null `pvBits`** **without** REDIR; require
   `lfb_writes>0` after framebuf enable.
4. Keep `KiBugCheckData @ fffff8000d48ca00` zero and do not claim setup GDI
   from boot-animation writes.
5. The older `bar0-c2` scratch lineage is comparison history, not the primary
   resume path.

## Proof inventory (scratch)

| File | Content |
|--|--|
| `PROOF-cold-bar0-c000021a.txt` | Watch capture `0xc000021a` |
| `PROOF-cold-bar0-c000021a-nomem.txt` | Arg decode + NO_MEMORY |
| `PROOF-cold-bar0-s2-hal-spin.txt` | d804e after s2 |
| `PROOF-cold4g-hal-spin-d7fae.txt` | 4 GiB early spin |
| `PROOF-c2-session-framebuf-loaded.txt` | Working lineage + framebuf |
| `PROOF-framebuf-enablesurface-probe.txt` | PE disasm probe |
| `PROOF-setup-framebuf-stype-device-redir.txt` | Forced STYPE_DEVICE, paint 0 |
| `PROOF-residual-ac1-setup-gdi.txt` | AC1 honesty table |
