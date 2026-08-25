# AeroGPU device ABI

> Normative contract between the Windows 7 AeroGPU WDDM driver stack (KMD + UMD)
> and the emulator's virtual GPU device model: PCI identity, BAR0 register file,
> the submission ring, the per-submit allocation table, the fence model, and the
> framing rules of the command stream.
>
> **The C headers under `drivers/aerogpu/protocol/` are the source of truth.**
> Where this page and a header disagree, the header wins and this page is the
> defect. The packet-by-packet payload layouts live in
> [aerogpu-command-stream.md](./aerogpu-command-stream.md); the driver-side view
> of the same contract lives in
> [windows7-aerogpu-wddm-driver.md](./windows7-aerogpu-wddm-driver.md).

## Scope and sources

| Concern | Header |
|---|---|
| PCI IDs, BAR layout, MMIO register map, shared enums | `aerogpu_pci.h` |
| Ring header, submit descriptor, allocation table, fence page | `aerogpu_ring.h` |
| Command stream packets ("AeroGPU IR") | `aerogpu_cmd.h` |
| Adapter/feature discovery blob for user mode | `aerogpu_umd_private.h` |
| WDDM allocation private data (`alloc_id`, `share_token`) | `aerogpu_wddm_alloc.h` |
| WOW64-stable user↔kernel blobs | `aerogpu_win7_abi.h` |
| Escape packet header and base operations | `aerogpu_escape.h` |
| Bring-up tooling escapes | `aerogpu_dbgctl_escape.h` |
| Legacy bring-up ABI (deprecated) | `legacy/aerogpu_protocol_legacy.h` |

The emulator carries Rust and TypeScript mirrors of the same ABI for host-side
parsing and tooling — `crates/aero-protocol/aerogpu/*.rs` and `*.ts`, published as
the `aero-protocol` crate. Mirrors move in lock-step with the headers, and the
lock-step is mechanically enforced: a small C "ABI dump" helper
(`crates/aero-protocol/tests/aerogpu_abi_dump.c`) is compiled and run, and its
constants, struct sizes, field offsets, and opcode coverage are compared against
both mirrors.

```bash
cargo test -p aero-protocol --locked
pnpm run test:protocol
```

Additional conformance: `crates/aero-protocol/tests/aerogpu_abi.rs`,
`aerogpu_abi.test.ts`, `aerogpu_pci_id_conformance.rs`.

## Identity

AeroGPU is a paravirtual PCI display controller. For AeroGPU the vendor/device ID
pair is *part of the ABI contract*: a Windows driver binds only to a device model
that implements the matching MMIO and ring protocol, so the ID is what tells the
guest which protocol it is talking to.

| Field | Value | Constant |
|---|---|---|
| Vendor ID | `0xA3A0` | `AEROGPU_PCI_VENDOR_ID` |
| Device ID | `0x0001` | `AEROGPU_PCI_DEVICE_ID` |
| Subsystem vendor ID | `0xA3A0` | `AEROGPU_PCI_SUBSYSTEM_VENDOR_ID` |
| Subsystem ID | `0x0001` | `AEROGPU_PCI_SUBSYSTEM_ID` |
| Base class | `0x03` (display controller) | `AEROGPU_PCI_CLASS_CODE_DISPLAY_CONTROLLER` |
| Subclass | `0x00` (VGA compatible) | `AEROGPU_PCI_SUBCLASS_VGA_COMPATIBLE` |
| Prog-IF | `0x00` | `AEROGPU_PCI_PROG_IF` |
| Hardware ID | `PCI\VEN_A3A0&DEV_0001` | — |

The VGA-compatible subclass is deliberate: it is what makes Windows 7 attach the
WDDM display stack to the device.

These IDs are project-local. They are not assigned by PCI-SIG and have no meaning
outside Aero.

### BARs

| BAR | Purpose | Size | Constant |
|---|---|---|---|
| 0 | MMIO register block | 64 KiB | `AEROGPU_PCI_BAR0_SIZE_BYTES` |
| 1 | Prefetchable MMIO VRAM aperture | 64 MiB | `AEROGPU_PCI_BAR1_SIZE_BYTES` |

BAR0 is sized well beyond the registers actually defined; the remainder is
reserved for forward-compatible register growth.

BAR1 exists for legacy VGA/VBE boot-display compatibility and sits **outside the
WDDM memory model** — the in-tree Win7 driver treats the adapter as
system-memory-backed and never advertises BAR1 as a WDDM VRAM segment. When the
AeroGPU-owned boot display path is active, firmware derives the VBE linear
framebuffer base from BAR1:

```
PhysBasePtr = BAR1_BASE + AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES   /* 0x40000 */
```

On `wasm32` hosts the BAR1 backing allocation is capped at 32 MiB to fit browser
heap constraints. The guest-visible BAR still reports the full aperture size;
reads beyond the backing return zero and writes are discarded.

### Interrupt wiring

AeroGPU delivers interrupts through legacy PCI INTx. A level-triggered line IRQ
is sufficient; MSI/MSI-X is a possible future addition.

