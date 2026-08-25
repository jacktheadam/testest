# Compute-expansion emulation (geometry and tessellation)

> WebGPU has no geometry shader stage, no hull or domain stage, and no
> transform feedback. Direct3D 10/11 pipelines that use them are therefore
> emulated by expanding primitives in compute passes and drawing the expanded
> buffers indirectly.
>
> This page specifies both emulations, which share a pipeline shape, a binding
> model, and a scratch-buffer discipline: geometry-shader expansion first, then
> tessellation.

WebGPU does **not** expose a geometry shader (GS) stage. Aero’s strategy is to **emulate GS via compute**
by expanding primitives into intermediate buffers, then drawing those buffers with a normal WebGPU render
pipeline.

This document describes:

- what is **implemented today** (command-stream plumbing, binding model, compute-expansion/compute-prepass scaffolding + current limitations; plus a minimal SM4 GS DXBC→WGSL compute path that is executed for a small set of IA input topologies (`PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj`) via the translated GS prepass), and
- the **next steps** (broaden VS-as-compute feeding for GS inputs (currently **very** minimal: `mov`/`add` only + incomplete draw-instancing coverage), then grow opcode/system-value/resource-binding coverage and bring up HS/DS emulation).

> Related: [`direct3d-10-11-translation.md`](direct3d-10-11-translation.md) (high-level D3D10/11→WebGPU mapping).
>
> Related: [`compute-expansion-emulation.md`](compute-expansion-emulation.md) (HS/DS emulation pipeline).

---

## Why emulation is required

Direct3D 10/11 pipelines can contain:

```
IA -> VS -> GS -> Rasterizer -> PS -> OM
```

WebGPU render pipelines only have:

```
Vertex -> Fragment -> OM
```

There is no GS equivalent, and WebGPU does not provide transform-feedback/stream-out to capture vertex
shader outputs as a buffer for later stages. As a result, when a GS is active Aero must:

1. **Run the VS as compute** (so we can explicitly write outputs to a storage buffer).
2. **Run the GS as compute** (reading VS outputs and writing expanded primitives).
3. **Render** the expanded primitives using a small passthrough vertex shader + the original pixel shader.

---

## Emulation pipeline (compute expansion)

### High-level flow

When a geometry shader is active, a draw is executed as:

1. **Vertex pulling + VS-as-compute**
   - Read IA vertex buffers and index buffers manually (“vertex pulling”).
   - Execute the D3D VS logic in a compute entry point.
   - Write `VsOut` structs into an intermediate storage buffer (one per input vertex invocation).

2. **GS-as-compute**
   - Assemble input primitives (point/line/triangle) from the post-VS data.
   - Execute the D3D GS logic as compute, using `EmitVertex` / `CutVertex` semantics.
   - Write expanded vertices into an output vertex buffer (storage).
   - Some prepasses also write an expanded index buffer (indexed list form), which the executor may
     expand into a dense vertex stream before rendering; the current synthetic-expansion fallback
     prepass only emits non-indexed vertices and uses `draw_indirect`.
   - Write an **indirect draw args** struct so the subsequent render pass can draw without CPU readback.

3. **Render expanded geometry**
   - Bind a render pipeline consisting of:
     - a small **passthrough VS** that reads the expanded vertex buffer, and
     - the original D3D pixel shader (translated to WGSL fragment).
   - Issue `draw_indirect` using the args buffer produced by step (2).
     - If the compute prepass produced an expanded index buffer, the executor first expands the
       indexed output into a dense (non-indexed) vertex stream so the render pass can stay
       non-indexed. This avoids relying on `draw_indexed_indirect` on downlevel backends.

Current status:

- The executor routes draws through a compute prepass when GS/HS/DS stages are bound (or when D3D11-only
  topologies like adjacency/patchlists are used).
- Patchlist draws without HS/DS and many GS cases still use built-in WGSL (“synthetic expansion”) to
   generate expanded geometry.
- Patchlist draws with HS+DS bound route through the tessellation prepass pipeline (VS-as-compute +
  HS/DS passthrough + tessellator layout + DS passthrough).
- There is an initial “real GS” path for **a small set of input-assembler (IA) topologies**
  (`PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj`) for both `Draw` and
  `DrawIndexed`:
  if the bound GS DXBC can be translated by `crates/aero-d3d11/src/runtime/gs_translate.rs`, the executor
  executes that translated WGSL compute prepass at draw time.
  - Today, GS `v#[]` inputs are populated via vertex pulling:
    - Translated-GS prepass paths prefer feeding `v#[]` from **VS outputs**
      via vertex pulling plus a minimal **VS-as-compute** implementation (**currently `mov`/`add` only**). If
      VS-as-compute translation fails, draws fail unless the VS is a strict passthrough (or
      `AERO_D3D11_ALLOW_INCORRECT_GS_INPUTS=1` is set to force IA-fill for debugging; may misrender
      because the GS observes pre-VS IA values).
  - This requires an input layout.

### Why we expand strips into lists

D3D GS outputs are typically declared as `line_strip` or `triangle_strip`, and can use `CutVertex`
to terminate the current strip and start a new one.

For simplicity and portability, Aero’s emulation expands strips into lists:

- `line_strip` → **line list** (each new vertex after the first emits one line segment)
- `triangle_strip` → **triangle list** (each new vertex after the first two emits one triangle)

This avoids needing to generate restart indices and keeps the draw stage in the most widely-supported
primitive topologies.

Note: there is an in-tree GS→WGSL compute translator at `crates/aero-d3d11/src/runtime/gs_translate.rs`.
It supports `pointlist`, `linestrip`, and `trianglestrip` GS output topologies. Strip topologies are
lowered to indexed list topologies (`linestrip` → **line list**, `trianglestrip` → **triangle list**)
suitable for indexed list drawing.
The executor currently converts indexed prepass outputs into a non-indexed vertex stream before
rendering so it can always use `draw_indirect` (avoiding `draw_indexed_indirect` on downlevel
backends).
It is partially wired into the command executor via the translated-GS prepass paths for `PointList`,
`LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj`; other cases still fall back to
synthetic expansion.

---

## Current implementation status (AeroGPU command-stream executor)

The AeroGPU D3D10/11 command-stream executor implements GS emulation as a GPU-side **compute expansion
prepass** + **indirect draw** path. It also has an initial “execute guest GS DXBC” path for a small
subset of IA input topologies (`PointList`, `LineList`, `TriangleList`, `LineListAdj`, and
`TriangleListAdj`), but it is not yet a complete GS implementation.

There are currently three compute-prepass “modes”:

- **Tessellation emulation (bring-up):** for patchlist draws with HS+DS bound, run a multi-pass
  tessellation prepass (VS-as-compute vertex pulling + HS passthrough + tessellator layout + DS
  passthrough) to expand patches into an indexed triangle list.
- **Real GS execution (supported subset):** translate the guest’s GS DXBC into a WGSL compute shader
  and run it to generate expanded vertices/indices + indirect args.
- **Fallback synthetic expansion (scaffolding):** run `GEOMETRY_PREPASS_CS_WGSL`, which emits synthetic primitives. This
  mode remains useful for adjacency/patchlist scaffolding and for tests that force the compute-prepass
  path without a real GS.

### Synthetic expansion vs translated GS prepass (current behavior)

The executor currently uses **two distinct** compute prepass implementations for GS-like expansion:

#### Built-in synthetic-expansion prepass (`GEOMETRY_PREPASS_CS_WGSL`)

- **What it does:** emits deterministic synthetic primitives (triangles) and writes indirect draw args.
- **What it does *not* do:** it does *not* execute any guest GS DXBC.
- **Dispatch shape:** `dispatch_workgroups(primitive_count, gs_instance_count, 1)`.
  - The WGSL treats:
    - `global_invocation_id.x` as a synthetic `SV_PrimitiveID`, and
    - `global_invocation_id.y` as a synthetic `SV_GSInstanceID` (used by GS instancing tests).
