# Windows 7 Direct3D user-mode driver interface

> The Windows 7 user-mode display driver surface Aero implements, gathered into
> one reference: the minimal D3D9Ex driver, fixed-function vertex processing,
> the D3D10 and D3D11 driver interfaces, the allocation and map/unmap
> contracts, the runtime callbacks and fence rules, the function tables that
> must be populated, and the swapchain expectations.
>
> Every entry is against the Windows 7 driver kit headers; where this page and
> a header disagree, the header wins.

This document is a **focused, implementation-ready spec** for the *user-mode driver (UMD)* side of a **Direct3D 9Ex** WDDM stack that is “just enough” to get **Windows 7 Desktop Window Manager (dwm.exe) + Aero Glass** running.

It is intentionally not a full D3D9 driver guide. It enumerates the **DDI entrypoints**, **caps**, and **behavioral contracts** that matter for DWM, and suggests conservative defaults that avoid DWM falling back to the Basic theme.

> Header references: symbol names match Windows 7-era WDK headers (`d3d9types.h`, `d3d9caps.h`, `d3dumddi.h` / `d3d9umddi.h` / `d3dhal.h` depending on kit). Some structures have multiple “v1/v2” variants across WDDM revisions; when in doubt, implement the newest version your chosen `D3D_UMD_INTERFACE_VERSION` requires and provide compatible fallbacks.

For the next API tier up (D3D10/D3D11 on Win7: SM4/SM5 + DXGI swapchains), see:

* `windows7-d3d-umd-ddi.md`

---

## Scope / assumptions (read this first)

This guide assumes:

* **Windows 7 SP1**, **WDDM 1.1**.
* You are implementing a **D3D9Ex-capable** UMD (i.e. the runtime can create `IDirect3D9Ex` / `IDirect3DDevice9Ex`).
* You want **dwm.exe composition enabled** (Aero), not just “a D3D9 device exists”.
* Kernel-mode details (VidMm, scheduling, allocations) are referenced only insofar as they affect UMD behavior; this document is still **UMD-focused**.

Non-goals for a first pass:

* Full D3D9 game compatibility.
* Complex fixed-function pipeline quirks.
* Advanced queries (occlusion, pipeline stats), multi-sampling, volume textures, D3D9Ex video decode, etc.

---

## How the compositor drives Direct3D 9Ex

### 1.1 High-level composition model

On Windows 7, **DWM is the compositor**. Applications render window contents into **redirected surfaces**; DWM composites those surfaces into the final desktop image each frame.

Key implications for the D3D9Ex UMD:

* DWM runs a **continuous render loop** (typically **~60 Hz**, vsync paced).
* DWM uses **render-to-texture + textured quad composition**:
  * Window surfaces are sampled as **textures**.
  * The desktop backbuffer is a **render target**.
  * Composition is mostly **2D**: screen-space triangles/quads with alpha blending.
* DWM uses **shaders**, not only fixed function:
  * **Vertex shader**: transforms a simple vertex format to clip space; can apply per-window transforms.
  * **Pixel shader**: samples window textures; applies alpha, color matrix, and effects (including blur for glass regions).
* DWM relies heavily on **resource sharing**:
  * DWM must be able to **open textures/surfaces shared by other processes** (each app’s redirected surface).

### 1.2 Swap chain / present cadence

DWM typically creates:

* One `IDirect3DSwapChain9Ex` per monitor (or a single swap chain spanning the desktop, depending on topology).
* **Windowed** swap chains that target the desktop/primary.
* Presentation with **vsync interval 1** is the normal steady-state.

DDI expectations:

* `Present`/`PresentEx` is called **once per composed frame**.
* DWM can use **dirty rects** and **scroll rects** (don’t require perfect optimization; correctness matters first).
* Present must not block indefinitely; if you block inside Present waiting for work that never completes, DWM will hang and may trigger **TDR**.

#### 1.2.1 Typical swap chain parameters (what you should be ready to see)

While exact values differ by GPU/driver, DWM commonly uses a presentation setup equivalent to:

* `Windowed = TRUE`
* `SwapEffect = D3DSWAPEFFECT_FLIPEX` (preferred on Ex) or `D3DSWAPEFFECT_DISCARD`
* `BackBufferFormat = D3DFMT_X8R8G8B8` (opaque alpha; sometimes `A8R8G8B8`)
* `BackBufferCount = 1` (double-buffered) or occasionally more
* `PresentationInterval = D3DPRESENT_INTERVAL_ONE`
* `hDeviceWindow = <DWM's top-level composition window>`

At the DDI level this translates into `pfnCreateSwapChain` + `pfnPresent` calls. Even if you implement presentation as a blit/copy internally, **behave like flip** when `D3DSWAPEFFECT_FLIPEX` is used: the presented buffer becomes the new front buffer and must not be rewritten until it is no longer queued for scanout.

### 1.3 Surface usage patterns (what DWM actually does)

Common DWM operations that should work:

* `Clear` the backbuffer.
* `StretchRect` / blit between surfaces (used for scaling and intermediate passes).
* Fullscreen (or window-rect) quad draws with:
  * Alpha blending.
  * Scissor clipping.
  * Linear filtering for scaled window textures.
* Occasional `ColorFill`.
* Upload/update paths for window content (coming from other processes); DWM itself usually **samples** those surfaces, not CPU-locks them.

### 1.4 Synchronization expectations (DWM “don’t hang me” rules)

DWM throttles itself and checks GPU progress using **queries/fences**. A minimal UMD must provide:

* An **event-like query** that can be “ended” and later polled for completion.
* `Flush` that actually submits work so queries can complete.
* `GetQueryData` that returns “not ready yet” without blocking.

If these are missing or never signal, DWM can spin at 100% CPU, stall forever, or decide the GPU is hung and disable composition.

---

## Minimum Direct3D 9 entrypoints for the compositor

This section enumerates the **minimal adapter/device entrypoints** a D3D9Ex-capable UMD should implement for DWM.

The exact entrypoint surface is controlled by the UMD interface version you advertise (e.g. `D3D_UMD_INTERFACE_VERSION`). Regardless of the versioning, the *functional* requirements below remain the same.

### 2.1 UMD exports: adapter open/close

Implement these exported entrypoints (names as in WDK):

| Export | Prototype typedef | Required? | Minimal semantics |
|---|---|---:|---|
| `OpenAdapter` | `PFND3DDDI_OPENADAPTER` | Yes | Provide adapter handle + fill `D3DDDI_ADAPTERFUNCS` with pointers; store `D3DDDI_ADAPTERCALLBACKS`. |
| `OpenAdapterFromHdc` | `PFND3DDDI_OPENADAPTERFROMHDC` | Highly recommended | Many components still open adapters via HDC path; forward to common open logic. |
| `OpenAdapterFromLuid` | `PFND3DDDI_OPENADAPTERFROMLUID` | Yes for Win7 robustness | D3D9Ex paths frequently open adapters by LUID. |

Notes:

* “Close” is typically through adapter function table (e.g. `pfnCloseAdapter`), not an export.
* Always support **one adapter instance per LUID**; DWM and multiple D3D clients expect consistent identity.

**AeroGPU-specific discovery:** during adapter open, query the KMD for the active AeroGPU device ABI + feature bits via:

* `D3DKMTQueryAdapterInfo(KMTQAITYPE_UMDRIVERPRIVATE)`

Decode the returned `aerogpu_umd_private_v1` from:

* `drivers/aerogpu/protocol/aerogpu_umd_private.h`

This allows the UMD to gate optional paths (vblank pacing, fence pages, etc.) on reported feature bits, and to determine whether it is running against the legacy `"ARGP"` device (optional; feature-gated behind `emulator/aerogpu-legacy`) or the versioned `"AGPU"` ABI.

### 2.2 Adapter function table: minimum `D3DDDI_ADAPTERFUNCS`

Your `OpenAdapter*` must return an adapter funcs table with (at least) the following implemented:

| Func pointer | Required? | Minimal semantics | What can be stubbed |
|---|---:|---|---|
| `pfnGetCaps` | Yes | Handle the caps queries D3D9Ex + DWM uses (see §3). Return consistent caps across calls. | Unused caps types can return `D3DDDIERR_INVALIDPARAMS`/`E_INVALIDARG` *if you are sure they aren’t queried*, but safest is to return “not supported” sizes/zeros for unknown types. |
| `pfnCreateDevice` | Yes | Create a device/context; return `hDevice`; fill `D3DDDI_DEVICEFUNCS`. | N/A |
| `pfnCloseAdapter` | Yes | Free adapter-private allocations; invalidate handles. | N/A |
| `pfnQueryAdapterInfo` | Recommended | Return stable answers for core `D3DDDIQUERYADAPTERINFO_*` requests used by runtime (driver info, WDDM model, etc.). | For unknown query types, prefer returning `S_OK` with a zeroed output buffer (and logging once) so unexpected runtime/DWM probes do not break device bring-up. |

Practical note: if `pfnQueryAdapterInfo` is wrong/incomplete, the runtime can still create devices, but DWM may refuse composition due to missing “driver model / feature” information.

#### 2.2.1 `pfnGetCaps`: minimum query types to support

`pfnGetCaps` is where D3D9Ex learns what you support. The runtime may call it directly and indirectly (to implement API calls like `CheckDeviceFormat`).

Support at least these `D3DDDICAPS_TYPE`-style requests (names vary slightly by header/version):

* **D3D9 device caps**: `D3DDDICAPS_GETD3D9CAPS` / `D3DDDICAPS_D3D9CAPS`
  * Output: `D3DCAPS9`
* **Format enumeration**: `D3DDDICAPS_GETFORMATCOUNT` and `D3DDDICAPS_GETFORMAT`
  * Used to enumerate `D3DDDIFORMAT` / `D3DFORMAT` support across usages.
* **StretchRect filter caps** and related blit capabilities (often folded into the `D3DCAPS9` output).
* **Multisample quality levels**: `D3DDDICAPS_GETMULTISAMPLEQUALITYLEVELS`
  * You can return “none supported” if you do not advertise MSAA, but the query itself should not fail.

Treat unknown caps types conservatively:

* If you don’t recognize a caps request, log it and return a clean “unsupported” result rather than crashing.
* Prefer returning `S_OK` with a zeroed output buffer (or otherwise deterministic “unsupported” output) over failing the call. If you must fail, prefer `E_INVALIDARG` over `E_FAIL`.

### 2.3 Device function table: minimum `D3DDDI_DEVICEFUNCS`

Below is the minimal functional surface. Many functions are “simple state setters”: they must accept calls, store state, and feed state to your backend at draw time.

#### 2.3.1 Device lifecycle / submission

| Func pointer | Required? | Minimal semantics | What can be stubbed |
|---|---:|---|---|
| `pfnDestroyDevice` | Yes | Free all device state; ensure no outstanding callbacks reference freed memory. | N/A |
| `pfnFlush` | Yes | Submit accumulated work so it becomes visible to the scheduler and queries can complete. Must return quickly. | Do not block waiting for GPU; keep it async. |
| `pfnWaitForIdle` | Recommended | Block until previously submitted work completes (used by some reset paths). | Can be implemented as “flush + wait on last fence”; avoid busy-wait. |
| `pfnReset` / `pfnResetEx` (version-dependent) | Recommended | Handle display mode / swap chain recreation without requiring a full device destroy. | For a first pass you can internally destroy and recreate swap chain resources, but keep the same `hDevice`. |

#### 2.3.2 Swap chain and presentation

| Func pointer | Required? | Minimal semantics | What can be stubbed |
|---|---:|---|---|
| `pfnCreateSwapChain` | Yes | Create swap chain + backbuffers. Support at least one backbuffer (double buffering recommended). | Multi-sample can be rejected if not advertised. |
| `pfnDestroySwapChain` | Yes | Destroy swap chain and its implicit resources. | N/A |
| `pfnCheckDeviceState` (D3D9Ex) | Recommended | Return `S_OK`, `S_PRESENT_OCCLUDED`, or `S_PRESENT_MODE_CHANGED` based on the destination window/monitor state. DWM uses this to decide whether to keep composing. | For early bring-up you can conservatively return `S_OK` always (composition stays enabled), but you must never block. |
| `pfnPresent` | Yes | Present composed backbuffer to the desktop. Implement `PresentEx`-style flags if surfaced through `D3DDDIARG_PRESENT::Flags`. | You can ignore dirty rect optimizations initially, but must obey clipping/rect correctness (i.e. don’t read invalid memory). |
| `pfnGetPresentStats` | Recommended | Return monotonically increasing present counters/timestamps. DWM may use this for pacing/diagnostics. | Can return zeros except a monotonic present count; don’t fail the call. |
| `pfnWaitForVBlank` | Optional | If called, block until next vblank or emulate a vblank tick. | Safe stub: sleep for ~1 refresh interval if you have timing; otherwise return `S_OK`. |

**Present return codes (important):**

* For occlusion/minimized scenarios, `PresentEx` paths expect `S_PRESENT_OCCLUDED`.
* For mode changes, `S_PRESENT_MODE_CHANGED` can be returned.
* Avoid returning `D3DERR_DEVICELOST`/`D3DDDIERR_DEVICEHUNG` unless truly fatal; DWM may disable composition.

#### 2.3.3 Resource creation / destruction / sharing

| Func pointer | Required? | Minimal semantics | What can be stubbed |
|---|---:|---|---|
| `pfnCreateResource` | Yes | Create textures/surfaces/buffers with correct bind flags: render target, texture sampling, dynamic/lockable. Must support **non-power-of-two** sizes. | Reject resource types you don’t advertise (e.g. volume textures; cubemaps if unsupported). |
| `pfnOpenResource` / `pfnOpenResource2` (version-dependent) | Yes for Aero | Open a shared resource created in another process. This is critical for redirected surfaces. | If you don’t support cross-process sharing, DWM won’t compose real apps. |
| `pfnDestroyResource` | Yes | Free resource and associated allocations; handle refcounting for shared resources (close vs destroy). | N/A |
| `pfnSetPriority` / `pfnQueryResourceResidency` | Optional | Priorities/residency queries can be conservative. | Safe stub: treat everything as resident, fixed priority. |

Resource types DWM commonly needs:

* `D3DDDIRESTYPE_SURFACE`
* `D3DDDIRESTYPE_TEXTURE`
* `D3DDDIRESTYPE_VERTEXBUFFER` (small dynamic buffers for quads)
* `D3DDDIRESTYPE_INDEXBUFFER` (optional; DWM may use non-indexed draws)

#### 2.3.4 CPU access: Lock/Unlock and update paths

| Func pointer | Required? | Minimal semantics | What can be stubbed |
|---|---:|---|---|
| `pfnLock` | Yes (for robustness) | Provide CPU mapping for lockable resources. Respect discard/no-overwrite semantics for dynamic buffers/textures where possible. | You may reject locks on DEFAULT render targets if you don’t advertise lockability. |
| `pfnUnlock` | Yes | Commit CPU writes; mark dirty ranges/rects (see §4). | N/A |
| `pfnUpdateSurface` / `pfnUpdateTexture` | Recommended | Provide fast upload/copy paths that the runtime uses as an alternative to lock/copy/unlock. | Can be internally implemented as a blit. |
| `pfnBlt` (StretchRect) | Yes | Copy/scale between surfaces; support filtering (point/linear) as requested. | If you don’t support scaling, don’t advertise it; but DWM uses scaling frequently, so implement at least point/linear. |
| `pfnColorFill` | Recommended | Fill a rect with a color. | Can be implemented as a tiny draw/blit; don’t fail. |
| `pfnClear` | Yes | Clear color target (and depth/stencil if you provide them). | If you don’t support depth/stencil, ignore those flags. |

#### 2.3.5 Shader creation and binding

| Func pointer | Required? | Minimal semantics | What can be stubbed |
|---|---:|---|---|
| `pfnCreateVertexShader` | Yes | Accept D3D9 shader token stream (vs_2_0 minimum). Compile/translate to backend. | You may reject shader models you don’t advertise (e.g. vs_3_0). |
| `pfnDeleteVertexShader` | Yes | Free shader. | N/A |
| `pfnSetVertexShader` | Yes | Bind current VS for subsequent draws. | N/A |
| `pfnSetVertexShaderConstF` | Yes | Store float constant registers; DWM uses these for transforms/effect params. | Int/bool variants can be stubbed if never called, but implementing them is easy and reduces risk. |
| `pfnCreatePixelShader` | Yes | Accept ps_2_0 minimum; used heavily by DWM for sampling/blend/effects. | Same as VS. |
| `pfnDeletePixelShader` | Yes | Free shader. | N/A |
| `pfnSetPixelShader` | Yes | Bind current PS. | N/A |
| `pfnSetPixelShaderConstF` | Yes | Store float constants. | Same as VS consts. |

Practical note: for a first-pass compositor, you can implement a narrow “shader translator” that supports only the instruction subset DWM emits. Use tracing (see §6) to expand.

#### 2.3.6 Fixed-function and render state (DWM uses only a subset)

Implement the following state-setting entrypoints and treat unknown state values as “store and ignore” rather than failing.

| Func pointer | Required? | Minimal semantics | What can be stubbed |
|---|---:|---|---|
| `pfnSetRenderTarget` | Yes | Bind the active render target(s). DWM typically uses MRT=1. | MRT>1 can be rejected if not advertised. |
| `pfnSetDepthStencil` | Optional | DWM is mostly 2D; but depth/stencil may be used for clip masks. | If unsupported, avoid advertising depth/stencil formats and accept calls as no-ops. |
| `pfnSetViewport` | Yes | Track viewport; map to backend viewport/scissor. | N/A |
| `pfnSetScissorRect` | Yes | Required for window clipping. | N/A |
| `pfnSetRenderState` | Yes | Must support alpha blend, cull, z-enable (even if ignored), color write mask. | Don’t fail unknown render states; store them. |
| `pfnSetTexture` | Yes | Bind textures for sampling (stage 0..N). DWM uses few stages. | Higher stages can be accepted and ignored if never used. |
| `pfnSetSamplerState` | Yes | Must support MIN/MAG filters and address modes; DWM relies on linear filtering. | Anisotropy can be ignored unless advertised. |
| `pfnSetTextureStageState` | Optional | If DWM uses shaders, this may be unused. | Safe behavior: accept and ignore. |
| `pfnSetFVF` | Optional | If the runtime routes DWM through FVF-based vertex formats, map FVF to an internal vertex layout (or synthesize a vertex declaration). | Safe behavior: accept and ignore only if you are sure DWM uses vertex declarations instead. |
| `pfnCreateVertexDeclaration` | Yes | Convert `D3DVERTEXELEMENT9`-style declarations into backend layouts. | N/A |
| `pfnDeleteVertexDeclaration` | Yes | Free declaration. | N/A |
| `pfnSetVertexDeclaration` | Yes | Bind declaration. | N/A |
| `pfnSetStreamSource` | Yes | Bind vertex buffer + stride/offset. | N/A |
| `pfnSetStreamSourceFreq` / `pfnGetStreamSourceFreq` | Optional | Cache per-stream frequency/divisor for D3D9-style stream instancing (`SetStreamSourceFreq`). Defaults to `1` for all streams. | If you don’t implement instancing, treat non-default values as cached-only (don’t fail), but instanced draws may fail or render incorrectly. |
| `pfnSetIndices` | Optional | If DWM uses indexed draws. | Implement anyway; it’s commonly exercised. |

Other D3D9-era state APIs (lights, materials, fog, texture transform matrices) can generally be stubbed as no-ops if you avoid advertising fixed-function reliance and DWM sticks to shaders.

**Render/sampler state that should work correctly for DWM:**

At minimum, implement the behavior of these states (store them and apply in your backend pipeline):

* Render states (`D3DRENDERSTATETYPE` via `pfnSetRenderState`):
  * `D3DRS_ALPHABLENDENABLE`
  * `D3DRS_SRCBLEND`, `D3DRS_DESTBLEND`, `D3DRS_BLENDOP`
  * `D3DRS_SEPARATEALPHABLENDENABLE` (if enabled by DWM; safe to support)
  * `D3DRS_SRCBLENDALPHA`, `D3DRS_DESTBLENDALPHA`, `D3DRS_BLENDOPALPHA`
  * `D3DRS_COLORWRITEENABLE`
  * `D3DRS_SCISSORTESTENABLE`
  * `D3DRS_CULLMODE`
  * `D3DRS_ZENABLE`, `D3DRS_ZWRITEENABLE` (even if you effectively ignore depth)
  * `D3DRS_SRGBWRITEENABLE` (optional; improves correctness for desktop gamma)
* Sampler states (`D3DSAMPLERSTATETYPE` via `pfnSetSamplerState`):
  * `D3DSAMP_ADDRESSU`, `D3DSAMP_ADDRESSV` (DWM commonly uses `D3DTADDRESS_CLAMP`)
  * `D3DSAMP_MINFILTER`, `D3DSAMP_MAGFILTER`, `D3DSAMP_MIPFILTER` (DWM commonly uses LINEAR for scaling)
  * `D3DSAMP_SRGBTEXTURE` (optional; only if you also implement sRGB sampling correctly)

If you implement only one blend mode initially, make sure you cover standard premultiplied-alpha composition:

* `SRCBLEND = ONE`, `DESTBLEND = INVSRCALPHA`, `BLENDOP = ADD`

#### 2.3.7 Draw calls

| Func pointer | Required? | Minimal semantics | What can be stubbed |
|---|---:|---|---|
| `pfnBeginScene` / `pfnEndScene` | Recommended | Track scene bracketing; some runtimes expect it for validation/flush decisions. | Can be no-ops returning `S_OK`. |
| `pfnDrawPrimitive` | Yes | Translate triangles/triangle strips for screen-space geometry. | N/A |
| `pfnDrawIndexedPrimitive` | Yes (robustness) | Same as above with indices. | N/A |
| `pfnDrawPrimitive2` / `pfnDrawIndexedPrimitive2` | Recommended | Handles `Draw*UP` paths where vertex data comes from user memory. | If you don’t implement, ensure runtime never routes DWM through UP draws (hard to guarantee). |

#### 2.3.8 Queries and synchronization (must not hang)

| Func pointer | Required? | Minimal semantics | What can be stubbed |
|---|---:|---|---|
| `pfnCreateQuery` | Yes | Support at least `D3DQUERYTYPE_EVENT` (or the DDI equivalent query type). | Reject unsupported query types with a clean failure (e.g. `D3DERR_NOTAVAILABLE`). |
| `pfnDestroyQuery` | Yes | Free query object. | N/A |
| `pfnIssueQuery` | Yes | On `END`, insert a fence into your backend queue. | `BEGIN` can be ignored for event queries. |
| `pfnGetQueryData` | Yes | Non-blocking poll: return `S_OK` when complete, else `S_FALSE`/`D3DERR_WASSTILLDRAWING` depending on API contract. Must not spin inside. | Timestamp queries etc can be unimplemented. |

**Rule:** if your backend is async (WebGPU, Vulkan, etc), the query completion path must be backed by a real fence/timeline so progress is guaranteed.

**Practical flag quirk:** for EVENT queries, be permissive about the `IssueQuery` flag encoding. Some D3D9Ex paths have been observed to pass `flags=0` for “END”, and some DDI header vintages use `0x2` for END at the DDI boundary. Treat `(flags == 0) || (flags & 0x1) || (flags & 0x2)` as END for EVENT queries.

Guest-side validation:

  * `drivers/aerogpu/tests/win7/d3d9ex_event_query` verifies `D3DQUERYTYPE_EVENT` completion behavior and that `GetData(D3DGETDATA_DONOTFLUSH)` remains non-blocking (including an initial poll before `Flush`; DWM relies on this polling pattern).

#### 2.3.9 State blocks and validation helpers (recommended for app compatibility)

While DWM itself typically relies on shaders + explicit state setting, many D3D9 apps (and some runtimes) use **state blocks** and **ValidateDevice**:

| Func pointer | Required? | Minimal semantics | What can be stubbed |
|---|---:|---|---|
| `pfnBeginStateBlock` / `pfnEndStateBlock` | Recommended | Begin/finish capturing a stateblock via subsequent DDI calls. | If unimplemented, return a clean failure (`D3DERR_INVALIDCALL`/`D3DERR_NOTAVAILABLE`) rather than crashing. |
| `pfnCreateStateBlock` | Recommended | Create a stateblock for `D3DSBT_ALL` / `D3DSBT_PIXELSTATE` / `D3DSBT_VERTEXSTATE`. | Same as above. |
| `pfnCaptureStateBlock` / `pfnApplyStateBlock` | Recommended | Capture current state into a block, and apply a block back to the device state. | N/A |
| `pfnDeleteStateBlock` | Recommended | Free a state block. | N/A |
| `pfnValidateDevice` | Recommended | Return a conservative pass count (typically `1`) for the supported shader pipeline. | Avoid hard-failing unless truly unsupported; callers often treat ValidateDevice as an advisory probe. |

Practical notes:

- State blocks are primarily about *state capture/restore*, not rendering. A minimal but robust first pass can cache the state you already track for your command stream and replay it on Apply.
- Some apps create a new stateblock by doing `BeginStateBlock → Apply(existing) → EndStateBlock`. In this scenario, **Apply must record state** into the in-progress capture even if the apply is a no-op.

Guest-side validation:

* `drivers/aerogpu/tests/win7/d3d9ex_stateblock_sanity` covers Begin/End + Create/Capture/Apply and includes the “nested Apply while recording” pattern.
* `drivers/aerogpu/tests/win7/d3d9_validate_device_sanity` covers `ValidateDevice`.

---

## Capability reporting

Windows 7 enables DWM composition only if it believes the adapter can sustain the compositor workload. You want to report **the minimum set that satisfies DWM** while avoiding caps that cause the runtime to exercise unimplemented paths.

### 3.1 Core “Aero gate” requirements (practical)

At the API level, DWM expects (indirectly via DDI caps):

* **WDDM driver model** (not XDDM).
* **D3D9Ex availability** (device creation succeeds through Ex path).
* **Shader Model 2.0 minimum**:
  * `D3DCAPS9::VertexShaderVersion >= D3DVS_VERSION(2,0)`
  * `D3DCAPS9::PixelShaderVersion  >= D3DPS_VERSION(2,0)`
* **Windowed rendering support**:
  * `D3DCAPS9::Caps2` includes `D3DCAPS2_CANRENDERWINDOWED`.
* **Shared resources** (redirected surfaces):
  * `D3DCAPS9::Caps2` includes `D3DCAPS2_CANSHARERESOURCE` (and you must implement `pfnOpenResource*` correctly).
* **Non-power-of-two textures** for arbitrary window sizes.
* **Alpha blending** and **linear filtering**.

### 3.2 Required formats (conservative minimal set)

Make the following formats work for the usage DWM needs:

**Render target / swap chain:**

* `D3DFMT_X8R8G8B8` (desktop backbuffer common; opaque alpha)
* `D3DFMT_A8R8G8B8` (composition surfaces with alpha)

**Texture sampling:**

* `D3DFMT_A8R8G8B8` (window textures)
* `D3DFMT_X8R8G8B8` (opaque surfaces; alpha treated as 1.0)

**Depth/stencil (optional but increases robustness):**

* `D3DFMT_D24S8` or `D3DFMT_D16`

If you do not implement depth/stencil correctly, do **not** report these as supported; prefer a pure-2D compositor first.

### 3.3 Caps knobs to set (D3DCAPS9-focused)

These are commonly required by compositor-style workloads:

* `D3DCAPS9::RasterCaps`:
  * `D3DPRASTERCAPS_SCISSORTEST` (DWM clips constantly)
* `D3DCAPS9::TextureFilterCaps` (at least):
  * `D3DPTFILTERCAPS_MINFPOINT`, `D3DPTFILTERCAPS_MINFLINEAR`
  * `D3DPTFILTERCAPS_MAGFPOINT`, `D3DPTFILTERCAPS_MAGFLINEAR`
* `D3DCAPS9::SrcBlendCaps` / `DestBlendCaps`:
  * `D3DPBLENDCAPS_ONE`, `D3DPBLENDCAPS_ZERO`
  * `D3DPBLENDCAPS_SRCALPHA`, `D3DPBLENDCAPS_INVSRCALPHA`
  * Recommended for compositor-style fades/tints: `D3DPBLENDCAPS_BLENDFACTOR`, `D3DPBLENDCAPS_INVBLENDFACTOR`
    (and implement `D3DRS_BLENDFACTOR` + `D3DBLEND_BLENDFACTOR` / `D3DBLEND_INVBLENDFACTOR` correctly).
* `D3DCAPS9::MaxTextureWidth/Height`:
  * Must be at least the maximum expected window size (recommend **4096** to start; 8192 if easy).
* `D3DCAPS9::MaxSimultaneousTextures`:
  * DWM typically uses 1–4; advertise **4** safely.
* `D3DCAPS9::MaxStreams`:
  * At least 1; advertise **1–4**.
* `D3DCAPS9::StretchRectFilterCaps`:
  * Include at least point+linear (DWM uses `StretchRect` for scaling and intermediate passes).
* `D3DCAPS9::PresentationIntervals`:
  * Include at least `D3DPRESENT_INTERVAL_ONE` and optionally `D3DPRESENT_INTERVAL_IMMEDIATE` (don’t advertise intervals you can’t honor).
* Non-power-of-two textures:
  * Ensure `D3DCAPS9::TextureCaps` does **not** force `D3DPTEXTURECAPS_POW2`.
  * If you only support restricted NPOT, set `D3DPTEXTURECAPS_NONPOW2CONDITIONAL` and follow its rules; otherwise support full NPOT.

### 3.4 What to avoid advertising initially (to reduce surface area)

Do **not** advertise features until you have test coverage and tracing proving correctness:

* Multi-sampling / MSAA.
* `D3DFMT_A16B16G16R16F`, HDR, wide gamut.
* Cubemaps (unless you have implemented `D3DRTYPE_CUBETEXTURE` end-to-end; AeroGPU now supports cubemaps via
  `D3DPTEXTURECAPS_CUBEMAP` and represents them as 2D array textures with 6 layers (`Depth == 6`, normalized even if the
  runtime leaves `Depth` at 1/0)).
* Volume textures (`D3DRTYPE_VOLUME` / `D3DRTYPE_VOLUMETEXTURE`) — not supported by AeroGPU today.
* Autogen mipmaps (`D3DUSAGE_AUTOGENMIPMAP`).
* Advanced blend ops if you don’t implement them (`D3DBLENDOP_*` beyond ADD).
* Query types beyond event (timestamps, occlusion).
* Any “driver-managed memory tricks” that change lock semantics (unless implemented).

The general strategy is:

1. **Advertise only the formats/usages you implement**.
2. If you see DWM calling an unimplemented DDI path, either:
   * implement it, or
   * stop advertising the capability that triggers it.

---

## Resource and memory update model

This is the “make it not glitch” contract for a compositor workload.

### 4.1 Treat `Unlock` as the commit point

For lockable resources:

* `pfnLock` returns:
  * a CPU pointer (or “pitch + pointer” for surfaces),
  * plus whatever metadata the runtime expects (row pitch, slice pitch).
* `pfnUnlock` must:
  * **record dirty ranges/rects**,
  * schedule upload to the backend before the resource is next used for drawing/sampling.

A minimal but robust model:

* Keep a per-subresource `dirty` flag and `dirty_rect` (union of all writes) or `dirty_range` for buffers.
* On unlock, merge the region.
* On first use after dirty, upload the dirty region (or whole resource if simpler).

### 4.2 Dynamic buffers: discard/no-overwrite semantics

Even if DWM is not a game, it often uses small dynamic vertex buffers.

Support these flags if present in `D3DDDIARG_LOCK::Flags`:

* **Discard**: allocate a fresh backing store (or advance a ring buffer) so GPU reads aren’t stalled.
* **NoOverwrite**: allow CPU writes without forcing a GPU sync; safe if you use a ring buffer strategy.

If you can’t implement them correctly yet, it is better to:

* allow `Discard` and treat it as “new allocation” always, and
* treat `NoOverwrite` the same as a normal lock (conservative).

### 4.3 Render targets: avoid CPU readback

DWM mainly uses render targets as GPU-only intermediates. For a first pass:

* Make DEFAULT render targets **not lockable**.
* If the runtime tries to lock them, return `D3DERR_INVALIDCALL` *only if you are sure this path is not required for DWM*; otherwise provide a slow readback path.

### 4.4 Shared resources (critical for redirected surfaces)

When implementing `pfnOpenResource*`:

* The opened resource must alias the same underlying storage as the creator’s resource.
* Reference counting matters:
  * “Destroy resource” in one process should not free storage if another process has it open.
* Synchronization:
  * DWM will sample a window texture while the app is rendering into it.
  * Correctness for the first pass can be “last completed content”; perfect cross-process fencing can come later, but avoid tearing by ensuring present/flush boundaries publish updates.

Pragmatic approach for early bring-up:

* Treat `Present`/`Flush` as publishing points:
  * When the producing process presents or flushes, the shared surface content becomes visible to DWM on the next frame.

Guest-side validation:

Canonical `share_token` contract and rationale: `aerogpu-device-abi.md`.

* `drivers/aerogpu/tests/win7/d3d9ex_shared_surface` exercises the cross-process “create shared → open shared” path.
  * Validates cross-process pixel sharing via readback by default.
  * Pass `--no-validate-sharing` to focus on open + minimal submit (`ColorFill` + `Flush`) only (`--dump` always validates).
* `drivers/aerogpu/tests/win7/d3d9ex_shared_surface_ipc` is a smaller producer/consumer variant that focuses on opening a shared D3D9Ex render-target texture in a second process and validating a readback of the producer’s clear color.
* On Win7 x64, `drivers/aerogpu/tests/win7/d3d9ex_shared_surface_wow64` validates cross-bitness shared-surface interop (WOW64 producer → native consumer; DWM scenario).
* For DWM-like multi-process batching and `alloc_id` collision coverage, also run `drivers/aerogpu/tests/win7/d3d9ex_shared_surface_many_producers` and `drivers/aerogpu/tests/win7/d3d9ex_alloc_id_persistence`.
* For MVP shared-surface allocation policy coverage (shared surfaces must be single-allocation; reject shared full mip chains), also run `drivers/aerogpu/tests/win7/d3d9ex_shared_allocations`.
* For open/close churn coverage (repeated create → open → destroy; catches hangs/crashes), also run `drivers/aerogpu/tests/win7/d3d9ex_shared_surface_stress`.

---

## Error handling and stability

### 5.1 Never block indefinitely inside a DDI call

Rules for DWM stability:

* All DDI entrypoints must return in **milliseconds**, not seconds.
* Any waiting must be bounded and tied to a real fence.
* If a query isn’t ready, return “not ready” (`S_FALSE` / `D3DERR_WASSTILLDRAWING`) rather than blocking.

Guest-side validation:

* `drivers/aerogpu/tests/win7/d3d9ex_dwm_ddi_sanity` exercises the DWM-critical D3D9Ex probes (device state checks, PresentEx throttling, vblank waits, present stats, residency, etc.) and asserts that each call remains non-blocking (per-call latency bound).

### 5.2 Keep GPU work chunks small

Windows TDR is designed to reset GPUs that appear hung. Even in emulation/translation:

* Break long shader compilations out of the render thread if possible (compile async, cache results).
* Avoid submitting huge command buffers in a single flush.
* Ensure `pfnFlush` actually advances some “completed fence” over time.

### 5.3 Return codes DWM tends to tolerate vs. ones that disable composition

**Generally tolerated (use for non-fatal conditions):**

* `S_OK`
* `S_FALSE` / `D3DERR_WASSTILLDRAWING` (query/data not ready)
* `S_PRESENT_OCCLUDED` (present when minimized/occluded)
* `S_PRESENT_MODE_CHANGED` (display mode changed; DWM will rebuild)

**Dangerous (often triggers Basic theme or device reset paths):**

* `D3DERR_DEVICELOST`
* `D3DERR_DEVICEREMOVED`
* `D3DDDIERR_DEVICEHUNG`
* `E_FAIL` in core rendering paths (CreateDevice, Present, CreateResource)

Strategy:

* For **unimplemented state**, prefer “accept + ignore + return `S_OK`”.
* For **unimplemented features**, avoid advertising the cap so the runtime never calls it.
* For **true allocation failures**, `E_OUTOFMEMORY` / `D3DERR_OUTOFVIDEOMEMORY` is better than generic `E_FAIL`, but expect DWM to potentially disable composition if resources can’t be created.
  * AeroGPU note: if allocations fail “too early” (while the guest still has free RAM), you may be hitting Win7’s WDDM segment budget rather than true exhaustion. AeroGPU is system-memory-backed, but dxgkrnl still enforces the KMD-reported non-local segment size; tune `HKR\Parameters\NonLocalMemorySizeMB` (see `windows7-aerogpu-validation.md` appendix and `drivers/aerogpu/kmd/README.md`).

---

## Tracing methodology for bring-up

Goal: confirm that your “minimal subset” matches what DWM actually calls, and catch the first missing function/cap quickly.

### 6.1 UMD-side instrumentation (fastest feedback loop)

Add structured logging at every DDI entrypoint.

In this repo, the AeroGPU D3D9 UMD already includes an **in-process DDI call trace facility** (ring buffer + one-shot dump triggers):

* `windows7-d3d-umd-ddi.md`

For example, to quickly identify the first stub-tagged DDI hit:

```cmd
set AEROGPU_D3D9_TRACE=1
set AEROGPU_D3D9_TRACE_MODE=unique
set AEROGPU_D3D9_TRACE_FILTER=stub
set AEROGPU_D3D9_TRACE_DUMP_ON_STUB=1
set AEROGPU_D3D9_TRACE_DUMP_ON_DETACH=1
```
This only triggers on entrypoints whose trace names include the explicit `(stub)` marker; if none are stub-tagged in the current build, it will not fire.

* Print:
  * function name,
  * thread id,
  * key handles (`hDevice`, `hResource`, `hSwapChain`, `hQuery`),
  * sizes/formats/usages,
  * and return `HRESULT`.
* Include a monotonically increasing **frame id** incremented on `pfnPresent`.
* Use a ring buffer + conditional flush to avoid slowing down DWM.

