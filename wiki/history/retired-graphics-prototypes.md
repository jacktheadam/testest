# Retired graphics prototypes

> **Historical.** The graphics paths Aero tried and abandoned: the guest GPU
> driver strategy debate that chose a custom WDDM driver over reusing
> virtio-gpu, the virtio-gpu prototype notes, an early command protocol, an
> experimental command and completion ring, and a shader/shared-surface audit
> from the sprint era.
>
> None of these are ABIs to target. The live contract is
> [../specs/aerogpu-device-abi.md](../specs/aerogpu-device-abi.md).

## Status (current canonical direction)

This repository’s **canonical** Windows 7 graphics stack is the custom **AeroGPU** WDDM device +
driver package:

- PCI device/ABI contract: `PCI\\VEN_A3A0&DEV_0001` (see `../specs/aerogpu-device-abi.md` and
  `drivers/aerogpu/protocol/*`)
- Canonical machine wiring: `crates/aero-machine` (`MachineConfig::enable_aerogpu=true` at `00:07.0`)
- In-tree Win7 driver package: `drivers/aerogpu/packaging/win7/`

The virtio-gpu reuse path described below is retained as **historical context / optional prototype**
work (for example, `crates/virtio-gpu-proto`). It is **not** the Windows driver binding contract
used by the canonical machine or Guest Tools.

## Goal

Choose the guest GPU driver path that gets to a **working Windows 7 desktop (and eventually Aero)** with the best leverage:

1. **Reuse an existing Windows virtio-gpu WDDM driver** (ideal if it exists, is signed, and is usable on Win7), or
2. **Build a custom “AeroGPU” WDDM driver stack** (highest control, highest cost).

This document focuses on what matters for Aero:

- Windows 7’s Desktop Window Manager (DWM) is built on **Direct3D 9Ex**.
- To get the “Aero Glass” experience, the guest must have a **WDDM driver path** that allows DWM’s D3D usage to function acceptably.
- In the browser, the host-side accelerated API is **WebGPU**, so the long-term plan is effectively “some guest GPU API → WebGPU”.

## Constraints / assumptions

- No proprietary Microsoft code can be copied.
- Shipping drivers is preferable only if licensing is permissive (MIT/Apache/BSD). If not permissive, the project should at minimum avoid embedding code and instead document how users obtain it themselves.
- Driver signing is a practical constraint:
  - **Using an already-signed third-party driver is a large time saver**.
  - A custom kernel-mode WDDM driver will require a signing strategy (test signing for dev; WHQL/attestation for distribution).

## Candidate Windows guest display driver options (survey)

This section is intentionally pragmatic: which drivers are “real” and installable on Windows 7 without heroic effort.

### WDDM version note (Windows 7)

- Windows 7 is fundamentally a **WDDM 1.1** operating system.
- Some modern “display-only driver” models (often abbreviated **DOD**) target newer WDDM versions and may not work on Win7.
- For Aero specifically, Win7 historically expects a functional WDDM driver path that allows DWM’s D3D9Ex usage to operate.

### virtio-win “viogpu/viogpudo” (virtio-gpu)

**What it is:** A Windows display driver intended for the `virtio-gpu` / `virtio-vga` device family (PCI vendor `0x1af4`, device `0x1050`).

**Reality check:** The virtio-win ecosystem definitely ships signed storage/net/balloon drivers. GPU driver support exists in some virtio-win distributions, but **Windows 7 support and feature level are the key unknowns**:

- Some virtio-win GPU drivers are **display-only** (“DOD”) designs intended for newer Windows versions; Windows 7’s WDDM model is older and may not be compatible with a DOD-style driver.
- If the virtio-win GPU driver is not WDDM 1.0/1.1 compatible, it won’t enable Win7 Aero.

**Licensing:** virtio-win sources are open, but licenses vary by component. Before committing to shipping any binaries, verify the exact license for the GPU driver component (and whether binaries are redistributable).

**Device model requirements (2D scanout path):**

- virtio-pci transport (modern or transitional)
- `controlq` virtqueue for:
  - `GET_DISPLAY_INFO`
  - `RESOURCE_CREATE_2D`
  - `RESOURCE_ATTACH_BACKING`
  - `TRANSFER_TO_HOST_2D`
  - `SET_SCANOUT`
  - `RESOURCE_FLUSH`
- optional `cursorq` virtqueue for cursor plane (can be stubbed initially)

**How it maps to D3D9 → WebGPU:**

- If the driver is display-only, it likely does **not** provide a D3D9 acceleration path; it just presents a framebuffer.
- If it supports 3D, it will likely do so via an existing virtio-gpu 3D protocol (e.g. “virgl”-style). That does **not** naturally map to “D3D9 command stream → WebGPU”; it maps to a *different* 3D API surface that would also need translation.

### SPICE/QXL WDDM driver (alternative reference point, not virtio-gpu)

**What it is:** The SPICE ecosystem historically shipped Windows QXL drivers, including WDDM variants that support Windows 7 in QEMU.

**Why it matters here:** Even if we choose virtio-gpu long-term, QXL is a useful comparator because it answers: “can an open driver get us to a Windows 7 desktop quickly?”

**Licensing:** open source, but often copyleft (verify exact terms). This may be acceptable as an optional user-provided driver, but is usually not ideal for direct inclusion in a permissively-licensed project.

**Device model requirements (high level):**

- QXL is *not* virtio; it has its own PCI device model and command/VRAM interfaces.
- Common expectations include:
  - a VRAM-like region for surfaces
  - a command ring / command queue with interrupts (“kick”/doorbell)
  - surface creation/destruction, blits, and cursor updates

**How it maps to D3D9 → WebGPU:** Similar to display-only: it’s fundamentally a 2D scanout/command approach, not a clean D3D9 command capture path.

### VMware/VirtualBox WDDM drivers (not viable for Aero)

- VMware SVGA and VirtualBox guest additions provide working Aero in many VMs, but:
  - binaries are typically proprietary or under licenses not compatible with this project’s distribution goals.
  - the device models are complex and not tailored to “browser-hosted WebGPU”.

### “Standard VGA/VBE”

