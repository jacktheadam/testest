# Build and tooling

> Building, running, and configuring Aero: the pinned toolchains, the task
> runner, the development environment, the full environment-variable surface,
> and how a build is turned into something servable.

Aero's workflows assume a small set of native tooling, all of it pinned. There
is deliberately **no dev container and no Nix flake**: containerised development
environments were purged as deployment theatre along with the rest of the
hosted-project apparatus, and the working environment is a plain headless Linux
box. Setup is `just setup`, which installs the Rust toolchains and targets and
runs the Node install with Playwright browser downloads disabled.

## Pinned toolchains

| Tool | Pin | Where |
|---|---|---|
| Rust stable | **1.92.0** (`rustfmt`, `clippy`, `wasm32-unknown-unknown`) | `rust-toolchain.toml`; `workspace.package.rust-version` matches |
| Rust nightly, threaded WASM only | **nightly-2025-12-08** | `scripts/toolchains.json` (`rust.nightlyWasm`) and `fuzz/rust-toolchain.toml` |
| Node | **24.18.0** (`engines: >=24 <25`) | `.nvmrc`, enforced by `scripts/check-node-version.mjs` from `pre*` scripts |
| Package manager | **pnpm 11.5.0** | `packageManager` in `package.json` |
| wasm-pack | 0.13.1 | a dev dependency, allow-listed to run its build script in `pnpm-workspace.yaml` |

**wasm-pack is pinned at 0.13.1 because upgrading is a dead end**, not because
nobody got round to it: its installer is broken at 0.14 and the organisation that
maintained it has been archived. Do not build new tooling on it. The intended
successor is calling `wasm-bindgen-cli` directly — its CLI version tracks the
crate version, so the two cannot drift — with Trunk as the alternative, and
Binaryen invoked directly for `wasm-opt`.

The test runner is `cargo test`. **cargo-nextest was chosen and never adopted**:
a CI wrapper script and an environment knob survive, but nothing in `xtask` or
the `justfile` invokes it and there is no configuration file. Either finish it or
drop the remnants.

Nightly exists **only** because threaded WebAssembly needs `-Z build-std` and
`rust-src`; it is pinned in two places rather than one because cargo-fuzz needs
its own toolchain file. Note that `scripts/ci/check-toolchains.mjs` validates
`rust-toolchain.toml`, `scripts/toolchains.json`, the WASM builder and the
`justfile` — it does **not** read `fuzz/rust-toolchain.toml`, so an unpinned
nightly there would pass the gate. See
[the Rust toolchain policy](../decisions/0009-rust-toolchain-policy.md).

**Two workspaces, one each.** The Rust workspace lists 94 members — 95 packages
once the root package itself is counted, which is why `cargo metadata` reports one
more than the manifest appears to. Every tool that was once a standalone
workspace has joined it — and `fuzz/` is separate only
because cargo-fuzz requires it. Node is one pnpm workspace over 11 packages.
`resolver = "3"` is deliberate and MSRV-aware. Feature unification across the
Rust workspace is handled by cargo-hakari (`.config/hakari.toml`,
`crates/workspace-hack/`), with a 55-crate traversal exclusion so the WebAssembly
dependency cone is not polluted by native-only features. Note that nothing
regenerates the hack automatically — `cargo hakari generate` is a manual step.

## The deferred majors

Several dependencies sit well behind current. This is deliberate: each lift is
its own migration with its own gate, not a version bump, and doing them during
the Windows 7 bring-up would trade the critical path for maintenance. What each
one costs when it is taken:

| Held at | Cost of lifting it |
|---|---|
| **wgpu 0.20.1** | The largest gap by far, and a yearly-breaking-release dependency. Touches `aero-webgpu`, `aero-gpu` and the D3D9/D3D11 executors. Must retain the WebGL fallback. |
| **Vite 5.4** | Vite 8 makes Rolldown the only bundler: no CommonJS config, `manualChunks` becomes `advancedChunks`, `rollupOptions` becomes `rolldownOptions`, and esbuild/Rollup plugin hooks need Rolldown equivalents. |
| **vitest 1.6.1** | `workspace` → `projects` is *removed* in v4, not deprecated; the default pool changed from threads to forks in v2; `mock.results` semantics changed (`mock.settledResults`); coverage defaults shifted. |
| **TypeScript** (spec `^5.5`, resolved 5.9.3) | Go through 6.0 first to surface deprecations — strict by default, ES5/AMD/UMD gone. TypeScript 7.0 has **no compiler API until 7.1**, so anything using it must stay on 6.0. |
| **Rust 1.92** | Expect new lints and errors; take it as its own gated step. |
| **Threaded-WASM nightly** | Nightlies from 2026-05-06 onward require `-Clink-arg=--export=__heap_base` or threaded builds break outright. |

Where each of those is heading, when it is taken: wgpu to the current 30.x line
(WebGPU is Baseline now, shipping in all four engines, so the fallback matters
less than it did); Rust to whatever stable is current at adoption; Vite to 8 on
Rolldown; vitest to 4.1; TypeScript to 7.0 **via a 6.0 bridge** — and before that
flip, someone has to inventory what still uses the compiler API, because 7.0 does
not have one until 7.1. Playwright is already ahead of its spec (1.61 installed
against `^1.57`), so that one is a version-bump, not a migration.

**Two standing rules that fall out of these.** New JavaScript is **ESM-only** —
pnpm and Vite both assume it, and the remaining CommonJS shims get ported rather
than carried. And for any Rust-side service, axum with tokio's LTS line is the
boring default; there is no appetite for a second service framework.

**Turborepo was evaluated and declined.** Task orchestration is `pnpm -r` plus
`xtask`, and Turborepo's value is caching for a CI that does not exist here.
Revisit only if build times actually hurt — the default answer is no.

There is currently **no JavaScript or TypeScript linter at all** — neither ESLint
nor Biome. `typecheck` runs `tsc` over the root and `apps/web` configs, and that
is the whole static gate for TypeScript. Adopting Biome is the intended answer
(one binary, nested configs for the workspace, and it sidesteps the
typescript-eslint/tsgo breakage), and it is worth taking together with the
deferred majors above rather than as separate churn.

## Known toolchain defects

These are real, verified, and worth fixing rather than documenting around:

- **A dead lockfile at `tools/win-offline-cert-injector/Cargo.lock`.** That crate
  is a root workspace member, so its own lockfile is never consulted. It is a
  leftover from the standalone-workspace consolidation.
- **The ban list exists in three divergent copies** —
  `tests/repo_hygiene.contract.test.js`, `tests/repo_hygiene_contract.test.js`,
  and `scripts/ci/check-repo-layout.sh` — with genuinely different entries: the
  shell copy omits `docs` and `services/image-gateway`. Because all three run,
  the effective policy is their *union*, so nothing is currently unguarded — but
  three lists that must be kept in step by hand will drift, and a reader cannot
  tell from any one of them what is actually banned. (`crates/emulator` is not a
  gap: the shell script guards it separately, and more strictly, by rejecting the
  path in the index rather than on disk.)
- **`crates/legacy/README.md` contradicts the manifest.** It states the legacy
  crates are excluded from the workspace, but `Cargo.toml` lists
  `crates/legacy/aero-d3d9-shader` as a member (correctly — the fuzz workspace
  uses it).
- **The disk-streaming conformance self-tests are broken.**
  `tools/disk-streaming-conformance/selftest_chunk_server_private.py` looks for
  `chunk_server.js` under the top-level `server/` tree, which was purged. The
  conformance checker itself is cited as current by
  [storage.md](./storage.md) and the delivery spec; the *self*-tests need
  rewiring to `tools/disk-gateway` plus a static chunk server before they mean
  anything.
- **`cargo check --offline` fails in `fuzz/`** — a few dependencies are not in
  the local registry cache, so an offline check tries to download and gives up.
  The lockfile itself is *not* drifted: `cargo metadata --locked` succeeds, and
  with network the whole workspace checks clean under `--all-features`. Worth
  knowing because the offline failure reads like lockfile rot and is not.