This alone is usually enough to discover:

* which caps queries you must answer,
* which DDI functions get hit during logon,
* and which state/render paths DWM uses on your configuration.

### 6.2 ETW/GPUView: validate scheduling and “no hangs”

On Windows 7, use ETW to correlate DWM pacing with GPU work submission:

* Providers to capture (common names):
  * `Microsoft-Windows-Dwm-Core`
  * `Microsoft-Windows-DxgKrnl`
  * `Microsoft-Windows-Win32k`
* Tools:
  * `xperf`/`wpr` (depending on what’s installed in the VM)
  * GPUView for visualization

Look for:

* regular present cadence,
* no multi-second stalls,
* queries completing and allowing DWM to advance frames.

### 6.3 PIX for Windows (D3D9) call-level inspection

PIX for Windows 7 can capture D3D9 call streams (API-level, not DDI-level). It’s useful to learn:

* what shaders DWM uses,
* what render states are set,
* what resources/formats are created.

Practical approach:

1. Disable composition (switch to Basic) so you can safely restart DWM.
2. Launch/attach PIX to `dwm.exe` (or a small D3D9Ex test app that mimics DWM’s usage).
3. Re-enable composition and capture a few frames.

Even if PIX cannot attach to the system DWM process in your environment, capturing a test app still validates that your DDI supports the expected D3D9Ex patterns.

### 6.4 Expected call sequence (sanity checklist)

During logon / enabling Aero you should see roughly:

1. Adapter open:
   * `OpenAdapterFromLuid` (common) + `pfnGetCaps` / `pfnQueryAdapterInfo`
2. Device bring-up:
   * `pfnCreateDevice`
   * `pfnCreateSwapChain` (+ backbuffer resources)
3. Pipeline setup:
   * `pfnCreateVertexDeclaration`
   * `pfnCreateVertexShader`, `pfnCreatePixelShader`
   * A handful of `pfnCreateResource` for intermediate surfaces
4. Frame loop:
   * `pfnBeginScene` (optional)
   * many state setters (`pfnSet*`)
   * `pfnDrawPrimitive` / `pfnDrawIndexedPrimitive`
   * queries: `pfnIssueQuery` / `pfnGetQueryData`
   * `pfnEndScene` (optional)
   * `pfnPresent`

If you do not reach a steady Present loop, DWM likely failed composition and fell back. The first failing HRESULT in your logs is usually the reason.

## Fixed-function vertex processing

This document summarizes how the AeroGPU Windows 7 D3D9Ex UMD applies **world/view/projection (WVP)** transforms for the
fixed-function fallback path, and how that relates to the `pfnProcessVertices` CPU transform subset.

It is referenced by:

- `../areas/graphics.md`
- `drivers/aerogpu/umd/d3d9/README.md` (“Fixed-function vertex formats (FVF)” → “Limitations (bring-up)”)

### Draw-time WVP for fixed-function `D3DFVF_XYZ*` draws

When the D3D9 runtime is using the fixed-function fallback path with an untransformed position FVF (`D3DFVF_XYZ*`), the UMD binds an internal VS variant that applies the combined `WORLD0 * VIEW * PROJECTION` transform on the GPU and sources the matrix from a reserved high VS constant register range.

#### VS WVP constants (XYZ fixed-function FVFs)

For these fixed-function FVFs, the UMD binds one of a few built-in vertex shaders that multiplies the input position by the cached `WORLD0 * VIEW * PROJECTION` matrix:

- `D3DFVF_XYZ | D3DFVF_DIFFUSE`: `fixedfunc::kVsWvpPosColor`
- `D3DFVF_XYZ | D3DFVF_DIFFUSE | D3DFVF_TEX1`: `fixedfunc::kVsWvpPosColorTex0`
- `D3DFVF_XYZ | D3DFVF_TEX1` (no diffuse): `fixedfunc::kVsTransformPosWhiteTex1` (supplies constant opaque white diffuse)
- `D3DFVF_XYZ | D3DFVF_NORMAL` (no diffuse): `fixedfunc::kVsWvpPosNormalWhite` (unlit; supplies constant opaque white diffuse) or `fixedfunc::kVsWvpLitPosNormal` (when `D3DRS_LIGHTING` is enabled)
- `D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_TEX1` (no diffuse): `fixedfunc::kVsWvpPosNormalWhiteTex0` (unlit; supplies constant opaque white diffuse) or `fixedfunc::kVsWvpLitPosNormalTex1` (when `D3DRS_LIGHTING` is enabled)
- `D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE`: `fixedfunc::kVsWvpPosNormalDiffuse` (unlit) or `fixedfunc::kVsWvpLitPosNormalDiffuse` (when `D3DRS_LIGHTING` is enabled)
- `D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE | D3DFVF_TEX1`: `fixedfunc::kVsWvpPosNormalDiffuseTex1` (unlit) or `fixedfunc::kVsWvpLitPosNormalDiffuseTex1` (when `D3DRS_LIGHTING` is enabled)

The matrix is computed from cached `Device::transform_matrices[...]` (`WORLD0`, `VIEW`, `PROJECTION`) and uploaded by
`ensure_fixedfunc_wvp_constants_locked()` into a reserved constant range:

- Constant range: `c240..c243` (`kFixedfuncMatrixStartRegister = 240`)
- Uploads are gated by `Device::fixedfunc_matrix_dirty` and occur at draw time and/or eagerly when `SetTransform`/`MultiplyTransform` updates relevant matrices while the **full fixed-function pipeline** is active (no user shaders bound).
- The cached matrices are row-major (`D3DMATRIX`); the upload transposes to column vectors so `dp4(v, cN)` computes row-vector multiplication.
- The constants live in a high register range so they are unlikely to collide with app/user shader constants when switching between fixed-function and programmable paths.
  - Even so, the UMD will proactively mark `fixedfunc_matrix_dirty` when switching back to fixed-function WVP vertex shaders so the constants are re-uploaded (user shaders may have written overlapping VS constant registers).
  - Some D3D9 runtimes also expect the reserved WVP constant range to be refreshed **immediately** when a user vertex shader is unbound (`SetShader(VS, NULL)`), so the UMD may force an upload at shader-unbind time (not just lazily at the next draw).

This covers the fixed-function FVFs:

- `D3DFVF_XYZ | D3DFVF_DIFFUSE` (VS WVP constants)
- `D3DFVF_XYZ | D3DFVF_DIFFUSE | D3DFVF_TEX1` (VS WVP constants)
- `D3DFVF_XYZ | D3DFVF_TEX1` (VS WVP constants; driver supplies default diffuse white)
- `D3DFVF_XYZ | D3DFVF_NORMAL` (VS WVP constants; driver supplies default diffuse white when unlit; optional fixed-function lighting)
- `D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_TEX1` (VS WVP constants; driver supplies default diffuse white when unlit; optional fixed-function lighting)
- `D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE` (VS WVP constants; optional fixed-function lighting)
- `D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE | D3DFVF_TEX1` (VS WVP constants; optional fixed-function lighting)

When `D3DRS_LIGHTING` is enabled for the `NORMAL` variants, the UMD additionally uploads a reserved fixed-function lighting constant block (`c208..c236`; `kFixedfuncLightingStartRegister = 208`) via `ensure_fixedfunc_lighting_constants_locked()` (world*view columns 0..2, packed light subset, material, global ambient).

- Uploads are gated by `Device::fixedfunc_lighting_dirty` and occur at draw time when a lit fixed-function VS variant is active.
- Like the WVP constants, this uses a high constant range to reduce collisions with app/user VS constants. Even so, the UMD will mark `fixedfunc_lighting_dirty` when user shaders or `SetShaderConstF` write overlapping registers so the constants are refreshed when switching back to the lit fixed-function path.

Note: for `TEX1` fixed-function FVFs, `TEXCOORD0` may be declared as `float1/2/3/4` via `D3DFVF_TEXCOORDSIZE*`.
The fixed-function fallback uses the first two components as `(u, v)` (`float1` implies `v=0`; extra components are ignored).
Multiple texture coordinate sets still require user shaders (layout translation is supported; fixed-function shading is not).

Note: patch rendering (`DrawRectPatch` / `DrawTriPatch`) consumes `TEXCOORD0` with the same bring-up conventions as the
fixed-function fallback path:

- `float1`: uses `.x` as `u` and treats `v = 0`
- `float2/float3/float4`: uses `.xy` as `(u, v)` (extra components are ignored)

### Pre-transformed `D3DFVF_XYZRHW*` draws (no WVP)

For pre-transformed screen-space vertices (`D3DFVF_XYZRHW*` / `POSITIONT`), the UMD does **not** use WVP transforms.
Instead, it converts `XYZRHW` to clip-space on the CPU via `convert_xyzrhw_to_clipspace_locked()` before emitting the draw.

Conversion details (bring-up):

- Uses the current D3D9 viewport (`X`, `Y`, `Width`, `Height`) and inverts the D3D9 `-0.5` pixel center convention
  (`x/y` are treated as pixel-center coordinates).
- Uses `w = 1/rhw` (with a safe fallback when `rhw==0`) and writes `clip.xyzw = {ndc.x*w, ndc.y*w, z*w, w}`.
- `z` is treated as D3D9 NDC depth (`0..1`); viewport `MinZ`/`MaxZ` are currently ignored.
- Converted vertices are uploaded into a scratch UP VB using the active fixed-function FVF stride/layout
   (`POSITIONT=float4` + optional `COLOR0` and/or `TEXCOORD0`), and drawn with the corresponding internal fixed-function
   passthrough VS variant (including a default-white diffuse variant when `D3DFVF_DIFFUSE` is omitted).

This is used by the fixed-function FVFs:

- `D3DFVF_XYZRHW | D3DFVF_DIFFUSE{,TEX1}`
- `D3DFVF_XYZRHW | D3DFVF_TEX1` (driver supplies default diffuse white)

### `pfnProcessVertices` fixed-function CPU transform subset

Independently of draw-time WVP, `pfnProcessVertices` has a bring-up fixed-function subset in `device_process_vertices_internal()`:

- Condition: no user **vertex** shader is bound (pixel shader binding does not affect `ProcessVertices`).
- Supported source `dev->fvf` values:
  - `D3DFVF_XYZRHW` (+ optional `D3DFVF_DIFFUSE`, + optional `D3DFVF_TEX1`)
  - `D3DFVF_XYZW` (+ optional `D3DFVF_DIFFUSE`, + optional `D3DFVF_TEX1`)
  - `D3DFVF_XYZ` (+ optional `D3DFVF_DIFFUSE`, + optional `D3DFVF_TEX1`)
- It writes screen-space `XYZRHW` into **stream 0** of the destination layout described by `hVertexDecl`. Declaration
  elements in other streams are ignored when inferring destination stride/offsets.
  - Destination stride: uses `DestStride` when present and non-zero; otherwise it infers the effective stride from
    **stream 0** of `hVertexDecl`. If the stride cannot be inferred, the fixed-function subset fails with
    `D3DERR_INVALIDCALL`.
  - In-place overlap safety: when the source and destination buffers alias the same resource and the strided ranges overlap
    (notably when `src_stride != dest_stride`), the implementation stages the source bytes before writing destinations to
    avoid self-overwrite.
  - For `D3DFVF_XYZ*` / `D3DFVF_XYZW*` inputs, it computes `WORLD0 * VIEW * PROJECTION`, applies the D3D9 viewport
    transform for `x/y`, and writes `XYZRHW` (screen space). For `D3DFVF_XYZW*`, the input `w` component is respected.
    - `z` stays in D3D9 NDC depth (`0..1`); viewport `MinZ`/`MaxZ` are currently ignored.
  - For `D3DFVF_XYZRHW*` inputs, `XYZRHW` is already in screen space and is passed through unchanged.
  - `D3DPV_DONOTCOPYDATA` (`ProcessVertices.Flags & 0x1`) controls whether non-position elements are written:
    - When set, the UMD writes **only** the output position (`POSITIONT` float4) and preserves all other destination bytes
      (no zeroing, no `DIFFUSE`/`TEXCOORD0` writes).
    - When not set, the UMD clears the full destination vertex before writing outputs so any elements not written become 0
      (deterministic output for extra decl fields).
    - When not set and the destination declaration includes `DIFFUSE`, the UMD copies it from the source when present,
      otherwise fills it with opaque white (matching fixed-function behavior). `TEXCOORD0` is copied only when present in
      both the source and destination layouts (supports `FLOAT1/2/3/4`; source texcoord size is derived from
      `D3DFVF_TEXCOORDSIZE*` when set). Some D3D9 runtimes appear to synthesize destination decls where `TEXCOORD0` uses
      `Usage=POSITION` (`0`) rather than `D3DDECLUSAGE_TEXCOORD`; the UMD is intentionally permissive and accepts this.

### Code anchors

- Fixed-function shader binding:
  - `ensure_fixedfunc_pipeline_locked()` (`drivers/aerogpu/umd/d3d9/src/aerogpu_d3d9_driver.cpp`)
- Internal fixed-function shader token streams:
  - `fixedfunc::kVsWvpPosColor`, `fixedfunc::kVsWvpPosColorTex0`, `fixedfunc::kVsTransformPosWhiteTex1`,
    `fixedfunc::kVsWvpPosNormalWhite{,Tex0}`, `fixedfunc::kVsWvpLitPosNormal{,Tex1}`,
    `fixedfunc::kVsWvpPosNormalDiffuse{,Tex1}`, `fixedfunc::kVsWvpLitPosNormalDiffuse{,Tex1}` (`drivers/aerogpu/umd/d3d9/src/aerogpu_d3d9_fixedfunc_shaders.h`)
- Draw-time fixed-function constant upload (untransformed `D3DFVF_XYZ*` fixed-function paths):
  - `ensure_fixedfunc_wvp_constants_locked()` + `emit_set_shader_constants_f_locked()` (`AEROGPU_CMD_SET_SHADER_CONSTANTS_F`)
  - `ensure_fixedfunc_lighting_constants_locked()` (fixed-function lighting `c208..c236` constant block for `D3DFVF_XYZ | D3DFVF_NORMAL{,DIFFUSE}{,TEX1}` when `D3DRS_LIGHTING` is enabled)
- CPU conversions for pre-transformed `XYZRHW*` draws:
  - `convert_xyzrhw_to_clipspace_locked()` (`XYZRHW`/`POSITIONT` → clip-space for pre-transformed fixed-function draws)
- Existing CPU vertex processing:
  - `device_process_vertices_internal()` (`ProcessVertices` fixed-function subset: XYZ→XYZRHW transform, XYZRHW pass-through, optional diffuse fill, honors `D3DPV_DONOTCOPYDATA`)
  - `device_process_vertices()` (DDI entrypoint; falls back to a stride-aware memcpy and clamps pre-transformed `XYZRHW*` copies to 16 bytes when `D3DPV_DONOTCOPYDATA` is set)
- Transform state cache:
  - `Device::transform_matrices[...]` (populated by `Device::SetTransform` / state blocks)

### Validation (Win7 guest tests)

The Win7 guest validation suite includes targeted coverage for these paths:

- `drivers/aerogpu/tests/win7/d3d9_fixedfunc_wvp_triangle` validates the fixed-function WVP transform path for
  `D3DFVF_XYZ | D3DFVF_DIFFUSE` (center pixel vs right-shifted pixel after a `StateBlock`-applied transform).
- `drivers/aerogpu/tests/win7/d3d9_fixedfunc_textured_wvp` validates the fixed-function WVP transform path for
  `D3DFVF_XYZ | D3DFVF_DIFFUSE | D3DFVF_TEX1` (order-sensitive WVP, stage0 texture sampling + MODULATE, and both
  `SetVertexDeclaration`-inferred FVF and explicit `SetFVF` paths).
- `drivers/aerogpu/tests/win7/d3d9_fixedfunc_lighting_directional` validates the minimal fixed-function lighting bring-up
  subset for `D3DFVF_XYZ | D3DFVF_NORMAL` (directional light 0 + material + `D3DRS_LIGHTING`).

## Tracing Direct3D 9 driver calls

This repo contains a small **in-UMD smoke-test trace facility** for the Win7 D3D9Ex user-mode display driver (UMD).

It is intended to answer one question during bring-up:

> “Which D3D9UMDDI entrypoints does `dwm.exe` (or a small D3D9Ex test) actually call, and with what key parameters / HRESULTs?”

The tracing implementation is **logging/introspection only**:

- No allocations on hot paths
- No file I/O on hot paths
- In-memory fixed-size buffer + a one-shot dump trigger via `OutputDebugStringA` (optionally also `stderr`)

Source: `drivers/aerogpu/umd/d3d9/src/aerogpu_trace.*`

The recommended repro apps below are part of the Win7 guest validation suite.
Build/run instructions live in: `drivers/aerogpu/tests/win7/README.md`.

Host-side unit tests (no Win7 VM required) live in:

- `drivers/aerogpu/umd/d3d9/tests/`

They validate the trace filtering and dump trigger behavior (including unique-mode force-record cases) and run under CI via `ctest` when `AERO_AEROGPU_BUILD_TESTS=ON`.

---

### Enabling tracing

Tracing is **disabled by default**. Enable it by setting environment variables in the target process (or globally, then restarting the process).

#### Required

- `AEROGPU_D3D9_TRACE=1`  
  Enables trace recording.

#### Optional controls

- `AEROGPU_D3D9_TRACE_MODE=unique|all` (default: `unique`)
  - `unique`: records only the **first call per entrypoint** (best for `dwm.exe`, avoids log spam)
  - `all`: records every call until the fixed buffer is full

- `AEROGPU_D3D9_TRACE_MAX=<N>` (default: 512)  
  Maximum number of records to store (clamped to `<= 512`). `0` is treated as “use the default” (512).

- `AEROGPU_D3D9_TRACE_FILTER=<TOKENS>`  
  Records only entrypoints whose trace name contains any of the comma-separated tokens (case-insensitive substring match).
  Leading/trailing whitespace around tokens is ignored.
  If the filter value is empty or contains only commas/whitespace (for example `AEROGPU_D3D9_TRACE_FILTER=,, ,`), the filter is treated as **unset** (`filter_on=0`).
  Note: the filter applies to recording and per-entrypoint dump triggers (`AEROGPU_D3D9_TRACE_DUMP_ON_FAIL=1` / `AEROGPU_D3D9_TRACE_DUMP_ON_STUB=1` will only fire for filtered-in entrypoints).
  The present-count dump trigger (`AEROGPU_D3D9_TRACE_DUMP_PRESENT`) is not suppressed by the filter, but the filter still controls which calls are recorded (including whether the triggering `Present` call is force-recorded).
  Example: `AEROGPU_D3D9_TRACE_FILTER=StateBlock,ValidateDevice`
  Tip: use `AEROGPU_D3D9_TRACE_FILTER=stub` to record only stub-tagged entrypoints (trace names include the substring `(stub)`).

- `AEROGPU_D3D9_TRACE_STDERR=1` (Windows-only; optional)  
  By default, trace output on Windows is emitted via `OutputDebugStringA` (for DebugView/WinDbg). When this is set, the trace output is also echoed to `stderr` (useful for console repro apps and host-side unit tests).

#### Common recipes

##### Debug StateBlock / ValidateDevice (minimal repro apps)

For `d3d9_validate_device_sanity` and `d3d9ex_stateblock_sanity`, a useful setup is:

```cmd
set AEROGPU_D3D9_TRACE=1
set AEROGPU_D3D9_TRACE_MODE=all
set AEROGPU_D3D9_TRACE_FILTER=StateBlock,ValidateDevice
set AEROGPU_D3D9_TRACE_DUMP_ON_DETACH=1
```

If you suspect the app is failing early, you can also use:

```cmd
set AEROGPU_D3D9_TRACE_DUMP_ON_FAIL=1
```

##### Identify the first stub-tagged DDI hit

When you suspect the runtime or `dwm.exe` is calling an unimplemented DDI, a useful setup is:

```cmd
set AEROGPU_D3D9_TRACE=1
set AEROGPU_D3D9_TRACE_MODE=unique
set AEROGPU_D3D9_TRACE_FILTER=stub
set AEROGPU_D3D9_TRACE_DUMP_ON_STUB=1
set AEROGPU_D3D9_TRACE_DUMP_ON_DETACH=1
```
Note: this relies on the `(stub)` marker in the trace name (`func_name()`); host tests use the trace-only `TraceTestStub` entrypoint to exercise this behavior without depending on real DDI stubs.
If no traced entrypoints are `(stub)`-tagged in the current build, this trigger will not fire.

#### Dump triggers (on-demand)

The trace buffer is only dumped when triggered:

- `AEROGPU_D3D9_TRACE_DUMP_PRESENT=<N>`  
  Dumps once when the UMD device `present_count` reaches `N` (works for both `Present` and `PresentEx`).
  Note: when `AEROGPU_D3D9_TRACE_MODE=unique`, the triggering `Present`/`PresentEx` call is force-recorded so the dump still includes the call that caused the trigger (unless it is filtered out by `AEROGPU_D3D9_TRACE_FILTER`).
  The dump still triggers even if the present entrypoints are filtered out; the dump just won't include the present call record.

- `AEROGPU_D3D9_TRACE_DUMP_ON_DETACH=1`  
  Dumps once on `DllMain(DLL_PROCESS_DETACH)`.

- `AEROGPU_D3D9_TRACE_DUMP_ON_FAIL=1`  
  Dumps once on the first traced entrypoint that returns a failing HRESULT (`FAILED(hr)`). The dump reason string is the failing entrypoint name.
  Note: when `AEROGPU_D3D9_TRACE_MODE=unique`, the failing call is force-recorded so the dump still includes the call that caused the trigger (unless it is filtered out by `AEROGPU_D3D9_TRACE_FILTER`).

- `AEROGPU_D3D9_TRACE_DUMP_ON_STUB=1`  
  Dumps once on the first traced entrypoint whose trace name is marked as a stub (contains the substring `(stub)`). This is useful for quickly identifying when the Win7 D3D9 runtime (or `dwm.exe`) exercises an unimplemented DDI.
  Note: some DDIs are intentionally treated as benign bring-up **no-ops** and are **not** marked as stubs in trace output, so they do not trigger this dump (see `drivers/aerogpu/umd/d3d9/README.md`).
  Note: when `AEROGPU_D3D9_TRACE_MODE=unique`, the triggering stub call is force-recorded so the dump still includes the call that caused the trigger (unless it is filtered out by `AEROGPU_D3D9_TRACE_FILTER`).

For `dwm.exe`, prefer `AEROGPU_D3D9_TRACE_DUMP_PRESENT` so you get logs *while DWM is running*, rather than only at shutdown.
For small repro apps that don't call `Present`/`PresentEx` (for example `d3d9_validate_device_sanity` and `d3d9ex_stateblock_sanity`), prefer `AEROGPU_D3D9_TRACE_DUMP_ON_DETACH=1` so the trace dumps when the process exits.

---

### Capturing logs (DebugView)

The dump uses `OutputDebugStringA` by default.

If you are tracing a **console app** (for example one of the Win7 guest validation tests), you can also set `AEROGPU_D3D9_TRACE_STDERR=1` so trace output appears in the console `stderr` stream.

If you run the guest tests via `aerogpu_test_runner.exe --log-dir=...`, enabling `AEROGPU_D3D9_TRACE_STDERR=1` will capture trace dumps into the per-test `*.stderr.txt` files, which is often more convenient than DebugView.

Recommended workflow on Win7:

1. Run **Sysinternals DebugView** as Administrator
2. Enable:
   - `Capture Win32`
   - `Capture Global Win32`
3. Start the target app:
   - `drivers/aerogpu/tests/win7/d3d9ex_dwm_probe`
   - `drivers/aerogpu/tests/win7/d3d9_validate_device_sanity`
   - `drivers/aerogpu/tests/win7/d3d9ex_triangle`
   - `drivers/aerogpu/tests/win7/d3d9ex_stateblock_sanity`
   - or restart `dwm.exe` after setting env vars

   Note: `d3d9_validate_device_sanity` and `d3d9ex_stateblock_sanity` don't call `Present`/`PresentEx`, so `AEROGPU_D3D9_TRACE_DUMP_PRESENT` won't trigger for them. Use `AEROGPU_D3D9_TRACE_DUMP_ON_DETACH=1` instead.

You should see lines starting with:

```
aerogpu-d3d9-trace: dump reason=...
```

---

### Reading the output

When tracing is enabled, the UMD prints a one-line banner describing the active configuration.
When a dump trigger fires, the first dump line repeats key parts of that configuration:

- `mode`: `unique` or `all`
- `max`: effective record capacity (after applying `AEROGPU_D3D9_TRACE_MAX`)
- `dump_present`: the configured `AEROGPU_D3D9_TRACE_DUMP_PRESENT` count (0 = disabled)
- `dump_on_detach`: whether `AEROGPU_D3D9_TRACE_DUMP_ON_DETACH=1` is enabled
- `dump_on_fail`: whether `AEROGPU_D3D9_TRACE_DUMP_ON_FAIL=1` is enabled
- `dump_on_stub`: whether `AEROGPU_D3D9_TRACE_DUMP_ON_STUB=1` is enabled
- `stderr_on`: whether `AEROGPU_D3D9_TRACE_STDERR=1` is enabled (Windows-only echo)
- `filter_on` / `filter_count`: whether `AEROGPU_D3D9_TRACE_FILTER` is active and how many entrypoints are included

Example:

```
aerogpu-d3d9-trace: #004 t=123456 tid=1234 Device::CreateResource a0=0x... a1=0x... a2=0x... a3=0x... hr=0x00000000
```

Fields:

- `#NNN`: record index (in call order, up to `AEROGPU_D3D9_TRACE_MAX`)
- `t`: raw timestamp (QPC ticks on Windows)
- `tid`: thread id
- function name: DDI entrypoint
- `a0..a3`: key arguments (packed as needed; see below)
- `hr`: HRESULT returned by the entrypoint
  - `hr=0x7fffffff` is a special “pending” marker that indicates the call was recorded but had not yet reached its `return` path when the dump was taken (rare; typically only possible if the process is crashing or a dump trigger fires mid-call).

#### Common argument packings

This trace is meant to be lightweight, so most values are logged as raw integers/pointers:

- `Device::CreateResource`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = pack_u32_u32(type, format)`
  - `a2 = pack_u32_u32(width, height)`
  - `a3 = pack_u32_u32(usage, pool)`

- `Device::PresentEx`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = hWnd`
  - `a2 = pack_u32_u32(sync_interval, d3d9_present_flags)`
  - `a3 = hSrc.pDrvPrivate`

- `Device::Present`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = hSwapChain.pDrvPrivate`
  - `a2 = hSrc.pDrvPrivate`
  - `a3 = pack_u32_u32(sync_interval, d3d9_present_flags)`

- `Device::GetQueryData`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = hQuery.pDrvPrivate`
  - `a2 = pack_u32_u32(data_size, flags)`
  - `a3 = pData`

- `Device::ProcessVertices`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = hDestBuffer.pDrvPrivate`
  - `a2 = pack_u32_u32(SrcStartIndex, DestIndex)`
  - `a3 = pack_u32_u32(VertexCount, DestStride)` (`DestStride` may be 0 depending on header/runtime)
  - Note: `pProcessVertices->Flags` (D3DPV_* bits) is not currently logged; `D3DPV_DONOTCOPYDATA` is bit 0 (`0x1`).

- `Device::WaitForVBlank`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = swap chain index` (`swap_chain_index`)

- `Device::GetRasterStatus`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = hSwapChain.pDrvPrivate` (or swap chain pointer, depending on header/runtime)
  - `a2 = pRasterStatus` (output struct pointer)

- `Device::CreateStateBlock`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = state block type` (`D3DSBT_ALL=1`, `D3DSBT_PIXELSTATE=2`, `D3DSBT_VERTEXSTATE=3`)
  - `a2 = out stateblock handle pointer` (either `phStateBlock` or the CreateStateBlock args struct pointer)
  - `a3 = (unused)`

- `Device::BeginStateBlock`
  - `a0 = hDevice.pDrvPrivate`

- `Device::EndStateBlock`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = out stateblock handle pointer` (`phStateBlock`)

- `Device::ApplyStateBlock` / `Device::CaptureStateBlock` / `Device::DeleteStateBlock`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = hStateBlock.pDrvPrivate`

- `Device::ValidateDevice`
  - `a0 = hDevice.pDrvPrivate`
  - `a1 = out pass count pointer` (either `pNumPasses` or the ValidateDevice args struct pointer)

- Legacy fixed-function state (cached for Get*/StateBlock compatibility; subsets are consumed by fixed-function emulation):
  - `Device::SetTextureStageState`: the stage 0 `D3DTSS_*` subset can affect fixed-function pixel shader selection.
  - `Device::SetTransform` / `Device::MultiplyTransform`: transforms can be consumed by fixed-function WVP paths (for
    fixed-function `D3DFVF_XYZ*` draws and the fixed-function `ProcessVertices` subset).
  - `Device::SetMaterial` / `Device::SetLight` / `Device::LightEnable`: consumed by the minimal fixed-function lighting
    bring-up subset (`D3DFVF_XYZ | D3DFVF_NORMAL{,DIFFUSE}{,TEX1}` when `D3DRS_LIGHTING` is enabled).
  - `Device::SetTextureStageState` / `Device::GetTextureStageState`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = pack_u32_u32(stage, state)`
    - `a2 = value` (Set) or `pValue` (Get)
  - `Device::SetTransform` / `Device::MultiplyTransform` / `Device::GetTransform`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = transform state id` (`D3DTRANSFORMSTATETYPE` numeric value)
    - `a2 = matrix pointer`
  - `Device::SetMaterial` / `Device::GetMaterial`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = material pointer` (`pMaterial`)
  - `Device::SetLight` / `Device::GetLight`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = light index`
    - `a2 = light pointer` (`pLight`)
  - `Device::LightEnable` / `Device::GetLightEnable`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = light index`
    - `a2 = enabled` (Set) or `pEnabled` (Get)
  - `Device::SetClipPlane` / `Device::GetClipPlane`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = plane index`
    - `a2 = plane pointer`
  - `Device::SetStreamSourceFreq` / `Device::GetStreamSourceFreq`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = stream index`
    - `a2 = value` (Set) or `pValue` (Get)
  - `Device::SetSoftwareVertexProcessing` / `Device::GetSoftwareVertexProcessing`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = enabled` (Set) or `pEnabled` (Get)
  - `Device::SetNPatchMode` / `Device::GetNPatchMode`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = mode` (Set) or `pMode` (Get)
  - `Device::SetShaderConstI/B` / `Device::GetShaderConstI/B`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = shader stage` (VS=0, PS=1)
    - `a2 = pack_u32_u32(start_register, count)`
    - `a3 = data pointer`

- Legacy cached state (not emitted to the AeroGPU command stream):
  - `Device::SetPaletteEntries` / `Device::GetPaletteEntries`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = palette index`
    - `a2 = entries pointer`
  - `Device::SetCurrentTexturePalette` / `Device::GetCurrentTexturePalette`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = palette index` (Set) or `pPalette` pointer (Get)
  - `Device::SetClipStatus` / `Device::GetClipStatus`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = clip status pointer`
  - `Device::SetGammaRamp` / `Device::GetGammaRamp`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = arg1` (runtime-specific)
    - `a2 = arg2` (runtime-specific)
    - `a3 = gamma ramp pointer`

- Resource priority + autogen filter type (cached-only):
  - `Device::SetPriority` / `Device::GetPriority`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = hResource.pDrvPrivate`
    - `a2 = new priority` (Set) or `pPriority` pointer (Get)
    - `a3 = pOldPriority` pointer (Set, when present)
  - `Device::SetAutoGenFilterType` / `Device::GetAutoGenFilterType`
    - `a0 = hDevice.pDrvPrivate`
    - `a1 = hResource.pDrvPrivate`
    - `a2 = new filter type` (Set) or `pFilterType` pointer (Get)

The exact packing per entrypoint is defined where the DDI is instrumented:
`drivers/aerogpu/umd/d3d9/src/aerogpu_d3d9_driver.cpp` (search for `D3d9TraceCall`).

---

### How to use this to drive implementation

1. Run `d3d9ex_dwm_probe` (or `dwm.exe`) with `AEROGPU_D3D9_TRACE_MODE=unique`.
2. Dump the trace (present-trigger recommended).
3. Treat the resulting call list as your **bring-up checklist**:
   - Any entrypoints that appear in the trace must be correct/stable for DWM.
   - If you see repeated failures (`hr != S_OK`) for a call, that’s often the *next missing feature*.
4. Iterate:
   - Add support for the next DDI/caps struct/state that the trace indicates is being queried or used.
   - Re-run and compare traces.

## Minimal Direct3D 10 and 11 driver surface

This document is an implementation-oriented checklist/spec for bringing up **Direct3D 10** and **Direct3D 11** on **Windows 7** in the AeroGPU WDDM stack, targeting a “stub → triangle → real apps” path.

**Scope (minimal):**

* Windows 7 SP1, **WDDM 1.1**, **DXGI 1.1**
* D3D10 runtime + D3D10 UMD DDI (`d3d10umddi.h`)
* D3D11 runtime + D3D11 UMD DDI (`d3d11umddi.h`) with initial feature level **FL10_0**
* Shader models **SM4.x** first (`vs_4_0`, `ps_4_0`, optional `gs_4_0`), roadmap to **SM5.0** (`*_5_0`)
* **Windowed swapchain only** initially (DWM composition)

**Non-goals (initial bring-up):**

* Exclusive fullscreen / flip-model swapchains (Win8+ only)
* Tessellation (HS/DS), UAV-heavy compute, tiled resources, video decode, DXGI 1.2+
* Performance tuning beyond “correct enough” to run common apps

**Related AeroGPU docs (read alongside this one):**

* `windows7-aerogpu-wddm-driver.md` — KMD+UMD architecture, memory model, fence/vblank requirements, and the guest↔emulator command transport.
* `windows7-aerogpu-validation.md` — bring-up/stability checklist (TDR avoidance, vblank pacing, debug playbook).
* `windows7-d3d-umd-ddi.md` — deprecated redirect (kept for link compatibility; points at the focused allocation + Map/Unmap docs).
* `windows7-d3d-umd-ddi.md` — CreateResource-side allocation contract details (allocation-info arrays, `pfnAllocateCb`/`pfnDeallocateCb`, and `DXGI_DDI_PRIMARY_DESC` primaries/backbuffers).
* `windows7-d3d-umd-ddi.md` — trace guide + invariants for Win7 DXGI swapchain backbuffer `CreateResource` parameters and allocation flags.
* `windows7-d3d-umd-ddi.md` — Win7 D3D11 `Map`/`Unmap` contract (`pfnLockCb`/`pfnUnlockCb`, DO_NOT_WAIT, staging readback sync).
* `windows7-d3d-umd-ddi.md` — D3D11 `d3d11umddi.h` function-table checklist (which entries must be non-null vs safely stubbed for FL10_0 bring-up).
* `windows7-d3d-umd-ddi.md` — Win7 WDK symbol-name reference for D3D10/11 UMD callbacks (submission, fences, `SetErrorCb`, WOW64 gotchas).
* `windows7-d3d-umd-ddi.md` — how to enable `GetCaps` + entrypoint tracing in the D3D10/11 UMD during Win7 bring-up.
* `aerogpu-device-abi.md` — Win7 shared-surface strategy (stable cross-process `share_token` vs user-mode shared `HANDLE` numeric values).

> Header references: the names in this doc match the WDK user-mode DDI headers:
> `d3d10umddi.h`, `d3d10_1umddi.h` (optional), `d3d11umddi.h`, and for swapchain/present: `dxgiddi.h`.

---

### Driver model overview (Windows 7 / WDDM 1.1)

#### 1.1 Where DXGI, D3D10/11 runtime, and the UMD fit

On Windows 7 the graphics API call flow (windowed) is roughly:

```
App
  ├─ D3D10 / D3D11 API (d3d10.dll / d3d11.dll)
  │    └─ D3D10/11 Runtime (validates, marshals, batching)
  │         └─ UMD DDI calls (your DLL: atidxx*, nvumd*, etc)
  │              └─ KMD via D3DKMT thunks / command submission
  └─ DXGI (dxgi.dll)
       ├─ adapter enumeration (IDXGIFactory/IDXGIAdapter)
       └─ swapchain creation/present (IDXGISwapChain)
            └─ DXGI DDI (dxgiddi.h) into the display driver stack