- Windows built-in VGA/VBE paths are required for boot and early UI.
- They do **not** provide a WDDM path suitable for Aero.

## Option A: reuse virtio-gpu + existing Windows driver (recommended first leverage if Win7-compatible driver exists)

### Why it’s attractive

- **Time-to-first-desktop** is dominated by driver complexity, not device-model complexity.
- virtio-gpu 2D scanout is comparatively small and well-specified.
- If the Windows driver is **already signed**, it eliminates the single biggest real-world blocker for distributing a usable Win7 image.

### What we must build (host/device side)

At minimum for a basic desktop scanout:

1. virtio-pci plumbing (PCI config space, BARs/capabilities, MSI/MSI-X or INTx)
2. virtqueue implementation (descriptor walking, avail/used rings)
3. virtio-gpu controlq command processing:
   - `GET_DISPLAY_INFO` (reports one enabled scanout mode)
   - `RESOURCE_CREATE_2D` (BGRA8888)
   - `RESOURCE_ATTACH_BACKING` (guest memory backing)
   - `TRANSFER_TO_HOST_2D` (copy guest → host resource)
   - `SET_SCANOUT` (bind resource to scanout)
   - `RESOURCE_FLUSH` (present)
4. scanout to browser surface (Canvas/WebGPU texture upload)

### Fit with D3D9 → WebGPU translator

This path is excellent for “pixels on the screen”, but ambiguous for Aero acceleration:

- If the guest driver provides only a framebuffer, DWM composition may still fall back to software or a non-Aero theme.
- If the guest driver provides 3D, it likely does so through a protocol that is *not* “D3D9 command stream”.

**Net:** reuse virtio-gpu is the fastest route to a *working display*, but it does not guarantee the intended **D3D9→WebGPU** translation architecture will be usable without additional work.

## Option B: custom “AeroGPU” WDDM driver stack (highest control, highest risk)

### Why it’s attractive

- The **cleanest conceptual mapping** to the project’s end state:
  - D3D9/D3D9Ex work submitted by Windows → captured in a known command stream
  - Host translates those commands to WebGPU
- Avoids needing to implement/translate an intermediate 3D protocol (e.g. OpenGL/virgl).

### Why it’s risky / slow

- WDDM (Win7-era) requires a **kernel-mode miniport** + **user-mode display driver**, and for acceleration, the relevant 3D user-mode driver interfaces.
- Requires a **driver signing plan** (test signing for development; production signing to ship anything usable).
- Debugging kernel drivers inside an emulator in a browser is an extreme integration challenge.

### Device model requirements (high level)

A custom stack gets to define its own “hardware”, but the device model must still be implementable efficiently in the browser/WASM runtime:

- PCI display controller (or similar) for enumeration
- one or more BARs for:
  - control registers / doorbells
  - shared-memory command ring (or use virtio transport instead)
  - shared “VRAM” aperture or explicit resource upload/download commands
- interrupts (MSI/MSI-X preferred) for completion notification
- a well-specified, versioned command protocol (so the host can translate to WebGPU deterministically)

### Fit with D3D9 → WebGPU translator

Best possible fit, but only once the driver stack exists.

## Decision / recommended path

### At-a-glance comparison

| Path | Time to “desktop pixels” | Time to “Aero” | Biggest risk | Best leverage |
|------|--------------------------|----------------|--------------|---------------|
| Reuse virtio-gpu Windows driver | Low (if Win7-compatible driver exists) | Unclear | Win7/WDDM compatibility + limited 3D | Avoid writing WDDM KMD/UMD early; likely signed driver |
| Custom AeroGPU WDDM | Very high | Very high | WDDM complexity + signing | Perfect D3D9→WebGPU mapping if completed |

### Recommendation (current repo): custom AeroGPU WDDM is the canonical path

The repo now has an explicit, versioned **AeroGPU** ABI and an in-tree Win7 WDDM driver stack.
New work that targets the Windows 7 graphics path should generally build on that canonical contract.

The virtio-gpu reuse path can still be useful as a *separate* “pixels on screen” exploration, but it
should not be treated as the project’s Windows 7 acceleration plan or binding contract.

If you are working on virtio-gpu experiments, keep them clearly labeled as prototypes and avoid
mixing their IDs/contracts with the canonical AeroGPU device (`A3A0:0001`).

Keeping these efforts separate reduces risk:
- We can still get early “pixels on screen” feedback from a framebuffer-style virtio-gpu prototype
  without changing the Windows driver contract.
- We do not prematurely lock the acceleration architecture to virgl/OpenGL-like semantics if the
  long-term goal is D3D9→WebGPU via AeroGPU.

## Prototype (in this repo)

This repository includes a narrow virtio-gpu 2D command-processing prototype:

- `crates/virtio-gpu-proto`
  - Implements the control-queue subset needed for basic scanout:
    - `GET_DISPLAY_INFO`
    - `GET_EDID` (returns a minimal EDID blob)
    - `RESOURCE_CREATE_2D`
    - `RESOURCE_ATTACH_BACKING`
    - `TRANSFER_TO_HOST_2D`
    - `SET_SCANOUT`
    - `RESOURCE_FLUSH`
  - Supports multi-entry (scatter/gather) backing and a small set of 32-bit formats (BGRA/BGRX variants).
  - Validation test:
  - `cargo test --locked -p virtio-gpu-proto`
  - `basic_2d_scanout_roundtrip` simulates a guest writing a BGRA framebuffer in “guest memory”, transferring it to the device, and flushing to scanout; the resulting scanout buffer is byte-for-byte verified.
  - Proof snippet: [virtio-gpu-proto-proof.md](retirements.md)

In addition, there is an end-to-end virtqueue/virtio-pci integration test:

- `crates/aero-virtio/src/devices/gpu.rs`
  - Wraps `virtio-gpu-proto` in the project’s virtio-pci + split-virtqueue transport (`aero-virtio`).
  - Intended as the “device model hooks” starting point for wiring into the emulator.
- `crates/aero-virtio/tests/virtio_gpu.rs`
  - `cargo test --locked -p aero-virtio virtio_gpu_2d_scanout_via_virtqueue`
  - Exercises the full controlq sequence through virtqueues and verifies scanout bytes after flush.

