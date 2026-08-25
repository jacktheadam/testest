# Windows guest drivers

> The driver stack that runs *inside* the guest: the four virtio drivers (three
> WDM, one KMDF), the AeroGPU WDDM display driver, and the guest tooling that
> installs them.
>
> These drivers are C and C++ by necessity — the reasoning is recorded in
> [../decisions/0017-guest-driver-language.md](../decisions/0017-guest-driver-language.md) — and are built with a
> Windows-only toolchain, so they are the one part of Aero that cannot be built
> or exercised on the Linux hosts the project otherwise runs on.

This page is the area reference for everything that makes Windows 7 work well
as an Aero guest: the virtio device contract that binds emulator device models
to guest drivers, the in-tree Windows 7 driver sources, the driver
build/sign/package pipeline, the Guest Tools media and in-guest installer, and
the install-media preparation + unattended-install tooling.

The AeroGPU display driver has its own area page; it is mentioned here only
where it participates in shared packaging (Guest Tools, dbgctl helper).

## Current state (verified)

- The **AERO-W7-VIRTIO contract v1** is implemented on both sides:
  - Emulator side: device profiles in `crates/devices/src/pci/profile.rs`
    (PCI IDs, subsystem IDs, `revision_id: 1`, BAR0 layout) and device models in
    `crates/aero-virtio/` (`src/pci.rs`, `src/devices/{blk,net,input,snd}.rs`).
    Queue sizes, ID_NAME strings, and the fixed BAR0 layout match the contract
    (spot-checked against the constants cited below).
  - Guest side: in-tree Windows 7 drivers under `drivers/windows7/` and
    `drivers/win7/`, with INFs revision-gated to `&REV_01` (e.g.
    `drivers/windows7/virtio-blk/inf/aero_virtio_blk.inf`,
    `drivers/windows7/virtio-snd/inf/aero_virtio_snd.inf`).
- The **driver build/sign/package pipeline** lives in `drivers/build/*.ps1`, and
  runs on Windows only. It is invoked by hand: the workflow automation that used
  to drive it was purged and is banned, so there is no scheduled driver build and
  no published artefact.
- **Guest Tools** media is produced by `tools/packaging/aero_packager/` plus
  specs in `tools/packaging/specs/`, driven from CI outputs by
  `drivers/build/package-guest-tools.ps1`. The in-guest installer lives in `guest-tools/`.
- **Install-media tooling** (BCD patching, offline certificate injection,
  unattended install) lives in `tools/windows/`, `tools/bcd_patch/`,
  `tools/win-offline-cert-injector/`, `tools/win7-slipstream/`, and
  `guest-tools/unattend/`.
- `protocol-vectors/windows-device-contract.json` and
  `protocol-vectors/windows-device-contract-virtio-win.json` are **data, not docs**: they
  are consumed by tests and tools (see "Device contract data files" below) and
  intentionally live in `protocol-vectors/`, not with the purged `docs/` prose.

## Every boot-critical device binds an inbox driver

**No guest-tools driver is needed to reach a desktop.** Each device the boot
depends on was checked against the INF store inside the install media itself, and
each one is claimed by a driver Windows 7 already ships. This is what makes the
bring-up tractable at all: the guest driver stack in this directory is for
*acceleration*, not for first light, and it stays off the critical path until the
desktop is held.

| Device | What the machine presents | What Windows binds |
|---|---|---|
| AHCI storage | ICH9 AHCI, `PCI\CC_010601` | `msahci_Inst` in `mshdc.inf` → `msahci.sys` |
| IDE storage | PIIX3 IDE function | the IDE chain in the same `mshdc.inf` |
| Network | e1000 as `8086:100E` (Intel 82540EM) | `nete1g3e.inf` carries a plain `PCI\VEN_8086&DEV_100E` entry with no subsystem-ID constraint, so PnP binds the inbox Intel PRO/1000 MT driver |
| Audio | *(nothing — see below)* | would be inbox `hdaudbus.sys` and `hdaudio.sys` via the `HDAUDIO\FUNC_01` function driver |
| Display | VGA/VBE | the inbox `vga.sys` boot display path |

Two consequences worth holding onto. The e1000 identity being QEMU-compatible is
load-bearing rather than incidental — presenting `8086:100E` is what avoids
needing a driver on the install path at all. And because the AeroGPU WDDM driver
has never been compiled (the kit is Windows-only; see the debt list below),
display bring-up deliberately impersonates a Bochs adapter through
`AERO_AEROGPU_STDVGA_IDS=1` so that `vga.sys` will bind.

Audio is the exception and deserves stating plainly: **the canonical machine
presents no audio controller at all.** `MachineConfig` has no audio field, and
`enable_hda` exists only on the experimental `PcMachine`, defaulting to off
everywhere. The inbox-driver claim above is what *would* bind once a controller
is presented; it is not something a guest has ever done here.

Networking needs configuration rather than drivers: the native CLI leaves
`enable_e1000` off by default, and the browser defaults enable it.

## The AERO-W7-VIRTIO device contract (v1)

The contract is the spine of this area. It deliberately specifies a small,
strict, testable subset of virtio: **modern virtio-pci only** (PCI vendor
capabilities + BAR0 MMIO), split virtqueues, INTx required, MSI-X optional.
Legacy/transitional I/O-port transport, packed rings, SR-IOV, and host offloads
are out of scope for v1.

### Identity