```

**Key idea:** the D3D runtime drives almost everything via **DDI function tables** you provide. The UMD does *not* expose the D3D API; it exposes **DDI entrypoints** and implements the low-level semantics (object creation, state binding, draws, resource updates, etc).

#### 1.2 UMD entrypoints (“exports”) that the OS/runtime loads

At minimum, provide exports matching the DDI your driver supports:

* D3D10: `OpenAdapter10` (and optionally `OpenAdapter10_2` for 10.1)
  * signature: `HRESULT APIENTRY OpenAdapter10(D3D10DDIARG_OPENADAPTER *pOpenData)`
* D3D11: `OpenAdapter11`
  * signature: `HRESULT APIENTRY OpenAdapter11(D3D10DDIARG_OPENADAPTER *pOpenData)`
  * note: on Win7/WDDM 1.1 the D3D11 runtime still uses the `D3D10DDIARG_OPENADAPTER` container for adapter open; the D3D11-specific DDI begins with `D3D11DDIARG_CREATEDEVICE` / `D3D11DDIARG_GETCAPS`.

These are declared in the WDK headers (`d3d10umddi.h`, `d3d11umddi.h`) and receive a single `_Inout_ ...ARG_OPENADAPTER*` which contains:

* pointers to the runtime **callback tables** (`D3D10DDI_ADAPTERCALLBACKS` / `D3D11DDI_ADAPTERCALLBACKS`)
* a place for you to return the **adapter function table** (`D3D10DDI_ADAPTERFUNCS` / `D3D11DDI_ADAPTERFUNCS`)
* interface version negotiation (e.g. `D3D10DDI_INTERFACE_VERSION`, `D3D11DDI_INTERFACE_VERSION` / `D3D11DDI_SUPPORTED`)

From there, the runtime calls your adapter funcs’ `pfnCreateDevice(...)`, and device creation returns a `D3D10DDI_DEVICEFUNCS` / `D3D11DDI_DEVICEFUNCS` table plus your private `D3D*DDI_HDEVICE`.

#### 1.3 “Handle + private memory” object model (critical for bring-up)

In the D3D10/11 DDI, most API objects are opaque handles such as:

* `D3D10DDI_HDEVICE`, `D3D10DDI_HRESOURCE`, `D3D10DDI_HRENDERTARGETVIEW`, `D3D10DDI_HVERTEXSHADER`, …
* `D3D11DDI_HDEVICE`, `D3D11DDI_HRESOURCE`, `D3D11DDI_HRENDERTARGETVIEW`, `D3D11DDI_HVERTEXSHADER`, …

The runtime owns handle allocation and typically also allocates **driver-private storage** for each object:

1. Runtime calls `pfnCalcPrivate*Size(...)` (e.g. `pfnCalcPrivateResourceSize`) to ask how many bytes you need.
2. Runtime allocates that many bytes and stores the pointer in the handle (e.g. `hResource.pDrvPrivate`).
3. Runtime calls `pfnCreate*(..., hXxx, hRTXxx)` and you write your object into `hXxx.pDrvPrivate`.

**Implication for AeroGPU:** implement a consistent “private object header” layout that stores:

* a stable object type tag (debugging)
* a host/emulator object ID (for the WebGPU side)
* resource/view descriptors that the translator needs at draw time

#### 1.4 Error reporting rules

Many DDI entrypoints are `void` and cannot return `HRESULT`. When failing such a call, the driver must report the error via the runtime callback (commonly `pfnSetErrorCb(...)`) and then return.

For the Win7/WDDM 1.1 **exact** callback/table names involved (`D3D*DDIARG_CREATEDEVICE::pCallbacks->pfnSetErrorCb`, and the related submission + fence wait callbacks in `d3dumddi.h`), see:

* `windows7-d3d-umd-ddi.md`

For DDI functions that *do* return `HRESULT`, return:

* `S_OK` on success
* `E_OUTOFMEMORY`, `E_INVALIDARG`, or `E_NOTIMPL` as appropriate for unsupported features
  * AeroGPU note: if you see `E_OUTOFMEMORY` “too early” (while the guest still has free RAM), you may be hitting Win7’s WDDM segment budget rather than true exhaustion. AeroGPU is system-memory-backed, but dxgkrnl still enforces the KMD-reported non-local segment size; tune `HKR\Parameters\NonLocalMemorySizeMB` (see `windows7-aerogpu-validation.md` appendix and `drivers/aerogpu/kmd/README.md`).

#### 1.5 AeroGPU-specific implementation layering (UMD → KMD → emulator)

This doc focuses on the *API contract* (D3D10/11 DDI) that the Microsoft runtimes will call. The implementation behind those entrypoints in AeroGPU should follow the existing project architecture:

* The UMD is primarily a **state tracker + command encoder**:
  * consume DDI calls
  * validate/normalize state
  * emit an **AeroGPU-specific command stream** (IR) suitable for execution by the emulator
* The KMD is primarily **submission + memory bookkeeping plumbing** (WDDM 1.1):
  * accept DMA buffers / submission packets from the runtime
  * provide a stable fence + interrupt completion path (avoid TDRs)
  * build a per-submission allocation table keyed by stable `alloc_id` values (see `drivers/aerogpu/protocol/aerogpu_ring.h`) and provide it via the submit descriptor, so command packets can reference guest-backed memory via `backing_alloc_id` (ABI details: `drivers/aerogpu/protocol/README.md`)

Practical implication for D3D10/11 bring-up: whenever this doc says “flush/submit”, the concrete implementation should enqueue a bounded unit of work to the emulator and ensure the WDDM-visible fence monotonically advances.

#### 1.6 AeroGPU device discovery (UMDRIVERPRIVATE)

UMDs must not assume optional features like vblank timing and fence pages exist, or a specific AeroGPU BAR0 ABI (legacy `"ARGP"` vs versioned `"AGPU"`; legacy is optional and feature-gated behind `emulator/aerogpu-legacy`).

During adapter open, query:

* `D3DKMTQueryAdapterInfo(KMTQAITYPE_UMDRIVERPRIVATE)`

 and decode the returned `aerogpu_umd_private_v1` (see `drivers/aerogpu/protocol/aerogpu_umd_private.h`). Use the reported feature bits to gate optional runtime behavior (e.g. vblank-paced present paths).

When validating the blob, require `struct_version == 1` and `size_bytes >= sizeof(aerogpu_umd_private_v1)` (not an exact match), so v1-compatible extensions that append trailing bytes remain usable.
  
---

### Minimum D3D10DDI + D3D11DDI entrypoints (Win7 bring-up set)

This section enumerates the **minimum practical** entrypoints to get:

* device creation
* resource creation
* shader creation/binding
* pipeline state
* basic draws
* swapchain-backed present

It also marks which entrypoints can initially return **NOT_SUPPORTED** and what that implies for feature levels / capabilities.

#### 2.1 D3D10: adapter + device entrypoints (D3D10DDI)

##### 2.1.1 Mandatory exports / adapter functions

* Export: `OpenAdapter10` (from `d3d10umddi.h`)
* Adapter function table (`D3D10DDI_ADAPTERFUNCS`) must minimally provide:
  * `pfnGetCaps` → handles `D3D10DDIARG_GETCAPS`
  * `pfnCalcPrivateDeviceSize`
  * `pfnCreateDevice` → fills `D3D10DDIARG_CREATEDEVICE` and returns `D3D10DDI_DEVICEFUNCS`
  * `pfnCloseAdapter`

**Initially NOT_SUPPORTED (safe to stub):**

* D3D10.1 specific negotiation (skip `OpenAdapter10_2` / `d3d10_1umddi.h` initially)

##### 2.1.2 Mandatory device/object creation (private-size + create + destroy)

To render anything, implement the “calc/create/destroy” triads for:

Resources
* `pfnCalcPrivateResourceSize` + `pfnCreateResource` + `pfnDestroyResource`
  * struct: `D3D10DDIARG_CREATERESOURCE`
  * note: destroy is typically `pfnDestroyResource(D3D10DDI_HDEVICE, D3D10DDI_HRESOURCE)` (no `*_ARG_DESTROY*` structure)

Views
* `pfnCalcPrivateShaderResourceViewSize` + `pfnCreateShaderResourceView` + `pfnDestroyShaderResourceView`
  * `D3D10DDIARG_CREATESHADERRESOURCEVIEW`
* `pfnCalcPrivateRenderTargetViewSize` + `pfnCreateRenderTargetView` + `pfnDestroyRenderTargetView`
  * `D3D10DDIARG_CREATERENDERTARGETVIEW`
* `pfnCalcPrivateDepthStencilViewSize` + `pfnCreateDepthStencilView` + `pfnDestroyDepthStencilView`
  * `D3D10DDIARG_CREATEDEPTHSTENCILVIEW`

Shaders
* `pfnCalcPrivateVertexShaderSize` + `pfnCreateVertexShader` + `pfnDestroyVertexShader`
  * `D3D10DDIARG_CREATEVERTEXSHADER`
* `pfnCalcPrivatePixelShaderSize` + `pfnCreatePixelShader` + `pfnDestroyPixelShader`
  * `D3D10DDIARG_CREATEPIXELSHADER`

Pipeline state
* `pfnCalcPrivateElementLayoutSize` + `pfnCreateElementLayout` + `pfnDestroyElementLayout`
  * `D3D10DDIARG_CREATEELEMENTLAYOUT`
* `pfnCalcPrivateSamplerSize` + `pfnCreateSampler` + `pfnDestroySampler`
  * `D3D10DDIARG_CREATESAMPLER`
* `pfnCalcPrivateRasterizerStateSize` + `pfnCreateRasterizerState` + `pfnDestroyRasterizerState`
  * `D3D10DDIARG_CREATERASTERIZERSTATE`
* `pfnCalcPrivateBlendStateSize` + `pfnCreateBlendState` + `pfnDestroyBlendState`
  * `D3D10DDIARG_CREATEBLENDSTATE`
* `pfnCalcPrivateDepthStencilStateSize` + `pfnCreateDepthStencilState` + `pfnDestroyDepthStencilState`
  * `D3D10DDIARG_CREATEDEPTHSTENCILSTATE`

**Initially NOT_SUPPORTED (safe to stub):**

* Geometry shader object creation:
  * `pfnCalcPrivateGeometryShaderSize` + `pfnCreateGeometryShader` + `pfnDestroyGeometryShader`
  * note: the **GS stage exists in D3D10**, but many “first triangle” tests never create/bind a GS (it is valid to have no GS bound).
  * AeroGPU note: WebGPU has no geometry shader stage. The command stream can encode a GS handle via
    `BIND_SHADERS` (legacy: `aerogpu_cmd_bind_shaders::reserved0`; newer streams may append `{gs,hs,ds}`
    after the stable 24-byte prefix). When appended handles are present they are authoritative; producers
    may optionally mirror `gs` into `reserved0` for best-effort compatibility with legacy hosts.
    GS DXBC is forwarded to the host. The host has compute-prepass plumbing for GS/HS/DS emulation; a minimal translator-backed GS prepass is executed for a small set of IA input topologies (`PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj`) for both `Draw` and `DrawIndexed` when supported, but broader GS DXBC execution is still bring-up work.
    See [`geometry-shader-emulation.md`](compute-expansion-emulation.md) and [`direct3d-10-11-translation.md`](direct3d-10-11-translation.md).
* Stream-output state / SO buffers (`pfnSoSetTargets`, etc)
* Queries/predication:
  * `pfnCreateQuery` / `pfnDestroyQuery`, `pfnBegin` / `pfnEnd`, `pfnSetPredication`

##### 2.1.3 Mandatory context/state binding + draw path

Minimal pipeline binding (D3D10DDI_DEVICEFUNCS):

Input Assembler
* `pfnIaSetInputLayout`
* `pfnIaSetVertexBuffers`
* `pfnIaSetIndexBuffer`
* `pfnIaSetTopology` (sets `D3D*_PRIMITIVE_TOPOLOGY`; corresponds to `IASetPrimitiveTopology` at the API level)

Shaders
* `pfnVsSetShader`
* `pfnPsSetShader`
* `pfnVsSetConstantBuffers`
* `pfnPsSetConstantBuffers`
* `pfnVsSetShaderResources` / `pfnPsSetShaderResources` (for texture test)
* `pfnVsSetSamplers` / `pfnPsSetSamplers` (for texture test)

Rasterizer / Output merger
* `pfnSetViewports`
* `pfnSetScissorRects` (can be ignored initially if you always clamp to the viewport)
* `pfnSetRasterizerState`
* `pfnSetBlendState`
* `pfnSetDepthStencilState`
* `pfnSetRenderTargets` (RTVs + DSV)

Clears and draws
* `pfnClearRenderTargetView`
* `pfnClearDepthStencilView` (needed for depth test app)
* `pfnDraw`
* `pfnDrawIndexed` (many samples use indexed draws)

Presentation / swapchain integration
* `pfnPresent` (DXGI ultimately drives this from `IDXGISwapChain::Present`)
  * struct: `D3D10DDIARG_PRESENT` (used by DXGI for both D3D10 and D3D11 devices on Win7)
* `pfnRotateResourceIdentities`
  * used by DXGI swapchains to rotate backbuffer “resource identities” after present without requiring a full copy

Resource update/copy (minimum)
* `pfnMap` + `pfnUnmap` (dynamic VB/IB/CB uploads) — `D3D10DDIARG_MAP`
* `pfnUpdateSubresourceUP` (user-memory upload path; some apps prefer this over map/unmap)
  * struct: `D3D10DDIARG_UPDATESUBRESOURCEUP`
* `pfnCopyResource` / `pfnCopySubresourceRegion` (optional but commonly used internally by runtimes)

See also:

* `windows7-d3d-umd-ddi.md` — deprecated redirect (kept for link compatibility; points at the focused docs below).
* `windows7-d3d-umd-ddi.md` — Win7/WDDM 1.1 resource allocation (`CreateResource` → `pfnAllocateCb`) contract.
* `windows7-d3d-umd-ddi.md` — Win7 `Map`/`Unmap` semantics (`LockCb`/`UnlockCb`) for dynamic uploads + staging readback.

Command submission
* `pfnFlush` (or equivalent submit/flush entrypoint in the DDI) to ensure GPU work reaches the KMD/host.

##### 2.1.4 AeroGPU allocation-backed resources (alloc_id semantics + dirty ranges)

For AeroGPU’s command stream, D3D10/11 resources (buffers, textures) are expected to be **backed by real WDDM allocations** so the emulator/host can:

* read CPU-written contents directly from guest memory (uploads), and
* write GPU results back into guest allocations (staging readback / correctness).

**Key decision:** the AeroGPU allocation table `alloc_id` (and `backing_alloc_id` in `AEROGPU_CMD_CREATE_*`) is a **stable driver-defined `u32` ID**, not a per-submit index and not an OS handle value.

* `backing_alloc_id == alloc_id` (stable `u32`; `0` means “host allocated”).
* On Win7/WDDM 1.1, the UMD provides this ID to the KMD via the **allocation private driver data blob** (`aerogpu_wddm_alloc_priv.alloc_id` in `drivers/aerogpu/protocol/aerogpu_wddm_alloc.h`).
  * This is required because the numeric value of the UMD-visible allocation handle (`D3DKMT_HANDLE` from `pfnAllocateCb`) is **not** the same identity the KMD later sees in `DXGK_ALLOCATIONLIST`.
* On every submission, the UMD must provide a `D3DDDI_ALLOCATIONLIST` containing the referenced `hAllocation` handles so the KMD can read the private driver data and build the per-submit allocation table.
* The KMD then emits a per-submit allocation table mapping `alloc_id → {gpa, size}`; the host resolves guest memory by `alloc_id`, not by allocation-list position.

See also:

* `aerogpu-device-abi.md` — authoritative `backing_alloc_id` / `alloc_id` semantics and host-side resolution rules.

**Dirty range notifications (MVP):**

* For resources backed by guest memory (`backing_alloc_id != 0`), any `Map` that permits CPU writing (`WRITE`, `WRITE_DISCARD`, `WRITE_NO_OVERWRITE`) must emit `AEROGPU_CMD_RESOURCE_DIRTY_RANGE` on `Unmap`.
  * For host-owned resources (`backing_alloc_id == 0`, e.g. dynamic buffers in the bring-up path), the UMD must upload bytes explicitly via `AEROGPU_CMD_UPLOAD_RESOURCE` instead of relying on dirty ranges.
* For MVP, mark the entire allocation dirty:
  * `offset_bytes = 0`
  * `size_bytes = allocation_size`
* If `CreateResource` copies initial data into the allocation, emit one `RESOURCE_DIRTY_RANGE` after the upload.

**Staging readback / `CopyResource` (MVP):**

* If the UMD uses a staging resource backed by guest memory (`backing_alloc_id != 0`) for `Map(READ)`-style readback, the `CopyResource`/`CopySubresourceRegion` path must emit:
  * `AEROGPU_CMD_COPY_BUFFER` / `AEROGPU_CMD_COPY_TEXTURE2D` with `AEROGPU_COPY_FLAG_WRITEBACK_DST`, so the host writes the copied bytes back into guest memory before signaling the fence.
* The submission must include an allocation-table entry for the destination resource’s `backing_alloc_id` (Win7: include the WDDM allocation handle in the submit allocation list).
  * The destination allocation must be marked writable for the submission (`WriteOperation` bit set in the WDDM allocation list); otherwise the KMD will mark it `AEROGPU_ALLOC_FLAG_READONLY` and the host will reject the writeback.

#### 2.2 D3D11: adapter + device/context entrypoints (D3D11DDI)

For a **table-by-table** checklist of which `d3d11umddi.h` function pointers must be non-null vs safely stubbable for a crash-free Win7 bring-up (FL10_0), see:
* `windows7-d3d-umd-ddi.md`

##### 2.2.1 Mandatory exports / adapter functions

* Export: `OpenAdapter11` (from `d3d11umddi.h`)
* Adapter function table (`D3D11DDI_ADAPTERFUNCS`) must minimally provide:
  * `pfnGetCaps` → handles `D3D11DDIARG_GETCAPS`
    * must report supported `D3D_FEATURE_LEVEL` list.
      * Minimal target: **`D3D_FEATURE_LEVEL_10_0` only**.
      * If you are intentionally gating out geometry shaders (e.g. host backend without compute pipelines), advertise only `D3D_FEATURE_LEVEL_9_x` until GS emulation is supported.
  * `pfnCalcPrivateDeviceSize`
  * `pfnCreateDevice` → uses `D3D11DDIARG_CREATEDEVICE`
    * `D3D11DDIARG_CREATEDEVICE` is where the driver returns both:
      * `D3D11DDI_DEVICEFUNCS` (device/object creation)
      * `D3D11DDI_DEVICECONTEXTFUNCS` (immediate context draw/state/update entrypoints)
  * `pfnCloseAdapter`

##### 2.2.2 Mandatory device/object creation

The D3D11 DDI is structurally similar to D3D10, with additional shader stages and optional view types.

**Important D3D11 DDI split (device vs immediate context):**

* Object creation/destruction lives on the **device function table** (`D3D11DDI_DEVICEFUNCS`) and is typically called with a `D3D11DDI_HDEVICE`.
* Most draw/clear/update/state-binding calls live on the **immediate context function table** (`D3D11DDI_DEVICECONTEXTFUNCS`) and are called with a `D3D11DDI_HDEVICECONTEXT`.

When implementing the Win7 D3D11 UMD, treat “device” and “context” as separate state holders even if your backend is single-threaded; it avoids conflating lifetime (device objects) with per-command-stream state (bindings and draws).

Resources
* `pfnCalcPrivateResourceSize` + `pfnCreateResource` + `pfnDestroyResource`
  * `D3D11DDIARG_CREATERESOURCE`

Views
* `pfnCalcPrivateShaderResourceViewSize` + `pfnCreateShaderResourceView` + `pfnDestroyShaderResourceView`
  * `D3D11DDIARG_CREATESHADERRESOURCEVIEW`
* `pfnCalcPrivateRenderTargetViewSize` + `pfnCreateRenderTargetView` + `pfnDestroyRenderTargetView`
  * `D3D11DDIARG_CREATERENDERTARGETVIEW`
* `pfnCalcPrivateDepthStencilViewSize` + `pfnCreateDepthStencilView` + `pfnDestroyDepthStencilView`
  * `D3D11DDIARG_CREATEDEPTHSTENCILVIEW`

Shaders (initial bring-up)
* `pfnCalcPrivateVertexShaderSize` + `pfnCreateVertexShader` + `pfnDestroyVertexShader`
  * `D3D11DDIARG_CREATEVERTEXSHADER`
* `pfnCalcPrivatePixelShaderSize` + `pfnCreatePixelShader` + `pfnDestroyPixelShader`
  * `D3D11DDIARG_CREATEPIXELSHADER`
* Geometry shader (required for `D3D_FEATURE_LEVEL_10_0` and above; can be deferred only if you advertise `D3D_FEATURE_LEVEL_9_x`)
  * `pfnCalcPrivateGeometryShaderSize` + `pfnCreateGeometryShader` + `pfnDestroyGeometryShader`
  * `D3D11DDIARG_CREATEGEOMETRYSHADER`

Pipeline state
* `pfnCalcPrivateElementLayoutSize` + `pfnCreateElementLayout` + `pfnDestroyElementLayout`
  * `D3D11DDIARG_CREATEELEMENTLAYOUT`
* `pfnCalcPrivateSamplerSize` + `pfnCreateSampler` + `pfnDestroySampler`
  * `D3D11DDIARG_CREATESAMPLER`
* `pfnCalcPrivateRasterizerStateSize` + `pfnCreateRasterizerState` + `pfnDestroyRasterizerState`
  * `D3D11DDIARG_CREATERASTERIZERSTATE`
* `pfnCalcPrivateBlendStateSize` + `pfnCreateBlendState` + `pfnDestroyBlendState`
  * `D3D11DDIARG_CREATEBLENDSTATE`
* `pfnCalcPrivateDepthStencilStateSize` + `pfnCreateDepthStencilState` + `pfnDestroyDepthStencilState`
  * `D3D11DDIARG_CREATEDEPTHSTENCILSTATE`

**Initially NOT_SUPPORTED (recommended):**

These can return `E_NOTIMPL` / set error until the driver claims a higher feature level (or otherwise advertises the corresponding capability as unsupported).

* Tessellation stages (requires FL11_0):
  * `pfnCreateHullShader` / `D3D11DDIARG_CREATEHULLSHADER`
  * `pfnCreateDomainShader` / `D3D11DDIARG_CREATEDOMAINSHADER`
  * `pfnHsSetShader`, `pfnDsSetShader`, and related CB/SRV/sampler bind calls
* Compute shader stage (roadmap item):
  * `pfnCreateComputeShader` / `D3D11DDIARG_CREATECOMPUTESHADER`
  * `pfnCsSetShader`, UAV binding, dispatch calls
* UAVs:
  * `pfnCalcPrivateUnorderedAccessViewSize` / `pfnCreateUnorderedAccessView` / `pfnDestroyUnorderedAccessView`
  * `D3D11DDIARG_CREATEUNORDEREDACCESSVIEW`

Geometry shader note:

* Geometry shaders are part of the D3D10-class pipeline and are expected at `D3D_FEATURE_LEVEL_10_0` and above.
* If you advertise **FL10_0** (or higher) from `pfnGetCaps`, implement `pfnCreateGeometryShader` / `D3D11DDIARG_CREATEGEOMETRYSHADER` (and the corresponding bind/state entrypoints) even if the first implementation is “limited but functional”.
  * **Outdated MVP note:** earlier bring-up guidance suggested “accept GS creation but ignore it”. This is no longer viable once you run real GS workloads and the Win7 regression tests below.
  * **AeroGPU approach (target): forward GS DXBC and emulate on the host (WebGPU).**
    * The guest UMD treats GS like VS/PS: it forwards the DXBC blob to the host and participates in normal shader lifetime + binding.
    * Since WebGPU has **no GS stage**, AeroGPU emulates GS by inserting a **compute prepass** when a GS is bound:
      - The executor already routes draws with a bound GS through a compute-prepass + indirect-draw path.
      - The in-tree GS DXBC→WGSL compute translator exists and is partially integrated: translation is attempted at `CREATE_SHADER_DXBC`, and draws using supported IA input topologies (`PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj`) can execute the translated compute prepass (minimal SM4 subset) for both `Draw` and `DrawIndexed`. If GS translation fails, draws with that GS bound currently return a clear error; other draws still use a synthetic expansion shader for bring-up/coverage (guest GS DXBC does not execute).
      - The intended end state is: VS-as-compute (vertex pulling) → GS-as-compute (primitive expansion) → render expanded buffers with a passthrough VS + the original PS.
    * This is **internal** WebGPU compute; it does *not* require exposing the D3D11 compute shader stage (you can still keep D3D11 CS as `NOT_SUPPORTED` initially).
    * Details: [`geometry-shader-emulation.md`](compute-expansion-emulation.md).
    * **Current repo status:** the host-side executor’s GS/HS/DS compute-prepass path uses synthetic expansion geometry for most draws, but `PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj` draws (both `Draw` and `DrawIndexed`) can execute a minimal translated SM4 GS subset. Feeding GS inputs from the bound VS is partially implemented via a minimal VS-as-compute path (simple VS subset), with an IA-fill fallback for strict passthrough VS. Broader GS DXBC execution is still WIP. Creating a GS that cannot be translated is supported for robustness, but draws with that GS bound currently return a clear “geometry shader not supported” error. See [`../areas/graphics.md`](../areas/graphics.md) and [`geometry-shader-emulation.md`](compute-expansion-emulation.md).
    * How to verify (host-side):
      ```bash
      bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_point_to_triangle --locked
      bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelist_emits_triangle --locked
      bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelistadj_emits_triangle --locked
      bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelist_emits_triangle --locked
      bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelistadj_emits_triangle --locked
      ```
* Win7 regression tests that define the minimum semantics to target:
  * `drivers/aerogpu/tests/win7/d3d11_geometry_shader_smoke` — basic GS create/bind/execute path.
  * `drivers/aerogpu/tests/win7/d3d11_geometry_shader_restart_strip` — validates `TriangleStream::RestartStrip` / DXBC `cut` handling.
    * This matters because the intended AeroGPU GS emulation expands `triangle_strip` output into a list topology for rendering; if you drop the cut/restart markers you can generate “bridging” triangles between strips (visible corruption, and this test fails by detecting pixels filled in the gap between two emitted strips).
* If you are not ready to support GS (e.g. host backend without compute), prefer advertising only `D3D_FEATURE_LEVEL_9_x` for D3D11 (while still supporting D3D10 separately), or be explicit that some FL10_0 apps will fail when they create/bind GS.

##### 2.2.3 Mandatory context/state binding + draw path

At FL10_0, D3D11 essentially needs the D3D10-era pipeline:

Immediate context function table: `D3D11DDI_DEVICECONTEXTFUNCS`

Input Assembler
* `pfnIaSetInputLayout`
* `pfnIaSetVertexBuffers`
* `pfnIaSetIndexBuffer`
* `pfnIaSetTopology` (sets `D3D*_PRIMITIVE_TOPOLOGY`; corresponds to `IASetPrimitiveTopology` at the API level)

Shaders + resource binding (VS/PS, plus GS if advertising FL10_0+)
* `pfnVsSetShader`, `pfnPsSetShader`
* `pfnVsSetConstantBuffers`, `pfnPsSetConstantBuffers`
* `pfnVsSetShaderResources`, `pfnPsSetShaderResources`
* `pfnVsSetSamplers`, `pfnPsSetSamplers`

Geometry shader stage (required for FL10_0+; optional only if you advertise FL9_x)
* `pfnGsSetShader`
* `pfnGsSetConstantBuffers`
* `pfnGsSetShaderResources`
* `pfnGsSetSamplers`

Rasterizer / Output merger
* `pfnSetViewports`, `pfnSetScissorRects`
* `pfnSetRasterizerState`
* `pfnSetBlendState`
* `pfnSetDepthStencilState`
* `pfnSetRenderTargets` (or the D3D11 DDI equivalent of OMSetRenderTargets)

Clears and draws
* `pfnClearRenderTargetView`
* `pfnClearDepthStencilView`
* `pfnDraw`, `pfnDrawIndexed`

Resource updates
* `pfnMap` + `pfnUnmap` (see [`win7-d3d11-map-unmap.md`](windows7-d3d-umd-ddi.md) for the definitive Win7 Map/Unmap + `LockCb`/`UnlockCb` contract)
  * must cover both:
    * dynamic update patterns (`D3D11_MAP_WRITE_DISCARD` / `D3D11_MAP_WRITE_NO_OVERWRITE`) for buffers/constant buffers, and
    * staging readback (`D3D11_MAP_READ` on `D3D11_USAGE_STAGING` resources) for tests and debugging
      * Win7 fence wait reference (exact CB struct + field names): [`win7-d3d10-11-umd-callbacks-and-fences.md`](windows7-d3d-umd-ddi.md)
* `pfnUpdateSubresourceUP` (user-memory upload path for `UpdateSubresource`)
  * struct: `D3D11DDIARG_UPDATESUBRESOURCEUP`
* `pfnCopyResource` / `pfnCopySubresourceRegion`
* `pfnFlush` (submits pending work; corresponds to `ID3D11DeviceContext::Flush`)

See also:

* `windows7-d3d-umd-ddi.md` — Win7/WDDM 1.1 resource allocation (`CreateResource` → `pfnAllocateCb`) contract.
* `windows7-d3d-umd-ddi.md` — Win7 `Map`/`Unmap` semantics (`LockCb`/`UnlockCb`) for dynamic uploads + staging readback.

---

### Swapchain + Present path (DXGI 1.1 expectations)

#### 3.1 Minimum DXGI swapchain behavior to target first

For Windows 7, a minimal implementation should accept (and test against) swapchains created with:

* `DXGI_SWAP_CHAIN_DESC::Windowed = TRUE`
* `DXGI_SWAP_CHAIN_DESC::SwapEffect = DXGI_SWAP_EFFECT_DISCARD`
* `DXGI_SWAP_CHAIN_DESC::BufferCount = 1` (most common on Win7 for DISCARD)
* `DXGI_SWAP_CHAIN_DESC::SampleDesc.Count = 1` (no MSAA initially)
* Common formats:
  * `DXGI_FORMAT_B8G8R8A8_UNORM` (very common for DWM)
  * `DXGI_FORMAT_R8G8B8A8_UNORM`
* `DXGI_USAGE_RENDER_TARGET_OUTPUT` (plus optionally `DXGI_USAGE_SHADER_INPUT`)

**Initially NOT_SUPPORTED:**

* `IDXGISwapChain::SetFullscreenState(TRUE, ...)` → return `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE`
* `DXGI_SWAP_EFFECT_SEQUENTIAL` (can be implemented later, but DISCARD first is enough)
* MSAA swapchains (`SampleDesc.Count > 1`)

#### 3.2 Present semantics to match common apps

Apps will call `IDXGISwapChain::Present(SyncInterval, Flags)`:

* `SyncInterval = 0` (immediate) and `SyncInterval = 1` (vsync) are the most common.
* The driver must not crash on other values; clamp to 1 initially.
* `Flags` to handle:
  * `0` (normal)
  * `DXGI_PRESENT_TEST`: do not present; only validate (return `S_OK` if it *would* succeed)
  * `DXGI_PRESENT_DO_NOT_WAIT`: if you can’t queue immediately, return `DXGI_ERROR_WAS_STILL_DRAWING`

On Win7 **windowed** swapchains, Present is effectively “make the backbuffer visible to DWM”; the minimal path for AeroGPU is:

1. Ensure all pending rendering to the backbuffer is flushed/submitted (`pfnFlush` or equivalent).
2. Signal the host/emulator with the presented resource ID and dirty rectangle (if any).
3. Host composites to the browser canvas.

Implementation note: in practice DXGI will often call into the UMD’s D3D10/11 DDI device functions for present and (for multi-buffer scenarios) buffer rotation:

* `pfnPresent` (with `D3D10DDIARG_PRESENT`)
* `pfnRotateResourceIdentities` (rotate swapchain backbuffer resources)

#### 3.3 ResizeBuffers / ResizeTarget expectations

Apps commonly call:

* `IDXGISwapChain::ResizeBuffers(0, 0, 0, DXGI_FORMAT_UNKNOWN, Flags)` to “resize to window”
* `IDXGISwapChain::ResizeTarget(&DXGI_MODE_DESC)` sometimes, but can be accepted-and-ignored for windowed-only

Minimum rules:

* Old backbuffer resources must become invalid for rendering once resized.
* Any views created on old buffers must be destroyed/recreated by the runtime/app; driver should handle destruction cleanly.
* If the runtime creates the new buffers via the DXGI DDI (`dxgiddi.h`), ensure resource IDs change and the host knows the new size.

---

### Resource binding model (CBs, SRVs/UAVs, samplers, RTV/DSV)

#### 4.1 What the D3D10/11 runtime expects

The runtime sets pipeline bindings by stage, using arrays of handles and slot ranges. The driver must implement “overwrite slots” semantics:

* A bind call updates `[StartSlot, StartSlot + Num* )`.
* `NULL` handles in the array **unbind** that slot.
* Bindings persist until overwritten.

#### 4.2 Minimal state to track in the UMD

For an initial implementation, track only what basic rendering needs:

Input Assembler
* `ElementLayout` (input layout)
* Vertex buffers: handles + `(Stride, Offset)` per slot
* Index buffer: handle + format + offset
* Primitive topology

Shader stages (minimum VS/PS)
* Current VS/PS objects (store DXBC hash/ID)
* Constant buffer bindings (resources) per stage
* SRV bindings per stage
* Sampler bindings per stage

Output merger / rasterizer
* RTV array (up to `D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT`, usually 8)
* One DSV
* Blend state + blend factor + sample mask
* Depth/stencil state + stencil ref
* Rasterizer state
* Viewports + scissor rects

#### 4.3 UAVs (optional for FL10_0 bring-up; buffer UAVs first)

UAVs only become required once you support compute or advanced pixel pipeline features.

**Win7 bring-up recommendation:**

* Implement RTV/DSV/SRV first.
* For an initial FL10_0 path, it is valid to return NOT_SUPPORTED / `E_NOTIMPL` for `CreateUnorderedAccessView` and the `*SetUnorderedAccessViews` family.

When you do add compute + UAV support (FL11_0-era features), start with **buffer UAVs**:

* `u#` bindings for `RWByteAddressBuffer` / `RWStructuredBuffer`-like access (`u0..u7` in D3D11 compute).
* You can still defer **typed UAV textures** (`RWTexture*`) until the required format plumbing exists.

#### 4.4 Hazard rules (minimal correctness)

D3D10/11 disallow (or define undefined behavior for) some simultaneous bindings, notably:

* A resource bound as an RTV/DSV cannot be simultaneously bound as an SRV in the same pipeline where it would be read.

Minimum viable driver behavior:

* When binding an RTV/DSV, **auto-unbind** that resource from SRV slots (VS/PS) to avoid feedback loops.
* When binding an SRV, auto-unbind it from RTV slots if already bound.

This matches the “helpful driver” behavior many apps implicitly rely on, and avoids host-side validation errors (e.g. WebGPU).

---

### Shader handling (DXBC → translator) and reflection needs

#### 5.1 DXBC handling in the DDI

Shader creation entrypoints provide DXBC bytecode through the `...ARG_CREATE*SHADER` structures:

* D3D10: `D3D10DDIARG_CREATEVERTEXSHADER`, `D3D10DDIARG_CREATEPIXELSHADER` (and `...CREATEGEOMETRYSHADER` if supported)
* D3D11: `D3D11DDIARG_CREATEVERTEXSHADER`, `D3D11DDIARG_CREATEPIXELSHADER`, etc.

Rules:

* Treat incoming pointers as read-only and short-lived; copy the DXBC blob into driver-owned memory (or immediately hash it and forward to host).
* Cache translation results by **(shader model, DXBC hash)**; shaders are frequently recreated across device loss/reset paths.
* Be prepared for “SM4 level 9” DXBC variants like `vs_4_0_level_9_1` / `ps_4_0_level_9_1` (commonly produced by `fxc` for feature level 9.x compatibility). These still use the DXBC container format; the shader version token simply indicates a more restricted instruction/resource subset.

#### 5.2 Forwarding strategy to the AeroGPU emulator translator

Minimal architecture that works well with “UMD in guest ↔ WebGPU in host”:

1. On `Create*Shader`, compute a content hash of the DXBC, store a small private object:
   * `{ shader_stage, dxbc_hash, bytecode_length, optional_cached_host_id }`
2. Send `{ dxbc_hash, stage, dxbc_bytes }` to the host translator once (or lazily on first bind).
3. Host translates DXBC → internal IR → WGSL (or other), creates a WebGPU shader module/pipeline key.
4. On draw, UMD sends the bound `dxbc_hash` (or host shader ID) plus current pipeline state.

#### 5.3 Minimal “reflection” requirements

For a minimal driver, do **not** depend on D3DCompiler reflection APIs at runtime. Instead:

* **Input layout validation:** needed to map `D3D10DDIARG_CREATEELEMENTLAYOUT` / `D3D11DDIARG_CREATEELEMENTLAYOUT` to what the vertex shader expects.
  * Option A (minimal): do not validate; accept the layout and rely on app correctness.
  * Option B (recommended): parse DXBC `ISGN` (input signature) chunk to validate semantics/types.

* **Resource bindings:** the runtime binds SRVs/CBs/samplers by slot; your translator needs to know which slots are referenced.
  * Minimal: infer referenced slots by scanning DXBC instructions for resource operands.
  * Recommended: parse the DXBC `RDEF` chunk (resource definitions) to learn declared bindings (t/s/b registers) and their dimensions.

No other reflection is required for “triangle/texture/depth” bring-up.

---

### Compatibility target (feature level) and roadmap

#### 6.1 Minimal feature level to claim first: **FL10_0**

**Why FL10_0 first:**

* D3D10 runtime requires a D3D10-capable driver (effectively 10_0/10_1 class).
* D3D11 apps often ship with a **10_0 fallback** path on Windows 7-era GPUs.
* FL10_0 avoids tessellation requirements and keeps the pipeline close to D3D10 (VS/GS/PS only, SM4.0).

**What you must support for a credible FL10_0 path:**

* Render-to-texture (RTV) + depth (DSV)
* `Draw` and `DrawIndexed`
* Viewports/scissors, rasterizer state, blend state, depth/stencil state
* Texture2D sampling + samplers
* Constant buffers (as resources) and updates (Map/Unmap or UpdateSubresource)
* Geometry shader plumbing (required by the FL10_0 pipeline even if your initial apps never bind a GS):
  * `pfnCreateGeometryShader` and the corresponding `*SetShader`/resource-binding entrypoints
  * it is valid for apps to keep the GS stage unbound; but the entrypoints should exist and work when used

**What can be NOT_SUPPORTED at FL10_0 bring-up (but will limit apps):**

* Queries/predication
* MSAA
* UAVs and compute
* Deferred contexts / command lists (if the runtime exposes them through your chosen D3D11 DDI interface version)

If you claim `D3D_FEATURE_LEVEL_10_0` but do not implement compute shaders, ensure the corresponding capability is reported as unsupported (for API-facing caps this is typically via `D3D11_FEATURE_DATA_D3D10_X_HARDWARE_OPTIONS::ComputeShaders_Plus_RawAndStructuredBuffers_Via_Shader_4_x = FALSE`).

#### 6.2 Roadmap to FL11_0 (SM5.0)

To claim `D3D_FEATURE_LEVEL_11_0`, plan these increments:

1. **Complete the FL10_0 feature set** (if you started at FL9_x for bring-up)
   * Geometry shader support is expected at FL10_0+:
     * implement `pfnCreateGeometryShader` and the GS bind/resource entrypoints (`pfnGsSet*`)
