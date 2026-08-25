# GPU trace format

> On-disk container for **Aero GPU command traces**. A trace records the
> guest→host GPU command stream — including shader blobs and resource uploads —
> so a graphics bug can be replayed deterministically in isolation, on another
> machine, in another browser, or in CI.
>
> Container version **2**. Any backwards-incompatible change bumps
> `container_version` in the file header.

## Two independent versions

A trace carries two version numbers that answer different questions, and
conflating them is the classic way to break a replayer.

**`container_version`** versions *this document* — the on-disk layout. Readers
stay backwards compatible with older containers and must reject an unknown newer
container deterministically rather than guessing. `RecordType::AerogpuSubmission`
is a v2+ record type and must never appear in a v1 trace.

**`command_abi_version`** versions the *recorded GPU command ABI*. For canonical
AeroGPU traces this is `AEROGPU_ABI_VERSION_U32` — `(major << 16) | minor` from
`aerogpu_pci.h` — and the AeroGPU compatibility rules apply to it: minor versions
are backwards-compatible extensions, major changes are breaking. The container
stores this value but never interprets it; validating it is the decoder's job.

## Goals and boundaries

A trace records the command stream **in submission order**, as the exact packet
bytes the guest-facing command processor emitted; the referenced resource uploads
and snapshots as bytes; shader blobs as DXBC, optionally alongside the translated
text the backend actually consumed; frame boundaries as begin-frame and present
markers; and a frame table of contents for random access.

It is deliberately not a general capture system. Traces are local files, not
streams; compression is left to an external layer or a future record flag; and
CPU state is out of scope entirely — this is a GPU-only trace.

## File layout

Every multi-byte integer is little-endian.

```
┌────────────────────────┐
│ TraceHeader (32 bytes) │
├────────────────────────┤
│ meta_json (meta_len)   │  UTF-8 JSON
├────────────────────────┤
│ record stream          │  variable
├────────────────────────┤
│ TraceToc               │  variable; located via the footer
├────────────────────────┤
│ TraceFooter (32 bytes) │
└────────────────────────┘
```

### TraceHeader

| Field | Type | Meaning |
|---|---|---|
| `magic` | `[u8; 8]` | `"AEROGPUT"` |
| `header_size` | `u32` | 32 |
| `container_version` | `u32` | 1 or 2 |
| `command_abi_version` | `u32` | Version of the recorded command ABI |
| `flags` | `u32` | Reserved, zero |
| `meta_len` | `u32` | Byte length of the UTF-8 JSON metadata blob |
| `reserved` | `u32` | Zero |

`meta_json` is implementation-defined but must carry at least
`emulator_version` (string) and `command_abi_version` (number, matching the
header):

```json
{
  "emulator_version": "0.0.0-dev",
  "command_abi_version": 65536,
  "notes": "optional"
}
```

### TraceFooter

| Field | Type | Meaning |
|---|---|---|
| `magic` | `[u8; 8]` | `"AEROGPUF"` |
| `footer_size` | `u32` | 32 |
| `container_version` | `u32` | Must match the header |
| `toc_offset` | `u64` | Absolute file offset of the `TraceToc` |
| `toc_len` | `u64` | Byte length of the `TraceToc` |

## Records

Each record is an 8-byte header followed by its payload.

| Field | Type | Meaning |
|---|---|---|
| `record_type` | `u8` | See below |
| `flags` | `u8` | Reserved, zero |
| `reserved` | `u16` | Zero |
| `payload_len` | `u32` | Payload byte length |

| Type | Name | Payload |
|---|---|---|
| `0x01` | `BeginFrame` | `u32 frame_index` |
| `0x02` | `Present` | `u32 frame_index` |
| `0x03` | `Packet` | Raw command packet bytes |
| `0x04` | `Blob` | `BlobHeader` followed by blob bytes |
| `0x05` | `AerogpuSubmission` | Structured submission referencing blobs (v2+) |

### Blob records

A blob payload opens with a 16-byte `BlobHeader` — `u64 blob_id`, `u32 kind`,
`u32 reserved` — and the blob bytes run from there to the end of the payload.
`blob_id` is unique within the trace and is referenced by other records, and by
command packets in ABIs that support blob references.

| Kind | Name | Content |
|---|---|---|
| `0x01` | `BufferData` | Exact bytes uploaded to a buffer |
| `0x02` | `TextureData` | Raw texture subresource bytes; format described by the command packets |
| `0x03` | `ShaderDxbc` | DXBC bytecode |
| `0x04` | `ShaderWgsl` | WGSL, UTF-8 |
| `0x05` | `ShaderGlslEs300` | GLSL ES 3.00, UTF-8, for the WebGL2 fallback |
| `0x100` | `AerogpuCmdStream` | Raw `aerogpu_cmd_stream_header` plus packets |
| `0x101` | `AerogpuAllocTable` | Raw `aerogpu_alloc_table_header` plus entries |
| `0x102` | `AerogpuAllocMemory` | Raw guest memory for one allocation table entry |

### AerogpuSubmission records

The v2 record that makes canonical AeroGPU traces replayable: it captures the
exact `aerogpu_cmd.h` byte stream the guest submitted, plus snapshots of whatever
guest memory the submission needed.

