# Aero — Repo State & Structure

> **This is the project's source-of-truth state document.** It is *evergreen*:
> edit it in the same change that makes something true, so it never describes a
> stale reality. Dated entries belong in §8 (History & decision context);
> everything else must read as *current* fact. If you cannot verify a claim
> against the tree, mark it **[unverified]** or delete it.
>
> Working agreements (how we work, doc style): `wiki/meta/working-agreements.md`.

---

## What Aero is

Aero is a **Windows 7 SP1 (32-bit and 64-bit) system emulator that runs
entirely in the browser**. The emulator core is Rust compiled to WebAssembly;
the host is TypeScript on Web Workers with `SharedArrayBuffer` (therefore the
page must be cross-origin isolated — COOP/COEP, the cross-origin isolation decision). Graphics is
translated from Direct3D 9/10/11 to WebGPU, accelerated in-guest by a custom
**AeroGPU** WDDM 1.1 driver; storage prefers OPFS with an IndexedDB fallback;
networking is an L2 tunnel over WebSocket to an unprivileged userspace proxy
(the L2 tunnel networking decision); performance-critical guest devices use **virtio** Windows 7 kernel
drivers (mostly WDM; only virtio-input is KMDF).

The top-level `AGENTS.md` describes the ambition. This document describes the
reality.

## Current state (kept current)

**Phase: the Windows 7 desktop is reached natively; the browser desktop is
not.** The native runner boots an installed Windows 7 SP1 x64 guest all the way
to a fully rendered desktop — the Harmony wallpaper, the Recycle Bin, and the
taskbar with Start orb, Internet Explorer, Windows Explorer, Media Player and a
running notification-area clock, **36 processes**, and **479,996 of 480,000
pixels** non-black at 800×600×32 — and does
so **without scaffolding**: no memory pokes, no stubbed kernel routines, no
forced resume, no JIT. From a cold `--boot hdd` the logon screen is about 19
billion instructions in. The result reproduces from its checkpoint, and was
reproduced byte-identically on a different interpreter build — the captures and
run logs are the evidence for the figures above; the byte-identical reproduction
was observed at the time and has no committed artefact, so treat that specific
claim as **[unverified]**.

![The Windows 7 desktop running in the Aero emulator](../assets/win7-desktop.png)

The headline goal is the desktop *in the browser*, and that is **not achieved**.
The graphics status matrix (`wiki/areas/graphics.md`) still marks browser WDDM
end-to-end as `[ ]`, and no guest driver has ever been compiled. What the native
result establishes is that the CPU, memory, platform, firmware and boot-display
paths are correct enough to run the real operating system to its shell.

**What remains is wider than the display path.** The stated completion criterion
is: Windows 7 boots to an interactive desktop in the canonical browser host,
verified headlessly; the guest browses the web through the L2 tunnel — a page
actually fetched from inside the guest; audio output is demonstrated; and the
milestone-ladder checkpoints are green. Two of those have never been exercised
against a Windows 7 guest at all: **no guest has ever been driven through the
networking stack or the audio stack**, only through their unit and contract
tests. The inbox drivers for both are verified to bind
([windows-drivers.md](../areas/windows-drivers.md)), which is a different and
much weaker statement than "it works".

What *should* be verified in a real browser is the machine-to-scanout path: four
Playwright specs (`tests/e2e/canonical_machine_vga_panel*.spec.ts` and
`canonical_machine_vga_worker_panel*.spec.ts`) drive the actual host page and
assert a non-black VGA scanout out of the canonical machine. Be precise about the
strength of that, though: they assert on the scanout, **not** on the absence of
page errors, and every one of them skips when the WebAssembly bundle is absent —
and per the table above, the browser suite has **not been run in this
environment**. So this is a check that exists, not a result that has been
observed here.

The plumbing from the emulated display through WebAssembly to a canvas is
therefore *covered*; running the Windows 7 guest through it is not even that. An
earlier static-image page used as evidence for this was correctly demoted and no
longer exists.

Getting there needs four environment settings, none of which alters guest
behaviour the way the retired bring-up wedges did:

- `AERO_AEROGPU_STDVGA_IDS=1` presents standard-VGA identity so the inbox
  `vga.sys` binds, the AeroGPU WDDM driver being uncompiled.
- `AERO_STDVGA_ENABLE_IO=1` is a **compatibility wedge for old checkpoints
  only**. PCI POST now enables legacy I/O decode for every VGA-*compatible* function (class `03/00`)
  (`bios_post_enables_fixed_legacy_io_decode_for_vga_compatible_controllers`), so
  a cold boot on current firmware does not need it; a snapshot taken before that
  rule landed preserves its old command word and does.
- `AERO_PM_TIMER_POLL_QUANTUM=300000` advances the timestamp counter and every
  platform clock together after a power-management timer read, so a polling loop
  makes progress without any clock diverging from the others.
- `AERO_RDTSC_QUANTUM=2000` survives for recipe compatibility and now only
  yields timestamp-poll loops to the scheduler. Its historical behaviour — broadly
  fast-forwarding the counter — is **banned**, having been shown to suppress
  plug-and-play root initialisation.

> **Evidence status.** The proof captures are in the repository at
> [`../assets/`](../assets/README.md), and the tools used to configure the guest
> are in `tools/win7-bringup/`. What is *not* durable is the checkpoints
> themselves — half a gigabyte each, still in a scratch working directory rather
> than the durable snapshots directory (`$AERO_IMAGES_DIR/snapshots/`). They are
> too large to commit, so moving them there is outstanding. An earlier scratch
> directory holding the evidence for a large body of bring-up findings has
> already been lost once; see
> the rule in [../meta/working-agreements.md](../meta/working-agreements.md).

### The seven failures are one theme: secondary-channel IDE/ATAPI interrupts

They will be the first thing a fresh session trips over, so:

| Target | Failing | What it asserts |
|---|---|---|
| `aero-machine` `machine_ide_identify_irq14` | 4 | two on ATAPI IDENTIFY word 0, two on IRQ15 timing |
| `aero-machine` `machine_mmio_pci_and_imcr` | 1 | the same IDENTIFY word, after an interrupt-mode switch |
| `aero-pc-platform` `aero_snapshot_storage_controllers` | 1 | `pic_pending_irq` is `None` after restore, expected `Some(14)` |
| `aero-pc-platform` `storage_controllers_st010` | 1 | same: IRQ14 level lost across snapshot restore |

Three of them assert ATAPI IDENTIFY word 0 is `0x8580` where the model returns
`0x85C0` — a difference of bit 6, the **DRQ response type**, not the
removable-media bit (bit 7, set in both). The model deliberately advertises the
interrupt-DRQ form; the assertions were left behind.

The other four are behavioural and *do* drive the machine: two assert IRQ15 has
not fired before bus-master DMA is enabled, and two assert an IRQ14 line survives
snapshot save and restore. Those are reachable by scheduling, interrupt or
snapshot changes — if they start failing *differently*, that is a signal, not
noise.

> Two earlier versions of this table were wrong, which is worth knowing before
> trusting it: it once claimed two `aero-l2-proxy` UDP metric tests were among
> the failures. **They pass** — they need raw ICMP, which this host has. Re-run
> before quoting a number, and use `--no-fail-fast`.

