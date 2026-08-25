# Testing

> Every layer of verification in the repo — Rust unit and integration tests,
> WebAssembly tests, TypeScript unit tests, Playwright end-to-end runs, guest
> boot tests, instruction conformance, fuzzing, and benchmarks — plus the
> fixture policy and the resource limits that keep any of it from taking the
> host down.

**Verification status** — the current numbers live in
[the state page](../state/repo-state-and-structure.md#verification-status) and are
not restated here, because a duplicated status is a status that goes stale. In
short: Rust and JavaScript gates both run and both pass apart from a handful of
triaged failures, and the browser end-to-end suite has not been run in this
environment. Everything below is the machinery, not a claim about a particular
run.

## Running the stack locally

Node must match `.nvmrc` (24.18.0); `node scripts/check-node-version.mjs`
(`pnpm run check:node`) enforces it, `AERO_ALLOW_UNSUPPORTED_NODE=1` downgrades
to a warning. Install deps once with `pnpm install --frozen-lockfile` (one pnpm
workspace — see the Node monorepo tooling decision); run package scripts as
`pnpm -C <path> run <script>`.

- **Unified runner:** `cargo xtask test-all` (transitional wrapper:
 `bash ./scripts/test-all.sh`). Steps, in order: `fixtures --check`,
 `bios-rom --check`, `cargo fmt --check`, clippy (deny warnings),
 `cargo test --locked --workspace --all-features`, `wasm-pack test --node`,
 wasm32 build sanity check, `pnpm run test:unit`, `pnpm run test:e2e`.
 Options: `--skip-fixtures`, `--skip-e2e`, `--webgpu`, `--pw-project <p>`
 (repeatable), `-- <extra playwright args>`. `cargo xtask` always runs
 Cargo with `--locked`.
- **Fast Node-only sanity:** `pnpm run test:contracts` (sets
 `AERO_REQUIRE_WEBGPU=0` unless overridden).
- **Minimal Rust sanity:** `cargo test -p aero-platform --locked`,
 `cargo check -p aero-machine --locked`,
 `cargo check -p aero-wasm --target wasm32-unknown-unknown --locked`.
- **Focused suites:** `cargo xtask input` (USB/input: `--rust-only`,
 `--usb-all`, `--machine`, `--with-wasm`, `--e2e`, `--node-dir`);
 `bash ./scripts/ci/run-vga-vbe-tests.sh` (boot-display regression).
- Directory resolution shared with CI: `node scripts/env/resolve.mjs`,
 `node scripts/ci/detect-node-dir.mjs`; wasm-crate detection prefers
 `crates/aero-wasm` (`AERO_WASM_CRATE_DIR` / `--wasm-crate-dir` override).

Wrap anything non-trivial in `bash ./scripts/safe-run.sh` (mandatory; see
§Resource limits).

## Make the weird normal

Normative home of the testing stance from
`wiki/meta/engineering-principles.md` (Testing policy). Apply it whenever you
add or extend a suite.

**If it does not hurt, it does not count.** A test that was never in danger of
failing is a comment with a runtime cost. Coverage is adversarial by default:
hunt the cases that hurt, and keep going after the first few stop hurting.

The valuable cases live in *interactions*, not in a single dimension:

- a boundary during a retry
- under a concurrent access change
- across a crash or snapshot restore
- while a cursor / `last_hit` / TLB / probe flag is stale

The state space is hyper-dimensional. Defects sit on the edges between
dimensions. Walk that edge until a genuinely novel weird case is hard to
invent.

This never licenses volume. A hundred variations of one covered path is
padding. Every case must:

1. **Name the defect** it catches (in the test name or a one-line comment).
2. **Reach a state, ordering, or interaction** the others do not.

Adversarial depth, never test count. When the suite is green, do not stop —
widen the oracle (new input category, different lens, an end-to-end pass),
don't clone the path that already passed.

## Test layers

### Rust unit / integration

`cargo test --locked --workspace` (single crate: `-p <name>`; name filter as
trailing arg; `-- --nocapture`, `-- --ignored`). Canonical shapes:

- CPU instruction tests (`crates/aero-cpu-core/tests/`): load bytes into a
 `FlatTestBus`, drive `exec::Vcpu` + `exec::Tier0Interpreter`, assert on
 `CpuState` and exits. Paging/TLB/fault delivery end-to-end:
 `crates/aero-cpu-core/tests/paging.rs`; walker internals:
 `crates/aero-mmu/src/lib.rs`.
- Device models: PIC/PIT/i8042-style register-semantics tests, ACPI PM
 (PM1 write-1-to-clear, S5 → power-off callback), snapshot round-trips
 (every save/restore-capable device must produce deterministic bytes and
 restore equivalent observable state).
- USB stack: `crates/aero-usb` (UHCI/EHCI/xHCI + passthrough mapping), plus
 fixture-vector tests consuming `protocol-vectors/*.json` (see §Fixtures).
- SMP/APIC: `crates/aero-smp` + `crates/aero-smp/tests/` (INIT/SIPI/fixed
 IPI delivery, AP bring-up against the deterministic model).
- TypeScript-side USB ring fast path: `apps/web/src/usb/*test.ts`, incl.
 `apps/web/src/usb/usb_proxy_ring_integration.test.ts`.

### WASM tests (`wasm-bindgen-test`)

Run from the single WASM crate (`crates/aero-wasm`): `wasm-pack test --node`.
Pitfalls: `wasm-pack` operates on one crate; `wasm32-unknown-unknown` target
must be installed; threaded builds need the pinned nightly +
`-Z build-std` + `rust-src` and the internal `wasm-threaded` feature;
`--node` has no DOM — browser-API behavior belongs in Playwright, not here.

### Instruction conformance / differential

- `crates/conformance`: randomized corpus compared against native x86_64
 host execution (unix x86_64 only; prints "skipped" elsewhere). Run:
 `cargo xtask conformance --cases 512`. Tuning env:
 `AERO_CONFORMANCE_CASES` (default 512), `AERO_CONFORMANCE_SEED` (decimal or
 `0x` hex), `AERO_CONFORMANCE_FILTER`, `AERO_CONFORMANCE_REFERENCE=qemu`
 (needs `--features qemu-reference` + a `qemu-system-*` binary),
 `AERO_CONFORMANCE_REPORT_PATH`, `AERO_CONFORMANCE_REFERENCE_ISOLATE=0`.
- `tools/qemu_diff` (crate `qemu_diff`): 16-bit synthetic snippets vs an
 external QEMU binary — `cargo test --locked -p aero-cpu-core --features qemu-diff`
 (skips without QEMU); the always-on half is the Tier-0 batch vs
 single-step equivalence suite in the same crate.

### TypeScript unit tests

`pnpm run test:unit` = repo-root typecheck + Node `node --test` suites
(`tests/*.test.js` and friends) + protocol tests + per-workspace tests
(`services/gateway`, `bench`, `tools/perf`, `tools/range-harness`,
`packages/aero-stats`, `services/image-gateway`, `net-proxy`,
`tools/net-proxy-server`, `web` node tests) + Vitest (`apps/web/src/**/*.test.ts`
colocated, `apps/web/test/**/*.vitest.ts`; config in `apps/web/vite.config.ts`).
Coverage via `pnpm run test:unit -- --coverage`.

> Debt: the workspace list and `test:unit` still name the purged `server/`
> workspace [unverified until purge lands fully].

### Playwright E2E

Specs: `tests/e2e/**/*.spec.ts`; config `playwright.config.ts`. Run:
`pnpm run test:e2e` (`-- --project=chromium|firefox|webkit`, `-- --ui`,
`-- --update-snapshots`, `PWDEBUG=1`). The config starts three servers:
the dev harness (auto-detected port from 5173), a COI preview server (from
4173), and a CSP variant server (from 4180); override via
`AERO_PLAYWRIGHT_DEV_PORT/_ORIGIN`, `AERO_PLAYWRIGHT_PREVIEW_PORT/_ORIGIN`,
`AERO_PLAYWRIGHT_CSP_PORT/_ORIGIN`; `AERO_PLAYWRIGHT_REUSE_SERVER=1` reuses
running servers locally; `AERO_PLAYWRIGHT_EXPOSE_GC=1` passes `--expose-gc`.

- **WebGPU gating:** WebGPU-required specs are tagged `@webgpu` and isolated
 in the `chromium-webgpu` project (extra Chromium flags:
 `--enable-unsafe-webgpu`, `--enable-features=WebGPU`,
 `--ignore-gpu-blocklist`, `--use-angle=swiftshader`, `--disable-gpu-sandbox`).
 Default projects exclude them. `AERO_REQUIRE_WEBGPU=1` turns skips into
 hard failures (`pnpm run test:webgpu`). GPU timestamp-query telemetry is
 always optional: when unsupported, `gpu_time_ms` must be `null`, never a
 failure.
- **Visual regression:** Playwright screenshot assertions under
 `tests/e2e/__screenshots__/` (per-project, per-OS names via
 `snapshotPathTemplate`). Rules: fixed viewport/`deviceScaleFactor`,
 reduced motion + injected CSS, declared fonts; synthetic scenes/UI only —
 **never** Windows or other copyrighted imagery. Update baselines:
 `pnpm run test:e2e:update`; review: `pnpm run test:e2e:report`.
- **CSP/COOP/COEP regression:** `tests/e2e/csp-fallback.spec.ts` (PoC app
 `apps/web/public/wasm-jit-csp/`; its old local server was removed with the
 `server/` purge) and `tests/e2e/security-headers.spec.ts`
 (`pnpm run test:security-headers`).
- **GPU presenter validation:** `tests/e2e/web/gpu_color.spec.ts` +
 `apps/web/src/gpu/validation-scene.ts` (test card hashing the *presented*
 output — distinct from the GPU worker `screenshot` source-framebuffer
 readback, `apps/web/src/gpu/presenter.ts`); `apps/web/src/pages/gpu_smoke.html` +
 `apps/web/src/workers/gpu_smoke.worker.js` +
 `tests/e2e/playwright/gpu_smoke.spec.ts` (present pattern → readback →
 SHA-256; WebGPU optional, forced-WebGL2 run mandatory).
- **GPU golden images:** `tests/e2e/playwright/gpu_golden.spec.ts` +
 `tests/e2e/playwright/utils/image_diff.ts` compare deterministic
 microtests (VGA text, VBE LFB bars, backend smoke, trace replay) against
 committed PNGs in `tests/golden/`. Regenerate:
 `pnpm run generate:goldens` (pure CPU, `tests/golden/generate_goldens.cjs`);
 CI-equivalent drift check: `pnpm run check:goldens`
 (`scripts/ci/check-goldens.mjs`); run: `pnpm run test:gpu`
 (`playwright.gpu.config.ts`). Failures write expected/actual/diff
 artifacts.

### QEMU boot tests

Guest-media boot flows (boot sector, FreeDOS, Win7) run **headlessly under
QEMU** — not under Aero's own CPU — registered as `[[test]]` targets in
`crates/aero-boot-tests/Cargo.toml` pointing at `tests/boot_sector.rs`,
`tests/freedos_boot.rs`, `tests/windows7_boot.rs`, `tests/boot/basic_boot.rs`
(harness `tests/harness/mod.rs`). Run them via
`cargo test -p aero-boot-tests --test <name> --locked`, **not** via the
workspace-root package. Prerequisites: `qemu-system-i386`, `mtools`,
`curl`/`unzip`; FreeDOS image via `bash ./scripts/prepare-freedos.sh`;
Windows via `bash ./scripts/prepare-windows7.sh` +
`-- --ignored`. Env: `AERO_QEMU`, `AERO_ARTIFACT_DIR`
(default `target/aero-test-artifacts`), `AERO_REQUIRE_TEST_IMAGES=1`
(missing fixtures become hard errors), `AERO_WINDOWS7_IMAGE`,
`AERO_WINDOWS7_GOLDEN`.

### Driver / guest-side validation

- Win7 virtio PCI-capability parser: portable C99 unit tests, hardware-free:
 `bash ./drivers/win7/virtio/tests/build_and_run.sh` (`CC=clang` optional).
- AeroGPU guest-side suite: `drivers/aerogpu/tests/win7/` — D3D9Ex/D3D11
 programs printing `PASS:`/`FAIL:`, run inside a Win7 guest with the driver
 installed; not part of default CI.
- virtio guest-driver checklist: `drivers/README.md`.

### The harness memory allocator handed workers a memory the module never used

Five IO-worker specs — the input-capture pair, the malformed-batch and drop-counter
specs, and the audio ring telemetry one — failed together with:

```
WASM memory wiring probe failed (wasm -> JS). mem_store_u32(offset=0x0000ffc0)
wrote 0x11223344 but JS read 0x00000000 from the provided WebAssembly.Memory.buffer.
memory=65536 bytes (shared)
```

`allocateHarnessSharedMemorySegments` existed to keep fixture pages cheap, and it
did that by allocating exactly the guest RAM asked for — 64KiB, one wasm page —
and declaring `guest_base = 0` with `runtime_reserved = 0`.

That layout cannot be right for this module. It is linked `--import-memory
--stack-first`, so the bottom of linear memory is its own stack and static data
and guest RAM begins above them at `guest_base`; the memory import declares a
21-page minimum. A one-page memory cannot satisfy it, so the glue's
`memory || new WebAssembly.Memory(...)` fallback quietly allocated its own. The
module then ran correctly in a memory the worker had no handle on, and the worker
read a buffer nothing ever wrote to. The probe is what turned that into a visible
failure rather than silent guest-RAM corruption.

The allocator now calls `computeGuestRamLayout` — the same function the
coordinator uses — and publishes the `guest_base`, `guest_size` and
`runtime_reserved` it returns. Fixtures stay cheap, because the layout still
honours a small requested guest size; they just also get the region the module
requires underneath it.

## Vitest was collecting another runner's tests

`vitest run` reported 83 failing files, all of them `No test suite found in file`.
Every one lives under `services/`, and every test under `services/` is written
against `node:test` and run by that service's own `test` script over its build
output. Vitest's `include` had `services/**/test/**/*.test.ts`, which collects
those files and then finds nothing in them, because `node:test` registrations are
invisible to it.

Dropping `services/**` from `include` (and from the coverage `include`, which
would otherwise report the directory as uncovered) takes the suite to 362 files
and 2,140 tests, all passing. The services keep their own coverage:
`services/net-proxy` alone runs 168 tests under `node --test`.

## Benchmarks

Criterion microbenches under `crates/*/benches/`; current set in
`crates/aero-cpu-core/benches/`: `emulator_critical` (legacy dispatch loop;
requires `--features legacy-interp`), `jit_bookkeeping`,
`jit_cache_bookkeeping`, `paging_bus_bulk`, `tier0_*`. Run:
`cargo bench --locked -p aero-cpu-core --bench <name> [--features legacy-interp] -- --noplot`;
`AERO_BENCH_PROFILE=ci` shortens runs. Compare runs:
`python3 scripts/bench_compare.py --base ... --new ... --thresholds-file bench/perf_thresholds.json --profile pr-smoke`
(uses Criterion 95% CIs; fails only on above-threshold *and* significant
regressions). `scripts/compare-benchmarks.sh` knobs:
`AERO_BENCH_COMPARE_PROFILE`, `AERO_BENCH_THRESHOLDS_FILE`,
`AERO_BENCH_COMPARE_{MARKDOWN,JSON}_OUT`.

### Disk-streaming endpoint conformance

Validate any disk-delivery endpoint against the browser client's
expectations (Range or chunked mode) with the dependency-free tool:
`python3 tools/disk-streaming-conformance/conformance.py --base-url ...`
(`--mode chunked --manifest-url ...`, `--strict`), plus the four
`selftest_*` servers in the same directory.

### Manual checklists & test plans

- Manual Win7 in-box HDA driver smoke (from `audio.md`,
 applies to `vmRuntime=legacy`): verify Device Manager shows "High
 Definition Audio Controller" + "High Definition Audio Device" on the
 Microsoft in-box stack (`hdaudbus.sys`/`hdaudio.sys`), play a system
 sound, confirm the host ring's `writeFrameIndex` advances with
 `overrunCount == 0` and stable `underrunCount` (≤128 startup frames
 tolerable), and exercise mic capture (browser `getUserMedia` → guest
 recording endpoint). Key failure splits: producer dead (`writeFrameIndex`
 stuck) vs consumer dead (`readFrameIndex` stuck / `AudioContext`
 suspended); QA bundle export covers metrics, codec state, WAV snapshots.
 Note: the doc's `/state/win7.iso` reference is stale — the dev-box ISO
 lives at `/root/aero-images/` (see root `AGENTS.md`).
- End-to-end subsystem test plans, now in the area pages — "single
 document" plans (device model ↔ guest driver ↔ web runtime). Only entry
 today is the virtio-input test plan, absorbed into
 `wiki/areas/usb-and-input.md`.

### Aspirational suites (not yet real)

Marked [aspirational] so nobody reads them as existing gates: JIT-vs-
interpreter memory differential/property tests; instruction-coverage
percentage gates; full app-compat E2E (notepad/calc/IE on a booted guest);
D3D9Ex/D3D10-11 conformance scenes; the "~10k unit / ~500 integration /
~50 E2E" pyramid numbers; guest CPU throughput bench suite
(`wiki/areas/cpu-and-jit.md`).

## Fixtures & test-asset policy

Normative policy (enforced by `scripts/ci/check-repo-policy.sh`):

- **Never commit** OS media, BIOS/firmware dumps, or Windows binaries:
 `.iso .img .vhd .vhdx .vmdk .qcow .qcow2 .wim .exe .dll`, or anything
 under `*/test_images/windows*` / `*/fixtures/windows*`. The repo-policy
 script is also what root `AGENTS.md` cites for this ban.
- **Allowlisted tiny deterministic fixtures** (each regenerable, CI-checked,
 ≤ 1 MiB): `assets/bios.bin` (`cargo xtask bios-rom [--check]`),
 `crates/firmware/acpi/dsdt{,_pcie}.aml` (`cargo xtask fixtures`; ASL
 sources alongside, `scripts/verify_dsdt.sh`), `tests/fixtures/boot/*`
 (boot sectors + tiny disk images), `tests/fixtures/bootsector.bin`,
 `tests/fixtures/realmode_vbe_test.bin`, `tests/fixtures/boot/int_sanity.bin`,
 `tools/qemu_diff/boot/boot.bin` (all from `cargo xtask fixtures`, with
 `.asm`/`.s` sources kept as documentation). Packaging-test placeholders
 under `tools/packaging/aero_packager/testdata/drivers/**` (`test.sys`,
 `test.dll`, `WdfCoInstaller*.dll`) are text stubs with hashes pinned in
 `check-repo-policy.sh`.
- `cargo xtask fixtures --check` fails if generated fixtures drift;
 regenerate with `cargo xtask fixtures`. It is the first step of
 `cargo xtask test-all`.
- General size ceiling: 20 MB per blob; prefer runtime generation or
 download-at-setup. New in-repo fixtures need provenance (source +
 license), smallness, determinism, and a README with a SHA-256.
- `tests/golden/` PNGs are synthetic and must match
 `pnpm run generate:goldens` output (see GPU golden tests).
- Local-only assets (downloaded OSS images, user Windows media):
 gitignored `test-images/` (`AERO_WINDOWS7_IMAGE` defaults to
 `test-images/local/windows7.img`). **Debt:** `test-images/` is deleted in
 the current working tree by the in-progress purge [unverified whether
 intentional].
- **Load-bearing vectors:** `protocol-vectors/*.json` (HID keyboard/consumer
 usage tables, gamepad report + clamping vectors, WebUSB passthrough wire
 vectors) are consumed by Rust tests via `include_str!`/manifest-relative
 paths in `crates/aero-usb/tests/`, `crates/aero-usb/src/hid/`,
 `crates/aero-usb/src/passthrough.rs`, and
 `crates/aero-devices-input/tests/hid_ps2_cross_fixture_consistency.rs`.
 They are test data, not docs — relocated to `protocol-vectors/` ahead of
 the `docs/` purge.

## Canonical environment variables (build/test/dev)

From `build-and-tooling.md`. Precedence: CLI flags > canonical `AERO_*` > legacy
aliases (one compatibility cycle, warn on use) > auto-detected default.
Normalize/inspect with `node scripts/env/resolve.mjs --format json`.

**Repo layout / toolchain**

| Variable | Default | Consumed by |
|---|---|---|
| `AERO_NODE_DIR` | auto: `.` then `frontend/` then `apps/web/` | xtask, CI, justfile |
| `AERO_WASM_CRATE_DIR` | auto (`crates/aero-wasm`) | xtask, CI |
| `AERO_WASM_PACKAGES` | unset (all) | `apps/web/scripts/build_wasm.mjs` |
| `AERO_ALLOW_UNSUPPORTED_NODE`, `AERO_CHECK_NODE_QUIET`, `AERO_NODE_VERSION_OVERRIDE` | unset | `scripts/check-node-version.mjs` |

**Test gates & selectors**

| Variable | Default | Effect |
|---|---|---|
| `AERO_REQUIRE_WEBGPU` | `0` | `@webgpu` tests fail instead of skip |
| `AERO_DISABLE_WGPU_TEXTURE_COMPRESSION` | `0` | Never request BC/ETC2/ASTC features |
| `AERO_REQUIRE_TEST_IMAGES` | unset | Missing boot-test fixtures are hard errors |
| `AERO_QEMU` | auto-detect | QEMU binary for boot tests |
| `AERO_ARTIFACT_DIR` | `target/aero-test-artifacts` | Failing-test screenshots/diffs |
| `AERO_WINDOWS7_IMAGE` / `AERO_WINDOWS7_GOLDEN` | `test-images/local/...` | Gated Win7 boot test inputs |
| `AERO_WIN7_ISO` / `AERO_WIN7_DRIVERS` / `AERO_WIN7_SLIPSTREAM_IMAGE` | unset / `aero/win7-slipstream` | `tools/win7-slipstream` integration test + container scripts |
| `AERO_UPDATE_TRACE_FIXTURES` | unset | Regenerate GPU trace fixtures instead of asserting |
| `AERO_CONFORMANCE_*` (CASES/SEED/FILTER/REFERENCE/REPORT_PATH/REFERENCE_ISOLATE) | see §conformance | `crates/conformance` |
| `AERO_PLAYWRIGHT_*` (REUSE_SERVER/EXPOSE_GC/DEV_*/PREVIEW_*/CSP_*) | auto ports 5173/4173/4180 | Playwright configs |
| `VITE_DISABLE_COOP_COEP` | `0` | Drop COOP/COEP headers on dev/preview (fallback testing) |
| `AERO_MAKE_FAT_IMAGE`, `AERO_DRIVER_PFX_BASE64`, `AERO_DRIVER_PFX_PASSWORD` | unset | Windows driver packaging/signing (`drivers/build/*.ps1`) |

**Resource limits (agent sandboxes)** — honored by `scripts/safe-run.sh` and
`scripts/agent-env.sh`:

| Variable | Default | Effect |
|---|---|---|
| `AERO_TIMEOUT` | 600 (Playwright fallback `AERO_PLAYWRIGHT_TIMEOUT` 1800) | Wall-clock timeout (with SIGKILL grace) |
| `AERO_MEM_LIMIT` | 12G (WASM-heavy Node fallback `AERO_NODE_TEST_MEM_LIMIT` 256G; Playwright fallback `AERO_PLAYWRIGHT_MEM_LIMIT` 256G) | RLIMIT_AS ceiling |
| `AERO_CARGO_BUILD_JOBS` | 1 | Cargo parallelism (raise if your sandbox allows) |
| `RUSTC_WORKER_THREADS`, `RAYON_NUM_THREADS`, `RUST_TEST_THREADS`, `NEXTEST_TEST_THREADS`, `AERO_TOKIO_WORKER_THREADS` | = `CARGO_BUILD_JOBS` | Keep rustc/Rayon/libtest/nextest/Tokio pools aligned under thread limits |
| `AERO_RUST_CODEGEN_UNITS` (alias `AERO_CODEGEN_UNITS`) | unset | Add `-C codegen-units=<n>` |
| `AERO_SAFE_RUN_RUSTC_RETRIES` | 3 | Retry transient EAGAIN/WouldBlock rustc failures (`1` disables) |
| `AERO_ISOLATE_CARGO_HOME` | unset | `1` → repo-local `./.cargo-home`; any path → custom `CARGO_HOME` (dodges registry lock contention) |
| `AERO_DISABLE_RUSTC_WRAPPER` | unset | Force-disable rustc wrappers (sccache env wrappers are cleared by default regardless) |
| `AERO_BENCH_*` (COMPARE_PROFILE/THRESHOLDS_FILE/COMPARE_MARKDOWN_OUT/COMPARE_JSON_OUT) | see §benchmarks | `scripts/compare-benchmarks.sh` |

### L2 proxy knobs (part of the agent defaults contract)

| Variable | Default | Effect |
|---|---|---|
| `AERO_L2_PROXY_LISTEN_ADDR` | unset | Listen address for the L2 proxy (e.g. `0.0.0.0:8090`; see `crates/aero-l2-proxy/Dockerfile`) |
| `AERO_L2_AUTH_MODE` | unset | Auth mode: `none`, `token`, `session`, `session_or_token`, `session_and_token`, `jwt`, `cookie_or_jwt` (aliases: `cookie`, `api_key`, `cookie_or_api_key`, `cookie_and_api_key`); see `crates/aero-l2-proxy/src/config.rs` |
| `AERO_L2_OPEN` | unset | Security escape hatch: `1` explicitly allows unauthenticated access (also requires `AERO_L2_INSECURE_ALLOW_NO_AUTH=1`; parsed strictly) |

Deprecated aliases (warn-and-accept): `AERO_WEB_DIR`, `WEB_DIR` →
`AERO_NODE_DIR`; `AERO_WASM_DIR`, `WASM_CRATE_DIR` → `AERO_WASM_CRATE_DIR`.

## Tests that depend on their environment

Two categories of test in this tree fail for reasons that have nothing to do
with the code under test. Both look like defects on a first read, so they are
worth recognising.

**Tests that need address space.** `aero-machine`'s hole-aware high-memory tests
construct multi-gigabyte guest physical maps. Under `safe-run.sh`'s default
12 GiB `RLIMIT_AS` they abort with a bare `memory allocation of N bytes failed`,
which reads like a leak and is not one. Run the workspace suite with
`AERO_MEM_LIMIT=48G`.

**Tests that reach outside the process for their verdict.** The UDP
send-failure metric tests in `aero-l2-proxy` (`tests/udp_send_fail_metrics.rs`)
are the current example, and they fail — the only two failures in the Rust
suite. The failure is worth describing precisely, because the obvious diagnosis
is wrong.

The tests forward a guest datagram to a closed local port so that the resulting
ICMP port-unreachable surfaces as `ECONNREFUSED` on a later send, then wait for
`l2_udp_send_fail_total` to increment. That looks like a dependency on host ICMP
behaviour, and the first guess is that the sandbox suppresses it. It does not: a
connected UDP socket to `127.0.0.1:1` reports `ECONNREFUSED` on the second send
here, exactly as the design assumes.

Reading the proxy's own metrics during the run gives the real answer. Frames
arrive (`l2_frames_rx_total` is non-zero) and nothing is refused
(`l2_policy_denied_total` is zero), but `l2_udp_flows_active` never leaves zero
— so **no UDP flow is ever opened**, and the send path the metric belongs to is
never reached. The datagrams the test synthesises are not being turned into a
send action by the network stack. That is a defect in the test's packet
construction or in the stack's handling of it, not in the metric plumbing the
test is named for, and not in the environment.

Two lessons, both cheap to reuse:

- A test that reaches outside the process is only as reliable as the thing it
  reached for — but "the environment did it" is a hypothesis, not a conclusion.
  Check it before believing it.
- When an assertion times out waiting for a counter, read *all* the counters.
  The one that stayed at zero upstream of the one you were watching is the
  answer.

## Resource limits & concurrency doctrine (shared/agent hosts)

Mandatory for all non-trivial builds/tests (root `AGENTS.md`):

- Always run through `bash ./scripts/safe-run.sh <cmd>` (timeout + RLIMIT_AS
 + retry-on-WouldBlock + lld thread caps). Components:
 `scripts/run_limited.sh` (RLIMIT_AS), `scripts/with-timeout.sh` (timeout;
 always with a `-k` SIGKILL grace — bare `timeout` lets processes ignore
 SIGTERM forever), `scripts/agent-env.sh` (sourced env defaults:
 `CARGO_BUILD_JOBS=1`, `NODE_OPTIONS=--max-old-space-size=4096`,
 `PW_TEST_WORKERS=1`, …), `scripts/agent-env-setup.sh` (one-time sanity).
- **Memory is the constraint**, not CPU/disk: target ~6–8 GB typical, 12 GB
 ceiling per agent. RLIMIT_AS caps *virtual* space; Node/V8/WASM reserve
 huge virtual ranges — that is why safe-run auto-bumps to 256G for
 Node/WASM/Playwright entrypoints, and why `WebAssembly.Memory(): could
 not allocate memory` under a 12G cap means "raise `AERO_MEM_LIMIT`", not
 real OOM.
- Transient shared-host failures (`Resource temporarily unavailable`,
 `failed to spawn helper thread (WouldBlock)`, ctrlc-handler panics) are
 environment limits, not code bugs: back off and re-run via safe-run;
 lower parallelism/codegen-units if persistent.
- Linker gotcha: cap lld threads via per-target
 `CARGO_TARGET_<TRIPLE>_RUSTFLAGS`; for wasm32 use `-C link-arg=--threads=<n>`
 (no `-Wl,` — rust-lld rejects it). Never set `-Wl,--threads` via global
 `RUSTFLAGS`; it breaks wasm builds.
- Cargo lock contention: "Blocking waiting for file lock on package cache"
 = another agent's cargo on the shared registry (wait, or
 `AERO_ISOLATE_CARGO_HOME=1`); "on build directory" = another cargo in this
 checkout (wait, or per-command `CARGO_TARGET_DIR`).
