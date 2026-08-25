# Platform and firmware

> The PC platform Aero presents to the guest: interrupt controllers and their
> semantics, timers, the PCI topology and device identity table, ACPI, the
> BIOS and its INT services, and El Torito CD boot. Also the migration state of
> the legacy device stack toward the canonical machine.

The canonical machine wiring, BIOS/firmware, platform contracts (PCI identity,
IRQ routing, snapshots), and the debugging/IPC surface that supports them.
Governed by the canonical VM core decision (the canonical VM core decision,
`wiki/decisions/0008-canonical-vm-core.md`) and the canonical machine stack
decision (the canonical machine stack decision, `wiki/decisions/0014-canonical-machine-stack.md`).

## Canonical machine and crate map

The only canonical VM wiring layer is `crates/aero-machine`
(`aero_machine::Machine`). Everything that runs the machine — browser WASM
exports, host integration tests, snapshot tooling — builds on it.

Supporting building blocks:

- `crates/firmware` — legacy BIOS implementation in Rust (POST + INT dispatch)
  plus firmware table generation (via `crates/aero-acpi`, SMBIOS, E820).
- `crates/memory` — guest physical memory backends + `PhysicalMemoryBus`;
  `MappedGuestMemory` covers PC/Q35 non-contiguous layouts (ECAM + PCI/MMIO
  holes + >4 GiB remap) with open-bus hole semantics (reads `0xFF`, writes
  ignored).
- `crates/platform` (`aero-platform`) — port I/O bus, chipset/reset wiring,
  interrupt routing.
- `crates/devices` (`aero-devices`) — reusable device models + PCI
  infrastructure (including `aero_devices::pci::profile`, the canonical PCI
  identity registry).
- `crates/aero-pc-platform` — PC platform composition helper
  (PIC/PIT/RTC/APIC/HPET + PCI bus + BAR MMIO mapping); expected to fold into
  `aero-machine` over time.
- `crates/aero-interrupts` — canonical APIC implementation, consumed by
  `aero-machine` / `aero-pc-platform`.
- `crates/aero-snapshot` — snapshot file format + save/restore machinery.
- `crates/aero-smp` — minimal deterministic SMP/APIC model (its type is `Machine`, not `SmpMachine`) for
  unit tests and snapshot validation; not part of the canonical wiring stack.
- `crates/aero-boot-tests` — test-only crate registering the QEMU-based boot
  tests under the workspace-root `tests/` directory.

Browser/WASM entrypoints (the web runtime loads the `aero-wasm` package from
`crates/aero-wasm`):

| WASM export | Backing runtime | JS entrypoint | Status |
|---|---|---|---|
| `Machine` | `aero_machine::Machine` | `apps/web/src/workers/machine_cpu.worker.ts` (`vmRuntime=machine`), `apps/web/src/main.ts` (demo) | **Canonical** full-system machine |
| `WasmVm` (`vm.rs`) | `aero_cpu_core` + `aero_mmu` | `apps/web/src/workers/cpu.worker.ts` | Legacy CPU-worker runtime; port I/O + MMIO forwarded to JS |
| `WasmTieredVm` (`tiered_vm.rs`) | tiered `aero_cpu_core` + JS Tier-1 JIT calls | `apps/web/src/workers/cpu.worker.ts` | Same, with Tier-0+Tier-1 tiering |
| `PcMachine` (`pc_machine.rs`) | `aero_machine::PcMachine` | unused by main runtime | Experimental; allocates guest RAM inside the wasm module |
| `DemoVm` | wrapper around `aero_machine::Machine` | `apps/web/src/workers/demo_vm_snapshot.worker.ts` | Deprecated snapshot demo API kept for UI panels |

### The retired second stack

There is one device stack: `aero-machine` plus the `aero-*` device crates. A second one,
`crates/emulator`, carried its own PCI framework, storage traits, disk formats, USB and input glue,
audio device models, and AeroGPU device and executor, and has been retired. The three components
that had no canonical equivalent moved out first — the software rasterizer to
`crates/aero-gpu-software`, the `AEROSPRS` format and its converter to `crates/aero-storage`, the
sample-rate and channel conversion to `aero_audio::dsp`. The rest was a duplicate of a canonical
crate or a re-export shim.

