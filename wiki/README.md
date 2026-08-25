# Aero wiki

This wiki is the project's memory and its only documentation home. Everything
Aero knows about itself lives here: what the system is, how each subsystem
works, the contracts that hold it together, the decisions that shaped it, how
work happens, and what has been retired along the way.

If a page and the tree disagree, the tree is right and the page is a defect —
fix it in the same change. If a page and a normative header disagree, the header
is right; the specs say so explicitly where it matters.

## Start here

| If you want to… | Read |
|---|---|
| Understand what Aero is | [overview.md](./overview.md) |
| Know where the project stands, or resume the bring-up | [state/repo-state-and-structure.md](./state/repo-state-and-structure.md) |
| Know how work is done here | [meta/working-agreements.md](./meta/working-agreements.md) |

## The system

[overview.md](./overview.md) is the whole machine in one page — the
guest-to-pixel path, the worker topology, and the shape of each subsystem. The
area pages below go deep on one subsystem each; they describe how things work
and where they stand, and they defer to `specs/` for anything normative.

| Area | Covers |
|---|---|
| [areas/cpu-and-jit.md](./areas/cpu-and-jit.md) | The x86-64 core, tiered execution, CPUID policy, SMP, CPU telemetry |
| [areas/memory-and-paging.md](./areas/memory-and-paging.md) | Guest physical memory, the page-table walker and TLB, the physical map |
| [areas/platform-and-firmware.md](./areas/platform-and-firmware.md) | Interrupt controllers, timers, PCI topology, ACPI, BIOS, CD boot |
| [areas/graphics.md](./areas/graphics.md) | Boot display, AeroGPU, Direct3D translation, browser presentation |
| [areas/storage.md](./areas/storage.md) | Disk formats, controllers, persistence, overlays |
| [areas/networking.md](./areas/networking.md) | The L2 tunnel, the proxy, guest network devices |
| [areas/audio.md](./areas/audio.md) | The HDA and virtio-sound paths through to AudioWorklet |
| [areas/usb-and-input.md](./areas/usb-and-input.md) | USB controllers, HID, WebUSB and WebHID passthrough |
| [areas/windows-drivers.md](./areas/windows-drivers.md) | The guest driver stack and the tooling that installs it |
| [areas/web-host.md](./areas/web-host.md) | The TypeScript host, workers, shared memory, isolation contract |
| [areas/build-and-tooling.md](./areas/build-and-tooling.md) | Toolchains, task runner, environment, configuration surface |
| [areas/testing.md](./areas/testing.md) | Every verification layer, the fixture policy, resource limits |
| [areas/performance.md](./areas/performance.md) | Optimisation strategy, telemetry, benchmarks |
| [areas/security.md](areas/security.md) | Trust boundaries, isolation, egress policy, input validation |
| [areas/debugging.md](./areas/debugging.md) | Tracing, watchpoints, dumps, capture and replay |

## The contracts

Pages under `specs/` are normative. They are what an implementation must satisfy,
and they are the arbiter when two implementations of the same protocol disagree.

**The virtual GPU**

- [specs/aerogpu-device-abi.md](./specs/aerogpu-device-abi.md) — identity, registers, ring, allocation table, fences
- [specs/aerogpu-command-stream.md](./specs/aerogpu-command-stream.md) — the packet language
- [specs/gpu-trace-format.md](specs/gpu-trace-format.md) — capture and replay container
- [specs/windows7-aerogpu-wddm-driver.md](./specs/windows7-aerogpu-wddm-driver.md) — the driver side of the device ABI
- [specs/windows7-aerogpu-validation.md](./specs/windows7-aerogpu-validation.md) — the bring-up and stability checklist

**Direct3D translation**

- [specs/direct3d-10-11-translation.md](./specs/direct3d-10-11-translation.md) — SM4/SM5 and the D3D11 pipeline onto WebGPU
- [specs/direct3d-9ex-and-dwm.md](./specs/direct3d-9ex-and-dwm.md) — the compositor's interface and its fence contract
- [specs/compute-expansion-emulation.md](./specs/compute-expansion-emulation.md) — geometry and tessellation stages WebGPU lacks
- [specs/browser-gpu-backends.md](./specs/browser-gpu-backends.md) — backend selection, presentation, recovery
- [specs/windows7-d3d-umd-ddi.md](./specs/windows7-d3d-umd-ddi.md) — the user-mode driver surface

**Guest devices and drivers**

- [specs/windows-device-contract.md](specs/windows-device-contract.md) — how paravirtual devices bind to drivers
- [specs/windows7-virtio-driver-contract.md](specs/windows7-virtio-driver-contract.md) — the virtio binding contract
- [specs/windows7-virtio-driver-implementation.md](./specs/windows7-virtio-driver-implementation.md) — how those drivers are built
- [specs/windows7-virtqueue-split-ring.md](./specs/windows7-virtqueue-split-ring.md) — queue mechanics under KMDF
- [specs/windows7-driver-packaging-and-media.md](./specs/windows7-driver-packaging-and-media.md) — signing, packaging, install media
- [specs/usb-controllers-and-passthrough.md](./specs/usb-controllers-and-passthrough.md) — USB controllers and host passthrough

