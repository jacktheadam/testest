# CPU and JIT

> The x86-64 CPU core, its tiered execution engine (Tier-0 interpreter through
> the Tier-1/Tier-2 WebAssembly JIT), CPUID feature policy, SMP bring-up, and
> CPU-side telemetry. Paging and the MMU have their own page —
> [memory-and-paging.md](./memory-and-paging.md).

The x86-64 CPU core, its tiered execution engine (Tier-0 interpreter →
Tier-1/Tier-2 WASM JIT), the MMU/paging stack, CPUID feature policy, SMP
bring-up, and CPU-side performance telemetry. Absorbed from
`cpu-and-jit.md`, `memory-and-paging.md`,
`performance.md`, `cpu-and-jit.md`, and
the sprint-era CPU, memory, performance, and SMP design documents, all of which this page supersedes.

## Current state (verified against the tree)

- The canonical CPU core is `crates/aero-cpu-core` (`aero_cpu_core`). One
  architectural state struct (`state::CpuState`) is shared by all execution
  tiers. The old interpreter stack (`aero_cpu_core::cpu` + `aero_cpu_core::bus`)
  is feature-gated behind `legacy-interp` (default-off; see
  `crates/aero-cpu-core/Cargo.toml`) and kept only for regression benchmarking.
- Tier-0 (`aero_cpu_core::interp::tier0`,
  `crates/aero-cpu-core/src/interp/tier0/`) is the canonical interpreter.
- A browser Tier-1 pipeline is live: hot blocks are compiled to standalone
  WASM modules in a dedicated worker and installed into the running VM
  (`crates/aero-wasm/src/tiered_vm.rs`, `crates/aero-jit-wasm`,
  `apps/web/src/workers/jit.worker.ts`). It is covered by Playwright E2E tests
  (`tests/e2e/jit-pipeline.spec.ts`) including self-modifying-code
  invalidation.
- Tier-2 (tracing/optimizing JIT) exists in `crates/aero-jit-x86/src/tier2/`
  — trace IR, optimization passes, a WASM code generator, a WASMtime backend
  for native execution, and a reference interpreter. The browser worker
  compiler (`crates/aero-jit-wasm`) today only emits Tier-1 blocks; Tier-2
  browser wiring is partial.
- SMP is **bring-up only**: `aero_machine::Machine` accepts `cpu_count > 1`
  and runs APs cooperatively, but there is no full SMP scheduler and no
  parallel vCPU execution. Use `cpu_count = 1` for real guest boots.
- In the browser, guest RAM lives in wasm32 `WebAssembly.Memory`, so total
  linear memory is ≤ 4 GiB (the shared memory layout decision, shared memory layout).

## Crate map

| Crate / path | Role |
| --- | --- |
| `crates/aero-cpu-core` | Canonical CPU: state, Tier-0, assists, interrupts, paging glue, JIT runtime (cache/hotness/page versions) |
| `crates/aero-mmu` | Page-table walker + software TLB (`Mmu`, `crates/aero-mmu/src/lib.rs`, `tlb.rs`) |
| `crates/aero-jit-x86` | JIT codegen: Tier-1 block compiler (`src/compiler/tier1.rs`), Tier-2 IR/opt/codegen (`src/tier2/`), JIT context + inline-TLB ABI (`src/jit_ctx.rs`, `src/abi.rs`) |
| `crates/aero-jit` | Stable import-path shim; re-exports `aero-jit-x86` (`crates/aero-jit/src/lib.rs`) |
| `crates/aero-jit-wasm` | wasm-bindgen wrapper exposing the Tier-1 compiler as a standalone WASM module for the browser JIT worker |
| `crates/aero-wasm` | WASM-facing VMs: canonical full-system `Machine` (`src/lib.rs`), legacy CPU-only `WasmVm` (`src/vm.rs`), tiered `WasmTieredVm` (`src/tiered_vm.rs`) |
| `crates/memory` | Guest RAM backends (`DenseMemory`, `SparseMemory` in `src/phys.rs`) and hole-aware `MappedGuestMemory` (`src/mapped.rs`) |
| `crates/aero-machine` | Canonical machine integration; owns BSP + AP `CpuCore`s |
| `crates/aero-smp` | Unit-test-oriented SMP prototype (APIC IPI, AP startup state machine) |
| `crates/aero-perf` | CPU/JIT telemetry counters + instruction-retirement integration |
| `crates/perf` | **Legacy.** Deprecated compatibility shim re-exporting `aero_perf::{jit, telemetry}`; do not use for new work |

Web-side glue: `apps/web/src/workers/cpu.worker.ts` (legacy runtime,
`vmRuntime=legacy`), `apps/web/src/workers/machine_cpu.worker.ts`
(`vmRuntime=machine`, drives the canonical `Machine` export — see the canonical machine stack decision),
`apps/web/src/workers/jit.worker.ts`, `apps/web/src/runtime/jit_wasm_loader.ts`.

## Key contracts

### `CpuState` is the JIT ABI

`aero_cpu_core::state::CpuState` (`crates/aero-cpu-core/src/state.rs`) is the
canonical in-memory ABI between Tier-0 and generated JIT code:

- `#[repr(C, align(16))]`; layout frozen by offset constants and the
  `jit_offsets_are_stable` unit test in the same file. Exported constants
  include `CPU_GPR_BASE_OFF` (intentionally 0), `CPU_GPR_OFF[i]`,
  `CPU_RIP_OFF`, `CPU_RFLAGS_OFF`, `CPU_XMM_OFF[i]`, `CPU_STATE_SIZE`
  (currently 1072), `CPU_STATE_ALIGN`.
- Treat it like a C ABI: do not reorder fields; update offset constants and
  their tests together; add new constants when new fields become JIT-visible.
- Only **architectural** state lives in the ABI. Runtime/bookkeeping state
  stays outside so it can evolve without breaking JIT code: CPUID policy and
  INVLPG logging (`assist::AssistContext`), virtual time (`time::TimeSource`),
  pending events (`interrupts::PendingEventState`), JIT caches/hotness
  (`aero_cpu_core::jit::*`).

### `CpuBus` and paging

Tier-0 accesses memory through `aero_cpu_core::mem::CpuBus`
(`crates/aero-cpu-core/src/mem.rs`), which is **linear-address based** — the
bus, not the interpreter, translates linear → physical. Two hooks matter for
paging: `sync(&CpuState)` (called at every instruction boundary so the bus
can observe CR0/CR3/CR4/EFER/CPL changes; `handle_assist` also syncs before
and after each assist) and `invlpg(vaddr)`.

`aero_cpu_core::PagingBus` (`crates/aero-cpu-core/src/paging_bus.rs`) is the
canonical adapter: it wraps an `aero_mmu::Mmu` plus a physical
`aero_mmu::MemoryBus` and refreshes cached MMU state on `sync()` via
`CpuState::sync_mmu`. The real `Mmu` (`crates/aero-mmu/src/lib.rs`) tracks
CR0/CR2/CR3/CR4/EFER, a `PagingMode`, PCID state, and a 4-way
set-associative software TLB (64 sets per bank, `crates/aero-mmu/src/tlb.rs`).
Paging-enabled tests live in `crates/aero-cpu-core/tests/paging.rs`;
`mem::FlatTestBus` is the minimal flat bus for unit tests.

### Execution loop and retirement semantics

`exec::ExecDispatcher` (`crates/aero-cpu-core/src/exec/mod.rs`) picks Tier-0
vs JIT per block; `exec::Vcpu` bundles `CpuCore` + a bus. Before each block,
`ExecCpu::maybe_deliver_interrupt()` gets a chance to deliver pending events
at an instruction boundary.