- Vendor ID `0x1AF4`; modern device IDs `0x1040 + <virtio device id>`;
  **PCI Revision ID `0x01` encodes contract major version 1**. Drivers must
  refuse unknown revisions; in-tree INFs match `...&REV_01`, and some drivers
  also check the revision at runtime. QEMU defaults to `REV_00`, so
  contract-v1 testing under QEMU needs `x-pci-revision=0x01` (and preferably
  `disable-legacy=on`) on each `-device virtio-*-pci,...` argument — the Win7
  host harness under `drivers/windows7/tests/host-harness/` does this
  automatically.

| Device | PCI ID | Subsystem ID | Verified in |
|---|---|---|---|
| virtio-net | `1AF4:1041` | `0x0001` | `crates/devices/src/pci/profile.rs` (`VIRTIO_NET`) |
| virtio-blk | `1AF4:1042` | `0x0002` | same (`VIRTIO_BLK`) |
| virtio-input keyboard (fn 0) | `1AF4:1052` | `0x0010` | same (`VIRTIO_INPUT_KEYBOARD`, `header_type: 0x80`) |
| virtio-input mouse (fn 1) | `1AF4:1052` | `0x0011` | same (`VIRTIO_INPUT_MOUSE`) |
| virtio-input tablet (fn 2, optional) | `1AF4:1052` | `0x0012` | same (`VIRTIO_INPUT_TABLET`) |
| virtio-snd | `1AF4:1059` | `0x0019` | same (`VIRTIO_SND`, `subsystem_id: 25`) |

virtio-input is a single multi-function PCI device: function 0 (keyboard)
advertises multi-function via `header_type = 0x80`, function 1 is the mouse,
function 2 is an optional absolute-pointer tablet.

### Transport (modern-only)

- One BAR: **BAR0, 64-bit MMIO, little-endian, 0x4000 bytes**. No I/O BARs.
- Fixed capability layout (all in BAR0), so conformance can be checked without
  guessing:

| Capability | `cfg_type` | Offset | Length |
|---|---:|---:|---:|
| Common config | 1 | `0x0000` | `0x0100` |
| Notify | 2 | `0x1000` | `0x0100` |
| ISR | 3 | `0x2000` | `0x0020` |
| Device config | 4 | `0x3000` | `0x0100` |

- `notify_off_multiplier = 4`; `queue_notify_off(q) = q`; notify address =
  `notify_base + q * 4`. Devices accept 16- and 32-bit notify writes.
- Undefined BAR0 offsets: reads return 0, writes are ignored.
- Verified constants: `crates/devices/src/pci/profile.rs`
  (`VIRTIO_BAR0_SIZE = 0x4000`, `VIRTIO_COMMON_CFG_BAR0_OFFSET = 0x0000`,
  `VIRTIO_NOTIFY_CFG_BAR0_OFFSET = 0x1000`,
  `VIRTIO_ISR_CFG_BAR0_OFFSET = 0x2000`,
  `VIRTIO_DEVICE_CFG_BAR0_OFFSET = 0x3000`,
  `VIRTIO_NOTIFY_OFF_MULTIPLIER = 4`).
- Driver-side layout validation has two modes: **permissive** (default; accept
  any valid virtio-pci placement, keeps QEMU usable as a compat target) and
  **strict** (enforce the fixed layout; build the shared transport with
  `VIRTIO_CORE_ENFORCE_AERO_MMIO_LAYOUT=1` for emulator conformance testing).
- Common-config selector registers (`device_feature_select`,
  `driver_feature_select`, `queue_select`) are device-global; drivers must
  serialize multi-step selector sequences with a per-device lock. Out-of-range
  `queue_select` reads back zeros and ignores writes.
- Reset (`device_status = 0`) clears queues, interrupts, and negotiation state,
  and also reverts MSI-X vector registers to `0xFFFF` — drivers must reprogram
  vectors after every reset.

### Features, virtqueues, interrupts

- Required feature bits: `VIRTIO_F_VERSION_1` (32) on every device;
  `VIRTIO_F_RING_INDIRECT_DESC` (28) offered by all devices.
- **Not offered:** `VIRTIO_F_RING_EVENT_IDX` (29) and `VIRTIO_F_RING_PACKED`
  (34). Rings therefore have **no `used_event`/`avail_event` fields**; size
  formulas are `desc = 16N`, `avail = 4 + 2N`, `used = 4 + 8N`. Drivers must
  not compute pointers to the EVENT_IDX fields. Notification semantics are
  always-notify (plus `VRING_AVAIL_F_NO_INTERRUPT` suppression).
- Ring alignment: descriptor table 16-byte, avail 2-byte, used 4-byte. All
  ring/descriptor addresses are guest-physical DMA addresses; 64-bit addresses
  (>4 GiB) must work.
- Queue sizes (verified in `crates/aero-virtio/src/devices/`):

| Device | Queues | Size |
|---|---|---|
| virtio-blk | 0 `requestq` | 128 (`blk.rs`) |
| virtio-net | 0 `rxq`, 1 `txq` | 256 each (`net.rs` `queue_max_size`) |
| virtio-input | 0 `eventq`, 1 `statusq` | 64 each (`input.rs`) |
| virtio-snd | 0 `controlq`, 1 `eventq`, 2 `txq`, 3 `rxq` | 64/64/256/64 (`snd.rs`) |

- Interrupts: **INTx (INTA#) required**; ISR byte is read-to-ack (bit 0 queue,
  bit 1 config). MSI-X is permitted but, when enabled at the PCI layer, is
  **exclusive**: an unassigned (`0xFFFF`) or undeliverable vector means
  interrupts for that source are suppressed — no INTx fallback. Drivers must
  stay fully functional on INTx alone.