2. **UAV plumbing + compute shaders**
   * `pfnCreateUnorderedAccessView` / `D3D11DDIARG_CREATEUNORDEREDACCESSVIEW`
   * `pfnCreateComputeShader` / `D3D11DDIARG_CREATECOMPUTESHADER`
   * UAV binding APIs and `Dispatch`
3. **Tessellation**
   * `pfnCreateHullShader` / `pfnCreateDomainShader`
   * HS/DS set calls + fixed-function tessellation state
4. **More formats and caps**
   * BC6H/BC7 (common in later D3D11 titles)
   * more `pfnGetCaps` coverage (format support, threading caps, etc)

Keep `pfnGetCaps` truthful: only advertise a feature level once the corresponding shader stages and bindings are implemented end-to-end.

---

### Testing plan (minimal apps and expected coverage)

The goal is to validate the UMD is functional *without* needing a full game engine.

For system-level smoke testing (separate from these app-level tests), use the validation checklist in `windows7-aerogpu-validation.md` (TDR/vblank stability, `dxdiag` checks, etc).

This repository also contains a guest-side test harness you can use as a starting point:

* `drivers/aerogpu/tests/win7/` (see `drivers/aerogpu/tests/win7/README.md`)
  * includes D3D11 coverage tests that exercise a large chunk of the Win7 DXGI/D3D11 path (swapchain present, offscreen render-to-texture readback, texture sampling, dynamic buffer/constant buffer updates, depth/stencil, etc).

#### 7.1 D3D10 triangle (windowed)

**Covers:**

* `OpenAdapter10` → `CreateDevice`
* swapchain creation (`DXGI_SWAP_CHAIN_DESC`)
* `CreateResource` (VB), `CreateRenderTargetView`
* `CreateVertexShader` / `CreatePixelShader` (SM4.0)
* `IASet*`, `VSSetShader`, `PSSetShader`, `OMSetRenderTargets`
* `Draw`, `Present`

**Existing in repo:**

* `drivers/aerogpu/tests/win7/d3d10_triangle/` — swapchain triangle + present.
* `drivers/aerogpu/tests/win7/d3d10_map_do_not_wait/` — validates `Map(READ, DO_NOT_WAIT)` behaves like a non-blocking poll for staging readback.

#### 7.2 D3D11 triangle (device at FL10_0)

**Covers:**

* `OpenAdapter11` → `CreateDevice`
* D3D11 binding path (VS/PS only)
* same draw/present path as above, but via D3D11 runtime/DDI

**Existing in repo:**

* `drivers/aerogpu/tests/win7/d3d11_triangle/` — swapchain triangle + present; validates pixels via staging readback.
* `drivers/aerogpu/tests/win7/readback_sanity/` — offscreen render-to-texture + staging readback (no present).
* `drivers/aerogpu/tests/win7/d3d11_map_do_not_wait/` — validates `Map(READ, DO_NOT_WAIT)` behaves like a non-blocking poll for staging readback.
* `drivers/aerogpu/tests/win7/d3d11_compute_smoke/` — compute shader smoke test: binds `b0` constant buffer + SRV/UAV buffers (structured + raw), `Dispatch`, and validates output via staging readback.

#### 7.2.1 D3D10.1 coverage (optional, still useful on Win7)

The Windows 7 D3D10.1 runtime can route some Map/Unmap patterns through slightly different DDIs.
Having a dedicated D3D10.1 test helps catch regressions in those entrypoints.

**Existing in repo:**

* `drivers/aerogpu/tests/win7/d3d10_1_triangle/` — swapchain triangle + present via the D3D10.1 runtime.
* `drivers/aerogpu/tests/win7/d3d10_1_map_do_not_wait/` — D3D10.1 variant of the `Map(READ, DO_NOT_WAIT)` non-blocking poll test.

#### 7.3 Texture sampling test (2D)

Render a textured quad:

**Covers:**

* `CreateResource` (Texture2D)
* `CreateShaderResourceView`, `CreateSampler`
* texture upload path (`Map/Unmap` or `UpdateSubresource`)
* `PSSetShaderResources`, `PSSetSamplers`
* shader translation handling `sample` ops

**Existing in repo:** `drivers/aerogpu/tests/win7/d3d11_texture_sampling_sanity/` (renders a point-sampled textured quad using an SRV + sampler and validates pixels via staging readback; exercises `IASetIndexBuffer` + `DrawIndexed`).

#### 7.4 Depth test

Draw two overlapping triangles with different Z and enable depth:

**Covers:**

* `CreateResource` (depth buffer)
* `CreateDepthStencilView`
* `CreateDepthStencilState` + `SetDepthStencilState`
* `ClearDepthStencilView`
* correct depth compare/write behavior in the host translation layer

**Existing in repo:** `drivers/aerogpu/tests/win7/d3d11_depth_test_sanity/` (offscreen RTV + DSV; clears depth then draws overlapping triangles and validates the center pixel is depth-tested).

#### 7.5 Dynamic constant buffer test

Draw using a dynamic constant buffer updated via `Map(WRITE_DISCARD)`:

**Covers:**

* `CreateBuffer` (dynamic constant buffer)
* `Map(WRITE_DISCARD)` + `Unmap` and constant-buffer binding (`*SetConstantBuffers`)
* validation that constant-buffer updates take effect between draws

**Existing in repo:** `drivers/aerogpu/tests/win7/d3d11_dynamic_constant_buffer_sanity/`.

Bring-up note: `Map(WRITE_DISCARD)` on dynamic buffers must work even before the KMD implements `DxgkDdiLock` / `DxgkDdiUnlock` (which back the runtime `pfnLockCb`/`pfnUnlockCb`). AeroGPU supports this by treating dynamic buffers as **host-owned** (`backing_alloc_id = 0`) and mapping an in-UMD shadow buffer, uploading via `AEROGPU_CMD_UPLOAD_RESOURCE` on Unmap.

#### 7.6 `Map(READ, DO_NOT_WAIT)` staging readback behavior

Validate that `Map(READ, DO_NOT_WAIT)` behaves like a **non-blocking poll** (returns `DXGI_ERROR_WAS_STILL_DRAWING` while GPU work is still in flight), and that the blocking `Map(READ)` variant waits for GPU completion and returns correct bytes.

**Existing in repo:**

* `drivers/aerogpu/tests/win7/d3d10_map_do_not_wait/`
* `drivers/aerogpu/tests/win7/d3d10_1_map_do_not_wait/`
* `drivers/aerogpu/tests/win7/d3d11_map_do_not_wait/`

#### 7.7 Shared resources / DXGI shared handles (cross-process)

On Win7, DWM (D3D9Ex) commonly consumes **DXGI shared handles** produced by D3D10/D3D11 apps. Ensure that:

* shared resources create a stable `share_token` in preserved WDDM allocation private data, and
* cross-process `OpenSharedResource(...)` drives `IMPORT_SHARED_SURFACE` using that stable token.

Canonical contract and rationale: `aerogpu-device-abi.md`.

**Existing in repo:**

* `drivers/aerogpu/tests/win7/d3d10_shared_surface_ipc/`
* `drivers/aerogpu/tests/win7/d3d10_1_shared_surface_ipc/`
* `drivers/aerogpu/tests/win7/d3d11_shared_surface_ipc/`

---

#### Appendix: practical bring-up order

1. Implement D3D11 DDI at FL10_0 first (many samples use D3D11 even when targeting “DX10 class”).
2. Reuse the same underlying object model to implement D3D10 DDI entrypoints.
3. Only after triangle/texture/dynamic-constant-buffer/depth pass, start expanding caps and feature level.

## Allocation contract

This document is the **single authoritative, implementation-oriented spec** for how a **Windows 7** (**WDDM 1.1**) **D3D10/D3D11 user-mode display driver (UMD)** allocates and frees memory through the Win7-era D3D UMD contracts.

> Terminology warning: Win7 D3D UMDs have *two* different “allocation” concepts that are easy to conflate:
>
> 1. **Resource backing allocations**: the WDDM allocations that back D3D buffers/textures (created during `CreateResource` via `D3DDDI_ALLOCATIONINFO` / `D3D11DDI_ALLOCATIONINFO` and `pfnAllocateCb`).
> 2. **DMA buffer (command buffer) allocation**: acquiring and releasing the per-submit command buffer + allocation list + patch list (also via `pfnAllocateCb`/`pfnDeallocateCb`, but using a different subset of fields in `D3DDDICB_ALLOCATE` / `D3DDDICB_DEALLOCATE`).
>
> This doc focuses on (1) for `CreateResource`, but also lists (2) because the callback names (`AllocateCb`/`DeallocateCb`) are otherwise confusing.

It is written against Win7-era user-mode DDI headers (the canonical set is WinDDK 7600.16385.1, but newer Windows Kits often ship compatible headers with extra fields):

* `d3d10umddi.h`
* `d3d11umddi.h`
* `d3dumddi.h`
* `dxgiddi.h`

The goal is that a developer with the Win7-era DDI headers can implement a correct `CreateResource` allocation flow without chasing definitions across multiple headers.

See also:

* `windows7-d3d-umd-ddi.md` — Win7 `Map`/`Unmap` (`pfnLockCb`/`pfnUnlockCb`), pitch rules, and staging readback synchronization.
* `windows7-d3d-umd-ddi.md` — submission callback contracts (DMA buffer acquisition, render/present, fence waits).
* `windows7-aerogpu-validation.md` — Win7 bring-up checklist; includes the `NonLocalMemorySizeMB` segment budget override for “early” allocation failures.
* `windows7-d3d-umd-ddi.md` — trace guide + invariants for Win7 DXGI swapchain backbuffer `CreateResource` parameters and required allocation flags.
* `aerogpu-device-abi.md` — Win7 shared-surface strategy (`share_token` vs user-mode shared `HANDLE` numeric values).
* `windows7-d3d-umd-ddi.md` — deprecated redirect (kept for link compatibility; points at the focused docs above).

### Related AeroGPU code/docs (cross-links)

* Win7 WDK UMD implementations (real runtime/WDDM path):
  * D3D10: `drivers/aerogpu/umd/d3d10_11/src/aerogpu_d3d10_umd_wdk.cpp`
  * D3D10.1: `drivers/aerogpu/umd/d3d10_11/src/aerogpu_d3d10_1_umd_wdk.cpp`
  * D3D11: `drivers/aerogpu/umd/d3d10_11/src/aerogpu_d3d11_umd_wdk.cpp`
* Repo-only ABI subset / portable bring-up: `drivers/aerogpu/umd/d3d10_11/src/aerogpu_d3d10_11_umd.cpp` (no-WDK stubs).
* KMD allocation behavior: `drivers/aerogpu/kmd/src/aerogpu_kmd.c` (`AeroGpuDdiCreateAllocation` / `AeroGpuDdiDestroyAllocation`).
* WDDM memory model: `windows7-aerogpu-wddm-driver.md` (§5 “Memory model (minimal)”).
* Allocation private-data blob (AeroGPU `alloc_id` / `share_token`): `drivers/aerogpu/protocol/aerogpu_wddm_alloc.h`
* `backing_alloc_id` contract (stable `alloc_id` semantics): `aerogpu-device-abi.md`
* Header/layout probe tool: `drivers/aerogpu/tools/win7_wdk_probe` (prints `sizeof`/`offsetof` for `D3D*DDIARG_CREATERESOURCE`, `D3DDDI_ALLOCATIONINFO`, and the resource-allocation members of `D3DDDICB_ALLOCATE` / `D3DDDICB_DEALLOCATE`).

---

### Win7 adapter-open + device-create wiring (where allocation callbacks come from)

#### 1.1 `OpenAdapter10` / `OpenAdapter11` (Win7 quirk)

Exports (names matter; signatures from Win7-era headers):

* D3D10: `HRESULT APIENTRY OpenAdapter10(D3D10DDIARG_OPENADAPTER* pOpenData)`
* D3D11: `HRESULT APIENTRY OpenAdapter11(D3D10DDIARG_OPENADAPTER* pOpenData)`

**Win7 quirk (WDDM 1.1):** `OpenAdapter11` still uses the **D3D10** open container type `D3D10DDIARG_OPENADAPTER`. The D3D11-specific DDI begins later at `D3D11DDIARG_CREATEDEVICE` / `D3D11DDIARG_GETCAPS`.

`D3D10DDIARG_OPENADAPTER` is the handoff that provides:

* runtime→UMD adapter callback table (store it)
* UMD→runtime adapter function table output slot (fill it)
* interface version negotiation

In practice, the adapter open struct fields you care about are:

* `D3D10DDI_HRTADAPTER hRTAdapter`
  * Runtime-owned adapter handle (passed back to adapter callbacks if needed).
* `const D3D10DDI_ADAPTERCALLBACKS* pAdapterCallbacks`
  * Runtime callback table for adapter-level interactions (store it in your adapter object).
* `D3D10DDI_HADAPTER hAdapter`
  * **Out**: your adapter handle.
* `D3D10DDI_ADAPTERFUNCS* pAdapterFuncs`
  * **Out**: you fill this with your adapter entrypoints (including `pfnCreateDevice`).

#### 1.2 `D3D10DDIARG_CREATEDEVICE` / `D3D11DDIARG_CREATEDEVICE` (store the device callbacks)

At adapter `pfnCreateDevice(...)` time, the runtime passes a `*_ARG_CREATEDEVICE` that contains (at minimum):

* an “RT device” handle (`D3D10DDI_HRTDEVICE` / `D3D11DDI_HRTDEVICE`) that must be passed back when invoking runtime callbacks
* pointers to runtime callback tables:
  * D3D10/11 “device callbacks” (wrapper table):
    * D3D10: `D3D10DDI_DEVICECALLBACKS`
    * D3D11: `D3D11DDI_DEVICECALLBACKS`
    * Always contains `pfnSetErrorCb` (error reporting from `void` DDIs).
    * Some header revisions also include `pfnAllocateCb`/`pfnDeallocateCb` and/or `pfnLockCb`/`pfnUnlockCb` here.
  * the shared `d3dumddi.h` callback table:
    * `D3DDDI_DEVICECALLBACKS`
    * This is the canonical “WDDM callback” layer (submission, allocation, lock/unlock, fence waits).

**Win7-era header wiring (field names; header-dependent):**

* `pCallbacks` / `pDeviceCallbacks` → `D3D10DDI_DEVICECALLBACKS` / `D3D11DDI_DEVICECALLBACKS`
* `pUMCallbacks` (if present) → `D3DDDI_DEVICECALLBACKS`

On Win7, the **allocation + mapping** callbacks are part of the shared `d3dumddi.h` contract. In **WDK 7.1** they are provided via `pUMCallbacks`, but some Win7-capable header sets either omit `pUMCallbacks` or embed these callbacks in the D3D10/11 wrapper table.

The required callbacks for resource backing allocations and mapping are:

* `pfnAllocateCb`
* `pfnDeallocateCb`
* (for CPU staging/mapping) `pfnLockCb`, `pfnUnlockCb`

**Rule:** Store both callback table pointer(s) and the RT-device handle in your per-device object. Every `CreateResource`/`DestroyResource` uses them. Use `drivers/aerogpu/tools/win7_wdk_probe` to confirm which table and member names your chosen headers expose.

##### `*_ARG_CREATEDEVICE` fields that matter for allocations

Both D3D10 and D3D11 create-device structs contain:

* `hRTDevice`
  * Runtime device handle you must pass as the first argument to `pfnAllocateCb` / `pfnDeallocateCb` / `pfnLockCb` / `pfnUnlockCb`.
* `pUMCallbacks` (if present)
  * Pointer to the shared callback table (`D3DDDI_DEVICECALLBACKS`) that contains `pfnAllocateCb` / `pfnDeallocateCb` (and also `pfnLockCb` / `pfnUnlockCb`).
* `pCallbacks` / `pDeviceCallbacks`
  * Pointer to the D3D10/11 wrapper callback table (always contains `pfnSetErrorCb`, and in some header revisions also provides `pfnAllocateCb` / `pfnDeallocateCb` / `pfnLockCb` / `pfnUnlockCb`).

D3D11 additionally wires both the device and immediate-context vtables during `CreateDevice`:

* `pDeviceFuncs` (`D3D11DDI_DEVICEFUNCS`)
* `pDeviceContextFuncs` (`D3D11DDI_DEVICECONTEXTFUNCS`)

---

### The CreateResource allocation sequence (minimal, resource-backing allocations)

#### 2.1 Sequence diagram (runtime ⇄ UMD ⇄ dxgkrnl ⇄ KMD)

```
App thread
  |
  |  (API call e.g. ID3D11Device::CreateTexture2D)
  v
D3D10/D3D11 runtime
  |
  | 1) pfnCalcPrivateResourceSize(hDevice, pCreateResource)
  |    -> runtime allocates hResource.pDrvPrivate
  |
  | 2) pfnCreateResource(hDevice, pCreateResource, hResource, hRTResource)
  v
UMD CreateResource
  |
  | 3) Decide allocation layout:
  |      - allocation count strategy
  |      - size/align per allocation
  |      - flags (Primary / RenderTarget / CpuVisible / etc)
  |
  | 4) Fill allocation info array (D3D11DDI_ALLOCATIONINFO / D3DDDI_ALLOCATIONINFO)
  |      - Size, Alignment, Flags, (optional) per-allocation private data
  |
  | 5) Call the runtime allocation callback to create WDDM allocations:
  |      // Callback table location is header-dependent (pUMCallbacks vs pCallbacks/pDeviceCallbacks)
  |      const auto* cb = /* callback table that exposes pfnAllocateCb */;
  |      D3DDDICB_ALLOCATE alloc = {...};
  |      alloc.hResource = hRTResource;
  |      alloc.NumAllocations = pCreateResource->NumAllocations;
  |      alloc.pAllocationInfo = pCreateResource->pAllocationInfo;
  |      hr = cb->pfnAllocateCb(hRTDevice, &alloc);
  |
  |    Runtime returns:
  |      - alloc.hKMResource
  |      - pAllocationInfo[i].hKMAllocation / hAllocation for each allocation (name is header-dependent)
  |
  v
dxgkrnl / VidMm
  |
  | 6) Calls KMD allocation DDIs:
  |      DxgkDdiCreateAllocation / DxgkDdiDestroyAllocation
  |
  v
UMD CreateResource (continues)
  |
  | 7) Store those KM handles in your resource private object
  v
Return to runtime
```

#### 2.2 The “one rule” about outputs

The only “real” outputs from `pfnAllocateCb` that the UMD must preserve are:

* `D3DDDICB_ALLOCATE::hKMResource` (resource-level kernel handle)
* `D3DDDI_ALLOCATIONINFO::{hKMAllocation|hAllocation}` for every allocation entry (name is header-dependent)
* (if creating a shared resource) `D3DDDICB_ALLOCATE::hSection`

Everything else is driver-owned bookkeeping.

> AeroGPU note (stable `alloc_id`): the returned allocation handle (`D3DKMT_HANDLE`) is used by the UMD
> for later callbacks like `pfnLockCb`/`pfnUnlockCb`, but it is **not** the stable `alloc_id` used by the
> AeroGPU host allocation table.
>
> On Win7/WDDM 1.1, the KMD later sees a different identity (`DXGK_ALLOCATIONLIST::hAllocation`, typically
> a pointer) when building the per-submit allocation table. Therefore, do **not** derive AeroGPU
> `alloc_id`/`backing_alloc_id` from the numeric value of `D3DKMT_HANDLE`.
>
> Instead, use the WDDM allocation **private driver data** blob (see `drivers/aerogpu/protocol/aerogpu_wddm_alloc.h`
> and `aerogpu-device-abi.md`).

---

### Runtime callback prototypes (Win7-era headers)

These callbacks are provided by the runtime (via the device callback table(s) handed to the UMD at `CreateDevice` time).

#### 3.1 Resource backing allocation callbacks: `pfnAllocateCb` / `pfnDeallocateCb`

The D3D10/D3D11 resource allocation contract is declared in `d3dumddi.h` as part of the shared **WDDM callback layer** (`D3DDDI_DEVICECALLBACKS`).

* In **WDK 7.1**, the runtime passes this table explicitly as `*_ARG_CREATEDEVICE::pUMCallbacks`.
* Some Win7-capable header sets either rename/omit `pUMCallbacks` or expose `pfnAllocateCb` / `pfnDeallocateCb` on the D3D10/11 wrapper callback table (`pCallbacks` / `pDeviceCallbacks`).

In all cases, the runtime expects the UMD to create and destroy WDDM allocations for resources via:

* `pfnAllocateCb` (create allocation(s) backing a resource)
* `pfnDeallocateCb` (destroy allocation(s) backing a resource)

These calls use:

* `D3DDDICB_ALLOCATE` with `NumAllocations` + `pAllocationInfo` (array of `D3DDDI_ALLOCATIONINFO`)
* `D3DDDICB_DEALLOCATE` with `NumAllocations` + `HandleList` / `phAllocations` (array of `D3DKMT_HANDLE`; name is header-dependent)

#### 3.2 DMA buffer (command buffer) allocation callbacks: `pfnAllocateCb` / `pfnDeallocateCb`

On Win7, the same callback names are also used to acquire and release the **DMA/command buffer** that a submission will be encoded into (plus its allocation list / patch list).

This is covered in detail in:

* `windows7-d3d-umd-ddi.md`

#### 3.3 Mapping callbacks (staging + dynamic updates): `pfnLockCb` / `pfnUnlockCb`

For CPU mapping (notably `D3D11_USAGE_STAGING` readback), the UMD uses:

* `pfnLockCb`
* `pfnUnlockCb`

The callback typedefs are declared in `d3dumddi.h`:

```c
typedef HRESULT (APIENTRY *PFND3DDDICB_ALLOCATE)(
    D3D10DDI_HRTDEVICE hDevice,
    D3DDDICB_ALLOCATE* pAllocateData
    );

typedef HRESULT (APIENTRY *PFND3DDDICB_DEALLOCATE)(
    D3D10DDI_HRTDEVICE hDevice,
    const D3DDDICB_DEALLOCATE* pDeallocateData
    );

// Used by Map/Unmap paths (notably D3D11_USAGE_STAGING)
typedef HRESULT (APIENTRY *PFND3DDDICB_LOCK)(D3D10DDI_HRTDEVICE hDevice, D3DDDICB_LOCK* pLockData);
typedef HRESULT (APIENTRY *PFND3DDDICB_UNLOCK)(D3D10DDI_HRTDEVICE hDevice, const D3DDDICB_UNLOCK* pUnlockData);
```

Notes:

* The first parameter is the runtime “RT device” handle passed at create-device time (commonly stored as `hRTDevice` in UMD code).
* Header revisions disagree on the *type* of that first parameter (`D3D10DDI_HRTDEVICE` vs `D3D11DDI_HRTDEVICE`, and some older sets use `HANDLE`). Use the exact prototype your installed headers define.
* `pfnAllocateCb` and `pfnLockCb` are in-out: they write handles/pointers back into the provided structs.

---

### Allocation data structures (field lists)

> Naming: Win7-capable header sets are not perfectly uniform (WinDDK 7600 vs later Windows Kits often add fields or rename members).
> This doc uses **WinDDK 7600-style spellings where possible**, and calls out common **header-dependent** member names explicitly.
> If in doubt, build and run `drivers/aerogpu/tools/win7_wdk_probe` against *your* headers and follow the identifiers it reports.

#### 4.1 `D3DDDICB_ALLOCATE` (from `d3dumddi.h`)

`D3DDDICB_ALLOCATE` is an in/out struct used with `pfnAllocateCb`.

On Win7/WDDM 1.1 it is used in two distinct places:

1. **CreateResource resource backing allocations** (fill `NumAllocations`/`pAllocationInfo` and get back allocation handles).
2. **Submission DMA buffer allocation** (request command buffer capacity and get back pointers).

##### 4.1.1 Resource allocation fields (CreateResource path)

* `D3DDDI_HRESOURCE hResource`
  * Runtime resource handle being allocated for (the `hRTResource` passed to CreateResource).
* `D3DDDI_HKMRESOURCE hKMResource`
  * **Out**: kernel-mode resource handle returned by the runtime.
* `UINT NumAllocations`
  * Count of entries in `pAllocationInfo`.
* `D3DDDI_ALLOCATIONINFO* pAllocationInfo`
  * Array of per-allocation descriptors:
    * UMD fills `Size`/`Alignment`/`Flags`/`pPrivateDriverData`…
    * runtime returns `hKMAllocation` (sometimes named `hAllocation`) in each entry.
* `VOID* pPrivateDriverData`
  * Optional resource-level private data blob for KMD (rare in minimal designs; most metadata is per-allocation).
* `UINT PrivateDriverDataSize`
  * Size of `pPrivateDriverData` (bytes).
* `HANDLE hSection`
  * **Out (shared resources):** section handle for cross-process sharing (`IDXGIResource::GetSharedHandle`).
    * This is a user-mode NT handle and its numeric value is process-local. When transferring it to another process, it must be duplicated/inherited (`DuplicateHandle`/inheritance) before calling `OpenSharedResource(...)`.
    * AeroGPU does not use the numeric user-mode shared `HANDLE` value as a stable host-facing identifier; shared surfaces are keyed by a stable `share_token` persisted in WDDM allocation private driver data (see `aerogpu-device-abi.md`).
* `D3DDDICB_ALLOCATEFLAGS Flags`
  * Resource-level allocation flags (header-dependent bitfield).
  * For Win7 `CreateResource` allocation calls you will commonly set:
    * `CreateResource = 1` (indicates this is a resource-backing allocation, not a DMA buffer allocation)
    * `CreateShared = 1` for shared resources
    * `Primary = 1` for DXGI primaries/backbuffers

* `ResourceFlags`
  * Resource classification flags (header-dependent bitfield).
  * Common bits:
    * `RenderTarget = 1` for RTV-capable resources
    * `ZBuffer = 1` for depth/stencil resources

##### `D3DDDICB_ALLOCATEFLAGS` (common bits used by AeroGPU)

The Win7 bring-up set of bits you should expect to touch:

* `CreateResource`
  * Set for **resource backing** allocations (the `CreateResource` path).
* `CreateShared`
  * Set when creating a **shared** resource (DXGI shared handles).
* `Primary`
  * Set for scanout-capable allocations (DXGI swapchain backbuffers / primaries).

##### 4.1.2 DMA buffer allocation fields (submission path)

* `D3DKMT_HANDLE hContext`
  * Kernel context handle that the DMA buffer will be associated with.
* `UINT DmaBufferSize` / `UINT CommandBufferSize`
  * Requested command buffer capacity (bytes). Header revisions may use either name.
* `VOID* pDmaBuffer` / `VOID* pCommandBuffer`
  * **Out**: pointer to the DMA/command buffer memory (owned by the runtime/OS).
* `D3DDDI_ALLOCATIONLIST* pAllocationList`
  * **Out**: pointer to the allocation list array for this submission.
* `UINT AllocationListSize`
  * Capacity of `pAllocationList` (in entries).
* `D3DDDI_PATCHLOCATIONLIST* pPatchLocationList`
  * **Out**: pointer to the patch-location list array for this submission.
* `UINT PatchLocationListSize`
  * Capacity of `pPatchLocationList` (in entries).
* `VOID* pDmaBufferPrivateData`
  * **Out (optional)**: pointer to a fixed-size per-submission blob shared with the KMD (size set by `DXGK_DRIVERCAPS::DmaBufferPrivateDataSize`).
* `UINT DmaBufferPrivateDataSize`
  * Capacity of `pDmaBufferPrivateData` (bytes).

#### 4.2 `D3DDDICB_DEALLOCATE` (from `d3dumddi.h`)

`D3DDDICB_DEALLOCATE` is used with `pfnDeallocateCb` and mirrors the same dual use:

##### 4.2.1 Resource deallocation fields (DestroyResource path)

* `D3DDDI_HRESOURCE hResource`
* `D3DDDI_HKMRESOURCE hKMResource`
* `UINT NumAllocations`
* `const D3DKMT_HANDLE* HandleList` / `phAllocations`
  * Array of allocation handles (`hKMAllocation` / `hAllocation`) to free (member name is header-dependent).

> AeroGPU note (guest-backed resources): if your UMD emits protocol cleanup packets (for example
> `AEROGPU_CMD_DESTROY_RESOURCE`) that depend on a non-zero `alloc_id` / `backing_alloc_id`
> being resolvable, ensure those packets are **submitted/flushed before** calling `pfnDeallocateCb`
> for the backing WDDM allocation(s). On Win7, the KMD’s per-submit `aerogpu_alloc_table` is
> derived from the submission allocation list, and submitting after freeing an allocation handle can
> lead to missing `alloc_id` entries or invalid handles during submission processing.

##### 4.2.2 DMA buffer release fields (submission path)

* `VOID* pDmaBuffer` / `VOID* pCommandBuffer`
  * The command buffer pointer previously returned by `D3DDDICB_ALLOCATE`.
* `D3DDDI_ALLOCATIONLIST* pAllocationList`
  * The allocation list pointer previously returned by `D3DDDICB_ALLOCATE`.
* `D3DDDI_PATCHLOCATIONLIST* pPatchLocationList`
  * The patch list pointer previously returned by `D3DDDICB_ALLOCATE`.
* `VOID* pDmaBufferPrivateData`
  * The private-data pointer previously returned by `D3DDDICB_ALLOCATE` (if used).

#### 4.3 `D3DDDI_ALLOCATIONINFO` (from `d3dumddi.h`)

This is the per-allocation descriptor used for **resource backing allocations**.

The D3D10/11 DDIs reuse this layout via the `D3D10DDI_ALLOCATIONINFO` / `D3D11DDI_ALLOCATIONINFO` typedefs, and `D3DDDICB_ALLOCATE::pAllocationInfo` points at an array of this type.

Fields (subset relevant to the UMD allocation contract; see `win7_wdk_probe` for the full header layout):

* `D3DKMT_HANDLE hKMAllocation` / `hAllocation`
  * **Out**: kernel allocation handle for this allocation entry (member name is header-dependent).
* `SIZE_T Size`
  * **In**: allocation size in bytes.
* `SIZE_T Alignment`
  * **In**: required alignment (0 = default).
* `D3DDDI_ALLOCATIONINFOFLAGS Flags`
  * Per-allocation flags (notably `CpuVisible`, and sometimes `Primary`).
* `SupportedReadSegmentSet`
  * **In**: bitmask of memory segments this allocation may be placed into for CPU reads.
* `SupportedWriteSegmentSet`
  * **In**: bitmask of memory segments this allocation may be placed into for CPU writes.
* `VOID* pPrivateDriverData`
  * Optional per-allocation private data blob for KMD (copied by the runtime during `pfnAllocateCb`).
  * AeroGPU uses this blob to persist stable allocation IDs and (for shared resources) `share_token`
    values across create/open:
    * `aerogpu_wddm_alloc_priv` / `aerogpu_wddm_alloc_priv_v2` in `drivers/aerogpu/protocol/aerogpu_wddm_alloc.h`
  * On Win7/WDDM 1.1, treat this as an **in/out** blob:
    * The UMD supplies initial fields (magic/version, `alloc_id`, flags, and `share_token = 0` placeholder).
    * The KMD writes back stable values (notably `share_token` and `size_bytes`).
    * dxgkrnl preserves the bytes for shared allocations and returns them verbatim on cross-process `OpenResource`,
      allowing the opening UMD to recover the same `alloc_id` and `share_token`.
* `UINT PrivateDriverDataSize`
  * Size of `pPrivateDriverData` in bytes.

##### `D3DDDI_ALLOCATIONINFOFLAGS` (minimal set you will actually use)

The Win7 bring-up set of flags you should expect to set in practice:

* `Primary`
  * If your headers expose this bit, set it for DXGI primaries/backbuffers.
  * Some Win7-capable header sets only expose “Primary” at the resource level via `D3DDDICB_ALLOCATEFLAGS.Primary`—validate expected behavior with traces (see §5.3).
* `CpuVisible`
  * Must be set for staging allocations that are CPU-mapped via `pfnLockCb`/`pfnUnlockCb`.
  * AeroGPU MVP often sets this for *all* allocations because it uses the single CPU-visible system segment.
> Note: “RenderTarget vs ZBuffer” classification is commonly expressed via `D3DDDICB_ALLOCATE::ResourceFlags`
> (e.g. `ResourceFlags.RenderTarget`, `ResourceFlags.ZBuffer`) rather than per-allocation flags.
>
> `D3DDDI_ALLOCATIONINFOFLAGS` contains more bits (overlay, shared, etc). Keep your initial implementation conservative: set only what you understand and what your KMD uses.

#### 4.4 `D3D11DDI_ALLOCATIONINFO` vs `D3DDDI_ALLOCATIONINFO`

In the Win7-era D3D10/11 UMD DDI, the DDIs reuse the `d3dumddi.h` allocation info layout:

* `D3D10DDI_ALLOCATIONINFO`
* `D3D11DDI_ALLOCATIONINFO`

Conceptually, **they are the same structure as** `D3DDDI_ALLOCATIONINFO` (the D3D10/11 headers typically typedef/alias this for API namespacing), but member spellings may differ (notably `hKMAllocation` vs `hAllocation`) and newer WDKs may add extra fields.

Practical implication:

* The allocation info array you fill for `CreateResource` can be passed directly to `D3DDDICB_ALLOCATE::pAllocationInfo` when calling `pfnAllocateCb`, and the runtime returns allocation handles in the same array (`hKMAllocation` / `hAllocation`, depending on headers).

---

### Resource descriptor fields that drive allocation (D3D11)

`D3D11DDIARG_CREATERESOURCE` (from `d3d11umddi.h`) is the UMD-visible description of the resource being created. For allocation, only a subset of fields matter:

#### 5.0 Allocation plumbing fields (how CreateResource hands you the output arrays)

These fields are the “bridge” between `CreateResource` and the `pfnAllocateCb` resource-allocation call:

* `UINT NumAllocations`
  * Number of allocations the runtime expects you to allocate for this resource.
* `D3D11DDI_ALLOCATIONINFO* pAllocationInfo`
  * Output array to fill and pass as `D3DDDICB_ALLOCATE::pAllocationInfo`.

#### 5.1 Common fields (all resource dimensions)

* `D3D10DDIRESOURCE_TYPE ResourceDimension`
  * Resource dimension discriminator.
  * **Win7 quirk:** D3D11 uses the **D3D10 DDI** enum values for this field:
    * `D3D10DDIRESOURCE_BUFFER`
    * `D3D10DDIRESOURCE_TEXTURE1D`
    * `D3D10DDIRESOURCE_TEXTURE2D`
    * `D3D10DDIRESOURCE_TEXTURE3D`
* `Usage`
  * Default/dynamic/staging semantics (values match `D3D11_USAGE_*`).
* `UINT BindFlags`
  * `D3D11_BIND_*` bits (render target, depth stencil, shader resource, etc).
* `UINT CPUAccessFlags`
  * `D3D11_CPU_ACCESS_READ` / `D3D11_CPU_ACCESS_WRITE` (staging and dynamic resources).
* `UINT MiscFlags`
  * `D3D11_RESOURCE_MISC_*` bits (shared resources, GDI compatibility, etc).

#### 5.2 Dimension-specific fields (allocation sizing inputs)

##### Buffer (`ResourceDimension == D3D10DDIRESOURCE_BUFFER`)

* `UINT ByteWidth`
* `UINT StructureByteStride`

##### Texture2D (`ResourceDimension == D3D10DDIRESOURCE_TEXTURE2D`)

* `UINT Width`
* `UINT Height`
* `UINT MipLevels`
* `UINT ArraySize`
* `DXGI_FORMAT Format`
* `DXGI_SAMPLE_DESC SampleDesc`

#### 5.3 Swapchain / backbuffer identification (DXGI primary)

When the resource is a DXGI swapchain backbuffer / primary, the DDI exposes this through a “primary descriptor” pointer (from `dxgiddi.h`):

* `const DXGI_DDI_PRIMARY_DESC* pPrimaryDesc`

**Rule of thumb (verify with traces):**

* `pPrimaryDesc != NULL` → treat this resource as a **primary/backbuffer** allocation.
* Allocate with both:
  * `D3DDDICB_ALLOCATEFLAGS.Primary = 1` (resource-level)
  * `D3DDDI_ALLOCATIONINFOFLAGS::Primary = 1` (per-allocation, if your header exposes it)
* And (for all swapchain backbuffers) treat the resource as an RTV-capable surface:
  * `D3DDDICB_ALLOCATE::ResourceFlags.RenderTarget = 1` (header-dependent bitfield)

See: `windows7-d3d-umd-ddi.md` for a Win7 trace-based workflow to confirm the exact backbuffer `CreateResource` descriptors and flags your runtime expects.

#### 5.4 D3D10 parity (`D3D10DDIARG_CREATERESOURCE`)

The D3D10 DDI uses `D3D10DDIARG_CREATERESOURCE` (from `d3d10umddi.h`) and the same *WDDM resource allocation model* as D3D11 on Win7:

* the runtime asks the UMD to fill an allocation-info array (`D3D10DDI_ALLOCATIONINFO* pAllocationInfo`), and
* the UMD calls `pfnAllocateCb` with `D3DDDICB_ALLOCATE` to create the allocation(s) and receive allocation handles (`hKMAllocation` / `hAllocation`, depending on headers).

For allocation purposes, the D3D10 create-resource argument carries the same “shape” of data as the D3D11 one:

* allocation plumbing:
  * `UINT NumAllocations`
  * `D3D10DDI_ALLOCATIONINFO* pAllocationInfo`
* resource classification and access:
  * `ResourceDimension`
  * `Usage`
  * `UINT BindFlags`
  * `UINT CPUAccessFlags`
  * `UINT MiscFlags`
* dimension-specific sizing fields (buffer vs texture)
* swapchain/backbuffer identification:
  * `const DXGI_DDI_PRIMARY_DESC* pPrimaryDesc`

In practice for AeroGPU, you can share almost all of the resource-allocation logic between D3D10 and D3D11 because the per-allocation descriptor layout (`D3DDDI_ALLOCATIONINFO`) is reused by both APIs (via `D3D10DDI_ALLOCATIONINFO` / `D3D11DDI_ALLOCATIONINFO`).