**Instruction retirement is the canonical accounting unit** for both virtual
time and interrupt-shadow aging, and all tiers must match Tier-0 exactly:

- A committed JIT block retires exactly `CompiledBlockMeta.instruction_count`
  guest instructions (provided out-of-band by the compiler).
- A rollback exit (MMIO/page-fault/bailout restoring pre-block state) retires
  **0**.
- `PendingEventState` (interrupt shadow: `STI`, `MOV SS`, `POP SS`) is aged by
  the same retired count — never on rollback exits, never per-block.
- `StepOutcome::Block { instructions_retired, .. }` reports the exact count
  regardless of tier; `aero_perf::retire_from_step_outcome`
  (`crates/aero-perf/src/lib.rs`) is the canonical perf-counter integration,
  with `ExecDispatcher::step_with_perf` as a convenience wrapper.

### Tier-0 assists

Tier-0 keeps its instruction set tight and exits with an `AssistReason`
(`crates/aero-cpu-core/src/exception.rs`) for state-dependent instructions:
`Io` (IN/OUT/INS*/OUTS*), `Cpuid`, `Msr` (RDMSR/WRMSR), `Privileged`
(descriptor tables, INVLPG, far transfers), `Interrupt` (INT*/IRET*/CLI/STI/
INTO), `Unsupported`. The assist layer (`crates/aero-cpu-core/src/assist.rs`,
entry points `handle_assist` / `handle_assist_decoded`) emulates them. Batch
entry points: `interp::tier0::exec::{step, run_batch, run_batch_with_assists,
run_batch_cpu_core_with_assists}` (`src/interp/tier0/exec.rs`). Prefer the
`exec::Tier0Interpreter` glue over calling assists directly for
interrupt-related reasons — canonical delivery lives in
`crates/aero-cpu-core/src/interrupts.rs` (`CpuCore` = `{ state, pending,
time }`, `PendingEventState` with external-interrupt FIFO, exception
nesting/double-fault escalation, IRET frame stack; triple fault records a
sticky `CpuExit`).

### Virtual time / TSC

`time::TimeSource` (`crates/aero-cpu-core/src/time.rs`) owns the TSC.
Default mode is **deterministic**: TSC advances by 1 per retired guest
instruction (`advance_cycles(1)`), and JIT execution must advance by the same
retired counts (block count on commit, 0 on rollback). An optional wall-clock
mode exists for native (non-WASM) integrations and is inherently
non-deterministic. `CpuState.msr.tsc` mirrors `TimeSource`; RDTSC/RDTSCP and
RDMSR/WRMSR `IA32_TSC` read/write through it.

### JIT context + inline TLB

JIT-compiled code must not call an imported helper per guest load/store. The
fast path is an inline TLB in linear memory, defined in
`crates/aero-jit-x86/src/jit_ctx.rs` and `src/abi.rs`:

- Both tiers use a 2-pointer ABI: `block(cpu_ptr: i32, jit_ctx_ptr: i32) -> i64`
  (Tier-1) and `trace(cpu_ptr, jit_ctx_ptr) -> i64` (Tier-2). `cpu_ptr` points
  at `CpuState` in shared WASM memory; `jit_ctx_ptr` points at a separate
  JIT-only region so acceleration structures stay out of the ABI.
- `JitContext` header: `ram_base` (offset 0), `tlb_salt` (offset 8), then a
  direct-mapped array of `JIT_TLB_ENTRIES = 256` 16-byte entries at offset 16:
  `{ tag: (vpn ^ tlb_salt) | 1, data: phys_page_base | flags }`. Tag 0 means
  invalid.
- `data` flags (`crates/aero-jit-x86/src/lib.rs` and `src/legacy/cpu.rs`):
  `TLB_FLAG_READ` (bit 0), `TLB_FLAG_WRITE` (1), `TLB_FLAG_EXEC` (2),
  `TLB_FLAG_IS_RAM` (3 — direct WASM load/store valid; clear ⇒ MMIO/ROM/
  unmapped, exit to the runtime). `phys_page_base` is always 4 KiB-aligned,
  even for 2 MiB/1 GiB guest mappings, so a "crosses 4 KiB" guard is
  sufficient.
- Slow path: `mmu_translate(cpu_ptr, jit_ctx_ptr, vaddr, access) -> i64`
  performs the walk + permission checks, fills the entry, and returns the
  packed word; `access` uses `MMU_ACCESS_READ = 0` / `MMU_ACCESS_WRITE = 1`
  (`src/abi.rs`). On fault it raises `#PF` and exits the block. Tier-1 uses a
  dedicated MMIO-exit helper; Tier-2 currently falls back to imported
  `mem_read_*`/`mem_write_*` helpers.
- Invalidation: bump `tlb_salt` for CR3 writes/INVPCID/full flushes and MMIO
  map changes (O(1) — stale tags simply stop matching); clear the entry tag
  for INVLPG.

### Self-modifying code

Handled by page-version snapshots in `aero_cpu_core::jit::runtime`
(`crates/aero-cpu-core/src/jit/runtime.rs`):

- `PageVersionTracker` keeps a wrapping `u32` version per 4 KiB guest
  physical page; `JitRuntime::on_guest_write(paddr, len)` bumps every covered
  page. Embedders must call it for any guest physical write that can hit RAM.
- The compiler snapshots `{ page, version }` pairs into
  `CompiledBlockMeta.page_versions` (`JitRuntime::snapshot_meta`);
  `install_handle` rejects stale compilation results (background-compile
  races), and `prepare_block` lazily invalidates stale cached blocks and
  requests recompilation.
- In the browser harness (`crates/aero-wasm/src/tiered_vm.rs`), Tier-0 writes
  are captured into a bounded `GuestWriteLog`
  (`crates/aero-wasm/src/jit_write_log.rs`) and drained into
  `on_guest_write` at block/interrupt boundaries; Tier-1 JS store helpers
  call the exported `WasmTieredVm::on_guest_write`. Tier-2 traces additionally
  embed `GuardCodeVersion` IR checks (`crates/aero-jit-x86/src/tier2/ir.rs`)
  so they can deopt in-WASM.

Direct JIT RAM stores that bypass the runtime's write observation are a known
future extension (a "CODE_WATCH" flag bit is reserved conceptually); today the
runtime must observe every code-page write.

### CPUID / feature policy

Modeled in `crates/aero-cpu-core/src/cpuid.rs` with feature bits grouped by
where they are reported (`CpuFeatureSet::leaf1_ecx`, `leaf1_edx`, `leaf7_*`,
`ext1_*`). Implemented leaves: 0x0, 0x1, 0x2, 0x4, 0x6 (stub), 0x7, 0xA
(stub), 0xB/0x1F topology (returned only when CPUID.1:ECX[x2APIC] is set —
verified in `cpuid_topology`), 0x8000_0000..=0x8000_0008 subset (feature
flags, brand string, L2 cache, invariant TSC, address sizes);
`max_basic_leaf = 0x1F`, unknown leaves return 0.

Two profiles (`CpuProfile`): **Win7Minimum** (long mode, SSE2, PAE, NX, APIC,
TSC, SYSCALL/SYSRET, LAHF/SAHF-in-LM, CMPXCHG16B — the minimum viable x86-64
CPU for Win7 boot) and **Optimized** (additional bits like SSE3–SSE4.2/POPCNT
only when actually implemented). `CpuFeatureOverrides` can force bits for
debugging; `force_enable` is capped by the implemented set unless
`allow_unsafe = true` (expected to break guests). MSR coherence is enforced:
clearing NX/SYSCALL/LM in CPUID masks writes to EFER.NXE/SCE/LME. Tests:
`crates/aero-cpu-core/tests/cpuid_policy.rs`, `cpuid_htt.rs`.