- **Bindings:**
  - `@group(0)` contains prepass IO:
    - expanded vertices (`out_vertices`),
    - packed indirect+counter state (`out_state`, sized by `GEOMETRY_PREPASS_PACKED_STATE_SIZE_BYTES`), and
    - small uniform parameters.
  - `@group(3)` provides the GS stage resource table (`cb#/t#/s#/u#`) and (optionally) internal IA vertex pulling
    bindings.
- **Used for:**
  - adjacency/patchlist scaffolding (when no translated-GS prepass is selected),
  - tessellation bring-up before real HS/DS execution exists, and
  - tests that validate compute-prepass+indirect plumbing without requiring a translated GS (many such
    tests force the emulation path by using an adjacency topology that WebGPU cannot draw directly).

#### Translated GS DXBC prepass (`runtime/gs_translate.rs`)

- **What it does:** executes a supported subset of guest GS DXBC as WGSL compute to produce expanded
  vertices/indices and indirect args.
- **When it runs:** for draws using one of the supported IA input topologies (`PointList`, `LineList`,
  `TriangleList`, `LineListAdj`, `TriangleListAdj`) where the bound GS DXBC successfully translated at
  `CREATE_SHADER_DXBC` time and its declared input primitive matches the IA topology.
 - **Pass sequence (translated-GS prepass paths today):**
   1. **Input fill:** a compute pass populates the packed `gs_inputs` payload from **VS outputs**, using
      vertex pulling to load IA data and a small VS-as-compute translator (**currently `mov`/`add` only**)
      to execute the guest VS instruction stream for the subset needed by GS tests.
      - If VS-as-compute translation fails, the executor only falls back to filling `gs_inputs` from the
        IA stream when the VS is a strict passthrough (or `AERO_D3D11_ALLOW_INCORRECT_GS_INPUTS=1` is set
        to force IA-fill for debugging; may misrender). Otherwise the draw fails with a clear error.
  2. **GS execution:** the translated GS WGSL compute entry point runs once per input primitive **per
     draw instance** (`dispatch_workgroups(primitive_count, instance_count, 1)`) and loops
     `gs_instance_id` in `0..GS_INSTANCE_COUNT`.
     The prepass uses `global_invocation_id.y` as an internal draw-instance selector to index into the
     per-instance `gs_inputs` payload (the guest GS itself does not observe a draw-instance ID).
     It appends outputs using atomics, performing strip→list conversion and honoring `cut` semantics.
  3. **Finalize:** a 1-workgroup dispatch runs `cs_finalize` to write `DrawIndexedIndirectArgs` from the
     counters (and to deterministically skip the draw if overflow occurred). For translated-GS prepass
     paths the finalize step emits a **non-instanced** indirect draw (`instance_count = 1`) because the
     prepass already expanded all draw instances.
  4. **Index expand (executor-side):** the executor runs an additional compute pass to expand the
     indexed output into a non-indexed vertex stream and repack `DrawIndexedIndirectArgs` into
     `DrawIndirectArgs`, so the render pass can always use `draw_indirect` (avoiding
     `draw_indexed_indirect`, which is unreliable on some backends). See
     `GEOMETRY_PREPASS_INDEX_EXPAND_CS_WGSL` in
     `crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`.
- **Bindings (translated GS prepass WGSL):**
  - `@group(0)` contains prepass IO (expanded vertices/indices, counters+indirect args, params, and
    `gs_inputs`).
  - `@group(3)` contains referenced GS stage resources (`cb#`, `t#`, `s#`, `u#`) following the shared binding
    model in `binding_model.rs`.
  - The IA vertex pulling bindings (`@group(3) @binding >= BINDING_BASE_INTERNAL`) are only required by
    the **input fill** pass, not by the translated GS itself.

Implemented today:

- **GS/HS/DS bindings**: resource-binding opcodes can target GS (and future HS/DS) binding tables
  without clobbering compute-stage bindings (see “Resource binding model” below). GS can be
  addressed either via `shader_stage = GEOMETRY` (preferred) or via the `stage_ex` compatibility
  encoding; HS/DS require `stage_ex`.
- **Extended `BIND_SHADERS`**: the `BIND_SHADERS` packet can carry `gs/hs/ds` handles via an
  append-only tail (when present, the appended handles are authoritative), and draws route through a
  dedicated “compute prepass” path when any of these stages are bound.
- **Compute→indirect→render pipeline plumbing**: the executor runs a compute prepass to write an
   expanded buffer(s) + indirect args, then renders via `draw_indirect`
   (see `crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`).
  - **Translated GS prepass (real GS subset):** for `PointList`, `LineList`, `TriangleList`,
    `LineListAdj`, and `TriangleListAdj` draws, a supported subset of SM4 GS DXBC is translated to WGSL
    compute and executed to produce expanded geometry (see `exec_geometry_shader_prepass_pointlist`,
    `exec_geometry_shader_prepass_linelist`, and `exec_geometry_shader_prepass_trianglelist` in
    `crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`).
    This prepass writes:
    - expanded vertices,
    - expanded indices, and
    - indirect args (packed with counters into a single storage buffer binding to stay within the
      downlevel WebGPU `max_storage_buffers_per_shader_stage = 4` budget).

    The executor then expands the indexed output into a non-indexed vertex stream and the subsequent
    render pass draws via `draw_indirect` (avoiding `draw_indexed_indirect` on downlevel backends).
  - **Synthetic-expansion prepass (fallback/scaffolding):** the built-in WGSL prepass is used as a
    fallback and for bring-up coverage tests (see `GEOMETRY_PREPASS_CS_WGSL` /
    `GEOMETRY_PREPASS_CS_VERTEX_PULLING_WGSL` in
    `crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`).

    This prepass writes:
    - expanded vertices, and
    - a packed `out_state` storage buffer containing indirect args + counters (sized by
      `GEOMETRY_PREPASS_PACKED_STATE_SIZE_BYTES`).

    It intentionally does **not** write expanded indices (to reduce storage buffer bindings under
    downlevel limits), so the executor always renders this path via `draw_indirect` (even for
    `DRAW_INDEXED` commands).
- **GS DXBC → WGSL compute translation (minimal subset)**:
  - GS DXBC is decoded to SM4 IR and translated to WGSL compute in
    `crates/aero-d3d11/src/runtime/gs_translate.rs` (invoked from `CREATE_SHADER_DXBC` for GS).
  - Strip-cut (`CutVertex` / `RestartStrip`) semantics are validated by deterministic reference
    implementations in `crates/aero-d3d11/src/runtime/strip_to_list.rs`.

Current limitations (high-level):

- Only a small “real GS” path is implemented today:
  - `PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj` draws can execute
    translated SM4 GS DXBC as the compute prepass when the shader is within the supported translator
    subset.
  - Other cases that route through compute-based emulation (including patchlist-only scaffolding, and
    unsupported strip topologies like `LineStrip`/`TriangleStrip`/`LineStripAdj`/`TriangleStripAdj`)
    currently use the built-in synthetic expansion WGSL prepass (and do not execute guest GS DXBC).
- VS-as-compute feeding for GS inputs is still incomplete:
  - The translated-GS prepass paths prefer a minimal VS-as-compute feeding path so the GS observes VS
    output registers (correct D3D11 semantics), but it is still a very small subset (**`mov`/`add` only**,
    simple VS expected).
  - If VS-as-compute translation fails, the executor only falls back to IA-fill when the VS is a
    strict passthrough (or `AERO_D3D11_ALLOW_INCORRECT_GS_INPUTS=1` is set to force IA-fill for
    debugging; may misrender). Otherwise the draw fails with a clear error.
- HS/DS are still scaffolding-only (no real HS/DS DXBC execution yet).

Test pointers:

- End-to-end translated GS execution:
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_point_to_triangle.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_instancing.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_restart_strip.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_pointlist_draw_indexed.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelist_emits_triangle.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelistadj_emits_triangle.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelist_instance_step_rate.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_trianglelist_vs_as_compute_feeds_gs_inputs.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_trianglelist_emits_triangle.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_trianglelistadj_emits_triangle.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_reads_srv_buffer_translated_prepass.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_texture_t0_translated_prepass.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_samples_texture_translated_prepass.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_translated_primitive_id.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_translated_prepass_sv_primitive_id.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_group3_resources.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_translate_cbuffer_cb1.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_cbuffer_b0_offsets_prepass.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_vs_as_compute_feeds_gs_inputs.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_output_topology_pointlist.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_cbuffer_b0_translated_prepass.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_line_strip_output.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_gs_emulation_passthrough.rs`
  - `crates/aero-d3d11/tests/aerogpu_cmd_gs_instance_count.rs`
- Compute prepass plumbing (synthetic expansion): `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_smoke.rs`
  (and `*_primitive_id.rs`, `*_vertex_pulling.rs`, etc)
- GS translator unit tests (standalone): `crates/aero-d3d11/tests/gs_translate.rs`
- DXBC tooling (opcode discovery / token shapes): `cargo run -p aero-d3d11 --bin dxbc_dump -- <gs_*.dxbc>`

---

## Supported GS feature subset (initial)

This section documents the *actual* supported subset for end-to-end GS execution (guest DXBC →
WGSL compute → expanded draw). Anything not listed here should be assumed unsupported.

### Input primitive types (end-to-end)

Supported end-to-end today (translated-GS prepass; for both `Draw` and `DrawIndexed`):

- `point`: `D3D11_PRIMITIVE_TOPOLOGY_POINTLIST`
- `line`: `D3D11_PRIMITIVE_TOPOLOGY_LINELIST`
- `triangle`: `D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST`
- `lineadj`: `D3D11_PRIMITIVE_TOPOLOGY_LINELIST_ADJ`
- `triadj`: `D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST_ADJ`

Note: adjacency topologies require adjacency-aware IA primitive assembly; the required vertex ordering
for `LINELIST_ADJ`/`LINESTRIP_ADJ` and `TRIANGLELIST_ADJ`/`TRIANGLESTRIP_ADJ` is specified in
[`direct3d-10-11-translation.md`](direct3d-10-11-translation.md) section 2.1.1b.

Note: for the current translated-GS prepass paths, the GS `v#[]` inputs are populated via vertex
pulling plus a minimal VS-as-compute feeding path (**currently `mov`/`add` only**) so the GS observes
VS output registers (correct D3D11 semantics).
If VS-as-compute translation fails, the executor only falls back to IA-fill when the VS is a strict
passthrough (or `AERO_D3D11_ALLOW_INCORRECT_GS_INPUTS=1` is set to force IA-fill for debugging; may
misrender because the GS observes pre-VS IA values). Otherwise the draw fails with a clear error.

Not yet supported end-to-end (these may still route through synthetic expansion for plumbing tests, but
do not execute guest GS DXBC):

- strip input topologies (`D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP`, `D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP`)
- adjacency strip topologies (`D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP_ADJ`, `D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP_ADJ`)
  - (tracked by the equivalent of the old Tasks 705/708/711)

### Output topology / streams

Supported end-to-end today (for the translated-GS prepass paths):

- `pointlist` output (indexed **point list**, rendered as `PointList`)
- `linestrip` output, lowered to an indexed **line list** (rendered as `LineList`)
- `trianglestrip` output, lowered to an indexed **triangle list** (rendered as `TriangleList`)
- only **stream 0**

Not yet supported:

- multi-stream output (`EmitStream` / `CutStream` / `SV_StreamID`)

### Supported instruction subset

Supported instructions/opcodes:

- **Primitive emission**
  - `EmitVertex` (`emit`)
  - `CutVertex` (`cut`)
  - `EmitVertex` + `CutVertex` (`emitthen_cut`)
- **Predication / predicate registers (subset)**
  - `setp` (predicate write; `p#` registers)
  - DXBC instruction predication for non-control-flow instructions (emitted as WGSL `if` wrappers)
- **Structured control flow**
  - `if` (`if_z` / `if_nz`)
  - `ifc` (`ifc` compare variants, including unsigned comparisons)
  - `else` / `endif`
  - `loop` / `endloop`
  - `break` / `continue`
  - `breakc` / `continuec`
  - `switch` / `case` / `default` / `endswitch`
  - `ret`
- **ALU (subset)**
  - `mov`, `movc`
  - `add`, `mul`, `mad`
  - `dp3`, `dp4`
  - `min`, `max`
  - `rcp`, `rsq`
  - integer/bitfield ops (subset): `iadd`, `isub`, `imul`, `umul`, `iaddc`, `uaddc`, `isubc`, `ishl`,
    `ishr`, `ushr`, `imin`, `imax`, `umin`, `umax`, `iabs`, `ineg`, `cmp`, `udiv`, `idiv`, `bfi`,
    `ubfe`, `ibfe`, `bfrev`, `countbits`, `firstbit_hi`, `firstbit_lo`, `firstbit_shi`
  - `and`/`or`/`xor`/`not` (bitwise ops on raw 32-bit lanes)
  - conversions: `itof`, `utof`, `ftoi`, `ftou`, `f32tof16`, `f16tof32`
- **Resource operations (subset)**
  - 2D textures:
    - `sample`, `sample_l` (`Texture2D.Sample*`)
    - `ld` (`Texture2D.Load`)
    - `resinfo` (`Texture2D.GetDimensions` / `Texture2D.GetDimensions` + mip count)
  - SRV buffers:
    - `ld_raw`
    - `ld_structured`
    - `bufinfo` (`ByteAddressBuffer.GetDimensions` / `StructuredBuffer.GetDimensions`)
  - UAV buffers (SM5 subset):
    - `ld_uav_raw`, `ld_structured_uav`
    - `store_raw`, `store_structured`
    - `atomic_add`
    - `bufinfo` (`RWByteAddressBuffer.GetDimensions` / `RWStructuredBuffer.GetDimensions`)

Supported operand surface (initial):

- temp regs (`r#`) and output regs (`o#`) (note: `o0` is treated as `SV_Position` and stored in
  `ExpandedVertex.pos`; non-position outputs `oN` are exported to `ExpandedVertex.varyings[N]` for the
  set of output registers declared/written by the GS. Slots the GS does not write get D3D's
    default fill for absent components — `(0, 0, 0, 1)` — not all zeroes, so a missing
    colour varying reads as opaque black rather than transparent)
- GS inputs via `v#[]` (no vertex index out of range for the declared input primitive)
- constant buffers (`cb#[]`) for statically indexed reads (requires `dcl_constantbuffer`)
- resources used by the supported resource ops above:
  - `t#` Texture2D (requires `dcl_resource_texture2d`)
  - `t#` SRV buffer (requires `dcl_resource_buffer`)
  - `u#` UAV buffer (requires `dcl_uav_raw` / `dcl_uav_structured`)
  - `s#` sampler (requires `dcl_sampler`)
- immediate32 `vec4` constants (treated as raw 32-bit lane values; typically `f32` bit patterns)
- swizzles, write masks, destination saturate (`_sat`), and basic operand modifiers (`abs` / `-` / `-abs`)
- system values:
  - `SV_PrimitiveID`
  - `SV_GSInstanceID` (honors `dcl_gsinstancecount` / `[instance(n)]`; the translated prepass loops
    `0..GS_INSTANCE_COUNT` per input primitive, values `0..(n-1)`; default is `n=1`, so the ID is
    always `0`)

Unsupported today (non-exhaustive): binding UAV textures from the command stream (so `dcl_uav_typed` /
`store_uav_typed` is not usable end-to-end yet), barrier/synchronization opcodes (`sync`), and most
other SM4/SM5 instructions. Unsupported features fail translation with a clear error.

---

## Current limitations / non-goals

Geometry shader emulation is intentionally *not* a full D3D11 GS implementation in its first version.
Known limitations include:

- **No multi-stream output**
  - No `EmitStream` / `CutStream` / `emit_stream` / `cut_stream`
  - Only stream 0 is supported; non-zero stream indices are rejected (fail-fast) at
    `CREATE_SHADER_DXBC` time.