Do not confuse this with the *other* ATAPI-identify entry in the "suite is the
arbiter" list below, which is a different, real regression about the 12-byte
command-packet advertisement.

### Resuming the desktop

```
aero-machine --disk          <work>/win7-installed.raw
             --disk-overlay  <work>/win7-desktop.aerospar
             --ram           2048
             --boot          hdd
             --snapshot-load <work>/win7-desktop.bin
```

`<work>` is the bring-up working directory, wherever you keep it.
`win7-installed.raw` is the flattened install with the offline logon
configuration applied; if it is lost, rebuild it from the base HDD image in
`$AERO_IMAGES_DIR` plus the bring-up overlay.

Two hazards. **A checkpoint is paired with a specific overlay** — the desktop
checkpoint has its own, and resuming it against a different one prints a warning
and produces something that is not evidence. And **`--ram` must match the
snapshot**; these are 2 GiB snapshots.

Before opening any overlay, run `pgrep -a aero-machine`. Two emulators sharing
one copy-on-write overlay corrupt it, and after a timeout the survivor is not
always obvious — kill the exact process and verify it exited.

The wider checkpoint lineages, including the QEMU ground-truth captures and every
cold-boot milestone, are under the boot-output tree (`$AERO_BOOT_OUT`, several
hundred snapshots). `$AERO_IMAGES_DIR/snapshots/` holds the ten survivors of the
scratch-directory loss.

| Area | State | Evidence |
|---|---|---|
| Win7 boot-to-desktop (browser) | Not achieved / unverified | `wiki/areas/graphics.md` |
| Farthest in-emulator boot | **The Windows 7 desktop**, natively and without scaffolding: wallpaper, Recycle Bin, and a taskbar with Start orb, Internet Explorer, Explorer, Media Player and a running tray clock. 36 processes; `non_zero=479996` of 480,000 pixels. (The earlier taskbar-only state is `non_zero=32144` — a 800×40 strip on a black desktop.) Reproduces from its checkpoint. Captures: [`assets/win7-desktop.png`](../assets/win7-desktop.png). | [../history/windows-7-bring-up.md](../history/windows-7-bring-up.md) |
| Win7/FreedOS "boot tests" | Run under **QEMU**, not Aero's CPU; `#[ignore]`d harnesses | `tests/windows7_boot.rs`, `tests/freedos_boot.rs` |
| CPU | Tier-0 interpreter is the validated path | `crates/aero-cpu-core` |
| JIT | Real two-tier x86→WASM JIT exists (tier1 block, tier2 trace/opt), but guest-CPU bench notes say only `mode: interpreter` is expected to work initially | `crates/aero-jit-x86/` (~57k LOC), `bench/` |
| D3D9 / D3D10-11 translation | Partial (`[~]` in status matrix): SM3→WGSL, DXBC parsers, D3D9 subset executor, D3D11 with GS-via-compute prepass + tessellation | `crates/aero-d3d9`, `crates/aero-d3d11`, `crates/aero-dxbc` |
| Windows drivers | Full source for virtio blk/net/input/snd plus the AeroGPU kernel and user-mode drivers, with build, signing and packaging tooling — **and not one compiled binary**, because the kit is Windows-only. Note only virtio-input is KMDF (1.9); blk, net, snd and the AeroGPU kernel driver are WDM | `drivers/`, `drivers/build/` |
| Rust build and tests | Green; see verification status | — |
| JS/TS build and tests | Cannot be run in this environment; see verification status | — |
| CI | None. Workflow automation is purged and banned by the cleanup doctrine — gates are commands anyone can run | [../meta/working-agreements.md](../meta/working-agreements.md) |

### Non-goals / reality checks (commonly misunderstood)

- `AGENTS.md`'s milestone table (boot time, FPS, app-compat) is **aspirational
  planning**, not measured reality.
- "virtio-gpu" appears in old docs; for Win7 the actual graphics path is the
  custom **AeroGPU** device + driver (PCI ID `A3A0:0001`, the AeroGPU PCI IDs and ABI decision). virtio
  covers blk/net/input/snd only.
- The tiered-JIT story is real code but not yet the shipping execution mode.

## Verification status

Run via `bash ./scripts/safe-run.sh` (mandated resource wrapper; see
`wiki/areas/testing.md`).

| Gate | Status |
|---|---|
| `cargo check --workspace --locked` | ✅ green |
| `cargo test --workspace --locked --no-fail-fast` | 🟡 **9,766 pass, 7 fail**, all seven in one theme — see the note below. Use `--no-fail-fast`: without it the run stops early and undercounts. |
| `bash scripts/ci/check-repo-policy.sh` | ✅ green |
| `bash scripts/ci/check-repo-layout.sh` | ✅ green |
| `pnpm -w run typecheck` | ✅ green |
| `pnpm -w run test:unit` | ✅ green |
| Playwright end-to-end | ⚠️ not yet run in this environment |
| Windows driver builds | ⚠️ not runnable here; the toolchain is Windows-only |

**Put the pinned toolchain on `PATH` before concluding anything about the
JavaScript gates.** The system `node` is a different, unsupported major; the
pinned Node and pnpm are installed under `fnm` and are invisible until you
activate them:

```bash
export PATH="$(dirname "$(fnm which node 2>/dev/null || command -v node)"):$PATH"
node --version   # v24.18.0
pnpm --version   # 11.5.0
```

Without that, `node --version` reports the system major, `pnpm` is not found at
all, and the root `node_modules` looks nearly empty — pnpm's isolated layout
keeps real packages in `node_modules/.pnpm` and links a handful at top level.
Every one of those signals reads as "the JavaScript side is not installed", and
all three are wrong. Check `node_modules/.pnpm` before believing them.

Do not export `AERO_ALLOW_UNSUPPORTED_NODE` while running the suite either: it
propagates into the child processes that the node-version contract test spawns,
and the test then observes a warning where it asserted an error.

Two things worth knowing before running the Rust suite:

**Give it address space.** `scripts/safe-run.sh` defaults to a 12 GiB
`RLIMIT_AS`, and `aero-machine`'s hole-aware high-memory tests construct
multi-gigabyte guest physical maps. Under the default cap they abort with an
allocation failure that looks like a code defect and is not one. Run the suite
with `AERO_MEM_LIMIT=48G`.

**The suite is the arbiter of the bring-up work, not the other way round.**
Several regressions have entered by changing device behaviour for the Windows 7
bring-up and not re-running the suites that pin that behaviour. Each was a real
find, and each is worth remembering as a shape:

- A guard asserting on the text of a runtime error message drifted from the
  message itself, so purging `docs/` broke a test with no apparent connection to
  documentation.
- An instrument latched its configuration in a `OnceLock` on first use, which is
  right on a hot path and wrong in a test binary where the first sibling test to
  touch memory decided the setting for every test after it.
- The ATAPI identify word was corrected to advertise 12-byte command packets,
  and three assertions elsewhere still expected the old value.
- Two device models advance their own clock when a *port is read*, so that a
  guest spin-wait inside one instruction batch can make progress. Host-side
  inspection went through the same path, so looking at the machine changed it.
  Reads issued by the host now suppress the nudge.
- A test asserted an intermediate liveness condition — "the AP is observably
  running" — that a fast enough AP legitimately skips by finishing inside the
  first slice. The completion flag it sets is the real evidence.