### Guest physical memory map

On the canonical PC platform, RAM is **not** a single contiguous region
(`crates/aero-pc-constants/src/lib.rs`):

- Low RAM ends at `PCIE_ECAM_BASE = 0xB000_0000`;
  `0xB000_0000..0xC000_0000` is the PCIe ECAM window
  (`PCIE_ECAM_SIZE = 0x1000_0000`), `0xC000_0000..0x1_0000_0000` the
  below-4 GiB PCI/MMIO hole. RAM beyond that is remapped above 4 GiB.
- Backends must therefore be hole-aware; unclaimed hole reads act as open bus
  (`0xFF`). `memory::MappedGuestMemory` (`crates/memory/src/mapped.rs`)
  encodes this; used via `platform::MemoryBus::wrap_pc_high_memory`
  (`crates/platform/src/memory.rs`) and
  `aero_machine::SystemMemory` (`crates/aero-machine/src/lib.rs`). The E820
  map is built in `crates/firmware/src/bios/interrupts.rs::build_e820_map`.
- `0x000A_0000–0x000B_FFFF` is device memory (legacy VGA VRAM window).
  Graphics-path ownership (standalone VGA/VBE vs AeroGPU BAR1) is covered in
  the graphics area page.

## Browser Tier-1 pipeline (end to end)

1. `WasmTieredVm` (`crates/aero-wasm/src/tiered_vm.rs`) runs Tier-0 with a
   hotness profile (browser `hot_threshold: 3`; the `JitRuntimeConfig`
   default is 32 — `crates/aero-cpu-core/src/jit/runtime.rs`) and exports
   `drain_compile_requests()` (hot entry RIPs), `install_tier1_block(
   entry_rip, table_index, code_paddr, byte_len)`, and
   `on_guest_write(paddr, byte_len)`.
2. The CPU worker (`apps/web/src/workers/cpu.worker.ts`, legacy runtime) forwards
   compile requests to the JIT worker and wires JS shims used by generated
   code: `globalThis.__aero_io_port_read/write`, `__aero_mmio_read/write`,
   and `__aero_jit_call`.
3. `apps/web/src/workers/jit.worker.ts` drives the `aero-jit-wasm` module (loaded
   via `apps/web/src/runtime/jit_wasm_loader.ts`, which prefers the
   single-threaded `apps/web/src/wasm/pkg-jit-single` package — generated by
   wasm-pack, not committed — to avoid huge `SharedArrayBuffer` allocation at
   instantiation) and compiles the returned bytes into a cached
   `WebAssembly.Module`. If modules are not structured-cloneable it reports
   `unsupported`.