---

### “Minimal correct” allocation strategies (Win7 bring-up)

The table below is a pragmatic “works first” allocation plan for AeroGPU’s MVP memory model (**single system-memory segment**, CPU-visible).

| Resource class | Allocation count strategy | Size computation | AllocateCb fields you must set |
|---|---:|---|---|
| Buffer | 1 allocation per resource | `Size = ByteWidth` (optionally align up to 256) | `alloc.Flags.CreateResource = 1`; `alloc_info.Flags.CpuVisible = 1` if CPU reads/writes are expected (dynamic/staging or `CPUAccessFlags != 0`); `alloc_info.SupportedReadSegmentSet = alloc_info.SupportedWriteSegmentSet = 1` (single-segment MVP) |
| Texture2D (default) | 1 allocation per resource | `rowPitch = Align(Width * bytesPerPixel(Format), 256)`; `Size = rowPitch * Height` (no mips/arrays in MVP) | `alloc.Flags.CreateResource = 1`; `alloc.ResourceFlags.RenderTarget = 1` if `BindFlags & D3D11_BIND_RENDER_TARGET`; `alloc_info.Flags.CpuVisible = 1` only if CPU access is requested; `alloc_info.SupportedReadSegmentSet = alloc_info.SupportedWriteSegmentSet = 1` |
| Swapchain backbuffer | 1 allocation per backbuffer | Same as Texture2D, but match the swapchain format exactly (commonly `DXGI_FORMAT_B8G8R8A8_UNORM`) | `alloc.Flags.CreateResource = 1`; `alloc.Flags.Primary = 1`; `alloc.ResourceFlags.RenderTarget = 1`; often `alloc_info.Flags.CpuVisible = 1` in AeroGPU MVP; set segment sets to `1` |
| Staging Texture2D | 1 allocation per resource | Same as Texture2D | `alloc.Flags.CreateResource = 1`; `alloc_info.Flags.CpuVisible = 1` (required); set segment sets to `1`; use `pfnLockCb`/`pfnUnlockCb` in `Map`/`Unmap` |

Notes / constraints for MVP:

* **Mipmaps and arrays:** simplest bring-up assumes `MipLevels == 1` and `ArraySize == 1`. If you see otherwise, either:
  * allocate one big linear blob and compute subresource offsets, or
  * return NOT_SUPPORTED / set error until implemented.
* **Depth/stencil:** treat as Texture2D sizing-wise; set `alloc.ResourceFlags.ZBuffer = 1` when the resource is DSV/depth-backed. The runtime’s bind flags still control whether DSV creation is legal.
* **All-system-memory (AeroGPU):** the KMD already advertises a single `CpuVisible` system segment (`DXGKQAITYPE_QUERYSEGMENT`). In that world, marking `CpuVisible` broadly is acceptable and keeps Map/Unmap simple.

---

### Win7 + AeroGPU-specific quirks (allocation-relevant)

#### 7.1 `OpenAdapter11` uses `D3D10DDIARG_OPENADAPTER`

Repeat because it’s easy to get wrong: on Win7/WDDM 1.1, the D3D11 runtime’s UMD entrypoint is:

* `OpenAdapter11(D3D10DDIARG_OPENADAPTER*)`

So your adapter-open code must be able to branch on the requested DDI interface/version and return the correct adapter function table for D3D10 vs D3D11.

#### 7.2 Single system-memory segment model

AeroGPU’s MVP KMD exposes **one** segment:

* Segment 1: CPU-visible “system memory” (`DXGK_MEMORY_SEGMENT_GROUP_NON_LOCAL`, `Flags.CpuVisible = 1`, `Flags.Aperture = 1`)

See:

* `windows7-aerogpu-wddm-driver.md` (§5)
* `drivers/aerogpu/kmd/src/aerogpu_kmd.c` (`DXGKQAITYPE_QUERYSEGMENT`)

**Segment size / budget hint (AeroGPU):**

Even though allocations are backed by guest system RAM, Win7’s `dxgkrnl` still enforces a per-adapter **segment budget** based on the KMD-reported non-local segment size. If the budget is too small, resource creation can fail with `E_OUTOFMEMORY` / `D3DERR_OUTOFVIDEOMEMORY` even when the guest still has free RAM.

To tune the reported budget, set `HKR\Parameters\NonLocalMemorySizeMB` (see `windows7-aerogpu-validation.md` appendix and `drivers/aerogpu/kmd/README.md`).

**Implication for UMD allocation flags:**

* You do not need complex residency/eviction policy to get correctness.
* You *do* still need to set `Primary`/`RenderTarget`/`CpuVisible` correctly so dxgkrnl routes scanout and Map/Unmap expectations correctly.

## Map and Unmap semantics

This document defines the **behavioral contract** AeroGPU must implement for **`Map`/`Unmap` on Windows 7** (WDDM 1.1) when implementing the **D3D11 user-mode display driver (UMD)**.

It exists to stop future implementers from reverse‑engineering Win7 runtime behavior repeatedly, and to ensure the AeroGPU stack satisfies the staging-readback patterns used by:

* `drivers/aerogpu/tests/win7/d3d11_triangle/`
* `drivers/aerogpu/tests/win7/readback_sanity/`
* `drivers/aerogpu/tests/win7/d3d11_map_roundtrip/`
* `drivers/aerogpu/tests/win7/d3d11_map_do_not_wait/`

> Header references: symbol/type names in this doc are from Win7-era user-mode DDI headers:
> `d3d11umddi.h` (D3D11 UMD DDI), `d3d10umddi.h` (some D3D10-era shared DDI enums/types still used by D3D11 on Win7), and `d3dumddi.h` (common runtime callback types like `D3DDDICB_LOCK`).
>
> For overall D3D10/11 bring-up context, see [`win7-d3d10-11-umd-minimal.md`](windows7-d3d-umd-ddi.md).

---

### What “Map/Unmap” means on Win7 WDDM (high-level)

On Windows 7, `ID3D11DeviceContext::Map` **does not** mean “driver returns a pointer to some driver-owned allocation”. In the WDDM model the **runtime + VidMm own allocation residency and CPU mappings**.

The concrete contract is:

1. The D3D11 runtime calls the UMD DDI entrypoint `D3D11DDI_DEVICECONTEXTFUNCS::pfnMap` / `pfnUnmap`.
2. The UMD implements `pfnMap` by calling back into the runtime via `pfnLockCb` (and `pfnUnmap` via `pfnUnlockCb`) using the `D3DDDICB_LOCK` / `D3DDDICB_UNLOCK` structures.
3. The runtime uses the KMD’s WDDM callbacks (notably `DxgkDdiLock` / `DxgkDdiUnlock`) plus scheduler/fence state to:
   * block until it is safe to expose the memory to CPU (unless DO_NOT_WAIT), and
   * return a CPU pointer + pitch metadata.

**AeroGPU implication:** for deterministic staging readback and correct DO_NOT_WAIT behavior, the stack must make the runtime’s LockCb path “real”:

* fences must advance correctly, and
* KMD lock/unlock must behave consistently (details in §6).

#### Bring-up fallback: host-owned Map/Unmap without LockCb

During early bring-up it is possible for the AeroGPU Win7 KMD to *not* expose `DxgkDdiLock` / `DxgkDdiUnlock` yet. In that case the runtime’s `pfnLockCb` / `pfnUnlockCb` path may be absent or fail, which would normally make `Map` unusable for dynamic updates.

AeroGPU supports a **host-owned** update path for resources where the command stream sets:

* `backing_alloc_id = 0` (host-allocated resource; no guest allocation table indirection)

For these resources, the UMD may implement write-style maps (`WRITE`, `WRITE_DISCARD`, `WRITE_NO_OVERWRITE`) by returning a pointer into an in-UMD shadow buffer (`Resource::storage`) and then emitting `AEROGPU_CMD_UPLOAD_RESOURCE` on `Unmap` to push the updated bytes to the host.

Implications:

* This fallback is intentionally **not** used for staging readback (`Map(READ)` / `READ_WRITE` on `D3D11_USAGE_STAGING`) because returning stale shadow bytes would violate readback correctness.
* For host-owned write maps, the UMD can succeed without waiting even if the runtime lock path reports “still drawing”, because the CPU pointer is not a direct mapping of the WDDM allocation.

---

### DDI entrypoints involved (UMD entrypoints + runtime callbacks)

#### 1.1 UMD DDI entrypoints (called by D3D11 runtime)

The D3D11 runtime calls these UMD entrypoints through the immediate-context function table:

* `D3D11DDI_DEVICECONTEXTFUNCS::pfnMap`
* `D3D11DDI_DEVICECONTEXTFUNCS::pfnUnmap`

Depending on the negotiated `D3D11DDI_INTERFACE_VERSION`, the Win7 runtime may also route specific Map patterns through additional entrypoints that should forward to the same underlying implementation/semantics:

* Staging helpers:
  * `D3D11DDI_DEVICECONTEXTFUNCS::pfnStagingResourceMap`
  * `D3D11DDI_DEVICECONTEXTFUNCS::pfnStagingResourceUnmap`
* Dynamic buffer helpers:
  * `D3D11DDI_DEVICECONTEXTFUNCS::pfnDynamicIABufferMapDiscard`
  * `D3D11DDI_DEVICECONTEXTFUNCS::pfnDynamicIABufferMapNoOverwrite`
  * `D3D11DDI_DEVICECONTEXTFUNCS::pfnDynamicIABufferUnmap`
  * `D3D11DDI_DEVICECONTEXTFUNCS::pfnDynamicConstantBufferMapDiscard`
  * `D3D11DDI_DEVICECONTEXTFUNCS::pfnDynamicConstantBufferUnmap`

The Win7-era `d3d11umddi.h` prototypes are conceptually:

```c
HRESULT APIENTRY pfnMap(
    D3D11DDI_HDEVICECONTEXT hContext,
    D3D11DDI_HRESOURCE hResource,
    UINT Subresource,
    /* Win7 WDK uses D3D10-era DDI types here; values match D3D11_MAP. */ D3D10_DDI_MAP MapType,
    UINT MapFlags,
    D3D11DDI_MAPPED_SUBRESOURCE* pMapped);

void APIENTRY pfnUnmap(
    D3D11DDI_HDEVICECONTEXT hContext,
    D3D11DDI_HRESOURCE hResource,
    UINT Subresource);
```

Notes:

* Depending on the negotiated D3D11 DDI interface version, `pfnMap` may return
  `HRESULT` or be `void`.
  * If it returns `HRESULT`, it must return `DXGI_ERROR_WAS_STILL_DRAWING` when
    `D3D11_MAP_FLAG_DO_NOT_WAIT` is set and the map would block.
  * If it is `void`, failures (including `DXGI_ERROR_WAS_STILL_DRAWING`) must be
    reported via `pfnSetErrorCb`.
* `pfnUnmap` is typically `void` (errors must be reported via `pfnSetErrorCb`).

#### 1.2 Runtime callback table entries used by Map/Unmap

The runtime exposes callback tables to the UMD during device creation (`D3D11DDIARG_CREATEDEVICE`):

* `D3D11DDIARG_CREATEDEVICE::pCallbacks` / `pDeviceCallbacks` → `D3D11DDI_DEVICECALLBACKS` (D3D11 wrapper callbacks)
* Some header revisions also expose `D3D11DDIARG_CREATEDEVICE::pUMCallbacks` → `D3DDDI_DEVICECALLBACKS` (shared WDDM submission callbacks from `d3dumddi.h`)

For exact field names across Win7 WDK revisions (and a probe tool you can build against your installed headers), see:

* [`win7-d3d10-11-umd-callbacks-and-fences.md`](windows7-d3d-umd-ddi.md)
* [`drivers/aerogpu/tools/win7_wdk_probe`](../decisions/README.md) (prints `sizeof`/`offsetof` for `D3DDDICB_LOCK` / `D3DDDICB_UNLOCK` and the CreateResource allocation structs)

Map/Unmap uses at least:

* `pfnLockCb` with `D3DDDICB_LOCK`
* `pfnUnlockCb` with `D3DDDICB_UNLOCK`
* `pfnSetErrorCb` (required for `pfnUnmap` error reporting and other void DDIs)

In Win7-era header sets, these callbacks are typically declared as `HRESULT`-returning functions that take the runtime device handle first:

```c
HRESULT APIENTRY pfnLockCb(D3D10DDI_HRTDEVICE hRTDevice, D3DDDICB_LOCK* pLock);
HRESULT APIENTRY pfnUnlockCb(D3D10DDI_HRTDEVICE hRTDevice, D3DDDICB_UNLOCK* pUnlock);
```

Important details:

* `D3DDDICB_LOCK::hAllocation` / `D3DDDICB_UNLOCK::hAllocation` is a `D3DKMT_HANDLE` (a 32-bit integer handle even on x64), **not** a pointer.
* Field spellings in `D3DDDICB_LOCK` / `D3DDDICB_LOCKFLAGS` vary across header revisions; build against your chosen WDK and use the exact member names it defines (see `win7_wdk_probe` link above).

For synchronization/fence-based implementations, the shared callback table (or an embedded equivalent) also provides (names vary slightly by interface version, but the Win7-era concept is consistent):

* `pfnWaitForSynchronizationObjectCb` / `D3DDDICB_WAITFORSYNCHRONIZATIONOBJECT`
* `pfnSignalSynchronizationObjectCb` / `D3DDDICB_SIGNALSYNCHRONIZATIONOBJECT`
* (optionally) CPU-specific wait/signal variants such as `...FROMCPU`

**Important:** even if AeroGPU chooses to wait via explicit fence callbacks, `pfnLockCb`/`pfnUnlockCb` must still be correct because the runtime uses them for:

* returning the actual CPU pointer and pitch, and
* (when not using explicit waits) the default “stall until safe” behavior.

---

### `D3D11DDIARG_MAP` / `D3D11DDIARG_UNMAP` field breakdown (Win7-era headers)

Some Win7 D3D11 documentation refers to the Map/Unmap argument bundle as `D3D11DDIARG_MAP` / `D3D11DDIARG_UNMAP`.
In practice, the Win7-era D3D11 UMD DDI passes Map/Unmap as **flat arguments** (rather than a single `*ARG_*` struct), but the logical field breakdown is the same.

#### 2.1 `pfnMap` arguments

`pfnMap` describes *which* subresource to map and *how* the caller wants to access it.

* `hResource` (`D3D11DDI_HRESOURCE`)
  * The runtime-provided handle for the resource being mapped.
  * The UMD must translate this to its private resource object and (ultimately) to the underlying WDDM allocation(s) that `pfnLockCb` understands.
* `Subresource` (`UINT`)
  * `D3D11CalcSubresource(MipSlice, ArraySlice, MipLevels)` encoding.
  * For buffers, this is typically `0`.
* `MapType` (DDI enum; typically `D3D10_DDI_MAP` on Win7, values mirroring `D3D11_MAP`)
  * One of:
    * `D3D11_MAP_READ`
    * `D3D11_MAP_WRITE`
    * `D3D11_MAP_READ_WRITE`
    * `D3D11_MAP_WRITE_DISCARD`
    * `D3D11_MAP_WRITE_NO_OVERWRITE`
* `MapFlags` (`UINT`)
  * D3D11 only defines one public map flag on Win7: `D3D11_MAP_FLAG_DO_NOT_WAIT`.
  * The DDI receives the same semantic flag (see §3.2).

#### 2.2 `pfnUnmap` arguments

`pfnUnmap` identifies the mapping to end:

* `hResource` (`D3D11DDI_HRESOURCE`)
* `Subresource` (`UINT`)

The UMD must treat `(hResource, Subresource)` as the key and ensure it matches the last successful map.

#### 2.3 `D3D11DDI_MAPPED_SUBRESOURCE` (output of `pfnMap`)

`pfnMap` must fill:

* `pData` (`void*`)
* `RowPitch` (`UINT`) — bytes per row for textures (for buffers can be set to `ByteWidth` or `0`; prefer deterministic values)
* `DepthPitch` (`UINT`) — bytes per 2D slice for 3D textures (for 2D textures can be `RowPitch * Height`)

For the `d3d11_triangle` / `readback_sanity` staging readback path, **`RowPitch` must be correct** for `DXGI_FORMAT_B8G8R8A8_UNORM` staging textures because the tests index pixels using the returned pitch.

---

### Mapping: `D3D11_MAP_*` → `D3DDDICB_LOCK` flags

#### 3.1 The principle

On Win7, a D3D11 `Map` call is implemented by translating the map request to a runtime `pfnLockCb` request:

* `D3D11_MAP_*` → `D3DDDICB_LOCKFLAGS` (`ReadOnly`, `Discard`, `NoOverwrite`, …)
* `D3D11_MAP_FLAG_DO_NOT_WAIT` → `D3DDDICB_LOCKFLAGS::{DoNotWait, DonotWait}` (header spelling varies)

#### 3.2 Map-type table

The table below is the required translation for correctness and to match runtime expectations.

| API MapType (`D3D11_MAP`) | `D3DDDICB_LOCKFLAGS::ReadOnly` | `...::Write` (WriteOnly/Write flag) | `...::Discard` | `...::NoOverwrite` |
|---|---:|---:|---:|---:|
| `D3D11_MAP_READ` | 1 | 0 | 0 | 0 |
| `D3D11_MAP_WRITE` | 0 | 1 | 0 | 0 |
| `D3D11_MAP_READ_WRITE` | 0 | 0 (read+write) | 0 | 0 |
| `D3D11_MAP_WRITE_DISCARD` | 0 | 1 | 1 | 0 |
| `D3D11_MAP_WRITE_NO_OVERWRITE` | 0 | 1 | 0 | 1 |

Notes:

* The exact “write” bit name in `D3DDDICB_LOCKFLAGS` is header-version dependent (commonly `WriteOnly`). The semantic requirement is the same: the lock must be treated as CPU-write.
* `WRITE_DISCARD` and `WRITE_NO_OVERWRITE` are meaningful for dynamic resources (see §5). For other usages, treat them as invalid.

#### 3.3 Map-flag table (DO_NOT_WAIT)

| API flag | `D3DDDICB_LOCKFLAGS` bit | Required return on contention |
|---|---|---|
| `D3D11_MAP_FLAG_DO_NOT_WAIT` | `DoNotWait/DonotWait = 1` | `DXGI_ERROR_WAS_STILL_DRAWING` |

Required behavior:

* If DO_NOT_WAIT is set, the UMD **must not block** inside `pfnMap`.
* The UMD must attempt `pfnLockCb` with `DoNotWait/DonotWait = 1`.
* If the runtime reports the allocation is still in use (i.e. the lock would block), `pfnMap` must return `DXGI_ERROR_WAS_STILL_DRAWING` (not `S_FALSE`, not `E_FAIL`).
  * Note: the runtime/KMD may report “would block” using different HRESULTs on Win7/WDDM 1.1 (observed values include `DXGI_ERROR_WAS_STILL_DRAWING`, `HRESULT_FROM_NT(STATUS_GRAPHICS_GPU_BUSY)`, `E_PENDING`, and various timeout HRESULTs like `HRESULT_FROM_WIN32(WAIT_TIMEOUT)` / `HRESULT_FROM_NT(STATUS_TIMEOUT)`). When DO_NOT_WAIT is requested, normalize these “busy” results to `DXGI_ERROR_WAS_STILL_DRAWING` so the D3D11 API sees the required error code.

Practical Win7 note: different WDK/runtime combinations do not always return `DXGI_ERROR_WAS_STILL_DRAWING` directly from `pfnLockCb`/fence waits when `DO_NOT_WAIT` is set. Treat the common "would block" variants as equivalent to still-drawing and return `DXGI_ERROR_WAS_STILL_DRAWING` to the API:

* `HRESULT_FROM_NT(STATUS_GRAPHICS_GPU_BUSY)` (`0xD01E0102`)
* `HRESULT_FROM_WIN32(WAIT_TIMEOUT)`
* `HRESULT_FROM_WIN32(ERROR_TIMEOUT)`
* `HRESULT_FROM_NT(STATUS_TIMEOUT)` (`0x10000102`; note that this is `SUCCEEDED()` and must be checked explicitly)
* `E_PENDING` (`0x8000000A`) (observed in some poll-style wait paths; typically for DO_NOT_WAIT / `Timeout = 0`)

---

### Synchronization rules (the part that makes staging readback work)

#### 4.1 Staging readback: must wait (unless DO_NOT_WAIT)

The staging readback used by `d3d11_triangle` and `readback_sanity` is:

1. Draw into a DEFAULT render target (or swapchain backbuffer).
2. Create a `D3D11_USAGE_STAGING` texture with `CPU_ACCESS_READ`.
3. `CopyResource(staging, renderTarget)`.
4. `Flush()`.
5. `Map(staging, 0, D3D11_MAP_READ, 0, &mapped)` and read pixels.

The Win7 correctness requirement is:

* `Map(READ)` on `D3D11_USAGE_STAGING + CPU_ACCESS_READ` **must not return until the GPU has completed all prior work that writes the staging resource**, including the `CopyResource`.
* The returned `pData` must contain the **final bytes** produced by the GPU copy.

Practical AeroGPU guidance:

* Avoid waiting on (or polling) the device’s “latest submitted fence” for staging readback.
  Instead, track a **per-resource fence** (`last_gpu_write_fence`) that is updated only when a command that *writes that resource* is recorded/submitted (e.g. `CopyResource(staging, …)` / `CopySubresourceRegion`).
  * This prevents `Map(DO_NOT_WAIT)` from spuriously returning `DXGI_ERROR_WAS_STILL_DRAWING` due to unrelated in-flight work.
  * It also reduces unnecessary stalls when a different command stream is still executing but the staging destination is already complete.

Unless:

* `D3D11_MAP_FLAG_DO_NOT_WAIT` is set, in which case:
  * if the copy hasn’t completed yet, return `DXGI_ERROR_WAS_STILL_DRAWING` and do not block.

#### 4.2 “Force submit” rule before waiting

In a command-buffering UMD (including AeroGPU), it is possible for the UMD to have pending GPU work *in user-mode* that the kernel scheduler has not seen yet.

Therefore, when `pfnMap` needs the GPU to be finished (typically READ/READ_WRITE staging maps), the UMD should:

1. **Flush/submit** pending work that could affect the mapped resource (or its source) *before* calling a blocking lock/wait path.
2. Then wait (via blocking `pfnLockCb` or explicit fence waits).

Why this matters:

* Waiting without submitting first can deadlock (you wait for work that hasn’t been queued).
* It also increases latency (runtime can’t start executing the copy until submission happens).

Practical guidance:

* If the app already called `ID3D11DeviceContext::Flush`, the runtime will typically call your `pfnFlush` before `pfnMap` anyway, but the UMD must not rely on this ordering.
* Treat “Map needing synchronization” as an implicit flush point.

#### 4.3 Which maps require GPU synchronization?

For Win7 correctness, at minimum:

* `D3D11_MAP_READ` on a staging resource requires synchronization if the resource might have been written by GPU.
* `D3D11_MAP_READ_WRITE` on staging similarly requires synchronization.

For write maps:

* `WRITE_DISCARD` should avoid synchronization by discarding/renaming (runtime may help if `Discard` is set in lock flags).
* `WRITE_NO_OVERWRITE` should avoid synchronization but requires the app to honor the no-overwrite contract; the driver must not internally “rename” in a way that breaks the API guarantee.

---

### Resource-usage validation rules (legal MapTypes by usage)

These are the D3D11 API-level rules the Win7 runtime enforces; the UMD must match them.

#### 5.1 Usage table

| `D3D11_USAGE` | Required `CPUAccessFlags` | Legal MapTypes | Notes |
|---|---|---|---|
| `D3D11_USAGE_DEFAULT` | `0` | *(none)* | CPU mapping is not allowed. Use `UpdateSubresource` / `Copy*` instead. |
| `D3D11_USAGE_IMMUTABLE` | `0` | *(none)* | Never mappable; contents fixed at creation. |
| `D3D11_USAGE_DYNAMIC` | `D3D11_CPU_ACCESS_WRITE` | `WRITE_DISCARD`, `WRITE_NO_OVERWRITE` | The “dynamic upload” path for VB/IB/CB updates. |
| `D3D11_USAGE_STAGING` | `D3D11_CPU_ACCESS_READ` | `READ` | Readback staging. Must synchronize (see §4). |
| `D3D11_USAGE_STAGING` | `D3D11_CPU_ACCESS_WRITE` | `WRITE` | CPU-only upload staging; later copied to DEFAULT resource. |
| `D3D11_USAGE_STAGING` | `D3D11_CPU_ACCESS_READ \| D3D11_CPU_ACCESS_WRITE` | `READ_WRITE` | Less common; treat as valid. |

Other validation:

* `D3D11_MAP_FLAG_DO_NOT_WAIT` is only legal if it’s the only bit set in `Flags`. Unknown bits must fail.
* `Subresource` must be within the resource’s subresource count.
* `pfnMap` must fail if the same subresource is already mapped.

#### 5.2 Error reporting for invalid Map/Unmap

* `pfnMap` must return an `HRESULT`:
  * `E_INVALIDARG` for illegal MapType/Flags/usage/subresource combinations.
  * `DXGI_ERROR_WAS_STILL_DRAWING` for DO_NOT_WAIT contention.
* `pfnUnmap` is `void`:
  * if the Unmap arguments are invalid (unknown resource, bad subresource, Unmap without a prior successful Map), report via the runtime callback `pfnSetErrorCb(<device-handle>, E_INVALIDARG)` (using whatever handle type your headers declare: `HRTDEVICE` vs `HDEVICE`) and return.

Do **not** silently ignore invalid Unmap in AeroGPU; hiding these errors makes runtime/device-state bugs extremely difficult to diagnose.

---

### KMD-side notes (why `DxgkDdiLock/Unlock` matter and what “coherent” means)

#### 6.1 Why `DxgkDdiLock` / `DxgkDdiUnlock` are usually required

On Win7 WDDM, the runtime’s `pfnLockCb` typically relies on the KMD’s `DxgkDdiLock` / `DxgkDdiUnlock` implementation to:

* validate that the allocation is CPU-mappable,
* return a stable CPU virtual address for the allocation/subresource,
* enforce synchronization against in-flight GPU usage (or return “still drawing” for DO_NOT_WAIT), and
* apply cache policy / flushing rules.

If AeroGPU’s KMD does not implement Lock/Unlock correctly, common failure modes include:

* `Map(READ)` returning stale data (copy completed on “GPU”, but CPU reads old bytes)
* DO_NOT_WAIT never returning `DXGI_ERROR_WAS_STILL_DRAWING` (leading to app hangs or unexpected stalls)
* runtime returning a pointer that becomes invalid or aliases unrelated memory

The minimal AeroGPU KMD architecture doc already calls out `DxgkDdiLock/Unlock` as required plumbing for CPU access (see §4.3 in [`win7-wddm11-aerogpu-driver.md`](windows7-aerogpu-wddm-driver.md)).

#### 6.2 Cache coherency expectations for staging readback

For staging readback correctness, the following must be true:

* When the UMD returns from `pfnMap(READ)` successfully, the bytes visible at `pMapped->pData` must reflect the **completed GPU write**.
* If AeroGPU’s emulator/host writes into guest allocations (system memory), the fence completion signal must not occur until those writes are globally visible to the CPU thread that will read them.

In other words: **“Fence complete” implies “data visible to CPU”** for readback destinations.

For a virtual GPU this often means:

* perform the host-side copy into the guest allocation memory *before* raising the completion interrupt / advancing the completed fence, and
* use appropriate host memory ordering primitives so the guest CPU thread cannot observe completion without observing the writes.

---

### Pseudocode: recommended `pfnMap` / `pfnUnmap` control flow

This pseudocode is intentionally explicit about where flushing, locking, and error handling occur.

#### 7.1 `pfnMap`

```c
HRESULT APIENTRY Map(hContext, hResource, Subresource, MapType, MapFlags, pOut) {
  if (!pOut) return E_INVALIDARG;

  Resource* res = lookup_resource(hResource);
  if (!res) return E_INVALIDARG;

  if (!validate_subresource(res, Subresource)) return E_INVALIDARG;
  if (!validate_usage_and_map_type(res, MapType, MapFlags)) return E_INVALIDARG;

  bool do_not_wait = (MapFlags & D3D11_MAP_FLAG_DO_NOT_WAIT) != 0;
  bool needs_gpu_sync = map_requires_sync(res, MapType);

  if (needs_gpu_sync) {
    // Important: submit any pending GPU work that may produce the readback bytes.
    flush_pending_work(hContext);
  }

  // Translate to runtime lock.
  D3DDDICB_LOCK lock = {};
  lock.hAllocation = res->allocation_handle;
  // Field spelling varies by WDK revision (`SubResourceIndex` / `SubresourceIndex`);
  // use the exact name exposed by the header you build against.
  lock.SubresourceIndex = Subresource;
  lock.Flags = translate_map_to_lockflags(MapType);
  // Field spelling varies by WDK revision (`DoNotWait` / `DonotWait`); use the
  // exact name exposed by the header you build against.
  lock.Flags.DoNotWait = do_not_wait ? 1 : 0;

  HRESULT hr = callbacks->pfnLockCb(hRTDevice, &lock);
  if (do_not_wait && (hr == DXGI_ERROR_WAS_STILL_DRAWING ||
                      hr == HRESULT_FROM_NT(STATUS_GRAPHICS_GPU_BUSY) ||
                      hr == E_PENDING ||
                      is_timeout_hr(hr))) {
    // The D3D11 API contract requires DXGI_ERROR_WAS_STILL_DRAWING for DO_NOT_WAIT.
    // Some Win7/WDDM 1.1 paths report “busy” using other HRESULTs; normalize them.
    return DXGI_ERROR_WAS_STILL_DRAWING;
  }
  if (FAILED(hr)) return hr;

  // The runtime filled lock.pData + pitch metadata.
  pOut->pData = lock.pData;
  pOut->RowPitch = lock.Pitch;
  pOut->DepthPitch = lock.SlicePitch;

  mark_mapped(res, Subresource, MapType);
  return S_OK;
}
```

#### 7.2 `pfnUnmap`

```c
void APIENTRY Unmap(hContext, hResource, Subresource) {
  Resource* res = lookup_resource(hResource);
  if (!res || !is_mapped(res, Subresource)) {
    callbacks->pfnSetErrorCb(<device-handle>, E_INVALIDARG);
    return;
  }

  // If this was a write map, ensure subsequent GPU use sees the data.
  // (In AeroGPU, this may mean emitting an upload command or marking the
  // allocation as dirty so the host reads from guest memory on next use.)
  if (last_map_was_write(res, Subresource)) {
    commit_cpu_writes(res, Subresource);
  }

  D3DDDICB_UNLOCK unlock = {};
  unlock.hAllocation = res->allocation_handle;
  // Field spelling varies by WDK revision (`SubResourceIndex` / `SubresourceIndex`);
  // use the exact name exposed by the header you build against.
  unlock.SubresourceIndex = Subresource;

  HRESULT hr = callbacks->pfnUnlockCb(hRTDevice, &unlock);
  if (FAILED(hr)) {
    callbacks->pfnSetErrorCb(<device-handle>, hr);
  }

  clear_mapped(res, Subresource);
}
```

---

### “Definition of done” for AeroGPU Map/Unmap on Win7

An implementation matches Win7 expectations when:

* `drivers/aerogpu/tests/win7/d3d11_triangle` reliably reads the expected center/corner pixels via staging `Map(READ)`.
* `drivers/aerogpu/tests/win7/readback_sanity` reliably reads expected pixels via staging `Map(READ)`.
* `drivers/aerogpu/tests/win7/d3d11_map_roundtrip` reliably round-trips a staging texture via `Map(WRITE)` + `Unmap` + `Map(READ)`.
* `Map(DO_NOT_WAIT)` returns `DXGI_ERROR_WAS_STILL_DRAWING` when the staging destination is still busy (validated by `drivers/aerogpu/tests/win7/d3d11_map_do_not_wait`).
* Invalid Map usage returns `E_INVALIDARG` and invalid Unmap reports `E_INVALIDARG` via `pfnSetErrorCb` (no silent success).

## Runtime callbacks, submission, and fences

This document pins down the **exact Windows 7 (WDDM 1.1) symbol names** (types, struct fields, and callback entrypoints) that matter for a D3D10/D3D11 **user-mode display driver (UMD)** implementing:

- DMA buffer allocation (command buffer acquisition)
- command submission (**render** and **present**)
- error reporting from `void` DDIs
- fence wait/poll for `Map(READ)` (staging readback)
- WOW64 (32-bit UMD on x64) ABI gotchas

It is intended to be used *together with* the Win7-era D3D UMD headers shipped with a Windows SDK/WDK install (WDK10+ supported), including:

- `d3d10umddi.h`, `d3d10_1umddi.h`
- `d3d11umddi.h`
- shared: `d3dumddi.h`, `d3dkmthk.h`

Clean-room note: this document **does not** include sample-driver code. It references only the public WDK DDI contracts and describes call flow and field usage.

Implementation note (AeroGPU):

- The in-repo Win7/WDDM 1.1 submission + fence wait implementation lives in
  `drivers/aerogpu/umd/d3d10_11/src/aerogpu_d3d10_11_wddm_submit.{h,cpp}` and is
  wired up by the WDK UMD entrypoints (`aerogpu_d3d10_1_umd_wdk.cpp` /
  `aerogpu_d3d11_umd_wdk.cpp`).

Related docs:

- High-level bring-up checklist: `windows7-d3d-umd-ddi.md`
- D3D11 Map/Unmap + LockCb/UnlockCb semantics (Win7): `windows7-d3d-umd-ddi.md`
- D3D11 function-table checklist (REQUIRED vs stubbable DDIs): `windows7-d3d-umd-ddi.md`
- KMD submission/fence architecture: `windows7-aerogpu-wddm-driver.md`

---

### Naming conventions / what “callbacks” means here

Windows D3D10/11 UMDs interact with three “layers” of function tables:

1. **UMD exports** (OS loads your DLL and calls these): `OpenAdapter10`, `OpenAdapter10_2`, `OpenAdapter11`.
2. **UMD-provided function tables** (runtime calls into these): `D3D10DDI_ADAPTERFUNCS`/`D3D11DDI_ADAPTERFUNCS`, `D3D10DDI_DEVICEFUNCS`/`D3D11DDI_DEVICEFUNCS`, and `D3D11DDI_DEVICECONTEXTFUNCS`.
3. **Runtime-provided callback tables** (UMD calls into these): `D3D10DDI_ADAPTERCALLBACKS`/`D3D11DDI_ADAPTERCALLBACKS` and `D3D10DDI_DEVICECALLBACKS`/`D3D11DDI_DEVICECALLBACKS` (plus the shared `D3DDDI_DEVICECALLBACKS`-style “CB” entrypoints in `d3dumddi.h`).

This doc focuses on (3): the callbacks the UMD uses for **submission and synchronization**, and where you receive them.

---

### Callback tables provided to the UMD (OpenAdapter + CreateDevice)

#### 1.1 OpenAdapter time (adapter callbacks)

**Exports (Win7):**

- D3D10: `HRESULT APIENTRY OpenAdapter10(D3D10DDIARG_OPENADAPTER* pOpenData)`
- D3D10.1: `HRESULT APIENTRY OpenAdapter10_2(D3D10DDIARG_OPENADAPTER* pOpenData)`
- D3D11 (Win7): `HRESULT APIENTRY OpenAdapter11(D3D10DDIARG_OPENADAPTER* pOpenData)`
  - On Windows 7, `OpenAdapter11` still receives a `D3D10DDIARG_OPENADAPTER` container; the D3D11-specific DDIs begin at device creation/caps.

**The OpenAdapter container: `D3D10DDIARG_OPENADAPTER`**

Fields that matter for submission work later:

- `D3D10DDI_HRTADAPTER hRTAdapter` — runtime-owned adapter handle (opaque to the driver).
- `D3D10DDI_HADAPTER hAdapter` — driver-owned adapter handle (`.pDrvPrivate` points at your adapter object).
- `const D3D10DDI_ADAPTERCALLBACKS* pAdapterCallbacks` — runtime callback table you must store.
- `D3D10DDI_ADAPTERFUNCS* pAdapterFuncs` — output table you fill (at minimum: `pfnCreateDevice`, `pfnCloseAdapter`, `pfnGetCaps`, `pfnCalcPrivateDeviceSize`).
- `UINT Interface` / `UINT Version` — interface/version negotiation.

> The exact adapter callback table type you receive depends on which OpenAdapter export is used:
>
> - D3D10/10.1: `D3D10DDI_ADAPTERCALLBACKS`
> - D3D11: `D3D11DDI_ADAPTERCALLBACKS` (still delivered through `D3D10DDIARG_OPENADAPTER` on Win7).

**Callbacks worth knowing about (for later device bring-up):**

- `pfnQueryAdapterInfoCb`-style callbacks (adapter info queries)
- allocations and residency are typically device-scoped (covered below), not adapter-scoped

For *submission* specifically, you mostly care about storing `hRTAdapter` and getting to `CreateDevice`, where the device callbacks are provided.

#### 1.2 CreateDevice time (device callbacks)

##### D3D10: `D3D10DDIARG_CREATEDEVICE`

The runtime calls your `D3D10DDI_ADAPTERFUNCS::pfnCreateDevice(...)`, passing a `D3D10DDIARG_CREATEDEVICE`.

Fields that matter for submission/sync:

- `D3D10DDI_HDEVICE hDevice` — driver device handle (where your device object lives).
- `D3D10DDI_HRTDEVICE hRTDevice` — runtime device handle (store it; needed for `pfnSetErrorCb`).
- Runtime callbacks (naming varies across Win7-capable header vintages):
  - `const D3D10DDI_DEVICECALLBACKS* pCallbacks`
    - contains `pfnSetErrorCb` for reporting errors from `void` DDIs
  - Some WDKs also expose `const D3DDDI_DEVICECALLBACKS* pUMCallbacks`
    - this is the shared `d3dumddi.h` callback table containing the submission/sync entrypoints (`pfnCreateContextCb2`, `pfnAllocateCb`, `pfnRenderCb`, `pfnPresentCb`, `pfnWaitForSynchronizationObjectCb`, etc).