### Per-device essentials

- **virtio-blk**: `capacity` in 512-byte sectors; features `SEG_MAX`,
  `BLK_SIZE`, `FLUSH`; request types IN(0)/OUT(1)/FLUSH(4) only — everything
  else completes `VIRTIO_BLK_S_UNSUPP`. Data length must be a multiple of 512;
  1..`seg_max` data descriptors per request; status byte written before the
  used entry; `used.len = 0`. FLUSH must not complete until prior writes are
  durable.
- **virtio-net**: features `MAC` + `STATUS`; no control queue, no offload
  features, `MRG_RXBUF` not offered (the Win7 miniport may opportunistically
  request it against non-contract targets like stock QEMU). Classic 10-byte
  `virtio_net_hdr`, zeroed in both directions. Frames 14..1522 bytes; bad TX
  frames are dropped but the chain still completes; bad RX frames are dropped
  without consuming a chain. `used.len = 10 + frame_len` on RX.
- **virtio-input**: selector config (`ID_NAME`, `ID_DEVIDS`, `EV_BITS`);
  ID_NAME strings verified in `crates/aero-virtio/src/devices/input.rs`
  ("Aero Virtio Keyboard/Mouse/Tablet"); events are Linux input events with
  mandatory `EV_SYN/SYN_REPORT` batching; one 8-byte event per used entry.
  `statusq` buffers must be consumed/completed (LED modeling optional).
- **virtio-snd**: two fixed PCM streams — playback stereo S16_LE 48 kHz
  (stream 0, `txq`), capture mono S16_LE 48 kHz (stream 1, `rxq`); `eventq` is
  reserved (devices must retain posted buffers and never complete them without
  an event). PCM payload per buffer is capped at 256 KiB (`BAD_MSG` above) as
  a defensive bound. The in-tree driver tolerates `jacks = 2` and optional
  PCM/JACK events as best-effort enhancements.

### Conformance harness

- `drivers/protocol/virtio/` — host-buildable Rust crate locking struct
  sizes/offsets for both sides.
- `drivers/win7/virtio/tests/` — portable C99 capability-parser tests that run
  on Linux (`build_and_run.sh`); parser at
  `drivers/win7/virtio/virtio-core/portable/virtio_pci_cap_parser.{h,c}`.
- `drivers/windows7/tests/` — Win7 QEMU harness: guest selftest
  (`guest-selftest/`, emits `AERO_VIRTIO_SELFTEST|...` markers), host harness
  (`host-harness/Invoke-AeroVirtioWin7Tests.ps1` /
  `invoke_aero_virtio_win7_tests.py`,
  `New-AeroWin7TestImage.ps1`), with opt-in MSI-X requirements
  (`-RequireVirtio{Net,Blk,Snd,Input}Msix`, guest flags
  `--expect-blk-msi` / `--require-{net,snd,input}-msix`) and a QMP PCI-revision
  preflight (`-QemuPreflightPci`).

### Device contract data files (`protocol-vectors/`)

`protocol-vectors/windows-device-contract.json` (canonical; Aero service names like
`aero_virtio_blk`) and `protocol-vectors/windows-device-contract-virtio-win.json`
(upstream virtio-win names like `viostor`/`netkvm`) are the machine-readable
form of the identity contract. Known consumers:

- `tools/device_contract_validator/` (Rust validator),
  `tools/packaging/aero_packager/` (generates `config/devices.cmd`),
  `tools/guest-tools/validate_config.py`,
  `scripts/generate-guest-tools-devices-cmd.py` and
  `scripts/generate-windows-device-contract-virtio-win.py`,
  `scripts/ci/check-windows7-virtio-contract-consistency.py` and
  `check-windows-virtio-contract.py`
- Tests: `crates/devices/tests/windows_device_contract*.rs`,
  `crates/aero-protocol/tests/windows_device_contract.rs`,
  `crates/aero-protocol/tests/aerogpu_pci_id_conformance.rs`
- Packaging scripts: `drivers/build/package-guest-tools.ps1`,
  `drivers/build/virtio-win-packaging-smoke.ps1`, `drivers/scripts/make-guest-tools-*.ps1`

`guest-tools/config/devices.cmd` is generated from the canonical JSON (header
in the file names the generator); regenerate via
`scripts/generate-guest-tools-devices-cmd.py` and drift-check with
`python3 scripts/ci/gen-guest-tools-devices-cmd.py --check`. Do not edit the
canonical JSON to virtio-win service names — the virtio-win variant exists for
that.

## Guest-side driver sources

| Driver | Model | Path | INF / service |
|---|---|---|---|
| virtio-blk | StorPort miniport | `drivers/windows7/virtio-blk/` | `inf/aero_virtio_blk.inf` / `aero_virtio_blk` |
| virtio-net | NDIS 6.20 miniport | `drivers/windows7/virtio-net/` | `inf/aero_virtio_net.inf` / `aero_virtio_net` |
| virtio-input | KMDF | `drivers/windows7/virtio-input/` | `inf/aero_virtio_input.inf` (kbd/mouse), `inf/aero_virtio_tablet.inf` / `aero_virtio_input` |
| virtio-snd | WDM PortCls + WaveRT | `drivers/windows7/virtio-snd/` | `inf/aero_virtio_snd.inf` / `aero_virtio_snd` |
| AeroGPU | WDDM 1.1 KMD + D3D9/10/11 UMDs | `drivers/aerogpu/` | see graphics area page |

Notes:

- The virtio-input canonical INF binds subsystem-qualified keyboard/mouse
  HWIDs plus a strict `PCI\VEN_1AF4&DEV_1052&REV_01` fallback ("Aero VirtIO
  Input Device"); the tablet INF's more specific HWID wins when both packages
  are installed. `inf/virtio-input.inf.disabled` is a filename-only alias that
  must stay byte-identical from `[Version]` onward
  (`drivers/windows7/virtio-input/scripts/check-inf-alias.py` enforces this);
  enabling it changes nothing about HWID matching.
- `drivers/win7/virtio/virtio-transport-test/` is a KMDF smoke-test driver,
  deliberately **not** CI-packaged (no `ci-package.json`) and bound to the
  non-contract HWID `PCI\VEN_1AF4&DEV_1040` so it cannot steal production
  bindings.

### Shared transport and virtqueue code

- `drivers/windows/virtio/pci-modern/` — WDF-free modern transport
  (`VirtioPciModernTransport*`, STRICT vs COMPAT policy), used by the WDM
  virtio-snd driver.
- `drivers/windows7/virtio/common/` — miniport-friendly shim
  (`virtio_pci_modern_miniport.{h,c}`) taking a mapped BAR0 pointer plus a
  256-byte PCI config snapshot (NDIS: `NdisMGetBusData`; StorPort:
  `StorPortGetBusData`), used by virtio-blk/virtio-net; plus INTx/MSI-X
  helpers (`virtio_pci_intx_wdm`, `virtio_pci_msix_wdm`,
  `virtio_pci_interrupts_wdm`).
- `drivers/win7/virtio/virtio-core/` — KMDF-centric transport plus the
  portable capability parser and canonical `include/virtio_spec.h` layouts,
  used by virtio-input.
- Virtqueue engines: the canonical split-ring engine
  `drivers/windows/virtio/common/virtqueue_split.{c,h}` (virtio-input,
  virtio-snd, host tests) versus the legacy portable engine
  `drivers/windows7/virtio/common/{src/virtqueue_split_legacy.c,include/virtqueue_split_legacy.h}`
  (virtio-blk, virtio-net). `virtqueue_split.h` is unique in-tree by policy;
  CI guardrails `scripts/ci/check-win7-virtqueue-split-headers.py`,
  `check-virtqueue-split-driver-builds.py`, and
  `check-win7-virtio-header-collisions.py` keep include paths unambiguous.
- DMA/SG helpers: the WDF-free MDL/PFN→SG builder
  `drivers/windows/virtio/common/virtio_sg_pfn.{h,c}` (host-tested in
  `drivers/windows/virtio/common/tests/`) and the shared SG entry type
  `drivers/windows7/virtio/common/include/virtio_sg.h`. (The KMDF DMA helper
  module `windows-drivers/virtio-kmdf/common/` described in the old virtqueue
  guides does not exist in the tree — dropped as stale.)

### Driver-side implementation rules (the evergreen parts of the old guides)

- Rings and indirect tables live in DMA-visible nonpaged memory: KMDF uses
  `WdfCommonBufferCreateWithConfig` (uncached, page-aligned; program
  `WdfCommonBufferGetAlignedLogicalAddress`, never `MmGetPhysicalAddress`);
  WDM uses `MmAllocateContiguousMemorySpecifyCache(MmNonCached)` at
  PASSIVE_LEVEL. With indirect descriptors each in-flight request costs one
  ring descriptor; indirect tables must be single contiguous DMA regions.
- Publish ordering: write descriptors → write `avail->ring[slot]` →
  `KeMemoryBarrier()` → increment `avail->idx` → notify. Consume ordering:
  observe `used->idx` → `KeMemoryBarrier()` → read used entries. Per-head
  cookies are mandatory (used entries complete out of order).
- Clamp WDF SG lists with `WdfDmaEnablerSetMaximumScatterGatherElements` and
  size `MaximumLength` ≥ max request bytes so one request = one DMA
  programming phase. Keep transactions alive until the used entry arrives.
- On PnP stop/rebalance, quiesce DMA (stop submissions, reset device, finalize
  transactions) before freeing ring buffers; DMA adapter addresses must not be
  reused across a rebalance.
- Interrupts: INTx ISR must read the ISR byte first (read-to-ack; return
  "not ours" when 0). MSI-X requires INF opt-in
  (`Interrupt Management\MessageSignaledInterruptProperties` `MSISupported=1`,
  optional `MessageNumberLimit`); create one `WDFINTERRUPT` per message and
  program `msix_config`/`queue_msix_vector` with the **MessageNumber** (MSI-X
  table entry), not the APIC vector; read back after programming and after
  every reset; fall back to "config + all queues on vector 0" when Windows
  grants fewer messages than `1 + numQueues`. The shared KMDF helper
  `drivers/windows/virtio/kmdf/virtio_pci_interrupts.{c,h}` implements
  prepare/program/quiesce/resume. On x64, BARs above 4 GiB may arrive as
  `CmResourceTypeMemoryLarge` with scaled `Length40/48/64` fields — decode
  them back to bytes (handled in e.g. `drivers/windows7/virtio-net/src/aero_virtio_net.c`
  and `drivers/win7/virtio/virtio-core/src/virtio_pci_modern.c`).
- The Win7 virtio-snd driver is a PortCls adapter with a WaveRT streaming
  miniport plus a topology miniport; fixed-format pin data ranges (stereo 48
  kHz render, mono 48 kHz capture), a 10 ms-period software-DMA loop feeding
  `txq`, control flow `SET_PARAMS → PREPARE → START/STOP → RELEASE` at
  PASSIVE_LEVEL, and INF registration via `ks.inf`/`wdmaudio.inf` with
  `SubClasses = "wave,topology"`.

## Build, catalog, and sign pipeline (Windows host)

Per-driver opt-in manifest: `drivers/<driver>/ci-package.json` (schema
`drivers/build/driver-package.schema.json`, template `drivers/_template/`). Only drivers
with this manifest are built/staged/packaged. Fields: `infFiles` (explicit INF
allowlist), `wow64Files` (copy named x86 DLLs into the x64 package),
`requiredBuildOutputFiles`, `additionalFiles` (non-binary only; `.exe`
refused), `toolFiles` (explicit `.exe` opt-in), `wdfCoInstaller`
(`kmdfVersion` → `WdfCoInstaller0101N.dll`). Real example:
`drivers/aerogpu/ci-package.json`.

Pipeline order (PowerShell, Windows host):

1. `drivers/build/install-wdk.ps1` — provisions the pinned toolchain (Windows Kits
   **10.0.22621.0**) and writes `out/toolchain.json`.
2. `drivers/build/validate-toolchain.ps1` — smoke-tests `Inf2Cat /os:7_X86,7_X64` and
   locks down catalog scope: a staged unreferenced sentinel file must be
   absent from the `.cat` (prints `INF2CAT_UNREFERENCED_FILE_HASHED=0`).
3. `drivers/build/build-drivers.ps1` — MSBuild into `out/drivers/<driver>/<arch>/`
   (logs + `.binlog` under `out/logs/drivers/`).
4. `drivers/build/build-aerogpu-dbgctl.ps1` — builds/copies the AeroGPU debug helper;
   `aerogpu_dbgctl.exe` is intentionally an x86 binary everywhere (runs under
   WOW64 on x64; CI checks the PE header).
5. `drivers/build/make-catalogs.ps1` — stages `out/packages/<driver>/<arch>/` and runs
   Inf2Cat. Catalogs cover the INF plus INF-referenced payload files only;
   unreferenced files in the package directory are not hashed (and therefore
   not tamper-protected — sign such tools separately if needed). Treat staged
   packages as immutable after cataloging.
6. `drivers/build/sign-drivers.ps1` — test-signs `.sys`/`.cat`; default SHA-1 for stock
   Win7 compatibility, `-DualSign` appends SHA-256. The certificate itself
   must be SHA-1-signed for unpatched Win7; if the runner cannot create one,
   the script fails unless `-AllowSha2CertFallback` (then stock Win7 needs
   KB3033929/KB4474419). Public cert: `out/certs/aero-test.cer`; stable-cert
   injection via `AERO_DRIVER_PFX_BASE64`/`AERO_DRIVER_PFX_PASSWORD`.
7. `drivers/build/package-drivers.ps1` — driver bundles:
   `out/artifacts/AeroVirtIO-Win7-<version>-{x86,x64,bundle}.zip`, a
   deterministic `.iso` (Rust writer; `-LegacyIso` for IMAPI2), per-artifact
   `*.manifest.json` (sha256/size/version/signing policy/build id), and an
   optional FAT32 VHD (`-MakeFatImage` or `AERO_MAKE_FAT_IMAGE=1`, via
   `drivers/build/make-fat-image.ps1`; Windows + admin; `-FatImageStrict` to require it)
   for Windows Setup's "Load Driver" flow (`x86/`/`x64/` layout +
   `INSTALL.txt`).