- **Clippy has 7 warnings** across `aero-cpu-core` (4 `needless_range_loop`),
  `aero-acpi` (`useless_vec`), `aero-interrupts` (`let_and_return`) and
  `aero-devices-storage` (`if_same_then_else`), plus 3 in a test target of
  `aero-cpu-core` (`unused_assignments`, `unused_mut`). It runs across the whole
  workspace again: a deny-by-default `never_loop` in `exec.rs` used to abort the
  run there, leaving every downstream crate unlinted.
- **`freedos_boot_smoke` passes without testing anything** when its disk image is
  absent — it prints a skip line and returns `Ok`, and is counted among the
  passes. Seventeen other tests name their conditionality (`*_if_fixture_present`,
  `*_when_available`); this one does not.
- **Two root package scripts point at a deleted config.** `dev:web` and
  `serve:coi:web` still reference `apps/web/vite.config.ts`, removed when the two
  browser trees were merged.

## Build speed, and when to spend effort on it

A dedicated `boot` profile and a faster linker are both worth having, and both
are worth doing **last**. The reason is the ordering lesson from the bring-up:
the build is only worth optimising once you have measured that it is the
expensive step, and for a snapshot-driven workflow it usually is not — the run it
feeds dominates. Measure first; the intuitive answer here has been wrong before.

## Playwright browsers

Browser binaries are not installed by setup. Install them once:

```bash
node scripts/playwright_install.mjs chromium
```

Then the full suite runs with `cargo xtask test-all`; `--skip-e2e` omits the
browser layer.

## Environment variables (canonical)

This repository uses environment variables to configure build/test/dev tooling and a few runtime components.
To reduce drift, **prefer the canonical `AERO_*` variables** listed here and avoid legacy aliases.

### Precedence rules

1. **CLI flags** (when available) override environment variables.
2. **Canonical env vars** (e.g. `AERO_NODE_DIR`) override legacy aliases.
3. **Legacy aliases** are accepted for one compatibility cycle and emit a warning to stderr.
4. If nothing is set, tooling uses a **repo-default auto-detected** value when possible.

### Validation / normalization helper

Many scripts share the same “where is the Node workspace / wasm crate?” questions. Use:

```bash
node scripts/env/resolve.mjs --format json
```

It validates and normalizes inputs (paths, booleans, URLs) and uses the same path
resolution logic as CI (`scripts/ci/detect-node-dir.mjs`, `scripts/ci/detect-wasm-crate.mjs`).

It also normalizes a few commonly-propagated boolean knobs (e.g. `AERO_REQUIRE_WEBGPU`,
`AERO_DISABLE_WGPU_TEXTURE_COMPRESSION`, `VITE_DISABLE_COOP_COEP`) to `0`/`1`.

It can also print shell assignments:

```bash
eval "$(node scripts/env/resolve.mjs --format bash --require-node-dir --require-wasm-crate-dir)"
```

### Canonical variables

