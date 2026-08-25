# USB controllers and passthrough

> The emulated USB host controllers and the human-interface stack on top of
> them, plus passthrough of real host devices via WebUSB and WebHID: controller
> models, the HID report-descriptor synthesis rules, and the wire contracts
> between the browser and the emulated bus.

This document is a **design + implementation contract** for Aero’s EHCI (Enhanced Host Controller
Interface) model: what we emulate, what we intentionally omit in the first version, and how the
controller integrates with Aero’s runtime (IRQs, timers, snapshots, and host passthrough).

It is written to be “spec-adjacent”: it calls out the EHCI concepts and register fields the guest
driver depends on, but it does **not** attempt to restate the entire EHCI specification.

> Source of truth for USB stack ownership: [Canonical USB stack](../decisions/0015-canonical-usb-stack.md) (canonical
> USB stack is `crates/aero-usb` + `crates/aero-wasm` + `apps/web/` host integration).

Related docs:

- USB HID devices/report formats: [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
- USB xHCI (USB 3.x) controller emulation: [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
- IRQ line-level semantics in the browser runtime: [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md)
- WebUSB passthrough architecture (supports UHCI and, when available, EHCI/xHCI; the async
  “pending → NAK” pattern applies to EHCI too): [`../areas/usb-and-input.md`](../areas/usb-and-input.md)

---

## Goals and scope

EHCI is the USB 2.0 host controller interface used by Windows 7’s in-box `usbehci.sys` driver. In a
PC, EHCI typically co-exists with USB 1.1 companion controllers (UHCI/OHCI) that service full-speed
and low-speed traffic for the same physical ports (see [Companion controllers](#companion-controllers-configflag--port_owner)).

**MVP goal:** enough EHCI behavior for Windows to enumerate high-speed devices and poll interrupt
endpoints reliably, with deterministic snapshot/restore.

### Implementation status (today) vs MVP target

The EHCI bring-up work in-tree is intentionally staged.

What exists today (bring-up stage):

- Capability + operational MMIO registers are implemented (including W1C status masking).
- EHCI extended capability: **USB Legacy Support** (`HCCPARAMS.EECP` → `USBLEGSUP` / `USBLEGCTLSTS`)
  is implemented for the “BIOS handoff” semaphore flow.
- Root hub ports are implemented with **deterministic timers** (reset/resume) and change-bit latching.
- `FRINDEX` advances in 1ms ticks (adds 8 microframes per tick) when the controller is running.
- IRQ line level is derived from `USBSTS`/`USBINTR` (notably `PCD` for port changes).
- A minimal **asynchronous schedule** engine (QH/qTD) is implemented for control + bulk transfers.
- A minimal **periodic schedule** engine (frame list + interrupt QH/qTD) is implemented for interrupt polling.
- Snapshot/restore is implemented for EHCI controller state and attached USB topology.
- Companion routing semantics (`CONFIGFLAG` / `PORT_OWNER`) are implemented. EHCI treats
  `PORT_OWNER=1` ports as companion-owned/unreachable.
- Shared-port routing is wired in the canonical `aero_machine::Machine`: when both
  `MachineConfig.enable_uhci` and `MachineConfig.enable_ehci` are enabled, the machine instantiates a
  shared `Usb2PortMux` and wires UHCI root ports **0–1** ↔ EHCI root ports **0–1** via
  `hub.attach_usb2_port_mux` on both controllers (see `crates/aero-machine/src/lib.rs` and
  `crates/aero-usb/src/usb2_port.rs`).
  - Current limitation: only the first two EHCI ports are muxed today; EHCI still exposes additional
    standalone root ports by default, and no TT/split transactions are implemented.
  - Browser/WASM integration convention (important for host passthrough):
    - EHCI root port **0** is used as the attachment point for the worker-managed “external hub”
      (WebHID passthrough + synthetic HID devices) in EHCI-only WASM builds.
    - EHCI root port **1** is reserved for the guest-visible WebUSB passthrough device
      (`EhciControllerBridge.set_connected`).
    - Treat this as ABI: host-side code assumes these root port indices are stable.

What is *not* implemented yet (still MVP-relevant):

- Isochronous periodic descriptors (`iTD` / `siTD`) and split/TT behavior.
- MSI/MSI-X (Aero uses PCI INTx for EHCI).
- Full platform wiring for **all shared root ports** between EHCI and UHCI companions (routing a
  single physical device between two PCI functions); today only ports 0–1 are muxed in
  `aero_machine::Machine`.

Current code locations:

- Rust EHCI controller core: `crates/aero-usb/src/ehci/{mod.rs, regs.rs, hub.rs, schedule_async.rs, schedule_periodic.rs}`
- Rust EHCI tests: `crates/aero-usb/tests/ehci*.rs`
- Shared USB2 port mux model (EHCI↔UHCI routing building block): `crates/aero-usb/src/usb2_port.rs`
  (+ `crates/aero-usb/tests/usb2_companion_routing.rs`)
- Canonical machine wiring (PC platform init; creates `Usb2PortMux` and attaches it via
  `hub.attach_usb2_port_mux` when UHCI+EHCI enabled): `crates/aero-machine/src/lib.rs`
- Browser PCI device wrapper (worker runtime): `apps/web/src/io/devices/ehci.ts` (+ `ehci.test.ts`)
- Native PCI device wrapper (MMIO BAR + IRQ + DMA gating): `crates/devices/src/usb/ehci.rs`
- Emulator crate glue (legacy/compat; feature-gated by `emulator/legacy-usb-ehci`): `emulator::io::usb::ehci` (thin wrapper around `aero_usb::ehci`; tracked for deletion in [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md))

Important: the async + periodic schedule engines now exist in `aero-usb`. The sections below document
the intended contracts and call out remaining limitations (e.g. no iTD/siTD, no split/TT).

### Planned for Aero’s EHCI MVP (target scope)

The EHCI MVP design covers:

1. **Capability and operational registers**
   - Standard EHCI capability regs (`CAPLENGTH`, `HCIVERSION`, `HCSPARAMS`, `HCCPARAMS`) and the full
     operational register block (`USBCMD`, `USBSTS`, `USBINTR`, `FRINDEX`, `CTRLDSSEGMENT`,
     `PERIODICLISTBASE`, `ASYNCLISTADDR`, `CONFIGFLAG`, `PORTSC[n]`).
   - Read/modify/write masking, W1C status behavior, and “reserved reads as 0” rules where relevant.
2. **Root hub ports + timers**
   - Root hub ports are exposed via `PORTSC[n]` with realistic connect/enable/reset/suspend state.
   - Port reset/resume behaviors use **real timers** (ms-scale) so OS drivers that wait/poll observe
     plausible transitions.
3. **Asynchronous schedule** (QH/qTD) for **control + bulk**
   - Walking the async schedule (`ASYNCLISTADDR`) with QH and qTD parsing, and performing USB
     transactions against attached device models.
4. **Periodic schedule** for **interrupt polling**
   - Walking the periodic frame list (`PERIODICLISTBASE`) and QH/qTD chains sufficient to poll
     interrupt IN endpoints (e.g. HID input).
5. **Interrupt/IRQ semantics** (Aero runtime contract)
   - EHCI uses **PCI INTx level-triggered interrupts** in Aero (no MSI initially).
   - `irq_level()` is derived from `USBSTS` + `USBINTR` gating; runtime translates this into
     `raiseIrq`/`lowerIrq` transitions (see [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md)).
6. **Snapshot/restore**
   - Deterministic save/load of EHCI register state, root hub port state/timers, and any internal
     scheduler bookkeeping required for forward progress.

### Intentionally not implemented (initially)

The initial EHCI implementation intentionally omits:

- **Isochronous transfers** (`iTD` / `siTD`)
  - Audio/video-class workloads are not targeted in the first bring-up; the schedule walker should
    treat non-QH periodic entries as “not supported / skipped” rather than crashing.
- **Split transactions / Transaction Translators (TT)**
  - EHCI can service full-/low-speed devices behind a high-speed hub using split transactions. That
    requires TT + companion routing behavior that we defer until we have a stable companion
    controller story.
- **MSI/MSI-X**
  - Aero’s EHCI uses PCI INTx only; `MSI` capability exposure and message-signaled interrupt
    routing are out of scope for MVP.

---

## PCI identity and wiring

EHCI is exposed as a **PCI function** with the standard USB/EHCI class code (`0x0c/0x03/0x20`) and
one MMIO BAR for the EHCI register space.

### PCI identity (native runtime)

Native (`aero_machine` / `crates/devices`) uses the `USB_EHCI_ICH9` profile
(`crates/devices/src/pci/profile.rs`) for a Windows-7-friendly identity:

| Field | Value |
|---|---|
| BDF | `00:12.0` |
| Vendor ID | `0x8086` (Intel) |
| Device ID | `0x293a` (ICH9 EHCI) |
| Class code | `0x0c/0x03/0x20` (Serial bus / USB / EHCI) |
| Interrupt | PCI INTx (INTA#) |
| BARs | BAR0 = MMIO (0x1000 bytes) |

Note: the IRQ *line* observed by the guest depends on platform routing (PIRQ swizzle); see
[`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md).

### PCI identity (web runtime)

The browser runtime exposes an equivalent identity via `apps/web/src/io/devices/ehci.ts`:

- BDF: `00:12.0`
- Vendor/device ID: `8086:293a`
- BAR0: `mmio32` 0x1000 bytes
- INTx: level-triggered, forwarded via `raiseIrq`/`lowerIrq` transitions

---

## Register model

EHCI exposes a **memory-mapped register block** (typically 4 KiB). The first part is the
capability registers; the operational registers start at offset `CAPLENGTH`.

The following contract is what Aero’s implementation targets; consult the EHCI spec for full bit
definitions.

Implementation note (bring-up):

- In the web runtime, the EHCI PCI function exposes a **4 KiB MMIO BAR** (`apps/web/src/io/devices/ehci.ts`).
- The current `aero-usb` EHCI model only implements the subset of registers needed for early driver
  bring-up (capability regs, operational regs, the USB Legacy Support extended capability, and
  `PORTSC[n]`). Reads from unimplemented offsets inside the “core” register window return `0`; reads
  beyond the modelled window are treated as open-bus (`0xff` bytes). This may be tightened to
  “reserved reads as 0” across the full 4 KiB window as the model matures.

### Capability registers (read-only)

The EHCI capability registers primarily allow the guest driver to discover:

- where the operational regs live (`CAPLENGTH`)
- number of root hub ports (`HCSPARAMS.N_PORTS`)
- whether 64-bit addresses are supported (`HCCPARAMS.AC64`)
- the optional “extended capabilities pointer” (`HCCPARAMS.EECP`)

**Aero contract:**

- Capability registers are **read-only**; writes are ignored.
- `CAPLENGTH` points to the start of the operational register block (commonly `0x20`).
- `HCSPARAMS.N_PORTS` matches the number of implemented `PORTSC[n]` registers.
- Aero currently models a **32-bit** EHCI controller (`HCCPARAMS.AC64=0`); schedule pointer upper
  bits (`CTRLDSSEGMENT`) are ignored and read back as 0.

#### EHCI extended capability: USB Legacy Support (BIOS handoff)

Many EHCI drivers (Windows/Linux) perform the “BIOS handoff” sequence so firmware stops owning the
controller before the OS begins DMA scheduling.

The EHCI spec places the **USB Legacy Support** capability (`USBLEGSUP` / `USBLEGCTLSTS`) in PCI
configuration space (and points to it via `HCCPARAMS.EECP`).

**Aero contract / current implementation:**

- `HCCPARAMS.EECP` is **non-zero** and points to the first extended capability.
- The extended capability registers are currently exposed **inside the EHCI MMIO window** at the
  `EECP` offset (see `crates/aero-usb/src/ehci/regs.rs` for rationale).
- `USBLEGSUP` behavior:
  - Low byte encodes `CAPID=0x01` (USB Legacy Support) and `NEXT=0`.
  - On reset, **BIOS-owned semaphore** starts set (`BIOS_SEM=1`).
  - When the guest sets **OS-owned semaphore** (`OS_SEM=1`), Aero immediately clears `BIOS_SEM`
    (models a successful handoff, without SMI/firmware timing).
- `USBLEGCTLSTS` is currently stored as a plain read/write register; SMI trap side-effects are not
  modeled.

### Operational registers (read/write)

The EHCI operational register block includes:

- `USBCMD`: run/stop, reset, schedule enables, doorbells.
- `USBSTS`: interrupt and schedule status (W1C for most status bits).
- `USBINTR`: interrupt enable mask.
- `FRINDEX`: current frame/microframe index.
- `CTRLDSSEGMENT`: upper 32 bits for 64-bit pointers (optional).
- `PERIODICLISTBASE`: base of periodic frame list.
- `ASYNCLISTADDR`: head of async QH list.
- `CONFIGFLAG`: port routing indicator (EHCI vs companions).
- `PORTSC[n]`: per-port status/control.

**Aero contract (read/write behavior):**

- Status bits that are defined as **W1C** are W1C in Aero. Writes that attempt to set read-only
  bits are masked out.
- `USBSTS.HCHALTED` is derived from `USBCMD.RunStop` and reset state; it is not directly writable.
- The “schedule status” bits (`USBSTS.PSS` / `USBSTS.ASS`) are modeled as **derived read-only** bits:
  - `USBSTS.PSS` reflects `USBCMD.RS && USBCMD.PSE` (periodic schedule enabled + controller running).
  - `USBSTS.ASS` reflects `USBCMD.RS && USBCMD.ASE` (async schedule enabled + controller running).
  - These bits are *not* treated as latched state inside `USBSTS`; reads derive them from `USBCMD`
    (stored `PSS/ASS` bits are masked out) so snapshots/tests cannot accidentally persist a stale
    schedule status.
- `USBCMD.HCRESET` resets controller-local state (registers and scheduler bookkeeping) but should
  not implicitly detach devices from the root hub; device topology is modeled separately.
- `USBCMD.IAAD` (Interrupt on Async Advance Doorbell) is implemented:
  - software can set it to request an async advance interrupt, and
  - the controller clears it deterministically at the end of a tick and sets `USBSTS.IAA` (W1C).
  - If `USBINTR.IAA` is enabled, this contributes to `irq_level()`.

---

## Root hub ports (PORTSC) and timers

EHCI exposes a “root hub” via `PORTSC[n]` registers. These are **not** a USB hub device model; they
are a set of register-backed ports with connect/reset/enable state.

### Port state modeled per port

Each port tracks:

- **Connected** + **Connect Status Change** (CSC)
- **Enabled** + **Port Enable/Disable Change** (PEDC)
- **Reset** (PR) and a **reset timer** (real-time delay before reset completes)
- **Suspend/Resume** (SUSP/FPR) and a **resume timer** (if modeled)
- **Port power** (`PP`) (Aero currently models ports as powered-on by default, but honors writes)
- **Port owner** (`PORT_OWNER`) for companion routing (modeled; `PORT_OWNER=1` makes the port
  unreachable from EHCI; if the port is backed by `Usb2PortMux` (canonical machine ports 0–1),
  ownership handoff routes the same attached device between EHCI and UHCI)
- **Speed reporting** via `PORTSC.HSP` (high-speed indicator) and `PORTSC.LS` (line status)

Implementation note:

- The current `aero-usb` EHCI model defaults to **6 root hub ports** (see
  `crates/aero-usb/src/ehci/mod.rs::DEFAULT_PORT_COUNT`), which is a common PC-style EHCI
  configuration.

### Speed reporting (`PORTSC.HSP` + `PORTSC.LS`)

Guests use a combination of the **high-speed indicator** and the **line status** bits to infer the
attached device speed and decide whether to hand off a port to a companion controller.

**Aero contract (matches `Usb2PortMux`):**

- `PORTSC.HSP` is set for **high-speed** devices **only when EHCI owns the port** (`PORT_OWNER=0`).
- `PORTSC.LS` (bits 10:11) is reported when the port is not in reset (`PR=0`):
  - Full-speed idle: J-state (`LS=0b10`, D+ high)
  - Low-speed idle: K-state (`LS=0b01`, D- high)
  - Resume signaling (full/low speed): K-state (`LS=0b01`)
  - High-speed: LS bits cleared (`LS=0b00`)

### Timing model

The guest driver typically performs sequences like:

1. detect connection (CCS/CSC)
2. set `PORTSC.PR` (reset)
3. wait for reset to complete
4. observe port enabled and begin enumeration

**Aero contract:**

- Port reset is modeled with a **~50 ms** countdown (USB-reset-scale), not “instant”.
- Timer advancement is driven by the VM/device tick (`tick_*`), not wall clock inside the device.
- When the reset timer expires:
  - `PORTSC.PR` clears.
  - The port becomes enabled (PED=1) if the device is still connected and the port is owned by EHCI.
  - Change bits are latched appropriately so the guest driver can observe the transition.

### Port change interrupts

EHCI has an interrupt cause for port changes. Aero models:

- Any event that sets a port change bit (CSC/PEDC/…) also latches `USBSTS.PCD` (Port Change Detect).
- If `USBINTR.PCD` is enabled, `USBSTS.PCD` contributes to `irq_level()`.

---

## Scheduler and time base

EHCI has two independent schedules:

- **Asynchronous schedule**: primarily control + bulk.
- **Periodic schedule**: interrupt and isochronous (we implement only the interrupt subset).

EHCI is defined in terms of **frames (1 ms)** subdivided into **microframes (125 µs)**.
`FRINDEX` carries both:

- low 3 bits: microframe (0–7)
- higher bits: frame index

**Aero contract (time stepping):**

- The controller advances `FRINDEX` in fixed increments from the emulator tick (e.g. an internal
  “step microframes” loop, or a “step 1ms” that performs 8 microframes).
- Work per tick is capped (like UHCI’s “max frames per tick” clamp) so background tab pauses do not
  cause multi-second catch-up stalls.
- Schedule processing is gated by:
  - PCI Bus Master Enable (DMA allowed) at the platform/device integration layer, and
  - `USBCMD.RunStop` + schedule enable bits (`USBCMD.ASE` / `USBCMD.PSE`) inside the controller.

---

## Asynchronous schedule (QH / qTD) — control + bulk

> Status: implemented (QH/qTD) in `crates/aero-usb/src/ehci/schedule_async.rs`.

### Structures (guest memory)

The async schedule is a linked list of **Queue Heads (QH)** starting at `ASYNCLISTADDR`. Each QH
contains an “overlay” region that behaves like the currently executing qTD.

Transfers are described by chained **queue element transfer descriptors (qTD)**.

**MVP:**

- QH + qTD parsing sufficient for:
  - control transfer stages (SETUP / DATA / STATUS)
  - bulk IN/OUT
- qTD buffer pointer handling sufficient for typical short, contiguous buffers used during
  enumeration and HID polling.

### Progress rules (how the guest observes completion)

In EHCI, “NAK” is not represented as an explicit completion code written back to guest memory; the
controller simply leaves the qTD **Active** and retries on later microframes.

**Aero contract:**

- When a transaction completes successfully:
  - qTD `Active` is cleared.
  - qTD `Total Bytes` is decremented to reflect **bytes remaining** (EHCI semantics).
  - The QH overlay is advanced to the next qTD.
- When a transaction is “pending” because it requires async host work (e.g. WebUSB/WebHID
  passthrough completion that hasn’t arrived yet), the qTD is left **Active** with no error bits set.
  This is the EHCI analogue of UHCI “return NAK while pending”.
- When a qTD has `IOC` set and completes, the controller latches `USBSTS.USBINT`.

### Error semantics (minimal but predictable)

For MVP, error modeling focuses on being deterministic and unblocking the guest:

- A device stall maps to qTD **Halted** + `USBSTS.USBERRINT` (if enabled).
- Other unexpected failures can map to a transfer error condition (and clear Active) rather than
  leaving the qTD permanently active.

---

## Periodic schedule — interrupt polling

> Status: implemented (periodic frame list + interrupt QH/qTD) in `crates/aero-usb/src/ehci/schedule_periodic.rs`.

The periodic schedule is driven by:

- `PERIODICLISTBASE`: base address of the periodic frame list.
- `FRINDEX`: selects the current frame list entry.

Real EHCI supports periodic entries for:

- iTD (isochronous)
- siTD (split isochronous / split interrupt)
- QH (interrupt)
- FSTN

**MVP:**

- Only periodic **QH** entries are executed.
- This is sufficient for interrupt IN polling used by:
  - USB HID keyboards/mice (when modeled as high-speed or behind a high-speed hub)
  - other “interrupt-style” devices that produce small periodic reports

### Microframe masks

High-speed interrupt endpoints use the QH **S-mask** to indicate which microframes the endpoint is
eligible to execute in.

**Aero contract:**

- The periodic walker honors `S-mask` at least at the coarse “eligible/not eligible” level so the
  guest driver doesn’t observe polling in every microframe.
- The walker does not implement split transaction `C-mask` behavior in MVP.

---

## Interrupt semantics

### PCI interrupt type

EHCI is exposed as a PCI device that signals interrupts using **INTx** (level-triggered).

> Aero runtime contract for INTx and shared lines: [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md)

**Aero contract:**

- The EHCI device model exposes `irq_level()` (or equivalent) which reflects *current* interrupt
  line assertion state.
- The platform integration translates transitions into `raiseIrq`/`lowerIrq` calls, and also gates
  assertion on PCI Command `Interrupt Disable`.

### What causes `irq_level()` to assert

`irq_level()` is asserted when:

1. A `USBSTS` interrupt cause bit is set **and**
2. The corresponding `USBINTR` enable bit is set.

Minimum set of causes implemented:

- `USBINT` (transaction completion; typically IOC-driven)
- `USBERRINT` (transaction error)
- `PCD` (Port Change Detect; root hub port change bits)
- `IAA` (interrupt on async advance doorbell)
  - Aero implements the `USBCMD.IAAD` → `USBSTS.IAA` doorbell/interrupt behavior.

Deassertion occurs once the guest clears the relevant `USBSTS` bits (W1C) and no other enabled
causes remain pending.

---

## Snapshot/restore requirements

EHCI state must be restorable deterministically across:

- native runs (`crates/emulator` / `aero_machine`)
- browser/WASM runs (`crates/aero-wasm` + `apps/web/`)

### What must be snapshotted (minimum)

The EHCI snapshot must include:

- Capability values that are not purely compile-time constants (if any).
- Operational registers:
  - `USBCMD`, `USBSTS`, `USBINTR`
  - `FRINDEX`
  - `CTRLDSSEGMENT` (if supported)
  - `PERIODICLISTBASE`, `ASYNCLISTADDR`
  - `CONFIGFLAG`
- USB Legacy Support extended capability registers (BIOS handoff semaphores / legacy control bits):
  - `USBLEGSUP`, `USBLEGCTLSTS`
- Root hub port state for each `PORTSC[n]`, including:
  - connect/enable/change bits
  - reset/suspend state
  - countdown timers (reset/resume)
- The full **USB device topology** behind the root hub (i.e. `AttachedUsbDevice` snapshots for
  any attached hubs/HID devices/passthrough wrappers). EHCI snapshots should follow the same
  “controller snapshot includes nested device-model snapshots” approach used by UHCI so restores
  do not depend on the host pre-attaching devices purely to satisfy snapshot loading.
- Any internal bookkeeping that is *not* represented in guest RAM (e.g. cached “previous port
  change” values used to latch global status bits).

### What must NOT be snapshotted

- The PCI INTx “wire level” itself should not be persisted; it is derived from the above registers.
- Guest schedule structures (QHs/qTDs) are in guest RAM and are captured by the VM memory snapshot
  layer, not by the device snapshot blob.

### Passthrough host-state after restore (important)

If EHCI drives passthrough devices (WebUSB/WebHID), **host in-flight work cannot be resumed** after
restore (Promises cannot be rewound). The restore path must:

- drop queued/in-flight host actions/completions, and
- leave guest-visible schedule descriptors in a state where the guest will retry and re-issue work.

In practice, the EHCI restore plumbing calls `EhciController::reset_host_state_for_restore()` after
loading the guest-visible snapshot so any attached device models can clear host-side async state
(e.g. WebUSB/WebHID requests backed by JS Promises) without altering guest-visible USB state.

This is the same rule described for UHCI passthrough in
[WebUSB passthrough, snapshot and restore](../areas/usb-and-input.md#snapshotrestore-save-state).

### Browser snapshot container note (current runtime wiring)

In the browser runtime, the I/O worker may store multiple USB controller blobs inside a single
`"AUSB"` container so newer snapshots can carry UHCI + EHCI side-by-side.

See:

- `apps/web/src/workers/usb_snapshot_container.ts` (`USB_SNAPSHOT_TAG_UHCI`, `USB_SNAPSHOT_TAG_EHCI`)
  - Container tags also include `USB_SNAPSHOT_TAG_XHCI` when xHCI is present (the container is controller-agnostic).

---

## Companion controllers (CONFIGFLAG / PORT_OWNER)

### Why companions exist

EHCI is fundamentally a **high-speed** controller. On real PCs, full-speed/low-speed traffic on
root ports is often serviced by **companion controllers** (UHCI on Intel, OHCI on some other
chipsets). EHCI participates in *routing* rather than directly handling FS/LS on root ports.

Two key pieces of guest-visible behavior:

- `CONFIGFLAG` (in EHCI operational regs)
  - When set to 1 by the OS, it indicates the OS is taking ownership and routing ports to EHCI.
- `PORTSC[n].PORT_OWNER`
  - When set, the port is owned by a companion controller; EHCI should treat the port as not under
    its control for scheduling.

### Current behavior in Aero (implemented)

In Aero’s EHCI model:

- Root hub ports start with `PORT_OWNER=1` (companion-owned) by default.
- `CONFIGFLAG` is implemented as the “claim/release all ports” knob:
  - `CONFIGFLAG` `0→1` clears `PORT_OWNER` on all ports (EHCI owns).
  - `CONFIGFLAG` `1→0` sets `PORT_OWNER` on all ports (companion owns).
- `PORT_OWNER` is only writable while the port is disabled (matches typical EHCI semantics).
- If `PORT_OWNER=1`, EHCI treats the port as **unreachable** for scheduling:
  - `CCS` still reflects physical connection.
  - Reset/enable/suspend/resume state is dropped when the port is handed off.
- Any ownership change (`CONFIGFLAG` transition or `PORT_OWNER` toggle) asserts `USBSTS.PCD` so the
  guest sees a port change interrupt when `USBINTR.PCD` is enabled.

Separate (but related): `aero_usb::usb2_port::Usb2PortMux` can model a **single physical USB 2.0
root port** shared between an EHCI controller and a UHCI companion. It is an MVP abstraction focused
on `CONFIGFLAG` / `PORT_OWNER` handoff:

- One physical port has one attached device model.
- Ownership changes move that device between EHCI and UHCI (modelled as a logical disconnect/reconnect).
- Split transactions / TT behaviour is **not** modelled.

This mux is covered by `crates/aero-usb/tests/usb2_companion_routing.rs` and is wired into the
canonical `aero_machine::Machine` when both UHCI and EHCI are enabled: ports 0–1 are backed by a
shared mux so ownership handoff moves the same attached device between controllers (see
`crates/aero-machine/src/lib.rs` and `crates/aero-usb/src/usb2_port.rs`).

### Platform wiring with UHCI companions (current + future)

Today (canonical machine):

- If only EHCI is enabled, all EHCI root ports are local to EHCI; `PORT_OWNER=1` makes a port
  companion-owned/unreachable (no companion device is actually attached).
- If both UHCI and EHCI are enabled, ports 0–1 are routed through a shared `Usb2PortMux`, so the same
  device can be handed between controllers via `CONFIGFLAG` and `PORT_OWNER`.

Longer term, the intended platform topology is:

```
PCI: EHCI function (USB 2.0, high-speed)
PCI: UHCI companions (USB 1.1, full/low-speed)
Shared physical root ports
```

**Contract for ownership/routing:**

- Each physical root port has a single “device attached” slot, but two logical controllers can
  potentially observe it depending on ownership.
- When `CONFIGFLAG=0` (or `PORT_OWNER=1`):
  - The port is routed to the companion controller.
  - EHCI reports the port as owned by companion and does not attempt to enumerate/schedule it.
- When `CONFIGFLAG=1` and `PORT_OWNER=0`:
  - The port is routed to EHCI and can enumerate high-speed devices.

### Companion mux wiring in the canonical machine (native)

In `aero_machine::Machine`, when both `MachineConfig.enable_uhci` and `MachineConfig.enable_ehci`
are enabled, the machine creates a shared `Usb2PortMux` with **2** ports and attaches it to both
controllers:

- UHCI root ports **0..1** ↔ mux ports **0..1**
- EHCI root ports **0..1** ↔ mux ports **0..1**

Code pointers:

- `crates/aero-machine/src/lib.rs` (mux creation + `hub.attach_usb2_port_mux` wiring)
- `crates/aero-usb/src/usb2_port.rs` (mux semantics: `CONFIGFLAG` + `PORT_OWNER` → effective owner)

This wiring allows a guest OS to route a shared port between controllers using the standard EHCI
hand-off mechanism:

- set `CONFIGFLAG=1` to claim ports for EHCI, and
- set `PORT_OWNER=1` on a given port to hand it to the UHCI companion when a full-/low-speed device
  is detected.

**Current limitations:**

- Only the first **two** EHCI ports are muxed. EHCI still exposes additional standalone root ports
  by default, which are EHCI-only today (no companion).
- No split transactions / TT behavior are implemented, so full-/low-speed devices *behind* a
  high-speed hub are not supported by EHCI yet.

---

## Testing strategy

EHCI touches guest memory scheduling, timing, interrupts, and snapshotting. The test plan is
layered:

### Rust unit tests (synthetic schedules, memory-bus)

Current bring-up tests cover basic register/port behavior, schedule traversal, and snapshot/restore:

- `crates/aero-usb/tests/ehci.rs` (capability regs stability, port reset timer, `HCHALTED` tracking)
- `crates/aero-usb/tests/ehci_ports.rs` (PORTSC behavior: `CONFIGFLAG`/`PORT_OWNER`, high-speed indicator, line status)
- `crates/aero-usb/tests/ehci_async.rs` (async schedule QH/qTD traversal)
- `crates/aero-usb/tests/ehci_periodic.rs` (periodic frame list + interrupt QH/qTD polling)
- `crates/aero-usb/tests/ehci_snapshot.rs` (small snapshot roundtrip smoke test)
- `crates/aero-usb/tests/ehci_snapshot_roundtrip.rs` (snapshot/restore)
- `crates/aero-usb/tests/ehci_legacy_handoff.rs` (legacy BIOS handoff)
- `crates/aero-usb/tests/usb2_companion_routing.rs` (shared-port mux routing: UHCI↔EHCI handoff via
  `CONFIGFLAG`/`PORT_OWNER`, plus an order-independent snapshot roundtrip regression test)

In `crates/aero-usb`, write unit tests that:

- build synthetic QH/qTD chains in a fake `MemoryBus`
- tick the controller through frames/microframes
- assert on:
  - qTD token updates (Active cleared, bytes updated, Halted on stall)
  - QH overlay advancement
  - `USBSTS` W1C behavior
  - interrupt causes → `irq_level()` transitions
  - port reset timing and change-bit latching

This is the EHCI analogue of the existing UHCI “schedule walker” tests.

### `aero-pc-platform` integration tests

Add platform-level integration tests that instantiate a PCI topology with:

- EHCI controller
- UHCI companions (even if companions are initially inert)
- a small set of attached synthetic USB devices

Then validate:

- Windows enumerates the EHCI function (class `0x0C0320`) and binds the in-box EHCI driver.
- Root hub ports behave sensibly (reset/enumeration works).
- Interrupt polling (periodic schedule) delivers HID input reports.

### Browser runtime harness (dev panel)

The browser runtime already has unit tests for the TypeScript PCI wrapper:

- `apps/web/src/io/devices/ehci.test.ts` (MMIO forwarding/masking, INTx level→edge transitions, tick→frame conversion)

#### Current dev harness: WebUSB EHCI passthrough harness (not a full schedule engine)

The repo includes a developer-facing “EHCI-like” WebUSB passthrough harness that is deliberately
**not** the full EHCI DMA schedule walker. It exists to validate the WebUSB action↔completion
plumbing and basic EHCI-style interrupt/status reporting (`USBSTS` bits + IRQ level) without needing
a guest OS EHCI driver.

Implementation pointers:

- WASM harness: `crates/aero-wasm/src/webusb_ehci_passthrough_harness.rs` (`WebUsbEhciPassthroughHarness`)
- Worker runtime pump + message schema: `apps/web/src/usb/webusb_ehci_harness_runtime.ts`
- UI panel (IO worker): `apps/web/src/main.ts` (`renderWebUsbEhciHarnessWorkerPanel`)

The panel can:

- attach/detach the harness “controller” and a passthrough device
- execute basic control transfers (`GET_DESCRIPTOR(Device)` / `GET_DESCRIPTOR(Config)`)
- display:
  - last forwarded `UsbHostAction` and last applied `UsbHostCompletion`
  - `USBSTS` bits (`USBINT`/`USBERRINT`/`PCD`) and the derived INTx level
  - descriptor bytes received

#### Future: full EHCI controller introspection panel

Now that EHCI schedule walking (async/periodic) exists, expand the dev harness/panel to show
controller-level state (e.g. `USBCMD/USBSTS/FRINDEX/PORTSC`) and schedule traversal (QH/qTD overlay
state, IOC behavior, periodic polling cadence).

This is invaluable for debugging timing and schedule traversal issues that are hard to see from
inside the guest OS alone.

## xHCI host controller

xHCI (“eXtensible Host Controller Interface”) is the USB host controller architecture used by most modern machines. Unlike UHCI/EHCI, xHCI is designed to support USB 3.x and also subsumes USB 2.0/1.1 device support.

This repo’s USB stack historically started with a **minimal UHCI (USB 1.1)** implementation (`crates/aero-usb`) because it is sufficient for Windows 7 in-box USB + HID drivers.

EHCI (USB 2.0) is implemented for **high-speed** device support; see
[`../areas/usb-and-input.md`](../areas/usb-and-input.md).

xHCI is being added to:

- Support **modern guests** that expect xHCI to exist (or prefer it for USB input).
- Remove full-speed-only constraints that limit **USB passthrough** compatibility (many real devices are high-speed-only or behave poorly when forced into a UHCI full-speed view).
- Provide the foundation for future **USB 3.x** support.

Status:

- xHCI support is **in progress** and is not expected to be feature-complete.
- UHCI remains the “known-good” controller for Windows 7 in-box driver binding today.
- Windows 7 does **not** include an in-box xHCI (USB 3.x) driver; xHCI is primarily targeted at
  modern guests (or Windows 7 only when an xHCI driver is installed).
- EHCI supports minimal async/periodic schedule walking (control/bulk + interrupt polling) and
  snapshot/restore; see [`../areas/usb-and-input.md`](../areas/usb-and-input.md) for current scope/limitations.
- Native builds can also expose xHCI via `crates/devices/src/usb/xhci.rs` (`XhciPciDevice`), a PCI/MMIO
  wrapper around `aero_usb::xhci::XhciController` that enforces PCI `COMMAND` gating (`MEM`/`BME`/
  `INTX_DISABLE`) and supports MSI/MSI-X delivery (when a platform `MsiTrigger` target is provided).
- The web runtime exposes an xHCI PCI function backed by `aero_wasm::XhciControllerBridge` (wrapping
  `aero_usb::xhci::XhciController`). It implements a limited subset of xHCI (MMIO registers, USB2
  root ports + PORTSC, interrupter 0 + ERST-backed event ring delivery, deterministic snapshot/restore,
  and some host-side topology/WebUSB hooks). Doorbell-driven EP0 control transfers can execute,
  doorbell 0-driven command ring processing exists for a limited subset of commands, and doorbelled
  bulk/interrupt endpoints can execute Normal TRBs when endpoint contexts are configured (via a
  bounded transfer executor). `XhciController::tick_1ms` (alias `tick_1ms_and_service_event_ring`)
  advances time/MFINDEX, executes transfer work, processes doorbell 0-kicked command ring work, and
  drains the event ring, so command-ring progress does not require further MMIO after doorbell 0 as
  long as the integration is calling the 1ms tick (and DMA/BME is enabled). However, full xHCI
  command coverage (e.g. `Reset Device`, `Force Event`), TRB types, and state-machine/scheduling
  behavior remain incomplete, so treat it as bring-up quality and incomplete.

> Canonical USB stack selection: see [Canonical USB stack](../decisions/0015-canonical-usb-stack.md) (`crates/aero-usb` + `crates/aero-wasm` + `apps/web/`).

Related docs:

- USB HID device/report details: [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
- EHCI (USB 2.0) controller bring-up + contract: [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
- WebUSB passthrough (supports UHCI and, when available, EHCI/xHCI; the async “pending → NAK” pattern applies to any controller): [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
- Canonical PCI layout + INTx routing: [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md)
- IRQ line semantics in the web runtime: [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md)

Regression tests:

- `crates/aero-wasm/tests/xhci_bme_event_ring.rs` asserts that the WASM xHCI bridge only DMAs and
  drains the guest event ring when PCI `COMMAND.BME` is enabled (and that doing so asserts INTx via
  `irq_asserted()`).
- `apps/web/src/usb/xhci_webusb_root_port_rust_drift.test.ts` asserts that the WebUSB root port reserved
  by the WASM xHCI bridge stays in sync with the shared web-runtime topology constants (and does not
  collide with the “external hub” root port used for WebHID/synthetic HID devices).
- `crates/devices/tests/xhci_msix_integration.rs` asserts MSI/MSI-X interrupt delivery semantics in
  the native PCI wrapper (suppresses INTx when MSI-X is active, tracks PBA bits for masked vectors,
  and gates MSI-X table/PBA MMIO on `COMMAND.MEM`).
- `crates/aero-machine/tests/xhci_snapshot.rs` asserts machine-level snapshot/restore semantics for
  xHCI (restores controller state, re-drives PCI INTx into the platform interrupt sink, preserves
  host-attached device handles, clears passthrough host async state, and preserves MSI-X table/PBA
  state so masked vectors re-deliver correctly after restore).
- `crates/aero-machine/tests/machine_xhci.rs` asserts machine-level PCI/MMIO integration semantics
  for xHCI (PCI identity, `COMMAND.MEM` gating for MMIO, `RUN/STOP` toggling `USBSTS.HCHALTED`, and
  MSI/MSI-X behavior).
- `crates/aero-machine/tests/machine_xhci_usb_attach_at_path.rs` asserts that host USB topology
  mutation helpers (attach/detach) work as expected for xHCI (including across machine reset).
- `crates/aero-machine/tests/machine_xhci_snapshot.rs` asserts snapshot/restore integration for xHCI
  at the machine level (including xHCI state roundtripping and sub-ms tick remainder behavior).
- `crates/aero-usb/tests/xhci_detach_pending_endpoints.rs` asserts that detaching a device clears
  any pending endpoint activations/doorbells so re-attaching at the same topology path does not
  spuriously consume TRBs without a new doorbell.
- `crates/aero-usb/tests/xhci_doorbell0.rs` asserts doorbell 0 behavior for the command ring,
  including that doorbells rung while `USBCMD.RUN=0` are deferred until the controller is started.
- `crates/aero-usb/tests/xhci_configure_endpoint_clears_pending_doorbells.rs` asserts that
  `Configure Endpoint` drop/deconfigure clears any pending endpoint doorbells so endpoints can be
  re-doorbelled after reconfiguration.
- `crates/aero-usb/tests/xhci_stop_endpoint_unschedules.rs` asserts that `Stop Endpoint` unschedules
  an active endpoint immediately (so it does not continue consuming per-tick budgets without a new
  doorbell).
- `crates/aero-usb/tests/xhci_usbcmd_run_gates_transfers.rs` asserts that transfer execution is
  gated on `USBCMD.RUN` (transfers do not progress while halted) and that already-doorbelled
  endpoints resume after `RUN` is re-enabled without requiring an additional doorbell.
- `crates/aero-usb/tests/xhci_snapshot_halted_active_endpoint_no_dcbaap.rs` asserts that a restored
  controller does not execute a queued endpoint when the guest Device Context is unavailable
  (DCBAAP=0) and the controller-local shadow Endpoint Context marks it Halted. This prevents a
  malformed snapshot image from bypassing doorbell gating via `active_endpoints`.
- `crates/aero-usb/tests/xhci_snapshot_halted_active_endpoint_shadow_state.rs` asserts that
  controller-local shadow halt/stop state is always respected even if guest memory advertises a
  running endpoint state.

---

### Goals and scope

**MVP goal:** enough xHCI behavior for modern guests to enumerate USB 2.0 devices and poll interrupt
endpoints reliably (HID), with deterministic snapshot/restore and a path toward high-speed passthrough.

The intended xHCI MVP covers:

1. **PCI function identity + MMIO BAR + INTx**
2. **USB2-only root hub ports** (connect/disconnect/reset/change) and delivery of port-change events
   via the guest-visible event ring
3. **Command ring + event ring** integration sufficient for OS driver bring-up (slot enable, address
   device, configure endpoints, and a minimal subset of endpoint commands)
4. **Transfers**
   - Endpoint 0 control transfers via Setup/Data/Status TRBs
   - Interrupt + bulk endpoints via Normal TRBs
5. **Snapshot/restore**
   - Guest RAM owns rings/contexts/buffers; device snapshot captures guest-visible regs + controller
     bookkeeping required for forward progress.

SuperSpeed, isochronous transfers, streams, and other advanced features remain out of scope for the
initial xHCI MVP.

### PCI identity and wiring

The xHCI controller is exposed as a **PCI function** with a single MMIO BAR for the xHCI register space and a single interrupt.

#### Where the code lives (at a glance)

Rust controller/model building blocks:

- xHCI core module: `crates/aero-usb/src/xhci/*`
  - Controller MMIO model: `crates/aero-usb/src/xhci/mod.rs` (`XhciController`)
  - Register offsets/constants: `crates/aero-usb/src/xhci/regs.rs`
  - Root hub port model + PORTSC bits: `crates/aero-usb/src/xhci/port.rs`
  - Snapshot encode/decode: `crates/aero-usb/src/xhci/snapshot.rs`
  - Interrupter 0 runtime regs (IMAN/ERST/ERDP): `crates/aero-usb/src/xhci/interrupter.rs`
  - Guest event ring producer (ERST-backed): `crates/aero-usb/src/xhci/event_ring.rs`
  - TRB helpers: `crates/aero-usb/src/xhci/trb.rs`
  - Ring helpers: `crates/aero-usb/src/xhci/ring.rs`
  - Command helpers: `crates/aero-usb/src/xhci/command_ring.rs`, `crates/aero-usb/src/xhci/command.rs`
  - Transfer helpers (Normal TRBs + EP0 control): `crates/aero-usb/src/xhci/transfer.rs`

Web runtime integration:

- Guest-visible PCI wrapper: `apps/web/src/io/devices/xhci.ts` (`XhciPciDevice`)
- Worker wiring: `apps/web/src/workers/io_xhci_init.ts` (`tryInitXhciDevice`)
- WASM bridge export: `crates/aero-wasm/src/xhci_controller_bridge.rs` (`XhciControllerBridge`)
- WebHID guest-topology manager (xHCI attachment path): `apps/web/src/hid/xhci_hid_topology.ts`
  (`XhciHidTopologyManager`)

Native integration (opt-in; disabled by default in both the canonical `aero_machine::Machine` and
the lower-level `aero_pc_platform::PcPlatform`):

- Canonical PCI profile (QEMU xHCI identity): `crates/devices/src/pci/profile.rs` (`USB_XHCI_QEMU`)
- Native PCI wrapper (canonical PCI glue): `crates/devices/src/usb/xhci.rs` (`XhciPciDevice`)
- Canonical machine wiring (optional): `crates/aero-machine/src/lib.rs`
  (`MachineConfig.enable_xhci`, default `false`)
- Emulator crate glue (legacy/compat; feature-gated by `emulator/legacy-usb-xhci`): `emulator::io::usb::xhci` (thin wrapper around `aero_usb::xhci`; tracked for deletion in [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md))
- PC platform wiring (optional): `crates/aero-pc-platform/src/lib.rs`
  (`PcPlatformConfig.enable_xhci`, default `false`)

Notes:

- `crates/devices/src/usb/xhci.rs` is the canonical native PCI/MMIO wrapper around
  `aero_usb::xhci::XhciController` (BAR sizing, PCI `COMMAND` gating for MMIO/DMA/INTx via
  `COMMAND.MEM`/`COMMAND.BME`/`COMMAND.INTX_DISABLE`, optional MSI/MSI-X, and snapshot/restore).
- `aero_machine::Machine` can expose xHCI behind `MachineConfig.enable_xhci` (default `false`).
  The shared controller model (`aero_usb::xhci::XhciController`) is also exercised via Rust tests,
  the web/WASM bridge (`aero_wasm::XhciControllerBridge`), and the lower-level PC platform
  (`crates/aero-pc-platform`) when `PcPlatformConfig.enable_xhci` is set.

#### PCI identity (canonical)

The repo defines a stable PCI identity for xHCI in `crates/devices`. The web runtime mirrors the key
identity fields (BDF, VID/DID, class code, BAR sizing) so guests enumerate a consistent xHCI PCI
function across environments. (Some platform-specific details like MSI capability exposure may
differ.)

| Field | Value |
|---|---|
| BDF | `00:0d.0` |
| Vendor ID | `0x1b36` (Red Hat / QEMU) |
| Device ID | `0x000d` |
| Class code | `0x0c/0x03/0x30` (Serial bus / USB / xHCI) |
| Interrupt | PCI INTx (INTA#, level-triggered) |
| BARs | BAR0 = MMIO32 (`0x10000` bytes) |

Notes:

- **Source of truth:** `crates/devices/src/pci/profile.rs` (`USB_XHCI_QEMU`).
- **Identity sync points** (keep consistent):
  - `crates/devices/src/pci/profile.rs` (`USB_XHCI_QEMU`) defines the canonical identity.
  - `crates/devices/src/usb/xhci.rs` unit tests assert `XhciPciDevice` matches the profile.
  - `apps/web/src/io/devices/xhci.ts` mirrors the identity for the web runtime.
- The canonical PCI profile reserves a 64KiB BAR0 even though the current controller model
  implements only a subset of the architectural register set.
- Interrupt delivery is **platform-dependent**:
  - Web runtime: INTx only.
  - Native integrations may choose INTx, MSI, or MSI-X. The canonical PCI profile exposes both MSI
    and a minimal single-vector MSI-X capability (table/PBA in BAR0), and the native PCI wrapper can
    deliver interrupts via MSI/MSI-X when enabled (falling back to INTx if no `MsiTrigger` target is
    configured).
- Web runtime wiring:
  - Guest-visible PCI wrapper: `apps/web/src/io/devices/xhci.ts` (`XhciPciDevice`).
  - Worker wiring: `apps/web/src/workers/io_xhci_init.ts` (`tryInitXhciDevice`). Prefers registering at
    `00:0d.0`, but falls back to auto-allocation if the slot is occupied.
  - WASM bridge export: `crates/aero-wasm/src/xhci_controller_bridge.rs` (`XhciControllerBridge`),
    which wraps the Rust controller model (`aero_usb::xhci::XhciController`) and exposes:
    - the full 64KiB MMIO window (`aero_usb::xhci::XhciController::MMIO_SIZE == 0x10000`, matching
      the TS BAR size `XHCI_MMIO_BAR_SIZE`),
    - MMIO reads/writes,
    - PCI command gating (DMA gated on Bus Master Enable via `set_pci_command()`),
    - deterministic stepping (`step_frames()` / `tick()` / `tick_1ms()`) for advancing controller time
      (frames are 1ms), and for executing DMA work when BME is enabled,
    - a non-time-advancing poll hook (`poll()`) that drains queued event TRBs into the guest event ring,
    - INTx IRQ level (`irq_asserted()` mirrors `XhciController::irq_level()`),
    - deterministic snapshot/restore (`XHCB` v1.1) of controller state + tick counter (+ optional WebUSB
      passthrough device state when connected).
    - optional host-side topology mutation APIs (`attach_hub`, `detach_at_path`, `attach_webhid_device`,
      `attach_usb_hid_passthrough_device`),
    - optional WebUSB passthrough APIs (`set_connected`, `drain_actions`, `push_completion`, `reset`,
      `pending_summary`).
- The IRQ line observed by the guest depends on platform routing (PIRQ swizzle); see [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md) and [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md).
- `aero_machine::Machine` does not expose an xHCI controller by default. USB controllers are opt-in
  via `MachineConfig.enable_uhci`/`enable_ehci`/`enable_xhci` (xHCI via `MachineConfig.enable_xhci`).
  The PC platform (`crates/aero-pc-platform`) can also expose xHCI behind the
  `PcPlatformConfig.enable_xhci` flag; treat xHCI as opt-in/experimental and the native PCI profile
  as the shared contract.
- WebHID passthrough attachment behind xHCI is managed via `XhciHidTopologyManager`
  (`apps/web/src/hid/xhci_hid_topology.ts`) and the optional topology APIs exported by
  `XhciControllerBridge` (`attach_hub`, `detach_at_path`, `attach_webhid_device`,
  `attach_usb_hid_passthrough_device`). The I/O worker routes WebHID passthrough devices to xHCI
  when these exports are present. Otherwise it falls back to the UHCI topology path (typically UHCI;
  in WASM builds that omit UHCI, the same topology manager can be backed by `EhciControllerBridge`).
- WebUSB passthrough supports both legacy UHCI (full-speed view) and high-speed controllers. When
  the WASM build exports the WebUSB passthrough hooks on xHCI/EHCI bridges (`set_connected`,
  `drain_actions`, `push_completion`, `reset`, `pending_summary`), the web runtime can opt into
  routing guest-visible WebUSB passthrough via EHCI/xHCI (default is UHCI). In EHCI/xHCI mode, the
  I/O worker disables the UHCI-only `OTHER_SPEED_CONFIGURATION` descriptor translation so the guest
  sees current-speed descriptors unmodified. Otherwise it falls back to the UHCI-based passthrough
  path. As of today, xHCI remains bring-up quality: command ring coverage is incomplete
  and the transfer plane is still limited (endpoint 0 + a subset of bulk/interrupt Normal TRB
  transfers) and under active development/validation, so UHCI remains the known-good attachment
  path for HID in practice. See
  [`../areas/usb-and-input.md`](../areas/usb-and-input.md).
- Synthetic USB HID devices (keyboard/mouse/gamepad/consumer-control) are still expected to attach
  behind UHCI when available (Windows 7 compatibility), with EHCI/xHCI used as a fallback for WASM
  builds that omit UHCI.
- The web runtime currently does **not** expose MSI/MSI-X capabilities for xHCI.

---

### Implementation status (today) vs MVP target

The current xHCI effort is intentionally staged. The long-term goal is a real xHCI host controller
for modern guests and for high-speed/superspeed passthrough, but the in-tree code today is mostly
**MVP scaffolding**: a minimal-but-realistic MMIO register model (USB2 ports + PORTSC, interrupter 0
runtime regs, ERST-backed event ring delivery) plus TRB/ring/command/transfer helpers used by tests
and harnesses. Major guest-visible pieces are still missing (full command-set/state-machine
coverage, broader TRB/transfer-plane coverage beyond the current subset, and large parts of the xHCI
spec), so treat the implementation as “bring-up” quality rather than a complete xHCI.

#### What exists today

##### Minimal controller MMIO surfaces

- Rust controller model: `aero_usb::xhci::XhciController`
  - 64KiB MMIO window (`XhciController::MMIO_SIZE == 0x10000`) with basic unaligned access handling.
  - Minimal MMIO register file with basic unaligned access handling:
    - Capability registers: CAPLENGTH/HCIVERSION, HCSPARAMS1 (port count), HCCPARAMS1 (xECP), DBOFF, RTSOFF.
    - A small xHCI extended capability list (xECP), including:
      - USB Legacy Support (BIOS owned cleared, OS owned set), and
      - Supported Protocol (USB 2.0 + speed IDs) sized to `port_count`.
    - Operational registers (subset): USBCMD, USBSTS, PAGESIZE, DNCTRL, CRCR, DCBAAP, CONFIG.
    - Runtime registers (subset): MFINDEX (advances **8 microframes per 1ms tick**, wraps to **14 bits**),
      and Interrupter 0 regs (`IMAN`, `IMOD`, `ERSTSZ`, `ERSTBA`, `ERDP`).
  - DBOFF/RTSOFF report realistic offsets. The doorbell array is **partially** implemented:
    - command ring doorbell (doorbell 0) is latched; while `USBCMD.RUN=1` it triggers
      bounded command ring processing (`No-Op`, `Enable Slot`, `Disable Slot`, `Address Device`,
      `Configure Endpoint`, `Evaluate Context`, `Stop Endpoint`, `Reset Endpoint`,
      `Set TR Dequeue Pointer`; most other commands complete with TRB Error) and queues
      `Command Completion Event` TRBs. Command ring processing is also performed on the controller’s
      1ms tick (`XhciController::tick_1ms`, alias `tick_1ms_and_service_event_ring`), so once doorbell
      0 is rung the controller can continue making progress without further MMIO as long as the
      integration is calling the tick and DMA (PCI BME) is enabled.
    - device endpoint doorbells are latched and can drive bounded transfer execution:
      - endpoint 0 control transfers (Setup/Data/Status TRBs), and
      - bulk/interrupt endpoints via Normal TRBs (via `transfer::XhciTransferExecutor`).
      Transfer execution is performed by the controller’s tick path (e.g. `tick_1ms` / `step_1ms`)
      when DMA is enabled and the controller is running (`USBCMD.RUN=1`). Endpoint doorbells may be
      rung while halted, but no DMA or transfer-ring progress occurs until RUN is set. MMIO doorbell
      writes only latch work, so wrappers must drive periodic ticks for forward progress.
    - runtime interrupter 0 registers + ERST-backed guest event ring producer are modeled (used by
      Rust tests and by the web/WASM bridge via `step_frames()`/`poll()`).
  - A small DMA read on the rising edge of `USBCMD.RUN` (primarily to validate **PCI Bus Master Enable gating** in wrappers).
    - MMIO register writes do not perform DMA, so the controller defers the read + synthetic
      interrupt until the next DMA-capable tick (`pending_dma_on_run`).
  - A level-triggered interrupt condition surfaced as `irq_level()` (interrupter 0 interrupt enable +
    pending; USBSTS.EINT is derived from pending), used to validate **INTx disable gating**.
  - DCBAAP register storage and controller-local slot allocation (Enable Slot scaffolding).
  - Partial slot / Address Device plumbing used by tests/harnesses:
    - resolves topology via Slot Context `RootHubPortNumber` + `RouteString`.
      - Route String encodes up to 5 downstream hub tiers as 4-bit port numbers (1..=15) terminated
        by 0; the least-significant nibble is closest to the device (so hex digits read root→device).
    - supports a limited Address Device command handler (Input Context parsing + EP0 `SET_ADDRESS` +
      Slot/EP0 context mirroring).
  - USB2-only root hub/port model: PORTSC operational registers + reset timer + Port Status Change
    Event TRBs (queued host-side and delivered via interrupter 0 event ring when configured).
- Web/WASM: `aero_wasm::XhciControllerBridge`
  - Wraps `XhciController` (shared Rust model) and forwards MMIO reads/writes from the TS PCI device.
  - Enforces **PCI BME DMA gating**: MMIO reads/writes never DMA, and stepping/polling only uses a
    DMA-capable `GuestMemoryBus` when PCI `COMMAND.BME` is enabled (otherwise it uses
    `tick_1ms_no_dma` and `poll()` becomes a no-op).
  - `step_frames()` advances controller time; when BME is enabled it also executes pending transfer
    ring work, processes doorbell 0-kicked command ring work, and drains queued events
    (`XhciController::tick_1ms`, alias `tick_1ms_and_service_event_ring`). When BME is disabled,
    `step_frames()` still advances port/reset timers so operations like PORTSC reset completion can
    make forward progress.
  - `poll()` drains any queued event TRBs into the guest event ring (`XhciController::service_event_ring`);
    DMA is gated on BME.
  - WebUSB passthrough device APIs (`set_connected`, `drain_actions`, `push_completion`, `reset`,
    `pending_summary`) used by the web I/O worker to attach/detach a passthrough device behind a
    reserved xHCI root port (typically root port index `1`; falls back to `0` if the controller only
    exposes a single root port).
  - `irq_asserted()` reflects `XhciController::irq_level()` (interrupter 0 interrupt enable + pending).
  - Optional host-side topology mutation APIs for passthrough HID/hubs (`attach_hub`,
    `detach_at_path`, `attach_webhid_device`, `attach_usb_hid_passthrough_device`).
  - Deterministic snapshot/restore of controller state + tick counter (and WebUSB device state when
    connected).

These are **not** full xHCI implementations. In particular, command ring coverage is still incomplete
(bounded to a small subset of commands), and transfer execution is limited to endpoint 0 plus a
subset of bulk/interrupt endpoints via Normal TRBs (no isochronous, streams, USB3/SuperSpeed, etc).

##### TRB + ring building blocks

`crates/aero-usb/src/xhci/` also provides:

- TRB encoding helpers (`trb`)
- TRB ring cursor/polling helpers (`ring`)
- Context parsing helpers (`context`)

These are used by tests and by higher-level “transfer engine” harnesses.

##### Command ring + endpoint-management helpers (used by tests)

`crates/aero-usb/src/xhci/` includes a few early building blocks that model **parts** of xHCI
command/event behavior:

- `XhciController::{set_command_ring,process_command_ring}`: command ring processing used by unit
  tests and by the guest-visible MMIO path (CRCR + doorbell 0). It consumes a guest command ring
  (via `RingCursor`) and queues `Command Completion Event` TRBs for a small subset of commands used
  during bring-up: `No-Op`, `Enable Slot`, `Disable Slot`, `Address Device`, `Configure Endpoint`,
  `Evaluate Context`, and endpoint commands `Stop Endpoint`, `Reset Endpoint`, `Set TR Dequeue Pointer`.
  The doorbell 0 “kick” persists (`cmd_kick`), so periodic `tick_1ms` calls can continue processing
  until the ring appears empty.
  These events are delivered to the guest only once the event ring is
  configured and `service_event_ring` is called (e.g. via the WASM bridge `step_frames()`/`poll()`
  hook).
- `command_ring::CommandRingProcessor`: parses a guest command ring and writes completion events into
  a guest event ring (single-segment).
  - Implemented commands (subset): `Enable Slot`, `Disable Slot`, `No-Op`, `Address Device`,
    `Configure Endpoint`, `Evaluate Context`, `Stop Endpoint`, `Reset Endpoint`, `Set TR Dequeue Pointer`.
- `command`: a minimal endpoint-management state machine used by tests and by early enumeration
  harnesses.

These helpers are partially wired into the guest-visible MMIO/doorbell model (CRCR + doorbell 0 for
the command subset, plus slot doorbells for endpoint 0 and bulk/interrupt Normal TRBs), but many
commands/endpoints remain bring-up-only. The more complete `command_ring::CommandRingProcessor`
remains a test/harness helper and is not used by the MMIO doorbell path today.
Bulk/interrupt Normal TRB execution is performed via `transfer::XhciTransferExecutor`, but coverage
is still limited to a subset of endpoint/TRB types.

##### Transfers (non-control endpoints via Normal TRBs)

`aero_usb::xhci::transfer::XhciTransferExecutor` can execute **Normal TRBs** for non-control endpoints:

- Interrupt IN/OUT (HID input/output reports)
- Bulk IN/OUT (primarily for passthrough/WebUSB-style flows)

Key semantics:

- `UsbInResult::Nak` / `UsbOutResult::Nak` leaves a TD pending so it can be retried on a later tick.
- Short packets generate a `ShortPacket` completion code and report *residual bytes* (xHCI semantics).
- `Stall` halts the endpoint and produces a `StallError` completion.

##### Transfers (endpoint 0 control via Setup/Data/Status TRBs)

`aero_usb::xhci::transfer::Ep0TransferEngine` can process **endpoint 0** control transfers from a
guest transfer ring:

- Setup Stage / Data Stage / Status Stage TRBs.
- IN + OUT directions.
- Data Stage supports buffer pointers (IDT=0) and immediate data (IDT=1, <=8 bytes).
- `NAK` leaves the TD pending and retries on the next `tick_1ms` (no busy loops).
- Emits Transfer Event TRBs into a simple contiguous event ring (used by unit tests).

This engine is currently a standalone transfer-plane component used by tests; `XhciController` has
its own minimal doorbell-driven endpoint-0 executor (driven by slot doorbells +
`XhciController::tick()`), so `Ep0TransferEngine` is not wired into the guest-visible MMIO model.
Note: the web/WASM bridge’s `step_frames()` path runs the DMA-capable `XhciController::tick_1ms`
(aka `tick_1ms_and_service_event_ring`) when PCI BME is enabled (and `tick_1ms_no_dma` when BME is
disabled), so doorbelled endpoint transfers (endpoint 0 + bulk/interrupt Normal TRBs) can make
forward progress, doorbell 0-kicked command ring work can continue across ticks, and queued events
are drained into the guest-configured event ring.

#### Device model layer

xHCI shares the same USB device model abstractions as UHCI (`crate::UsbDeviceModel` / `device::AttachedUsbDevice`), so device work (HID descriptors, report formats, passthrough normalization) does not need to be duplicated per controller type.

##### Test-only: xHCI-style command + control transfer harness

`crates/aero-usb/tests/xhci_webusb_passthrough.rs` contains a small **xHCI-style** harness that
consumes TRBs from guest memory (via `RingCursor`) and drives the existing `AttachedUsbDevice`
control pipe:

- Command ring bring-up: `Enable Slot` → `Address Device` → `Configure Endpoint`.
- EP0 control-IN transfer built from `Setup Stage` / `Data Stage` / `Status Stage` TRBs (e.g.
  `GET_DESCRIPTOR`).
- Bulk IN/OUT via Normal TRBs for passthrough-style flows.

This harness is a reference/validation tool; it is **not** yet integrated into the guest-visible
controller MMIO/doorbell model.

Dedicated EP0 unit tests also exist:

- `crates/aero-usb/tests/xhci_control_get_descriptor.rs`
- `crates/aero-usb/tests/xhci_control_set_configuration.rs`
- `crates/aero-usb/tests/xhci_control_in_nak_retry.rs`
- `crates/aero-usb/tests/xhci_control_immediate_data.rs`
- `crates/aero-usb/tests/xhci_controller_immediate_data.rs`

#### Still MVP-relevant but not implemented yet

- Full root hub model (USB3 ports, additional link states, full port register/event coverage).
- Full command ring coverage via doorbell 0 (doorbell 0 is modeled, but only a subset of commands is
  implemented today) and the corresponding slot/endpoint context state machines (`Configure Endpoint`,
  `Evaluate Context`, endpoint commands, etc).
- More complete transfer-plane coverage (additional TRB types beyond Normal, more complete endpoint
  state machines, isochronous transfers, etc).
- Better transfer scheduling/performance for bulk/interrupt endpoints (today execution is
  intentionally bounded to keep guest-induced work finite).
- More complete event-ring servicing / “main loop” integration in wrappers: regularly call
  `tick_1ms` (alias `tick_1ms_and_service_event_ring`) or equivalent so port timers, transfer
  execution, command ring work, and event delivery make forward progress, with DMA gated on PCI BME.
- Making xHCI enabled-by-default in the canonical machine/topology (native + web). Today it remains
  opt-in/experimental (`MachineConfig.enable_xhci` requires `MachineConfig.enable_pc_platform`,
  and `PcPlatformConfig.enable_xhci`).

---

### Unsupported features / known gaps

xHCI is a large spec. The MVP intentionally leaves out many features that guests and/or real hardware may use:

- **Root hub / port model** beyond the current USB2-only PORTSC subset + reset timer scaffolding (no USB3 ports/link states yet).
- **Doorbell-driven command ring + transfer execution**: the doorbell array is partially implemented
  (doorbell 0 triggers a bounded subset of command ring processing; endpoint doorbells can drive
  endpoint 0 and bulk/interrupt Normal TRBs), but the command set and many endpoint state machines /
  advanced xHCI semantics remain incomplete.
- **Full xHCI slot/endpoint context state machines** (`Configure Endpoint`, `Evaluate Context`,
  endpoint commands, etc). A subset of command processing is exposed today via doorbell 0, but full
  coverage/state machines remain incomplete.
- **Transfer-plane coverage**: bulk/interrupt Normal TRBs are supported via
  `transfer::XhciTransferExecutor`, but remain incomplete (bounded work per tick, limited TRB
  coverage, and incomplete endpoint state-machine coverage). Many TRB types and endpoint behaviors
  remain unimplemented (isochronous, streams, etc).
- **USB 3.x SuperSpeed** (5/10/20Gbps link speeds) and related link state machinery.
- **Isochronous transfers** (audio/video devices).
- **MSI/MSI-X in the web runtime**: the TS PCI wrapper currently uses INTx only (no MSI/MSI-X
  capabilities exposed to the guest). Native wrappers can deliver MSI/MSI-X (single-vector MSI-X)
  when configured.
- **Bandwidth scheduling** / periodic scheduling details beyond “enough to exercise basic interrupt polling”.
- **Streams** (bulk streams), TRB chaining corner cases, and advanced endpoint state transitions.
- **Multiple interrupters**, interrupt moderation, and more complex event-ring configurations.
- **Power management** features (D3hot/D3cold, runtime PM, USB link power management) beyond the minimal bits required for driver bring-up.

If you are debugging a device/guest issue and you see the guest attempting to use one of the above features, it is likely hitting an unimplemented xHCI path.

---

### Snapshot / restore behavior

Snapshotting follows the repo’s general device snapshot conventions (see [`snapshot-format.md`](snapshot-format.md)):

- **Guest RAM** holds most of the xHCI “data plane” structures (rings, contexts, transfer buffers). These are captured by the VM memory snapshot, not duplicated inside the xHCI device snapshot.
- The xHCI device snapshot captures **guest-visible register state** and any controller bookkeeping that is not stored in guest RAM.
  - Today, `aero_usb::xhci::XhciController` snapshots (device ID `XHCI`, version `0.9`) capture:
    - operational/runtime state (`USBCMD`, `USBSTS`, `CONFIG`, `MFINDEX`, `CRCR`, `DCBAAP`, port count,
      `DNCTRL`, controller time bookkeeping (`time_ms`, `last_tick_dma_dword`), deferred DMA-on-RUN
      probe flag (`pending_dma_on_run`), Interrupter 0 regs: `IMAN`, `IMOD`, `ERSTSZ`, `ERSTBA`,
      `ERDP` + internal generation counters),
    - per-port snapshot records (connection/change bits/reset timers/link state/speed + nested
      `AttachedUsbDevice` snapshot, when present),
    - controller-local slot/endpoint state (enabled slots, Slot/Endpoint context mirrors + transfer
      ring cursors, active endpoints, EP0 control TD state), and
    - controller-local forward-progress state (command ring cursor/kick flag, pending event TRBs,
      dropped-event counter, and event ring producer state).
  - Current limitations: the snapshot only covers the subset of xHCI behavior implemented by this
    model; guest RAM contents for rings/contexts/buffers are still owned by the VM memory snapshot,
    and host-side async work (WebUSB/WebHID) is reset across restore.
- The web/WASM bridge (`aero_wasm::XhciControllerBridge`) snapshots as `XHCB` (version `1.1`) and currently stores:
  - the underlying `aero_usb::xhci::XhciController` snapshot bytes,
  - a tick counter (used for deterministic stepping in future scheduling work), and
  - (when connected) the `UsbWebUsbPassthroughDevice` snapshot bytes.
- The native PCI wrapper (`crates/devices/src/usb/xhci.rs`) snapshots as `XHCP` (version `1.2`) and stores:
  - PCI config space (including MSI/MSI-X capability state and BAR bookkeeping),
  - internal IRQ bookkeeping,
  - MSI-X table + PBA state (when present), and
  - the underlying `aero_usb::xhci::XhciController` snapshot bytes.
- **Host resources are not snapshotted.** Any host-side asynchronous USB work (e.g. in-flight WebUSB/WebHID requests) must be treated as **reset** across restore; the host integration is responsible for resuming forwarding after restore.
  - After loading an xHCI snapshot, integrations should clear any host-side async bookkeeping that
    cannot be resumed (e.g. WebUSB host actions backed by JS Promises) by calling
    `XhciController::reset_host_state_for_restore`. The native `XhciPciDevice` wrapper and WASM
    `XhciControllerBridge` call this during restore.
  - For WebUSB passthrough specifically, `UsbWebUsbPassthroughDevice::reset_host_state_for_restore`
    drops queued actions/completions and clears in-flight tracking so guest TD retries can re-emit
    host actions (monotonic action IDs are preserved by the snapshot).

Practical implication: restores are deterministic for pure-emulated devices, but passthrough devices may need re-authorization/re-attachment and may observe a transient disconnect.

---

### Testing

Rust-side USB/controller/device-model tests:

```bash
cargo test -p aero-usb --locked
cargo test -p aero-devices --locked
cargo test -p aero-machine --locked
```

Web runtime unit tests (includes USB broker/runtime helpers, rings, and device wrappers):

```bash
pnpm -C apps/web run test:unit
```

USB-related unit tests commonly live under:

- `apps/web/src/usb/*.test.ts`
- `apps/web/src/io/devices/xhci.ts` + `apps/web/src/io/devices/xhci.test.ts` (xHCI PCI wrapper + INTx semantics)
- `apps/web/src/workers/io_xhci_init.test.ts` (xHCI WASM bridge init + device registration)
- `apps/web/src/hid/xhci_hid_topology.test.ts` (xHCI guest USB topology manager)

Rust xHCI-focused tests commonly live under:

- `crates/aero-usb/tests/xhci_controller_mmio.rs`
- `crates/aero-usb/tests/xhci_mmio_smoke.rs`
- `crates/aero-usb/tests/xhci_mmio_doorbell_command_ring.rs`
- `crates/aero-usb/tests/xhci_doorbell_bme_gating.rs` (PCI BME gating + event ring delivery)
- `crates/aero-usb/tests/xhci_hce_*.rs` (Host Controller Error paths: halt DMA, snapshot/restore)
- `crates/aero-usb/tests/xhci_event_ring.rs`
- `crates/aero-usb/tests/xhci_trb_ring.rs`
- `crates/aero-usb/tests/xhci_command_ring.rs`
- `crates/aero-usb/tests/xhci_command_ring_tick.rs`
- `crates/aero-usb/tests/xhci_context_parse.rs`
- `crates/aero-usb/tests/xhci_extcaps.rs`
- `crates/aero-usb/tests/xhci_supported_protocol.rs`
- `crates/aero-usb/tests/xhci_ports.rs`
- `crates/aero-usb/tests/xhci_detach_pending_endpoints.rs` (detach clears pending doorbell work)
- `crates/aero-usb/tests/xhci_configure_endpoint_clears_pending_doorbells.rs` (Configure Endpoint clears pending doorbells)
- `crates/aero-usb/tests/xhci_configure_endpoint_slot_context.rs` (Configure Endpoint slot context parsing)
- `crates/aero-usb/tests/xhci_stop_endpoint_unschedules.rs` (Stop Endpoint unschedules active endpoints)
- `crates/aero-usb/tests/xhci_snapshot_*.rs` (snapshot/restore, legacy compatibility, determinism)
- `crates/aero-usb/tests/xhci_interrupt_in.rs`
- `crates/aero-usb/tests/xhci_control_*.rs` (EP0 control transfer behavior)
- `crates/aero-usb/tests/xhci_webusb_passthrough.rs`
- `crates/aero-machine/tests/machine_xhci.rs` (machine-level PCI/MMIO integration)
- `crates/aero-machine/tests/machine_xhci_usb_attach_at_path.rs` (machine-level host attach/detach integration)
- `crates/aero-machine/tests/machine_xhci_snapshot.rs` (machine-level snapshot/restore integration)
- `crates/aero-machine/tests/xhci_snapshot.rs` (machine-level snapshot/restore integration)
- `crates/devices/tests/xhci_msix_integration.rs` (native PCI wrapper MSI-X + controller integration)
- `crates/emulator/tests/xhci_mmio_gating.rs` (emulator-side PCI/MMIO/BME gating; requires `--features legacy-usb-xhci`)

When adding or extending xHCI functionality, prefer adding focused Rust tests (for controller semantics) and/or web unit tests (for host integration and PCI wrapper behavior) alongside the implementation.

## USB HID: Browser input → HID usages and reports

This project models input devices as **USB HID** (Human Interface Device) peripherals so that, once a USB controller (UHCI/EHCI/xHCI) exists, Windows can use native HID drivers rather than legacy PS/2 emulation.

This document is **separate** from PS/2 scancodes (see `../areas/usb-and-input.md`), because USB HID uses **usages** (IDs from standardized tables), not scancodes.

For xHCI (USB 3.x) host controller details and current limitations, see [`../areas/usb-and-input.md`](../areas/usb-and-input.md).

> Source of truth: [Canonical USB stack](../decisions/0015-canonical-usb-stack.md) defines the canonical USB
> stack for the browser runtime (`crates/aero-usb` + `apps/web/` host integration). This document focuses
> on HID usages and report formats on top of that stack.

For controller-level design/contract notes:

- EHCI (USB 2.0): [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
- xHCI (USB 3.x): [`../areas/usb-and-input.md`](../areas/usb-and-input.md)

The Rust-side usage mapping helpers live in `aero_usb::hid::usage` (`crates/aero-usb/src/hid/usage.rs`).
Browser-oriented convenience wrappers live in `aero_usb::web` (`crates/aero-usb/src/web.rs`) (e.g.
`keyboard_code_to_hid_usage`, `mouse_button_to_hid_mask`).
The emulator re-exports these helpers at `emulator::io::usb::hid::usage`.
The browser-side `KeyboardEvent.code -> HID usage` mapping lives in `apps/web/src/input/hid_usage.ts`.

For USB HID **gamepad** details (Windows 7 driver binding expectations, and the exact gamepad report descriptor/report bytes), see
[`../areas/usb-and-input.md`](../areas/usb-and-input.md). The Rust↔TypeScript report packing contract is pinned by
`protocol-vectors/hid_gamepad_report_vectors.json` (plus the clamping-focused
`protocol-vectors/hid_gamepad_report_clamping_vectors.json`) and validated by tests on both sides.

For WebHID passthrough (synthesizing HID report descriptors from WebHID metadata
because browsers do not expose raw report descriptor bytes), see
[`../areas/usb-and-input.md`](../areas/usb-and-input.md).

For the end-to-end “real device” passthrough architecture and security model
(main thread owns the handle; worker models a guest-visible USB controller + a generic HID device), see
[`../areas/usb-and-input.md`](../areas/usb-and-input.md).

---

### Keyboard: `KeyboardEvent.code` → HID Usage (Keyboard/Keypad page 0x07)

#### Why `code` (not `key`)

- Use `KeyboardEvent.code` because it represents the **physical key position** ("KeyA", "Digit1", …) and is stable across keyboard layouts.
- `KeyboardEvent.key` is the produced character, which depends on layout and modifiers, and is not what USB HID reports.

#### Modifier keys

USB HID keyboard modifiers are special usages `0xE0..=0xE7` which map to a bitfield in the keyboard report:

| `KeyboardEvent.code` | Usage | Modifier bit |
| --- | --- | --- |
| `ControlLeft` | `0xE0` | `1<<0` |
| `ShiftLeft` | `0xE1` | `1<<1` |
| `AltLeft` | `0xE2` | `1<<2` |
| `MetaLeft` | `0xE3` | `1<<3` |
| `ControlRight` | `0xE4` | `1<<4` |
| `ShiftRight` | `0xE5` | `1<<5` |
| `AltRight` | `0xE6` | `1<<6` |
| `MetaRight` | `0xE7` | `1<<7` |

#### Common key usages (subset)

USB HID usages for letters and digits are fixed (independent of layout):

| `KeyboardEvent.code` | Usage |
| --- | --- |
| `KeyA`..`KeyZ` | `0x04`..`0x1D` |
| `Digit1`..`Digit0` | `0x1E`..`0x27` |
| `Enter` | `0x28` |
| `Escape` | `0x29` |
| `Backspace` | `0x2A` |
| `Tab` | `0x2B` |
| `Space` | `0x2C` |
| `Minus` | `0x2D` |
| `Equal` | `0x2E` |
| `BracketLeft` | `0x2F` |
| `BracketRight` | `0x30` |
| `Backslash` | `0x31` |
| `IntlHash` | `0x32` |
| `Semicolon` | `0x33` |
| `Quote` | `0x34` |
| `Backquote` | `0x35` |
| `Comma` | `0x36` |
| `Period` | `0x37` |
| `Slash` | `0x38` |
| `CapsLock` | `0x39` |
| `F1`..`F12` | `0x3A`..`0x45` |
| `PrintScreen` | `0x46` |
| `ScrollLock` | `0x47` |
| `Pause` | `0x48` |
| `Insert` | `0x49` |
| `Home` | `0x4A` |
| `PageUp` | `0x4B` |
| `Delete` | `0x4C` |
| `End` | `0x4D` |
| `PageDown` | `0x4E` |
| `ArrowRight` | `0x4F` |
| `ArrowLeft` | `0x50` |
| `ArrowDown` | `0x51` |
| `ArrowUp` | `0x52` |
| `IntlBackslash` | `0x64` |
| `NumpadEqual` | `0x67` |
| `NumpadComma` | `0x85` |
| `IntlRo` | `0x87` |
| `IntlYen` | `0x89` |

#### Keeping the mapping consistent (Rust ↔ TypeScript)

There are two independent implementations of `KeyboardEvent.code → HID usage`:

- Rust: `crates/aero-usb/src/hid/usage.rs::keyboard_code_to_usage`
- TypeScript: `apps/web/src/input/hid_usage.ts::keyboardCodeToHidUsage`

To prevent drift between them, we keep a shared fixture of supported mappings (including full
alphanumeric ranges like `KeyA..KeyZ`, `Digit0..Digit9`, and `F1..F12`) at:

- `protocol-vectors/hid_usage_keyboard.json`

Both sides have unit tests that validate their mapping function against that fixture:

- Rust: `crates/aero-usb/tests/hid_usage_keyboard_fixture.rs`
- TypeScript: `apps/web/src/input/hid_usage.test.ts`

When adding support for a new key code:

1. Add it to `protocol-vectors/hid_usage_keyboard.json` (as `code` + expected usage).
2. Update **both** mapping functions.
3. Run `cargo xtask input` (recommended) or run the equivalent commands manually:
   - `cargo xtask input` runs the HID usage fixture tests as part of its focused `aero-usb` subset.
     Use `cargo xtask input --usb-all` if you want the full USB integration suite.
   - Manual Rust equivalent (focused):
     - `cargo test -p aero-usb --locked --test hid_usage_keyboard_fixture --test hid_usage_consumer_fixture`
     - (or, to run everything: `cargo test -p aero-usb --locked`)
   - `pnpm -C apps/web run test:unit -- src/input`

   If you don't have Node deps available (e.g. a constrained sandbox), you can still validate the
   Rust side with `cargo xtask input --rust-only` (but it won't run the TypeScript fixture tests).

(The mapping is still not intended to be exhaustive, but the fixture is intentionally thorough so a
change on either side requires updating the shared list.)

#### Report model (boot keyboard)

The modeled keyboard uses the standard 8-byte boot keyboard input report:

```
Byte 0: modifier bits (Ctrl/Shift/Alt/GUI)
Byte 1: reserved (0)
Byte 2..7: up to 6 concurrently pressed non-modifier key usages
```

Notes:

- The implementation keeps a stable ordering of pressed keys (in press order) to avoid spurious “key up/down” events in simplistic host stacks.
- If more than 6 non-modifier keys are held, the report uses the HID `ErrorRollOver` code (`0x01`) in all 6 slots.

---

### Consumer control (media keys): `KeyboardEvent.code` → HID Usage (Consumer page 0x0C)

Some “media keys” are not part of the Keyboard/Keypad usage page (`0x07`). They live on the HID
**Consumer** usage page (`0x0C`) and are exposed by browsers as `KeyboardEvent.code` values like:

- `AudioVolumeUp`
- `AudioVolumeDown`
- `AudioVolumeMute`
- `MediaPlayPause`
- `MediaStop`
- `MediaTrackNext`
- `MediaTrackPrevious`

Aero models these inputs using a dedicated USB HID **consumer-control** device model:

- Rust: `crates/aero-usb/src/hid/consumer_control.rs` (`UsbHidConsumerControl`)
  - Interrupt IN report format: **2 bytes**, little-endian `u16` usage ID (`0` = none pressed)

Mapping helpers (keep in sync):

- Rust: `crates/aero-usb/src/hid/usage.rs::keyboard_code_to_consumer_usage`
- Rust (browser convenience wrapper): `crates/aero-usb/src/web.rs::keyboard_code_to_consumer_usage`
- TypeScript: `apps/web/src/input/hid_usage.ts::keyboardCodeToConsumerUsage`

To prevent drift, the supported mapping set is pinned by:

- `protocol-vectors/hid_usage_consumer.json`

…and validated by tests on both sides:

- Rust: `crates/aero-usb/tests/hid_usage_consumer_fixture.rs`
- TypeScript: `apps/web/src/input/hid_usage.test.ts`

---

### Mouse: browser mouse events → HID usages and reports

#### Buttons (`MouseEvent.buttons`)

In browsers, `MouseEvent.buttons` is a bitfield:

- `1` = left
- `2` = right
- `4` = middle
- `8` = back
- `16` = forward

The modeled mouse report supports 5 buttons (left/right/middle/back/forward) and encodes them as:

| HID button | Bit | Typical meaning |
| --- | --- | --- |
| Button 1 | `1<<0` | left |
| Button 2 | `1<<1` | right |
| Button 3 | `1<<2` | middle |
| Button 4 | `1<<3` | back / side |
| Button 5 | `1<<4` | forward / extra |

Note: in **HID boot protocol**, the standard boot mouse format only defines 3 buttons; Aero masks
buttons 4/5 to zero when emitting boot-protocol reports. Because these buttons are not guest-visible
in boot mode, Aero also avoids enqueueing “no-op” interrupt reports when only buttons 4/5 change.

#### Movement (`PointerLock` + `MouseEvent.movementX/Y`)

Use Pointer Lock so that `movementX` / `movementY` are **relative deltas** rather than absolute coordinates.

- HID X/Y are signed 8-bit relative values (`-127..=127`).
- Large deltas should be split across multiple reports (the device model does this internally).
- Sign convention:
  - `movementX > 0` → HID X positive (move right)
  - `movementY > 0` → HID Y positive (move down)

#### Wheel (`WheelEvent.deltaY`)

HID wheel (`Usage 0x38`) is also a signed 8-bit relative value.

Browser wheel events typically use:

- `deltaY > 0` for scrolling down (wheel “towards the user”)
- `deltaY < 0` for scrolling up

For a conventional HID mouse, scroll **up** is usually represented as a positive wheel step in guest OS input APIs.
When wiring browser events to the device model, invert as needed:

```
hid_wheel_step = -WheelEvent.deltaY.signum()
```

#### Horizontal wheel (`WheelEvent.deltaX`)

Many mice and trackpads can also generate **horizontal scroll** events, which are exposed by browsers
as `WheelEvent.deltaX`.

In Aero's synthetic USB mouse, horizontal scroll is modeled using the HID **AC Pan** usage (Consumer
page `0x0C`, usage `0x0238`), encoded as a signed 8-bit relative value in the **report protocol**
input report.

- `deltaX > 0` (scroll right) should typically map to a **positive** AC Pan value.
- Unlike `deltaY`, horizontal scroll usually does **not** need sign inversion:

```
hid_hwheel_step = WheelEvent.deltaX.signum()
```

When both `deltaY` and `deltaX` are present (e.g. trackpads), Aero can emit **one** HID mouse report
that contains both the vertical wheel byte and the horizontal wheel (AC Pan) byte. The web runtime
uses a combined `mouse_wheel2(wheel, hwheel)` injection path when available to preserve this
“diagonal scroll in one frame” behavior.

#### Report model

The modeled mouse uses a 5-byte report in **HID report protocol**:

```
Byte 0: buttons (bits 0..4), remaining bits padding
Byte 1: X delta (i8)
Byte 2: Y delta (i8)
Byte 3: wheel delta (i8)
Byte 4: horizontal wheel delta / AC Pan (i8)
```

It also supports **HID boot protocol** (host-selectable via `SET_PROTOCOL`), which omits the wheel
bytes:

```
Byte 0: buttons
Byte 1: X delta
Byte 2: Y delta
```

Note: because boot protocol reports cannot carry wheel/hwheel deltas, scroll input is ignored while
the guest has selected boot protocol (though it still counts as user activity for remote-wakeup
purposes).

---

### HID report descriptor synthesis notes (WebHID)

When synthesizing HID report descriptors from WebHID metadata, be aware that **Unit Exponent**
(global item `0x55`) is defined by HID 1.11 as a **4-bit signed value** (`-8..=7`) stored in the
low nibble of a *single* byte.

- High nibble is reserved and must be `0`.
- Examples:
  - `unitExponent = -1` → `0x55 0x0F` (not `0x55 0xFF`)
  - `unitExponent = -2` → `0x55 0x0E`

Also note that Aero currently caps WebHID passthrough **input reports** to fit in a single USB
**full-speed interrupt** packet (**64 bytes**, including the optional report ID prefix). We
currently do not support splitting a single HID input report across multiple interrupt packets, so
input reports larger than 64 bytes are rejected during normalization/descriptor synthesis.

## USB HID Gamepad + Synthetic HID Topology

This document describes two related pieces of the USB input story:

1. The **USB HID gamepad report format** used by the emulator.
2. The guest-visible **synthetic HID topology** used to expose keyboard/mouse/gamepad input
   while consuming only **one root port** on the guest USB controller.

> Source of truth: [Canonical USB stack](../decisions/0015-canonical-usb-stack.md) defines the canonical USB
> stack for the browser runtime (`crates/aero-usb` + `apps/web/` host integration). This document focuses
> on the HID report/device contract on top of that stack.

---

### Guest-visible topology (external hub on root port 0)

The legacy guest USB controllers used by Aero (especially UHCI) expose only a small number of root
ports. To expose multiple HID devices without fighting for root ports, the input stack uses an
**external USB hub** attached on **root port 0**, then attaches synthetic devices behind it.

Current topology (see also [`../areas/usb-and-input.md`](../areas/usb-and-input.md)):

- Root port **0**: external USB hub
  - hub port **1**: USB HID keyboard
  - hub port **2**: USB HID mouse
  - hub port **3**: USB HID gamepad
  - hub port **4**: USB HID consumer-control (media keys)
  - hub ports **5+**: dynamically-allocated passthrough devices (e.g. WebHID)
- Root port **1**: reserved for WebUSB passthrough

Source-of-truth constants live in `apps/web/src/usb/uhci_external_hub.ts` and are used by the worker
runtime for UHCI/EHCI/xHCI builds. Note: when the hub is hosted behind xHCI, hub port numbers must
be <= **15** (xHCI Route String encodes downstream hub ports as 4-bit values), so the external hub
port count is clamped accordingly and “hub ports 5+” means ports 5..=15.

This approach keeps Windows 7 driver binding simple: each device binds via the in-box HID stack
(`hidusb.sys` + `hidclass.sys`) and then the appropriate client driver (`kbdhid.sys`, `mouhid.sys`,
`hidgame.sys`, …).

Note: `aero_usb::hid::composite::UsbCompositeHidInput` (device ID `UCMP`) still exists as an
alternative/legacy composite-device model used by some tests, but the default runtime topology is
the external-hub approach above.

---

### Gamepad report format

The emulator models a USB HID **Game Pad** top-level collection (`Usage 0x05` on the
Generic Desktop page) with:

- 16 digital buttons (HID usages Button 1..16)
- 1 hat switch (d-pad)
- 4 analog axes: X, Y, Rx, Ry (`int8`, `-127..=127`)

#### Report model (8 bytes)

The modeled gamepad uses a fixed 8-byte input report (no report ID):

```
Byte 0..1: Buttons bitfield (u16 little-endian)
Byte 2:    Hat switch (low 4 bits). 0=Up, 1=Up-Right, … 7=Up-Left. 8 = neutral/null.
Byte 3:    X  (int8)
Byte 4:    Y  (int8)
Byte 5:    Rx (int8)
Byte 6:    Ry (int8)
Byte 7:    Padding (0)
```

Notes:

- The hat switch uses HID “Null state” (`Input (… Null)`) so the centered value
  is represented by `8` (outside the logical range `0..=7`).
- The canonical report struct is `aero_usb::hid::GamepadReport`
  (`crates/aero-usb/src/hid/gamepad.rs`).
- The browser-side pack/unpack helpers live in `apps/web/src/input/gamepad.ts`.

#### Keeping report packing consistent (Rust ↔ TypeScript)

There are two independent implementations of the 8-byte gamepad report packing:

- Rust: `crates/aero-usb/src/hid/gamepad.rs::GamepadReport::to_bytes`
- TypeScript: `apps/web/src/input/gamepad.ts::packGamepadReport` + `unpackGamepadReport`

To prevent drift between them, we keep a shared fixture of report field values
and their expected packed bytes at:

- `protocol-vectors/hid_gamepad_report_vectors.json`
- `protocol-vectors/hid_gamepad_report_clamping_vectors.json` (includes out-of-range inputs to pin down clamping/masking semantics)

Both sides validate their packing logic against this fixture:

- Rust: `crates/aero-usb/tests/hid_gamepad_report_fixture.rs`
- Rust (clamping): `crates/aero-usb/tests/hid_gamepad_report_clamping_fixture.rs`
- TypeScript: `apps/web/src/input/gamepad.test.ts`

#### Button bitfield mapping (browser host)

When capturing a controller via the browser **Gamepad API** using the **standard mapping**
(`Gamepad.mapping === "standard"`), the host maps Gamepad button indices into the 16-bit
button bitfield as follows:

| Bit | Gamepad button index | Meaning |
| --- | --- | --- |
| 0 | 0 | A / Cross |
| 1 | 1 | B / Circle |
| 2 | 2 | X / Square |
| 3 | 3 | Y / Triangle |
| 4 | 4 | LB / L1 |
| 5 | 5 | RB / R1 |
| 6 | 6 | LT / L2 (digital `pressed`) |
| 7 | 7 | RT / R2 (digital `pressed`) |
| 8 | 8 | Back / Select |
| 9 | 9 | Start |
| 10 | 10 | Left stick press |
| 11 | 11 | Right stick press |
| 12 | 16 | Guide / Home |
| 13 | 17 | Extra (if present) |
| 14 | 18 | Extra (if present) |
| 15 | 19 | Extra (if present) |

The d-pad quartet (`buttons[12..15]`) is converted into the hat value and is not included
in the bitfield.

## WebUSB constraints for USB passthrough (Chromium)

This document describes what **WebUSB can and cannot do** in Chromium-based browsers, and how those constraints shape Aero’s **“non-HID USB passthrough”** integration (guest-visible WebUSB passthrough is possible, but remains limited/experimental).

Runtime note: WebUSB passthrough is currently implemented only for the legacy browser runtime (`vmRuntime="legacy"`). The canonical
machine runtime (`vmRuntime="machine"`) does not yet support WebUSB passthrough.

> Source of truth: [Canonical USB stack](../decisions/0015-canonical-usb-stack.md) defines the canonical USB stack
> selection for the browser runtime (`aero-usb` + `aero-wasm` + `apps/web/`).

The important takeaway is that WebUSB is **not a general-purpose “attach any USB device to the VM”** mechanism. It is usable primarily for **vendor-specific bulk/interrupt devices** that can be driven via **WinUSB/libusb** without the host OS binding a native class driver.

In practice, WebUSB failures are dominated by two constraints:

1. **Browser restrictions**: some USB interface classes are treated as “protected” and cannot be accessed via WebUSB.
2. **Host OS driver / permissions**: even when `navigator.usb` exists, `open()` / `claimInterface()` can fail if the OS driver binding or permissions are incorrect (especially on Windows).

See also: [`../areas/usb-and-input.md`](../areas/usb-and-input.md) for the end-to-end passthrough
architecture and security model (WebHID MVP + experimental WebUSB passthrough).

For the detailed UHCI ↔ WebUSB transfer/TD mapping and the host action/completion bridge,
see [`../areas/usb-and-input.md`](../areas/usb-and-input.md).

---

### Troubleshooting (common reasons WebUSB fails)

If WebUSB calls like `requestDevice()`, `device.open()`, or `device.claimInterface()` fail with an opaque `DOMException`:

- For a quick browser-level smoke test, use the in-app **WebUSB** panel (main UI) or open the standalone diagnostics page:
  - For an end-to-end passthrough smoke test (UHCI TDs → WASM harness → WebUSB → completions), use the in-app **“UHCI passthrough harness (WebUSB)”** panel. It runs a minimal USB enumeration sequence and prints the resulting device + configuration descriptor bytes.
  - For an EHCI-oriented passthrough smoke test (validates WebUSB action↔completion plumbing + EHCI-style `USBSTS`/IRQ semantics; not a full qTD/QH schedule engine), use the in-app **“EHCI passthrough harness (WebUSB)”** panel.
  - `/webusb_diagnostics.html`
  - The diagnostics page can also list `navigator.usb.getDevices()` (already-granted devices) and copy a JSON summary for bug reports.
  - Note: these panels are currently only available in `vmRuntime="legacy"` mode.
- **Secure context required:** WebUSB requires `https://` or `http://localhost` (`isSecureContext === true`).
- **User gesture required:** `navigator.usb.requestDevice()` must be triggered by a user gesture (e.g. a button click).
  - Call `requestDevice()` directly from the gesture handler; if you `await` before calling it, the user gesture can be lost.
  - User activation does **not** propagate across `postMessage()` to workers, so do the chooser step on the main thread.
  - If you previously granted permission to the wrong device (or want to revoke access), use the in-app **“Forget permission”** action when available (Chromium `USBDevice.forget()`), or remove the site's USB permission in browser settings and try again.
- **Permissions Policy / iframes:** WebUSB can be blocked by Permissions Policy. If you're running in an iframe, ensure the frame is allowed to use USB (e.g. `allow="usb"`) and that the response headers permit it.
- **Chooser canceled:** closing the chooser can surface as `NotFoundError` or `AbortError`. Just run `requestDevice()` again.
- **Worker transferability:** if you see `DataCloneError` (e.g. “could not be cloned”), your browser cannot structured-clone a `USBDevice` to a worker. Keep WebUSB I/O on the main thread and proxy requests to workers, or have the worker call `navigator.usb.getDevices()` after permission is granted.
  - Aero’s main-thread proxy pattern is implemented by `UsbBroker` + `WebUsbPassthroughRuntime` (`apps/web/src/usb/usb_broker.ts`, `apps/web/src/usb/webusb_passthrough_runtime.ts`, and `apps/web/src/usb/usb_proxy_protocol.ts`).
- **Protected interface classes:** WebUSB cannot access some interface classes (HID, mass storage, audio/video, etc.). Prefer a vendor-specific interface (class `0xFF`) or a more appropriate Web API (e.g. WebHID/WebSerial).
- **Isochronous endpoints (audio/video):** if you see `NotSupportedError` mentioning isochronous transfers, the device likely can't be used via WebUSB (WebUSB is generally control/bulk/interrupt only).
- **Endpoint / transfer errors:** if you see `InvalidAccessError` / `OperationError`, double-check that you're using the correct interface and endpoint numbers (and that the interface is claimed). If transfers fail intermittently, try unplug/replug and consider `device.reset()` / close+reopen.
- **Device busy / not readable:** `NotReadableError` or `NetworkError` on `open()` / `claimInterface()` can mean the host OS driver (or another application) has the device open exclusively. Close other apps, unplug/replug, and verify driver binding/permissions (WinUSB / udev).
- **Windows (WinUSB):** WebUSB typically requires the relevant interface to be bound to **WinUSB**.
  - For development: tools like **Zadig** can install WinUSB for a specific VID/PID/interface.
  - For production devices: ship **Microsoft OS 2.0 descriptors** / WinUSB Compatible ID descriptors so Windows binds WinUSB automatically.
- **Linux (udev / kernel driver):**
  - Ensure your user has permission to access the device (via `udev` rules).
  - Ensure no kernel driver is attached to the interface; a bound kernel driver can prevent `claimInterface()`.
- **macOS / Android:** support varies. If the OS has a built-in driver attached to the interface (or mobile USB/OTG restrictions apply), WebUSB may not be able to claim it. Vendor-specific interfaces (class `0xFF`) are the most feasible.
- **Chromium debugging:** in Chrome/Edge, `chrome://usb-internals` (or `edge://usb-internals`) can help confirm what the browser thinks is attached/blocked.

---

### Chromium “protected interface classes”

Chromium maintains a list of **protected USB interface classes**. Interfaces in these classes are treated as **security-sensitive** and cannot be requested/claimed by WebUSB.

#### Protected classes (Aero-relevant subset)

| `bInterfaceClass` (hex) | USB-IF class name | Practical impact for WebUSB |
|---:|---|---|
| `0x01` | Audio | Blocked (no USB audio passthrough/streaming via WebUSB) |
| `0x03` | HID (Human Interface Device) | Blocked (keyboards, mice, many game controllers) |
| `0x08` | Mass Storage | Blocked (flash drives, external HDDs/SSDs) |
| `0x09` | Hub | Blocked |
| `0x0B` | Smart Card | Blocked |
| `0x0E` | Video | Blocked (USB webcams/capture devices) |
| `0x10` | Audio/Video | Blocked |
| `0xE0` | Wireless Controller | Blocked (e.g. Bluetooth HCI adapters) |

> Note: Chromium’s protected list is maintained in Chromium source and may evolve. The table above captures the classes that matter most for Aero’s “USB passthrough” planning.

Implementation note: Aero mirrors a best-effort version of this list in code (for diagnostics / UI):

- `apps/web/src/platform/webusb_protection.ts`
- `apps/web/src/platform/webusb.ts`

#### What “protected” means in practice

- **Devices with only protected interfaces won’t appear** in the `navigator.usb.requestDevice()` chooser at all.
- **Composite devices can still be requestable** *if they contain at least one non-protected interface*.
  - Example: a device exposing `HID (0x03)` + `Vendor Specific (0xFF)` may appear in the chooser because of the vendor-specific interface.
  - However, **protected interfaces remain unclaimable**: attempts to `claimInterface()` on a protected interface will fail, so only the non-protected portion of the device is usable via WebUSB.

This matters for Aero because a “passthrough” implementation can only forward traffic for interfaces that WebUSB can actually claim.

---

### Transfer-type limitations (no practical USB audio/video streaming)

WebUSB exposes these transfer types:

- **Control transfers** (`controlTransferIn/Out`)
- **Bulk transfers** (`transferIn/Out` on bulk endpoints)
- **Interrupt transfers** (`transferIn/Out` on interrupt endpoints)

**Isochronous transfers are not generally available/stable in WebUSB** across browsers/platforms, which makes true passthrough of:

- USB Audio (typically isochronous)
- USB Video / UVC cameras (typically isochronous)

impractical via WebUSB.

If Aero ever targets isochronous support, it should be treated as **experimental** and gated behind explicit “this may not work” UX (Chromium flags/origin trials may be involved depending on the state of the platform).

---

### Host OS driver friction (the #1 reason “it works on my machine” fails)

Even when an interface class is not protected, WebUSB still needs the host OS to allow the browser process to open and claim that interface.

#### Windows: WinUSB is required per-interface

On Windows, WebUSB can only reliably talk to interfaces bound to **WinUSB** (or another libusb-compatible driver stack).

**Symptoms when WinUSB is not installed for the interface:**

- Device shows in the chooser, but `device.open()` or `claimInterface()` fails.
- Errors tend to surface as `NetworkError`/`NotFoundError`/`SecurityError` depending on the failure point.

**Strategy options:**

1. **Best (device/firmware controlled): ship WinUSB binding via Microsoft OS 2.0 descriptors**
   - Many vendor devices can advertise **MS OS 2.0 descriptors** (WCID) so Windows automatically associates the interface with WinUSB.
   - This is the lowest-friction path for end users and is the only approach that scales for production.
2. **Fallback (user installs a driver): use Zadig to bind WinUSB**
   - Users can use **Zadig** to replace the driver for a specific interface with WinUSB.
   - This typically requires admin rights and can break vendor software that expects the original driver.
   - For composite devices, users must select the correct interface (not the whole device) where Zadig exposes per-interface entries.

**Aero guidance:** for any “USB passthrough” UX on Windows, assume that **WinUSB installation is a prerequisite** and design the onboarding/troubleshooting flow accordingly.

#### Linux: udev permissions + kernel driver detachment

On Linux, WebUSB access usually fails for one of two reasons:

1. **Permissions:** the browser process cannot open `/dev/bus/usb/...`
   - Fix via a udev rule granting access to the relevant devices (vendor/product IDs).
   - Common patterns include `TAG+="uaccess"` (logind-managed access) or assigning to a group like `plugdev`.
2. **Kernel driver bound to the interface**
   - If a kernel driver already claimed the interface, the browser may fail to claim it.
   - Vendor-specific interfaces (`0xFF`) are most likely to work because they often have no in-kernel driver.

**Example udev rule (template):**

```udev
# /etc/udev/rules.d/99-aero-webusb.rules
SUBSYSTEM=="usb", ATTR{idVendor}=="1234", ATTR{idProduct}=="5678", TAG+="uaccess"
```

After adding a rule: `sudo udevadm control --reload-rules && sudo udevadm trigger` (or replug the device).

#### macOS: generally OK for vendor-specific, but system drivers still win

macOS does not have a WinUSB-style driver install step, but the same core rule applies:

- If the OS has a system driver bound to the interface, WebUSB cannot “steal” it reliably.
- Vendor-specific interfaces are the most feasible.

---

### Feasibility matrix for Aero use-cases

The table below translates WebUSB constraints into Aero product guidance.

| Aero use-case | Typical interface class | WebUSB feasibility | Notes / alternatives |
|---|---:|---|---|
| **Vendor-specific “bulk device passthrough”** (custom hardware, firmware tools, dongles) | `0xFF` (Vendor Specific) | **Works in principle** | Needs bulk/interrupt endpoints. On Windows requires WinUSB. This is the primary viable target for Aero’s “non-HID USB passthrough”. |
| Serial adapters / microcontrollers presenting as COM ports | `0x02/0x0A` (CDC ACM) | **Usually not via WebUSB** | Often bound to OS serial drivers. Prefer **WebSerial** as a separate integration path. |
| Keyboards / mice / most game controllers | `0x03` (HID) | **Not possible via WebUSB alone** | Chromium protects HID interfaces. Use **Pointer Lock + keyboard events**, **Gamepad API**, or (for some devices) **WebHID** as separate paths. |
| USB flash drives / external storage | `0x08` (Mass Storage) | **Not possible via WebUSB alone** | Protected class + OS driver binding. Use **File System Access API** / upload flows instead of block-device passthrough. |
| USB audio interfaces / headsets | `0x01` (Audio) | **Not possible via WebUSB alone** | Protected class + isochronous. Use **Web Audio** / OS audio routing. |
| Webcams / capture devices | `0x0E` (Video) | **Not possible via WebUSB alone** | Protected class + isochronous. Use **`getUserMedia()`** / MediaDevices. |
| Smart card readers | `0x0B` (Smart Card) | **Not possible via WebUSB alone** | Protected class. Consider domain-specific flows (e.g. WebAuthn) rather than VM passthrough. |

---

### “User steps” checklist (what Aero’s UI/UX must assume)

For any WebUSB-backed feature:

- Page must be in a **secure context**: `https://` or `http://localhost`
- **User gesture required**: `navigator.usb.requestDevice(...)` must run in a click/tap handler
- **Chromium-based browser required** (Chrome / Edge). Firefox and Safari do not provide WebUSB.
- Device must expose at least one **non-protected interface**; otherwise it will not show up in the chooser.
- **Windows:** the interface must be bound to **WinUSB** (MS OS 2.0 descriptors preferred; Zadig as a manual fallback).
- **Linux:** expect udev permissions work (and possible kernel driver detachment failures).
- **Isochronous (if ever targeted):** expect to require experimental flags and treat as non-production.

---

### Implications for Aero “non-HID USB passthrough”

When we say “USB passthrough” in Aero, what is realistically achievable in the browser is:

- **Forwarding USB control/bulk/interrupt transfers** for a **vendor-specific interface** that WebUSB can claim.

Note: the **guest-visible USB controller** matters. Aero’s default passthrough path is currently
UHCI (full-speed), which can be incompatible with some **high-speed-only** devices. When available
in the deployed build, the web runtime can instead route guest-visible WebUSB passthrough via
EHCI/xHCI (high-speed view), but this remains experimental/bring-up quality.

We should avoid promising:

- HID passthrough (keyboards/mice/controllers)
- Mass storage passthrough (flash drives)
- USB audio/video streaming devices
- Smart card readers

Those need separate, purpose-built integrations (virtio input, file import/export, `getUserMedia`, etc.) rather than raw USB forwarding.

### Related docs

- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — UHCI ↔ WebUSB passthrough architecture and TD-level NAK pending semantics
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — EHCI (USB 2.0) emulation contracts
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — xHCI (USB 3.x) emulation contracts

## WebUSB passthrough (browser → guest USB controller) architecture

This document describes the **USB passthrough** path where a *real* USB device
is exposed to the guest OS using **WebUSB** in the browser.

Runtime note: This document describes the legacy browser runtime (`vmRuntime="legacy"`) where guest USB controllers and the
`UsbPassthroughBridge` live in the I/O worker. The canonical machine runtime (`vmRuntime="machine"`) does not currently support WebUSB
passthrough.

> Source of truth: [Canonical USB stack](../decisions/0015-canonical-usb-stack.md) defines the canonical USB stack
> selection for the browser runtime. This document describes the chosen `aero-usb` + `apps/web/` design
> in detail.

The goal is to keep three moving parts coherent and spec-aligned:

- **Guest USB host controller emulation**:
  - **UHCI** (USB 1.1 full-speed; synchronous, TD-driven)
  - **EHCI** (USB 2.0 high-speed; supported/experimental for passthrough)
  - **xHCI** (USB 3.x, also handles USB 2.0/1.1; supported/experimental for passthrough)
    - EHCI controller emulation design/implementation notes: [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
    - xHCI controller emulation design/implementation notes: [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
- **Rust device model** (`UsbPassthroughDevice`; runs inside WASM/worker)
- **TypeScript WebUSB broker/executor** (runs where WebUSB is available; usually main thread)

Implementation references (current repo):

- Rust wire contract + action/completion queue (`UsbPassthroughDevice`): `crates/aero-usb/src/passthrough.rs`
- Rust guest-visible UHCI controller + TD handshake mapping: `crates/aero-usb/src/uhci/mod.rs`
- WASM export bridge (`UsbPassthroughBridge`): `crates/aero-wasm/src/lib.rs`
- WASM guest-visible UHCI controller (`UhciControllerBridge`) + WebUSB passthrough device lifecycle (`set_connected`, `drain_actions`, `push_completion`, `reset` on root port 1): `crates/aero-wasm/src/uhci_controller_bridge.rs` (re-exported from `crates/aero-wasm/src/lib.rs`)
- WASM guest-visible EHCI controller (`EhciControllerBridge`) + WebUSB passthrough device lifecycle (`set_connected`, `drain_actions`, `push_completion`, `reset` on root port 1; root port 0 remains available for an external hub / HID passthrough): `crates/aero-wasm/src/ehci_controller_bridge.rs` (re-exported from `crates/aero-wasm/src/lib.rs`)
- WASM guest-visible xHCI controller (`XhciControllerBridge`) + WebUSB passthrough device lifecycle (`set_connected`, `drain_actions`, `push_completion`, `reset` on a reserved root port; typically root port 1): `crates/aero-wasm/src/xhci_controller_bridge.rs` (re-exported from `crates/aero-wasm/src/lib.rs`)
- (Dev/harness) WASM standalone WebUSB UHCI bridge (`WebUsbUhciBridge`): `crates/aero-wasm/src/webusb_uhci_bridge.rs` (re-exported from `crates/aero-wasm/src/lib.rs`)
- WASM demo driver (`UsbPassthroughDemo`; queues GET_DESCRIPTOR requests to validate the action↔completion contract end-to-end): `crates/aero-wasm/src/lib.rs`
- WASM UHCI enumeration harness (dev smoke; `WebUsbUhciPassthroughHarness`): `crates/aero-wasm/src/webusb_uhci_passthrough_harness.rs`
- (Dev/harness) WASM EHCI passthrough harness (EHCI-like; validates action↔completion + USBSTS/IRQ semantics without full qTD/QH walking): `crates/aero-wasm/src/webusb_ehci_passthrough_harness.rs`
- TS canonical wire types (`SetupPacket`/`UsbHostAction`/`UsbHostCompletion`): `apps/web/src/usb/usb_passthrough_types.ts`
- TS WebUSB backend/executor (`WebUsbBackend`): `apps/web/src/usb/webusb_backend.ts` (+ `apps/web/src/usb/webusb_executor.ts`)
- TS main-thread broker for workers (optional): `apps/web/src/usb/usb_broker.ts` (+ `apps/web/src/usb/usb_proxy_protocol.ts`, `apps/web/src/usb/usb_proxy_ring.ts`)
- TS worker-side completion ring dispatcher (completion-ring fan-out when multiple runtimes subscribe): `apps/web/src/usb/usb_proxy_ring_dispatcher.ts`
- TS worker-side passthrough runtime (action/completion pump): `apps/web/src/usb/webusb_passthrough_runtime.ts`
- Guest-visible worker wiring (guest controller init/selection + WebUSB hotplug + passthrough runtime; `vmRuntime="legacy"`): `apps/web/src/workers/io.worker.ts`
- TS worker-side demo runtime (drains `UsbPassthroughDemo` actions, pushes completions, defines `usb.demo.run`, emits `usb.demoResult`): `apps/web/src/usb/usb_passthrough_demo_runtime.ts`
- TS worker-side UHCI harness runner (dev smoke): `apps/web/src/usb/webusb_harness_runtime.ts`
- TS worker-side EHCI harness runner (dev smoke): `apps/web/src/usb/webusb_ehci_harness_runtime.ts`
- TS guest-visible UHCI PCI device (I/O worker): `apps/web/src/io/devices/uhci.ts`
- (Dev/harness) TS standalone WebUSB UHCI PCI device: `apps/web/src/io/devices/uhci_webusb.ts`
- TS UI harness panels:
  - WebUSB diagnostics panel: `apps/web/src/usb/webusb_panel.ts`
  - WebUSB passthrough broker panel: `apps/web/src/usb/usb_broker_panel.ts` (rendered from `apps/web/src/main.ts`)
  - WebUSB UHCI harness panel (main thread): `apps/web/src/usb/webusb_uhci_harness_panel.ts`
- WebUSB passthrough demo panel (IO worker result + Run buttons, including a “Configuration full” rerun when `wTotalLength` indicates truncation): `apps/web/src/main.ts` (`renderWebUsbPassthroughDemoWorkerPanel`)
- WebUSB UHCI harness panel (I/O worker): `apps/web/src/main.ts` (`renderWebUsbUhciHarnessWorkerPanel`)
- WebUSB EHCI harness panel (I/O worker): `apps/web/src/main.ts` (`renderWebUsbEhciHarnessWorkerPanel`)
- Cross-language wire fixture: `protocol-vectors/webusb_passthrough_wire.json`
- (Legacy repo-root WebUSB demo broker/client RPC; removed; not the passthrough wire contract): previously lived under `src/platform/legacy/webusb_{broker,client,protocol}.ts`

Note: An early WebUSB passthrough prototype lived in `crates/aero-wasm/src/usb_passthrough.rs`.
It has been removed in favor of the single canonical `UsbPassthroughBridge` WASM export in
`crates/aero-wasm/src/lib.rs` that uses `aero_usb::passthrough::{UsbHostAction, UsbHostCompletion}`
with `serde_wasm_bindgen`.

Note: `crates/emulator` consumes `crates/aero-usb` via a thin integration layer (PCI/PortIO wiring +
compatibility re-exports) for native/emulator tests. Per [Canonical USB stack](../decisions/0015-canonical-usb-stack.md),
the browser/WASM runtime uses `aero-usb` + `aero-wasm` + `apps/web/` and should not grow a parallel USB
stack in `crates/emulator`.

---

### Problem statement: async WebUSB vs synchronous UHCI

WebUSB operations are **asynchronous** (`Promise`-based) and can take an unbounded amount
of time from the VM’s point of view (scheduler latency, permission prompts, OS USB stack
latency, device latency).

UHCI, in contrast, is driven by a periodic schedule (frames) and consumes **Transfer
Descriptors (TDs)** that describe individual USB transactions. When the controller
processes a TD, the emulation must produce a **synchronous outcome** (“this TD is still
active”, “it completed with ACK”, “it stalled”, etc.) without blocking the worker thread.

#### Why “TD-level NAK while pending” is the right mechanism

On real USB, endpoints can legitimately respond with **NAK** to indicate “not ready yet”.
For UHCI this maps cleanly to:

- leaving the TD **Active** so it remains scheduled, and
- retrying the TD on a later frame without treating it as an error.

This is the correct way to represent “we started a host-side WebUSB transfer but do not
have a completion yet”:

1. The guest continues running (no blocking `await` in the emulation core).
2. The controller naturally retries at the same granularity the guest OS expects (TDs).
3. Guest-side timeouts/cancellation behave correctly (drivers already handle long NAK
   streaks on bulk transfers and control-transfer stages).

Avoid alternatives such as:

- **Blocking the emulation thread** until a Promise resolves (would freeze the VM and can
  deadlock UI/worker scheduling).
- **Completing TDs later without NAK semantics** (guest drivers can observe “stuck” TDs
  and expect NAK/timeout behavior; also risks re-issuing transfers incorrectly).

---

### Two-layer design

#### Layer 1 (Rust/WASM): `UsbPassthroughDevice`

The Rust-side device model must never call WebUSB directly. Instead it:

- emits **host actions** (`UsbHostAction`) describing what needs to happen on the host,
- consumes **host completions** (`UsbHostCompletion`) to finish TDs and deliver data.

Key properties:

- Pure state machine: deterministic given input TD stream + completions.
- No `async`/`await` required inside the VM core; “pending” is represented by NAK.
- One in-flight action per endpoint (recommended) to keep data-toggle behavior coherent
  (see [Bulk transfers](#bulk-transfers-packet-granularity-and-toggle-sync)).

Canonical wire shapes (locked down by `protocol-vectors/webusb_passthrough_wire.json` and shared
between Rust and TypeScript; see `crates/aero-usb/src/passthrough.rs` and
`apps/web/src/usb/usb_passthrough_types.ts` (re-exported from `apps/web/src/usb/webusb_backend.ts`):

```ts
type SetupPacket = {
  bmRequestType: number;
  bRequest: number;
  wValue: number;
  wIndex: number;
  wLength: number;
};

type UsbHostAction =
  | { kind: "controlIn"; id: number /* u32 */; setup: SetupPacket }
  | { kind: "controlOut"; id: number /* u32 */; setup: SetupPacket; data: Uint8Array }
  | { kind: "bulkIn"; id: number /* u32 */; endpoint: number /* endpoint address */; length: number }
  | { kind: "bulkOut"; id: number /* u32 */; endpoint: number /* endpoint address */; data: Uint8Array };

type UsbHostCompletion =
  | { kind: "controlIn"; id: number /* u32 */; status: "success"; data: Uint8Array }
  | { kind: "controlIn"; id: number /* u32 */; status: "stall" }
  | { kind: "controlIn"; id: number /* u32 */; status: "error"; message: string }
  | { kind: "controlOut"; id: number /* u32 */; status: "success"; bytesWritten: number }
  | { kind: "controlOut"; id: number /* u32 */; status: "stall" }
  | { kind: "controlOut"; id: number /* u32 */; status: "error"; message: string }
  | { kind: "bulkIn"; id: number /* u32 */; status: "success"; data: Uint8Array }
  | { kind: "bulkIn"; id: number /* u32 */; status: "stall" }
  | { kind: "bulkIn"; id: number /* u32 */; status: "error"; message: string }
  | { kind: "bulkOut"; id: number /* u32 */; status: "success"; bytesWritten: number }
  | { kind: "bulkOut"; id: number /* u32 */; status: "stall" }
  | { kind: "bulkOut"; id: number /* u32 */; status: "error"; message: string };
```

For bulk transfers, `endpoint` is a USB endpoint **address** (direction bit included), not just the
endpoint number. See [Bulk transfers](#bulk-transfers-packet-granularity-and-toggle-sync) for details.

The `id` correlates an action with its completion and is also how we prevent duplicate
WebUSB calls when the guest retries a NAKed TD.

⚠️ **WASM note:** ids are generated in Rust and must fit in a JS `number` without loss.
The canonical wire contract uses **non-zero `u32` ids** (`1..=0xFFFF_FFFF`; `0` is reserved/invalid).
The worker-side runtime
(`apps/web/src/usb/webusb_passthrough_runtime.ts`) accepts `number` or `bigint` ids from WASM, but
will reject and reset the bridge if an action id is missing or out of the `u32` range (to avoid
deadlocking the Rust-side action queue on an action we can never complete).

⚠️ **WASM note:** the USB passthrough drain APIs return `null` when there are no queued actions
(to keep the poll path allocation-free). Treat `null`/`undefined` as “no work”.
This applies to:
- `UsbPassthroughBridge.drain_actions()`
- `UhciControllerBridge.drain_actions()`
- `EhciControllerBridge.drain_actions()`
- `XhciControllerBridge.drain_actions()`
- `WebUsbUhciBridge.drain_actions()`
- `UhciRuntime.webusb_drain_actions()`

Cancellation behavior:

- A new control SETUP can legally abort a previous control transfer. `UsbPassthroughDevice` treats
  any new `(setup, data)` tuple as a new request. It cancels the previous in-flight id and:
  - if the host has not dequeued the old `UsbHostAction` yet, it drops it from the action queue so
    we do not execute a stale control transfer, and
  - ignores any later completion for the canceled id (WebUSB does not provide strong cancellation
    for an in-flight transfer).
- Stale completions are expected and must be ignored (the Rust model does this by checking `id`
  against in-flight state in `push_completion`).
- `UsbPassthroughDevice::reset()` clears queued actions/completions and cancels all in-flight
  requests.

#### Snapshot/restore (save-state)

`crates/aero-usb` device models support deterministic snapshot/restore using the repo-standard
`aero-io-snapshot` TLV encoding (`aero_io_snapshot::io::state::IoSnapshot`).

For WebUSB passthrough specifically:

- `UsbPassthroughDevice` snapshots only its monotonic action id counter (`next_id`) and **drops all**
  queued/in-flight host I/O on restore. This prevents replaying side effects after restore; the guest
  will naturally retry transfers, emitting fresh `UsbHostAction`s.
- `UsbWebUsbPassthroughDevice` snapshots guest-visible USB state (address + control pipe stage) so a
  restore taken mid-control-transfer does not deadlock the guest. Newer snapshots also include the
  full internal `UsbPassthroughDevice` host-action state, so an in-flight transfer can resume
  deterministically: the relevant TD will continue returning NAK until a completion with the same
  action id is injected (and no duplicate `UsbHostAction`s are emitted).

Browser integration note (important):

- WebUSB host actions are backed by JS Promises that cannot be resumed after a VM snapshot restore.
  The WASM restore paths therefore call `reset_host_state_for_restore()` after restore (e.g.
  `UhciControllerBridge.load_state()`, `WebUsbUhciBridge.load_state()`, `UhciRuntime.load_state()`,
  `EhciControllerBridge.load_state()`, `XhciControllerBridge.load_state()`).
  - This clears queued/in-flight host actions/completions, preventing deadlock on a completion that
     will never arrive.
  - The monotonic `next_id` is preserved, so re-emitted actions still have deterministic ids.

WASM snapshot API note:

- The UHCI WASM bridges expose deterministic snapshot bytes via `snapshot_state()/restore_state()`
  (aliases over the existing `save_state/load_state` entrypoints). These snapshot bytes represent only
  USB stack state (controller + device models), not guest RAM.

Idempotency / retry behavior (important):

- A NAKed TD will be retried by the UHCI schedule. This means the device model entrypoints
  (`handle_control_request`, `handle_in_transfer`, `handle_out_transfer`) can be called multiple
  times for the same guest-visible transfer.
- `UsbPassthroughDevice` is written to be **idempotent** under retries:
  - control requests use an “in-flight” record to avoid emitting duplicate host actions while a
    completion is pending
  - non-control endpoints use a per-endpoint in-flight map for the same reason
  - completions are keyed by `id` and consumed exactly once

#### Layer 2 (host/TS): WebUSB executor + broker (main thread)

The host side owns the actual `USBDevice` handle and performs WebUSB calls:

- **Executor**: receives `UsbHostAction`, runs the corresponding WebUSB operation, and
  sends back `UsbHostCompletion`.
- **Broker**: deals with UI and lifecycle concerns:
  - `navigator.usb.requestDevice()` (must be triggered by user activation)
  - (re)open/select configuration/claim interfaces
  - handling `disconnect` events and surfacing errors to UI

In this repo, the canonical WebUSB passthrough integration lives under `apps/web/src/usb/`:

- **Executor** (canonical `UsbHostAction` contract): `apps/web/src/usb/webusb_backend.ts`
  - (thin wrapper): `apps/web/src/usb/webusb_executor.ts`
- **Main thread broker** (worker proxy): `apps/web/src/usb/usb_broker.ts`
  - (message schema + validators): `apps/web/src/usb/usb_proxy_protocol.ts`

Note: the repo previously included a separate **legacy/demo** WebUSB broker/client RPC under
`src/platform/legacy/webusb_*`. It has been removed and was never the `UsbHostAction` passthrough
contract described in this document.

#### Device lifecycle: open/configuration/interface claiming

WebUSB requires the browser process to:

1. `device.open()`
2. `device.selectConfiguration(...)` (if no active configuration)
3. `device.claimInterface(...)` for any interface whose endpoints will be used

In this repo:

- `WebUsbBackend.ensureOpenAndClaimed()` (`apps/web/src/usb/webusb_backend.ts`) performs the open/select
  configuration/claim steps before executing a `UsbHostAction`.
- If WebUSB must run on the main thread, `UsbBroker` (`apps/web/src/usb/usb_broker.ts`) owns the
  `USBDevice` handle and services worker requests via the `usb_proxy_protocol.ts` message schema.

The TypeScript executor (`WebUsbBackend.execute()`) also recognizes a few **standard USB control
requests** that represent high-level device state transitions and routes them through the
dedicated WebUSB APIs instead of emitting a raw `controlTransferOut`:

- `SET_CONFIGURATION` → `USBDevice.selectConfiguration(...)`
  - Note: WebUSB can reject `selectConfiguration` while interfaces are claimed; the executor
    releases claimed interfaces first.
  - The executor handles this before the general “claim interfaces” path so that a guest-driven
    configuration switch cannot be blocked by unrelated interface-claim failures.
- `SET_INTERFACE` → `USBDevice.selectAlternateInterface(...)` (claiming the interface if needed)
- `CLEAR_FEATURE(ENDPOINT_HALT)` → `USBDevice.clearHalt(...)`

This keeps the canonical `UsbHostAction` / `UsbHostCompletion` wire contract unchanged, but tends
to be more reliable on real devices because browsers/OS stacks may treat these requests specially.

Important constraints for passthrough:

- **Do not blindly claim every interface** on composite devices. Devices can expose a mix of
  protected (unclaimable) and unprotected interfaces; claiming a protected interface can fail
  even when the device was selectable due to an unprotected interface.
  - Use the repo’s protected-class classifier (`apps/web/src/platform/webusb_protection.ts`) to choose
    claimable interfaces.
- **Guest-visible configuration vs host-visible configuration:** the guest may issue
  `SET_CONFIGURATION` / `SET_INTERFACE`. The passthrough backend must decide whether to:
  - mirror those changes to the physical device (calling WebUSB `selectConfiguration` and
    `claimInterface`/`selectAlternateInterface` as needed), or
  - virtualize them (presenting descriptors/configuration state to the guest without mutating the
    already-open physical device).

The current Rust `UsbHostAction` surface does not yet include “select configuration / claim
interface” actions; open/config/claim is still handled by `WebUsbBackend.ensureOpenAndClaimed()`,
but the executor will mirror the most common standard configuration/interface transitions to the
host device as described above (and update its internal “claimed interface” cache when the active
configuration changes).

#### Encoding `SetupPacket` for WebUSB

Rust-side USB control requests use a USB 1.1-style `SetupPacket`:

```text
bmRequestType, bRequest, wValue, wIndex, wLength
```

WebUSB represents the same information as:

- operation choice: `controlTransferIn(...)` vs `controlTransferOut(...)` (direction)
- `USBControlTransferParameters`:
  - `requestType: 'standard' | 'class' | 'vendor'`
  - `recipient: 'device' | 'interface' | 'endpoint' | 'other'`
  - `request` / `value` / `index`
- and an explicit `length` argument for IN transfers

Important rules:

- The **direction bit** (`bmRequestType & 0x80`) must match the WebUSB call you make.
  - For `controlTransferIn`, require Device→Host.
  - For `controlTransferOut`, require Host→Device.
- For OUT transfers, the payload length must match `wLength`. For a zero-length OUT request (`wLength == 0`),
  omit the data argument (or use an empty payload in the `UsbHostAction` contract).

In this repo:

- The production WebUSB executor has helpers for this conversion and direction checking:
  `apps/web/src/usb/webusb_backend.ts` (`parseBmRequestType`, `validateControlTransferDirection`,
  `setupPacketToWebUsbParameters`).
- `WebUsbBackend` normalizes WebUSB `DataView` payloads into `Uint8Array` completions
  (`dataViewToUint8Array`). If you forward completions across `postMessage`, you may transfer the
  underlying `ArrayBuffer` to avoid copies.
  - `apps/web/src/usb/usb_proxy_protocol.ts` exports helpers (`getTransferablesForUsbProxyMessage`, etc.)
    to pick the correct transfer list for `usb.action` / `usb.completion` messages.
  - Note: transferring detaches the sender’s `ArrayBuffer`. Treat the payload `Uint8Array` as
    consumed after `postMessage`.
  - Some buffers (notably `WebAssembly.Memory.buffer`) are not transferable and will throw if put
    in the transfer list. Production code should fall back to non-transfer `postMessage` (copy) in
    that case; the built-in passthrough runtime/broker already do this.

#### Physical disconnect / guest hot-unplug

When the physical device is unplugged, the browser fires a `navigator.usb` `"disconnect"` event.
For guest correctness, this must be reflected as a **USB disconnect** on the emulated root hub
port so the guest OS can tear down drivers cleanly.

In this repo:

- `UsbBroker` listens for `navigator.usb` disconnect events and tears down the selected device
  (`apps/web/src/usb/usb_broker.ts`). It resolves any in-flight actions and broadcasts
  `{ type: "usb.selected", ok: false, error: ... }` to attached worker ports.
- Guest-side hot-unplug should detach the emulated device from its UHCI port so the guest observes
  the connect-status-change bits (e.g. `UhciController::hub_mut().detach(port_index)` in
  `crates/aero-usb/src/uhci/mod.rs`).

Recommended behavior on a physical disconnect:

1. Detach the passthrough device model from the associated emulated port (root hub or downstream hub).
2. Cancel any in-flight host actions (call `reset()` / cancel hooks on the *existing* device model).
3. Ignore any completions that arrive after detach (they will be stale by `id` **as long as ids are not reused**;
   see [Action id monotonicity across disconnect/reconnect](#action-id-monotonicity-across-disconnectreconnect)).

##### Action id monotonicity across disconnect/reconnect

`UsbPassthroughDevice` treats completions as *stale* by checking whether the completion `id` is currently
recorded as “in flight” (`push_completion` drops completions whose id is not in the in-flight maps).
This relies on action ids being **monotonic / never reused** across the lifetime of the passthrough device
model.

Because WebUSB transfers are Promise-based, a transfer started *before* a disconnect can still resolve
later (success/error) even after the browser has fired a `"disconnect"` event. If the host integration
**drops** the `UsbWebUsbPassthroughDevice` / `UsbPassthroughDevice` instance on disconnect and later creates
a fresh one, its action ids restart at `1`. Late completions from the previous “session” can then collide
with newly reused ids and be incorrectly accepted as completions for the new in-flight transfers.

**Requirement / recommendation:**

- Keep a single `UsbWebUsbPassthroughDevice` (or underlying `UsbPassthroughDevice`) instance alive across
  connect/disconnect/reconnect.
- On disconnect:
  - detach it from the emulated hub/port (`set_connected(false)` / `hub.detach(...)`), and
  - call `reset()` (or equivalent) to clear queued/in-flight host actions/completions,
  - but **do not reset** the monotonic `next_id` counter (note: `UsbPassthroughDevice::reset()` clears
    host state but intentionally does *not* reset `next_id`).
- On reconnect, reattach the same device model instance so ids continue increasing.

This applies equally to WASM UHCI integrations: keep the same `WebUsbUhciBridge` / `UhciControllerBridge`
handle alive across physical disconnect/reconnect, and use `set_connected(false)` + `reset()` instead of
destroying and recreating the bridge (which would restart ids).

Data flow (conceptual):

```
Guest UHCI TDs
    │
    ▼
UHCI emulation (worker) ──calls──► UsbPassthroughDevice (worker)
    │                                 │
    │                                 ├─ emits UsbHostAction ───────────────┐
    │                                 │                                       │
    │                                 ◄─ consumes UsbHostCompletion ─────────┘
    │
    ▼
IRQ / TD status updates back to guest

Host-side (main thread)
  WebUSB broker/executor owns USBDevice and services actions.
```

---

### Stage mapping (UHCI TDs ↔ WebUSB transfers)

#### Control transfers (SETUP/DATA/STATUS)

WebUSB exposes control transfers as **one call**:

- `controlTransferIn(setup, length)`
- `controlTransferOut(setup, data)`

UHCI represents the same operation as a TD chain:

1. **SETUP TD** (8 bytes, PID=SETUP, DATA0)
2. **DATA TD(s)** (PID=IN or OUT, DATA1 toggling)
3. **STATUS TD** (zero-length, PID opposite of DATA, DATA1)

Aero mapping (current `aero-usb` stack):

- **SETUP TD**
  - Decoded and dispatched by `UhciController` (`crates/aero-usb/src/uhci/mod.rs`): it reads the
    8-byte setup packet, parses `aero_usb::SetupPacket`, and forwards it to the addressed
    `device::AttachedUsbDevice::handle_setup` (`crates/aero-usb/src/device.rs`).
  - SETUP TDs always complete with **ACK** once a device is present. NAK is not used for SETUP.

- **DATA + STATUS TDs**
  - The UHCI-visible passthrough device wrapper turns the full control request (setup + optional OUT
    data stage) into exactly one `UsbHostAction::ControlIn` / `UsbHostAction::ControlOut`
    (wire `kind: "controlIn" | "controlOut"`) via
    `UsbPassthroughDevice::handle_control_request` (`crates/aero-usb/src/passthrough.rs`).
  - While the host action is in-flight, retries of the relevant **DATA** or **STATUS** TD return
    **NAK**, leaving the TD active so the UHCI schedule retries it on later frames.
    - Control-IN: pending is applied to the **DATA (IN)** stage (and to **STATUS (OUT)** when
      `wLength == 0`).
    - Control-OUT: OUT **DATA** TDs are ACKed as bytes are buffered; pending is applied to
      **STATUS (IN)** once the full payload is buffered (or immediately when `wLength == 0`).
- When the host completion arrives:
  - Control-IN: the completion’s `data` is served to IN TDs. An empty payload is represented as an
    ACK with `bytes=0` (ZLP). (`UhciController` encodes a 0-byte completion as `actlen=0x7FF`.)
  - Control-OUT: once the completion reports `status: "success"`, the STATUS stage ACKs with a
    0-byte packet.
  - `status: "stall"` maps to STALL; `status: "error"` maps to TIMEOUT (see
    [Host completion to guest TD status mapping](#host-completion-to-guest-td-status-mapping)).

Note on **short packets** (Control-IN):

- Real devices may legally return fewer bytes than `wLength` (e.g. descriptor reads where the OS
  asks for 255 bytes but the descriptor is shorter). This appears to the UHCI layer as a **short
  packet** (`actlen < maxlen`) on an IN DATA TD.
- Guest UHCI drivers typically rely on the UHCI **short packet detect** (SPD) bit + the
  `USBINTR_SHORT_PACKET` enable bit to get an interrupt and terminate the DATA stage early (skipping
  any remaining IN TDs and proceeding to the STATUS stage).
- `aero-usb`’s UHCI model honors SPD by stopping further TD processing for the current queue head
  within the same frame when a short packet is received and SPD is set.

Special-case note: `SET_ADDRESS` must be virtualized for full guest enumeration (guest-visible USB
address changes must not be forwarded to the physical device, which is already host-enumerated).

#### Bulk transfers: packet granularity and toggle sync

UHCI bulk/interrupt transfers are naturally TD-per-packet. WebUSB bulk/interrupt APIs
(`transferIn`/`transferOut`) can represent **multi-packet** transfers, but we generally
should not use that for UHCI passthrough.

**Recommendation: issue one `UsbHostAction` per guest TD**, with `length <= wMaxPacketSize`.

Why:

- The guest driver sets the TD’s **DATA0/DATA1 toggle** expecting it to advance exactly
  once per successful packet.
- Collapsing multiple guest TDs into one WebUSB transfer advances the physical endpoint’s
  toggle multiple times while the guest only advances once, desynchronizing the stream.

Note: the current `aero-usb` UHCI implementation (`crates/aero-usb/src/uhci/mod.rs`) does not yet model
the TD token’s data-toggle bit. The “one packet per action” rule is therefore forward-looking, but
still the recommended shape to avoid subtle bugs once toggle tracking is implemented.

Mapping:

- **Bulk OUT TD** → `UsbHostAction::BulkOut { endpoint, data }` → `USBDevice.transferOut(endpoint & 0x0f, ...)`
- **Bulk IN TD** → `UsbHostAction::BulkIn { endpoint, length }` → `USBDevice.transferIn(endpoint & 0x0f, ...)`

(`endpoint` is a USB endpoint **address**. For IN transfers it is `0x80 | ep_num` (e.g. `0x81`);
for OUT transfers it is `ep_num` with bit7 clear (e.g. `0x02`). The host side should use
`endpoint & 0x0f` as the WebUSB `endpointNumber`, and use the action kind to determine direction.)

Pending behavior:

- When an action is in flight, retry attempts of the same TD return **NAK** without
  emitting another action (keyed by `id` / TD identity).
- When completion arrives, complete the TD with:
  - actual byte count (short packets are valid and often meaningful), or
  - STALL / error mapping (see [Host completion to guest TD status mapping](#host-completion-to-guest-td-status-mapping)).

---

### Host completion to guest TD status mapping

At the browser layer, WebUSB returns a `USBTransferStatus` (`"ok" | "stall" | "babble"`) for
`controlTransfer*` / `transfer*` calls, and can also throw `DOMException`s for other failures
(permissions, disconnects, OS driver issues, etc).

`apps/web/src/usb/webusb_backend.ts` normalizes those outcomes into the canonical `UsbHostCompletion`
wire shape:

Recommended normalization rules (current implementation):

- `status === "ok"` → `status: "success"` (with `data` for IN or `bytesWritten` for OUT)
- `status === "stall"` → `status: "stall"`
- `status === "babble"` → `status: "error"` (with a message)
- thrown `DOMException` / other failures → `status: "error"` (with a message)

Guest-visible behavior is then derived from the Rust mapping in
`crates/aero-usb/src/passthrough.rs`:

| Host completion (`UsbHostCompletion`) | Guest-visible outcome | Notes |
|---|---|---|
| `{ status: "success", data }` (IN kinds) | `DATA` (for IN TDs) | Data is truncated to `wLength` for control-IN and to the TD `max_len` for bulk IN. |
| `{ status: "success", bytesWritten }` (OUT kinds) | `ACK` | OUT TD completes successfully. |
| `{ status: "stall" }` | `STALL` | TD completes with STALLED; guest driver is responsible for recovery. |
| `{ status: "error", message }` | `TIMEOUT` | Passthrough maps non-stall errors to a UHCI timeout/CRC error to unblock the guest. |
| (no completion yet; action in-flight) | `NAK` | Keep TD active so the UHCI schedule naturally retries without duplicating host work. |

Implementation note: in `aero-usb`, `Nak` is a first-class “retry later” outcome:

- `UhciController` sets `TD_CTRL_NAK` and leaves the TD active (`crates/aero-usb/src/uhci/mod.rs`).
- `UsbPassthroughDevice` returns `ControlResponse::Nak` / `UsbInResult::Nak` / `UsbOutResult::Nak`
  while a host action is pending (`crates/aero-usb/src/passthrough.rs`).

---

### Speed and descriptor handling (UHCI vs EHCI/xHCI)

#### UHCI mode: guest is full-speed

When the passthrough device is attached to a guest **UHCI** controller, the guest sees it
as a **full-speed** device.

This creates a mismatch when the physical device is high-speed (USB 2.0) on the real
machine: we must present a **full-speed-compatible configuration** to the guest so it
chooses correct max packet sizes and intervals.

#### UHCI-only fixup: `OTHER_SPEED_CONFIGURATION` → `CONFIGURATION`

USB 2.0 devices can expose an `OTHER_SPEED_CONFIGURATION` descriptor that describes how
they would look at the *other* speed:

- If the device is currently operating at **high-speed**, `OTHER_SPEED_CONFIGURATION`
  describes the **full-speed** configuration (endpoint max packet sizes, intervals, etc).

Approach:

1. During enumeration, issue a standard control request:
   - `GET_DESCRIPTOR(OTHER_SPEED_CONFIGURATION, index=0)`
2. If present and well-formed:
   - Use it as the basis for the guest-visible configuration, but rewrite the top-level
      `bDescriptorType` from `OTHER_SPEED_CONFIGURATION` to `CONFIGURATION` before exposing
      it to the guest stack (the layout is otherwise identical).
3. If not present (or rejected by WebUSB):
   - Fall back to the regular `CONFIGURATION` descriptor.

Practical implications:

- Devices that are **high-speed-only** without a usable other-speed configuration are not
  good candidates for UHCI passthrough; they should be attached via **EHCI/xHCI** so the
  guest can enumerate them at high-speed.
- Even for high-speed devices, sending smaller full-speed-sized transfers via WebUSB is
  typically valid (it is legal to transfer less than max packet size), but correctness
  depends on descriptors matching what the guest believes.

In this repo, the production WebUSB executor performs this as a best-effort fixup inside
`executeWebUsbControlIn` (`apps/web/src/usb/webusb_backend.ts`; see `shouldTranslateConfigurationDescriptor`
and `rewriteOtherSpeedConfigAsConfig`).

⚠️ This translation is **UHCI/full-speed-only**. When a passthrough device is attached to a
guest **EHCI/xHCI** controller (high-speed view), the host executor must **not**:

- attempt to fetch `OTHER_SPEED_CONFIGURATION`, or
- rewrite an `OTHER_SPEED_CONFIGURATION` descriptor into a `CONFIGURATION` descriptor.

In EHCI/xHCI mode the guest expects the device’s **high-speed** descriptors as-is, and any
UHCI-oriented fixups would produce incorrect endpoint packet sizes/intervals.

#### EHCI/xHCI mode: guest is high-speed

When a passthrough device is attached to a guest **EHCI/xHCI** controller (supported in the web
runtime when the WASM build exports the required WebUSB passthrough hooks on those controller
bridges), the intended behavior is:

- EHCI controller model design/contract: [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
- xHCI controller model design/contract: [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
- The guest enumerates the physical device as **high-speed**.
- The WebUSB executor should forward `GET_DESCRIPTOR(CONFIGURATION)` results without rewriting
  descriptor types or attempting other-speed translation.
- Devices that are **high-speed-only** should be attached via EHCI/xHCI rather than UHCI, since a
  UHCI/full-speed view cannot represent them correctly.

---

### Worker and threading constraints (browser reality)

#### User activation is required for device selection

`navigator.usb.requestDevice()` must be called from a **user-activated** event handler
(e.g. button click). This forces a “broker” role on the UI layer:

- the main thread is responsible for prompting and persisting the selected device,
- the emulator worker cannot autonomously attach arbitrary devices.

In this repo, the production UI triggers selection via `UsbBroker.requestDevice()` (which must be
called directly from a click handler; see `apps/web/src/usb/usb_broker_panel.ts` and
`apps/web/src/usb/usb_broker.ts`).

#### `USBDevice` is likely non-transferable

In practice, `USBDevice` should be treated as **non-structured-cloneable** and therefore
non-transferable to workers. Even if some browser versions eventually allow worker access,
this should not be relied upon for the core architecture.

The production WebUSB diagnostics panel includes a probe worker that attempts structured cloning of
the selected `USBDevice` (`apps/web/src/usb/webusb_panel.ts`, `apps/web/src/usb/webusb_probe_worker.ts`).

#### Recommended architecture when WebUSB is unavailable in workers

Assume WebUSB calls must run on the main thread:

- **Worker (WASM):** UHCI + `UsbPassthroughDevice` emits actions via a queue/ring buffer.
- **Main thread:** broker/executor receives actions, calls WebUSB, and returns completions.

##### Default / legacy path: `postMessage` + transferred `ArrayBuffer`s

This pattern is implemented by `UsbBroker` (main thread) using a `postMessage` protocol
(`apps/web/src/usb/usb_broker.ts`, `apps/web/src/usb/usb_proxy_protocol.ts`):

- worker → main thread: `{ type: "usb.action", action: UsbHostAction }`
- main thread → worker: `{ type: "usb.completion", completion: UsbHostCompletion }`

Byte payloads (`Uint8Array`) are transferred where possible to avoid copies. (A legacy broker/client RPC
implementation previously existed under `src/platform/legacy/webusb_*`, but it has been removed and was not the
canonical passthrough wire contract.)

##### Fast path: SharedArrayBuffer ring buffers (`usb.ringAttach`)

When `globalThis.crossOriginIsolated === true` (COOP/COEP enabled) and `SharedArrayBuffer`/`Atomics`
are available, the broker/worker proxy enables an optional SharedArrayBuffer-backed ring-buffer fast path
negotiated by `{ type: "usb.ringAttach", actionRing, completionRing }`:

- **`actionRing` (worker → main thread):**
  - the worker-side passthrough runtime writes `UsbHostAction` records into the ring (SPSC producer).
  - the main thread drains actions on a timer and executes them via `WebUsbBackend`.
- **`completionRing` (main thread → worker):**
  - the main thread writes `UsbHostCompletion` records into the ring (SPSC producer).
  - the worker drains completions via a shared dispatcher (`usb_proxy_ring_dispatcher.ts`) so multiple runtimes
    on the same port can observe completions without racing each other to `popCompletion()`.

The fast path is opportunistic: when a ring is full (or a record is too large to fit), the sender falls back
to `postMessage` (`usb.action` / `usb.completion`) so passthrough continues to make forward progress even under
temporary backpressure.

If a worker-side runtime starts after the initial `usb.ringAttach` (e.g. WASM finished loading late), it can
request the ring handles via `{ type: "usb.ringAttachRequest" }`. The production `UsbBroker` responds by
re-sending the ring buffers when possible; older brokers may ignore this message and the runtime should keep
functioning via the `postMessage` path.

If a ring buffer becomes corrupted (e.g. a decode error while popping records) the runtime can request the
broker to disable the SharedArrayBuffer fast path for that port by sending `{ type: "usb.ringDetach", reason? }`.
The broker will stop draining the action ring / pushing completions into the completion ring and will fall back
to `postMessage` (`usb.action` / `usb.completion`). Runtimes should treat `usb.ringDetach` as a signal to detach
their local ring wrappers and continue proxying via `postMessage`.

Implementation pointers (current code):

- Ring buffer: `apps/web/src/usb/usb_proxy_ring.ts` (`UsbProxyRing`)
- Protocol schema: `apps/web/src/usb/usb_proxy_protocol.ts` (`usb.ringAttach`, `usb.ringAttachRequest`, `usb.ringDetach`)
- Main thread setup + action-ring drain: `apps/web/src/usb/usb_broker.ts` (`attachRings`, `drainActionRing`)
- Worker-side attach + action forwarding: `apps/web/src/usb/webusb_passthrough_runtime.ts` (`attachRings`, `pollOnce`)
- Worker-side completion drain + fan-out: `apps/web/src/usb/usb_proxy_ring_dispatcher.ts`
- Integration tests: `apps/web/src/usb/usb_proxy_ring_integration.test.ts`

If the emulator uses WASM threads / SharedArrayBuffer (preferred), use the existing
cross-thread IPC mechanism described in [`../areas/web-host.md`](../areas/web-host.md)
and [Cross-origin isolation](../decisions/0002-cross-origin-isolation.md).

---

### Security and compatibility notes

- **Cross-origin isolation:** not required by WebUSB itself, but required for Aero’s
  threaded build (`SharedArrayBuffer`, WASM threads). See:
  - [Browser APIs: deployment headers](../areas/web-host.md#deployment-headers)
  - [the cross-origin isolation decision: Cross-Origin Isolation](../decisions/0002-cross-origin-isolation.md)
- **Browser support:** WebUSB is effectively **Chromium-only** (Chrome/Edge). Expect no
  support in Firefox/Safari; passthrough must be optional and feature-detected.
- **Secure context:** WebUSB requires HTTPS (or `http://localhost`).

#### Protected interface classes (Chromium WebUSB restrictions)

Chrome blocks WebUSB access to certain “protected” USB interface classes (to avoid
interfering with system devices/drivers).

Aero maintains a best-effort list for diagnostics and UX:

- `apps/web/src/platform/webusb_protection.ts` (`PROTECTED_USB_INTERFACE_CLASSES`)
- `apps/web/src/platform/webusb.ts` (`WEBUSB_PROTECTED_INTERFACE_CLASSES`)

These lists may differ slightly by Chromium version. When in doubt, verify on the target
browser via `chrome://usb-internals` and keep the repo’s classifier in sync.

For the canonical documentation of Chromium’s protected interface classes (and Aero’s
compatibility guidance), see [`../areas/usb-and-input.md`](../areas/usb-and-input.md#1-chromium-protected-interface-classes).

---

### Testing plan

#### Web UI smoke panel (manual)

Use the Web UI:

- WebUSB in-app diagnostics panel: `apps/web/src/usb/webusb_panel.ts` (rendered from `apps/web/src/main.ts`)
- WebUSB standalone diagnostics page: `/webusb_diagnostics.html` (`apps/web/src/webusb_diagnostics.ts`)
- WebUSB passthrough broker panel: `apps/web/src/usb/usb_broker_panel.ts` (rendered from `apps/web/src/main.ts`)
- WebUSB passthrough demo panel (IO worker): `apps/web/src/main.ts` (`renderWebUsbPassthroughDemoWorkerPanel`)
- WebUSB UHCI harness panel (IO worker): `apps/web/src/main.ts` (`renderWebUsbUhciHarnessWorkerPanel`)
- WebUSB EHCI harness panel (IO worker): `apps/web/src/main.ts` (`renderWebUsbEhciHarnessWorkerPanel`)

These panels cover: device selection (`requestDevice`), open/claim failures, protected interface
behavior, and basic `GET_DESCRIPTOR` control transfer smoke tests (including via `usb.demo.run`).
For configuration descriptors, the demo panel can optionally rerun the transfer using the
descriptor’s `wTotalLength` field to request the full blob.

#### Rust/TypeScript unit tests (automated)

The Rust passthrough device model has unit tests in `crates/aero-usb/src/passthrough.rs` that validate:

- control-IN/OUT emits exactly one `UsbHostAction` and returns NAK while in flight
- bulk IN/OUT emits one action per endpoint while in flight (no duplicates) and completes on
  injected `UsbHostCompletion`

That file also includes a serde round-trip test to ensure the canonical wire fixture
(`protocol-vectors/webusb_passthrough_wire.json`) matches the `UsbHostAction`/`UsbHostCompletion` shapes.

The TypeScript side has unit tests for the WebUSB executor/broker under `apps/web/src/usb/*test.ts`.

---

### Related docs

- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — overall physical device passthrough model (WebHID MVP + experimental WebUSB passthrough)
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — WebUSB troubleshooting (permissions, WinUSB, udev, etc.)

## WebHID/WebUSB passthrough (physical device access)

This document describes the intended architecture and security model for
passing a **real, host-connected device** through a browser page into the guest
VM as a USB peripheral.

> Source of truth: [Canonical USB stack](../decisions/0015-canonical-usb-stack.md) defines the canonical USB stack
> selection for the browser runtime (`aero-usb` + `aero-wasm` + `apps/web/`).

The goal is to support “real hardware” use cases (game controllers, specialty
HID devices, etc) without turning Aero into a native app.

### Scope: WebHID vs WebUSB

#### WebHID (MVP)

WebHID is a browser API for **HID-class** devices (Human Interface Devices).
It exposes:

- a device identity (vendor/product IDs, names)
- HID report metadata (collections, report IDs/sizes)
- the ability to receive **input reports** and send **output/feature reports**

Why WebHID is the MVP for passthrough:

- **Narrower emulation surface:** we only need a generic USB + HID device model,
  not arbitrary USB class drivers and endpoint types.
- **Good UX fit:** the browser’s chooser UI is already oriented around “input-ish”
  peripherals.
- **Safer default:** compared to raw USB access, HID is more constrained (though
  still security-sensitive; see below).

#### WebUSB (experimental; general USB devices)

WebUSB is a general USB API that can expose non-HID devices and arbitrary USB
transfers (control/bulk/interrupt, multiple interfaces, etc).

Aero includes an end-to-end **guest-visible WebUSB passthrough** stack, using the same “main thread owns
the `USBDevice`, worker owns the guest-visible USB controller bridge + device model” split as WebHID:

- **WASM UHCI controller with WebUSB passthrough device:** `crates/aero-wasm::UhciControllerBridge`
  (`crates/aero-wasm/src/uhci_controller_bridge.rs`, re-exported from `crates/aero-wasm/src/lib.rs`)
  - the passthrough device is attached on **UHCI root port 1** (root port 0 is typically used for the
    WebHID external hub).
  - EHCI/xHCI bridges follow the same convention (reserve root port 1 for WebUSB) so WebUSB passthrough
    can coexist with the external hub / WebHID / synthetic HID topology in high-speed-controller-only
    WASM builds.
- **Worker-side proxy/runtime:** `apps/web/src/usb/webusb_passthrough_runtime.ts` (`WebUsbPassthroughRuntime`)
  - proxies `UsbHostAction`/`UsbHostCompletion` traffic to the main thread broker
  - supports an optional SharedArrayBuffer ring fast path negotiated by `usb.ringAttach` when
    `crossOriginIsolated` (and can be disabled via `usb.ringDetach` on ring corruption, falling back
    to typed `postMessage` forwarding)
- **Main-thread broker/executor:** `apps/web/src/usb/usb_broker.ts` (`UsbBroker`) +
  `apps/web/src/usb/webusb_backend.ts` (`WebUsbBackend`)

Dev/harness note: the repo also contains a standalone WebUSB UHCI bridge (`crates/aero-wasm::WebUsbUhciBridge`)
and a matching PCI device wrapper (`apps/web/src/io/devices/uhci_webusb.ts`) used by tests/panels, but the
production guest-visible passthrough path is via `UhciControllerBridge`.

Compared to WebHID, WebUSB passthrough has a larger surface area and is more sensitive to device/OS/browser
quirks (interface claiming, protected interface classes, full-speed UHCI constraints). WebHID remains the
recommended path for HID peripherals.

Note: the default passthrough controller path is **UHCI** (full-speed). The web runtime can also
route guest-visible WebUSB passthrough via **EHCI/xHCI** (high-speed view) when the deployed WASM
build exports the required passthrough hooks on those controller bridges.

For devices that are high-speed-only (or that behave poorly when forced into a UHCI/full-speed
view), prefer EHCI/xHCI; see:

- [`../areas/usb-and-input.md`](../areas/usb-and-input.md#speed-and-descriptor-handling-uhci-vs-ehcixhci)
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) / [`../areas/usb-and-input.md`](../areas/usb-and-input.md)

The repo also includes a small end-to-end demo driver (`UsbPassthroughDemo` + `usb.demoResult`) that queues
GET_DESCRIPTOR requests via the broker to validate the action↔completion wiring (rerun via `usb.demo.run`
in the Web UI) (`crates/aero-wasm/src/lib.rs`, `apps/web/src/usb/usb_passthrough_demo_runtime.ts`, `apps/web/src/main.ts`).

For the detailed UHCI ↔ WebUSB transfer/TD mapping (including TD-level NAK pending
semantics), see:

- [`../areas/usb-and-input.md`](../areas/usb-and-input.md)

Note: WebUSB is also a poor fit for many HID-class devices because browsers
treat some USB interface classes as “protected” and disallow access via WebUSB.
For HID peripherals, prefer WebHID.

### High-level architecture

The key constraint is that **browser device handles are main-thread objects**.

- `HIDDevice` is not structured-cloneable and cannot be transferred to a worker.
- `USBDevice` transfer/worker support is browser-dependent, but Aero’s baseline
  architecture assumes the handle stays on the main thread (user-activation and
  permission UX are tied to the Window event loop).

So the design is split:

- **Main thread (Window):** selects the physical device, opens it, and forwards
  reports/transfer requests across a host ↔ worker boundary.
- **Worker (I/O / device-model):** emulates guest-visible USB controller(s) (UHCI by default;
  EHCI/xHCI when available) + guest-visible USB device models
  (WebHID-backed HID devices and/or the WebUSB passthrough device wrapper) and exposes
  them to the guest OS like any other USB peripherals.

Data flow (WebHID):

```
Physical HID device
  ↕ (WebHID API: HIDDevice)
Main thread (owns the handle)
  ↕ (report forwarding)
I/O worker (USB controller + device model)
  ↕ (USB transfers)
Guest Windows USB/HID stack
```

### Current status

The repo has the core building blocks for passthrough, and the “main thread owns the device handle,
I/O worker owns the USB device model” split is wired end-to-end in the web runtime for:

- **WebHID:** main↔worker report forwarding (`hid.*`)
- **WebUSB:** main↔worker host action/completion forwarding (`usb.*`) for guest-visible passthrough (UHCI by default; EHCI/xHCI when available)

> Runtime note: the current end-to-end passthrough wiring targets the **legacy** browser runtime
> (`vmRuntime=legacy`), where the guest-visible USB controller/device models live in the I/O worker.
> In `vmRuntime=machine`, guest USB device models live inside the canonical `api.Machine` owned by the
> machine CPU worker and the I/O worker runs in host-only stub mode, so WebHID/WebUSB passthrough is
> not yet available.

Already implemented:

- **Rust device models (`aero-usb`)**
  - `UsbHidPassthrough` (generic USB HID device with bounded input/output report queues)
  - WebHID metadata → HID report descriptor synthesis (`aero_usb::hid::webhid`)
  - `UsbPassthroughDevice` (WebUSB host action/completion queue)
- **WASM exports (`aero-wasm`)**
  - `WebHidPassthroughBridge` (wraps `UsbHidPassthrough` for JS/WASM interop)
  - `UsbPassthroughBridge` (wraps `UsbPassthroughDevice` for WebUSB host action/completion RPC)
  - `UhciControllerBridge` (guest-visible UHCI controller; also exposes the WebUSB passthrough device on root port 1)
  - `EhciControllerBridge` / `XhciControllerBridge` (guest-visible high-speed controllers; optionally expose the WebUSB passthrough device via a reserved root port when the WASM build includes the passthrough hooks)
- **Main-thread WebHID UX / bookkeeping (TypeScript)**
  - `WebHidPassthroughManager` + the debug panel UI
- **Main-thread ↔ I/O worker WebHID broker (TypeScript)**
  - `WebHidBroker` (`apps/web/src/hid/webhid_broker.ts`) + protocol (`apps/web/src/hid/hid_proxy_protocol.ts`)
    forward report traffic:
    - Preferred fast path (when `crossOriginIsolated`): SharedArrayBuffer ring buffers:
      - `hid.ring.init` (IPC `RingBuffer`) for high-frequency **input reports** (main → worker)
      - `hid.ringAttach` (`HidReportRing`) for **output/feature reports** (worker → main; can be
        disabled via `hid.ringDetach` on ring corruption)
      (see [Forwarding mechanism](#forwarding-mechanism)).
    - Fallback/legacy path: `postMessage` forwarding (`hid.inputReport` / `hid.sendReport` /
      `hid.getFeatureReport` / `hid.featureReportResult`).
- **Worker-side WASM bridge (TypeScript)**
  - `apps/web/src/workers/io.worker.ts` creates a WASM `WebHidPassthroughBridge` per attached device and
    drains output reports back to the broker.
- **Main-thread ↔ I/O worker WebUSB broker (TypeScript)**
  - `UsbBroker` (`apps/web/src/usb/usb_broker.ts`) + protocol (`apps/web/src/usb/usb_proxy_protocol.ts`)
    forward `UsbHostAction`/`UsbHostCompletion` traffic:
    - Preferred fast path (when `crossOriginIsolated`): SharedArrayBuffer rings negotiated by
      `usb.ringAttach` (`apps/web/src/usb/usb_proxy_ring.ts`)
    - Fallback path: typed `postMessage` (`usb.action` / `usb.completion`)
- **Worker-side WebUSB passthrough runtime (TypeScript)**
  - `WebUsbPassthroughRuntime` (`apps/web/src/usb/webusb_passthrough_runtime.ts`) drains actions from
    the guest-visible WebUSB passthrough device (via the selected guest USB controller bridge) and applies completions.
- **Guest-visible USB controllers + topology wiring (TypeScript + WASM)**
  - `apps/web/src/io/devices/uhci.ts` / `apps/web/src/io/devices/ehci.ts` / `apps/web/src/io/devices/xhci.ts` expose
    guest-visible UHCI/EHCI/xHCI PCI functions backed by the corresponding WASM controller bridges
    (`UhciControllerBridge` / `EhciControllerBridge` / `XhciControllerBridge`).
  - `apps/web/src/hid/uhci_hid_topology.ts` and `apps/web/src/hid/xhci_hid_topology.ts` wire WebHID passthrough bridges
    into the guest USB topology (including attaching an external hub when a `guestPath` requires it).
  - The I/O worker prefers routing WebHID passthrough attachments to xHCI when the xHCI topology exports are
    available. Otherwise it falls back to the UHCI topology manager. In WASM builds that omit UHCI, the worker
    can reuse the UHCI topology manager with `EhciControllerBridge` since EHCI exposes a compatible attachment
    API (see `apps/web/src/workers/io.worker.ts`, `apps/web/src/hid/ehci_hid_topology_shim.ts`, and
    `apps/web/src/workers/io_hid_topology_mux.ts`).

Dev-only scaffolding (useful for tests / manual bring-up, but **not** the target architecture):

- `WebHidPassthroughRuntime` runs on the **main thread** and directly wires `HIDDevice` events into
  a WASM `WebHidPassthroughBridge` instance (bypasses the broker/worker split).

Still missing / in progress (guest-visible USB integration):

- Full VM snapshot/restore integration for passthrough device state (queued reports, USB configuration
  state, etc). The underlying device models in `crates/aero-usb` implement deterministic `aero-io-snapshot`
  TLV save/restore via `IoSnapshot`, and the WASM USB entrypoints now expose deterministic snapshot bytes:
  - `UhciControllerBridge.snapshot_state()/restore_state()`
  - `WebUsbUhciBridge.snapshot_state()/restore_state()`
  - `UhciRuntime.snapshot_state()/restore_state()` (recommended for self-contained WebHID/WebUSB topology)

  Higher-level VM snapshot wiring (coordinator/device entry plumbing, guest RAM capture, and browser-side
  orchestration) is still pending.

### Host-side model (main thread owns the device)

#### Why the main thread owns it

- `navigator.hid.requestDevice(...)` / `navigator.usb.requestDevice(...)` must
  be called from a **user gesture** on the main thread.
- `HIDDevice` is **not structured-cloneable** and therefore cannot be sent to a
  Worker via `postMessage`.
- `USBDevice` structured clone / worker access is not reliable enough to be a
  design assumption; treat the main thread as the default owner and proxy I/O as
  needed (see [`../areas/usb-and-input.md`](../areas/usb-and-input.md)).

#### Responsibilities

The main-thread “passthrough manager” is responsible for:

In the current TypeScript runtime this role is split between:

- `WebHidPassthroughManager` (`apps/web/src/platform/webhid_passthrough.ts`) for user-driven selection,
  open/close lifecycle, and guest-path allocation bookkeeping.
- `WebHidBroker` (`apps/web/src/hid/webhid_broker.ts`) for attaching to the I/O worker port and proxying
  input/output report traffic.
- `UsbBroker` (`apps/web/src/usb/usb_broker.ts`) for WebUSB device selection + open/claim, and proxying
  `UsbHostAction`/`UsbHostCompletion` traffic to the I/O worker.

1. **User-initiated selection**
   - Trigger a chooser from an explicit UI action (“Connect device…”).
2. **Open/close lifecycle**
   - `await device.open()` when attaching to a VM.
   - `await device.close()` when detaching or when the VM stops.
3. **Input report forwarding**
   - Listen for `inputreport` events.
   - Forward `(reportId, data bytes, timestamp)` to the worker.
4. **Output report execution**
   - Receive worker requests to send an output/feature report.
   - Call `device.sendReport(...)` / `device.sendFeatureReport(...)`.
5. **Feature report read execution** (`GET_REPORT Feature`)
   - Receive worker requests to read a feature report.
   - Call `device.receiveFeatureReport(...)` and return the bytes back to the worker asynchronously.
6. **WebUSB transfer execution** (if using WebUSB passthrough)
   - Receive `UsbHostAction` requests from the worker (control/bulk transfers).
   - Execute the corresponding WebUSB call and return a `UsbHostCompletion`.

#### Forwarding mechanism

The WebHID handle is main-thread-only, so input/output report traffic is forwarded across the
main-thread ↔ worker boundary using runtime-selected mechanisms. The implementation prefers
SharedArrayBuffer ring buffers when available, and otherwise falls back to `postMessage`.

##### Default / legacy path: `postMessage` + transferred `ArrayBuffer`s

When SharedArrayBuffer is unavailable, forwarding uses `postMessage` with typed payloads and
transfers the underlying `ArrayBuffer` for report bytes (so the common case is zero-copy), e.g.
`{ type: "hid.inputReport", deviceId, reportId, data: Uint8Array }`.

Protocol schema + validators:

- `apps/web/src/hid/hid_proxy_protocol.ts` (`hid.inputReport`, `hid.sendReport`, `hid.getFeatureReport`, `hid.featureReportResult`)

##### Fast path: SharedArrayBuffer ring buffers

When `globalThis.crossOriginIsolated === true` (COOP/COEP enabled) and `SharedArrayBuffer`/`Atomics`
are available, the runtime uses SAB-backed rings to avoid per-report `postMessage` overhead on
high-frequency devices.

There are currently **two** ring types involved:

###### Input reports (main thread → worker): IPC `RingBuffer` (`hid.ring.init`)

For high-frequency WebHID `inputreport` events, the main thread initializes an IPC-style
`RingBuffer` and sends it to the I/O worker via:

`{ type: "hid.ring.init", sab: SharedArrayBuffer, offsetBytes }`

Input reports are encoded as compact, versioned binary records (magic + version + header + bytes)
and pushed into the ring with `tryPushWithWriter(...)`. This path is best-effort: when the ring is
full, reports are dropped and a drop counter is incremented.

If the worker fails to decode a ring record (invalid magic/version/length), the record is treated
as dropped (incrementing the invalid/drop counters). This ring is a best-effort performance
optimization: it is not required for correctness, and when SAB rings are unavailable the runtime
continues using the `postMessage` forwarding path.

Implementation pointers:

- Ring buffer: `apps/web/src/ipc/ring_buffer.ts` (`RingBuffer`)
- Record codec: `apps/web/src/hid/hid_input_report_ring.ts` (magic `"HIDR"`, versioned header)
- Message schema: `apps/web/src/hid/hid_proxy_protocol.ts` (`hid.ring.init`)
- Main thread producer: `apps/web/src/hid/webhid_broker.ts` (`#maybeInitInputReportRing`, `inputreport` listener)
- Worker drain loop: `apps/web/src/workers/io_hid_input_ring.ts` (`drainIoHidInputRing`)

###### Output/feature reports (worker → main thread): HID `HidReportRing` (`hid.ringAttach`)

When `globalThis.crossOriginIsolated === true` (COOP/COEP enabled) and `SharedArrayBuffer`/`Atomics`
are available, `WebHidBroker` allocates two SharedArrayBuffers and sends them to the worker via
`{ type: "hid.ringAttach", inputRing, outputRing }`:

- **`inputRing` (main thread → worker):**
  - Legacy/compatibility SAB ring for input reports. Newer runtimes prefer `hid.ring.init` above.
- **`outputRing` (worker → main thread):**
  - the I/O worker writes output/feature report requests into the ring as
    `(deviceId, reportType, reportId, bytes)`.
  - the main thread periodically drains the ring and executes the corresponding WebHID call
    (`device.sendReport(...)` / `device.sendFeatureReport(...)`).

The ring implementation is a bounded, single-producer/single-consumer, variable-length record ring
buffer with an Atomics-managed control header; it is designed to avoid per-report allocations and
reduce `postMessage` overhead on high-frequency devices.

Semantics / guarantees:

- **In-order execution per device (correctness):** output/feature reports are executed **in-order per
  deviceId**. Even though WebHID `sendReport`/`sendFeatureReport` are `Promise`-based, the broker
  serializes calls per device so later reports are not started until earlier ones have settled.
  (Reports from different devices may interleave.)
- **Unified FIFO across transports:** both the SAB output ring (`hid.ringAttach`) and the fallback
  `postMessage` path (`hid.sendReport`) enqueue into the same per-device FIFO, so reports do not run
  concurrently even if the runtime temporarily mixes forwarding mechanisms.
- **Ordering when mixing ring + `postMessage`:** because the output ring is normally drained on a
  timer, a `postMessage` fallback could otherwise overtake earlier ring records (or vice-versa, a
  later ring write could become visible in SharedArrayBuffer memory before an earlier `postMessage`
  is delivered). To preserve guest order:
  - the worker includes an `outputRingTail` snapshot on `hid.sendReport` / `hid.getFeatureReport`
    fallbacks when an output ring is present
  - the broker drains the output ring only up to that snapshot *before* enqueuing the message into
    the per-device FIFO
  - if the consumer `head` has already advanced past the snapshot (meaning the periodic drain ran
    before the message was delivered), the broker does not drain any additional ring records while
    handling that message (avoiding making ordering inversions worse)
- **Bounded send queue:** WebHID report I/O is serialized through a bounded per-device queue. If the
  guest produces reports faster than the host can execute them (or a `sendReport()` Promise never
  resolves), the broker drops new output/feature sends once the cap is reached (drop newest, keep
  already-queued FIFO order). Feature report reads may return an immediate
  `hid.featureReportResult { ok: false }` when the per-device queue is saturated.
- **Fallback on ring overflow/corruption:** the SAB rings are an optimization, not a correctness
  requirement. If a report cannot be enqueued (ring full, record too large), the I/O worker falls
  back to `postMessage` (`hid.sendReport`) for that report so guest→device reports are not silently
  lost.
  If either side detects ring corruption (e.g. the consumer cannot decode/drain records), it sends
  `{ type: "hid.ringDetach", reason? }` to disable the SharedArrayBuffer fast paths and fall back to
  `postMessage` (`hid.inputReport` / `hid.sendReport`). In the current runtime, `hid.ringDetach`
  disables both the `hid.ringAttach` rings and the `hid.ring.init` input report ring. Any reports
  already queued in a detached ring may be dropped/abandoned.
- **Size limits:** the SAB `HidReportRing` record format stores `len` as a `u16` and the default
  output ring capacity is 1 MiB (configurable via `WebHidBroker({ outputRingCapacityBytes })`).
  Feature reports larger than the maximum ring record payload (≈64 KiB, due to the `u16` length
  field) are forwarded via `postMessage` even when rings are enabled.

Implementation pointers:

- Ring buffer: `apps/web/src/usb/hid_report_ring.ts` (`HidReportRing`)
- Message schema: `apps/web/src/hid/hid_proxy_protocol.ts` (`hid.ringAttach`, `hid.ringDetach`)
- Main thread setup + drain: `apps/web/src/hid/webhid_broker.ts` (`#attachRings`, `#drainOutputRing`)
- Worker-side attach: `apps/web/src/workers/io.worker.ts` (`attachHidRings`, `hidHostSink.sendReport`)

Note: `SharedArrayBuffer` requires cross-origin isolation (COOP/COEP) in modern browsers. When the
page is not `crossOriginIsolated`, the runtime automatically falls back to the `postMessage` path.
See [`../areas/web-host.md`](../areas/web-host.md).

When rings were previously attached but later become unavailable (e.g., worker restart/reattach),
the runtime should treat this as a performance downgrade only and continue operating via the
`postMessage` path.

### Guest-side model (guest USB controller + generic HID passthrough device)

#### Emulated topology

The guest-visible device is modeled as:

- A guest-visible USB host controller:
  - The I/O worker prefers routing WebHID passthrough attachments to **xHCI** when the deployed WASM build
    exports the xHCI topology APIs.
  - Otherwise it falls back to the **UHCI topology manager** (typically backed by UHCI). In WASM builds that
    omit UHCI, the worker can instead back this topology manager with **EHCI** since `EhciControllerBridge`
    exposes a compatible attachment API (see `apps/web/src/workers/io.worker.ts`,
    `apps/web/src/hid/ehci_hid_topology_shim.ts`, and `apps/web/src/workers/io_hid_topology_mux.ts`).
- A **root hub** with one or more root ports (controller-dependent).
- (optional) **external USB hub device** (USB class `0x09`) attached behind a root port to
  provide additional downstream ports
- **one generic HID device per physical passthrough device**

On attach, the worker hot-plugs the device onto an available guest USB attachment path (root port or
downstream hub port), which triggers the guest USB stack to enumerate it.

#### Device identity and descriptors

The passthrough HID device should expose stable USB descriptors derived from the
WebHID device metadata:

- `idVendor` / `idProduct`: from WebHID vendor/product IDs
- strings: best-effort from `productName` / `manufacturerName` if available

WebHID does **not** expose the raw HID report descriptor byte stream. It exposes
a structured view (`HIDDevice.collections`, reports, and report items), so we
synthesize a semantically equivalent HID report descriptor from that metadata.
See [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
for the exact synthesis contract.

The USB device model must still provide the normal USB descriptors used during
enumeration:

- device descriptor
- configuration/interface/endpoint descriptors
- HID descriptor

#### Report queues (bridge between WebHID and USB polling)

WebHID is event-driven, but the guest’s USB HID stack is poll/transfer-driven.
To connect them, the worker maintains per-device queues:

##### Input reports (device → guest)

- Main thread pushes `(reportId, bytes)` into the device’s **input report queue**.
- When the guest performs an interrupt IN transfer:
  - if a report is queued: return it as the transfer payload
  - if the queue is empty: NAK (guest will poll again)

Note: Aero currently caps passthrough HID **input reports** to fit in a single interrupt IN packet
(**64 bytes**, including the report ID prefix when report IDs are in use). We currently do not
support splitting a single HID input report across multiple interrupt transactions, so oversized
input reports are rejected at attach/descriptor-synthesis time. (Output reports may be delivered
via HID-class `SET_REPORT` control transfers and/or an interrupt OUT endpoint (device-dependent);
output/feature reports are normalized and capped separately. Very large feature report payloads
may fall back to `postMessage` on the host boundary when the SAB output ring is enabled.)

##### Output/feature reports (guest → device)

- When the guest sends a `SET_REPORT` (control transfer) or an interrupt OUT
  transfer (device-dependent):
  - worker enqueues `(reportId, bytes, kind)` into the **output report queue**
  - main thread drains the queue and calls the appropriate WebHID send method

This queue boundary is also where we can implement:

- backpressure / bounded memory
- ordering guarantees (preserve report order) — **output/feature reports are executed in-order per
  device**
- VM snapshot/restore (queue contents are part of device state)

##### Feature report reads (`GET_REPORT` Feature)

The passthrough path supports reading feature reports from the host. The guest-visible USB HID
device proxies a HID-class `GET_REPORT` request with report type **Feature** to the physical device
using WebHID `receiveFeatureReport(reportId)`.

Because WebHID is async, the USB control transfer cannot complete immediately. The device model
represents “waiting for the host” by returning **NAK** until the host Promise settles; once the host
responds, the next retry completes with the returned report bytes. Host failures are surfaced as a
retryable transfer error (controller-specific) so the guest can retry.

Host boundary protocol:

- Worker → main: `hid.getFeatureReport` `(requestId, deviceId, reportId)`
- Main → worker: `hid.featureReportResult` `(requestId, ok, data?)`

Feature report reads are also serialized per device: the main thread processes `receiveFeatureReport`
requests in the same per-device FIFO as output/feature report writes so `GET_REPORT` cannot overtake
earlier queued `SET_REPORT`/output writes for that device.

Length normalization / safety:

- The main thread normalizes the returned payload length to the descriptor-derived feature report
  length (truncating or zero-padding when the browser returns a different length).
- If the expected feature report length is unknown, the result is capped (currently 4096 bytes) to
  avoid unbounded allocations from a bogus device/browser.

Caching / coalescing behaviour (device model):

- Successful `GET_REPORT Feature` results are **cached per reportId**. After the host responds, later
  `GET_REPORT Feature` requests for the same reportId are served from the cached bytes without
  issuing another host read until the cache is invalidated.
- The cache entry for a reportId is invalidated when the guest sends `SET_REPORT Feature` for that
  reportId (and on device reset).
- At most **one in-flight** feature report request is tracked per reportId. While a request is
  pending, subsequent guest polls NAK until the host responds (no duplicate host reads are issued).

Snapshot/restore note:

- Feature report reads are backed by async host operations (Promises). After restoring a VM snapshot,
  any in-flight host-side feature report bookkeeping must be discarded (the guest will retry the
  control transfer and the device model will re-emit a fresh host request).

### Security and UX constraints

Passing through a physical device is **powerful and risky**. The UX must make
the security boundary explicit: you are giving an **untrusted guest OS** direct
access to a real device.

#### User gesture requirement (`requestDevice`)

Both APIs require a user gesture:

- WebHID: `navigator.hid.requestDevice(...)`
- WebUSB: `navigator.usb.requestDevice(...)`

Do not attempt to call these APIs automatically on page load or in response to
background events; it will fail and is also poor UX.

In practice:

- Call `requestDevice()` directly from the gesture handler; if you `await` before
  calling it, the user activation can be lost.
- User activation does not propagate across `postMessage()`, so a “click →
  postMessage → worker calls `requestDevice()`” flow will fail.

#### Secure context requirement

WebHID/WebUSB require a **secure context** (`https://` or `http://localhost`).
Passthrough should be disabled (with a clear UI error) when `isSecureContext`
is false.

#### Origin-scoped permission persistence and revocation

Permissions are **scoped to the web origin** and may persist across reloads.
Typical flow:

- The first time, the user grants access via the chooser UI.
- On later visits, the page can often rediscover devices via
  `navigator.hid.getDevices()` / `navigator.usb.getDevices()` without showing
  the chooser.

Security model requirement for Aero:

- Even if the origin has permission, **do not auto-attach** the device to a VM
  without an explicit user action in the UI (e.g. a “Connect” button).

Revocation:

- There is no **portable** JS API to revoke permission across browsers.
- Some Chromium builds expose `HIDDevice.forget()` / `USBDevice.forget()` which can
  revoke the permission in-app when available.
- Otherwise, the user must revoke via browser UI (site settings / device permissions).
  The app should provide a help link/instructions in its settings panel and keep
  the fallback UX available even when `forget()` is supported.

#### Explicit warnings and safer defaults

When offering passthrough, the UI should warn that the guest can:

- read inputs from the device (potentially sensitive)
- send outputs back to the device (e.g. LEDs, vibration, device state changes)

Recommended guardrails:

- Require the user to opt in per session (“Attach to this VM”).
- Show a persistent “Device connected to VM” indicator and a one-click
  “Disconnect” action.
- Prefer allowlisting device types that make sense (game controllers, specialty
  hardware) and avoid exposing high-risk devices by default.

### Current limitations (MVP constraints)

- **Guest USB root hub port count varies by controller**
  - UHCI exposes **2** root ports; EHCI/xHCI expose more root ports by default.
  - The browser runtime uses a consistent topology convention across controllers:
    - root port **0** hosts an emulated external USB hub (synthetic HID devices + WebHID passthrough)
    - root port **1** is reserved for the guest-visible WebUSB passthrough device
  - The browser/WASM USB stack includes an external USB hub device model (`UsbHubDevice`, USB class
    `0x09`) that can be attached behind a root port to expose additional downstream ports.
    - Implementation: `crates/aero-usb/src/hub.rs`
    - UHCI integration tests: `crates/aero-usb/tests/uhci_external_hub.rs`
  - Current host-side WebHID UI assumes an external hub is attached on guest root port 0. The first few
    downstream hub ports (1..4) are reserved for Aero's synthetic HID devices (keyboard/mouse/gamepad/
    consumer-control), so WebHID passthrough allocations start at guest paths like `0.5`.
    - Guest root port 1 is reserved for the guest-visible WebUSB passthrough device, so WebHID attachments
      do not use path `1`. Increase the external hub port count instead if you need more guest
      attachment paths (note: clamped to 15 when the hub is hosted behind xHCI).
    - When the external hub is hosted behind xHCI, hub port numbers are limited to **1..=15** (xHCI Slot
      Context Route String 4-bit hub-port encoding), so the external hub port count is effectively capped
      at 15 and “0.5+” means hub ports 5..=15 (guest paths like `0.5`, `0.6`, …, `0.15`).
    - Implementation: `apps/web/src/platform/webhid_passthrough.ts` (guest path allocator + UI hint)
- **No low-speed modeling**
  - Low-speed (1.5 Mbps) USB devices are not modeled correctly yet.
  - Expect some HID peripherals to fail enumeration or behave incorrectly.
- **Guest-visible WebUSB passthrough is experimental**
  - The canonical UHCI controller (`UhciControllerBridge`) exposes a guest-visible WebUSB passthrough
    device on root port 1.
  - When available in a given WASM build, the guest-visible high-speed controllers
    (`EhciControllerBridge` / `XhciControllerBridge`) expose the same passthrough device lifecycle via
    a reserved root port too:
    - EHCI: root port 1
    - xHCI: typically root port 1 (falls back to root port 0 if the controller only exposes a single
      root port)
  - The I/O worker runs `WebUsbPassthroughRuntime` to proxy host actions/completions between the
    WASM device model and the main-thread WebUSB broker (`UsbBroker`).
    - Optional SharedArrayBuffer ring fast path (`usb.ringAttach`) when `crossOriginIsolated`,
      falling back to typed `postMessage` otherwise.
  - WebUSB cannot access many common USB classes in Chromium (protected interface classes), so it is
    not a replacement for WebHID for HID peripherals.
  - See:
    - [`../areas/usb-and-input.md`](../areas/usb-and-input.md) (UHCI TD ↔ WebUSB transfer mapping)
    - [`../areas/usb-and-input.md`](../areas/usb-and-input.md) (Chromium limits, WinUSB/udev, troubleshooting)

### Testing strategy

#### Browser-side (TypeScript)

- Tests should use a mocked `navigator.hid` + fake `HIDDevice` objects to cover:
  - attach/detach lifecycle (`open()`/`close()`) and disconnect handling
  - report forwarding semantics and output/feature report execution
- Implementation references:
  - `apps/web/src/platform/webhid_passthrough.test.ts` (manager + debug UI)
  - `apps/web/src/hid/webhid_broker.test.ts` (main↔worker report forwarding)
  - `apps/web/src/hid/hid_proxy_protocol.test.ts` (message schema validators)
  - `apps/web/src/usb/webhid_passthrough_runtime.test.ts` (dev-only main-thread runtime wiring)

#### Device model (Rust)

- Unit tests for the passthrough HID device model:
  - descriptor generation (stable and spec-compliant enough for Win7)
  - input/output queue behavior (ordering, boundedness, snapshotability)
  - translation between WebHID report IDs and guest-visible USB transfers
- Implementation references:
  - `crates/aero-usb/src/hid/passthrough.rs`
  - `crates/aero-usb/src/hid/webhid.rs`
  - `crates/aero-usb/tests/webhid_passthrough.rs`

### Implementation references (current code)

- **WASM exports (browser build)**
  - `crates/aero-wasm/src/lib.rs`
    - `WebHidPassthroughBridge`
    - `UsbPassthroughBridge`
    - `UhciControllerBridge` (guest-visible UHCI controller; also exposes the WebUSB passthrough device lifecycle)
    - `EhciControllerBridge` (guest-visible EHCI controller; optionally exposes the WebUSB passthrough device lifecycle)
    - `XhciControllerBridge` (guest-visible xHCI controller; optionally exposes WebHID topology helpers + the WebUSB passthrough device lifecycle)
    - `WebUsbUhciBridge` (standalone UHCI + WebUSB passthrough bridge used by harness/tests)
- **Rust device models**
  - WebHID → HID report descriptor synthesis: `crates/aero-usb/src/hid/webhid.rs`
  - Generic USB HID passthrough device model: `crates/aero-usb/src/hid/passthrough.rs`
  - WebUSB host action/completion queue: `crates/aero-usb/src/passthrough.rs` (`UsbPassthroughDevice`)
  - UHCI-visible WebUSB passthrough device wrapper: `crates/aero-usb/src/passthrough_device.rs` (`UsbWebUsbPassthroughDevice`)
- **Host-side (TypeScript)**
  - WebHID attach/detach + debug UI: `apps/web/src/platform/webhid_passthrough.ts`
  - Main↔worker report proxying broker: `apps/web/src/hid/webhid_broker.ts`
  - Main↔worker report proxying protocol: `apps/web/src/hid/hid_proxy_protocol.ts`
  - SharedArrayBuffer report ring: `apps/web/src/usb/hid_report_ring.ts`
  - WebUSB broker/executor: `apps/web/src/usb/usb_broker.ts`, `apps/web/src/usb/webusb_backend.ts`
  - WebUSB proxy protocol + SAB ring fast path: `apps/web/src/usb/usb_proxy_protocol.ts`, `apps/web/src/usb/usb_proxy_ring.ts`, `apps/web/src/usb/usb_proxy_ring_dispatcher.ts`
  - Worker-side WebUSB passthrough runtime: `apps/web/src/usb/webusb_passthrough_runtime.ts`
  - Guest-visible USB PCI devices (worker runtime):
    - UHCI (default): `apps/web/src/io/devices/uhci.ts`
    - EHCI (optional): `apps/web/src/io/devices/ehci.ts`
    - xHCI (optional): `apps/web/src/io/devices/xhci.ts`
  - (Dev/harness) Standalone WebUSB UHCI PCI device: `apps/web/src/io/devices/uhci_webusb.ts`
  - Guest USB attachment path schema (root port index + downstream hub ports): `apps/web/src/platform/hid_passthrough_protocol.ts`
  - WebHID normalization (input to descriptor synthesis): `apps/web/src/hid/webhid_normalize.ts`
  - Dev-only main-thread runtime wiring WebHID ↔ WASM bridge: `apps/web/src/usb/webhid_passthrough_runtime.ts`
  - I/O worker wiring point (guest-visible USB controllers + USB topology + passthrough runtimes):
    `apps/web/src/workers/io.worker.ts`

### Related docs

- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — overall input strategy
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — HID usages and report formats
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — WebHID metadata → HID report descriptor bytes
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — WebUSB constraints and troubleshooting
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — WebUSB async passthrough design (UHCI + host actions/completions)

## WebHID → HID report descriptor synthesis (Windows 7 contract)

### Background / why this exists

When we use the browser’s **WebHID** API to talk to a physical HID device, we do **not** get access to the device’s raw HID **report descriptor bytes**.

Instead, WebHID exposes a structured view of the descriptor:

- `HIDDevice.collections` → a tree of `HIDCollectionInfo`
- Each `HIDCollectionInfo` has `inputReports` / `outputReports` / `featureReports`
- Each `HIDReportInfo` has a `reportId` and a list of `HIDReportItem`s

To present that physical device to the Windows 7 guest as a USB HID device (or to run any code that expects descriptor bytes), we synthesize a **semantically equivalent** HID report descriptor from the WebHID metadata.

This document defines the **contract** for that synthesis:

- how the WebHID data model maps onto HID report descriptor items
- how main-item flags are derived
- the encoding rules we follow (short items, minimal-size payloads, signed encoding)
- validation rules and known limitations (including ordering loss)

> Source of truth: [Canonical USB stack](../decisions/0015-canonical-usb-stack.md) defines the canonical USB
> stack for the browser runtime. This document specifies the HID report descriptor synthesis
> contract used by the browser/runtime USB device models (implemented in `crates/aero-usb`).

Windows 7 note: the output is intentionally “boring HID 1.11” to maximize compatibility with `hidclass.sys` / `hidparse.sys` on Windows 7.

Implementation references:

- Browser normalization: `apps/web/src/hid/webhid_normalize.ts`
- Rust synthesis: `crates/aero-usb/src/hid/report_descriptor.rs`
  - WebHID JSON schema + conversion layer: `crates/aero-usb/src/hid/webhid.rs`
- Wire contract fixtures:
  - Fixture JSON:
    - `tests/fixtures/hid/webhid_normalized_mouse.json`
    - `tests/fixtures/hid/webhid_normalized_keyboard.json`
    - `tests/fixtures/hid/webhid_normalized_gamepad.json`
  - TS contract tests:
    - `apps/web/test/webhid_normalize_fixture.test.ts` (node:test; run via repo-root `pnpm test`)
    - `apps/web/test/webhid_normalize_fixture.vitest.ts` (vitest; quick run via `pnpm -C apps/web test -- webhid_normalize_fixture`)
  - Rust contract tests:
    - JSON schema roundtrip + synthesis smoke: `crates/aero-usb/tests/webhid_passthrough.rs`
    - Descriptor bytes regression: `crates/aero-wasm/tests/webhid_report_descriptor_synthesis.rs`
    - WASM export regression (wasm32 only): `crates/aero-wasm/tests/webhid_report_descriptor_synthesis_wasm.rs`

(`crates/emulator` re-exports the same synthesis code via `emulator::io::usb`, but `crates/aero-usb`
is the implementation source of truth.)

TypeScript/WebHID types:

- WebHID is defined by the WICG spec: https://wicg.github.io/webhid/
- The TypeScript DOM libs do not consistently ship WebHID type definitions across versions, so this
  repo depends on `@types/w3c-web-hid` and pulls it in via `apps/web/src/vite-env.d.ts`.

For the end-to-end “real device” passthrough architecture (main thread owns the
`HIDDevice`, worker models a guest-visible USB controller + a generic HID device), see
[`../areas/usb-and-input.md`](../areas/usb-and-input.md).

Windows 7 compatibility goals (what we optimize for):

- Prefer descriptor forms that Windows 7 parses reliably (`hidparse.sys`), i.e. **short items** and common main-item flag patterns.
- Preserve the **top-level application collection** `Usage Page`/`Usage` because Windows 7 uses it to bind client drivers (`kbdhid.sys`, `mouhid.sys`, `hidgame.sys`, …).
- Avoid uncommon HID tags (strings/designators/long items) unless we find a real device that requires them.

---

### High-level algorithm (deterministic descriptor emission)

We emit a descriptor by walking the WebHID collection tree.

For each `HIDCollectionInfo`:

1. Emit the collection “header” items:
   - `Usage Page` (from `usagePage`)
   - `Usage` (from `usage`)
   - `Collection(type)` (from `collectionType`)
2. Inside the collection, emit the report definitions in a deterministic grouping:
   - all `inputReports`, then all `outputReports`, then all `featureReports`
   - within a given `HIDReportInfo`, preserve the order of `items` (this defines bit/field layout)
3. Recurse into `children` (depth-first), then emit `End Collection`.

Because WebHID does not expose the original descriptor byte stream, this grouping may not match the device’s original interleaving of “report items vs child collections”. See [Known limitations](#known-limitations).

---

### Data model → report descriptor mapping

#### Collections (`HIDCollectionInfo`)

`HIDCollectionInfo.usagePage/usage/collectionType/children` maps to:

```
Usage Page (usagePage)
Usage (usage)
Collection (collectionType)
  …contents…
End Collection
```

Notes:

- `collectionType` is emitted as the 1-byte collection type value used by the HID specification (e.g. `Application`, `Physical`, …).
- We **do not** emit `Push`/`Pop`; each report item is emitted with the global state it needs (see below).

Collection type codes (HID `Collection(...)` payload byte).

Note: Depending on the browser / WebHID typing source, `HIDCollectionInfo.type` may be exposed as
either the string enum (e.g. `"application"`) or the numeric code (`0..=6`). Our normalization layer
accepts both.

| WebHID collection type (`type`) | `collectionType` / `Collection(...)` byte |
| --- | ---: |
| `physical` | `0x00` |
| `application` | `0x01` |
| `logical` | `0x02` |
| `report` | `0x03` |
| `namedArray` | `0x04` |
| `usageSwitch` | `0x05` |
| `usageModifier` | `0x06` |

JSON note:

- Normalized JSON emitted by `webhid_normalize.ts` uses a numeric `collectionType` code (`0..=6`), matching the HID `Collection(...)` payload byte.
- The WebHID `type` field may appear as either:
  - the string enum form (as shown in the table above), or
  - the numeric HID code (`0..=6`) in some typings/spec revisions.
  Both the TypeScript normalizer and Rust deserializer accept either representation.

#### Reports (`HIDReportInfo`)

Each WebHID report group (`inputReports` / `outputReports` / `featureReports`) maps to a sequence of main items inside the current collection.

##### Report ID

`HIDReportInfo.reportId` maps to the global `Report ID` item:

- `reportId == 0`: **omit** `Report ID` entirely (descriptor has no report IDs; report bytes have no leading report-id byte).
- `reportId != 0`: emit `Report ID (reportId)` before the first main item of that report.

See [Validation rules](#validation-rules) for the “mixed 0/non-zero” policy.

#### Report items (`HIDReportItem`)

Each `HIDReportItem` maps to:

1. A set of **global** + **local** items that define the next main item.
2. The **main item** itself:
   - `Input(flags)` for input reports
   - `Output(flags)` for output reports
   - `Feature(flags)` for feature reports

We treat the WebHID `HIDReportInfo` that the item came from as the authoritative “main item kind” (`Input` vs `Output` vs `Feature`).

WebHID exposes the following boolean properties on `HIDReportItem` (as defined by the WICG spec and
exposed by Chromium; note that some WebHID type definitions omit/rename fields, e.g. `isRelative`
may be missing (it is redundant with `isAbsolute`) and `wrap` may be used instead of `isWrapped`):

- `isConstant`
- `isArray`
- `isAbsolute` / `isRelative` (redundant; `isRelative` may be omitted and can be derived as `!isAbsolute`)
- `isWrapped`
- `isLinear`
- `hasPreferredState`
- `hasNull`
- `isVolatile`
- `isBufferedBytes`
- `isRange` (controls whether `usages` is treated as a min/max range)

WebHID also exposes less-common HID locals (`strings`, `designators`) and related min/max fields. These are currently ignored by synthesis (see [Known limitations](#known-limitations)).

#### Usage locals: `isRange` vs `usages`

WebHID surfaces both:

- `item.isRange` + `item.usageMinimum` / `item.usageMaximum`
- `item.usages` (a list)

Normalized JSON note:

- In our **normalized** metadata contract (output of `webhid_normalize.ts`), range items are
  represented compactly: when `item.isRange == true`, `item.usages` is always the canonical
  `[usageMinimum, usageMaximum]` form (even when `min == max`).
  - Therefore, when `item.isRange == true`, callers should treat `item.usages` as a **2-element**
    array (`[min, max]`).
- Expanded `usages` lists are not required and are not guaranteed to be preserved for range items.

Synthesis interpretation:

- If `item.isRange == true`, we treat `item.usages` as the set of usages covered by the range.
  - In WebHID, `item.usages` is expected to be the *expanded list* (inclusive), e.g. keyboard
    modifiers are `E0..E7` → `[0xE0, 0xE1, …, 0xE7]` (not `[0xE0, 0xE7]`).
  - We emit `Usage Minimum` + `Usage Maximum` only if the usages can be represented as a single
    contiguous span. Otherwise we fall back to emitting explicit `Usage` tags.
  - For robustness, if `item.usages` is empty we fall back to `item.usageMinimum` /
    `item.usageMaximum` (and may store the compact `[min, max]` representation internally to avoid
    allocating huge arrays).
- If `item.isRange == false`, we emit one `Usage` per entry in `item.usages` (in order) and ignore
  `item.usageMinimum` / `item.usageMaximum`.
- Empty `item.usages` is allowed (common for constant/padding fields); in that case no usage locals are emitted for the item.

#### Deterministic per-item emission order

Because we are regenerating bytes from metadata (not replaying the original descriptor), we emit a canonical sequence of items for each `HIDReportItem` (matching the canonical encoder in `crates/aero-usb/src/hid/report_descriptor.rs`, which is called by `crates/aero-usb/src/hid/webhid.rs`):

1. Globals (in this order):
   - `Usage Page` (`item.usagePage`)
   - `Logical Minimum` / `Logical Maximum`
   - `Physical Minimum` / `Physical Maximum`
   - `Unit Exponent`
   - `Unit`
   - `Report Size`
   - `Report Count`
2. Usage locals:
   - if `item.isRange`: prefer emitting `Usage Minimum` + `Usage Maximum` using the contiguous span
     described by `item.usages` (or, if `usages` is empty, by `item.usageMinimum`/`item.usageMaximum`)
     - the internal encoder may choose to fall back to an explicit `Usage` list if the usages are not contiguous
   - else: emit one `Usage` item per entry in `item.usages`
3. Main item: `Input` / `Output` / `Feature`

---

### Main item flags

HID main items (`Input`/`Output`/`Feature`) have a bitfield payload.

The synthesis treats these flags as a single `u16` and emits either a 1-byte or 2-byte payload:

- if `flags <= 0xFF`: emit 1 byte
- otherwise: emit 2 bytes (little-endian)

Implementation note: we reuse the canonical synthesizer in `crates/aero-usb/src/hid/report_descriptor.rs`, which preserves the main-item flags exposed by WebHID (including `hasNull` for hat switches) while following the HID 1.11 bit assignments for each main item kind.

#### Bit layout (LSB = bit 0)

Bits are defined by the HID specification as:

| Bit | Meaning when 0 | Meaning when 1 |
| --- | --- | --- |
| 0 | Data | Constant |
| 1 | Array | Variable |
| 2 | Absolute | Relative |
| 3 | No Wrap | Wrap |
| 4 | Linear | Non Linear |
| 5 | Preferred State | No Preferred |
| 6 | No Null Position | Null State |
| 7 (Input) | Bitfield | Buffered Bytes |
| 7 (Output/Feature) | Non Volatile | Volatile |
| 8 (Output/Feature) | Bitfield | Buffered Bytes |

#### Derivation from WebHID booleans

The synthesis preserves the main-item flag booleans exposed by WebHID using the HID 1.11 bit layout:

| WebHID property | HID bit | Notes |
| --- | ---: | --- |
| `isConstant` | 0 | `true` sets bit 0 (Constant). |
| `isArray` | 1 | Inverted (`false` sets bit 1 = Variable). |
| `isAbsolute` | 2 | Inverted (`false` sets bit 2 = Relative). |
| `isWrapped` | 3 | `true` sets bit 3 (Wrap). |
| `isLinear` | 4 | Inverted (`false` sets bit 4 = Non Linear). |
| `hasPreferredState` | 5 | Inverted (`false` sets bit 5 = No Preferred). |
| `hasNull` | 6 | `true` sets bit 6 (Null State). |
| `isVolatile` | 7 (Output/Feature) | Ignored for Input (bit 7 is Buffered Bytes for Input). |
| `isBufferedBytes` | 7 (Input) / 8 (Output/Feature) | Input uses bit 7; Output/Feature uses bit 8 (requires 2-byte payload). |

---

### Encoding rules

#### Short items only

We only emit **short items** (the normal HID item prefix with 0/1/2/4-byte payload).

- We never emit **long items** (`0xFE …`) because they are rare in practice and are a common source of compatibility problems.

#### Minimal payload size selection

HID short items support payload sizes of `{ 0, 1, 2, 4 }` bytes.

For numeric values we emit, we choose the minimal payload size among `{ 1, 2, 4 }` bytes that can represent the value:

- Unsigned fields (e.g. `Usage Page`, `Usage`, `Report Size`, `Report Count`, `Report ID`) use the smallest unsigned width.
- Signed fields (`Logical Min/Max`, `Physical Min/Max`) use the smallest *signed* width that can represent the value.
- **Unit Exponent** (`0x55`) is **special** in HID 1.11: it is a **4-bit signed value** (`-8..=7`) stored in the **low nibble** of a **single byte** (high nibble reserved and emitted as `0`).

All payloads are encoded **little-endian**.

#### Signed encoding

Signed values are encoded in two’s complement **except Unit Exponent**.

Example: `Logical Minimum (-1)` uses a 1-byte payload: `0xFF`.

Unit Exponent encoding (HID 1.11):

- `Unit Exponent (-1)` → `0x55 0x0F` (not `0x55 0xFF`)
- `Unit Exponent (-2)` → `0x55 0x0E`

---

### Validation rules

The synthesis step validates the WebHID metadata before emitting bytes.

#### Report ID range

- `reportId` MUST be either:
  - `0` (meaning “no report IDs used in this descriptor”), or
  - in the inclusive range `1..=255`.

#### Usage range sanity

When using `Usage Minimum` / `Usage Maximum`:

- `usageMax` MUST be `>= usageMin`.
- The range length (`usageMax - usageMin + 1`) SHOULD be consistent with `reportCount` when used for variable fields (common case: `reportCount == rangeLen`).

#### Unit Exponent range

- `unitExponent` MUST be in the inclusive range `-8..=7` (HID 1.11 4-bit signed field).

#### Report size / count bounds

To keep synthesized descriptors compatible with Windows 7 (and to avoid generating absurdly large
reports):

- `reportSize` MUST be in `1..=255` (HID `REPORT_SIZE` is an 8-bit global item).
- `reportCount` MUST be in `0..=65535` (policy cap).
- `reportSize * reportCount` MUST fit in `u32`.
- The total bit length of a given `(kind, reportId)` across the descriptor MUST fit in `u32`.
- Input reports MUST fit within a single USB full-speed interrupt IN transfer:
  - `ceil(totalBits/8) + (reportId != 0 ? 1 : 0) <= 64`
- Output/feature reports MUST fit within a USB control transfer:
  - `ceil(totalBits/8) + (reportId != 0 ? 1 : 0) <= 65535`
- The synthesized report descriptor byte length MUST fit in `u16` (`<= 65535`) because USB HID
  encodes it as a 16-bit `wDescriptorLength` field in the HID descriptor.

#### Min/max ordering

- `logicalMinimum` MUST be `<= logicalMaximum`.
- `physicalMinimum` MUST be `<= physicalMaximum`.

#### Numeric type bounds (JSON schema)

The normalized WebHID JSON contract is consumed by Rust (`crates/aero-usb`) and therefore must fit
in the Rust value types:

- All unsigned numeric fields (`u32`) MUST be integers in `0..=4294967295`.
- HID usage pages/usages MUST additionally fit in `u16` (`0..=65535`) because we emit `Usage Page`,
  `Usage`, and `Usage Min/Max` using their standard 1-2 byte encodings (we do not support the
  32-bit “Usage = page<<16 | id” form):
  - `usagePage` (collection and report-item)
  - `usage` (collection)
  - `usageMinimum`, `usageMaximum`, and all entries of `usages`
- All signed numeric fields (`i32`) MUST be integers in `-2147483648..=2147483647`:
  - `logicalMinimum`, `logicalMaximum`, `physicalMinimum`, `physicalMaximum`
- Boolean fields MUST be booleans (not `0/1` or other truthy/falsy values):
  - `isAbsolute`, `isArray`, `isBufferedBytes`, `isConstant`, `isLinear`, `isRange`, `isRelative`,
    `isVolatile`, `hasNull`, `hasPreferredState`, `isWrapped`

#### Collection tree bounds

- Collection nesting depth MUST be <= 32.
- Cyclic collection graphs are rejected.

#### Mixed reportId 0/non-zero policy

Windows HID stacks (including Windows 7) treat “report IDs are present” as an interface-wide decision:

- If **any** report uses a non-zero report ID, then **all** reports are expected to include a report ID byte in the transmitted report data.

Policy: the synthesis emits `Report ID` items exactly as provided by WebHID (omitting it when `reportId == 0`) and does not attempt to “fix up” mixed usage. Callers should treat mixed `0`/non-zero report IDs as a metadata error unless they have a device-specific reason to allow it.

---

### Known limitations

- **Ordering loss / canonicalization**
  - WebHID does not give us the original report descriptor byte stream.
  - In particular, it may not preserve the exact ordering/interleaving of:
    - report main items vs nested child collection declarations
    - global item “state machine” usage (`Push`/`Pop`, reusing globals across items, etc.)
  - The synthesis produces a deterministic “canonical” ordering (reports grouped before children) and explicitly re-emits globals per report item.
- **Not all HID tags are supported**
  - We currently do not synthesize less-common local/global tags such as:
    - Designators
    - Strings
    - Delimiters
    - Long Items
  - These can be added if/when we encounter real devices that require them for correct behavior.

---

### Example: synthesized boot-mouse style descriptor (structure)

Given WebHID metadata corresponding to a typical 3-button relative mouse with wheel, the synthesized descriptor is structurally:

```
Usage Page (Generic Desktop)
Usage (Mouse)
Collection (Application)
  Usage (Pointer)
  Collection (Physical)
    Report ID (1)                ; omitted if reportId == 0

    Usage Page (Button)
    Usage Min (Button 1)
    Usage Max (Button 3)
    Logical Min (0)
    Logical Max (1)
    Report Count (3)
    Report Size (1)
    Input (Data, Variable, Absolute)

    Report Count (1)
    Report Size (5)
    Input (Constant)             ; padding

    Usage Page (Generic Desktop)
    Usage (X)
    Usage (Y)
    Usage (Wheel)
    Logical Min (-127)
    Logical Max (127)
    Report Count (3)
    Report Size (8)
    Input (Data, Variable, Relative)
  End Collection
End Collection
```

This is the shape Windows 7 expects for a conventional HID mouse (and is representative of how the synthesis expands WebHID report items into explicit global/local/main items).

### Example: synthesized boot-keyboard style descriptor (structure)

A typical “boot keyboard” shape (modifier bits + 6-key rollover array) looks like:

```
Usage Page (Generic Desktop)
Usage (Keyboard)
Collection (Application)
  Report ID (1)                ; omitted if reportId == 0

  ; Modifiers: 8 one-bit fields (E0..E7)
  Usage Page (Keyboard/Keypad)
  Usage Min (Left Control)
  Usage Max (Right GUI)
  Logical Min (0)
  Logical Max (1)
  Report Size (1)
  Report Count (8)
  Input (Data, Variable, Absolute)

  ; Reserved byte
  Report Size (8)
  Report Count (1)
  Input (Constant)

  ; Key array: 6 bytes
  Usage Page (Keyboard/Keypad)
  Usage Min (0)
  Usage Max (101)
  Logical Min (0)
  Logical Max (101)
  Report Size (8)
  Report Count (6)
  Input (Data, Array, Absolute)
End Collection
```

(Optional output reports like keyboard LEDs are represented the same way: a set of globals/locals followed by an `Output(...)` main item.)

---