4. The compiler crate uses its **own private linear memory** (no import of
   the emulator's shared memory) to avoid multiple Rust runtimes aliasing one
   `WebAssembly.Memory`.

E2E coverage: `tests/e2e/jit-pipeline.spec.ts` (compilation, installation,
SMC invalidation, execution in real browsers).

## SMP status

Bring-up only, in the canonical integrations (`aero_machine::Machine`,
`aero_machine::PcMachine`, `aero_pc_platform::PcPlatform`):

- **Landed:** per-vCPU LAPIC instances + MMIO routing, AP wait-for-SIPI
  state, INIT+SIPI delivery via the LAPIC ICR, a bounded cooperative AP loop
  in `Machine::run_slice`, firmware topology generation (ACPI MADT, DSDT
  `_PR_`, SMBIOS) for `cpu_count > 1`. `cpu_count == 0` fails construction
  with `MachineError::InvalidCpuCount` (`crates/aero-machine/src/lib.rs`).
- **Coverage:** `crates/aero-machine/tests/` (`ap_tsc_sipi_sync.rs`,
  `lapic_mmio_per_vcpu.rs`, `ioapic_routes_to_apic1.rs`,
  `smp_lapic_timer_wakes_ap.rs`, `smp_timer_irq_routed_to_ap.rs`,
  `smp_ipi_delivery.rs`, and more) and `crates/platform/tests/smp_*`
  (INIT-deassert IPI, IOAPIC/MSI destination routing).
- **Missing:** a real multi-vCPU scheduler (fairness, guest-driven AP
  execution, parallelism), hardened guest OS AP bring-up, full IPI semantics
  (AP→BSP/AP, broadcast), and multi-vCPU snapshot/restore (`aero-snapshot`
  has multi-vCPU `CPUS` state, but `Machine` snapshots only the BSP today).
- **Guidance:** use `cpu_count = 1` for real guest boots; use snapshots, not
  multi-core, for faster boot-to-desktop. `crates/aero-smp/` is the
  test-oriented prototype for experimenting with AP startup/IPI logic.

## Performance & telemetry

- **Instruction counting:** `instructions_executed` counts retired guest
  architectural instructions (not micro-ops; string/REP* retires as 1 — track
  iterations via a separate `rep_iterations` counter). CPU workers keep
  non-atomic local counters in hot loops and batch-flush to shared totals
  every ~1k–10k retired instructions; JIT execution attributes counts at
  block granularity via `CompiledBlockMeta.instruction_count`. MIPS =
  instructions_delta / wall_delta / 1e6, reported as rolling average + p95.
- **JIT metrics:** the minimum telemetry surface (compile time per tier and
  per Tier-2 pass, blocks compiled per tier, code-cache capacity/used bytes,
  cache hit/miss, deopt and guard-failure counters, plus 1-second rolling
  hit-rate / compile-ms-per-s / blocks-per-s) is implemented in
  `crates/aero-perf` (`aero_perf::jit`, `aero_perf::telemetry`). Counters are
  cheap, amortized, and gathered per-worker before merging. A synthetic demo
  runs via `cargo run --locked -p perf --example jit_metrics_demo`.
  Interpreted-only runs must leave all `jit.*` counters at zero.
- **Hot-spot collection:** hot basic blocks are recorded per block entry
  (`hits += 1`, `instructions += block_len`) with a bounded streaming
  heavy-hitter structure, and exported via `window.aero.perf.export().hotspots`.
- **Shared metric definitions:** HUD, exports, and bench tooling share one
  implementation in `packages/aero-stats` (avg/median/1%-low/0.1%-low FPS
  from frame-time percentiles; Welford population variance).
- **Benchmark harness:** versioned history in `bench/history.json`; the
  browser benchmark runner lives in `tools/perf/`; guest CPU throughput
  microbenchmarks live under `bench/` (see also
  `crates/aero-cpu-core/tests/jit_equivalence.rs` for correctness
  cross-checks between tiers).
- **Headline targets** [aspirational]: boot < 60 s (floor 120 s), desktop
  ≥ 30 FPS, app launch < 5 s, input latency < 50 ms, ≥ 500 MIPS sustained.
  No current measured values are recorded in-tree against these targets
  [unverified].

## Open issues / debt

- **`--jit` computes a wrong SHA-256 in the guest, and the cause is unknown.**
  This is the most serious open correctness defect in the tree. Under `--jit`,
  the guest's cryptographic provider computes a digest of
  `53ea9c24…` where the interpreter — on the *same snapshot* — computes the
  correct `d9c291de…`. The consequence downstream is concrete: signature
  verification fails, the virtual disk service refuses to start, and Windows
  Setup cannot prepare a disk.

  Everything cheap has been ruled out. The mapped provider image matches the file
  on disk; the guest's own digest routines match the reference implementation
  *under the interpreter*; and an isolated compress of a known vector through the
  IR cache is also correct. The divergence reproduces only in the live snapshot.

  There is no fix, only a rule: **do not run `--jit` across that phase.**
  *(Both digests and the reproduction are recorded from a live run; no captured
  artefact for them is in the tree, so treat the exact values as
  **[unverified]** — the divergence itself was reproduced repeatedly.)* A
  closely related defect was found and fixed the same way (missing `ADC`/`SBB`
  decoding broke a 4096-bit signature check while the 2048-bit one passed), which
  shows the shape but did not close the class.
- **The guard on caching memory operations was removed along with its
  restriction.** Cached compilation originally refused any block containing a
  load, a store, or a helper call, because `Tier1Bus` has *infallible* read and
  write signatures — it returns `0xff` on a guest page fault and silently drops
  failed writes, which cannot host a faulting architecture. That was explicitly
  an interim fail-closed measure "until Tier-1 has an exception-aware,
  transactional memory bridge". That bridge does not exist, but the restriction
  has since been relaxed to admit **all** loads and stores (`crates/aero-machine/src/jit.rs`),
  and the test that enforced the original invariant
  (`jit_does_not_cache_memory_blocks_without_fault_propagation`) **is no longer in
  the tree**. Only `CallHelper` is still rejected.
- **Tier-1 throughput collapses on some guest code** — roughly 4.5 down to 1.2
  million instructions per second once execution sits in a compare-and-store
  merge loop inside the management-instrumentation host process. **[unverified]**:
  measured during bring-up, with no benchmark record committed. Remembering
  rejected store-heavy addresses and admitting one trailing store reduced it; it
  is still present.
- **The Tier-0 decompression fuses can livelock the guest.** The instruction-fusion
  helpers added for the Windows Setup expand phase livelock the kernel at
  `CR8=0xf` on the transition out of that phase, on an empty overlay and a
  partially-expanded one alike, where the pre-fusion build proceeds. The cause is
  a pattern matching a sequence it must not — a false match — and it is not
  identified. **Treat those fuses as expand-only until it is** — and note this is
  a convention, not a guard: nothing in the code enforces the restriction, so a
  future caller can walk straight into the livelock. The general
  lesson is recorded in [../history/windows-7-bring-up.md](../history/windows-7-bring-up.md):
  a fusion validated by a positive oracle also needs a *negative* one.
- **Near `Ret` derives its pop width from the code segment, not the operand
  size** (`interp/tier0/ops_cf.rs`, `Mnemonic::Ret`): `ret_size` is
  `state.bitness() / 8`. This is the same defect family as the far-return width
  bug that broke the Windows 7 boot loader's entry into 32-bit mode, which was
  fixed by deriving the width from `instr.code()` instead. A `66 c3` in a 16-bit
  protected-mode code segment would mis-pop here in exactly the same way. It has
  never been observed in the boot path, so it is recorded rather than fixed —
  but it is a real divergence from the architecture, not a theoretical one. See
  [the bring-up catalogue](../history/windows-7-bring-up.md).
- Tier-2 is not fully wired into the browser pipeline: `aero-jit-wasm`
  compiles Tier-1 blocks only; Tier-2's WASM codegen and wasmtime backend are
  exercised natively (`crates/aero-jit-x86/src/tier2/`, `src/backend/`).
- Tier-2 memory slow path still uses imported `mem_read_*`/`mem_write_*`
  helpers instead of the dedicated MMIO-exit import.
- SMP: everything in the "Missing" list above; snapshots are BSP-only.
- Direct JIT RAM stores are not yet observable by the page-version tracker
  (reserved "CODE_WATCH" extension); all code-page writes must currently go
  through runtime-observed paths.
- Stale doc reference: the perf docs and `tools/perf/README.md` cite a
  nightly workflow at `.github/workflows/perf-nightly.yml`, but no `.github/`
  directory exists in this checkout — the scheduled nightly run is
  [unverified]; only the local `tools/perf/` runner and `bench/history.json`
  are present.
- `crates/perf` is a deprecated shim; use `crates/aero-perf` directly.

## Material deliberately not carried over

- The long Rust listings in the old memory and performance docs (multi-bank
  iTLB/dTLB/STLB model, `MemoryBatcher`, `LazyMemory`, `CowDiskImage`,
  `DmaController`, `ThreadedInterpreter`, `JitConfig` with fixed 10/1000
  tier thresholds, draw batchers, shader-cache sketches, packet coalescers,
  lock-free queues) were **illustrative pseudocode**, not repo code. Real
  counterparts are cited above (`aero-mmu` TLB, `memory::SparseMemory`,
  `JitRuntimeConfig.hot_threshold`). Graphics/storage/network sketches belong
  to their own area pages.
- Coded task identifiers used in the old perf docs (PF-00x) are dropped per
  house rules; the features survive under natural names (perf HUD, JIT
  telemetry, hot-spot collection, guest CPU microbenchmarks).
- Generic x86 paging primer material (page-table walk diagrams, PTE bit
  tables, paging-mode matrix) is textbook content; consult the Intel SDM.
  Repo-specific facts (4-level paging for Win7 x64, NX/SYSCALL/LM coherence)
  are kept above.

## Pointers

- **The Windows 7 bring-up:** [`../history/windows-7-bring-up.md`](../history/windows-7-bring-up.md) — every CPU defect found and fixed, by class, each with its regression test. The stack-address-size root cause (SS.B, not CS.D) is documented there. The instruments used to find them are in [`debugging.md`](./debugging.md).
- CPU core: `crates/aero-cpu-core/src/{state,mem,assist,interrupts,time,cpuid,paging_bus}.rs`, `src/interp/tier0/`, `src/exec/`, `src/jit/`
- MMU: `crates/aero-mmu/src/{lib,tlb}.rs`
- JIT: `crates/aero-jit-x86/src/{jit_ctx,abi,lib}.rs`, `src/compiler/tier1.rs`, `src/tier2/`; `crates/aero-jit-wasm`
- Browser integration: `crates/aero-wasm/src/tiered_vm.rs`, `apps/web/src/workers/{cpu.worker,jit.worker,machine_cpu.worker}.ts`, `apps/web/src/runtime/jit_wasm_loader.ts`
- Tests: `crates/aero-cpu-core/tests/` (paging, cpuid_policy, jit_equivalence, exec_*), `tests/e2e/jit-pipeline.spec.ts`, `crates/aero-machine/tests/` (smp_*), `crates/platform/tests/smp_*`
- ADRs: 0003 (shared memory layout), 0014 (canonical machine stack)
- Sibling areas: memory/storage, graphics, machine/platform, perf tooling

## Native JIT wiring (2026-07-27)

See `wiki/notes/bring-up-findings-divergence-and-sight-audits.md` §7 for details.

The native JIT has been wired into the Machine execute loop:

- **IR block cache** (`aero-machine/src/jit.rs`): `IrBlockCache` discovers hot
  blocks (threshold: 50 hits), translates to Tier-1 IR, caches them. `run_ir_block`
  executes cached blocks via the Tier-1 IR interpreter with raw-pointer `CpuBus`
  adapters.
- **CLI**: `--jit` flag on `aero-machine-cli` enables the IR JIT path.
- **Machine**: `run_slice_jit()` dispatches to the IR fast-path before falling
  back to `run_batch_cpu_core_with_assists`.
- **WASM backend**: `process_compile_requests` now actually compiles via
  Cranelift (`compile_and_install`) instead of draining the queue. The full
  `ExecDispatcher::step()` path with WASM execution is available but requires a
  memory bridge (wasmtime linear memory must hold full guest RAM).

### Benchmark

On the kernel spin-loop snapshot, the IR interpreter shows **no net speedup**
(5.4 vs 5.5 Minst/s) because the Tier-0 decode cache already handles tight loops.
The real 10-50x speedup requires native WASM compilation (Cranelift), which
eliminates the interpreter loop entirely. The blocking item is sizing the
wasmtime linear memory to 2+ GiB and syncing guest RAM.

### Perf optimizations applied to Tier-0 interpreter

- `set_rflags`: guard `lazy_flags.clear()` (was unconditional 32-byte memset per ALU op)
- `#[inline]` on `bitness`, `sync_mmu`, `mask_bits`, `read_reg`, `write_reg`, `seg_base_reg`, `logic_flags`, `set_logic_szp`, `parity8`
- `calc_ea`: i128 → u64 wrapping arithmetic
- `add_with_flags`/`sub_with_flags`: u128 specialization for bits<64
- `FXSAVE`/`FXRSTOR` legacy: 16 XMM in IA-32e mode (was 8 — silent XMM8-15 corruption)

## CPU Emulation Engine

### Canonical implementation (post-refactor)

The canonical CPU core lives in `crates/aero-cpu-core` (`aero_cpu_core`). The public API is intentionally centered around a **single architectural state struct** that is shared by all execution tiers.

**Primary components:**

- **Canonical CPU state / JIT ABI:** `aero_cpu_core::state::CpuState`  
  Source: [`crates/aero-cpu-core/src/state.rs`](../../crates/aero-cpu-core/src/state.rs)
- **Canonical interpreter (Tier 0):** `aero_cpu_core::interp::tier0`  
  Source: [`crates/aero-cpu-core/src/interp/tier0/mod.rs`](../../crates/aero-cpu-core/src/interp/tier0/mod.rs)
- **Paging integration:** `aero_cpu_core::PagingBus` (adapter wrapping `aero_mmu`)  
  Source: [`crates/aero-cpu-core/src/paging_bus.rs`](../../crates/aero-cpu-core/src/paging_bus.rs)
- **Architectural interrupt/exception delivery:** `aero_cpu_core::CpuCore` (`CpuState` + `PendingEventState` + `time::TimeSource`)  
  Source: [`crates/aero-cpu-core/src/interrupts.rs`](../../crates/aero-cpu-core/src/interrupts.rs)
- **Tiered exec glue:** `aero_cpu_core::exec` (`Vcpu`, `Tier0Interpreter`, `ExecDispatcher`)  
  Source: [`crates/aero-cpu-core/src/exec/mod.rs`](../../crates/aero-cpu-core/src/exec/mod.rs)

**Legacy CPU stacks:**

- The old interpreter stack (`aero_cpu_core::cpu` + `aero_cpu_core::bus`) is **feature-gated** behind `legacy-interp` (default-off). It is not the primary path and should not be used for new work.

---

### CPU state = JIT ABI (`CpuState`)

`CpuState` is the **canonical in-memory ABI** between:

- the Tier-0 interpreter (`interp::tier0`), and
- dynamically generated JIT blocks (WASM codegen; Tier-1+).

It is:

- `#[repr(C, align(16))]` (layout is intentional)
- validated by compile-time asserts and unit tests
- accessed by JIT code via exported byte offsets

#### ABI stability rules

When modifying `CpuState`, treat it like a public C ABI:

1. **Do not reorder fields casually.** Reordering changes offsets and will break any JIT backend that assumes them.
2. **Update offsets + tests together.** The crate exposes offset constants and tests that freeze them.
3. **Add new offset constants when new fields become JIT-visible.** (The JIT can’t safely “guess” Rust layout.)

The currently exported offsets (see `state.rs`) include:

- `CPU_GPR_BASE_OFF` (intentionally 0)
- `CPU_GPR_OFF[i]` for `RAX..R15`
- `CPU_RIP_OFF`
- `CPU_RFLAGS_OFF`
- `CPU_XMM_OFF[i]` for `XMM0..XMM15`
- `CPU_STATE_SIZE`, `CPU_STATE_ALIGN`

The corresponding “do not regress this” tests live alongside the type:

- `jit_offsets_are_stable` in [`state.rs`](../../crates/aero-cpu-core/src/state.rs)

#### What lives outside the ABI?

`CpuState` contains **architectural CPU state** (GPRs, RIP/RFLAGS, segment caches, control/debug registers, MSRs, FPU/SSE state, etc). Runtime/bookkeeping state lives outside the ABI so it can evolve without breaking JIT code:

- CPUID policy, INVLPG logging: `assist::AssistContext`
- Virtual time / TSC model: `time::TimeSource` (typically stored in `CpuCore.time`)
- Pending interrupts/exceptions, interrupt shadow bookkeeping, IRET frame stack: `interrupts::PendingEventState`
- JIT runtime caches/profiling/hotness counters: `jit::*`

---

### Execution loop shape (Tier-0 + tiered runtime)

At a high level, the CPU runs in a loop that:

1. **delivers any pending architectural events** (exceptions, software interrupts, external interrupts), then
2. **executes a “block”** using either Tier-0 or the JIT, then
3. repeats.

In `aero_cpu_core`, this is modeled by:

- `exec::ExecDispatcher` (chooses Tier-0 vs JIT per block)
- `exec::Vcpu` (bundles `CpuCore` + a bus and implements `ExecCpu`)

`ExecDispatcher::step()` always gives interrupts a chance at *instruction boundaries* via `ExecCpu::maybe_deliver_interrupt()` before running the next block.

#### Instruction retirement accounting (time + interrupt-shadow)

“Instruction retirement” is the canonical unit used by:

- virtual time / TSC progression (`time::TimeSource`), and
- interrupt-shadow aging (`interrupts::PendingEventState`).

Tier-0 already performs these updates once per retired guest instruction. **Tiered/JIT execution must preserve the exact same semantics.**

In particular:

- **Committed JIT exits:** a compiled block must retire exactly `block_instruction_count` guest instructions.
  - `block_instruction_count` is provided out-of-band by the compilation pipeline via
    `CompiledBlockMeta.instruction_count` (stored alongside the cached block handle).
- **Rollback JIT exits:** if a block exits via an MMIO/page-fault/runtime bailout that restores the pre-block
  architectural state, it must retire **0** guest instructions.
- **Interrupt shadow:** `PendingEventState` must be aged by the **same retired-instruction count** as virtual time.
  - Do **not** age the shadow on rollback exits, and do **not** “fake” retirement by counting blocks instead of
    guest instructions.

This retirement count is also the right unit for embedding-facing instruction counters (see “Perf counters
integration” below).

#### Tiered/JIT block exit signaling (Tasks 5/6)

To make the above unambiguous and hard to regress, the tiered runtime uses explicit API surfaces:

- `CompiledBlockMeta.instruction_count`: number of guest architectural instructions in the compiled block.
- `JitBlockExit`: includes committed vs rollback signaling in addition to the next RIP / “exit to interpreter”
  decision.
- `ExecDispatcher::StepOutcome::Block { instructions_retired, .. }` (Task 6): reports the exact number of guest
  instructions retired by the step, regardless of which tier executed.

---

### Browser Tier-1 JIT compilation pipeline (browser workers)

The project includes a browser-only Tier-1 JIT integration that compiles x86 basic blocks into standalone
WASM modules **inside a worker**:

- **Tiered runtime (Tier-0 + dispatch + cache):** `crates/aero-wasm/src/tiered_vm.rs` (`WasmTieredVm`)
  - The WASM runtime exports:
    - `drain_compile_requests()` → entry RIPs that became hot
    - `install_tier1_block(entry_rip, table_index, code_paddr, byte_len)` → installs compiled blocks
- **Tier-1 compiler (x86 → WASM):** `crates/aero-jit-wasm` (`aero-jit-wasm`)
  - Built as a separate wasm-bindgen package (`apps/web/src/wasm/pkg-jit-*`).
  - Uses its **own private linear memory** (does not import the emulator's shared memory) to avoid
    undefined behaviour from multiple Rust runtimes aliasing one `WebAssembly.Memory`.
- **JS glue (web runtime):**
  - CPU worker (legacy runtime, `vmRuntime=legacy`): `apps/web/src/workers/cpu.worker.ts` drives the `WasmVm` export (and may use `WasmTieredVm` for tiered/JIT
    execution) and wires up the JS shims used by the runtime (`globalThis.__aero_io_port_*`, `globalThis.__aero_mmio_*`,
    and Tier-1 `globalThis.__aero_jit_call`).
  - JIT worker: `apps/web/src/workers/jit.worker.ts` compiles provided WASM bytes into a `WebAssembly.Module` (cached by
    content hash). If `WebAssembly.Module` is not structured-cloneable, it reports an `unsupported` error instead.
  - Loader: `apps/web/src/runtime/jit_wasm_loader.ts` loads `aero-jit-wasm` and currently prefers the
    single-threaded package to avoid wasm-bindgen allocating huge `SharedArrayBuffer`s during
    instantiation when `--max-memory` is large.

> Note: `crates/aero-wasm` exposes multiple WASM-facing VM wrappers. The canonical **full-system** export is
> `aero_wasm::Machine` (backed by `aero_machine::Machine`). The web worker runtime has two integration modes:
> - `vmRuntime=legacy`: CPU worker (`apps/web/src/workers/cpu.worker.ts`) drives the legacy CPU-only `WasmVm` export (and
>   may use `WasmTieredVm` for tiered/JIT execution).
> - `vmRuntime=machine`: CPU worker (`apps/web/src/workers/machine_cpu.worker.ts`) drives the canonical `Machine` export.
>
> See [`../state/repo-state-and-structure.md`](../state/repo-state-and-structure.md) and [Canonical machine stack](../decisions/0014-canonical-machine-stack.md) for the
> up-to-date mapping.

This pipeline is exercised by Playwright E2E smoke tests (for example `tests/e2e/jit-pipeline.spec.ts`) to validate
Tier-1 compilation, installation, invalidation (self-modifying code), and execution in real browsers.

---

### Tier-0 interpreter (`interp::tier0`)

Tier-0 is the canonical interpreter and executes directly on:

- `&mut state::CpuState` (architectural state / JIT ABI), and
- `&mut impl mem::CpuBus` (abstract memory + IO).

Key entry points:

- `interp::tier0::exec::step` (single instruction)
- `interp::tier0::exec::run_batch` (execute up to N instructions)
- `interp::tier0::exec::run_batch_with_assists` (resolves non-interrupt assist exits)
- `interp::tier0::exec::run_batch_cpu_core_with_assists` (resolves all assists, including interrupt-related ones)

Tier-0 intentionally keeps the “core” instruction set tight. When it encounters instructions that depend on additional platform/system state, it returns an **assist exit**:

- `AssistReason::Io` (`IN/OUT/INS*/OUTS*`)
- `AssistReason::Cpuid` (`CPUID`)
- `AssistReason::Msr` (`RDMSR/WRMSR`)
- `AssistReason::Privileged` (descriptor tables, `INVLPG`, far control transfers, etc.)
- `AssistReason::Interrupt` (`INT*`, `IRET*`, `CLI/STI`, `INTO`)

Assists are resolved by the `assist` layer (below). For interrupt-related semantics, the *canonical* delivery logic lives in `interrupts` (see next section), so most integrations should prefer the glue in `exec::Tier0Interpreter` instead of calling `assist` directly for `AssistReason::Interrupt`.

---

### Assist handling (`assist.rs`)

The assist layer emulates instructions that Tier-0 does not implement directly.

API:

- `assist::handle_assist` (fetch + decode + execute)
- `assist::handle_assist_decoded` (execute already-decoded instruction)

Runtime state for assists is split across:

- `assist::AssistContext` (non-ABI):
  - `features`: CPUID feature policy (also used to mask MSR writes coherently)
  - `invlpg_log`: optional log of invalidated linear addresses (useful for tests)
- `time::TimeSource` (non-ABI): owns virtual TSC progression and is typically stored on `CpuCore` as `cpu.time`.

Callers that use the assist layer directly should pass both the architectural state and the time source:

```rust
assist::handle_assist(&mut ctx, &mut cpu.time, &mut cpu.state, &mut bus, reason)?;
```

Important integration detail: assists may modify paging-related state (`CR0/CR3/CR4/EFER`, CPL via segment loads, etc.). The CPU bus contract supports this via `CpuBus::sync(state)`:

- Tier-0 calls `bus.sync(state)` once per instruction boundary.
- `handle_assist` also calls `bus.sync(state)` before and after executing the assist to keep translation state coherent even when used outside the Tier-0 loop.

#### Virtual time / TSC semantics (`time::TimeSource`)

The CPU’s timestamp counter is modeled by [`time::TimeSource`](../../crates/aero-cpu-core/src/time.rs) (not by `CpuState` or `AssistContext`):

- **Deterministic mode (default):** TSC advances on *guest instruction retirement*.
  - Tier-0 increments time via `TimeSource::advance_cycles(1)` once per retired instruction (including assists).
  - JIT/tiered execution must advance by the exact same number of retired guest instructions:
    - committed JIT block: advance by `CompiledBlockMeta.instruction_count`
    - rollback JIT exit: advance by `0`
- **Wall-clock mode (optional):** TSC is derived from a host `Instant` stored inside `TimeSource` (intended for native/non-WASM integrations; it is inherently non-deterministic).
- **Coherency:** `CpuState.msr.tsc` mirrors `TimeSource`:
  - execution glue updates `state.msr.tsc = time.read_tsc()` after each retirement, and
  - `RDTSC/RDTSCP` and `RDMSR/WRMSR IA32_TSC` read/write through `TimeSource` and update `state.msr.tsc` as part of their architectural semantics.

#### Perf counters integration

Instruction retirement is also the unit used by `aero-perf` counters (`PerfWorker::retire_instructions`).
When driving the CPU through `ExecDispatcher`, embedders should use the dispatcher-provided retirement count.
The canonical integration is
[`aero_perf::retire_from_step_outcome`](../../crates/aero-perf/src/lib.rs), which already accounts for interpreter
vs JIT execution and committed vs rollback exits (rollback retires 0 instructions):

```rust
use aero_cpu_core::exec::StepOutcome;

loop {
    let outcome = dispatcher.step(&mut vcpu);
    aero_perf::retire_from_step_outcome(&mut perf, &outcome);

    // `tier` can be used for profiling, but instruction counting does not need to special-case it.
    if let StepOutcome::Block { tier, .. } = outcome {
        let _ = tier;
    }
}
```

If you already have a `PerfWorker`, `aero_cpu_core::exec::ExecDispatcher` also provides
`ExecDispatcher::step_with_perf(&mut vcpu, &mut perf)` as a convenience wrapper around the above pattern.

---

### Paging integration (`PagingBus` + `CpuBus::sync/invlpg`)

#### `CpuBus` contract

Tier-0 reads/writes memory through `mem::CpuBus`, which is intentionally *linear-address based*:

- Tier-0 passes **linear addresses** (after segmentation/A20 masking) to the bus.
- A paging-aware bus is responsible for translating linear → physical.

`CpuBus` also includes two hooks that matter for paging:

- `sync(&CpuState)`: called at instruction boundaries so the bus can observe changes to CR0/CR3/CR4/EFER and CPL.
- `invlpg(vaddr)`: called by the `INVLPG` assist to invalidate a single translation.

See: [`crates/aero-cpu-core/src/mem.rs`](../../crates/aero-cpu-core/src/mem.rs)

#### `PagingBus`

`PagingBus<B>` is the canonical adapter that implements `CpuBus` by wrapping:

- an `aero_mmu::Mmu` (page table walker + TLB), and
- an underlying **physical** bus `B: aero_mmu::MemoryBus`.

It translates every access using `aero-mmu`, and it updates cached MMU state on `sync()` by calling `CpuState::sync_mmu(&mut mmu)`.

See: [`crates/aero-cpu-core/src/paging_bus.rs`](../../crates/aero-cpu-core/src/paging_bus.rs)

---

### Interrupts/exceptions (`interrupts.rs`)

Architectural delivery (IVT/IDT, privilege stack switching, IST, IRET) lives in `aero_cpu_core::interrupts`.

Key types:

- `CpuCore` (re-exported as `aero_cpu_core::CpuCore`) = `{ state: CpuState, pending: PendingEventState, time: time::TimeSource }`
- `interrupts::PendingEventState` tracks:
  - deferred pending faults/traps/interrupts
  - external interrupt FIFO
  - interrupt-shadow state (`STI`, `MOV SS`, `POP SS`)
  - exception nesting / double-fault escalation
  - an internal IRET frame stack (so `IRET*` can validate/match the correct frame)

Execution engines are responsible for aging the interrupt shadow state by calling
`PendingEventState::retire_instruction()` after each successfully executed instruction.
The `exec::Tier0Interpreter` glue handles this for you.

#### How execution glue should deliver faults

Tier-0’s `step()` returns `Result<StepExit, aero_cpu_core::Exception>`. An `Err(e)` indicates a **synchronous fault** at the current instruction boundary (e.g. `#PF`, `#GP(0)`).

The canonical pattern is:

1. Let Tier-0 set architectural side effects (today: CR2 for `#PF`) via `CpuState::apply_exception_side_effects`.
2. Convert the error into an architectural pending event (`exceptions::Exception`) using
   `PendingEventState::raise_exception_fault(...)`.
3. Deliver it via `CpuCore::deliver_pending_event(&mut bus)` at the next boundary.

Most integration code should use `exec::Vcpu` and deliver through `Vcpu::maybe_deliver_interrupt()`, which already routes to `CpuCore::{deliver_pending_event, deliver_external_interrupt}` and records a sticky `CpuExit` on triple fault.

---

### Developer quickstart (tests)

This is the smallest “bring-up” loop for unit tests using the canonical types:

- `exec::Vcpu<FlatTestBus>`
- `exec::Tier0Interpreter`
- external interrupt injection via `PendingEventState`

```rust
use aero_cpu_core::exec::{Interpreter as _, Tier0Interpreter, Vcpu};
use aero_cpu_core::mem::{CpuBus as _, FlatTestBus};
use aero_cpu_core::state::CpuMode;
use aero_x86::Register;

// A tiny real-mode program: NOP; HLT
let mut bus = FlatTestBus::new(0x10000);
let code_base = 0x0100u64;
bus.load(code_base, &[0x90, 0xF4]);

// IVT[0x20] -> 0000:0500, handler = HLT
let vector = 0x20u8;
let handler_off = 0x0500u16;
let ivt_addr = (vector as u64) * 4;
bus.write_u16(ivt_addr, handler_off).unwrap();
bus.write_u16(ivt_addr + 2, 0).unwrap();
bus.load(handler_off as u64, &[0xF4]);

let mut vcpu = Vcpu::new_with_mode(CpuMode::Real, bus);
vcpu.cpu.state.write_reg(Register::CS, 0);
vcpu.cpu.state.write_reg(Register::SS, 0);
vcpu.cpu.state.write_reg(Register::SP, 0x8000);
vcpu.cpu.state.set_rflags(0x0202); // IF=1, bit1=1
vcpu.cpu.state.set_rip(code_base);

let mut interp = Tier0Interpreter::new(1024);

// Run until the first HLT.
interp.exec_block(&mut vcpu);
assert!(vcpu.cpu.state.halted);

// Inject an external interrupt (e.g. PIC/APIC) and run again.
vcpu.cpu.pending.inject_external_interrupt(vector);
interp.exec_block(&mut vcpu);

// The interrupt handler ran and halted.
assert!(vcpu.cpu.state.halted);
assert_eq!(vcpu.cpu.state.segments.cs.selector, 0x0000);
assert_eq!(vcpu.cpu.state.rip(), handler_off as u64 + 1); // HLT advances RIP
assert_eq!(vcpu.cpu.state.read_reg(Register::SP) as u16, 0x7FFA); // 3 pushes in real mode
```

#### Optional: paging-enabled tests

If you want Tier-0 to run with paging enabled, wrap a physical memory bus (`aero_mmu::MemoryBus`) in `PagingBus`:

```rust
use aero_cpu_core::{exec::Vcpu, state::CpuMode, PagingBus};
use aero_mmu::MemoryBus;

struct MyPhysBus { /* ... */ }
impl MemoryBus for MyPhysBus { /* ... */ }

let phys = MyPhysBus { /* ... */ };
let bus = PagingBus::new(phys);
let mut vcpu = Vcpu::new_with_mode(CpuMode::Long, bus);
```

For a concrete `MemoryBus` implementation, see the unit tests under
[`crates/aero-cpu-core/tests/paging.rs`](../../crates/aero-cpu-core/tests/paging.rs).

---

### CPUID / feature model

See [`cpu-and-jit.md`](cpu-and-jit.md) for current CPUID leaf coverage and feature profiles.

## CPU: CPUID & Feature Policy

Windows 7 boot and many drivers gate behavior based on CPUID leaves and feature bits. If we expose **inconsistent** or **overly optimistic** CPUID information, the guest may:

- take an instruction path we do not implement (e.g. SSE4.2/AVX),
- enable paging features we don’t support (e.g. NX without EFER.NXE behavior),
- or fail early during boot due to missing mandatory capabilities.

This repository models CPUID + MSR behavior in `crates/aero-cpu-core`, exposing a coherent x86-64 feature surface for a Windows 7 guest.

### Implemented CPUID Leaves

The CPUID dispatcher in `crates/aero-cpu-core/src/cpuid.rs` implements common leaves used by Windows 7 and typical drivers:

- `0x0000_0000` – vendor string + max basic leaf
- `0x0000_0001` – signature + baseline feature flags
- `0x0000_0002` – legacy cache/TLB descriptors (QEMU-like constants)
- `0x0000_0004` – deterministic cache parameters (simple L1/L2/L3 model)
- `0x0000_0006` – thermal/power (stubbed as 0)
- `0x0000_0007` – extended features (subleaf 0)
- `0x0000_000A` – perf monitoring (stubbed as 0)
- `0x0000_000B` / `0x0000_001F` – topology enumeration (only exposed when `x2APIC` is enabled)
- `0x8000_0000` – max extended leaf
- `0x8000_0001` – extended feature flags (NX/SYSCALL/LM/LAHF-LM, etc.)
- `0x8000_0002..=0x8000_0004` – brand string
- `0x8000_0006` – extended cache info (L2)
- `0x8000_0007` – invariant TSC
- `0x8000_0008` – physical/virtual address sizes

Unknown leaves return 0 (safe default for bring-up).

### Feature Policy (What We Advertise)

The CPU feature surface is modeled in the same “shape” as CPUID via `CpuFeatureSet`:

- `CPUID.1:ECX` → `CpuFeatureSet::leaf1_ecx`
- `CPUID.1:EDX` → `CpuFeatureSet::leaf1_edx`
- `CPUID.7.0:*` → `CpuFeatureSet::leaf7_*`
- `CPUID.80000001:*` → `CpuFeatureSet::ext1_*`

This keeps it explicit *where* a feature bit is reported, which helps avoid accidental inconsistencies.

#### Profiles

`CpuProfile` (see `crates/aero-cpu-core/src/cpuid.rs`) defines two intended configurations:

1. **Win7Minimum** – minimum viable x86-64 CPU for Windows 7 boot:
   - x86-64 / long mode (`LM`)
   - `SSE2`
   - `PAE`
   - `NX`
   - `APIC`
   - `TSC`
   - `SYSCALL/SYSRET`
   - `LAHF/SAHF` in long mode
   - `CMPXCHG16B`

2. **Optimized** – allows additional feature bits (SSE3/SSSE3/SSE4.2/POPCNT, etc.) **only when the emulator implements them**.

#### Overrides

`CpuFeatureOverrides` can force-enable/disable specific CPUID bits for debugging.

By default, `force_enable` is still capped by the `implemented_features` set (we don’t advertise what we can’t execute).

Setting `allow_unsafe = true` allows forcing bits that are not implemented, which is useful for bring-up experiments but is expected to break guests.

### CPUID/MSR Coherence

Some CPUID bits imply MSR behavior. The crate enforces the most important ones for Windows boot:

- If `CPUID.80000001:EDX[NX]` is cleared, writes to `EFER.NXE` are masked.
- If `CPUID.80000001:EDX[SYSCALL]` is cleared, writes to `EFER.SCE` are masked.
- If `CPUID.80000001:EDX[LM]` is cleared, writes to `EFER.LME` are masked.

Unit tests in `crates/aero-cpu-core/tests/cpuid_policy.rs` validate these coherency rules.

## SMP / Multi-vCPU Bring-up

### Status (today)

The canonical machine integrations:

- `aero_machine::Machine` (`crates/aero-machine`)
- `aero_machine::PcMachine` (`crates/aero-machine`)
- `aero_pc_platform::PcPlatform` (`crates/aero-pc-platform`)

are still **SMP bring-up only**, but progress has landed:

- `aero_machine::Machine` can be configured with `cpu_count > 1` and includes basic SMP plumbing:
  per-vCPU LAPIC instances/MMIO routing, AP wait-for-SIPI state, INIT+SIPI delivery via the LAPIC
  ICR, and a bounded cooperative AP execution loop inside `Machine::run_slice`.
  This is sufficient for SMP contract/bring-up tests, but it is **not** a full SMP scheduler or
  parallel vCPU execution environment yet.
  - Tests/coverage: `crates/aero-machine/tests/ap_tsc_sipi_sync.rs`, `lapic_mmio_per_vcpu.rs`,
    `ioapic_routes_to_apic1.rs`, `smp_lapic_timer_wakes_ap.rs`, `smp_timer_irq_routed_to_ap.rs`.
- `aero_machine::PcMachine` and `aero_pc_platform::PcPlatform` still execute only the BSP today;
  `cpu_count > 1` there is primarily for firmware-table enumeration tests.
- Lower-level interrupt-fabric SMP semantics are covered in `crates/platform/tests/smp_*`
  (INIT deassert IPI behavior, IOAPIC destination routing, and MSI destination/broadcast routing).

`cpu_count` is allowed to be `>= 1` so firmware can publish SMP-capable CPU topology (ACPI/SMBIOS)
and platform code can size per-vCPU state (for example LAPIC instances). Multi-vCPU guests are not
expected to run robustly yet (especially OS SMP), but the building blocks are now testable.

Attempting to construct a canonical machine with `cpu_count == 0` fails with
`MachineError::InvalidCpuCount`.

### Why SMP is disabled in the canonical machine/platform

The project has building blocks for multi-vCPU guests (ACPI table generation can emit multiple CPU
entries, and there is a prototype SMP model in `crates/aero-smp/`), but the end-to-end
full-system wiring is not complete yet.

Key missing pieces include (what’s still left even with the current bring-up support):

1. **Robust multi-vCPU execution + scheduling**
   - `aero_machine::Machine` now owns a BSP `CpuCore` plus AP `CpuCore`s and runs APs cooperatively,
     but this is still a minimal bring-up scheduler.
   - Remaining work includes fairness, guest-driven AP execution (not just host-driven bring-up),
     and eventually parallel execution.

2. **AP startup (BSP + AP bring-up)**
   - APs must start in a wait-for-SIPI state.
   - BSP must be able to deliver INIT/SIPI via the Local APIC ICR.
   - Basic INIT/SIPI bring-up is now implemented in `aero_machine::Machine`, but full guest OS AP
     bring-up sequences still need hardening and coverage.

3. **LAPIC/IPI plumbing**
    - Per-vCPU LAPIC state exists and IOAPIC destination routing works for non-BSP LAPICs.
    - Remaining work includes full IPI delivery semantics (AP→BSP/AP, broadcast modes), per-vCPU
      interrupt polling/injection hardening (beyond bring-up), and safety/determinism under nested
      interrupt activity.

4. **Firmware topology and OS discovery**
   - ACPI MADT must enumerate all CPUs and their APIC IDs.
   - DSDT must include `_PR_` processor objects for all CPUs.
   - SMBIOS should report the correct CPU/core count.
   - These pieces exist in `aero-acpi`/`firmware` and can be generated for `cpu_count > 1`, but
     multi-vCPU execution is not wired end-to-end yet.

5. **Snapshot/restore**
   - `aero-snapshot` supports multi-vCPU `CPUS` state, but `aero_machine::Machine` snapshots/restores
     only the BSP CPU state today. AP CPU state + LAPIC/IPI state must be included in a stable,
     deterministic multi-vCPU snapshot contract.
   - SMP bring-up must define the per-vCPU snapshot contract and deterministic restore ordering.

### Where to track progress

- This doc (`cpu-and-jit.md`) is the canonical “what’s missing / what’s next” reference.
- Prototype SMP plumbing lives in `crates/aero-smp/` (APIC IPI + AP startup state machine).
  - For backwards compatibility it can also be accessed via `emulator::smp` (a pure re-export shim).
- The higher-level roadmap mentions multi-core as a Windows 7 compatibility milestone:
  - `../history/project-history.md` → “Phase 4 … Multi-core / SMP emulation”
- Firmware-side conceptual background (not necessarily implemented end-to-end yet):
  - `platform-and-firmware.md` → “SMP Boot (BSP + APs)”

### Recommended workarounds (until SMP lands)

- **For real guest boots today, use `cpu_count = 1`** for canonical machines/platforms.
- For faster “boot to desktop” workflows, use snapshots (`../specs/snapshot-format.md`) rather than relying
  on multi-core to speed up boot.
- If you specifically want to experiment with AP startup/IPI logic (without full PCI/BIOS/Windows),
  look at the unit-test-oriented SMP prototype in `crates/aero-smp/`.
