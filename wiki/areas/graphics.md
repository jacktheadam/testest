# Graphics

> The graphics stack end to end: the boot display, the AeroGPU virtual GPU and
> its Windows 7 WDDM driver, host-side Direct3D translation onto WebGPU, and
> browser presentation.
>
> The normative contracts live in `wiki/specs/`: the device ABI, the command
> stream, the WDDM driver architecture, the D3D translation rules, and the
> compute-expansion emulation used for stages WebGPU does not have. This page
> is the map and the current state; those pages are the law.

## What it is

Aero renders a Windows 7 guest in the browser. The graphics stack has four
layers, each of which exists in-tree today:

1. **Boot display** — a legacy VGA + Bochs VBE device model for BIOS POST,
   bootloader, and pre-driver Windows boot graphics.
2. **AeroGPU virtual GPU** — a custom PCI display device (`A3A0:0001`) with a
   versioned guest↔host ABI (MMIO registers, submission ring, fence model,
   command stream), plus the in-tree Win7 WDDM 1.1 KMD/UMD driver package that
   binds to it.
3. **Host-side translation** — Rust crates that decode D3D9 (SM2/SM3) and
   D3D10/11 (DXBC SM4/SM5) command streams and execute them on wgpu/WebGPU.
4. **Browser presentation** — a GPU worker that owns the canvas
   (`OffscreenCanvas`), presents via WebGPU (preferred) or WebGL2 (fallback),
   and reads guest pixels through shared-memory contracts.

The canonical guest path is: Windows D3D runtime → AeroGPU UMD (builds the
AeroGPU command stream) → dxgkrnl → AeroGPU KMD (writes ring submissions) →
emulator device model → host executor (WebGPU/wgpu) → scanout presented to the
browser canvas.

## Current state (verified)

| Area | State | Where |
|---|---|---|
| Boot display (VGA text, mode 13h, VBE LFB) | Implemented, tested — and carried Windows 7 to a [rendered desktop](../history/windows-7-bring-up.md) natively, which is the strongest validation this path has | `crates/aero-gpu-vga/`, wired in `crates/aero-machine/` |
| AeroGPU ABI (C headers + Rust/TS mirrors + ABI drift tests) | Implemented, tested | `drivers/aerogpu/protocol/`, `crates/aero-protocol/` |
| AeroGPU device in canonical machine (BAR0/BAR1, ring decode, scanout/vblank/cursor, error-info) | Implemented (MVP); command execution is external or optional | `crates/aero-machine/src/aerogpu.rs` |
| Scanout/cursor/framebuffer shared-memory contracts | Implemented, tested | `crates/aero-shared/`, `apps/web/src/ipc/` |
| Browser presenters (WebGPU, raw WebGL2, wgpu-over-WebGL2) | Implemented | `apps/web/src/gpu/` |
| D3D9 translation + execution | Partial subset | `crates/aero-d3d9/`, `crates/aero-gpu/src/aerogpu_d3d9_executor.rs` |
| D3D10/11 translation + execution | Partial subset (VS/PS/CS; GS compute-prepass for a small topology set) | `crates/aero-d3d11/` |
| Win7 guest driver stack (KMD + D3D9Ex and D3D10/11 UMDs + tests) | Implemented in-tree; guest-side suite exists | `drivers/aerogpu/` |
| **End-to-end Win7 WDDM + accelerated rendering in the canonical browser machine** | **Not verified** — driver install → submissions → execution → scanout → DWM stability has never been validated end-to-end in the browser runtime | see “Open issues” below |

## Browser runtime topology

Three execution contexts cooperate over `SharedArrayBuffer`s:

- **Main thread / coordinator** (`apps/web/src/runtime/coordinator.ts`) — owns the
  visible canvas, allocates shared segments
  (`apps/web/src/runtime/shared_layout.ts`), routes AeroGPU submissions and fence
  completions between workers, schedules GPU worker ticks
  (`apps/web/src/main/frameScheduler.ts`).
- **CPU worker** (`apps/web/src/workers/machine_cpu.worker.ts`) — runs the
  `aero-wasm` machine; the guest-visible AeroGPU PCI/MMIO device model lives
  here.
- **GPU worker** (`apps/web/src/workers/gpu-worker.ts`) — owns the WebGPU/WebGL2
  context, presents frames, executes drained AeroGPU command streams
  (`handleSubmitAerogpu`), and reads scanout/cursor pixels from guest RAM or
  the shared VRAM aperture.

The submission bridge flow: guest rings the BAR0 doorbell → device model
decodes ring entries → CPU worker calls `Machine.aerogpu_drain_submissions()`
→ coordinator forwards `submit_aerogpu` to the GPU worker → GPU worker
executes and replies `submit_complete` → coordinator calls
`Machine.aerogpu_complete_fence(fence)`. Robustness rule: if a submission
cannot be delivered (bounded queue overflow while the GPU worker is not ready,
postMessage failure, worker restart), the coordinator **force-completes the
fence** so the guest cannot deadlock or TDR; rendering is best-effort in those
failure modes. Covered by `apps/web/src/runtime/coordinator.test.ts` and
`tests/e2e/web/gpu_submit_aerogpu.spec.ts`.

## Boot display: VGA/VBE

- Device model: `crates/aero-gpu-vga/` (`VgaDevice`) — VGA register file, 80×25
  text mode with built-in font and cursor, mode 13h, and Bochs/QEMU-style
  `VBE_DISPI` with a linear framebuffer.
- Canonical machine wiring: `MachineConfig::enable_vga` in
  `crates/aero-machine/src/lib.rs`. With `enable_pc_platform=true` the machine
  exposes a minimal Bochs-compatible “Standard VGA” PCI function (currently
  `00:0c.0`, `1234:1111`) and routes the VBE LFB through its BAR0; the
  BIOS-assigned BAR base is mirrored into the VBE `PhysBasePtr` and the device
  model so guests see a coherent LFB base. With the PC platform off, the LFB is
  mapped directly at the configured base.
- The host can snapshot the display via `Machine::display_present()` /
  `display_framebuffer()` / `display_resolution()`.
- Tests: unit tests in `crates/aero-gpu-vga/` (text/mode13h/VBE golden hashes),
  machine tests (`crates/aero-machine/tests/boot_int10_*.rs`,
  `vga_vbe_lfb_pci.rs`), and the CI script `scripts/ci/run-vga-vbe-tests.sh`.

`enable_vga` and `enable_aerogpu` are mutually exclusive machine
configurations.

## AeroGPU device and ABI

### Canonical identity and ABI sources

- PCI identity: `VID:DID = A3A0:0001` at BDF `00:07.0`, VGA-compatible class
  (`0x03/0x00/0x00`). Contract: this page (absorbed from `../specs/aerogpu-device-abi.md`).
- **Normative ABI = the C headers** (headers win over prose):
  - `drivers/aerogpu/protocol/aerogpu_pci.h` — IDs, BAR layout, MMIO register
    map, feature bits, `enum aerogpu_format`, error codes.
  - `drivers/aerogpu/protocol/aerogpu_ring.h` — ring header, submit descriptor,
    allocation table, fence page.
  - `drivers/aerogpu/protocol/aerogpu_cmd.h` — the command stream (“AeroGPU
    IR”, `ACMD` magic).
  - `drivers/aerogpu/protocol/aerogpu_escape.h`,
    `aerogpu_wddm_alloc.h`, `aerogpu_umd_private.h`, `vblank.md`,
    `allocation-table.md`.
- Rust + TypeScript mirrors: `crates/aero-protocol/aerogpu/*.rs|*.ts` (crate
  `aero-protocol`), with drift/conformance tests under
  `crates/aero-protocol/tests/`.
- Current ABI version: 1.4 (`AEROGPU_ABI_MAJOR/MINOR` in `aerogpu_pci.h`).
- BAR0: 64 KiB MMIO register block. BAR1: 64 MiB prefetchable VRAM aperture
  used for legacy VGA/VBE compatibility; the in-tree Win7 driver treats the
  adapter as system-memory-backed, so BAR1 sits outside the WDDM memory model.

### BAR0 register surface (summary)

- Discovery: `MAGIC` (`"AGPU"`), `ABI_VERSION`, `FEATURES_LO/HI`
  (`FENCE_PAGE`, `CURSOR`, `SCANOUT`, `VBLANK`, `TRANSFER`, `ERROR_INFO`).
- Ring transport: `RING_GPA_LO/HI`, `RING_SIZE_BYTES`, `RING_CONTROL`
  (enable/reset), `DOORBELL`.
- Completion: `COMPLETED_FENCE_LO/HI` (always available) plus an optional
  shared fence page (`FENCE_GPA_LO/HI`); `AEROGPU_IRQ_FENCE`.
- Interrupts: `IRQ_STATUS`/`IRQ_ENABLE`/`IRQ_ACK` (W1C); causes: fence,
  scanout vblank, error.
- Error latches (ABI 1.3+, behind `ERROR_INFO`): `ERROR_CODE`,
  `ERROR_FENCE_LO/HI`, `ERROR_COUNT` — latched when `AEROGPU_IRQ_ERROR`
  asserts, not cleared by IRQ ack.
- Scanout0 (behind `SCANOUT`): `SCANOUT0_ENABLE` (0x0400), `WIDTH`, `HEIGHT`,
  `FORMAT`, `PITCH_BYTES`, `FB_GPA_LO/HI`.
- Vblank timing (behind `VBLANK`): `SCANOUT0_VBLANK_SEQ_LO/HI`,
  `VBLANK_TIME_NS_LO/HI`, `VBLANK_PERIOD_NS`; semantics in
  `drivers/aerogpu/protocol/vblank.md`.
- Hardware cursor (behind `CURSOR`): `CURSOR_*` position/hotspot/format/FB/pitch.

### Ring, fences, and command stream

- Single shared ring in guest memory (`"ARNG"` header, power-of-two slots,
  monotonic `head`/`tail`), 64-byte `aerogpu_submit_desc` per slot carrying
  `cmd_gpa/cmd_size_bytes`, an optional per-submit allocation table, and a
  64-bit `signal_fence`. Submit = write desc, bump `tail`, write `DOORBELL`.
- Fences are monotonic 64-bit values; completion is observable via MMIO and the
  optional fence page, with `AEROGPU_IRQ_FENCE` unless the submission sets
  `NO_IRQ`. The Win7 KMD extends WDDM’s 32-bit submission fence into this
  64-bit device fence and reports only the low 32 bits back to dxgkrnl.
- Command buffers are `ACMD` streams of `{opcode, size_bytes}` packets.
  Forward-compat rules: skip unknown opcodes by `size_bytes`; known packets
  may grow by append-only tails (e.g. `BIND_SHADERS` grows `{gs,hs,ds}`
  handles after its 24-byte prefix; appended handles are authoritative).
- Extended stages (ABI 1.3+): GS binds directly via
  `shader_stage = GEOMETRY`; HS/DS (and a GS compatibility encoding) use the
  `stage_ex` tag carried in reserved fields when `shader_stage == COMPUTE`.
  Mirror helpers: `AerogpuShaderStageEx` in
  `crates/aero-protocol/aerogpu/aerogpu_cmd.rs`.

### Device model in the canonical machine

`crates/aero-machine/src/aerogpu.rs` implements BAR0 (register model, ring
decode, submission capture, fence/vblank/scanout/cursor state, error latches);
`crates/aero-machine/src/lib.rs` wires BAR1 VRAM and legacy VGA decode
(`0xA0000..0xBFFFF` window aliasing into VRAM, permissive VGA port I/O) and
host presentation (`display_present_aerogpu_scanout`).

Execution is deliberately external to the device model. Three executor modes
(`aero-machine`):

1. **No-op bring-up (default):** fences complete without executing ACMD so
   early guests cannot wedge; vsync PRESENTs may still be paced to vblank.
2. **Submission bridge:** `Machine::aerogpu_enable_submission_bridge()` +
   `aerogpu_drain_submissions()` / `aerogpu_complete_fence()`; the browser
   runtime uses this to execute in the GPU worker. The device model still owns
   fence pacing: once the host reports completion, vsync-present fences become
   visible on the next vblank. Hosts should report completion when execution
   finishes rather than faking vsync timing.
3. **In-process backends (native/tests):** `aerogpu_set_backend_immediate()`,
   `aerogpu_set_backend_null()`, and a feature-gated wgpu backend
   (`aero-machine/aerogpu-wgpu-backend`, smoke test
   `crates/aero-machine/tests/aerogpu_wgpu_backend_smoke.rs`). Mutually
   exclusive with the bridge; preserved across `Machine::reset()`.

A shared device-side library (regs/ring/executor + reusable PCI wrapper +
optional native wgpu backend) lives in `crates/aero-devices-gpu/`; a legacy
CPU rasterizer for the same command stream lives in `crates/aero-gpu-software`.

### Guest-memory backing: `backing_alloc_id`

Resources created by `CREATE_BUFFER` / `CREATE_TEXTURE2D` name their guest
backing via a **stable per-allocation** `backing_alloc_id` (`u32`; `0` = host
allocated) — never a slot index into the current submission’s allocation
table, whose ordering is not stable. Each submission’s sideband allocation
table maps `alloc_id → {gpa, size_bytes, flags}`; hosts resolve by ID, validate
bounds, and treat a missing ID as a validation error. `flags` carries
`AEROGPU_ALLOC_FLAG_READONLY` (derived on Win7 from the `DXGK_ALLOCATIONLIST`
write-intent bit) so hosts can reject writeback into read-only allocations.
Re-`CREATE_*` of an existing handle rebinds backing only if all immutable
properties match. Host-side enforcement: `crates/aero-gpu/src/command_processor.rs`.

### Shared surfaces: `share_token`

D3D9Ex/DWM and DXGI cross-process sharing is keyed by a stable 64-bit
`share_token`, **not** by the user-mode shared `HANDLE` numeric value (NT
handles are process-local; token-style handles are not duplicatable and may
collide with real handles). The Win7 KMD generates the token and persists it in
the WDDM allocation private-data blob (`aerogpu_wddm_alloc_priv.share_token` in
`drivers/aerogpu/protocol/aerogpu_wddm_alloc.h`), which dxgkrnl replays
verbatim on cross-process open. The blob also carries the stable `alloc_id`,
size, and row-pitch metadata needed by cross-API consumers (DWM opening a
D3D10/11 texture, D3D10/11 opening a D3D9Ex surface).

Host contract (`EXPORT_SHARED_SURFACE` / `IMPORT_SHARED_SURFACE` /
`RELEASE_SHARED_SURFACE` / `DESTROY_RESOURCE`):

- export creates the `share_token → resource` mapping; import returns an alias
  handle and refcounts the underlying resource;
- token collisions, imports of unknown/released tokens, and token reuse after
  release fail deterministically — but host failures must not block fence
  completion (the submission still completes with an error indication);
- the KMD emits `RELEASE_SHARED_SURFACE` when the last cross-process
  allocation wrapper closes (a best-effort internal submission that must not
  advance the OS-visible fence; hosts must tolerate duplicate fence values).
- MVP restriction: shared resources must map to exactly one WDDM allocation
  (`NumAllocations == 1`; shared full-mip-chain textures are rejected).

Host bookkeeping is centralized in `crates/aero-gpu/src/shared_surface.rs`
(`SharedSurfaceTable`), used by both the D3D9 and D3D11 executors. Guest-side
validation lives under `drivers/aerogpu/tests/win7/` (e.g.
`d3d9ex_shared_surface_ipc`, `d3d9ex_shared_surface_stress`,
`d3d9ex_shared_surface_wow64`, `d3d9ex_shared_allocations`,
`d3d11_shared_surface_ipc`, `d3d10_shared_surface_ipc`).

### Legacy VGA/VBE compatibility and the boot → WDDM scanout handoff

AeroGPU owns both the legacy boot display and the WDDM path so no second
“legacy VGA” adapter is needed:

- VRAM layout in BAR1: `0x00000..0x3FFFF` reserved for legacy VGA planar/text
  backing; the VBE linear framebuffer starts at
  `BAR1_BASE + 0x40000` (`AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES`), and the BIOS
  reports that as `PhysBasePtr` in **canonical** AeroGPU mode.
- **Bring-up STDVGA** (`AERO_AEROGPU_STDVGA_IDS=1`): IDs `1234:1111`, BAR0 =
  16 MiB LFB alias (pixel 0 at BAR0+0 → `VRAM[VBE_LFB_OFFSET..]`), VBE
  **PhysBasePtr = BAR0**, **BAR1 cleared**, and PCI command `IO|MEM` because a
  class `03/00` VGA-compatible function decodes fixed legacy I/O ranges even
  without an I/O BAR. Dual 16+64 MiB framebuffer apertures correlated with
  cold session `0xC000021A`/`STATUS_NO_MEMORY` — see
  `wiki/notes/stdvga-gdi-session-residual-2026-07-28.md`. Modern QEMU
  standard VGA also has a separate 4 KiB BAR2 control window, so “single-BAR”
  is not its literal topology; Aero's BAR2/revision-2 parity remains
  unimplemented and must be isolated separately. Legacy PCI I/O enablement
  produced no change when applied at the pre-framebuf +1B checkpoint, but
  static `videoprt.sys` analysis proves it is required during
  `VgaFindAdapter`; the late replay is not a cold-rule rejection until its
  timing is established. Tests: `aerogpu_stdvga_pci_ids`,
  `bios_post_enables_fixed_legacy_io_decode_for_vga_compatible_controllers`.
- The `0xA0000..0xBFFFF` window aliases into VRAM: a linear alias when VBE is
  inactive; with VBE active, `0xA0000..0xAFFFF` becomes the 64 KiB banked
  window into the VBE region.
- Scanout ownership is **sticky**: once scanout0 is claimed by a valid config
  with `SCANOUT0_ENABLE=1`, WDDM owns scanout until VM reset. Writing
  `SCANOUT0_ENABLE=0` is a visibility toggle — it blanks output, stops vblank
  pacing, and publishes a *disabled WDDM descriptor* (base/width/height/pitch
  = 0) — it does not hand scanout back to legacy output. Legacy INT 10h mode
  changes cannot steal scanout while WDDM holds it.
- There is no separate COMMIT register: the `SCANOUT0_ENABLE` write is the
  commit point (with the HI half of `FB_GPA` treated as the commit point for
  the 64-bit address), and invalid configs (e.g. `FB_GPA=0`) must not claim
  scanout.
- Rust tests: `crates/aero-machine/tests/aerogpu_scanout_handoff.rs`,
  `aerogpu_scanout_disable_publishes_wddm_disabled.rs`.

### Format semantics (scanout + cursor)

`SCANOUT0_FORMAT` / `CURSOR_FORMAT` / `ScanoutState.format` carry `enum
aerogpu_format` discriminants (`aerogpu_pci.h`), not a bespoke enum:

- `*X8*` formats carry no alpha; consumers force `A = 0xFF`.
- `*_SRGB` variants are layout-identical to UNORM; only interpretation
  differs — presenters must not double-apply gamma. The GPU worker decodes
  sRGB→linear after swizzle so blending/compositing happens in linear space.

## Browser presentation

### Shared-memory contracts

Three distinct `SharedArrayBuffer` structures, each with a Rust definition and
a TypeScript mirror:

- **`SharedFramebuffer`** — double-buffered RGBA8 framebuffer with an atomic
  header and optional per-slot dirty-tile bitsets. Producer publishes by
  flipping `active_index` and bumping `frame_seq` (canonical ordering in
  `SharedFramebufferWriter::write_frame`); `frame_dirty` is a
  producer→consumer liveness flag that consumers clear as an ACK.
  Rust: `crates/aero-shared/src/shared_framebuffer.rs`; TS:
  `apps/web/src/ipc/shared-layout.ts`; Rust consumer helper:
  `crates/aero-gpu/src/frame_source.rs`.
- **`ScanoutState`** — a seqlock descriptor (busy bit in `generation`) of the
  current scanout source: `LEGACY_TEXT` / `LEGACY_VBE_LFB` / `WDDM`, plus base
  guest-physical address, geometry, and format. It lets the GPU worker keep
  presenting when the legacy framebuffer is idle (e.g. after WDDM takes over).
  Rust: `crates/aero-shared/src/scanout_state.rs`; TS:
  `apps/web/src/ipc/scanout_state.ts`. Consumers use bounded-retry snapshots
  (`trySnapshotScanoutState`) so present loops recover from contention.
- **`CursorState`** — same seqlock pattern for the hardware cursor.
  Rust: `crates/aero-shared/src/cursor_state.rs`; TS:
  `apps/web/src/ipc/cursor_state.ts`.

A separate small “frame status” SAB (`apps/web/src/ipc/gpu-protocol.ts`,
`FRAME_STATUS_INDEX`/`FRAME_DIRTY`/`FRAME_PRESENTED` + metrics) is used for
main-thread↔GPU-worker tick pacing — distinct from the framebuffer header’s
`frame_dirty` despite the similar name.

### GPU worker presentation paths

- Legacy framebuffer path: validate header → select active slot → derive dirty
  rects (no dirty bits with tracking enabled = full-frame dirty) → upload full
  frame or dirty rects (`chooseDirtyRectsForUpload` in
  `apps/web/src/gpu/dirty-rect-policy.ts`) → clear `frame_dirty`.
- Scanout readback path: for `source = WDDM` or `LEGACY_VBE_LFB` with a real
  `base_paddr`, the worker reads pixels from the VRAM SAB (BAR1 backing) or
  guest RAM and normalizes to tightly packed RGBA8
  (`tryReadScanoutFrame`/`tryReadScanoutRgba8`; helper
  `apps/web/src/runtime/scanout_readback.ts`). Supported scanout formats today:
  32bpp `B8G8R8X8/A8`, `R8G8B8X8/A8` (+ sRGB variants) and 16bpp `B5G6R5`,
  `B5G5R5A1`. `base_paddr == 0` with WDDM source is either the *disabled*
  descriptor (all geometry zero) or a harness-only stand-in (non-zero
  geometry); legacy VBE always expects a real framebuffer.
- Main-thread fallback presenter for the legacy framebuffer (no GPU worker /
  no OffscreenCanvas): `apps/web/src/display/shared_layout_presenter.ts`
  (`SharedLayoutPresenter`, 2D canvas).
- Validation harnesses: `apps/web/wddm-scanout-debug.html` (interactive),
  `apps/web/wddm-scanout-smoke.html` / `apps/web/wddm-scanout-vram-smoke.html`
  (Playwright: `tests/e2e/wddm_scanout_smoke.spec.ts`,
  `tests/e2e/wddm_scanout_vram_smoke.spec.ts`).

### Guest-physical address contract (web runtime)