This prototype is not a Windows driver and does not claim Win7 driver compatibility; it is a **device-model foundation** that can be wired into the emulator once PCI/virtqueue infrastructure exists.

## Win7 validation checklist (manual, once integrated)

When the emulator has PCI + virtio-pci + scanout wiring, validate the virtio-gpu reuse path in a real Windows 7 guest:

1. Boot Windows 7 with VGA/VBE first (baseline display must work).
2. Expose a `virtio-gpu` PCI function (vendor `0x1af4`, device `0x1050`) and confirm it enumerates in Device Manager.
3. Install the candidate virtio-gpu Windows driver (virtio-win package) and confirm:
   - the driver binds to the device (no Code 12/Code 28/Code 43)
   - the display switches to the virtio-gpu adapter
   - resolution changes work (at least a fixed 1024×768 mode)
4. Confirm DWM/Aero behavior:
   - whether Aero can be enabled
   - whether D3D9Ex applications create devices successfully

If step (3) fails due to driver support gaps (or step (4) fails due to lack of acceleration), treat virtio-gpu as a “bring-up display” only and continue with the acceleration-specific plan (custom command stream / custom WDDM).

## Risks / unknowns to resolve next

1. **Does a signed virtio-gpu WDDM driver actually support Windows 7 Aero?**
   - If not, virtio-gpu still helps for early pixels, but a separate plan is required for Aero.
2. **Driver licensing / redistribution**
   - Confirm the license for any virtio-gpu Windows driver we plan to ship or bundle.
3. **3D protocol choice**
   - If using virtio-gpu 3D, decide whether to translate virgl/gfxstream → WebGPU (large) vs building a custom command stream (also large).
4. **Driver signing strategy for any custom WDDM work**
   - Without a credible signing plan, “custom WDDM” remains a research topic rather than a shipping path.

## virtio-gpu-proto proof

This repo includes a narrow virtio-gpu 2D scanout prototype in `crates/virtio-gpu-proto`.

### Minimal automated validation

Command:

```bash
cargo test --locked -p virtio-gpu-proto
```

Expected output (trimmed):

```text
running 1 test
test basic_2d_scanout_roundtrip ... ok

test result: ok. 1 passed; 0 failed
```

What the test covers:

- a simulated “guest memory” buffer contains BGRA pixels
- virtio-gpu commands create a 2D resource, attach the backing, transfer pixels, bind scanout, and flush
- the device’s scanout buffer is byte-for-byte equal to the original guest pixels
- exercises multi-entry (scatter/gather) backing, partial rect updates, and a simple mode switch via `SET_SCANOUT`

Additional covered behaviors (unit-tested indirectly via the command flow):

- `GET_EDID` returns a minimal EDID blob (transport test)
- BGRX formats are accepted and coerced to opaque alpha on upload

### Virtio transport integration proof

The same scanout sequence is also tested *through* a real virtio-pci + split-virtqueue transport:

```bash
cargo test --locked -p aero-virtio virtio_gpu_2d_scanout_via_virtqueue
```

## Task 489 audit: SM3 / DXBC / shared-surface tracking cleanup

This document maps legacy **scratchpad task IDs** (the ones referenced by Agent-3 planning) to the
current **in-tree implementations and tests**, to reduce duplicate work.

All file paths are repository-relative.

**Test-running notes (agent environments):**
- If you see Cargo stuck at `Blocking waiting for file lock on package cache`, retry with an isolated
  Cargo home to avoid shared registry lock contention:
  `AERO_ISOLATE_CARGO_HOME=1 bash ./scripts/safe-run.sh ...`
- On cold builds, some targets may need a longer timeout (e.g. `AERO_TIMEOUT=1200`).
- Some integration tests depend on `wgpu`/WebGPU availability and may auto-skip on headless systems.
  Set `AERO_REQUIRE_WEBGPU=1` to force these tests to fail instead of skipping.

---

### Task 40 — SM3 IR → WGSL generator + naga tests

**Status:** ✅ Done

**Implementation (key files):**
- `crates/aero-d3d9/src/sm3/{decode.rs,ir.rs,ir_builder.rs,wgsl.rs,verify.rs}`

**Implementing commits (high-signal):**
- `81181fad` — `feat(aero-d3d9): add sm3 WGSL backend with stable varying locations`
- `62b870b1` — `feat(sm3): derive WGSL declarations from register usage`

**Tests (WGSL + naga validation):**
- `crates/aero-d3d9/tests/sm3_wgsl.rs` (many `naga::front::wgsl::parse_str` + validator assertions)
- `crates/aero-d3d9/tests/sm3_wgsl_math.rs`
- `crates/aero-d3d9/tests/sm3_loop_wgsl.rs`
- `crates/aero-d3d9/tests/sm3_fixtures_wgsl.rs` (naga validation for real `fxc`-produced DXBC fixtures)

**How to run:**
```bash
# CPU-only WGSL + naga validation coverage:
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl_math --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_loop_wgsl --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_fixtures_wgsl --locked
```

---

### Task 49 — semantic-based VS input remap via `StandardLocationMap`

**Status:** ✅ Done

**Implementation (key files):**
- `crates/aero-d3d9/src/vertex/location_map.rs` (`StandardLocationMap` + `AdaptiveLocationMap`)
- `crates/aero-d3d9/src/sm3/ir_builder.rs` (semantic-driven input remap / duplicate detection)
- `crates/aero-d3d9/src/sm3/wgsl.rs` (emits the remapped `@location(n)` interface)

**Notes:**
- The remap is implemented via `AdaptiveLocationMap`, which:
  - reserves the fixed legacy assignments from `StandardLocationMap` for common semantics, and
  - deterministically allocates any additional declared semantics (e.g. `TEXCOORD8`) to the lowest
    remaining free locations.

**Implementing commits (high-signal):**
- `be5d5b05` — `feat(aero-d3d9/sm3): remap vertex inputs to canonical WGSL locations`

