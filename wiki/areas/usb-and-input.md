# USB and input

> Keyboard, mouse, tablet and gamepad input, the USB host controllers that
> carry them, and passthrough of real host devices through WebUSB and WebHID.
> Controller-level detail and the passthrough wire formats are specified in
> [../specs/usb-controllers-and-passthrough.md](../specs/usb-controllers-and-passthrough.md).

How browser keyboard/mouse/gamepad input reaches the Windows 7 guest, the USB
host-controller stack it rides on, and the optional passthrough of real host
devices. Evergreen: everything below is current fact with a cited path, or is
explicitly marked.

> Absorbed from `usb-and-input.md`, `usb-and-input.md`,
> `usb-and-input.md`, `usb-and-input.md`, `usb-and-input.md`,
> `usb-and-input.md`,
> `usb-and-input.md`, `usb-and-input.md`,
> `usb-and-input.md`, `windows-drivers.md`, and
> `windows-drivers.md` (all purged with `docs/`). Canonical USB
> stack decision: [canonical USB stack](../decisions/0015-canonical-usb-stack.md).

---

## What this area is

Aero gives the guest input through three backend families, all driven from the
same captured browser event stream:

- **PS/2 (i8042)** — legacy keyboard/mouse, works out of the box on Win7.
  Canonical model: `crates/aero-devices-input` (`I8042Controller`,
  `ps2_keyboard.rs`, `ps2_mouse.rs`).
- **Synthetic USB HID** — guest-visible USB keyboard/mouse/gamepad/
  consumer-control devices behind an emulated hub (inbox Win7 `hidusb.sys` +
  `kbdhid.sys`/`mouhid.sys`). Device models: `crates/aero-usb/src/hid/`.
- **virtio-input** — paravirtual fast path (keyboard + mouse + optional
  tablet); requires the in-tree Win7 KMDF driver
  (`drivers/windows7/virtio-input/`). Device model:
  `crates/aero-virtio/src/devices/input.rs`.

Optionally, a **real host-connected device** can be passed through using the
browser's WebHID or WebUSB APIs (§8).

## Two runtime shapes

Input integration depends on `vmRuntime`:

- **`vmRuntime=legacy`** — CPU worker + I/O worker. The I/O worker
  (`apps/web/src/workers/io.worker.ts`) owns the guest device models (i8042, USB
  controllers, virtio-input) and performs input routing. WebHID/WebUSB
  passthrough is wired end-to-end **only** in this runtime.
- **`vmRuntime=machine`** — the machine CPU worker
  (`apps/web/src/workers/machine_cpu.worker.ts`) runs the canonical
  `aero_machine::Machine` (exported to JS as `crates/aero-wasm::Machine`),
  which owns all guest devices. The I/O worker runs in host-only stub mode.
  Synthetic-HID and virtio-input injection work here; WebHID/WebUSB
  passthrough is **not** wired up in this runtime.

The JS-facing machine API:

- `new api.Machine(ramSizeBytes)` uses `MachineConfig::browser_defaults`
  (UHCI on, xHCI/virtio-input off) plus `enable_synthetic_usb_hid = true`
  (`crates/aero-wasm/src/lib.rs`, `crates/aero-machine/src/lib.rs`).
- Opt-in constructors: `Machine.new_with_input_backends(ram, enableVirtioInput,
  enableSyntheticUsbHid)` and `Machine.new_with_options(ram, {...})` (snake_case
  keys, e.g. `enable_virtio_input`).
- Injection methods include `inject_browser_key`, `inject_key_scancode_bytes`,
  `inject_keyboard_bytes`, `inject_mouse_motion`, `inject_mouse_button`,
  `inject_mouse_buttons_mask`,
  `inject_virtio_key/rel/button/wheel/hwheel/wheel2`,
  `inject_usb_hid_keyboard_usage`, `inject_usb_hid_consumer_usage`,
  `inject_usb_hid_mouse_move/buttons/wheel/hwheel/wheel2`,
  `inject_usb_hid_gamepad_report` (`crates/aero-wasm/src/lib.rs`).
- Readiness probes for routing: `virtio_input_keyboard_driver_ok()`,
  `virtio_input_mouse_driver_ok()`, `usb_hid_keyboard_configured()`,
  `usb_hid_mouse_configured()`, `usb_hid_gamepad_configured()`,
  `usb_hid_consumer_control_configured()`. LED-state helpers
  (`usb_hid_keyboard_leds()`, `virtio_input_keyboard_leds()`,
  `ps2_keyboard_leds()`) all return the HID-style LED bitmask (bit0 Num, bit1
  Caps, bit2 Scroll, bit3 Compose, bit4 Kana).
- The worker runtime also uses the `UsbHidBridge` WASM export
  (`crates/aero-wasm/src/lib.rs`) to translate captured input into USB HID
  reports without a full machine.

Coordinate conventions: relative motion uses browser-style deltas (`dx>0`
right, `dy>0` down); wheel `>0` is wheel-up; PS/2-convention helpers
(`inject_ps2_mouse_motion`, +Y up) convert internally.

## Browser capture → worker transport

Canonical capture/batching lives in `apps/web/src/input/`:

- `input_capture.ts` (`InputCapture`) — listeners on the VM canvas, Pointer
  Lock on click, blur/visibility handling. On blur or page-hidden it exits
  pointer lock and performs a **release-all flush** (key-ups, buttons=0,
  neutral gamepad report) so the guest can't get stuck inputs. A host-only
  release chord can be configured via `InputCaptureOptions.releasePointerLockChord`.
- `pointer_lock.ts` — Pointer Lock state machine.
- `event_queue.ts` — allocation-free batching; defines `InputEventType`.
- `scancodes.ts` — **generated** from `tools/gen_scancodes/scancodes.json`
  (regenerate: `pnpm run gen:scancodes`); Rust side is
  `crates/aero-devices-input/src/scancodes_generated.rs`. Both come from the
  same JSON source of truth.
- `scancode.ts` — lookup + `preventDefault` policy: function keys, Alt, and
  browser-navigation keys are swallowed while the VM is focused; `Meta` and
  `Ctrl`-only shortcuts pass to the host; `Ctrl+Alt` (AltGr) is captured.
- `hid_usage.ts` — `KeyboardEvent.code` → HID usage mapping (§6.3).
- `gamepad.ts` — Gamepad API polling + 8-byte report packing (§6.2).
- `input_backend_selection.ts` — routing policy (§4).

**Wire format.** Batches are posted to the input-injecting worker as
`{ type: 'in:input-batch', buffer, recycle?: true }`; `buffer` is an
`Int32Array`-compatible payload `[count, batchSendTimestampUs, then per event
(type, eventTimestampUs, a, b)]`. Event types (`apps/web/src/input/event_queue.ts`):

| Type | Payload |
|---|---|
| `KeyScancode` (1) | `a`=little-endian packed PS/2 Set-2 bytes (max 4/event, split in-order), `b`=byteLen |
| `MouseMove` (2) | `a`=dx, `b`=dy (PS/2 coords: +dy up; Y inverted once in `InputCapture`) |
| `MouseButtons` (3) | `a`=DOM `MouseEvent.buttons` bitmask |
| `MouseWheel` (4) | `a`=dz (+up), `b`=horizontal dx (+right; → `REL_HWHEEL` / AC Pan) |
| `GamepadReport` (5) | `a`/`b` = two u32 words of the 8-byte HID gamepad report |
| `KeyHidUsage` (6) | `a`=(usage & 0xFF) | (pressed<<8); keyboard page 0x07 |
| `HidUsage16` (7) | `a`=(usagePage & 0xFFFF) | (pressed<<16), `b`=usageId (e.g. Consumer page 0x0C media keys) |

Capture emits PS/2 scancodes **and** HID usages for the same key so the worker
can feed whichever backend is active without re-translating. With
`recycle: true` the worker transfers the buffer back
(`in:input-batch-recycle`) to avoid per-flush allocation.

## Backend selection and routing

Policy lives in `apps/web/src/input/input_backend_selection.ts`
(`chooseKeyboardInputBackend`, `chooseMouseInputBackend`; unit tests
`input_backend_selection.test.ts`) and is applied by `io.worker.ts`
(`maybeUpdateKeyboardInputBackend` / `maybeUpdateMouseInputBackend`) in legacy
runtime, and inside `machine_cpu.worker.ts` in machine runtime:

- **Keyboard:** virtio-input (once guest sets `DRIVER_OK`) → synthetic USB
  keyboard (once configured, i.e. `SET_CONFIGURATION != 0`) → PS/2.
- **Mouse:** virtio-input (once `DRIVER_OK`) → PS/2 while the synthetic USB
  mouse is unconfigured → synthetic USB mouse once configured.
- **Gamepad:** synthetic USB gamepad only (no PS/2/virtio fallback).
- Switching is **gated on held state** — no backend switch while any key or
  button is held, preventing stuck-key/button states in the guest.

Debug signal for virtio readiness: one-time console logs
`"[virtio-input] keyboard driver_ok"` / `"[virtio-input] mouse driver_ok"`
(`apps/web/src/io/devices/virtio_input.ts`, `VirtioInputPciFunction::driverOk`,
which reads `device_status` bit `0x04` at BAR0+`0x14`).

## PS/2 (i8042) fallback

- Canonical model: `crates/aero-devices-input` (`I8042Controller` in
  `i8042.rs`, `ps2_keyboard.rs`, `ps2_mouse.rs`, Set-2 translation in
  `scancode.rs` + `scancodes_generated.rs`). The web runtime has an equivalent
  TS device (`apps/web/src/io/devices/i8042.ts`) with parity tests against the WASM
  model.
- The PS/2 mouse only emits movement packets after the guest enables data
  reporting (`0xF4`); OS drivers do this during boot, bare-metal tests may
  need to do it explicitly.
- Back/forward buttons (4/5) only appear in PS/2 stream packets when the guest
  enables the IntelliMouse Explorer 5-button extension (device ID `0x04`).
- i8042 IRQ1/IRQ12 are edge-triggered in Aero's model; interrupt *levels* are
  not snapshotted (the interrupt controller owns those). IRQ line semantics
  contract: `wiki/areas/platform-and-firmware.md`.

## Synthetic USB HID devices

Device models live in `crates/aero-usb/src/hid/` (`keyboard.rs`, `mouse.rs`,
`gamepad.rs`, `consumer_control.rs`), exposed to the guest behind a fixed
hub topology. `MachineConfig.enable_synthetic_usb_hid` attaches the hub and
all four devices; the `aero-wasm` `Machine` constructors turn this on by
default (§2). Enabled devices bind via the Win7 inbox HID stack.

### 6.1 External hub topology (reserved-port ABI)

A fixed, guest-visible topology lets synthetic and passthrough devices coexist
without fighting over ports. Treat these assignments as ABI — host-side code
assumes them:

- Root port **0**: external USB hub (synthetic HID + WebHID passthrough).
- Root port **1**: reserved for the WebUSB passthrough device.
- Hub port **1**: keyboard · **2**: mouse · **3**: gamepad · **4**:
  consumer-control (media keys) · **5+**: dynamic WebHID passthrough
  allocation (`UHCI_EXTERNAL_HUB_FIRST_DYNAMIC_PORT`).
- UHCI exposes exactly 2 root ports; the external hub defaults to 16
  downstream ports. Behind xHCI, hub ports are clamped to **≤15** (Route
  String encodes hub ports as 4-bit values) and legacy root-port-only paths
  are remapped behind root port 0 by the xHCI topology manager.

Source-of-truth constants: `apps/web/src/usb/uhci_external_hub.ts`
(`EXTERNAL_HUB_ROOT_PORT`, `DEFAULT_EXTERNAL_HUB_PORT_COUNT = 16`,
`UHCI_EXTERNAL_HUB_FIRST_DYNAMIC_PORT = 5`) and
`crates/aero-machine/src/lib.rs` (`Machine::UHCI_*`, kept in sync — a drift
test exists at `apps/web/src/usb/uhci_machine_topology_rust_drift.test.ts`).

Legacy note: `aero_usb::hid::composite::UsbCompositeHidInput` (snapshot device
ID `UCMP`, `crates/aero-usb/src/hid/composite.rs`) still exists as an
alternative composite-device model used by some tests; the external-hub
topology above is the runtime default.

### 6.2 Report formats

