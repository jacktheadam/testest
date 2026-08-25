# AeroGPU command stream

> The packet language ("AeroGPU IR") that a submission carries: the opcode set,
> what each family does, and the encoding rules that let the stream evolve
> without breaking older readers.
>
> `drivers/aerogpu/protocol/aerogpu_cmd.h` is the source of truth for every
> packet payload. This page is the map of that header — which opcodes exist,
> what they mean, and the rules that govern all of them. Where the two
> disagree, the header wins.
>
> The transport that delivers these streams — ring, submit descriptor,
> allocation table, fences — is
> [aerogpu-device-abi.md](./aerogpu-device-abi.md), which also specifies the
> framing, forward-compatibility, and stage-selector rules that apply here.

## Shape of a stream

A command buffer is one `aerogpu_cmd_stream_header` — magic `"ACMD"`, ABI
version, and a `size_bytes` covering the header and every packet — followed by
packets. Each packet opens with an `aerogpu_cmd_hdr` carrying its opcode and its
own `size_bytes`, which includes the header, must be at least 8 bytes, and must
be 4-byte aligned.

Those two length fields carry the whole forward-compatibility story. A reader
skips an opcode it does not recognise by advancing `size_bytes`, and ignores
trailing bytes in a packet it *does* recognise but whose newer fields it does not
understand. A writer emits accurate lengths, zeroes reserved fields, and uses
only what the negotiated ABI version and feature bits allow.

## Object model

Packets refer to objects by `aerogpu_handle_t` — a guest-chosen protocol handle
such as `resource_handle`, `buffer_handle`, or `texture_handle`. They refer to
*guest memory* separately, through `backing_alloc_id`, resolved per submission
through the allocation table.