## Toolchains & pins (the spec)

| Tool | Pin | Where pinned |
|---|---|---|
| Rust (main) | stable **1.92.0** | `rust-toolchain.toml` |
| Rust (threaded WASM) | **nightly-2025-12-08** with `-Z build-std` | `scripts/toolchains.json` |
| Node | **24.18.0** | `.nvmrc`, matched by every manifest's `engines`, enforced by `scripts/check-node-version.mjs` |
| Go | 1.22 | `proxy/webrtc-udp-relay/go.mod` |
| Package manager (JS) | **pnpm 11** | `packageManager` in root `package.json`; `pnpm-workspace.yaml` is the only workspace declaration |
| Lockfiles | `Cargo.lock` and `pnpm-lock.yaml`, both committed; every build is `--locked`/`--frozen-lockfile` | — |
| Task runners | `cargo xtask` canonical (`wasm`, `web`, `test-all`, `wasm-check`, `fixtures`, `snapshot`, `conformance`); `justfile` aliases | `xtask/src/main.rs` |
| Resource wrapper | `scripts/safe-run.sh` required for builds/tests | `AGENTS.md` |

JavaScript workspaces (11): `apps/web`, `bench`, `services/gateway`,
`services/net-proxy`, `packages/{aero-stats,transport-safety}`,
`tools/{perf,range-harness,net-proxy-server,disk-streaming-browser-e2e}`,
`proxy/webrtc-udp-relay/e2e`. `pnpm-workspace.yaml` is the *only* workspace
declaration — `package.json` has no `workspaces` key, and the hygiene contract
test fails if one reappears. (`packages/shared/` has no `package.json` and is
therefore not a member; it is imported by path.)

**Rust is one workspace: 94 listed members** (95 packages including the root). Every tool that once had its own
workspace — the disk gateway, the image chunker, the slipstreamer, the packager,
the certificate-store exporter — has joined it. `fuzz/` is the sole standalone
workspace, and only because cargo-fuzz requires one.