| Variable | Meaning | Default | Consumed by | Examples |
| --- | --- | --- | --- | --- |
| `AERO_NODE_DIR` | Path (relative to repo root or absolute) to the **Node workspace entrypoint** directory containing `package.json`. With a single pnpm workspace this normally stays at `.` (repo root); use `pnpm -C <path> …` to run scripts in another package. | Auto-detected: prefer `.` then `frontend/` then `apps/web/` | `cargo xtask test-all`, `cargo xtask input`, `justfile` | `AERO_NODE_DIR=.`, `AERO_NODE_DIR=web` |
| `AERO_WASM_CRATE_DIR` | Path (relative to repo root or absolute) to the **wasm-pack Rust crate** (must contain `Cargo.toml` and declare a `cdylib`). | Auto-detected via `scripts/ci/detect-wasm-crate.mjs` (prefers `crates/aero-wasm`; otherwise uses `cargo metadata` only when the `cdylib` crate is unambiguous; fails if ambiguous). | `cargo xtask test-all` | `AERO_WASM_CRATE_DIR=crates/aero-wasm` |
| `AERO_WASM_PACKAGES` | Comma-separated list of WASM package IDs to build when running `apps/web/scripts/build_wasm.mjs` (and scripts that call it, like `pnpm -C apps/web run wasm:build`). Known IDs: `core`, `gpu`, `d3d11`, `jit`. | *(unset)* (build all packages) | `apps/web/scripts/build_wasm.mjs`, `cargo xtask input --e2e` | `AERO_WASM_PACKAGES=core pnpm -C apps/web run wasm:build` |
| `AERO_RSAENH` | Path to a local `rsaenh.dll` so the Authenticode hasher test can check its digest against the real signed file. Not committed, same reason as above; the test returns early when unset. | *(unset)* | `crates/aero-cpu-core/tests/cryptsp_sha256_update.rs` | `AERO_RSAENH=/path/to/rsaenh.dll cargo test -p aero-cpu-core` |
| `AERO_CRYPTSP_IMAGE` | Path to a locally-supplied mapped `cryptsp.dll` image for the guest-code differential tests. That image is a Microsoft binary and is deliberately not committed; without it the tests that need it **skip** rather than fail. Falls back to `crates/aero-cpu-core/tests/data/cryptsp_mapped.bin`, which is gitignored. | *(unset)* | `crates/aero-cpu-core/tests/cryptsp_sha256_update.rs`, `crates/aero-machine/src/jit.rs` | `AERO_CRYPTSP_IMAGE=/path/to/cryptsp_mapped.bin cargo test -p aero-cpu-core` |
| `AERO_REQUIRE_WEBGPU` | When true, WebGPU-tagged browser tests **must fail** if WebGPU is unavailable (instead of skipping). Accepts `1/0/true/false/yes/no/on/off`. | `0` | `cargo xtask test-all`, Playwright specs (`tests/**`) | `AERO_REQUIRE_WEBGPU=1 pnpm run test:e2e` |
| `AERO_DISABLE_WGPU_TEXTURE_COMPRESSION` | When true, Aero will **not request GPU texture compression features** (BC/ETC2/ASTC) even if the adapter supports them. This forces Aero to treat texture compression support as unavailable (enabling CPU decompression fallbacks where implemented). Note: Aero also avoids requesting these compression features on the **wgpu GL backend** by default due to correctness issues; this env var is an additional opt-out for other backends. Accepts `1/0/true/false/yes/no/on/off`. | `0` | wgpu device feature negotiation (`crates/aero-webgpu`, `crates/aero-gpu`, `crates/aero-d3d9`, etc.) and env normalization (`scripts/env/resolve.mjs`) | `AERO_DISABLE_WGPU_TEXTURE_COMPRESSION=1 cargo test -p aero-webgpu --locked`, `eval "$(node scripts/env/resolve.mjs --format bash --disable-wgpu-texture-compression)"` |
| `AERO_ALLOW_UNSUPPORTED_NODE` | If set to `1/true/yes/on`, `scripts/check-node-version.mjs` will **warn but exit 0** when the current Node.js version does not match the repo's pinned `.nvmrc`. Intended for constrained environments where you cannot install the pinned Node version. Note: `scripts/agent-env.sh` may auto-enable this when it detects a Node major mismatch. | *(unset)* | `scripts/check-node-version.mjs` (and therefore anything that calls it, like `cargo xtask`/`just`) | `AERO_ALLOW_UNSUPPORTED_NODE=1 cargo xtask test-all --skip-e2e` |
| `AERO_CHECK_NODE_QUIET` | If set to `1/true/yes/on`, `scripts/check-node-version.mjs` will **exit 0 without printing the “CI baseline differs” note** when the current version is newer than `.nvmrc` (or otherwise non-failing). This is useful for reducing noise in environments that intentionally run a newer Node major (unsupported; best-effort). | *(unset)* | `scripts/check-node-version.mjs` | `AERO_CHECK_NODE_QUIET=1 node scripts/check-node-version.mjs` |
| `AERO_NODE_VERSION_OVERRIDE` | Test-only override for `scripts/check-node-version.mjs`. When set, the script will treat it as the “detected” Node.js version instead of `process.versions.node` (useful for hermetic tooling/tests). | *(unset)* | `scripts/check-node-version.mjs` | `AERO_NODE_VERSION_OVERRIDE=22.11.0 node scripts/check-node-version.mjs` |
| `AERO_ISOLATE_CARGO_HOME` | Controls Cargo home isolation for agent sandboxes. If set to `1/true/yes/on`, agent helper scripts override `CARGO_HOME` to a **repo-local** `./.cargo-home` to avoid global Cargo registry lock contention on shared hosts. If set to any other non-false value, it is treated as a path (supports `~/` expansion; non-absolute paths are relative to repo root) and used as `CARGO_HOME`. Note: `scripts/safe-run.sh` will also auto-use an existing `./.cargo-home` when `AERO_ISOLATE_CARGO_HOME` is unset and `CARGO_HOME` is unset/default. | *(unset)* | `scripts/agent-env.sh`, `scripts/safe-run.sh`, Node test harnesses that spawn `cargo` (`tools/rust_l2_proxy.js`) | `AERO_ISOLATE_CARGO_HOME=1 source ./scripts/agent-env.sh` / `AERO_ISOLATE_CARGO_HOME=/tmp/aero-cargo-home bash ./scripts/safe-run.sh cargo build --locked` |
| `AERO_DISABLE_RUSTC_WRAPPER` | If set to `1/true/yes/on`, agent helper scripts will **force-disable** rustc wrappers by exporting empty wrapper env vars (which overrides global Cargo config). When unset, helper scripts still clear **environment-based sccache** wrappers by default for reliability, but preserve other wrappers like `ccache`. | *(unset)* | `scripts/agent-env.sh`, `scripts/safe-run.sh` | `AERO_DISABLE_RUSTC_WRAPPER=1 source ./scripts/agent-env.sh` / `AERO_DISABLE_RUSTC_WRAPPER=1 bash ./scripts/safe-run.sh cargo test --locked` |
| `AERO_CARGO_BUILD_JOBS` | Positive integer. If set, agent helper scripts will use this for Cargo parallelism (`CARGO_BUILD_JOBS`). Useful for tuning parallelism in agent sandboxes where high parallelism can cause `rustc` OS-resource panics (e.g. `failed to spawn helper thread (WouldBlock)` or `called Result::unwrap() on an Err value: Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }`). | `1` (agent default) | `scripts/agent-env.sh`, `scripts/safe-run.sh` | `AERO_CARGO_BUILD_JOBS=2 source ./scripts/agent-env.sh` |
| `RUSTC_WORKER_THREADS` | Positive integer. Controls rustc's internal worker pool size. Agent helper scripts default/sanitize it to `CARGO_BUILD_JOBS` for reliability under tight thread limits. | *(unset)* (agent default: `CARGO_BUILD_JOBS`) | rustc, `scripts/agent-env.sh`, `scripts/safe-run.sh` | `RUSTC_WORKER_THREADS=1 bash ./scripts/safe-run.sh cargo build --locked` |
| `RAYON_NUM_THREADS` | Positive integer. Controls Rayon global thread pool size (used by rustc and other crates). Agent helper scripts default/sanitize it to `CARGO_BUILD_JOBS` for reliability under tight thread limits. | *(unset)* (agent default: `CARGO_BUILD_JOBS`) | Rayon, `scripts/agent-env.sh`, `scripts/safe-run.sh` | `RAYON_NUM_THREADS=1 bash ./scripts/safe-run.sh cargo build --locked` |
| `RUST_TEST_THREADS` | Positive integer. Controls Rust's built-in test harness parallelism (libtest). Agent helper scripts default it to `CARGO_BUILD_JOBS` for reliability under tight thread limits. | *(unset)* (libtest default: `num_cpus`; agent default: `CARGO_BUILD_JOBS`) | libtest (`cargo test`), `scripts/agent-env.sh`, `scripts/safe-run.sh` | `RUST_TEST_THREADS=1 bash ./scripts/safe-run.sh cargo test --locked` |
| `NEXTEST_TEST_THREADS` | Positive integer (or `num-cpus`). Controls `cargo nextest run` test concurrency (`--test-threads`). Agent helper scripts default/sanitize it to `CARGO_BUILD_JOBS` for reliability under tight thread limits. | *(unset)* (nextest default: `num-cpus`; agent default: `CARGO_BUILD_JOBS`) | cargo-nextest, `scripts/agent-env.sh`, `scripts/safe-run.sh` | `NEXTEST_TEST_THREADS=1 bash ./scripts/safe-run.sh cargo nextest run --locked --workspace` |
| `AERO_TOKIO_WORKER_THREADS` | Positive integer. When set, supported Aero binaries will size their Tokio multi-thread runtime worker pool to this value (instead of Tokio's default `num_cpus`). Agent helper scripts default/sanitize it to `CARGO_BUILD_JOBS` for reliability under tight thread limits. | *(unset)* (agent default: `CARGO_BUILD_JOBS`) | `aero-l2-proxy`, `aero-storage-server`, `disk-gateway`, `tools/aero-gateway-rs`, `tools/image-chunker`, `scripts/agent-env.sh`, `scripts/safe-run.sh` | `AERO_TOKIO_WORKER_THREADS=1 bash ./scripts/safe-run.sh cargo run --locked --bin disk-gateway` |
| `AERO_RUST_CODEGEN_UNITS` | Optional positive integer. If set, agent helper scripts will add `-C codegen-units=<n>` to `RUSTFLAGS` when running Cargo commands (unless `RUSTFLAGS` already contains a `codegen-units=` setting). This can reduce per-crate thread usage and improve reliability under tight thread/PID limits. Alias: `AERO_CODEGEN_UNITS`. | *(unset)* | `scripts/agent-env.sh`, `scripts/safe-run.sh` | `AERO_RUST_CODEGEN_UNITS=1 bash ./scripts/safe-run.sh cargo test --locked` / `AERO_CODEGEN_UNITS=1 source ./scripts/agent-env.sh` |
| `AERO_SAFE_RUN_RUSTC_RETRIES` | Positive integer (attempt count, including the first run). Controls how many times `scripts/safe-run.sh` will retry Rust build/test commands (Cargo and common wrappers like `npm`/`wasm-pack`) when it detects transient OS resource-limit failures during thread/process spawn (EAGAIN/WouldBlock), e.g. `failed to spawn helper thread (WouldBlock)`, `called Result::unwrap() on an Err value: Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }`, or `fork: retry: Resource temporarily unavailable`. Set to `1` to disable retries. | `3` | `scripts/safe-run.sh` | `AERO_SAFE_RUN_RUSTC_RETRIES=1 bash ./scripts/safe-run.sh cargo test --locked` |
| `AERO_TIMEOUT` | Timeout in seconds used by `scripts/safe-run.sh` (wraps commands via `with-timeout.sh`). | `600` | `scripts/safe-run.sh` | `AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test --locked` |
| `AERO_MEM_LIMIT` | Virtual address space (RLIMIT_AS) limit used by `scripts/safe-run.sh` (passed through to `run_limited.sh`). | `12G` | `scripts/safe-run.sh` | `AERO_MEM_LIMIT=32G bash ./scripts/safe-run.sh pnpm -C apps/web run test:unit` |
| `AERO_NODE_TEST_MEM_LIMIT` | Fallback RLIMIT_AS limit used by `scripts/safe-run.sh` for **WASM-heavy Node test runners** when `AERO_MEM_LIMIT` is unset (helps under RLIMIT_AS). Applies to `node --test ...` and common wrappers like `pnpm run test:*` / Vitest and `wasm-pack test --node`. | `256G` | `scripts/safe-run.sh` | `AERO_NODE_TEST_MEM_LIMIT=24G bash ./scripts/safe-run.sh node --test tests/*.test.js` |
| `AERO_PLAYWRIGHT_MEM_LIMIT` | Fallback RLIMIT_AS limit used by `scripts/safe-run.sh` for **Playwright/browser E2E runs** when `AERO_MEM_LIMIT` is unset. Chromium/WebAssembly can reserve huge *virtual* address space; too-low RLIMIT_AS can crash Chromium or break `WebAssembly.Memory()` allocations. Applies to `pnpm run test:e2e`, `pnpm exec playwright ...`, and `cargo xtask ... --e2e`. | `256G` | `scripts/safe-run.sh` | `AERO_PLAYWRIGHT_MEM_LIMIT=128G bash ./scripts/safe-run.sh pnpm run test:e2e` |
| `AERO_PLAYWRIGHT_TIMEOUT` | Fallback timeout (seconds) used by `scripts/safe-run.sh` for Playwright/browser E2E runs when `AERO_TIMEOUT` is unset. E2E runs often trigger a WebAssembly build step (`pnpm -C apps/web run wasm:build`) which can exceed the default 10-minute timeout on cold caches. | `1800` | `scripts/safe-run.sh` | `AERO_PLAYWRIGHT_TIMEOUT=2400 bash ./scripts/safe-run.sh cargo xtask input --e2e` |
| `AERO_BENCH_COMPARE_PROFILE` | Optional perf-threshold profile name for benchmark comparison reports (e.g. `pr-smoke`, `nightly`). | Auto-detected by `scripts/compare-benchmarks.sh` based on input layout | `scripts/compare-benchmarks.sh` | `AERO_BENCH_COMPARE_PROFILE=pr-smoke bash ./scripts/compare-benchmarks.sh` |
| `AERO_BENCH_THRESHOLDS_FILE` | Path (relative to repo root or absolute) to the benchmark regression thresholds JSON file. | `bench/perf_thresholds.json` | `scripts/compare-benchmarks.sh` | `AERO_BENCH_THRESHOLDS_FILE=bench/perf_thresholds.json bash ./scripts/compare-benchmarks.sh` |
| `AERO_BENCH_COMPARE_MARKDOWN_OUT` | Output path for the generated benchmark comparison markdown report. | `bench_reports/compare.md` | `scripts/compare-benchmarks.sh` | `AERO_BENCH_COMPARE_MARKDOWN_OUT=/tmp/compare.md bash ./scripts/compare-benchmarks.sh` |
| `AERO_BENCH_COMPARE_JSON_OUT` | Output path for the generated benchmark comparison JSON artifact. | `bench_reports/compare.json` | `scripts/compare-benchmarks.sh` | `AERO_BENCH_COMPARE_JSON_OUT=/tmp/compare.json bash ./scripts/compare-benchmarks.sh` |
| `AERO_QEMU` | Path to a QEMU binary for Rust QEMU boot tests (`qemu-system-i386` / `qemu-system-x86_64`). | Auto-detected (`qemu-system-i386`, else `qemu-system-x86_64`) | Rust QEMU harness (`tests/harness/mod.rs`) | `AERO_QEMU=/usr/bin/qemu-system-i386 cargo test -p aero-boot-tests --test boot_sector --locked` |
| `AERO_ARTIFACT_DIR` | Output directory for failing test artifacts (e.g. screenshots/diffs). | `target/aero-test-artifacts` | Rust QEMU harness (`tests/harness/mod.rs`) | `AERO_ARTIFACT_DIR=/tmp/aero-artifacts cargo test -p aero-boot-tests --test windows7_boot --locked -- --ignored` |
| `AERO_REQUIRE_TEST_IMAGES` | When set (to any value), missing test fixture assets are treated as **hard errors** instead of skips. | *(unset)* | Rust QEMU harness (`tests/harness/mod.rs`) | `AERO_REQUIRE_TEST_IMAGES=1 cargo test -p aero-boot-tests --test freedos_boot --locked` |
| `AERO_UPDATE_TRACE_FIXTURES` | When set (to any value), GPU trace fixture tests will overwrite/regenerate the committed fixture files instead of asserting stability. | *(unset)* | `crates/aero-gpu-trace` tests, `crates/aero-gpu-trace-replay` tests | `AERO_UPDATE_TRACE_FIXTURES=1 cargo test -p aero-gpu-trace --locked` |
| `AERO_CONFORMANCE_CASES` | Number of instruction test cases to run in the differential conformance harness (`crates/conformance`). | `512` | `crates/conformance` | `AERO_CONFORMANCE_CASES=4096 cargo xtask conformance -- --nocapture` |
| `AERO_CONFORMANCE_SEED` | RNG seed for generating conformance test cases (to reproduce failures). Accepts decimal or `0x...` hex; `_` separators allowed. | `0x_52c6_71d9_a4f2_31b9` | `crates/conformance` | `AERO_CONFORMANCE_SEED=0x52c671d9a4f231b9 cargo xtask conformance -- --nocapture` |
| `AERO_CONFORMANCE_FILTER` | Filter string to select a subset of instruction templates/corpus entries (matches template `coverage_key` or substring of template name). | *(unset)* | `crates/conformance` | `AERO_CONFORMANCE_FILTER=add AERO_CONFORMANCE_CASES=4096 cargo xtask conformance -- --nocapture` |
| `AERO_CONFORMANCE_REFERENCE` | Select the reference backend for the instruction conformance harness. Set to `qemu` to compare against QEMU (requires the `conformance` crate built with `--features qemu-reference` and a `qemu-system-*` binary). Any other value/unset uses the native host reference backend (x86_64 unix). | *(unset)* (host) | `crates/conformance` | `AERO_CONFORMANCE_REFERENCE=qemu AERO_CONFORMANCE_CASES=16 cargo test -p conformance --features qemu-reference --test cpu_instruction_conformance instruction_conformance_host_reference --locked` |
| `AERO_CONFORMANCE_REPORT_PATH` | Optional output path for a JSON conformance report (written on failure; also written on success when set). | *(unset)* | `crates/conformance` | `AERO_CONFORMANCE_REPORT_PATH=target/conformance.json cargo xtask conformance -- --nocapture` |
| `AERO_CONFORMANCE_REFERENCE_ISOLATE` | When set to `0`, run the native host reference backend without `fork()` isolation (useful in sandboxed environments). | `1` | `crates/conformance` | `AERO_CONFORMANCE_REFERENCE_ISOLATE=0 cargo xtask conformance -- --nocapture` |
| `AERO_WINDOWS7_IMAGE` | Path to a user-supplied Windows 7 disk image for the gated/ignored `windows7_boot` test. | `test-images/local/windows7.img` (gitignored; not provided by repo) | `tests/windows7_boot.rs`, `scripts/prepare-windows7.sh` | `AERO_WINDOWS7_IMAGE=/path/to/windows7.img cargo test -p aero-boot-tests --test windows7_boot --locked -- --ignored` |
| `AERO_WINDOWS7_GOLDEN` | Path to an expected “golden” Windows 7 framebuffer screenshot to compare against for the gated/ignored `windows7_boot` test. | `test-images/local/windows7_login.png` (gitignored) | `tests/windows7_boot.rs`, `scripts/prepare-windows7.sh` | `AERO_WINDOWS7_GOLDEN=/path/to/windows7_login.png cargo test -p aero-boot-tests --test windows7_boot --locked -- --ignored` |
| `AERO_WIN7_ISO` | Path to a user-supplied Windows 7 install ISO used by the optional `aero-win7-slipstream` integration test. If unset, the test is skipped. | *(unset)* | `tools/win7-slipstream/tests/integration.rs` | `AERO_WIN7_ISO=/path/to/win7.iso AERO_WIN7_DRIVERS=/path/to/aero-drivers cargo test -p aero-win7-slipstream --locked` |
| `AERO_WIN7_DRIVERS` | Path to an Aero driver pack directory used by the optional `aero-win7-slipstream` integration test. If unset, the test is skipped. | *(unset)* | `tools/win7-slipstream/tests/integration.rs` | `AERO_WIN7_ISO=/path/to/win7.iso AERO_WIN7_DRIVERS=/path/to/aero-drivers cargo test -p aero-win7-slipstream --locked` |
| `AERO_WIN7_SLIPSTREAM_IMAGE` | Container image name for `tools/win7-slipstream/scripts/slipstream-in-container.{sh,ps1}`. | `aero/win7-slipstream` | `tools/win7-slipstream/scripts/slipstream-in-container.*` | `AERO_WIN7_SLIPSTREAM_IMAGE=aero/win7-slipstream tools/win7-slipstream/scripts/slipstream-in-container.sh deps` |
| `AERO_MAKE_FAT_IMAGE` | When set to `1/true/yes/on`, `drivers/build/package-drivers.ps1` will attempt to create an additional FAT32 VHD driver install media artifact (`*-fat.vhd`) alongside the normal ZIP/ISO outputs. This requires Windows + admin privileges; by default it is skipped with a warning if unavailable unless `-FatImageStrict` is passed. | *(unset)* | `drivers/build/package-drivers.ps1` | `AERO_MAKE_FAT_IMAGE=1 pwsh drivers/build/package-drivers.ps1` |
| `AERO_DRIVER_PFX_BASE64` | Base64-encoded PFX bytes used by `drivers/build/sign-drivers.ps1` to sign Windows drivers. | *(unset)* | `drivers/build/sign-drivers.ps1` | `AERO_DRIVER_PFX_BASE64=<base64> AERO_DRIVER_PFX_PASSWORD=<password> pwsh drivers/build/sign-drivers.ps1` |
| `AERO_DRIVER_PFX_PASSWORD` | Password for the PFX provided via `AERO_DRIVER_PFX_BASE64` when signing Windows drivers. | *(unset)* | `drivers/build/sign-drivers.ps1` | `AERO_DRIVER_PFX_BASE64=<base64> AERO_DRIVER_PFX_PASSWORD=<password> pwsh drivers/build/sign-drivers.ps1` |
| `AERO_PLAYWRIGHT_REUSE_SERVER` | When set to `1/true`, Playwright will reuse already-running harness/preview/CSP servers instead of starting new ones. Off by default in automated runs. | `0` | `playwright.config.ts`, `playwright.gpu.config.ts` | `AERO_PLAYWRIGHT_REUSE_SERVER=1 pnpm run test:e2e` |
| `AERO_PLAYWRIGHT_EXPOSE_GC` | When set to `1`, Playwright will pass `--expose-gc` to Chromium (via `--js-flags=--expose-gc`) for tests that need manual GC. | `0` | `playwright.config.ts`, `playwright.gpu.config.ts` | `AERO_PLAYWRIGHT_EXPOSE_GC=1 pnpm run test:e2e -- --project chromium` |
| `AERO_PLAYWRIGHT_DEV_PORT` | TCP port for the Playwright dev harness server (`pnpm run dev:harness`). | Auto-detected free port starting at `5173` | `playwright.config.ts`, `playwright.gpu.config.ts` | `AERO_PLAYWRIGHT_DEV_PORT=5174 pnpm run test:e2e` |
| `AERO_PLAYWRIGHT_DEV_ORIGIN` | Origin (URL, must include explicit port) for the Playwright dev harness server. When set, it overrides `AERO_PLAYWRIGHT_DEV_PORT` and is exported back into the environment for tests that need the resolved origin. | Auto: `http://127.0.0.1:<devPort>` | `playwright.config.ts`, `playwright.gpu.config.ts` | `AERO_PLAYWRIGHT_DEV_ORIGIN=http://127.0.0.1:5174 pnpm run test:e2e` |
| `AERO_PLAYWRIGHT_PREVIEW_PORT` | TCP port for the Playwright COI preview server (`pnpm run serve:coi:harness`). | Auto-detected free port starting at `4173` | `playwright.config.ts`, `playwright.gpu.config.ts` | `AERO_PLAYWRIGHT_PREVIEW_PORT=4174 pnpm run test:e2e` |
| `AERO_PLAYWRIGHT_PREVIEW_ORIGIN` | Origin (URL, must include explicit port) for the Playwright COI preview server. When set, it overrides `AERO_PLAYWRIGHT_PREVIEW_PORT` and is exported back into the environment for tests that need the resolved origin. | Auto: `http://127.0.0.1:<previewPort>` | `playwright.config.ts`, `playwright.gpu.config.ts` | `AERO_PLAYWRIGHT_PREVIEW_ORIGIN=http://127.0.0.1:4174 pnpm run test:e2e` |
| `AERO_PLAYWRIGHT_CSP_PORT` | TCP port for the Playwright CSP test server (`node tests/helpers/csp_server.mjs`). | Auto-detected free port starting at `4180` | `playwright.config.ts`, `playwright.gpu.config.ts` | `AERO_PLAYWRIGHT_CSP_PORT=4181 pnpm run test:e2e` |
| `AERO_PLAYWRIGHT_CSP_ORIGIN` | Origin (URL, must include explicit port) for the Playwright CSP test server. When set, it overrides `AERO_PLAYWRIGHT_CSP_PORT` and is exported back into the environment for tests that need the resolved origin. | Auto: `http://127.0.0.1:<cspPort>` | `playwright.config.ts`, `playwright.gpu.config.ts` | `AERO_PLAYWRIGHT_CSP_ORIGIN=http://127.0.0.1:4181 pnpm run test:e2e` |
| `VITE_DISABLE_COOP_COEP` | Disable COOP/COEP response headers on dev/preview servers (useful for validating the non-`SharedArrayBuffer` fallback). Accepts `1/0/true/false/yes/no/on/off`. | `0` | `vite.harness.config.ts`, `apps/web/vite.config.ts`, `apps/web/scripts/serve.cjs`, `tests/helpers/csp_server.mjs` | `VITE_DISABLE_COOP_COEP=1 pnpm run dev` |

### Backend / proxy URL variables (used by networking tooling)

These are not required for the core build/test pipeline, but are common when running local networking relays.

| Variable | Meaning | Default | Consumed by | Examples |
| --- | --- | --- | --- | --- |
| `UDP_RELAY_BASE_URL` | Base URL of the UDP relay service (`proxy/webrtc-udp-relay`) used by `services/gateway` to return `udpRelay` connection metadata in `POST /session`. Accepts `http(s)://` or `ws(s)://`. Browser clients must normalize between HTTP and WebSocket schemes when calling relay endpoints (e.g. `fetch()` does not support `ws(s)://`). | *(unset)* | `services/gateway` | `UDP_RELAY_BASE_URL=https://relay.example.com`, `UDP_RELAY_BASE_URL=wss://relay.example.com` |
| `UDP_RELAY_AUTH_MODE` | Relay auth mode used by `services/gateway` when minting `udpRelay.token` (`none`, `api_key`, `jwt`). | `none` | `services/gateway` | `UDP_RELAY_AUTH_MODE=jwt` |
| `UDP_RELAY_API_KEY` | API key to return when `UDP_RELAY_AUTH_MODE=api_key` (intended for local/dev only). | *(unset)* | `services/gateway` | `UDP_RELAY_API_KEY=dev-key` |
| `UDP_RELAY_JWT_SECRET` | HS256 secret used when `UDP_RELAY_AUTH_MODE=jwt` (gateway mints short-lived JWTs for the relay). | *(unset)* | `services/gateway` | `UDP_RELAY_JWT_SECRET=...` |
| `UDP_RELAY_TOKEN_TTL_SECONDS` | Token lifetime in seconds for gateway-minted relay credentials. | `300` | `services/gateway` | `UDP_RELAY_TOKEN_TTL_SECONDS=300` |
| `UDP_RELAY_AUDIENCE` | Optional JWT `aud` claim when `UDP_RELAY_AUTH_MODE=jwt`. | *(unset)* | `services/gateway` | `UDP_RELAY_AUDIENCE=relay` |
| `UDP_RELAY_ISSUER` | Optional JWT `iss` claim when `UDP_RELAY_AUTH_MODE=jwt`. | *(unset)* | `services/gateway` | `UDP_RELAY_ISSUER=aero-gateway` |
| `AERO_WEBRTC_UDP_RELAY_PUBLIC_BASE_URL` | Public base URL for the WebRTC UDP relay (logging only). Must be a valid URL. | *(unset)* | `proxy/webrtc-udp-relay` | `AERO_WEBRTC_UDP_RELAY_PUBLIC_BASE_URL=https://relay.example.com` |
| `AERO_STUN_URLS` | Comma-separated STUN URLs (e.g. `stun:stun.l.google.com:19302`). | *(unset)* | `proxy/webrtc-udp-relay` | `AERO_STUN_URLS=stun:stun.l.google.com:19302` |
| `AERO_TURN_URLS` | Comma-separated TURN URLs. | *(unset)* | `proxy/webrtc-udp-relay` | `AERO_TURN_URLS=turn:turn.example.com:3478?transport=udp` |
| `UDP_BINDING_IDLE_TIMEOUT` | Close idle UDP port bindings after this duration (Go duration, e.g. `60s`). | `60s` | `proxy/webrtc-udp-relay` | `UDP_BINDING_IDLE_TIMEOUT=30s` |
| `UDP_INBOUND_FILTER_MODE` | Inbound UDP filtering mode (`address_and_port` or `any`). `any` behaves like full-cone NAT and is less safe. | `address_and_port` | `proxy/webrtc-udp-relay` | `UDP_INBOUND_FILTER_MODE=address_and_port` / `UDP_INBOUND_FILTER_MODE=any` |
| `UDP_REMOTE_ALLOWLIST_IDLE_TIMEOUT` | Expire inactive remote allowlist entries after this duration (only applies when `UDP_INBOUND_FILTER_MODE=address_and_port`). | Defaults to `UDP_BINDING_IDLE_TIMEOUT` | `proxy/webrtc-udp-relay` | `UDP_REMOTE_ALLOWLIST_IDLE_TIMEOUT=30s` |
| `MAX_ALLOWED_REMOTES_PER_BINDING` | Cap the number of remote endpoints tracked per UDP binding allowlist (DoS hardening). | `1024` | `proxy/webrtc-udp-relay` | `MAX_ALLOWED_REMOTES_PER_BINDING=1024` |
| `WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES` | Advertised WebRTC DataChannel max message size (`a=max-message-size`). Best-effort guardrail for compliant peers (0 = auto). | Auto: `max(MAX_DATAGRAM_PAYLOAD_BYTES+24, L2_MAX_MESSAGE_BYTES)+256` (`24` is the v2 IPv6 UDP relay frame overhead; see `proxy/webrtc-udp-relay/internal/udpproto.MaxFrameOverheadBytes`) | `proxy/webrtc-udp-relay` | `WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES=65536` |
| `WEBRTC_SCTP_MAX_RECEIVE_BUFFER_BYTES` | Hard cap on SCTP receive buffering before `DataChannel.OnMessage` runs (0 = auto; must be ≥ `WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES` and ≥ `1500`). | Auto: `max(1048576, 2*WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES)` | `proxy/webrtc-udp-relay` | `WEBRTC_SCTP_MAX_RECEIVE_BUFFER_BYTES=1048576` |
| `WEBRTC_SESSION_CONNECT_TIMEOUT` | Close server-side WebRTC sessions that fail to reach `PeerConnectionStateConnected` within this duration (prevents session leaks; Go duration). | `30s` | `proxy/webrtc-udp-relay` | `WEBRTC_SESSION_CONNECT_TIMEOUT=30s` |

### L2 tunnel proxy variables (`crates/aero-l2-proxy`)

These variables configure the L2 tunnel termination proxy used by the “Option C” networking path (see [`networking.md`](networking.md) for a practical guide).

This table is **not exhaustive** (there are additional stack tuning and observability knobs); it covers the most commonly-used and security-sensitive options.

| Variable | Meaning | Default | Consumed by | Examples |
| --- | --- | --- | --- | --- |
| `AERO_L2_PROXY_LISTEN_ADDR` | Socket address to listen on for the proxy’s HTTP/WebSocket endpoints (including `/l2` and legacy alias `/eth`, plus `/metrics`, `/healthz`). Alias: `AERO_L2_PROXY_BIND_ADDR`. | `0.0.0.0:8090` | `crates/aero-l2-proxy` | `AERO_L2_PROXY_LISTEN_ADDR=127.0.0.1:8090 cargo run --locked -p aero-l2-proxy` |
| `AERO_L2_ALLOWED_ORIGINS` | Comma-separated allowlist of normalized Origin values allowed to upgrade to `/l2` (and legacy alias `/eth`). Supports `"*"` to allow any valid Origin (but still requires the header unless `AERO_L2_OPEN=1`). Legacy alias: `ALLOWED_ORIGINS`. | Same-host only (empty allowlist) | `crates/aero-l2-proxy` | `AERO_L2_ALLOWED_ORIGINS=http://localhost:5173 cargo run --locked -p aero-l2-proxy` |
| `AERO_L2_ALLOWED_ORIGINS_EXTRA` | Additional comma-separated Origin entries appended to the base allowlist (useful when `ALLOWED_ORIGINS` is shared across services). | *(unset)* | `crates/aero-l2-proxy` | `ALLOWED_ORIGINS=https://example.com AERO_L2_ALLOWED_ORIGINS_EXTRA=,http://localhost:5173 cargo run --locked -p aero-l2-proxy` |
| `AERO_L2_ALLOWED_HOSTS` | Comma-separated allowlist of Host header values accepted at WebSocket upgrade time. When unset/empty, Host validation is disabled. | *(unset)* | `crates/aero-l2-proxy` | `AERO_L2_ALLOWED_HOSTS=proxy.example.com` |
| `AERO_L2_TRUST_PROXY_HOST` | When true, prefer proxy-provided host headers (`Forwarded: host=` / `X-Forwarded-Host`) over `Host` when validating `AERO_L2_ALLOWED_HOSTS`. Only enable behind a trusted reverse proxy. | `0` | `crates/aero-l2-proxy` | `AERO_L2_TRUST_PROXY_HOST=1` |
| `AERO_L2_OPEN` | Security escape hatch: when set to exactly `1`, disable Origin enforcement (trusted local development only). Note: this does **not** automatically disable authentication. | `0` | `crates/aero-l2-proxy` | `AERO_L2_OPEN=1 AERO_L2_AUTH_MODE=none cargo run --locked -p aero-l2-proxy` |
| `AERO_L2_INSECURE_ALLOW_NO_AUTH` | When set to exactly `1` alongside `AERO_L2_OPEN=1`, allow the proxy to start with no auth configured (explicit unauthenticated mode). | `0` | `crates/aero-l2-proxy` | `AERO_L2_OPEN=1 AERO_L2_INSECURE_ALLOW_NO_AUTH=1 cargo run --locked -p aero-l2-proxy` |
| `AERO_L2_AUTH_MODE` | Authentication mode for `/l2` upgrades (and legacy alias `/eth`) (`none`, `token`, `session`, `session_or_token`, `session_and_token`, `jwt`, `cookie_or_jwt`, …). | Auto-detected (may fail if no auth is configured) | `crates/aero-l2-proxy` | `AERO_L2_AUTH_MODE=jwt AERO_L2_JWT_SECRET=... cargo run --locked -p aero-l2-proxy` |
| `AERO_L2_API_KEY` | Static API key used when token auth is enabled. Legacy alias: `AERO_L2_TOKEN`. | *(unset)* | `crates/aero-l2-proxy` | `AERO_L2_AUTH_MODE=token AERO_L2_API_KEY=sekrit cargo run --locked -p aero-l2-proxy` |
| `AERO_L2_SESSION_SECRET` | HMAC secret used to verify the `aero_session` cookie when session/cookie auth is enabled. Also accepts shared `SESSION_SECRET` / `AERO_GATEWAY_SESSION_SECRET`. | *(unset)* | `crates/aero-l2-proxy` | `AERO_L2_AUTH_MODE=session AERO_L2_SESSION_SECRET=sekrit cargo run --locked -p aero-l2-proxy` |
| `AERO_L2_JWT_SECRET` | HMAC secret used to verify JWTs when JWT auth is enabled. | *(unset)* | `crates/aero-l2-proxy` | `AERO_L2_AUTH_MODE=jwt AERO_L2_JWT_SECRET=sekrit cargo run --locked -p aero-l2-proxy` |
| `AERO_L2_JWT_AUDIENCE` | Optional expected JWT `aud` claim (JWT auth modes). | *(unset)* | `crates/aero-l2-proxy` | `AERO_L2_JWT_AUDIENCE=aero` |
| `AERO_L2_JWT_ISSUER` | Optional expected JWT `iss` claim (JWT auth modes). | *(unset)* | `crates/aero-l2-proxy` | `AERO_L2_JWT_ISSUER=aero-gateway` |
| `AERO_L2_MAX_CONNECTIONS` | Process-wide concurrent tunnel cap (`0` disables). | `64` | `crates/aero-l2-proxy` | `AERO_L2_MAX_CONNECTIONS=128` |
| `AERO_L2_MAX_CONNECTIONS_PER_SESSION` | Concurrent tunnel cap per authenticated session principal (`0` disables). Legacy alias: `AERO_L2_MAX_TUNNELS_PER_SESSION`. | `0` | `crates/aero-l2-proxy` | `AERO_L2_MAX_CONNECTIONS_PER_SESSION=4` |
| `AERO_L2_MAX_CONNECTIONS_PER_IP` | Concurrent tunnel cap per client IP (`0` disables). Note: only meaningful when the proxy can determine a stable client IP (see `AERO_L2_TRUST_PROXY`). | `0` | `crates/aero-l2-proxy` | `AERO_L2_MAX_CONNECTIONS_PER_IP=8` |
| `AERO_L2_TRUST_PROXY` | When true, trust proxy-provided client IP headers (`Forwarded` / `X-Forwarded-For`) for per-IP limits and logging. Only enable behind a trusted reverse proxy. | `0` | `crates/aero-l2-proxy` | `AERO_L2_TRUST_PROXY=1` |
| `AERO_L2_MAX_BYTES_PER_CONNECTION` | Total bytes per connection (rx + tx, `0` disables). | `0` | `crates/aero-l2-proxy` | `AERO_L2_MAX_BYTES_PER_CONNECTION=100000000` |
| `AERO_L2_MAX_FRAMES_PER_SECOND` | Inbound messages per second per connection (`0` disables). | `0` | `crates/aero-l2-proxy` | `AERO_L2_MAX_FRAMES_PER_SECOND=1000` |
| `AERO_L2_MAX_FRAME_PAYLOAD` | Max `FRAME` payload bytes for the L2 tunnel protocol (defense in depth). Legacy alias: `AERO_L2_MAX_FRAME_SIZE`. Values must be positive integers; `0`/blank are treated as unset (defaults apply). | `2048` | `crates/aero-l2-proxy`, `services/gateway` (surfaced via `POST /session`) | `AERO_L2_MAX_FRAME_PAYLOAD=2048` |
| `AERO_L2_MAX_CONTROL_PAYLOAD` | Max control payload bytes for the L2 tunnel protocol (PING/PONG/ERROR; defense in depth). Values must be positive integers; `0`/blank are treated as unset (defaults apply). | `256` | `crates/aero-l2-proxy`, `services/gateway` (surfaced via `POST /session`) | `AERO_L2_MAX_CONTROL_PAYLOAD=256` |
| `AERO_L2_CAPTURE_DIR` | When set, write a per-tunnel PCAPNG capture file into this directory. | *(unset)* | `crates/aero-l2-proxy` | `AERO_L2_CAPTURE_DIR=/tmp/aero-l2-captures` |
| `AERO_L2_CAPTURE_MAX_BYTES` | Maximum **capture file size** written per tunnel session (includes PCAPNG container overhead, not just Ethernet payload). `0` disables the cap. | `67108864` (64 MiB) | `crates/aero-l2-proxy` | `AERO_L2_CAPTURE_MAX_BYTES=67108864` |
| `AERO_L2_CAPTURE_FLUSH_INTERVAL_MS` | Flush interval for capture writers. `0` disables periodic flushing (capture is flushed on close). | `1000` | `crates/aero-l2-proxy` | `AERO_L2_CAPTURE_FLUSH_INTERVAL_MS=1000` |
| `AERO_L2_PING_INTERVAL_MS` | When set to a positive integer, the proxy will send protocol-level PINGs at this interval and record RTT metrics. | *(unset)* | `crates/aero-l2-proxy` | `AERO_L2_PING_INTERVAL_MS=1000` |
| `AERO_L2_IDLE_TIMEOUT_MS` | When set to a positive integer, close the tunnel if no inbound messages are received for this duration. | *(unset)* | `crates/aero-l2-proxy` | `AERO_L2_IDLE_TIMEOUT_MS=30000` |
| `AERO_L2_ALLOW_PRIVATE_IPS` | When true, allow egress to RFC1918/private/reserved IPv4 ranges (dev-only; production should keep this off). | `0` | `crates/aero-l2-proxy` | `AERO_L2_ALLOW_PRIVATE_IPS=1` |
| `AERO_L2_ALLOWED_TCP_PORTS` | Comma-separated TCP destination port allowlist. When set, TCP egress is denied by default unless the port is listed. | Allow all | `crates/aero-l2-proxy` | `AERO_L2_ALLOWED_TCP_PORTS=80,443` |
| `AERO_L2_ALLOWED_UDP_PORTS` | Comma-separated UDP destination port allowlist. When set, UDP egress is denied by default unless the port is listed. | Allow all | `crates/aero-l2-proxy` | `AERO_L2_ALLOWED_UDP_PORTS=53,3478` |
| `AERO_L2_ALLOWED_DOMAINS` | Comma-separated DNS name suffix allowlist. When non-empty, outbound DNS/TCP destinations must match at least one suffix. | Allow all | `crates/aero-l2-proxy` | `AERO_L2_ALLOWED_DOMAINS=example.com,example.org` |
| `AERO_L2_BLOCKED_DOMAINS` | Comma-separated DNS name suffix denylist (applied before `AERO_L2_ALLOWED_DOMAINS`). | *(unset)* | `crates/aero-l2-proxy` | `AERO_L2_BLOCKED_DOMAINS=metadata.google.internal` |

### Deprecated aliases (will be removed)

These names are still accepted but emit warnings. Prefer the canonical variables above.

| Deprecated | Use instead |
| --- | --- |
| `AERO_WEB_DIR` | `AERO_NODE_DIR` |
| `WEB_DIR` | `AERO_NODE_DIR` |
| `AERO_WASM_DIR` | `AERO_WASM_CRATE_DIR` |
| `WASM_CRATE_DIR` | `AERO_WASM_CRATE_DIR` |

## Build cost of the bring-up binary

`aero-machine-cli` is the binary a CPU-core change is tested with, so its
dependency cone sets the cost of every bring-up experiment. That cone is **357
crates**, and it includes `wgpu` and `naga` — a boot-debugging tool pulls in a
GPU shader compiler, because the canonical machine wires the GPU device model in
unconditionally. Feature-gating the GPU backend out of the CLI would let a
CPU-core edit rebuild a handful of crates and relink [aspirational].

Weigh that against where the time actually goes before acting on it: an
incremental release rebuild after a CPU-core edit is fast relative to the guest
run it feeds, so build tuning is real but usually small next to the run itself.
The measure-the-loop principle in
[../meta/engineering-principles.md](../meta/engineering-principles.md) applies
directly here — time both the cold and the steady-state case before choosing
what to optimise.

The release profile uses fat LTO with a single codegen unit. That is not a
size or micro-tuning choice: the interpreter's hot path crosses a crate boundary
on nearly every guest instruction, and without cross-crate inlining each one
pays several real calls. See
[performance.md](./performance.md) for the measurements.

## Releases and deployment

**There is no release train, and deploying is out of scope.** Continuous
integration, container registries, hosting-provider recipes,
infrastructure-as-code and Kubernetes manifests were all purged and are on the
ban list — see [the working agreements](../meta/working-agreements.md) and the
inventory in [the theater and legacy purge](../history/retirements.md). This
page used to carry a full runbook for all of it, referencing workflow files and
deployment directories that no longer exist.

What survives is the part that is real: the provenance a build emits, and the
cross-origin isolation the application genuinely requires in order to run at
all.


### Provenance metadata

#### Web

- `GET /aero.version.json` is generated during the Vite build and includes:
  - `version` (tag or ref)
  - `gitSha`
  - `builtAt` (UTC ISO-8601)
- The UI renders the same information under **Build info**.

#### Gateway

- `GET /version` returns:
  - `version`
  - `gitSha`
  - `builtAt`

#### L2 tunnel proxy

- `GET /version` returns:
  - `version`
  - `gitSha`
  - `builtAt`

#### Storage server

- `GET /version` returns:
  - `version`
  - `gitSha`
  - `builtAt`

#### WebRTC UDP relay

- `GET /version` returns:
  - `commit`
  - `buildTime`

---

### Reproducibility / pinned toolchains

Release workflows intentionally use the same pinned toolchain policy as CI:

- Node.js is pinned via the repo root [`.nvmrc`](../../.nvmrc).
- Rust stable is pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml).
- The pinned nightly used for threaded WASM lives in [`scripts/toolchains.json`](../../scripts/toolchains.json) (`rust.nightlyWasm`).