- Exit code 0 ≠ success: verify artifacts exist (`target/`, `pkg/*.wasm`,
 non-empty `dist/`), watch for silent skips and flaky-after-retry passes.
- GPU-less/headless hosts: everything except WebGPU works (WebGL2 via
 llvmpipe/SwiftShader, slow); WebGPU tests skip by default —
 `AERO_REQUIRE_WEBGPU=1` will fail there by design.

## CI mapping (historical — purge in progress)

The `.github/` tree (workflows `ci.yml`, `e2e-matrix.yml`, `webgpu.yml`,
`bench.yml`, `conformance.yml`, `codeql.yml`, plus the
`setup-rust`/`setup-node-workspace`/`setup-playwright` composite actions) is
deleted in the current working tree by the CI/deploy purge (2026-07-23) and
is **historical until CI is re-established**. The design that was in force:

- PR CI: Chromium-only Playwright, `AERO_REQUIRE_WEBGPU=0`; scheduled/manual
 cross-browser matrix (Chromium+Firefox+WebKit) opened a tracking issue on
 failure; scheduled WebGPU lane with `AERO_REQUIRE_WEBGPU=1`; bench lane
 compared PR base vs head through `scripts/bench_compare.py`; conformance
 lane ran a fast subset on PRs and a larger corpus on schedule.
- Local commands above were chosen to match CI one-for-one; that property
 should be preserved when CI returns.

## Pointers

- Fixture generators: `xtask/src/` (`cmd_fixtures`, `cmd_bios_rom`,
 `cmd_test_all`, `cmd_conformance`, `cmd_input`).
- Policy enforcement: `scripts/ci/check-repo-policy.sh`,
 `scripts/ci/check-goldens.mjs`.
- Web-host contracts tested here: [web-host.md](./web-host.md); repo state:
 `wiki/state/repo-state-and-structure.md` §3.

## Decoder differential fuzzing — proposed, not built