- **Keyboard:** standard 8-byte boot report (modifier byte, reserved byte, 6
  key slots in press order; >6 keys → `ErrorRollOver` `0x01` in all slots).
- **Consumer control:** 2-byte little-endian u16 usage ID, 0 = none pressed
  (`crates/aero-usb/src/hid/consumer_control.rs`).
- **Mouse:** 5-byte report protocol: buttons (bits 0..4), X (i8), Y (i8),
  wheel (i8), horizontal wheel / AC Pan (i8). Boot protocol (`SET_PROTOCOL`)
  drops to 3 bytes (buttons/X/Y) and masks buttons 4/5; scroll is ignored in
  boot mode. `movementY>0` → HID Y positive (down); `deltaY>0` (scroll down)
  maps to negative HID wheel; `deltaX>0` maps to positive AC Pan. The web
  runtime uses a combined `mouse_wheel2(wheel, hwheel)` injection when
  available so diagonal scroll arrives in one report.
- **Gamepad:** fixed 8-byte report, no report ID
  (`crates/aero-usb/src/hid/gamepad.rs::GamepadReport`): u16 LE button
  bitfield (16 buttons), hat (0..7, `8` = null/centered via HID Null state),
  X/Y/Rx/Ry (i8, `-127..=127`), 1 padding byte. Browser standard-mapping
  assignment: bits 0..3 = A/B/X/Y, 4/5 = LB/RB, 6/7 = LT/RT (digital), 8/9 =
  Back/Start, 10/11 = stick presses, 12..15 = Guide + extras; d-pad buttons
  12..15 fold into the hat and are not in the bitfield. TS pack/unpack:
  `apps/web/src/input/gamepad.ts`.

### 6.3 Usage mapping and shared fixtures

`KeyboardEvent.code` (physical position, layout-stable) is mapped to HID
usages twice — Rust `crates/aero-usb/src/hid/usage.rs`
(`keyboard_code_to_usage`, `keyboard_code_to_consumer_usage`; browser
convenience wrappers in `crates/aero-usb/src/web.rs`) and TS
`apps/web/src/input/hid_usage.ts`. Parity is pinned by shared JSON fixtures
consumed by tests on both sides:

- `protocol-vectors/hid_usage_keyboard.json` (keyboard page 0x07)
- `protocol-vectors/hid_usage_consumer.json` (consumer page 0x0C media keys)
- `protocol-vectors/hid_gamepad_report_vectors.json` +
  `protocol-vectors/hid_gamepad_report_clamping_vectors.json` (gamepad packing)
- `protocol-vectors/webusb_passthrough_wire.json` (WebUSB wire contract, §8.3)

Tests: `crates/aero-usb/tests/hid_usage_keyboard_fixture.rs`,
`hid_usage_consumer_fixture.rs`, `hid_gamepad_report_fixture.rs`,
`hid_gamepad_report_clamping_fixture.rs`; TS: `apps/web/src/input/hid_usage.test.ts`,
`apps/web/src/input/gamepad.test.ts`. Focused gate: `cargo xtask input`
(`xtask/src/cmd_input.rs`; `--usb-all` for the full `aero-usb` suite,
`--rust-only` when Node is unavailable).

> **Purge note:** these fixtures live under `protocol-vectors/` (relocated
> from `docs/`) and are load-bearing (`include_str!` / `readFile` from Rust
> and TS tests). They must never be deleted with the docs/ purge.

## USB host controllers

All controller models live in `crates/aero-usb` and share the device-model
layer (`device::AttachedUsbDevice`, `hub.rs`), so HID/passthrough device work
is written once per device, not per controller. Native PCI wrappers:
`crates/devices/src/usb/`; web PCI wrappers: `apps/web/src/io/devices/`; WASM
bridges: `crates/aero-wasm/src/*_controller_bridge.rs`. The `crates/emulator`
shims (`emulator::io::usb::*`) are legacy glue — EHCI/xHCI are feature-gated
(`legacy-usb-ehci` / `legacy-usb-xhci` in `crates/emulator/Cargo.toml`) — and
the layer is tracked for deletion.

| | UHCI | EHCI | xHCI |
|---|---|---|---|
| Status | Known-good default for Win7 | Implemented (bring-up complete for MVP scope) | Bring-up quality, incomplete |
| PCI identity | `00:01.2` (`USB_UHCI_PIIX3`) | `00:12.0` `8086:293a` (`USB_EHCI_ICH9`) | `00:0d.0` `1b36:000d` (`USB_XHCI_QEMU`) |
| Class code | `0c/03/00` | `0c/03/20` | `0c/03/30` |
| Root ports | 2 | 6 default (`DEFAULT_PORT_COUNT`) | USB2-only subset |
| Speed | Full-speed | High-speed | High-speed (USB3 not modeled) |
| Interrupt | INTx | INTx (no MSI) | INTx on web; MSI/MSI-X optional natively |
| Win7 driver | Inbox | Inbox (`usbehci.sys`) | **None inbox** — needs a third-party driver |

Profiles: `crates/devices/src/pci/profile.rs`. Enablement:
`MachineConfig.enable_uhci/enable_ehci/enable_xhci` (all default `false`
natively; `browser_defaults` turns on UHCI only; `PcPlatformConfig.enable_xhci`
separately).

**EHCI** (`crates/aero-usb/src/ehci/`): capability + operational MMIO with W1C
masking, USB Legacy Support extended capability (`USBLEGSUP`/`USBLEGCTLSTS`
BIOS-handoff semaphores, exposed in the MMIO window at the `EECP` offset),
root hub ports with ~50 ms reset timers and change-bit latching, `FRINDEX`
advancing 8 microframes per 1 ms tick, async schedule (QH/qTD, control+bulk)
and periodic schedule (interrupt polling) engines, snapshot/restore. IRQ level
derives from `USBSTS` causes (`USBINT`/`USBERRINT`/`PCD`/`IAA`) gated by
`USBINTR`. Pending host work is represented as qTD left Active (the EHCI
analogue of UHCI NAK-while-pending). Not implemented: isochronous
(`iTD`/`siTD`), split/TT transactions, MSI. Tests: `crates/aero-usb/tests/ehci*.rs`.

**Companion routing:** EHCI `CONFIGFLAG`/`PORT_OWNER` are implemented
(`PORT_OWNER=1` ⇒ companion-owned/unreachable). When both UHCI and EHCI are
enabled, `aero_machine::Machine` creates a shared `Usb2PortMux`
(`crates/aero-usb/src/usb2_port.rs`) wiring UHCI ports 0–1 ↔ EHCI ports 0–1 so
ownership handoff moves the same device between controllers (tests:
`crates/aero-usb/tests/usb2_companion_routing.rs`). Only the first two EHCI
ports are muxed; remaining EHCI ports are standalone.

**xHCI** (`crates/aero-usb/src/xhci/`): 64 KiB MMIO window
(`XhciController::MMIO_SIZE == 0x10000`), capability/operational/runtime
register subsets, USB Legacy Support + Supported Protocol extended caps,
USB2 root ports with PORTSC + reset timers and port-status-change events,
interrupter 0 (IMAN/IMOD/ERST/ERDP) with ERST-backed guest event ring,
doorbell 0 command-ring processing for a bounded command subset (No-Op,
Enable/Disable Slot, Address Device, Configure Endpoint, Evaluate Context,
Stop/Reset Endpoint, Set TR Dequeue Pointer; most others → TRB Error), and
doorbell-driven transfers: EP0 control (Setup/Data/Status TRBs) plus
bulk/interrupt Normal TRBs via `XhciTransferExecutor`. `tick_1ms` advances
MFINDEX (8 µframes/ms, 14-bit wrap), runs transfer + command-ring work, and
drains the event ring; DMA is gated on PCI Bus Master Enable. NAK leaves a TD
pending for retry; short packets produce `ShortPacket` completions with
residuals. Snapshot IDs: controller `XHCI` v0.9, WASM bridge `XHCB` v1.1,
native wrapper `XHCP` v1.2. Not implemented: USB3/SuperSpeed, isochronous,
streams, full command/state-machine coverage, multiple interrupters, web-side
MSI/MSI-X. WebHID passthrough attaches behind xHCI via
`apps/web/src/hid/xhci_hid_topology.ts` (`XhciHidTopologyManager`) when the WASM
build exports the topology APIs; otherwise the worker falls back to the UHCI
topology manager (or an EHCI-backed shim in UHCI-less builds,
`apps/web/src/hid/ehci_hid_topology_shim.ts`,
`apps/web/src/workers/io_hid_topology_mux.ts`). Regression tests are numerous —
see `crates/aero-usb/tests/xhci_*.rs`,
`crates/aero-machine/tests/{machine_xhci,xhci_snapshot}*.rs`,
`crates/devices/tests/xhci_msix_integration.rs`, and
`apps/web/src/usb/xhci_webusb_root_port_rust_drift.test.ts`.

**Snapshots:** controller snapshots include register state, root-hub port
state/timers, and the attached device topology (nested device snapshots).
Guest RAM structures (TDs/QHs/TRBs/rings) belong to the VM memory snapshot.
Passthrough host state is **not** restorable (JS Promises can't rewind):
restore paths call `reset_host_state_for_restore()` so in-flight host work is
dropped and the guest retries (§8.3). The browser I/O worker can pack
multiple controller blobs into one `"AUSB"` container tagged per controller
(`apps/web/src/workers/usb_snapshot_container.ts`: `USB_SNAPSHOT_TAG_UHCI/EHCI/XHCI`).

## Physical-device passthrough (WebHID / WebUSB)

Optional “real hardware” path: attach a host-connected device to the guest as
a USB peripheral. Wired end-to-end in the **legacy** runtime only.

### 8.1 Architecture

Browser device handles are main-thread objects: `requestDevice()` needs a user
gesture, `HIDDevice` is not structured-cloneable, and `USBDevice` worker
support is unreliable. So:

- **Main thread** owns the handle: selection, open/close, executing WebHID
  report sends/reads and WebUSB transfers.
- **I/O worker** owns the guest-visible USB controller + device model and
  hot-plugs the device into the §6.1 topology.

Building blocks:

- Rust models: `UsbHidPassthrough` (`crates/aero-usb/src/hid/passthrough.rs`),
  `UsbPassthroughDevice` (`crates/aero-usb/src/passthrough.rs`),
  `UsbWebUsbPassthroughDevice` (`crates/aero-usb/src/passthrough_device.rs`).
- WASM bridges (`crates/aero-wasm/src/lib.rs`): `WebHidPassthroughBridge`,
  `UsbPassthroughBridge`, `UhciControllerBridge` / `EhciControllerBridge` /
  `XhciControllerBridge` (each exposes the WebUSB passthrough device on its
  reserved root port — 1, falling back to 0 for single-port xHCI), plus
  dev/harness-only `WebUsbUhciBridge`, `WebUsbUhciPassthroughHarness`,
  `WebUsbEhciPassthroughHarness`, and the `UsbPassthroughDemo` smoke driver.
- TypeScript: main-thread `WebHidPassthroughManager`
  (`apps/web/src/platform/webhid_passthrough.ts`), `WebHidBroker`
  (`apps/web/src/hid/webhid_broker.ts` + `hid_proxy_protocol.ts`), `UsbBroker`
  (`apps/web/src/usb/usb_broker.ts` + `webusb_backend.ts`/`webusb_executor.ts` +
  `usb_proxy_protocol.ts`), worker-side `WebUsbPassthroughRuntime`
  (`apps/web/src/usb/webusb_passthrough_runtime.ts`), topology managers
  (`apps/web/src/hid/uhci_hid_topology.ts`, `xhci_hid_topology.ts`), guest-path
  schema (`apps/web/src/platform/hid_passthrough_protocol.ts`).
- Transport: typed `postMessage` with transferred `ArrayBuffer`s by default;
  when `crossOriginIsolated`, SharedArrayBuffer ring fast paths —
  `hid.ring.init` (input reports main→worker,
  `apps/web/src/hid/hid_input_report_ring.ts`), `hid.ringAttach`
  (`HidReportRing`, `apps/web/src/usb/hid_report_ring.ts`) for output/feature
  worker→main, and `usb.ringAttach` (`apps/web/src/usb/usb_proxy_ring.ts` +
  `usb_proxy_ring_dispatcher.ts`). Rings are best-effort: overflow falls back
  to `postMessage`; corruption triggers `*.ringDetach` and a permanent
  downgrade. Output/feature reports execute **in order per device** through a
  bounded per-device FIFO shared by both transports.

Dev-only scaffolding (not the target architecture): `WebHidPassthroughRuntime`
(`apps/web/src/usb/webhid_passthrough_runtime.ts`) runs on the main thread and
bypasses the broker split; `apps/web/src/io/devices/uhci_webusb.ts` +
`WebUsbUhciBridge` are harness/test-only.

### 8.2 WebHID path: descriptor synthesis and report proxying

WebHID exposes structured report metadata, not raw descriptor bytes, so Aero
synthesizes a semantically equivalent HID report descriptor targeting “boring
HID 1.11” for Win7 `hidparse.sys`:

- Normalization: `apps/web/src/hid/webhid_normalize.ts` (accepts string or numeric
  collection types; range items carry compact `[min, max]` usages).
- Synthesis: `crates/aero-usb/src/hid/report_descriptor.rs` (canonical
  encoder) driven by `crates/aero-usb/src/hid/webhid.rs` (JSON schema +
  conversion). Deterministic order: per collection, input → output → feature
  reports, then children depth-first; globals re-emitted per item (Usage Page,
  Logical/Physical Min/Max, Unit Exponent, Unit, Report Size, Report Count);
  short items only, minimal payload widths, signed fields two's-complement
  **except** Unit Exponent (4-bit signed nibble: `-1` → `0x55 0x0F`).
- Validation caps: report IDs `0|1..=255` (mixed 0/non-zero is a metadata
  error), usage pages/usages fit u16, collection depth ≤ 32, and **input
  reports must fit a single 64-byte full-speed interrupt packet** (larger
  reports are rejected; no multi-packet splitting).
- Known limits: original item ordering/interleaving is lost (canonical
  regrouping), and strings/designators/delimiters/long items are not emitted.
- Contract fixtures: `tests/fixtures/hid/webhid_normalized_{mouse,keyboard,gamepad}.json`;
  tests `apps/web/test/webhid_normalize_fixture.{test.ts,vitest.ts}`,
  `crates/aero-usb/tests/webhid_passthrough.rs`,
  `crates/aero-wasm/tests/webhid_report_descriptor_synthesis{,_wasm}.rs`.

Report proxying: input reports queue device-side; an empty queue NAKs the
guest's interrupt-IN poll. Output/feature reports (`SET_REPORT` or interrupt
OUT) queue worker-side for the main thread to send. `GET_REPORT` Feature is
proxied via `hid.getFeatureReport`/`hid.featureReportResult`: the device
model NAKs until the host Promise settles, caches successful reads per
reportId (invalidated by `SET_REPORT` and reset), tracks at most one in-flight
read per reportId, and the main thread normalizes reply length
(truncate/zero-pad; 4096-byte cap when the expected length is unknown).