Keeping those two namespaces apart is what removes relocation from the design.
A traditional WDDM driver patches guest-physical addresses into its command
buffer through a patch-location list; AeroGPU never embeds an address, so the
patch list is always empty and the kernel-mode driver has no relocation logic to
get wrong. The full contract for identifier stability, aliasing, collisions, and
write intent is in
[aerogpu-device-abi.md](./aerogpu-device-abi.md#allocation-table-and-backing_alloc_id).

## Opcode families

Opcodes are grouped by leading nibble so a decode listing sorts meaningfully.

### Stream control

| Opcode | Name | Purpose |
|---:|---|---|
| `0x000` | `AEROGPU_CMD_NOP` | Padding; no effect |
| `0x001` | `AEROGPU_CMD_DEBUG_MARKER` | UTF-8 label for capture and trace tooling |

### Resources

| Opcode | Name | Purpose |
|---:|---|---|
| `0x100` | `AEROGPU_CMD_CREATE_BUFFER` | Create or rebind a buffer, optionally guest-backed |
| `0x101` | `AEROGPU_CMD_CREATE_TEXTURE2D` | Create or rebind a 2D texture; cube maps use `array_layers = 6` |
| `0x102` | `AEROGPU_CMD_DESTROY_RESOURCE` | Release a resource handle |
| `0x103` | `AEROGPU_CMD_RESOURCE_DIRTY_RANGE` | Notify that the CPU wrote a range of guest-backed memory |
| `0x104` | `AEROGPU_CMD_UPLOAD_RESOURCE` | Upload bytes inline in the stream, for host-owned resources |
| `0x105` | `AEROGPU_CMD_COPY_BUFFER` | Buffer-to-buffer copy, optionally writing back to guest memory |
| `0x106` | `AEROGPU_CMD_COPY_TEXTURE2D` | Texture copy, optionally writing back to guest memory |
| `0x107` | `AEROGPU_CMD_CREATE_TEXTURE_VIEW` | Create a typed view over a texture |
| `0x108` | `AEROGPU_CMD_DESTROY_TEXTURE_VIEW` | Release a view handle |

Two rules separate the upload paths, and mixing them up is a common bug.
`RESOURCE_DIRTY_RANGE` is *only* meaningful for a guest-backed resource
(`backing_alloc_id != 0`): it tells the host that memory it can already read has
changed. A host-owned resource has no guest memory to read, so its contents must
arrive explicitly via `UPLOAD_RESOURCE`.

Writeback is the mirror image. A `COPY_*` with `WRITEBACK_DST` asks the host to
write *into* guest memory, which requires the destination allocation to be
present in the submission's allocation table and not marked read-only — and
requires the device to complete the write before advancing the fence.

### Shaders and input layout

| Opcode | Name | Purpose |
|---:|---|---|
| `0x200` | `AEROGPU_CMD_CREATE_SHADER_DXBC` | Upload a DXBC blob and create a shader for a stage |
| `0x201` | `AEROGPU_CMD_DESTROY_SHADER` | Release a shader handle |
| `0x202` | `AEROGPU_CMD_BIND_SHADERS` | Bind the pipeline's stages |
| `0x203` | `AEROGPU_CMD_SET_SHADER_CONSTANTS_F` | Float constant registers |
| `0x204` | `AEROGPU_CMD_CREATE_INPUT_LAYOUT` | Create a vertex input layout |
| `0x205` | `AEROGPU_CMD_DESTROY_INPUT_LAYOUT` | Release an input layout |
| `0x206` | `AEROGPU_CMD_SET_INPUT_LAYOUT` | Bind an input layout |
| `0x207` | `AEROGPU_CMD_SET_SHADER_CONSTANTS_I` | Integer constant registers |
| `0x208` | `AEROGPU_CMD_SET_SHADER_CONSTANTS_B` | Boolean constant registers |

`BIND_SHADERS` is the worked example of the append-only extension rule: a stable
24-byte prefix carrying vertex, pixel, and compute handles, extended in newer
streams with geometry, hull, and domain handles when `hdr.size_bytes >= 36`. The
appended handles are authoritative when present; the legacy `reserved0` field is
read as a geometry handle only when the packet is exactly 24 bytes.

The integer and boolean constant banks exist because Direct3D 9 shader models
have register files that Direct3D 10 and later folded into constant buffers.
Translated shaders address all three banks through one uniform block, with the
boolean bank packed four registers per element to satisfy WGSL's uniform layout
rules while staying compact.

### Pipeline state

| Opcode | Name | Purpose |
|---:|---|---|
| `0x300` | `AEROGPU_CMD_SET_BLEND_STATE` | Blend state |
| `0x301` | `AEROGPU_CMD_SET_DEPTH_STENCIL_STATE` | Depth and stencil state |
| `0x302` | `AEROGPU_CMD_SET_RASTERIZER_STATE` | Rasteriser state |

### Render targets and viewport

| Opcode | Name | Purpose |
|---:|---|---|
| `0x400` | `AEROGPU_CMD_SET_RENDER_TARGETS` | Bind colour targets and depth-stencil |
| `0x401` | `AEROGPU_CMD_SET_VIEWPORT` | Viewport rectangle and depth range |
| `0x402` | `AEROGPU_CMD_SET_SCISSOR` | Scissor rectangle |

### Input assembler

| Opcode | Name | Purpose |
|---:|---|---|
| `0x500` | `AEROGPU_CMD_SET_VERTEX_BUFFERS` | Bind vertex buffer slots |
| `0x501` | `AEROGPU_CMD_SET_INDEX_BUFFER` | Bind the index buffer |
| `0x502` | `AEROGPU_CMD_SET_PRIMITIVE_TOPOLOGY` | Topology, including adjacency and patch lists |

### Bindings

| Opcode | Name | Purpose |
|---:|---|---|
| `0x510` | `AEROGPU_CMD_SET_TEXTURE` | Bind a texture to a stage slot |
| `0x511` | `AEROGPU_CMD_SET_SAMPLER_STATE` | Direct3D 9-style inline sampler state |
| `0x512` | `AEROGPU_CMD_SET_RENDER_STATE` | Direct3D 9-style render state |
| `0x520` | `AEROGPU_CMD_CREATE_SAMPLER` | Create a sampler object |
| `0x521` | `AEROGPU_CMD_DESTROY_SAMPLER` | Release a sampler |
| `0x522` | `AEROGPU_CMD_SET_SAMPLERS` | Bind sampler objects to stage slots |
| `0x523` | `AEROGPU_CMD_SET_CONSTANT_BUFFERS` | Bind constant buffers to stage slots |
| `0x524` | `AEROGPU_CMD_SET_SHADER_RESOURCE_BUFFERS` | Bind shader-resource buffers |
| `0x525` | `AEROGPU_CMD_SET_UNORDERED_ACCESS_BUFFERS` | Bind unordered-access buffers |

The binding packets carry a stage selector, which is where the extended-stage
encoding matters: hull and domain stages have no legacy stage value and must be
addressed through `stage_ex`, while geometry is addressable either way and should
prefer the direct encoding. The gating rules are in
[aerogpu-device-abi.md](./aerogpu-device-abi.md#extended-shader-stage-selector).

### Work submission

| Opcode | Name | Purpose |
|---:|---|---|
| `0x600` | `AEROGPU_CMD_CLEAR` | Clear render targets or depth-stencil |
| `0x601` | `AEROGPU_CMD_DRAW` | Non-indexed draw |
| `0x602` | `AEROGPU_CMD_DRAW_INDEXED` | Indexed draw |
| `0x603` | `AEROGPU_CMD_DISPATCH` | Compute dispatch |

`DISPATCH` is implicitly compute and has no stage field, so its trailing
`reserved0` doubles as the extended-stage selector under the same ABI-minor
gating — this is how compute passes emulating geometry, hull, and domain stages
address their own binding tables.

### Presentation

| Opcode | Name | Purpose |
|---:|---|---|
| `0x700` | `AEROGPU_CMD_PRESENT` | Present the current scanout configuration |
| `0x701` | `AEROGPU_CMD_PRESENT_EX` | Present carrying Direct3D 9Ex flags |

A present reads the scanout registers as currently programmed rather than
carrying its own framebuffer address, so a driver implements a flip by updating
`SCANOUT0_FB_GPA_*` before presenting. When the submission is vsynced, the fence
is paced to the next vblank; that pacing belongs to the device model and holds
in every host execution mode.

### Shared surfaces

| Opcode | Name | Purpose |
|---:|---|---|
| `0x710` | `AEROGPU_CMD_EXPORT_SHARED_SURFACE` | Publish a resource under a `share_token` |
| `0x711` | `AEROGPU_CMD_IMPORT_SHARED_SURFACE` | Resolve a `share_token` to a resource |
| `0x712` | `AEROGPU_CMD_RELEASE_SHARED_SURFACE` | Retire a `share_token` on final close |

These three carry the cross-process composition path: the desktop compositor
consumes surfaces produced by other processes, across Direct3D versions. The
token must be stable across processes and must never be derived from a user-mode
`HANDLE` value; the reasoning, the uniqueness policy, and the reference-counted
lifetime are in
[aerogpu-device-abi.md](./aerogpu-device-abi.md#shared-surfaces-and-share_token).

`RELEASE_SHARED_SURFACE` is the reason hosts must tolerate duplicate fence
values: it is emitted as a best-effort internal submission that reuses the most
recent fence with the no-IRQ flag, precisely so it cannot disturb the
operating-system-visible fence domain.

### Scheduling

| Opcode | Name | Purpose |
|---:|---|---|
| `0x720` | `AEROGPU_CMD_FLUSH` | Explicit scheduling point |

## Reading a captured stream

```bash
cargo run -p aero-gpu-trace-replay -- decode-cmd-stream <cmd-stream.bin>
cargo run -p aero-gpu-trace-replay -- decode-cmd-stream --strict <cmd-stream.bin>
```

The default is forward-compatible and prints `UNKNOWN` for opcodes it does not
recognise; `--strict` fails on them instead, which is what a conformance run
wants. Output is one packet per line, stable and grep-friendly.

## Where this is implemented

| Role | Location |
|---|---|
| Normative header | `drivers/aerogpu/protocol/aerogpu_cmd.h` |
| Rust and TypeScript mirrors | `crates/aero-protocol/aerogpu/` (crate `aero-protocol`) |
| Host parser | `crates/aero-gpu/src/protocol.rs` |
| Host state machine and validation | `crates/aero-gpu/src/command_processor.rs` |
| Direct3D 9 executor | `crates/aero-gpu/src/aerogpu_d3d9_executor.rs` |
| Direct3D 10/11 executor | `crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs` |
| Browser executor | `apps/web/src/workers/aerogpu-acmd-executor.ts` |
| Writers | `aero_protocol::aerogpu::cmd_writer`, the TypeScript `AerogpuCmdWriter`, and the C++ `aerogpu::*CmdStreamWriter` family |

## Related pages

- [aerogpu-device-abi.md](./aerogpu-device-abi.md) — transport, framing, allocation table, fences
- [direct3d-10-11-translation.md](./direct3d-10-11-translation.md) — how Direct3D concepts become these packets
- [direct3d-9ex-and-dwm.md](./direct3d-9ex-and-dwm.md) — the present and shared-surface path the compositor depends on
- [gpu-trace-format.md](gpu-trace-format.md) — capturing and replaying these streams