## Deployment & Hosting (COOP/COEP / cross-origin isolation)

Aero performs best with **WebAssembly threads / shared memory** (SharedArrayBuffer + Atomics).
Browsers only enable these capabilities in a **cross-origin isolated** context (see
[Cross-origin isolation](../decisions/0002-cross-origin-isolation.md)).

This repo also ships a **non-shared-memory fallback** WASM build (see
[WebAssembly build variants](../decisions/0004-wasm-build-variants.md)). When COOP/COEP headers are missing,
the web runtime will automatically load the single-threaded/non-shared variant so
the app can still start (degraded functionality/performance is expected).

That means your deployment **must** send these headers on the top-level document
and all subresources (JS, WASM, worker scripts, etc.):

- `Cross-Origin-Opener-Policy: same-origin` (COOP)
- `Cross-Origin-Embedder-Policy: require-corp` (COEP)
- `Cross-Origin-Resource-Policy: same-origin` (CORP, recommended hardening)

This repository includes production-ready header templates for common hosts.
For the full recommended hardening set (including CSP with `wasm-unsafe-eval` for Aero’s WASM-based JIT),
see [`security.md`](security.md).

Recommended additional hardening headers (included in the templates):

- `Cross-Origin-Resource-Policy: same-origin`
- `Origin-Agent-Cluster: ?1`