8. `drivers/build/package-guest-tools.ps1` — Guest Tools media (next section).

Signing policy is threaded through packaging: `test` (default; bundles
`aero-test.cer`, INSTALL.txt includes test-signing steps), `production`/`none`
(no certs, no test-signing instructions). WDK redistributables
(`WdfCoInstaller*.dll`) are never included by default; opting in requires both
the manifest field and `drivers/build/make-catalogs.ps1 -IncludeWdfCoInstaller`, and CI
refuses checked-in coinstaller DLLs under `drivers/`.

INF requirements for cataloging: valid `[Version]` with `DriverVer` and a
`CatalogFile` name matching the generated `.cat` (per-arch
`CatalogFile.NTx86/.NTamd64` also valid).

## Guest Tools media

Producer: `tools/packaging/aero_packager/` (Rust). Entry points:

- `drivers/build/package-guest-tools.ps1` — from signed CI packages (`out/packages/` +
  `out/certs/`); spec `tools/packaging/specs/win7-signed.json` in CI,
  `win7-aero-guest-tools.json` (stricter HWID validation) as the local
  default. Convenience wrapper: `drivers/scripts/make-guest-tools-from-ci.ps1`.
- `drivers/scripts/make-guest-tools-from-aero-virtio.ps1` — from locally
  built in-tree virtio drivers (spec `win7-aero-virtio.json`, modern-only IDs;
  transitional `0x1000..` IDs are rejected at packaging time).
- `drivers/scripts/make-guest-tools-from-virtio-win.ps1` (+ `.sh`) — from an
  upstream `virtio-win.iso` (`tools/virtio-win/extract.py` for cross-platform
  extraction; profiles `full`/`minimal` with specs `win7-virtio-full.json` /
  `win7-virtio-win.json`; optional `vioinput`/`viosnd` are included only when
  present for **both** architectures unless `-StrictOptional`). Uses
  `protocol-vectors/windows-device-contract-virtio-win.json` so `setup.cmd` pre-seeding
  matches upstream `AddService` names.

