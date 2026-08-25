# WebAssembly build variants

**Decision (0004).** Build both threaded and single-threaded WebAssembly variants and choose between them at runtime, because cross-origin isolation cannot be assumed of every deployment.

## Context

Aero benefits significantly from multi-threading, but **WASM threads require `SharedArrayBuffer`**, which in turn requires **cross-origin isolation** (the cross-origin isolation decision). Not all deployment environments can set COOP/COEP headers, and not all browsers/users will run in an isolated context.

We need a strategy that:

- Uses threads where available for performance.
- Still allows the project to run (with reduced performance) without special hosting.

## Decision

Ship **two WebAssembly build variants**:

1. **Threaded build**
   - Compiled with WASM atomics/threads enabled.
   - Uses `WebAssembly.Memory({ shared: true })`.
   - Requires `crossOriginIsolated === true` at runtime.
   - Built with the internal Cargo feature `wasm-threaded` so Rust code takes the shared-memory-safe
     paths (byte-granular atomic loads/stores, shared scanout/cursor state headers, etc).

2. **Single-threaded build**
   - Compiled without threads/atomics.
   - Uses `WebAssembly.Memory({ shared: false })`.
   - Works without cross-origin isolation.

### Implementation details (repo-specific)

This repo implements the two variants as **two sets of wasm-pack packages** (single vs threaded).

The canonical VM/runtime crate (`crates/aero-wasm`) produces:

- `apps/web/src/wasm/pkg-threaded/` – shared-memory build (requires `crossOriginIsolated`).
- `apps/web/src/wasm/pkg-single/` – non-shared-memory fallback build.

Additional wasm-pack packages follow the same variant split (where applicable), for example:

- GPU runtime (`crates/aero-gpu-wasm`):
  - `apps/web/src/wasm/pkg-threaded-gpu/`
  - `apps/web/src/wasm/pkg-single-gpu/`
- Tier-1 compiler / JIT support (`crates/aero-jit-wasm`, when present):
  - `apps/web/src/wasm/pkg-jit-threaded/`
  - `apps/web/src/wasm/pkg-jit-single/`

Note: the Tier-1 compiler itself is single-threaded. The browser JIT worker loader
(`apps/web/src/runtime/jit_wasm_loader.ts`) currently **prefers `pkg-jit-single` even in
crossOriginIsolated / WASM-threads-capable environments**, because a shared-memory
wasm-bindgen build + a large `--max-memory` can cause the glue code to eagerly
allocate a multi‑GiB `SharedArrayBuffer` during instantiation.

Build commands:

```bash
pnpm run wasm:build        # builds both variants (via the `apps/web/` workspace)
```

For the threaded build, `apps/web/scripts/build_wasm.mjs` uses a **pinned nightly toolchain** (see
[Rust toolchain policy](0009-rust-toolchain-policy.md)) and nightly `build-std` so the standard library is rebuilt with
atomics/bulk-memory enabled (required by `--shared-memory`). It also passes `--features wasm-threaded`
to the Rust crates that opt into the threaded variant.

At runtime, select the variant via feature detection:

- `crossOriginIsolated === true`
- `typeof SharedArrayBuffer !== 'undefined'`
- (optionally) validate that shared `WebAssembly.Memory` is constructible

The JS/TS host should present a consistent API to the rest of the app regardless of the selected variant.

In this repo, runtime selection is implemented by:

- [`apps/web/src/runtime/wasm_loader.ts`](../../apps/web/src/runtime/wasm_loader.ts) (`initWasm()` returns `{ api, variant, reason }`)
- [`apps/web/src/runtime/wasm_context.ts`](../../apps/web/src/runtime/wasm_context.ts) (worker-safe wrapper; prefers `wasm_loader` and falls back to the embedded demo wasm)

The build script that generates the two packages is:

- [`apps/web/scripts/build_wasm.mjs`](../../apps/web/scripts/build_wasm.mjs)

### Testing the fallback path

To simulate a non-cross-origin-isolated deployment locally, start Vite with COOP/COEP disabled:

```bash
VITE_DISABLE_COOP_COEP=1 pnpm run dev
```

## Alternatives considered

1. **Threaded-only**
   - Pros: simplest code path; best performance.
   - Cons: blocks many hosting scenarios; harder to iterate/share demos.

2. **Single-threaded-only**
   - Pros: simplest deployment.
   - Cons: likely cannot meet performance goals.

3. **One build with “optional threads”**
   - Pros: fewer artifacts.
   - Cons: in practice still requires compiling different feature sets; the memory sharing model differs fundamentally.

## Consequences

- Build and CI must produce and test two artifacts.
- Host code must handle runtime selection and expose a stable interface.
- Documentation must clearly state that “threads/SAB are required for the high-performance path” and explain deployment headers.