---

### Why COOP/COEP is required

When COOP/COEP are missing, browsers will report:

- `crossOriginIsolated === false`
- `SharedArrayBuffer` may be unavailable
- WASM `shared: true` memories / thread pools will fail to initialize (the threaded build cannot load)

In practice, the runtime loader will fall back to the non-shared-memory build.
This is primarily intended as a compatibility path; Windows 7 workloads will be
unacceptably slow without threads.

---

### Local verification (preview server)

The Vite preview server is configured to send COOP/COEP (and the rest of the recommended hardening set).

The canonical values live in `scripts/headers.json`. `scripts/security_headers.mjs`
reads that file and re-exports it as `crossOriginIsolationHeaders`,
`baselineSecurityHeaders`, and `cspHeaders`.

Only the two Vite configs import it, so only they are guaranteed to agree with it:

- `vite.harness.config.ts`
- `apps/web/vite.config.ts`

The other places that send the same headers restate them as literals, and are
therefore free to drift:

- `services/gateway/src/middleware/crossOriginIsolation.ts`
- `services/gateway/src/middleware/securityHeaders.ts`
- `apps/web/public/_headers` — the one surviving static-host header template

Drift used to be caught by a checker that required the Vite configs to import the
canonical module and compared the literal copies against it value by value. It
went with the rest of CI, so nothing catches drift now: editing `headers.json`
silently updates the dev and preview servers and leaves the gateway and the
static-host template behind. Until a replacement exists, treat an edit to
`headers.json` as an edit to all three literal copies too.