```
u32 record_version;      /* = 1                                    */
u32 header_size;         /* fixed header size; 56 for record v1     */
u32 submit_flags;        /* aerogpu_submit_desc.flags               */
u32 context_id;          /* aerogpu_submit_desc.context_id          */
u32 engine_id;           /* aerogpu_submit_desc.engine_id           */
u32 reserved0;           /* = 0                                     */
u64 signal_fence;        /* aerogpu_submit_desc.signal_fence        */
u64 cmd_stream_blob_id;  /* BlobKind::AerogpuCmdStream              */
u64 alloc_table_blob_id; /* BlobKind::AerogpuAllocTable; 0 if absent */
u32 memory_range_count;
u32 reserved1;           /* = 0                                     */

/* memory_range_count entries follow: */
u32 alloc_id;
u32 flags;
u64 gpa;
u64 size_bytes;
u64 blob_id;             /* BlobKind::AerogpuAllocMemory            */
```

## Table of contents

The TOC exists so a replayer can seek to a frame without scanning the record
stream. Its header is `magic` `"AEROTOC\0"`, `u32 toc_version` (1), and
`u32 frame_count`, followed by 32-byte frame entries:

| Field | Type | Meaning |
|---|---|---|
| `frame_index` | `u32` | Monotonic, `0..N-1` |
| `flags` | `u32` | Reserved, zero |
| `start_offset` | `u64` | Absolute offset of the `BeginFrame` record |
| `present_offset` | `u64` | Absolute offset of the `Present` record; 0 if absent |
| `end_offset` | `u64` | Absolute offset immediately after the frame's last record |

## What makes a trace replayable

Three rules, and each exists because violating it produces a trace that replays
only on the machine that captured it.

**Record after translation.** Packets are recorded once translated from the guest
API into the stable AeroGPU command ABI — never as guest-API calls, which would
make the replayer depend on the translator's version.

**Serialise everything the replay needs.** All resource and shader data must be
in the trace, whether inline in the command stream or as `Blob` records. A
replayer must never read guest RAM; there is none.

**Record both shader representations.** DXBC for postmortem analysis, and the
backend-consumable form the run actually used — WGSL for WebGPU, GLSL ES 3.00 for
the WebGL2 fallback.

## Canonical AeroGPU traces

For the Windows 7 WDDM path the recommended representation is a
`RecordType::AerogpuSubmission` referencing a `BlobKind::AerogpuCmdStream`, plus
`AerogpuAllocTable` and `AerogpuAllocMemory` blobs where the submission needs
them.

Some AeroGPU packets embed variable-length data directly — `UPLOAD_RESOURCE`
carries resource bytes, `CREATE_SHADER_DXBC` carries DXBC. **Those bytes stay
inline**, as part of the recorded command stream, whether that lands in a
`Packet` payload or an `AerogpuCmdStream` blob. The trace does not lift them into
separate `Blob` records and rewrite packet fields into blob IDs; the recorded
bytes are the bytes the guest submitted.

The header's `command_abi_version` must equal `AEROGPU_ABI_VERSION_U32`. The
retired legacy and prototype GPU ABIs are not supported for capture or replay.

## Reference command ABI

The tree also carries a deliberately tiny command ABI, used only to produce a
deterministic triangle fixture and replay it in CI — it validates the container
and replayer plumbing, and is not a guest driver contract.

Each `RecordType::Packet` is a sequence of little-endian `u32` dwords: dword 0 is
the opcode, dword 1 is `total_dwords` including the two-dword header, and the
rest is opcode-specific. Blob IDs are `u64` split into `(lo, hi)`.

| Opcode | Name | Payload dwords |
|---:|---|---|
| `0x0001` | `CREATE_BUFFER` | `buffer_id`, `size_bytes`, `usage` |
| `0x0002` | `UPLOAD_BUFFER` | `buffer_id`, `offset_bytes`, `data_len_bytes`, `blob_id_lo`, `blob_id_hi` |
| `0x0003` | `CREATE_SHADER` | `shader_id`, `stage` (0 = VS, 1 = FS), `glsl_blob_id_lo/hi`, `wgsl_blob_id_lo/hi`, `dxbc_blob_id_lo/hi` |
| `0x0004` | `CREATE_PIPELINE` | `pipeline_id`, `vs_shader_id`, `fs_shader_id` |
| `0x0005` | `SET_PIPELINE` | `pipeline_id` |
| `0x0006` | `SET_VERTEX_BUFFER` | `buffer_id`, `stride_bytes`, `position_offset_bytes`, `color_offset_bytes` |
| `0x0007` | `SET_VIEWPORT` | `width_px`, `height_px` |
| `0x0008` | `CLEAR` | `r_f32_bits`, `g_f32_bits`, `b_f32_bits`, `a_f32_bits` |
| `0x0009` | `DRAW` | `vertex_count`, `first_vertex` |
| `0x000A` | `PRESENT` | — |

## Related pages

- [aerogpu-device-abi.md](./aerogpu-device-abi.md) — the recorded transport and ABI versioning
- [aerogpu-command-stream.md](./aerogpu-command-stream.md) — the recorded packets
- [../areas/graphics.md](../areas/graphics.md) — capture and replay tooling in context