### 8.3 WebUSB path: async actions over a synchronous controller

WebUSB is Promise-based; guest controllers need synchronous TD outcomes. The
device model (`crates/aero-usb/src/passthrough.rs`) therefore emits
`UsbHostAction`s and consumes `UsbHostCompletion`s, representing
“waiting for host” as **NAK-while-pending** (TD stays active and is retried —
never block the VM, never fake completion).

Wire contract (pinned by `protocol-vectors/webusb_passthrough_wire.json`; TS
mirror `apps/web/src/usb/usb_passthrough_types.ts`):

- Actions: `controlIn`, `controlOut`, `bulkIn`, `bulkOut` (bulk carries a full
  endpoint **address** incl. direction bit; host uses `endpoint & 0x0f`).
- Completions per kind: `success` (data / bytesWritten) | `stall` | `error`.
- IDs are non-zero u32, monotonic for the life of the device model;
  `drain_actions()` returns `null` when idle; stale completions (unknown IDs)
  are dropped, so bridges/device models must be kept alive across
  disconnect/reconnect (`reset()` clears in-flight state but preserves
  `next_id`).
- Stage mapping (UHCI): SETUP TDs ACK once a device is present; a control
  request becomes exactly one action, with NAK applied to DATA (control-IN) or
  STATUS (control-OUT, after the payload buffers); empty IN data surfaces as a
  ZLP (`actlen=0x7FF`); short packets honor the UHCI SPD bit.
- Completion→guest mapping: success → DATA/ACK, stall → STALL, error →
  TIMEOUT, pending → NAK. One in-flight action per endpoint; issue one action
  per guest TD (≤ `wMaxPacketSize`) to keep data-toggle sync (toggle modeling
  itself is not yet implemented in the UHCI model — forward-looking rule).
- The TS executor recognizes standard requests and routes them to dedicated
  WebUSB APIs: `SET_CONFIGURATION` → `selectConfiguration` (releasing claims
  first), `SET_INTERFACE` → `selectAlternateInterface`,
  `CLEAR_FEATURE(ENDPOINT_HALT)` → `clearHalt`. `SET_ADDRESS` is always
  virtualized (the physical device is already host-enumerated).
- **Speed views:** behind UHCI the guest sees full-speed; the executor
  best-effort translates `OTHER_SPEED_CONFIGURATION` → `CONFIGURATION`
  (`shouldTranslateConfigurationDescriptor` / `rewriteOtherSpeedConfigAsConfig`
  in `apps/web/src/usb/webusb_backend.ts`). Behind EHCI/xHCI (when the WASM build
  exports the passthrough hooks) the guest sees high-speed descriptors
  unmodified — the UHCI fixup must be disabled. High-speed-only devices should
  use EHCI/xHCI, but that routing is experimental; UHCI is the known-good path.
- Snapshots: `UsbPassthroughDevice` persists only `next_id`;
  `UsbWebUsbPassthroughDevice` also persists guest-visible USB state (address,
  control pipe stage). After restore, `reset_host_state_for_restore()` drops
  queued/in-flight host work (called by the controller bridge `load_state`
  paths); the guest retries and fresh actions re-emit with deterministic IDs.

### 8.4 Chromium constraints (why WebUSB is narrow)

- **Protected interface classes** are unclaimable: Audio `0x01`, HID `0x03`,
  Mass Storage `0x08`, Hub `0x09`, Smart Card `0x0B`, Video `0x0E`, A/V
  `0x10`, Wireless `0xE0`. Devices with only protected interfaces don't appear
  in the chooser; composite devices need ≥1 unprotected interface. Aero's
  mirror lists: `apps/web/src/platform/webusb_protection.ts`, `apps/web/src/platform/webusb.ts`.
- **No isochronous transfers** in practice ⇒ no USB audio/video passthrough.
- **Host OS friction:** Windows needs the interface bound to WinUSB (MS OS 2.0
  descriptors preferred; Zadig as manual fallback); Linux needs udev
  permissions and no bound kernel driver. Vendor-specific (`0xFF`)
  bulk/interrupt devices are the realistic target.
- WebUSB is effectively **Chromium-only**; WebHID is the recommended path for
  HID peripherals.
- Diagnostics: in-app panels (`apps/web/src/usb/webusb_panel.ts`,
  `usb_broker_panel.ts`, harness panels), standalone page
  `/webusb_diagnostics.html` (`apps/web/src/webusb_diagnostics.ts`).

### 8.5 Security / UX rules

- Secure context required (`https` or `localhost`); both APIs require a user
  gesture — call `requestDevice()` directly in the handler (activation doesn't
  cross `postMessage`, and `await` before the call can lose it).
- Never auto-attach a previously-granted device without an explicit per-session
  user action; show a persistent “connected to VM” indicator with one-click
  disconnect.
- Revocation: use `HIDDevice.forget()`/`USBDevice.forget()` when available,
  otherwise point users at browser site settings.

## Virtio-input (paravirtual fast path)

The low-latency input path once the guest driver is installed: events are
pushed into a virtqueue instead of modeling USB frames and HID polling.

### 9.1 Device contract

Single multi-function PCI device (AERO-W7-VIRTIO contract v1 — full text in
`wiki/areas/windows-drivers.md`):

- Vendor/Device `1AF4:1052` (`VIRTIO_ID_INPUT`), Revision `0x01` (`REV_01`).
- Function 0 **keyboard** `00:0A.0` (SUBSYS `0x0010`, `header_type=0x80`),
  function 1 **mouse** `00:0A.1` (SUBSYS `0x0011`), optional function 2
  **tablet** `00:0A.2` (SUBSYS `0x0012`, absolute `EV_ABS` pointer).
  Profiles: `crates/devices/src/pci/profile.rs` (`VIRTIO_INPUT_KEYBOARD/MOUSE/TABLET`).
- BAR0: 64-bit MMIO, `0x4000` bytes, fixed virtio capability layout
  (COMMON `0x0000`, NOTIFY `0x1000` multiplier 4, ISR `0x2000`, DEVICE `0x3000`).
- Two split virtqueues per function, 64 entries each: **eventq**
  (device→driver) and **statusq** (driver→device, LED output events — all
  descriptors must be consumed/completed even if contents are ignored).
- Events are Linux-ABI `virtio_input_event { le16 type, le16 code, le32 value }`
  (8 bytes; eventq buffers complete with `used.len = 8`), batched with
  `EV_SYN/SYN_REPORT`.
- Device-specific config space supports `ID_NAME` (“Aero Virtio Keyboard /
  Mouse / Tablet”), `ID_SERIAL`, `ID_DEVIDS`, `EV_BITS` bitmaps (keyboard:
  `EV_SYN|EV_KEY|EV_LED`; mouse: `EV_SYN|EV_KEY|EV_REL` incl. `REL_WHEEL`/
  `REL_HWHEEL`; tablet: `EV_SYN|EV_ABS` + `ABS_INFO` ranges).
- Interrupts: INTx baseline; MSI-X supported in the canonical machine via
  `VirtioMsixInterruptSink` → `PlatformInterrupts::trigger_msi`
  (`crates/aero-machine/src/lib.rs`; regression test
  `crates/aero-machine/tests/virtio_input_msix.rs`).

### 9.2 Host integration

- Device model: `crates/aero-virtio/src/devices/input.rs`.
- Legacy runtime: `apps/web/src/io/devices/virtio_input.ts` (PCI wrapper +
  injection), registered by `apps/web/src/workers/io_virtio_input_register.ts`.
- Machine runtime: `MachineConfig.enable_virtio_input = true` (requires
  `enable_pc_platform`); JS opt-in via `new_with_options`/`new_with_input_backends`;
  injection `inject_virtio_*` (Linux evdev codes; `dy>0` = down). Disabled by
  default everywhere for backwards compatibility.

### 9.3 Windows 7 driver

Win7 has no inbox virtio-input driver; the in-tree KMDF HID minidriver lives
at `drivers/windows7/virtio-input/` (binds the PCI functions, translates
`virtio_input_event` → HID reports into `hidclass.sys`, forwards LED output
reports to statusq). Devices appear as **Aero VirtIO Keyboard / Mouse /
Tablet** under HIDClass.

- INFs are **revision-gated** (`...&REV_01`): `inf/aero_virtio_input.inf`
  carries subsystem-qualified keyboard/mouse HWIDs plus a generic
  `PCI\VEN_1AF4&DEV_1052&REV_01` fallback; `inf/aero_virtio_tablet.inf` binds
  the tablet. `inf/virtio-input.inf.disabled` is an optional filename alias
  that must stay byte-identical from `[Version]` onward (guardrails:
  `drivers/windows7/virtio-input/scripts/check-inf-alias.py`,
  `scripts/ci/check-windows7-virtio-contract-consistency.py`). Ship only one
  basename at a time.
- The driver is strict about `ID_NAME`; QEMU devices reporting non-Aero names
  are refused unless compat mode (`CompatIdName=1`) is enabled (see
  `drivers/windows7/virtio-input/docs/virtio-input-notes.md`).
- Install flow: `bcdedit /set testsigning on` + reboot, import the test cert
  into Trusted Root + Trusted Publishers, then Device Manager → Have Disk
  (full workflow: `drivers/windows7/virtio-input/README.md`).

### 9.4 Validation ladder