- Interrupt Pin (config `0x3D`) = `1` (INTA#)
- Interrupt Line (config `0x3C`) = the line chosen by the platform INTx router
  (swizzle plus PIRQ→GSI mapping)

At the canonical BDF `00:07.0` with the default PC routing table
(`PIRQ[A-D] → GSI[20-23]`), this resolves to **GSI 23**. In legacy PIC mode
that route mirrors to IRQ13, which remains the value reported in PCI
config-space `Interrupt Line`. The contract is held by
`crates/aero-machine/tests/aerogpu_pci_enumeration.rs`.

### ABI generations

Two generations exist in the tree, distinguished by vendor ID; the device ID is
`0x0001` in both.

| Generation | PCI IDs | MMIO magic | Header | Status |
|---|---|---|---|---|
| Versioned | `A3A0:0001` | `"AGPU"` | `aerogpu_pci.h` + `aerogpu_ring.h` + `aerogpu_cmd.h` | Canonical |
| Legacy bring-up | `1AED:0001` | `"ARGP"` | `legacy/aerogpu_protocol_legacy.h` | Deprecated; retained for regression testing |

The legacy generation is the original minimal ABI used to bring the Windows 7
WDDM stack up at all. Its emulator device model is feature-gated behind
`emulator/aerogpu-legacy`, and the shipped INFs
(`drivers/aerogpu/packaging/win7/{aerogpu.inf,aerogpu_dx11.inf}`) bind only to
`PCI\VEN_A3A0&DEV_0001`. Installing against the legacy device model requires the
separate INFs under `drivers/aerogpu/packaging/win7/legacy/`. The Win7 KMD itself
speaks both transports and auto-detects which is live by reading the BAR0 magic,
so a single driver binary covers both device models.

Two further prototype protocols are retired rather than deprecated, and neither
is an ABI anyone should target: a toy `CREATE_SURFACE`/`PRESENT` ring used for
the earliest smoke tests, and an experimental cmd/completion ring built to
exercise GPU-trace plumbing. Both used the stale placeholder vendor ID `1AE0`.
Their history is recorded in [history/retirements.md](../history/retirements.md).

## Versioning, endianness, packing

The versioned ABI carries a **major.minor** version, reported by the device in
`AEROGPU_MMIO_REG_ABI_VERSION` as `AEROGPU_ABI_VERSION_U32`
(`(major << 16) | minor`). The current version is **1.4**.

- **Major** changes are breaking. A driver built for major *N* must refuse a
  device advertising major *N+1*.
- **Minor** changes are forwards compatible; a driver accepts a higher minor
  within the same major. A minor bump may add MMIO registers in reserved space,
  new command opcodes, new optional feature bits, or larger structures — the last
  only where an explicit `size_bytes` field makes the growth detectable, or via a
  new opcode.

The guest is x86/x64 Windows, so every multi-byte field is little-endian. Command
buffers and tables are sequences of packed structures; headers use
`#pragma pack(push, 1)` wherever the layout must be exact. Any structure that can
vary in size carries an explicit `size_bytes`, and packets additionally carry one
in their header.

## BAR0 register file

All registers are 32 bits unless stated otherwise, and 64-bit values occupy
consecutive `_LO`/`_HI` halves.

### Discovery and features

| Offset | Register | Access | Description |
|---:|---|:--:|---|
| `0x0000` | `AEROGPU_MMIO_REG_MAGIC` | RO | `AEROGPU_MMIO_MAGIC` = `0x55504741` (`"AGPU"` LE) |
| `0x0004` | `AEROGPU_MMIO_REG_ABI_VERSION` | RO | `AEROGPU_ABI_VERSION_U32` |
| `0x0008` | `AEROGPU_MMIO_REG_FEATURES_LO` | RO | Feature mask, low 32 bits |
| `0x000C` | `AEROGPU_MMIO_REG_FEATURES_HI` | RO | Feature mask, high 32 bits |

Feature bits, read as one 64-bit mask:

| Bit | Feature | Meaning | Since |
|---:|---|---|---|
| 0 | `AEROGPU_FEATURE_FENCE_PAGE` | Shared fence page supported | 1.0 |
| 1 | `AEROGPU_FEATURE_CURSOR` | Cursor registers implemented | 1.0 |
| 2 | `AEROGPU_FEATURE_SCANOUT` | Scanout registers implemented | 1.0 |
| 3 | `AEROGPU_FEATURE_VBLANK` | Vblank IRQ and timing registers implemented | 1.0 |
| 4 | `AEROGPU_FEATURE_TRANSFER` | Transfer/copy commands, including optional writeback into guest memory | 1.1 |
| 5 | `AEROGPU_FEATURE_ERROR_INFO` | Structured error registers implemented | 1.3 |

A register block gated by a feature bit must not be read unless the bit is set.

### Ring programming and doorbell

| Offset | Register | Access | Description |
|---:|---|:--:|---|
| `0x0100` | `AEROGPU_MMIO_REG_RING_GPA_LO` | RW | GPA of `aerogpu_ring_header`, low half |
| `0x0104` | `AEROGPU_MMIO_REG_RING_GPA_HI` | RW | GPA of `aerogpu_ring_header`, high half |
| `0x0108` | `AEROGPU_MMIO_REG_RING_SIZE_BYTES` | RW | Bytes mapped at `RING_GPA`; must be ≥ `aerogpu_ring_header.size_bytes` |
| `0x010C` | `AEROGPU_MMIO_REG_RING_CONTROL` | RW | `ENABLE` (bit 0), `RESET` (bit 1) |
| `0x0200` | `AEROGPU_MMIO_REG_DOORBELL` | WO | Any write notifies the device that `tail` advanced |

`AEROGPU_RING_CONTROL_ENABLE` is set by the driver once the ring is initialised
and programmed. `AEROGPU_RING_CONTROL_RESET` requests a ring reset and is the
mechanism the KMD uses to recover from a TDR.

### Fence and completion

| Offset | Register | Access | Description |
|---:|---|:--:|---|
| `0x0120` | `AEROGPU_MMIO_REG_FENCE_GPA_LO` | RW | GPA of `aerogpu_fence_page`, low half (only with `FENCE_PAGE`) |
| `0x0124` | `AEROGPU_MMIO_REG_FENCE_GPA_HI` | RW | GPA of `aerogpu_fence_page`, high half |
| `0x0130` | `AEROGPU_MMIO_REG_COMPLETED_FENCE_LO` | RO | Completed fence, low half |
| `0x0134` | `AEROGPU_MMIO_REG_COMPLETED_FENCE_HI` | RO | Completed fence, high half |

### Interrupts

| Offset | Register | Access | Description |
|---:|---|:--:|---|
| `0x0300` | `AEROGPU_MMIO_REG_IRQ_STATUS` | RO | Pending cause bits |
| `0x0304` | `AEROGPU_MMIO_REG_IRQ_ENABLE` | RW | Enable mask |
| `0x0308` | `AEROGPU_MMIO_REG_IRQ_ACK` | WO | Write-1-to-clear against `IRQ_STATUS` |

| Bit | Cause | Meaning |
|---:|---|---|
| 0 | `AEROGPU_IRQ_FENCE` | Completed fence advanced |
| 1 | `AEROGPU_IRQ_SCANOUT_VBLANK` | Scanout 0 vblank tick (only with `VBLANK`) |
| 31 | `AEROGPU_IRQ_ERROR` | Fatal device error |

The interrupt line is asserted while `(IRQ_STATUS & IRQ_ENABLE) != 0`.

### Error reporting

With `AEROGPU_FEATURE_ERROR_INFO` (ABI 1.3+), the device latches structured
detail alongside `AEROGPU_IRQ_ERROR`, because a single boolean cause bit carries
too little information to act on.

| Offset | Register | Access | Description |
|---:|---|:--:|---|
| `0x0310` | `AEROGPU_MMIO_REG_ERROR_CODE` | RO | `enum aerogpu_error_code` |
| `0x0314` | `AEROGPU_MMIO_REG_ERROR_FENCE_LO` | RO | Associated fence, low half |
| `0x0318` | `AEROGPU_MMIO_REG_ERROR_FENCE_HI` | RO | Associated fence, high half |
| `0x031C` | `AEROGPU_MMIO_REG_ERROR_COUNT` | RO | Monotonic, saturating error counter |

| Code | Value | Meaning |
|---|---:|---|
| `AEROGPU_ERROR_NONE` | 0 | No latched error |
| `AEROGPU_ERROR_CMD_DECODE` | 1 | Malformed command stream / decode failure |
| `AEROGPU_ERROR_OOB` | 2 | Out-of-bounds access or address overflow |
| `AEROGPU_ERROR_BACKEND` | 3 | Backend or device execution error |
| `AEROGPU_ERROR_INTERNAL` | `0xFFFF` | Internal or unclassified error |

The payload is **sticky**: it survives `IRQ_ACK` and persists until the next
error overwrites it or the device resets. Acknowledging the interrupt clears the
status bit, not the diagnosis.

### Scanout 0

Present only with `AEROGPU_FEATURE_SCANOUT`.

| Offset | Register | Access | Description |
|---:|---|:--:|---|
| `0x0400` | `AEROGPU_MMIO_REG_SCANOUT0_ENABLE` | RW | 0/1 |
| `0x0404` | `AEROGPU_MMIO_REG_SCANOUT0_WIDTH` | RW | Width in pixels |
| `0x0408` | `AEROGPU_MMIO_REG_SCANOUT0_HEIGHT` | RW | Height in pixels |
| `0x040C` | `AEROGPU_MMIO_REG_SCANOUT0_FORMAT` | RW | `enum aerogpu_format` |
| `0x0410` | `AEROGPU_MMIO_REG_SCANOUT0_PITCH_BYTES` | RW | Bytes per row |
| `0x0414` | `AEROGPU_MMIO_REG_SCANOUT0_FB_GPA_LO` | RW | Framebuffer GPA, low half |
| `0x0418` | `AEROGPU_MMIO_REG_SCANOUT0_FB_GPA_HI` | RW | Framebuffer GPA, high half |

Vblank timing, present only with `AEROGPU_FEATURE_VBLANK`:

| Offset | Register | Access | Description |
|---:|---|:--:|---|
| `0x0420` | `AEROGPU_MMIO_REG_SCANOUT0_VBLANK_SEQ_LO` | RO | Vblank sequence, low half |
| `0x0424` | `AEROGPU_MMIO_REG_SCANOUT0_VBLANK_SEQ_HI` | RO | Vblank sequence, high half |
| `0x0428` | `AEROGPU_MMIO_REG_SCANOUT0_VBLANK_TIME_NS_LO` | RO | Last vblank, ns since device boot, low half |
| `0x042C` | `AEROGPU_MMIO_REG_SCANOUT0_VBLANK_TIME_NS_HI` | RO | Last vblank, ns since device boot, high half |
| `0x0430` | `AEROGPU_MMIO_REG_SCANOUT0_VBLANK_PERIOD_NS` | RO | Nominal period (16 666 667 ns for 60 Hz) |

`SCANOUT0_ENABLE` is a **visibility toggle, not an ownership release.** Once WDDM
has claimed scanout, clearing the bit blanks output, stops vblank pacing, and
flushes vsync-paced fences, but the `wddm_scanout_active` latch stays held so the
legacy VGA/VBE path cannot reclaim scanout until the machine resets. The device
publishes a *disabled WDDM* descriptor in that state (source WDDM; base, width,
height and pitch all zero) rather than reverting to a legacy descriptor. Held by
`aerogpu_scanout_disable_publishes_wddm_disabled` and the unit test
`scanout_disable_keeps_wddm_ownership_latched`.

### Cursor

Present only with `AEROGPU_FEATURE_CURSOR`.

| Offset | Register | Access | Description |
|---:|---|:--:|---|
| `0x0500` | `AEROGPU_MMIO_REG_CURSOR_ENABLE` | RW | 0/1 |
| `0x0504` | `AEROGPU_MMIO_REG_CURSOR_X` | RW | Signed X position |
| `0x0508` | `AEROGPU_MMIO_REG_CURSOR_Y` | RW | Signed Y position |
| `0x050C` | `AEROGPU_MMIO_REG_CURSOR_HOT_X` | RW | Hotspot X |
| `0x0510` | `AEROGPU_MMIO_REG_CURSOR_HOT_Y` | RW | Hotspot Y |
| `0x0514` | `AEROGPU_MMIO_REG_CURSOR_WIDTH` | RW | Width in pixels |
| `0x0518` | `AEROGPU_MMIO_REG_CURSOR_HEIGHT` | RW | Height in pixels |
| `0x051C` | `AEROGPU_MMIO_REG_CURSOR_FORMAT` | RW | `enum aerogpu_format` |
| `0x0520` | `AEROGPU_MMIO_REG_CURSOR_FB_GPA_LO` | RW | Cursor framebuffer GPA, low half |
| `0x0524` | `AEROGPU_MMIO_REG_CURSOR_FB_GPA_HI` | RW | Cursor framebuffer GPA, high half |
| `0x0528` | `AEROGPU_MMIO_REG_CURSOR_PITCH_BYTES` | RW | Bytes per row |

### Format semantics for scanout and cursor

Two rules govern `enum aerogpu_format` wherever it appears in a scanout or cursor
register, and getting either wrong produces visibly wrong output:

- **X8 channels are fully opaque.** In `B8G8R8X8*` and `R8G8B8X8*`, the `X` byte
  carries no information. Consumers ignore its stored value and substitute
  `A = 0xFF` when converting to RGBA for presentation or cursor blending.
- **sRGB changes interpretation, not layout.** A `*_UNORM_SRGB` format is byte-identical to
  its `*_UNORM` sibling. Sampling decodes sRGB→linear and render-target writes may
  encode linear→sRGB, but presenters must not apply gamma a second time.

Scanout presentation currently accepts a bounded set of packed layouts: the 32bpp
family (`B8G8R8X8`, `B8G8R8A8`, `R8G8B8X8`, `R8G8B8A8` and their sRGB variants,
with X8 treated as opaque) and the 16bpp family (`B5G6R5`, opaque; `B5G5R5A1`,
one-bit alpha). Anything else publishes a deterministic disabled descriptor
rather than guessing.

## Submission transport

### Ring layout

The ring is one contiguous guest-memory region beginning at `RING_GPA`:

```
ring_gpa + 0x00   struct aerogpu_ring_header          (64 bytes)
ring_gpa + 0x40   slot[entry_count]                   (entry_stride_bytes each)
```

Each slot begins with an `aerogpu_submit_desc` prefix; `entry_stride_bytes` may
exceed that prefix to leave forward-compatible extension space.

`struct aerogpu_ring_header`, packed, 64 bytes:

| Offset | Type | Field | Notes |
|---:|---|---|---|
| `0x00` | `u32` | `magic` | `AEROGPU_RING_MAGIC` = `0x474E5241` (`"ARNG"` LE) |
| `0x04` | `u32` | `abi_version` | `AEROGPU_ABI_VERSION_U32` |
| `0x08` | `u32` | `size_bytes` | Declared ring size; ≤ `AEROGPU_MMIO_REG_RING_SIZE_BYTES` |
| `0x0C` | `u32` | `entry_count` | Slot count; must be a power of two |
| `0x10` | `u32` | `entry_stride_bytes` | ≥ `sizeof(aerogpu_submit_desc)` (64) |
| `0x14` | `u32` | `flags` | Reserved, zero |
| `0x18` | `volatile u32` | `head` | Device-owned, monotonic submission index |
| `0x1C` | `volatile u32` | `tail` | Driver-owned, monotonic submission index |
| `0x20` | `u32` | `reserved0` | Zero |
| `0x24` | `u32` | `reserved1` | Zero |
| `0x28` | `u64[3]` | `reserved2` | Zero |

`size_bytes` is the declared extent — `sizeof(ring_header) + entry_count *
entry_stride_bytes` — and may be smaller than the mapped size, which is allowed
to be larger for page rounding or extension space.

`head` and `tail` are monotonic indices, not masked offsets; the slot for an
index is `index % entry_count`. The driver must not advance `tail` past the point
where it would overwrite unconsumed entries, keeping `(tail - head) <
entry_count` under `u32` wraparound rules.

### Submitting

1. Write an `aerogpu_submit_desc` into slot `tail % entry_count`.
2. Increment `ring->tail` by one.
3. Write any value to `AEROGPU_MMIO_REG_DOORBELL`.

The device consumes entries in order and advances `ring->head`.

### Submission descriptor

`struct aerogpu_submit_desc`, packed, 64-byte prefix:

| Offset | Type | Field | Description |
|---:|---|---|---|
| `0x00` | `u32` | `desc_size_bytes` | ≥ 64; ≤ `ring.entry_stride_bytes` |
| `0x04` | `u32` | `flags` | `enum aerogpu_submit_flags` |
| `0x08` | `u32` | `context_id` | Driver-defined; 0 means default/unknown |
| `0x0C` | `u32` | `engine_id` | `enum aerogpu_engine_id`; only `AEROGPU_ENGINE_0` |
| `0x10` | `u64` | `cmd_gpa` | Command buffer GPA |
| `0x18` | `u32` | `cmd_size_bytes` | Command buffer size |
| `0x1C` | `u32` | `cmd_reserved0` | Zero |
| `0x20` | `u64` | `alloc_table_gpa` | Allocation table GPA; 0 when absent |
| `0x28` | `u32` | `alloc_table_size_bytes` | Allocation table size; 0 when absent |
| `0x2C` | `u32` | `alloc_table_reserved0` | Zero |
| `0x30` | `u64` | `signal_fence` | Fence value to signal on completion |
| `0x38` | `u64` | `reserved0` | Zero |

Flags:

- `AEROGPU_SUBMIT_FLAG_PRESENT` (bit 0) — the submission contains a present; a
  scheduling and pacing hint.
- `AEROGPU_SUBMIT_FLAG_NO_IRQ` (bit 1) — suppress the fence IRQ for this
  submission.

Validation, enforced by every consumer:

- `cmd_gpa` and `cmd_size_bytes` are both zero (an empty submission) or both
  non-zero.
- When non-zero, `cmd_gpa + cmd_size_bytes` must not overflow `u64`.
- `alloc_table_gpa` and `alloc_table_size_bytes` are both zero (absent) or both
  non-zero (present).
- When non-zero, `alloc_table_gpa + alloc_table_size_bytes` must not overflow
  `u64`.

## Allocation table and `backing_alloc_id`

### Why the table exists

Command packets reference guest-backed memory through a compact
`backing_alloc_id`. A guest physical address cannot stand in for that reference,
because WDDM is free to move an allocation between submissions. Every submission
that might require guest-memory access therefore carries its own table mapping
`alloc_id → (gpa, size_bytes, flags)`, and the host resolves through that table
every time.

The decisive design choice is that **`alloc_id` is a stable per-allocation
identifier, not an index into the current submission's table.** The table's
ordering is not stable across submissions; had the ID been positional, a resource
created in one submission could silently bind to different memory in a later one
unless the UMD re-emitted `CREATE_*` every frame. That would be fragile in
exactly the multi-frame, many-surface case — DWM composition — that matters most.

### Identifier namespaces

Defined in `aerogpu_wddm_alloc.h`:

| Range | Owner | Rules |
|---|---|---|
| `0` | — | Reserved and invalid; in a `backing_alloc_id` it means "no guest backing" (host-allocated resource) |
| `1 .. 0x7fffffff` | UMD (`AEROGPU_WDDM_ALLOC_ID_UMD_MAX`) | Stable for the allocation's lifetime; collision-resistant across processes |
| `0x80000000 .. 0xffffffff` | KMD (`AEROGPU_WDDM_ALLOC_ID_KMD_MIN`) | Runtime/kernel allocations created without an AeroGPU private-data blob |

`alloc_id` is explicitly a `u32`. A 64-bit kernel pointer or OS handle must never
be truncated into one; a driver that needs the association keeps its own
`{handle64 → alloc_id}` map and allocates IDs from a monotonic counter.

Because DWM may compose redirected surfaces from many processes in a single
submission, shared allocations need IDs that do not collide across processes. A
workable implementation draws from a cross-process monotonic counter — named
shared memory plus an atomic increment — masked into the UMD range and skipping
zero. The ID is embedded in the preserved WDDM allocation private-data blob so
another process recovers the same value on `OpenResource`.

### Aliasing and collisions

Windows may hand out several WDDM allocation handles for the same underlying
allocation, typically one from `CreateAllocation` and another from
`OpenAllocation` for a shared resource. All such aliases **must carry the same
`alloc_id`**; host lookup is keyed by `alloc_id`, never by handle value.

Within a submission the table is a map, so:

- The KMD deduplicates identical aliases.
- If one `alloc_id` would map to different `(gpa, size)` values in a single
  submission, that is a collision and the submission fails deterministically —
  the Win7 KMD returns `STATUS_INVALID_PARAMETER`.
- If the host sees a malformed or ambiguous table, including duplicate
  `alloc_id`s, it fails the submission but **still advances the fence**, because
  a guest waiting on that fence would otherwise deadlock.

### Table format

```
alloc_table_gpa:
  struct aerogpu_alloc_table_header      /* magic "ALOC" */
  struct aerogpu_alloc_entry entries[]
```

```c
struct aerogpu_alloc_entry {
  uint32_t alloc_id;    /* 0 is invalid */
  uint32_t flags;       /* AEROGPU_ALLOC_FLAG_* */
  uint64_t gpa;
  uint64_t size_bytes;
  uint64_t reserved0;   /* must be 0 */
};
```

Header rules: `magic == AEROGPU_ALLOC_TABLE_MAGIC`; major ABI must match and
minor may be newer; `size_bytes >= sizeof(header)` and `<=
alloc_table_size_bytes`; `entry_stride_bytes >= sizeof(aerogpu_alloc_entry)`; and
`entry_count * entry_stride_bytes` must fit inside `size_bytes`.

Entry rules: `alloc_id != 0`; `size_bytes != 0`; `gpa + size_bytes` must not
overflow `u64`; `alloc_id` unique within the table. Note that `gpa` **may** be
zero — physical address zero is legitimate backing, and only `alloc_table_gpa`
uses zero as an absence sentinel.

### Which packets require resolution

A packet requires `alloc_id` resolution — and therefore requires both that the
table is present and that the ID appears in it — when it either carries a
`backing_alloc_id` directly or operates on a guest-backed resource in a way that
needs host access to guest memory:

- `CREATE_BUFFER`, `CREATE_TEXTURE2D` — carry `backing_alloc_id`.
- `RESOURCE_DIRTY_RANGE` — the common Map/Unmap upload notification.
- `COPY_BUFFER` / `COPY_TEXTURE2D` with `WRITEBACK_DST` — staging readback.

This has a guest-side consequence that is easy to miss. On Win7 the KMD builds
the table from the submission's `DXGK_ALLOCATIONLIST`, so only allocations
present in that list can contribute entries. "Currently bound" is not sufficient:
`RESOURCE_DIRTY_RANGE` and `COPY_* WRITEBACK_DST` can be emitted while the
resource is unbound and still require the allocation to be listed for that
submission. A missing entry is a validation error.

### Backing interpretation

**`CREATE_BUFFER`.** With `backing_alloc_id == 0` the buffer is host-allocated.
Otherwise the backing range is

```
base_gpa     = alloc_table[backing_alloc_id].gpa
buffer_bytes = [base_gpa + backing_offset_bytes,
                base_gpa + backing_offset_bytes + size_bytes)
```

and the host validates `backing_offset_bytes + size_bytes <= alloc.size_bytes`.

**`CREATE_TEXTURE2D`.** A guest-backed texture is a *linear packed subresource
chain*. Subresources use D3D11 ordering, `subresource = mip + array_layer *
mip_levels`, and are packed in increasing `(array_layer, mip)` order with **no
padding or alignment between subresources**:

```
for array_layer in 0..array_layers:
    for mip in 0..mip_levels:
        write subresource(mip, array_layer)
```

`row_pitch_bytes` in the packet describes **mip 0 only**. Every mip beyond the
first uses a tight pitch:

```
mip_width  = max(1, width  >> mip)
mip_height = max(1, height >> mip)
row_pitch_bytes(mip > 0) = min_row_pitch_bytes(format, mip_width)
```

For block-compressed formats the pitch is measured in 4×4 block rows:

```
blocks_w        = ceil(mip_width / 4)
row_pitch_bytes = blocks_w * bytes_per_block
rows_in_layout  = ceil(mip_height / 4)
```

and for linear RGBA/BGRA formats:

```
row_pitch_bytes = mip_width * bytes_per_pixel
rows_in_layout  = mip_height
```

Each subresource occupies `row_pitch_bytes * rows_in_layout` bytes, and
`(mip=0, array_layer=0)` begins at `base_gpa + backing_offset_bytes`. The host
validates that `row_pitch_bytes` is non-zero, that it is at least the mip-0
minimum for the format and width, and that
`backing_offset_bytes + total_packed_size_bytes <= alloc.size_bytes`.

Win7 shared-surface interop assumes `mip_levels == 1` and `array_layers == 1`.

### `CREATE_*` against an existing handle

Re-issuing `CREATE_BUFFER` or `CREATE_TEXTURE2D` for a handle that already exists
is a **rebind of the backing memory**, and only when every immutable property
matches:

- Buffers: `size_bytes`, `usage_flags`.
- Textures: `format`, `width`, `height`, `mip_levels`, `array_layers`,
  `row_pitch_bytes`, `usage_flags`.

If any immutable property differs the host raises a validation error; the guest
is expected to `DESTROY_RESOURCE` and create a fresh handle instead.

### Write intent and `READONLY`

`aerogpu_alloc_entry.flags` carries `AEROGPU_ALLOC_FLAG_READONLY`, which means the
host must never write that allocation's guest backing. Any command requesting a
writeback into a READONLY allocation is rejected — explicit flags such as
`COPY_* WRITEBACK_DST` and any implicit writeback path alike.

On Win7 the KMD derives the flag from the submission's allocation-list write
intent (`DXGK_ALLOCATIONLIST::Flags.Value` bit 0, `WriteOperation`):

| `WriteOperation` | Entry flags |
|---|---|
| `0` | `AEROGPU_ALLOC_FLAG_READONLY` |
| `1` | `0` (writable) |

This is defence in depth: the host executor can reject any command stream that
asks to write memory the WDDM submission metadata never declared writable.

## Fence and completion model

Fences are monotonic 64-bit values chosen by the guest. Each submission supplies
`signal_fence`; once the submission finishes, the device raises the completed
fence to at least that value. Completion is observable through
`AEROGPU_MMIO_REG_COMPLETED_FENCE_LO/HI`, which is always available, and
optionally through a shared fence page.

If a submission writes anything back into guest backing memory, the device must
complete those writebacks and make them visible **before** advancing the fence.
Otherwise a guest that wakes on the fence can read stale data.

With interrupts enabled, the device raises `AEROGPU_IRQ_FENCE` when the completed
fence advances, unless the submission set `AEROGPU_SUBMIT_FLAG_NO_IRQ`.

### Optional fence page

With `AEROGPU_FEATURE_FENCE_PAGE`, the driver may program `FENCE_GPA_LO/HI` with
the address of a guest page holding `struct aerogpu_fence_page` (packed; 56 bytes
used, mapped as a full 4 KiB page):

| Offset | Type | Field | Notes |
|---:|---|---|---|
| `0x00` | `u32` | `magic` | `AEROGPU_FENCE_PAGE_MAGIC` = `0x434E4546` (`"FENC"` LE) |
| `0x04` | `u32` | `abi_version` | `AEROGPU_ABI_VERSION_U32` |
| `0x08` | `volatile u64` | `completed_fence` | Mirrors the MMIO completed fence |
| `0x10` | `u64[5]` | `reserved0` | Zero |

### Windows 7's 32-bit fence, and duplicates

Windows 7 WDDM 1.1 gives the miniport a **32-bit** `SubmissionFenceId`
(`DXGKARG_SUBMITCOMMAND`), while the AeroGPU ring requires monotonic **64-bit**
fences. The KMD bridges the two by extending the 32-bit value into a 64-bit
domain — tracking an epoch and composing `(epoch << 32) | fence32` across
wraparound — and using that as `signal_fence`. When notifying dxgkrnl of DMA
completion or fault it truncates back to the low 32 bits, preserving the Win7 ABI.

Guests also legitimately emit **duplicate** fence values. Internal KMD
submissions reuse the most recent fence and set `AEROGPU_SUBMIT_FLAG_NO_IRQ`;
`RELEASE_SHARED_SURFACE` is emitted this way. Hosts must therefore tolerate
duplicates without losing metadata:

- A fence still raises `IRQ_FENCE` if *any* submission carrying it requested an
  IRQ.
- Present and writeback metadata attached to that fence must be preserved.
- A duplicate may arrive with `signal_fence <= completed_fence` — reusing the
  most recent fence while the GPU is idle. Such a submission carries real side
  effects and **must still execute**, even though it advances nothing.

## Vblank and present pacing

Windows 7's display stack expects periodic vblank events, and DWM's scheduling
depends on them. AeroGPU is virtual, so the device model generates a fixed-rate
tick — 60 Hz by default — from its host timer.

Rules for the vblank block, which the device must implement in full whenever
`AEROGPU_FEATURE_VBLANK` is advertised:

- Vblank counters and timestamps advance every tick, independently of whether the
  IRQ is enabled.
- The `IRQ_STATUS` bit is set once per tick **only while the cause is enabled in
  `IRQ_ENABLE`**. Latching status while masked would fire a stale interrupt the
  moment the driver re-enables the cause, breaking
  `D3DKMTWaitForVerticalBlankEvent` pacing.
- Coalescing is permitted: if the bit is already pending the device may leave it
  set while counters keep advancing.
- Tick generation is gated on `SCANOUT0_ENABLE`. When scanout is disabled the
  device stops scheduling vblanks and clears any pending vblank status bit, which
  matches the Windows expectation that vblank waits are only live while a VidPN
  source is visible.

`vblank_seq` starts at 0 or 1, increments by one per tick, and never moves
backwards. `last_vblank_time_ns` is monotonic nanoseconds since a device-chosen
boot epoch, updated each tick, and likewise never moves backwards.

Extending this to multiple VidPN sources is mechanical: replicate the
`SCANOUTn_VBLANK_*` register set per scanout and either allocate additional IRQ
cause bits or require the driver to disambiguate by reading per-scanout sequence
counters. The former is preferred.

## Command stream framing

The command buffer referenced by `cmd_gpa`/`cmd_size_bytes` is an AeroGPU command
stream: one `aerogpu_cmd_stream_header` followed by a sequence of packets, each
introduced by an `aerogpu_cmd_hdr`. The opcode set and packet payloads are
specified in [aerogpu-command-stream.md](./aerogpu-command-stream.md); this
section covers only the framing and compatibility rules, which are part of the
transport contract.

The stream header carries `magic = AEROGPU_CMD_STREAM_MAGIC` (`0x444D4341`,
`"ACMD"` LE), `abi_version`, and `size_bytes` — the total bytes used including
the header. It must be at least `sizeof(aerogpu_cmd_stream_header)` and at most
`aerogpu_submit_desc::cmd_size_bytes`; trailing bytes in the buffer beyond
`size_bytes` are ignored, which is what makes over-allocated command buffers and
forward-compatible padding safe.

```c
struct aerogpu_cmd_hdr {
  u32 opcode;     /* enum aerogpu_cmd_opcode */
  u32 size_bytes; /* total packet size, including this header */
};
```

Consumers must validate the stream magic and that the major ABI is supported,
require `size_bytes >= sizeof(aerogpu_cmd_hdr)` and 4-byte alignment, and **skip
unknown opcodes using `size_bytes`** rather than treating them as fatal.
Producers must emit correct `size_bytes` for every packet, zero every reserved
field except where an encoding rule explicitly repurposes it, and use only
opcodes and features permitted by the negotiated ABI version and feature bits.

### Append-only packet extension

Known opcodes may also grow, by appending fields after a stable prefix. The
prefix layout never changes; `aerogpu_cmd_hdr.size_bytes` tells the reader how
much is present; readers decode what they understand and ignore the rest.

`AEROGPU_CMD_BIND_SHADERS` is the worked example. Its stable prefix is 24 bytes —
`hdr(8) + vs(4) + ps(4) + cs(4) + reserved0(4)`. When `hdr.size_bytes >= 36` the
packet appends three more `u32` handles: `gs`, `hs`, `ds`. **When the appended
handles are present they are authoritative**, and `reserved0` is ignored;
`reserved0` is interpreted as a legacy GS handle only when `hdr.size_bytes == 24`.
Writers may additionally mirror `gs` into `reserved0` as a courtesy to hosts that
only understand the 24-byte form.

Helpers that emit the extended layout directly:

| Language | Entry points |
|---|---|
| Rust | `aero_protocol::aerogpu::cmd_writer::AerogpuCmdWriter::{bind_shaders_ex, bind_shaders_ex_with_gs_mirror, bind_shaders_hs_ds}` |
| TypeScript | `AerogpuCmdWriter.bindShadersEx(vs, ps, cs, gs, hs, ds, mirrorGsToReserved0?)` — also accepts `{gs, hs, ds}` — and `AerogpuCmdWriter.bindShadersHsDs(hs, ds)` |
| C++ (UMD) | `aerogpu::{CmdStreamWriter, SpanCmdStreamWriter, VectorCmdStreamWriter}::{bind_shaders_ex, bind_shaders_hs_ds}` |

### Extended shader stage selector

The legacy `enum aerogpu_shader_stage` encodes only VS, PS, CS, and later
`GEOMETRY = 3`. To carry D3D11's hull and domain stages without breaking older
hosts, certain packets reuse their trailing `reserved0` field as an **extended
stage selector**, active only when `shader_stage == COMPUTE`. `DISPATCH` is
implicitly compute and has no `shader_stage` field, so its trailing `reserved0`
is treated as `stage_ex` under the same rules.

The extension is gated on ABI minor **≥ 3**. When decoding a stream whose header
reports a lower minor, hosts must treat `reserved0` as zero even when the stage is
compute — otherwise legacy reserved data is misread as a stage selector.

The encoding invariant, enforced by writers and hosts alike:

- `shader_stage != COMPUTE` ⟹ `reserved0` **must** be zero and is ignored.
- `shader_stage == COMPUTE` and `reserved0 == 0` ⟹ the real compute stage.
- `shader_stage == COMPUTE` and `reserved0 != 0` ⟹ `reserved0` holds a non-zero
  `enum aerogpu_shader_stage_ex`.

`aerogpu_shader_stage_ex` values align deliberately with DXBC program-type numbers
from the shader version token, but only the non-legacy stages are representable:
`2 = gs`, `3 = hs`, `4 = ds`, `5 = cs` (an optional alias — writers should encode
compute as `reserved0 = 0`). `stage_ex = 1` (vertex) is intentionally invalid;
vertex shaders use the legacy `shader_stage = VERTEX` encoding. Pixel shaders are
not representable either, because `0` is reserved for legacy compute packets.

Geometry shaders can be encoded either way. Producers should prefer the direct
form (`shader_stage = GEOMETRY`, `reserved0 = 0`); the `stage_ex` form
(`shader_stage = COMPUTE`, `reserved0 = 2`) remains valid for compatibility.
Hull and domain shaders have no direct encoding and require `stage_ex`.

## Guest-side feature discovery

The tree contains both the legacy `"ARGP"` and versioned `"AGPU"` device models,
so a UMD must not hardcode which ABI it is running against, nor assume which
optional features are present. Early in adapter open it calls
`D3DKMTQueryAdapterInfo(KMTQAITYPE_UMDRIVERPRIVATE)` and decodes
`aerogpu_umd_private_v1` from `aerogpu_umd_private.h`:

| Field | Meaning |
|---|---|
| `device_mmio_magic` | `"ARGP"` (legacy) or `"AGPU"` (versioned), via `AEROGPU_UMDPRIV_MMIO_MAGIC_*` |
| `device_abi_version_u32` | Legacy MMIO version, or `AEROGPU_ABI_VERSION_U32` |
| `device_features` | Versioned feature bitset; zero on legacy |
| `flags` | Convenience bits such as `AEROGPU_UMDPRIV_FLAG_HAS_VBLANK`, `AEROGPU_UMDPRIV_FLAG_HAS_FENCE_PAGE` |

## Shared surfaces and `share_token`

Cross-process D3D9Ex shared surfaces are keyed by a 64-bit `share_token`, used by
`EXPORT_SHARED_SURFACE` and `IMPORT_SHARED_SURFACE`.

The token must be stable across guest processes and **must not be derived from
the numeric value of a user-mode shared `HANDLE`**. For a real NT handle that
value is process-local and commonly differs in a consumer after
`DuplicateHandle`; some stacks additionally hand out token-style shared handles
that must not be treated as stable identifiers, and must not be passed to
`CloseHandle`.

The canonical mechanism: the Win7 KMD generates a stable non-zero `share_token`
and persists it in the preserved WDDM allocation private-data blob
(`aerogpu_wddm_alloc_priv.share_token`). dxgkrnl preserves that blob verbatim and
replays it on cross-process `OpenResource`, so every process observes identical
bytes.

`share_token` is globally unique, and the host detects and rejects three abuses:
exporting a token already bound to a different resource, re-exporting a token
that was released, and importing an unknown or released token. Current host
behaviour marks the submission failed — raising the error IRQ — while still
advancing the fence.

The shared-surface ABI assumes a shared surface is backed by a **single** WDDM
allocation, one contiguous guest range. Many WDDM resources can span several
allocations for mips, array slices, or planes. To keep the contract tractable and
to match what Win7 DWM redirected surfaces actually need, the driver stack rejects
shared resources that would require more than one: `mip_levels == 1` (a
`MipLevels`/`Levels` of `0`, requesting a full chain, is rejected) and
`array_layers == 1`.

Lifetime is reference-counted by the KMD, keyed by `share_token`. Windows 7's call
patterns vary — `DxgkDdiCloseAllocation` is nominally the per-open callback and
`DxgkDdiDestroyAllocation` the final one, but either may be used to release a
handle in practice. Each successful create or open wrapper increments the count;
close and destroy decrement it *only* when they actually untrack a wrapper, so
duplicated callback sequences cannot underflow. When the count reaches zero the
KMD emits `RELEASE_SHARED_SURFACE` so the host drops its `share_token → resource`
mapping. That release is a best-effort internal ring submission which must not
advance the OS-visible fence domain, which is precisely why it reuses the most
recent fence with `NO_IRQ` — and why hosts must tolerate duplicate fences.

## Host execution modes

The device model owns the guest-facing contract; *where* commands actually
execute is a host integration choice. `aero_machine::Machine` supports three
modes, and the choice is mutually exclusive between the bridge and an in-process
backend.

**Bring-up (default).** No backend, bridge disabled. Submissions are decoded but
the command stream is not executed, and fences complete so the guest makes
forward progress. This is what lets an early guest boot and enumerate the device
before any renderer exists. Vblank pacing still applies: a submission carrying a
`PRESENT` with the vsync flag has its fence paced to the next vblank tick, because
Win7 and DWM depend on that cadence regardless of whether anything was drawn.
`Machine::aerogpu_drain_submissions()` may be called purely to inspect decoded
submissions without changing fence behaviour on native hosts.

**Submission bridge (out-of-process).** Enabled via
`Machine::aerogpu_enable_submission_bridge()`, and enabled automatically by the
`crates/aero-wasm` exports so the browser contract stays explicit. The host drains
submissions, executes them elsewhere — the GPU worker — and reports completion
with `Machine::aerogpu_complete_fence(signal_fence)`.

The division of responsibility matters: the host controls *when work executes*,
but the device model retains the *guest-facing pacing contract*. Once the host
reports a fence complete, a fence belonging to a vsync `PRESENT` still completes
on the next vblank and still blocks later immediate fences until then. Hosts
should therefore report completion as soon as execution finishes and never try to
"fake vsync" by delaying the call to match host tick cadence.

**In-process backend (native/headless).** For native integration tests:

| Entry point | Behaviour |
|---|---|
| `Machine::aerogpu_set_backend(Box<dyn AeroGpuCommandBackend>)` | Install a custom backend |
| `Machine::aerogpu_set_backend_immediate()` | Complete fences synchronously, render nothing |
| `Machine::aerogpu_set_backend_null()` | Drop submissions and never complete fences — for testing fence/IRQ gating |
| `Machine::aerogpu_set_backend_wgpu()` | wgpu-backed execution; feature `aerogpu-wgpu-backend`, native only |

An installed backend survives `Machine::reset()` — it is reset, not uninstalled.

### Fence forward progress is a safety property

In the browser the coordinator will **force-complete fences** rather than let a
guest hang: when a submission cannot be delivered to the GPU worker at all —
bounded queue overflow while the worker is not ready, a `postMessage` or
transfer-list failure, or a worker restart with fences in flight. The consequence
is honest and worth stating plainly: those submissions are never rendered.
Rendering is best-effort in these failure modes; forward progress is not. The
alternative is a guest deadlock or a TDR, which is strictly worse.

### Mode contract tests

```bash
# Bring-up mode: fences complete without executing ACMD
cargo test -p aero-machine --test aerogpu_ring_noop_fence --locked
# Submission bridge: drain plus host fence completion
cargo test -p aero-machine --test aerogpu_submission_bridge --locked
# Fence gating and backend switching
cargo test -p aero-machine --test aerogpu_complete_fence_gating --locked
cargo test -p aero-machine --test aerogpu_deferred_fence_completion --locked
# Vblank pacing for vsync presents in default mode
cargo test -p aero-machine --test aerogpu_vsync_fence_pacing --locked
# In-process backends
cargo test -p aero-machine --test aerogpu_immediate_backend_completes_fence --locked
cargo test -p aero-machine --test aerogpu_wgpu_backend_smoke --locked --features aerogpu-wgpu-backend
```

## End-to-end flow

1. The UMD encodes AeroGPU IR packets into a command buffer in guest memory.
2. The KMD writes an `aerogpu_submit_desc` into the ring, builds the per-submit
   allocation table from the WDDM allocation list, and rings the doorbell.
3. The device model consumes ring entries, parses the command buffer, validates
   it against the allocation table, and translates the IR to WebGPU operations.
4. On completion the device signals the fence — MMIO always, fence page if
   configured — and raises `AEROGPU_IRQ_FENCE` unless suppressed.
5. The KMD's ISR reads the completed fence, acknowledges via `IRQ_ACK`, and
   notifies dxgkrnl so waiting UMD threads unblock and the scheduler advances.

## Debugging a raw command stream

`aero-gpu-trace-replay` decodes a captured command buffer into a stable,
grep-friendly opcode listing. The input must be the raw
`aerogpu_cmd_stream_header` followed by the packet sequence.

```bash
cargo run -p aero-gpu-trace-replay -- decode-cmd-stream <cmd-stream.bin>
# Fail on unknown opcodes; the default is forward-compatible and prints UNKNOWN
cargo run -p aero-gpu-trace-replay -- decode-cmd-stream --strict <cmd-stream.bin>
```

```
0x00000018 CreateBuffer size_bytes=40 ...
0x00000040 UploadResource size_bytes=36 ...
```

## Related pages

- [aerogpu-command-stream.md](./aerogpu-command-stream.md) — opcodes and packet payloads
- [windows7-aerogpu-wddm-driver.md](./windows7-aerogpu-wddm-driver.md) — the KMD/UMD side of this contract
- [gpu-trace-format.md](gpu-trace-format.md) — capture/replay trace container
- [../areas/graphics.md](../areas/graphics.md) — the graphics stack as a whole
- [../decisions/0005-aerogpu-pci-ids-and-abi.md](../decisions/0005-aerogpu-pci-ids-and-abi.md) — the identity decision