The full account — what each piece was, where it went, what was deliberately not carried over, and
the one integration that was left unbuilt rather than faked — is
[below](#the-retirement-of-cratesemulator).

Fully retired predecessors live under `crates/legacy/` (`vm`, `aero-emulator`,
`aero-vm` — the latter a deprecated deterministic demo VM excluded from the
workspace).

## BIOS firmware (HLE)

Aero's canonical boot path is a legacy BIOS implemented in Rust as high-level
emulation; there is no external BIOS blob and UEFI is not the canonical path.

- Implementation: `crates/firmware` (`firmware::bios`); machine integration in
  `crates/aero-machine`. Source-of-truth module docs:
  `crates/firmware/README.md` and `crates/firmware/src/bios/mod.rs`.
- `firmware::bios::build_bios_rom()` builds a 64 KiB ROM mapped at
  `BIOS_BASE` (`0x000F_0000`) and aliased at `BIOS_ALIAS_BASE` (`0xFFFF_0000`)
  so the reset vector at `0xFFFF_FFF0` works (constants verified in
  `crates/firmware/src/bios/mod.rs`). The image contains both HLE interrupt
  stubs and a real 16-bit VBE handler that can be executed by a guest-side
  BIOS interpreter.
- ACPI tables are generated by `crates/aero-acpi` and written into guest RAM
  during POST.

### HLT-in-ROM-stub "hypercall" dispatch

The CPU core does not trap `INT xx` for BIOS services. For most services, POST
points IVT vectors at ROM stubs shaped `HLT; IRET`:

1. Guest executes `INT imm8` architecturally; CS:IP lands on the ROM stub.
2. The stub's `HLT` is treated by Tier-0 as the VM-exit
   `BatchExit::BiosInterrupt(vector)` (dispatched in
   `crates/aero-machine/src/lib.rs`, verified at `:13057`).
3. The machine calls `Bios::dispatch_interrupt(vector, …)`
   (`crates/firmware/src/bios/interrupts.rs`; POST in `post.rs`).
4. Guest resumes at the stub's `IRET`.

This keeps the CPU core generic (JIT-friendly) while BIOS services run in
Rust.

INT 10h has one deliberate hybrid exception. Its ROM entry tests `AH`:
non-VBE services retain the `HLT; IRET` host-HLE path, while `AH=4Fh` jumps to
a real 16-bit ROM implementation of VBE `4F00`, `4F01`, `4F02`, `4F03`,
`4F06`, and `4F10`. This is required after an operating system takes over.
Win7's `videoprt!VideoPortInt10` calls `hal!x86BiosCall`; HAL copies the first
MiB and software-interprets the ROM. Aero's private `HLT` VM exit has no
meaning in that interpreter, so an HLE-only VBE stub makes every late
`VideoPortInt10` request fail even though boot-time calls made by Aero's own
CPU work.

The ROM's immutable mode-info templates are generated from the same
`VbeDevice` mode definitions as host HLE. The final `PhysBasePtr` cannot be
frozen into them because PCI assigns the VGA/AeroGPU aperture after ROM
mapping. Firmware therefore publishes the final 32-bit LFB address at EBDA
physical `0x9F040`; the ROM overwrites mode-info offset 40 from that value.
`aero-machine` republishes it after PCI display wiring and again in snapshot
`post_restore`, after RAM restoration, so old snapshots cannot erase it.

The interpreted handler changes the hardware DISPI register file but cannot
update Rust-only `Bios::video.vbe.current_mode`. This distinction matters for
old snapshots that preserve an earlier HLE boot mode. Direct DISPI writes mark
the register file guest-owned; from that point its state is authoritative for
legacy presentation and shared scanout publication, even if the restored BIOS
cache still names a different mode. Before guest ownership, BIOS state remains
the source of the mirrored DISPI defaults.

The device also implements the Bochs/QEMU enable-edge rule used by that ROM
sequence. Transitioning register 4 from disabled to enabled resets virtual
width and x/y offsets, derives virtual height from the available framebuffer,
and clears the new visible surface unless `NOCLEARMEM` is set. This prevents
an earlier BIOS mode's virtual pitch from surviving a later interpreted-ROM
mode set.

### Boot flow and drive numbers

`Machine::reset()` constructs the BIOS, maps the ROM, and runs
`Bios::post_with_pci(...)`, which boots per `BiosConfig::boot_drive`:

- HDD/floppy boot: LBA0 → `0000:7C00`, jump there.
- CD boot (`boot_drive` in `0xE0..=0xEF`): parse the El Torito boot catalog
  and load the no-emulation boot image to `load_segment:0000`.

Drive-number conventions (canonical Win7 topology can service both an AHCI HDD
and an IDE/ATAPI CD simultaneously):

| Device | `DL` | INT 13h sector size |
|---|---:|---:|
| HDD0 | `0x80` | 512 bytes |
| CD0 | `0xE0` | 2048 bytes (via EDD) |

Sector units by drive class: EDD DAP `lba`/`count` are 512-byte sectors for
`DL=0x80..=0xDF` and 2048-byte ISO logical blocks for `DL=0xE0..=0xEF`. The
BIOS interrupt entrypoint takes a 512-byte `BlockDevice` backend for HDDs plus
an optional 2048-byte `CdromDevice` backend for CDs (with an internal
512↔2048 conversion fallback when only raw ISO bytes are exposed).

Boot-selection API (verified in `crates/aero-machine/src/lib.rs` and
`crates/firmware/src/bios/mod.rs`):

- `MachineConfig::boot_drive` / `Machine::set_boot_drive(...)` (before reset);
  `MachineConfig::win7_install_defaults(...)`.
- Optional "CD-first when present" policy
  (`BiosConfig::boot_from_cd_if_present`, `cd_boot_drive`); the configured HDD
  boot drive remains the fallback.
- Convenience: `Machine::configure_win7_install_boot(iso)` /
  `Machine::new_with_win7_install(...)`.
- What firmware actually booted from: `Bios::booted_from_cdrom()` /
  `Machine::active_boot_device()`.

### SMP boot (BSP + APs) — bring-up only

`MachineConfig::cpu_count >= 1` publishes CPU topology via ACPI MADT + SMBIOS.
`aero_machine::Machine` has basic SMP plumbing (per-vCPU LAPIC state/MMIO,
INIT/SIPI AP bring-up, a cooperative bounded AP run loop in
`Machine::run_slice`) but it is **not** a full SMP scheduler — no parallel
execution, many OS-level SMP paths untested. Tests:
`crates/aero-machine/tests/ap_tsc_sipi_sync.rs`,
`lapic_mmio_per_vcpu.rs`, `ioapic_routes_to_apic1.rs`.

### Firmware fixtures

Deterministic in-repo blobs, regenerated/checked with `cargo xtask fixtures
[--check]` (or `cargo xtask bios-rom [--check]` for just the ROM):

- `assets/bios.bin` — BIOS ROM image from `build_bios_rom()`.
- `crates/firmware/acpi/dsdt.aml` — DSDT with legacy PCI root bridge (no
  ECAM); `cargo run -p firmware --bin gen_dsdt --locked` regenerates only this
  one.
- `crates/firmware/acpi/dsdt_pcie.aml` — DSDT with PCIe ECAM/MMCONFIG, matching
  the canonical PC platform (BIOS also publishes an `MCFG` table).
- Human-readable `dsdt*.asl` references alongside; `scripts/verify_dsdt.sh`
  keeps ASL/AML in sync.

## El Torito CD boot and INT 13h extensions

Minimal behavior needed to boot Windows 7 install media: no-emulation El
Torito only (no floppy/HDD emulation modes, no boot menus, no UEFI).
Implementation: `crates/firmware/src/bios/eltorito.rs` +
`crates/firmware/src/bios/interrupts.rs::handle_int13`.

Unit discipline (the classic foot-gun): ISO9660/El Torito LBAs are 2048-byte
blocks; the underlying `BlockDevice` backend is 512-byte sectors, so
`lba512 = lba2048 * 4`. Volume descriptors start at ISO LBA 16. Boot-catalog
and boot-image LBAs are 2048-byte units, but the boot entry's `sector_count`
is already in 512-byte units.

Boot sequence implemented by the BIOS:

1. Scan ISO9660 volume descriptors from ISO LBA 16 (validate `"CD001"` +
  version `0x01`), stopping at the set terminator (`0xFF`), end of image, or a
  bound of `MAX_VOLUME_DESCRIPTOR_SCAN = 128` blocks (verified in
  `eltorito.rs`). No boot record ⇒ not BIOS-bootable via El Torito.
2. Boot Record Volume Descriptor: type `0x00`,
  `boot_system_id = "EL TORITO SPECIFICATION"`, boot catalog pointer at offset
  `0x47` (u32 LE, ISO LBA).
3. Boot catalog: validate the Validation Entry (header `0x01`, key bytes
  `0x55 0xAA`, and the 16-word u16 checksum summing to 0 mod 0x10000), then
  scan up to 4 ISO blocks of 32-byte entries for the first bootable
  (`0x88`), no-emulation (`0x00`), x86-BIOS entry (section headers
  `0x90`/`0x91` track platform).
4. Load the boot image: `load_segment` defaults to `0x07C0` when 0;
   `sector_count` defaults to 4 (2048 bytes) when 0; read
   `sector_count * 512` bytes from `load_rba * 4` to `load_segment << 4`;
   enter at `CS:IP = load_segment:0000` with `DL = boot drive`,
   `DS=ES=SS=0`, `SP=0x7C00`.

INT 13h surface for CD drive numbers (`0xE0..=0xEF`): `AH=00h`/`01h`
supported; `AH=03h`/`05h`/`43h` return write-protected (`CF=1`, `AH=03h`);
`AH=15h` reports the type and sector count in 2048-byte units; `AH=41h`
reports EDD 3.0 with `42h`+`48h` support; `AH=42h` extended read (16- and
24-byte DAPs; for CDs, LBA/count in 2048-byte units); `AH=48h` reports
`bytes_per_sector = 2048`; `AH=4Bh` implements only `AX=4B00h` (no-op success
for no-emulation) and `AX=4B01h` (0x13-byte emulation status packet, only when
actually booted via El Torito). Everything else returns `CF=1`, `AH=01h`.
Aero does not consume `-boot-info-table`.

## PCI device identity and IRQ routing

Canonical PCI identity registry: `crates/devices/src/pci/profile.rs`,
validated by `crates/devices/tests/pci_profile.rs` (which also has a
`pci_dump` helper for eyeballing regressions). Stable BDFs on bus 0 (verified
against the profiles):

| BDF | Device | Vendor:Device | Class | Notes |
|-----|--------|---------------|-------|-------|
| 00:01.0 | ISA bridge | 8086:7000 | 06/01/00 | PIIX3-compatible; `header_type=0x80` exposes 00:01.1/.2 |
| 00:01.1 | IDE | 8086:7010 | 01/01/80 | PIIX3 IDE, legacy-compat + bus master; data IRQs are ISA IRQ14/15. **`80`, not `8A`** — see storage.md |
| 00:01.2 | UHCI | 8086:7020 | 0C/03/00 | USB 1.1 |
| 00:02.0 | AHCI | 8086:2922 | 01/06/01 | SATA; ABAR 8 KiB (`AHCI_ABAR_SIZE`) |
| 00:03.0 | NVMe | 1B36:0010 | 01/08/02 | optional |
| 00:04.0 | HDA | 8086:2668 | 04/03/00 | Intel HD Audio; 16 KiB MMIO BAR |
| 00:05.0 | E1000 | 8086:100E | 02/00/00 | 82540EM |
| 00:06.0 | RTL8139 | 10EC:8139 | 02/00/00 | alternate NIC |
| 00:07.0 | AeroGPU | A3A0:0001 | 03/00/00 | WDDM target `PCI\VEN_A3A0&DEV_0001`; BAR0 64 KiB regs + BAR1 prefetchable VRAM; reserved BDF |
| 00:08.0 | virtio-net | 1AF4:1041 | 02/00/00 | modern-only, REV_01 |
| 00:09.0 | virtio-blk | 1AF4:1042 | 01/00/00 | modern-only, REV_01 |
| 00:0A.0/.1 | virtio-input | 1AF4:1052 | 09/80/00 | keyboard (fn 0, `header_type=0x80`, SUBSYS_00101AF4) + mouse (fn 1, SUBSYS_00111AF4), REV_01 |
| 00:0B.0 | virtio-snd | 1AF4:1059 | 04/01/00 | modern-only, SUBSYS_00191AF4, REV_01 |
| 00:0c.0 | VGA stub | 1234:1111 | 03/00/00 | Bochs/QEMU "Standard VGA" stub (`VGA_TRANSITIONAL_STUB`); only when `enable_vga=true` + PC platform; must be absent when AeroGPU is enabled |
| 00:0d.0 | xHCI | 1B36:000D | 0C/03/30 | QEMU identity; optional/experimental, no Win7 in-box driver |
| 00:12.0 | EHCI | 8086:293A | 0C/03/20 | ICH9-family; Win7 in-box `usbehci.sys` |

Display is mutually exclusive: `MachineConfig::enable_aerogpu=true` exposes
AeroGPU at 00:07.0 (BAR1-backed VRAM + permissive legacy VGA decode for boot
display; BAR1 is outside the WDDM memory model); `enable_vga=true` exposes the
standalone `aero_gpu_vga` VGA/VBE model, routing the VBE LFB through the VGA
stub's BAR0 under the PC platform. AeroGPU-owned VBE reports
`PhysBasePtr = BAR1_BASE + 0x40000` (`AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES`).

PCI firmware command policy is BAR-derived (`IO` for I/O BARs, `MEM` for
MMIO BARs) with one architectural exception: a VGA-compatible class `03/00`
function also receives command `IO`, because its fixed legacy VGA and VBE
DISPI port ranges are not described by an I/O BAR. This matches the working
QEMU standard-VGA command state. It is also directly required by Win7:
`vgapnp.sys!VgaFindAdapter` calls `VideoPortGetVgaStatus`, whose PCI path
reads config space and rejects the adapter unless `COMMAND.IO=1` and the
class is VGA-compatible before mapping port `0x3da`. The miniport's data
also includes fixed DISPI range `0x1ce..0x1cf`; it does not import the
previously attributed `VideoPortVerifyAccessRanges`.
Regression:
`bios_post_enables_fixed_legacy_io_decode_for_vga_compatible_controllers`.

Virtio IDs use the modern space (`0x1040 + device_id`); transitional IDs
(1AF4:1000/1001/1011) are historical context only, not part of Aero contract
v1.

Stability requirements for driver binding: vendor/device IDs, class codes,
`header_type` multi-function bits, BAR types/sizes, and virtio vendor
capabilities must stay exactly as above.

### IRQ routing

PCI INTx is level-triggered, routed via PIRQ A–D:

- Swizzle: `PIRQ = (INTx + device_number) mod 4` (INTA=0); all canonical
  devices use INTA, so PIRQ = `device_number mod 4`.
- Q35 APIC mapping: PIRQ A→GSI20, B→GSI21, C→GSI22, D→GSI23. These
  are the root-bus E–H link routes published by the canonical DSDT and keep
  PCI level interrupts out of the ISA IRQ space (in particular, away from
  i8042 mouse IRQ12).
- PIC-mode compatibility mapping: PCI GSIs 20–23 mirror to PIC IRQs 10–13.
  PCI config-space `Interrupt Line` therefore remains 10–13 while the
  IOAPIC-facing route is 20–23.
- Caveat: "legacy" PCI functions can still use ISA IRQs — PIIX3 IDE delivers
  ATA/ATAPI completion on IRQ14/IRQ15 regardless of its INTx config fields.

## IRQ semantics (browser runtime wire contract)

Applies to `vmRuntime=legacy`, where the IO worker owns device models and
signals the CPU worker over shared IPC events (`irqRaise`/`irqLower`). In
`vmRuntime=machine`, IRQ delivery happens inside `api.Machine`.

Contract (implementation `apps/web/src/io/irq_refcount.ts`, tests
`apps/web/src/io/irq_refcount.test.ts`, sink type `apps/web/src/io/device_manager.ts`):

- `raiseIrq(irq)` asserts a line; `lowerIrq(irq)` deasserts it. These are wire
  levels, not "deliver now" events.
- Shared lines are refcounted wire-OR: level is asserted while refcount > 0.
  Underflow (extra lowers) is ignored with a dev warning; refcounts saturate
  at `0xFFFF`.
- Edge-triggered sources (i8042 IRQ1/IRQ12) must pulse: `raiseIrq` then
  `lowerIrq` (same turn is fine). A pulse on an already-asserted line is
  naturally suppressed, matching hardware. The WASM
  `crates/aero-wasm::I8042Bridge::drain_irqs()` returns a bitmask of pending
  pulses (bit0=IRQ1, bit1=IRQ12) so pulses are not lost when the output buffer
  refills immediately after a port 0x60 read — verified in
  `crates/aero-wasm/src/i8042_bridge.rs`.
- Level-triggered sources (PCI INTx: UHCI/EHCI/xHCI, etc.) hold the line while
  the condition is pending.
- The current CPU worker publishes a level bitmap for observability only; a
  faithful PIC/APIC model must latch rising edges for edge-triggered inputs
  [aspirational for the browser runtime].

## Snapshots (save-state / restore-state)

Format and machinery: `crates/aero-snapshot`; canonical-machine integration
tests in `crates/aero-machine/tests/` (e.g. `bios_post_checkpoint.rs`).

### File format v1

- Header: magic `AEROSNAP` (8 bytes), u16 format version (`1`), u8 endianness
  tag (`1` = little-endian), reserved. All integers little-endian.
- TLV sections: `u32 section_id`, `u16 section_version`, `u16 flags`,
  `u64 section_len`, payload. Unknown sections skip via `section_len`;
  per-section versions allow targeted evolution.
- Section IDs (`crates/aero-snapshot/src/format.rs`, verified): `META=1`,
  `CPU=2`, `MMU=3`, `DEVICES=4`, `DISKS=5`, `RAM=6`, `CPUS=7`, `MMUS=8`.
- Deterministic encoding: canonical ordering for `DEVICES`
  (`(device_id, version, flags)`), `DISKS` (`disk_id`), `CPUS`/`MMUS`
  (`apic_id`), sorted/deduped dirty-page lists; restore re-canonicalizes
  before applying.
- Corruption handling: duplicate core sections, mixed `CPU`+`CPUS` or
  `MMU`+`MMUS`, duplicate entries in canonical lists, and non-increasing dirty
  page lists are all rejected as corrupt.
- Hard restore-time limits live in `crates/aero-snapshot/src/limits.rs`
  (shared by the library and xtask tooling): max 256 CPUs, 4096 device
  entries, 256 MiB DEVICES payload, 64 MiB per device entry / vCPU blob,
  2 MiB RAM page, 64 MiB RAM chunk, etc. `save_snapshot` enforces the same
  bounds.

### Section contents

- `META`: snapshot id, parent id, timestamp, optional label.
- `CPU`/`CPUS`: v1 = minimal legacy (GPRs/RIP/RFLAGS/selectors/XMM); v2 =
  `aero_cpu_core::state::CpuState` (full segments, x87, SSE, 512-byte FXSAVE
  image, plus an extension block with A20 gate state and the pending
  BIOS-hypercall vector for the `HLT; IRET` stub mechanism).
- `MMU` (legacy single-vCPU) / `MMUS` (per-vCPU keyed by `apic_id`): v2 covers
  CRs, DRs, key MSRs (EFER, STAR/LSTAR/CSTAR/SFMASK, SYSENTER, FS/GS base,
  APIC_BASE, TSC), GDTR/IDTR, LDTR/TR. Restore should re-seed the core's
  virtual-time source from `TSC`.
- `DEVICES`: typed TLV device states (registry below).
- `DISKS`: `DiskOverlayRef { disk_id, base_image, overlay_image }` — opaque
  host identifiers (empty string = not configured). Canonical Win7 mapping:
  `disk_id 0` = AHCI port 0 HDD, `1` = IDE secondary master ATAPI CD, `2` =
  optional IDE primary master ATA. Guard test:
  `crates/aero-machine/tests/machine_disk_overlays_snapshot.rs`.
- `RAM`: chunked (1 MiB default) with optional LZ4 compression, or dirty-page
  diffs. Dirty snapshots require the RAM `page_size` to match the VM's dirty
  tracking granularity, are not standalone, and must be applied on top of
  their parent (`RestoreOptions.expected_parent_snapshot_id`; non-seekable
  readers additionally require `META` before `RAM` — use
  `restore_snapshot_checked`).

### Device payload convention and the `DeviceId` registry

Convention: `DeviceState.id` = outer `DeviceId`; `DeviceState.data` = the raw
`aero-io-snapshot` TLV blob; `DeviceState.version`/`flags` mirror the inner
device `SnapshotVersion (major, minor)`. Opt-in helpers behind the
`io-snapshot` feature: `aero_snapshot::io_snapshot_bridge`.

Outer IDs (verified in `crates/aero-snapshot/src/format.rs`) with web-runtime
`kind` strings (mapping in `apps/web/src/workers/vm_snapshot_wasm.ts`, mirrored in
`crates/aero-wasm/src/vm_snapshot_device_kind.rs`):

| ID | Name | Web `kind` | Inner TLV / notes |
|---:|---|---|---|
| 1 | `PIC` | `device.1` | legacy PIC |
| 2 | `APIC` | `device.2` | legacy id for the platform interrupt complex (`INTR`) |
| 3 | `PIT` | `device.3` | 8254 (`PIT4`) |
| 4 | `RTC` | `device.4` | RTC/CMOS (`RTCC`) |
| 5 | `PCI` | `device.5` | legacy PCI core state; prefer 14+15 |
| 6 | `DISK_CONTROLLER` | `device.6` | single `DSKC` wrapper nesting per-BDF controller snapshots (`AHCP`, `VPCI`, `NVMP`, `IDE0`) |
| 7 | `VGA` | `device.7` | VGA/VESA model |
| 8 | `SERIAL` | `device.8` | UART 16550 |
| 9 | `CPU_INTERNAL` | `device.9` | pending external interrupts + interrupt-inhibit (v2) |
| 10 | `BIOS` | `device.10` | firmware runtime state |
| 11 | `MEMORY` | `device.11` | memory-bus glue (A20, ROM windows) |
| 12 | `USB` | `usb` | one entry max; multi-controller via `"AUSB"` container (`apps/web/src/workers/usb_snapshot_container.ts`); plain UHCI blob when UHCI-only |
| 13 | `I8042` | `input.i8042` | `8042`; includes output-port A20 bit |
| 14 | `PCI_CFG` | `device.14` | `PciConfigPorts` (`PCPT`) |
| 15 | `PCI_INTX_ROUTER` | `device.15` | `PciIntxRouter` (`INTX`) |
| 16 | `ACPI_PM` | `device.16` | PM1 + PM timer (`ACPM`) |
| 17 | `HPET` | `device.17` | `HPET` |
| 18 | `HDA` | `audio.hda` | `HDA0` — see `wiki/areas/audio.md` |
| 19 | `E1000` | `net.e1000` | `E1K0` |
| 20 | `NET_STACK` | `net.stack` | `NETS` (legacy `NETL`) |
| 21 | `PLATFORM_INTERRUPTS` | `device.21` | preferred id for the interrupt complex (`INTR`) |
| 22 | `VIRTIO_SND` | `audio.virtio_snd` | web inner `VSND` |
| 23 | `VIRTIO_NET` | `net.virtio_net` | `VPCI` |
| 24 | `VIRTIO_INPUT` | `input.virtio_input` | multi-function keyboard+mouse (`VINP`) |
| 25 | `AEROGPU` | `gpu.aerogpu` | AeroGPU device state |
| 26/27 | `VIRTIO_INPUT_KEYBOARD` / `_MOUSE` | — | split functions (in code; not in the old doc table) |
| 28 | `GPU_VRAM` | `gpu.vram` | web BAR1 VRAM backing; may be chunked with `flags = chunk_index`; IO worker applies locally |
| 29 | `VIRTIO_INPUT_TABLET` | — | in code |

Unknown IDs use the fallback spelling `device.<decimal u32>` (ASCII digits
only). Reserved web-only host-state ids: `device.1000000000`
(`RuntimeDiskWorker` persistence state) and `device.1000000001+` (legacy
IO-worker VRAM chunks, superseded by `gpu.vram`).

### Restore notes that bite

- `DEVICES` rejects duplicate `(DeviceId, version, flags)` tuples — this is
  why PCI core state splits into `PCI_CFG` + `PCI_INTX_ROUTER` and why storage
  controllers nest under one `DSKC` wrapper keyed by packed BDF
  (`aero_devices::pci::PciBdf::pack_u16`).
- After restoring `PciIntxRouter` **and** the interrupt controller, call
  `PciIntxRouter::sync_levels_to_sink()`; same for `Hpet::sync_levels_to_sink()`
  (per-timer `irq_asserted` is intentionally not snapshotted). Both verified
  in `crates/devices/src/pci/irq_router.rs` and `crates/devices/src/hpet.rs`.
- `PM_TMR` must derive from deterministic platform time (`Clock::now_ns()` /
  shared `ManualClock`), never host wall-clock, or restores diverge. Register
  reads are pure samples: they must not advance a device-private clock.
  Poll-loop acceleration, when enabled for native bring-up, advances the CPU
  TSC and the shared platform clock by the same quantum before the next read.
- Storage controllers drop host backends on `load_state()` (AHCI ports clear
  drives; ATAPI devices restore as placeholder CD-ROMs); the host must
  re-open and re-attach disks per the `DISKS` refs and the stable `disk_id`
  mapping before resuming. Guest-visible "disc present" is snapshotted
  independently of host ISO attachment.
- i8042 IRQ1/IRQ12 are edge-triggered: the i8042 snapshot must not encode IRQ
  levels or emit spurious pulses from buffered bytes on restore.
- Web runtime ordering: pause CPU → I/O → NET, clear `NET_TX`/`NET_RX` rings
  (not part of the snapshot), save/restore, resume CPU+I/O then NET; tunnels
  reconnect best-effort.
- Audio specifics (ring indices vs contents, Web Audio graph recreation):
  `wiki/areas/audio.md`.

### Tooling and browser persistence

- `cargo xtask snapshot inspect|validate [--deep] <file>` — header/section
  dump and validation without (or with, ≤512 MiB RAM) decompression.
- Streaming to OPFS: `crates/aero-opfs` (`OpfsSyncFile`, a
  `Read`/`Write`/`Seek` wrapper over `FileSystemSyncAccessHandle`);
  `crates/aero-wasm` exposes `snapshot_full_to_opfs` /
  `snapshot_dirty_to_opfs` / `restore_snapshot_from_opfs` for `Machine` and
  `DemoVm` (verified in `crates/aero-wasm/src/`).
- A `proptest` decoder robustness test lives in `crates/aero-snapshot`.

## Debugging and introspection

- Serial console: 16550 UART, COM1–COM4 (`0x3F8`/4, `0x2F8`/3, `0x3E8`/4,
  `0x2E8`/3). TX bytes reach the main thread as the binary IPC event
  `aero_ipc::protocol::Event::SerialOutput { port, data }` (tag `0x1500`,
  verified in `crates/aero-ipc/src/protocol.rs`). Owner: IO worker
  (`vmRuntime=legacy`) or machine CPU worker (`vmRuntime=machine`).
- DebugCon (port `0xE9`): enabled by default via
  `MachineConfig::enable_debugcon`; host reads via
  `Machine::take_debugcon_output()` / `debugcon_output_len()`, and in WASM
  `Machine.debugcon_output()` / `debugcon_output_len()` (all verified).
- Native CLI runner (`crates/aero-machine-cli`): runs the canonical machine
  headless for boot debugging — `--disk`, `--install-iso`, `--boot
  hdd|cdrom|cd-first`, `--serial-out stdout`, `--debugcon-out stdout`,
  `--snapshot-save/--snapshot-load`, `--vga-png`, copy-on-write
  `--disk-overlay`. Wrap invocations in `bash ./scripts/safe-run.sh`.
- Breakpoints/stepping/tracing: `crates/aero-debug` (`Debugger` with
  `check_before_exec` / `check_after_exec` / `check_watchpoint`; `Tracer` with
  type filters, sampling, bounded buffering, JSON export).
- Browser automation debug API: `window.aero.debug.*` (implementation
  `apps/web/src/runtime/boot_device_backend.ts`, types `shared/aero_api.ts`) —
  `getBootDisks()`, `getMachineCpuActiveBootDevice()`,
  `getMachineCpuBootConfig()` (distinguishes requested boot policy from what
  firmware actually booted).
- `apps/web/debug.html`: lightweight dev debug UI; hosts forward events to
  `window.aeroDebug.onEvent(...)` and listen for `aero-debug-command`.
- Runtime logs: workers emit structured `log` events on their event ring; the
  coordinator prints `[role]`-prefixed lines and records WARN/ERROR.
- Host-side storage diagnostic (absorbed from `docs/compat-smoke-test.html`;
  IndexedDB/OPFS contract now in `wiki/areas/web-host.md`): a standalone
  manual page (no build step) that self-tests IndexedDB as a sparse block
  store for when OPFS sync access handles are unavailable; it is a diagnostic
  only and does not back the synchronous Rust disk path. Belongs to the
  storage area.

## IPC protocol (AIPC)

Inter-thread IPC between the coordinator, web workers (TypeScript), and
Rust/WASM, built on SharedArrayBuffer + Atomics. Reference implementations:
`crates/aero-ipc/src/{layout,ring,protocol}.rs` (+ `wasm.rs`) and
`apps/web/src/ipc/{layout,ring_buffer,protocol}.ts` (+ `ipc.ts`); demo
`apps/web/demo/ipc_demo.html`.

- Topology: SPSC queue pairs per worker (`cmd[i]` coordinator→worker,
  `evt[i]` worker→coordinator); the ring primitive itself is MPSC-safe via a
  reservation + in-order commit scheme.
- Shared buffer header at offset 0: `magic = 0x4350_4941` ("AIPC", verified),
  `version = 1`, `total_bytes`, `queue_count`, then 16-byte queue descriptors
  (`kind`, `offset_bytes`, `capacity_bytes`, reserved).
- Ring header: `Int32Array[4]` = `head`, `tail_reserve`, `tail_commit`,
  `capacity`; cursors are wrapping u32 driven by `Atomics.*`.
- Records: `u32 payload_len` + payload + padding to 4-byte alignment;
  wrap marker `0xFFFF_FFFF`; records never split across the ring end.
- Blocking: consumer waits on `tail_commit`, producers may wait on `head`
  (workers only; the main thread polls or uses `Atomics.waitAsync`).
- Queue kinds for the browser `ioIpcSab` segment
  (`apps/web/src/runtime/shared_layout.ts:createIoIpcSab()`, mirrored in
  `aero_ipc::layout::io_ipc_queue_kind`, verified): `CMD=0`, `EVT=1`,
  `NET_TX=2`, `NET_RX=3`, `HID_IN=4`. In `vmRuntime=machine` only
  `NET_TX`/`NET_RX` are used (raw Ethernet frames, one per record, ≤2048
  bytes, drop-on-full); `HID_IN` carries WebHID reports (`"HIDR"` record
  format, `apps/web/src/hid/hid_input_report_ring.ts`) and is legacy-runtime only.
- Message tags (16-bit): commands `Nop 0x0000`, `Shutdown 0x0001`,
  `MmioRead/Write 0x0100/0x0101`, `PortRead/Write 0x0102/0x0103`,
  `DiskRead/Write 0x0104/0x0105`; events `Ack 0x1000`, `MmioReadResp 0x1100`,
  `PortReadResp 0x1101`, `MmioWriteResp 0x1102`, `PortWriteResp 0x1103`,
  `DiskReadResp/DiskWriteResp 0x1104/0x1105`, `FrameReady 0x1200`,
  `IrqRaise/IrqLower 0x1300/0x1301`, `A20Set 0x1302`, `ResetRequest 0x1303`,
  `Log 0x1400`, `SerialOutput 0x1500`, `Panic 0x1FFE`, `TripleFault 0x1FFF`.
- `DiskRead`/`DiskWrite` `guest_offset` is a guest physical address: with the
  PC/Q35 layout, guest RAM is non-contiguous above the ECAM base
  (`0xB000_0000`), so GPAs must be translated before indexing flat backing
  stores.

## Open issues / debt

- SMP is bring-up only: topology enumeration works, but there is no parallel
  multi-vCPU scheduler; the relationship between `aero-machine` SMP plumbing
  and the standalone `crates/aero-smp` model is unresolved.
- `crates/aero-pc-platform` is expected to fold into `aero-machine` as the
  canonical machine grows [aspirational].
- AeroGPU has two integration surfaces — the canonical machine glue and
  `crates/aero-devices-gpu` — which are not yet consolidated. The software
  rasterizer in `crates/aero-gpu-software` is a third executor that no backend
  trait currently reaches; see the retirement account below for what bridging
  it would take.
- The browser runtime's IRQ level bitmap is observability-only; a latching
  PIC/APIC model for the worker runtime is future work [aspirational].
- **Adopting SeaBIOS is held in reserve as a way to delete a variable.** The
  firmware here is home-grown: `assets/bios.bin` is a 64 KiB ROM generated
  in-tree. Windows is exacting about firmware, and bring-up debugs a
  from-scratch CPU, a from-scratch BIOS and from-scratch device models against
  it simultaneously — the earliest boot failures were inside the ROM, before the
  boot sector was reached, and one long-lived wall was literally `0xc0000225`
  ("BIOS is not ACPI compatible"). SeaBIOS is what QEMU boots the same media
  with, so adopting it would remove a whole category of divergence and sharpen
  the QEMU comparison: with identical firmware on both sides, any remaining
  divergence is unambiguously Aero's. It is the change that most contradicts the
  existing design, so it is worth holding ready rather than doing pre-emptively
  — but if the milestone ladder stalls on firmware again, take it.

## Pointers

- `wiki/areas/audio.md` — audio devices, rings, and audio snapshot kinds.
- Decisions: `wiki/decisions/0008-canonical-vm-core.md`,
  `wiki/decisions/0014-canonical-machine-stack.md`,
  `wiki/decisions/0015-canonical-usb-stack.md`,
  `wiki/decisions/0010-canonical-audio-stack.md`.
- Cross-area material from the purged `docs/` (now living in the area
  pages): canonical storage topology + `disk_id` mapping, trait
  consolidation, IndexedDB storage story → `wiki/areas/storage.md`; SMP gap
  list → `wiki/areas/cpu-and-jit.md`; USB EHCI/xHCI notes →
  `wiki/areas/usb-and-input.md`; AeroGPU PCI identity + VGA/VESA compat →
  `wiki/areas/graphics.md`; PCAPNG tracing → `wiki/areas/networking.md`;
  testing policy → `wiki/areas/testing.md`.

*Absorbed from `platform-and-firmware.md`, `platform-and-firmware.md`,
`platform-and-firmware.md`, `platform-and-firmware.md`,
`debugging.md`, `../specs/snapshot-format.md`,
`platform-and-firmware.md`, `../state/repo-state-and-structure.md`,
`../specs/worker-ipc-protocol.md`, and `docs/compat-smoke-test.html` (2026-07); paths and
constants verified against the tree at absorption time.*

## Device-model fixes applied (2026-07-26/27)

See `wiki/notes/bring-up-findings-divergence-and-sight-audits.md` §6 for details.

- **ACPI FADT**: set WBINVD (bit 0) + PROC_C1 (bit 2) flags (were both clear —
  ACPI spec violation). The original follow-up added MADT ISO overrides for
  PCI INTx on GSIs 10–13; the Q35 correction supersedes that layout:
  root-bus PCI INTx uses GSIs 20–23, which are natively active-low/level,
  while ISA IRQs 1–15 remain active-high/edge except SCI9.
- **MSR**: `AERO_IGNORE_UNKNOWN_MSRS=1` env-gate (default off; read-0/write-noop,
  mirroring QEMU/KVM IGNORE_MSRS=1); added thermal stubs (0x1A2, 0x1A4-0x1A6) +
  EBC_FREQUENCY_ID (0x2A) read=0.
- **Snapshot**: IA32_TSC_AUX (0xC0000103) added to snapshot save/restore (was
  silently zeroed → RDTSCP returns ECX=0 after restore → per-CPU ID divergence).
- **Serial**: THRE interrupt now generated (was only RDA); default MSR to
  DCD/DSR/CTS active (was 0 → modem-aware drivers stall).

## BIOS & Firmware

### What exists today (canonical stack)

Aero’s canonical boot path is **legacy BIOS**, implemented in Rust as **HLE firmware**:

- **BIOS implementation:** `crates/firmware::bios`
  - Builds a 64 KiB ROM image (`build_bios_rom()`) containing interrupt stubs.
  - Implements POST + a minimal INT service surface (video, disk, E820, keyboard, time, …).
- **Machine integration / wiring:** `crates/aero-machine` (see [Canonical machine stack](../decisions/0014-canonical-machine-stack.md))
  - Maps the BIOS ROM, runs POST, and dispatches BIOS interrupt “hypercalls”.
- **ACPI tables:** generated by `crates/aero-acpi` (used by `firmware::bios` during POST).

For CD-ROM boot (El Torito) and the minimal INT 13h extensions required for Windows install media,
see [`platform-and-firmware.md`](platform-and-firmware.md).

#### INT 13h expectations (HDD/floppy vs CD-ROM)

BIOS disk I/O has two relevant sector sizes:

- **HDD/floppy:** 512-byte sectors (traditional INT 13h semantics).
- **CD-ROM:** 2048-byte logical blocks (ISO9660 / El Torito).

For CD boot and CD reads, Aero’s BIOS expects **El Torito (no-emulation)** boot and requires **INT
13h Extensions (EDD)** for CD drive numbers (recommend `DL=0xE0` for the first CD). At minimum,
Windows-style bootloaders expect `AH=41h` (extensions check), `AH=42h` (extended read), and `AH=48h`
(extended drive parameters) to work for CD, with `AH=48h` reporting `bytes_per_sector = 2048`.
Legacy CHS reads like `AH=02h` are HDD/floppy-oriented and are **not required** for CD media (as long
as the EDD path works).

See [`platform-and-firmware.md`](platform-and-firmware.md) for the exact structures and call
contracts.

#### Canonical BIOS drive numbers (HDD0 + CD0)

In the canonical `aero_machine::Machine` Win7 storage topology, both an AHCI HDD and an IDE/ATAPI
CD-ROM may be attached simultaneously. Aero’s BIOS INT 13h can service **both** devices when the
corresponding backends are present (HDD0 via `DL=0x80`, CD0 via `DL=0xE0`).
Boot selection is still primarily driven by the boot drive number (`BiosConfig::boot_drive` / `DL`)
used when transferring control to the boot sector / El Torito boot image, with an optional “CD-first
when present” policy flag for convenience.

Drive-number conventions still follow common PC/BIOS ranges:

| Device | BIOS drive number (`DL`) | Sector size exposed via INT 13h | Notes |
|---|---:|---:|---|
| HDD0 (primary disk) | `0x80` | 512 bytes | Traditional HDD semantics. |
| CD0 (install ISO) | `0xE0` | 2048 bytes | Via INT 13h Extensions (EDD); `AH=48h` reports 2048. |

Sector units by drive class:

- For HDD drive numbers (`DL=0x80..=0xDF`), EDD DAP `lba`/`count` are in **512-byte sectors**.
- For CD drive numbers (`DL=0xE0..=0xEF`), EDD DAP `lba`/`count` are in **2048-byte logical blocks**
  (ISO LBAs).

Implementation note: in `firmware::bios`, this is expressed by the BIOS interrupt entrypoint taking
both:

- a 512-byte-sector `BlockDevice` backend (for HDD drive numbers `0x80..=0xDF`), and
- an optional 2048-byte-sector `CdromDevice` backend (for CD drive numbers `0xE0..=0xEF`).

To boot install media, the host must either:

- select CD boot (`boot_drive=0xE0`) before reset, **or**
- enable the optional firmware “CD-first when present” policy (`boot_from_cd_if_present=true`,
  `cd_boot_drive=0xE0`) so BIOS attempts a CD boot when an ISO is attached and otherwise falls back
  to the configured HDD boot drive.

See [`storage.md`](storage.md). When booting install media,
early Windows boot code reads ISO9660 logical blocks from the CD via the EDD path (`AH=42h`/`48h`).

UEFI is **not** the canonical path today. If you see older docs implying an external BIOS blob or a
still-unimplemented firmware stack, treat those as outdated.

### Deterministic firmware fixtures

This repo keeps a handful of **tiny, deterministic, in-repo** firmware blobs for tests and CI:

- `assets/bios.bin` (BIOS ROM image; generated from `firmware::bios::build_bios_rom()`)
- `crates/firmware/acpi/dsdt.aml` (ACPI DSDT AML; legacy PCI root bridge; generated from `aero-acpi`)
- `crates/firmware/acpi/dsdt_pcie.aml` (ACPI DSDT AML; PCIe ECAM/MMCONFIG-enabled variant; generated from `aero-acpi`; matches the canonical PC platform config)

Human-readable references live alongside as `crates/firmware/acpi/dsdt*.asl` and are kept in sync
with the shipped AML blobs (see `scripts/verify_dsdt.sh`).

Regenerate or verify all in-repo fixtures with:

```bash
cargo xtask fixtures [--check]
```

To regenerate/check just the BIOS ROM fixture:

```bash
cargo xtask bios-rom [--check]
```

---

### BIOS dispatch contract (HLT-in-ROM-stub “hypercall”)

The BIOS does **not** rely on the CPU core trapping `INT xx` directly. Instead, BIOS services are
implemented on the host side, and the guest reaches them via tiny real-mode ROM stubs.

#### ROM mapping

`firmware::bios::build_bios_rom()` returns a 64 KiB image that the machine maps at:

- `firmware::bios::BIOS_BASE` (`0x000F_0000`)
- and (optionally/typically) also aliases at `firmware::bios::BIOS_ALIAS_BASE` (`0xFFFF_0000`) so the
  architectural reset vector at `0xFFFF_FFF0` works.

The ROM contains:

- a reset vector FAR JMP, and
- per-interrupt stubs (plus a default handler).

#### Interrupt stub shape

During POST, the BIOS initializes the IVT so that important vectors point into the ROM. Each INT
handler stub is:

```text
HLT
IRET
```

Execution flow:

1. Guest executes `INT imm8` architecturally (push FLAGS/CS/IP, clear IF/TF, load CS:IP from IVT).
2. CS:IP now points at the ROM stub for that vector.
3. Stub executes `HLT`.
4. Tier-0 treats this specific `HLT` (reached from an INT stub) as a VM-exit:
   `BatchExit::BiosInterrupt(vector)`.
5. The machine calls `firmware::bios::Bios::dispatch_interrupt(vector, …)`.
6. Guest resumes at the next instruction in the stub: `IRET`, returning to the original caller.

This keeps the CPU core generic (important for a future JIT), while still implementing BIOS
services in Rust.

**Source of truth:** `crates/firmware/README.md` and `crates/firmware/src/bios/mod.rs` (module docs).

---

### How `aero_machine::Machine` wires BIOS interrupts

`aero_machine::Machine` owns the canonical “BIOS integration loop”:

- On reset, it:
  - constructs a `firmware::bios::Bios`,
  - maps the ROM returned by `build_bios_rom()` into guest physical memory,
  - runs `Bios::post_with_pci(...)`, which performs POST and then loads/jumps to boot code based on
    `firmware::bios::BiosConfig::boot_drive` (configured via `aero_machine::MachineConfig::boot_drive`
    at construction time, or updated via `aero_machine::Machine::set_boot_drive(...)` before reset):
    - **HDD/floppy boot:** reads LBA0 into `0000:7C00` and jumps to `0000:7C00`.
    - **CD-ROM boot:** when `boot_drive` is in `0xE0..=0xEF`, parses the El Torito boot catalog and
      loads the **no-emulation** boot image to `load_segment:0000` (commonly `07C0:0000`), then
      jumps there (see [`storage.md`](storage.md) for the
      canonical Windows 7 install/recovery flow).
- During execution, Tier-0 returns `BatchExit::BiosInterrupt(vector)` when a BIOS stub `HLT` is hit.
  The machine handles it by calling:

  - `Machine::handle_bios_interrupt(vector)` → `bios.dispatch_interrupt(vector, ...)`

In the canonical machine, BIOS INT 13h can expose **both** of the canonical media backends:

- HDD0: `DL=0x80` (512-byte sectors), routed to the machine’s primary HDD backend (also attached to
  AHCI port 0).
- CD0: `DL=0xE0` (2048-byte sectors via INT 13h Extensions/EDD), routed to the machine’s install ISO
  backend when present (also attached to IDE secondary master ATAPI).

Boot selection is still primarily driven by `boot_drive` (the `DL` value firmware uses when
transferring control to the boot sector / El Torito boot image), configured via
`MachineConfig::boot_drive` at construction time or via `Machine::set_boot_drive(...)` +
`Machine::reset()`. For host convenience, firmware also supports an optional “CD-first when present”
policy flag (`firmware::bios::BiosConfig::boot_from_cd_if_present`).

Note: when the “CD-first when present” policy is enabled, the configured `boot_drive` / boot-device
preference remains the **fallback** (typically HDD0, `DL=0x80`) even when the current boot actually
came from CD-ROM. Hosts that need to know what firmware actually booted from should use:

- `firmware::bios::Bios::booted_from_cdrom()`, or
- `aero_machine::Machine::active_boot_device()`.

For CD boots/reads, the BIOS supports two backend shapes:

- Prefer a 2048-byte-sector `CdromDevice` backend when servicing `DL=0xE0..=0xEF`.
- Or use the legacy fallback where the raw ISO bytes are exposed via the 512-byte-sector
  `BlockDevice` interface and the BIOS performs the 2048↔512 conversions internally.

Boot selection note: `Machine` defaults to `boot_drive=0x80`. For install-media boot, callers
should attach an ISO and select CD boot (`boot_drive=0xE0`) either:

- at construction time via `MachineConfig::boot_drive` / `MachineConfig::win7_install_defaults(...)`, or
- at runtime via `Machine::set_boot_drive(0xE0)` **before** invoking `Machine::reset()`.
- or enable the firmware “CD-first when present” policy (`Machine::set_cd_boot_drive(0xE0)`,
  `Machine::set_boot_from_cd_if_present(true)`) while keeping `boot_drive=0x80` as the fallback.
  - Convenience: `Machine::configure_win7_install_boot(iso)` (or `Machine::new_with_win7_install(...)`)
    does the CD-first enable + ISO attach + reset in one call.

Relevant code:

- BIOS interrupt exit handling: `crates/aero-machine/src/lib.rs::handle_bios_interrupt`
- BIOS implementation: `crates/firmware/src/bios/interrupts.rs` (dispatch table) and `post.rs`

Note: some BIOS services are HLE and update guest memory / BIOS internal state without touching
device registers directly. For example, VGA/VBE mode changes are mirrored into the emulated VGA
device by `Machine::handle_bios_interrupt` so the host-visible display updates immediately.

---

### ACPI tables (generated by `aero-acpi`)

ACPI is generated by the `aero-acpi` crate and written into guest RAM during BIOS POST.

High-level contract:

- `firmware::bios` builds ACPI tables via `aero_acpi::AcpiTables::build(...)`.
- Tables are placed **automatically at the top of low RAM**, below the ECAM window,
  the way real PC BIOSes do it: a zero `tables_base`/`nvs_base` in
  `aero_acpi::AcpiPlacement` means auto. The old fixed placement at 1 MiB was
  wrong in a way that took a while to see — winload loads a PE image directly over
  that region. The struct is still configurable via
  `firmware::bios::BiosConfig::acpi_placement`.
- The BIOS also reports the reclaimable + NVS regions so the E820 map can mark them with the correct
  types (ACPI reclaimable vs ACPI NVS).

#### Regenerating the checked-in DSDT fixtures

The runtime uses the **Rust generator**; the repo also keeps checked-in DSDT AML blobs for
validation/diffing:

- Fixtures (used by tests and `scripts/validate-acpi.sh`):
  - `crates/firmware/acpi/dsdt.aml` (legacy PCI root bridge; ECAM/MMCONFIG disabled)
  - `crates/firmware/acpi/dsdt_pcie.aml` (PCIe root bridge; ECAM/MMCONFIG enabled)

Note: the canonical PC platform enables PCIe ECAM/MMCONFIG, so `firmware::bios` typically emits
DSDT content matching `dsdt_pcie.aml` (and also publishes an `MCFG` table describing the ECAM
window). The `dsdt.aml` fixture is kept as a legacy/no-ECAM reference.

Regenerate the repo fixtures (recommended; this also refreshes `assets/bios.bin`):

```bash
cargo xtask fixtures
```

Or regenerate just the legacy `dsdt.aml` fixture directly:

```bash
cargo run -p firmware --bin gen_dsdt --locked
```

Note: `gen_dsdt` only regenerates `dsdt.aml`. To refresh `dsdt_pcie.aml`, use `cargo xtask fixtures`.

---

### Current limitations / known gaps

- **SMP / multi-vCPU execution is still bring-up only (not a full SMP guest yet):**
  `MachineConfig::cpu_count` accepts values `>= 1`. When `cpu_count > 1`, firmware publishes the
  configured CPU topology for **guest enumeration** via **ACPI MADT + SMBIOS**.
  `aero_machine::Machine` includes basic SMP plumbing (per-vCPU LAPIC state/MMIO + INIT/SIPI AP
  bring-up + a cooperative AP run loop inside `Machine::run_slice`), but it is still **not** a full
  SMP scheduler (no parallel execution, limited AP scheduling, many OS-level SMP paths untested).
  See [`cpu-and-jit.md`](cpu-and-jit.md).

---

### SMP boot (BSP + APs)

On x86, application processors (APs) start in a reset/halted state and are brought online by the
bootstrap processor (BSP) using **INIT + SIPI** (via the local APIC ICR).

In Aero today:

- Firmware publishes the CPU topology (MADT + SMBIOS) for `cpu_count >= 1`.
- `aero_machine::Machine` instantiates a BSP (vCPU0) and AP `CpuCore`s and routes LAPIC MMIO
  per-vCPU. The BSP can start APs by writing INIT/SIPI to the LAPIC ICR, and `Machine::run_slice`
  will execute runnable APs in a simple bounded cooperative loop.

Useful references/tests:

- AP INIT/SIPI bring-up smoke test: `crates/aero-machine/tests/ap_tsc_sipi_sync.rs`
- Per-vCPU LAPIC MMIO routing: `crates/aero-machine/tests/lapic_mmio_per_vcpu.rs`
- IOAPIC destination routing to a non-BSP LAPIC: `crates/aero-machine/tests/ioapic_routes_to_apic1.rs`

### Tests to run while iterating on firmware/integration

```bash
# Firmware unit tests (BIOS services, ACPI/SMBIOS publication, ROM layout).
bash ./scripts/safe-run.sh cargo test -p firmware --locked

# Canonical machine integration tests (BIOS POST, devices, interrupts, snapshots).
bash ./scripts/safe-run.sh cargo test -p aero-machine --locked
```

## BIOS CD-ROM Boot (El Torito + INT 13h extensions)

This document captures the **minimal El Torito CD-ROM boot behavior** (plus the **INT 13h
extensions** surface) that Aero relies on to boot **Windows 7 install media**. It is intended to be
precise enough that a future contributor could re-implement the logic from scratch without
regressing Win7.

Scope:

* **El Torito, no-emulation boot only** (what Windows 7 install ISOs use for BIOS boot).
* **ISO9660 Volume Descriptor** scanning to find the El Torito boot catalog.
* **INT 13h extensions** used by Windows-style bootloaders: `AH=41h`, `AH=42h`, `AH=48h`.
* Explicit non-goal: floppy/hard-disk emulation modes, multi-entry boot menus, UEFI boot.

---

### Sector sizes and addressing (2048 vs 512)

El Torito and ISO9660 describe locations in **ISO logical blocks** of **2048 bytes**.

In Aero’s BIOS:

* **Externally (INT 13h for CD drives):** CD-ROM media is exposed as **2048-byte sectors** (and
  `AH=48h` reports `bytes_per_sector = 2048`).
* **Internally:** there are two relevant backends:
  * **El Torito boot/catalog scanning** uses the firmware `BlockDevice` interface (**512-byte
    sectors**) and converts ISO LBAs (`2048-byte sectors`) into 512-byte LBAs.
  * **INT 13h CD-ROM reads** may be backed either by a firmware `CdromDevice` (**2048-byte sectors**)
    or (legacy fallback) by exposing the raw ISO bytes via `BlockDevice` (**512-byte sectors**) and
    letting the BIOS perform the conversion.

When using the 512-byte `BlockDevice` path for ISO media, this creates a critical conversion rule
between **2048-byte LBAs** and the underlying 512-byte sector backend:

* `lba512 = lba2048 * 4` (and similarly `count512 = count2048 * 4`)

You will see this conversion repeatedly:

* ISO9660 volume descriptors start at **ISO LBA 16** → underlying 512-sector LBA `16 * 4 = 64`.
* The Boot Record volume descriptor’s **boot catalog pointer** is an **ISO LBA**.
* The selected boot entry’s **`load_rba`** is an **ISO LBA**.
* The selected boot entry’s **`sector_count`** is in **512-byte sectors** (already the
  underlying unit), even though the catalog’s LBAs are 2048-byte ISO LBAs.

If you forget which fields are 2048-LBA vs 512-sector units, you will load the wrong bytes and
Windows boot will fail very early.

---

### ISO9660 Volume Descriptor scan (starts at ISO LBA 16)

ISO9660 volume descriptors are a contiguous sequence of 2048-byte sectors starting at **ISO LBA 16**
(`0x10`). Aero’s CD boot logic scans them linearly until it finds the **Boot Record Volume
Descriptor** that advertises the El Torito boot catalog.

#### Scan algorithm (conceptual)

1. For `iso_lba = 16..`:
    1. Read 2048 bytes from the CD at `iso_lba`.
       * If your underlying disk interface is 512-byte sectors, this means reading 4 consecutive
         512-byte sectors at `lba512 = iso_lba * 4`.
    2. Validate `vd[1..6] == "CD001"` (standard identifier) and `vd[6] == 0x01` (version).
    3. Dispatch on `vd[0]` (volume descriptor type):
       * `0x00` → **Boot Record VD** (El Torito is identified by a magic string; see below)
       * `0x01` → Primary Volume Descriptor (not used for El Torito discovery)
       * `0x02` → Supplementary (Joliet) (not used for El Torito discovery)
       * `0xFF` → Volume Descriptor Set Terminator (**stop scanning**)
       * other types are ignored
2. If we hit the terminator without finding a valid El Torito Boot Record VD, the ISO is treated as
   **not BIOS-bootable via El Torito**.

Implementation note: Aero bounds this scan for robustness and safety. It stops when it sees the
Volume Descriptor Set Terminator (`0xFF`), encounters a non-ISO9660 descriptor, reaches the end of
the image, **or after a fixed maximum descriptor count** (`MAX_VOLUME_DESCRIPTOR_SCAN`, currently
`128` ISO blocks). If the image ends while scanning, Aero treats it as **missing the El Torito boot
record** rather than surfacing a disk read error.

---

### Boot Record Volume Descriptor (El Torito pointer)

The El Torito boot catalog is located via a special ISO9660 Volume Descriptor: the **Boot Record
Volume Descriptor**.

Required fields:

* `type` = `0x00`
* `standard_id` = `"CD001"`
* `version` = `0x01`
* `boot_system_id` = `"EL TORITO SPECIFICATION"` (ASCII, space-padded)
* `boot_catalog_lba` at offset `0x47` (little-endian `u32`, **ISO 2048-byte LBA**)

#### Boot Record VD layout (2048 bytes)

| Offset | Size | Meaning | Notes |
|---:|---:|---|---|
| `0x00` | 1 | Volume Descriptor Type | Must be `0x00` for Boot Record |
| `0x01` | 5 | Standard Identifier | Must be ASCII `"CD001"` |
| `0x06` | 1 | Version | Usually `0x01` |
| `0x07` | 32 | Boot System Identifier | Must equal `"EL TORITO SPECIFICATION"` (padded) |
| `0x27` | 32 | Boot System Use | Unused by Aero |
| `0x47` | 4 | Boot Catalog Pointer | **ISO LBA (2048-byte)**, little-endian `u32` |
| `0x4B` | … | Reserved | Unused |

Once `boot_catalog_lba` is read, convert to BIOS sectors if needed:

* `boot_catalog_lba512 = boot_catalog_lba2048 * 4`

---

### Boot Catalog (validation entry + boot entry scanning)

The **Boot Catalog** is an array of 32-byte entries (typically stored in 2048-byte ISO blocks).
The first catalog block normally begins with a **Validation Entry** at offset `0x00` (entry #0),
followed by the “initial/default entry” at offset `0x20` (entry #1).

In the Windows install ISO case, the bootable BIOS no-emulation entry is commonly the
initial/default entry, but Aero does **not** assume that; it scans entries for the first usable BIOS
no-emulation boot entry.

Aero’s implemented behavior:

1. Read the catalog starting at `boot_catalog_lba` (2048-byte ISO LBA).
   * We read a bounded prefix (currently **up to 4 ISO blocks**) for safety.
2. Parse entry #0 (offset `0x00`) as the **Validation Entry** and validate it (checksum + key bytes).
3. Scan subsequent 32-byte entries for the first **bootable, no-emulation, BIOS/x86** boot entry:
   * Bootable: `boot_indicator == 0x88`
   * No-emulation: `boot_media_type == 0x00`
   * Platform: **x86 BIOS** (platform id `0`)
   * Section header entries (`0x90`/`0x91`) update the current platform id for subsequent entries.
   * Unknown/extension entries are ignored.

Entries are 32 bytes each.

#### 3.1 Validation Entry (required)

The Validation Entry is a fixed-format 32-byte record that validates the catalog itself.

| Offset | Size | Field | Required value / rule |
|---:|---:|---|---|
| `0x00` | 1 | Header ID | Must be `0x01` |
| `0x01` | 1 | Platform ID | Common values: `0x00` x86 BIOS, `0xEF` EFI. This is the default platform for the initial/default entry; section headers can override it. Aero only boots BIOS/x86 entries. |
| `0x02` | 2 | Reserved | Typically `0x0000` |
| `0x04` | 24 | ID string | Ignored (space padded) |
| `0x1C` | 2 | Checksum | See checksum rule below |
| `0x1E` | 1 | Key byte 1 | Must be `0x55` |
| `0x1F` | 1 | Key byte 2 | Must be `0xAA` |

##### Validation checksum rule

Interpret the 32-byte validation entry as **16 little-endian `u16` words**. The catalog is valid if:

```
sum(words[0..16]) mod 0x10000 == 0
```

Pseudo-code:

```text
sum = 0u16
for i in [0, 2, 4, ..., 30]:
  sum = sum + u16_le(entry[i..i+2])
valid = (sum == 0)
```

If the checksum or the key bytes are wrong, **do not attempt to boot**.

#### 3.2 Boot Entry (initial/default entry on Windows media)

The boot entry specifies the boot image location and how to load it. On Windows install media this
is typically the “Initial/Default Entry”.

| Offset | Size | Field | Notes |
|---:|---:|---|---|
| `0x00` | 1 | Boot Indicator | `0x88` = bootable, `0x00` = not bootable |
| `0x01` | 1 | Boot Media Type | `0x00` = **no emulation** (required for Win7) |
| `0x02` | 2 | Load Segment | `u16` LE. If `0`, BIOS must default to **`0x07C0`** |
| `0x04` | 1 | System Type | Ignored by Aero for no-emulation |
| `0x05` | 1 | Unused | Must be ignored |
| `0x06` | 2 | Sector Count | `u16` LE, **count of 512-byte sectors to load**. If `0`, BIOS must default to **4×512B sectors** (**2048 bytes**) for **no-emulation** boot (per spec; Windows install media relies on this). |
| `0x08` | 4 | Load RBA | `u32` LE, **ISO LBA (2048-byte)** of boot image |
| `0x0C` | 20 | Unused | Must be ignored |

Minimal acceptance rules:

* Bootable (`boot_indicator == 0x88`)
* No-emulation (`boot_media_type == 0x00`)

Other media types (floppy/hdd emulation) are intentionally unsupported in the minimal path. Aero
does **not** implement interactive “boot menus”; if multiple bootable BIOS no-emulation entries
exist, Aero will simply pick the **first** one it finds during the scan described above.

---

### No-emulation boot image loading rules (Win7 critical)

Given the selected boot entry:

1. Compute `load_segment`:
   * If `load_segment != 0`, use it.
   * If `load_segment == 0`, default to **`0x07C0`**.
2. Compute `sector_count`:
   * If `sector_count != 0`, use it.
   * If `sector_count == 0`, default to **4** (2048 bytes total).
3. Compute destination physical address: `dst = load_segment << 4`.
4. Compute how many bytes to read: `bytes_to_load = sector_count * 512`.
5. Convert the boot image start to BIOS LBA:
   * `boot_image_lba512 = load_rba2048 * 4`
6. Read exactly `sector_count` **512-byte** sectors starting at `boot_image_lba512` into memory at
   `dst`.

Important subtlety:

* `sector_count` is in **512-byte units**
* `load_rba` is in **2048-byte units**

This is why the conversion step above matters.

After loading, BIOS transfers control to the boot image at `CS:IP = load_segment:0000` (physical
`dst`), with `DL` set to the BIOS drive number of the boot device (see below).

In Aero, the real-mode register state at entry is:

* `CS = load_segment`, `IP = 0x0000`
* `DL = boot_drive` (typically `0xE0` for the first CD-ROM)
* `DS = ES = SS = 0x0000`
* `SP = 0x7C00`

---

### Drive number conventions (Aero BIOS)

Aero uses BIOS drive numbers that match common PC conventions:

* **Hard disks:** `0x80`, `0x81`, …
* **CD-ROM (El Torito boot device):** `0xE0`, `0xE1`, …

When booting from a CD, the El Torito boot image expects to find the CD drive number in **`DL`**
when it starts executing.

Note: Aero’s BIOS can service multiple drive numbers via INT 13h when the corresponding backends
are present. In the canonical Win7 topology this means:

* `DL=0xE0` routes to the install-media ISO (2048-byte sectors via EDD).
* `DL=0x80` routes to the primary HDD (512-byte sectors via EDD).

Boot selection (the `DL` value provided to the boot image) is still primarily driven by
`BiosConfig::boot_drive`, and firmware also supports an optional “CD-first when present” policy
flag (`BiosConfig::boot_from_cd_if_present`) that attempts a CD boot when an ISO is attached and
otherwise falls back to the configured HDD boot drive.

Note: when using the “CD-first when present” policy, the configured `boot_drive` remains the HDD
fallback (typically `0x80`) even when the current boot actually came from CD-ROM. Host integrations
can query `Bios::booted_from_cdrom()` (or `Machine::active_boot_device()`) to report the actual boot
device for the current boot session.

When booting from CD (`DL=0xE0..=0xEF`), the BIOS Data Area fixed-disk count is still derived from
HDD backend presence so guests can access HDD0 during install/recovery.

---

### INT 13h calls required for Windows-style boot

Windows boot code (both HDD and CD paths) commonly relies on **INT 13h Extensions** (EDD) rather
than CHS reads. The minimum required calls are:

* `AH=41h` — Extensions check
* `AH=42h` — Extended read (Disk Address Packet)
* `AH=48h` — Extended get drive parameters

Implementation reference: `crates/firmware/src/bios/interrupts.rs::handle_int13` (match arms
`0x41`, `0x42`, `0x48`).

#### Supported INT 13h functions for CD-ROM drives (Aero BIOS)

When `DL` is a CD drive number (`0xE0..=0xEF`), Aero’s BIOS implements a minimal, read-oriented INT
13h surface:

| AH | Function | Notes |
|---:|---|---|
| `00h` | Reset disk system | Supported. |
| `01h` | Get status of last operation | Supported. |
| `03h` | Write sectors (CHS) | Not supported; returns write-protected (`CF=1`, `AH=03h`). |
| `05h` | Format track (CHS) | Not supported; returns write-protected (`CF=1`, `AH=03h`). |
| `15h` | Get disk type | Supported; reports presence and returns sector count in **2048-byte sectors**. |
| `41h` | Extensions check (EDD) | Supported; reports EDD 3.0 and `42h`+`48h` support. |
| `42h` | Extended read (DAP) | Supported (read-only). For CD drives, `LBA` + `count` are in **2048-byte sectors**. |
| `43h` | Extended write (DAP) | Not supported; returns write-protected (`CF=1`, `AH=03h`). |
| `48h` | Extended get drive parameters | Supported; reports `bytes_per_sector = 2048` and total sectors in 2048-byte units. |
| `4Bh` | El Torito disk emulation services | Partially supported (only when booted via El Torito). |
| other | Legacy CHS, etc. | Not supported; returns `CF=1`, `AH=01h`. |

#### 6.1 AH=41h — Extensions check

Inputs:

* `AH=0x41`
* `BX=0x55AA`
* `DL=drive`

Required outputs on success:

* `CF=0`
* `BX=0xAA55` (signature echoed back swapped)
* `AH=0x30` (report EDD 3.0)
* `CX` feature bits:
  * bit 0 (`0x0001`): extended disk access (`AH=42h`)
  * bit 2 (`0x0004`): extended drive parameters (`AH=48h`)

#### 6.2 AH=42h — Extended read (Disk Address Packet)

Inputs:

* `AH=0x42`
* `DL=drive`
* `DS:SI` points to a **Disk Address Packet** (DAP)

DAP formats we accept:

**16-byte DAP (size `0x10`)**

| Offset | Size | Field |
|---:|---:|---|
| `0x00` | 1 | size (`0x10`) |
| `0x01` | 1 | reserved (must be `0`) |
| `0x02` | 2 | sector count (`u16` LE, must be non-zero) |
| `0x04` | 2 | buffer offset |
| `0x06` | 2 | buffer segment |
| `0x08` | 8 | starting LBA (`u64` LE) |

**24-byte DAP (size `0x18`)**

Same as above, plus:

| Offset | Size | Field |
|---:|---:|---|
| `0x10` | 8 | optional 64-bit flat buffer pointer (if non-zero, overrides segment:offset) |

Semantics:

* Reads `count` sectors starting at `lba` into the destination buffer.
  * For **CD drives** (`DL=0xE0..`): `lba` and `count` are in **2048-byte sectors** (ISO logical
    blocks), and the transfer size is `count * 2048` bytes.
* Error handling must set `CF=1` and return an INT 13h status code in `AH`.

##### Sector-size rule (HDD vs CD-ROM)

In Aero, the “sector” unit for the DAP depends on the drive class:

* **HDD (`DL=0x80..=0xDF`)**: DAP `count`/`lba` are in **512-byte sectors** (standard EDD behavior).
* **CD-ROM (`DL=0xE0..=0xEF`)**: DAP `count`/`lba` are in **2048-byte sectors**.
  * Internally, the BIOS reads from a 2048-byte-sector `CdromDevice` backend when provided.
    Otherwise it falls back to reading raw ISO bytes from a 512-byte-sector [`BlockDevice`] and
    converts:
    * `lba512 = lba2048 * 4`
    * `count512 = count2048 * 4`

This matches the `AH=48h` “bytes per sector” value for the drive (see below) and is required for
Windows install-media bootloaders that read from CD-ROM via EDD.

#### 6.3 AH=48h — Extended get drive parameters

Inputs:

* `AH=0x48`
* `DL=drive`
* `DS:SI` points to a caller-allocated buffer whose first `u16` is the buffer size in bytes.

Minimum behavior:

* Require `buffer_size >= 0x1A`.
* Fill the EDD parameter table fields needed by Windows boot code (in particular, **bytes per
  sector** and **total sector count**).
    * For **CD drives**, this means reporting `bytes_per_sector = 2048` and `total_sectors` in
      **2048-byte units**.

#### 6.4 AH=4Bh — El Torito disk emulation services (compatibility)

Some El Torito boot images query El Torito metadata via INT 13h `AH=4Bh`. Windows 7 install media does
not typically require this path, but Aero implements a minimal subset for compatibility when booting
via El Torito (since it is closely tied to boot-catalog parsing).

Constraints:

* Only available when the BIOS actually booted via El Torito and captured boot-catalog metadata
  during POST.
* Only valid for the El Torito boot drive (must match the `DL` used to boot, typically `0xE0`).

Supported subfunctions (selected by `AL`; Aero supports only these `AX` values):

* `AX=4B00h` (`AL=00h`) — Terminate disk emulation
  * For **no-emulation** boots, Aero treats this as a no-op success.
* `AX=4B01h` (`AL=01h`) — Get disk emulation status
  * Writes a status packet at `ES:DI` (caller provides the buffer).
  * Compatibility rule: if the caller sets the first byte to a non-zero buffer size, Aero requires
    it to be `>= 0x13`.

All other subfunctions are unsupported.

##### Status packet layout (0x13 bytes)

All multi-byte fields are little-endian.

| Offset | Size | Field | Notes |
|---:|---:|---|---|
| `0x00` | 1 | packet size | `0x13` |
| `0x01` | 1 | media type | `0x00` = no-emulation |
| `0x02` | 1 | boot drive | `DL` value used for El Torito boot (typically `0xE0`) |
| `0x03` | 1 | controller index | currently `0` |
| `0x04` | 4 | boot image LBA | ISO LBA (**2048-byte units**) of the boot image (`u32` LE) |
| `0x08` | 4 | boot catalog LBA | ISO LBA (**2048-byte units**) of the boot catalog (`u32` LE) |
| `0x0C` | 2 | load segment | real-mode segment used to load boot image (e.g. `0x07C0`) |
| `0x0E` | 2 | sector count | number of **512-byte** sectors loaded for the initial image |
| `0x10` | 3 | reserved | zero |

`boot image LBA` and `boot catalog LBA` use **ISO logical block addressing** (2048-byte sectors, the
same unit as ISO9660 and the El Torito boot catalog). If you need underlying 512-byte LBAs:
`lba512 = lba2048 * 4` (only relevant when the ISO is exposed via a 512-byte-sector `BlockDevice`).

---

### Note on `-boot-info-table` (mkisofs/xorriso)

Tools like `mkisofs`/`xorriso` support `-boot-info-table`, which patches a “boot info table” into
the boot image for the benefit of **some bootloaders** (historically `isolinux`-style).

Important:

* **Aero BIOS does not consume `-boot-info-table`.**
* It is an optional bootloader-side convenience, not part of El Torito catalog discovery.

## IRQ semantics (browser runtime)

This document defines the **single, unambiguous contract** for interrupt request (IRQ) delivery in Aero's browser worker runtime.

Runtime note: This document currently describes the legacy worker runtime (`vmRuntime=legacy`), where guest device models live in the
I/O worker and assert/deassert IRQ lines on the CPU worker via shared IPC events. In `vmRuntime=machine`, guest devices live inside the
canonical `api.Machine` runtime owned by `apps/web/src/workers/machine_cpu.worker.ts`, so IRQ delivery is handled inside the machine and this
IO→CPU IRQ transport is not used.

It exists to remove ambiguity between:

- **Edge-triggered** interrupt sources (e.g. the legacy i8042 PS/2 controller on ISA IRQ1/IRQ12)
- **Level-triggered** interrupt sources (e.g. PCI INTx devices like UHCI/EHCI/xHCI)

### What `raiseIrq()` / `lowerIrq()` mean

In `apps/web/src`, `IrqSink` models **physical interrupt input line levels**:

- `raiseIrq(irq)` **asserts** the line
- `lowerIrq(irq)` **deasserts** the line

These calls manipulate the *wire* (line level). They do **not** mean "deliver an interrupt right now".

See also:

- `apps/web/src/io/device_manager.ts` (`IrqSink`)
- `apps/web/src/workers/io.worker.ts` (legacy device IRQ wiring)
- `apps/web/src/workers/cpu.worker.ts` (legacy IRQ bitmap/refcount)

### Shared IRQ lines: refcounted wire-OR

Multiple devices may share an IRQ line (e.g. PCI INTx, legacy PIC inputs). Aero models this as a refcounted **wire-OR**:

- each `raiseIrq()` increments a per-line refcount
- each `lowerIrq()` decrements it
- the effective line level is **asserted while the refcount is > 0**

#### Balanced usage

Repeated `raiseIrq()` calls without an intervening `lowerIrq()` are legal, but they must eventually be balanced:

```ts
raiseIrq(1);
raiseIrq(1); // refcount now 2
lowerIrq(1); // refcount now 1 (still asserted)
lowerIrq(1); // refcount now 0 (deasserted)
```

#### Guardrails (underflow/overflow)

The worker runtime clamps common misuse patterns:

- **Underflow**: extra `lowerIrq()` calls when the refcount is already 0 are ignored (dev-time warning).
- **Overflow**: refcounts **saturate at `0xffff`** to avoid `Uint16Array` wraparound (dev-time warning).

The shared helper that defines this behaviour is:

- `apps/web/src/io/irq_refcount.ts`

And the unit tests that lock in the contract are:

- `apps/web/src/io/irq_refcount.test.ts`

### Edge-triggered sources

Edge-triggered sources MUST be represented as an explicit *pulse* (0→1→0):

1. `raiseIrq(irq)`
2. `lowerIrq(irq)` (immediately after; same turn/microtask is fine)

Notes:

- A rising edge cannot be observed if the line is already asserted (for example because another device is holding the line high). In that case the pulse is naturally suppressed by wire-OR refcounting.
  This matches real hardware: you cannot get a new rising edge on an already-high signal.

Example:

- **i8042 keyboard/mouse controller** (ISA IRQ1/IRQ12): pulse when a byte becomes available and interrupts are enabled.

Implementation note (WASM bridge):

- The browser runtime's `crates/aero-wasm::I8042Bridge` exposes `drain_irqs()` which returns a bitmask of **pending IRQ pulses**
  since the last call (bit0=IRQ1, bit1=IRQ12). This exists because a *level-only* API can miss pulses when the i8042 refills the
  output buffer immediately after a port `0x60` read (multiple bytes pending).
  Consumers should prefer `drain_irqs()` when available and translate each bit into an explicit `raiseIrq`+`lowerIrq` pulse.

### Level-triggered sources

Level-triggered devices assert their interrupt line while an interrupt condition remains pending, and deassert it once the condition is cleared/acknowledged.

Example:

- **PCI INTx** devices (e.g. UHCI/EHCI/xHCI): INTx is a shared, wired-OR, *level-triggered* signal in PCI.

### Worker transport (`irqRaise` / `irqLower`)

In the legacy worker runtime (`vmRuntime=legacy`), IRQs are transported between the I/O worker and CPU worker as discrete AIPC events:

- `irqRaise` (line asserted)
- `irqLower` (line deasserted)

These events are still *line levels*; edge-triggered interrupts are represented as explicit pulses.

Implementation note: some paths may coalesce nested assertions for efficiency (only emit `irqRaise` on 0→1 and `irqLower` on 1→0). This is compatible with the level/refcount contract.

### Future PIC/APIC behaviour

The current CPU worker publishes a **level bitmap** of asserted IRQ lines into shared memory for debugging/observability.

A level bitmap alone is not sufficient to faithfully represent edge-triggered interrupts (a short pulse can be missed by a sampler). When a real PIC/APIC model is implemented in the browser runtime, it should:

1. Track the **external line level** per IRQ input (driven by `irqRaise`/`irqLower`).
2. For **edge-triggered inputs**, latch rising edges into a pending register (e.g. PIC IRR) so they remain pending until acknowledged/EOI.
3. For **level-triggered inputs**, treat “line asserted” as “interrupt pending” until the device clears the condition and deasserts the line.

## PCI Device Compatibility (Windows 7 Driver Binding)

Windows (and Linux) bind drivers primarily based on PCI **Vendor ID / Device ID** and the **Class Code (base / sub / prog-if)**, and in some cases also require a sane PCI **BAR layout** and/or specific capabilities. If these fields are inconsistent, guests can fail to attach in-box drivers and instead show “Unknown device”, breaking boot or core functionality.

This document defines Aero’s canonical PCI identity + wiring for I/O devices, with a focus on stable Windows 7 binding.

The canonical **paravirtual** device identities are encoded in `crates/devices/src/pci/profile.rs`, and validated by `crates/devices/tests/pci_profile.rs`.

Note: the canonical `aero_machine::Machine` supports **two mutually-exclusive** display configurations:

- `MachineConfig::enable_aerogpu=true`: expose the **AeroGPU PCI identity** at `00:07.0`
  (`A3A0:0001`) with the canonical BAR layout (BAR0 regs + BAR1 VRAM aperture). This is the
  canonical Windows driver binding target (`PCI\VEN_A3A0&DEV_0001`). In `aero_machine` today:

  - BAR1 is backed by a dedicated VRAM buffer for legacy VGA/VBE boot display compatibility and
    implements permissive legacy VGA decode (VGA port I/O + VRAM-backed `0xA0000..0xBFFFF` window;
    see `graphics.md`).
  - Note: the in-tree Win7 AeroGPU driver treats the adapter as system-memory-backed (no dedicated
    WDDM VRAM segment). BAR1 is outside the WDDM memory model (see
    `../specs/windows7-aerogpu-wddm-driver.md`).
  - BAR0 implements a minimal MMIO surface sufficient for the in-tree Win7 KMD to initialize:
    - ring/fence transport (submission decode/capture + fence-page/IRQ plumbing; default bring-up
      behavior can complete fences without executing the command stream; browser/WASM runtimes can
      enable an out-of-process “submission bridge” via `Machine::aerogpu_drain_submissions` +
      `Machine::aerogpu_complete_fence`; native builds can install an in-process backend such as the
      feature-gated headless wgpu backend via `Machine::aerogpu_set_backend_wgpu`), and
    - scanout0 register storage + vblank counters/IRQ semantics for `WaitForVerticalBlankEvent`
      pacing (see `drivers/aerogpu/protocol/vblank.md`).
- `MachineConfig::enable_vga=true`: expose the standalone legacy VGA/VBE implementation
  (`aero_gpu_vga`) for BIOS/bootloader VGA/VBE compatibility.
  - When `enable_pc_platform=false`, the machine maps the VBE LFB MMIO aperture directly at the
    configured LFB base (historically defaulting to `0xE000_0000` / `aero_gpu_vga::SVGA_LFB_BASE`).
  - When `enable_pc_platform=true`, the machine exposes a minimal Bochs/QEMU-compatible “Standard
    VGA” PCI stub (`aero_devices::pci::profile::VGA_TRANSITIONAL_STUB`, `1234:1111` at `00:0c.0`)
    and routes the VBE LFB through its BAR0 inside the PCI MMIO window. The BAR base is assigned by
    BIOS POST / the PCI allocator (and may be relocated when other PCI devices are present), and is
    mirrored into the BIOS VBE `PhysBasePtr` and the VGA device model so guests observe a coherent
    LFB base.
  - This path is not part of the long-term Windows paravirtual device contract.

### Canonical PCI layout (bus/dev/fn)

We assume a single PCI bus (`bus 0`) with stable device numbers. Not all devices must be present in every VM configuration, but when enabled they should retain their canonical BDF for predictable guest enumeration.

| BDF      | Device | Vendor:Device | Class (base/sub/progif) | INTx pin | Notes |
|----------|--------|---------------|--------------------------|----------|-------|
| 00:01.0  | ISA    | 8086:7000     | 06/01/00                 | -        | PIIX3-compatible ISA bridge (function 0 of a multi-function slot; `header_type=0x80` so guests discover 00:01.1/00:01.2) |
| 00:01.1  | IDE    | 8086:7010     | 01/01/80                 | INTA     | PIIX3-compatible PCI IDE (legacy compatibility mode, bus mastering DMA). Note: ATA/ATAPI completion interrupts are legacy ISA IRQ14/IRQ15 (see below + `storage.md`). |
| 00:01.2  | USB1   | 8086:7020     | 0C/03/00                 | INTA     | UHCI (USB 1.1) |
| 00:02.0  | SATA   | 8086:2922     | 01/06/01                 | INTA     | AHCI (SATA) |
| 00:03.0  | NVMe   | 1B36:0010     | 01/08/02                 | INTA     | NVMe controller (optional) |
| 00:04.0  | Audio  | 8086:2668     | 04/03/00                 | INTA     | Intel HD Audio (HDA) controller |
| 00:05.0  | NIC    | 8086:100E     | 02/00/00                 | INTA     | Intel E1000 (82540EM) |
| 00:06.0  | NIC    | 10EC:8139     | 02/00/00                 | INTA     | RTL8139 (alternate NIC option) |
| 00:07.0  | GPU    | A3A0:0001     | 03/00/00                 | INTA     | AeroGPU display controller (WDDM). **Canonical BDF + VID/DID contract** for Windows driver binding (`PCI\VEN_A3A0&DEV_0001`). Do not assign any other device to `00:07.0`. See `../specs/aerogpu-device-abi.md`. Canonical PCI profile defines BAR0 (64KiB regs) + BAR1 (prefetchable VRAM aperture) per `graphics.md`. |
| 00:08.0  | vNIC   | 1AF4:1041     | 02/00/00                 | INTA     | virtio-net (Aero Win7 contract v1: modern-only, `REV_01`; upstream transitional = 1AF4:1000) |
| 00:09.0  | vBlk   | 1AF4:1042     | 01/00/00                 | INTA     | virtio-blk (Aero Win7 contract v1: modern-only, `REV_01`; upstream transitional = 1AF4:1001) |
| 00:0A.0  | vInput | 1AF4:1052     | 09/80/00                 | INTA     | virtio-input keyboard (Aero Win7 contract v1: `SUBSYS_00101AF4`, `REV_01`, `header_type=0x80` for multi-function discovery) |
| 00:0A.1  | vInput | 1AF4:1052     | 09/80/00                 | INTA     | virtio-input mouse (Aero Win7 contract v1: `SUBSYS_00111AF4`, `REV_01`) |
| 00:0B.0  | vSnd   | 1AF4:1059     | 04/01/00                 | INTA     | virtio-snd (Aero Win7 contract v1: modern-only, `REV_01`) |
| 00:0c.0  | VGA (stub) | 1234:1111 | 03/00/00 | - | Bochs/QEMU “Standard VGA” PCI stub identity (see `aero_devices::pci::profile::VGA_TRANSITIONAL_STUB`). Exposed only for the standalone legacy VGA/VBE boot-display path when `enable_vga=true` (and `enable_aerogpu=false`) with the PC platform enabled (routes the VBE LFB through PCI BAR0). Must be absent when `enable_aerogpu=true`. |
| 00:0d.0  | USB3   | 1B36:000D     | 0C/03/30                 | INTA     | xHCI (USB 3.x) controller (QEMU xHCI identity). Wired in the web runtime when the WASM build exports `XhciControllerBridge` (optional/experimental; Windows 7 has no in-box xHCI driver). See [`usb-and-input.md`](usb-and-input.md). |
| 00:12.0  | USB2   | 8086:293A     | 0C/03/20                 | INTA     | EHCI (USB 2.0) controller (ICH9-family identity; Windows 7 in-box `usbehci.sys`). See [`usb-and-input.md`](usb-and-input.md). |

#### Notes on display (AeroGPU vs VGA/VBE boot display)

- The canonical AeroGPU Windows 7 driver binds to `PCI\VEN_A3A0&DEV_0001`. See
  [`abi/aerogpu-pci-identity.md`](../specs/aerogpu-device-abi.md).
- With `MachineConfig::enable_aerogpu=true`, the machine exposes the AeroGPU PCI identity at
  `00:07.0` (`A3A0:0001`) for driver binding. In the intended AeroGPU-owned VGA/VBE boot display
  path (see [`16-aerogpu-vga-vesa-compat.md`](graphics.md)), firmware derives
  the VBE mode-info `PhysBasePtr` from AeroGPU BAR1: `PhysBasePtr = BAR1_BASE + 0x40000`
  (`AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES`; see `crates/aero-machine/src/lib.rs::VBE_LFB_OFFSET`).
- With `MachineConfig::enable_vga=true` (and `enable_aerogpu=false`), boot display is provided by
  the standalone `aero_gpu_vga` VGA/VBE device model. Firmware reports the configured VBE LFB base
  address (historically defaulting to `0xE000_0000`).
  - When `enable_pc_platform=true`, this base is the BAR0 base of the “Standard VGA” PCI stub
    (`00:0c.0`), assigned by BIOS POST / the PCI allocator (and may be relocated when other PCI
    devices are present).
  - When `enable_pc_platform=false`, the machine maps the LFB MMIO aperture directly at the
    configured physical address.
- This `enable_vga` VGA/VBE path is a stepping stone and does **not** implement the full AeroGPU
  WDDM MMIO/ring protocol described by
  [`16-aerogpu-vga-vesa-compat.md`](graphics.md).

#### Notes on virtio IDs (transitional vs modern)

Virtio uses vendor ID `0x1AF4`.

Aero’s canonical profile (and the Windows 7 virtio device contract, `AERO-W7-VIRTIO` v1) uses the virtio 1.0+
**modern** virtio-pci device ID space (`0x1040 + device_id`) and a modern-only transport (PCI capabilities + MMIO).

The **transitional** IDs below are listed only for upstream/historical context; Aero contract v1 does not require and
may not expose transitional IDs.

| Virtio function | Transitional ID | Modern ID |
|-----------------|-----------------|-----------|
| net             | 1AF4:1000       | 1AF4:1041 |
| blk             | 1AF4:1001       | 1AF4:1042 |
| input           | 1AF4:1011       | 1AF4:1052 |

Note: virtio-snd is treated as **modern-only** in `AERO-W7-VIRTIO` v1; do not rely on any legacy/transitional ID space for driver binding.

### IRQ routing (INTx → PIRQ → PIC/APIC)

PCI INTx interrupts are level-triggered and are routed by the chipset via “PIRQ” lines (A–D).

Note: some “legacy” devices (notably PIIX3 IDE) use ISA IRQs (IRQ14/IRQ15) for their data-plane
interrupts even though they are exposed as PCI functions and have PCI INTx fields in config space.
Do not assume that every PCI function’s interrupts are delivered via the PIRQ→GSI mapping.

#### INTx swizzle (root bus)

For a device on the root bus, the PIRQ line is selected using the standard PCI swizzle:

```
PIRQ = (INTx + device_number) mod 4
```

Where `INTA=0, INTB=1, INTC=2, INTD=3`.

In the canonical layout above, devices use `INTA`, so the effective PIRQ line is `device_number mod 4`.

#### PIRQ → PIC IRQ (8259)

In legacy PIC mode, Aero mirrors the Q35 PCI links to PIC IRQs 10–13:

| PIRQ | PIC IRQ |
|------|---------|
| A    | 10      |
| B    | 11      |
| C    | 12      |
| D    | 13      |

#### PIRQ → IOAPIC GSI

In APIC mode, the canonical Q35 root-bus links use IOAPIC GSIs `20..23`.
This is intentionally distinct from the ISA IRQ range:

| PIRQ | IOAPIC GSI |
|------|------------|
| A    | 20         |
| B    | 21         |
| C    | 22         |
| D    | 23         |

IOAPIC board polarity follows the bus, not the old numeric shortcut:
ISA GSIs 1–15 are active-high except SCI9; PCI GSIs 16 and above are
active-low. This distinction is required for i8042 mouse IRQ12 to assert on
the high level while PCI GSI20 asserts on the low level.

### PCI config requirements (what must stay stable)

For Windows 7 and Linux to bind drivers predictably:

1. **Vendor ID / Device ID** must match the expected device model.
2. **Class/Subclass/ProgIF** must match:
    - IDE: `01/01/80` — **not `0x8A`**. Advertising the native-capable programming
      interface makes Windows treat the function as relocatable, size BAR0–3, leave
      them unassigned, and never add the ISA channel resources — so the optical
      drive never enumerates. See [storage.md](./storage.md).
    - AHCI: `01/06/01`
    - NVMe: `01/08/02`
    - HDA: `04/03/00`
    - UHCI: `0C/03/00`
    - EHCI: `0C/03/20`
    - xHCI: `0C/03/30`
3. **Header type** must be `0x00` (type-0 endpoint), except when a device intentionally exposes
     multiple functions on the same slot:
    - PIIX3 (function 0 at `00:01.0`) must set `header_type = 0x80` so guests enumerate the IDE
      and UHCI functions at `00:01.1` and `00:01.2`.
   - `virtio-input` keyboard (function 0) must set `header_type = 0x80` (multi-function bit) so
      guests enumerate the paired mouse function (function 1).
4. **BAR types and sizes** must be correct:
    - Example: AHCI’s ABAR must be MMIO and large enough for the implemented port set (Aero uses 8KiB
      via `aero_devices::pci::profile::AHCI_ABAR_SIZE`).
    - Example: HDA MMIO must be 16KiB (`0x4000`) per spec.
    - Example: AeroGPU’s canonical PCI profile defines BAR0 (64KiB regs) and BAR1 (prefetchable VRAM aperture)
      for legacy VGA/VBE compatibility (`graphics.md`).
5. **Virtio PCI capabilities** must be present and internally consistent for modern drivers:
    - Virtio vendor-specific capabilities for Common/ISR/Device/Notify regions
    - MSI/MSI-X capabilities where supported by the platform implementation

### Debugging

The unit tests include a “PCI dump” string generator (`aero_devices::pci::profile::pci_dump`) to help spot regressions in IDs/class codes/IRQ mapping when adding new devices.

## The retirement of `crates/emulator`

For most of this repository's life there were two device stacks. The canonical one is
`aero-machine` plus the `aero-*` device crates. The other was `crates/emulator`: its own PCI
framework, its own storage traits and disk formats, its own USB and input glue, its own audio
device models, and its own AeroGPU device and command executor — around twenty-six thousand lines
of it.

It is gone. What follows is what it contained, where the parts that mattered went, and what was
left behind.

### Why it could go

The decisive fact was that **nothing depended on it**. Not one workspace crate, service, or tool
imported `emulator::` anything. Its only consumers were its own forty test files and one benchmark,
which is to say it was verified but unused — a stack whose tests proved that it still worked, not
that anything still wanted it.

Its own crate documentation said as much, naming `aero-machine` as the canonical VM wiring and
itself as legacy. A guardrail script existed for the specific purpose of stopping other crates from
depending on it. The question was never whether it should survive; it was what inside it was worth
keeping.

### What moved, and why each was worth keeping

**The software rasterizer** is now [`crates/aero-gpu-software`](../../crates/aero-gpu-software/).
It executes the AeroGPU command stream on the CPU — vertex fetch, triangle setup, depth testing,
texture sampling, blending — against plain memory. Every other backend needs a graphics API, an
adapter, and a driver; this one needs none of them, which makes it the only backend guaranteed to
run anywhere. That matters more than it sounds: the graphics tests skip when no adapter is present,
and a suite that skips its hardest cases reports the same green as one that passed them. See
[testing](testing.md#the-gpu-tests-were-skipping) for what that concealed.

The move tightened one thing. The rasterizer reconstructs command packets from untrusted guest
bytes through a generic helper that was bounded only by `Copy` — a bound that permits `bool`,
`char`, and enums, none of which are valid for every bit pattern. It is now bounded by a
`WireStruct` marker implemented solely for the twenty-eight `#[repr(C)]` integer-only protocol
structs it actually reads, which is what makes the single remaining `unsafe` block sound rather
than merely conventional.

**The `AEROSPRS` disk format** is now [`crates/aero-storage/src/sparse_v1.rs`](../../crates/aero-storage/src/sparse_v1.rs),
and the converter that migrates such images to the current `AEROSPAR` format is
`aerosparse_convert`, a binary of the same crate. Nothing writes `AEROSPRS` any more, but images in
that format exist, and a format nothing can open is data destroyed however obsolete it is. The port
moved the implementation onto `VirtualDisk`'s byte-offset addressing and the canonical error type;
`aerosprs_to_aerospar_conversion` covers the round trip, because reading the old format correctly
is not the same as the contents surviving the trip.

**The audio DSP** is now [`aero_audio::dsp`](../../crates/aero-audio/src/dsp/). It converts between
sample formats, sample rates, and channel layouts — a guest may open a 22.05 kHz mono stream while
the host's audio context runs 48 kHz stereo. It arrived with a windowed-sinc resampler (behind the
`sinc-resampler` feature) and arbitrary-channel remixing, neither of which `aero-audio` had; its
own resampling was linear and its decoding stereo-only. The golden-WAV test and the benchmark moved
with it.

### What was left behind, and why

Everything else fell into two groups.

**Duplicates of a canonical crate.** The AeroGPU PCI device model duplicated
`aero-devices-gpu::pci`; the AeroGPU executor duplicated `aero-devices-gpu::executor` method for
method, differing only in tracing hooks and an inlined call to the software rasterizer. The HDA
device model duplicated `aero-audio`. The IDE, AHCI, and NVMe wrappers duplicated
`aero-devices-storage` and `aero-devices-nvme`. Keeping a duplicate alive means every change to the
contract has to be made twice and stays correct only as long as someone remembers.

**Re-export shims.** The networking, SMP, VGA, input, and machine modules had already been hollowed
out into `pub use` of the canonical crate, preserving historical import paths for callers that no
longer existed.

Two things were left behind that are neither, and are worth naming precisely so the decision is
re-examinable rather than merely made:

- **The AC97 device model.** The canonical audio stack is `aero-audio` plus `aero-virtio`, which is
  a standing decision, and AC97 is not part of it. This was a decision applied, not a new one.
- **The coalescing storage backend and the write-policy block cache.** These are genuinely absent
  from `aero-storage`, which has a simpler block cache with no write-back policy and no incremental
  flush, and nothing that merges adjacent scatter-gather ranges. Adopting the coalescer would mean
  adding vectored I/O to `VirtualDisk`, which is a change to the canonical trait rather than a
  migration, and so belongs to whoever needs it.

The retired tree is not deleted; it sits outside the repository under `.attic/legacy-crates/`.

### The one integration that was not attempted

The software rasterizer executes against guest physical memory addressed by GPA, reading the
command stream and allocation table itself from the submission descriptor. The canonical
`AeroGpuCommandBackend` trait hands a backend a command stream that has *already* been read, and
carries no GPAs. The two do not meet.

Bridging them honestly requires one of: an entry point on the rasterizer that takes an
already-read command stream and allocation table instead of addresses, or a backend trait that
passes the submission descriptor through. Either is a real design decision about where the memory
read belongs. The rasterizer is preserved and tested at its own interface — which is the device
model's interface, not the backend trait's — and no adapter was written to paper over the gap.

### What stops it coming back

`scripts/ci/check-repo-layout.sh` fails if any file under `crates/emulator/` is tracked, and
`tests/repo_hygiene.contract.test.js` carries the directory on its ban list. The two scripts that
previously guarded against dependency edges into the crate — `check-no-emulator-deps.py` and
`check-usb-doc-paths.py` — are retired along with it; a dependency edge is no longer the failure
mode, the directory returning is.

## A thread-local with a destructor cannot live in the browser runtime (2026-07-30)

`vm_boot_vga_serial` and `workers_panel_input_capture_machine_runtime` both died
the moment the interpreter ran, with

```
[cpu] vm run_slice failed: RuntimeError: unreachable
```

followed by an unbroken stream of `recursive use of an object detected which
would lead to unsafe aliasing in rust`. The second message is noise: the trap
unwound out of a `#[wasm_bindgen]` method while its `WasmRefCell` was still
borrowed, so every later call on that object failed too. Only the first one
matters.

A release wasm build carries no symbols, so the trap was just a function index.
Rebuilding with `wasm-opt` skipped — it is the step that strips names — gave the
real stack:

```
std::alloc::rust_oom
alloc::alloc::handle_alloc_error
alloc::raw_vec::handle_error
RawVec<(*mut u8, unsafe extern "C" fn(*mut u8)), std::alloc::System>::grow_one
std::sys::thread_local::destructors::list::register
Storage<RefCell<aero_cpu_core::interp::tier0::exec::DecodeCache>>::get_or_init_slow
WasmVm::run_slice
```

It is an allocation failure, not a panic — which is why no panic message ever
appeared, and why installing a panic hook did not produce one.

The chain is worth stating in full, because each link is deliberate:

- `aero-wasm` installs a `#[global_allocator]` (`runtime_alloc`) bounded to the
  reserved low region, so a runaway allocation fails instead of walking into
  guest RAM.
- The guest `WebAssembly.Memory` is created with `maximum` equal to `initial`,
  so guest RAM has a fixed address and never moves.
- `std::sys::thread_local::destructors::list::register` allocates through
  `std::alloc::System` — deliberately, to avoid recursing into the global
  allocator. On wasm32 `System` is dlmalloc, which obtains memory only by
  growing the linear memory.

So the one allocation path that bypasses the bounded heap is also the one that
needs a growable memory, and the runtime has neither. **Any `thread_local!` whose
value implements `Drop` will abort the module the first time it is touched.**

`DECODE_CACHE` in the Tier-0 interpreter held a `RefCell<DecodeCache>` owning a
boxed slot array. It is now `ManuallyDrop`, which skips the destructor
registration entirely. Nothing is lost: the cache is allocated once and wants to
live as long as the thread anyway.

The same crates' other thread-locals live in the GPU wasm packages, which do not
use the bounded allocator, so they are unaffected.

### Why there is no panic hook

A hook forwarding panic messages to `console.error` was tried and removed. It
could not have helped here — an allocation failure aborts without going through
the panic machinery — and it is actively unsafe in the threaded build, for the
same reason described below: the hook is a `Box<dyn Fn>` living in the shared
heap, and its code pointer is a function-table index that only means something in
the instance that created it.

Diagnosing a release-build trap is instead a matter of rebuilding with `wasm-opt`
skipped, which is what strips the name section. That turns `wasm-function[4874]`
into a Rust symbol and is usually enough on its own.

## JS handles cannot cross a wasm instance boundary

`audio-loopback-synthetic` underruns badly — tens of thousands of frames per
second — and the reason is in the log:

```
WASM mic bridge read failed; falling back to JS ring reader:
RuntimeError: table index is out of bounds
  at platform::audio::mic_bridge::...::read_f32_into
```

`MicRingBridge` holds `self.samples`, a `js_sys::Float32Array`. Built with
`--enable-reference-types`, wasm-bindgen keeps JS values in a table and a
Rust-side handle is an index into it — and while linear memory is shared between
workers, a table is per-instance. That is a plausible mechanism, and it is worth
checking, but **it is not established**: the sibling `self.header` handle is read
on the same code path without trapping, which it should not survive if the whole
bridge came from another instance.

What is established is narrower:

- The trap is real and appears in most runs of this demo.
- The caller catches it and falls back to a JS ring reader, which cannot keep up,
  so every missed quantum is counted as an underrun.
- The spec's outcome tracks the trap without being determined by it: it passed 1
  of 4 runs with the optimized module and 3 of 4 with an unoptimized one. Both
  samples are small and they overlap.

What was ruled out, or rather never held up: `wasm-opt` looked like the culprit
at first, because an unoptimized build produced no trap. It does not survive
measurement. Bisecting optimization levels against the trap gave `-O1` and `-O2`
trapping while `-O3` and `-O4` came out clean — yet the shipped `-O4` pipeline
build traps. A signal that inverts like that is not a function of the
optimization level, and any single-run comparison here will mislead.

Anyone picking this up should start by making the trap reproducible on demand —
a focused harness that drives `read_f32_into` directly, rather than the full
audio demo — because every conclusion above is limited by not having one.