```bash
pnpm install --frozen-lockfile
pnpm run build
pnpm run preview
```

Then open the printed URL (usually `http://localhost:4173`) and verify:

- Open DevTools Console and run: `crossOriginIsolated` → should be `true`
- Optionally also check: `typeof SharedArrayBuffer !== 'undefined'`

> Note: `http://localhost` is treated as a secure context by browsers, so COOP/COEP works
> in local preview mode. In production you must serve over **HTTPS** (non-localhost
> `http://` will not be cross-origin isolated and will not get SharedArrayBuffer).

If `crossOriginIsolated` is `false`, inspect the **Network** tab and confirm the
main document response includes the required headers.

This repo also includes a Playwright integration test that asserts headers are
present on HTML + JS + worker + WASM responses:

```bash
pnpm run test:security-headers
```

#### Testing the fallback path (no COOP/COEP)

The repo-root Vite dev server can be started with COOP/COEP disabled:

```bash
VITE_DISABLE_COOP_COEP=1 pnpm run dev
```

In this mode `crossOriginIsolated` should be `false` and the runtime will load the
single-threaded/non-shared WASM variant.

---

### Caching defaults (safe + update-friendly)

The provided `_headers` rules do the following:

- **HTML / routes**: `Cache-Control: no-cache` (so updates roll out quickly)
- **Hashed static assets** (`/assets/*`): `Cache-Control: public, max-age=31536000, immutable`