**Tests:**
- `crates/aero-d3d9/tests/sm3_semantic_locations.rs`
- `crates/aero-d3d9/tests/sm3_wgsl_semantic_locations.rs`
- `crates/aero-d3d9/src/vertex/location_map.rs` (unit tests for `AdaptiveLocationMap` determinism + edge cases)
- `crates/aero-gpu/tests/aerogpu_d3d9_semantic_locations.rs`
- `tests/d3d9_vertex_input.rs` (integration coverage via `aero-d3d9` test harness wiring)

**How to run (focused):**
```bash
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_semantic_locations --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl_semantic_locations --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --lib vertex::location_map --locked
bash ./scripts/safe-run.sh cargo test -p aero --test d3d9_vertex_input --locked
bash ./scripts/safe-run.sh cargo test -p aero-gpu --test aerogpu_d3d9_semantic_locations --locked
```

---

### Task 51 / 55 / 58 — DXBC parsing consolidation + `build_container` + RDEF/CTAB moved to `aero-dxbc`

**Status:** ✅ Done

**Implementation (key files):**
- `crates/aero-dxbc/src/{dxbc.rs,lib.rs,rdef.rs,ctab.rs,signature.rs,test_utils.rs}`
- `crates/aero-d3d9/src/dxbc.rs` (uses `aero_dxbc::DxbcFile` for container parsing)
- (No separate wrapper crate) other crates should depend on `crates/aero-dxbc/` directly for DXBC parsing.

**Implementing commits (high-signal):**
- `2bb1bbca` — `refactor(d3d9): reuse aero-dxbc for shader bytecode extraction`
- `4447967e` — `feat(dxbc): add test utils container builder`
- `85f15d9f` — `feat(dxbc): unify RDEF/CTAB parsing in aero-dxbc`

**Tests:**
- `crates/aero-dxbc/src/tests.rs` (unit tests; includes `tests_{parse,rdef,rdef_ctab,signature,sm4}.rs`)
- `crates/aero-d3d9/tests/sm3_ir.rs` (uses `aero_dxbc::test_utils::build_container`)

**How to run:**
```bash
bash ./scripts/safe-run.sh cargo test -p aero-dxbc --locked
# `sm3_ir` uses `aero_dxbc::test_utils::build_container`.
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_ir --locked
```

---

### Task 60 / 102 — DXBC robust feature gating + robust parser moved to `aero-dxbc`

**Status:** ✅ Done

**Implementation (key files):**
- `crates/aero-dxbc/src/lib.rs` (`#[cfg(feature = "robust")] pub mod robust;`)
- `crates/aero-dxbc/src/robust/*` (robust container parsing + reflection/disasm helpers)
- `crates/aero-d3d9/src/dxbc/robust.rs` (re-export shim)

**Implementing commits (high-signal):**
- `96c10295` — `refactor(dxbc): move robust DXBC parsing into aero-dxbc`

**Tests:**
- `crates/aero-dxbc/src/robust/container.rs` (robust parser unit tests; run with `--features robust`)
- `crates/aero-d3d9/tests/dxbc_parser.rs` (`#![cfg(feature = "dxbc-robust")]`)

**How to run:**
```bash
# Run the robust parser unit tests in `aero-dxbc` itself:
bash ./scripts/safe-run.sh cargo test -p aero-dxbc --features robust --locked

# Enables aero-dxbc's `robust` feature via aero-d3d9's `dxbc-robust` feature.
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --features dxbc-robust --test dxbc_parser --locked
```

---

### Task 62 / 66 / 69 — `SharedSurfaceTable` refactor across command processors + executors (D3D9/D3D11)

**Status:** ✅ Done