- **No stream-out (SO / transform feedback)**
  - GS output cannot be captured into D3D stream-out buffers
- **Limited VS-as-compute feeding for GS inputs**
  - The translated-GS prepass paths run a small VS-as-compute translator to populate the GS `v#[]`
    register payload from VS outputs.
    - Current bring-up limitation: VS-as-compute supports **`mov` and `add` only** (plus `ret`), so
      many vertex shaders will fail translation.
  - If VS-as-compute translation fails, the executor only falls back to IA-fill when the VS is a strict
    passthrough (or `AERO_D3D11_ALLOW_INCORRECT_GS_INPUTS=1` is set to force IA-fill for debugging; may
    misrender because the GS observes pre-VS IA values). Otherwise the draw fails with a clear error.
- **Draw instancing (`instance_count > 1`) is partially validated**
  - Translated-GS prepass paths can execute once per draw instance (prepass expands all instances and
    emits a non-instanced indirect draw; `instance_count = 1`), and this is exercised by
    `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_instancing.rs`.
  - Some translated prepass variants still fail-fast when `instance_count != 1` (bring-up limitation;
    notably the `LineList` translated prepass path).
- **Limited output topology / payload**
  - Output topology is limited to `pointlist`, `linestrip`, and `trianglestrip` (stream 0 only).
    Strip topologies are lowered to list topologies for rendering (`linestrip` → line list,
    `trianglestrip` → triangle list).
  - The expanded-vertex record stores `SV_Position` plus up to 32 `@location(N)` varyings
    (`vec4<f32>` each, indexed by location). The translated GS prepass exports non-position output
    registers `oN` to `ExpandedVertex.varyings[N]` for the set of output registers declared/written by
    the GS; varyings that are not written default to zero.
- **No layered rendering semantics**
  - No `SV_RenderTargetArrayIndex` / `SV_ViewportArrayIndex` style outputs (future work)
- **No fixed-function GS-side rasterizer discard**
  - WebGPU does not expose rasterizer discard; the emulation always runs the render pass
- **WebGL2 backend**
  - WebGL2 has no compute; GS emulation is WebGPU-only (or requires a separate CPU fallback path)
- **Downlevel per-stage storage-buffer limits**
  - Some downlevel backends have very low `max_storage_buffers_per_shader_stage` (commonly 4). GS
    emulation binds multiple storage buffers (expanded vertices + optional expanded indices,
    counters/indirect args, and sometimes IA pulling buffers), so some prepass variants may not be
    available on those backends.
    See `crates/aero-d3d11/tests/common/mod.rs` (`require_gs_prepass_or_skip`,
    `skip_if_compute_or_indirect_unsupported`) for the current skip heuristics used by tests.

Error policy:

- Some unsupported GS features are rejected with clear errors (e.g. non-zero stream indices at
  `CREATE_SHADER_DXBC` time).
- If the guest GS DXBC cannot be translated by the current `gs_translate` subset, the GS handle is
  still accepted/stored, but draws with that GS bound currently fail with a clear
  “geometry shader not supported” error (rather than silently running the synthetic-expansion
  prepass).
- The synthetic-expansion prepass is intended for scaffolding/tests and for non-GS cases that still
  need the compute-prepass path; it is not meant as a “compatibility fallback” for arbitrary
  unsupported GS bytecode.

---

## Synthetic-expansion prepass (`GEOMETRY_PREPASS_CS_WGSL`)

Even with real GS execution available for a small subset, the executor keeps a **synthetic-expansion**
compute prepass (`GEOMETRY_PREPASS_CS_WGSL` in
`crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`) for cases where the command stream must
route through the emulation path but there is no real GS/HS/DS kernel to run yet.

### Output layout (packed state) and indirect draw behavior

The synthetic-expansion fallback prepasses (`GEOMETRY_PREPASS_CS_WGSL` and
`GEOMETRY_PREPASS_CS_VERTEX_PULLING_WGSL`) write:

- `out_vertices`: the expanded vertex buffer consumed by the emulation passthrough VS.
- `out_state`: a single packed storage buffer containing:
  - indirect args, and
  - a small counter block reserved for future GS/HS/DS emulation bookkeeping.

The packed `out_state` buffer is sized by `GEOMETRY_PREPASS_PACKED_STATE_SIZE_BYTES` (see
`crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`). Packing args + counters into a single
binding (and omitting an expanded index buffer in this fallback path) is motivated by downlevel
WebGPU’s tight storage-buffer binding budget (`max_storage_buffers_per_shader_stage = 4` in
`wgpu::Limits::downlevel_defaults()`).

Because this fallback prepass does not produce an expanded index buffer, the executor always draws it
via `draw_indirect` (including when the original command was `DRAW_INDEXED`).

Current uses:

- **HS/DS scaffolding:** bring-up work for tessellation uses the same “compute prepass + indirect draw”
  shape, even before HS/DS DXBC execution exists.
- **Patchlist scaffolding:** D3D11 patchlist topologies (`*_PATCHLIST_*`) are routed through the
  emulation path even before full tessellation is available.
- **Tests that force emulation:** adjacency topologies (`*_ADJ`) are commonly used by tests to force the
  compute-prepass path (because WebGPU cannot draw adjacency primitives directly). List-adjacency
  topologies can execute the translated-GS prepass when a compatible GS is bound; otherwise (including
  when no GS is bound, when the IA topology is not supported by the translated prepass, or for
  adjacency-strip topologies) these route through the synthetic-expansion fallback prepass.
- **Tests that force compute-prepass without a real GS:** e.g.
  `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_smoke.rs` forces the
  compute-prepass path by selecting an adjacency topology (without binding any GS/HS/DS shader).

---

## How to test

GS-related tests use checked-in DXBC fixtures under `crates/aero-d3d11/tests/fixtures/`.
See [`crates/aero-d3d11/tests/fixtures/README.md`](../decisions/README.md)
for details (including how the GS fixtures are authored and how to dump token streams with
`dxbc_dump`).

End-to-end GS emulation (compute prepass executes guest GS DXBC) is covered by:

- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_point_to_triangle.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_instancing.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_restart_strip.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_pointlist_draw_indexed.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelist_emits_triangle.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelistadj_emits_triangle.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_linelist_instance_step_rate.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_trianglelist_emits_triangle.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_trianglelistadj_emits_triangle.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_trianglelist_vs_as_compute_feeds_gs_inputs.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_reads_srv_buffer_translated_prepass.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_texture_t0_translated_prepass.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_samples_texture_translated_prepass.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_translated_primitive_id.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_translated_prepass_sv_primitive_id.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_vs_as_compute_feeds_gs_inputs.rs`
- `crates/aero-d3d11/tests/aerogpu_cmd_gs_instance_count.rs`

These tests require compute shaders and indirect execution, so they may skip on downlevel backends
(e.g. WebGL2, or wgpu-GL adapters with low storage-buffer binding limits).

Example:

```bash
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_point_to_triangle
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_compute_prepass_instancing
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_restart_strip
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_pointlist_draw_indexed
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelist_emits_triangle
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelistadj_emits_triangle
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelist_instance_step_rate
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelist_emits_triangle
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelistadj_emits_triangle
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelist_vs_as_compute_feeds_gs_inputs
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_reads_srv_buffer_translated_prepass
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_texture_t0_translated_prepass
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_samples_texture_translated_prepass
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_translated_primitive_id
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_translated_prepass_sv_primitive_id
cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_vs_as_compute_feeds_gs_inputs
cargo test -p aero-d3d11 --test aerogpu_cmd_gs_instance_count
```

To make skips fail-fast in CI-like environments, set `AERO_REQUIRE_WEBGPU=1` (tests will panic rather
than printing “skipping ...”).

For synthetic-expansion/scaffolding coverage, see:

- `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_compute_prepass_smoke.rs`

---

## Resource binding model

### Bind group indices

Aero’s binding model is stage-scoped. In the AeroGPU command-stream executor (`crates/aero-d3d11/src/runtime/bindings.rs`):

- `@group(0)`: VS resources
- `@group(1)`: PS resources
- `@group(2)`: CS resources
- `@group(3)`: reserved internal / emulation group (keeps the total bind-group count within WebGPU’s
  baseline `maxBindGroups >= 4` guarantee):
  - GS/HS/DS resources (tracked separately from CS to avoid clobbering)
  - internal expansion helpers (vertex pulling, etc) using `@binding >= BINDING_BASE_INTERNAL` to
    avoid collisions with D3D `b#`/`t#`/`s#`/`u#` bindings.