Outputs: `aero-guest-tools.iso`, `aero-guest-tools.zip`, `manifest.json`
(+ `aero-guest-tools.manifest.json` alias) with `signing_policy`
(`test`/`production`/`none`), `certs_required`, per-file hashes, and build
metadata. Determinism via `SOURCE_DATE_EPOCH` (default: HEAD commit
timestamp); `build_id` defaults to the HEAD commit SHA. The packager validates
required drivers for both arches, HWID regexes, and INF-referenced file
existence; it excludes build debris (`*.pdb`, sources, project files) and
**hard-refuses private key material** (`*.pfx/*.pvk/*.snk/*.key/*.pem`) and
symlinks. Media root layout: `setup.cmd`, `uninstall.cmd`, `verify.cmd`,
`verify.ps1`, `README.md`, `THIRD_PARTY_NOTICES.md`, `manifest.json`,
`config/devices.cmd` (generated at packaging time), `certs/` (required for
`test`), `drivers/{x86,amd64}/<driver>/...`, optional `tools/`, `licenses/`.

### In-guest installer flow (`guest-tools/`)

Supported flow (see `guest-tools/README.md` for the authoritative reference):

1. Install Windows 7 SP1 on **baseline devices** (AHCI HDD + IDE/ATAPI CD-ROM,
   e1000, VGA, PS/2, HDA).