**Host, storage, and network protocols**

- [specs/worker-ipc-protocol.md](./specs/worker-ipc-protocol.md) — coordinator/worker IPC and shared memory
- [specs/snapshot-format.md](./specs/snapshot-format.md) — save state and restore
- [specs/disk-image-delivery-and-lifecycle.md](./specs/disk-image-delivery-and-lifecycle.md) — image delivery and access control
- [specs/chunked-disk-image-format.md](./specs/chunked-disk-image-format.md) — the no-range streaming container
- [specs/l2-tunnel-protocol.md](specs/l2-tunnel-protocol.md) — the Ethernet tunnel wire format
- [specs/gateway-api.md](./specs/gateway-api.md) — the networking backend's contract
- [specs/auth-tokens.md](specs/auth-tokens.md) — token formats and verification

## Decisions

Standing architectural decisions live in [decisions/](./decisions/). Each records
what was decided, why, and what it rules out. They are referenced by title;
the numbers are stable aliases, not names.

A digest of every standing decision is in
[state/repo-state-and-structure.md](./state/repo-state-and-structure.md).

## The Windows 7 bring-up

Booting Windows 7 to a verified interactive desktop is the project's central
goal, and it has its own working set:

- [history/windows-7-bring-up.md](./history/windows-7-bring-up.md) — the as-built record: every root cause and its regression test
- [areas/testing.md](./areas/testing.md#the-boot-ladder-as-a-metric) — the milestone ladder and its ground-truth reference
- [areas/debugging.md](./areas/debugging.md#the-loop-itself-is-a-debugging-tool) — the bring-up loop and its tooling
- [overview.md](./overview.md#whether-this-has-been-done-before) — what is and is not precedented about the goal

## How we work

- [meta/working-agreements.md](./meta/working-agreements.md) — workflow, documentation rules, git discipline, verification discipline
- [meta/engineering-principles.md](./meta/engineering-principles.md) — the adopted engineering constitution
- [decisions/0001-repo-layout.md](./decisions/0001-repo-layout.md) — the structure the repo converges on, and the rule behind it

## History

History is kept, not deleted. Superseded material stays readable so the
reasoning behind the current shape remains recoverable — but it is clearly
marked historical and is never normative.

- [history/retirements.md](./history/retirements.md) — the graveyard index: what was removed, why, and where its content went
- [history/windows-7-bring-up.md](./history/windows-7-bring-up.md) — the as-built record of the bring-up: every defect by class, and what was scaffolded
- [history/sprint-era-record.md](./history/sprint-era-record.md) — the eight-day sprint's plan, workstreams, and milestone table
- [history/purged-material.md](./history/purged-material.md) — documents removed during cleanup, preserved verbatim
- [history/retired-graphics-prototypes.md](./history/retired-graphics-prototypes.md) — the GPU paths tried and abandoned

## Notes

[notes/](./notes/) holds material that is interesting but not normative:
investigation records, sketches, and speculative designs. Notes are labelled as
such and never referenced as authority.

- [notes/bring-up-findings-divergence-and-sight-audits.md](./notes/bring-up-findings-divergence-and-sight-audits.md)
- [notes/stdvga-gdi-session-residual-2026-07-28.md](./notes/stdvga-gdi-session-residual-2026-07-28.md)
- [notes/logon-wall-no-enabled-account-2026-08-25.md](./notes/logon-wall-no-enabled-account-2026-08-25.md)

## House rules

These are the rules that keep the wiki worth trusting. The full versions live in
[meta/working-agreements.md](./meta/working-agreements.md).

**The wiki is the memory.** Every investigation, probe, decision, and discovery
is folded in as the work happens, not afterwards. Absorption is gardening —
extend the existing structure and keep the whole coherent — never grafting a
fragment onto the side. If it is not in the wiki, it did not happen.

**Evergreen.** Pages are written as current fact and edited in the same change
that alters reality. Dated entries belong only in designated history sections.
A page that describes last month's state as if it were today's is a bug.

**Verify or mark.** Every claim cites a path in the tree or is marked
`[unverified]`. Aspirational content says so.

**Label canon.** Components are marked canonical, active, legacy, or historical.
Legacy is never presented as current.

**Record removals.** When something is removed, superseded, or deliberately not
carried, the graveyard records what it was, why, and where its content lives on.

**Natural-language names.** No coded identifiers as primary names — no task
codes, no decision IDs, no numbered workstreams — in code, tests, filenames, or
here. Things are named in words and referenced by those words.

**Design lives in docs.** Code comments are terse why-notes at hazard sites.
Larger design discussion belongs here, and a comment may point at a wiki topic by
name — never by URL or fragile path.