GS/HS/DS stages are emulated using compute passes, but their **D3D-stage resource bindings** are
tracked independently and are expected to be provided to the emulation pipelines via a reserved
bind group:

- `@group(3)` for GS/HS/DS resources (selected either via the direct `shader_stage = GEOMETRY`
  encoding for GS, or via the `stage_ex` ABI extension when `shader_stage = COMPUTE` (required for
  HS/DS; optional GS compatibility encoding)).
- Internal emulation helpers use a mix of:
  - **per-pass internal bindings** (most compute-prepass IO lives in `@group(0)` with small binding
    numbers), and
  - **reserved internal bindings in `@group(3)`** (for shared helpers like IA vertex pulling and the
    expanded-draw vertex buffer).
  Bindings in the internal reserved range start at `BINDING_BASE_INTERNAL = 256` (defined in
  `crates/aero-d3d11/src/binding_model.rs`) so they do not collide with D3D register bindings.

### Binding number ranges within a stage group

Within each stage’s bind group, D3D register spaces are mapped to disjoint `@binding` ranges:

| D3D register space | WGSL `@binding` | Notes |
|---|---:|---|
| `b#` / `cb#` | `BINDING_BASE_CBUFFER + slot` | constant buffers |
| `t#` | `BINDING_BASE_TEXTURE + slot` | SRV textures/buffers |
| `s#` | `BINDING_BASE_SAMPLER + slot` | samplers |
| `u#` | `BINDING_BASE_UAV + slot` | UAV buffers + UAV storage textures (SM5); only UAV buffers are currently bindable via the command stream |

Constants (current defaults):

- `BINDING_BASE_CBUFFER = 0`
- `BINDING_BASE_TEXTURE = 32`
- `BINDING_BASE_SAMPLER = 160`
- `BINDING_BASE_UAV = 176` (`160 + 16`)
- `MAX_UAV_SLOTS = 8` (`u0..u7`)

### Vertex pulling + expansion internal bindings (`@group(3)`)

When running VS/GS/HS/DS as compute, vertex attributes must be loaded from IA vertex buffers manually
(“vertex pulling”), and intermediate outputs must be written to scratch buffers.

Vertex pulling uses a dedicated bind group (`VERTEX_PULLING_GROUP` in
`crates/aero-d3d11/src/runtime/vertex_pulling.rs`):

- `@group(3) @binding(BINDING_BASE_INTERNAL)`: a small uniform containing per-slot `base_offset` + `stride` (+ draw params)
- `@group(3) @binding(BINDING_BASE_INTERNAL + 1 + i)`: vertex buffer slot `i` as a storage buffer (read-only)

These bindings are **internal** to the emulation path; they are not part of the D3D register binding model.
The broader compute-expansion pipeline also defines additional internal scratch bindings; see
[`direct3d-10-11-translation.md`](direct3d-10-11-translation.md) (including the reserved internal
binding-number range).

Note: the full GS/HS/DS emulation pipeline will need a unified bind-group layout that accommodates
 both GS/HS/DS D3D bindings (low `@binding` ranges) and vertex pulling/expansion internal bindings
 (`@binding >= BINDING_BASE_INTERNAL`) within `@group(3)` (keeping the bind group count within the
 WebGPU baseline of 4).

### AeroGPU command stream note: `stage_ex`

The AeroGPU command stream has legacy `shader_stage` enums that mirror WebGPU (VS/PS/CS) and also
includes an explicit Geometry stage (`shader_stage = GEOMETRY`).

To support additional D3D programmable stages (HS/DS) without breaking ABI, some packets support a
“stage_ex” extension that overloads the `reserved0` field when `shader_stage == COMPUTE` (see
`crates/aero-protocol/aerogpu/aerogpu_cmd.rs`).

This extension was introduced in the command stream ABI **1.3** (minor = 3). When decoding command
streams with ABI minor < 3, hosts must ignore `reserved0` even when `shader_stage == COMPUTE`, to
avoid misinterpreting legacy reserved data.

- Preferred GS encoding:
  - set `shader_stage = GEOMETRY` and `reserved0 = 0`
  - this avoids accidentally clobbering CS bindings on hosts that do not implement `stage_ex`
- `stage_ex` encoding (required for HS/DS; may also be used for GS for compatibility):
  - set `shader_stage = COMPUTE` (legacy value `2`)
  - set `reserved0` to a non-zero DXBC program type:
    - `1 = VS`, `2 = GS`, `3 = HS`, `4 = DS`, `5 = CS`
  - `reserved0 == 0` retains legacy compute semantics.
  - Pixel shaders use `shader_stage = PIXEL`; `stage_ex` cannot represent Pixel because `0` is
    reserved for legacy compute.

Packets that carry a `stage_ex` selector in `reserved0` include: `CREATE_SHADER_DXBC`, `SET_TEXTURE`,
`SET_SAMPLERS`, `SET_CONSTANT_BUFFERS`, `SET_SHADER_RESOURCE_BUFFERS`,
`SET_UNORDERED_ACCESS_BUFFERS`, `SET_SHADER_CONSTANTS_F`, and `DISPATCH` (which uses `reserved0` as
an extended stage selector for compute-based GS/HS/DS passes).

---

## Performance characteristics

GS emulation is significantly more expensive than native GS hardware support because it introduces:

- **Extra passes**: one or more compute passes (GS-input fill (IA-fill or VS-as-compute), GS itself, and
  potentially additional expansion passes as tessellation/adjacency coverage grows)
  before the render pass.
- **Intermediate buffers**: VS output + expanded vertex buffer (+ optional expanded index buffer) +
  indirect args/state.
- **Strip→list expansion cost**:
  - `triangle_strip` with `N` emitted vertices produces `(N-2)` triangles, i.e. **`3*(N-2)` list vertices**.
  - `line_strip` with `N` emitted vertices produces `(N-1)` segments, i.e. **`2*(N-1)` list vertices**.

In practice:

- GS-heavy workloads will be bandwidth-bound and should be expected to perform worse than on native D3D.
- The emulation path is best treated as a **compatibility** feature; “fast paths” (pattern-based lowering)
  may still be desirable later for common GS usage patterns.

## Tessellation Emulation (D3D11 HS/DS → WebGPU)

WebGPU does **not** expose hardware tessellation: there is no hull shader (HS), domain shader (DS),
or fixed-function tessellator stage. To support D3D11 tessellation (`VS → HS → Tessellator → DS`),
Aero emulates tessellation by running the missing stages as **compute kernels**, expanding patches
into explicit vertex/index buffers, and then drawing those buffers with a normal WebGPU render
pipeline.

This document describes the chosen HS/DS emulation approach so future contributors can extend it
(quads, fractional partitioning, performance). It focuses on:

- trigger conditions (when draws route through HS/DS emulation),
- pass sequencing (VS-as-compute → HS CP → HS PC → layout → DS → index → render),
- buffer layouts (register files, expanded geometry, indirect args),
- binding model decisions (`@group(3)` for extended stages, plus per-pass internal bind groups),
- limits/clamps and tuning knobs,
- and testing/fixtures.