- `D3D10DDI_DEVICEFUNCS* pDeviceFuncs` — output function table you fill.

##### D3D11: `D3D11DDIARG_CREATEDEVICE`

The runtime calls your `D3D11DDI_ADAPTERFUNCS::pfnCreateDevice(...)`, passing a `D3D11DDIARG_CREATEDEVICE`.

Fields that matter for submission/sync:

- `D3D11DDI_HDEVICE hDevice`
- `D3D11DDI_HRTDEVICE hRTDevice` (present in Win7 headers; used by `pfnSetErrorCb` in header revisions where that callback takes `HRTDEVICE`)
- `D3D11DDI_HDEVICECONTEXT hImmediateContext` (Win7 immediate context handle you own)
- Runtime callbacks (naming varies across Win7-capable header vintages):
  - `const D3D11DDI_DEVICECALLBACKS* pCallbacks` **or** `pDeviceCallbacks`
    - contains `pfnSetErrorCb` for reporting errors from `void` DDIs
  - Some WDKs also expose `const D3DDDI_DEVICECALLBACKS* pUMCallbacks`
    - this is the shared `d3dumddi.h` callback table containing the submission/sync entrypoints (`pfnAllocateCb`, `pfnRenderCb`, `pfnPresentCb`, `pfnWaitForSynchronizationObjectCb`, etc).
- output tables:
  - `D3D11DDI_DEVICEFUNCS* pDeviceFuncs`
  - `D3D11DDI_DEVICECONTEXTFUNCS* pDeviceContextFuncs`

> **Why this matters:** the D3D11 runtime will call `pfnMap`/`pfnFlush` on the device-context table, so your fence tracking must be reachable from the context object too.

---

### Error reporting from `void` DDIs (Win7 D3D10/D3D11)

Many D3D10/D3D11 DDIs are declared `void APIENTRY ...(...)` and **cannot return** an `HRESULT`.

#### 2.1 The callback: `pfnSetErrorCb`

On WDDM 1.1 / Win7, the D3D10/D3D11 runtimes provide an error callback named:

- `pfnSetErrorCb`

It is reachable from the **device callbacks** you receive during `CreateDevice`:

- D3D10: `D3D10DDIARG_CREATEDEVICE::pCallbacks->pfnSetErrorCb`
- D3D11: `D3D11DDIARG_CREATEDEVICE::{pCallbacks|pDeviceCallbacks}->pfnSetErrorCb`

**Signature (Win7 header variations):**

- D3D10 (and D3D10.1): commonly either:
  - `pfnSetErrorCb(D3D10DDI_HRTDEVICE, HRESULT)`, or
  - `pfnSetErrorCb(D3D10DDI_HDEVICE, HRESULT)`
- D3D11: commonly either:
  - `pfnSetErrorCb(D3D11DDI_HRTDEVICE, HRESULT)`, or
  - `pfnSetErrorCb(D3D11DDI_HDEVICE, HRESULT)`

#### 2.2 Rule: “set error then return”

When a `void` DDI encounters an error:

1. Call `pfnSetErrorCb(<handle>, hr)` (using the handle type required by your header’s prototype).
2. Return immediately (do not continue executing the DDI).

The runtime associates the error with the originating API call.

#### 2.3 Acceptable `HRESULT` values (practical Win7 set)

Use **specific** errors. The common “safe set” for Win7 bring-up:

- `E_OUTOFMEMORY` — allocation failure (including inability to get a DMA buffer).
  - AeroGPU note: if you see `E_OUTOFMEMORY` “too early” (while the guest still has free RAM), you may be hitting Win7’s WDDM segment budget. AeroGPU is system-memory-backed, but dxgkrnl still enforces the KMD-reported non-local segment size; tune `HKR\Parameters\NonLocalMemorySizeMB` (see `windows7-aerogpu-validation.md` appendix and `drivers/aerogpu/kmd/README.md`).
- `E_INVALIDARG` — runtime provided invalid arguments (should be rare; runtime usually validates).
- `E_NOTIMPL` — feature not implemented but the call was reached anyway.

Device-removal style errors are also valid but should be used only for genuine “device is broken” scenarios:

- `DXGI_ERROR_DEVICE_REMOVED`
- `DXGI_ERROR_DEVICE_HUNG`
- `DXGI_ERROR_DEVICE_RESET`

Avoid `E_FAIL` for predictable conditions; it makes debugging harder and can push the runtime into harsh recovery paths.

---

### Win7 submission model: acquire DMA buffer → fill → submit

On Win7/WDDM 1.1, a D3D10/11 UMD submits work by building:

- a **DMA buffer** (your command stream)
- an **allocation list** describing referenced allocations
- an optional **patch-location list** (relocations)
- optional **DMA-buffer private data** (per-submission sideband blob)

#### 3.0.1 AeroGPU note: the allocation list is also the input to `aerogpu_alloc_table`

For AeroGPU guest-backed resources, the KMD builds the per-submit `aerogpu_alloc_table` (used to resolve protocol `backing_alloc_id`) from the submission’s WDDM allocation list (`DXGK_ALLOCATIONLIST`).

Practical implication for UMDs:

- Any submission that includes packets requiring `alloc_id` resolution must include the corresponding WDDM allocation handle(s) in the submit allocation list so the KMD can provide the `alloc_id → gpa` mapping to the host. This includes:
  - `CREATE_*` packets with `backing_alloc_id != 0`
  - `AEROGPU_CMD_RESOURCE_DIRTY_RANGE` for guest-backed resources
  - `COPY_*` packets with `WRITEBACK_DST` (staging readback)
- Do not rely solely on “currently bound” state when building the list: these packets may be emitted while the resource is not bound and still require the allocation to be listed for that submit.
- The allocation list also carries **per-allocation write intent** via the WDDM 1.1 `WriteOperation` bit (`DXGK_ALLOCATIONLIST::Flags.Value & 0x1`), which the AeroGPU KMD propagates into `aerogpu_alloc_entry.flags` as `AEROGPU_ALLOC_FLAG_READONLY` when the allocation is not written by the submission. The host rejects any guest-memory writeback (e.g. `COPY_* WRITEBACK_DST`) into a READONLY allocation, so UMDs must ensure writeback destinations are marked writable for that submission.

Then it submits via the runtime callbacks which route into:

- KMD `DxgkDdiRender` for “render” submissions, or
- KMD `DxgkDdiPresent` for “present” submissions,

followed by dxgkrnl scheduling and eventual KMD `DxgkDdiSubmitCommand`.

> AeroGPU note: the DMA buffer payload is an AeroGPU command stream:
> `drivers/aerogpu/protocol/aerogpu_cmd.h` (`aerogpu_cmd_stream_header` followed
> by `aerogpu_cmd_hdr` packets). The stream header’s `magic` and `abi_version`
> must match, and `size_bytes` must be within the submitted buffer length
> (`<= cmd_size_bytes`; any trailing bytes beyond `size_bytes` are ignored). The
> in-repo Win7 submission backend
> (`drivers/aerogpu/umd/d3d10_11/src/aerogpu_d3d10_11_wddm_submit.cpp`)
> validates this before submission.

#### 3.1 Create the kernel device + context (get `hContext`, `hSyncObject`, and initial DMA buffer pointers)

The submission callbacks in `d3dumddi.h` are **context-scoped**: before you can submit anything, you need:

- a kernel device handle (`D3DKMT_HANDLE hDevice`),
- a kernel context handle (`D3DKMT_HANDLE hContext`), and
- a synchronization object (`D3DKMT_HANDLE hSyncObject`) you can wait on with a target fence value.

On Win7, these are created via callbacks in the shared device callback table:

- `D3DDDI_DEVICECALLBACKS::pfnCreateDeviceCb`
- `D3DDDI_DEVICECALLBACKS::pfnCreateContextCb2` (preferred on WDDM 1.1) or `pfnCreateContextCb`

##### Create the kernel device: `pfnCreateDeviceCb` + `D3DDDICB_CREATEDEVICE`

Struct:

- `D3DDDICB_CREATEDEVICE`

Important fields:

- `HANDLE hAdapter` (input) — the adapter handle you returned from `OpenAdapter*` (for D3D10/11 this is typically the `.pDrvPrivate` pointer behind `D3D10DDI_HADAPTER` / `D3D11DDI_HADAPTER`).
- `D3DKMT_HANDLE hDevice` (output) — kernel device handle; store it.

##### Create the kernel context: `pfnCreateContextCb2`/`pfnCreateContextCb` + `D3DDDICB_CREATECONTEXT`

Struct:

- `D3DDDICB_CREATECONTEXT`

Important inputs:

- `D3DKMT_HANDLE hDevice` — the kernel device handle from `pfnCreateDeviceCb`.
- `UINT NodeOrdinal` — set to `0` for a single-node MVP.
- `UINT EngineAffinity` — set to `0` for a single-engine MVP.
- `Flags` — **zero-initialize** for bring-up unless you know you need a bit.
- `VOID* pPrivateDriverData` / `UINT PrivateDriverDataSize` — bring-up can pass `NULL`/`0` unless your KMD needs context-private data.

Important outputs:

- `D3DKMT_HANDLE hContext` — kernel context handle; pass this to render/present/wait CBs.
- `D3DKMT_HANDLE hSyncObject` — monitored-fence synchronization object; pass this in wait calls.
- Initial submission buffers (owned by the runtime; treat as the “current DMA buffer”):
  - `VOID* pCommandBuffer` + `UINT CommandBufferSize` (**bytes**)
  - `D3DDDI_ALLOCATIONLIST* pAllocationList` + `UINT AllocationListSize` (**entries**)
  - `D3DDDI_PATCHLOCATIONLIST* pPatchLocationList` + `UINT PatchLocationListSize` (**entries**)
  - If your header exposes it: `VOID* pDmaBufferPrivateData` + `UINT DmaBufferPrivateDataSize` (**bytes**)

> **Key Win7 rule:** the runtime is allowed to **rotate** DMA buffers and lists over time. After each submission, update your stored pointers/sizes from whatever “out” fields your header exposes (see render/present notes below).

##### Lifetime / cleanup callbacks

At shutdown, these additional `D3DDDI_DEVICECALLBACKS` entries may exist (check for presence in your headers):

- `pfnDestroySynchronizationObjectCb` (takes a struct with `hSyncObject`)
- `pfnDestroyContextCb` (takes a struct with `hContext`)
- `pfnDestroyDeviceCb` (takes a struct with `hDevice`)

#### 3.2 The core submission structs (d3dumddi.h)

The *shared* WDDM 1.x CB structs used by D3D10/11 are declared in `d3dumddi.h`:

The corresponding **function pointers** are in the shared runtime callback table:

- `D3DDDI_DEVICECALLBACKS`
  - `pfnCreateDeviceCb`
  - `pfnCreateContextCb2` / `pfnCreateContextCb`
  - `pfnAllocateCb`
  - `pfnDeallocateCb`
  - `pfnGetCommandBufferCb`
  - `pfnRenderCb`
  - `pfnPresentCb`
  - `pfnWaitForSynchronizationObjectCb`

##### Allocate a DMA buffer (common Win7 D3D10/11 pattern): `pfnAllocateCb`

Callback:

- `pfnAllocateCb`

Struct:

- `D3DDDICB_ALLOCATE`

Important fields (header names vary slightly across WDK vintages; both names are common):

- Requested/returned command buffer capacity (bytes):
  - `UINT DmaBufferSize` **or** `UINT CommandBufferSize`
- Output pointers (memory owned by runtime/OS for this DMA buffer instance):
  - `VOID* pDmaBuffer` **or** `VOID* pCommandBuffer`
  - `D3DDDI_ALLOCATIONLIST* pAllocationList` + `UINT AllocationListSize` (entries)
  - `D3DDDI_PATCHLOCATIONLIST* pPatchLocationList` + `UINT PatchLocationListSize` (entries)
  - If exposed by your header: `VOID* pDmaBufferPrivateData` + `UINT DmaBufferPrivateDataSize` (bytes)
- Some headers also include `D3DKMT_HANDLE hContext` (context-scoped allocation); if present, you must fill it.

Rule: treat the size fields returned by the callback as **hard capacities** and never write past them.

##### Return a DMA buffer to the runtime: `pfnDeallocateCb`

Callback:

- `pfnDeallocateCb`

Struct:

- `D3DDDICB_DEALLOCATE`

Important fields:

- Pass back the same pointers you received from `D3DDDICB_ALLOCATE`:
  - `pDmaBuffer`/`pCommandBuffer`
  - `pAllocationList`
  - `pPatchLocationList`
  - (and `pDmaBufferPrivateData` if your header includes it)

> **Lifecycle rule:** For the allocate/deallocate model, always call `pfnDeallocateCb` after you finish submitting (even on failure paths) so the runtime can recycle its DMA buffer backing.

##### Acquire / (re)acquire a command buffer (`pfnGetCommandBufferCb`)

Callback:

- `pfnGetCommandBufferCb`

Struct:

- `D3DDDICB_GETCOMMANDINFO`

CreateContext already provides the **initial** `pCommandBuffer` / lists. `pfnGetCommandBufferCb` is the runtime entrypoint used to acquire a *fresh* DMA buffer instance (and is the standard place where the UMD receives a pointer to `pDmaBufferPrivateData`).

Important fields (header names):

- `D3DKMT_HANDLE hContext` — kernel context handle to build commands for.
- output pointers (memory owned by runtime/OS for this DMA buffer instance):
  - `VOID* pCommandBuffer`
  - `D3DDDI_ALLOCATIONLIST* pAllocationList`
  - `D3DDDI_PATCHLOCATIONLIST* pPatchLocationList`
  - `VOID* pDmaBufferPrivateData`
- output capacities (max sizes you are allowed to write):
  - `UINT CommandBufferSize` (bytes)
  - `UINT AllocationListSize` (count of `D3DDDI_ALLOCATIONLIST` entries)
  - `UINT PatchLocationListSize` (count of `D3DDDI_PATCHLOCATIONLIST` entries)
  - `UINT DmaBufferPrivateDataSize` (bytes)

> The capacity fields are critical: **do not write past them**. If you need more space, end the current buffer and submit, then acquire a new one.

##### Submit a render DMA buffer

Callback:

- `pfnRenderCb`

Struct:

- `D3DDDICB_RENDER`

Important fields:

- `D3DKMT_HANDLE hContext`
- `UINT CommandLength` (bytes written to `pCommandBuffer`)
- `VOID* pCommandBuffer`
- If your header uses the “DMA buffer” naming:
  - `VOID* pDmaBuffer` (often the same pointer as `pCommandBuffer`)
  - `UINT DmaBufferSize` (bytes; often the same value as `CommandLength`)
- `UINT CommandBufferSize` (bytes; some WDKs include this as an in/out field)
- Allocation list fields (**header drift warning**):
  - Legacy structs use `UINT AllocationListSize` as the **used count** (entries).
  - Some Win7-era structs instead split **capacity vs. used** across:
    - `UINT AllocationListSize` (**capacity**) and
    - `UINT NumAllocations` (**used count**)
- Patch list fields (AeroGPU typically uses an empty patch list, but fields must be consistent):
  - Legacy structs use `UINT PatchLocationListSize` as the **used count**.
  - Some structs split **capacity vs. used** across:
    - `UINT PatchLocationListSize` (**capacity**) and
    - `UINT NumPatchLocations` (**used count**)
- `VOID* pDmaBufferPrivateData`

Fence output (Win7 pattern):

- `UINT64 NewFenceValue` (written by the callback on success; use as the target value when waiting for completion via `WaitForSynchronizationObject`)
- Some headers instead expose a 32-bit `UINT SubmissionFenceId`; if so, treat it as a monotonically increasing fence value and widen to `UINT64` when waiting.

> **Buffer rotation:** in some Win7-era header revisions, `D3DDDICB_RENDER` treats the buffer/list pointer+size fields as **IN/OUT** (you pass the current buffers, and on return the runtime may overwrite them with the next buffers/capacities). If your header has this behavior, update your stored `pCommandBuffer` / `pAllocationList` / `pPatchLocationList` and their capacities after each successful submit.

##### Submit a present DMA buffer

Callback:

- `pfnPresentCb`

Struct:

- `D3DDDICB_PRESENT`

Important common submission fields (present has additional present-specific fields; see the header):

- `D3DKMT_HANDLE hContext`
- `UINT CommandLength` (bytes)
- `VOID* pCommandBuffer`
- If your header uses the “DMA buffer” naming:
  - `VOID* pDmaBuffer` (often the same pointer as `pCommandBuffer`)
  - `UINT DmaBufferSize` (bytes; often the same value as `CommandLength`)
- `UINT CommandBufferSize` (bytes; some WDKs include this as an in/out field)
- `UINT AllocationListSize` (count) + `D3DDDI_ALLOCATIONLIST* pAllocationList`
- `UINT PatchLocationListSize` (count) + `D3DDDI_PATCHLOCATIONLIST* pPatchLocationList`
- `VOID* pDmaBufferPrivateData`

Fence output (Win7 pattern):

- `UINT64 NewFenceValue` (written by the callback on success)
- Some headers instead expose a 32-bit `UINT SubmissionFenceId`; if so, treat it as a monotonically increasing fence value and widen to `UINT64` when waiting.

#### 3.3 Minimal call sequence (render submission)

At a “flush boundary” (e.g. `D3D10DDI_DEVICEFUNCS::pfnFlush` or `D3D11DDI_DEVICECONTEXTFUNCS::pfnFlush`):

1. **Ensure you have a context** (once per device, at bring-up):
   - `pfnCreateDeviceCb` → kernel `hDevice`
   - `pfnCreateContextCb2`/`pfnCreateContextCb` → `hContext`, `hSyncObject`, and an initial `pCommandBuffer` + list pointers/capacities.
2. **Acquire** a DMA buffer (per submission):
   - Preferred Win7 D3D10/11 pattern: `pfnAllocateCb` + `D3DDDICB_ALLOCATE`
     - Fill requested sizes (e.g. `CommandBufferSize`/`DmaBufferSize`, list sizes, and `hContext` if present).
     - Call `pfnAllocateCb(&alloc)` and read back the pointers/capacities.
   - Alternative names in some headers:
     - `pfnGetCommandBufferCb` + `D3DDDICB_GETCOMMANDINFO`
   - Some runtimes/context interfaces also provide a “current” DMA buffer via `CreateContext` and/or rotate it through in/out submit structs.
3. **Fill**:
   - Write your DMA stream to `pCommandBuffer`.
   - Write allocation references into `pAllocationList[0..N)`.
   - Write patch entries into `pPatchLocationList[0..M)` (for AeroGPU, typically `M=0`).
   - If `pDmaBufferPrivateData != NULL`, write per-submit metadata into it (fixed-size).
4. **Submit**:
      - Fill `D3DDDICB_RENDER`:
       - `hContext = ...`
       - `CommandLength = <bytes actually written>`
       - `pCommandBuffer = pCommandBuffer`
       - if your header uses `pDmaBuffer`/`DmaBufferSize`, set them consistently:
         - `pDmaBuffer = pCommandBuffer`
         - `DmaBufferSize = CommandLength`
        - Allocation list:
          - If `NumAllocations` exists: `AllocationListSize = <capacity>`, `NumAllocations = N`
          - Else: `AllocationListSize = N`
          - Always set `pAllocationList = pAllocationList`
        - Patch list (AeroGPU uses `M=0`, but match struct layout):
          - If `NumPatchLocations` exists: `PatchLocationListSize = <capacity>`, `NumPatchLocations = M`
          - Else: `PatchLocationListSize = M`
          - Always set `pPatchLocationList` consistently (either `NULL` with size 0, or a valid pointer with size 0)
        - `pDmaBufferPrivateData = pDmaBufferPrivateData` (or `NULL` if not used / size is 0)
     - Call `pfnRenderCb(&render)`.
     - On success:
       - read back `render.NewFenceValue` (or `render.SubmissionFenceId`) and treat it as the fence value for this submission (store it as “last submitted”, and use it to update per-resource “last write fence” tracking)
       - if your header treats buffer/list fields as in/out, update your stored pointers/capacities from `render` for the next submission.
5. **Return** the DMA buffer (if you used `pfnAllocateCb`):
   - Call `pfnDeallocateCb(&dealloc)` with the same pointers you received from `D3DDDICB_ALLOCATE`.

#### 3.4 Minimal call sequence (present submission)

In the runtime’s **Present DDI** (Win7 DXGI 1.1; uses `D3D10DDIARG_PRESENT` even for D3D11 devices):

- D3D10 / D3D10.1: `D3D10DDI_DEVICEFUNCS::pfnPresent`
- D3D11: `D3D11DDI_DEVICECONTEXTFUNCS::pfnPresent`

1. Flush/submit any outstanding render work that must precede present.
2. Acquire a DMA buffer (either via `pfnAllocateCb`, `pfnGetCommandBufferCb`, or by using the current runtime-provided buffer pointers).
3. Encode your present command(s) into the DMA buffer (e.g. an `AEROGPU_CMD_PRESENT` / `AEROGPU_CMD_PRESENT_EX` packet; scanout selection is done via MMIO `SCANOUT0_*` registers, not by referencing a backbuffer in the packet).
4. Submit via `pfnPresentCb(&present)`.
5. On success, read back `present.NewFenceValue` (or `present.SubmissionFenceId`) and treat it as the fence value for the present submission (useful for throttling and for “present implies completion” queries).
6. If you used `pfnAllocateCb`, return the DMA buffer via `pfnDeallocateCb`.

#### 3.5 Patch lists: “empty is valid” if you design for it

If your DMA stream never embeds GPU virtual addresses/relocations (AeroGPU command streams use protocol object handles and `alloc_id` lookups, not GPU virtual addresses), you can submit with:

- `PatchLocationListSize = 0`
- `pPatchLocationList = NULL` (or a valid pointer with 0 size)

**Do not** put uninitialized junk in the patch list. If `PatchLocationListSize != 0`, dxgkrnl and the KMD may attempt to interpret it.

#### 3.6 DMA buffer private data (`pDmaBufferPrivateData`)

The private-data blob is sized by the KMD via `DXGK_DRIVERCAPS::DmaBufferPrivateDataSize`.

Where you receive the pointer depends on the exact Win7-era header/interface revision:

- Common D3D10/11 pattern: `D3DDDICB_ALLOCATE::pDmaBufferPrivateData` with capacity `DmaBufferPrivateDataSize`
- Common path: `D3DDDICB_GETCOMMANDINFO::pDmaBufferPrivateData` with capacity `DmaBufferPrivateDataSize`
- Some headers also surface it alongside the initial DMA buffer in `D3DDDICB_CREATECONTEXT` and/or treat it as an in/out field on submit structs. In that case, treat it as part of your “current DMA buffer state” just like `pCommandBuffer`.

Rules:

- Treat it as **opaque fixed-size bytes** shared with the KMD.
- Use a fixed-width, pointer-free layout (see WOW64 notes).
- Typical uses:
  - classify submissions (render vs present vs paging)
  - include a tiny “submission header” version for debugging

---

### Fence wait/poll for `Map(READ)` on Win7

#### 4.1 What `Map(READ)` needs (D3D11 staging readback)

For `D3D11_MAP_READ` on a staging resource, the UMD must ensure:

- the GPU copy into the staging resource has completed, and
- the CPU mapping observes the completed data.

Note: the *CPU pointer itself* is returned through the runtime lock callbacks (`pfnLockCb` / `pfnUnlockCb` using `D3DDDICB_LOCK` / `D3DDDICB_UNLOCK`). The full Win7 Map/Unmap + lock-flag translation contract is documented in:

- `windows7-d3d-umd-ddi.md`

In the repo’s Win7 tests (`drivers/aerogpu/tests/win7/readback_sanity`), the pattern is:

1. `CopyResource(staging, renderTarget)`
2. `Flush()`
3. `Map(staging, D3D11_MAP_READ, Flags=0)`

So the UMD’s `Map` must block (or poll+block) on a fence.

#### 4.2 The Win7 wait callback (preferred): `pfnWaitForSynchronizationObjectCb`

The shared wait CB entrypoint lives in `d3dumddi.h`:

- Callback: `pfnWaitForSynchronizationObjectCb`
- Struct: `D3DDDICB_WAITFORSYNCHRONIZATIONOBJECT`

Important fields:

- `D3DKMT_HANDLE hContext` — context whose sync objects/fences are relevant.
- `UINT ObjectCount`
- `const D3DKMT_HANDLE* ObjectHandleArray` (one per sync object)
- `const UINT64* FenceValueArray` (target values; one per sync object)
- `UINT64 Timeout` (milliseconds; `0` is a poll, `~0ULL` is effectively “infinite wait”)

**Which sync object handle to wait on:**

- Use the `hSyncObject` returned by `pfnCreateContextCb2`/`pfnCreateContextCb` (see §3.1). This is the monitored-fence object whose value advances with your submissions.

**How to pick the target fence value:**

- Track a monotonically increasing fence/timeline value per submission.
  - On Win7, this is typically the `NewFenceValue` (or `SubmissionFenceId`, depending on header revision) returned by the last `pfnRenderCb` / `pfnPresentCb` submission that produced the data you need.
- Store “last write fence” on resources that are written by the GPU.
- When mapping for read, wait for `completed >= last_write_fence`.

**Polling (for DO_NOT_WAIT paths):**

- Call the wait callback with `Timeout = 0`.
- If it indicates not-ready (timeout), return `DXGI_ERROR_WAS_STILL_DRAWING` from the `Map` DDI (D3D11), or (for `void`-returning map variants) call `pfnSetErrorCb(<device-handle>, DXGI_ERROR_WAS_STILL_DRAWING)` (using the handle type your header expects: `HRTDEVICE` vs `HDEVICE`) and return.
  - Note: different Win7-era stacks report "not ready" using different HRESULTs (e.g. `HRESULT_FROM_WIN32(WAIT_TIMEOUT)`, `HRESULT_FROM_WIN32(ERROR_TIMEOUT)`, `HRESULT_FROM_NT(STATUS_TIMEOUT)` (0x10000102), and sometimes `HRESULT_FROM_NT(STATUS_GRAPHICS_GPU_BUSY)` (0xD01E0102)). For DO_NOT_WAIT semantics, normalize these to `DXGI_ERROR_WAS_STILL_DRAWING`.

Practical Win7 note: the wait callback does not always report "not ready" as `DXGI_ERROR_WAS_STILL_DRAWING`. Depending on header/runtime vintage, a poll (`Timeout = 0`) may yield one of several timeout/pending HRESULTs; treat them as still-drawing for `Map(DO_NOT_WAIT)`:

- `DXGI_ERROR_WAS_STILL_DRAWING`
- `HRESULT_FROM_NT(STATUS_GRAPHICS_GPU_BUSY)`
- `HRESULT_FROM_WIN32(WAIT_TIMEOUT)`
- `HRESULT_FROM_WIN32(ERROR_TIMEOUT)`
- `HRESULT_FROM_NT(STATUS_TIMEOUT)` (`0x10000102`; `SUCCEEDED()`, so don't rely solely on `FAILED(hr)`)
- `E_PENDING` (`0x8000000A`) (seen in some stacks, typically for `Timeout = 0` polls)

#### 4.3 Direct thunk alternative: `D3DKMTWaitForSynchronizationObject`

If you are not using the runtime’s wait callback (e.g., in standalone tooling), you can call the kernel thunk directly:

- Function: `NTSTATUS APIENTRY D3DKMTWaitForSynchronizationObject(D3DKMT_WAITFORSYNCHRONIZATIONOBJECT* pData)`
- Struct: `D3DKMT_WAITFORSYNCHRONIZATIONOBJECT` (in `d3dkmthk.h`)

Important fields (header names):

- `D3DKMT_HANDLE hAdapter`
- `UINT ObjectCount`
- `const D3DKMT_HANDLE* ObjectHandleArray`
- `const UINT64* FenceValueArray`
- `UINT64 Timeout` (**milliseconds**; `0` is a poll, `~0ULL` is effectively “infinite wait”)

The “target fence value” is specified via `FenceValueArray[i]` for each sync object handle in `ObjectHandleArray[i]`.

**Recommendation:** in a real UMD, prefer the runtime callback if available; it keeps the driver insulated from some OS-version quirks and ensures WOW64 thunking is correct.

#### 4.4 Getting a kernel `hAdapter` (`D3DKMT_HANDLE`) for direct KMT calls

`D3DKMTWaitForSynchronizationObject` (and other `D3DKMT*` thunks such as
`D3DKMTEscape`) operate on **kernel object handles** like `D3DKMT_HANDLE hAdapter`,
not the UMD’s `D3D10DDI_HADAPTER`/`D3D11DDI_HADAPTER` `.pDrvPrivate` pointer.

In particular:

- `D3DDDICB_CREATEDEVICE::hAdapter` is the adapter handle you returned from
  `OpenAdapter*` (typically your adapter object pointer), *not* a `D3DKMT_HANDLE`.
- The wait callback form (`pfnWaitForSynchronizationObjectCb`) usually does not
  require a kernel `hAdapter`, but some Win7-era header variants include an
  `hAdapter` field in the wait args; if present, fill it with a real
  `D3DKMT_HANDLE`.

Common approach in user mode:

1. Create a display DC (e.g. `CreateDCW(L"DISPLAY", L"\\\\.\\DISPLAY1", ...)` or
   enumerate the primary display).
2. Call `D3DKMTOpenAdapterFromHdc` (exported from `gdi32.dll` on Win7) to obtain
   a `D3DKMT_HANDLE`.
3. Close the DC.
4. Close the adapter handle at teardown with `D3DKMTCloseAdapter`.

This gives you the kernel adapter handle needed for direct thunk calls and for
debug-only escape plumbing.

---

### WOW64 notes (32-bit UMD on x64)

Windows 7 x64 will load **both**:

- a 64-bit UMD for 64-bit processes, and
- a 32-bit UMD under WOW64 for 32-bit processes.

Both UMDs talk to the same 64-bit kernel (dxgkrnl + your x64 KMD). The biggest pitfalls are ABI and “binary blob” layouts.

#### 5.1 Handle sizes: pointer-sized vs 32-bit

**Pointer-sized (differs between x86 and x64 UMD builds):**

- D3D10/11 DDI handles like `D3D10DDI_HRESOURCE`, `D3D10DDI_HDEVICE`, `D3D11DDI_HDEVICECONTEXT`, etc:
  - these are wrapper structs containing `.pDrvPrivate` pointers.

**Always 32-bit (even on x64):**

- `D3DKMT_HANDLE` (declared as a `UINT`)
  - used for kernel objects: adapter/device/context/allocation/synchronization object handles.

Rule: never assume `sizeof(D3DKMT_HANDLE) == sizeof(void*)`.

#### 5.2 Packing pitfalls

- Do not use `#pragma pack(1)` globally in a UMD; it will break the WDK struct ABI.
- Ensure all compilation units that include `d3d10umddi.h` / `d3d11umddi.h` see the default packing expected by the headers.

#### 5.3 The critical cross-arch blob: `pDmaBufferPrivateData`

`pDmaBufferPrivateData` is a **binary packet shared between UMD and KMD**.

On x64:

- the **KMD** is always 64-bit, and it defines the size via `DXGK_DRIVERCAPS::DmaBufferPrivateDataSize`.
- the **32-bit UMD** still receives a buffer of that x64-defined size.

Therefore:

- The private-data layout must be **explicitly architecture-independent**.
- **Do not embed pointers** (user-mode or kernel-mode) in this blob.
- Use fixed-width integers (`uint32_t`, `uint64_t`) and explicit padding if needed.

#### 5.4 Calling D3DKMT directly under WOW64

If you call `D3DKMT*` thunks directly:

- call the documented user-mode exports (typically from `gdi32.dll`), not private syscalls
- do not hand-roll struct layouts; include the SDK/WDK `d3dkmthk.h`

This ensures the WOW64 layer performs the correct pointer-size translation for thunk parameter structs.

#### 5.5 x86 stdcall export decoration (common loader gotcha)

On x86 (including WOW64), the UMD exports are `__stdcall`, so the *raw* symbol names are decorated with a stack size suffix:

- `_OpenAdapter10@4`
- `_OpenAdapter10_2@4`
- `_OpenAdapter11@4`

However, the runtime uses undecorated names (`"OpenAdapter10"`, etc) with `GetProcAddress`, so your DLL must also export:

- `OpenAdapter10`, `OpenAdapter10_2`, `OpenAdapter11`

In AeroGPU this is handled by `.def` files:

- `drivers/aerogpu/umd/d3d10_11/aerogpu_d3d10_x86.def` (x86, maps undecorated → decorated)
- `drivers/aerogpu/umd/d3d10_11/aerogpu_d3d10_x64.def` (x64, no `@N` decoration)

#### 5.6 64-bit monitored fence reads on x86 (torn read hazard)

The monitored fence value exposed by `D3DDDICB_CREATECONTEXT` (field name varies:
`pMonitoredFenceValue` vs `pFenceValue`) is a **64-bit counter** that is updated
by the kernel/GPU while user mode is reading it.

On a **32-bit UMD** (including WOW64), a plain `*volatile uint64_t` read can be
**torn** (two independent 32-bit loads), producing a transient garbage value.

Practical guidance:

- Prefer `pfnWaitForSynchronizationObjectCb` for correctness (it avoids direct
  reads).
- If you do read the fence value, use an atomic 64-bit read primitive such as
  an interlocked operation (`InterlockedCompareExchange64` used as a read) or a
  “read high/low twice” loop.
- Some stacks map the fence page read-only; avoid atomic helpers that may write
  to the page on a compare match (use a sentinel comparand that should never
  occur, or use a read-only-safe technique).

In AeroGPU, the canonical implementation is in:

- `drivers/aerogpu/umd/d3d10_11/src/aerogpu_d3d10_11_wddm_submit.cpp`
  - x86 uses a “read high/low/high until stable” loop to avoid torn reads without
    issuing interlocked writes to the fence page.

---

### Optional: Win7 WDK layout probe tool (sizeof/offsetof)

To catch header/version mismatches early (especially when switching between SDKs/WDKs or x86/x64),
the repo includes a small Windows-only probe you can build with any toolchain that provides the
Win7-era D3D10/11 UMD DDI headers:

- `drivers/aerogpu/tools/win7_wdk_probe/`

It includes the Win7 D3D10/11 UMD DDI headers and prints `sizeof`/`offsetof` for:

- CreateResource allocation contract (resource backing allocations):
  - `D3D10DDIARG_CREATERESOURCE`
  - `D3D11DDIARG_CREATERESOURCE`
  - `D3DDDI_ALLOCATIONINFO`
  - `D3DDDICB_ALLOCATE` / `D3DDDICB_DEALLOCATE` (resource-allocation members)
- CreateDevice wiring (where `pCallbacks` / `pUMCallbacks` live):
  - `D3D10DDIARG_CREATEDEVICE`
  - `D3D11DDIARG_CREATEDEVICE`
- Context bring-up:
  - `D3DDDICB_CREATEDEVICE`
  - `D3DDDICB_CREATECONTEXT`
- DMA buffer acquisition/release:
  - `D3DDDICB_ALLOCATE`
  - `D3DDDICB_DEALLOCATE`
- `D3DDDICB_GETCOMMANDINFO`
- `D3DDDICB_RENDER`
- `D3DDDICB_PRESENT`
- `D3DDDICB_WAITFORSYNCHRONIZATIONOBJECT`
- `D3DKMT_WAITFORSYNCHRONIZATIONOBJECT`

This is not part of runtime/CI; it is a developer-side sanity check.

---

### Appendix A) “What you actually need to implement” checklist (submission/fences only)

To implement correct Win7 submission + `Map(READ)` synchronization in a D3D10/11 UMD, you will need:

1. Store runtime callbacks from:
   - `D3D10DDIARG_OPENADAPTER::pAdapterCallbacks`
   - D3D10 CreateDevice:
     - `D3D10DDIARG_CREATEDEVICE::pCallbacks` (D3D10 wrapper callbacks; includes `pfnSetErrorCb`)
     - if present: `D3D10DDIARG_CREATEDEVICE::pUMCallbacks` (shared `D3DDDI_DEVICECALLBACKS` submission table)
   - D3D11 CreateDevice:
     - `D3D11DDIARG_CREATEDEVICE::{pCallbacks|pDeviceCallbacks}` (D3D11 wrapper callbacks; includes `pfnSetErrorCb`)
     - if present: `D3D11DDIARG_CREATEDEVICE::pUMCallbacks` (shared `D3DDDI_DEVICECALLBACKS` submission table)
2. Implement `pfnSetErrorCb` usage for all failing `void` DDIs.
3. Create and store kernel submission state via `d3dumddi.h` callbacks:
   - `pfnCreateDeviceCb` + `D3DDDICB_CREATEDEVICE` → `hDevice`
   - `pfnCreateContextCb2`/`pfnCreateContextCb` + `D3DDDICB_CREATECONTEXT` → `hContext`, `hSyncObject`, and initial DMA buffer pointers/capacities
4. Implement “acquire/allocate → fill → submit”:
   - acquire (common D3D10/11 path): `pfnAllocateCb` + `D3DDDICB_ALLOCATE`
   - release: `pfnDeallocateCb` + `D3DDDICB_DEALLOCATE`
   - alternate acquire naming in some headers: `pfnGetCommandBufferCb` + `D3DDDICB_GETCOMMANDINFO`
   - submit render: `pfnRenderCb` + `D3DDDICB_RENDER`
   - submit present: `pfnPresentCb` + `D3DDDICB_PRESENT`
   - update stored DMA buffer pointers if the submit structs are in/out in your header revision.
5. Track per-resource “last GPU write fence” and implement `Map(READ)` wait:
   - `pfnWaitForSynchronizationObjectCb` + `D3DDDICB_WAITFORSYNCHRONIZATIONOBJECT` with `ObjectHandleArray[0] = hSyncObject`

## Function tables: required versus stubbable

This is an implementation-grade reference for bringing up a **crash-free** D3D11 UMD on
**Windows 7 SP1 (WDDM 1.1 / DXGI 1.1)**.

It answers a very practical question:

> When the Win7 D3D11 runtime loads your UMD, which `d3d11umddi.h` function table entries
> must be non-null, which ones must actually *work* for `D3D_FEATURE_LEVEL_10_0`, and which
> ones can be safely stubbed with `E_NOTIMPL` / `SetErrorCb(E_NOTIMPL)` until later?

This doc is intentionally biased toward a **safe skeleton**:

* **Never leave a DDI function pointer NULL.** If the runtime calls a NULL pointer, you crash the process.
* Prefer “present but failing cleanly” over “missing”.
* Keep `pfnGetCaps` conservative: do not advertise features you don’t implement end-to-end.

> Related bring-up doc: `windows7-d3d-umd-ddi.md` (conceptual bring-up plan and minimal feature set).
>
> Callback/fence symbol-name reference (Win7 WDK): `windows7-d3d-umd-ddi.md`
>
> Repo pointers (AeroGPU implementation):
> * UMD code: `drivers/aerogpu/umd/d3d10_11/`
> * Win7 D3D11 guest tests referenced below:
>   * `drivers/aerogpu/tests/win7/d3d11_caps_smoke`
>   * `drivers/aerogpu/tests/win7/d3d11_triangle`
>   * `drivers/aerogpu/tests/win7/d3d11_texture_sampling_sanity`
>   * `drivers/aerogpu/tests/win7/d3d11_dynamic_constant_buffer_sanity`
>   * `drivers/aerogpu/tests/win7/d3d11_depth_test_sanity`
>   * `drivers/aerogpu/tests/win7/d3d11_update_subresource_texture_sanity`
>   * `drivers/aerogpu/tests/win7/d3d11_map_dynamic_buffer_sanity`
>   * `drivers/aerogpu/tests/win7/d3d11_rs_om_state_sanity`
>   * `drivers/aerogpu/tests/win7/d3d11_swapchain_rotate_sanity`
>   * `drivers/aerogpu/tests/win7/d3d11_geometry_shader_smoke`
>   * `drivers/aerogpu/tests/win7/readback_sanity`

---

### TL;DR: minimal non-null + must-work set (FL10_0 + core Win7 D3D11 tests)

If your goal is “the Win7 runtime creates a D3D11 device at **FL10_0** and the core Win7 D3D11 tests don’t crash”,
this is the smallest practical set to treat as **must be non-null and must succeed**.

> Tests referenced:
> * `drivers/aerogpu/tests/win7/d3d11_triangle`
> * `drivers/aerogpu/tests/win7/d3d11_texture_sampling_sanity`
> * `drivers/aerogpu/tests/win7/d3d11_dynamic_constant_buffer_sanity`
> * `drivers/aerogpu/tests/win7/d3d11_depth_test_sanity`
> * `drivers/aerogpu/tests/win7/readback_sanity`
>
> See §6.6+ for additional Win7 D3D11 tests that tighten semantics (swapchain rotation, scissor/blend, geometry shaders, etc).

#### Adapter (`D3D11DDI_ADAPTERFUNCS`)

Must be non-null and must succeed:

* `pfnGetCaps`
* `pfnCalcPrivateDeviceSize`
* `pfnCalcPrivateDeviceContextSize` (if present in your `D3D11DDI_ADAPTERFUNCS` layout)
* `pfnCreateDevice` (must fill both device + immediate context tables)
* `pfnCloseAdapter`

#### Device funcs (`D3D11DDI_DEVICEFUNCS`)

Must be non-null and must succeed for the tests:

* Device lifetime:
  * `pfnDestroyDevice`
* Resources:
  * `pfnCalcPrivateResourceSize`, `pfnCreateResource`, `pfnDestroyResource`
  * must handle (at minimum):
    * `D3D11_USAGE_DEFAULT` buffers created with `D3D11_SUBRESOURCE_DATA` (initial data upload)
    * `D3D11_USAGE_DEFAULT` `Texture2D` render targets (BGRA)
    * `D3D11_USAGE_STAGING` `Texture2D` with `CPU_ACCESS_READ` (staging readback)
* RTV:
  * `pfnCalcPrivateRenderTargetViewSize`, `pfnCreateRenderTargetView`, `pfnDestroyRenderTargetView`
* SRV + sampler:
  * `pfnCalcPrivateShaderResourceViewSize`, `pfnCreateShaderResourceView`, `pfnDestroyShaderResourceView`
  * `pfnCalcPrivateSamplerSize`, `pfnCreateSampler`, `pfnDestroySampler`
* Depth/stencil:
  * `pfnCalcPrivateDepthStencilViewSize`, `pfnCreateDepthStencilView`, `pfnDestroyDepthStencilView`
  * `pfnCalcPrivateDepthStencilStateSize`, `pfnCreateDepthStencilState`, `pfnDestroyDepthStencilState`
* Shaders:
  * `pfnCalcPrivateVertexShaderSize`, `pfnCreateVertexShader`, `pfnDestroyVertexShader`
  * `pfnCalcPrivatePixelShaderSize`, `pfnCreatePixelShader`, `pfnDestroyPixelShader`
* Input layout:
  * `pfnCalcPrivateElementLayoutSize`, `pfnCreateElementLayout`, `pfnDestroyElementLayout`

Everything else should still be **non-null** (stubbed), but may return `E_NOTIMPL`.

#### Immediate context (`D3D11DDI_DEVICECONTEXTFUNCS`)

Must be non-null and must succeed for the tests:

* Binding/state:
  * `pfnSetRenderTargets`
  * `pfnSetViewports`
  * `pfnIaSetInputLayout`, `pfnIaSetTopology`, `pfnIaSetVertexBuffers`, `pfnIaSetIndexBuffer`
  * `pfnVsSetShader`, `pfnPsSetShader`
  * `pfnVsSetShaderResources`, `pfnPsSetShaderResources`
  * `pfnVsSetSamplers`, `pfnPsSetSamplers`
  * `pfnVsSetConstantBuffers`, `pfnPsSetConstantBuffers` (dynamic constant buffer binding)
  * `pfnSetDepthStencilState`
* Clears/draws:
  * `pfnClearRenderTargetView`
  * `pfnClearDepthStencilView`
  * `pfnDraw`, `pfnDrawIndexed`
* Resource updates:
  * `pfnUpdateSubresourceUP`
* Readback path:
  * `pfnCopyResource`
  * `pfnFlush`
  * `pfnMap`, `pfnUnmap`
* Win7 DXGI present integration (swapchains):
  * `pfnPresent` (DXGI uses `D3D10DDIARG_PRESENT` even for D3D11 devices on Win7)
  * `pfnRotateResourceIdentities`

Everything else should still be **non-null** (stubbed, usually via `SetErrorCb(E_NOTIMPL)` for `void` DDIs),
because the runtime may call “reset to default” entrypoints like `ClearState` during initialization.

Practical stub tip:

* Many state-setting DDIs are called with **NULL** handles specifically to unbind/reset state.
  For those “unbind” patterns, it’s usually better to treat the call as a no-op success (no `SetErrorCb`),
  otherwise you can end up reporting errors during normal initialization/`ClearState` even though the app is not using the missing feature.
  * Examples seen in practice on Win7: HS/DS/CS binds (tessellation/compute), `SoSetTargets`, `SetPredication`,
    and debug markers / discards (`SetMarker`/`BeginEvent`/`EndEvent`, `DiscardResource`, `DiscardView`).

---

### Terminology and rules used in this checklist

#### Status tags

Each entrypoint is marked as one of:

* **REQUIRED**: must be non-null and implemented correctly for a functional **FL10_0** device (and the “core bring-up” tests listed in the TL;DR section above).
* **REQUIRED-BUT-STUBBABLE**: must be non-null (the runtime *may* call it), but it can fail cleanly until the feature is implemented.
* **OPTIONAL**: not required for FL10_0 bring-up; can usually be stubbed and may never be called unless the app opts into the feature.

#### Stubbing failure modes (`HRESULT` vs `SetErrorCb`)

The D3D11 UMD DDI has two error-reporting styles:

* **`HRESULT`-returning DDIs**: return `E_NOTIMPL` / `E_INVALIDARG` / `E_OUTOFMEMORY` as appropriate.
  * AeroGPU note: if you see `E_OUTOFMEMORY` “too early” (while the guest still has free RAM), you may be hitting Win7’s WDDM segment budget rather than true exhaustion. AeroGPU is system-memory-backed, but dxgkrnl still enforces the KMD-reported non-local segment size; tune `HKR\Parameters\NonLocalMemorySizeMB` (see `windows7-aerogpu-validation.md` appendix and `drivers/aerogpu/kmd/README.md`).
* **`void` DDIs**: report failure through the runtime callback (commonly `pfnSetErrorCb(...)`) and return.

Practical rule:

* If the DDI is `void`, use: `pfnSetErrorCb(<device-handle>, E_NOTIMPL)` (or `E_INVALIDARG`) using whatever handle type your headers declare (`D3D11DDI_HRTDEVICE` vs `D3D11DDI_HDEVICE`).
* If the DDI returns `HRESULT`, return the error code directly.

Do **not** “half-stub” a `void` DDI by silently doing nothing if it is supposed to create/modify state the runtime relies on; that often leads to later GPU hangs or invalid command streams.

Important detail: most `void` DDIs live on the **device context table** and are called as:

* `pfnSomething(D3D11DDI_HDEVICECONTEXT hCtx, ...)` (no `hDevice` parameter)

But the error callback is device-scoped and Win7-era WDK headers disagree on whether it expects the **runtime device handle** (`D3D11DDI_HRTDEVICE`) or the driver `D3D11DDI_HDEVICE`.

In practice that means your context-private struct should point back to the parent device object so you can reach the stored device handle and call:

* `pfnSetErrorCb(<device-handle>, E_NOTIMPL);`

For exact Win7 WDK symbol names/fields (`D3D11DDIARG_CREATEDEVICE::hRTDevice`, `...::pCallbacks->pfnSetErrorCb`, etc), see:

* `windows7-d3d-umd-ddi.md`

#### Non-null discipline: stub-fill, then override

For Win7 stability, the simplest pattern is:

1. Build a “fully stubbed” `D3D11DDI_ADAPTERFUNCS` / `D3D11DDI_DEVICEFUNCS` / `D3D11DDI_DEVICECONTEXTFUNCS` where **every field is non-null**.
2. In `OpenAdapter11` and `pfnCreateDevice`, start from the stub table and overwrite only the functions you’ve implemented.

This is robust against:

* “surprise” runtime calls into rarely-used entrypoints during initialization, and
* adding fields when you switch `D3D11DDI_INTERFACE_VERSION` (new fields defaulting to NULL is a common crash source).

Pseudocode shape:

```c
// 1) A stub that matches the failure style of the DDI entrypoint.
static HRESULT APIENTRY Stub_HRESULT(...) { return E_NOTIMPL; }
static void APIENTRY Stub_VOID(D3D11DDI_HDEVICECONTEXT hCtx, ...) {
  g_DeviceCallbacks.pfnSetErrorCb(DeviceHandleFromContext(hCtx), E_NOTIMPL);
}

// 2) A fully-populated table (every field assigned).
static const D3D11DDI_DEVICEFUNCS kStubDeviceFuncs = { /* ...all fields... */ };
static const D3D11DDI_DEVICECONTEXTFUNCS kStubCtxFuncs = { /* ...all fields... */ };

// 3) In CreateDevice: copy then override.
*pCreateDevice->pDeviceFuncs = kStubDeviceFuncs;
*pCreateDevice->pDeviceContextFuncs = kStubCtxFuncs;
pCreateDevice->pDeviceFuncs->pfnCreateResource = &MyCreateResource;
pCreateDevice->pDeviceContextFuncs->pfnDraw = &MyDraw;
```

Don’t overthink the stub implementation: `E_NOTIMPL` + `SetErrorCb(E_NOTIMPL)` is enough as long as it never dereferences invalid handles.

#### Stub templates by signature (copy/paste starting point)

Most of the D3D11 UMD DDI surface fits into a few signature patterns. For a skeleton driver, it’s common to implement a small set of generic stubs and use them to populate the tables.

```c
// CalcPrivate*Size: runtime uses this to allocate hXxx.pDrvPrivate storage.
static SIZE_T APIENTRY Stub_CalcPrivateSize(...) {
  return sizeof(uint64_t); // keep non-zero; easiest to reason about
}

// Create*: HRESULT-returning (common for object creation).
static HRESULT APIENTRY Stub_Create_HRESULT(...) {
  return E_NOTIMPL;
}

// Destroy*: void-returning (common for object destruction).
static void APIENTRY Stub_Destroy_VOID(...) {
  // Must be safe on partially-initialized objects.
}

// Context-state setters and draws are usually void and take HDEVICECONTEXT first.
//
// For bring-up, consider special-casing “unbind” calls as a no-op (do not set
// an error) since the runtime will frequently reset state by binding NULLs.
static void APIENTRY Stub_Ctx_VOID(D3D11DDI_HDEVICECONTEXT hCtx, ...) {
  g_DeviceCallbacks.pfnSetErrorCb(DeviceHandleFromContext(hCtx), E_NOTIMPL);
}
```

These are intentionally “dumb but safe”. Once you start implementing a feature, override the specific entrypoints while leaving unrelated ones stubbed.

---

### Win7 loader flow (what calls what, in what order)

On Win7, the D3D11 runtime loads your UMD (a DLL) and uses the exported `OpenAdapter11` entrypoint to obtain an adapter function table.

High-level call flow:

```text
LoadLibrary(<your_umd>.dll)
  GetProcAddress("OpenAdapter11")
    OpenAdapter11(D3D10DDIARG_OPENADAPTER* pOpenData)
      -> driver fills: D3D11DDI_ADAPTERFUNCS (adapter function table)
      -> driver stores: runtime callback tables (adapter/device callbacks; used for `SetErrorCb`, allocation callbacks, etc)

    runtime calls adapter->pfnGetCaps(...)  [multiple queries]
    runtime calls adapter->pfnCalcPrivateDeviceSize(...)
    runtime allocates driver-private memory for the handles it passes to CreateDevice
    (at least a `D3D11DDI_HDEVICE`, and typically an immediate `D3D11DDI_HDEVICECONTEXT` as well).

    runtime calls adapter->pfnCreateDevice(...)
      -> driver constructs device + immediate context in provided private memory
      -> driver fills BOTH:
           D3D11DDI_DEVICEFUNCS         (device/object creation & lifetime)
           D3D11DDI_DEVICECONTEXTFUNCS  (immediate context: state, draws, copies, map/unmap, flush)
```

#### 1.1 Callback tables: what you must store to report errors safely

The runtime provides callback tables at adapter/device creation time. You must store them in your private adapter/device objects and treat them as **valid only until the corresponding Close/Destroy call**.

At minimum you need the callback that reports errors from `void` DDIs:

* `pfnSetErrorCb` (device-scoped; see §0 “Stubbing failure modes” for the context-vs-device detail)

Practical guidance:

* Store callbacks in the object that “owns” the handle they are associated with:
  * adapter callbacks in the adapter private struct
  * device callbacks in the device private struct
  * context private struct should point back to the parent device (so it can reach the stored device handle and call `pfnSetErrorCb`)
* Never call callbacks after `pfnCloseAdapter` / `pfnDestroyDevice`.

Win7-specific gotchas:

* `OpenAdapter11` is declared as `HRESULT APIENTRY OpenAdapter11(D3D10DDIARG_OPENADAPTER *pOpenData)` on Win7:
  the container is still `D3D10DDIARG_OPENADAPTER` even though you return **D3D11** tables.
* DXGI 1.1 swapchains drive present through the D3D10-style present structures:
  * `D3D10DDIARG_PRESENT` is used even for D3D11 devices.
  * buffer rotation uses `pfnRotateResourceIdentities`.

#### 1.2 Interface version negotiation (`D3D11DDI_INTERFACE_VERSION`)

The Win7 D3D11 runtime uses `D3D10DDIARG_OPENADAPTER::Interface` / `::Version` as an ABI negotiation step:

* `Interface` must match the D3D11 DDI selector (commonly `D3D11DDI_INTERFACE_VERSION`; some WDKs also expose `D3D11DDI_INTERFACE`)
* `Version` determines the expected struct layout for the device/context function tables

If you accept an unsupported `Version`, the runtime may interpret your filled
`D3D11DDI_DEVICEFUNCS` / `D3D11DDI_DEVICECONTEXTFUNCS` with the wrong layout and crash.

Recommended driver behavior:

* `OpenAdapter11` validates the incoming interface/version.
* If the runtime requests a newer `Version` than you support, clamp `pOpenData->Version` down to your supported version (commonly `D3D11DDI_SUPPORTED` or `D3D11DDI_INTERFACE_VERSION`, depending on the WDK), matching the D3D10.x negotiation pattern.
* Store the negotiated `Version` in adapter-private state and ensure `pfnCreateDevice` fills
  `D3D11DDI_DEVICEFUNCS` / `D3D11DDI_DEVICECONTEXTFUNCS` matching that struct layout.

---

### Adapter function table: `D3D11DDI_ADAPTERFUNCS`

You return `D3D11DDI_ADAPTERFUNCS` from `OpenAdapter11`. On Win7, treat every field as **must be non-null**.

If your chosen `D3D11DDI_INTERFACE_VERSION` adds adapter-func fields beyond the ones listed here, apply the same rule:

* keep the pointer **non-null**, and
* return a clean failure (`E_NOTIMPL` / `E_INVALIDARG`) rather than leaving it NULL.

| Field | Status | Must succeed? | Notes / failure guidance |
|---|---|---:|---|
| `pfnGetCaps` | REQUIRED | **Yes** for the “minimum caps set” in §3 | Return conservative answers; unknown `Type` must not crash. |
| `pfnCalcPrivateDeviceSize` | REQUIRED | Yes | Must return a valid non-zero size for your `D3D11DDI_HDEVICE` private storage (and, depending on interface version, may include immediate context storage). |
| `pfnCreateDevice` | REQUIRED | Yes | Must fill `D3D11DDI_DEVICEFUNCS` + `D3D11DDI_DEVICECONTEXTFUNCS` and return `S_OK`. |
| `pfnCloseAdapter` | REQUIRED | N/A | Free adapter-private state; never call back into the runtime after closing. |

---

### `pfnGetCaps`: minimum `D3D11DDIARG_GETCAPS::Type` coverage for FL10_0

`pfnGetCaps` is where the D3D11 runtime learns what you support. Device creation is gated by the results.

#### 3.1 “Unknown caps types” must be handled gracefully

This is a reliability requirement: **Win7 will probe more caps than you expect**, and the probe set differs by OS patch level.

Recommended robust behavior:

1. Treat `pData == NULL` as a “size query” when possible:
   * For fixed-size outputs, set `DataSize = sizeof(<expected struct>)` (if `DataSize` is in/out for your header version) and return `S_OK`.
   * For variable-size outputs, report the required size for the current adapter/device configuration and return `S_OK`.
2. For non-null `pData`, validate `DataSize` is at least what you need for that `Type` (fail with `E_INVALIDARG` rather than overrunning the buffer).
3. If `Type` is unknown:
   * **zero-fill** `pData` (up to `DataSize`) and return `S_OK`, **or**
   * return `E_INVALIDARG` (only if you’ve confirmed Win7 runtime tolerates failure for that `Type`).
4. Log unknown `Type` values (once) so you can expand coverage intentionally.

#### 3.2 Minimum caps queries that typically gate device creation (FL10_0)

The exact set can vary, but in practice the Win7 D3D11 runtime usually needs at least:

| `D3D11DDIARG_GETCAPS::Type` | Required? | What to return (conservative baseline) |
|---|---:|---|
| `D3D11DDICAPS_TYPE_FEATURE_LEVELS` | Yes | Return a feature level list containing `D3D_FEATURE_LEVEL_10_0` (and *only* the levels you truly support). In practice, Win7-era runtimes/headers may interpret the buffer as either `{ UINT NumFeatureLevels; const D3D_FEATURE_LEVEL* pFeatureLevels; }` **or** `{ UINT NumFeatureLevels; D3D_FEATURE_LEVEL FeatureLevels[...]; }` (inline). When possible (x64), populate both layouts to avoid mismatched interpretation. On x86 the pointer field overlaps the first inline element, so you **cannot** satisfy both simultaneously; prefer the `{count, pointer}` layout because returning an inline `D3D_FEATURE_LEVEL` value where a pointer is expected can crash the runtime (attempting to dereference e.g. `0xA000`). |
| `D3D11DDICAPS_TYPE_THREADING` | Yes | Disable advanced threading unless implemented: `DriverConcurrentCreates = FALSE`, `DriverCommandLists = FALSE`. |
| `D3D11DDICAPS_TYPE_SHADER` | Yes | Claim only SM4.x for FL10_0: VS/GS/PS `*_4_0`-class support; no SM5-only stages. The output begins with per-stage shader-model “version tokens” (`UINT`) in DXBC encoding: `(program_type << 16) | (major << 4) | minor` (typically PS/VS/GS/HS/DS/CS order), with unimplemented stages left as 0. |
| `D3D11DDICAPS_TYPE_FORMAT` | Yes | Report support for the formats you need for DXGI swapchains + staging readback (see §3.3). |
| `D3D11DDICAPS_TYPE_D3D10_X_HARDWARE_OPTIONS` | Recommended | For FL10_0 bring-up: set `ComputeShaders_Plus_RawAndStructuredBuffers_Via_Shader_4_x = FALSE` unless you implement CS + raw/structured buffers. |
| `D3D11DDICAPS_TYPE_D3D11_OPTIONS` | Recommended | Return all options `FALSE` initially (no UAV-only features, no logic ops, etc). |
| `D3D11DDICAPS_TYPE_ARCHITECTURE_INFO` | Recommended | Conservative: `TileBasedDeferredRenderer = FALSE`, `UMA = FALSE`, `CacheCoherentUMA = FALSE`. |
| `D3D11DDICAPS_TYPE_DOUBLES` | Recommended | For FL10_0 bring-up: return `DoublePrecisionFloatShaderOps = FALSE`. |
| `D3D11DDICAPS_TYPE_MULTISAMPLE_QUALITY_LEVELS` | Recommended | If asked for `SampleCount==1`, return `NumQualityLevels >= 1` **only** for supported formats (Win7 apps often probe this early). Return `0` for unsupported formats so DXGI/D3D won’t pick an MSAA path for a format you can’t create. |

> Why “Recommended” is still important: many apps call `ID3D11Device::CheckFeatureSupport(...)`
> early. Even if the runtime can create a device without these, returning garbage here causes
> surprising app behavior.

#### 3.3 Minimum *format* support required by the repo’s Win7 tests

For the current guest tests, the runtime needs at least:

| Format | Required usages (minimum) | Where it’s used |
|---|---|---|
| `DXGI_FORMAT_B8G8R8A8_UNORM` | `RENDER_TARGET`, `TEXTURE2D` | swapchain backbuffer in `d3d11_triangle` and render-target texture in `readback_sanity`. |
| `DXGI_FORMAT_R8G8B8A8_UNORM` | `RENDER_TARGET`, `TEXTURE2D` | common app fallback; good to support early even if tests use BGRA. |
| Depth formats (`DXGI_FORMAT_D24_UNORM_S8_UINT` or `DXGI_FORMAT_D32_FLOAT`) | `DEPTH_STENCIL`, `TEXTURE2D` | required by `d3d11_depth_test_sanity`; common for real apps. |
| `DXGI_FORMAT_R16_UINT` / `DXGI_FORMAT_R32_UINT` | `BUFFER`, `IA_INDEX_BUFFER` | required by `d3d11_caps_smoke` and common for indexed draws. |
| `DXGI_FORMAT_R32G32_FLOAT`, `DXGI_FORMAT_R32G32B32A32_FLOAT` | `BUFFER`, `IA_VERTEX_BUFFER` | required for the repo’s input layouts (multiple tests). |

BGRA device flag note:

* The Win7 tests create the device with `D3D11_CREATE_DEVICE_BGRA_SUPPORT`.
* In practice this means you must report BGRA support in `pfnGetCaps` (format caps) and successfully create BGRA render targets, or `D3D11CreateDevice*` may fail early.

Staging readback path requirements:

* `CopyResource` / `CopySubresourceRegion` must be able to copy from a DEFAULT render target into a STAGING texture.
* `Map(D3D11_MAP_READ)` on that staging texture must succeed.

If you are not ready to support a format for a given usage:

* make `D3D11DDICAPS_TYPE_FORMAT` report it as unsupported, **and**
* return a clean failure from the corresponding create call (`E_INVALIDARG` or `E_NOTIMPL`).

---

### Device function table: `D3D11DDI_DEVICEFUNCS` checklist

This is the “device-level” function table you fill in `pfnCreateDevice`. It covers object lifetime and creation.

#### 4.1 Minimum rule for crash-free bring-up

Populate **every** field in `D3D11DDI_DEVICEFUNCS` with a non-null function pointer. Even if you are not implementing a feature, provide a stub:

* `CalcPrivate*Size` returns a non-zero size (often `sizeof(YourDummyObject)`).
* `Create*` returns `E_NOTIMPL` / `E_INVALIDARG` if unsupported.
* `Destroy*` is a safe no-op if the object was never successfully created.

If a field exists in your `d3d11umddi.h` but is not explicitly mentioned in this doc, treat it as:

* **OPTIONAL** for FL10_0 bring-up, and
* still **non-null** (stubbed).

#### 4.2 Function pointer checklist (grouped)

> Note: For any “Create*” below: if you don’t support the object yet, return `E_NOTIMPL` and do not touch the handle’s private memory.

##### 4.2.1 Device lifecycle

| Field | Status | Stub failure mode |
|---|---|---|
| `pfnDestroyDevice` | REQUIRED | N/A (must work; freeing device is not optional). |

##### 4.2.2 Core resources (buffers + textures)

| Field | Status | Stub failure mode |
|---|---|---|
| `pfnCalcPrivateResourceSize` | REQUIRED | Return `sizeof(resource)` (even for unsupported resource kinds). |
| `pfnCreateResource` | REQUIRED | `HRESULT`: `E_NOTIMPL` for unsupported descs; `E_INVALIDARG` for invalid descs. |
| `pfnDestroyResource` | REQUIRED | `void`: must be safe on partially-initialized objects. |

Optional but common for real apps:

| Field | Status | Stub failure mode |
|---|---|---|
| `pfnOpenResource` | REQUIRED-BUT-STUBBABLE | `HRESULT`: `E_NOTIMPL` is acceptable for early bring-up, but breaks DXGI/D3D11 shared-resource interop (`ID3D11Device::OpenSharedResource`). AeroGPU implements this in the Win7/WDDM 1.1 WDK UMD build (see `drivers/aerogpu/tests/win7/d3d11_shared_surface_ipc/`). |

##### 4.2.3 Views (SRV / RTV / DSV / UAV)

| Field | Status | Stub failure mode |
|---|---|---|
| `pfnCalcPrivateShaderResourceViewSize` | REQUIRED | Return `sizeof(SRV)`. |
| `pfnCreateShaderResourceView` | REQUIRED | `HRESULT`: must succeed for Texture2D SRVs (used by `d3d11_texture_sampling_sanity`). |
| `pfnDestroyShaderResourceView` | REQUIRED | `void` no-op is OK. |
| `pfnCalcPrivateRenderTargetViewSize` | REQUIRED | Return `sizeof(RTV)`. |
| `pfnCreateRenderTargetView` | REQUIRED | `HRESULT`: must work for swapchain RTs and Texture2D RTs. |
| `pfnDestroyRenderTargetView` | REQUIRED | `void`. |
| `pfnCalcPrivateDepthStencilViewSize` | REQUIRED | Return `sizeof(DSV)`. |
| `pfnCreateDepthStencilView` | REQUIRED | `HRESULT`: must succeed for the depth formats you report (used by `d3d11_depth_test_sanity`). |
| `pfnDestroyDepthStencilView` | REQUIRED | `void`. |
| `pfnCalcPrivateUnorderedAccessViewSize` | OPTIONAL | Return `sizeof(UAV)`; `Create*` may return `E_NOTIMPL` for FL10_0. |
| `pfnCreateUnorderedAccessView` | OPTIONAL | `HRESULT`: `E_NOTIMPL` for FL10_0. |
| `pfnDestroyUnorderedAccessView` | OPTIONAL | `void`. |

##### 4.2.4 Shaders

| Field | Status | Stub failure mode |
|---|---|---|
| `pfnCalcPrivateVertexShaderSize` | REQUIRED | Return `sizeof(VS)`. |
| `pfnCreateVertexShader` | REQUIRED | `HRESULT`: must accept SM4.x DXBC. |
| `pfnDestroyVertexShader` | REQUIRED | `void`. |
| `pfnCalcPrivatePixelShaderSize` | REQUIRED | Return `sizeof(PS)`. |
| `pfnCreatePixelShader` | REQUIRED | `HRESULT`: must accept SM4.x DXBC. |
| `pfnDestroyPixelShader` | REQUIRED | `void`. |
| `pfnCalcPrivateGeometryShaderSize` | REQUIRED-BUT-STUBBABLE | Return `sizeof(GS)`. Required to pass `d3d11_geometry_shader_smoke`. |
| `pfnCreateGeometryShader` | REQUIRED | `HRESULT`: must accept SM4.x DXBC if advertising FL10_0. Returning `E_NOTIMPL` breaks FL10_0 apps using GS and the `d3d11_geometry_shader_smoke` test (advertise only FL9_x if you are not ready). |
| `pfnDestroyGeometryShader` | REQUIRED-BUT-STUBBABLE | `void`. |
| `pfnCalcPrivateGeometryShaderWithStreamOutputSize` | OPTIONAL | Return `sizeof(GS+SO)`; `Create*` may return `E_NOTIMPL` until SO is implemented. |
| `pfnCreateGeometryShaderWithStreamOutput` | OPTIONAL | `HRESULT`: `E_NOTIMPL` is acceptable until SO is implemented. AeroGPU currently accepts this entrypoint but ignores the stream-output declaration (behaves like `pfnCreateGeometryShader`); binding real SO targets via `SoSetTargets` still reports `E_NOTIMPL`. |

SM5/tessellation/compute (not required for FL10_0 bring-up):

| Field | Status | Stub failure mode |
|---|---|---|
| `pfnCalcPrivateHullShaderSize` / `pfnCreateHullShader` / `pfnDestroyHullShader` | OPTIONAL | `Create*` returns `E_NOTIMPL`. |
| `pfnCalcPrivateDomainShaderSize` / `pfnCreateDomainShader` / `pfnDestroyDomainShader` | OPTIONAL | `Create*` returns `E_NOTIMPL`. |
| `pfnCalcPrivateComputeShaderSize` / `pfnCreateComputeShader` / `pfnDestroyComputeShader` | OPTIONAL | `Create*` returns `E_NOTIMPL` unless you also report CS support in caps. |

##### 4.2.5 Fixed-function / pipeline state objects

| Field | Status | Stub failure mode |
|---|---|---|
| `pfnCalcPrivateElementLayoutSize` | REQUIRED | Return `sizeof(InputLayout)`; must support layouts used by tests. |
| `pfnCreateElementLayout` | REQUIRED | `HRESULT`: must work (D3D11 input layouts are required for most apps). |
| `pfnDestroyElementLayout` | REQUIRED | `void`. |
| `pfnCalcPrivateSamplerSize` / `pfnCreateSampler` / `pfnDestroySampler` | REQUIRED | Must succeed for point/clamp samplers (`d3d11_texture_sampling_sanity`). |
| `pfnCalcPrivateRasterizerStateSize` / `pfnCreateRasterizerState` / `pfnDestroyRasterizerState` | REQUIRED-BUT-STUBBABLE | Accept + store. To pass `d3d11_rs_om_state_sanity`, cull mode + front-face winding + scissor enable must work. |
| `pfnCalcPrivateBlendStateSize` / `pfnCreateBlendState` / `pfnDestroyBlendState` | REQUIRED-BUT-STUBBABLE | Accept + store. To pass `d3d11_rs_om_state_sanity`, basic alpha blending must work. |
| `pfnCalcPrivateDepthStencilStateSize` / `pfnCreateDepthStencilState` / `pfnDestroyDepthStencilState` | REQUIRED | Must succeed for depth test state (`d3d11_depth_test_sanity`). |

##### 4.2.6 Queries / predication / counters

| Field | Status | Stub failure mode |
|---|---|---|
| `pfnCalcPrivateQuerySize` / `pfnCreateQuery` / `pfnDestroyQuery` | OPTIONAL | `Create*` returns `E_NOTIMPL`. |
| `pfnCalcPrivatePredicateSize` / `pfnCreatePredicate` / `pfnDestroyPredicate` | OPTIONAL | `Create*` returns `E_NOTIMPL`. |
| `pfnCalcPrivateCounterSize` / `pfnCreateCounter` / `pfnDestroyCounter` | OPTIONAL | `Create*` returns `E_NOTIMPL`. |

##### 4.2.7 Deferred contexts / command lists / class linkage (advanced)

If you don’t implement deferred contexts yet, **still provide stubs** so an app calling `CreateDeferredContext` fails cleanly.

| Field | Status | Stub failure mode |
|---|---|---|
| `pfnCalcPrivateDeferredContextSize` / `pfnCreateDeferredContext` / `pfnDestroyDeferredContext` | OPTIONAL | `Create*` returns `E_NOTIMPL`. |
| `pfnCalcPrivateCommandListSize` / `pfnCreateCommandList` / `pfnDestroyCommandList` | OPTIONAL | `Create*` returns `E_NOTIMPL`. |
| `pfnCalcPrivateClassLinkageSize` / `pfnCreateClassLinkage` / `pfnDestroyClassLinkage` | OPTIONAL | `Create*` returns `E_NOTIMPL`. |
| `pfnCalcPrivateClassInstanceSize` / `pfnCreateClassInstance` / `pfnDestroyClassInstance` | OPTIONAL | `Create*` returns `E_NOTIMPL`. |

##### 4.2.8 DXGI present integration (Win7 specific)

On Win7, DXGI uses `D3D10DDIARG_PRESENT` (DXGI 1.1) even for D3D11 devices.

Depending on `D3D11DDI_INTERFACE_VERSION`, the present/rotation entrypoints may be surfaced on either:

* `D3D11DDI_DEVICEFUNCS` (device table), or
* `D3D11DDI_DEVICECONTEXTFUNCS` (immediate context table; common for Win7 D3D11).

Treat `pfnPresent` and `pfnRotateResourceIdentities` as REQUIRED wherever they appear.
See §5.8 for the checklist entries and stub guidance.

---

### Immediate context table: `D3D11DDI_DEVICECONTEXTFUNCS` checklist

This is the “immediate context” function table filled in `pfnCreateDevice`. It implements most of what `ID3D11DeviceContext` does.

#### 5.1 Minimum rule for crash-free bring-up

Just like device funcs: populate **every field** with a non-null pointer and fail cleanly where unsupported. The runtime often calls many state setters during initialization (binding `NULL` to reset state); missing entrypoints here commonly crash on first device creation.

If a field exists in your `d3d11umddi.h` but is not explicitly mentioned in this doc, treat it as:

* **OPTIONAL** for FL10_0 bring-up, and
* still **non-null** (stubbed, usually via `SetErrorCb(E_NOTIMPL)` for `void` context DDIs).

#### 5.2 Core pipeline binding (IA / VS / PS / GS)

| Field | Status | Notes / stub guidance |
|---|---|---|
| `pfnIaSetInputLayout` | REQUIRED | Must accept valid layouts; accept NULL to unbind. |
| `pfnIaSetVertexBuffers` | REQUIRED | Must accept NULL buffers to unbind. |
| `pfnIaSetIndexBuffer` | REQUIRED | Used by `d3d11_texture_sampling_sanity`. |
| `pfnIaSetTopology` | REQUIRED | Required for `IASetPrimitiveTopology`. |
| `pfnVsSetShader` | REQUIRED | Must accept NULL to unbind. |
| `pfnPsSetShader` | REQUIRED | Must accept NULL to unbind. |
| `pfnGsSetShader` | REQUIRED | Required to pass `d3d11_geometry_shader_smoke`. Must accept non-NULL GS bindings if advertising FL10_0 (forward the GS handle/state into your command stream even if host-side GS execution is still bring-up work). |

Resource/CB/sampler binding for FL10_0 pipeline:

| Field | Status | Notes / stub guidance |
|---|---|---|
| `pfnPsSetConstantBuffers` | REQUIRED | Used by `d3d11_dynamic_constant_buffer_sanity`. Runtime may call with NULL to clear; handle that without error. |
| `pfnVsSetConstantBuffers` | REQUIRED | Used by `d3d11_dynamic_constant_buffer_sanity` (binds the CB to both VS and PS). Runtime may call with NULL to clear; handle that without error. |
| `pfnGsSetConstantBuffers` | REQUIRED-BUT-STUBBABLE | Keep non-null; implement as stages gain coverage. |
| `pfnPsSetShaderResources` | REQUIRED | Used by `d3d11_texture_sampling_sanity`; handle NULL to unbind. |
| `pfnVsSetShaderResources` | REQUIRED | Used by `d3d11_texture_sampling_sanity` (binds SRV to both VS and PS); handle NULL to unbind. |
| `pfnGsSetShaderResources` | REQUIRED-BUT-STUBBABLE | Keep non-null; implement as stages gain coverage. |
| `pfnPsSetSamplers` | REQUIRED | Used by `d3d11_texture_sampling_sanity`; handle NULL to unbind. |
| `pfnVsSetSamplers` | REQUIRED | Used by `d3d11_texture_sampling_sanity` (binds samplers to both VS and PS); handle NULL to unbind. |
| `pfnGsSetSamplers` | REQUIRED-BUT-STUBBABLE | Keep non-null; implement as stages gain coverage. |

##### 5.2.1 HS/DS/CS stages (optional for FL10_0 bring-up)

Even if you don’t support tessellation/compute yet, the context table will usually still contain entrypoints for:

* `pfnHsSet*` (hull shader stage)
* `pfnDsSet*` (domain shader stage)
* `pfnCsSet*` (compute shader stage)

Recommended stub behavior:

* If called with “unbind” semantics (NULL shader, NULL resources), treat it as a no-op success (don’t spam `SetErrorCb` during `ClearState`).
* If called with a non-NULL shader/resource that implies real execution, report `E_NOTIMPL` via `SetErrorCb`.

#### 5.3 Rasterizer / output merger binding

| Field | Status | Notes / stub guidance |
|---|---|---|
| `pfnSetViewports` | REQUIRED | `RSSetViewports`. |
| `pfnSetScissorRects` | REQUIRED-BUT-STUBBABLE | `d3d11_rs_om_state_sanity` requires scissor to clip rendering. If unimplemented, at minimum treat NULL/empty as disable (no error), but expect scissor-dependent tests to fail. |
| `pfnSetRasterizerState` | REQUIRED-BUT-STUBBABLE | `d3d11_rs_om_state_sanity` requires cull mode + front-face winding + scissor enable. For bring-up you can accept+store only, but expect state-dependent tests to fail. |
| `pfnSetBlendState` | REQUIRED-BUT-STUBBABLE | `d3d11_rs_om_state_sanity` requires basic alpha blending. For bring-up you can accept+store only, but expect blend-dependent tests to fail. |
| `pfnSetDepthStencilState` | REQUIRED | Must accept/store state for `d3d11_depth_test_sanity`. |
| `pfnSetRenderTargets` | REQUIRED | Must bind RTVs/DSV for draws and clears. |

#### 5.3.1 State reset / convenience

| Field | Status | Notes / stub guidance |
|---|---|---|
| `pfnClearState` | REQUIRED-BUT-STUBBABLE | Many apps (and sometimes runtimes) call `ID3D11DeviceContext::ClearState`. A safe bring-up implementation is “reset tracked bindings to defaults” (or even a no-op). Avoid calling `SetErrorCb` here: `ClearState` is commonly used as a non-failing reset path. |

#### 5.4 Clears and draws

| Field | Status | Notes / stub guidance |
|---|---|---|
| `pfnClearRenderTargetView` | REQUIRED | Must work for swapchain backbuffer RTV and texture RTVs. |
| `pfnClearDepthStencilView` | REQUIRED | Used by `d3d11_depth_test_sanity`. |
| `pfnDraw` | REQUIRED | Used by multiple Win7 tests. |
| `pfnDrawIndexed` | REQUIRED | Used by `d3d11_texture_sampling_sanity`; many samples use it. |
| `pfnDrawInstanced` / `pfnDrawIndexedInstanced` / `pfnDrawAuto` | OPTIONAL | Stub with `SetErrorCb(E_NOTIMPL)` if called with non-zero counts. |
| `pfnDrawInstancedIndirect` / `pfnDrawIndexedInstancedIndirect` | OPTIONAL | Stub. |

#### 5.5 Resource update/copy/resolve

| Field | Status | Notes / stub guidance |
|---|---|---|
| `pfnUpdateSubresourceUP` | REQUIRED | Must accept user-memory uploads. `d3d11_update_subresource_texture_sanity` specifically checks: padded `RowPitch` (not tightly packed) and non-NULL `D3D11_BOX` partial updates for both textures and buffers. |
| `pfnCopyResource` | REQUIRED | Used by multiple Win7 tests for staging readback. |
| `pfnCopySubresourceRegion` | REQUIRED-BUT-STUBBABLE | Many real apps use subresource copies; implement soon. |
| `pfnResolveSubresource` | OPTIONAL | Stub until MSAA is implemented. |
| `pfnGenerateMips` | OPTIONAL | Stub until autogen mips are implemented. |

#### 5.5.1 Queries / predication (often unused for bring-up)

| Field | Status | Notes / stub guidance |
|---|---|---|
| `pfnBegin` / `pfnEnd` | OPTIONAL | If unimplemented, use `SetErrorCb(E_NOTIMPL)` on non-null queries. |
| `pfnSetPredication` | OPTIONAL | Stub with `SetErrorCb(E_NOTIMPL)` until queries/predication are implemented. |

#### 5.6 Map/Unmap (dynamic updates + staging readback)

| Field | Status | Notes / stub guidance |
|---|---|---|
| `pfnMap` | REQUIRED | Must support at least: `D3D11_MAP_READ` on STAGING textures and `D3D11_MAP_WRITE_DISCARD` for dynamic buffer uploads. `d3d11_map_dynamic_buffer_sanity` also exercises `D3D11_MAP_WRITE_NO_OVERWRITE` and DISCARD renaming hazards (in-flight copies must see the pre-discard contents). |
| `pfnUnmap` | REQUIRED | Must commit writes / release mappings. |

Special note for Win7 bring-up:

* The tests call `Map(..., D3D11_MAP_READ, 0, ...)` (no `DO_NOT_WAIT`). It is acceptable for Map to block waiting for the copy to complete, but it must be bounded and backed by a real fence (avoid TDRs).
* If you implement a “submit-on-Flush” backend, make sure `CopyResource + Flush + Map(READ)` results in completed readback data.
* Some D3D11 DDI interface versions expose additional map-style entrypoints (for example, staging-specific map helpers). If your chosen `D3D11DDI_DEVICECONTEXTFUNCS` struct has them, wire them to the same underlying map/unmap implementation and keep them non-null.
* For the definitive Win7 Map/Unmap + `LockCb`/`UnlockCb` contract, see:
  * `windows7-d3d-umd-ddi.md`
* For the Win7/WDDM 1.1 callback-level contract (how a UMD blocks/polls on fences for `Map(READ)`), see:
  * `windows7-d3d-umd-ddi.md`

#### 5.7 Flush / submission

| Field | Status | Notes / stub guidance |
|---|---|---|
| `pfnFlush` | REQUIRED | Must actually submit queued work so fences advance and readbacks complete. `ID3D11DeviceContext::Flush` is `void` → failures must use `SetErrorCb`. |

#### 5.8 Present callouts (Win7 DXGI)

Present/RotateResourceIdentities may be surfaced on either the device or immediate context table (interface-version dependent).
On Win7, DXGI uses `D3D10DDIARG_PRESENT` (DXGI 1.1) even for D3D11 devices:

| Field | Status | Notes / stub guidance |
|---|---|---|
| `pfnPresent` | REQUIRED | `HRESULT`: must succeed for windowed swapchains. Signature is table-dependent, but DXGI always passes a `D3D10DDIARG_PRESENT` (DXGI 1.1). Consider implicit flush/submit so rendering to the backbuffer is visible by the time present returns. |
| `pfnRotateResourceIdentities` | REQUIRED | `void`: must rotate the “identity” of backbuffer resources after present. `d3d11_swapchain_rotate_sanity` validates `BufferCount=2` rotation. |

Present also interacts with context submission:

* DXGI typically expects rendering to the backbuffer to be **submitted** before present returns.
* A common minimal policy is: `pfnPresent` performs an implicit `Flush` / submit of outstanding work.
* DXGI swapchains also use `pfnRotateResourceIdentities` to rotate backbuffer identities after present.

---

### Mapping: Win7 guest tests → DDI entrypoints exercised

The repo’s Win7 D3D11 tests are good coverage targets because they exercise device creation + caps queries, basic rendering, staging readback, dynamic updates, and swapchain present/rotation.

Tests:

* `drivers/aerogpu/tests/win7/d3d11_caps_smoke`
* `drivers/aerogpu/tests/win7/d3d11_triangle`
* `drivers/aerogpu/tests/win7/readback_sanity`
* `drivers/aerogpu/tests/win7/d3d11_texture_sampling_sanity`
* `drivers/aerogpu/tests/win7/d3d11_dynamic_constant_buffer_sanity`
* `drivers/aerogpu/tests/win7/d3d11_depth_test_sanity`
* `drivers/aerogpu/tests/win7/d3d11_update_subresource_texture_sanity`
* `drivers/aerogpu/tests/win7/d3d11_map_dynamic_buffer_sanity`
* `drivers/aerogpu/tests/win7/d3d11_rs_om_state_sanity`
* `drivers/aerogpu/tests/win7/d3d11_swapchain_rotate_sanity`
* `drivers/aerogpu/tests/win7/d3d11_geometry_shader_smoke`

#### 6.1 `d3d11_triangle` (D3D11CreateDeviceAndSwapChain + Present)

| API call in test | DDI entrypoints you should expect |
|---|---|
| `D3D11CreateDeviceAndSwapChain` | `OpenAdapter11` → adapter `pfnGetCaps` (several types) → `pfnCalcPrivateDeviceSize` → `pfnCreateDevice` (fills `D3D11DDI_DEVICEFUNCS` + `D3D11DDI_DEVICECONTEXTFUNCS`). |
| `IDXGISwapChain::GetBuffer` | runtime bookkeeping; usually no direct DDI call, but the backbuffer is a DDI resource created during swapchain creation. |
| `ID3D11Device::CreateRenderTargetView` | `pfnCalcPrivateRenderTargetViewSize` → `pfnCreateRenderTargetView`. |
| `ID3D11DeviceContext::OMSetRenderTargets` | context `pfnSetRenderTargets`. |
| `ID3D11DeviceContext::RSSetViewports` | context `pfnSetViewports`. |
| `CreateVertexShader` / `CreatePixelShader` | `pfnCalcPrivate*ShaderSize` → `pfnCreate*Shader`. |
| `CreateInputLayout` | `pfnCalcPrivateElementLayoutSize` → `pfnCreateElementLayout`. |
| `CreateBuffer` (VB) | `pfnCalcPrivateResourceSize` → `pfnCreateResource` (must support initial data upload via `D3D11_SUBRESOURCE_DATA`). |
| `IASetInputLayout` / `IASetPrimitiveTopology` / `IASetVertexBuffers` | context `pfnIaSetInputLayout` / `pfnIaSetTopology` / `pfnIaSetVertexBuffers`. |
| `VSSetShader` / `PSSetShader` | context `pfnVsSetShader` / `pfnPsSetShader`. |
| `ClearRenderTargetView` | context `pfnClearRenderTargetView`. |
| `Draw` | context `pfnDraw`. |
| `CreateTexture2D` (staging) | `pfnCalcPrivateResourceSize` → `pfnCreateResource` (must support `D3D11_USAGE_STAGING` + `CPU_ACCESS_READ`). |
| `CopyResource` | context `pfnCopyResource`. |
| `Flush` | context `pfnFlush`. |
| `Map` / `Unmap` | context `pfnMap` / `pfnUnmap`. |
| `IDXGISwapChain::Present` | `pfnPresent` + `pfnRotateResourceIdentities` (table depends on interface version; DXGI uses `D3D10DDIARG_PRESENT`). |

#### 6.2 `readback_sanity` (render-to-texture + staging readback; no Present)

Same as above except:

* No swapchain creation / `pfnPresent` / `pfnRotateResourceIdentities` required.
* Render target is a regular `Texture2D` created via `pfnCreateResource` + `pfnCreateRenderTargetView` (BGRA, `D3D11_BIND_RENDER_TARGET`).

#### 6.3 `d3d11_texture_sampling_sanity` (SRV + sampler + indexed draw + readback)

| API call in test | DDI entrypoints you should expect |
|---|---|
| `CreateTexture2D` (src texture) | device `pfnCalcPrivateResourceSize` → `pfnCreateResource`. |
| `UpdateSubresource` | context `pfnUpdateSubresourceUP`. |
| `CreateShaderResourceView` | device `pfnCalcPrivateShaderResourceViewSize` → `pfnCreateShaderResourceView`. |
| `CreateSamplerState` | device `pfnCalcPrivateSamplerSize` → `pfnCreateSampler`. |
| `IASetIndexBuffer` | context `pfnIaSetIndexBuffer`. |
| `VSSetShaderResources` / `VSSetSamplers` | context `pfnVsSetShaderResources` / `pfnVsSetSamplers`. |
| `PSSetShaderResources` / `PSSetSamplers` | context `pfnPsSetShaderResources` / `pfnPsSetSamplers`. |
| `DrawIndexed` | context `pfnDrawIndexed`. |
| `CopyResource` + staging `Map(READ)` | context `pfnCopyResource` + `pfnMap` / `pfnUnmap`. |

#### 6.4 `d3d11_dynamic_constant_buffer_sanity` (dynamic CB bind + draw + readback)

| API call in test | DDI entrypoints you should expect |
|---|---|
| `CreateBuffer` (dynamic constant buffer) | device `pfnCalcPrivateResourceSize` → `pfnCreateResource`. |
| `Map(WRITE_DISCARD)` | context `pfnMap` / `pfnUnmap` (the runtime may route dynamic CB discard through specialized DDIs depending on interface version). |
| `PSSetConstantBuffers` | context `pfnPsSetConstantBuffers`. |
| `Draw` | context `pfnDraw`. |

#### 6.5 `d3d11_depth_test_sanity` (DSV + depth state + clear + draw + readback)

| API call in test | DDI entrypoints you should expect |
|---|---|
| `CreateTexture2D` (depth) | device `pfnCalcPrivateResourceSize` → `pfnCreateResource`. |
| `CreateDepthStencilView` | device `pfnCalcPrivateDepthStencilViewSize` → `pfnCreateDepthStencilView`. |
| `CreateDepthStencilState` | device `pfnCalcPrivateDepthStencilStateSize` → `pfnCreateDepthStencilState`. |
| `OMSetDepthStencilState` | context `pfnSetDepthStencilState`. |
| `ClearDepthStencilView` | context `pfnClearDepthStencilView`. |
| `Draw` | context `pfnDraw`. |

#### 6.6 `d3d11_update_subresource_texture_sanity` (UpdateSubresource on textures + boxed updates)

| API call in test | DDI entrypoints you should expect |
|---|---|
| `UpdateSubresource(tex, 0, NULL, pData, RowPitch, ...)` | context `pfnUpdateSubresourceUP` (must respect the caller-provided `RowPitch`, which may be larger than `Width*BytesPerPixel`). |
| `UpdateSubresource(tex, 0, &D3D11_BOX, ...)` | context `pfnUpdateSubresourceUP` (partial/boxed update). |
| `UpdateSubresource(buffer, 0, &D3D11_BOX, ...)` | context `pfnUpdateSubresourceUP` (for buffers, the box encodes byte offsets in `left/right`). |
| Readback verification | context `pfnCopyResource` + `pfnFlush` + `pfnMap`/`pfnUnmap`. |

#### 6.7 `d3d11_map_dynamic_buffer_sanity` (Map DISCARD/NO_OVERWRITE correctness)

This test is intentionally “mean”: it checks that `Map(WRITE_DISCARD)` renames/allocates fresh storage so that
GPU work queued before the discard still sees the **old** contents.

| API call in test | DDI entrypoints you should expect |
|---|---|
| `CreateBuffer(DYNAMIC, CPU_WRITE)` (VB/IB/CB) | device `pfnCalcPrivateResourceSize` → `pfnCreateResource`. |
| Bind dynamic buffers (`IASetVertexBuffers`, `IASetIndexBuffer`, `VSSetConstantBuffers`) | context `pfnIaSetVertexBuffers` / `pfnIaSetIndexBuffer` / `pfnVsSetConstantBuffers`. |
| `Map(WRITE_DISCARD)` / `Map(WRITE_NO_OVERWRITE)` | context `pfnMap` / `pfnUnmap` (some interface versions route these through specialized dynamic-map DDIs; if your struct has them, keep them non-null and implement equivalent semantics). |
| `CopyResource(staging, dynamic_buf)` | context `pfnCopyResource` (copies must read the correct “version” of a renamed/discarded buffer). |
| Readback verification | context `pfnFlush` + `pfnMap(READ)` on staging buffers. |

#### 6.8 `d3d11_rs_om_state_sanity` (scissor + cull mode + blending)

| API call in test | DDI entrypoints you should expect |
|---|---|
| `CreateRasterizerState` | device `pfnCalcPrivateRasterizerStateSize` → `pfnCreateRasterizerState`. |
| `RSSetState` / `RSSetScissorRects` | context `pfnSetRasterizerState` / `pfnSetScissorRects`. |
| `CreateBlendState` | device `pfnCalcPrivateBlendStateSize` → `pfnCreateBlendState`. |
| `OMSetBlendState` | context `pfnSetBlendState`. |
| Draw + readback | context `pfnDraw` + `pfnCopyResource` + `pfnFlush` + `pfnMap`/`pfnUnmap`. |

#### 6.9 `d3d11_swapchain_rotate_sanity` (RotateResourceIdentities, BufferCount=2)

This test validates that after `Present`, swapchain buffer identities rotate (buffer0 becomes buffer1, etc).

| API call in test | DDI entrypoints you should expect |
|---|---|
| `IDXGISwapChain::Present` | context `pfnPresent` + `pfnRotateResourceIdentities` (rotation is what the test is checking). |
| Readback of both swapchain buffers | context `pfnCopyResource` + `pfnFlush` + `pfnMap`/`pfnUnmap`. |

#### 6.10 `d3d11_geometry_shader_smoke` (GS creation + bind + draw)

| API call in test | DDI entrypoints you should expect |
|---|---|
| `CreateGeometryShader` | device `pfnCalcPrivateGeometryShaderSize` → `pfnCreateGeometryShader`. |
| `GSSetShader` | context `pfnGsSetShader`. |
| Draw + readback | context `pfnDraw` + `pfnCopyResource` + `pfnFlush` + `pfnMap`/`pfnUnmap`. |

#### 6.11 `d3d11_caps_smoke` (caps queries: feature support, formats, MSAA)

This test is primarily about making `pfnGetCaps` return conservative-but-consistent answers:

| API call in test | DDI entrypoints you should expect |
|---|---|
| `CheckFeatureSupport(D3D11_FEATURE_THREADING/DOUBLES/D3D10_X_HARDWARE_OPTIONS/D3D11_OPTIONS)` | adapter `pfnGetCaps` with `D3D11DDICAPS_TYPE_*` matching the queried feature. |
| `CheckFormatSupport(...)` | adapter `pfnGetCaps(D3D11DDICAPS_TYPE_FORMAT)` for the probed `DXGI_FORMAT`s (BGRA/RGBA/depth/index). |
| `CheckMultisampleQualityLevels(format, 1, ...)` | adapter `pfnGetCaps(D3D11DDICAPS_TYPE_MULTISAMPLE_QUALITY_LEVELS)` should report at least 1 quality level for `SampleCount==1` on supported formats. |

---

### Practical bring-up ordering (what to implement first)

If your goal is “device creates and the basic Win7 guest tests pass”:

1. **Adapter bring-up**
   * `OpenAdapter11` export
   * `D3D11DDI_ADAPTERFUNCS::{pfnGetCaps,pfnCalcPrivateDeviceSize,pfnCreateDevice,pfnCloseAdapter}`
2. **Device funcs (creation)**
   * Resource + RTV/SRV/DSV + sampler + depth-stencil state + shader + input layout creation (triads)
3. **Immediate context**
   * `pfnSetRenderTargets`, `pfnSetViewports`, IA bindings (incl. index buffer), VS/PS binds
   * PS resource/sampler/CB binding, depth-stencil state, clears, `pfnDraw`/`pfnDrawIndexed`
4. **Upload + readback path**
   * `pfnUpdateSubresourceUP`, `pfnCopyResource`, `pfnFlush`, `pfnMap`/`pfnUnmap` for STAGING read
5. **DXGI present** (swapchain tests)
   * context `pfnPresent` + `pfnRotateResourceIdentities`

Everything else can initially be “present-but-stubbed”, as long as it fails cleanly and never dereferences invalid handles.

---

### Appendix: common early crash sources (and what the checklist prevents)

If you’re bringing up a new UMD and seeing immediate access violations in `d3d11.dll` / `dxgi.dll`, the root cause is often one of:

* **NULL DDI function pointer** in `D3D11DDI_DEVICEFUNCS` / `D3D11DDI_DEVICECONTEXTFUNCS`.
  * Fix: stub-fill all fields (see §0 “Non-null discipline”).
* **Wrong calling convention / prototype mismatch** (stack imbalance).
  * Fix: make sure you compile with the exact `PFND3D11DDI_*` typedefs from `d3d11umddi.h` and use `__stdcall`/`APIENTRY`.
* **`pfnGetCaps` writing past `DataSize`**, or assuming `pData` is always non-null.
  * Fix: treat `pData == NULL` as a size query when applicable, validate `DataSize` before writing, and be conservative on unknown types.
* **Returning “supported” in caps but failing creation later** (leads to confusing app behavior and sometimes runtime asserts).
  * Fix: keep caps truthful; only advertise what you implement end-to-end.

This doc’s core recommendation (“fill everything with safe stubs first; then incrementally implement”) is specifically to avoid the first class of bring-up crashes.

## Tracing capability queries and entrypoints

This note describes how to enable a **lightweight tracing facility** in the AeroGPU D3D10/11 UMD so you can quickly discover:

* Which `D3D10DDIARG_GETCAPS::Type` values the D3D10 runtime/DXGI request during device + swapchain creation.
* Which D3D10DDI entrypoints the runtime calls beyond the minimal “triangle” set (helps avoid NULL DDI function pointer crashes).

Tracing is implemented with **`OutputDebugStringA`** and can be captured in a Windows 7 VM using **Sysinternals DebugView**.

Related checklist (what the runtime might call once you switch to WDK headers):

* `windows7-d3d-umd-ddi.md` — D3D11 `d3d11umddi.h` function-table checklist (which entries must be non-null vs safely stubbed for FL10_0 bring-up).

---

### Enable tracing

#### 1.1 Compile-time gate

Build the UMD with:

* `AEROGPU_D3D10_TRACE=1`

For **caps-only bring-up** (discovering which `GetCaps` query IDs Win7 requests),
you can also build with:

* `AEROGPU_D3D10_11_CAPS_LOG=1`

`AEROGPU_D3D10_11_CAPS_LOG` is a low-noise trace that prints the raw caps query
`Type` values (and `DataSize`) via `OutputDebugStringA` across:

* D3D10 (`d3d10umddi.h`)
* D3D10.1 (`d3d10_1umddi.h`)
* D3D11 (`d3d11umddi.h`)

It is **compile-time only** and intentionally bypasses the runtime logging gate
used by `AEROGPU_D3D10_11_LOG`, since bring-up often happens on retail builds
where the environment-controlled log may be disabled.

If you are building the real Win7 driver against WDK headers, also build with:

* `/p:AeroGpuUseWdkHeaders=1` (defines `AEROGPU_UMD_USE_WDK_HEADERS=1` in the D3D10/11 UMD project)

#### 1.2 Runtime gate (environment variable)

At runtime, set:

* `AEROGPU_D3D10_TRACE=1` – log high-level calls (adapter open, `GetCaps`, device creation, resource/view creation, `Present`, etc.)
* `AEROGPU_D3D10_TRACE=2` – verbose (includes per-draw/per-state calls like `SetRenderTargets`, clears, draws, etc.)

Example (per-process):

```cmd
set AEROGPU_D3D10_TRACE=2
your_test_app.exe
```

To enable globally for system processes (e.g. if tracing `dwm.exe`), set it via System Properties or:

```cmd
setx AEROGPU_D3D10_TRACE 2
```

---

### Capture output on Win7

1. Copy **DebugView** (`DbgView.exe`) into the VM (Sysinternals suite).
2. Run it as Administrator (recommended).
3. Enable:
   * `Capture Win32`
   * (optional) `Capture Global Win32`
4. Run the D3D10 app/test (e.g. a simple `D3D10CreateDeviceAndSwapChain` sample).

---

### Interpreting the trace

Typical lines look like:

```text
[AeroGPU:D3D10 t=123456 tid=1337 #0] OpenAdapter10
[AeroGPU:D3D10 t=123456 tid=1337 #1] OpenAdapterCommon iface=... ver=...
[AeroGPU:D3D10 t=123457 tid=1337 #2] GetCaps10 Type=12 DataSize=64 pData=0x...
[AeroGPU:D3D10 t=123457 tid=1337 #3] GetCaps10 -> hr=0x00000000
[AeroGPU:D3D10 t=123458 tid=1337 #4] CreateDevice hAdapter=0x... hDevice=0x...
[AeroGPU:D3D10 t=123458 tid=1337 #5] CreateDevice -> hr=0x00000000
[AeroGPU:D3D10 t=123500 tid=1337 #6] CreateRTV hDevice=0x... hResource=0x...
[AeroGPU:D3D10 t=123500 tid=1337 #7] Present hDevice=0x... syncInterval=1 backbuffer=0x...
[AeroGPU:D3D10 t=123501 tid=1337 #8] SetErrorCb hr=0x80004001
```

#### 3.1 `GetCaps` sequencing

* The `Type=<n>` field is the raw `D3D10DDIARG_GETCAPS::Type` value requested by the runtime.
  * Depending on which runtime path is active you may see `GetCaps10` (D3D10.0) and/or `GetCaps` (D3D10.1).
* Cross-reference `<n>` against the Win7-era WDK header (`d3d10umddi.h`) to find the corresponding `D3D10DDI_GETCAPS_TYPE` / `D3D10DDICAPS_TYPE_*` enum entry.

Bring-up workflow:

1. Enable tracing.
2. Run the app until it fails.
3. Find the last `GetCaps Type=...` query; implement that caps type (or return a conservative “not supported” result).
4. Repeat until you reach RTV creation + `Present`.

#### 3.2 Unexpected entrypoints / NULL-vtable avoidance

When you start wiring up the real WDK DDI tables, make sure **every function pointer** in the returned DDI tables is non-NULL (even if it’s just a stub that logs and returns `E_NOTIMPL` / calls `pfnSetErrorCb`).

The intent of this trace facility is to make those unexpected calls visible quickly so you can either:

* implement the entrypoint, or
* stop advertising the capability that triggers it.

## Swapchain backbuffer creation

This note documents how to **empirically capture** the `CreateResource` parameters the Windows 7 **DXGI 1.1 + D3D10/11 runtime** passes to the AeroGPU **D3D10/11 UMD** when creating **swapchain backbuffers**, and how to translate those parameters into allocation flags that keep `Present` stable.

The main goal is to avoid “guessing” the backbuffer recipe: on Win7/WDDM 1.1, swapchain buffers are created by DXGI/runtime on the app’s behalf, and the *UMD must match what the runtime expects*.

### Capturing the runtime’s `CreateResource` calls

#### Build an instrumented UMD

`drivers/aerogpu/umd/d3d10_11/src/aerogpu_d3d10_11_umd.cpp` contains trace logging guarded by:

* `AEROGPU_UMD_TRACE_RESOURCES`

The Visual Studio project `drivers/aerogpu/umd/d3d10_11/aerogpu_d3d10_11.vcxproj` defines this macro for **Debug** builds only.

The trace is emitted via the standard D3D10/11 UMD logging helper (`AEROGPU_D3D10_11_LOG`), which writes to `OutputDebugStringA`.
Lines are prefixed by the logging helper (currently `AEROGPU_D3D11DDI:`) and then tagged with:

* `trace_resources:`

> Note: the trace hooks are compiled into both the repo “portable ABI subset” UMD path and the WDK-backed Win7 UMD DDIs
> (`aerogpu_d3d10_umd_wdk.cpp`, `aerogpu_d3d10_1_umd_wdk.cpp`, `aerogpu_d3d11_umd_wdk.cpp`). This means the default
> WDK build (`/p:AeroGpuUseWdkHeaders=1`) will still emit the `trace_resources:` lines.

#### Run the DXGI probe app on Win7

The guest-side probe lives at:

* `drivers/aerogpu/tests/win7/dxgi_swapchain_probe/`

It creates:

* a D3D11 device by default (`--api=d3d11`)
  * or a D3D10 device (`--api=d3d10`)
  * or a D3D10.1 device (`--api=d3d10_1`)
* a **windowed** `DXGI_SWAP_CHAIN_DESC` swapchain (defaults to **2 buffers**; configurable via `--buffers` / `--swap-effect` / etc)
* RTVs for each buffer
* a few `Present(1,0)` frames (vsync)

Build on Win7 (VS2010 toolchain):

```cmd
cd \path\to\repo\drivers\aerogpu\tests\win7
build_all_vs2010.cmd
```

Run:

```cmd
bin\dxgi_swapchain_probe.exe --api=d3d11 --require-vid=0xA3A0 --require-did=0x0001
bin\dxgi_swapchain_probe.exe --api=d3d10 --require-vid=0xA3A0 --require-did=0x0001
bin\dxgi_swapchain_probe.exe --api=d3d10_1 --require-vid=0xA3A0 --require-did=0x0001
```

The probe defaults to a 256x256 swapchain with 2 buffers (`DXGI_SWAP_EFFECT_DISCARD`), but you can override:
* `--width=N`
* `--height=N`
* `--buffers=1|2`
* `--swap-effect=discard|sequential`
* `--format=b8g8r8a8_unorm|b8g8r8x8_unorm|r8g8b8a8_unorm|87`
* `--buffer-usage=0x########`
* `--swapchain-flags=0x########`

Using an “odd” size can make it easier to correlate `--dump-createalloc` entries by allocation size.

#### Capture the UMD output

Use Sysinternals **DebugView** (or any debugger) to capture `OutputDebugStringA` output while the probe runs.

Alternatively, the UMD logging helper can also append to a file (useful when DebugView/WinDbg is not convenient):

```cmd
set AEROGPU_D3D10_11_LOG=1
set AEROGPU_D3D10_11_LOG_FILE=C:\aerogpu_d3d10_11_umd.log
bin\dxgi_swapchain_probe.exe ...
```

Note: `AEROGPU_D3D10_11_LOG` defaults to enabled in `_DEBUG` builds; for Release builds you must set it explicitly.

### What to extract from the trace

The UMD prints three key call sites:

* `CreateResource` (resource descriptors)
* `RotateResourceIdentities` (the set of swapchain buffer identities, before/after rotation)
* `Present` (which backbuffer identity is presented and with what sync interval)

> Note: depending on which UMD build you are running, the `Present` trace line may
> print the presented resource as either `src_handle=<id>` (WDK-backed DDIs) or
> `backbuffer_handle=<id>` (portable ABI subset). They refer to the same protocol
> resource handle space.

To identify *which* `CreateResource` calls are swapchain backbuffers:

1. Find the handles printed by `RotateResourceIdentities`.
2. Match those handles to the immediately preceding `CreateResource => created tex2d handle=...` lines.

For WDK-backed UMD builds, the `=> created` lines also include `alloc_id=<u32>`, which should match the
`alloc_id` field in `aerogpu_dbgctl.exe --dump-createalloc` output (useful for direct UMD↔KMD correlation without
relying on size matching).

> Note: some swapchains (notably single-buffer `DXGI_SWAP_EFFECT_DISCARD`) may not call
> `RotateResourceIdentities`. In that case, use the handle printed in the `Present`
> trace line (`src_handle=` / `backbuffer_handle=`) and/or the `primary=1` marker.
>
> Tip: when using the WDK-backed DDI path, `CreateResource` descriptors may also include:
>
> * `primary_desc=<ptr>` (mirrors `D3D10DDIARG_CREATERESOURCE::pPrimaryDesc` / `D3D11DDIARG_CREATERESOURCE::pPrimaryDesc`)
> * `primary=0/1` (derived from `primary_desc != NULL`)
>
> `primary_desc != NULL` / `primary=1` is a strong signal that the resource is a **DXGI primary/backbuffer**
> allocation, which can make it easier to scan logs manually. The parser script will include `primary` when present
> (and can infer it from `primary_desc` for older logs).

#### Optional: automated extraction

For convenience, the repo includes a small host-side parser that scans a captured log and prints the
backbuffer handles observed via `RotateResourceIdentities` along with their matching `CreateResource`
descriptors:

```bash
python scripts/parse_win7_dxgi_swapchain_trace.py aerogpu_d3d10_11_umd.log
python scripts/parse_win7_dxgi_swapchain_trace.py --json=swapchain_trace.json aerogpu_d3d10_11_umd.log
```

If you also captured `aerogpu_dbgctl.exe --dump-createalloc` output, you can pass it to the parser to
correlate swapchain backbuffer handles to recent `DxgkDdiCreateAllocation` flag values (matched by
allocation size):

```cmd
aerogpu_dbgctl.exe --dump-createalloc > createalloc.txt
```

```bash
python scripts/parse_win7_dxgi_swapchain_trace.py --createalloc=createalloc.txt aerogpu_d3d10_11_umd.log
python scripts/parse_win7_dxgi_swapchain_trace.py --json=swapchain_trace.json --createalloc=createalloc.txt aerogpu_d3d10_11_umd.log
```

If the CreateAllocation ring buffer contains a lot of unrelated noise (e.g. DWM allocations), you can take a baseline dump before
running the probe and then filter to only the newly-written entries:

```cmd
:: (Run from the directory containing aerogpu_dbgctl.exe, or use a full path; see dbgctl note below.)
aerogpu_dbgctl.exe --dump-createalloc > createalloc_before.txt
bin\dxgi_swapchain_probe.exe ...
aerogpu_dbgctl.exe --dump-createalloc > createalloc_after.txt
```

```bash
python scripts/parse_win7_dxgi_swapchain_trace.py --createalloc-before=createalloc_before.txt --createalloc-after=createalloc_after.txt aerogpu_d3d10_11_umd.log
```

To decode `flags_in`/`flags_out` and `create_flags` into named bitfields, build and run the Win7 WDK ABI probe
(`drivers/aerogpu/kmd/tools/wdk_abi_probe`) and pass its output to the parser:

```bash
python scripts/parse_win7_dxgi_swapchain_trace.py --wdk-abi=wdk_abi_probe.txt --createalloc=createalloc.txt aerogpu_d3d10_11_umd.log
```

#### Capturing KMD-facing allocation flags (optional but recommended)

To understand which **WDDM allocation flags** are required for `Present` stability, capture what
dxgkrnl/runtime passes into the miniport via `DxgkDdiCreateAllocation`.

`drivers/aerogpu/kmd/src/aerogpu_kmd.c` supports two capture paths:

1. **Escape-based (recommended; no kernel debugger required)**  
   The KMD maintains a small ring buffer of recent `DxgkDdiCreateAllocation` events and exposes it via
   the dbgctl escape `AEROGPU_ESCAPE_OP_DUMP_CREATEALLOCATION`.

   On a Win7 guest, `aerogpu_dbgctl.exe` is shipped on the Guest Tools ISO/zip under:
   - `<GuestToolsDrive>:\drivers\amd64\aerogpu\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe`
   - `<GuestToolsDrive>:\drivers\x86\aerogpu\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe`
   - Optional top-level tools payload (when present): `<GuestToolsDrive>:\tools\aerogpu_dbgctl.exe` (or under `<GuestToolsDrive>:\tools\<arch>\aerogpu_dbgctl.exe`)

   CI-staged driver packages also include dbgctl at:
   - `out\packages\aerogpu\x64\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe`
   - `out\packages\aerogpu\x86\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe`

   Example (Win7 x64; replace `<GuestToolsDrive>` with your mounted ISO/zip drive letter, e.g. `D`):

   ```cmd
   cd /d <GuestToolsDrive>:\drivers\amd64\aerogpu\tools\win7_dbgctl\bin
   aerogpu_dbgctl.exe --dump-createalloc
   ```

   If your Guest Tools ISO is mounted as `X:` (common), these are copy/pastable:

   ```cmd
   :: Win7 x64:
   X:\drivers\amd64\aerogpu\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe --dump-createalloc
   :: Win7 x86:
   X:\drivers\x86\aerogpu\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe --dump-createalloc
   ```

   The dump includes:
   * the **incoming** `DXGK_ALLOCATIONINFO::Flags.Value` from dxgkrnl/runtime (`flags_in`)
   * the **final** flags after AeroGPU applies its required bits (`flags_out`, currently adds `CpuVisible` + `Aperture`)

2. **DbgPrint-based (DBG builds; optional extra verbosity)**  
   Build the KMD with:

   * `AEROGPU_KMD_TRACE_CREATEALLOCATION=1`

   This logs the first few `CreateAllocation` calls via `DbgPrintEx` and includes `flags=0xIN->0xOUT` style lines:

   ```
   aerogpu-kmd: CreateAllocation: alloc_id=... flags=0x12345678->0x1234D678
   ```

   These are easiest to capture under WinDbg (kernel debug) or any setup that collects `DbgPrintEx`.

### Backbuffer allocation recipe (Win7 / WDDM 1.1)

The backbuffer “recipe” should be derived directly from the `CreateResource` trace lines, but the stable *invariants* that the allocation logic should enforce are:

#### Resource descriptor invariants

For a standard Win7 windowed swapchain (`DXGI_SWAP_EFFECT_DISCARD`, `SampleDesc.Count = 1`):

* `Dimension`: `TEX2D`
* `Width`/`Height`: swapchain buffer size
* `MipLevels`: `1`
* `ArraySize`: `1`
* `Format`: swapchain format (commonly `DXGI_FORMAT_B8G8R8A8_UNORM` on Win7 + DWM)
* `BindFlags`: must include render-target output (e.g. `D3D11_BIND_RENDER_TARGET`)
  * may include shader input if the swapchain `BufferUsage` requested it
* `CPUAccessFlags`: `0`
* `Usage`: typically `DEFAULT` (driver should treat any other value as suspicious for swapchain buffers)
* `SampleDesc`: typically `(Count=1, Quality=0)` (MSAA swapchains are out-of-scope for early bring-up)

#### Allocation flag invariants (KMD-facing)

For AeroGPU’s current MVP memory model (single system-memory segment), stability requirements are:

* **Preserve runtime-requested flags**:
  * In `DxgkDdiCreateAllocation`, do **not** zero `DXGK_ALLOCATIONINFO::Flags` for normal allocations.
    DXGI/runtime may set “special” bits for swapchain buffers; clearing them can break `Present`.
* **Ensure CPU visibility** (so the emulator can read/write the backing):
  * Set `DXGK_ALLOCATIONINFO::Flags.CpuVisible = 1`
  * Set `DXGK_ALLOCATIONINFO::Flags.Aperture = 1`

These invariants are intentionally conservative; as the trace data is collected, tighten the rules to match
exact Win7 runtime behavior.

---