- The web runtime reserves VRAM at a fixed guest-physical address:
  `VRAM_BASE_PADDR = PCI_MMIO_BASE = 0xE000_0000` (`apps/web/src/arch/guest_phys.ts`,
  `crates/aero-wasm/src/guest_layout.rs`), inside the Q35-style PCI/MMIO hole.
  This is a web-runtime contract; in the canonical machine model, BAR bases
  are BIOS-assigned and dynamic.
- Guest RAM is clamped to `≤ PCI_MMIO_BASE`; above `LOW_RAM_END`
  (`0xB000_0000`) the physical map splits into low RAM / hole / high RAM
  (remapped above 4 GiB within the same wasm linear memory). Any code reading
  guest RAM by physical address must use the translation helpers:
  `apps/web/src/arch/guest_ram_translate.ts` (JS) and
  `crates/aero-wasm/src/guest_phys.rs` (Rust). This is why a scanout
  `base_paddr` may legitimately point into the PCI/MMIO hole.
- VRAM is a dedicated SAB (not wasm linear memory) so the I/O worker can
  service BAR1 MMIO and the GPU worker can read scanout/cursor pixels without
  copying; other MMIO BARs are allocated after the VRAM range. Default size
  64 MiB (`DEFAULT_VRAM_MIB`; `vramMiB=0` disables it in tests).

### Presenter backends and color policy

- Backends: WebGPU (`apps/web/src/gpu/webgpu-presenter-backend.ts`), raw WebGL2
  (`apps/web/src/gpu/raw-webgl2-presenter-backend.ts`), and wgpu-over-WebGL2 via
  WASM (`apps/web/src/gpu/wgpu-webgl2-presenter.ts`). Selection inputs:
  `GpuRuntimeInitOptions` in `apps/web/src/ipc/gpu-protocol.ts` (`forceBackend`,
  `disableWebGpu`, `preferWebGpu`); selection logic:
  `initPresenterForRuntime()` in `apps/web/src/workers/gpu-worker.ts`.
- Source convention everywhere: RGBA8 bytes, top-left origin (enforced in the
  blit shaders under `apps/web/src/gpu/shaders/`).
- Alpha policy: canvas configured opaque (`alphaMode: "opaque"` on WebGPU;
  `{ alpha: false, premultipliedAlpha: false }` on WebGL2).
- Validation presenters (`apps/web/src/gpu/webgpu-presenter.ts`,
  `apps/web/src/gpu/raw-webgl2-presenter.ts`) pin color-space (linear vs sRGB) and
  alpha behavior for deterministic WebGPU↔WebGL2 comparison; Playwright
  coverage: `tests/e2e/web/gpu_color.spec.ts`,
  `gpu_worker_presented_color_policy.spec.ts`,
  `gpu_hardware_cursor_state.spec.ts`, plus `screenshot_presented` readback in
  `apps/web/src/ipc/gpu-protocol.ts` and the test card in `apps/web/src/gpu/test-card.ts`.
- The WebGL2 fallback is intentionally limited: no compute shaders, no storage
  buffers, fewer binding slots, inconsistent float/compressed-texture support —
  it is a “present frames and run 2D” path; full D3D10/11 translation is out
  of scope for it. wgpu’s GL backend does not request BC/ETC2/ASTC compression
  by default (CPU decompression fallbacks instead); native/headless runs can
  also force compression off via `AERO_DISABLE_WGPU_TEXTURE_COMPRESSION=1`.

## Host-side D3D translation

### D3D9 (SM2/SM3) — `crates/aero-d3d9`

- Pipeline: bytecode → SM3 IR → WGSL (`crates/aero-d3d9/src/sm3/`), with a
  legacy token-stream fallback translator (`crates/aero-d3d9/src/shader.rs`); a
  standalone legacy parser remains at `crates/legacy/aero-d3d9-shader/` (not
  used by the runtime).
- Execution: `crates/aero-gpu/src/aerogpu_d3d9_executor.rs` consumes `ACMD`
  packets on wgpu. Shared-surface bookkeeping: `SharedSurfaceTable`.
- Implemented (tested) features include: SM3 texture sampling variants,
  `texkill` with predication, derivatives (`dsx`/`dsy`) and gradient sampling,
  int/bool constant banks, PS `MISCTYPE` builtins (vPos/vFace), PS depth output
  (`oDepth` → `frag_depth`), semantic-based VS input location mapping, and the
  D3D9 half-pixel center convention (injected `@group(3) @binding(0)`
  `HalfPixel` uniform, updated on `SetViewport`).
- Binding contract: `@group(0)` = shared VS/PS constants UBO (float/int/bool
  banks), `@group(1)` = VS samplers/textures, `@group(2)` = PS
  samplers/textures (sampler `sN` → bindings `2N`/`2N+1`).
- Translation caches: in-memory (`ShaderCache`) and a WASM-only persistent
  cache (`crates/aero-d3d9/src/runtime/shader_cache.rs` +
  `apps/web/gpu-cache/persistent_cache.ts`).
- Known gaps: the command stream only creates guest-backed 2D textures (cube
  via `array_layers=6`), so 1D/3D samplers are backed by dummy textures;
  comparison samplers are not modeled; the SM3 IR builder rejects some
  control-flow/addressing forms.
- The Win7 D3D9Ex UMD additionally implements the fixed-function fallback:
  built-in WVP vertex-shader variants per FVF with the matrix uploaded to
  reserved constant range `c240..c243` (plus a minimal lighting subset at
  `c208..c236`), CPU `XYZRHW` → clip-space conversion for pre-transformed
  draws, and a `ProcessVertices` CPU transform subset. Code:
  `drivers/aerogpu/umd/d3d9/src/aerogpu_d3d9_driver.cpp`,
  `aerogpu_d3d9_fixedfunc_shaders.h`; guest tests `d3d9_fixedfunc_*` under
  `drivers/aerogpu/tests/win7/`.

### D3D10/11 (DXBC SM4/SM5) — `crates/aero-d3d11`

- DXBC container + token decoding (`crates/aero-d3d11/src/sm4/`, shared DXBC
  utilities in `crates/aero-dxbc/`) → WGSL
  (`crates/aero-d3d11/src/shader_translate.rs`); command-stream executor:
  `crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs` (wgpu-backed).
- Translation strategy: VS/PS map to a render pipeline, CS to a compute
  pipeline; D3D context state is shadow-tracked and baked into a cached
  `PipelineKey` (shader hash, input-layout hash, topology, rasterizer,
  depth/stencil, blend, RT formats/sample count). cbuffers are represented as
  arrays of 16-byte registers with bitcast typed loads (sidestepping HLSL/WGSL
  packing mismatches); `MAP_WRITE_DISCARD` is buffer renaming. Input layouts
  arrive as `"ILAY"` blobs with FNV-1a semantic-name hashes and are compacted
  into WebGPU’s 8-buffer/16-attribute baseline.
- Binding model (`crates/aero-d3d11/src/binding_model.rs`): stage-scoped bind
  groups — `@group(0)` VS, `@group(1)` PS, `@group(2)` CS, `@group(3)`
  reserved for extended stages (GS/HS/DS) + internal emulation helpers.
  Within a group: `cb#/b#` at `BINDING_BASE_CBUFFER = 0`, `t#` at
  `BINDING_BASE_TEXTURE = 32`, `s#` at `BINDING_BASE_SAMPLER = 160`, `u#` at
  `BINDING_BASE_UAV = 176` (`MAX_UAV_SLOTS = 8`), internal bindings at
  `BINDING_BASE_INTERNAL = 256`. Only resources actually referenced by the
  shader are emitted/bound.
- Compute (SM5) subset: core thread-ID system values, SRV/UAV buffers,
  store/atomic subsets, `sync` barrier lowering with documented rejections
  (fence-only sync in divergent flow, barriers after conditional returns,
  unknown sync flags). Missing: `groupshared`, most typed UAV textures, most
  `Interlocked*`.
- **Geometry shaders**: WebGPU has no GS stage, so draws route through a
  compute prepass + indirect draw. Two prepass modes:
  - *Translated GS prepass* — a subset of SM4 GS DXBC is translated to WGSL
    compute at `CREATE_SHADER_DXBC` time
    (`crates/aero-d3d11/src/runtime/gs_translate.rs`) and executed for draws
    whose IA topology is `PointList`, `LineList`, `TriangleList`,
    `LineListAdj`, or `TriangleListAdj`. GS inputs are fed from VS outputs via
    a minimal VS-as-compute path (`mov`/`add` only; strict-passthrough or
    `AERO_D3D11_ALLOW_INCORRECT_GS_INPUTS=1` IA-fill fallback, otherwise the
    draw fails cleanly). Strip output topologies expand to lists
    (`crates/aero-d3d11/src/runtime/strip_to_list.rs`); GS instancing is
    supported; stream-out and multi-stream output are not.
  - *Synthetic expansion* (`GEOMETRY_PREPASS_CS_WGSL`) — a deterministic
    scaffolding prepass that emits synthetic triangles; used for adjacency /
    patchlist scaffolding and unsupported cases.
  Unsupported end-to-end today: strip and adjacency-strip IA topologies,
  layered rendering system values, and `LineList` translated-prepass draws
  with `instance_count != 1`.
- **Tessellation (HS/DS)**: bring-up only. Patchlist draws with HS+DS bound
  (currently PatchList3) route through a multi-pass prepass (VS-as-compute
  stub → HS passthrough with fixed tess factor 4.0 → layout pass → stand-in
  DS evaluation → tri-domain integer index generation); guest HS/DS DXBC is
  not executed yet. Building blocks: `crates/aero-d3d11/src/runtime/tessellation/`.
- Primitive restart for indexed strips uses native restart where reliable and
  an emulated strip→list conversion elsewhere (notably wgpu GL).
- A pixel-compare conformance suite is defined in tiers (triangle, cbuffer
  updates, texturing, depth, blending, instancing first; GS variants and
  adjacency next; compute blur / UAV writes / tri+quad tessellation last) —
  see the test lists under `crates/aero-d3d11/tests/`.

## Windows 7 guest driver stack (WDDM 1.1)

- Package: `drivers/aerogpu/` — KMD `aerogpu.sys` (`drivers/aerogpu/kmd/`),
  D3D9Ex UMD (`drivers/aerogpu/umd/d3d9/`, ships `aerogpu_d3d9.dll` +
  `aerogpu_d3d9_x64.dll`), D3D10/10.1/11 UMDs (`drivers/aerogpu/umd/d3d10_11/`),
  INFs and packaging under `drivers/aerogpu/packaging/win7/` (`aerogpu.inf`
  D3D9-only, `aerogpu_dx11.inf` DX11-capable; CI stages the DX11 variant).
  INFs bind only to the canonical `A3A0:0001` device.
- Architecture: the KMD is thin (adapter/VidPN modeset, allocations,
  ring/doorbell submission, ISR/DPC, TDR reset); UMDs are state trackers +
  command encoders emitting the AeroGPU IR. Memory model: a single
  system-memory segment (physically contiguous locked pages per allocation);
  the KMD-reported non-local budget defaults to 512 MB and is tunable via the
  `HKR\Parameters\NonLocalMemorySizeMB` registry value when workloads hit
  early `E_OUTOFMEMORY`.
- Feature discovery: UMDs query `KMTQAITYPE_UMDRIVERPRIVATE` and decode
  `aerogpu_umd_private_v1` (`drivers/aerogpu/protocol/aerogpu_umd_private.h`)
  to learn the active ABI magic and feature bits rather than assuming them.
- **D3D9Ex/DWM contract** (what keeps composition enabled): `PresentEx` with
  max-frame-latency throttling (default 3; `D3DPRESENT_DONOTWAIT` →
  `D3DERR_WASSTILLDRAWING` when full), monotonic present stats, non-blocking
  `GetData` (no waiting even with `D3DGETDATA_FLUSH`), bounded
  `WaitForVBlank`, permissive `CheckDeviceState`/`CheckResourceResidency`/
  caps probes (best-effort success over `E_NOTIMPL`), shared surfaces as
  above, and `ResetEx` that does not invalidate default-pool resources. The
  per-submission fence value must come from the D3D runtime submission
  callbacks (`D3DDDICB_RENDER`/`PRESENT`), never from a global “last fence”
  escape query. Guest regression coverage: `d3d9ex_dwm_probe`,
  `d3d9ex_dwm_ddi_sanity`, `d3d9ex_query_latency`, `d3d9ex_event_query`,
  `d3d9ex_submit_fence_stress` under `drivers/aerogpu/tests/win7/`.
- **Vblank/present timing contract**: a free-running 60 Hz vblank per enabled
  source, interrupt-gated (`DXGK_INTERRUPT_TYPE_CRTC_VSYNC` on Win7), with
  monotonic sequence + timestamp registers; vblank must not be present-driven;
  `SyncInterval=1` presents complete no earlier than the next vblank;
  `GetScanLine` is a time-based synthetic anchored on the vblank tick (apps
  poll it at high frequency — cache the anchor). Guest-side pacing tests:
  `wait_vblank_pacing`, `vblank_state_sanity`, `dwm_flush_pacing`.
- **Guest test suite + tooling**: `drivers/aerogpu/tests/win7/` (runner
  `aerogpu_test_runner.exe`), and the `aerogpu_dbgctl.exe` debug tool
  (ring/fence/vblank/allocation dumps, last-submission command dumps; layered
  on `aerogpu_dbgctl_escape.h`). In-UMD tracing for bring-up:
  `AEROGPU_D3D9_TRACE*` env knobs (`drivers/aerogpu/umd/d3d9/src/aerogpu_trace.*`)
  and `AEROGPU_D3D10_TRACE` / `AEROGPU_D3D10_11_CAPS_LOG` for the D3D10/11 UMD.
- Build/signing: WDK10 + MSBuild (`drivers/aerogpu/aerogpu.sln`; CI wrappers
  `drivers/build/build-drivers.ps1`, `drivers/build/make-catalogs.ps1`, `drivers/build/sign-drivers.ps1`), test
  signing for development.

## Configuration and environment knobs

- Presenter backend selection: `GpuRuntimeInitOptions.forceBackend` /
  `preferWebGpu` / `disableWebGpu` (`apps/web/src/ipc/gpu-protocol.ts`).
- `AERO_DISABLE_WGPU_TEXTURE_COMPRESSION=1` — force off wgpu texture
  compression features in native/headless runs.
- `AERO_D3D11_ALLOW_INCORRECT_GS_INPUTS=1` — debug escape hatch: feed GS
  inputs from the IA stream when VS-as-compute fails (may misrender).
- `AERO_REQUIRE_WEBGPU=1` — make wgpu-dependent Rust tests fail instead of
  skipping on headless systems.
- Guest-side (Win7): `AEROGPU_D3D9_TRACE*` tracing knobs;
  `AEROGPU_D3D10_TRACE`, `AEROGPU_D3D10_11_CAPS_LOG`,
  `AEROGPU_D3D10_11_LOG[_FILE]`; `HKR\Parameters\NonLocalMemorySizeMB`;
  `HKLM\...\Services\aerogpu\Parameters\EnableMapSharedHandleEscape` (debug-only
  shared-handle mapping escape).

## Open issues and debt

- **End-to-end Win7 validation is the critical-path gap.** Unit/integration
  coverage of the ABI, device model, executors, and scanout plumbing is
  extensive, but “driver install → ring submissions → ACMD execution → scanout
  present → DWM/Aero stability” has never been validated end-to-end on the
  canonical browser machine. Baseline boot harness: `tests/windows7_boot.rs`.
- **Duplicate AeroGPU device models**: canonical machine MVP
  (`crates/aero-machine/src/aerogpu.rs`), the shared device-side library
  (`crates/aero-devices-gpu/`), and the legacy sandbox surface
  — a second device model and executor — has been retired along with the rest
  of that stack. Consolidating the canonical machine glue with
  `crates/aero-devices-gpu` is still outstanding; what went away was the third
  copy, not the duplication between these two. The retired stack was a legacy
  sandbox slated for migration/deletion.
- **One command execution path in the web runtime**: the TypeScript executor
  in the GPU worker (`apps/web/src/workers/aerogpu-acmd-executor.ts`). A second,
  Rust implementation once sat alongside it, but it had no consumer, no test,
  and no path through the WebAssembly bridge; it is retired
  (see [../history/retirements.md](../history/retirements.md)).
- **GS/HS/DS emulation gaps** as listed above (topology coverage,
  VS-as-compute opcode coverage, no stream-out, HS/DS not executed, low
  storage-buffer limits on downlevel backends can block the prepass).
- **D3D9 texture-dimension gap**: no protocol support for guest-backed 1D/3D
  textures (dummy views today).
- **WebGL2 fallback ceiling**: presentation + 2D only by design.
- Web runtime fixes VRAM at `0xE000_0000` while the canonical machine treats
  BAR bases as dynamic — a documented divergence to revisit if BAR1 becomes a
  fully dynamic PCI BAR in the browser runtime.

## Legacy and historical (kept for context, not the contract)

- **Legacy bring-up AeroGPU ABI** (`"ARGP"` magic, vendor `1AED`/`1AE0` IDs):
  header `drivers/aerogpu/protocol/legacy/aerogpu_protocol_legacy.h`, device
  bring-up model has been retired along with the legacy vendor ID, formerly behind the
  `emulator/aerogpu-legacy` feature. The Win7 KMD auto-detects it via BAR0
  magic, but shipped INFs bind only to the canonical device. Not a target for
  new work.
- **Removed toy protocols**: a minimal `CREATE_SURFACE`/`PRESENT` paravirtual
  GPU and an experimental ring ABI were deleted; their specs are archived
  in `wiki/history/retirements.md` and the code under `crates/legacy/aero-emulator/`.
- **Archived prototype Win7 driver tree**: sources reference
  `prototype/legacy-win7-aerogpu-1ae0/` (stale PCI IDs, not WOW64-complete) as
  the archive location for the early prototype driver stack; that path is not
  present in the current tree — the supported package is
  `drivers/aerogpu/packaging/win7/`.
- **virtio-gpu exploration**: a 2D-scanout virtio-gpu prototype exists in
  `crates/virtio-gpu-proto/` (plus a virtqueue integration test in
  `crates/aero-virtio/`). It was evaluated as a possible “reuse an existing
  Windows driver” path and rejected as the Win7 acceleration plan; the custom
  AeroGPU WDDM stack is canonical. The prototype is a device-model foundation
  only — not a Windows driver contract.

## Pointers

- ABI/protocol: `drivers/aerogpu/protocol/README.md`, PCI identity contract
  on this page, `drivers/aerogpu/protocol/vblank.md`,
  `drivers/aerogpu/protocol/allocation-table.md`.
- Machine/device: `crates/aero-machine/src/lib.rs`,
  `crates/aero-machine/src/aerogpu.rs`, `crates/aero-devices-gpu/`.
- Host executors: `crates/aero-gpu/`, `crates/aero-d3d9/`,
  `crates/aero-d3d11/`, `crates/aero-dxbc/`.
- Web runtime: `apps/web/src/workers/gpu-worker.ts`,
  `apps/web/src/runtime/coordinator.ts`, `apps/web/src/gpu/`, `apps/web/src/ipc/`.
- Guest drivers: `drivers/aerogpu/` (start at `drivers/aerogpu/README.md`).
- Common regression commands:

```bash
bash ./scripts/safe-run.sh cargo test -p aero-gpu-vga -p aero-machine -p aero-shared --locked
bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_submission_bridge --locked
bash ./scripts/safe-run.sh cargo test -p aero-protocol -p aero-gpu -p aero-d3d9 -p aero-d3d11 -p aero-dxbc --locked
bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/wddm_scanout_smoke.spec.ts
bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu.spec.ts
```

## Device-model fixes applied (2026-07-26/27)

See `wiki/notes/bring-up-findings-divergence-and-sight-audits.md` §6 for details.

- **VGA vblank retrace**: advance vblank clock on each 0x3DA read (was frozen
  within a single execution batch → tight retrace-poll loops could hang,
  making the display appear black while execution continued).
- **USB EHCI**: HCCPARAMS EECP set to 0 (was 0x40 advertising USBLEGSUP in MMIO,
  but Win7 reads it from PCI config space → sees list terminator → skips
  BIOS-ownership handoff).
- **USB UHCI**: HCHALTED is now RS-only (was set during EGSM global suspend,
  causing suspend/resume confusion).

## Graphics Subsystem (current implementation)

This document is an **architecture overview of the graphics/presentation code that exists today**.
It intentionally avoids aspirational pseudocode; **the referenced code is the source of truth**.

For the current “what works today vs what’s missing for Win7 usability” checklist, see:

- [`graphics.md`](graphics.md)

If you are looking for deeper background on the AeroGPU/WDDM direction, start with:

- `../specs/windows7-aerogpu-wddm-driver.md` (Windows driver model + guest strategy)
- `../specs/aerogpu-device-abi.md` and `../specs/aerogpu-command-stream.md` (command ABI/protocol)
- `../specs/aerogpu-device-abi.md` (map of similarly named in-tree “GPU protocol” docs)
- `../specs/aerogpu-device-abi.md` (canonical machine execution/fence completion modes: no-op bring-up vs submission bridge vs in-process backend)
- `../specs/aerogpu-device-abi.md` (stable `backing_alloc_id` semantics for Win7/WDDM 1.1 guest-backed resources)

### What exists today (summary)

#### Boot/early display: VGA + VBE

The canonical machine provides a legacy boot display using a **VGA + Bochs VBE (“VBE_DISPI”)** device model:

- Device model: `crates/aero-gpu-vga` (`aero_gpu_vga::VgaDevice`)
- Canonical machine wiring + host-facing `display_present()` API:
  - `crates/aero-machine/src/lib.rs` (see `MachineConfig::enable_vga`, VGA/VBE port+MMIO wiring, and `Machine::display_present`)

Canonical machine GPU device modes (today):

- Boot graphics path (`enable_vga=true`, `enable_aerogpu=false`): `aero_gpu_vga::VgaDevice`.
  - When `enable_pc_platform=false`, the VBE LFB is mapped directly at the configured LFB base.
  - When `enable_pc_platform=true`, the machine exposes a minimal Bochs/QEMU-compatible “Standard VGA”
    PCI function (currently `00:0c.0`, `1234:1111`) and routes the VBE LFB through PCI BAR0 inside the PCI MMIO
    window / BAR router. The BAR base is assigned by BIOS POST / the PCI allocator (and may be
    relocated when other PCI devices are present), and the machine mirrors it into the BIOS VBE
    `PhysBasePtr` and the VGA device model so guests observe a coherent LFB base.