> Current repo status (important):
>
> - The D3D11 executor already routes draws through a compute prepass when GS/HS/DS emulation is
>   required (see `gs_hs_ds_emulation_required()` in
>   `crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`).
> - Patchlist topology **without HS/DS bound** currently runs the built-in **synthetic expansion**
>   compute prepass that expands a deterministic triangle (to validate render-pass splitting +
>   indirect draw plumbing), not real tessellation semantics.
> - Patchlist topology **with HS+DS bound** (currently PatchList3 only) routes through an initial
>   tessellation prepass pipeline (VS-as-compute vertex pulling placeholder + HS passthrough + layout
>   pass + DS evaluation via `DomainEvalPipeline` + tri-domain integer index generation via
>   `TriDomainIntegerIndexGen`). This path currently requires an input layout for vertex pulling.
>   Guest HS/DS DXBC is not executed yet; tess factors are currently fixed in the passthrough HS
>   (currently `4.0`).
> - The in-progress tessellation runtime lives under
>   `crates/aero-d3d11/src/runtime/tessellation/` and contains real building blocks (layout pass,
>   tri-domain integer index generation, DS evaluation templates, sizing/guardrails). This code is
>   now wired into the command-stream draw path for PatchList3+HS+DS bring-up, but the guest HS/DS
>   DXBC is still not executed yet (the pipeline currently uses passthrough/stub stages).
>
> See [`../areas/graphics.md`](../areas/graphics.md) for an implementation-status checklist.

> Related:
>
> - High-level D3D10/11 mapping: [`direct3d-10-11-translation.md`](direct3d-10-11-translation.md)
> - GS compute expansion notes: [`compute-expansion-emulation.md`](compute-expansion-emulation.md)

---

### When tessellation emulation triggers

Tessellation emulation is required when **either** of these are true:

1. A **patch-list primitive topology** is selected:
   - `AEROGPU_TOPOLOGY_*_CONTROL_POINT_PATCHLIST` (values 33–64)
   - Host-side enum: `CmdPrimitiveTopology::PatchList { control_points }`
2. A **Hull Shader** and/or **Domain Shader** is bound:
   - `hs != 0` and/or `ds != 0` in the extended `AEROGPU_CMD_BIND_SHADERS` packet.

The executor currently uses a single predicate (`gs_hs_ds_emulation_required()`) that triggers the
compute-expansion path for **all** “missing WebGPU stages / topologies” cases:

- GS/HS/DS bound, **or**
- adjacency topologies, **or**
- patchlists.

Patchlist topology triggers the compute-prepass path even before full tessellation is implemented:

- Patchlist draws without HS/DS use the synthetic expansion prepass (bring-up coverage).
- Patchlist draws with HS+DS bound use the tessellation prepass pipeline (VS/HS/layout/DS/index passes).

---

### Why emulation is required

D3D11’s graphics pipeline (simplified) can include tessellation:

```
IA -> VS -> HS -> Tessellator (fixed-function) -> DS -> (GS) -> Rasterizer -> PS
```

WebGPU render pipelines only have:

```
Vertex -> Fragment -> OM
```

There is no programmable HS/DS stage, and there is no fixed-function tessellator. Therefore, when a
D3D11 draw uses patchlists or binds an HS/DS, Aero must:

1. **Execute the missing stages in compute** (VS/HS/DS compiled to WGSL compute entry points).
2. **Explicitly materialize tessellated geometry** into buffers (storage → vertex/index).
3. **Render** the generated buffers with a small passthrough vertex shader and the original pixel
   shader.

This mirrors the approach used for geometry shader emulation: compute expansion + indirect render.

---

### Pipeline sequence (per draw)

At a high level, tessellation emulation is a fixed sequence of compute passes followed by a render
pass:

```
VS-as-compute ->
  HS control-point phase ->
  HS patch-constant phase ->
  tessellator layout ->
  DS evaluation ->
  index generation ->
render (passthrough VS + original PS)
```

If a GS is bound, an additional **GS-as-compute** expansion pass runs after DS and before render
(see the GS doc).

Bring-up note (what is currently wired into the executor for PatchList3+HS+DS):

- `VS-as-compute` is executed as a compute shader using vertex pulling, but it is still a
  passthrough placeholder (guest VS DXBC is not executed yet).
- The HS is currently a **passthrough** kernel that copies control points and writes a fixed tess
  factor (`4.0`) into `hs_tess_factors` (one `vec4<f32>` per patch).
- The layout pass is wired and writes per-patch metadata (`PatchMeta`) plus indexed indirect args.
- The DS is currently a **placeholder** evaluation kernel executed via
  `domain_eval::build_triangle_domain_eval_wgsl` + `DomainEvalPipeline`.
  - Current placeholder behavior: linearly interpolates control-point positions and writes the
    barycentric domain location (`SV_DomainLocation`) into varyings.
- Indices are generated by the dedicated tri-domain integer index-gen pass
  (`tri_domain_integer::TriDomainIntegerIndexGen`).

The remainder of this section describes the *target* HS/DS pipeline; many passes already exist as
unit-testable building blocks, but guest HS/DS DXBC execution and full stage linking are still
in-progress.

#### Input assembler: patchlists

Tessellation draws come from the D3D11 IA stage with a **patchlist** topology:

- `PatchListN` means **N control points per patch** (`N ∈ [1, 32]`).
- The patch count is:
  - non-indexed: `patch_count = vertex_count / N`
  - indexed: `patch_count = index_count / N`
  - instanced: `patch_count_total = patch_count * instance_count` (the patch stream is replicated
    per instance). Many compute passes flatten `(instance_id, patch_id)` into a single
    `patch_instance_id = instance_id * patch_count + patch_id` (see
    `direct3d-10-11-translation.md`).

In native D3D11, VS runs per control point, HS runs per patch (and per output control point), and
DS runs per generated domain point.

#### VS-as-compute (vertex pulling)

Compute shaders cannot consume WebGPU’s vertex input interface. For any draw that routes through
compute expansion (GS/HS/DS), Aero runs the D3D VS as a compute kernel using **vertex pulling**:

- Manually load vertex attributes from the bound IA vertex buffers / index buffer.
- Execute the translated VS logic (or a stub during bring-up).
- Write VS outputs into a storage buffer (`vs_out`).

Implementation references:

- shared vertex pulling ABI: `crates/aero-d3d11/src/runtime/vertex_pulling.rs`
- tessellation VS-as-compute bring-up stub: `crates/aero-d3d11/src/runtime/tessellation/vs_as_compute.rs`

#### HS control-point phase (HS CP)

D3D11 hull shaders have a control-point phase that runs once for each **output control point**:

- Input: the patch’s input control points (from `vs_out`).
- Output: the patch’s output control points (`hs_cp_out`).
- System values: at minimum `SV_OutputControlPointID`, `SV_PrimitiveID`.

In emulation this is a compute pass with `patch_count_total * hs_output_cp_count` invocations.

Host-side dispatch plumbing lives in `crates/aero-d3d11/src/runtime/tessellation/hull.rs`.

#### HS patch-constant phase (HS PC)

Hull shaders also have a patch-constant function that runs **once per patch**:

- Inputs: the patch’s control points (VS outputs and/or HS CP outputs).
- Outputs:
  - **tess factors** (`SV_TessFactor[]`, `SV_InsideTessFactor[]`)
  - user patch constants (arbitrary HS `patchconstant` output struct)
- System values: `SV_PrimitiveID`.

In emulation this is a compute pass with `patch_count_total` invocations.

#### Tessellator layout (fixed-function emulation)

The native D3D11 tessellator is fixed-function hardware that:

1. Reads tess factors produced by HS PC.
2. Chooses a tessellation pattern based on `domain` + `partitioning` + output topology.
3. Produces a list of **domain points** (`SV_DomainLocation`) and a connectivity pattern
   (triangles/lines).

In Aero, this is a compute pass that produces metadata needed by DS + index generation:

- Per-patch **derived tess level** and clamped tess factors (after rounding rules).
- Per-patch **output counts** (how many domain points and indices).
- Per-patch **base offsets** into the global vertex/index buffers.
- Final **indirect draw args** (so render can be indirect without CPU readback).

Current implementation note: the repo contains a deterministic prefix-sum layout pass for
triangle-domain integer tessellation in
`crates/aero-d3d11/src/runtime/tessellation/layout_pass.rs`.

#### DS evaluation (domain shader as compute)

The D3D11 domain shader runs once per generated domain point:

- Inputs:
  - HS output control points
  - HS patch constants
  - `SV_DomainLocation` (for `domain("tri")`: barycentric `(u,v,w)` with `u+v+w=1`)
  - `SV_PrimitiveID`
- Output: a vertex suitable for rasterization (position + varyings).

In Aero, DS is executed as compute and writes to a storage buffer that is also used as the final
vertex buffer for the render pass.

Implementation reference: `crates/aero-d3d11/src/runtime/tessellation/domain_eval.rs`.

#### Index generation

WebGPU rasterization consumes triangles through either:

- non-indexed draws (`draw_indirect`), or
- indexed draws (`draw_indexed_indirect`).

For tessellation, indexed rendering is preferred because the tessellator generates a regular mesh
with shared vertices.

Current implementation note: `crates/aero-d3d11/src/runtime/tessellation/tri_domain_integer.rs`
implements a compute pass that writes a packed `u32` triangle-list index buffer for triangle-domain
integer tessellation.

Bring-up note: PatchList3+HS+DS bring-up now uses this dedicated index-generation pass
(`TriDomainIntegerIndexGen`) instead of emitting indices from the DS evaluation pass.

#### Render pass

Finally, the draw is rendered with a normal WebGPU render pipeline:

- Vertex stage: a small **passthrough** vertex shader that loads the generated vertex struct and
  writes the expected `@builtin(position)` + `@location` varyings.
- Fragment stage: the translated D3D pixel shader.
- Draw call: `draw_indexed_indirect` (or `draw_indirect`) using the args written by the compute
  expansion passes.

---

### Buffer layouts

Tessellation emulation is “buffer-first”: each stage writes explicit results into storage buffers.
The runtime sources transient allocations from the per-frame scratch allocator
`ExpansionScratchAllocator` (`crates/aero-d3d11/src/runtime/expansion_scratch.rs`) so these buffers
do not churn per draw.

#### Register files (“register-file stride”)

DXBC stages communicate via **register files** (e.g. `o0..oN` outputs). When running a stage as
compute, Aero models a register file as a runtime-sized array of **16-byte registers**:

```wgsl
// One register = 16 bytes (4x u32) so float/int/bool bit patterns are preserved.
struct AeroRegFile {
  regs: array<vec4<u32>>,
};
```

Each invocation (control point, patch, or domain point) owns a contiguous slice of `regs`:

- `REG_STRIDE_REGS`: number of `vec4<u32>` registers per invocation
- `REG_STRIDE_BYTES = REG_STRIDE_REGS * 16`
- `linear_index = invocation_index * REG_STRIDE_REGS + reg_index`

In tessellation code, this addressing scheme is used for:

- `vs_out`: VS outputs per **input control point**
- `hs_out`: HS outputs per **output control point**
- `hs_patch_constants`: HS patch constants per **patch**

#### HS tess factors (`hs_tess_factors`)

The tessellation layout pass consumes tess factors from a compact, per-patch buffer:

- element type: `vec4<f32>`
- count: `HS_TESS_FACTOR_VEC4S_PER_PATCH` `vec4<f32>` values per patch (currently 1)
- meaning for the current tri-domain integer path: `{edge0, edge1, edge2, inside}`

This buffer is written by the HS patch-constant phase (and today by the passthrough HS) and then
consumed by the deterministic serial layout pass (`runtime/tessellation/layout_pass.rs`).

#### Expanded vertices + indices

The final render pass needs a WebGPU vertex buffer. Aero’s compute-expansion paths use an
“expanded vertex” format. There are currently **two** formats used in-tree:

1. **Storage-buffer ExpandedVertex record** (used by the current `aerogpu_cmd` expanded-draw path)
2. **Register-stride vertex buffer** (used by the in-progress tessellation DS evaluation templates)

##### Storage-buffer ExpandedVertex record (current expanded-draw path)

This format matches the storage-buffer ABI expected by the autogenerated passthrough VS
(`wgsl_link::generate_passthrough_vs_wgsl`), and is what the current compute-prepass placeholder
shaders write:

```wgsl
struct ExpandedVertex {
  pos: vec4<f32>,                       // SV_Position (clip space)
  varyings: array<vec4<f32>, 32>,        // v0..v31 / @location(0..31)
}
```

Notes:

- `32` is `EXPANDED_VERTEX_MAX_VARYINGS` in `crates/aero-d3d11/src/binding_model.rs`.
- The stride is `(1 + EXPANDED_VERTEX_MAX_VARYINGS) * 16` bytes.
- The buffer is bound as `var<storage, read>` at
  `@group(BIND_GROUP_INTERNAL_EMULATION) @binding(BINDING_INTERNAL_EXPANDED_VERTICES)` and is
  indexed by `@builtin(vertex_index)` (so it is *not* limited by WebGPU vertex attribute count).
- Index buffers are typically `u32` triangle lists with `STORAGE | INDEX` usage.

##### Register-stride vertex buffer (tessellation DS templates)

Some tessellation building blocks currently treat the “expanded vertices” buffer as a **register
file**: a flat `array<vec4<f32>>` where each vertex owns `OUT_REG_COUNT` contiguous registers.

This is the format written by the DS evaluation WGSL template in
`crates/aero-d3d11/src/runtime/tessellation/domain_eval.rs`:

- element type: `vec4<f32>` (one “register” = 16 bytes)
- stride: `OUT_REG_COUNT * 16` bytes per vertex

This format is convenient for stage linking (DS outputs are literally `o0..oN` registers), but it
requires a **vertex-buffer attribute** passthrough strategy (each register becomes a `@location`
vertex input) or a conversion pass to the storage-buffer `ExpandedVertex` record.

#### Indirect draw args

Compute expansion avoids CPU readback by writing a single indirect args struct at offset 0. Aero
uses the canonical layouts in `crates/aero-d3d11/src/runtime/indirect_args.rs`:

- `DrawIndirectArgs` (16 bytes) for `draw_indirect`
- `DrawIndexedIndirectArgs` (20 bytes) for `draw_indexed_indirect`

#### Per-patch offsets and `PatchMeta`

Because WebGPU cannot allocate buffers dynamically on the GPU, tessellation must write into
pre-allocated output buffers. The tessellator layout pass establishes per-patch offsets using a
deterministic prefix sum:

- `vertex_base`/`index_base` are element offsets (not bytes) into the expanded buffers.
- `vertex_count`/`index_count` encode the patch’s contribution.
- the layout pass also writes the final `DrawIndexedIndirectArgs` total counts.

See:

- Rust layout: `TessellationLayoutPatchMeta` in `crates/aero-d3d11/src/runtime/tessellation/mod.rs`
- WGSL layout pass: `crates/aero-d3d11/src/runtime/tessellation/layout_pass.rs`

---

### Bind group model

#### Stage-scoped bind groups + `@group(3)` for extended stages

Aero’s DXBC→WGSL translation and command-stream executor share a stable binding model:

- `@group(0)`: VS resources
- `@group(1)`: PS resources
- `@group(2)`: CS resources

WebGPU guarantees `maxBindGroups >= 4`, so Aero uses `@group(3)` as a reserved internal/emulation
group (`BIND_GROUP_INTERNAL_EMULATION`) that hosts:

- **extended D3D stages**: GS/HS/DS resources (bound via `stage_ex`), and
- internal emulation helpers (vertex pulling, expansion scratch, indirect args, etc).

Within a group, binding numbers are derived from D3D register indices (see
`crates/aero-d3d11/src/binding_model.rs`), and internal bindings use
`@binding >= BINDING_BASE_INTERNAL` to avoid collisions.

#### `stage_ex` and binding the HS/DS tables

To support additional D3D programmable stages (HS/DS) without breaking the ABI, some
resource-binding packets overload their trailing reserved field as a **`stage_ex` selector** when
`shader_stage == COMPUTE` (see `drivers/aerogpu/protocol/aerogpu_cmd.h`).

This extension was introduced in the command stream ABI **1.3** (minor = 3). When decoding command
streams with ABI minor < 3, hosts must ignore `reserved0` even when `shader_stage == COMPUTE`, to
avoid misinterpreting legacy reserved data.