If you change Vite’s `assetsDir` or add non-hashed critical files, review and
adjust caching rules accordingly.

---

### Troubleshooting / gotchas

1. **Avoid third-party CDNs for JS/WASM/worker scripts**
   - With `COEP: require-corp`, cross-origin subresources must be served with
     CORS or `Cross-Origin-Resource-Policy` headers. The simplest path is to
     serve everything from the same origin.
2. **WASM content-type**
   - Some static hosts serve `.wasm` with an incorrect `Content-Type`, which can
     break `WebAssembly.compileStreaming(...)`. The templates include
     `Content-Type: application/wasm` for `*.wasm` assets.

---

### Browser storage quota & persistence (OPFS durability)

This project stores large disk images in the browser (OPFS / `navigator.storage.getDirectory()`).
Browsers are allowed to evict site storage under pressure unless the origin has been granted
**persistent storage** (`navigator.storage.persist()`).

#### Storage quota reporting

The Disk Images panel shows:

- total estimated usage
- total estimated quota
- percent used

When usage exceeds ~80%, the UI warns that imports may fail.

#### Requesting persistent storage

The Disk Images panel includes a **Request persistent storage** button.

- If supported and granted, the UI shows `Persistent storage: granted`.
- If denied, the UI shows `Persistent storage: not granted`.
- If unsupported, the UI shows `Persistent storage: unsupported`.

#### Manual test instructions

1. Start the web UI:
   - `pnpm install --frozen-lockfile`
   - `pnpm -C apps/web run dev`
2. Open the Disk Images panel and verify quota numbers render (Chrome/Edge/Firefox support `navigator.storage.estimate()`).
3. Click **Request persistent storage**:
   - Chrome/Edge: often grants automatically for installed PWAs or high-engagement sites.
   - Firefox: may grant or deny depending on settings.
   - Safari: typically does not support the persistence APIs (expect `unsupported`).
4. Attempt importing a large file when storage is nearly full:
   - The import flow checks estimated remaining space and prompts for confirmation if it appears low.

---