**See also:**
- [`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md) (the `share_token` vs user-mode `HANDLE` contract + collision/retirement policy; cross-process test pointers)

**Implementation (key files):**
- `crates/aero-gpu/src/shared_surface.rs` (single source of truth)
- Used by:
  - `crates/aero-gpu/src/command_processor.rs`
  - `crates/aero-gpu/src/command_processor_d3d9.rs`
  - `crates/aero-gpu/src/aerogpu_d3d9_executor.rs`
  - `crates/aero-gpu/src/acmd_executor.rs`
  - `crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs` (via a thin wrapper)

**Implementing commits (high-signal):**
- `36c5e5f2` — `refactor(aero-gpu): use SharedSurfaceTable in command processor`
- `d37c607a` — `refactor: reuse SharedSurfaceTable in D3D9 command processor`
- `f75daac9` — `refactor(aero-gpu): use SharedSurfaceTable in D3D9 executor`
- `527ac6db8` — `refactor(shared-surface): reuse aero-gpu table in D3D11 executor`

**Tests:**
- `crates/aero-gpu/src/shared_surface.rs` (unit tests for token retirement/idempotency/etc)
- `crates/aero-gpu/src/tests/shared_surface.rs` (additional unit tests for alias/refcount/token rules)
- `crates/aero-gpu/tests/shared_surface_aliasing.rs`
- `crates/aero-gpu/tests/aerogpu_d3d9_shared_surface.rs`
- `crates/aero-gpu/tests/aerogpu_d3d9_cmd_stream_shared_surface.rs` (end-to-end cmd-stream coverage)
- `crates/aero-d3d11/tests/aerogpu_cmd_shared_surface.rs` (D3D11 executor integration coverage; may skip if wgpu/WebGPU is unavailable)

**Notes:**
- The D3D11 executor reuses the canonical `aero-gpu` shared-surface bookkeeping:
  - `crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs` wraps `aero_gpu::SharedSurfaceTable`
    (thin error-message adaptation), so D3D9 + D3D11 share the same alias/refcount/token-retirement
    semantics.

**How to run (focused):**
```bash
# CPU-only unit tests for the shared-surface bookkeeping:
bash ./scripts/safe-run.sh cargo test -p aero-gpu --lib shared_surface::tests --locked
bash ./scripts/safe-run.sh cargo test -p aero-gpu --lib tests::shared_surface --locked

bash ./scripts/safe-run.sh cargo test -p aero-gpu --test shared_surface_aliasing --locked
bash ./scripts/safe-run.sh cargo test -p aero-gpu --test aerogpu_d3d9_shared_surface --locked
bash ./scripts/safe-run.sh cargo test -p aero-gpu --test aerogpu_d3d9_cmd_stream_shared_surface --locked

# D3D11 executor shared-surface behavior (wgpu/WebGPU; may skip unless AERO_REQUIRE_WEBGPU=1):
bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_shared_surface --locked
```

---

### Task 85 / 87 / 88 / 92 / 93 / 94 — SM3 opcode + modifier + const support

Ops/features referenced by the scratchpad tasks:
`frc`, `cmp`, `mova`, `defi`, `defb`, source modifiers, `lrp`, `exp`, `log`, `pow`.

**Status:** ✅ Done

**Implementation (key files):**
- `crates/aero-d3d9/src/sm3/{decode.rs,ir.rs,ir_builder.rs,wgsl.rs,software.rs,verify.rs}`

**Implementing commits (high-signal):**
- `d190c9a6` — `feat(sm3): support defi/defb consts in IR + WGSL`
- `9f4ff084` — `feat(aero-d3d9/sm3): support frc/cmp opcodes in decode/IR/WGSL`
- `6f0e9530` — `feat(sm3): add mova opcode and WGSL lowering for address regs`
- `570416e6` — `feat(aero-d3d9/sm3): support D3D9 src modifiers in decode+WGSL`
- `4c3f1e25` — `feat(sm3): support lrp and emit WGSL`
- `77dca861` — `feat(aero-d3d9/sm3): add exp/log/pow + WGSL lowering`

**Tests:**
- `crates/aero-d3d9/src/tests.rs`
  - `micro_ps2_src_and_result_modifiers_pixel_compare` (src modifiers + result modifiers)
  - `micro_ps3_lrp_pixel_compare`
  - `sm3_exp_log_pow_pixel_compare`
- `crates/aero-d3d9/tests/sm3_wgsl.rs` / `sm3_wgsl_math.rs` (naga-validated WGSL lowering)
- `crates/aero-d3d9/tests/sm3_wgsl_mova.rs` (mova + relative constant addressing)
- `crates/aero-d3d9/tests/sm3_wgsl_relative_const_defs.rs` (relative const indexing with many embedded `def` overrides)

**How to run:**
```bash
# CPU-only WGSL + naga validation coverage:
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl_math --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl_mova --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl_relative_const_defs --locked
```

---

### Task 124 — D3D9 half-pixel center convention (`half_pixel_center`)

**Status:** ✅ Done

**What:** Optional emulation of D3D9’s classic “half-pixel offset” by nudging the final clip-space
vertex position by `(-1/viewport_width, +1/viewport_height) * w` in translated vertex shaders.
This is enabled via `WgslOptions::half_pixel_center` and is wired end-to-end through the D3D9
executor.

**Implementation (key files):**
- Translation (SM3-first path): `crates/aero-d3d9/src/shader_translate.rs`
  - `inject_half_pixel_center_sm3_vertex_wgsl` injects `@group(3) @binding(0)` `HalfPixel` uniform
    + clip-space adjustment.
- Translation (legacy fallback path): `crates/aero-d3d9/src/shader.rs`
  - `WgslOptions::half_pixel_center` emits the same uniform + adjustment for the legacy
    token-stream translator.
- Execution: `crates/aero-gpu/src/aerogpu_d3d9_executor.rs`
  - creates/binds the half-pixel bind group at `@group(3) @binding(0)`
  - updates the uniform on `AeroGpuCmd::SetViewport`.

**Tests:**
- `crates/aero-gpu/tests/aerogpu_d3d9_half_pixel_center.rs` (pixel-level rasterization shift)

**How to run:**
```bash
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-gpu --test aerogpu_d3d9_half_pixel_center --locked
```

---

### Task 125 / 400 — consistent VS↔PS varying location mapping + WGSL IO structs

**Status:** ✅ Done

**Implementation (key files):**
- `crates/aero-d3d9/src/sm3/wgsl.rs` (emits `VsInput`/`VsOut`/`FsIn`/`FsOut` structs and stable `@location(n)` mapping)

**Implementing commits (high-signal):**
- `81181fad` — `feat(aero-d3d9): add sm3 WGSL backend with stable varying locations`
- `fdc5ee53` — `test(aero-d3d9/sm3): cover VS->PS varying @location mapping`

**Tests:**
- `crates/aero-d3d9/tests/sm3_wgsl.rs::wgsl_vs_outputs_and_ps_inputs_use_consistent_locations`
- `crates/aero-d3d9/tests/sm3_wgsl_semantic_locations.rs::sm3_vs_output_and_ps_input_semantics_share_locations`

**How to run (focused):**
```bash
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl_semantic_locations --locked
```

---

### Task 216 / 217 — `dp2` + `dsx`/`dsy` derivatives

**Status:** ✅ Done

**Implementation (key files):**
- `crates/aero-d3d9/src/sm3/{decode.rs,ir_builder.rs,wgsl.rs,verify.rs}`

**Implementing commits (high-signal):**
- `571cfa54` — `feat(d3d9-sm3): add dp2 opcode end-to-end`
- `5067c94f` — `feat(d3d9-sm3): support dsx/dsy derivatives`
- `8660b7710` — `feat(d3d9): add dsx/dsy support to legacy shader translator` (fallback path)
- `4c9adf49c` — `feat(legacy): add dsx/dsy opcode support to aero-d3d9-shader parser` (reference disassembler)

**Tests:**
- `crates/aero-d3d9/tests/sm3_wgsl_dp2.rs`
- `crates/aero-d3d9/tests/sm3_wgsl.rs`
  - `wgsl_dsx_dsy_derivatives_compile`
  - `wgsl_dsx_dsy_can_feed_texldd_gradients`
  - `wgsl_predicated_derivative_avoids_non_uniform_control_flow`
- `crates/aero-d3d9/tests/sm3_decode.rs`
  - `decode_rejects_dsx_in_vertex_shader`
  - `decode_rejects_dsy_in_vertex_shader`
- `crates/aero-d3d9/src/tests.rs::translate_entrypoint_legacy_fallback_supports_derivatives`

**How to run (focused):**
```bash
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl_dp2 --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_decode --locked
```

---

### Task 401 / 402 — `TexSample` lowering/bindings + `texkill` semantics

**Status:** ✅ Done

**See also:** `../areas/graphics.md` (short “don’t duplicate work” status note for SM2/SM3 shader translation).

**Implemented:**
- WGSL lowering for `texld`/`texldp`/`texldb`/`texldd`/`texldl` (`textureSample*` variants) and bind group layout mapping for samplers/textures.
- `texkill` lowers to D3D9 semantics: `discard` when **any component** of the operand is `< 0`, and preserves predication nesting.
- Sampler declarations map texture types to WGSL texture bindings and coordinate dimensionality:
  - `dcl_1d` → `texture_1d<f32>` (`x`)
  - `dcl_2d` → `texture_2d<f32>` (`xy`)
  - `dcl_volume` → `texture_3d<f32>` (`xyz`)
  - `dcl_cube` → `texture_cube<f32>` (`xyz`)
  - If a sampler has no `dcl_*` declaration, it defaults to `Texture2D` and is recorded as such in `bind_group_layout.sampler_texture_types`.

**Implementation (key files):**
- `crates/aero-d3d9/src/sm3/wgsl.rs` (sampler bindings + `IrOp::TexSample` lowering + `Stmt::Discard`)
- `crates/aero-d3d9/src/sm3/ir_builder.rs` (decode → IR for tex ops)

**Implementing commits (high-signal):**
- `aa89e80b` — `feat(aero-d3d9/sm3): emit WGSL for TexSample ops`
- `57aa3f8c` — `fix(sm3): preserve texkill predication and D3D9 discard semantics`

**Tracking cleanup / additional coverage commits:**
- `04c80402` — `docs(d3d9-sm3): mark TexSample/texkill tasks done`
- `2f099e8dd` — `test(sm3): cover cube/3D sampler dcl in WGSL`
- `1dcec36ab` — `test(sm3): add 1D sampler dcl coverage`
- `0516665f7` — `test(sm3): cover non-2D texldp/texldd swizzles`
- `b6b6bec11` — `test(sm3): cover 1D texldp/texldd and clean up software matcher`
- `362261e8d` — `test(sm3): cover texldl swizzles for 1D/3D/cube samplers`
- `02e042470` — `fix(sm3): support texldb bias for 1D textures in WGSL`
- `dae0504ad` — `test(sm3): ensure predicated texldb avoids non-uniform control flow`
- `b10bb36f3` — `docs(graphics): mark SM3 TexSample/texkill tasks 401/402 done`
- `6617e2bc5` — `docs(graphics): cross-link SM3 shader translation task notes`
- `9f3c546f8` — `docs(graphics): link task-489 audit from SM3 translation notes`
- `5fb505938` — `docs(graphics): shorten SM3 shader translation status table`
- `e8523a8f9` — `test(sm3): assert default sampler texture types in bind layout`
- `de317d81a` — `test(sm3): cover translate_to_wgsl wrapper`
- `b0ccdf25e` — `docs(graphics): document default Texture2D sampler type`
- `9bb17163c` — `feat(d3d9): support cube textures in software shader interpreters`

**Tests:**
- `crates/aero-d3d9/tests/sm3_wgsl.rs`
  - `sm3_translate_to_wgsl_wrapper_produces_bind_layout`
  - `wgsl_texld_emits_texture_sample`
  - `wgsl_texldp_emits_projective_divide`
  - `wgsl_texldb_emits_texture_sample_bias`
  - `wgsl_texldd_emits_texture_sample_grad`
  - `wgsl_vs_texld_emits_texture_sample_level`
  - `wgsl_texldl_emits_texture_sample_level_explicit_lod`
  - `wgsl_predicated_texld_avoids_non_uniform_control_flow`
  - `wgsl_predicated_texldb_avoids_non_uniform_control_flow`
  - `wgsl_predicated_texldb_1d_avoids_non_uniform_control_flow`
  - `wgsl_predicated_texldp_avoids_non_uniform_control_flow`
  - `wgsl_predicated_texldd_is_valid_with_non_uniform_predicate`
  - `wgsl_nonuniform_if_texld_avoids_invalid_control_flow`
  - `wgsl_nonuniform_if_else_texld_avoids_invalid_control_flow`
  - `wgsl_nonuniform_if_texldb_avoids_invalid_control_flow`
  - `wgsl_nonuniform_if_texldb_1d_avoids_invalid_control_flow`
  - `wgsl_nonuniform_if_predicated_texld_avoids_invalid_control_flow`
  - `wgsl_nonuniform_if_else_predicated_texld_avoids_invalid_control_flow`
  - `wgsl_dcl_1d_sampler_emits_texture_1d_and_x_coord`
  - `wgsl_dcl_1d_sampler_texldp_emits_projective_divide_x`
  - `wgsl_dcl_1d_sampler_texldb_emits_texture_sample_grad_x_with_bias`
  - `wgsl_dcl_1d_sampler_texldd_emits_texture_sample_grad_x`
  - `wgsl_dcl_1d_sampler_texldl_emits_texture_sample_level_x_lod`
  - `wgsl_dcl_cube_sampler_emits_texture_cube_and_xyz_coords`
  - `wgsl_dcl_volume_sampler_emits_texture_3d_and_xyz_coords`
  - `wgsl_dcl_cube_sampler_texldp_emits_projective_divide_xyz`
  - `wgsl_dcl_cube_sampler_texldb_emits_texture_sample_bias_xyz`
  - `wgsl_dcl_volume_sampler_texldp_emits_projective_divide_xyz`
  - `wgsl_dcl_cube_sampler_texldd_emits_texture_sample_grad_xyz`
  - `wgsl_dcl_volume_sampler_texldd_emits_texture_sample_grad_xyz`
  - `wgsl_dcl_cube_sampler_texldl_emits_texture_sample_level_xyz_lod`
  - `wgsl_dcl_volume_sampler_texldb_emits_texture_sample_bias_xyz`
  - `wgsl_dcl_volume_sampler_texldl_emits_texture_sample_level_xyz_lod`
  - `wgsl_texkill_is_conditional`
  - `wgsl_predicated_texkill_is_nested_under_if`
- `crates/aero-d3d9/tests/sm3_wgsl_tex.rs`
  - `wgsl_ps3_texldp_is_valid`
  - `wgsl_ps3_texld_cube_is_valid`
  - `wgsl_ps3_texkill_discard_is_valid`
  - `wgsl_ps3_texld_cube_sampler_emits_texture_cube`
  - `wgsl_ps3_texld_3d_sampler_emits_texture_3d`

**How to run (focused):**
```bash
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl_tex --locked
```

**Notes / follow-ups:**
- The SM3 WGSL backend supports sampler texture types 1D/2D/3D/cube.
- WGSL does not support `textureSampleBias` for `texture_1d`; `texldb` for 1D samplers is lowered via
  `textureSampleGrad` with `dpdx`/`dpdy` scaled by `exp2(bias)`.
- Software interpreters emulate 1D + 3D sampling:
  - Legacy interpreter: `crates/aero-d3d9/src/software.rs` (`Texture1D`/`Texture3D` + `Op::Texld`)
  - SM3 interpreter: `crates/aero-d3d9/src/sm3/software.rs` (`IrOp::TexSample`)
  - Tests: `crates/aero-d3d9/tests/software_textures.rs` (CPU sampling correctness)
- The legacy token-stream translator in `crates/aero-d3d9/src/shader.rs` supports sampler texture type declarations for
  1D/2D/3D/cube (`dcl_1d` / `dcl_2d` / `dcl_volume` / `dcl_cube`) and emits the corresponding WGSL bindings
  (`texture_1d<f32>` / `texture_2d<f32>` / `texture_3d<f32>` / `texture_cube<f32>`) with the correct coordinate
  dimensionality (`x` / `xy` / `xyz`).
  - Evidence: `crates/aero-d3d9/src/tests.rs::legacy_translator_emits_texture_1d_and_x_coords` and
    `crates/aero-d3d9/src/tests.rs::legacy_translator_emits_texture_3d_and_xyz_coords`.
- Note: the AeroGPU command protocol currently only creates 2D textures (`CREATE_TEXTURE2D`), so it cannot yet supply
  real guest-backed 1D/3D textures (cube is represented as `array_layers=6`).
  - Translation supports 1D/3D sampler declarations and sampling ops, but the D3D9 executor binds dummy 1D/3D textures
    for those sampler dimensions until the protocol gains real 1D/3D texture creation/binding.
- The WGSL generator does not attempt to model sampler *state* (filtering/address modes/LOD bias/etc.) directly;
  those are handled in runtime pipeline setup. Depth-compare sampling is also not modeled in the SM3 WGSL generator.
  (This is tracked in `../areas/graphics.md`.)

---

### Task 439 — SM3 PS MISCTYPE builtins (vPos/vFace)

**Status:** ✅ Done

**Implementation (key files):**
- `crates/aero-d3d9/src/sm3/wgsl.rs` (builtin emission + local mapping for `misc0`/`misc1`)

**Implementing commits (high-signal):**
- `0b946c43c` — `feat(d3d9): Add WGSL support for SM3 vPos/vFace MISCTYPE`

**Tests:**
- `crates/aero-d3d9/tests/sm3_wgsl.rs`
  - `wgsl_ps3_vpos_misctype_builtin_compiles`
  - `wgsl_ps3_vface_misctype_builtin_compiles`

**How to run (focused):**
```bash
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl --locked
```

---

### Task 468 — SM3 PS oDepth / frag_depth

**Status:** ✅ Done

**Implementation (key files):**
- `crates/aero-d3d9/src/sm3/wgsl.rs` (maps `DepthOut` / `oDepth` to `@builtin(frag_depth)` and assigns from `oDepth.x`)

**Implementing commits (high-signal):**
- `a360c250a` — `feat(aero-d3d9): map SM2/SM3 oDepth to WGSL frag_depth`

**Tests:**
- `crates/aero-d3d9/tests/sm3_wgsl_depth_out.rs`
  - `wgsl_ps30_writes_odepth_emits_frag_depth`

**How to run (focused):**
```bash
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl_depth_out --locked
```

## AeroGPU Prototype Command Protocol (Surface ABI, v0.1)

> **DEPRECATED / PROTOTYPE ABI**
>
> This document describes an **early, surface-centric** AeroGPU command protocol
> (`CREATE_SURFACE` / `UPDATE_SURFACE` / `PRESENT`) that matched the legacy
> toy prototype device model (since removed from the codebase).
>
> That prototype is now removed; this is **not** the Windows 7
> WDDM AeroGPU ABI currently supported by the project.
>
> **Canonical (supported) AeroGPU ABI docs:** `drivers/aerogpu/protocol/README.md`
>
> **Headers:**
> - `drivers/aerogpu/protocol/aerogpu_pci.h`
> - `drivers/aerogpu/protocol/aerogpu_ring.h`
> - `drivers/aerogpu/protocol/aerogpu_cmd.h`

---

This document specifies the **guest ↔ host** command ABI for the legacy AeroGPU
prototype used by Aero’s Windows 7 emulator.

The intent was to provide a *stable transport* layer that later DirectX/WDDM
translation components could target, without baking in Direct3D details yet.

### Wire rules

#### Endianness

All multi-byte fields are **little-endian**.

#### Alignment

- All command and event entries in rings are **8-byte aligned**.
- `size_bytes` for ring entries **must be a multiple of 8**.
- Ring buffer size must be a multiple of 8.

Rationale: commands frequently contain 64-bit guest physical addresses; 8-byte alignment avoids straddling across cache lines and simplifies wrap handling.

#### Versioning

The ABI is versioned as `{major, minor, patch}`:

- **Major** increments on incompatible layout/semantic changes.
- **Minor** increments when new opcodes/capabilities are added in a backwards-compatible way.
- **Patch** increments for bugfixes with no ABI changes.

The device exposes a packed version in `MMIO.VERSION`:

```
VERSION = (major << 16) | (minor << 8) | patch
```

Guests should:

1) Read `VERSION`
2) Check `major` is supported
3) Use `minor`/`CAPS` to enable optional features

### Shared memory layout

The device provides a shared-memory region (e.g. a PCI BAR, or a fixed MMIO mapping) that contains:

- A **capabilities struct**
- A **command ring** (guest → host)
- An optional **event ring** (host → guest)

#### 2.1 Capabilities

The host exposes a capability bitmask (`CAPS` register) and a shared `Caps` struct. The current minimal capabilities are:

- `CAPS_EVENT_RING` (bit 0): event ring is present
- `CAPS_FORMAT_RGBA8888` (bit 1): `SurfaceFormat::Rgba8888` is supported

##### `Caps` struct layout

The shared capabilities struct is a fixed-size, little-endian blob:

```c
// size: 16 bytes
struct Caps {
  u32 caps_bits;
  u32 max_surface_width;
  u32 max_surface_height;
  u32 max_surfaces;
}
```

#### 2.2 Rings

Rings are single-producer/single-consumer and use atomic indices in shared memory:

- `head`: read position (advanced by consumer)
- `tail`: committed write position (advanced by producer *after* writing full entries)

Indices are measured in **bytes**, are **monotonic modulo 2³²**, and are interpreted modulo `ring_size_bytes` when indexing into the ring data array.

##### Ring control layout (conceptual)

```c
struct RingControl {
  atomic_u32 head; // bytes
  atomic_u32 tail; // bytes
  u32 ring_size_bytes;
  u32 reserved;
  u8  ring_data[ring_size_bytes];
}
```

##### Wrap-around handling

Ring entries are required to be **contiguous** within the ring’s byte array. If a producer cannot fit a command at the end of the ring, it must:

1) Emit a **NOP** padding entry that consumes the remaining bytes to the end of the ring
2) Continue writing the next entry at offset 0

The consumer skips NOP padding entries.

##### Backpressure (full ring)

Producer must not advance `tail` if there isn’t enough free space (`ring_size_bytes - used_bytes`) to write the entry (and any required wrap padding). If full, the producer must retry later.

### MMIO register block

The device exposes a small MMIO register block (offsets in bytes):

| Offset | Name | R/W | Description |
|---:|---|:--:|---|
| 0x00 | `VERSION` | R | Packed ABI version |
| 0x04 | `CAPS` | R | Capability bitmask |
| 0x08 | `CMD_RING_DOORBELL` | W | Rings the doorbell to wake the host GPU worker |
| 0x0C | `IRQ_STATUS` | R | Interrupt status bits |
| 0x10 | `IRQ_ACK` | W | Write-1-to-clear interrupt bits |
| 0x14 | `CMD_RING_HEAD` | R | Debug: current command ring head (mod ring size) |
| 0x18 | `CMD_RING_TAIL` | R | Debug: current command ring tail (mod ring size) |
| 0x1C | `RESET` | W | Device reset (clears rings/resources) |

#### Interrupt bits (`IRQ_STATUS`)

- bit 0: `CMD_PROCESSED` – some command(s) have been processed since last ACK
- bit 1: `PRESENT_DONE` – a `PRESENT` has completed

### Ring entry formats

#### 4.1 Command header

All commands begin with:

```c
struct CmdHeader {
  u32 opcode;
  u32 size_bytes; // total entry size including this header
}
```

#### 4.2 Event header

All events begin with:

```c
struct EventHeader {
  u32 event_type;
  u32 size_bytes; // total entry size including this header
}
```

### Minimal opcodes (smoke testing)

All opcodes below are **required** for the v0.1 smoke tests.

Unless otherwise specified, all fields are `u32` little-endian.

#### 5.1 `CREATE_SURFACE` (opcode = 1)

Creates a new host-side surface and returns its ID via an event.

Payload:

```
u32 width
u32 height
u32 format
```

#### 5.2 `UPDATE_SURFACE` (opcode = 2)

Updates a surface’s pixel contents by copying from guest physical memory.

Payload:

```
u32 surface_id
u64 guest_phys_addr
u32 stride_bytes
```

#### 5.3 `CLEAR_RGBA` (opcode = 3)

Clears the entire surface to a constant color.

Payload:

```
u32 surface_id
u32 rgba // packed r | g<<8 | b<<16 | a<<24
```

#### 5.4 `DRAW_TRIANGLE_TEST` (opcode = 4)

Diagnostic opcode: draws a fixed red triangle into the surface (software reference implementation for now).

Payload:

```
u32 surface_id
```

#### 5.5 `PRESENT` (opcode = 5)

Presents a surface to the device’s front buffer and triggers `PRESENT_DONE`.

Payload:

```
u32 surface_id
```

### Error reporting

The primary error-reporting channel is the **event ring**.

For each processed command, the host emits an `EVENT_CMD_STATUS` entry:

```
event_type = 1 (CMD_STATUS)
payload:
  u32 opcode
  u32 status
  u32 data0
  u32 data1
  u32 data2
  u32 data3
```

`status` is one of:

- `0 OK`
- `1 INVALID_OPCODE`
- `2 INVALID_SIZE`
- `3 INVALID_ARGUMENT`
- `4 SURFACE_NOT_FOUND`
- `5 UNSUPPORTED_FORMAT`
- `6 GUEST_MEMORY_FAULT`
- `7 OUT_OF_MEMORY`

Unknown opcodes must not crash the host; they must return `INVALID_OPCODE` and continue processing subsequent commands.

## Legacy (Obsolete): Experimental GPU Command ABI (retired)

> Status: **retired**. This ABI is not implemented by the current emulator device model
> and is not used by the Win7 AeroGPU WDDM driver stack.

This repository previously contained an experimental GPU command ABI used for early
host-side experiments. It has been retired so that new work converges on the single
canonical AeroGPU protocol defined by the versioned headers:

- `drivers/aerogpu/protocol/*` (source of truth)
- `crates/aero-protocol/aerogpu/*` (generated Rust/TypeScript mirrors)

If you are implementing AeroGPU, writing a Windows driver, or capturing/replaying GPU
traces, use the canonical A3A0 protocol (`drivers/aerogpu/protocol/README.md`) and do
not depend on any legacy experimental command ABIs.

---