Packets that currently support the `stage_ex` encoding:

- `CREATE_SHADER_DXBC`
- `SET_TEXTURE`
- `SET_SAMPLERS`
- `SET_CONSTANT_BUFFERS`
- `SET_SHADER_RESOURCE_BUFFERS` (SRV buffers, `t#` where the SRV is a buffer view)
- `SET_UNORDERED_ACCESS_BUFFERS` (UAV buffers, `u#` where the UAV is a buffer view)
- `SET_SHADER_CONSTANTS_F`
- `DISPATCH` (uses `reserved0` as the `stage_ex` selector for compute-based HS/DS work)

Encoding invariant:

- If `shader_stage != COMPUTE`, the `stage_ex`/`reserved0` field must be 0 and is ignored.
- If `shader_stage == COMPUTE`:
  - `stage_ex == 0` means the real/legacy Compute stage.
  - `stage_ex != 0` means `stage_ex` is present and encodes a non-zero DXBC program type selector.

- Preferred GS encoding: `shader_stage = GEOMETRY`, `stage_ex = 0`.
- `stage_ex` encoding (required for HS/DS; may also be used for GS for compatibility):
  - `2 = Geometry`
  - `3 = Hull`
  - `4 = Domain`

Pixel shaders are intentionally not representable via `stage_ex` because `0` is reserved for legacy
compute packets; pixel bindings always use `shader_stage = PIXEL`.

On the host, these are tracked as distinct per-stage binding tables so that “real compute” state is
not overwritten by graphics emulation state.

Implementation references:

- protocol enums/encoding: `crates/aero-protocol/aerogpu/aerogpu_cmd.rs`
- executor decoding: `ShaderStage::from_aerogpu_u32_with_stage_ex` in
  `crates/aero-d3d11/src/runtime/bindings.rs`

#### Internal resources: per-pass internal groups (and the “group 3” exception)

Tessellation emulation needs many non-D3D bindings (register files, counters, patch metadata,
domain points, vertex pulling inputs). These are **not** part of the guest-visible D3D binding
model.

Decision: internal resources are bound using **per-pass internal bind groups** with layouts owned by
the executor, rather than reserving additional global bind-group indices.

In practice, there are two patterns in the current codebase:

1. **Separate internal group (preferred when possible)**  
   Example: DS evaluation uses `@group(0)` for internal buffers and `@group(3)` for DS resources
   (`DOMAIN_EVAL_INTERNAL_GROUP = 0`, `DOMAIN_EVAL_DOMAIN_GROUP = 3` in
   `runtime/tessellation/domain_eval.rs`).
2. **Share `@group(3)` for internal + stage resources**  
   Some kernels reuse `@group(3)` for everything because:
   - vertex pulling is already defined to live at `VERTEX_PULLING_GROUP = 3`, and/or
   - HS kernels currently build bind groups via the generic D3D binding-provider machinery and bind
     internal scratch buffers via `internal_buffer()` (see the comment in
     `runtime/tessellation/hull.rs`).

Both approaches preserve the key ABI invariant: D3D register-space bindings (`b#/t#/s#/u#`) remain
stable, and internal resources are always in executor-owned layouts/ranges.

---

### Limits, clamps, and tuning knobs

Tessellation can amplify geometry dramatically. The emulation must enforce deterministic limits to
avoid unbounded scratch allocations or invalid dispatch sizes.

#### Tess factor clamps

- D3D11 hardware tessellation clamps to a maximum factor of **64** (`MAX_TESS_FACTOR` in
  `runtime/tessellator.rs`).
- The current WebGPU emulation path additionally clamps tessellation to a smaller, conservative
  limit: `MAX_TESS_FACTOR_SUPPORTED` (currently **16**) in `runtime/tessellation/mod.rs`.

Raising `MAX_TESS_FACTOR_SUPPORTED` increases scratch usage and can easily exceed default scratch
budgets; do so together with scratch sizing/tuning.

#### Output-budget clamps (scratch guardrails)

The GPU layout pass (`runtime/tessellation/layout_pass.rs`) is parameterized by:

- `patch_count`
- `max_vertices`
- `max_indices`

It will clamp the derived tess level down per patch to keep the total expanded output within these
budgets, and writes a debug flag when clamping occurs.

On the CPU, `TessellationRuntime::alloc_draw_scratch` computes conservative worst-case sizes and
returns a structured `TessellationScratchOomError` when scratch capacity is insufficient.

#### Scratch allocator tuning

`ExpansionScratchAllocator` is configured by `ExpansionScratchDescriptor`:

- `per_frame_size`: increase for tessellation-heavy workloads (default is small because most draws
  don’t yet use GS/HS/DS emulation)
- `frames_in_flight`: typically 2–3; should be ≥ the number of GPU frames that can be in flight to
  avoid reuse hazards

#### Dispatch-size limits

HS/DS compute passes must respect `device.limits().max_compute_workgroups_per_dimension`.
Host-side helpers like `compute_dispatch_x` in `runtime/tessellation/hull.rs` validate this and
error early with actionable messages.

---

### Testing strategy (and adding fixtures)

Tessellation emulation has two orthogonal correctness surfaces:

1. **Tessellator math/layout correctness** (domain points + connectivity + clamping).
2. **End-to-end pipeline plumbing** (bindings, scratch offsets, indirect args, stage linking).

#### Unit tests (CPU-side, deterministic)

- `crates/aero-d3d11/src/runtime/tessellator.rs`: tri-domain integer tessellation math tests.
- `crates/aero-d3d11/src/runtime/tessellation/buffers.rs`: sizing/overflow tests.

#### GPU tests (building blocks)

- `crates/aero-d3d11/tests/tessellation_layout_pass.rs`: runs WGSL layout pass and validates patch
  meta + indirect args.
- `crates/aero-d3d11/tests/tessellation_tri_domain_integer_index_gen.rs`: validates GPU index
  generation against the CPU reference tessellator.
- `crates/aero-d3d11/tests/tessellation_vs_as_compute.rs`: VS-as-compute vertex pulling + register
  addressing.
- `crates/aero-d3d11/tests/tessellation_scratch_guardrails.rs`: validates scratch OOM errors include
  computed sizes and clamped tess factors.

#### Executor tests (command-stream integration)

- `crates/aero-d3d11/tests/aerogpu_cmd_tessellation_smoke.rs`: patchlist+HS/DS routes through the
  tessellation compute prepass.
- `crates/aero-d3d11/tests/aerogpu_cmd_tessellation_hs_ds_compute_prepass_error.rs`: despite the
  name, this currently documents early tessellation prepass error policy (e.g. missing input
  layouts) without panicking.
- `crates/aero-d3d11/tests/aerogpu_cmd_stage_ex_bindings_hs_ds.rs`: validates `stage_ex` routing for
  HS/DS resource binding packets.

#### Adding HS/DS DXBC fixtures

Fixture binaries live in `crates/aero-d3d11/tests/fixtures/` (see `README.md` in that directory).
The repo already includes:

- `hs_minimal.dxbc`, `hs_tri_integer.dxbc`
- `ds_tri_passthrough.dxbc`, `ds_tri_integer.dxbc`

To add new tessellation fixtures:

1. Keep them tiny and deterministic (avoid texture sampling for early tests).
2. Prefer constant tess factors and simple interpolation patterns.
3. Add the new `hs_*.dxbc` / `ds_*.dxbc` files to the fixtures directory.
4. Document the behavior and (if applicable) the compilation command in
   `crates/aero-d3d11/tests/fixtures/README.md`.

---

### Future work (expected extensions)

- **Quad domain** tessellation (different domain coordinate mapping + index generation).
- **Isoline domain** tessellation.
- **Fractional partitioning** (`fractional_even`, `fractional_odd`, `pow2`) with D3D-compatible
  rounding rules.
- **Performance work**: fuse passes, cache index patterns, use workgroup shared memory for control
  points/patch constants, reduce register-file bandwidth by packing only live outputs.