Both lockfiles have known drift; see
[known debt](#known-debt--problem-areas-ranked-by-cleanup-relevance).

## Structure

### 5.1 Top-level map

| Path | What it is | Canon |
|---|---|---|
| `crates/` | The Rust emulator core, including `aero-protocol` (the AeroGPU ABI mirror with its cross-language conformance tests) | active |
| `apps/web/` | **The canonical browser host** — shell, runtime modules, workers, storage, GPU, networking, and the WASM build tooling, merged from the former root `src/` and `apps/web/` trees. `vite.harness.config.ts` sets COOP/COEP and CSP. Two shells over one module library: `index.html` (canonical, driven by Playwright) and `bringup.html` (**legacy**, still used for manual Windows 7 bring-up) | canonical |
| `drivers/` | Windows driver C/C++ source: `windows7/virtio-{blk,net,input,snd}` (KMDF), `aerogpu/` (WDDM 1.1 KMD + D3D9/10/11 UMDs), `protocol/` canonical AeroGPU ABI C headers, `virtio/prebuilt/` (README only; no binaries in-tree) | active |
| `services/gateway/` | The networking backend: WebSocket `/tcp` and `/tcp-mux`, DNS-over-HTTPS, sessions | canonical |
| `services/net-proxy/` | TS local-dev WS→TCP/UDP relay + DoH (runs beside `vite dev`) | active (dev) |
| `proxy/webrtc-udp-relay/` | Go UDP relay over WebRTC DataChannel (pion), WS fallback | active; **slated for port to TS** |
| `drivers/build/` | The driver build system — kit install, build, catalog, sign, package. PowerShell because the kit is; it is a build system, not CI | active |
| `bench/` | pnpm workspace package `aero-bench`: scenarios, runner, dashboard, `history.json`, `perf_thresholds.json` | active |
| `fuzz/` | Separate cargo-fuzz workspace, 40+ targets (DXBC, D3D9 SM3, D3D11 SM4, AHCI/ATAPI, E1000, EHCI, HDA, HID, sparse disk formats, BIOS ints) | active |
| `protocol-vectors/` | Canonical golden JSON vectors (auth-tokens, l2-tunnel, tcp-mux, udp-relay, origin, device contracts, HID/webusb fixtures) consumed by cross-language conformance tests | active |
| `tools/` | ~20 tools (`bcd_patch`, `win7-slipstream`, `qemu_diff`, `aero-disk-convert`, `image-chunker`, packager, `disk-gateway`, …) | active |
| `tests/` | Root integration tests (Rust boot tests, JS contract tests) + `tests/e2e/` (Playwright specs) + `tests/helpers/` | active |
| `guest-tools/` | Guest-tools packaging (`setup.cmd`, `verify.*`, certs, driver drop) + `unattend/` (Win7 SP1 unattend media) | active |
| `assets/` | `bios.bin` | active |
| `packages/` | Shared TypeScript libraries: `shared/` (status and API types) and `aero-stats/` (metric definitions used by the HUD, exports and bench tooling) | active |
| `scripts/` | Dev/agent env + source-check scripts; `safe-run.sh` is the mandated wrapper | active |
| `xtask/` | The canonical task runner (`wasm`, `web`, `test-all`, `wasm-check`, `fixtures`, `snapshot`, `conformance`); the `justfile` is thin aliases over it | active |
| `wiki/` | **The source of truth** | current |

### 5.2 Rust crates by subsystem

| Subsystem | Crates |
|---|---|
| CPU | `aero-cpu-core` (state, Tier-0 interpreter, JIT runtime), `aero-cpu-decoder`, `aero-x86`, `aero-mmu`, `aero-smp` |
| JIT | `aero-jit` (facade) → `aero-jit-x86` (`src/tier1/` block codegen, `src/tier2/` trace/opt/verify/wasm_codegen), `aero-jit-wasm` |
| Machine / WASM API | `aero-machine` (**canonical VM**, the canonical VM core decision/0014), `aero-wasm` (`Machine` bindgen surface), `aero-gpu-wasm`, `aero-d3d11-wasm`, `aero-machine-cli` |
| Graphics | `aero-d3d9`, `aero-d3d11`, `aero-dxbc` (DXBC parser), `aero-gpu` (+`-trace`, `-replay`, `-utils`), `aero-gpu-vga`, `aero-webgpu`, `aero-aerogpu`, `aero-devices-gpu`, `aero-edid`, `virtio-gpu-proto`, `legacy/aero-d3d9-shader` |
| Storage | `aero-storage`, `aero-storage-adapters`, `aero-opfs`, `st-idb` (IndexedDB), `aero-http-range`, `aero-storage-server`, `aero-devices-storage`, `aero-devices-nvme` |
| Network | `aero-net-stack`, `aero-net-e1000`, `aero-net-backend`, `aero-net-pump`, `aero-net-trace`, `aero-l2-proxy` (+`aero-l2-protocol`), `aero-tcp-mux-protocol`, `aero-udp-relay-protocol`, `aero-pcapng`, `aero-auth-tokens` |
| USB / input | `aero-usb` (UHCI/EHCI/xHCI + HID; **canonical**, the canonical USB stack decision), `aero-devices-input` |
| Audio | `aero-audio` (Intel HDA + codec, AudioWorklet helpers; **canonical**, the canonical audio stack decision) |
| Platform / firmware | `aero-acpi`, `aero-pc-platform`, `aero-pc-constants`, `aero-pci-routing`, `aero-pci-firmware-adapter`, `aero-interrupts`, `aero-timers`, `aero-time`, `aero-virtio` |
| Support | `aero-ipc`, `aero-shared`, `aero-snapshot`, `aero-io-snapshot`, `aero-types`, `aero-core`, `aero-mem`, `aero-guest-phys`, `aero-debug`, `aero-perf`, `aero-fuzz`, `conformance`, `nt-packetlib`, `aero-boot-tests`, `aero-tests`, `aero-microbench` |
| Graphics (device side) | `aero-devices-gpu` (AeroGPU PCI device, ring, executor), `aero-gpu-software` (CPU rasterizer for the command stream), `aero-gpu-vga`, `aero-gpu-trace` |
| **Legacy stack** | `crates/devices(+compat)`, `crates/memory`, `crates/platform(+compat)`, `crates/firmware` (BIOS), `crates/perf`, `crates/legacy/*` — legacy per the repo-layout decision (wiki/decisions/0001) + `wiki/history/retirements.md`; migration outstanding (see wiki/areas/platform-and-firmware.md) |

### 5.3 Browser host

- **Canonical host**: `apps/web/`, entry `apps/web/index.html`, which Playwright drives. `apps/web/bringup.html` is a second shell kept for manual Windows 7 bring-up and is legacy.
- **Workers** (ADR-era architecture): `apps/web/src/workers/{cpu,gpu,io,jit,net}*.worker.ts`
  incl. `machine_cpu.worker.ts`, `gpu-worker.ts`; coordination in
  `apps/web/src/runtime/coordinator.ts`; shared memory per the shared memory layout decision (multiple SABs,
  wasm32 4 GiB constraint).
- **Two VM runtime modes**: `legacy` (WASM CPU + TS device shims,
  `tests/node_vm_harness/vm_coordinator.js`) and `machine` (`aero_machine::Machine`;
  required for the Win7 storage topology: AHCI ICH9 + PIIX3 IDE/ATAPI).
- **WASM build**: `apps/web/scripts/build_wasm.mjs` → `apps/web/src/wasm/pkg-single*` and
  `pkg-threaded*` (gitignored); runtime selects variant (the WebAssembly build variants decision).

## Known debt & problem areas (ranked by cleanup relevance)

1. **Boot gap** — no verified Windows 7 boot-to-desktop. The central open goal;
   everything else on this list is subordinate to it.
2. **No guest driver has ever been built.** `drivers/` carries full source and
   INFs for the AeroGPU WDDM driver and the four virtio drivers, and not one
   compiled binary. The kit is Windows-only and work happens on headless Linux,
   so the entire guest half of the graphics and virtio stacks is unexercised by
   construction. This is why display bring-up runs through
   `AERO_AEROGPU_STDVGA_IDS=1`, impersonating a Bochs adapter so the inbox
   `vga.sys` will bind at all. Every AeroGPU claim in the tree is host-side.
3. **AeroGPU has two integration surfaces** — the canonical machine glue and
   `crates/aero-devices-gpu` — not yet consolidated, plus a CPU rasterizer in
   `crates/aero-gpu-software` that the backend trait cannot currently reach. See
   [../areas/platform-and-firmware.md](../areas/platform-and-firmware.md).
4. **Two entry points, one tree.** `apps/web/index.html` is the shell CI and
   Playwright drive; `apps/web/bringup.html` is the fuller one used for manual
   Windows 7 bring-up. They share the module library, which is what the merge
   was for. The second goes when the first can do that job.
5. **The Go relay is still Go.** `proxy/webrtc-udp-relay/` is 99 `.go` files
   plus its own `go.mod` and compose file — the last language in the tree
   outside Rust, TypeScript and the sealed C/C++ driver domain. The intended
   end state is a TypeScript port at `services/webrtc-udp-relay/`, with the wire
   protocol held fixed by the golden vectors in `protocol-vectors/`. Those
   vectors are what make it verifiable, and it is still a rewrite of a component
   that currently works — so this is a real project, not a directory move.

   The target binding is **`node-datachannel`** (maintained bindings to
   libdatachannel, 0.32.x, Node ≥ 18.20). The alternatives were considered and
   rejected, and the reasoning is worth keeping because whoever does the port is
   exactly the person who will re-ask: `wrtc`/`node-webrtc` is
   [archived](https://github.com/node-webrtc/node-webrtc) and dead;
   [webrtc-rs](https://github.com/webrtc-rs/webrtc) is in maintenance only; and
   [str0m](https://github.com/algesten/str0m) is Sans-I/O, which costs more
   integration work than a small relay justifies.
   ([node-datachannel](https://github.com/murat-dogan/node-datachannel).)
9. **Two scope questions are open, not settled.** `bench/` is currently a
   workspace package; whether it stays one or its scenarios fold into `tests/`
   was deliberately deferred until the structural migration reached it, and it
   never did. And `tests/` was to be reorganised into `tests/{e2e,contract}/`;
   `tests/e2e/` exists, but the contract tests are still flat files in `tests/`.
   Neither is urgent; both are recorded so they are not mistaken for decisions.
6. **`crates/legacy/` is nearly empty and slightly wrong.** It holds a README and
   `aero-d3d9-shader`, which stays because the fuzz workspace genuinely uses it.
   The README claims the legacy crates are excluded from the workspace; the
   manifest lists that crate as a member. One of the two is a defect.
7. **Deliberately deferred dependency majors** — most consequentially **wgpu,
   pinned at 0.20.1**, plus vitest 4, Vite 8, TypeScript 6, autocannon 8,
   fast-check 4, and the hyper 1.x migration for the `aero-storage` test servers.
   Each needs a deliberate pass, not a bot bump; the cost of each lift is in
   [../areas/build-and-tooling.md](../areas/build-and-tooling.md#the-deferred-majors),
   alongside the verified toolchain defects (a dead
   Cargo lockfile, and the ban list existing in three divergent copies).
8. **A read-side clock nudge remains in the VGA model.** The retrace bit
   advances its own time when the port is *read*, so a guest spin-wait inside a
   single instruction batch can make progress
   (`crates/aero-gpu-vga/src/lib.rs`). It works, and it is a hack around
   virtual-time granularity being too coarse in I/O loops; host-side reads
   suppress it so that inspecting the machine does not change it. The real fix is
   to drive it from a virtual clock sampled at I/O time and delete the nudge.
   The ACPI power-management timer **had** the same defect and no longer does —
   it is a pure sample of shared platform time now, with acceleration available
   as an explicit, coherent opt-in. That is the model to copy here.

## Architecture decisions (ADR digest)

Full text: `wiki/decisions/`. One-line summary of each standing decision:

| ADR | Decision |
|---|---|
| 0001 | Repo layout: Rust workspace + `apps/web/` Vite app; legacy stack marked |
| 0002 | Cross-origin isolation (COOP/COEP) to enable threads + SharedArrayBuffer |
| 0003 | Shared memory = multiple SABs (avoid single >4 GiB buffer; wasm32 limit) |
| 0004 | Two WASM build variants (threaded / single), selected at runtime |
| 0005 | AeroGPU PCI IDs + ABI canonical (`A3A0:0001`), with deprecation plan |
| 0006 | Node monorepo: one pnpm workspace, one lockfile, one workspace declaration |
| 0007 | Rust crate naming conventions |
| 0008 | `aero-machine` is the canonical VM core crate |
| 0009 | Rust toolchain policy: pinned stable; pinned nightly only for threaded WASM |
| 0010 | Canonical audio stack: `aero-audio` + `aero-virtio`; legacy emulator audio gated |
| 0012 | `Cargo.lock` committed; builds use `--locked` |
| 0013 | Networking: L2 tunnel (Option C) to unprivileged proxy; browser forwards frames |
| 0014 | Canonical machine/VM stack = `aero-machine` |
| 0015 | Canonical browser USB stack = `crates/aero-usb` + `apps/web/src/usb/*` |
| 0016 | Win7 virtio driver naming + layout |

(No ADR 0011 exists.)

## History & decision context (dated; append-only)

### 8.1 The sprint (2026-01-10 → 2026-01-17)

The entire repo history was written in **8 days** by massively parallel
agent-driven development on EC2: 31,667 commits (authors: `Wilson Lin`,
`root@ip-10-140-*.ec2.internal`). Cadence peaked at 10,626 commits on 01-13.
Work was organized via 8 workstreams (`instructions/`). The final 3 days were a
repo-wide security-hardening pass driven by `REFACTOR.md` (61 phases: bounded
error strings, strict UTF-8 protocol decoding, WS close-race hardening,
sink-guardrail contract tests, CI action hardening) — not boot-gap closure.
The repo then sat untouched for ~6 months.

At freeze, `main` @ `98dafdd32` **did not compile**
(`crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`: missing
`bind_empty_groups_before_vertex_pulling` import + duplicated
`dummy_storage_texture_views` field — verified against a clean worktree).

### 8.2 Branch consolidation (2026-07-22)

All 60 non-`main` branches (15 feature, 45 dependabot) were absorbed and
deleted; `main` advanced `98dafdd32 → 4f9b19ac1` (+101 commits) and is green
per §3. Dispositions:

- **Merged**: `refactor/hardening-guardrails` (22 commits, +7.6k lines
  hardening — fast-forward), 4 feature branches (EHCI HCRESET mux reset, 3
  virtio-net VLAN bounds), 29 dependabot bumps.
- **Already on main (patch-identical)**: d3d9 host-backed mip uploads,
  indexed-draw GS runtime (×2 snapshots), EHCI HCRESET (×3 duplicate
  snapshots), virtio-net VLAN size bound (×1 duplicate). Deleted unmerged.
- **Superseded**: 3× `agent236/xhci-015*` snapshots (main's own 4-commit xHCI
  PCI identity series is newer); 4× `types/node-25.0.6` (main has `^25.0.9`
  hoisted).
- **Re-applied onto current main**: reqwest 0.11.27→0.12.28 (root +
  tools/image-chunker), jsonwebtoken 9→10 (disk-gateway), which 6→8
  (win7-slipstream).
- **NOT carried** (deliberate; recorded here as the decision log):
  - vitest 4 / `@vitest/coverage-v8` 4 (×2 branches), vite 7/8, typescript 6,
    autocannon 8, fast-check 4, and the `test-lint-tooling-major` groups (×3)
    — major toolchain migrations that must be done deliberately, not by merging
    6-month-stale bot branches into restructured manifests.
  - `tools/image-chunker` cargo group bump — stale; a fresh `cargo update`
    pulls versions requiring rustc newer than the pinned 1.92.0.
  - hyper 1.8.1 for `aero-storage` dev-deps — breaks five hyper-0.14-era test
    servers; reverted to 0.14 pending a deliberate hyper 1.x test migration.
- **Breakage repaired during consolidation** (all verified green after):
  axum 0.8 ws API (`Message::Binary`/`CloseFrame` — aero-l2-proxy,
  aero-gateway-rs), axum-core 0.5 `Option<ConnectInfo>` removal
  (aero-gateway-rs), rand 0.9 API (disk-gateway, aero-jit-x86 tests), zip 7
  `SimpleFileOptions` (aero-packager), the pre-existing aero-d3d11 compile
  error, fuzz lockfile regen.
- Deleting branch heads auto-closed any open PRs on those heads; the commits
  remain reachable via GitHub `refs/pull/*`.

### 8.3 Working-agreement changes

- 2026-07-22: **No automatic commits** — git mutations (commit/push/etc.) only
  on explicit user request.
- 2026-07-22: `wiki/` established as the living source of truth; meta /
  working style encoded in `wiki/meta/working-agreements.md`.
- 2026-07-22: Repo declared **greenfield, owned entirely by agents** — full
  license to act anywhere in the tree (the working agreements §1.1); git-mutation
  rule above still applies.
- 2026-07-22: **Engineering-principles constitution adopted** as normative
  (the working agreements Part 2): verification-first feedback loops, effects-as-
  values, testing policy (spec-derived oracles, never weaken a failing test),
  knowledge/stigmergy practice (record surprises), delegation discipline
  (planner/worker, handoffs not transcripts, verify like an outsider),
  context discipline.
- 2026-07-22: **Cleanup doctrine adopted** (now in `wiki/meta/working-agreements.md`): the repo
  is an agent-owned engineering project, not a simulated open-source
  foundation — project theater (CI, governance/legal templates, production
  infra for imaginary deployments) is purged and banned; everything outside
  the browser-Win7-emulator scope justifies itself or goes; all docs are
  absorbed into `wiki/**` (CURATE→purge for `docs/`). Execution pending
  go-ahead.
- 2026-07-22: **Method refinements**: (a) design-first — ideal target
  structure designed from first principles, not overfitting the sprint-era
  tree (recorded in `wiki/decisions/`, backed by research: rustwasm org
  archived, wasm-bindgen-cli direct, wgpu 30, WebGPU Baseline; pnpm 11,
  Node 24, TS 7.0 GA w/ TS 6 bridge, vitest 4.1, Vite 8/Rolldown, Biome 2.5;
  drivers pinned to WDK 10.0.19041 + VS2019, windows-drivers-rs rejected;
  Go module retires → TS + node-datachannel). (b) absorb-not-bulldoze —
  per-file review, merge/pick-superior, purge only residue. Also resolved:
  the license-files decision (purge ALL of them), the meta-docs location
  decision (meta → wiki/meta, thin AGENTS.md pointers), the services-scope
  decision (services/image-gateway = production LARP → purge after protocol
  absorption), the windows-directory decision (windows/ unattend → guest-tools/).
- 2026-07-22: **House rules added** (the working agreements §1.2): absorption
  fallout lives in `wiki/notes/` (non-normative); **ban on coded
  identifiers** (task codes like `A1`/`W2`/`PF-008`, decision IDs) as
  primary names — natural-language names everywhere; **design lives in
  docs, not code comments** — comments may cite wiki topics by
  natural-language name; agents always run on a **headless Linux VM**
  (Windows/GUI-requiring steps, e.g. WDK driver builds, are marked and
  deferred).
- 2026-07-23: **Wiki-gardening rule encoded** (the working agreements §1.2, rule
  2): the wiki is the memory — every investigation, probe, decision, and
  discovery is absorbed into it in the same breath as the work, as
  *gardening* (extend the existing structure, keep the whole coherent),
  not bulldozing or grafting. If it isn't in the wiki, it didn't happen.
- 2026-07-22: **Host-language decision — TS shell + Rust workers stays.**
  Full-Rust host (Leptos/Dioxus/Yew) researched and rejected for Aero's
  shape: no framework WebGPU story (all drop to web-sys), single-threaded
  framework assumption vs our SAB/multi-worker build, AudioWorklet
  inexpressible in web-sys (a JS shim is unavoidable), 0.x ecosystem risk,
  worse debugging. Open option recorded: egui-on-wgpu for the debug/HUD
  layer (see the host language decision in `wiki/decisions/`).
- 2026-07-22: **Driver-language decision — all guest drivers stay C/C++**
  (deep-verified). "Windows has Rust APIs" holds only for Win10 2004+,
  x64/ARM64, WDM/KMDF-1.33-class — and even there non-production-labeled.
  Kernel-mode hard-blocked for Win7: no `_NT_TARGET_VERSION`
  (windows-drivers-rs #149), no x86 (#254), zero dxgkrnl bindings anywhere,
  no Rust WDDM precedent. User-mode UMDs soft-blocked: Win7 Rust targets
  are tier-3 nightly-only, zero DDI metadata in windows-rs, no graphics-UMD
  precedent; hybrid (C KMD + Rust UMD) is a Win10+-only future path.
  Revisit conditions recorded in the guest driver language decision.
- 2026-07-22/23: **First empirical boot probe + strategy assessment**
  (absorbed; see `wiki/history/retirements.md`). Run against a user-supplied
  Windows 7 SP1 x64 ISO. The `aero-machine` native CLI was built and run
  against it: **execution dies
  on `InvalidOpcode` after ~2,500–4,000 guest instructions — inside the
  BIOS, before POST completes** (assist-layer catch-alls in
  `crates/aero-cpu-core/src/assist.rs`, e.g. unimplemented `Rsm`). This is
  the verified integration frontier: component maturity is high, but the
  boot path had never been walked. The host had KVM available but was missing
  QEMU, browsers, pinned Node and media tools (install plan in the strategy
  doc).
- 2026-07-23: **Goal set**: Win7 SP1 x64 to a verified interactive desktop
  in the browser (networking + audio + performance), working autonomously.
  **Order corrected by user: foundations (cleanup phases) before bring-up.**
  Exception instrumentation added to `aero-machine-cli` (exceptions now
  report rip/cs/linear/mode/instruction bytes/executed) — immediately
  identified the first bring-up blocker: unimplemented `ENTER 0x26,0` at
  `2000:056c`, executed=2883. Recorded in the strategy doc; fix resumes
  after foundations.
- 2026-07-23: **Cleanup phase 1 (theater + ban purge) executed and
  gate-green** — see the theater and legacy purge in
  `wiki/history/retirements.md` for the full deletion/trim list. Notable: all license files purged per user decision;
  `check-repo-layout.sh` gained an unstaged-deletion guard (purges are
  uncommitted by rule, so git ls-files still lists them); REFACTOR.md
  graveyard-deleted (recorded in `wiki/history/retirements.md`).
- 2026-07-23: **Cleanup phases 2 + 3 (legacy purge, docs absorption)
  executed and gate-green** — `docs/` is gone; the wiki is now the only
  docs home (state/, areas/, decisions/, meta/, history/, specs/, notes/).
  ~250 references swept; doc-coupled guards migrated; data files relocated
  to `protocol-vectors/`. Machine tooling landed: QEMU 10.2.1 + KVM
  (ground truth: Win7 ISO reaches the Install Windows GUI in ~75 s),
  wimlib/xorriso/mtools, Node 24.18 + pnpm 11.5, PIL for screendumps.
  Tracked follow-ups: origin-allowlist 13 cases adjudication; conformance
  selftests rewiring; `services/image-gateway` purged with its dead client
  (protocol knowledge lives in `wiki/areas/storage.md`).
- 2026-07-23: **Structural consolidation (phase: one workspace + TS
  monorepo) landed.** All six former standalone cargo tools joined the one
  cargo workspace (standalone lockfiles deleted); cargo-hakari
  `workspace-hack` added and verified. Rust full suite: **9,959 passed /
  0 failed**. TS side: pnpm 11 + Node 24.18 installed; **all JS gates
  green** — root contract 560/560, backend 374/374, web 2,140 vitest,
  net-proxy 168/168, bench/net-proxy-server/range-harness/tools-perf/
  aero-stats all green, typecheck clean. Origin-allowlist adjudication
  settled: the implementation matches all 88 `protocol-vectors/origin.json`
  cases (the docs-variant unification is moot). **Surprise recorded**:
  cargo-hakari's `workspace-hack` cannot be a dependency of wasm32-targeted
  crates — it pulls native-only deps (tokio/mio via hyper) into the wasm
  tree (mio hard-errors on wasm32). Fixed by excluding the whole 55-crate
  wasm dependency cone via `[traversal-excludes]` in `.config/hakari.toml`;
  the emulator core loses feature unification but keeps correctness. Also
  fixed along the way: disk-gateway axum `{param}` + wildcard routes and
  jsonwebtoken-10.4 explicit `rust_crypto` feature (both latent from the
  consolidation merges); a duplicate xtask test-list entry; several
  purge-dangling JS test imports; `safeErrorMessageInput` export; the
  disk-streaming-browser-e2e harness's stale `server/disk-gateway` path.
- **Documentation absorption completed.** The wiki now carries the entire
  documentation corpus — roughly ninety thousand lines that had been spread
  across the purged `docs/` tree, per-directory READMEs, the sprint-era
  workstream briefs, and the retired subtrees. The earlier absorption had
  compressed that corpus more than fivefold and lost most of its detail; this
  pass restores it.

  The structure that came out of it: an overview page, fifteen area pages that
  describe how a subsystem works and where it stands, and a `specs/` tree of
  normative contracts that the area pages defer to. Reference material that was
  scattered across many small documents was merged into one page per contract —
  the AeroGPU device ABI, the command stream, the Windows 7 user-mode driver
  interface, the virtio driver implementation, driver packaging, USB
  controllers, disk delivery. Concatenation seams, competing titles, and legacy
  numeric prefixes were removed so each page reads as one document.

  Cross-references were rewritten to the new homes and audited: every internal
  wiki link resolves. Source files that pointed at purged `docs/` paths now name
  wiki topics in words, per the house rule.

  Nothing was discarded. Originals live in a gitignored `.attic/`.
- **Broken gates repaired.** `cargo check --workspace --locked` had been failing
  outright: `tools/image-chunker` declares unpinned AWS SDK dependencies, and
  folding it into the shared workspace let them resolve to versions requiring a
  newer compiler than the pinned toolchain. The decision log had predicted this
  exact hazard and declined the bump; the shared lockfile reintroduced it
  anyway. Fixed properly rather than by pinning versions by hand: the workspace
  moved to the MSRV-aware dependency resolver and every member now declares the
  pinned `rust-version`, so the resolver selects versions the toolchain can
  build. Nine failing tests and one test target that did not compile were also
  repaired — the shapes are recorded under verification status.

---

*Maintenance: update the state, verification, structure, and debt sections in
the same change that alters reality; append to this log for history. See
[../meta/working-agreements.md](../meta/working-agreements.md) for the full doc
rules.*
<!-- absorbed: repo-layout + vm-crate-map -->

## Where new work goes

The current top-level map is [§5.1](#51-top-level-map) above, and the structural
rule behind it is
[the repo-layout decision](../decisions/0001-repo-layout.md). A few conventions
are worth stating here because they are easy to get wrong.

**Crate naming.** Packages are `aero-foo` in lowercase kebab-case, in a matching
`crates/aero-foo/` directory; Rust `use` paths normalise the hyphen to an
underscore (`aero-cpu-core` is imported as `aero_cpu_core`). A few older crates
predate the convention — `crates/memory`, `crates/devices`, `crates/platform`,
`crates/firmware` — and remain workspace members. New crates follow the
convention. See
[Rust crate naming](../decisions/0007-rust-crate-naming.md).

**Golden vectors.** Wire protocols with more than one independent
implementation keep canonical byte-level vectors in `protocol-vectors/`,
consumed by conformance tests on every side. That is what stops the
implementations drifting apart, and it is why the vectors are data in the
repository rather than prose in the wiki.

**Canonical targets for the things that have had more than one generation.**
The AeroGPU ABI is [the device ABI spec](../specs/aerogpu-device-abi.md); the
browser USB stack is `crates/aero-usb` plus `apps/web/src/usb/*`, per
[the canonical USB stack decision](../decisions/0015-canonical-usb-stack.md); the
machine and VM core is `aero-machine`, per
[the canonical machine stack decision](../decisions/0014-canonical-machine-stack.md).

> This section used to be a second, longer layout description covering `src/`,
> `apps/web/`, `server/`, `poc/` and `prototype/`. Those directories are gone — see
> [the theater and legacy purge](../history/retirements.md) — and the duplicate
> description went with them rather than being maintained twice.

## VM crate map (core wiring)

This repo historically accumulated multiple "VM" / "emulator" crates with overlapping goals. This document maps what exists today and (together with [Canonical VM core](../decisions/0008-canonical-vm-core.md)) establishes which crate is **canonical**.

### Canonical path (post-the canonical VM core decision)

The canonical VM wiring crate is:

- `crates/aero-machine` (`aero-machine`) — `aero_machine::Machine`

Everything that wants to *run the Aero machine* (browser WASM exports, host integration tests, snapshot tooling) should build on that crate.

### Browser/WASM entrypoints (what runs where)

The web runtime loads the `aero-wasm` wasm-bindgen package (built from `crates/aero-wasm`) and then
uses different exports depending on the runtime path.

| Rust crate | WASM export | Backing Rust runtime | JS entrypoint(s) | Notes |
|---|---|---|---|---|
| `crates/aero-wasm` | `Machine` | `aero_machine::Machine` (`crates/aero-machine`) | `apps/web/src/workers/machine_cpu.worker.ts` (`vmRuntime=machine`)<br>`apps/web/src/main.ts` (demo) | **Canonical full-system machine**. Owns PCI/IO/MMIO/device wiring in Rust/WASM and supports attaching `NET_TX`/`NET_RX` rings for networking. This is the canonical web runtime when `vmRuntime=machine`. |
| `crates/aero-wasm` | `WasmVm` | `aero_cpu_core` + `aero_mmu` | `apps/web/src/workers/cpu.worker.ts` | **Legacy CPU-worker runtime**. CPU executes in WASM; all port I/O + MMIO are forwarded to JS via `globalThis.__aero_io_port_*` / `globalThis.__aero_mmio_*`. |
| `crates/aero-wasm` | `WasmTieredVm` | `aero_cpu_core` tiered runtime + JS Tier-1 JIT calls | `apps/web/src/workers/cpu.worker.ts` | Same as `WasmVm`, but with Tier-0+Tier-1 tiering; Tier-1 blocks execute via `globalThis.__aero_jit_call`. |
| `crates/aero-wasm` | `PcMachine` | `aero_machine::PcMachine` (`crates/aero-machine`) | (not used by main runtime) | Experimental wrapper; allocates its own guest RAM inside the wasm module (does not use `guest_ram_layout`). |
| `crates/aero-wasm` | `DemoVm` | wrapper around `aero_machine::Machine` | `apps/web/src/workers/demo_vm_snapshot.worker.ts` | Deprecated snapshot demo API kept for UI panels. |

#### High-level crate graph

```text
crates/aero-wasm      (wasm-bindgen JS API)
  ├── crates/aero-machine  (canonical machine wiring + stable API)
  │     ├── crates/aero-cpu-core  (Tier-0 interpreter + JIT ABI state)
  │     ├── crates/memory         (physical memory bus + guest memory backends)
  │     ├── crates/platform       (port I/O bus, chipset/reset wiring)
  │     ├── crates/devices        (core device models: serial/i8042/A20/reset)
  │     ├── crates/firmware       (BIOS HLE + ACPI/SMBIOS helpers)
  │     └── crates/aero-snapshot  (snapshot file format + save/restore machinery)
  └── (deprecated) `DemoVm` export is implemented as a thin wrapper around `aero-machine`
```

### Crate responsibilities (inventory)

#### Canonical VM wiring

##### `crates/aero-machine` (`aero-machine`) — **canonical**

**What it does**
- Owns the *machine object* (`aero_machine::Machine`) and its stable public API:
  - machine config (`MachineConfig`)
  - run loop (`run_slice`, `RunExit`)
  - device attachment hooks (disk image, input injection, serial drain)
  - snapshot hooks (via `aero-snapshot`)

**Who should depend on it**
- `crates/aero-wasm` (browser/WASM exports)
- Host integration tests that need an **in-process VM** (BIOS POST, device wiring, snapshot determinism; see `crates/aero-machine/tests/*`)
- QEMU-based boot tests live under the workspace root `tests/` directory and are registered under
  `crates/aero-boot-tests` (see [`../areas/testing.md`](../areas/testing.md))

#### Supporting building blocks

##### `crates/firmware` (`firmware`)

**What it does**
- Legacy BIOS implementation in Rust (POST + INT dispatch).
- Firmware table generation (ACPI, SMBIOS, E820).

**How it fits**
- Called by `aero-machine` during `Machine::reset()` (POST) and when the CPU triggers a BIOS interrupt hypercall.
-
**Note on the retired `crates/machine` harness**

Historically, the BIOS was typed on a separate `crates/machine` abstraction (`machine::CpuState`,
`machine::MemoryAccess`, etc). That harness has been retired; BIOS now runs directly on the
canonical CPU core state (`aero_cpu_core::state::CpuState`) and the canonical guest physical memory
bus (`memory::MemoryBus`), with a small set of firmware-local traits for ROM mapping, A20 control,
and block devices.

##### `crates/memory` (`memory`)

**What it does**
- Guest physical memory backends and the physical memory bus (`PhysicalMemoryBus`).
  - Includes `MappedGuestMemory` for PC/Q35-style **non-contiguous** RAM layouts (ECAM + PCI/MMIO
    holes + >4 GiB remap) with **open-bus** semantics for holes (reads return `0xFF`, writes
    ignored).

**How it fits**
- Used by `aero-machine` as the canonical physical address space implementation.

##### `crates/platform` (`aero-platform`) and `crates/devices` (`aero-devices`)

**What it does**
- Port I/O bus + chipset wiring (`aero-platform`) and reusable device models (`aero-devices`).

##### `crates/aero-pc-platform` (`aero-pc-platform`)

**What it does**
- Higher-level PC platform composition helper (PIC/PIT/RTC/APIC/HPET + PCI bus + BAR MMIO mapping).

**How it fits**
- This is a *platform builder* rather than the canonical VM object itself.
- It is used by targeted platform/unit tests and is expected to be folded into `aero-machine` as
  the canonical machine grows to include more devices (PCI, timers, interrupts, etc.).

##### `crates/aero-boot-tests` (`aero-boot-tests`)

**What it does**

- Registers the QEMU-based boot tests under the workspace root `tests/` directory (e.g.
  `tests/boot_sector.rs`, `tests/freedos_boot.rs`, `tests/windows7_boot.rs`) as explicit `[[test]]`
  targets (see `crates/aero-boot-tests/Cargo.toml`).

**How it fits**

- This crate is intentionally **test-only**: it keeps QEMU harness dev-dependencies (Tokio, image,
  etc.) out of unrelated crates and avoids accidentally running QEMU tests via the workspace root
  `aero` package.
- Run via `cargo test -p aero-boot-tests --test ...` (see [`../areas/testing.md`](../areas/testing.md)).

Some low-level, cross-runtime primitives are factored into small shared crates rather than living
in any one device crate. For networking, the minimal `NetworkBackend` trait and the L2 tunnel
backends (`L2TunnelBackend`, `L2TunnelRingBackend`) live in `crates/aero-net-backend`.

##### `crates/aero-smp` (`aero-smp`)

**What it does**

- A **minimal deterministic SMP/APIC model** (`aero_smp::Machine`) used for unit tests and snapshot
  validation (INIT/SIPI bring-up, IPI delivery, deterministic scheduling).

**How it fits**

- This is not part of the canonical VM wiring stack; it exists to keep a small, fully deterministic
  SMP model available for testing without colliding with `aero_machine::Machine`.

#### Legacy / prototypes (excluded from workspace)

These were valuable stepping stones, but they are **not** used by production wiring anymore and are kept under `crates/legacy/` for reference.

##### `crates/legacy/vm` (`vm`) — legacy

- Historical "Minimal VM wiring for the BIOS firmware tests".
- Superseded by `crates/aero-machine`.

##### `crates/legacy/aero-emulator` (`aero-emulator`) — legacy

- Prototype emulator implementation (VBE/VGA/AeroGPU experiments).
- Superseded by the canonical `crates/aero-machine` wiring crate and the `aero-*` device crates.

#### `crates/legacy/aero-vm` (`aero-vm`) — legacy demo VM (excluded from workspace)

- A deterministic toy VM used by snapshot demo panels.
- Marked `#[deprecated]` in favor of `aero_machine::Machine`.
- Archived under `crates/legacy/` once `crates/aero-wasm::DemoVm` switched to wrapping the canonical
  `aero-machine` implementation.
- **Structural consolidation continued.** The repository moved several steps
  closer to the target architecture, each verified before the next began:
  `emulator/protocol` became `crates/aero-protocol` and the `emulator/`
  directory is gone; `ci/` became `drivers/build/`, which is what it always
  was — the drivers' build system, not continuous integration; `shared/` folded
  into `packages/`; and the two network services moved under `services/`.

  The JavaScript toolchain migration finished at the same time. pnpm is now the
  only package manager, with one lockfile and one workspace declaration; Node 24
  is pinned in `.nvmrc` and matched by every manifest's `engines`; and `xtask`
  and the `justfile` invoke pnpm rather than npm.

  Moving a package between directory depths is not free, and the fallout is
  worth knowing about because it is systematic rather than random. Relative
  paths that were correct at one depth silently resolve somewhere else at
  another: a package script reaching the repo root, source files re-exporting
  shims across packages, and — least obviously — a test that resolves fixture
  data relative to its *compiled* location rather than its source. Contract
  tests that scan the tree by path need the same treatment, as do the task
  runner's assertions about the commands it builds.

  None of that is visible to a typecheck. It surfaces only when the suites run,
  which is the argument for moving one thing at a time and running them in
  between.
- **The ban list became a gate.** `tests/repo_hygiene.contract.test.js` fails if
  a banned path reappears, if a second documentation tree or lockfile shows up,
  if `package.json` reintroduces a `workspaces` key, or if any package script
  invokes npm. It caught a missed script on its first run, which is roughly the
  strongest argument for writing it.
- **Three kinds of duplication removed.** The transport-safety helpers — the
  hardening wrappers around sockets, HTTP responses and property access that the
  security contract tests police — existed in triplicate: an ES module copy, a
  CommonJS copy with the same logic written twice, and a layer of re-export
  aliases in the network proxy. They now live once in
  `packages/transport-safety/`, with one implementation, one declaration, and
  thin surfaces for each module system.

  Two contract tests existed specifically to compare those copies for equality.
  Rather than delete the safety net, they were rewritten to assert the stronger
  property the new structure provides: that drift is *unrepresentable* because
  both surfaces point at one file, and that no copy has reappeared elsewhere in
  the tree.

  The one-line text formatters and the HTTP token validators went the same way,
  and were the better illustration of why comparison is the weaker guarantee.
  Between them they had eleven copies across the host, both services, two tools,
  the bench dashboard, the CI scripts and the browser-served assets, with four
  parity tests holding them together. Two of those copies had *already* drifted:
  `formatOneLineError`'s third argument meant a fallback string in one family and
  an error-name-fallback mode in the other, so the same call did different things
  depending on which copy the caller reached. The merged implementation accepts
  both, distinguished by type, because both spellings were in real use.

  The package is also the right way round now. It used to keep the implementation
  in `<name>.cjs` with `<name>.js` re-exporting it, which reads as reasonable and
  meant the browser could not use any of it: Vite serves a `.cjs` file inside the
  project verbatim, so the re-export linked against a module with no ES exports.
  The implementation is the ES module; the CommonJS surface is a one-line
  `require()` of it, which the pinned Node performs synchronously.

  One copy survives, and is generated rather than written. Files under
  `apps/web/public/` are served verbatim at their own URL, so a static asset there
  cannot import from `packages/` — a real constraint, not an oversight.
  `scripts/generate-public-text-one-line.mjs` derives that file from the single
  implementation and `tests/public_text_asset_contract.test.js` fails if the
  committed asset stops matching. Where a copy genuinely must exist, making it a
  build product is the next best thing to not having one.

  The second duplication was the pair of AeroGPU command executors. The
  TypeScript one in the GPU worker is the live path with its own tests; the Rust
  one was referenced only inside its own file — no consumer, no test, no export
  path through the WebAssembly bridge. It was dead code wearing the costume of
  an architectural choice, and it is retired.

  The third was three alias crates — `devices-compat`, `platform-compat` and
  `emulator-protocol-compat` — that existed so older package names kept
  resolving. Their tests turned out to be two-line forwarders into the canonical
  crates' tests, so 67 test files were being compiled and executed twice per
  workspace run for no additional coverage. Where a compat test was not a
  forwarder, the canonical crate's version was equivalent or a strict superset.
  All three are retired; one name per thing, as the crate-naming decision says.