1. **Rust conformance:** `bash ./scripts/safe-run.sh cargo test -p aero-virtio --locked --test virtio_input`
   (`crates/aero-virtio/tests/virtio_input.rs`), plus contract-level suites
   (`win7_contract_queue_sizes`, `win7_contract_ring_features`,
   `win7_contract_dma_64bit`, `pci_profile`, `pci_bar0_mmio_access_sizes`),
   `crates/devices/tests/pci_virtio_input_multifunction.rs` and
   `windows_device_contract_virtio_input.rs`,
   `crates/aero-wasm/tests/virtio_input_pci_device_core.rs`.
2. **Driver portable unit tests** (no WDK needed): `cd drivers/windows7/virtio-input && bash tests/run.sh`
   — covers `hid_translate.c` (must map `KEY_F1..F12` → usages `0x3A..0x45`,
   `KEY_NUMLOCK` → `0x53`, `KEY_SCROLLLOCK` → `0x47`) and the LED
   parse/translate pipeline.
3. **QEMU bring-up (manual):** attach
   `virtio-keyboard-pci`/`virtio-mouse-pci` with `disable-legacy=on,x-pci-revision=0x01`,
   install the test-signed driver, verify HID keyboard + mouse enumerate
   (details + `hidtest.exe` probes: `drivers/windows7/virtio-input/tests/qemu/README.md`,
   `tools/hidtest/README.md`). Expected failures: Code 28 pre-install,
   Code 52 = signing, Code 10 = contract/ID_NAME mismatch.
4. **Automated host harness** (QEMU + guest selftest):
   `drivers/windows7/tests/host-harness/Invoke-AeroVirtioWin7Tests.ps1` or
   `invoke_aero_virtio_win7_tests.py` (README in the same directory). Serial
   markers: `virtio-input`, `virtio-input-bind`, and optional
   `virtio-input-led` / `virtio-input-events` / `virtio-input-wheel` /
   `virtio-input-media-keys` / `virtio-input-events-*` /
   `virtio-input-tablet-events` / `virtio-input-msix`, each gated by matching
   guest provisioning flags and host `-With*`/`--with-*` switches; event
   markers are driven by QMP `input-send-event` injection.
5. **Web runtime:** boot a driver-installed Win7 image; confirm fallback input
   works before `DRIVER_OK`, the `driver_ok` console logs appear, and routing
   switches (after releasing held keys/buttons). Unit checks:
   `pnpm -C apps/web run test:unit -- src/io/devices/virtio_input_pci.test.ts`,
   `virtio_input_keymap.test.ts`, `virtio_input_pci_integration.test.ts`
   (needs `pnpm run wasm:ensure`), `apps/web/src/input/input_backend_selection.test.ts`.

Windows images are not distributed by this repo; supply your own Win7 media
for steps 3–4.

## Open issues and debt

- **xHCI is bring-up quality**: bounded command subset, EP0 + limited
  bulk/interrupt Normal TRBs only, no USB3/iso/streams, opt-in everywhere,
  and Win7 has no inbox xHCI driver. UHCI remains the known-good controller.
- **EHCI gaps**: no isochronous (`iTD`/`siTD`), no split/TT, no MSI; only the
  first two root ports are muxed with UHCI companions.
- **Passthrough is legacy-runtime-only**: `vmRuntime=machine` has no
  WebHID/WebUSB passthrough wiring.
- **Higher-level VM snapshot wiring for passthrough** (coordinator/device
  plumbing around the deterministic device snapshots) is incomplete.
- **Low-speed USB devices are not modeled**; some HID peripherals fail
  enumeration as a result.
- **UHCI data-toggle modeling is not implemented**; the one-action-per-TD rule
  is forward-looking.
- **WebUSB high-speed routing (EHCI/xHCI) is experimental**; UHCI is the
  default attachment path.
- **`protocol-vectors/` relocation is done**: the HID/WebUSB fixture JSONs
  moved there from `docs/` (§6.3); Rust tests `include_str!` them from the
  new location.
- `crates/emulator` USB shims are legacy and tracked for deletion (see
  `wiki/areas/platform-and-firmware.md`).

## Pointers

- Canonical USB stack decision: `wiki/decisions/0015-canonical-usb-stack.md`
  ([canonical USB stack](../decisions/0015-canonical-usb-stack.md)). Virtio contract: `wiki/areas/windows-drivers.md`
  (AERO-W7-VIRTIO v1). IRQ line semantics: `wiki/areas/platform-and-firmware.md`.
  PCI layout: `wiki/areas/platform-and-firmware.md`.
- Rust: `crates/aero-usb` (controllers, hub, HID devices, passthrough,
  descriptor synthesis), `crates/aero-devices-input` (i8042/PS/2),
  `crates/aero-virtio/src/devices/input.rs`, `crates/devices/src/{pci/profile.rs,usb/}`,
  `crates/aero-machine`, `crates/aero-wasm` (JS exports).
- Web: `apps/web/src/input/`, `apps/web/src/usb/`, `apps/web/src/hid/`,
  `apps/web/src/io/devices/{i8042,uhci,ehci,xhci,virtio_input}.ts`,
  `apps/web/src/workers/{io.worker,machine_cpu.worker}.ts`,
  `apps/web/src/platform/{webhid_passthrough,webusb,webusb_protection,hid_passthrough_protocol}.ts`.
- Drivers: `drivers/windows7/virtio-input/`, harness
  `drivers/windows7/tests/host-harness/`.
- Gates: `cargo xtask input`, `cargo test -p aero-usb --locked`,
  `cargo test -p aero-virtio --locked --test virtio_input`,
  `pnpm -C apps/web run test:unit` — wrap heavier runs in `bash ./scripts/safe-run.sh`.

## Device-model fixes applied (2026-07-27)

See `wiki/notes/bring-up-findings-divergence-and-sight-audits.md` §6 for details.

- **EHCI**: HCCPARAMS EECP set to 0 (was advertising USBLEGSUP in wrong space).
- **UHCI**: HCHALTED is RS-only per spec (was set during EGSM global suspend).

## Input Device Emulation

### Overview

Windows 7 requires keyboard and mouse input. The baseline approach is to capture browser events and translate them into **legacy PS/2** (works out-of-the-box), or **USB HID** (also inbox, but requires a much larger emulation surface).

#### Two runtime shapes: full-system `Machine` vs worker runtime

This repo supports input in two different (but related) integration styles:

- **Canonical full-system VM (`aero_machine::Machine` → `crates/aero-wasm::Machine`)**
  - One object owns the CPU core + device models.
  - Primarily used by native Rust integration tests and by the JS/WASM “single machine” API.
  - Input is injected by calling `Machine.inject_*` methods directly.
- **Browser worker runtime (production)**
  - A main-thread coordinator plus workers. The exact shape depends on `vmRuntime`:
    - `vmRuntime=legacy`: **CPU worker** (executes guest CPU in WASM) + **I/O worker** (owns device models + routing).
    - `vmRuntime=machine`: **machine CPU worker** running the canonical `api.Machine`.
      - The I/O worker runs in host-only stub mode and does not own guest input devices.
  - Browser input is captured + batched in `apps/web/src/input/*` and delivered as `in:input-batch` messages to the worker that injects input:
    - `vmRuntime=legacy`: I/O worker (`apps/web/src/workers/io.worker.ts`)
    - `vmRuntime=machine`: machine CPU worker (`apps/web/src/workers/machine_cpu.worker.ts`)
  - In `vmRuntime=legacy` the I/O worker routes each event to one of: **PS/2** (fallback), **virtio-input** (fast path), or **synthetic USB HID devices behind the guest-visible USB controller** (when enabled).
  - In `vmRuntime=machine` the CPU worker injects input directly into the canonical `api.Machine` instance and performs backend selection/routing (virtio-input → synthetic USB HID → PS/2) based on guest readiness + configuration.

When editing the browser runtime input pipeline, treat `apps/web/src/input/*` as canonical capture/batching. Injection/routing is handled by:

- `vmRuntime=legacy`: `apps/web/src/workers/io.worker.ts`
- `vmRuntime=machine`: `apps/web/src/workers/machine_cpu.worker.ts`

The `aero-wasm::Machine` injection API is still a useful ergonomic/testing surface.

#### WASM / browser integration: canonical `Machine` input injection

For JS/WASM-side input testing and future in-browser integration, the canonical full-system VM (`aero_machine::Machine`) is exported to JS via `crates/aero-wasm::Machine`.

The WASM-facing `Machine` wrapper exposes **explicit** input injection methods for:

- **PS/2 via i8042** (legacy; always works)
- **virtio-input** (paravirtualized; requires the Aero Win7 virtio-input driver; opt-in at construction time)
- **synthetic USB HID devices behind the guest-visible USB controller** (keyboard + mouse + gamepad + consumer-control; enabled by default for `new Machine(ramSizeBytes)`; UHCI by default)

To explicitly configure these backends from JS, construct the machine via:

- `Machine.new_with_input_backends(ramSizeBytes, enableVirtioInput, enableSyntheticUsbHid)`
- `Machine.new_with_options(ramSizeBytes, { enable_virtio_input: true, enable_synthetic_usb_hid: true })`
  - `new_with_options` uses wasm-bindgen/Rust **snake_case** option keys and can configure additional device flags beyond input.

##### Coordinate conventions (important)

- For *relative pointer movement* APIs, Aero uses **browser-style deltas**:
  - `dx > 0` is right
  - `dy > 0` is down
- For *wheel* APIs, `wheel > 0` means **wheel up**.
- Convenience helper: `Machine.inject_ps2_mouse_motion(dx, dy, wheel)` accepts **PS/2-style** `dy > 0` = up and converts internally.

##### PS/2 (i8042)

- Keyboard: `Machine.inject_browser_key(code, pressed)` where `code` is DOM `KeyboardEvent.code`.
- Keyboard (raw Set-2 bytes, matches `apps/web/src/input/event_queue.ts` packing):
  - `Machine.inject_key_scancode_bytes(packed, len)` where:
    - `packed` is little-endian packed bytes (b0 in bits 0..7) containing up to 4 bytes
    - `len` is the number of valid bytes (1..=4)
  - `Machine.inject_keyboard_bytes(bytes)` for arbitrary-length Set-2 byte sequences (e.g. multi-byte keys like PrintScreen/Pause).
- Mouse motion + wheel: `Machine.inject_mouse_motion(dx, dy, wheel)`
  - `dx`/`dy` use browser-style coordinates: +X is right, +Y is down.
  - `wheel` uses PS/2 convention: positive is wheel up.
  - Optional convenience for PS/2 coordinate conventions: `Machine.inject_ps2_mouse_motion(dx, dy, wheel)` where +Y is up.
- Mouse buttons:
  - `Machine.inject_mouse_button(button, pressed)` uses DOM `MouseEvent.button` mapping:
    - `0`: left, `1`: middle, `2`: right, `3`: back, `4`: forward (other values ignored)
  - `Machine.inject_mouse_buttons_mask(mask)` uses DOM `MouseEvent.buttons` bitmask:
    - bit0 (`0x01`): left, bit1 (`0x02`): right, bit2 (`0x04`): middle, bit3 (`0x08`): back, bit4 (`0x10`): forward (higher bits ignored)
  - Optional alias: `Machine.inject_ps2_mouse_buttons(mask)` (same bit mapping).
  - Convenience helpers also exist: `inject_mouse_left/right/middle/back/forward(pressed)`.
  - For ergonomics, `crates/aero-wasm` also exports enums that mirror these DOM mappings:
    - `MouseButton` (`Left=0`, `Middle=1`, `Right=2`, `Back=3`, `Forward=4`)
    - `MouseButtons` bit values (`Left=1`, `Right=2`, `Middle=4`, `Back=8`, `Forward=16`) which can be OR'd into a mask

##### Virtio-input (paravirtualized)