- AeroGPU device (MVP; `enable_aerogpu=true`, `enable_vga=false`): requires `enable_pc_platform=true`.
  - Exposes the canonical AeroGPU PCI identity at **`00:07.0`** (`VID:DID = A3A0:0001`).
  - Wires BAR1-backed VRAM (legacy VGA window aliasing / VBE compatibility mapping).
  - Exposes a minimal BAR0 MMIO surface used for bring-up (ABI/features, ring+fence transport +
    submission decode/capture + IRQs, scanout0/cursor registers, vblank counters; default behavior
    can complete fences without executing ACMD, and browser runtimes can enable the AeroGPU
    submission bridge to drain submissions for out-of-process execution; native builds can install
    a feature-gated in-process wgpu backend; implementation: `crates/aero-machine/src/aerogpu.rs`).
- On the Rust side, the host can call `Machine::display_present()` to update a host-visible RGBA framebuffer cache (`Machine::display_framebuffer()` / `Machine::display_resolution()`).
  - In AeroGPU mode (no standalone VGA device model), `display_present()` prefers the WDDM scanout0 framebuffer once it has been claimed by a valid scanout config; otherwise it falls back to BIOS VBE LFB or BIOS text mode (see `Machine::display_present` in `crates/aero-machine/src/lib.rs`).
      - Once WDDM scanout is claimed, WDDM ownership remains sticky until VM reset. Writing `SCANOUT0_ENABLE=0` blanks presentation but does not release WDDM ownership back to legacy output.
      - When scanout is claimed but cannot be presented (e.g. PCI `COMMAND.BME=0`), `display_present()` clears the cached framebuffer instead of falling back to legacy output.
  - `aero-machine` does not execute the AeroGPU command stream in-process by default; browser
    runtimes can enable the submission bridge (`Machine::aerogpu_drain_submissions` /
    `Machine::aerogpu_complete_fence`) and execute drained submissions in the GPU worker. Native
    builds can also install an in-process headless wgpu backend (feature-gated;
    `Machine::aerogpu_set_backend_wgpu`).
  - Shared device-side AeroGPU building blocks (regs/ring/executor + reusable PCI wrapper) live in
    `crates/aero-devices-gpu/`. A legacy sandbox integration surface remains in
    `crates/aero-devices-gpu/src/pci.rs` (see
    [`platform-and-firmware.md`](platform-and-firmware.md)).

#### Browser presentation: shared-memory framebuffer → GPU worker → canvas

In the browser runtime, the “GPU worker” reads a shared framebuffer in a `SharedArrayBuffer` and uploads it to a presenter backend:

- Shared framebuffer layout (Rust): `crates/aero-shared/src/shared_framebuffer.rs`
- Shared framebuffer layout (TS mirror): `apps/web/src/ipc/shared-layout.ts`
- GPU worker consumption and present loop: `apps/web/src/workers/gpu-worker.ts`

Presenter backend selection (GPU worker):

- Backends (current implementations):
  - WebGPU: `apps/web/src/gpu/webgpu-presenter-backend.ts`
  - WebGL2 (raw): `apps/web/src/gpu/raw-webgl2-presenter-backend.ts`
  - WebGL2 (wgpu via WASM): `apps/web/src/gpu/wgpu-webgl2-presenter.ts`
- Selection inputs:
  - `GpuRuntimeInitOptions` in `apps/web/src/ipc/gpu-protocol.ts` (`forceBackend`, `disableWebGpu`, `preferWebGpu`)
  - selection logic: `initPresenterForRuntime()` in `apps/web/src/workers/gpu-worker.ts`

Compatibility note: the GPU worker can also consume an older “shared framebuffer protocol” header (RGBA8888 + frame counter, no dirty tiles) used by some harnesses/demos:

- `apps/web/src/display/framebuffer_protocol.ts` (layout)
- `apps/web/src/workers/gpu-worker.ts` (`refreshFramebufferProtocolViews`)

#### Scanout coordination: `ScanoutState` (seqlock)

There is a second shared-memory structure that is **not a framebuffer**; it is a **lock-free descriptor** of the current scanout source.
It exists so the GPU worker can be ticked/woken even when the legacy shared framebuffer is idle (e.g. after WDDM “takes over”).

- ScanoutState layout + publish protocol (Rust): `crates/aero-shared/src/scanout_state.rs`
- ScanoutState layout + publish protocol (TS mirror): `apps/web/src/ipc/scanout_state.ts`
  - The GPU worker typically uses `trySnapshotScanoutState()` (bounded retries; returns `null` on failure) rather than `snapshotScanoutState()` (which throws on timeout) so present loops can recover.
- Used by:
  - `apps/web/src/main/frameScheduler.ts` (decides when to tick the GPU worker)
  - `apps/web/src/workers/gpu-worker.ts` (chooses between legacy framebuffer and other output sources)

Developer-facing scanout validation harnesses (served under `/web/` when running the repo-root harness via
`pnpm run dev` / `pnpm run dev:harness`):

- `apps/web/wddm-scanout-debug.html` — interactive scanoutState validation (guest RAM vs BAR1/VRAM backing, pitch, alpha policy)
- `apps/web/wddm-scanout-smoke.html` — non-interactive smoke harness (Playwright: `tests/e2e/wddm_scanout_smoke.spec.ts`)
- `apps/web/wddm-scanout-vram-smoke.html` — BAR1/VRAM-backed scanout smoke harness (Playwright: `tests/e2e/wddm_scanout_vram_smoke.spec.ts`)

Related shared-memory descriptor: **hardware cursor state** uses the same seqlock pattern:

- CursorState layout + publish protocol (Rust): `crates/aero-shared/src/cursor_state.rs`
- CursorState layout + publish protocol (TS mirror): `apps/web/src/ipc/cursor_state.ts`
  - The GPU worker typically uses `trySnapshotCursorState()` (bounded retries; returns `null` on failure) rather than `snapshotCursorState()` (which throws on timeout).
- Consumed by the GPU worker: `apps/web/src/workers/gpu-worker.ts` (cursor snapshot + presenter cursor APIs / compositing)
  - `CursorState.format` also uses AeroGPU `AerogpuFormat` discriminants and follows the same X8 alpha + sRGB interpretation rules as scanout formats.

#### Host-side AeroGPU execution / translation building blocks (implemented)

The repo also contains substantial implemented host-side GPU infrastructure (even though full guest WDDM integration is still evolving):

- AeroGPU command processing + present + recovery/telemetry primitives: `crates/aero-gpu`
- AeroGPU device-model helpers (PCI/MMIO/ring executor/vblank pacing building blocks): `crates/aero-devices-gpu`
- D3D-related crates:
  - `crates/aero-d3d9`
  - `crates/aero-d3d11`
  - `crates/aero-dxbc`
- Canonical protocol mirror (Rust + TypeScript): `crates/aero-protocol` (source headers: `drivers/aerogpu/protocol/`)

### Runtime topology (browser)

At a high level, the runtime is:

```
┌────────────────────────────┐
│ Main thread                │
│ - owns the visible canvas  │
│ - schedules GPU worker tick│
└───────────────┬────────────┘
                │ postMessage("tick") + SharedArrayBuffer handles
                ▼
┌────────────────────────────┐
│ GPU worker                 │
│ - reads shared memory      │
│ - uploads to WebGPU/WebGL2 │
│ - presents to canvas       │
└───────────────┬────────────┘
                │ SharedArrayBuffer / WebAssembly.Memory
                ▼
┌────────────────────────────┐
│ CPU/VM side (WASM)         │
│ - produces scanout content │
│ - writes shared framebuffer│
│ - may publish ScanoutState │
└────────────────────────────┘
```

The details of worker orchestration are outside the scope of this doc, but the “presentation boundary” (what memory is shared and how) is defined by the shared-memory structures below.

Note: there is also a main-thread fallback presenter for the legacy shared framebuffer path (no GPU worker / no OffscreenCanvas transfer). This uses a 2D canvas and polls `SharedFramebufferHeaderIndex.FRAME_SEQ`:

- implementation: `apps/web/src/display/shared_layout_presenter.ts` (`SharedLayoutPresenter`)
- wired in the web UI/runtime: `apps/web/src/main.ts` (`ensureVgaPresenter`)

Shared-memory wiring note: in the canonical multi-worker runtime, the SharedArrayBuffers / `WebAssembly.Memory` handles are distributed to workers via a coordinator init message:

- Message type: `WorkerInitMessage` (`kind: "init"`) in `apps/web/src/runtime/protocol.ts` (includes `guestMemory`, optional `vram`, `scanoutState`, `cursorState`, `sharedFramebuffer`, and optional `frameStateSab`).
- Segment construction: `apps/web/src/runtime/shared_layout.ts` (allocates the shared framebuffer, scanout/cursor descriptors, and optional VRAM aperture).

### Frame pacing / “new frame” state (SharedArrayBuffer)

In addition to the pixel/scanout structures, the browser runtime uses a small `SharedArrayBuffer` as a cross-thread “frame status” flag + metrics block.

- Definition (indices + values): `apps/web/src/ipc/gpu-protocol.ts` (`FRAME_STATUS_INDEX`, `FRAME_DIRTY`, `FRAME_PRESENTING`, `FRAME_PRESENTED`, plus metrics fields)
- Main-thread scheduler that posts `tick` messages to the GPU worker based on this state (and optionally `ScanoutState`): `apps/web/src/main/frameScheduler.ts`
- GPU worker updates this state as it receives/presents frames: `apps/web/src/workers/gpu-worker.ts`

Note: this “frame status” SAB is **separate** from the legacy shared framebuffer header’s `frame_dirty` flag (`SharedFramebufferHeaderIndex.FRAME_DIRTY`). The names are similar but they serve different purposes:

- `FRAME_STATUS_INDEX` / `FRAME_DIRTY` / `FRAME_PRESENTED` (in `apps/web/src/ipc/gpu-protocol.ts`): main-thread↔GPU-worker **tick/pacing coordination**.
- `SharedFramebufferHeaderIndex.FRAME_DIRTY` (in `apps/web/src/ipc/shared-layout.ts` / `crates/aero-shared/src/shared_framebuffer.rs`): producer→consumer **“new frame” / liveness** flag for the shared framebuffer itself.

### Shared-memory display path #1: `SharedFramebuffer` (double-buffered + dirty tiles)

**Goal:** move pixels from the VM/CPU side to the GPU worker with minimal copying and an efficient “only upload what changed” option.

#### Layout and publish protocol

Defined in:

- Rust: `crates/aero-shared/src/shared_framebuffer.rs`
- TypeScript mirror: `apps/web/src/ipc/shared-layout.ts`

Key properties:

- **Header is an array of 32-bit atomics** so it can be accessed from both Rust and JS via `AtomicU32` / `Int32Array + Atomics`.
- **Double buffering** (`slot 0` and `slot 1`):
  - producer writes into the “back” slot, then publishes it by flipping `active_index` and incrementing `frame_seq`.
- Producer also sets a `frame_dirty` flag (`SharedFramebufferHeaderIndex.FRAME_DIRTY`) on publish. This is a producer→consumer “new frame” / liveness flag. Implementations that want to block for new frames typically `Atomics.wait` on `frame_seq` (the canonical “new frame” address), and may also treat `frame_dirty` as a best-effort **consumer acknowledgement** (ACK): consumers clear it after they finish copying/presenting the active buffer, and producers may choose to throttle publishing until it is cleared to avoid overwriting a buffer that is still being read.
  (Not to be confused with the frame pacing state value `FRAME_DIRTY` in `apps/web/src/ipc/gpu-protocol.ts`.)
  - Published by: `SharedFramebufferWriter::write_frame()` in `crates/aero-shared/src/shared_framebuffer.rs`
- Cleared by consumers after consuming a frame (examples):
  - Rust: `FrameSource::ack_frame(frame.seq)` in `crates/aero-gpu/src/frame_source.rs`
  - Browser GPU worker: `presentOnce()` in `apps/web/src/workers/gpu-worker.ts` (clears after consuming a legacy frame, and also when scanout owns output to avoid stale legacy-dirty state).
  - Main-thread fallback presenter: `SharedLayoutPresenter` in `apps/web/src/display/shared_layout_presenter.ts` (clears after `putImageData`).
- Optional **dirty-tile tracking**:
  - each slot may have a dirty bitset (`dirty_words_per_buffer`)
  - dirty tiles are converted to pixel rects by:
    - Rust: `dirty_tiles_to_rects()` in `crates/aero-shared/src/shared_framebuffer.rs`
    - TS: `dirtyTilesToRects()` in `apps/web/src/ipc/shared-layout.ts`

The canonical publish ordering (important for Atomics-based consumers) is documented and implemented in:

- Rust: `SharedFramebufferWriter::write_frame()` in `crates/aero-shared/src/shared_framebuffer.rs`

Rust-side consumer (host/presenter utilities):

- `crates/aero-gpu/src/frame_source.rs` (`FrameSource`) polls `frame_seq`, selects the active slot, and converts dirty tiles into rects for the presenter. Consumers can call `FrameSource::ack_frame` after they are finished reading/copying a frame to clear the shared `frame_dirty` flag (ACK).

#### Consumption in the GPU worker

The GPU worker:

1. Validates the header (`magic`, `version`).
2. Reads `active_index` to select the active slot.
3. Optionally derives dirty rects from the per-slot dirty bitset.
   - If dirty tracking is enabled but the producer sets **no** dirty bits, the consumer treats the frame as **full-frame dirty** (mirrors `FrameSource` behavior to avoid interpreting `[]` as “nothing changed”).
4. Uploads either:
   - a full frame (`present()`), or
   - rect updates (`presentDirtyRects()` when the selected backend supports it).
   - The worker may still choose to fall back to a full-frame upload even when dirty rects are available (e.g. too many rects or estimated upload bytes too high). Policy helper:
     - `chooseDirtyRectsForUpload()` in `apps/web/src/gpu/dirty-rect-policy.ts`
5. Clears the producer→consumer `frame_dirty` flag (`SharedFramebufferHeaderIndex.FRAME_DIRTY`) after consuming a frame.

Code pointers:

- View creation: `refreshSharedFramebufferViews()` in `apps/web/src/workers/gpu-worker.ts`
- Frame selection + dirty-rect derivation: `getCurrentFrameInfo()` in `apps/web/src/workers/gpu-worker.ts`

There is an end-to-end Playwright test that exercises this path:

- `tests/e2e/web/aero-gpu-shared-framebuffer.spec.ts`

### Shared-memory display path #2: `ScanoutState` (seqlock scanout descriptor)

**Goal:** share a coherent “what should be displayed” descriptor across workers without locks, and without forcing the legacy shared framebuffer to be “busy” forever.

This is a small `u32[]` / `Int32Array` structure containing:

- generation (seqlock-style)
- source (`LEGACY_TEXT`, `LEGACY_VBE_LFB`, `WDDM`)
- base physical address (lo/hi)
- width/height/pitch/format
  - `format` uses the AeroGPU `AerogpuFormat` numeric (`u32`) discriminants (where `0` is reserved for `Invalid`).
  - Format semantics (from the AeroGPU protocol):
    - `*X8*` formats (`B8G8R8X8*`, `R8G8B8X8*`) do not carry alpha; treat alpha as fully opaque (`0xFF`) when converting to RGBA.
    - `*_SRGB` variants are layout-identical to UNORM; only the interpretation differs (avoid double-applying gamma in presenters).

Defined in:

- Rust: `crates/aero-shared/src/scanout_state.rs`
- TypeScript mirror: `apps/web/src/ipc/scanout_state.ts`

#### Seqlock publish protocol

The key implementation detail is the “busy bit” seqlock:

- Writer sets `SCANOUT_STATE_GENERATION_BUSY_BIT` before updating fields.
- Writer publishes a new generation (with the busy bit cleared) **as the final store**.
- Reader retries if:
  - the busy bit is set, or
  - generation changes during the read.

Code pointers:

- Rust: module-level docs + `ScanoutState::publish()` / `ScanoutState::snapshot()` in `crates/aero-shared/src/scanout_state.rs`
- TS: `publishScanoutState()` / `snapshotScanoutState()` / `trySnapshotScanoutState()` in `apps/web/src/ipc/scanout_state.ts`

#### How it is used today

- **Main thread scheduling:** `apps/web/src/main/frameScheduler.ts` uses `ScanoutState` to decide whether to keep ticking the GPU worker even when the shared framebuffer is in the `PRESENTED` state.
  - When `ScanoutState.source` is `WDDM` or `LEGACY_VBE_LFB`, it keeps ticking so the worker can poll/present scanout output even if the legacy shared framebuffer is idle.
  - When WDDM publishes the **disabled** descriptor (`base/width/height/pitch = 0`), the scheduler stops continuous ticking (vblank pacing is effectively off) but will still wake on scanout generation changes.