2. Mount `aero-guest-tools.iso`; run `setup.cmd` as Administrator (optionally
   `setup.cmd /check /verify-media` first). Depending on `signing_policy` it
   installs `certs\*.cer` into LocalMachine Root + TrustedPublisher, enables
   Test Signing on x64 when `test`, stages every INF under
   `drivers\{x86,amd64}\` via `pnputil`, and pre-seeds boot-critical
   virtio-blk state (service `Start=0` + `CriticalDeviceDatabase` keys for the
   HWIDs in `config\devices.cmd`). Override flags include `/stageonly`,
   `/testsigning`, `/notestsigning`, `/nointegritychecks` (not recommended),
   `/forcesigningpolicy:...`, `/noreboot`, `/skipstorage` (GPU-only media;
   **do not** switch the boot disk to virtio-blk after using it). Logs:
   `C:\AeroGuestTools\install.log`.
3. Reboot once on baseline devices.
4. Switch devices one at a time, in order: **AHCI → virtio-blk**, e1000 →
   virtio-net, VGA → Aero GPU, PS/2 → virtio-input, HDA → virtio-snd
   (audio/input optional). Rollback for any failure: power off, switch that
   device back, boot, re-run `setup.cmd`.
5. Run `verify.cmd` as Administrator; it writes
   `C:\AeroGuestTools\report.txt` / `report.json` and prints
   `Overall: PASS/WARN/FAIL` (exit 0/1/2+). It checks media integrity,
   signing policy vs BCD state, cert stores, KB3033929, per-device binding
   (including the `AERO_VIRTIO_SND_SERVICE` fallback list in
   `config\devices.cmd`), AeroGPU UMD placement, and boot-critical storage
   readiness.

Browser-runtime audio caveat: with `vmRuntime="legacy"` the I/O worker
attaches host AudioWorklet rings to HDA when present, so virtio-snd is the
active audio device only in HDA-less builds (or with explicit host-side
selection); `vmRuntime="machine"` does not currently expose guest audio to the
host audio stack. Keep HDA as the fallback.

## Install-media preparation and unattended install

> **The machine has exactly one optical slot.** `Machine` exposes a single
> `attach_ide_secondary_master_atapi` — IDE secondary master — and the CLI takes
> one `--install-iso`. Unattended install normally wants *two* discs mounted: the
> install media, and a small configuration disc carrying `autounattend.xml` at
> its root, which Setup finds by scanning removable media at startup. Today they
> cannot both be attached.
>
> Three ways out, none yet chosen: add a second ATAPI or AHCI optical slot,
> rebuild the install ISO with the answer file integrated, or attach the answer
> file as a floppy — for which no controller model exists. The decision can wait,
> because the first Setup boot is verified by watching the screen anyway.
>
> The answer file itself is prepared and checked against the real media: the
> install media's `install.wim` is single-edition, and
> `guest-tools/unattend/autounattend_amd64.xml` selects `/IMAGE/INDEX` 1
> accordingly, wipes disk 0 to a single NTFS partition, accepts the licence,
> creates the user, and covers all four Setup passes with `WillShowUI=OnError`.

Win7 x64 enforces driver signatures from boot (`winload.exe`), so test-signed
boot-critical drivers need three coordinated patches; x86 does not enforce
signing the same way.

1. **Media BCD stores** (`boot\BCD` BIOS; `efi\microsoft\boot\bcd` UEFI when
   present): `testsigning on` (preferred) or `nointegritychecks on` (lab
   only). Patch every boot-loader object, not just `{default}`.
2. **`boot.wim`** (index 2 = Setup, mandatory; index 1 = WinPE/WinRE,
   recommended): inject the test certificate into the offline SOFTWARE hive;
   optionally `dism /Add-Driver` so Setup can see virtio disks.
3. **`install.wim`** (every edition index to be installed): inject the
   certificate and patch `Windows\System32\Config\BCD-Template` so the
   installed OS inherits the boot policy. Nested
   `Windows\System32\Recovery\winre.wim` is an optional extra.

Tooling:

- `tools/windows/patch-win7-media.ps1` — one-command Windows-first patcher
  (BCD stores, cert injection via `win-offline-cert-injector`, `BCD-Template`,
  optional `-DriversPath`, index selection flags). Modifies files in place;
  work on a copy.
- `tools/bcd_patch/` — cross-platform BCD hive patcher
  (`cargo run -p bcd-patch -- win7-tree --root <tree>`); constants in
  `tools/bcd_patch/src/constants.rs`, object selection in `src/lib.rs`. BCD
  stores are REGF hives; the two boolean elements written are
  `0x16000048` (`nointegritychecks`) and `0x16000049` (`testsigning`), encoded
  as `[u32 type][u32 len=4][u32 bool]` under
  `Objects\{GUID}\Elements\<8-hex>\Element`. The patcher targets
  `{globalsettings}`, `{bootloadersettings}`, `{resumeloadersettings}`, all
  `winload*`/`winresume*` objects, and `{bootmgr}`-referenced entries.
  Auditable `.reg` equivalents live in `tools/win7-slipstream/patches/`
  (apply with `hivexregedit --merge`).
- `tools/win-offline-cert-injector/` — Windows-native CLI that loads the
  offline SOFTWARE hive and uses CryptoAPI to write the registry-backed cert
  store entry (`--windows-dir <mount> --store ROOT --store TrustedPublisher
  <cert>`). Prefer it over hand-written registry values: the `Blob` value is
  **not** raw DER. Byte-level format (verified by the `tools/win-blob-dump/`
  harness against `CertSerializeCertificateStoreElement`): 8-byte header
  (`dwCertEncodingType` `0x00010001`, `cbCertEncoded`), DER bytes, zero
  padding to 4-byte alignment, then `cProperties` + property entries
  (`dwPropId`, `cbValue`, value, padding). The cross-platform fallback
  `tools/win7-slipstream/scripts/cert-to-reg.py` writes raw DER and is
  best-effort only; `tools/win-certstore-regblob-export` generates
  CryptoAPI-correct `.reg` files on Windows for later cross-platform merge.

Unattended install:

- Recommended packaging is a separate **config ISO** (no Microsoft files):
  `autounattend.xml` at the root plus `Drivers/{WinPE,Offline}/{amd64,x86}/`,
  `Scripts/`, `Cert/`. Driver injection points:
  `Microsoft-Windows-PnpCustomizationsWinPE` (`windowsPE` pass, setup-critical
  storage/NIC) and `Microsoft-Windows-PnpCustomizationsNonWinPE`
  (`offlineServicing` pass, stages INF-based drivers into the installed OS;
  `.exe`/`.msi` installers need scripting instead).
- Script hooks: `specialize` → `RunSynchronous` (SYSTEM, first boot),
  `oobeSystem` → `FirstLogonCommands` (user), and
  `%WINDIR%\Setup\Scripts\SetupComplete.cmd` (SYSTEM, runs once). Production
  scripts in `guest-tools/unattend/scripts/` (`SetupComplete.cmd`,
  `InstallDriversOnce.cmd`) enable test signing, import the cert (accepting
  several file names), `pnputil`-install every INF under the Drivers folder,
  and avoid reboot loops with marker files + self-deleting scheduled tasks.
  Templates: `guest-tools/unattend/autounattend_{amd64,x86}.xml`; golden
  reference templates in `tools/win7-slipstream/templates/`.
- Reliability rule (validated by the old playbook): `%configsetroot%` and
  `$OEM$` processing from a secondary CD are **not** guaranteed. The robust
  pattern is a root marker file (`AERO_CONFIG.MEDIA`) + drive-letter scan +
  copying payloads to a stable path (`C:\Aero\`) during `specialize`; never
  build a flow that only works when `%configsetroot%` is set.
- Validation/triage: WinPE logs under `X:\Windows\Panther\` (and
  `UnattendGC\` for unattend selection), copied post-install to
  `C:\Windows\Panther\`; driver binding detail in
  `C:\Windows\inf\setupapi.dev.log` (search `!!!`); `Shift+F10` opens a WinPE
  command prompt at any Setup screen. Custom script logs:
  `C:\Windows\Temp\aero-setup.log`, `aero-driver-install.log`.

## Legacy and transitional virtio transport (optional compatibility)

Contract v1 is modern-only. For upstream virtio-win compatibility, the web
runtime can expose virtio-net/virtio-input/virtio-snd as **transitional** or
**legacy** devices instead: config keys `virtioNetMode`, `virtioInputMode`,
`virtioSndMode` = `"modern" | "transitional" | "legacy"` (also URL query
overrides; verified in `apps/web/src/config/aero_config.test.ts`). Changing the
mode changes guest-visible PCI IDs/BAR layout and needs a VM restart.
Transitional devices use the `0x1000..` legacy-compatible ID range (e.g.
`PCI_DEVICE_ID_VIRTIO_INPUT_TRANSITIONAL = 0x1011`,
`..._SND_TRANSITIONAL = 0x1018` in `crates/devices/src/pci/profile.rs`) and
expose the virtio 0.9 I/O-port register block (`HOST_FEATURES`,
`GUEST_FEATURES`, `QUEUE_PFN`, `QUEUE_NUM`, `QUEUE_SEL`, `QUEUE_NOTIFY`,
`STATUS`, `ISR_STATUS` at offsets `0x00..0x13`, then device config): legacy
negotiation sees only the low 32 feature bits and programs rings via
`QUEUE_PFN` with 4096-byte vring alignment. A transitional device should lock
into one transport per reset cycle (first meaningful write decides) and ignore
the other until reset. This mode exists for compatibility testing (e.g. older
virtio-win bundles, QEMU parity); Aero's own drivers must not rely on it.

## Troubleshooting quick map

- **Code 52** (signature): check `testsigning` (`bcdedit /enum {current}`),
  cert in LocalMachine Root + TrustedPublisher, KB3033929 for SHA-256,
  architecture match, guest clock. One-boot bypass: F8 → Disable Driver
  Signature Enforcement.
- **Catalog hash mismatch**: package changed after signing — regenerate and
  re-sign; or mixed/corrupt media — re-fetch and `setup.cmd /check
  /verify-media`.
- **`0x0000007B INACCESSIBLE_BOOT_DEVICE`** after AHCI → virtio-blk: storage
  not pre-seeded (or `/skipstorage` was used — check
  `C:\AeroGuestTools\storage-preseed.skipped.txt`); roll back to AHCI, re-run
  `setup.cmd`, verify `Start=0` + `CriticalDeviceDatabase` entries.
- **Code 28 / Code 10**: stage drivers (`setup.cmd` or `pnputil -i -a`), check
  the device reports `REV_01`, then resolve signing.
- **Windows Setup can't see the virtio disk**: use "Load Driver" with the
  driver ISO/FAT VHD, or slipstream into `boot.wim` — or just install on AHCI
  first.
- Logs: `C:\AeroGuestTools\{install.log,report.txt,report.json}`,
  `C:\Windows\inf\setupapi.dev.log`, `C:\Windows\Panther\`.
- GPU-specific diagnostics (`aerogpu_dbgctl.exe --status/--query-scanout/
  --dump-last-submit`, `NonLocalMemorySizeMB` budget): see the graphics area
  page; dbgctl ships inside the AeroGPU driver directory in packaged media.

## Open issues and debt

- CI workflow files referenced by docs (`.github/workflows/*.yml`) are absent
  from this checkout — pipeline orchestration is **[unverified]**; the `drivers/build/`
  and `scripts/ci/` scripts themselves exist.
- The contract JSONs live in `protocol-vectors/` as data consumed by tests
  and tools; everything else in this area now lives here.
- EVENT_IDX remains unimplemented by design in contract v1 (always-notify);
  any future adoption is a contract minor/major revision.
- virtio-snd activation in the browser runtime is HDA-preferring (see the
  caveat above) — a real host-side selection mechanism is still open.
- Legacy/transitional transport is maintained only as an opt-in compatibility
  mode; there is no in-tree Win7 driver targeting it.

## Pointers

- Contract (machine-readable): `protocol-vectors/windows-device-contract.json`,
  `protocol-vectors/windows-device-contract-virtio-win.json`
- Emulator device models: `crates/aero-virtio/`,
  `crates/devices/src/pci/profile.rs`
- Guest drivers: `drivers/windows7/virtio-{blk,net,input,snd}/`,
  `drivers/win7/virtio/`, `drivers/aerogpu/`
- Shared driver code: `drivers/windows/virtio/`, `drivers/windows7/virtio/common/`,
  `drivers/protocol/virtio/`
- Harnesses: `drivers/windows7/tests/`, `drivers/win7/virtio/tests/`
- Pipeline: `drivers/build/*.ps1`, `drivers/build/driver-package.schema.json`, `drivers/_template/`,
  `tools/packaging/`
- Guest Tools: `guest-tools/`, `tools/guest-tools/validate_config.py`,
  `scripts/generate-guest-tools-devices-cmd.py`
- Install media: `tools/windows/patch-win7-media.ps1`, `tools/bcd_patch/`,
  `tools/win-offline-cert-injector/`, `tools/win-certstore-regblob-export/`,
  `tools/win-blob-dump/`, `tools/win7-slipstream/`, `guest-tools/unattend/`
- virtio-win compatibility: `drivers/scripts/make-*.ps1`, `tools/virtio-win/`,
  `tools/driver-iso/`

## Source docs absorbed

Content of the following `docs/` files is absorbed into this page (evergreen
parts only; step-by-step command transcripts, duplicated checklists, licensing
discussion, and release ceremony were dropped):

- `windows-drivers.md`, `windows-drivers.md`
  → transport contract + legacy mode sections
- `../specs/windows7-virtio-driver-contract.md` → contract section (condensed; the
  binding text remains the JSON + code)
- `../specs/windows7-virtio-driver-contract.md`,
  `../specs/windows7-virtio-driver-contract.md`,
  `windows-drivers.md`, `windows-drivers.md`,
  `docs/windows/virtio-pci-modern-{wdm,interrupts,interrupt-debugging}.md`,
  `windows-drivers.md`,
  `windows-drivers.md` → driver-source and
  implementation-rules sections
- `windows-drivers.md`,
  `windows-drivers.md`, `windows-drivers.md`
  → pipeline section
- `windows-drivers.md`, `windows-drivers.md`,
  `windows-drivers.md`, `windows-drivers.md` → Guest
  Tools section
- `windows-drivers.md`, `windows-drivers.md`,
  `windows-drivers.md`, `windows-drivers.md`,
  `windows-drivers.md`, `windows-drivers.md`,
  `windows-drivers.md` → install-media section
- `windows-drivers.md` → troubleshooting quick map
- `../specs/windows7-d3d-umd-ddi.md` was already a
  deprecated redirect to `docs/graphics/` (graphics area); nothing absorbed
  here.

## Contract fixes applied (2026-07-27)

See `wiki/notes/bring-up-findings-divergence-and-sight-audits.md` §6 for details.

- **virtio-blk**: removed DISCARD + WRITE_ZEROES feature bits per Win7 driver
  contract §3.1.3 (device MUST NOT offer them); zeroed config fields 0x24-0x38
  per §3.1.4 (MUST read as 0).