Virtio-input is disabled by default for backwards compatibility. Enable it at construction time via
`Machine.new_with_input_backends` (see [`Virtio-input injection (WASM-facing)`](#virtio-input-injection-wasm-facing)).

- Keyboard (Linux input key codes): `Machine.inject_virtio_key(linux_key, pressed)`
- Mouse (relative):
  - motion: `Machine.inject_virtio_rel(dx, dy)` (alias: `Machine.inject_virtio_mouse_rel(dx, dy)`)
  - buttons: `Machine.inject_virtio_button(btn, pressed)` (Linux `BTN_*` codes; alias: `Machine.inject_virtio_mouse_button(btn, pressed)`)
  - wheel: `Machine.inject_virtio_wheel(delta)` (`delta > 0` = wheel up; alias: `Machine.inject_virtio_mouse_wheel(delta)`)
  - horizontal wheel: `Machine.inject_virtio_hwheel(delta)` (`delta > 0` = wheel right)
  - combined: `Machine.inject_virtio_wheel2(wheel, hwheel)` (single `SYN_REPORT`)

Driver status helpers exist for routing decisions:

- `Machine.virtio_input_keyboard_driver_ok()`
- `Machine.virtio_input_mouse_driver_ok()`

These calls are only meaningful once the guest driver has finished initialization (i.e. after the guest sets `DRIVER_OK`).

Keyboard LED state helpers exist for host/UI diagnostics and input parity checks:

- `Machine.usb_hid_keyboard_leds()` — synthetic USB HID keyboard (last `SET_REPORT` output report)
- `Machine.virtio_input_keyboard_leds()` — virtio-input keyboard (last `statusq` LED events)
- `Machine.ps2_keyboard_leds()` — PS/2 i8042 keyboard (last `Set LEDs` command)

All three return the same HID-style LED bitmask layout:

- bit0: Num Lock
- bit1: Caps Lock
- bit2: Scroll Lock
- bit3: Compose
- bit4: Kana

Synthetic USB HID readiness helpers exist for routing decisions:

- `Machine.usb_hid_keyboard_configured()`
- `Machine.usb_hid_mouse_configured()`
- `Machine.usb_hid_gamepad_configured()`
- `Machine.usb_hid_consumer_control_configured()`

These reflect whether each synthetic HID device is present **and configured** by the guest (`SET_CONFIGURATION != 0`).

##### USB HID (synthetic devices behind the external hub)

In the production browser runtime, browser keyboard/mouse/gamepad input (including consumer-control
“media keys”) can be exposed to the guest as **synthetic USB HID devices behind the external
hub** (inbox Win7 drivers).

For the full-system `Machine` wrapper, synthetic USB HID injection is available via:

- Keyboard (USB HID usage IDs, Usage Page 0x07): `Machine.inject_usb_hid_keyboard_usage(usage, pressed)`
- Consumer Control (USB HID usage IDs, Usage Page 0x0C): `Machine.inject_usb_hid_consumer_usage(usage, pressed)`
- Mouse:
  - motion: `Machine.inject_usb_hid_mouse_move(dx, dy)` (`dy > 0` = down)
  - buttons: `Machine.inject_usb_hid_mouse_buttons(mask)` (low bits match DOM `MouseEvent.buttons`)
  - wheel: `Machine.inject_usb_hid_mouse_wheel(delta)` (`delta > 0` = wheel up)
  - horizontal wheel: `Machine.inject_usb_hid_mouse_hwheel(delta)` (`delta > 0` = wheel right / AC Pan)
  - combined: `Machine.inject_usb_hid_mouse_wheel2(wheel, hwheel)` (single report)
- Gamepad: `Machine.inject_usb_hid_gamepad_report(packed_lo, packed_hi)` (matches `apps/web/src/input/gamepad.ts` packing)

In the production worker runtime, input is typically translated into USB HID reports using the WASM export `UsbHidBridge`:

- Keyboard (USB HID usage IDs, Usage Page 0x07): `UsbHidBridge.keyboard_event(usage, pressed)`
- Consumer Control (USB HID usage IDs, Usage Page 0x0C): `UsbHidBridge.consumer_event(usage, pressed)`
- Mouse:
  - motion: `UsbHidBridge.mouse_move(dx, dy)` (`dy > 0` = down)
  - buttons: `UsbHidBridge.mouse_buttons(mask)` (low bits match DOM `MouseEvent.buttons`)
  - wheel: `UsbHidBridge.mouse_wheel(delta)` (`delta > 0` = wheel up)
  - horizontal wheel: `UsbHidBridge.mouse_hwheel(delta)` (`delta > 0` = wheel right / AC Pan; optional for older WASM builds)
  - combined: `UsbHidBridge.mouse_wheel2(wheel, hwheel)` (single report; optional for older WASM builds)
- Gamepad: `UsbHidBridge.gamepad_report(packed_lo, packed_hi)` (matches `apps/web/src/input/gamepad.ts` packing)

See [`USB HID (Optional)`](#usb-hid-optional) for the guest-visible external hub topology + reserved ports.

Note: The legacy 3-byte PS/2 packet format only carries left/right/middle in the first status byte.
Back/forward are only emitted to the guest in PS/2 stream packets if the guest enables the
IntelliMouse Explorer (5-button) extension (device ID `0x04`).

Example:

```ts
// `initWasm` is the browser/worker WASM loader in `apps/web/src/runtime/wasm_loader.ts`.
const { api } = await initWasm({ variant: "single" });
const machine = new api.Machine(64 * 1024 * 1024);

machine.inject_browser_key("KeyA", true);
machine.inject_browser_key("KeyA", false);

machine.inject_mouse_motion(10, 5, 1); // dx=+10, dy=+5, wheel=+1 (up)
machine.inject_mouse_button(0, true); // left down
machine.inject_mouse_button(0, false); // left up
machine.inject_mouse_buttons_mask(0x01 | 0x02); // left+right held
machine.inject_mouse_buttons_mask(0x00); // release all
```

If the WASM build exports `MouseButton`/`MouseButtons`, callers can avoid hardcoding numeric values:

```ts
const MB = api.MouseButton ?? { Left: 0, Middle: 1, Right: 2, Back: 3, Forward: 4 };
const MBS = api.MouseButtons ?? { Left: 1, Right: 2, Middle: 4, Back: 8, Forward: 16 };

machine.inject_mouse_button(MB.Left, true);
machine.inject_mouse_buttons_mask(MBS.Left | MBS.Right);
```

Note: the PS/2 mouse model only emits movement packets when mouse reporting is enabled (e.g. after
the guest sends `0xF4` via the i8042 “write to mouse” command). Most OS drivers enable this during
boot; very early bare-metal tests may need to do so explicitly.

#### Virtio-input injection (WASM-facing)

For best performance and lowest complexity on the host side, Aero also supports a
**paravirtualized virtio-input** path (keyboard + mouse) via the canonical
`aero_machine::Machine` exported to JS as `crates/aero-wasm::Machine`.

To keep `new api.Machine(ramSizeBytes)` backwards-compatible, virtio-input is
**disabled by default**. Enable it at construction time with
`Machine.new_with_input_backends(...)`:

```ts
const machine = api.Machine.new_with_input_backends(64 * 1024 * 1024, true, false);
```

Once enabled, JS callers can inject Linux `evdev`-style event codes directly:

- Keyboard: `Machine.inject_virtio_key(linux_key, pressed)` (e.g. `KEY_A`)
- Mouse movement: `Machine.inject_virtio_rel(dx, dy)` (`REL_X`/`REL_Y`; alias: `Machine.inject_virtio_mouse_rel(dx, dy)`)
- Mouse buttons: `Machine.inject_virtio_button(btn, pressed)` (e.g. `BTN_LEFT`; alias: `Machine.inject_virtio_mouse_button(btn, pressed)`)
- Mouse wheel: `Machine.inject_virtio_wheel(delta)` (`REL_WHEEL`; alias: `Machine.inject_virtio_mouse_wheel(delta)`)
- Mouse horizontal wheel: `Machine.inject_virtio_hwheel(delta)` (`REL_HWHEEL`)
- Mouse wheel (combined): `Machine.inject_virtio_wheel2(wheel, hwheel)` (`REL_WHEEL` + `REL_HWHEEL`)

`dy > 0` means down (Linux `REL_Y`, matches browser coordinates).

The definitive virtio-input device contract for Aero (transport + queues + event
codes) is specified in
[`../specs/windows7-virtio-driver-contract.md`](../specs/windows7-virtio-driver-contract.md).

For a single end-to-end “do these steps” validation checklist (device model + Win7 driver + web runtime routing), see:

- [`virtio-input-test-plan.md`](windows-drivers.md)

Physical device passthrough (optional): on Chromium-based browsers, Aero can also (optionally) attach a **real host-connected device** to the guest via WebHID/WebUSB. See:

- [`usb-and-input.md`](usb-and-input.md)
- [`usb-and-input.md`](usb-and-input.md)

For the canonical USB stack selection for the browser runtime, see [Canonical USB stack](../decisions/0015-canonical-usb-stack.md).

---

### Input Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Input Stack                                   │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Browser Events                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  KeyboardEvent, MouseEvent, PointerEvent, GamepadEvent   │    │
│  └─────────────────────────────────────────────────────────┘    │
│       │                                                          │
│       ▼                                                          │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  Input Capture Layer                                     │    │
│  │    - Pointer Lock API (mouse capture)                    │    │
│  │    - Keyboard Lock API (best-effort reserved key capture)│    │
│  │    - Keyboard event handling                             │    │
│  │    - Gamepad API                                         │    │
│  └─────────────────────────────────────────────────────────┘    │
│       │                                                          │
│       ▼                                                          │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  Translation + Routing                                   │    │
│  │    - Browser keyCode → PS/2 scancode (early boot)         │    │
│  │    - Browser keyCode → virtio-input events (fast path)    │    │
│  │    - Mouse movement → PS/2 packets or virtio-input REL_*  │    │
│  │    - USB HID report generation (optional)                 │    │
│  └─────────────────────────────────────────────────────────┘    │
│       │                                                          │
│       ▼                                                          │
│  ┌────────────────────┐  ┌────────────────────┐  ┌─────────────┐│
│  │   PS/2 Controller  │  │   USB Controller   │  │ virtio-input ││
│  │   (i8042)          │  │ (UHCI/EHCI/xHCI)   │  │ (kbd + mouse)││
│  └────────────────────┘  └────────────────────┘  └─────────────┘│
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

### IRQ semantics (browser runtime)

Input devices ultimately notify the guest via IRQ lines (IRQ1/IRQ12 for PS/2,
PCI INTx for USB host controllers like UHCI/EHCI/xHCI, etc). In the browser runtime these are delivered as
refcounted *line level* transitions (`raiseIrq` / `lowerIrq`). Edge-triggered
sources are represented as explicit pulses (0→1→0).

See [`platform-and-firmware.md`](platform-and-firmware.md) for the canonical contract and
guardrails (underflow/overflow behaviour, wire-OR semantics, and tests).

### Snapshot/Restore (Save States)

Input snapshots must preserve any **pending bytes** that the guest has not yet consumed, along with controller/device command state.

#### What must be captured

 - **USB (UHCI/EHCI/xHCI + hub/HID devices)**
   - controller register state and per-port timers/flags (per-controller)
   - USB hub device state (per-port status/change bits and reset timers)
   - USB HID device state (address/config, endpoint halt bits, protocol/idle state)
   - For passthrough HID devices: pending **input report** queue (so restoring mid-session doesn’t drop buffered input)

- **i8042 controller**
  - status register and command byte
  - pending controller command (if awaiting a data byte)
  - output buffer contents (bytes queued for port `0x60`)
  - **IRQ behavior**: i8042 IRQ1/IRQ12 are edge-triggered in Aero’s model. The controller device
    should not snapshot an “IRQ level”; pending interrupts should be captured by the interrupt
    controller (PIC/APIC) state. On restore, the i8042 device must avoid emitting spurious IRQ
    pulses for already-buffered output bytes (see [`platform-and-firmware.md`](platform-and-firmware.md)).
- **PS/2 keyboard and mouse**
  - mode/configuration (scancode set, LEDs, sample rate, resolution, scaling)
  - command parsing state (e.g. “expecting data”)
  - device output queues

This ensures that a snapshot taken between a host keypress and guest consumption will restore deterministically.

### PS/2 Controller (i8042)

> IRQ signaling semantics (edge vs level) in the browser runtime are documented in
> [`platform-and-firmware.md`](platform-and-firmware.md).

#### Controller Emulation

> Note: the canonical i8042 + PS/2 keyboard/mouse device model lives in
> [`crates/aero-devices-input`](../../crates/aero-devices-input) as
> `aero_devices_input::I8042Controller` so it can be shared by both native tests
> and WASM builds. The Rust snippet below is illustrative pseudocode, not the
> exact implementation.

```rust
pub struct I8042Controller {
    // Status register
    status: u8,
    
    // Command byte
    command_byte: u8,
    
    // Output buffer
    output_buffer: u8,
    output_source: OutputSource,
    
    // Input buffer (for commands)
    input_buffer: u8,
    
    // Pending command
    pending_command: Option<u8>,
    
    // Devices
    keyboard: Ps2Keyboard,
    mouse: Ps2Mouse,
    
    // IRQ state
    keyboard_irq_pending: bool,
    mouse_irq_pending: bool,
}

// Status register bits
const STATUS_OBF: u8 = 0x01;   // Output Buffer Full
const STATUS_IBF: u8 = 0x02;   // Input Buffer Full
const STATUS_SYS: u8 = 0x04;   // System flag
const STATUS_A2: u8 = 0x08;    // Address line A2
const STATUS_INH: u8 = 0x10;   // Inhibit flag
const STATUS_MOBF: u8 = 0x20;  // Mouse Output Buffer Full
const STATUS_TOUT: u8 = 0x40;  // Timeout error
const STATUS_PERR: u8 = 0x80;  // Parity error

impl I8042Controller {
    pub fn read_port(&mut self, port: u16) -> u8 {
        match port {
            0x60 => {
                // Data port - read output buffer
                self.status &= !STATUS_OBF;
                self.output_buffer
            }
            0x64 => {
                // Status port
                self.status
            }
            _ => 0xFF,
        }
    }
    
    pub fn write_port(&mut self, port: u16, value: u8) {
        match port {
            0x60 => {
                // Data port
                if let Some(cmd) = self.pending_command.take() {
                    self.execute_controller_command_data(cmd, value);
                } else {
                    // Send to keyboard
                    self.send_to_keyboard(value);
                }
            }
            0x64 => {
                // Command port
                self.execute_controller_command(value);
            }
            _ => {}
        }
    }
    
    fn execute_controller_command(&mut self, cmd: u8) {
        match cmd {
            0x20 => {
                // Read command byte
                self.set_output(self.command_byte, OutputSource::Controller);
            }
            0x60 => {
                // Write command byte (next byte)
                self.pending_command = Some(cmd);
            }
            0xA7 => {
                // Disable mouse port
                self.command_byte |= 0x20;
            }
            0xA8 => {
                // Enable mouse port
                self.command_byte &= !0x20;
            }
            0xA9 => {
                // Test mouse port
                self.set_output(0x00, OutputSource::Controller);  // Pass
            }
            0xAA => {
                // Self test
                self.set_output(0x55, OutputSource::Controller);  // Pass
            }
            0xAB => {
                // Test keyboard port
                self.set_output(0x00, OutputSource::Controller);  // Pass
            }
            0xAD => {
                // Disable keyboard
                self.command_byte |= 0x10;
            }
            0xAE => {
                // Enable keyboard
                self.command_byte &= !0x10;
            }
            0xD4 => {
                // Write to mouse (next byte)
                self.pending_command = Some(cmd);
            }
            _ => {
                log::debug!("Unknown i8042 command: {:02x}", cmd);
            }
        }
    }
    
    fn set_output(&mut self, value: u8, source: OutputSource) {
        self.output_buffer = value;
        self.output_source = source;
        self.status |= STATUS_OBF;
        
        match source {
            OutputSource::Keyboard => {
                if self.command_byte & 0x01 != 0 {
                    self.keyboard_irq_pending = true;
                }
            }
            OutputSource::Mouse => {
                self.status |= STATUS_MOBF;
                if self.command_byte & 0x02 != 0 {
                    self.mouse_irq_pending = true;
                }
            }
            OutputSource::Controller => {}
        }
    }
}
```

#### PS/2 Keyboard

```rust
pub struct Ps2Keyboard {
    // Scancode set (1, 2, or 3)
    scancode_set: u8,
    
    // LED state
    leds: u8,
    
    // Typematic settings
    typematic_delay: u8,
    typematic_rate: u8,
    
    // Output queue
    output_queue: VecDeque<u8>,
    
    // Command state
    expecting_data: bool,
    last_command: u8,
}

impl Ps2Keyboard {
    pub fn handle_command(&mut self, cmd: u8) -> Option<u8> {
        if self.expecting_data {
            self.expecting_data = false;
            return self.handle_command_data(cmd);
        }
        
        match cmd {
            0xED => {
                // Set LEDs (next byte contains LED state)
                self.expecting_data = true;
                self.last_command = cmd;
                Some(0xFA)  // ACK
            }
            0xEE => {
                // Echo
                Some(0xEE)
            }
            0xF0 => {
                // Get/Set scancode set
                self.expecting_data = true;
                self.last_command = cmd;
                Some(0xFA)
            }
            0xF2 => {
                // Identify keyboard
                self.output_queue.push_back(0xFA);
                self.output_queue.push_back(0xAB);
                self.output_queue.push_back(0x83);
                None
            }
            0xF3 => {
                // Set typematic rate/delay
                self.expecting_data = true;
                self.last_command = cmd;
                Some(0xFA)
            }
            0xF4 => {
                // Enable scanning
                Some(0xFA)
            }
            0xF5 => {
                // Disable scanning
                Some(0xFA)
            }
            0xF6 => {
                // Set default parameters
                self.scancode_set = 2;
                self.typematic_delay = 0x0B;
                self.typematic_rate = 0x0B;
                Some(0xFA)
            }
            0xFF => {
                // Reset
                self.output_queue.push_back(0xFA);
                self.output_queue.push_back(0xAA);  // Self-test passed
                None
            }
            _ => {
                log::debug!("Unknown keyboard command: {:02x}", cmd);
                Some(0xFA)
            }
        }
    }
    
    pub fn key_event(&mut self, scancode: u8, pressed: bool) {
        match self.scancode_set {
            1 => {
                // Set 1: break = make | 0x80
                if pressed {
                    self.output_queue.push_back(scancode);
                } else {
                    self.output_queue.push_back(scancode | 0x80);
                }
            }
            2 => {
                // Set 2: break = 0xF0, make
                if pressed {
                    self.output_queue.push_back(scancode);
                } else {
                    self.output_queue.push_back(0xF0);
                    self.output_queue.push_back(scancode);
                }
            }
            _ => {}
        }
    }
}
```

#### PS/2 Mouse

```rust
pub struct Ps2Mouse {
    // Mouse mode
    mode: MouseMode,
    
    // Resolution (counts per mm)
    resolution: u8,
    
    // Sample rate
    sample_rate: u8,
    
    // Scaling
    scaling: Scaling,
    
    // Button state
    buttons: u8,
    
    // Accumulated movement
    dx: i32,
    dy: i32,
    dz: i32,  // Scroll wheel
    
    // Output queue
    output_queue: VecDeque<u8>,
    
    // Command state
    expecting_data: bool,
    last_command: u8,
}

#[derive(Clone, Copy, PartialEq)]
pub enum MouseMode {
    Stream,
    Remote,
    Wrap,
}

impl Ps2Mouse {
    pub fn handle_command(&mut self, cmd: u8) -> Option<u8> {
        if self.expecting_data {
            self.expecting_data = false;
            return self.handle_command_data(cmd);
        }
        
        match cmd {
            0xE6 => {
                // Set scaling 1:1
                self.scaling = Scaling::Linear;
                Some(0xFA)
            }
            0xE7 => {
                // Set scaling 2:1
                self.scaling = Scaling::Double;
                Some(0xFA)
            }
            0xE8 => {
                // Set resolution
                self.expecting_data = true;
                self.last_command = cmd;
                Some(0xFA)
            }
            0xE9 => {
                // Status request
                self.output_queue.push_back(0xFA);
                self.output_queue.push_back(self.get_status_byte());
                self.output_queue.push_back(self.resolution);
                self.output_queue.push_back(self.sample_rate);
                None
            }
            0xEA => {
                // Set stream mode
                self.mode = MouseMode::Stream;
                Some(0xFA)
            }
            0xEB => {
                // Read data (remote mode)
                self.send_movement_packet();
                Some(0xFA)
            }
            0xEC => {
                // Reset wrap mode
                Some(0xFA)
            }
            0xEE => {
                // Set wrap mode
                self.mode = MouseMode::Wrap;
                Some(0xFA)
            }
            0xF0 => {
                // Set remote mode
                self.mode = MouseMode::Remote;
                Some(0xFA)
            }
            0xF2 => {
                // Get device ID
                self.output_queue.push_back(0xFA);
                self.output_queue.push_back(self.get_device_id());
                None
            }
            0xF3 => {
                // Set sample rate
                self.expecting_data = true;
                self.last_command = cmd;
                Some(0xFA)
            }
            0xF4 => {
                // Enable data reporting
                Some(0xFA)
            }
            0xF5 => {
                // Disable data reporting
                Some(0xFA)
            }
            0xF6 => {
                // Set defaults
                self.resolution = 4;
                self.sample_rate = 100;
                self.scaling = Scaling::Linear;
                Some(0xFA)
            }
            0xFF => {
                // Reset
                self.output_queue.push_back(0xFA);
                self.output_queue.push_back(0xAA);
                self.output_queue.push_back(0x00);  // Device ID
                None
            }
            _ => {
                log::debug!("Unknown mouse command: {:02x}", cmd);
                Some(0xFA)
            }
        }
    }
    
    pub fn movement(&mut self, dx: i32, dy: i32, dz: i32) {
        self.dx += dx;
        self.dy += dy;
        self.dz += dz;
        
        if self.mode == MouseMode::Stream {
            self.send_movement_packet();
        }
    }
    
    pub fn button_event(&mut self, button: u8, pressed: bool) {
        if pressed {
            self.buttons |= button;
        } else {
            self.buttons &= !button;
        }
        
        if self.mode == MouseMode::Stream {
            self.send_movement_packet();
        }
    }
    
    fn send_movement_packet(&mut self) {
        // Clamp movement
        let dx = self.dx.clamp(-256, 255);
        let dy = self.dy.clamp(-256, 255);
        
        // Build packet
        let mut byte0 = self.buttons & 0x07;  // Buttons
        byte0 |= 0x08;  // Always 1
        
        if dx < 0 {
            byte0 |= 0x10;  // X sign
        }
        if dy < 0 {
            byte0 |= 0x20;  // Y sign (inverted in PS/2)
        }
        
        self.output_queue.push_back(byte0);
        self.output_queue.push_back((dx & 0xFF) as u8);
        self.output_queue.push_back(((-dy) & 0xFF) as u8);  // Y is inverted
        
        // Scroll wheel for IntelliMouse
        if self.get_device_id() >= 3 {
            self.output_queue.push_back((self.dz.clamp(-8, 7) & 0x0F) as u8);
        }
        
        // Clear accumulated movement
        self.dx = 0;
        self.dy = 0;
        self.dz = 0;
    }
}
```

---

### Browser Event Capture

#### TypeScript Host Input Capture (Implemented)

The repository includes a concrete browser-side input capture implementation:

- `apps/web/src/input/input_capture.ts` — attaches listeners to the emulator canvas, manages focus/blur, and requests Pointer Lock on click.
- `apps/web/src/input/pointer_lock.ts` — minimal Pointer Lock state machine.
- `apps/web/src/input/event_queue.ts` — allocation-free event queue and batching transport to the input injector worker (`io.worker.ts` in `vmRuntime=legacy`, `machine_cpu.worker.ts` in `vmRuntime=machine`).
- `apps/web/src/input/scancodes.ts` — auto-generated `KeyboardEvent.code` → PS/2 Set 2 scancode mapping (including multi-byte sequences like PrintScreen/Pause).
- `apps/web/src/input/scancode.ts` — small helpers (allocation-free lookup + browser `preventDefault` policy).
  - The default policy prevents browser/UI actions while the VM is focused (e.g. function keys like **F5** refresh, **Alt** menu/address-bar focus, browser keys like **BrowserBack**/**BrowserSearch**).
  - `Meta` shortcuts are intentionally *not* prevented by default so host shortcuts can win when desired.
    - Exception: browser navigation keys like `BrowserBack`/`BrowserSearch` are always swallowed to avoid navigating away from the VM.
  - `Ctrl`-only shortcuts are also not prevented by default (copy/paste/etc.), but `Ctrl+Alt` (often reported for **AltGr** layouts) is treated as a capture combination and prevented by default so it reaches the guest reliably.
- Input handlers that swallow events call **both** `event.preventDefault()` and `event.stopPropagation()` so that:
  - the browser does not perform its default action (scroll, navigate, etc.), and
  - other app-level/global listeners do not observe the event while the VM is actively capturing input.

##### Workers panel VM canvas UX (`InputCapture`)

The primary interactive UI is the **Workers panel** VM canvas:

- The VGA canvas element is created/rendered by `apps/web/src/main.ts::renderWorkersPanel`.
- That canvas is wired through `apps/web/src/input/input_capture.ts` (`InputCapture`), and flushed to the active input injector worker (I/O worker in `vmRuntime=legacy`, CPU worker in `vmRuntime=machine`).

Expected user interaction:

- **Click the VM canvas** to focus it and request pointer lock.
  - Pointer lock is required for relative mouse deltas (`movementX`/`movementY`) and to prevent the cursor from leaving the canvas.
- While capture is active, **keyboard / mouse / gamepad** input is batched and forwarded to the active input injector worker.
- To **exit pointer lock**, press **Escape** (browser default).
  - Optionally, hosts can configure a *host-only* pointer-lock release chord via `InputCaptureOptions.releasePointerLockChord`.
    - When set, the chord is swallowed (not forwarded to the guest) and the matching keyup is also suppressed.
- On **blur** (canvas blur or window blur) or **page visibility change** (`document.visibilityState === "hidden"`),
  `InputCapture` exits pointer lock and performs an immediate **release-all flush**:
  - emits "key up" for any pressed keys,
  - sets mouse buttons to `0`,
  - emits a neutral gamepad report (if enabled),
  - flushes the batch immediately so the guest cannot get stuck keys/buttons while the tab is backgrounded.

##### Worker Transport / Wire Format

Input batches are delivered to the **input injector worker** via `postMessage` with:

```ts
{ type: 'in:input-batch', buffer: ArrayBuffer, recycle?: true }
```

On the receiving side, batches are handled by:

- `apps/web/src/workers/io.worker.ts` in `vmRuntime=legacy`
- `apps/web/src/workers/machine_cpu.worker.ts` in `vmRuntime=machine`

(see the `"in:input-batch"` message case and `handleInputBatch(...)` in each worker).

`buffer` contains a small `Int32Array`-compatible payload:

| Word | Meaning |
|------|---------|
| 0 | `count` (number of events) |
| 1 | `batchSendTimestampUs` (u32, `performance.now()*1000`, wraps) |
| 2.. | `count` events, each 4 words: `[type, eventTimestampUs, a, b]` |

Event types are defined in `apps/web/src/input/event_queue.ts` (`InputEventType`):

- `KeyScancode (1)`: `a=packedBytesLE`, `b=byteLen` (PS/2 Set 2 bytes including `0xE0`/`0xF0`). Long sequences are split across multiple `KeyScancode` events in-order (max 4 bytes per event).
- `KeyHidUsage (6)`: `a=(usage & 0xFF) | ((pressed ? 1 : 0) << 8)`, `b=unused` (USB HID keyboard usage events on Usage Page 0x07). Emitted in addition to `KeyScancode` so the runtime can drive both PS/2 and USB HID paths from the same captured input.
- `HidUsage16 (7)`: `a=(usagePage & 0xFFFF) | ((pressed ? 1 : 0) << 16)`, `b=usageId & 0xFFFF` (e.g. Consumer Control / media keys on Usage Page `0x0C`)
- `MouseMove (2)`: `a=dx`, `b=dy` (PS/2 coords: `dx` right, `dy` up)
- `MouseButtons (3)`: `a=buttons` (bit0..bit7 = buttons 1..8; DOM mapping typically uses bit0..bit4 for left/right/middle/back/forward, with bit5+ as additional buttons)
- `MouseWheel (4)`: `a=dz` (positive=wheel up), `b=dx` (positive=wheel right / horizontal scroll; used as `REL_HWHEEL` on virtio-input)
- `GamepadReport (5)`: `a=packedBytes0to3LE`, `b=packedBytes4to7LE` (8-byte USB HID gamepad input report; see `apps/web/src/input/gamepad.ts` for packing and `crates/aero-usb/src/hid/gamepad.rs::GamepadReport` for the canonical report layout)

This keeps the hot path allocation-free and allows the worker to convert to
i8042 keyboard/mouse bytes with minimal overhead.

###### Optional buffer recycling

If `recycle: true` is set on the batch, the worker may transfer the same buffer
back to the sender once processed:

```ts
{ type: 'in:input-batch-recycle', buffer: ArrayBuffer }
```

This avoids allocating a new `ArrayBuffer` per flush on the main thread.

##### Capture lifecycle (focus / pointer lock / release-all flush)

See `apps/web/src/input/input_capture.ts` for the exact event listeners and gating conditions. At a high level:

- Capture becomes active when the window is focused, the page is visible, and the canvas is focused **or** pointer lock is active.
- `keydown` / `keyup` events are listened on `window` (capture phase) and translated into:
  - PS/2 Set-2 scancode bytes (`InputEventType.KeyScancode`),
  - USB HID keyboard usages (`InputEventType.KeyHidUsage`), and
  - additional HID usages on other pages (`InputEventType.HidUsage16`, e.g. Consumer Control / media keys),
  then enqueued into `InputEventQueue`.
  - The worker that injects input (`vmRuntime=legacy`: I/O worker; `vmRuntime=machine`: machine CPU worker) decides which events to consume based on the active backend (PS/2 vs USB vs virtio), to avoid duplicates.
- `mousemove` events are listened on `document` (capture phase) while pointer lock is active and forwarded as relative deltas (`MouseMove`).
  - Y is inverted once in `InputCapture` so `MouseMove` is already in PS/2 coordinate space (positive is up).
- `wheel` events are forwarded as `MouseWheel` with both vertical (`dz`) and horizontal (`dx`) scroll components.
- On blur / hidden-page, `InputCapture` emits a release-all snapshot and flushes immediately (see above).

##### Consumption + routing in the worker runtime

###### Legacy runtime (`vmRuntime=legacy`, I/O worker injects input)

Input batches are consumed in the I/O worker (`apps/web/src/workers/io.worker.ts`) by handling `message` events with `type: "in:input-batch"`.

The worker decodes the `InputEventType` stream and routes each event into the currently selected guest-facing backend (see `maybeUpdateKeyboardInputBackend` / `maybeUpdateMouseInputBackend`):

- **PS/2 fallback**: i8042 + PS/2 keyboard/mouse (`crates/aero-devices-input` is the canonical model; the browser runtime uses an equivalent bridge/model).
- **virtio-input fast path**: routed to the virtio-input PCI functions (once the guest sets `DRIVER_OK`).
- **USB HID**: routed to synthetic USB HID devices behind the external hub on root port 0 (or to passthrough devices when enabled).

###### Machine runtime (`vmRuntime=machine`, CPU worker injects input)

In machine runtime, input batches are consumed by the machine CPU worker (`apps/web/src/workers/machine_cpu.worker.ts`) and injected into the canonical `api.Machine` instance.

> Note: machine runtime reuses the same high-level backend selection policy (virtio-input → USB HID → PS/2), but the routing is implemented in `machine_cpu.worker.ts` (not the legacy I/O worker).

##### Scancode Translation

```rust
// Scancode translation is generated from a single source-of-truth table:
//
//   tools/gen_scancodes/scancodes.json
//
// Outputs:
//   - apps/web/src/input/scancodes.ts
//   - crates/aero-devices-input/src/scancodes_generated.rs
//
// This keeps the JS capture side and Rust/WASM side in sync, including extended
// keys (0xE0 prefix) and special multi-byte sequences like PrintScreen/Pause.
use aero_devices_input::scancode::browser_code_to_set2_bytes;

pub fn key_event_bytes(code: &str, pressed: bool) -> Option<Vec<u8>> {
    browser_code_to_set2_bytes(code, pressed)
}
```

---

### USB HID (Optional)

For browser input → USB HID usage mapping and report format details, see
[`usb-and-input.md`](usb-and-input.md).

For EHCI (USB 2.0) host controller emulation design notes and emulator/runtime contracts (regs,
root hub ports, async/periodic schedules, IRQ and snapshot requirements), see
[`usb-and-input.md`](usb-and-input.md).

For xHCI (USB 3.x) host controller scaffolding, PCI identity, and current limitations, see
[`usb-and-input.md`](usb-and-input.md).
#### External hub topology (reserved ports)

When USB input is enabled, Aero uses a **fixed, guest-visible topology** so synthetic devices and
passthrough devices can coexist without fighting over port numbers:

- **Root ports (guest-visible USB controller, 0-based):**
  - root port **0**: external USB hub (synthetic HID devices + WebHID passthrough)
  - root port **1**: reserved for **WebUSB passthrough**
  - UHCI exposes exactly two root ports (0–1). EHCI/xHCI expose more root ports by default, but the
    browser/WASM integration treats ports 0–1 as reserved for these roles so WebUSB passthrough can
    coexist with the external hub / WebHID / synthetic HID devices (including in WASM builds that
    omit UHCI).
  - Note: for xHCI, the WASM bridge enforces the WebUSB reserved root port (typically root port 1).
    The xHCI WebHID topology manager also follows the same `root port 0 external hub / root port 1
    WebUSB` convention: it rejects device attachments behind the reserved WebUSB root port and
    remaps legacy root-port-only paths (`[0]` / `[1]`) onto stable hub-backed paths behind root port
    0.
- **External hub on root port 0**:
  - default downstream port count: **16** (UHCI). For xHCI-backed topologies, keep hub port counts
    <= **15** (xHCI Route String encodes hub ports as 4-bit values).
  - reserved synthetic devices:
    - hub port **1**: USB HID keyboard
    - hub port **2**: USB HID mouse
    - hub port **3**: USB HID gamepad
    - hub port **4**: USB HID consumer-control (media keys)
  - dynamic passthrough allocation starts at hub port **5** (`UHCI_EXTERNAL_HUB_FIRST_DYNAMIC_PORT`)

Source-of-truth constants live in:

- `apps/web/src/usb/uhci_external_hub.ts`
- `crates/aero-machine/src/lib.rs` (`aero_machine::Machine::UHCI_*`)

##### Does the canonical `aero_machine::Machine` auto-attach these devices?

Yes, when `MachineConfig.enable_synthetic_usb_hid = true` (or when constructing via
`aero-wasm::Machine.new_with_input_backends(..., enableSyntheticUsbHid=true)`).

`MachineConfig.enable_uhci` by itself only attaches the UHCI controller (PCI `00:01.2`). Enabling
synthetic USB HID causes the canonical machine to attach the external hub on UHCI root port 0 and
attach the synthetic HID devices on hub ports 1..=4, matching the reserved port layout above.

In the legacy browser runtime (`vmRuntime=legacy`), the I/O worker (and the worker-side UHCI runtime) attaches the external hub + synthetic devices
according to the reserved port layout above.

#### Browser runtime wiring (current implementation)

This section describes the legacy browser worker runtime (`vmRuntime=legacy`), where the guest USB stack is owned by the I/O worker.

In `vmRuntime=machine`, the I/O worker runs in a host-only stub mode (it does not own guest device models). The guest-visible USB stack (including the external hub + synthetic HID devices) lives inside the canonical `api.Machine` instance owned by the machine CPU worker. Synthetic HID input injection is supported in machine runtime, but WebHID/WebUSB passthrough is not yet wired up.

In the legacy web runtime, browser keyboard/mouse/gamepad events can be exposed to the guest as **guest-visible USB HID devices**
(inbox drivers on Windows 7).

Current implementation details:

- The I/O worker attaches an **external USB hub** on root port 0 (UHCI by default; EHCI/xHCI when
  available) and then attaches four
  fixed "synthetic" USB HID devices behind it:
  - hub port 1: USB keyboard (boot protocol)
  - hub port 2: USB mouse (boot protocol; report protocol includes wheel + horizontal wheel / AC Pan)
  - hub port 3: USB gamepad (Aero's fixed 8-byte report)
  - hub port 4: USB consumer-control (media keys)
  - See: `apps/web/src/usb/uhci_external_hub.ts`, `apps/web/src/workers/io.worker.ts`
- Browser input capture emits both PS/2 scancodes and HID usage events so the runtime can drive
  multiple backends from the same captured stream.
- The I/O worker dynamically selects an input backend. The switching policy is implemented in:
  - `apps/web/src/input/input_backend_selection.ts`:
    - `chooseKeyboardInputBackend`
    - `chooseMouseInputBackend`
  - Switching is explicitly **gated on held-state** (do not switch while any key/mouse button is held) to avoid stuck key/button states in the guest.
  - Current backend selection order (matches `apps/web/src/workers/io.worker.ts`):
    - **Keyboard:** virtio-input (once the guest sets `DRIVER_OK`) → synthetic USB keyboard (once configured) → PS/2 i8042
    - **Mouse:** virtio-input (once the guest sets `DRIVER_OK`) → PS/2 i8042 while the synthetic USB mouse is unconfigured → synthetic USB mouse (once configured; or if PS/2 is unavailable)
    - **Gamepad:** synthetic USB gamepad (no virtio/PS/2 fallback)

For USB HID **gamepad** details (report descriptor + byte layout), see
[`usb-and-input.md`](usb-and-input.md).

For WebHID device passthrough (where the browser does not expose the raw HID
report descriptor bytes), see
[`usb-and-input.md`](usb-and-input.md).

For the end-to-end “real device” passthrough architecture (main thread owns the
handle; worker models a guest-visible USB controller + a generic HID device), see
[`usb-and-input.md`](usb-and-input.md).

#### USB HID Keyboard

```rust
pub struct UsbHidKeyboard {
    endpoint: u8,
    report: KeyboardReport,
    pending_reports: VecDeque<KeyboardReport>,
}

#[repr(C, packed)]
pub struct KeyboardReport {
    modifiers: u8,     // Ctrl, Shift, Alt, GUI
    reserved: u8,
    keys: [u8; 6],     // Up to 6 simultaneous keys
}

impl UsbHidKeyboard {
    pub fn key_event(&mut self, usage: u8, pressed: bool) {
        if is_modifier(usage) {
            let modifier_bit = modifier_to_bit(usage);
            if pressed {
                self.report.modifiers |= modifier_bit;
            } else {
                self.report.modifiers &= !modifier_bit;
            }
        } else {
            if pressed {
                // Add key to report
                for slot in &mut self.report.keys {
                    if *slot == 0 {
                        *slot = usage;
                        break;
                    }
                }
            } else {
                // Remove key from report
                for slot in &mut self.report.keys {
                    if *slot == usage {
                        *slot = 0;
                    }
                }
                // Compact the array
                self.report.keys.sort_by(|a, b| {
                    if *a == 0 { std::cmp::Ordering::Greater }
                    else if *b == 0 { std::cmp::Ordering::Less }
                    else { std::cmp::Ordering::Equal }
                });
            }
        }
        
        self.pending_reports.push_back(self.report.clone());
    }
    
    pub fn get_descriptor(&self) -> &[u8] {
        // HID Report Descriptor for keyboard
        static DESCRIPTOR: &[u8] = &[
            0x05, 0x01,  // Usage Page (Generic Desktop)
            0x09, 0x06,  // Usage (Keyboard)
            0xA1, 0x01,  // Collection (Application)
            
            // Modifier keys
            0x05, 0x07,  // Usage Page (Keyboard)
            0x19, 0xE0,  // Usage Minimum (Left Control)
            0x29, 0xE7,  // Usage Maximum (Right GUI)
            0x15, 0x00,  // Logical Minimum (0)
            0x25, 0x01,  // Logical Maximum (1)
            0x75, 0x01,  // Report Size (1)
            0x95, 0x08,  // Report Count (8)
            0x81, 0x02,  // Input (Data, Variable, Absolute)
            
            // Reserved byte
            0x95, 0x01,  // Report Count (1)
            0x75, 0x08,  // Report Size (8)
            0x81, 0x01,  // Input (Constant)
            
            // Key array
            0x95, 0x06,  // Report Count (6)
            0x75, 0x08,  // Report Size (8)
            0x15, 0x00,  // Logical Minimum (0)
            0x25, 0x89,  // Logical Maximum (137)
            0x05, 0x07,  // Usage Page (Keyboard)
            0x19, 0x00,  // Usage Minimum (0)
            0x29, 0x89,  // Usage Maximum (137)
            0x81, 0x00,  // Input (Data, Array)
            
            0xC0,        // End Collection
        ];
        DESCRIPTOR
    }
}
```

---

### WebHID/WebUSB passthrough (optional “real devices”)

In addition to synthesizing PS/2 or USB HID events from browser input events,
the host can optionally pass through a **real, host-connected HID device** into
the guest using browser device APIs (WebHID for MVP; WebUSB is available but more limited/experimental).

See [`usb-and-input.md`](usb-and-input.md) for the
intended architecture, security model, and current limitations.

---

### Virtio-input (Paravirtualized Keyboard/Mouse)

Virtio-input is the virtio device for input peripherals. Conceptually it is a stream of small typed events (similar to Linux `evdev`) delivered over virtqueues, rather than a full USB bus + endpoints + HID polling model.

For Aero, virtio-input is the intended “fast path” for keyboard/mouse once the guest driver is installed:

- **Host side is simpler**: no USB host controller state machines, device enumeration, descriptors, periodic interrupt transfers, etc.
- **Lower latency / easier batching**: we can push events as they happen (or coalesce them per frame) into a ring buffer and raise an interrupt, instead of modeling USB frames/microframes.
- **Guest sees standard Windows input**: the custom virtio driver exposes a normal HID keyboard/mouse stack, so applications remain unaware of virtio.

#### Browser events → virtio-input events

Virtio-input uses `virtio_input_event { type, code, value }` entries, with `EV_SYN` used to delimit a coherent “report” (similar to committing a packet).

At a high level we map:

- **Keyboard**
  - `keydown` → `EV_KEY` + `KEY_*` + `value=1`
  - `keyup` → `EV_KEY` + `KEY_*` + `value=0`
  - End of report → `EV_SYN` + `SYN_REPORT` + `value=0`
- **Mouse (relative, pointer-lock)**
  - `mousemove` (`movementX/Y`) → `EV_REL` + `REL_X`/`REL_Y` + signed delta
  - Wheel → `EV_REL` + `REL_WHEEL` (and optionally `REL_HWHEEL`)
  - Buttons (`mousedown`/`mouseup`) → `EV_KEY` + `BTN_LEFT`/`BTN_RIGHT`/`BTN_MIDDLE`… + `value=1/0`
  - End of report → `EV_SYN` + `SYN_REPORT` + `value=0`

Compared to USB HID, this is essentially a direct “event injection” interface: the host translates browser input into a small fixed struct and places it in a queue.

#### Windows 7 guest driver model (HID minidriver)

Windows 7 does not ship a virtio-input driver, so virtio-input requires a custom guest driver that speaks virtio and presents a HID device to the OS.

Intended stack:

```
virtio-input device (PCI / virtqueues)
  → our virtio-input KMDF HID minidriver
    → hidclass.sys
      → kbdhid.sys / mouhid.sys
        → kbdclass.sys / mouclass.sys
```

Responsibilities of the virtio-input driver at a conceptual level:

- Initialize the virtio device and negotiate features.
- Provide buffers for the **eventq** (device → driver) and parse incoming `EV_KEY`/`EV_REL`/`EV_SYN`.
- Convert virtio-input events into **HID input reports** and submit them up to `hidclass.sys`.
- Accept **HID output reports** (keyboard LEDs) and emit virtio-input output events over the **statusq**.

#### Two-queue model (eventq + statusq) and LEDs

Virtio-input uses two virtqueues:

- **eventq**: host/device publishes input events (key presses, relative motion).
- **statusq**: guest/driver publishes output events back to the host (primarily keyboard LED state like Num Lock / Caps Lock / Scroll Lock, plus optional Compose/Kana).

For Aero, we should handle LED output events even if we don’t initially surface them to the browser UI. Keeping the round-trip correct avoids subtle guest driver behavior differences (e.g., toggling Caps Lock producing output reports that must be acknowledged).

#### Recommended device model: multi-function PCI virtio-input (2+ functions)

Contract v1 exposes virtio-input as a **single multi-function PCI device** with two required virtio-input **functions** (and an optional third):

1. Function 0: virtio-input **keyboard** (`SUBSYS 0x0010`, `header_type = 0x80` to advertise multi-function)
2. Function 1: virtio-input **mouse** (relative pointer, `SUBSYS 0x0011`)
3. (Optional) Function 2: virtio-input **tablet** (absolute pointer / `EV_ABS`, `SUBSYS 0x0012`)

This still avoids composite HID device complexity and lets Windows naturally bind the inbox `kbdhid.sys` and `mouhid.sys` clients, while keeping the PCI topology stable for driver matching.

#### `aero_machine::Machine` integration (canonical BDFs)

The canonical full-system VM (`aero_machine::Machine`) can attach virtio-input when:

- `MachineConfig.enable_virtio_input = true` (requires `enable_pc_platform = true`).

Virtio-input is exposed at fixed PCI BDFs (stable across runs/snapshots):

- `00:0A.0` — virtio-input **keyboard** (`aero_devices::pci::profile::VIRTIO_INPUT_KEYBOARD`)
- `00:0A.1` — virtio-input **mouse** (`aero_devices::pci::profile::VIRTIO_INPUT_MOUSE`)
- (Optional) `00:0A.2` — virtio-input **tablet** (`aero_devices::pci::profile::VIRTIO_INPUT_TABLET`, when attached)

#### Testing notes

- **End-to-end test plan:** [`windows-drivers.md`](windows-drivers.md) (Rust conformance tests + browser wiring checks + Win7 driver bring-up).
- **Reference implementation**: validate the guest driver + device model in QEMU first using `virtio-keyboard-pci` and `virtio-mouse-pci` (or `virtio-tablet-pci` if experimenting with absolute coordinates).
  - For strict `AERO-W7-VIRTIO` **contract v1** driver testing, use **modern-only** virtio (`disable-legacy=on`) and force the **contract Revision ID** (`x-pci-revision=0x01`), since many QEMU virtio devices report `REV_00` by default.
  - See: `drivers/windows7/virtio-input/tests/qemu/README.md` for full command lines.
- **Windows 7 driver install**: plan on **test signing** for development (enable test mode and sign the KMDF driver with a test certificate) before tackling production signing/distribution.

---

### Gamepad Support

Gamepads can be surfaced to the guest either via a paravirtualized path (future)
or as a USB HID game controller. For the USB HID gamepad spec used by Aero, see
[`usb-and-input.md`](usb-and-input.md).

```rust
pub struct GamepadHandler {
    gamepads: HashMap<u32, GamepadState>,
}

impl GamepadHandler {
    pub fn poll(&mut self) -> Vec<GamepadEvent> {
        let mut events = Vec::new();
        
        let gamepads = navigator().get_gamepads().unwrap_or_default();
        
        for gamepad in gamepads.iter().filter_map(|g| g) {
            let id = gamepad.index();
            let old_state = self.gamepads.entry(id).or_default();
            
            // Check buttons
            for (i, button) in gamepad.buttons().iter().enumerate() {
                let pressed = button.pressed();
                if pressed != old_state.buttons[i] {
                    events.push(GamepadEvent::Button {
                        gamepad: id,
                        button: i as u8,
                        pressed,
                    });
                    old_state.buttons[i] = pressed;
                }
            }
            
            // Check axes
            for (i, axis) in gamepad.axes().iter().enumerate() {
                let value = *axis as f32;
                if (value - old_state.axes[i]).abs() > 0.01 {
                    events.push(GamepadEvent::Axis {
                        gamepad: id,
                        axis: i as u8,
                        value,
                    });
                    old_state.axes[i] = value;
                }
            }
        }
        
        events
    }
}
```

---

### Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Input Latency | < 16ms | One frame at 60 FPS |
| Key Repeat | Configurable | Match Windows settings |
| Mouse Resolution | ≥ 1000 DPI | High precision |
| Poll Rate | 125-1000 Hz | USB standard rates |

---

### Next Steps

- See [BIOS/Firmware](platform-and-firmware.md) for system initialization
- See [Browser APIs](web-host.md) for Pointer Lock API details
- See [Task Breakdown](../history/sprint-era-record.md) for input tasks
- See [virtio-input](windows-drivers.md) for the paravirtual keyboard/mouse fast path