- **GPU worker output selection:** `apps/web/src/workers/gpu-worker.ts` snapshots `ScanoutState` during `presentOnce()` and uses it to avoid “flashing back” to the legacy framebuffer after WDDM scanout is considered active.
- **GPU worker scanout readback (guest-memory scanout):** when `ScanoutState.source` is `WDDM` or `LEGACY_VBE_LFB` and `base_paddr` points at a real guest framebuffer, `apps/web/src/workers/gpu-worker.ts` reads pixels from either the shared VRAM aperture (BAR1 backing) or guest RAM and normalizes to a tightly-packed RGBA8 buffer (`tryReadScanoutFrame()` / `tryReadScanoutRgba8()`).
  - Supported formats today (AeroGPU `AerogpuFormat` discriminants):
    - 32bpp packed: `B8G8R8X8` / `B8G8R8A8` / `R8G8B8X8` / `R8G8B8A8` (plus `_SRGB` variants).
    - 16bpp packed: `B5G6R5` (opaque) and `B5G5R5A1` (1-bit alpha).
    X8 formats force `A=255`; A8 formats preserve alpha. `_SRGB` variants are layout-identical; the GPU worker decodes sRGB→linear after swizzle so the intermediate RGBA8 buffer is in linear space for blending/presentation.
  - Shared helper used by readback paths (size checks + guest-RAM conversion): `apps/web/src/runtime/scanout_readback.ts` (`tryComputeScanoutRgba8ByteLength`, `MAX_SCANOUT_RGBA8_BYTES`, `readScanoutRgba8FromGuestRam`).
  - Note: for `source=WDDM`, `base_paddr == 0` is used in two distinct ways:
    - **Placeholder descriptor** for the host-side AeroGPU path: `base_paddr=0` but **non-zero** `width/height/pitch`.
    - **Disabled descriptor** (WDDM retains ownership but blanks output): `base/width/height/pitch = 0`.
    Legacy VBE scanout expects a real framebuffer (`base_paddr != 0`).
  - The RAM-vs-VRAM resolution and the VRAM base-paddr contract are documented in [`graphics.md`](graphics.md#vram-bar1-backing-as-a-sharedarraybuffer).
  - Unit tests: `apps/web/src/workers/gpu-worker_wddm_scanout_readback.test.ts`, `apps/web/src/workers/gpu-worker_wddm_scanout_screenshot_refresh.test.ts`, `apps/web/src/workers/gpu-worker_scanout_vram_missing.test.ts`, `apps/web/src/workers/gpu-worker_wddm_tick_gate.test.ts`.
- **Canonical Rust machine (optional):** `crates/aero-machine/src/lib.rs` can publish scanout-source updates into an `aero_shared::scanout_state::ScanoutState` provided by the host:
  - `Machine::set_scanout_state()` installs the shared descriptor.
  - `Machine::reset()` publishes `LEGACY_TEXT` on reset.
  - `Machine::handle_bios_interrupt()` publishes legacy scanout transitions (`LEGACY_TEXT` ↔ `LEGACY_VBE_LFB`) on BIOS INT 10h mode changes, while refusing to let legacy INT 10h steal scanout while WDDM is active (until the VM resets).
  - `Machine::process_aerogpu()` publishes updates derived from AeroGPU scanout0 registers, including publishing a disabled WDDM scanout descriptor when the guest clears `SCANOUT0_ENABLE` (visibility toggle) so legacy scanout does not steal ownership back.

Note: `ScanoutState` is also the intended mechanism for a device model to describe a guest-memory scanout buffer (base paddr + pitch etc). The full AeroGPU/WDDM scanout plumbing is owned elsewhere; see “AeroGPU status” below.

### Canonical machine boot display path (VGA/VBE)

The canonical Rust machine (`aero_machine::Machine`) wires in a legacy VGA/VBE device for BIOS + early boot output:

- VGA/VBE implementation: `crates/aero-gpu-vga`
- Integration into the canonical machine: `crates/aero-machine/src/lib.rs`
  - `MachineConfig::enable_vga`
  - VGA+VBE wiring (search for `VGA / SVGA integration`)
  - `Machine::display_present()`

Important ABI notes:

- `MachineConfig::enable_vga` and `MachineConfig::enable_aerogpu` are mutually exclusive.
- The canonical **AeroGPU PCI identity** is reserved at `00:07.0` (`VID:DID = A3A0:0001`) and documented in:
  - `../specs/aerogpu-device-abi.md`
- When `enable_vga=true` and the PC platform is enabled (`enable_pc_platform=true`), the canonical
  machine exposes a minimal Bochs/QEMU-compatible “Standard VGA” PCI stub (currently `00:0c.0`,
  `1234:1111`) and routes the VBE linear framebuffer (LFB) through its BAR0 inside the PCI MMIO
  window.
  The BAR base is assigned by BIOS POST / the PCI allocator unless pinned via
  `MachineConfig::{vga_lfb_base,vga_vram_bar_base}`, and the machine mirrors the assigned base into
  the BIOS VBE `PhysBasePtr` and the VGA device model so guests observe a coherent LFB base.

### Presenter backends and color/alpha policy

Presentation policy is also centralized on the Rust side (useful for native tests and wgpu-backed paths):

- `crates/aero-gpu/src/present.rs` (presentation policy enums + selection helpers, plus dirty-rect upload utilities)

#### Source pixel format and conventions

Across the browser presentation code, the “CPU → presenter” source is treated as:

- **RGBA8** byte order `[R, G, B, A]`
- **top-left origin** (first row is the top scanline)

Code pointers:

- WebGPU worker presenter uploads: `apps/web/src/gpu/webgpu-presenter-backend.ts` (`frameTexture` is `rgba8unorm`)
  - Top-left origin convention is enforced in the blit shader: `apps/web/src/gpu/shaders/blit.wgsl`
- WebGL2 worker presenter uploads: `apps/web/src/gpu/raw-webgl2-presenter-backend.ts` (`tex(Sub)Image2D` with `gl.UNPACK_FLIP_Y_WEBGL = 0`)
  - Top-left origin convention is enforced in the shaders: `apps/web/src/gpu/shaders/blit.vert.glsl`, `apps/web/src/gpu/shaders/blit.frag.glsl`

#### Alpha policy: treat output as opaque

The browser canvas is configured to avoid blending with the page background.

Enforced in:

- WebGPU presenter backend: `apps/web/src/gpu/webgpu-presenter-backend.ts` (`ctx.configure({ alphaMode: "opaque", ... })`)
- Raw WebGL2 presenter backend: `apps/web/src/gpu/raw-webgl2-presenter-backend.ts` (context created with `{ alpha: false, premultipliedAlpha: false }`)

#### Linear vs sRGB policy and validation

For deterministic comparisons between WebGPU and WebGL2, Aero also ships “validation presenters” that explicitly implement:

- output color space selection (`linear` vs `srgb`)
- alpha mode (`opaque` vs `premultiplied`)
- optional flip-Y

Code pointers:

- WebGPU validation presenter: `apps/web/src/gpu/webgpu-presenter.ts`
  - uses `viewFormats` (e.g. `bgra8unorm-srgb`) when available; otherwise falls back to shader sRGB encoding
- WebGL2 validation presenter: `apps/web/src/gpu/raw-webgl2-presenter.ts`
  - does sRGB encoding in shader for deterministic output (WebGL default framebuffer sRGB behavior varies)
- Playwright validation: `tests/e2e/web/gpu_color.spec.ts`

Worker presented-output validation (post sRGB/alpha policy; canvas pixels):

- Debug readback message: `screenshot_presented` in `apps/web/src/ipc/gpu-protocol.ts`
- Test card generator: `apps/web/src/gpu/test-card.ts`
- Playwright E2E:
  - Color policy (gamma/alpha/origin): `tests/e2e/web/gpu_worker_presented_color_policy.spec.ts`
  - Cursor overlay blending (linear blend + sRGB encode): `tests/e2e/gpu_worker_presented_cursor_overlay.spec.ts`
  - CursorState upload + screenshot include/exclude: `tests/e2e/web/gpu_hardware_cursor_state.spec.ts`

### AeroGPU status (high level; protocol references)

This doc does **not** define the AeroGPU PCI/MMIO integration work (owned elsewhere). It only links the current contracts and the code that exists today.

Protocol references:

- Guest-facing protocol headers: `drivers/aerogpu/protocol/README.md`
  - includes `aerogpu_pci.h`, `aerogpu_ring.h`, `aerogpu_cmd.h`
- In-tree Rust/TS mirror: `crates/aero-protocol`
- ABI docs:
  - `../specs/aerogpu-device-abi.md` (PCI identity)
  - `../specs/aerogpu-command-stream.md` and `../specs/aerogpu-device-abi.md` (command stream format / ABI)

Canonical machine note:

- `MachineConfig::enable_aerogpu` wires BAR1 VRAM (plus legacy VGA window aliasing / VBE compatibility mapping) and an MVP BAR0 register block (ABI/features, ring/fence + IRQ transport, scanout0/cursor + vblank registers) that is sufficient for detection/bring-up and basic pacing.
  It decodes ring submissions and exposes them via a submission bridge for the browser runtime (GPU worker), and also supports optional in-process backends for native/tests. End-to-end Win7 validation is still pending (see `graphics.md`).
- The MVP BAR0 MMIO surface + ring/fence/vblank/scanout implementation in the canonical machine lives in: `crates/aero-machine/src/aerogpu.rs`.
- A shared AeroGPU device-side library exists in `crates/aero-devices-gpu/` (regs, ring, executor, and the backend boundary), and a CPU rasterizer for the command stream in `crates/aero-gpu-software/`. The canonical browser machine (`crates/aero-machine` + `crates/aero-wasm` + web workers) has its own BAR0/BAR1 integration layer; consolidating those two surfaces remains outstanding.

### How to validate (tests)

TypeScript/unit tests (fast, good for IPC/layout changes):

```bash
pnpm run test:unit:coverage
```

Playwright GPU-focused e2e tests:

```bash
# WebGPU project (includes gpu_color.spec.ts when WebGPU is available)
pnpm run test:webgpu

# Targeted test for the SharedFramebuffer + dirty-tiles presentation path
pnpm run test:e2e -- tests/e2e/web/aero-gpu-shared-framebuffer.spec.ts

# Targeted test for ScanoutState-driven scanout presentation (guest-memory scanout readback)
pnpm run test:e2e -- tests/e2e/web/runtime_workers_scanout_state.spec.ts

# Targeted scanout harness smoke tests (served by `/web/wddm-scanout-*.html`)
pnpm run test:e2e -- tests/e2e/wddm_scanout_smoke.spec.ts
pnpm run test:e2e -- tests/e2e/wddm_scanout_vram_smoke.spec.ts

# Targeted test for presenter backend fallback (WebGPU disabled → WebGL2)
pnpm run test:e2e -- tests/e2e/web/gpu-fallback.spec.ts
```

Rust tests relevant to shared-memory graphics/presentation:

```bash
cargo test -p aero-shared
cargo test -p aero-gpu
cargo test -p aero-machine
```

## AeroGPU Legacy VGA/VBE Compatibility (Boot Display)

### Goal

Windows 7 must be able to **show a working boot display before the AeroGPU WDDM driver is installed/loaded**:

- BIOS POST text output (INT 10h + direct text VRAM writes)
- Windows boot logo / installer UI (VBE linear framebuffer graphics mode)
- After the AeroGPU WDDM KMD loads, scanout must **handoff** to the WDDM-programmed framebuffer without requiring a second “legacy VGA” adapter.

This document specifies the emulator-side behavior required for the **AeroGPU virtual PCI device** to be VGA/VESA-compatible enough for early boot, while still supporting a clean transition to the WDDM path.

### Current status (canonical machine)

This document describes the **desired** end state for the AeroGPU device model (A3A0:0001) to own
both the legacy VGA/VBE boot display path and the modern WDDM/MMIO/ring protocol.

The canonical `aero_machine::Machine` supports **two mutually-exclusive** display configurations:

- **AeroGPU (canonical / long-term):** `MachineConfig::enable_aerogpu=true` exposes the canonical
  AeroGPU PCI identity at `00:07.0` (`A3A0:0001`) with the canonical BAR layout (BAR0 regs + BAR1
  VRAM aperture). In `aero_machine` today this wires the **BAR1 VRAM aperture** to a dedicated
  host-backed VRAM buffer and implements minimal **legacy VGA decode** (permissive VGA port I/O +
  a VRAM-backed `0xA0000..0xBFFFF` window). An MVP BAR0 device model is also present (ring decode +
  fences + scanout/cursor regs + vblank pacing + error-info latches + submission capture for external execution), and
  `Machine::display_present()`
  will prefer the WDDM-programmed scanout framebuffer once scanout0 has been **claimed** (valid config +
  `SCANOUT0_ENABLE=1`). After claim, WDDM scanout remains authoritative until the VM resets.
  Writing `SCANOUT0_ENABLE=0` acts as a visibility toggle (blanking / stopping vblank pacing) and
  does not release WDDM ownership (legacy output remains suppressed until reset).

  Concretely:

  - VRAM backing + legacy decode: `crates/aero-machine/src/lib.rs` (`AeroGpuDevice`,
    `AeroGpuLegacyVgaMmio`, `AeroGpuVgaPortWindow`)
  - BAR0 MMIO + ring/fence + scanout/vblank register storage: `crates/aero-machine/src/aerogpu.rs`
    (`AeroGpuMmioDevice`) + host presentation in `crates/aero-machine/src/lib.rs`
    (`Machine::display_present_aerogpu_scanout`)

  The BIOS VBE implementation uses a linear framebuffer inside BAR1. `aero_machine` sets the VBE
  `PhysBasePtr` to `BAR1_BASE + 0x40000` (`AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES`; see
  `crates/aero-machine/src/lib.rs::VBE_LFB_OFFSET`) so INT 10h VBE mode set/clear writes land in
  BAR1-backed VRAM (leaving the first 256KiB reserved for legacy VGA planar/text backing:
  4 × 64KiB planes).

  `aero_machine` does not execute `AEROGPU_CMD` in-process by default. Instead, it supports:
  - the **submission bridge** (browser runtime): drain decoded submissions and complete fences from an external executor (GPU worker), and/or
  - optional in-process backends for native/tests (`immediate`/`null`, plus a feature-gated wgpu backend).

  When no backend/bridge is installed, BAR0 completes fences without executing ACMD so the Win7 KMD
  doesn't deadlock.

  Shared device-side building blocks (ring helpers + backend boundary + native backend wrapper) live in
  `crates/aero-devices-gpu`, with a CPU rasterizer in `crates/aero-gpu-software`
  (see: [`21-emulator-crate-migration.md`](platform-and-firmware.md)).
- **Legacy VGA/VBE (transitional):** `MachineConfig::enable_vga=true` uses the standalone
  `aero_gpu_vga` VGA/VBE device model for boot display.
  - When `MachineConfig::enable_pc_platform=true`, the machine exposes a minimal Bochs/QEMU-compatible
    “Standard VGA” PCI function (currently `00:0c.0`, `1234:1111`) and routes the VBE linear framebuffer (LFB)
    through its BAR0 inside the PCI MMIO window. BIOS POST / the PCI resource allocator assigns BAR0
    (and may relocate it when other PCI devices are present); `aero_machine` mirrors the chosen BAR
    base into the BIOS VBE `PhysBasePtr` and the `aero_gpu_vga::VgaDevice` LFB base so guests see
    hardware-like behavior.
  - When `MachineConfig::enable_pc_platform=false`, the LFB is mapped directly at the configured
    base, which historically defaults to `0xE000_0000` via `aero_gpu_vga::SVGA_LFB_BASE`.

`enable_aerogpu` and `enable_vga` are **mutually exclusive** (the machine rejects configurations
that enable both).

See:

- [`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md) (canonical AeroGPU VID/DID)
- [`platform-and-firmware.md`](platform-and-firmware.md) (BDF allocation)

---

### High-level model

At all times, the browser canvas renders from exactly one active scanout source:

```text
Legacy VGA text / VBE LFB  ──(WDDM claims scanout)──▶  WDDM scanout (owned)
            ▲                                         │
            │                                         ├──(SCANOUT0_ENABLE=0)──▶  WDDM scanout (disabled / blank)
            │                                         │                            │
            │                                         └──(SCANOUT0_ENABLE=1)◀─────┘
            └──────────────────────(VM reset)─────────┘
```

Implementation-wise, AeroGPU owns **both**:

1. A minimal legacy VGA/VBE frontend (ports + legacy VRAM window + VBE mode set)
2. The modern AeroGPU MMIO/WDDM frontend (command queue, allocations, scanout registers)

The emulator’s display subsystem selects which framebuffer to present based on a single authoritative state structure (`ScanoutState`), updated by either:

- VBE `Set Mode` during boot, or
- AeroGPU WDDM scanout registers once the driver is active.

---

### PCI identity and legacy decode requirements

#### PCI class

AeroGPU must enumerate as a **VGA-compatible display controller**:

- **Class code:** `0x03` (Display controller)
- **Subclass:** `0x00` (VGA compatible controller)
- **Prog IF:** `0x00`

This ensures firmware/OS treat it as the primary boot display candidate, and enables the expectation that it decodes legacy VGA ranges.

#### BAR layout (recommended)

To support both WDDM and legacy/VBE:

| BAR | Type | Size | Purpose |
|-----|------|------|---------|
| BAR0 | MMIO | 64KB | AeroGPU control registers (incl. WDDM scanout regs) |
| BAR1 | Prefetchable MMIO | 64MiB (canonical profile) | Dedicated VRAM aperture (contains legacy VGA window + VBE LFB + optional “VRAM allocations”) |

The emulator BIOS assigns BAR addresses within the reserved below-4 GiB PCI/MMIO hole
(`0xC000_0000..0x1_0000_0000`). The current BAR allocator places device MMIO BARs starting at
`0xE000_0000`. Firmware and guests must treat the assigned AeroGPU BAR bases as **dynamic** (do not
assume a fixed physical address).

#### Legacy ranges to decode

Regardless of BARs, AeroGPU must decode these standard VGA ranges:

##### I/O ports

- `0x3C0–0x3DF` (attribute, sequencer, graphics controller, CRTC, misc, status)
- `0x3B0–0x3BB` and `0x3D0–0x3DF` CRTC aliasing (mono vs color)
- Palette/DAC ports: `0x3C6–0x3C9` (subset of `0x3C0–0x3DF`, called out because Windows uses them)

##### Legacy VRAM window

- `0xA0000–0xBFFFF` (128KB)

The emulator’s memory bus must route this window to the AeroGPU legacy VGA frontend (not to RAM), even though it lies in the “conventional memory” address region.

---

### VRAM mapping and how it relates to WDDM allocations

#### Dedicated VRAM region

For determinism and to avoid conflicts with guest RAM paging, legacy VGA/VBE uses a dedicated VRAM buffer owned by AeroGPU and exposed via BAR1:

```text
BAR1 (VRAM aperture) base: BAR1_BASE (assigned by BIOS)

VRAM offset    Purpose
0x00000..0x3FFFF  Legacy VGA planar memory (4 × 64KiB planes; includes the CPU-visible `0xA0000..0xBFFFF` window backing)
0x40000..          VBE linear framebuffer (LFB) base (`AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES`; packed-pixel VBE modes)
...                Optional: WDDM allocations in VRAM (if implemented)
```

#### Legacy window aliasing

The guest-visible legacy VGA decode range is still the standard 128KiB aperture:

- `0xA0000–0xBFFFF` (128KB)

In the canonical `aero_machine` implementation today, this range is VRAM-backed:

```text
# VBE inactive (VGA/text): linear alias.
0xA0000..0xBFFFF  <->  VRAM[0x00000..0x1FFFF]

# VBE active: 0xA0000..0xAFFFF becomes the VBE banked window into the VBE framebuffer region
# starting at VBE_LFB_OFFSET (selected by the current 64KiB bank).
0xA0000..0xAFFFF  <->  VRAM[VBE_LFB_OFFSET + vbe_bank*64KiB + off]
0xB0000..0xBFFFF  <->  VRAM[0x10000..0x1FFFF]
```

This is sufficient for BIOS POST + bootloader text output and for the firmware VBE implementation
to share the same VRAM backing store.

Note: legacy VGA hardware exposes a 128KiB CPU-visible window (`0xA0000..0xBFFFF`). In
`aero_machine` today this is modeled as a simple 128KiB linear alias at `VRAM[0x00000..0x1FFFF]`.

#### VBE LFB base address

When AeroGPU is enabled, VBE mode info must report:

`PhysBasePtr = BAR1_BASE + 0x40000` (`AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES`; aligned to 64KB; `VBE_LFB_OFFSET` in `aero_machine`).

Windows 7 boot graphics and installer UI will draw directly into this linear framebuffer.

#### Relation to WDDM allocations

The recommended rule is:

- **Legacy VGA uses a fixed reserved subregion of VRAM** (`0x00000..0x3FFFF`).
- **The VBE packed-pixel linear framebuffer starts at `0x40000`** (`AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES`) and consumes
  `width * height * bytes_per_pixel` bytes (depending on the active VBE mode).
- **WDDM allocations (today)** in the in-tree Win7 AeroGPU driver are system-memory-backed (guest
  RAM), i.e. the driver reports no dedicated VRAM segment (see
  `../specs/windows7-aerogpu-wddm-driver.md`). BAR1 exists primarily for legacy VGA/VBE
  compatibility.
- **Optional future:** WDDM allocations may use the remaining BAR1 VRAM space (not overlapping the
  active legacy/VBE region) if/when the device model + driver contract are extended to support it.

To keep the handoff simple, the scanout logic treats the WDDM-programmed scanout base as a *guest physical address* that can point to either:

1. BAR1 VRAM space, or
2. Guest RAM (for “shared” allocations)

The emulator must be able to read pixels from either backing store when presenting.

---

### Minimal legacy VGA behavior (boot text visibility)

Windows 7 setup/boot mostly relies on VBE for graphics, but BIOS POST and many bootloaders rely on VGA text mode.

#### Text mode: required behavior

Implement enough for mode `0x03` (80x25 color text):

- The visible text buffer is at `0xB8000` (aliased to VRAM as described above).
- Each cell is 2 bytes: `[char][attr]`.
- The renderer converts this to pixels using an 8x16 or 9x16 font (any consistent VGA-ish font is acceptable for the emulator).
- Basic cursor support is optional for Windows boot, but BIOS POST benefits from it; implement CRTC cursor registers if available.

#### Mode 13h: optional behavior

Mode `0x13` (320x200x256) is not required for Windows 7 boot (which uses VBE LFB modes), but some
bootloaders and DOS-style guests use it. A minimal implementation can model mode 13h as a simple
linear 64KiB framebuffer at `0xA0000` with 8-bit palette indices and render it using the VGA DAC
palette.

#### VGA ports: minimal subset

For early boot stability, most VGA ports can be permissive no-ops, but these should behave plausibly:

- `0x3C2` Misc Output (store written value; influences CRTC base port selection in real hardware)
- `0x3C4/0x3C5` Sequencer index/data (store regs)
- `0x3CE/0x3CF` Graphics controller index/data (store regs; allow reads)
- `0x3D4/0x3D5` CRTC index/data (store regs; allow reads)
- `0x3C0/0x3C1` Attribute controller (index flip-flop; store regs)
- `0x3DA` Input Status 1 (read resets attribute flip-flop; return a value with bit 3 “vertical retrace” toggling is optional)

If a port is unimplemented, returning `0xFF` on reads and ignoring writes is typically sufficient to keep guests alive.

---

### VBE (VESA BIOS Extensions) for Windows 7 boot graphics

Windows 7’s boot path will query VBE via INT 10h `AX=4Fxx` and set a linear framebuffer mode.

#### Required modes

Expose at least these 32bpp linear framebuffer modes:

| Resolution | BitsPerPixel | Mode number (suggested) |
|------------|--------------|--------------------------|
| 800x600    | 32           | `0x115` |
| 1024x768   | 32           | `0x118` |
| 1280x720   | 32           | `0x160` (OEM-defined) |

The specific mode numbers are not important as long as:

- They appear in the VBE mode list returned by `4F00h`
- `4F01h` returns valid mode info
- `4F02h` can set them with linear framebuffer enabled

#### Pixel format

Use a standard direct-color layout:

- `BitsPerPixel = 32`
- `RedMaskSize=8`, `RedFieldPosition=16`
- `GreenMaskSize=8`, `GreenFieldPosition=8`
- `BlueMaskSize=8`, `BlueFieldPosition=0`
- `ReservedMaskSize=8`, `ReservedFieldPosition=24`

This corresponds to little-endian **B8G8R8X8** in memory.

#### Pitch

Set `BytesPerScanLine = width * 4`.

If the implementation prefers alignment (e.g. 16-byte), it must be reflected in `BytesPerScanLine`, and the renderer must use the programmed pitch.

#### INT 10h VBE functions to implement

Minimum viable set for Windows boot:

- `AX=4F00h` Get Controller Info
- `AX=4F01h` Get Mode Info
- `AX=4F02h` Set VBE Mode (support bit 14 = linear framebuffer)
- `AX=4F03h` Get Current Mode

All other functions can return failure (`AX=014Fh`, CF=1) unless a specific guest requires them.

Current firmware also implements `4F06h` scan-line information and accepts
the `4F10h` DPMS call used by Win7's generic VGA miniport. VBE is present in
two coherent forms: Rust HLE for calls executed by Aero's CPU and a real
16-bit handler in the 64 KiB BIOS ROM for operating systems that copy and
software-interpret low memory. Win7 uses the latter through
`videoprt!VideoPortInt10` → `hal!x86BiosCall`; a private `HLT` hypercall stub
is not a portable option there.

The ROM handler and HLE share `VbeDevice` mode definitions. Because the final
PCI framebuffer address is assigned after the immutable ROM is mapped, POST
and machine display wiring publish it in a reserved EBDA field at physical
`0x9F040`. `4F01h` copies its ROM template, then replaces `PhysBasePtr` from
that runtime field. Snapshot restore republishes the field after restoring
RAM.

An operating-system BIOS interpreter executes the ROM without entering Aero's
Rust HLE, so a ROM-driven `4F02h` changes DISPI but not the private
`Bios::video.vbe.current_mode` cache. Direct DISPI programming therefore
claims legacy scanout ownership: guest-owned DISPI state supersedes any stale
BIOS mode for both `Machine::display_present()` and published
`ScanoutState`. Until the first direct write, BIOS mode state continues to
drive the mirrored register defaults. This ordering is covered by
`guest_owned_dispi_mode_supersedes_stale_bios_vbe_mode`.

DISPI mode enable has device semantics beyond storing register 4. On a
disabled→enabled edge, Bochs/QEMU resets virtual width and both display
offsets, derives a tightly packed pitch from the new visible width, and clears
the visible surface unless bit 7 (`NOCLEARMEM`) is set. Aero follows that
rule. This is essential when changing from a 1024-wide BIOS mode to Win7's
late 800×600 mode: retaining the old 1,024-pixel virtual width makes the guest
write 3,200-byte rows while presentation reads 4,096-byte rows. A guest that
wants a wider virtual surface or panning programs registers 6, 8, and 9 after
enabling.

---

### Scanout selection and handoff to WDDM

#### Scanout state (single source of truth)

Define a scanout description readable by the presentation pipeline:

```rust
#[repr(C)]
pub struct ScanoutState {
    pub generation: u32,          // increment on every complete update
    pub source: u32,              // 0=LegacyText, 1=LegacyVbeLfb, 2=Wddm
    pub base_paddr_lo: u32,       // guest physical address (low)
    pub base_paddr_hi: u32,       // guest physical address (high)
    pub width: u32,
    pub height: u32,
    pub pitch_bytes: u32,
    pub format: u32,              // AerogpuFormat / enum aerogpu_format (e.g. B8G8R8X8Unorm = 2)
}
```

`format` must store the AeroGPU format discriminant, matching
[`drivers/aerogpu/protocol/aerogpu_pci.h`](../../drivers/aerogpu/protocol/aerogpu_pci.h)
`enum aerogpu_format` (i.e. it is *not* a bespoke 0-based scanout-only enum).

The update rule:

1. Writer populates all fields except `generation`
2. Writer publishes the update by incrementing `generation` last (release semantics)
3. Reader snapshots `generation`, reads fields, then re-reads `generation` to verify a consistent view

**Implementation note (recommended):** To prevent readers from observing a partially-updated
descriptor, implementations may temporarily mark `generation` as “busy” (e.g. by setting a high
bit) during an update. Readers should retry if `generation` is marked busy.

This makes scanout switching glitch-free without locks.

#### Scanout format semantics (presentation)

- **sRGB vs UNORM:** sRGB variants are byte-identical to their UNORM counterparts, but the *interpretation*
  differs. Sampling should decode sRGB→linear and render-target writes/views may encode linear→sRGB.
  Presenters must avoid double-applying gamma when handling `*_SRGB` scanout formats.
- **X8 alpha semantics:** `B8G8R8X8*` / `R8G8B8X8*` formats must be treated as fully opaque when
  presenting. If converting to RGBA (e.g. browser canvas), alpha is implicitly `1.0` / `0xFF`.

#### Required WDDM scanout programming surface (BAR0)

The canonical A3A0:0001 MMIO register map is defined by
[`drivers/aerogpu/protocol/aerogpu_pci.h`](../../drivers/aerogpu/protocol/aerogpu_pci.h)
(grep for `AEROGPU_MMIO_REG_SCANOUT0_*`). This section exists only to summarize the scanout
programming subset relevant to **boot-display handoff**.

Scanout 0 registers (BAR0):

| Offset | Name | Width | Description |
|--------|------|-------|-------------|
| `0x0400` | `SCANOUT0_ENABLE` (`AEROGPU_MMIO_REG_SCANOUT0_ENABLE`) | 32 | 0/1 |
| `0x0404` | `SCANOUT0_WIDTH` (`AEROGPU_MMIO_REG_SCANOUT0_WIDTH`) | 32 | Width in pixels |
| `0x0408` | `SCANOUT0_HEIGHT` (`AEROGPU_MMIO_REG_SCANOUT0_HEIGHT`) | 32 | Height in pixels |
| `0x040C` | `SCANOUT0_FORMAT` (`AEROGPU_MMIO_REG_SCANOUT0_FORMAT`) | 32 | `enum aerogpu_format` / `AerogpuFormat` (e.g. `B8G8R8X8Unorm = 2`) |
| `0x0410` | `SCANOUT0_PITCH_BYTES` (`AEROGPU_MMIO_REG_SCANOUT0_PITCH_BYTES`) | 32 | Bytes per scanline |
| `0x0414` | `SCANOUT0_FB_GPA_LO` (`AEROGPU_MMIO_REG_SCANOUT0_FB_GPA_LO`) | 32 | Framebuffer guest physical address (low 32) |
| `0x0418` | `SCANOUT0_FB_GPA_HI` (`AEROGPU_MMIO_REG_SCANOUT0_FB_GPA_HI`) | 32 | Framebuffer guest physical address (high 32) |

**Atomic / "commit" semantics (canonical A3A0):**

- There is **no** separate `COMMIT` register/bit in the canonical A3A0 MMIO ABI. The Windows KMD
  stages `SCANOUT0_*` values and then writes `SCANOUT0_ENABLE`.
- Treat the write to `SCANOUT0_ENABLE` as the **commit point**, but only claim/publish WDDM scanout
  once the configuration is **valid**:
  - `SCANOUT0_ENABLE` must be `1` and the programmed scanout config must be valid (non-zero
    framebuffer GPA, non-zero width/height, supported pixel format, pitch large enough for the row
    size, etc). If the config is invalid (e.g. `FB_GPA=0` during early init), do **not** publish a
    WDDM scanout descriptor and do **not** steal legacy VGA/VBE presentation.
  - Avoid publishing a torn 64-bit `SCANOUT0_FB_GPA`: drivers typically write LO then HI, so treat
    the HI write as the commit point for the combined 64-bit address.
- After a valid WDDM scanout has been claimed, update `ScanoutState` on configuration changes
  (including flips via `SCANOUT0_FB_GPA_*` updates before PRESENT).
- If the guest clears `SCANOUT0_ENABLE` after claim, publish a **disabled WDDM scanout descriptor**
  (`base/width/height/pitch = 0`) so legacy VGA/VBE cannot steal scanout back while WDDM ownership
  is held.
- Presentation commands read the current scanout programming: `AEROGPU_CMD_PRESENT` uses the
  currently-programmed `SCANOUT0_*` registers, and drivers may update `SCANOUT0_FB_GPA_*` before
  PRESENT to implement flips (see
  [`drivers/aerogpu/protocol/aerogpu_cmd.h`](../../drivers/aerogpu/protocol/aerogpu_cmd.h)).

#### When legacy VGA/VBE owns scanout

- At reset/power-on: `source = LegacyText`, base points at the legacy text buffer (or can be implicit)
- When BIOS/bootloader sets a VBE LFB mode: `source = LegacyVbeLfb`, base = `BAR1_BASE + 0x40000` (`AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES`), width/height/pitch filled

#### When WDDM owns scanout

When the AeroGPU WDDM driver is loaded, it programs the AeroGPU scanout registers (BAR0). As soon
as the device observes `SCANOUT0_ENABLE=1` with a **valid** scanout configuration (including a
committed 64-bit framebuffer address), it updates `ScanoutState`:

- `source = Wddm`
- `base_paddr = value programmed by driver`
- `width/height/pitch/format = values programmed by driver`

From this point onward, legacy VGA/VBE writes do not affect the visible display while WDDM owns scanout.

#### Compatibility rule once WDDM is active

After the first successful WDDM scanout enable (`SCANOUT0_ENABLE=1`):

- Legacy VGA/VBE ports and memory windows may continue to accept reads/writes for compatibility.
- The emulator presentation must ignore legacy sources once WDDM has claimed scanout.
- If WDDM disables scanout (`SCANOUT0_ENABLE=0`), it blanks output (stopping vblank pacing) but does
  not release ownership: legacy output remains suppressed until VM reset.
- VM reset always releases WDDM ownership and reverts to legacy scanout.

This prevents legacy writes (e.g. an errant `INT 10h`) from stealing the primary display after the desktop is up.

---

### Browser canvas presentation requirements

The emulator must render:

1. VGA text mode (from `0xB8000` backing) during POST/bootloader
2. VBE LFB modes for Windows 7 boot + installer graphics
3. WDDM scanout after the AeroGPU driver loads

Presentation pipeline requirements:

- Source selection is based solely on `ScanoutState`.
- Pixel conversion handles BGRA/BGRX/RGBA/RGBX (and sRGB variants) → canvas RGBA. X8 formats must force `alpha=255`.
- Switching sources is atomic and does not free/relocate the backing memory, preventing stale pointers.

---

### Web runtime implementation notes (wasm32 browser runtime)

This section documents the **current** implementation used by the web/wasm32 runtime. It exists to
explain the somewhat non-obvious contract between:

- guest physical addresses (including the PCI/MMIO hole),
- the browser runtime’s `SharedArrayBuffer` layout, and
- scanout readback in the GPU worker.

#### VRAM (BAR1 backing) as a SharedArrayBuffer

In the web runtime, BAR1/VRAM is represented as a dedicated `SharedArrayBuffer` (separate from the
`WebAssembly.Memory` guest RAM buffer) so that:

- VRAM does not consume wasm32 linear memory budget (wasm32 is limited to 4 GiB without `memory64`),
- the I/O worker can service MMIO writes into BAR1, and
- the GPU worker can read back scanout/cursor pixels from the same bytes without copying.

Allocation and wiring:

- VRAM is allocated by the coordinator in
  [`apps/web/src/runtime/shared_layout.ts`](../../apps/web/src/runtime/shared_layout.ts)
  `allocateSharedMemorySegments(...)`.
  - Default size is `DEFAULT_VRAM_MIB = 64`.
  - `vramMiB=0` disables the segment (primarily for tests).
- Shared memory contract: when present, `segments.vram` backs the guest physical address range:
  `[VRAM_BASE_PADDR, VRAM_BASE_PADDR + vram.byteLength)`.
  (See `SharedMemorySegments.vram` in the same file.)
- The I/O worker maps this buffer as an MMIO region at `vramBasePaddr` so guest CPU reads/writes to
  BAR1 land in the VRAM SAB (see [`apps/web/src/workers/io.worker.ts`](../../apps/web/src/workers/io.worker.ts),
  `DeviceManager.registerMmio(...)`).
- The VRAM aperture reserves the front of the PCI/MMIO BAR allocation window. The I/O worker
  configures the PCI BAR allocator base to start *after* VRAM (`pciMmioBase = vramBasePaddr +
  vramSizeBytes`) so other MMIO BARs do not overlap the VRAM range (see
  `DeviceManagerOptions.pciMmioBase` in [`apps/web/src/io/device_manager.ts`](../../apps/web/src/io/device_manager.ts)).

#### VRAM base paddr contract and why `base_paddr` can live in the PCI/MMIO hole

The web runtime uses fixed constants for BAR placement:

- `PCI_MMIO_BASE = 0xE000_0000` in
  [`apps/web/src/arch/guest_phys.ts`](../../apps/web/src/arch/guest_phys.ts) and
  [`crates/aero-wasm/src/guest_layout.rs`](../../crates/aero-wasm/src/guest_layout.rs).
- `VRAM_BASE_PADDR = PCI_MMIO_BASE` (i.e. VRAM lives inside the canonical PC/Q35 PCI/MMIO hole).

Note: this is a **web-runtime implementation contract**. It differs from the canonical
`aero_machine` model where BAR bases are assigned by BIOS and must be treated as dynamic by the
guest. In the browser runtime today, VRAM is reserved at a fixed guest-physical address to keep
multi-worker mapping simple; if/when BAR1 becomes a fully dynamic PCI BAR in the web runtime, this
contract may be revisited.

To avoid overlapping the contiguous wasm linear-memory guest RAM buffer with the PCI BAR window,
the web runtime clamps `guest_size <= PCI_MMIO_BASE` (see `computeGuestRamLayout(...)` in
`apps/web/src/runtime/shared_layout.ts`).

Combined with the Q35-style physical map used by the web runtime (`LOW_RAM_END = 0xB000_0000`,
`HIGH_RAM_START = 0x1_0000_0000`), guest RAM is backed by `WebAssembly.Memory` but guest *physical*
addresses are not always identity-mapped:

- If `guest_size <= LOW_RAM_END`, RAM is contiguous and identity-mapped: `[0, guest_size)` is RAM.
- If `guest_size > LOW_RAM_END`, RAM is split:
  - Low RAM:  `[0, LOW_RAM_END)` (backed by wasm memory at offset 0)
  - Hole:     `[LOW_RAM_END, HIGH_RAM_START)` (ECAM + PCI/MMIO; **not** backed by RAM)
  - High RAM: `[HIGH_RAM_START, HIGH_RAM_START + (guest_size - LOW_RAM_END))` (backed by the
    “high” portion of the contiguous wasm buffer)

BAR1/VRAM is mapped into the hole at `VRAM_BASE_PADDR = 0xE000_0000`, so scanout/cursor pointers
can legitimately live in the PCI/MMIO region.

This is why `ScanoutState.base_paddr` can point into the PCI/MMIO hole while still being valid: it
is a **guest physical address**, not a “RAM offset”.

#### RAM translation helpers (Q35 hole + high-RAM remap)

Because the PC/Q35 guest physical map includes an ECAM/MMIO hole and can remap “high RAM” above
4 GiB, the web runtime cannot treat `guestU8[paddr]` as valid once those features are in play.

Any code that needs to read guest RAM from a guest physical address must use the shared translation
helpers:

- JS: [`apps/web/src/arch/guest_ram_translate.ts`](../../apps/web/src/arch/guest_ram_translate.ts)
  - `guestPaddrToRamOffset(ramBytes, paddr)` → `number | null`
  - `guestRangeInBounds(ramBytes, paddr, len)` → `boolean`
- Rust mirror (used by wasm-side DMA bridges): [`crates/aero-wasm/src/guest_phys.rs`](../../crates/aero-wasm/src/guest_phys.rs)
  (`translate_guest_paddr_range`, `translate_guest_paddr_chunk`, etc.)

In addition to scanout/cursor readback, the GPU worker’s TypeScript AeroGPU command executor uses
the same “RAM vs VRAM aperture” resolution policy when it needs to slice guest physical memory for
DMA-style operations (uploads/copies/etc):

- `apps/web/src/workers/aerogpu-acmd-executor.ts` (`sliceGuestChecked(...)`)

#### How the GPU worker resolves scanout `base_paddr` (RAM vs VRAM)

In the browser runtime, scanout presentation happens in
[`apps/web/src/workers/gpu-worker.ts`](../../apps/web/src/workers/gpu-worker.ts).

For `ScanoutState.source = Wddm` (`SCANOUT_SOURCE_WDDM`), the worker:

1. Snapshots the shared scanout descriptor (`scanoutState` SAB).
2. Computes the required byte range:
   `requiredReadBytes = (height-1)*pitchBytes + width*bytesPerPixel`, where `bytesPerPixel` is derived
   from `ScanoutState.format` (typically 4 for `*8G8R8*8` formats and 2 for `B5G6R5` / `B5G5R5A1`).
3. Resolves `base_paddr` to a backing store:
   - If `base_paddr ∈ [vramBasePaddr, vramBasePaddr + vramSizeBytes)`, read from the VRAM SAB
     (`vramU8`) at offset `base_paddr - vramBasePaddr`.
   - Otherwise treat it as guest RAM and use `guestRangeInBounds` + `guestPaddrToRamOffset` to
     translate it into an offset into `guestU8`.
4. Converts the scanout surface to a tightly-packed RGBA8 buffer for presentation:
   - X8 formats force `alpha=0xFF` (fully opaque); A8 formats preserve alpha.
   - For `*_SRGB` scanout formats, the GPU worker decodes sRGB→linear after swizzle so blending/presentation happens in linear space.

Notes on `base_paddr == 0` for `source=Wddm`:

- **Disabled WDDM descriptor** (matches the required scanout handoff contract above): when the guest disables scanout after WDDM has claimed it, the device model publishes a descriptor with
  `base/width/height/pitch = 0`. This represents **blank output** while WDDM retains scanout ownership (legacy output remains suppressed until reset).
- **Placeholder WDDM descriptor** (web-runtime/harness-only): some host-side harnesses publish `base_paddr=0` with **non-zero** `width/height/pitch` as a placeholder for the host-side AeroGPU path (no guest-memory scanout readback). This is not part of the guest-facing AeroGPU MMIO ABI; it is an implementation detail used by some tests.

This same “VRAM aperture fast-path” idea is also used for WDDM hardware cursor surfaces (which are
often allocated in VRAM).

#### Worker init / shared-memory fields (web runtime contract)

The coordinator hands these buffers to workers via the `postMessage` init message
[`WorkerInitMessage`](../../apps/web/src/runtime/protocol.ts):

- `vram`, `vramBasePaddr`, `vramSizeBytes`: describe the shared VRAM aperture (BAR1 backing).
- `scanoutState`, `scanoutStateOffsetBytes`: shared scanout descriptor
  (layout in [`apps/web/src/ipc/scanout_state.ts`](../../apps/web/src/ipc/scanout_state.ts)).
- `cursorState`, `cursorStateOffsetBytes`: shared hardware cursor descriptor.

Workers should treat these as immutable for the lifetime of a VM instance.

#### Current limitations

- WDDM scanout readback currently supports:
  - 32bpp packed: `B8G8R8X8` / `B8G8R8A8` / `R8G8B8X8` / `R8G8B8A8` (plus their sRGB variants).
  - 16bpp packed: `B5G6R5` (opaque) and `B5G5R5A1` (1-bit alpha).
- WDDM hardware cursor surfaces support `B8G8R8X8` / `B8G8R8A8` / `R8G8B8X8` / `R8G8B8A8`
  (plus their sRGB variants).
- Readback paths require `base_paddr` and derived byte ranges to fit within JS safe integer range
  (`<= 2^53-1`).
- Some unit tests/harnesses set `vramMiB=0`, in which case VRAM-backed scanout/cursor surfaces are
  unavailable.

#### Tests / validation pointers

- Rust: scanout handoff + disable semantics
  - `bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_scanout_handoff --locked`
  - `bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_scanout_disable_publishes_wddm_disabled --locked`
- E2E (guest RAM scanout): `bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/wddm_scanout_smoke.spec.ts`
  (harness: `apps/web/wddm-scanout-smoke.ts`)
- E2E (VRAM aperture scanout): `bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/wddm_scanout_vram_smoke.spec.ts`
  (harness: `apps/web/wddm-scanout-vram-smoke.ts`)

---

### Acceptance checklist (manual)

In the emulator UI:

1. Power on and see BIOS POST text output.
2. Windows 7 boot graphics appears (not just blind boot).
3. Windows 7 installer UI appears in a VBE LFB mode.
4. After AeroGPU WDDM driver loads, the desktop continues rendering using the WDDM scanout path without flashing back to VGA/VBE.

## D3D9 SM2/SM3 shader translation status (aero-d3d9)

This doc is a small “don’t duplicate work” scratchpad for the D3D9 Shader Model 2/3
translator (`crates/aero-d3d9/src/sm3/`).

It tracks task-level status for shader bytecode → IR → WGSL lowering work.

Translation output is cached:

- Per-session in-memory cache: `crates/aero-d3d9/src/shader_translate.rs` (`ShaderCache`)
- WASM-only persistent cache (IndexedDB/OPFS): `crates/aero-d3d9/src/runtime/shader_cache.rs` +
  `apps/web/gpu-cache/persistent_cache.ts` (wired into `crates/aero-gpu/src/aerogpu_d3d9_executor.rs`)

For the broader “scratchpad task ID → implementation/test” audit, see
[`task-489-sm3-dxbc-sharedsurface-audit.md`](../history/retirements.md).

### Task status

#### Derivatives (`dsx`/`dsy`) and `dp2`

**What:** Support `dsx`/`dsy` derivative ops (lowered to WGSL `dpdx`/`dpdy`) and the `dp2` opcode, including
predication-safe lowering for derivatives (avoid non-uniform control flow).

**Where:**
- `crates/aero-d3d9/src/sm3/{decode.rs,ir_builder.rs,verify.rs,wgsl.rs}`

**Tests:**
- `crates/aero-d3d9/tests/sm3_wgsl.rs`
  - `wgsl_dsx_dsy_derivatives_compile`
  - `wgsl_dsx_dsy_can_feed_texldd_gradients`
  - `wgsl_predicated_derivative_avoids_non_uniform_control_flow`
- `crates/aero-d3d9/tests/sm3_wgsl_dp2.rs`

#### Texture sampling lowering

**What:** Texture sampling lowering via `IrOp::TexSample`, plus texture/sampler binding emission and
bind layout population (`bind_group_layout.{sampler_group,sampler_bindings,sampler_texture_types}`).

Sampler texture types come from `dcl_* s#` when present; when absent, samplers default to `Texture2D`
(and this default is recorded in `bind_group_layout.sampler_texture_types`).

Supported texture types in the SM3 WGSL backend: 1D/2D/3D/cube, with coordinate dimensionality
`x`/`xy`/`xyz` (including for `texldp`/`texldb`/`texldd`/`texldl`).

Note: Translation supports 1D/2D/3D/cube samplers (including sampling ops). The remaining limitation
is on the runtime/protocol side: the AeroGPU D3D9 command stream can only create guest-backed **2D**
textures today (`CREATE_TEXTURE2D`; cube is represented as `array_layers=6`), so it cannot yet supply
real guest-backed 1D/3D textures. Until the protocol gains 1D/3D texture creation/binding, the D3D9
executor binds a dummy 1D/3D texture view for those sampler dimensions.

Unknown sampler-type encodings are still rejected by the translation entrypoint:
`validate_sampler_texture_types` in `crates/aero-d3d9/src/shader_translate.rs`.

Note: WGSL does not support `textureSampleBias` for `texture_1d`, so SM3 `texldb` with a 1D sampler is
lowered via `textureSampleGrad` with `dpdx`/`dpdy` scaled by `exp2(bias)`.

Note: `texld`/`texldp`/`texldb` use implicit derivatives in WGSL (`textureSample*`) and must execute in
uniform control flow. Predicated texture sampling lowers via unconditional sampling + `select(...)`
rather than `if (p0) { ... }` to satisfy naga uniformity validation.

**Where:**
- `crates/aero-d3d9/src/sm3/wgsl.rs`

**Tests:**
- `crates/aero-d3d9/tests/sm3_wgsl.rs` (core texld/texldp/texldd/texldl + sampler `dcl_*` texture-type coverage + wgpu pipeline-layout compatibility check)
- `crates/aero-d3d9/tests/sm3_wgsl_tex.rs` (additional sampler-type coverage)

**Binding contract (AeroGPU + translators):**
- `@group(0)` — constants shared by VS/PS (packed for stable bindings across stages)
  - `@binding(0)` — `Constants` UBO:
    - `c: array<vec4<f32>, 512>` — float4 constants (`c#`)
    - `i: array<vec4<i32>, 512>` — int4 constants (`i#`)
    - `b: array<vec4<u32>, 128>` — bool constants (`b#`, packed 4 per element; `b0` is `b[0].x`, etc)
- `@group(1)` — VS samplers/textures
- `@group(2)` — PS samplers/textures
- For sampler `sN`, bindings are `(2*N, 2*N+1)` for `(texture, sampler)`

#### `texkill` semantics and predication

**What:** `texkill` lowering (`discard` if **any** component `< 0`) and correct predication behavior
(predicated `texkill` must be nested under an `if`).

**Where:**
- `crates/aero-d3d9/src/sm3/ir_builder.rs` (`Opcode::TexKill`)
- `crates/aero-d3d9/src/sm3/wgsl.rs` (`Stmt::Discard`)

**Tests:**
- `crates/aero-d3d9/tests/sm3_wgsl.rs`
  - `wgsl_texkill_is_conditional`
  - `wgsl_predicated_texkill_is_nested_under_if`

#### Pixel-shader position and facing builtins

**What:** Pixel shader `MISCTYPE` builtins:

- `misc0` (vPos) maps to WGSL `@builtin(position)` (in `FsIn.frag_pos`) and is exposed to the shader
  body as `misc0: vec4<f32>`.
- `misc1` (vFace) maps to WGSL `@builtin(front_facing)` and is exposed to the shader body as a
  D3D-style `misc1: vec4<f32>` with `face` = `+1` or `-1`.

**Where:**
- `crates/aero-d3d9/src/sm3/wgsl.rs` (misc input tracking + builtin emission)

**Tests:**
- `crates/aero-d3d9/tests/sm3_wgsl.rs`
  - `wgsl_ps3_vpos_misctype_builtin_compiles`
  - `wgsl_ps3_vface_misctype_builtin_compiles`

#### Pixel-shader depth output

**What:** Support pixel shader depth output by lowering D3D9 `oDepth` / `RegFile::DepthOut` to WGSL
`@builtin(frag_depth)` and assigning from `oDepth.x`.

**Where:**
- `crates/aero-d3d9/src/sm3/wgsl.rs`

**Tests:**
- `crates/aero-d3d9/tests/sm3_wgsl_depth_out.rs`
  - `wgsl_ps30_writes_odepth_emits_frag_depth`

#### Half-pixel centre convention

**What:** Optional emulation of D3D9’s “half-pixel offset” by nudging the final clip-space vertex
position by `(-1/viewport_width, +1/viewport_height) * w` in translated vertex shaders.

**Where:**
- SM3-first translation path: `crates/aero-d3d9/src/shader_translate.rs` (`inject_half_pixel_center_sm3_vertex_wgsl`)
- Legacy fallback translator: `crates/aero-d3d9/src/shader.rs` (`WgslOptions::half_pixel_center`)
- Executor plumbing: `crates/aero-gpu/src/aerogpu_d3d9_executor.rs` (bind group(3) uniform updated on `SetViewport`)

**Test:**
- `crates/aero-gpu/tests/aerogpu_d3d9_half_pixel_center.rs`

### Remaining / known limitations (true delta)

- Sampler state mapping (filtering, address modes, LOD bias, etc.) is handled in the runtime pipeline setup,
  not in the SM2/SM3 WGSL generator. Comparison samplers / depth-compare sampling are not modeled here yet.
- The runtime/protocol can only supply guest-backed **2D** textures today (`CREATE_TEXTURE2D`; cube uses `array_layers=6`).
  1D/3D samplers are translated, but are backed by dummy 1D/3D textures in the D3D9 executor until the protocol grows
  real 1D/3D texture creation/binding.

## Graphics status (Windows 7 UX)

This is the **single authoritative status doc** for the graphics stack.
It tracks what is **implemented in-tree today** vs what is still **missing** to reach a “Windows 7 feels usable” experience (boot → desktop → DWM/Aero + apps).

Legend:

- `[x]` = implemented (exists in-tree and has tests)
- `[~]` = partial / stubbed / exists in an alternate stack (see notes)
- `[ ]` = missing / not wired / not validated end-to-end

Coordination note:

- For mapping from “legacy agent scratchpad task IDs” (SM3/DXBC/shared-surface) to the current
  in-tree implementations/tests, see
  [`../history/retirements.md`](../history/retirements.md).

### Read first (architecture + contracts)

- [`graphics.md`](graphics.md) — architecture overview
- [`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md) — canonical AeroGPU PCI identity contract
- [`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md) — how the canonical machine (`aero_machine`) drives AeroGPU submission execution + fence forward progress (no-op bring-up vs submission bridge vs in-process backends)
- [`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md) — stable `backing_alloc_id` semantics for Win7/WDDM 1.1 guest-backed resources (required for correct host-side alloc resolution)
- [`graphics.md`](graphics.md) — required VGA/VBE compatibility + boot→WDDM scanout handoff
- [`../specs/windows7-aerogpu-wddm-driver.md`](../specs/windows7-aerogpu-wddm-driver.md) — Win7 vblank/present timing contract (DWM stability)

> Scope note: the repo currently contains both:
>
> - a **canonical machine integration** (`crates/aero-machine`, surfaced to the browser via `crates/aero-wasm`), and
> - a sandbox/legacy “monolithic emulator” crate, since retired.
>
> This doc calls out both where it matters, but treats `aero-machine` as the canonical integration surface unless explicitly marked “legacy/sandbox”.
>
> See also: [`platform-and-firmware.md`](platform-and-firmware.md) (the machine and device stack, and the account of the retired second stack).

---

### At-a-glance matrix

| Area | Status | Where to look |
|---|---|---|
| Boot display (VGA text + VBE LFB) | `[x]` | [`crates/aero-gpu-vga/`](../../crates/aero-gpu-vga/) wired into [`crates/aero-machine/`](../../crates/aero-machine/) |
| AeroGPU ABI (C headers + Rust/TS mirrors + ABI tests) | `[x]` | [`drivers/aerogpu/protocol/`](../../drivers/aerogpu/protocol/) + [`crates/aero-protocol/aerogpu/`](../../crates/aero-protocol/aerogpu/) |
| AeroGPU PCI identity + BAR0/BAR1 transport + ring decode (submission bridge) | `[~]` | [`crates/aero-machine/src/lib.rs`](../../crates/aero-machine/src/lib.rs) + [`crates/aero-machine/src/aerogpu.rs`](../../crates/aero-machine/src/aerogpu.rs) |
| AeroGPU CPU rasterizer (executes the command stream with no adapter) | `[x]` | [`crates/aero-gpu-software/src/lib.rs`](../../crates/aero-gpu-software/src/lib.rs) |
| Scanout shared-memory contracts | `[x]` | [`crates/aero-shared/src/`](../../crates/aero-shared/src/) + [`apps/web/src/ipc/`](../../apps/web/src/ipc/) |
| D3D9 translation/execution (subset) | `[~]` | [`crates/aero-d3d9/`](../../crates/aero-d3d9/) + [`crates/aero-gpu/src/aerogpu_d3d9_executor.rs`](../../crates/aero-gpu/src/aerogpu_d3d9_executor.rs) + [`graphics.md`](graphics.md) |
| D3D10/11 translation/execution (subset; VS/PS/CS + GS compute-prepass (translated GS prepass supports `PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj` IA topologies; other cases use synthetic expansion)) | `[~]` | [`crates/aero-d3d11/`](../../crates/aero-d3d11/) |
| Web presenters/backends (WebGPU + WebGL2) | `[x]` | [`apps/web/src/gpu/`](../../apps/web/src/gpu/) |
| End-to-end Win7 WDDM + accelerated rendering in the **canonical browser machine** | `[ ]` | See [7) Critical path integration gaps](#7-current-critical-path-integration-gaps-factual) |

---

### Boot display (VGA text, VBE LFB)

Win7 UX goal: the **same virtual GPU** should provide both boot VGA/VBE output and the later WDDM scanout path (no device swap).

#### Implemented today: standalone VGA/VBE device (`crates/aero-gpu-vga`)

Status checklist:

- [x] VGA register file emulation (sequencer/graphics/attribute/CRTC)
- [x] Text mode rendering (80×25) with built-in bitmap font + cursor
- [x] Mode 13h rendering (320×200×256, chain-4)
- [x] Bochs/QEMU-style VBE (`VBE_DISPI`) register interface + linear framebuffer backing

Code pointers:

- [`crates/aero-gpu-vga/src/lib.rs`](../../crates/aero-gpu-vga/src/lib.rs) (`VgaDevice`, VBE LFB at configurable base; legacy default `SVGA_LFB_BASE`)

Test pointers:

- [`crates/aero-gpu-vga/src/lib.rs`](../../crates/aero-gpu-vga/src/lib.rs) (module `tests`)
  - `text_mode_golden_hash`
  - `mode13h_golden_hash`
  - `vbe_linear_framebuffer_write_shows_up_in_output`

CI/regression command:

```bash
# Runs the boot-display stack end-to-end (VGA/VBE device model + BIOS INT10 + machine wiring).
bash ./scripts/ci/run-vga-vbe-tests.sh
```

#### Wired into the canonical machine (`crates/aero-machine`)

When `MachineConfig::enable_vga=true`, `aero_machine::Machine` wires the VGA/VBE device model for boot display.

Note: VBE LFB routing depends on `enable_pc_platform`:

- When `enable_pc_platform=false`, the VBE LFB MMIO aperture is mapped directly at the configured base.
- When `enable_pc_platform=true`, the machine exposes a minimal Bochs/QEMU-compatible “Standard VGA”
  PCI function (currently `00:0c.0`, `1234:1111`) and routes the VBE LFB through its BAR0 inside the ACPI-reported
  PCI MMIO window / BAR router (BAR base assigned by BIOS POST / the PCI allocator, and may be
  relocated when other PCI devices are present). The machine mirrors the assigned BAR base into the
  BIOS VBE `PhysBasePtr` and the VGA device model so guests observe a coherent LFB base.

Code pointers:

- [`crates/aero-machine/src/lib.rs`](../../crates/aero-machine/src/lib.rs)
  - `MachineConfig::enable_vga` docs (port + address ranges)
  - `Machine::reset` (device wiring)
  - `Machine::display_present` / `display_framebuffer` / `display_resolution` (host-facing RGBA8888 snapshot)

Test pointers:

- [`crates/aero-machine/tests/boot_int10_vbe_sets_mode.rs`](../../crates/aero-machine/tests/boot_int10_vbe_sets_mode.rs) (INT 10h VBE mode set)
- [`crates/aero-machine/tests/boot_int10_active_page_renders_text.rs`](../../crates/aero-machine/tests/boot_int10_active_page_renders_text.rs) (text mode active-page behavior)
- [`crates/aero-machine/tests/vga_vbe_lfb_pci.rs`](../../crates/aero-machine/tests/vga_vbe_lfb_pci.rs) (VBE LFB reachable via the PC platform MMIO mapping)

#### Implemented today: AeroGPU boot-display foundation (`enable_aerogpu=true`)

`MachineConfig::enable_aerogpu=true` disables the standalone VGA device and instead provides:

- [x] BAR1-backed VRAM
- [x] legacy VGA window decode (`0xA0000..0xBFFFF`) backed by BAR1 VRAM (mode-dependent aliasing):
  - VBE inactive: `0xA0000..0xBFFFF` ↔ `VRAM[0x00000..0x1FFFF]`
  - VBE active:
    - `0xA0000..0xAFFFF` becomes the VBE banked window into `VRAM[VBE_LFB_OFFSET + bank*64KiB + off]`
    - `0xB0000..0xBFFFF` remains `VRAM[0x10000..0x1FFFF]`
- [x] BIOS VBE LFB base set into BAR1: `PhysBasePtr = BAR1_BASE + VBE_LFB_OFFSET` (`VBE_LFB_OFFSET = 0x40000`, protocol: `AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES`)
- [x] host-side presentation fallback when VGA is disabled:
  - If WDDM scanout0 has been claimed:
    - `SCANOUT0_ENABLE=1`: present the WDDM scanout framebuffer
    - `SCANOUT0_ENABLE=0`: present a blank frame (WDDM ownership is sticky; no fallback to legacy until reset)
  - Otherwise, present in priority order:
    - VBE LFB (from BIOS state)
    - VGA mode 13h (320×200×256) (from BIOS state)
    - text mode (scan `0xB8000`)

Implementation note: `SCANOUT0_ENABLE` is treated as a **visibility toggle**, not an ownership release.
Clearing it (`SCANOUT0_ENABLE=0`) blanks output (and stops vblank pacing / flushes vsync-paced fences), but keeps the sticky
`wddm_scanout_active` latch held so legacy VGA/VBE cannot reclaim scanout until reset.

- Ownership latch + disable handling: [`crates/aero-machine/src/aerogpu.rs`](../../crates/aero-machine/src/aerogpu.rs)
  (`AEROGPU_MMIO_REG_SCANOUT0_ENABLE`, `wddm_scanout_active`) + unit test `scanout_disable_keeps_wddm_ownership_latched`.
- Host-side presentation behavior when disabled: [`crates/aero-machine/src/lib.rs`](../../crates/aero-machine/src/lib.rs)
  (`display_present_aerogpu_scanout`).

Code pointers:

- [`crates/aero-machine/src/lib.rs`](../../crates/aero-machine/src/lib.rs)
  - `MachineConfig::enable_aerogpu` docs
  - `Machine::display_present` + `display_present_aerogpu_*` helpers

Test pointers:

- [`crates/aero-machine/tests/boot_int10_aerogpu_vbe_115_sets_mode.rs`](../../crates/aero-machine/tests/boot_int10_aerogpu_vbe_115_sets_mode.rs)
- [`crates/aero-machine/tests/aerogpu_text_mode_scanout.rs`](../../crates/aero-machine/tests/aerogpu_text_mode_scanout.rs)
- [`crates/aero-machine/tests/aerogpu_vbe_lfb_base_bar1.rs`](../../crates/aero-machine/tests/aerogpu_vbe_lfb_base_bar1.rs)
- [`crates/aero-machine/tests/aerogpu_vbe_clear_fastpath.rs`](../../crates/aero-machine/tests/aerogpu_vbe_clear_fastpath.rs) (VBE clear/no-clear semantics + preserve pre-LFB VRAM)

#### Missing / still required (boot → WDDM)

- [~] Boot framebuffer → WDDM scanout handoff: host-facing `Machine::display_present` prefers WDDM scanout once scanout0 is claimed by a valid configuration (and enabled), but this path still needs end-to-end validation in the browser runtime and shared-scanout publication (see Section 7).
  - Code: [`crates/aero-machine/src/lib.rs`](../../crates/aero-machine/src/lib.rs) (`display_present`, `display_present_aerogpu_scanout`)
  - Contract/design: [`graphics.md`](graphics.md)

---

### AeroGPU protocol + device model (ABI + host-side processors)

#### Canonical ABI “source of truth” (C headers)

The canonical AeroGPU ABI is defined in C headers under `drivers/aerogpu/protocol/`.

Code pointers:

- [`drivers/aerogpu/protocol/`](../../drivers/aerogpu/protocol/)
  - [`drivers/aerogpu/protocol/aerogpu_pci.h`](../../drivers/aerogpu/protocol/aerogpu_pci.h) (PCI IDs, MMIO register map, feature bits)
  - [`drivers/aerogpu/protocol/aerogpu_ring.h`](../../drivers/aerogpu/protocol/aerogpu_ring.h) (submission ring + fence page)
  - [`drivers/aerogpu/protocol/aerogpu_cmd.h`](../../drivers/aerogpu/protocol/aerogpu_cmd.h) (ACMD packet stream)
  - [`drivers/aerogpu/protocol/aerogpu_wddm_alloc.h`](../../drivers/aerogpu/protocol/aerogpu_wddm_alloc.h), [`drivers/aerogpu/protocol/aerogpu_escape.h`](../../drivers/aerogpu/protocol/aerogpu_escape.h) (WDDM-facing structs)

#### Rust + TypeScript mirrors (`aero-protocol` crate)

The Rust/TS mirrors live in the **`aero-protocol`** crate, located at `crates/aero-protocol/`:

- [`crates/aero-protocol/Cargo.toml`](../../crates/aero-protocol/Cargo.toml) (package name: `aero-protocol`)
- [`crates/aero-protocol/aerogpu/`](../../crates/aero-protocol/aerogpu/) (Rust `*.rs` and TS `*.ts` mirrors)

Test pointers (ABI conformance / drift detection):

- [`crates/aero-protocol/tests/aerogpu_abi.rs`](../../crates/aero-protocol/tests/aerogpu_abi.rs) (Rust sizes/offsets/consts)
- [`crates/aero-protocol/tests/aerogpu_abi.test.ts`](../../crates/aero-protocol/tests/aerogpu_abi.test.ts) (TS sizes/offsets/consts)
- [`crates/aero-protocol/tests/aerogpu_pci_id_conformance.rs`](../../crates/aero-protocol/tests/aerogpu_pci_id_conformance.rs)

#### Device models

##### Canonical machine (`crates/aero-machine`): BAR0/BAR1 + backend boundary (submission bridge / in-process backends)

`MachineConfig::enable_aerogpu=true` exposes the canonical identity:

- [x] `VID:DID = A3A0:0001`
- [x] BDF `00:07.0`
- [x] BAR1 VRAM + legacy VGA window aliasing
- [~] BAR0 MMIO register block + ring/fence transport + scanout/vblank + cursor + error-info registers
  - Ring processing decodes `aerogpu_ring` submissions and can capture `AEROGPU_CMD` payloads into a bounded queue
    for host-driven execution (`Machine::aerogpu_drain_submissions`).
  - Fence forward-progress policy is selectable:
    - default (no backend, submission bridge disabled): fences complete automatically (bring-up / no-op execution),
      with optional vblank pacing when vblank is active and the submission contains a vsync present.
    - submission bridge enabled (`Machine::aerogpu_enable_submission_bridge`): fences are deferred until the host reports completion (`Machine::aerogpu_complete_fence`).
      - The device model **still applies vblank pacing** for vsync PRESENT submissions in this mode: once the host reports a fence as completed,
        the fence becomes eligible to complete on the **next vblank** (and blocks later immediate fences until then, mirroring the emulator device model).
      - The external executor (e.g. browser GPU worker) should generally report completion as soon as execution finishes, rather than trying to “fake vsync”
        by delaying `submit_complete` based on host tick cadence.
    - in-process backend installed: fences complete when the backend reports completions (see below).
  - Error-info latches are implemented (ABI 1.3+) behind `AEROGPU_FEATURE_ERROR_INFO` (`AEROGPU_MMIO_REG_ERROR_*` + `AEROGPU_IRQ_ERROR`).

Code pointers:

- [`crates/aero-machine/src/lib.rs`](../../crates/aero-machine/src/lib.rs) (`MachineConfig::enable_aerogpu`, BAR1 aliasing, display helpers)
- [`crates/aero-machine/src/aerogpu.rs`](../../crates/aero-machine/src/aerogpu.rs) (BAR0 register model, ring decode, submission bridge + backend boundary, vblank/scanout/cursor)

Test pointers:

- [`crates/aero-machine/tests/pci_display_bdf_contract.rs`](../../crates/aero-machine/tests/pci_display_bdf_contract.rs) (BDF contract)
- [`crates/aero-machine/tests/machine_aerogpu_pci_identity.rs`](../../crates/aero-machine/tests/machine_aerogpu_pci_identity.rs)
- [`crates/aero-machine/tests/aerogpu_ring_noop_fence.rs`](../../crates/aero-machine/tests/aerogpu_ring_noop_fence.rs) (default “bring-up” completion policy + drains submissions)
- [`crates/aero-machine/tests/aerogpu_submission_bridge.rs`](../../crates/aero-machine/tests/aerogpu_submission_bridge.rs) (submission bridge requires host fence completion)
- [`crates/aero-machine/tests/aerogpu_immediate_backend_completes_fence.rs`](../../crates/aero-machine/tests/aerogpu_immediate_backend_completes_fence.rs) (in-process backend APIs)
- [`crates/aero-machine/tests/aerogpu_bar0_mmio_vblank.rs`](../../crates/aero-machine/tests/aerogpu_bar0_mmio_vblank.rs)

In-process backend APIs (native/tests):

- `Machine::aerogpu_set_backend_immediate()` / `Machine::aerogpu_set_backend_null()` (always available)
  - Test: `bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_immediate_backend_completes_fence --locked`
- `Machine::aerogpu_set_backend_wgpu()` (feature-gated, native-only: `aero-machine/aerogpu-wgpu-backend`)
  - Build sanity: `bash ./scripts/safe-run.sh cargo test -p aero-machine --features aerogpu-wgpu-backend --locked`
  - Canonical end-to-end smoke test (executes real ACMD + validates host-visible scanout):
    `AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-machine --features aerogpu-wgpu-backend --test aerogpu_wgpu_backend_smoke --locked`
  - Additional backend end-to-end coverage also lives in `aero-devices-gpu`:
    `bash ./scripts/safe-run.sh cargo test -p aero-devices-gpu --features wgpu-backend --test aerogpu_end_to_end --locked`

##### Canonical browser runtime (`crates/aero-wasm` + `apps/web/`): submission bridge + GPU worker execution

The canonical browser integration runs:

- the guest-visible AeroGPU PCI/MMIO device model **in the CPU worker** (inside the `aero-wasm` `Machine`), and
- `AEROGPU_CMD` execution **in the GPU worker** (`apps/web/src/workers/gpu-worker.ts`).

Message flow (high level):

1. Guest rings the AeroGPU doorbell (BAR0 MMIO), causing the in-process device model to decode ring entries.
2. CPU worker calls `Machine.aerogpu_drain_submissions()` (WASM export) and posts each submission to the coordinator:
   `kind: "aerogpu.submit"` (payload includes `cmdStream`, optional `allocTable`, and `signalFence`).
3. Coordinator buffers submissions until the GPU worker is READY, then forwards them to the GPU worker using the GPU
   protocol message `type: "submit_aerogpu"`.
4. GPU worker executes the command stream (TypeScript CPU executor and/or `aero-gpu-wasm` D3D9 path) and replies with
   `type: "submit_complete"` containing `completedFence`.
5. Coordinator forwards `kind: "aerogpu.complete_fence"` to the CPU worker, which calls the WASM export
   `Machine.aerogpu_complete_fence(fence: BigInt)` to update the guest-visible fence page + IRQ status.

Robustness note (important for WDDM forward progress):

- While the GPU worker is not READY (startup/restart windows), the coordinator buffers `aerogpu.submit` messages in a
  bounded FIFO queue. If the queue overflows, it drops the oldest submission and **force-completes its fence** so the
  guest cannot deadlock waiting for a `submit_complete` that will never arrive.
- If posting `submit_aerogpu` to the GPU worker fails (for example: transfer list rejected, structured clone error, or the
  GPU worker is terminated while fences are in flight), the coordinator **force-completes** the affected fences. Rendering
  correctness is best-effort in these failure modes.

Code pointers:

- WASM bridge exports (enables external-executor semantics on first drain):
  - [`crates/aero-wasm/src/lib.rs`](../../crates/aero-wasm/src/lib.rs) (`Machine::aerogpu_drain_submissions`, `Machine::aerogpu_complete_fence`)
- CPU worker drain + completion queue:
  - [`apps/web/src/workers/machine_cpu.worker.ts`](../../apps/web/src/workers/machine_cpu.worker.ts) (`drainAerogpuSubmissions`, `processPendingAerogpuFenceCompletions`)
- Coordinator routing/buffering + fence forwarding:
  - [`apps/web/src/runtime/coordinator.ts`](../../apps/web/src/runtime/coordinator.ts)
    (`forwardAerogpuSubmit`, `sendAerogpuSubmitToGpuWorker`, `forwardAerogpuFenceComplete`, `completeInFlightAerogpuFences`)
  - [`apps/web/src/runtime/coordinator.test.ts`](../../apps/web/src/runtime/coordinator.test.ts)
    (“buffers aerogpu.submit…”, “force-completes dropped fences”, “forces completion of in-flight fences…”)
- GPU worker executor + vsync completion policy:
  - [`apps/web/src/workers/gpu-worker.ts`](../../apps/web/src/workers/gpu-worker.ts) (`handleSubmitAerogpu`, `enqueueAerogpuSubmitComplete`)

Test commands:

```bash
# Rust: submission bridge semantics (host must complete fences)
bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_submission_bridge --locked

# Web unit: coordinator message flow (aerogpu.submit -> submit_aerogpu -> submit_complete -> aerogpu.complete_fence)
AERO_TIMEOUT=600 AERO_MEM_LIMIT=32G bash ./scripts/safe-run.sh pnpm -C apps/web run test:unit -- src/runtime/coordinator.test.ts

# Playwright: GPU worker ACMD execution + submit_complete semantics
bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu.spec.ts
bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu_vsync_completion.spec.ts
```

##### The persistent shader cache stored everything and returned nothing

The browser-side persistent GPU cache (`apps/web/gpu-cache/persistent_cache.ts`) backs both
translation paths. It wrote entries by picking two fields out of the artifact by name — `wgsl` and
`reflection` — which is the shape D3D9 translation produces.

D3D11's artifact is a different shape: `wgsl`, `stage`, `bindings`, `vs_input_signature`. Every
D3D11 entry was therefore written back missing the fields its reader requires. On the next lookup
the record failed to deserialise, the reader treated it as corrupt, deleted it, and retranslated —
so the cache never once returned a D3D11 shader, and never reported an error either. Both failure
paths are best-effort by design, which is what made it silent.

The cache now stores the artifact whole and the reader is the only thing that knows its shape.
Records written in the old form still load, through a fallback to the two fields that shape had, so
an existing cache is not discarded.

##### The CPU rasterizer

`crates/aero-gpu-software` executes the AeroGPU command stream on the CPU — vertex fetch, triangle
setup, depth testing, texture sampling, blending — against plain memory. It needs no adapter and no
driver, which makes it the one backend that runs anywhere.

It is not currently reachable through `AeroGpuCommandBackend`: the rasterizer reads the command
stream and allocation table itself from guest physical addresses, and the trait hands a backend a
stream that has already been read. Bridging the two is a real design decision about where the memory
read belongs, and no adapter was written to paper over it.

- [`crates/aero-gpu-software/src/lib.rs`](../../crates/aero-gpu-software/src/lib.rs)
- [`crates/aero-gpu-software/tests/shared_surface_refcount.rs`](../../crates/aero-gpu-software/tests/shared_surface_refcount.rs)

##### Shared device-side library (`crates/aero-devices-gpu`): regs/ring/executor + portable PCI wrapper

The `crates/aero-devices-gpu` crate is the shared “device-side” home for:

- MMIO register constants + backing `AeroGpuRegs`,
- ring + fence page structs/helpers,
- the ring executor (doorbell processing, submission decode, fence tracking, vsync/vblank pacing), and
- a lightweight PCI device wrapper (`AeroGpuPciDevice`) that can be reused by multiple hosts.

Code pointers:

- [`crates/aero-devices-gpu/src/executor.rs`](../../crates/aero-devices-gpu/src/executor.rs)
- [`crates/aero-devices-gpu/src/pci.rs`](../../crates/aero-devices-gpu/src/pci.rs)
- [`crates/aero-devices-gpu/src/ring.rs`](../../crates/aero-devices-gpu/src/ring.rs)
- [`crates/aero-devices-gpu/src/regs.rs`](../../crates/aero-devices-gpu/src/regs.rs)

Test pointers:

- [`crates/aero-devices-gpu/tests/aerogpu_executor_decode.rs`](../../crates/aero-devices-gpu/tests/aerogpu_executor_decode.rs)
- [`crates/aero-devices-gpu/tests/aerogpu_pci_device.rs`](../../crates/aero-devices-gpu/tests/aerogpu_pci_device.rs)
- [`crates/aero-devices-gpu/tests/vram_bar1.rs`](../../crates/aero-devices-gpu/tests/vram_bar1.rs)
- Feature-gated wgpu end-to-end: [`crates/aero-devices-gpu/tests/aerogpu_end_to_end.rs`](../../crates/aero-devices-gpu/tests/aerogpu_end_to_end.rs) (run with `cargo test -p aero-devices-gpu --features wgpu-backend --test aerogpu_end_to_end`)

#### Host-side processors/executors (wgpu/WebGPU)

The canonical “host-side” consumption of the AeroGPU command stream lives in `crates/aero-gpu/` and friends.

Code pointers:

- Protocol parsing:
  - [`crates/aero-gpu/src/protocol.rs`](../../crates/aero-gpu/src/protocol.rs) (`parse_cmd_stream`, `AeroGpuCmd`)
- Command processors:
  - [`crates/aero-gpu/src/command_processor.rs`](../../crates/aero-gpu/src/command_processor.rs)
  - [`crates/aero-gpu/src/command_processor_d3d9.rs`](../../crates/aero-gpu/src/command_processor_d3d9.rs)
  - [`crates/aero-gpu/src/protocol_d3d11.rs`](../../crates/aero-gpu/src/protocol_d3d11.rs)
- Executors:
  - [`crates/aero-gpu/src/aerogpu_executor.rs`](../../crates/aero-gpu/src/aerogpu_executor.rs) (minimal executor)
  - [`crates/aero-gpu/src/aerogpu_d3d9_executor.rs`](../../crates/aero-gpu/src/aerogpu_d3d9_executor.rs) (D3D9-focused)
  - [`crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`](../../crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs) (D3D10/11-focused)

Test pointers:

- [`crates/aero-gpu/tests/`](../../crates/aero-gpu/tests/) (protocol + executor behavior)
  - Example: [`crates/aero-gpu/tests/aerogpu_ex_protocol.rs`](../../crates/aero-gpu/tests/aerogpu_ex_protocol.rs)

---

### Scanout contracts (shared memory)

There are two distinct shared-memory contracts used between Rust/WASM and JS:

1. `ScanoutState`: a compact, lock-free descriptor of *where the “current scanout” lives* (guest paddr + geometry).
2. `SharedFramebuffer`: a double-buffered RGBA8 framebuffer used for CPU-produced frames (with optional dirty-tile bitsets).

#### 3.1) `ScanoutState`

Status checklist:

- [x] Seqlock-style publish protocol (busy-bit in `generation`)
- [x] Explicit `source` enum (`LEGACY_TEXT`, `LEGACY_VBE_LFB`, `WDDM`)
- [x] TS mirror uses `Atomics.*` on an `Int32Array`

Code pointers:

- Rust: [`crates/aero-shared/src/scanout_state.rs`](../../crates/aero-shared/src/scanout_state.rs)
- TS mirror: [`apps/web/src/ipc/scanout_state.ts`](../../apps/web/src/ipc/scanout_state.ts)

Test pointers:

- Rust: [`crates/aero-shared/src/scanout_state.rs`](../../crates/aero-shared/src/scanout_state.rs) (unit + loom tests)
- TS: [`apps/web/src/ipc/scanout_state.test.ts`](../../apps/web/src/ipc/scanout_state.test.ts)

#### 3.2) `SharedFramebuffer`

Status checklist:

- [x] Stable, aligned shared layout (`SharedFramebufferLayout`)
- [x] Atomic header protocol (`active_index`, `frame_seq`, `frame_dirty`, per-buffer seq)
- [x] Optional per-tile dirty bitset + rect extraction
- [x] TS mirror layout + dirty-rect logic

Code pointers:

- Rust: [`crates/aero-shared/src/shared_framebuffer.rs`](../../crates/aero-shared/src/shared_framebuffer.rs)
- TS mirror: [`apps/web/src/ipc/shared-layout.ts`](../../apps/web/src/ipc/shared-layout.ts)

Test pointers:

- Rust: [`crates/aero-shared/src/shared_framebuffer.rs`](../../crates/aero-shared/src/shared_framebuffer.rs) (unit + loom tests)
- TS: [`apps/web/src/ipc/shared-layout.test.ts`](../../apps/web/src/ipc/shared-layout.test.ts)

---

### D3D9 stack (`crates/aero-d3d9*`)

#### What exists today

The D3D9 implementation is split into:

- D3D9 shader parsing/translation primitives in `crates/aero-d3d9` (+ legacy parser in `crates/legacy/aero-d3d9-shader`)
- a D3D9-focused AeroGPU command executor in `crates/aero-gpu` consuming `aerogpu_cmd.h` packets
- [x] D3D9 half-pixel center convention
  - Translation (SM3-first path): [`crates/aero-d3d9/src/shader_translate.rs`](../../crates/aero-d3d9/src/shader_translate.rs) injects the `@group(3) @binding(0)` `HalfPixel` uniform + clip-space XY adjustment into translated vertex WGSL when `WgslOptions::half_pixel_center` is enabled (`inject_half_pixel_center_sm3_vertex_wgsl`).
  - Translation (legacy fallback path): [`crates/aero-d3d9/src/shader.rs`](../../crates/aero-d3d9/src/shader.rs) emits the same `HalfPixel` uniform + adjustment for the legacy token-stream translator when `WgslOptions::half_pixel_center` is enabled.
  - Execution: [`crates/aero-gpu/src/aerogpu_d3d9_executor.rs`](../../crates/aero-gpu/src/aerogpu_d3d9_executor.rs) creates/binds the group(3) bind group and updates the uniform on `AeroGpuCmd::SetViewport`.
  - Test: [`crates/aero-gpu/tests/aerogpu_d3d9_half_pixel_center.rs`](../../crates/aero-gpu/tests/aerogpu_d3d9_half_pixel_center.rs) (`bash ./scripts/safe-run.sh cargo test -p aero-gpu --test aerogpu_d3d9_half_pixel_center --locked`)
- [x] SM3 derivatives (`dsx`/`dsy`) and gradient sampling (`texldd`)
  - Translation: [`crates/aero-d3d9/src/sm3/`](../../crates/aero-d3d9/src/sm3/) lowers derivatives to WGSL `dpdx`/`dpdy`.
  - Legacy-fallback translation: [`crates/aero-d3d9/src/shader.rs`](../../crates/aero-d3d9/src/shader.rs) also supports `dsx`/`dsy` for best-effort compatibility.
  - Tests: `crates/aero-d3d9/tests/sm3_wgsl.rs` (derivatives + `texldd`), `crates/aero-d3d9/src/tests.rs` (fallback path).
- [x] SM3 texture sampling + `texkill` semantics
  - `texld`/`texldp`/`texldb`/`texldd`/`texldl` lower to WGSL `textureSample*` variants, with texture/sampler binding emission and bind-layout mapping.
  - `texkill` lowers to `discard` when any component of the operand is `< 0`, preserving predication nesting.
  - Details + tests: [`graphics.md`](graphics.md)
- [x] Shader constant updates include int/bool registers (`SetShaderConstantsI` / `SetShaderConstantsB`)
  - Protocol: new D3D9 command stream opcodes in `drivers/aerogpu/protocol/aerogpu_cmd.h` (mirrored by `aero-protocol`).
  - Translation: shaders use a stable `@group(0) @binding(0)` `Constants` UBO with packed float/int/bool register banks; the bool bank is stored as `array<vec4<u32>, 128>` (4 scalar bool regs per element) to satisfy WGSL uniform layout rules while staying compact.
  - Execution: the D3D9 executor uploads float/int/bool constant data alongside other state.
  - Tests: `crates/aero-gpu/tests/aerogpu_d3d9_int_bool_constants.rs`, `aerogpu_d3d9_bool_constants.rs`, `aerogpu_d3d9_int_constants_dynamic.rs`, `aerogpu_d3d9_bool_constants_stage_isolation.rs`.
- [x] SM3 pixel shader `MISCTYPE` builtins: `misc0` (vPos) + `misc1` (vFace)
  - `misc0` (vPos) maps to WGSL `@builtin(position)` in [`FsIn.frag_pos`](../../crates/aero-d3d9/src/sm3/wgsl.rs), exposed to the shader body as `misc0: vec4<f32>`.
  - `misc1` (vFace) maps to WGSL `@builtin(front_facing)`, exposed as a D3D-style `misc1: vec4<f32>` where `face` is `+1` or `-1` replicated across all lanes.
  - Translation: [`crates/aero-d3d9/src/sm3/wgsl.rs`](../../crates/aero-d3d9/src/sm3/wgsl.rs)
  - Tests: [`crates/aero-d3d9/tests/sm3_wgsl.rs`](../../crates/aero-d3d9/tests/sm3_wgsl.rs)
    - `wgsl_ps3_vpos_misctype_builtin_compiles`
    - `wgsl_ps3_vface_misctype_builtin_compiles`
- [x] SM3 pixel shader depth output (`oDepth`)
  - D3D9 `oDepth` / `RegFile::DepthOut` lowers to WGSL `@builtin(frag_depth)` and is assigned from `oDepth.x`.
  - Translation: [`crates/aero-d3d9/src/sm3/wgsl.rs`](../../crates/aero-d3d9/src/sm3/wgsl.rs)
  - Test: [`crates/aero-d3d9/tests/sm3_wgsl_depth_out.rs`](../../crates/aero-d3d9/tests/sm3_wgsl_depth_out.rs)
    - `wgsl_ps30_writes_odepth_emits_frag_depth`
- [x] D3D9 shader translation cache (in-memory + WASM-only persistent cache)
  - In-memory cache: [`crates/aero-d3d9/src/shader_translate.rs`](../../crates/aero-d3d9/src/shader_translate.rs) (`ShaderCache`)
  - Persistent cache (WASM): [`crates/aero-d3d9/src/runtime/shader_cache.rs`](../../crates/aero-d3d9/src/runtime/shader_cache.rs) + browser backing store [`apps/web/gpu-cache/persistent_cache.ts`](../../apps/web/gpu-cache/persistent_cache.ts)
  - Executor wiring: [`crates/aero-gpu/src/aerogpu_d3d9_executor.rs`](../../crates/aero-gpu/src/aerogpu_d3d9_executor.rs)
  - Test (WASM): [`crates/aero-gpu/tests/wasm/aerogpu_d3d9_shader_cache_wasm.rs`](../../crates/aero-gpu/tests/wasm/aerogpu_d3d9_shader_cache_wasm.rs)

Code pointers:

- Translator + runtime primitives:
  - [`crates/aero-d3d9/`](../../crates/aero-d3d9/)
  - Legacy standalone shader parser (not used by the runtime): [`crates/legacy/aero-d3d9-shader/`](../../crates/legacy/aero-d3d9-shader/)
- AeroGPU D3D9 executor:
  - [`crates/aero-gpu/src/aerogpu_d3d9_executor.rs`](../../crates/aero-gpu/src/aerogpu_d3d9_executor.rs)

Representative test pointers:

- Translator tests: [`crates/aero-d3d9/src/tests.rs`](../../crates/aero-d3d9/src/tests.rs)
- Executor tests: [`crates/aero-gpu/tests/`](../../crates/aero-gpu/tests/)
  - [`crates/aero-gpu/tests/aerogpu_d3d9_triangle.rs`](../../crates/aero-gpu/tests/aerogpu_d3d9_triangle.rs)
  - [`crates/aero-gpu/tests/aerogpu_d3d9_fixedfunc_triangle.rs`](../../crates/aero-gpu/tests/aerogpu_d3d9_fixedfunc_triangle.rs)
- Guest-side Win7 tests live under [`drivers/aerogpu/tests/win7/`](../../drivers/aerogpu/tests/win7/) (see [`drivers/aerogpu/tests/win7/README.md`](../decisions/README.md)), including fixed-function regression coverage (`d3d9_fixedfunc_wvp_triangle`, `d3d9_fixedfunc_textured_wvp`, `d3d9_fixedfunc_lighting_directional`).

Known gaps / limitations (enforced by code):

- Shader translation rejects unsupported tokens/opcodes:
  - [`crates/aero-d3d9/src/shader.rs`](../../crates/aero-d3d9/src/shader.rs) (`ShaderError::Unsupported*`)
- Sampler texture types (1D/2D/3D/cube):
  - Translation accepts used sampler texture types 1D/2D/3D/cube and rejects only unknown sampler-type encodings:
    - [`crates/aero-d3d9/src/shader_translate.rs`](../../crates/aero-d3d9/src/shader_translate.rs) (`validate_sampler_texture_types`)
    - Tests: [`crates/aero-gpu/tests/aerogpu_d3d9_resource_validation.rs`](../../crates/aero-gpu/tests/aerogpu_d3d9_resource_validation.rs) (accepts `dcl_1d` + `dcl_volume`, rejects unknown)
  - Current runtime/protocol limitation: the AeroGPU command stream can only create guest-backed **2D** textures today
    (`CREATE_TEXTURE2D`; cube is represented as `array_layers=6`), so it cannot yet supply real guest-backed 1D/3D
    textures. For 1D/3D sampler declarations, the D3D9 executor binds a dummy 1D/3D texture view for that binding slot.
    - Tests: [`crates/aero-d3d9/tests/software_textures.rs`](../../crates/aero-d3d9/tests/software_textures.rs) (CPU/software 1D/3D sampling correctness)
- SM3 IR builder rejects some control-flow / addressing forms:
  - [`crates/aero-d3d9/src/sm3/ir_builder.rs`](../../crates/aero-d3d9/src/sm3/ir_builder.rs)

For Win7 D3D9Ex/DWM context:

- [`../specs/direct3d-9ex-and-dwm.md`](../specs/direct3d-9ex-and-dwm.md)
- [`../specs/windows7-d3d-umd-ddi.md`](../specs/windows7-d3d-umd-ddi.md)

---

### D3D10/11 stack (`crates/aero-d3d11`)

#### What exists today

`crates/aero-d3d11` contains:

1. DXBC SM4/SM5 decode + WGSL translation (VS/PS/CS today; plus GS/HS/DS `stage_ex` plumbing; a minimal SM4 GS DXBC→WGSL compute translator exists and is executed via the translated-GS prepass for a small set of IA input topologies (`PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj`); HS/DS translation/execution is not implemented).
2. A wgpu-backed executor for the AeroGPU command stream (`aerogpu_cmd.h`).

Code pointers:

- Translation:
  - [`crates/aero-d3d11/src/shader_translate.rs`](../../crates/aero-d3d11/src/shader_translate.rs)
  - [`crates/aero-d3d11/src/sm4/`](../../crates/aero-d3d11/src/sm4/)
- Command execution:
  - [`crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`](../../crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs)

Representative test pointers:

- [`crates/aero-d3d11/tests/aerogpu_cmd_smoke.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_smoke.rs)
- [`crates/aero-d3d11/tests/aerogpu_cmd_textured_triangle.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_textured_triangle.rs)
- Compute translation/execution: [`crates/aero-d3d11/tests/d3d11_runtime_compute_dispatch.rs`](../../crates/aero-d3d11/tests/d3d11_runtime_compute_dispatch.rs), [`crates/aero-d3d11/tests/shader_translate_compute.rs`](../../crates/aero-d3d11/tests/shader_translate_compute.rs)
- GS compute-prepass plumbing (synthetic expansion bring-up): [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_smoke.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_smoke.rs), [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_vertex_pulling.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_vertex_pulling.rs), [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_primitive_id.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_primitive_id.rs)
- GS translator unit tests: [`crates/aero-d3d11/tests/gs_translate.rs`](../../crates/aero-d3d11/tests/gs_translate.rs)
- GS prepass execution tests (translated SM4 subset):
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_point_to_triangle.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_point_to_triangle.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_instancing.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_instancing.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelist_emits_triangle.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelist_emits_triangle.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelistadj_emits_triangle.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelistadj_emits_triangle.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelist_instance_step_rate.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelist_instance_step_rate.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_restart_strip.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_restart_strip.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_pointlist_draw_indexed.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_pointlist_draw_indexed.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_vs_as_compute_feeds_gs_inputs.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_vs_as_compute_feeds_gs_inputs.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_output_topology_pointlist.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_output_topology_pointlist.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_trianglelist_emits_triangle.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_trianglelist_emits_triangle.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_trianglelistadj_emits_triangle.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_trianglelistadj_emits_triangle.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_translated_prepass_sv_primitive_id.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_translated_prepass_sv_primitive_id.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_cbuffer_b0_translated_prepass.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_cbuffer_b0_translated_prepass.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_line_strip_output.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_line_strip_output.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_gs_emulation_passthrough.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_gs_emulation_passthrough.rs)
  - [`crates/aero-d3d11/tests/aerogpu_cmd_gs_instance_count.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_gs_instance_count.rs)
- Guest-side Win7 tests live under [`drivers/aerogpu/tests/win7/`](../../drivers/aerogpu/tests/win7/) (see e.g. `d3d10_*`, `d3d11_*`)

Known gaps / limitations (enforced by code/tests):

- Geometry shaders require compute-based emulation on WebGPU (no GS stage):
  - The executor routes draws through a compute prepass (expanded buffers + indirect args) followed by a normal render pass:
    - Code: [`crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`](../../crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs) (`gs_hs_ds_emulation_required`, `exec_draw_with_compute_prepass`)
  - **Binding model:** `@group(3)` is reserved for extended D3D stages (GS/HS/DS) plus internal emulation helpers.
    - D3D register bindings within a stage group use the shared offsets from
      [`crates/aero-d3d11/src/binding_model.rs`](../../crates/aero-d3d11/src/binding_model.rs):
      - `b#` / `cb#` → `@binding(BINDING_BASE_CBUFFER + slot)` (`BINDING_BASE_CBUFFER = 0`)
      - `t#` → `@binding(BINDING_BASE_TEXTURE + slot)` (`BINDING_BASE_TEXTURE = 32`)
      - `s#` → `@binding(BINDING_BASE_SAMPLER + slot)` (`BINDING_BASE_SAMPLER = 160`)
      - `u#` → `@binding(BINDING_BASE_UAV + slot)` (`BINDING_BASE_UAV = 176`, `MAX_UAV_SLOTS = 8`)
    - Internal-only helpers use `@binding >= BINDING_BASE_INTERNAL` (`256`) to stay disjoint from D3D register
      spaces (e.g. IA vertex pulling, expanded-draw buffers).
  - The compute prepass includes a built-in WGSL path that emits deterministic synthetic triangle geometry for bring-up/fallback (see `GEOMETRY_PREPASS_CS_WGSL`).
  - For a small supported subset of geometry shaders with supported IA input topologies (`PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj`), the executor translates GS DXBC→WGSL compute at create time and can execute it as the prepass for eligible draws (`Draw` and `DrawIndexed`) (see `exec_geometry_shader_prepass_*` in `aerogpu_cmd_executor.rs`):
    - Translator: [`crates/aero-d3d11/src/runtime/gs_translate.rs`](../../crates/aero-d3d11/src/runtime/gs_translate.rs)
    - Translator tests: [`crates/aero-d3d11/tests/gs_translate.rs`](../../crates/aero-d3d11/tests/gs_translate.rs)
    - GS input feeding (current in-tree behavior):
      - Prepass paths populate GS `v#[]` from **VS outputs** via a minimal
        VS-as-compute path (vertex pulling + **`mov`/`add` only** today). If VS-as-compute translation fails,
        draws fail unless the VS is a strict passthrough (or `AERO_D3D11_ALLOW_INCORRECT_GS_INPUTS=1` is set
        to force IA-fill for debugging; may misrender because GS observes pre-VS IA values).
    - Supported resource ops in the translated GS subset (non-exhaustive):
      - Texture2D: `sample`/`sample_l`/`ld`/`resinfo`
      - SRV buffers: `ld_raw`/`ld_structured`/`bufinfo`
      - UAV buffers (SM5): `ld_uav_raw`/`ld_structured_uav`/`store_raw`/`store_structured`/`atomic_add`/`bufinfo`
  - Strip output expansion helpers for `CutVertex` / `RestartStrip` semantics:
    - Reference implementation: [`crates/aero-d3d11/src/runtime/strip_to_list.rs`](../../crates/aero-d3d11/src/runtime/strip_to_list.rs)
    - Unit tests: `crates/aero-d3d11/src/runtime/strip_to_list.rs` (module `tests`)
  - Known GS emulation gaps / next steps:
    - Broaden VS-as-compute feeding (opcode coverage + broader draw-instancing coverage).
    - Strip and adjacency-strip IA topologies (`LINESTRIP`, `TRIANGLESTRIP`, `LINESTRIP_ADJ`, `TRIANGLESTRIP_ADJ`) are not supported end-to-end yet (until Tasks 705/708/711 equivalents land).
    - Draw instancing (`instance_count > 1`) is now exercised for the translated-GS prepass (pointlist)
      by `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_instancing.rs`, but
      coverage is still incomplete (the `LINELIST` translated prepass path still fails-fast when
      `instance_count != 1`).
    - Some downlevel backends have very low per-stage storage-buffer limits (commonly `max_storage_buffers_per_shader_stage = 4`), which can block compute-prepass execution.
  - Owning doc: [`../specs/compute-expansion-emulation.md`](../specs/compute-expansion-emulation.md)
    - GS DXBC + ILAY fixtures: [`crates/aero-d3d11/tests/fixtures/README.md`](../decisions/README.md)
    - Example GS test runs:
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_point_to_triangle`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_compute_prepass_instancing`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelist_emits_triangle`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelistadj_emits_triangle`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelist_instance_step_rate`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_restart_strip`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_reads_srv_buffer_translated_prepass`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_texture_t0_translated_prepass`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelist_emits_triangle`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelistadj_emits_triangle`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_vs_as_compute_feeds_gs_inputs`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_samples_texture_translated_prepass`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_translated_primitive_id`
      - `cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_translated_prepass_sv_primitive_id`
- GS/HS/DS shader objects can be created/bound (the command stream binds these stages via
  `BIND_SHADERS`; newer streams may append `{gs,hs,ds}` handles after the stable 24-byte prefix—when
  present the appended handles are authoritative). HS/DS currently compile to minimal compute shaders
  for state tracking and are not executed. GS shaders attempt translation to a compute prepass at
  create time:
  - If translation succeeds, draws with supported IA input topologies (`PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj`) execute translated GS DXBC; other cases use synthetic expansion (guest GS DXBC does not execute).
  - If translation fails, draws with that GS bound currently return a clear “geometry shader not supported” error.
    - Code: [`crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`](../../crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs) (`exec_create_shader_dxbc`, `from_aerogpu_u32_with_stage_ex`)
    - Tests: [`crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_ignore.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_ignore.rs)
- Current GS translator limitations / initial target subset (non-exhaustive):
  - No multi-stream output (`emit_stream` / `cut_stream`); only stream 0 is supported
  - Output topology (GS→WGSL compute translator): `pointlist`, `linestrip`, `triangle_strip`
    - `linestrip` is expanded into an indexed **line list**
    - `triangle_strip` is expanded into an indexed **triangle list**
    - Note: executor wiring is still partial; the end-to-end translated-GS prepass path is only
      exercised for the supported IA input topology subset. For that path, the expanded draw topology
      is derived from the GS output topology kind (`PointList`/`LineList`/`TriangleList`, with strips
      expanded to lists).
  - GS instancing (`dcl_gsinstancecount` / `[instance(n)]`, `SV_GSInstanceID`) is supported:
    - Test: [`crates/aero-d3d11/tests/aerogpu_cmd_gs_instance_count.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_gs_instance_count.rs)
  - No stream-out (SO / transform feedback)
- Tessellation (Hull/Domain) emulation is bring-up only:
  - Patchlist topology routes draws through the compute-prepass expansion path
    (see `gs_hs_ds_emulation_required`, `exec_draw_with_compute_prepass`).
  - Patchlist topology **without GS/HS/DS bound** uses the synthetic expansion prepass (emits a
    deterministic triangle) so apps that select patchlists speculatively can still render *something*
    during bring-up.
  - Patchlist topology **with HS+DS bound** routes through an initial tessellation compute prepass
    pipeline (currently PatchList3 only): VS-as-compute vertex pulling stub → HS passthrough → layout
    pass (patch metadata + indirect args) → DS evaluation via `DomainEvalPipeline` (placeholder DS) →
    tri-domain integer index generation via `TriDomainIntegerIndexGen`. Guest HS/DS DXBC is not
    executed yet; the HS passthrough currently writes a fixed tess factor: `4.0`.
  - Tessellation building blocks live under `crates/aero-d3d11/src/runtime/tessellation/` (VS-as-compute
    stub, HS passthrough, layout pass, DS evaluation (`DomainEvalPipeline`), tri-domain integer index
    gen, sizing/guardrails).
  - Guest HS/DS handles/resources are tracked for state/binding, but not executed yet.
  - Design doc: [`../specs/compute-expansion-emulation.md`](../specs/compute-expansion-emulation.md)
  - Code: [`crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`](../../crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs) (`CmdPrimitiveTopology::PatchList`, `gs_hs_ds_emulation_required`, `exec_draw_with_compute_prepass`)
  - Tests:
    - [`crates/aero-d3d11/tests/aerogpu_cmd_tessellation_smoke.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_tessellation_smoke.rs)
    - [`crates/aero-d3d11/tests/aerogpu_cmd_tessellation_hs_ds_compute_prepass_error.rs`](../../crates/aero-d3d11/tests/aerogpu_cmd_tessellation_hs_ds_compute_prepass_error.rs)
- SM5 compute/UAV bring-up is partially supported, but still has important limitations:
  - `sync` barriers are translated for compute shaders.
    - Fence-only variants (no thread-group sync) do not have a perfect WGSL/WebGPU mapping; the current translation uses `storageBarrier()` as an approximation and therefore rejects fence-only `sync` in potentially divergent control flow (see `crates/aero-d3d11/src/shader_translate.rs`).
    - `*WithGroupSync` barriers are translated, but are rejected when they appear after potentially conditional returns (to avoid deadlocks when not all invocations reach the barrier).
  - Typed UAV stores and UAV buffer atomics are supported for a small subset of formats/operations, but broader `RWTexture*` and `Interlocked*` coverage is still missing.

Roadmap/plan docs:

- [`../specs/direct3d-10-11-translation.md`](../specs/direct3d-10-11-translation.md)
- [`../specs/windows7-d3d-umd-ddi.md`](../specs/windows7-d3d-umd-ddi.md)

---

### Web presenters/backends (`apps/web/src/gpu/*`)

The browser “present” layer takes RGBA8 frames and draws them to an `OffscreenCanvas`.

Status checklist:

- [x] WebGPU presenter (native WebGPU API)
- [x] WebGL2 fallback presenter (raw WebGL2)
- [x] WebGL2 presenter via `wgpu` (WASM, forcing the wgpu GL backend)

Code pointers:

- API surface: [`apps/web/src/gpu/presenter.ts`](../../apps/web/src/gpu/presenter.ts)
- WebGPU presenter: [`apps/web/src/gpu/webgpu-presenter.ts`](../../apps/web/src/gpu/webgpu-presenter.ts)
- Raw WebGL2 presenter: [`apps/web/src/gpu/raw-webgl2-presenter.ts`](../../apps/web/src/gpu/raw-webgl2-presenter.ts)
- wgpu-over-WebGL2 presenter: [`apps/web/src/gpu/wgpu-webgl2-presenter.ts`](../../apps/web/src/gpu/wgpu-webgl2-presenter.ts)

Test pointers:

- [`apps/web/src/gpu/webgpu-presenter-backend.test.ts`](../../apps/web/src/gpu/webgpu-presenter-backend.test.ts)
- [`apps/web/src/gpu/frame_pacing.test.ts`](../../apps/web/src/gpu/frame_pacing.test.ts)

---

### Current critical path integration gaps (factual)

This section lists integration blockers that prevent a full “Win7 WDDM + accelerated rendering” experience on the canonical machine today.

#### AeroGPU command execution: external executor exists; Win7 validation is still pending

What exists today:

- `aero-machine` implements **BAR0/BAR1 transport** (ring + fences + vblank/scanout/cursor regs) and decodes submissions.
  It can route `AEROGPU_CMD` payloads via:
  - the **submission bridge** (external executor), or
  - an optional **in-process backend** (immediate/null, plus feature-gated native wgpu backend).
- Default bring-up behavior (no backend, submission bridge disabled) completes fences without executing commands (to avoid wedging early guests).
- The canonical browser runtime uses the **submission bridge** and executes command streams in the GPU worker
  (`apps/web/src/workers/gpu-worker.ts`), completing fences back into the device model.

Evidence/pointers:

- Device-side capture + bridge/backends: [`crates/aero-machine/src/aerogpu.rs`](../../crates/aero-machine/src/aerogpu.rs)
  (`enable_submission_bridge`, `drain_pending_submissions`, `complete_fence_from_backend`, `set_backend`)
- Browser control-plane routing: [`apps/web/src/runtime/coordinator.ts`](../../apps/web/src/runtime/coordinator.ts)
  (`forwardAerogpuSubmit`, `forwardAerogpuFenceComplete`)
- GPU worker executor: [`apps/web/src/workers/gpu-worker.ts`](../../apps/web/src/workers/gpu-worker.ts) (`handleSubmitAerogpu`)

Fast regression commands:

```bash
bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_submission_bridge --locked
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-machine --locked --features aerogpu-wgpu-backend --test aerogpu_wgpu_backend_smoke
AERO_TIMEOUT=600 AERO_MEM_LIMIT=32G bash ./scripts/safe-run.sh pnpm -C apps/web run test:unit -- src/runtime/coordinator.test.ts
bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu.spec.ts
bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu_vsync_completion.spec.ts
```

What is still missing (P0):

- **End-to-end Win7 bring-up + accelerated rendering validation** on the canonical browser machine:
  driver install → ring submissions → ACMD execution → scanout present → DWM/Aero stability.
  (See: [`../specs/windows7-aerogpu-validation.md`](../specs/windows7-aerogpu-validation.md))
- Validate vblank + vsynced present behavior against the documented contract (DWM stability):
  [`../specs/windows7-aerogpu-wddm-driver.md`](../specs/windows7-aerogpu-wddm-driver.md).

#### WDDM scanout publication into `ScanoutState` exists (MVP) but needs end-to-end validation

- `aero-machine` publishes **legacy** scanout transitions (text ↔ VBE LFB) to `ScanoutState`, and can also publish WDDM scanout state from BAR0 scanout0 registers when `Machine::process_aerogpu()` runs (atomic builds).
  - Code: [`crates/aero-machine/src/lib.rs`](../../crates/aero-machine/src/lib.rs) (`process_aerogpu`, INT 10h scanout publishing)
  - Code: [`crates/aero-machine/src/aerogpu.rs`](../../crates/aero-machine/src/aerogpu.rs) (`take_scanout0_state_update`)
  - Tests: [`crates/aero-machine/tests/aerogpu_wddm_scanout_state_format_mapping.rs`](../../crates/aero-machine/tests/aerogpu_wddm_scanout_state_format_mapping.rs)
  - Disable semantics: after WDDM scanout is claimed, clearing `SCANOUT0_ENABLE=0` publishes a **disabled WDDM** descriptor
    (source=WDDM, base/width/height/pitch=0) so legacy scanout cannot steal ownership back.
    - Test: [`crates/aero-machine/tests/aerogpu_scanout_disable_publishes_wddm_disabled.rs`](../../crates/aero-machine/tests/aerogpu_scanout_disable_publishes_wddm_disabled.rs)
- The GPU worker can present WDDM scanout from either guest RAM **or** the shared VRAM aperture (BAR1 backing) when `ScanoutState` is published with `source=WDDM` and a non-zero `base_paddr`:
  - Code: [`apps/web/src/workers/gpu-worker.ts`](../../apps/web/src/workers/gpu-worker.ts) (`tryReadScanoutFrame` / `tryReadScanoutRgba8`)
  - E2E test (guest RAM base_paddr): [`tests/e2e/wddm_scanout_smoke.spec.ts`](../../tests/e2e/wddm_scanout_smoke.spec.ts) (harness: [`apps/web/wddm-scanout-smoke.ts`](../../apps/web/wddm-scanout-smoke.ts))
  - E2E test (VRAM aperture base_paddr): [`tests/e2e/wddm_scanout_vram_smoke.spec.ts`](../../tests/e2e/wddm_scanout_vram_smoke.spec.ts) (harness: [`apps/web/wddm-scanout-vram-smoke.ts`](../../apps/web/wddm-scanout-vram-smoke.ts))
  - VRAM/base-paddr contract notes: [`graphics.md`](graphics.md#vram-bar1-backing-as-a-sharedarraybuffer)
- Manual harness: [`apps/web/wddm-scanout-debug.html`](../../apps/web/wddm-scanout-debug.html) (interactive toggles for scanoutState source/base_paddr/pitch and BGRX X-byte alpha forcing)
- Current limitation: scanout presentation is currently limited to a small set of packed formats:
  - 32bpp layouts (`B8G8R8X8` / `B8G8R8A8` / `R8G8B8X8` / `R8G8B8A8` + sRGB variants; X8 treated as fully opaque)
  - 16bpp layouts (`B5G6R5` (opaque) / `B5G5R5A1` (1-bit alpha))
  Unsupported formats publish a deterministic disabled descriptor.

Repro commands:

```bash
# Rust: scanout handoff + disable semantics
bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_scanout_handoff --locked
bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_scanout_disable_publishes_wddm_disabled --locked

# Browser e2e: scanout presentation from guest RAM / VRAM aperture
bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/wddm_scanout_smoke.spec.ts
bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/wddm_scanout_vram_smoke.spec.ts
```

Impact:

- End-to-end validation is still required that the Win7 driver + browser runtime converge on supported scanout formats + update cadence (see docs below).

Owning docs:

- [`../specs/windows7-aerogpu-wddm-driver.md`](../specs/windows7-aerogpu-wddm-driver.md)
- [`../specs/windows7-aerogpu-wddm-driver.md`](../specs/windows7-aerogpu-wddm-driver.md)

#### Canonical machine vs sandbox: duplicate device models

- The AeroGPU device-side library is `crates/aero-devices-gpu` (portable PCI wrapper, ring executor, and an optional native wgpu backend).
  The canonical in-browser machine (`crates/aero-machine` + `crates/aero-wasm` + web workers) currently has its own BAR0/BAR1 integration layer behind the same PCI identity (`A3A0:0001`) to satisfy the Windows 7 driver binding/boot-display contract, and relies on the submission bridge (browser) or optional in-process backends (native) for command execution.
  Consolidating these integration surfaces onto a single device model remains outstanding.
  - Shared device-side library: [`crates/aero-devices-gpu/src/pci.rs`](../../crates/aero-devices-gpu/src/pci.rs), [`crates/aero-devices-gpu/src/executor.rs`](../../crates/aero-devices-gpu/src/executor.rs)
  - CPU rasterizer: [`crates/aero-gpu-software/src/lib.rs`](../../crates/aero-gpu-software/src/lib.rs)
  - Canonical machine integration: [`crates/aero-machine/src/aerogpu.rs`](../../crates/aero-machine/src/aerogpu.rs) + WASM bridge in [`crates/aero-wasm/src/lib.rs`](../../crates/aero-wasm/src/lib.rs) + web runtime wiring in [`apps/web/src/workers/machine_cpu.worker.ts`](../../apps/web/src/workers/machine_cpu.worker.ts) / [`apps/web/src/workers/gpu-worker.ts`](../../apps/web/src/workers/gpu-worker.ts)

#### End-to-end Win7 graphics validation: needs verification

The repo contains extensive unit/integration tests for ABI correctness and host-side execution, but new contributors should treat these items as **unknown until verified end-to-end in the browser runtime**:

- Win7 install boots to desktop under `aero-wasm` + web runtime.
- Win7 AeroGPU driver can be installed and submit work end-to-end (including scanout handoff and vblank waits).

Where to start verifying:

- [`tests/windows7_boot.rs`](../../tests/windows7_boot.rs) (baseline Win7 boot)
- [`../specs/windows7-aerogpu-validation.md`](../specs/windows7-aerogpu-validation.md) (driver + validation checklist)

---

### Appendix: Known duplicates / tech debt (pointers)

 - VGA device model wiring has two *integration* surfaces, but one shared implementation:
   - canonical VGA/VBE device model: [`crates/aero-gpu-vga/`](../../crates/aero-gpu-vga/)
- Multiple AeroGPU device models exist for the canonical versioned ABI (`A3A0:0001`):
  - canonical machine MVP: [`crates/aero-machine/src/aerogpu.rs`](../../crates/aero-machine/src/aerogpu.rs) + display/VRAM glue in [`crates/aero-machine/src/lib.rs`](../../crates/aero-machine/src/lib.rs)
  - shared device-side library: [`crates/aero-devices-gpu/src/pci.rs`](../../crates/aero-devices-gpu/src/pci.rs)
  - legacy bring-up ABI (`1AED:0001`): retired along with the device model that implemented it
  - contract doc: [`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md)
- Command execution in the web runtime happens in one place: the TypeScript
  executor at [`apps/web/src/workers/aerogpu-acmd-executor.ts`](../../apps/web/src/workers/aerogpu-acmd-executor.ts).
- Shared-surface bookkeeping is centralized in [`crates/aero-gpu/src/shared_surface.rs`](../../crates/aero-gpu/src/shared_surface.rs) (`SharedSurfaceTable`) and used by both the D3D9 and D3D11 executors.
  - Lightweight mirrors (for protocol tests + tooling): [`apps/web/src/workers/aerogpu-acmd-executor.ts`](../../apps/web/src/workers/aerogpu-acmd-executor.ts), [`apps/web/tools/gpu_trace_replay.ts`](../../apps/web/tools/gpu_trace_replay.ts)
  - Unit tests: `crates/aero-gpu/src/tests/shared_surface.rs` and `crates/aero-gpu/src/shared_surface.rs` (module `tests`)
  - Executor-level tests: `crates/aero-gpu/tests/*shared_surface*` and `crates/aero-d3d11/tests/aerogpu_cmd_shared_surface.rs`

---

### Appendix: “Known good” local test commands

These are the fast, repeatable commands used to validate the current graphics stack.

```bash
# Boot display (VGA/VBE) + machine wiring
bash ./scripts/safe-run.sh cargo test -p aero-gpu-vga --locked
bash ./scripts/safe-run.sh cargo test -p aero-machine --locked

# wasm32 guardrail (compile-only; does not require a JS runtime)
# (Increase timeout if needed; first-time wasm32 builds can be slow without a warm Cargo cache.)
AERO_TIMEOUT=1800 bash ./scripts/safe-run.sh cargo xtask wasm-check

# AeroGPU bridge/backends (canonical in-browser integration boundary)
bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_submission_bridge --locked
bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_immediate_backend_completes_fence --locked

# AeroGPU protocol + host-side command processing
bash ./scripts/safe-run.sh cargo test -p aero-protocol --locked
bash ./scripts/safe-run.sh pnpm run test:protocol
bash ./scripts/safe-run.sh cargo test -p aero-gpu --locked

# D3D translation layers
bash ./scripts/safe-run.sh cargo test -p aero-dxbc --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --locked

# Legacy/sandbox emulator path (device model + e2e tests)
bash ./scripts/safe-run.sh cargo test -p aero-devices-gpu --locked
bash ./scripts/safe-run.sh cargo test -p emulator --locked

# Browser e2e smoke tests
bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/wddm_scanout_smoke.spec.ts
bash ./scripts/safe-run.sh pnpm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu.spec.ts
```

## WebGL2 context-loss recovery (2026-07-30)

`wddm_scanout_recovery_smoke` loses the WebGL2 context on purpose and expects the
presenter to come back and re-present from guest RAM. It failed at the first step
for three separate reasons, each hidden behind the one before it.

**`restoreContext()` could never be reached.** `debugRestoreContext` fetched
`WEBGL_lose_context` with `gl.getExtension(...)` at the moment it wanted to
restore — but `getExtension` returns null once a context is lost, so a handle
fetched on demand is only ever good for *losing* one. The extension object is now
captured during `init()`, while the context is alive, and both helpers use it.

**Recovery raced the backend's rebuild.** Both the GPU worker and the raw WebGL2
backend listen for `webglcontextrestored` on the same canvas, with no ordering
between them. When the worker won, it drove recovery through textures and
programs belonging to the dead context. `PresenterInitOptions` now carries an
`onContextRestored` callback — previously only failures were reported, so a
successful restore was silent — and the backend fires it after rebuilding.

**A stale GL error aborted the recovery.** With the first two fixed, recovery
died on `texImage2D: WebGL error 0x502` for a plain 64x64 RGBA8 upload, with the
context alive and the right texture bound. `getError()` returns the oldest error
recorded since it was last called, so an unchecked failure anywhere upstream is
reported against whichever operation asserts next. `clearWebGlErrors` now scopes
the check: drain first, then assert, so the label names the call that actually
failed.

### What is still wrong

Recovery completes — `recoveries_succeeded_wddm` reaches 1 and eleven presents
land — and the picture comes back. The four corner samples are exactly the
expected red, green, blue and white, and the readback is 64x64, matching the
pre-loss present. But the frame no longer hashes equal to the source, which is
what the test asserts.

So the difference is somewhere in the interior, at the right size, with the right
corners. Two things worth ruling out early were checked and are not it: the
output dimensions (identical) and the `u_force_opaque_alpha` uniform (recomputed
from the same options on re-init).

Whoever picks this up will want a pixel diff rather than another hash — the
harness currently reports four corner samples and a hash, which is exactly enough
information to know something is wrong and not enough to say what.