The instruction *decoder* has a correctness surface that execution differential
testing does not reach: the existing conformance work compares what happens when
an instruction runs, which says nothing about the encodings we decode wrongly or
refuse to decode at all. The technique for that is differential fuzzing of the
decoder against another implementation — the
[mishegos and sandsifter](https://blog.trailofbits.com/2019/10/31/destroying-x86_64-instruction-decoders-with-differential-fuzzing/)
approach — and the structural idea worth stealing from v86 is a *single
instruction table* that generates both the decoder and its tests, so the two
cannot disagree.

This is **not built**. Given how many bring-up defects turned out to be
"the decoder reports a mnemonic we never routed", it is better-motivated now than
when it was first proposed. The 44 existing fuzz targets cover other surfaces.

## The boot ladder as a metric

A suite can be entirely green while the headline goal does not work — for a long
stretch of the Windows 7 bring-up, thousands of Rust tests and browser
end-to-end tests passed while the guest did not boot. Tests that pass regardless
of whether the project's central goal is met cannot report on it, so the ladder
exists to give that goal a number.

The ladder runs the real install media and reports the **furthest milestone
reached**. `scripts/win7.sh ladder [run_dir]` reports it from a run directory.
Run on every change, it converts "stuck" from a binary that can read "no" for a
week into a monotone metric — something to defend against regressions and watch
move. The tooling is described in
[debugging.md](./debugging.md#the-bring-up-loop-scripts).

What makes the ladder trustworthy is that every rung is a **signal the guest can
only produce by having got that far**, greppable from the run log without reading
pixels or exercising judgement:

| Rung | Signal in `run.log` | Why only that milestone produces it |
|---|---|---|
| 1 POST | `inst=` non-zero | The guest retired instructions at all |
| 2 Protected mode | `mode=Protected` | The firmware got past real mode |
| 3 VBE mode programmed | `display=1024x768` | Something drove the VBE registers deliberately |
| 4 Long mode | `mode=Long` | winload made the 64-bit transition |
| 5 Framebuffer content | `non_zero=` five digits or more | Real pixels, not an incidentally dirty buffer |
| 6 Kernel entered | `rip=0xfffff…` | A high-half `rip` can only be the kernel — winload runs from low addresses, so this *is* the `OslArchTransferToKernel` handoff |
| 7 Kernel raised IRQL | `cr8` non-zero | `cr8` tracks IRQL, which nothing but the kernel raises |
| 8 Guest serial output | `serial.log` non-empty | The kernel debugger is talking |

Rungs 6 and 7 are the pattern worth copying: rather than trying to recognise a
screen or match a string the guest might never print, they key on architectural
state that is *unreachable* before the milestone. A signature like that cannot
report a false pass.

The ladder also prints instruction depth and rate, because a frozen framebuffer
says nothing about whether the CPU is still retiring work — the difference
between a slow boot and a hang is forward progress, not pixels.

### The ground-truth reference

The same install media boots to the Windows "Install Windows" GUI in about **75
seconds** under QEMU with KVM (QEMU 10.2.1, 4096 MB). The in-tree reference
script `scripts/win7-install.sh` uses 4 vCPU by default; the 75-second figure
itself has no committed artifact and is **[unverified]** as stated. That is the
end-to-end reference the ladder is measured against: it establishes that the
media and the configuration are sound, so any failure the ladder reports is
Aero's. Keeping a known-good reference next to the metric is what makes a bad
result diagnostic instead of ambiguous.

## QEMU↔Aero divergence harness (2026-07-26)

See `wiki/notes/bring-up-findings-divergence-and-sight-audits.md` §2 for details.

A self-contained QEMU TCG plugin + diff tooling for forward first-divergence
debugging. The method: boot the same Win7 ISO under both QEMU (ground truth) and
Aero, capture per-instruction execution RIP streams, align at the El Torito boot
sector entry (`0x7c00`), and diff. The first RIP where the two part company is
the root cause — regardless of where the eventual crash surfaces.

### Tools (`tools/qemu-trace-plugin/`)

| Tool | Purpose |
|---|---|
| `aero_trace.so` | QEMU TCG plugin: per-instruction RIP + RAM-write streams |
| `aero_diff.py` | Instruction-level first-divergence diff (resync-aware) |
| `aero_execlog_diff.py` | QEMU `-d exec` fast-capture diff (TB-level, 30× faster) |
| `aero_ram_diff.py` | Value-level RAM snapshot diff |
| `aero_diverge.sh` | One-command wrapper: QEMU-capture → Aero-capture → diff |

### One-command usage

```bash
tools/qemu-trace-plugin/aero_diverge.sh --max-insts 12000000
```

Validated end-to-end: Aero's 12M-instruction boot matched QEMU exactly (0 real
divergences; 24 BIOS-service-call gaps correctly resynced).

## Testing Strategy & Validation

### Overview

Comprehensive testing is critical for an emulator. We must verify correctness at the instruction level, system level, and application level.

### Practical guide (running tests locally)

This document describes *what* we test and *why*. For the practical, developer-facing guide to running the full test stack locally (Rust, WASM, TypeScript, Playwright), plus common issues like COOP/COEP and WebGPU gating, see:

the practical sections later in this page.

Deterministic manual smoke-test procedures, for the cases automation cannot
cover, and the single-document end-to-end subsystem plans (device model through
guest driver to web runtime) were both absorbed into the area pages themselves —
for example the Windows 7 audio checklist in [audio.md](audio.md) and the
virtio-input plan in [usb-and-input.md](usb-and-input.md).

---

### Testing Pyramid

```
┌─────────────────────────────────────────────────────────────────┐
│ Testing Pyramid │
├─────────────────────────────────────────────────────────────────┤
│ │
│ ┌───────┐ │
│ / \ │
│ / End-to-End\ │
│ / (E2E) \ │
│ / ~50 tests \ │
│ ───────────────────── │
│ / \ │
│ / Integration \ │
│ / ~500 tests \ │
│ ─────────────────────────────── │
│ / \ │
│ / Unit Tests \ │
│ / ~10,000 tests \ │
│ ───────────────────────────────────── │
│ │
└─────────────────────────────────────────────────────────────────┘
```

---

### Unit Tests

#### CPU Instruction Tests

The canonical interpreter is Tier-0 (`aero_cpu_core::interp::tier0`). Instruction-level unit tests
live under `crates/aero-cpu-core/tests/` and generally follow the same shape:

- allocate a small `FlatTestBus` (linear-address bus)
- load a short instruction sequence into guest memory
- drive execution with `exec::Vcpu` + `exec::Tier0Interpreter`
- assert on architectural state (`CpuState` registers/flags) and any exits (`Exception`, assists)

```rust
use aero_cpu_core::exec::{Interpreter as _, Tier0Interpreter, Vcpu};
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::state::{CpuMode, RFLAGS_CF, RFLAGS_ZF};
use aero_x86::Register;

#[test]
fn mov_and_add_update_registers_and_flags() {
 let mut bus = FlatTestBus::new(0x2000);
 let code_base = 0x100u64;

 // MOV RAX, RBX; ADD RAX, 1; HLT
 bus.load(
 code_base,
 &[
 0x48, 0x89, 0xD8, // mov rax, rbx
 0x48, 0x83, 0xC0, 0x01, // add rax, 1
 0xF4, // hlt
 ],
 );

 let mut vcpu = Vcpu::new_with_mode(CpuMode::Long, bus);
 vcpu.cpu.state.set_rip(code_base);
 vcpu.cpu.state.write_reg(Register::RBX, 0xFFFF_FFFF_FFFF_FFFF);

 let mut interp = Tier0Interpreter::new(1024);
 while !vcpu.cpu.state.halted {
 interp.exec_block(&mut vcpu);
 }

 assert_eq!(vcpu.cpu.state.read_reg(Register::RAX), 0);
 assert!(vcpu.cpu.state.get_flag(RFLAGS_CF));
 assert!(vcpu.cpu.state.get_flag(RFLAGS_ZF));
}
```

For patterns around faults/exceptions (turning an `Exception` into a pending event and delivering it
through `CpuCore`), see [`cpu-and-jit.md`](cpu-and-jit.md).

#### Memory Subsystem Tests

Paging and TLB behavior is implemented in `crates/aero-mmu` and integrated into the CPU core via
`aero_cpu_core::PagingBus` (a `CpuBus` wrapper that performs translation and routes accesses to a
physical `aero_mmu::MemoryBus`).

Concrete, end-to-end paging tests (Tier-0 + paging + INVLPG/CR3/CR4/EFER interaction + fault
delivery) live under:

- [`crates/aero-cpu-core/tests/paging.rs`](../../crates/aero-cpu-core/tests/paging.rs)

And the core page table walker / TLB unit tests live under:

- [`crates/aero-mmu/src/lib.rs`](../../crates/aero-mmu/src/lib.rs) (implementation + internal tests)

#### JIT vs Interpreter Memory Differential Tests

Memory is where subtle correctness bugs hide (TLB invalidation, permission checks, MMIO routing, cross-page accesses). For the baseline JIT memory fast path we need **differential tests** that run the same guest program in:

1. interpreter (reference)
2. baseline JIT (candidate)

…and compare architectural state after execution.

```rust
#[test]
fn jit_memory_loop_matches_interpreter() {
 // Program: tight RAM loop stressing loads/stores.
 // - sequential accesses (TLB miss once per page)
 // - random accesses (TLB pressure)
 // - mixed sizes (1/2/4/8/16)
 let program = assemble("
 mov rsi, 0x100000 ; base
 mov rcx, 1000000
 loop:
 mov rax, [rsi]
 add rax, 1
 mov [rsi], rax
 add rsi, 8
 dec rcx
 jnz loop
 hlt
 ");

 let interp = run_interpreter(&program);
 let jit = run_baseline_jit(&program);

 assert_eq!(jit.regs, interp.regs);
 assert_eq!(jit.flags, interp.flags);
 assert_eq!(jit.memory_digest(), interp.memory_digest());

 // Performance sanity check: the JIT should not be calling translation helpers per access.
 assert!(jit.stats.mmu_translate_calls < 10_000);
}
```

#### Property tests for randomized memory ops

```rust
proptest! {
 #[test]
 fn jit_random_memory_ops_match_interpreter(ops in arbitrary_mem_ops()) {
 let interp = run_ops_interpreter(&ops);
 let jit = run_ops_baseline_jit(&ops);

 prop_assert_eq!(jit.regs, interp.regs);
 prop_assert_eq!(jit.flags, interp.flags);
 prop_assert_eq!(jit.memory_digest(), interp.memory_digest());
 }
}
```

#### MMIO exit validation

```rust
#[test]
fn jit_mmio_access_causes_exit() {
 // Map an MMIO region and execute a program that touches it.
 let mmio_base = 0xFEE0_0000; // Local APIC (example)
 let program = assemble("
 mov eax, [0xFEE00030] ; read APIC register (e.g. Local APIC version)
 hlt
 ");

 let jit = run_baseline_jit(&program);

 // The JIT must not directly load from RAM for MMIO ranges.
 assert_eq!(jit.exit_reason, ExitReason::Mmio);
 assert!(jit.stats.jit_exit_mmio_calls >= 1);
}
```

#### Device Tests

```rust
#[cfg(test)]
mod device_tests {
 #[test]
 fn test_pic_irq_priority() {
 let mut pic = Pic::new();
 
 // Raise IRQ 1 and IRQ 3
 pic.raise_irq(1);
 pic.raise_irq(3);
 
 // IRQ 1 should be higher priority
 assert_eq!(pic.get_pending_irq(), Some(1));
 
 pic.acknowledge(1);
 assert_eq!(pic.get_pending_irq(), Some(3));
 }
 
 #[test]
 fn test_pit_countdown() {
 let mut pit = Pit::new();
 
 // Set channel 0 to mode 2, count 1000
 pit.write_command(0x34); // Channel 0, mode 2, lo/hi
 pit.write_data(0, 1000 & 0xFF);
 pit.write_data(0, (1000 >> 8) & 0xFF);
 
 // Tick 500 times
 for _ in 0..500 {
 pit.tick();
 }
 
 assert_eq!(pit.read_count(0), 500);
 }
}
```

#### USB (UHCI/EHCI/xHCI + WebHID/WebUSB passthrough) tests

The canonical browser USB stack (controllers + device models) is `crates/aero-usb` (see [Canonical USB stack](../decisions/0015-canonical-usb-stack.md)).
Keep correctness locked down with:

- Rust unit/integration tests under `crates/aero-usb`:
 - UHCI schedule + passthrough mapping tests
 - EHCI tests (regs + root hub timers + async/periodic schedule walking + snapshot roundtrips),
 see [`usb-and-input.md`](usb-and-input.md)
 - xHCI controller tests (MMIO/ring plumbing, transfers, robustness), see [`usb-and-input.md`](usb-and-input.md)
- TypeScript unit/integration tests under `apps/web/src/usb/*test.ts` (including coverage for the
 SharedArrayBuffer ring fast path negotiated by `usb.ringAttach`/`usb.ringDetach` in
 `apps/web/src/usb/usb_proxy_ring_integration.test.ts`)
- Web smoke panels (manual) described in [`usb-and-input.md`](usb-and-input.md)

#### GPU Persistent Cache Tests
 
The persistent GPU cache should have unit tests that lock down **keying and versioning**, since subtle mistakes can lead to hard-to-debug correctness or performance issues.
 
```rust
#[cfg(test)]
mod gpu_cache_tests {
 use super::*;

 #[test]
 fn cache_key_changes_with_schema_version() {
 let shader_bytes = b"dxbc...";
 let k1 = CacheKey::new(1, BackendKind::DxbcToWgsl, shader_bytes, None);
 let k2 = CacheKey::new(2, BackendKind::DxbcToWgsl, shader_bytes, None);
 assert_ne!(k1, k2);
 }

 #[test]
 fn cache_key_changes_with_backend_kind() {
 let shader_bytes = b"dxbc...";
 let k1 = CacheKey::new(1, BackendKind::DxbcToWgsl, shader_bytes, None);
 let k2 = CacheKey::new(1, BackendKind::HlslToWgsl, shader_bytes, None);
 assert_ne!(k1, k2);
 }
}
```

#### ACPI Power Management Tests

Power-management correctness is mostly about **register semantics** and **host
orchestration**, not just table generation. Encode the expected behavior with
unit and integration tests:

```rust
#[test]
fn test_pm1_status_write_one_to_clear() {
 let cfg = AcpiPmConfig::default();
 let pm = Rc::new(RefCell::new(AcpiPmIo::new(cfg)));
 let mut bus = IoPortBus::new();
 register_acpi_pm(&mut bus, pm.clone());

 pm.borrow_mut().trigger_power_button();
 assert_ne!(pm.borrow().pm1_status() & PM1_STS_PWRBTN, 0);

 // Writing a 1 clears the bit.
 bus.write(cfg.pm1a_evt_blk, 2, PM1_STS_PWRBTN as u32);
 assert_eq!(pm.borrow().pm1_status() & PM1_STS_PWRBTN, 0);
}

#[test]
fn test_s5_shutdown_requests_poweroff() {
 let cfg = AcpiPmConfig::default();
 let powered_off = Rc::new(Cell::new(false));
 let powered_off_cb = powered_off.clone();

 let callbacks = AcpiPmCallbacks {
 request_power_off: Some(Box::new(move || powered_off_cb.set(true))),
 ..Default::default()
 };
 let pm = Rc::new(RefCell::new(AcpiPmIo::new_with_callbacks(cfg, callbacks)));
 let mut bus = IoPortBus::new();
 register_acpi_pm(&mut bus, pm);

 // SLP_TYP(S5) + SLP_EN
 bus.write(cfg.pm1a_cnt_blk, 2, ((SLP_TYP_S5 as u32) << 10) | (1 << 13));
 assert!(powered_off.get());
}
```
 
---
 
#### Snapshot Round-Trip Tests

Every device that supports save/restore should have a unit test that validates:

1. `save_state()` produces deterministic bytes for a given state
2. `load_state()` restores an equivalent observable state

```rust
#[test]
fn test_i8042_snapshot_roundtrip() {
 let mut dev = I8042Controller::new();
 dev.inject_scancode(0x1C);

 let snap = dev.save_state();

 let mut restored = I8042Controller::new();
 restored.load_state(&snap).unwrap();

 assert_eq!(dev.read_data_port(), restored.read_data_port());
}
```
#### SMP / APIC IPI Tests

Multi-core enablement needs focused tests because bugs often manifest as hangs or heisenbugs:

- **IPI delivery unit tests**
 - Decode APIC ICR writes.
 - Verify destination selection (physical destination + shorthand modes).
 - Verify delivery modes:
 - **INIT** transitions AP into *wait-for-SIPI*.
 - **SIPI** starts AP at `vector << 12`.
 - **Fixed** delivers an interrupt vector to the target vCPU.
- **AP bring-up integration test**
 - Synthetic "guest" that:
 1. Reads ACPI MADT and asserts multiple processors are present.
 2. BSP sends INIT+SIPI to start an AP.
 3. BSP sends a fixed IPI; AP observes/handles it.

These tests are implemented against the minimal deterministic SMP/APIC model in:

- `crates/aero-smp` (crate `aero_smp`)
 - Tests: `crates/aero-smp/tests/`

### Integration Tests

#### Boot Tests

```rust
use aero_machine::{BootDevice, Machine, MachineConfig, RunExit};

#[test]
fn test_boot_sector_fixture_writes_vga_text() {
 let mut m = Machine::new(MachineConfig {
 ram_size_bytes: 16 * 1024 * 1024,
 boot_device: BootDevice::Hdd,
 ..Default::default()
 })
 .unwrap();
 
 // CI-safe: generated from source via `cargo xtask fixtures`.
 m.set_disk_image(std::fs::read("tests/fixtures/boot/boot_vga_serial_8s.img").unwrap())
 .unwrap();
 m.reset();
 
 // Run a bounded instruction budget (Aero does not model real wall-clock time here).
 for _ in 0..100 {
 match m.run_slice(10_000) {
 RunExit::Completed { .. } | RunExit::Halted { .. } => {}
 other => panic!("unexpected exit: {other:?}"),
 }
 }
 
 // VGA text mode memory starts at 0xB8000.
 let vga = m.read_physical_bytes(0xB8000, 10);
 assert_eq!(vga, tests::fixtures::boot::boot_vga_serial::EXPECTED_VGA_TEXT_BYTES);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_windows_7_boot() {
 let mut m = Machine::new(MachineConfig::win7_storage_defaults(2 * 1024 * 1024 * 1024)).unwrap();
 
 // Aero does not ship Windows images. This test is opt-in and requires a
 // locally-provided disk image path.
 let win7_image = std::env::var("AERO_WINDOWS7_IMAGE")
 .expect("set AERO_WINDOWS7_IMAGE to a locally-provided Windows 7 disk image");
 let bytes = tokio::fs::read(&win7_image).await.unwrap();
 m.set_disk_image(bytes).unwrap();
 m.reset();
 
 // Boot (bounded instruction budget) and periodically present the display so the host can
 // capture frames for visual regression checks.
 for _ in 0..1000 {
 let _ = m.run_slice(100_000);
 m.display_present();
 // In a real test you would detect a stable framebuffer or some guest-visible milestone.
 }
 let screenshot = m.display_framebuffer().to_vec();
 
 // Visual regression test
 assert!(image_matches(screenshot, "expected/win7_login.png", 0.95));
}
```

#### Snapshot/Restore Integration Test

Scripted scenario:

1. perform a disk write (ensure it survives flush)
2. inject keyboard input (pending i8042 bytes)
3. create a network connection (TCP proxy)
4. snapshot, reset VM, restore snapshot
5. verify disk data + input bytes are preserved, and network follows the configured restore policy (drop/reconnect)

```rust
#[test]
fn test_io_snapshot_restore() {
 let mut vm = TestVm::new();

 vm.disk_write(0, b"hello");
 vm.key_press("A");
 vm.tcp_connect("example.com:80");

 let snap = vm.snapshot();
 vm.reset();
 vm.restore(&snap);

 assert_eq!(vm.disk_read(0, 5), b"hello");
 assert!(vm.has_pending_keyboard_bytes());
 assert!(vm.network_is_reconnected_or_dropped_per_policy());
}
```

#### Graphics Tests
 
Graphics correctness needs **two layers**:

1. **Non-GPU tests (fast, deterministic):** shader/pipeline validation, pipeline key hashing, render-state caching, command encoding, etc. These can run as normal Rust unit tests or `wasm-bindgen-test` in a non-GPU JS environment.
2. **Real-GPU smoke tests (browser E2E):** ensure WebGPU and the WebGL2 fallback can initialize, render, and read back pixels. These should run in real browsers via Playwright.

For `wasm-bindgen-test` suites, prefer running via `wasm-pack` (e.g. `wasm-pack test --node`) so tests execute in a JS environment without requiring GPU access.

```rust
#[wasm_bindgen_test]
async fn test_vga_text_mode() {
 let mut vga = VgaEmulator::new();
 
 // Set text mode
 vga.set_mode(0x03);
 
 // Write character at position (0, 0)
 vga.write_char(0, 0, 'A', 0x07);
 
 // Render to framebuffer
 let framebuffer = vga.render();
 
 // Check that 'A' is rendered correctly
 let expected = load_reference_image("vga_text_A.png");
 assert_image_matches(&framebuffer, &expected);
}

#[wasm_bindgen_test]
async fn test_vbe_lfb_mode_1024x768x32() {
 let mut gpu = AeroGpuEmulator::new();

 // Set VBE mode (0x118 suggested) with linear framebuffer bit set.
 gpu.bios_int10_vbe_set_mode(0x118 | (1 << 14));

 // Draw a simple pattern into the reported LFB physical address.
 let lfb = gpu.current_scanout().base_paddr();
 gpu.mem_write_u32(lfb, 0x00FF_0000); // top-left pixel: red (B8G8R8X8)

 let frame = gpu.present().await;
 assert_eq!(frame.resolution(), (1024, 768));
 assert_eq!(frame.pixel(0, 0), Rgba::new(255, 0, 0, 255));
}

#[wasm_bindgen_test]
async fn test_scanout_handoff_vbe_to_wddm() {
 let mut gpu = AeroGpuEmulator::new();

 // Boot uses VBE LFB.
 gpu.bios_int10_vbe_set_mode(0x118 | (1 << 14));
 let legacy_lfb = gpu.current_scanout().base_paddr();
 gpu.mem_write_u32(legacy_lfb, 0x0000_FF00); // green
 let legacy_frame = gpu.present().await;
 assert_eq!(legacy_frame.pixel(0, 0), Rgba::new(0, 255, 0, 255));

 // WDDM driver claims scanout via AeroGPU MMIO registers.
 let wddm_fb = gpu.allocate_wddm_framebuffer(1024, 768, PixelFormat::B8G8R8X8);
 gpu.mem_write_u32(wddm_fb.base_paddr(), 0x0000_0000); // black
 gpu.mmio_set_scanout(wddm_fb.base_paddr(), 1024, 768, 1024 * 4, PixelFormat::B8G8R8X8);

 // Now the visible frame must come from the WDDM-programmed framebuffer.
 let wddm_frame = gpu.present().await;
 assert_eq!(wddm_frame.pixel(0, 0), Rgba::new(0, 0, 0, 255));
}

#[wasm_bindgen_test]
async fn test_directx_triangle() {
 let mut gpu = GpuEmulator::new().await;
 
 // Submit D3D9 draw call
 gpu.set_vertex_shader(PASSTHROUGH_VS);
 gpu.set_pixel_shader(RED_PS);
 gpu.draw_triangle(&[
 Vertex { pos: [0.0, 0.5, 0.0], color: [1.0, 0.0, 0.0, 1.0] },
 Vertex { pos: [-0.5, -0.5, 0.0], color: [1.0, 0.0, 0.0, 1.0] },
 Vertex { pos: [0.5, -0.5, 0.0], color: [1.0, 0.0, 0.0, 1.0] },
 ]);
 
 let output = gpu.present().await;
 
 assert!(output.contains_red_triangle());

 // graphics bottleneck analysis graphics telemetry: even this trivial scene should record
 // at least one draw call and at least one pipeline bind.
 let stats = gpu.last_frame_telemetry();
 assert_eq!(stats.graphics.draw_calls, 1);
 assert!(stats.graphics.pipeline_switches >= 1);

 // GPU timing is best-effort (timestamp-query feature). In environments
 // without support (common in headless CI), it must be null/None rather
 // than causing a failure.
 if stats.graphics.gpu_timing.supported && stats.graphics.gpu_timing.enabled {
 assert!(stats.graphics.gpu_time_ms.is_some());
 } else {
 assert!(stats.graphics.gpu_time_ms.is_none());
 }
}
```

##### Browser GPU smoke tests (Playwright)

The repository includes a minimal harness page (`apps/web/src/pages/gpu_smoke.html`) and a dedicated smoke-test GPU worker (`apps/web/src/workers/gpu_smoke.worker.js`) used by Playwright (`tests/e2e/playwright/gpu_smoke.spec.ts`).

The smoke test does:

- create a canvas and transfer it to the worker (`OffscreenCanvas`)
- `present_test_pattern` (renders a deterministic quadrant pattern)
- `request_screenshot` (GPU readback of the rendered output into an RGBA buffer)
- SHA-256 hash compare against an expected value

WebGPU is treated as **optional** (gated on capability detection); the forced WebGL2 fallback smoke test is **required** so CI continues to validate the fallback path even if headless WebGPU is unavailable.

##### WebGPU notes for headless CI

Chromium's WebGPU availability varies by environment and version. When running headless, it may require browser flags (e.g. `--enable-unsafe-webgpu`) to expose `navigator.gpu`. If WebGPU is still unavailable (or readback fails), the WebGPU smoke test should be skipped rather than failing the suite, while the WebGL2 forced test remains mandatory.

##### D3D9Ex (DWM-facing) smoke test

Windows 7 composition uses **D3D9Ex**, so we need at least one guest-side test that exercises:

- `Direct3DCreate9Ex`
- `CreateDeviceEx`
- `PresentEx`
- `GetPresentStats` / `GetLastPresentCount`

The test should validate that the calls succeed and that present counts are monotonic (full-fidelity timing is not required for initial bring-up).

See: [D3D9Ex / DWM Compatibility](../specs/direct3d-9ex-and-dwm.md#tests).

##### D3D10/11 Conformance Scenes (SM4/SM5)

As D3D10/11 support comes online, grow a small suite of shader-based scenes that render to an offscreen texture and use pixel-compare against known-good outputs.
The intent is to validate the translator at the level D3D apps actually stress:

- constant buffers (cbuffers) and update patterns
- resource views (SRV/RTV/DSV + compute-stage UAV buffers; typed UAV textures still pending)
- input layout semantics mapping
- blend/depth/rasterizer state objects
- instancing and `baseVertex`

See: [16 - Direct3D 10/11 Translation (SM4/SM5 → WebGPU)](../specs/direct3d-10-11-translation.md#conformance-suite-sm45--d3d11-features)

```rust
#[wasm_bindgen_test]
async fn test_d3d11_sm5_constant_buffer_updates() {
 let mut gpu = GpuEmulator::new().await;

 // Load SM5 DXBC shaders (VS/PS).
 gpu.d3d11_set_vertex_shader(SM5_TRIANGLE_VS_DXBC);
 gpu.d3d11_set_pixel_shader(SM5_COLOR_PS_DXBC);

 // Update cb0 every frame (WRITE_DISCARD-like pattern).
 for frame in 0..4 {
 gpu.d3d11_update_constant_buffer(0, &FrameConstants {
 color: [frame as f32 / 3.0, 0.0, 0.0, 1.0],
 });
 gpu.d3d11_draw_triangle();
 }

 let output = gpu.present().await;
 assert!(image_matches(output, "expected/d3d11_sm5_cb_updates.png", 0.995));
}
```

#### Input Tests
 
```rust
#[test]
fn test_keyboard_scancode_translation() {
 let input = InputHandler::new();
 
 // Test common keys
 assert_eq!(input.translate_keycode("KeyA"), 0x1C);
 assert_eq!(input.translate_keycode("Space"), 0x29);
 assert_eq!(input.translate_keycode("Enter"), 0x5A);
 
 // Test extended keys
 let (scancode, extended) = input.translate_keycode_extended("ArrowUp");
 assert_eq!(scancode, 0x75);
 assert!(extended);
}

#[test]
fn test_mouse_packet_generation() {
 let mut mouse = Ps2Mouse::new();
 mouse.set_mode(MouseMode::Stream);
 
 // Move mouse
 mouse.movement(10, -5, 0);
 
 // Get packet
 let packet = mouse.get_packet();
 assert_eq!(packet.len(), 3);
 assert_eq!(packet[1], 10); // X movement
 assert_eq!(packet[2], 5); // Y movement (inverted)
}
```

---

### End-to-End Tests

#### Application Compatibility Tests

```rust
// NOTE: End-to-end Windows application tests require a user-supplied Windows
// installation and are not expected to run in default OSS CI.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_notepad() {
 let vm = boot_windows_7().await;
 
 // Launch notepad
 vm.press_keys(&["Win", "r"]);
 vm.type_text("notepad");
 vm.press_key("Enter");
 
 // Wait for notepad window
 vm.wait_for_window("Untitled - Notepad", Duration::from_secs(10)).await;
 
 // Type some text
 vm.type_text("Hello, World!");
 
 // Verify text appeared
 let screenshot = vm.take_screenshot();
 assert!(ocr_contains(screenshot, "Hello, World!"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_calculator() {
 let vm = boot_windows_7().await;
 
 vm.launch_application("calc.exe").await;
 
 // Perform calculation: 2 + 2 =
 vm.click_button("2");
 vm.click_button("+");
 vm.click_button("2");
 vm.click_button("=");
 
 // Check result
 let result = vm.read_calculator_display();
 assert_eq!(result, "4");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_internet_explorer() {
 let vm = boot_windows_7().await;
 
 // Launch IE
 vm.launch_application("iexplore.exe").await;
 
 // Navigate to test page
 vm.type_in_address_bar("http://example.com");
 vm.press_key("Enter");
 
 // Wait for page load
 vm.wait_for_page_load(Duration::from_secs(30)).await;
 
 // Verify content
 let screenshot = vm.take_screenshot();
 assert!(ocr_contains(screenshot, "Example Domain"));
}
```

#### Guest-side GPU driver validation (Windows 7)

To validate the AeroGPU WDDM driver stack end-to-end **inside a Windows 7 guest**, use the guest-side test suite in:

* `drivers/aerogpu/tests/win7/`

This suite contains small D3D9Ex/D3D11 programs that render a known pattern, read back pixels to assert correctness, and print a clear `PASS:`/`FAIL:` line (non-zero exit code on failure). A `run_all.cmd` harness is included to execute the suite and aggregate results.

These tests are intended for Win7 VMs with AeroGPU installed and are not expected to run in default OSS CI.

#### Performance Benchmarks

Rust microbenchmarks in this repo use Criterion and live under `crates/*/benches/`. For CPU work,
start with:

- `crates/aero-cpu-core/benches/` (Criterion harness)

Note: the current `emulator_critical` microbench targets the legacy interpreter dispatch loop and
is gated behind `--features legacy-interp` (see [`testing.md`](testing.md) for the exact
commands used in CI). Tier-0 and future JIT microbenches should follow the same Criterion structure
but drive `exec::Tier0Interpreter` / `exec::ExecDispatcher`.

For D3D10/11 specifically, add a “many draws” microbench once the translation layer exists:

- 1–10k draw calls with a stable pipeline key (measures per-draw binding overhead)
- pipeline churn test (measures pipeline-cache behavior and compilation costs)
- constant-buffer update bandwidth (measures ring allocator / renaming strategy)

##### Benchmark output should include JIT telemetry 

Raw throughput numbers (e.g. MIPS) are not enough to debug regressions in a tiered JIT: a 10% slowdown can come from "execution got slower" *or* "we started compiling more / compiling slower".

For any benchmark that executes guest code, the runner should also emit a compact `jit` summary derived from the JIT telemetry surface telemetry, for example:

```
bench_instruction_throughput ... 510.2 MIPS
jit: hit_rate=98.7% blocks(t1=1234,t2=56) compile_ms(t1=87.4,t2=45.1) compile_ms/s=3.4 deopt=0 guard_fail=0 cache=100MiB/256MiB
```

This makes it possible to attribute changes quickly:

- **MIPS ↓ + compile_ms/s ↑** → compilation overhead increased (thresholds, cache misses, slower passes)
- **MIPS ↓ + hit_rate ↓** → cache thrash / poor block keys / frequent invalidation
- **MIPS ↓ + deopt ↑** → unstable Tier 2 assumptions (guard policy, profiling, invalidation)

##### Synthetic workload to validate the JIT telemetry surface

Maintain at least one synthetic benchmark that intentionally forces compilation:

- Execute a large number of distinct basic blocks (e.g. by generating a code buffer with many unique addresses) to trigger Tier 1 compilation.
- Run long enough to promote a subset to Tier 2 (if Tier 2 exists).

Verification criteria:

- With JIT enabled: `blocks_compiled_total > 0` and `compile_ms_total > 0` in exported/printed metrics.
- With compilation disabled (interpreter-only): all `jit.*` totals remain `0`, and benchmark overhead stays minimal.

#### Guest CPU Throughput Benchmarks 

To measure emulator CPU performance **without** booting an OS image, run small deterministic x86/x86-64 payloads inside the CPU core and compute IPS/MIPS from retired instruction counts.

These benches are designed to run as soon as the interpreter exists and to show clear speedups when JIT tiers land. They must validate correctness via checksums and **fail the run** on mismatches.

See: [Guest CPU Instruction Throughput Benchmarks](performance.md).

---

### Conformance Testing

#### Against Reference Implementation

The repository includes an initial differential testing harness at `crates/conformance/` that
compares a small deterministic corpus against native host execution on `x86_64` (user-mode
instructions only).

Run it locally (recommended) with:

```bash
cargo xtask conformance --cases 512
```

Note: the native reference backend currently requires a unix `x86_64` host. On other platforms,
`cargo xtask conformance` prints a friendly "skipped" message and exits 0.

You can also invoke the tests directly via Cargo:

```bash
cargo test --locked -p conformance --test conformance -- --nocapture
```

Tuning / reproducing failures:

- `AERO_CONFORMANCE_CASES=<n>` to increase/decrease the corpus size (default `512`).
- `AERO_CONFORMANCE_SEED=<n>` to reproduce a specific random corpus (decimal or `0x...` hex; `_` separators allowed).
- `AERO_CONFORMANCE_FILTER=<expr>` to select a subset of the corpus/templates (e.g. `add`).
- `AERO_CONFORMANCE_REFERENCE=qemu` to compare against a QEMU reference backend (requires running the `conformance` crate with `--features qemu-reference` and a `qemu-system-*` binary).
- `AERO_CONFORMANCE_REPORT_PATH=<path>` to write a JSON report (written on failure; also written on success when set).
- `AERO_CONFORMANCE_REFERENCE_ISOLATE=0` to disable `fork()` isolation in the native reference backend (can help in sandboxed environments).

Using the xtask wrapper is just a convenient way to set these env vars:

```bash
cargo xtask conformance \
 --cases 5000 \
 --seed 0x52c671d9a4f231b9 \
 --filter add \
 --report target/conformance.json

# Forward extra args to `cargo test` after `--`, e.g. to show test output:
cargo xtask conformance --cases 32 -- --nocapture
```

CI runs a fast subset on PRs and a larger corpus on a schedule via
`.github/workflows/conformance.yml`.

In addition, `tools/qemu_diff/` (Cargo package `qemu-diff`, crate identifier `qemu_diff`) provides a
**CI-friendly differential harness** that compares Aero execution against QEMU on synthetic 16-bit
snippets (no QEMU/GPL code shipped; QEMU is an external tool invoked by tests).

- `tools/qemu_diff/` builds a tiny bootable floppy image and runs it under an external
 `qemu-system-*` binary.
- `crates/aero-cpu-core` contains snippet runners that execute under the tier-0 engine (both a
 single-step path and a batch path) and compares results against QEMU.

Run locally:

```bash
# Tier-0 batch vs tier-0 single-step equivalence (always runs)
cargo test --locked -p aero-cpu-core

# Differential tests vs QEMU (skips if QEMU is not installed)
cargo test --locked -p aero-cpu-core --features qemu-diff
```

```rust
/// Compare Aero execution against a reference backend (e.g. native host execution or QEMU)
#[test]
fn conformance_test_instructions() {
 for instruction in ALL_X86_INSTRUCTIONS {
 let aero_result = run_in_aero(&instruction);
 let qemu_result = run_in_qemu(&instruction);
 
 assert_eq!(
 aero_result.registers, 
 qemu_result.registers,
 "Instruction {} produced different results",
 instruction.name
 );
 
 assert_eq!(
 aero_result.flags,
 qemu_result.flags,
 "Instruction {} produced different flags",
 instruction.name
 );
 }
}
```

#### Instruction Set Coverage

```rust
#[test]
fn test_instruction_coverage() {
 let coverage = InstructionCoverage::new();
 
 // Run test suite
 run_all_tests(&mut coverage);
 
 // Check coverage
 let report = coverage.generate_report();
 
 println!("Instruction coverage: {:.2}%", report.percentage);
 println!("Missing instructions:");
 for inst in &report.uncovered {
 println!(" - {}", inst);
 }
 
 assert!(report.percentage >= 95.0, "Instruction coverage below 95%");
}
```

---

### Browser Testing

#### Cross-Browser Test Suite

To balance fast PR feedback with high-confidence compatibility coverage, CI typically splits browser automation into:

- **PR CI:** run the Playwright suite on a single browser (Chromium) for speed.
- **Cross-browser CI:** run the suite across Chromium/Firefox/WebKit on a schedule and via manual trigger.

See `.github/workflows/e2e-matrix.yml` for the scheduled cross-browser matrix.

```javascript
// playwright.config.js
module.exports = {
 projects: [
 {
 name: 'chromium',
 use: { browserName: 'chromium' },
 },
 {
 name: 'firefox',
 use: { browserName: 'firefox' },
 },
 {
 name: 'webkit',
 use: { browserName: 'webkit' },
 },
 ],
};

// tests/browser.spec.js
test.describe('Browser Compatibility', () => {
 test('initializes emulator', async ({ page }) => {
 await page.goto('/');
 
 const status = await page.evaluate(() => {
 return window.aero.getStatus();
 });
 
 expect(status.initialized).toBe(true);
 expect(status.webgpu).toBe(true);
 expect(status.wasm_simd).toBe(true);
 });
 
 test('boots to desktop', async ({ page }) => {
 await page.goto('/');
 await page.click('#start-button');
 
 // Wait for boot
 await page.waitForFunction(() => {
 return window.aero.isBooted();
 }, { timeout: 120000 });
 
 // Take screenshot
 const screenshot = await page.screenshot();
 expect(screenshot).toMatchSnapshot('desktop.png');
 });
});
```

#### Browser Integration Test: Persistent Shader Cache

Use a browser automation test to verify persistence across reloads:

1. Load the app.
2. Trigger a shader translation/compile path that is known to populate the persistent cache.
3. Capture telemetry counters (hits/misses, bytes written).
4. Reload the page (new JS context).
5. Trigger the same shader path again.
6. Assert that **persistent cache hits** increased and translation work did not run.

```javascript
test('persists shader translations across reload', async ({ page }) => {
 await page.goto('/');

 // Ensure clean slate.
 await page.evaluate(() => window.aero.gpu.clearCache());

 // Warm the cache.
 await page.evaluate(async () => {
 await window.aero.gpu.compileKnownShaderSetForTests();
 });
 const warmStats = await page.evaluate(() => window.aero.gpu.getCacheTelemetry());
 expect(warmStats.bytes_written).toBeGreaterThan(0);

 // New session.
 await page.reload();

 await page.evaluate(async () => {
 await window.aero.gpu.compileKnownShaderSetForTests();
 });
 const coldStats = await page.evaluate(() => window.aero.gpu.getCacheTelemetry());
 expect(coldStats.persistent_hits).toBeGreaterThan(0);
});
```

#### GPU Presenter Color/Alpha Validation

In addition to full-system screenshots, we need a deterministic GPU-only validation that catches:

- double-applied gamma (too dark / too bright output)
- incorrect canvas alpha mode (premultiplied vs opaque haloing)
- backend-dependent Y-flips / UV convention mismatches

Use a simple **test card** (grayscale ramp + alpha gradient + corner markers) and hash the
presented pixels per backend. See `apps/web/src/gpu/validation-scene.ts` and the Playwright spec
`tests/e2e/web/gpu_color.spec.ts`.

> Terminology note: this test hashes the **presented output** (post color-space/alpha policy). The GPU worker
> screenshot API (`type: "screenshot"`) is defined separately as a deterministic readback of the **source framebuffer**
> bytes for hashing guest frames (see `apps/web/src/gpu/presenter.ts`).

#### CSP/COOP/COEP regression tests (implemented)

This repo includes a small WASM/JIT CSP PoC app plus Playwright coverage that asserts:

- COOP/COEP is enabled (`crossOriginIsolated === true`)
- a strict CSP without `wasm-unsafe-eval` blocks dynamic wasm compilation and triggers a fallback
- adding `script-src 'wasm-unsafe-eval'` enables dynamic compilation again

Entry points:

- PoC app: `apps/web/public/wasm-jit-csp/` (served by `tests/helpers/csp_server.mjs`)
- Tests: `tests/e2e/csp-fallback.spec.ts`
- Run: `pnpm run test:e2e`

---

### GPU Golden-Image Correctness Tests (Playwright)

The graphics subsystem needs **deterministic, automated correctness tests** that can catch subtle rendering regressions without requiring a full Windows boot.

This repository includes a minimal Playwright-based harness that:

- Renders **deterministic microtests**:
 - VGA-style text (chars + attrs)
 - VBE-style LFB color bars
 - WebGL2/WebGPU direct rendering microtests
 - GPU backend “smoke page” (`apps/web/gpu-smoke.html`) capture
 - GPU trace replay (`tests/fixtures/triangle.aerogputrace`) capture
- Captures the rendered frame as **raw RGBA bytes** (WebGL2 `readPixels`, WebGPU buffer readback).
- Compares the output against committed **golden PNGs**.
- On failure, writes **expected/actual/diff** images as Playwright test artifacts.

Key files:

- `tests/e2e/playwright/gpu_golden.spec.ts` — microtests + capture
- `tests/e2e/playwright/utils/image_diff.ts` — pixel diff + artifact emission
- `tests/golden/*.png` — committed goldens (synthetic scenes only)
- `playwright.gpu.config.ts` — Playwright config (Chromium+WebGPU flags + Firefox WebGL2)

Local usage:

```bash
# Generate/update goldens (pure CPU generation, no browser required)
pnpm install --frozen-lockfile
pnpm run generate:goldens

# Run golden tests (requires Playwright browsers; CI installs/caches them via `.github/actions/setup-playwright`)
pnpm run test:gpu
```

CI enforcement:

- CI runs `pnpm run generate:goldens` and fails if anything under `tests/golden/` changes afterwards.
- If your change intentionally affects golden output, rerun `pnpm run generate:goldens` locally and commit the updated PNGs.
 - Tip: `pnpm run check:goldens` runs the same “regenerate + fail on drift” check CI uses.

---

### Visual regression (Playwright screenshots)

We use **Playwright screenshot assertions** as a "golden image" visual regression suite.

#### Conventions

- **Snapshot location:** `tests/e2e/__screenshots__/…`
 - Configured via `playwright.config.ts` `snapshotPathTemplate`.
- **Naming:** each screenshot name is tied to:
 - test file path (`{testFilePath}`)
 - the screenshot name passed to `toHaveScreenshot('…')` (`{arg}`)
 - Playwright project (`{-projectName}`; typically `chromium`)
 - OS platform (`{-platform}`; `linux`/`darwin`/`win32`) to avoid cross-OS font rasterization churn.
 - Example: `tests/e2e/__screenshots__/visual.spec.ts/aero-window-chromium-linux.png`
- **Scope:** snapshots must only cover **synthetic scenes/UI we own**.
 - Do **not** add screenshots of Windows 7 or any copyrighted imagery.

#### Rendering stability

Defaults live in `playwright.config.ts`:

- Fixed `viewport` and `deviceScaleFactor` for deterministic output.
- `reducedMotion: 'reduce'` + test-injected CSS to disable animations/transitions.
- Deterministic fonts: prefer explicitly declaring a font family known to exist in CI (e.g. `DejaVu Sans` on Linux) instead of relying on `system-ui`.
- Screenshot tolerance via:
 - `expect.toHaveScreenshot.maxDiffPixelRatio`
 - per-test overrides when needed.

#### Developer workflow

- Run E2E + visual regression tests:
 - `pnpm run test:e2e`
- Update screenshot baselines after an intentional UI change:
 - `pnpm run test:e2e:update`
- Review diffs locally in the HTML report:
 - `pnpm run test:e2e:report` (opens `playwright-report/`)

#### CI integration

The GitHub Actions workflow uploads artifacts on failures (including screenshot diffs):

- `playwright-report/` (HTML report)
- `test-results/` (attachments: diffs, traces, videos)

This makes snapshot mismatches easy to review directly from CI.

---

### Continuous Integration

> **These workflows no longer exist.** Continuous integration was purged and is
> on the ban list — see [the working agreements](../meta/working-agreements.md).
> What follows describes the jobs as they were configured, which is still the
> best record of *what should be run and in what combination*; run them by
> hand. Nothing here runs automatically.

#### Running WASM unit tests locally

```bash
cd crates/aero-wasm
wasm-pack test --node
```

#### GitHub Actions Workflow

```yaml
name: Aero CI

on: [push, pull_request]

jobs:
 test:
 runs-on: ubuntu-latest
 steps:
 - uses: actions/checkout@v4
 
 - name: Setup Rust
 id: setup-rust
 uses: ./.github/actions/setup-rust
 with:
 toolchain: stable
 targets: wasm32-unknown-unknown
 locked: always

 - name: Install wasm-pack
 uses: taiki-e/install-action@v2
 with:
 tool: wasm-pack
 
 - name: Run unit tests
 run: cargo test ${{ steps.setup-rust.outputs.cargo_locked_flag }} --all-features
 
 - name: Run WASM tests (node)
 working-directory: crates/aero-wasm
 run: wasm-pack test --node
 
 - name: Build
 run: cargo build ${{ steps.setup-rust.outputs.cargo_locked_flag }} --release --target wasm32-unknown-unknown
 
 browser-tests:
 runs-on: ubuntu-latest
 steps:
 - uses: actions/checkout@v4
 
 - name: Setup Node workspace
 uses: ./.github/actions/setup-node-workspace
 env:
 PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD: "1"
 
 - name: Setup Playwright (cached)
 uses: ./.github/actions/setup-playwright
 with:
 browsers: chromium
 
 - name: Run browser tests
 run: pnpm exec playwright test
 
 benchmarks:
 runs-on: ubuntu-latest
 steps:
 - uses: actions/checkout@v4
 
 - name: Run benchmarks
 run: |
 cargo bench --locked
 # Criterion writes results to `target/criterion/`. Move them out so they
 # don't get overwritten and so we can compare them against a baseline.
 rm -rf target/bench-new/criterion
 mkdir -p target/bench-new
 mv target/criterion target/bench-new/criterion
 
 - name: Compare with baseline
 run: |
 # See `.github/workflows/bench.yml` for the full baseline download + PR
 # base/head comparison logic.
 python3 scripts/bench_compare.py \
 --base baseline/target/bench-new/criterion \
 --new target/bench-new/criterion \
 --thresholds-file bench/perf_thresholds.json \
 --profile pr-smoke
```

##### CI note: GPU timing is optional

GPU timing via WebGPU timestamp queries (`timestamp-query`) should be treated as **informational**:

- Headless CI or software WebGPU adapters may not expose timestamp queries.
- Tests and smoke perf runs should validate counter-based metrics (draw calls, pipeline switches, uploads) without requiring GPU timing.
- If a CI lane wants to assert GPU timing, gate it behind an explicit opt-in (e.g. `AERO_ENABLE_GPU_TIMING=1`) and skip when unsupported.

---

### Guest driver validation (virtio)

For the paravirtualized “fast path” devices (virtio-blk/net/snd/input), validation requires both:

1. Host-side/unit tests for shared protocol structs (layout/ABI), and
2. In-guest smoke tests (Device Manager binding + basic throughput checks).

See `drivers/README.md` for the current driver-pack workflow and the minimal in-guest validation checklist.

---

### Test Data Management

**Important:** This repository must not include proprietary OS media (e.g., Windows ISOs/images) or other disallowed binary fixtures. Keep fixtures small and open-source, and prefer generating or downloading test assets during local/CI setup. See [Fixtures & Test Assets Policy](testing.md).

#### Disk Image Fixtures

```rust
// Generate minimal test disk images
fn create_test_disk(scenario: TestScenario) -> DiskImage {
 match scenario {
 TestScenario::EmptyDisk => DiskImage::empty(8 * MB),

 // CI-safe: checked in as tiny deterministic binaries generated from source.
 TestScenario::BootFixture => {
 DiskImage::from_file("tests/fixtures/boot/boot_vga_serial_8s.img")
 }

 // Open-source OS images are valuable too, but are typically generated or
 // downloaded during local setup (not committed).
 TestScenario::BootableDos => create_freedos_image(),

 // Aero does not ship Windows disk images. When developing locally, keep
 // any Windows image outside the repo and plumb it in via configuration.
 TestScenario::BootableWindows7 => DiskImage::from_path(
 std::env::var("AERO_WINDOWS7_IMAGE")
 .expect("set AERO_WINDOWS7_IMAGE to a locally-provided Windows 7 disk image"),
 ),
 }
}
```

---

### Next Steps

- See [Legal Considerations](../history/sprint-era-record.md) for test image licensing
- See [Project Milestones](../history/sprint-era-record.md) for testing phases

## Testing (local + CI)

This document is the practical companion to [`12-testing-strategy.md`](testing.md). It focuses on **how to run Aero’s test stack locally**, how that maps to CI, and common browser-specific failure modes.

> **Policy note (fixtures):** The repository must not include proprietary Windows images/ISOs, BIOS ROMs, or other copyrighted firmware blobs. Tests and CI should run using **open fixtures** (synthetic images, open-source OS images, generated data). See [`FIXTURES.md`](testing.md) and [`13-legal-considerations.md`](../history/sprint-era-record.md). CI also enforces this via `scripts/ci/check-repo-policy.sh`.

---

### Node.js version

CI uses the exact Node.js version declared in the repo root [`.nvmrc`](../../.nvmrc). Local tooling requires
**at least** that version and will warn if your local version differs (to help debug flaky toolchain issues).

From the repo root:

```bash
nvm install && nvm use # if you use nvm
node scripts/check-node-version.mjs
# or: pnpm run check:node
```

If you need to run tooling in an environment where you can't change the Node version (unsupported), you can bypass the hard error:

```bash
 AERO_ALLOW_UNSUPPORTED_NODE=1 node scripts/check-node-version.mjs
```

---

### Manual testing checklists

Some end-to-end behaviors (especially audio) are hard to validate automatically. The repo keeps a small set of
**manual, reproducible smoke-test checklists**, which live in the area pages themselves.

- Windows 7 audio (in-box HDA driver): [`audio.md`](audio.md)

### Disk streaming endpoint conformance (HTTP Range / chunked)

If you are debugging a disk image delivery endpoint (local, staging, prod) and want to validate that it matches
the browser-side streaming client expectations, use the dependency-free conformance tool:

- Range mode (single object + HTTP `Range`): `python3 tools/disk-streaming-conformance/conformance.py --base-url ...`
- Chunked mode (`manifest.json` + `chunks/*.bin`, no `Range` header): `python3 tools/disk-streaming-conformance/conformance.py --mode chunked --manifest-url ...`

For quick local sanity checks against the repo’s dev servers:

- `python3 tools/disk-streaming-conformance/selftest_range_server.py`
- `python3 tools/disk-streaming-conformance/selftest_range_server_private.py`
- `python3 tools/disk-streaming-conformance/selftest_chunk_server.py`
- `python3 tools/disk-streaming-conformance/selftest_chunk_server_private.py`

Use `--strict` to fail on warnings (recommended when validating CDN/edge behavior).

### Device/driver end-to-end test plans

Some subsystems also have “single document” end-to-end test plans (device model ↔ guest drivers ↔ web runtime).

- virtio-input (device model + Win7 driver + web runtime): [`windows-drivers.md`](windows-drivers.md)

### Test fixtures (boot sectors + tiny disk images)

Boot/system tests use **tiny deterministic fixtures** under `tests/fixtures/` (e.g. a 512-byte boot sector that writes a known pattern to VGA/serial).

Regenerate them with:

```bash
cargo xtask fixtures
```

To verify they are up-to-date without modifying your working tree:

```bash
cargo xtask fixtures --check
```

CI runs the `--check` form (also the first step of `cargo xtask test-all`) and fails if any fixture generated by
`cargo xtask fixtures` is missing or out-of-date. Use `cargo xtask test-all --skip-fixtures` to skip this check
while iterating.

`cargo xtask test-all` runs the same fixture checks automatically (unless you pass `--skip-fixtures`).

### Quick start: run the full test suite

#### Unified runner (recommended)

From the repo root:

```bash
cargo xtask test-all
```

Note: in this repo, `cargo xtask` runs Cargo with `--locked` (matching CI). If you hit a lockfile error, update
the lockfile (e.g. `cargo generate-lockfile`, or `cargo generate-lockfile --manifest-path path/to/tool/Cargo.toml`
for standalone tools) and include the `Cargo.lock` diff in your PR.

`bash ./scripts/test-all.sh` is kept as a thin wrapper around `cargo xtask test-all`
for a transition period, but `cargo xtask` is the canonical implementation (and
 works on Windows without bash).

The unified runner executes (in order):

1. `cargo xtask fixtures --check` (validate committed deterministic fixtures)
2. `cargo xtask bios-rom --check` (validate `assets/bios.bin`)
3. `cargo fmt --all -- --check`
4. `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
5. `cargo test --locked --workspace --all-features`
6. `wasm-pack test --node` (in the WASM crate)
7. `cargo check --target wasm32-unknown-unknown -p aero-devices-storage -p aero-machine` (WASM build sanity)
8. `pnpm run test:unit`
9. `pnpm run test:e2e`

For a **fast Node-only sanity check** (no TypeScript typecheck, no Playwright), run:

```bash
pnpm run test:contracts
```

By default it sets `AERO_REQUIRE_WEBGPU=0` (matching CI) unless you explicitly enable it.

Common options:

```bash
# Skip deterministic fixture checks while iterating
cargo xtask test-all --skip-fixtures

# Skip the slowest step
cargo xtask test-all --skip-e2e

# Require WebGPU for tests that gate on it
cargo xtask test-all --webgpu

# Select Playwright projects (repeatable)
cargo xtask test-all --pw-project chromium --pw-project firefox

# Forward additional Playwright CLI args (everything after --)
cargo xtask test-all --pw-project chromium -- --grep smoke
```

#### Minimal Rust sanity checks (no Node / no Playwright)

If you want a fast, Rust-only smoke check (useful while iterating, or when debugging CI failures), run:

```bash
bash ./scripts/safe-run.sh cargo test -p aero-platform --locked
bash ./scripts/safe-run.sh cargo check -p aero-machine --locked
bash ./scripts/safe-run.sh cargo check -p aero-wasm --target wasm32-unknown-unknown --locked
```

#### Focused test runners

If you're working on a specific subsystem, `xtask` also provides smaller suites, and some subsystems
have dedicated `scripts/ci/*` helpers:

```bash
# USB + input (Rust + focused web unit tests; optional Playwright subset)
cargo xtask input
cargo xtask input --rust-only
cargo xtask input --usb-all
cargo xtask input --machine
cargo xtask input --with-wasm
cargo xtask input --rust-only --with-wasm
cargo xtask input --wasm --rust-only
cargo xtask input --e2e

# If your Node workspace entrypoint is `apps/web/` (rather than repo root), use:
cargo xtask input --node-dir web

# Boot display (VGA/VBE/INT10) regression suite (device model + BIOS INT10 + machine wiring)
bash ./scripts/ci/run-vga-vbe-tests.sh
```

#### CPU instruction conformance / differential tests (x86_64 unix)

The conformance harness compares Aero instruction semantics against native host execution on `x86_64` unix.

```bash
# Run a small corpus
cargo xtask conformance --cases 512

# (Optional) same via `just`
just test-conformance --cases 512

# Reproduce a failure (seed is decimal or 0x-hex; `_` separators allowed)
cargo xtask conformance \
 --cases 5000 \
 --seed 0x52c671d9a4f231b9 \
 --filter key:add \
 --report target/conformance.json \
 -- --nocapture
```

If your repo layout differs from the defaults, override directories:

- `AERO_NODE_DIR` / `--node-dir`: the directory containing `package.json` (deprecated aliases: `AERO_WEB_DIR`, `WEB_DIR`)
- `AERO_WASM_CRATE_DIR` / `--wasm-crate-dir`: the crate directory containing the WASM `Cargo.toml`

To see what directory CI/local tooling will select by default, run:

```bash
node scripts/ci/detect-node-dir.mjs
```

---

### Node workspaces: install once, run per-package scripts

The repo uses **one pnpm workspace** (see [`../decisions/0006-node-monorepo-tooling.md`](../decisions/0006-node-monorepo-tooling.md)). npm is rejected by the hygiene contract test.

Install all Node dependencies from the repo root:

```bash
pnpm install --frozen-lockfile
```

Run package-scoped scripts using `pnpm -C <path>`:

```bash
# Frontend dev server
pnpm run dev

# Gateway unit tests
pnpm -C services/gateway test
```

### Security header regression tests (COOP/COEP/CSP)

Cross-origin isolation and Aero’s WASM/JIT features depend on a consistent set of security headers
(COOP/COEP/CORP/OAC + CSP with `wasm-unsafe-eval`). To prevent template/server drift, the repo includes:

```bash
# Fast static validation (templates + Vite configs + proxies)
pnpm run check:security-headers

# Runtime validation (Playwright, preview server)
pnpm run test:security-headers
```

#### Manual (equivalent) commands

From the repo root:

```bash
# Rust format/lint/test (host)
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features

# Rust → WASM tests (run from the WASM crate directory; see below)
# wasm-pack test --node

# TypeScript / JS unit tests
pnpm install --frozen-lockfile
pnpm run test:unit

# Playwright E2E
node scripts/playwright_install.mjs chromium --with-deps
pnpm run test:e2e
```

Notes:

- The `wasm-pack` step is usually run from the specific crate that produces WASM (often under `crates/`).
- Playwright browser downloads are large; locally you typically only need to run `node scripts/playwright_install.mjs chromium --with-deps` once per machine.
 - CI uses `.github/actions/setup-playwright` to cache the Playwright browser binaries directory and re-install only if the cache is missing.
 - `just setup` sets `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1` for the install so browser binaries come from the explicit install step instead.

---

### Rust unit tests (host)

Run all Rust tests in the workspace:

```bash
cargo test --locked --workspace
```

Run tests for a single crate:

```bash
cargo test --locked -p <crate-name>
```

Run a single test (by name filter):

```bash
cargo test --locked -p <crate-name> <test_name_substring>
```

Useful flags:

```bash
# Show stdout/stderr for passing tests
cargo test --locked -p <crate-name> -- --nocapture

# Run ignored tests (if any are marked #[ignore])
cargo test --locked -p <crate-name> -- --ignored
```

---

### QEMU boot tests (serial + framebuffer)

Some integration tests boot guest media **headlessly under QEMU** to validate early boot flows and
enable deterministic framebuffer/serial checkpoints.

These tests are defined under the workspace root `tests/` directory (e.g. `tests/boot_sector.rs`,
`tests/freedos_boot.rs`, `tests/windows7_boot.rs`, and `tests/boot/basic_boot.rs`).

The canonical way to run them is via the dedicated `aero-boot-tests` crate: it registers them as
explicit `[[test]]` targets in `crates/aero-boot-tests/Cargo.toml` (paths like
`../../tests/boot_sector.rs`), so you can run them with `cargo test -p aero-boot-tests --test ...`.

Note: this is a **test harness / dependency hygiene** choice (see the note below on `-p aero`), and
does *not* imply a device crate is the canonical VM wiring layer. Canonical machine wiring lives
in `crates/aero-machine` (`aero_machine::Machine`); see [`../state/repo-state-and-structure.md`](../state/repo-state-and-structure.md)
and [`platform-and-firmware.md`](platform-and-firmware.md).

Avoid running them via the workspace root package (`-p aero`): `aero` intentionally does not carry
the `firmware`/`memory` dev-deps needed by `boot_basic`, and it also pulls in heavyweight GPU
dev-dependencies that significantly slow compilation.

#### Prerequisites

- `qemu-system-i386`
- `mtools` (for patching floppy images in `scripts/prepare-freedos.sh`)
- `unzip` + `curl` (for downloading FreeDOS)

#### Boot sector + FreeDOS (open-source, CI-safe)

```bash
# Generate the synthetic boot fixtures used by the tests (committed output is checked in CI).
cargo xtask fixtures

# Verify the committed fixture outputs are up-to-date (this is what CI runs).
cargo xtask fixtures --check

# Download + patch FreeDOS 1.4 floppy image (written under gitignored test-images/).
bash ./scripts/prepare-freedos.sh

# Run the QEMU boot tests.
cargo test -p aero-boot-tests --test boot_sector --test freedos_boot --locked

# In constrained/contended sandboxes (agents/CI-like), prefer safe-run.sh (timeout + memory limit).
# The first run can take >10 minutes to compile on a cold checkout.
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-boot-tests --test boot_sector --test freedos_boot --locked
```

#### Windows 7 (local only)

The Windows boot test is intentionally gated and requires user-supplied media:

```bash
bash ./scripts/prepare-windows7.sh
cargo test -p aero-boot-tests --test windows7_boot --locked -- --ignored
```

#### Useful environment variables

- `AERO_QEMU=/path/to/qemu-system-i386` to override the QEMU binary
- `AERO_ARTIFACT_DIR=...` to control where failing screenshots/diffs are written
- `AERO_REQUIRE_TEST_IMAGES=1` to turn missing fixture files into hard errors (CI uses this)

---

### WASM tests (Rust compiled to WebAssembly)

For crates that use `wasm-bindgen-test`, run tests in a Node environment:

```bash
# From the WASM crate directory (where Cargo.toml for the WASM crate lives)
wasm-pack test --node
```

Notes:

- `cargo xtask test-all` resolves the wasm-pack crate via `scripts/ci/detect-wasm-crate.*` (shared with CI tooling).
 - Override with `AERO_WASM_CRATE_DIR` / `--wasm-crate-dir` if needed.
 - Otherwise it prefers the canonical crate (`crates/aero-wasm`) when present.
 - If it must fall back to `cargo metadata`, it fails if multiple `cdylib` crates exist (set an override).

Common pitfalls:

- **Wrong directory:** `wasm-pack` operates on a *single crate*. Run it from the crate that builds to WASM.
- **Missing target:** ensure the WASM target is installed:
 ```bash
 rustup target add wasm32-unknown-unknown
 ```
- **Threaded/shared-memory builds:** the web app’s threaded WASM build uses `-Z build-std` and requires the **pinned**
 nightly toolchain declared in `scripts/toolchains.json` (`rust.nightlyWasm`) plus `rust-src`. Run `just setup` to
 install the correct nightly toolchain (or see README for the manual commands).
 - The build also enables the internal Cargo feature `wasm-threaded` on relevant crates (via the web build
 scripts) so Rust takes the shared-memory-safe paths.
- **Node vs browser environment:** `--node` does **not** provide DOM APIs (`document`, `window`, etc.). Keep `--node` tests focused on pure logic/WASM exports. If a test needs browser APIs, it should use a browser runner (e.g. `wasm-pack test --headless --chrome`) or be covered by Playwright.
- **WASM threads:** if a test requires `SharedArrayBuffer` / WASM threads, Node support may differ from browsers. Prefer testing thread-dependent behavior in a real browser (Playwright) where COOP/COEP can be enforced.

---

### TypeScript unit tests

Install dependencies:

```bash
pnpm install --frozen-lockfile
```

Run unit tests:

```bash
pnpm run test:unit
```

#### Test topology

`pnpm run test:unit` runs two harnesses:

- **Node-only tests (`node:test`)**: Node integration/unit suites (e.g. `apps/web/test/**/*.test.ts`).
- **Unit tests (Vitest)**: colocated unit tests under `apps/web/src/**/*.test.ts`, plus any dedicated Vitest suites under
 `apps/web/test/**/*.vitest.ts` (configured via `apps/web/vite.config.ts`).

Run with coverage (most runners accept `--coverage` via argument passthrough):

```bash
pnpm run test:unit -- --coverage
```

Typical output locations:

- Terminal summary (pass/fail)
- `coverage/` directory (HTML + LCOV), depending on the runner configuration

---

### Playwright E2E tests

Playwright specs live under `tests/e2e/**/*.spec.ts` (configured by the repo-root `playwright.config.ts`).

Run headless E2E tests:

```bash
pnpm run test:e2e
```

Notes:

- Playwright starts the **repo-root Vite app** (`pnpm run dev:harness`, same as `pnpm run dev`) on `127.0.0.1:5173` and a COI preview server on `127.0.0.1:4173`.
 If you already have another server on those ports (for example the legacy `apps/web/` Vite app: `pnpm run dev:web` / `pnpm -C apps/web run dev`), stop it before running Playwright.
- To reuse an already-running harness server while iterating locally:
 `AERO_PLAYWRIGHT_REUSE_SERVER=1 pnpm run test:e2e`

Run a specific browser project:

```bash
pnpm run test:e2e -- --project=chromium
pnpm run test:e2e -- --project=firefox
pnpm run test:e2e -- --project=webkit
```

Open Playwright UI mode (interactive runner):

```bash
pnpm run test:e2e -- --ui
```

Update snapshots (for screenshot/visual regression tests):

```bash
pnpm run test:e2e -- --update-snapshots
```

Debugging tips:

```bash
# Run a single test file
pnpm run test:e2e -- path/to/test.spec.ts

# Keep the browser open on failure (Playwright convention)
PWDEBUG=1 pnpm run test:e2e
```

If E2E tests fail early with errors about `SharedArrayBuffer` or `crossOriginIsolated`, see the COOP/COEP section below.

---

### GPU golden-image tests (Playwright)

The repo includes small deterministic graphics microtests that compare rendered output against committed PNG
goldens under `tests/golden/`.

Regenerate/update the CPU-generated goldens:

```bash
pnpm install --frozen-lockfile
pnpm run generate:goldens
```

Or run the same “regenerate + fail on drift” check that CI uses:

```bash
pnpm install --frozen-lockfile
pnpm run check:goldens
```

Run the GPU golden tests (Playwright):

```bash
pnpm run test:gpu
```

CI enforces that `pnpm run generate:goldens` does **not** produce changes under `tests/golden/`. If CI fails with a
golden diff, rerun the generator locally and commit the updated PNGs.

---

### COOP/COEP + `crossOriginIsolated` (SharedArrayBuffer / WASM threads)

#### Why this matters

Aero relies on **WASM threads** and shared memory for performance (e.g. CPU emulation in Web Workers with `Atomics`). Browsers only expose `SharedArrayBuffer` in a **cross-origin isolated** context, which requires COOP/COEP headers.

See also:

- [Deployment & Hosting (COOP/COEP)](build-and-tooling.md)
- [the cross-origin isolation decision: Cross-origin isolation](../decisions/0002-cross-origin-isolation.md)
- [the WebAssembly build variants decision: WASM build variants (threaded vs single)](../decisions/0004-wasm-build-variants.md)

If your page is not cross-origin isolated:

- `window.crossOriginIsolated` will be `false`
- `SharedArrayBuffer` may be `undefined`
- `WebAssembly.Memory({ shared: true, ... })` will fail
- any thread-dependent code will fail or silently fall back to single-thread behavior (depending on implementation)

#### Required headers

Your dev server / test server must send:

- `Cross-Origin-Opener-Policy: same-origin`
- `Cross-Origin-Embedder-Policy: require-corp` (or `credentialless` if supported and appropriate)

#### How to verify (DevTools)

In the browser console:

```js
crossOriginIsolated
typeof SharedArrayBuffer
```

Expected:

- `crossOriginIsolated === true`
- `typeof SharedArrayBuffer === "function"`

Chrome also shows cross-origin isolation status in **DevTools → Security**.

#### Common causes of failure

- **Serving from `file://`**: COOP/COEP isolation requires a proper origin; open the app via a dev server (and usually a secure context). `http://localhost` is treated as secure, but arbitrary `http://` origins are not.
- **Missing headers in your server/proxy**: ensure the *final* server (including any reverse proxy) sets COOP/COEP headers on HTML and subresources as needed.
- **Blocked cross-origin subresources under COEP**: with `Cross-Origin-Embedder-Policy: require-corp`, the browser will block cross-origin scripts/fonts/images that do not explicitly opt in via CORS or `Cross-Origin-Resource-Policy` headers. Symptoms show up as red errors in the console/network panel.
 - Fix by self-hosting assets, adding proper CORS, or using resources that send the correct headers.

---

### WebGPU testing policy

#### Why WebGPU tests are gated in CI

WebGPU availability varies across:

- runners (most CI VMs do not have stable GPU access)
- operating systems / driver stacks
- headless browser configurations

To keep CI reliable, tests that **require** WebGPU are typically **skipped** unless explicitly requested. Tests should either:

- run with a non-WebGPU fallback (e.g. WebGL2) in default CI, or
- be conditionally enabled only when WebGPU is available and required

In Playwright, WebGPU-required tests are tagged with `@webgpu` and isolated in a
dedicated Playwright project, `chromium-webgpu` (see `playwright.config.ts`).
Default projects (`chromium`, `firefox`, `webkit`) exclude `@webgpu` tests to
keep PR CI stable on runners where WebGPU is missing or unreliable.

#### Forcing WebGPU-required tests

Set `AERO_REQUIRE_WEBGPU=1` to make WebGPU a hard requirement:

```bash
# E2E (Playwright, WebGPU-only project)
AERO_REQUIRE_WEBGPU=1 pnpm run test:webgpu

# Or run the project directly
AERO_REQUIRE_WEBGPU=1 pnpm exec playwright test --project=chromium-webgpu

# Unit tests that exercise WebGPU-dependent paths (if applicable)
AERO_REQUIRE_WEBGPU=1 pnpm run test:unit
```

Expected behavior when `AERO_REQUIRE_WEBGPU=1` is set:

- tests **fail** (rather than skip/fallback) if `navigator.gpu` is missing or cannot create a device
- CI jobs that do not provide WebGPU will fail, by design

#### CI workflows

- `.github/workflows/ci.yml` (PR CI) sets `AERO_REQUIRE_WEBGPU=0` and runs the
 non-WebGPU Playwright projects.
- `.github/workflows/webgpu.yml` (schedule + workflow_dispatch) sets
 `AERO_REQUIRE_WEBGPU=1` and runs the `chromium-webgpu` project; it uploads
 Playwright reports/artifacts so WebGPU regressions are actionable.

---

### CI behavior (what runs where)

> **These workflows no longer exist.** Continuous integration was purged and is
> on the ban list — see [the working agreements](../meta/working-agreements.md).
> What follows describes the jobs as they were configured, which is still the
> best record of *what should be run and in what combination*; run them by
> hand. Nothing here runs automatically.

CI should be reproducible locally with the same top-level commands:

- Full stack (recommended): `cargo xtask test-all` (or `bash ./scripts/test-all.sh` wrapper)
- Rust: `cargo test --locked --workspace`
- WASM tests (Node): `wasm-pack test --node`
- WASM builds (single + threaded): `pnpm -C apps/web run wasm:build`
 - Threaded builds require nightly + `rust-src` (see the WebAssembly build variants decision).
- TypeScript unit tests: `pnpm run test:unit` (often with coverage enabled)
- Browser E2E: `pnpm run test:e2e`

Tip: if you need to open a PR or check CI status in an environment without GitHub CLI (`gh`), use:

```bash
pnpm run pr:url # compare/PR URL
pnpm run pr:links # compare/PR URL + GitHub Actions URL
```

#### Cross-browser Playwright policy (PR vs scheduled)

Playwright E2E coverage is split into two workflows:

- **PR CI (fast):** `.github/workflows/ci.yml` runs Playwright in **Chromium only**
 (`--project=chromium`) to keep pull request feedback fast.
- **Cross-browser E2E (high confidence):** `.github/workflows/e2e-matrix.yml` runs a
 matrix over **Chromium + Firefox + WebKit** on a nightly schedule and via
 `workflow_dispatch`. This workflow is intended to catch browser-specific
 regressions without blocking PR merges.
 - When the scheduled run fails, the workflow opens/updates a single tracking
 issue so failures are visible outside of Actions.

Environment variables commonly affect CI behavior:

- `AERO_REQUIRE_WEBGPU=1`: require WebGPU (see above)

When debugging a CI failure locally, prefer matching the CI environment as closely as possible:

- use `pnpm install --frozen-lockfile` (not a plain install) for deterministic dependency resolution
- run Playwright in headless mode (default) unless you specifically need `--ui`

---

### Win7 virtio portable tests (C, hardware-free)

The Win7 virtio driver stack includes a small **portable C99** module that parses the PCI
capability list for Virtio 1.0 "modern" devices. It is unit-tested using synthetic PCI
config-space images (runs on Linux CI; no hardware required).

From the repo root:

```bash
bash ./drivers/win7/virtio/tests/build_and_run.sh
```

Optionally select a compiler:

```bash
CC=clang bash ./drivers/win7/virtio/tests/build_and_run.sh
```

---

### Rust microbenchmarks (Criterion)

We use [Criterion.rs](https://github.com/bheisler/criterion.rs) to measure a
small set of emulator-critical hot paths with stable statistics.

Current benchmarks:

- `decoder_throughput`: legacy string-op decoder throughput (`aero_cpu_core::interp::decode`, `legacy-interp`)
- `x86_decode_throughput`: instruction decode throughput via the `aero_x86` wrapper (iced-x86)
- `interpreter_hot_loop`: legacy interpreter dispatch loop (`aero_cpu_core::interp::exec`, `legacy-interp`)
- `memory/bulk_copy_1mib`: legacy `RamBus` bulk copy throughput (1 MiB, `legacy-interp`)

Note: the canonical interpreter is `aero_cpu_core::interp::tier0`; the current Criterion bench target is kept behind `--features legacy-interp`.

#### Run the emulator-critical microbenchmarks

```bash
# Full (slower, more stable)
cargo bench --locked -p aero-cpu-core --bench emulator_critical --features legacy-interp -- --noplot
```

Criterion writes results to `target/criterion/`.

In CI we move `target/criterion` into `target/bench-*/criterion` so the base/head
runs don't overwrite each other.

#### CI / PR profile (fast)

In CI we run a shorter benchmark configuration to keep PR runtime low:

```bash
AERO_BENCH_PROFILE=ci cargo bench --locked -p aero-cpu-core --bench emulator_critical --features legacy-interp -- --noplot
```

#### Run the JIT bookkeeping microbenchmarks

`jit_bookkeeping` benchmarks core JIT runtime bookkeeping overhead (CodeCache LRU maintenance,
HotnessProfile under capacity pressure, and PageVersionTracker snapshotting).

```bash
# Default profile (moderate; good for local runs)
cargo bench --locked -p aero-cpu-core --bench jit_bookkeeping -- --noplot

# CI/PR profile (fast)
AERO_BENCH_PROFILE=ci cargo bench --locked -p aero-cpu-core --bench jit_bookkeeping -- --noplot

# Full profile (slower, more stable)
AERO_BENCH_PROFILE=full cargo bench --locked -p aero-cpu-core --bench jit_bookkeeping -- --noplot
```

### Benchmark regression CI

The workflow `.github/workflows/bench.yml` runs these microbenchmarks and fails
on regressions:

- `aero-cpu-core/benches/emulator_critical` (legacy-interp)
- `aero-cpu-core/benches/jit_bookkeeping`

- **pull_request**: benchmarks the PR base commit and the PR head commit (same
 runner), then compares results. The workflow fails if any benchmark slows down
 by more than the configured threshold (see `bench/perf_thresholds.json` → `profiles.pr-smoke.criterion.maxRegressionPct`).
- **schedule / workflow_dispatch**: runs the suite on `main`, compares against
 the previous successful `main` run artifact, and uploads the current results
 as the new baseline artifact (`criterion`).

Regression detection uses Criterion's 95% confidence intervals to avoid flakey
failures on noisy CI runners (it only fails when the slowdown is both above the
threshold and statistically significant).

Artifacts:

- `bench-pr` contains both PR base + head Criterion results plus the comparison report.
- `criterion` is the moving baseline artifact for `main` (only updated on successful runs).
- `criterion-run-<run_id>` is uploaded for every `main` run to aid debugging.

#### Manual comparison

You can compare two Criterion output directories locally:

```bash
python3 scripts/bench_compare.py \
 --base path/to/base/criterion \
 --new path/to/new/criterion \
 --thresholds-file bench/perf_thresholds.json \
 --profile pr-smoke
```

## Fixtures & Test Assets Policy

This repository must **not** distribute proprietary operating system media (especially Microsoft Windows), BIOS/firmware dumps, proprietary drivers, or other copyrighted binaries.

In addition, large binary fixtures quickly bloat git history and slow down CI. To reduce this risk, CI enforces a repository policy check.

For local-only fixtures (downloaded OSS images, user-supplied Windows media), use the gitignored `test-images/` directory (see `test-images/README.md`).

### What is NOT allowed in-repo

Do not commit files that are (or look like) OS installation media, disk images, or Windows binaries, including (non-exhaustive):

- Disk/VM images: `.iso`, `.img`, `.vhd`, `.vhdx`, `.vmdk`, `.qcow`, `.qcow2`, `.wim`
- Windows binaries: `.exe`, `.dll`
- Anything under Windows fixture directories such as `*/test_images/windows*` or `*/fixtures/windows*`

#### Guest-code fixtures, and where the line is

Differential tests want *real* guest instruction sequences, not hand-written
approximations of them — several defects were only ever caught that way,
including a missing add-with-carry in the compiled tier that produced a
wrong-but-plausible 4096-bit signature check. That means a small amount of
compiled Windows code genuinely earns its place as test input.

The line drawn is **excerpt, not image**:

- **Kept:** two single compiled functions, about 3 KB each — the SHA-256 and
  SHA-512 compression rounds. They are pinned by blob hash in
  `scripts/ci/check-repo-policy.sh` so they cannot silently change, and their
  origin is stated in `crates/aero-cpu-core/tests/data/README.md`. They are not
  ours, and the tree says so.
- **Not kept:** the ~92 KB mapped image of the same DLL. That is not an excerpt,
  it is most of a library. It is gitignored; tests needing it read
  `AERO_CRYPTSP_IMAGE` or the conventional path and **skip when it is absent**
  rather than failing.

The cost of that split is close to zero here — verification is run by hand, on
machines that already have the install media — so it is worth taking.

Published algorithm constants should not be blobs at all. The SHA-2 round tables
used to be binary files and are now generated in
`crates/aero-cpu-core/src/sha2_constants.rs`, with a test that re-derives them
from the FIPS definition using integer arithmetic, so a typo cannot hide.

> A related hazard, found while doing this: `.gitignore` blanket-ignores `*.bin`
> and re-admits specific directories by negation. Production code was
> `include_bytes!`-ing a `.bin` that fell outside every negation — so it was
> never committed, and a fresh clone could not compile that crate. If you add a
> binary include, check `git check-ignore` on the target.

#### Allowlisted tiny binary fixtures

Some binary blobs are intentionally kept in-repo because they are **our own**
small, deterministic test fixtures:

- `assets/bios.bin` (generated fixture; canonical ROM comes from `firmware::bios::build_bios_rom()`; see [`assets/README.md`](../decisions/README.md); regenerate via `cargo xtask bios-rom`, verify with `cargo xtask bios-rom --check`)
- `crates/firmware/acpi/dsdt.aml` and `crates/firmware/acpi/dsdt_pcie.aml` (ACPI DSDT AML built from this repo; regenerate via `cargo xtask fixtures` (legacy-only alternative: `cargo run -p firmware --bin gen_dsdt --locked`))
 - Clean-room, human-readable references live alongside as `crates/firmware/acpi/dsdt.asl` and `crates/firmware/acpi/dsdt_pcie.asl`, and are kept in sync with the shipped AML blobs (see `scripts/verify_dsdt.sh`).
- `tests/fixtures/boot/*.bin` and `tests/fixtures/boot/*.img` (boot sector + tiny disk images generated by `cargo xtask fixtures`)
- `tests/fixtures/bootsector.bin` and `tests/fixtures/realmode_vbe_test.bin` (tiny boot/test programs generated by `cargo xtask fixtures`; `.asm`/`.s` sources live alongside as documentation)
- `tests/fixtures/boot/int_sanity.bin` (tiny BIOS interrupt sanity boot sector generated by `cargo xtask fixtures`; `int_sanity.asm` is kept as documentation/reference)
- `tools/qemu_diff/boot/boot.bin` (QEMU-diff boot sector generated by `cargo xtask fixtures`; assembly source `boot.S` lives alongside as documentation; used by the `qemu-diff` Cargo package)

These paths are explicitly allowlisted by `scripts/ci/check-repo-policy.sh` and must remain small and reproducible.

In addition, a handful of **tiny text placeholders** with normally-forbidden Windows driver extensions are kept
in-repo for packaging tests:

- `tools/packaging/aero_packager/testdata/drivers/**/test.sys`
- `tools/packaging/aero_packager/testdata/drivers/**/test.dll`
- `tools/packaging/aero_packager/testdata/drivers/**/WdfCoInstaller*.dll` (KMDF coinstaller placeholder)

These are **not real Windows binaries**; CI pins their blob hashes in `scripts/ci/check-repo-policy.sh` to prevent
silent drift into proprietary artifacts.

### Size limits

New/changed blobs should generally stay **under 20MB**. If you need larger assets for tests, prefer:

- Generating fixtures at runtime (e.g., create minimal disk images during tests).
- Downloading fixtures as part of local-only setup or CI setup from an approved external source (public OSS mirror or private bucket).

If a fixture truly needs to live in-repo despite policy checks (for size or filetype), it must be explicitly allowlisted in `scripts/ci/check-repo-policy.sh` with a clear justification. Filetype exceptions should be pinned (e.g., by blob hash) so they cannot silently drift into proprietary binaries.

In addition, allowlisted disk/firmware-like blobs (the `.bin`/`.img` fixtures above) are held to a **much stricter** size limit in CI (1 MiB each).

### Golden images (synthetic, committed)

The `tests/golden/` directory contains small, synthetic golden PNGs used for graphics regression tests.

Some of these images are generated by `pnpm run generate:goldens`, and CI enforces that the checked-in PNGs
match the generator output (contributors must regenerate + commit when changing the underlying scene code).

### Adding permissible fixtures (open source / small)

When adding a fixture that is safe to store in-repo:

1. Ensure the asset is **open-source / redistributable** and include provenance (source URL + license).
2. Keep it small, stable, and deterministic (avoid generated blobs when a tiny textual representation is possible).
3. Prefer placing fixtures in a dedicated directory with a short README describing:
 - Where it came from
 - License
 - Hash (SHA256) for integrity

### CI enforcement

CI runs `scripts/ci/check-repo-policy.sh` on PRs and pushes. If it fails, remove the disallowed file(s) and use one of the alternatives above.

In addition, CI enforces **determinism** for the allowlisted tiny binary fixtures by running:

```bash
cargo xtask fixtures --check
```

This fails if any generated fixture is missing or out-of-date (regenerate via `cargo xtask fixtures`).

## Manual testing checklists

This directory contains **manual**, **reproducible** smoke-test procedures for behaviors that are hard to validate automatically.

Guidelines:

- Keep steps deterministic and copy/paste-friendly.
- Prefer built-in guest OS actions (e.g. Windows Control Panel tests) over external tools.
- Do **not** commit proprietary artifacts (Windows images/ISOs, driver binaries, etc.). Link to existing repo guidance instead.

End-to-end subsystem validation plans (device model through guest driver to web
runtime) live in the relevant area page — see [usb-and-input.md](usb-and-input.md)
for the virtio-input example.

### Checklists

- Windows 7 audio (in-box HD Audio / Intel HDA): [`audio-windows7.md`](audio.md)

## End-to-end test plans

This directory contains **single-document, end-to-end** test plans that tie together:

- the Rust device model (host-side conformance tests),
- guest drivers (Windows 7 bring-up + automated harness), and
- browser/web runtime validation (routing + integration checks).

These are intended to be copy/paste-friendly “do these steps” docs that a contributor can follow without reading source code.

### Index

- virtio-input (device model + Win7 driver + web runtime): [`virtio-input.md`](windows-drivers.md)

## Agent Resource Limits & Concurrency Guide

**Audience:** Coding agents developing Aero (not end-users or CI runners).

**Goal:** Maximum speed without OOM-killing the shared host.

---

### 🛡️ Core Principle: Assume Hostile Processes

**Every process you spawn is potentially hostile, pathological, or malfunctioning.**

This isn't paranoia—it's operational reality. `cargo`, `rustc`, `node`, `npm`, `chromium`, and even simple shell commands can:

| Failure Mode | Example | Your Defense |
|--------------|---------|--------------|
| **Hang forever** | rustc stuck on codegen, npm waiting for network | Always use timeouts |
| **Consume infinite memory** | LTO linking, webpack bundling | Always use memory limits |
| **Spin CPU forever** | Infinite loop in proc macro, bad regex | Timeouts + kill |
| **Ignore SIGTERM** | Misbehaving Chrome process | SIGTERM → wait → SIGKILL |
| **Leave zombies** | Crashed parent, orphaned children | Kill process groups, not just PIDs |
| **Corrupt state** | Partial writes, lock files | Validate outputs, clean state |
| **Lie about success** | Exit 0 but wrong output | Check outputs, not just exit codes |

**Your code is not special.** It will also misbehave. Defend against yourself.

---

### The One Rule That Matters

**Memory is the constraint.** CPU and disk I/O are handled gracefully by the Linux scheduler under contention. But if 200 agents each try to use 16 GB during a Rust build, the machine will OOM and become unresponsive.

**Target:** ~6-8 GB typical, 12 GB hard ceiling per agent.

Everything else in this doc is secondary to this.

---

### Quick Setup

Run this once per checkout (sanity checks and prints activation instructions):

```bash
# From repo root
bash ./scripts/agent-env-setup.sh
```

> Troubleshooting: in some agent environments, the working tree can lose executable bits and/or be missing tracked fixtures.
>
> - If you get `Permission denied` running `./scripts/*.sh`, run via bash: `bash ./scripts/agent-env-setup.sh` / `bash ./scripts/safe-run.sh …`.
> - If `git status` shows many mode-only changes or deleted/empty tracked files, restore the checkout:
> - `git checkout -- .` (bigger hammer), or at least:
> - `git checkout -- scripts tools/packaging/aero_packager/testdata tools/disk-streaming-browser-e2e/fixtures`
> - Non-git fallback: `find scripts -name '*.sh' -exec chmod +x {} +`

Then activate the recommended environment in your current shell:

```bash
source ./scripts/agent-env.sh
```

---

### Memory Limit Enforcement

We use RLIMIT_AS (virtual address space limit) via `prlimit` or `ulimit`. This is simpler and more portable than cgroups/systemd-run.

#### RLIMIT_AS caveat (Node/V8/WebAssembly)

RLIMIT_AS limits **virtual address space**, not resident memory (RSS). Some runtimes reserve large virtual ranges up-front (especially Node/V8 and WebAssembly memories). Under the default `12G` cap, you may see spurious failures like:

- `WebAssembly.Instance(): Out of memory: Cannot allocate Wasm memory for new instance`
- `WebAssembly.Memory(): could not allocate memory`

Note: `scripts/safe-run.sh` will **auto-bump** its default address-space cap for **Node/WASM-heavy test entrypoints** (and Playwright) when `AERO_MEM_LIMIT` is unset. This is why you may see `Memory: 256G` in safe-run logs even though the general default is `12G`.

If you hit this while running JS/TS tooling (for example `pnpm -C apps/web run test:unit`), re-run with a larger address-space limit for that command (or disable it) while keeping the timeout:

```bash
# Try raising the cap first
AERO_TIMEOUT=600 AERO_MEM_LIMIT=32G bash ./scripts/safe-run.sh pnpm -C apps/web run test:unit

# If it still fails, disable RLIMIT_AS for that command (still keeps the timeout)
AERO_TIMEOUT=600 AERO_MEM_LIMIT=unlimited bash ./scripts/safe-run.sh pnpm -C apps/web run test:unit

# Alternative (no address-space limit, but keep a timeout):
bash ./scripts/with-timeout.sh 600 bash ./scripts/run_limited.sh --no-as -- pnpm -C apps/web run test:unit
```

#### RLIMIT_AS caveat (rustc/LLVM)

Rust builds can also consume large **virtual address space** (via mmap/allocators), even when the
actual resident memory usage is reasonable. In some sandboxes this can surface as rustc panics like:

- `failed to spawn helper thread: Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }`
- `thread 'rustc' panicked at 'called Result::unwrap() on an Err value: Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }'`

If you hit this under `safe-run.sh`, re-run the command with a larger address-space limit:

```bash
# Common fix for large Cargo builds/tests
AERO_TIMEOUT=900 AERO_MEM_LIMIT=32G bash ./scripts/safe-run.sh cargo test --locked

# If your sandbox allows it, you can also disable RLIMIT_AS for that command:
AERO_TIMEOUT=900 AERO_MEM_LIMIT=unlimited bash ./scripts/safe-run.sh cargo test --locked
```

#### Linker caveat (lld threads + wasm `RUSTFLAGS` gotcha)

On Linux, the pinned Rust toolchain links via **LLVM lld** (`-fuse-ld=lld`). lld defaults to using
all available hardware threads and can hit per-user thread limits under shared-host contention.

To keep builds reliable, `scripts/agent-env.sh` and `scripts/safe-run.sh` cap lld’s parallelism via
Cargo’s **per-target rustflags** environment variables:

```
CARGO_TARGET_<TRIPLE>_RUSTFLAGS="... -C link-arg=-Wl,--threads=<n>"
```

For **wasm32** targets, rustc invokes `rust-lld -flavor wasm` directly, so the native `-Wl,` prefix
is invalid. Use the wasm-compatible form:

```
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="... -C link-arg=--threads=<n>"
```

Avoid setting linker thread caps via **global `RUSTFLAGS`** (or `CARGO_ENCODED_RUSTFLAGS`), e.g.:

```bash
# ⚠️ Avoid: breaks wasm builds because rust-lld doesn't understand -Wl,
export RUSTFLAGS="-C link-arg=-Wl,--threads=1"
```

If you need to change the cap, prefer adjusting build parallelism instead:

```bash
export AERO_CARGO_BUILD_JOBS=2
source ./scripts/agent-env.sh
```

#### Using `run_limited.sh` (Recommended)

```bash
# Limit to 12GB virtual address space
bash ./scripts/run_limited.sh --as 12G -- cargo build --release --locked

# Or use safe-run.sh which combines timeout + memory limit
bash ./scripts/safe-run.sh cargo build --release --locked
```

#### How it works

1. **`prlimit`** (preferred): Sets RLIMIT_AS on the current process, inherited by children
2. **`ulimit -v`** (fallback): Same effect via shell builtin

This approach:
- Works in containers (no cgroups/systemd needed)
- Works on most Linux systems without root
- Works on macOS (via ulimit)
- Handles Rustup shims correctly (resolves to real cargo binary first)

#### Soft limits (reduce peak usage)

These don't enforce hard limits but reduce memory spikes:

```bash
export CARGO_BUILD_JOBS=1 # Limit parallel rustc (agent default; raise if your sandbox allows)
export RUSTC_WORKER_THREADS=1 # Limit rustc's internal worker pool (avoid "WouldBlock" rustc ICEs)
export RAYON_NUM_THREADS=1 # Keep rustc/Rayon pools aligned with Cargo parallelism
export RUST_TEST_THREADS=1 # Limit Rust's built-in test harness parallelism (libtest)
export NEXTEST_TEST_THREADS=1 # Limit cargo-nextest test concurrency (if using cargo nextest)
export AERO_TOKIO_WORKER_THREADS=1 # Limit Tokio runtime worker threads for supported Aero binaries
export AERO_RUST_CODEGEN_UNITS=1 # Optional: reduce per-crate parallelism (slower, but can help under tight thread/process limits); alias: AERO_CODEGEN_UNITS
```

`bash ./scripts/safe-run.sh` also includes a small backoff + retry loop for Rust build/test commands
(Cargo and common wrappers like `npm`/`wasm-pack`) when it detects transient `rustc` thread-spawn
panics (e.g. `failed to spawn helper thread (WouldBlock)` or `called Result::unwrap() on an Err value: Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }`), which can happen when many agents share
the same host. Override with
`AERO_SAFE_RUN_RUSTC_RETRIES=1` to disable retries.

---

### Recommended Build Settings

These balance speed with reasonable memory usage. They're defaults, not hard constraints—override if you know what you're doing.

#### Common agent-sandbox failures (and what to do)

On shared hosts running many agents concurrently, Linux per-user process/thread limits can be hit even when
`CARGO_BUILD_JOBS=1`. When that happens you may see transient errors like:

- `Resource temporarily unavailable (os error 11)`
- `fork: retry: Resource temporarily unavailable`
- rustc panic: `failed to spawn helper thread (WouldBlock)`
- rustc panic: `Unable to install ctrlc handler: ... WouldBlock (Resource temporarily unavailable)`
- rustc panic: `called Result::unwrap() on an Err value: Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }`

These are typically **environment/resource-limit** issues, not code bugs. The best remediation is to:

1. Wait with backoff (a few seconds → tens of seconds)
2. Re-run the command (still using `safe-run.sh`)

#### Cargo config (`.cargo/config.toml`)

This repo tracks `.cargo/config.toml` for the `cargo xtask` alias, and it is kept intentionally minimal so CI isn't affected by agent-only settings.

Recommended memory-friendly Cargo settings live in environment variables (next section), not in the repo-tracked Cargo config.

#### Environment (source `scripts/agent-env.sh`)

```bash
# Rust
# Cargo parallelism is defaulted to `-j1` for reliability in constrained sandboxes.
# Override by setting `AERO_CARGO_BUILD_JOBS` before sourcing `scripts/agent-env.sh`.
export CARGO_BUILD_JOBS=1
export RUSTC_WORKER_THREADS=1 # Limit rustc internal worker threads (reliability under contention)
export RAYON_NUM_THREADS=1 # Keep rayon pools aligned with Cargo parallelism
export RUST_TEST_THREADS=1 # Limit libtest parallelism (helps when per-user thread limits are tight)
export NEXTEST_TEST_THREADS=1 # Limit cargo-nextest test concurrency (if using cargo nextest)
export AERO_TOKIO_WORKER_THREADS=1 # Limit Tokio runtime worker threads for supported Aero binaries
export CARGO_INCREMENTAL=1

# Node (if running JS/TS tooling)
export NODE_OPTIONS="--max-old-space-size=4096"

# If your environment doesn't have the repo's pinned Node version from `.nvmrc`,
# you can bypass the hard error (it will still warn):
export AERO_ALLOW_UNSUPPORTED_NODE=1

# Playwright (if running browser tests)
export PW_TEST_WORKERS=1
```

---

### Timeouts (Non-Negotiable)

**Every command gets a timeout. No exceptions.**

Processes hang. Network calls block. Locks deadlock. Without timeouts, a single stuck process can block your entire session indefinitely.

```bash
# NEVER do this:
cargo build --locked # Can hang forever

# ALWAYS do this (use safe-run.sh for both timeout + memory limit):
bash ./scripts/safe-run.sh cargo build --locked

# Or just timeout:
timeout -k 10 600 cargo build --locked
bash ./scripts/with-timeout.sh 600 cargo build --locked
```

#### The `-k` flag is critical

**Always use `timeout -k <grace>` — never bare `timeout`.**

Misbehaving code can ignore SIGTERM indefinitely. The `-k 10` sends SIGKILL 10 seconds after SIGTERM if the process is still running.

```bash
# CORRECT — SIGKILL after 10s grace period:
timeout -k 10 600 cargo build --locked

# WRONG — process can ignore SIGTERM forever:
timeout 600 cargo build --locked
```

#### Recommended Timeouts

| Operation | Timeout | Rationale |
|-----------|---------|-----------|
| `cargo build` (debug) | 10 min | Should complete in 2-5 min normally |
| `cargo build --release` | 20 min | LTO/optimization takes longer |
| `cargo test` | 10 min | Tests shouldn't take forever |
| `npm install` | 5 min | Network can be slow, but not infinite |
| `pnpm run build` | 10 min | Bundling is finite |
| Playwright tests | 5 min per test | Browser can hang |
| Any network request | 30 sec | DNS/connect/read timeouts |

#### What Happens on Timeout

1. SIGTERM sent to the process
2. 10 second grace period for cleanup
3. SIGKILL if still running (non-negotiable)

**If something times out, it's a bug or a hang.** Investigate—don't just increase the timeout.

---

### Killing Processes: Do It Right

When something goes wrong, kill it properly. Half-killed processes are worse than running processes.

#### Kill a Process Group (Preferred)

```bash
# Kill the entire process tree, not just the parent
kill -TERM -$PGID # SIGTERM to process group
sleep 2
kill -KILL -$PGID # SIGKILL if still alive
```

#### Find and Kill Orphans

 ```bash
 # Find processes using excessive memory
 ps aux --sort=-%mem | head -20

 # Find your orphaned cargo/rustc processes
 pgrep -u $(whoami) -f 'cargo|rustc|node|chrome' | xargs -r ps -p

 # Kill a specific PID (choose from the pgrep output above)
 kill -TERM <PID>
 sleep 2
 kill -KILL <PID>
 ```

#### Clean Up Lock Files

Crashed processes leave locks. Remove them:

```bash
# Cargo build directory lock
rm -f target/.cargo-lock

# npm lock
rm -f package-lock.json.lock node_modules/.package-lock.json

# OPFS/IndexedDB (browser storage) - clear via browser devtools or fresh profile
```

---

### Validating Outputs (Trust Nothing)

**Exit code 0 does not mean success.** Verify:

```bash
# BAD: Assumes success
cargo build --locked
./target/debug/mybin

# GOOD: Verify the artifact exists and is valid
cargo build --locked
if [[ ! -x ./target/debug/mybin ]]; then
 echo "ERROR: Build claimed success but binary missing" >&2
 exit 1
fi
./target/debug/mybin --version || { echo "Binary crashes on --version" >&2; exit 1; }
```

#### Common "Successful Failures"

| Tool | Silent Failure Mode | How to Detect |
|------|---------------------|---------------|
| `cargo build` | Partial build, missing artifact | Check file exists + is executable |
| `wasm-pack` | Missing `.wasm` file | Check `pkg/*.wasm` exists |
| `pnpm run build` | Empty dist folder | Check `dist/` is non-empty |
| `cargo test` | Some tests skipped silently | Parse test output for skip count |
| `playwright test` | Flaky pass after retries | Check retry count in output |

---

### What NOT to Worry About

- **CPU contention**: The scheduler handles this. Don't reduce parallelism purely due to CPU contention — but note some agent sandboxes have low thread/process limits; `scripts/agent-env.sh` defaults to `CARGO_BUILD_JOBS=1` for stability (override via `AERO_CARGO_BUILD_JOBS`).
- **Disk I/O**: NVMe + Linux I/O scheduler handles contention fine. No need for `ionice` or I/O limits.
- **Disk space**: 110 TB is plenty. Clean up your target dirs occasionally but don't stress.
- **Network**: Not a factor for local development.

---

### When Memory Spikes Happen

Common memory-hungry operations:

| Operation | Typical Peak | Mitigation |
| ----------------------- | ------------ | ----------------------------------- |
| `cargo build --release --locked` | 8-16 GB | cap Cargo parallelism (agent default: `CARGO_BUILD_JOBS=1`) + memory limit |
| `cargo build --locked` (debug) | 4-8 GB | Usually fine |
| `wasm-pack build` | 4-8 GB | Usually fine |
| Playwright + Chrome | 2-4 GB | `PW_TEST_WORKERS=1` |
| `cargo doc` | 4-8 GB | Run alone if needed |
| Linking large binaries | 4-8 GB | lower codegen parallelism (`-C codegen-units=1`) helps |

If you're doing something unusual (like building with `-j16`), wrap it in a memory limit.

---

### Troubleshooting

#### rustc fails to spawn helper threads ("Resource temporarily unavailable")

In heavily constrained sandboxes (especially when building `wasm32-unknown-unknown` + `wasm-threaded`), Rust may fail to create its internal thread pools and ICE with errors like:

```text
failed to spawn helper thread: Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }
```

Or it may surface as a panic/unwrap error (newer rustc versions):

```text
thread 'rustc' panicked at 'called Result::unwrap() on an Err value: Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }'
```

Mitigations (start with the top-most):

```bash
# Limit how many rustc processes Cargo runs in parallel.
export CARGO_BUILD_JOBS=1

# rustc uses Rayon internally; keep its pool small as well.
export RAYON_NUM_THREADS=1

# If you still hit thread-spawn failures in debug/dev builds, also reduce per-crate
# codegen parallelism (especially helpful for threaded wasm builds).
export CARGO_PROFILE_DEV_CODEGEN_UNITS=1
```

Example (threaded WASM dev build of the core package):

```bash
CARGO_BUILD_JOBS=1 RAYON_NUM_THREADS=1 CARGO_PROFILE_DEV_CODEGEN_UNITS=1 \
 node apps/web/scripts/build_wasm.mjs threaded dev --packages core
```

#### Build was killed unexpectedly

Probably OOM. Check with:

```bash
dmesg | tail -20 | grep -i oom
```

Retry with:

```bash
bash ./scripts/run_limited.sh --as 12G -- cargo build --locked
```

#### Cargo says "Blocking waiting for file lock on package cache"

If `cargo` appears stuck and repeatedly prints:

```
Blocking waiting for file lock on package cache
```

it usually means another `cargo` process on the shared host is currently
updating/using the global Cargo registry cache (common when many agents run
builds concurrently). This is typically transient: once the other `cargo`
finishes its download/unpack step, the lock is released and your command
continues.

Mitigations:

- **Wait** (best default). The lock should clear on its own.
- **Stagger heavy Cargo runs** across agents (avoid everyone starting a fresh
 build at once).
- If you need full isolation, run with a **per-checkout `CARGO_HOME`** (at the
 cost of duplicating cache data / doing your own downloads):

 ```bash
 CARGO_HOME="$PWD/.cargo-home" cargo build --locked
 ```

 If you are running via `safe-run.sh`, you can opt into the same behavior without
 sourcing `scripts/agent-env.sh`:

 ```bash
 AERO_ISOLATE_CARGO_HOME=1 bash ./scripts/safe-run.sh cargo build --locked
 ```

 If you are using the agent env helper, you can opt into the same behavior:

 ```bash
 export AERO_ISOLATE_CARGO_HOME=1
 # Or pick a custom directory:
 # export AERO_ISOLATE_CARGO_HOME="/tmp/aero-cargo-home"
 source ./scripts/agent-env.sh
 ```

 Note: this intentionally overrides any existing `CARGO_HOME` so the isolation
 actually takes effect.

#### Cargo says "Blocking waiting for file lock on build directory"

If `cargo` prints:

```
Blocking waiting for file lock on build directory
```

it means **another Cargo process is currently using this checkout's build output directory** (typically `target/`).
This is different from the package cache lock above (which is about `CARGO_HOME` / registry state shared across
agents).

Common causes:

- You started two `cargo` commands in parallel in the same checkout.
- An IDE/background task is running `cargo check` continuously.
- A previous `cargo` invocation is still running (or got stuck) in this repo.

Mitigations:

- **Wait** for the other build to finish (best default).
- If you need to run multiple builds concurrently, use a **separate `CARGO_TARGET_DIR`** per command:

 ```bash
 CARGO_TARGET_DIR="$PWD/target-alt" cargo test --locked -p <crate>
 ```

 (This uses more disk space, but avoids the lock contention.)

#### rustc wrapper errors (`sccache`, etc)

Some environments configure a rustc wrapper (most commonly `sccache`) either via `~/.cargo/config.toml`:

```toml
[build]
rustc-wrapper = "sccache"
```

or via environment variables (`RUSTC_WRAPPER=sccache`, `RUSTC_WORKSPACE_WRAPPER=sccache`, etc).

If the wrapper daemon/socket is unhealthy, Cargo can fail with errors like:

```
sccache: error: failed to execute compile
```

Mitigations:

- **Disable wrappers for the command**:
 ```bash
 RUSTC_WRAPPER= RUSTC_WORKSPACE_WRAPPER= \
 CARGO_BUILD_RUSTC_WRAPPER= CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER= \
 cargo test --locked
 ```
- Or, when using `safe-run.sh` (recommended), note that it clears *environment-based* `sccache`
 wrappers by default. To force-disable wrappers (including those injected via Cargo config),
 set:
 ```bash
 AERO_DISABLE_RUSTC_WRAPPER=1 bash ./scripts/safe-run.sh cargo test --locked
 ```
- Or, when using the agent env helper:
 ```bash
 export AERO_DISABLE_RUSTC_WRAPPER=1
 source ./scripts/agent-env.sh
 ```

Notes:

- By default, `scripts/safe-run.sh` and `scripts/agent-env.sh` clear *only* `sccache` wrappers they
 see in the environment and preserve non-sccache wrappers (e.g. `ccache`). Use
 `AERO_DISABLE_RUSTC_WRAPPER=1` to force-disable all wrappers.

#### Build is very slow

You might be over-constrained. `scripts/agent-env.sh` defaults to `-j1` for reliability in constrained sandboxes; if your environment can handle more parallelism, override it:

```bash
export AERO_CARGO_BUILD_JOBS=2 # or 4, etc
source ./scripts/agent-env.sh
echo $CARGO_BUILD_JOBS
```

#### rustc panics with "failed to spawn helper thread" / "Resource temporarily unavailable"

In heavily contended agent sandboxes, `rustc` (or the linker driver it spawns) can fail with errors like:

- `failed to spawn helper thread (WouldBlock)`
- `failed to spawn work thread: Resource temporarily unavailable`
- `called Result::unwrap() on an Err value: Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }`
- `could not exec the linker \`cc\`: Resource temporarily unavailable`

These are usually **transient shared-host resource issues** (PID/thread limits), not deterministic build failures.

Mitigations:

- **Wait and retry** (best default).
- Ensure you're using minimal parallelism (`-j1` is the agent default):
 ```bash
 AERO_CARGO_BUILD_JOBS=1 bash ./scripts/safe-run.sh cargo test --locked
 ```
- If it still happens, reduce per-crate codegen parallelism further:
 ```bash
 AERO_RUST_CODEGEN_UNITS=1 bash ./scripts/safe-run.sh cargo test --locked
 # (alias): AERO_CODEGEN_UNITS=1 bash ./scripts/safe-run.sh cargo test --locked
 ```

#### "Too many open files"

```bash
ulimit -n 4096
```

---

### Helper Scripts

The `scripts/` directory contains:

| Script | Purpose |
| -------------------- | ------------------------------------------------- |
| `safe-run.sh` | **Recommended**: Run with both timeout + memory limit |
| `run_limited.sh` | Run a command with RLIMIT_AS memory limit |
| `with-timeout.sh` | Run a command with a timeout (uses `-k` for SIGKILL) |
| `agent-env.sh` | Source this to set recommended env vars |
| `agent-env-setup.sh` | One-time sanity checks + environment validation |

#### Quick Reference

```bash
# One-time setup (validates environment, shows warnings)
bash ./scripts/agent-env-setup.sh

# Activate environment in current shell
source ./scripts/agent-env.sh

# Run a build with full protection (RECOMMENDED)
bash ./scripts/safe-run.sh cargo build --release --locked

# Override defaults
AERO_TIMEOUT=1200 AERO_MEM_LIMIT=16G bash ./scripts/safe-run.sh cargo build --release --locked

# Just timeout (always use -k for SIGKILL fallback!)
timeout -k 10 600 cargo test --locked
bash ./scripts/with-timeout.sh 600 cargo test --locked

# Just memory limit (RLIMIT_AS)
bash ./scripts/run_limited.sh --as 12G -- cargo build --release --locked
```

---

### Headless / GPU-less Development (EC2, etc.)

Developing on headless GPU-less VMs (typical EC2 instances) works fine for most tasks. Here's what to know:

#### Works perfectly

- Rust/WASM compilation
- All CPU emulator tests
- WASM SIMD (uses CPU SSE/AVX)
- SharedArrayBuffer / COOP/COEP (just HTTP headers)
- Storage, networking, audio subsystem tests
- Most Playwright tests (headless Chrome/Firefox)

#### Friction: WebGPU is unavailable

On GPU-less systems, `navigator.gpu` is typically undefined even in headless Chrome. This means:

- **WebGPU tests will skip** (not fail) by default
- **WebGL2 fallback tests still run** (software rendered via llvmpipe/SwiftShader)
- **GPU golden-image tests may differ** from hardware baselines

**This is fine for most development.** The test harness gates WebGPU:

```bash
# Default: WebGPU tests skip if unavailable (GPU-less friendly)
pnpm run test:e2e

# Force WebGPU requirement (will fail on GPU-less systems)
AERO_REQUIRE_WEBGPU=1 pnpm run test:e2e
```

#### What you can't test locally on GPU-less

- WebGPU render correctness (pixel-perfect output)
- WebGPU performance characteristics
- GPU timestamp queries

**Workaround:** Push to a branch and let CI with GPU runners validate, or use a GPU-equipped dev machine for graphics work.

#### Software rendering (WebGL2)

WebGL2 works via software rendering (llvmpipe), but it's slow:

```bash
# These flags help headless Chrome use software GL
export PLAYWRIGHT_CHROMIUM_ARGS="--disable-gpu --use-gl=swiftshader"
```

Functional tests pass; performance is not representative.

#### Summary for GPU-less development

| Task | Works? |
| -------------------- | ------------------ |
| Build and compile | ✅ Yes |
| CPU emulator tests | ✅ Yes |
| Playwright (non-GPU) | ✅ Yes |
| WebGL2 smoke tests | ✅ Yes (slow) |
| WebGPU tests | ⚠️ Skip by default |
| GPU perf benchmarks | ❌ Meaningless |

---

### Safe Command Execution Patterns

#### The Fully Defensive Pattern

For any non-trivial command:

```bash
#!/bin/bash
set -euo pipefail

TIMEOUT=600
MEM_LIMIT=12G
CMD=(cargo build --release --locked)

echo "[run] Starting: ${CMD[*]}"
echo "[run] Timeout: ${TIMEOUT}s, Memory limit: $MEM_LIMIT"

# Capture both stdout and stderr, with timeout and memory limit
if ! AERO_TIMEOUT="$TIMEOUT" AERO_MEM_LIMIT="$MEM_LIMIT" bash ./scripts/safe-run.sh "${CMD[@]}" 2>&1 | tee build.log; then
 echo "[run] FAILED: ${CMD[*]}" >&2
 echo "[run] Last 50 lines of output:" >&2
 tail -50 build.log >&2
 exit 1
fi

# Verify output exists
if [[ ! -f target/release/aero ]]; then
 echo "[run] ERROR: Command succeeded but expected output missing" >&2
 exit 1
fi

echo "[run] SUCCESS: ${CMD[*]}"
```

#### Quick Defensive One-Liners

```bash
# Build with all protections
AERO_TIMEOUT=600 AERO_MEM_LIMIT=12G bash ./scripts/safe-run.sh cargo build --locked 2>&1 | tee build.log

# Test with timeout (tests should be fast)
bash ./scripts/with-timeout.sh 300 cargo test --locked 2>&1 | tee test.log

# npm with timeout (network can hang)
bash ./scripts/with-timeout.sh 300 pnpm install --frozen-lockfile 2>&1 | tee install.log
```

#### Recovering from Failures

When something fails:

1. **Check what's still running:**
 ```bash
 pgrep -u $(whoami) -af 'cargo|rustc|node|npm|chrome'
 ```

2. **Kill orphans:**
 ```bash
 # Use the output from step 1; be intentional and kill specific PIDs.
 kill -TERM <PID>
 sleep 2
 kill -KILL <PID>
 ```

3. **Clean corrupted state:**
 ```bash
 rm -rf target/.cargo-lock
 cargo clean -p <crate-that-failed>
 ```

4. **Retry with more visibility:**
 ```bash
 RUST_BACKTRACE=1 cargo build --locked -vv 2>&1 | tee verbose-build.log
 ```

---

### Windows 7 Test ISO

A Windows 7 Professional x64 ISO is available at:

```
/state/win7.iso
```

Use this for integration testing once the emulator can boot. Do not redistribute.

---

### Summary

1. **Assume hostility** — every process can hang, OOM, or misbehave
2. **Memory is the hard constraint** — use `run_limited.sh`/`safe-run.sh` for heavy builds
3. **Timeouts are mandatory** — no command runs without a deadline
4. **Verify outputs** — exit code 0 doesn't mean success
5. **Kill aggressively** — SIGTERM, wait, SIGKILL; clean up orphans
6. **Tune parallelism intentionally** — agent-env defaults to `-j1` for stability; increase via `AERO_CARGO_BUILD_JOBS` if your sandbox allows it
7. **GPU-less is fine** — WebGPU tests skip gracefully, WebGL2 works via software

## The GPU tests were skipping

A large body of graphics tests is conditional on a usable WebGPU adapter, and
skips cleanly when there is none — `require_gs_prepass_or_skip` and
`skip_if_compute_or_indirect_unsupported` in `crates/aero-d3d11/tests/common/`.
On a host without graphics libraries that means they never run, and a suite that
reports "ok" while quietly skipping its hardest tests is indistinguishable from
one that passed them.

Installing the browser test dependencies (`playwright install-deps`, which pulls
in Mesa and the GL libraries) gives wgpu a software adapter and switches those
tests on. Doing that surfaced a shader-generation bug immediately: the geometry
prepass emitted `const GS_INPUT_VERTS_PER_PRIM` twice — once from the runtime
vertex count and once from a stray Rust constant — which WGSL rejects outright.
Two code paths, neither of which had ever executed on a machine with an adapter.

That first bug was not the only one. Switching the adapter on turned a
comfortably green crate red, and the failures divided cleanly into two kinds.

**Defects in code that had never executed.** The duplicate
`GS_INPUT_VERTS_PER_PRIM` emission broke twenty-seven tests at once, because
every geometry-shader test compiles the same generated prelude. Nothing else in
the crate had ever asked a shader compiler what it thought of that output.

**Tests written against an implementation that later changed underneath them.**
These are the more interesting ones, because each encodes an assumption that was
true when it was written and silently stopped being true — with no failing run to
say so.

- Two domain-shader tests packed control points one `vec4` apart. The translated
  shader addresses control point `c` at `vec4` index `c * DS_CP_IN_STRIDE`, the
  same fixed `pos` plus `EXPANDED_VERTEX_MAX_VARYINGS` record the whole
  tessellation path uses. The two trailing control points were written where
  nothing reads and the bounds-checked loads returned zero, so the interpolation
  result was exactly the first control point scaled by its barycentric weight —
  a plausible-looking number that is wrong.
- The scratch-allocator growth test filled the *metadata* arena and then
  allocated from the *storage* arena. The allocator keeps the two in separate
  buffers, so nothing was ever exhausted and no growth was triggered. The test
  predated the split.
- The tessellation out-of-memory test recomputed the expected expanded-vertex
  size from the shader's declared output-register count. That count sizes the
  per-control-point payload; the expanded vertex buffer uses the fixed record.
- Two rasterisation tests asserted a fully transparent clear where D3D's
  absent-component fill of `(0, 0, 0, 1)` applies, and one texture-load test
  asserted a sampled colour for a coordinate that is consumed as raw bits.

In every one of these the implementation was right and the test was wrong — but
that is only knowable by reading both. The reflex to "fix the code until the
test passes" would have broken four working code paths.

### The browser end-to-end suite could not start

The end-to-end suite had been unable to run, and the reason turned out to have
nothing to do with any test. Vite's dev server watches the project root, and this
repository's Rust build directory holds over 1.3 million files against an inotify
limit of about 1.05 million. The server therefore died with `ENOSPC` during
startup on any machine that had ever run `cargo build` — every spec then failed
at `page.goto`, which looks exactly like an application that will not load.

`server.watch.ignored` now excludes `target/`, `node_modules/`, `dist/`,
`.attic/`, and the report directories. None is a source input, and the fix is in
both Vite configs.

Two real defects surfaced immediately behind it:

- **The shared-helper package's browser surface had never worked.**
  `packages/transport-safety` kept one implementation per helper as `<name>.cjs`
  with `<name>.js` doing `export * from "./<name>.cjs"`. Node interops CommonJS,
  so every Node consumer was fine. Vite serves a `.cjs` file inside the project
  verbatim, so in the browser that re-export linked against a module with no ES
  exports and the import failed before anything ran. Turning the package around —
  ES module implementation, one-line `require()` wrapper for CommonJS, which the
  pinned Node 24 supports — fixes the browser and costs Node nothing.
- **An end-to-end helper imported a source path at runtime.**
  `probeOpfsSyncAccessHandle` pulled `formatOneLineUtf8` from the source tree by
  URL, but the specs that use it run against the *preview* server, which serves a
  built bundle where no such path exists. It needed one bounded single-line
  string, so it now formats its own.

A third defect had hidden behind those two. The golden-image specs inject
`apps/web/tools/gpu_trace_replay.ts` into the page with `addScriptTag({ path })`,
which copies the file in verbatim — no bundler, no transpile. The file is four
and a half thousand lines of what is otherwise plain JavaScript, and it carried
exactly three TypeScript constructs: one `let` annotation and two `as` casts, in
one function nobody was calling. They were enough to make the whole tool fail to
parse, so `window.AeroGpuTraceReplay` was never defined and every trace-replay
golden failed on reading `load` of `undefined`. Written without them the file is
valid in both languages, and the nine golden tests pass.

The suite goes from not starting to 120 passing, 19 failing, 23 skipped on
Chromium. Two of the remainder are order-dependent rather than broken: `watchdog.spec.ts`
passes all four of its cases when run alone and fails two under four parallel
workers. Running the suite with a single worker confirms it — the deterministic
count barely moves. The rest are individual defects across VBE mode switching,
WDDM scanout recovery, shader-cache persistence and the disk manager. That is a
backlog now rather than a blocked loop.

### What the end-to-end failures turned out to be

Working through the suite's failures, most were not what they looked like.

**Two were the test lying about its own subject.** `webusb-diagnostics` matched a
button with `/Try open \\+ claim/` — a doubly-escaped `+`, which matches a run of
literal backslashes and so could never match anything. `boot_device_debug_api`
asserted three accessors return `null` while calling them as
`dbg?.getBootDisks?.() ?? "missing"`, where the `??` replaces exactly the `null`
being asserted. Neither could ever have passed.

**Two were serving arrangements that could not work.** The shader-cache specs
stood up a plain file server over the source tree and pointed a browser at a page
importing a TypeScript module; no browser executes TypeScript, so the page never
ran. And `tests/helpers/csp_server.mjs` resolved its repository root with a single
`..` from `tests/helpers/`, landing on `tests/` — every asset path it built had
never existed.

**Three were real defects.** The persistent GPU cache silently discarded every
D3D11 entry (see [graphics](graphics.md)). A public asset referenced an undeclared
`textEncoder`, so the one code path the CSP fallback test exercises threw. And a
disk created on the IndexedDB backend reported a size of zero, because that
backend measured materialised chunks while the OPFS one measured the disk.

**One was the environment, said plainly.** The UDP relay specs build a Go binary;
where there is no Go toolchain they now skip with that as the stated reason,
rather than reporting a missing compiler as a broken relay.

A general rule came out of it, now enforced by
`tests/public_asset_imports_contract.test.js`: **an asset under `public/` may not
import from outside `public/`.** Those files are served verbatim, so their
specifiers resolve against the URL rather than the filesystem, and a path that
climbs out of the served root fetches nothing. It fails quietly, too — a module
worker that cannot load surfaces as an opaque error event naming no file.

### The lint gate had drifted red

`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` is a declared gate
in the `justfile`, and it had accumulated around a hundred findings. Most were cosmetic. Three were
not:

- Two probe conditions in `aero-machine-cli` read `x < 0x7f00_0000_0000 && x < 0x7000_0000`, where
  the second comparison already implies the first. The wider bound had never decided anything; if
  an upper bound on user-space addresses was intended there, it was never in effect.
- `crates/aero-machine/src/jit.rs` documented a `MachineConfig::jit_enabled` toggle routing the
  execute loop through tiered interpreter-plus-JIT execution. No such field existed and nothing
  constructed the types involved. Roughly two hundred lines of it were scaffolding for a feature
  that had never been wired, sitting under documentation that described it as done. The live half —
  the IR block cache the execute loop does use — stayed; the rest is in `.attic/dead-code/`.
- A private helper in `crates/devices` and a constant in an `aero-audio` test were simply unreached.

A gate that is red is not a gate, and the longer it stays red the more it hides. The two probe
conditions are the kind of thing a lint is *for*: both compile, both run, and both silently mean
something narrower than they appear to.

### The browser could not have been rebuilt

Chasing the remaining graphics failures turned up something more basic: the
`wasm32` build of `aero-machine` was broken, so `pnpm -C apps/web run wasm:build`
failed outright. The WebAssembly packages checked into `apps/web/src/wasm/` were
therefore several days older than the Rust they are built from — every browser
test was exercising a binary nobody could reproduce.

The cause was narrow. `crate::jit` is native-only, but `run_slice_jit` and
`run_slice_inner` named `jit::IrBlockCache` in their signatures unconditionally.
The execute loop is now shared: on `wasm32` the cache type is an uninhabited
`enum`, so the `Option` is always `None`, and the two fast-path blocks — which
call methods no placeholder can have — are compiled out.

The gate for this already existed. `cargo xtask wasm-check` compiles
`aero-machine` and `aero-wasm` for `wasm32-unknown-unknown` and would have caught
it the day it broke. It had simply not been run, and `cargo check --workspace`
never covers a second target, so nothing else could notice. **A gate nobody runs
is documentation.**

Rebuilding changed the picture in a way worth stating carefully. It did not fix
the failing graphics tests — staleness was not their cause. But it did raise the
suite's failure count from seven to seventeen, because ten more tests had been
passing against a binary that no longer matched the source.

So the earlier figure was measuring the wrong thing: seven was an artefact of a
stale artefact, and seventeen was the suite's real state against current Rust.

The ten newly-revealed failures clustered in the IO worker and the HDA and
virtio-snd audio paths, and all ten had findable causes — an inverted HDA reset
sequence in five host call sites, a wasm module that was never declared in its
crate, and a harness memory allocator handing workers a buffer the module could
not use. Each is written up below or in `wiki/areas/audio.md`.

**Six specs now fail deterministically**, and both guest-execution failures are
gone (see `wiki/areas/platform-and-firmware.md` for what was stopping the
interpreter). The six are the five graphics specs that predate the rebuild, plus
`audio-loopback-synthetic`, whose cause — a JS handle carried across a wasm
instance boundary — is written up there too.

### The suite is load-sensitive, so read a single run carefully

Full runs land on eight or nine failures, but not the same eight or nine.
Across consecutive runs `watchdog`, `tests/e2e/web/worker-audio.spec.ts`,
`audio-worklet-snapshot-resume`, `shared_framebuffer_smoke`,
`runtime_workers_cursor_forwarding` and `workers_panel_input_capture` have each
failed once and then passed in isolation, every time. They are the specs that
assert on timing — underrun counts, frame sequence progress, message round
trips — and a serial run on a busy machine is enough to trip them. One run put a
full `cargo clippy` alongside the suite and produced a completely different
failure set from the same artefacts.

So the count from a single run is not the number to quote. Re-run anything that
fails, on an otherwise idle machine, before treating it as a defect; the six
above survive that treatment.

The packages are gitignored build outputs, so none of this is a change to
anything checked in — only to what the tests were actually exercising.

### Benchmarks are code too

`cargo test --workspace` does not build benchmark targets, and nothing else in
the repository built them either. One of them had not compiled for some time:
`crates/platform/benches/a20_disabled_physical.rs` carried a stray closing
parenthesis, left behind when `PhysicalMemoryBus` gained a type parameter and
every call site had to change. The same change broke a boot test, which was
caught immediately because tests run.

It surfaced by accident. `cargo fmt` parses what it formats, so it stopped on the
file and refused to proceed — a formatter reporting a syntax error the compiler
had never been asked to look at. `cargo check --workspace --benches` builds them
and is worth running when the workspace's shared types change.

The lesson generalises past graphics. **Check what a green suite skipped.** A
conditional skip is the right behaviour for a test that cannot run, and it is
also a place where breakage accumulates silently for as long as the condition
stays false — in the code the test guards *and* in the test itself. A test that
has never run is not a test; it is an untested assertion about an implementation
that has had every opportunity to move.
