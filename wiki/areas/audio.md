# Audio

> Guest audio from the emulated codec to the browser's audio output: the Intel
> HDA device model, the virtio-sound path, the ring contracts that carry
> samples between workers, and the AudioWorklet that finally plays them.

Guest audio for the browser-based Windows 7 emulator: an Intel HD Audio (HDA)
controller and a paravirtual virtio-snd device, bridged to the host via Web
Audio / AudioWorklet. Stack selection is governed by the canonical audio stack
[canonical audio stack](../decisions/0010-canonical-audio-stack.md) decision.

Data flow:

- Playback: guest → (HDA / virtio-snd DMA) → device model → playback ring → AudioWorklet → speakers
- Capture: microphone → mic ring → device model → guest capture DMA

## Current state (verified)

Canonical device models and bridges:

- `crates/aero-audio/src/hda.rs` — HDA device model (playback + capture) with codec/PCM glue.
- `crates/aero-audio/src/hda_pci.rs` — PCI function wrapper for the HDA model (config space + BAR0 MMIO).
- `crates/aero-virtio/src/devices/snd.rs` — virtio-snd device model.
- `crates/aero-wasm/src/hda_controller_bridge.rs` — WASM export `HdaControllerBridge` (MMIO read/write, `process(frames)`, ring attachment).
- `crates/aero-wasm/src/virtio_snd_pci_bridge.rs` — WASM export `VirtioSndPciBridge` (BAR0 MMIO + `poll()`, ring attachment).
- `crates/platform/src/audio/worklet_bridge.rs` — playback SharedArrayBuffer ring layout + producer-side `WorkletBridge` (re-exported as `aero_audio::worklet_bridge`).
- `crates/platform/src/audio/mic_bridge.rs` — microphone ring layout + consumer-side `MicBridge` (re-exported as `aero_audio::mic_bridge`).

Browser runtime (web/):

- `apps/web/src/platform/audio.ts` — `createAudioOutput`, the canonical Web Audio output entrypoint.
- `apps/web/src/platform/audio-worklet-processor.js` — AudioWorklet playback ring consumer.
- `apps/web/src/audio/mic_capture.ts`, `apps/web/src/audio/mic-worklet-processor.js`, `apps/web/src/audio/mic_ring.js` — microphone capture path.
- `apps/web/src/audio/audio_worklet_ring.ts` + `apps/web/src/platform/audio_worklet_ring_layout.js` — playback ring header layout constants + math.
- `apps/web/src/audio/audio_frame_clock.ts` — deterministic time→audio-frame conversion (mirrors `crates/aero-audio/src/clock.rs`).
- `apps/web/src/io/devices/hda.ts` (`HdaPciDevice`) and `apps/web/src/io/devices/virtio_snd.ts` (`VirtioSndPciDevice`) — IO-worker PCI device wrappers over the WASM bridges.
- `apps/web/src/workers/io.worker.ts` + `apps/web/src/workers/io_virtio_snd_init.ts` — device registration (`maybeInitHdaDevice()`, `tryInitVirtioSndDevice()`).

Windows 7 driver (virtio-snd only; HDA uses the in-box Windows driver):

- `drivers/windows7/virtio-snd/` — WDM PortCls/WaveRT driver; INF `drivers/windows7/virtio-snd/inf/aero_virtio_snd.inf` binds only `PCI\VEN_1AF4&DEV_1059&REV_01` (verified in the INF).
- Opt-in compat packages for non-Aero transports: `aero-virtio-snd-legacy.inf` (transitional `DEV_1018`) and `aero-virtio-snd-ioport.inf` (legacy I/O-port transport). Neither is part of the Aero contract.

Runtime modes: in `vmRuntime=legacy` the IO worker owns guest audio devices
(PCI/MMIO registration + ring attachment). In the canonical `vmRuntime=machine`
mode, guest devices live inside `api.Machine` in the machine CPU worker
(`apps/web/src/workers/machine_cpu.worker.ts`) and the IO worker runs host-only;
guest audio devices are not exposed through the IO-worker host stack in that
mode.

## Ring buffers (SPSC contracts)

Both host rings are single-producer/single-consumer; only the owner thread may
mutate its indices. Violating SPSC corrupts indices and causes audible
glitches.

### Playback ring

Layout (`Uint32Array` header + `Float32Array` payload, little-endian;
canonical definition in `crates/platform/src/audio/worklet_bridge.rs`):

| Byte offset | Type | Meaning |
|------------:|------|---------|
| 0  | u32 | `readFrameIndex` (monotonic frame counter, consumer-owned) |
| 4  | u32 | `writeFrameIndex` (monotonic frame counter, producer-owned) |
| 8  | u32 | `underrunCount` (missing output frames rendered as silence) |
| 12 | u32 | `overrunCount` (frames dropped by producer, buffer full) |
| 16.. | f32[] | interleaved PCM (`L0, R0, L1, R1, …`) |

Indices are monotonic u32 frame counters (wrap at 2^32) so "read == write" is
unambiguous. `capacityFrames` and `channelCount` are passed out-of-band, not
stored in the header. Backpressure policy is **drop-new**: the producer never
advances `readFrameIndex`; writes are truncated to free space and dropped
frames counted in `overrunCount`.

### Microphone ring

Mono f32 ring (`crates/platform/src/audio/mic_bridge.rs`):

| Byte offset | Type | Meaning |
|------------:|------|---------|
| 0  | u32 | `writePos` (monotonic sample counter, producer-owned) |
| 4  | u32 | `readPos` (monotonic sample counter, consumer-owned) |
| 8  | u32 | `droppedSamples` |
| 12 | u32 | `capacitySamples` (constant) |
| 16.. | f32[] | PCM samples at `writePos % capacitySamples` |

Backpressure is **keep-latest** (drop the oldest part of the current block).
Because the mic is a host resource that is not serialized in snapshots,
consumers discard buffered samples (`readPos := writePos`) on ring attachment
and on snapshot-resume/pause boundaries to avoid stale capture latency.

### Ring attachment policy

The coordinator (`apps/web/src/runtime/coordinator.ts`) owns attachment policy
(`RingBufferOwner`) because each ring is SPSC:

- Playback ring: default owner is the CPU worker in demo mode, the IO worker in
  VM mode; messages `SetAudioRingBufferMessage` in `apps/web/src/runtime/protocol.ts`.
- Mic ring: default owner is CPU worker in demo mode, IO worker in VM mode
  (`vmRuntime=legacy`) or CPU worker (`vmRuntime=machine`); message
  `SetMicrophoneRingBufferMessage`.
- Overrides: `WorkerCoordinator.setAudioRingBufferOwner(...)` /
  `setMicrophoneRingBufferOwner(...)` accept `"cpu" | "io" | "none" | null`.
  `"both"` exists in the type but is rejected at runtime because it violates
  SPSC.
- Inside the IO worker, the rings attach to **HDA when present**, falling back
  to **virtio-snd** only when HDA is unavailable (`attachAudioRingBuffer` /
  `attachMicRingBuffer` in `apps/web/src/workers/io.worker.ts`). If both devices
  enumerate, virtio-snd stays silent unless rings are explicitly re-attached.

Low-rate telemetry mirrors of the ring counters live in the shared status
header: `StatusIndex.AudioBufferLevelFrames`, `AudioUnderrunCount`,
`AudioOverrunCount` (`apps/web/src/runtime/shared_layout.ts`).

## Sample rates

Browsers may ignore the requested `AudioContext` sample rate (Safari/iOS often
runs at 44.1 kHz); `audioOutput.context.sampleRate` is authoritative. Device
models must produce/consume at the actual rate:

- HDA: set `output_rate_hz` on `aero_audio::hda::HdaController`.
- virtio-snd: the guest ABI is fixed at 48 kHz S16_LE; the model resamples via
  `host_sample_rate_hz` (playback) and `capture_sample_rate_hz` (capture,
  defaulting to `host_sample_rate_hz`) — see
  `VirtioSnd::new_with_host_sample_rate`,
  `new_with_capture_and_host_sample_rate`, `set_host_sample_rate_hz`,
  `set_capture_sample_rate_hz` in `crates/aero-virtio/src/devices/snd.rs`
  (verified present).

### Format, rate, and channel conversion

The conversion itself lives in [`aero_audio::dsp`](../../crates/aero-audio/src/dsp/), which exists
because the guest and the host each pick their own stream format and neither yields. A title may
open a 22.05 kHz mono stream while the browser's audio context runs 48 kHz stereo.

Four steps, each its own module: `pcm` decodes the guest's sample format, `resample` reconciles
rates, `remix` reconciles channel layouts, and `mix` combines streams. `StreamProcessor` chains
them in the order that keeps the most information — decode, then resample, then remix — so a
downmix never throws away detail the resampler could have used.

Resampling is linear by default. The `sinc-resampler` feature substitutes a windowed-sinc
implementation, which costs more CPU and is worth it when the ratio is awkward: 44.1 kHz to 48 kHz
is the common case, and linear interpolation is audibly poor at it.

## HDA model specifics

Capture exposure (verified against the crate):

- One capture stream on `SD1` (input stream 0), DMA-writing PCM into guest
  memory via BDL entries.
- Codec topology: input converter widget `NID 4` + mic pin widget `NID 5` so
  Windows enumerates a recording endpoint.
- Host samples arrive via `aero_audio::capture::AudioCaptureSource`
  (implemented for `MicBridge` on wasm).

Pin/power gating (Windows uses these to mute endpoints; DMA still advances):

- Playback is silenced when AFG power (`NID 1`) is not D0, output pin
  (`NID 3`) `pin_ctl == 0`, or `NID 3` power is not D0.
- Capture DMAs silence when AFG power is not D0, mic pin (`NID 5`) power is
  not D0, or `NID 5` `pin_ctl == 0`; in these states the model must not
  consume host mic samples.
- Locked in by `crates/aero-audio/tests/hda_volume_mute.rs` and
  `crates/aero-audio/tests/hda_capture_pin_gating.rs`.

## virtio-snd device contract (summary)

The authoritative device/transport contract is the Windows 7 virtio driver
contract (`AERO-W7-VIRTIO`; see pointers). Summary of the fixed v1 surface,
all verified against `crates/devices/src/pci/profile.rs` (`VIRTIO_SND`
profile) and `crates/aero-virtio/src/devices/snd.rs`:

- PCI identity: `1AF4:1059` (modern ID space `0x1040 + 25`), subsystem
  `1AF4:0019`, revision `0x01` (contract v1), canonical BDF `00:0B.0`, class
  `04/01/00`, INTA. Modern-only virtio-pci transport (vendor capabilities +
  BAR0 MMIO); no legacy I/O-port BARs.
- Feature bits: `VIRTIO_F_VERSION_1` + `VIRTIO_F_RING_INDIRECT_DESC` only.
- Virtqueue sizes: controlq=64, eventq=64, txq=256, rxq=64.
- Two fixed-format streams: stream 0 playback stereo 48 kHz S16_LE; stream 1
  capture mono 48 kHz S16_LE.
- Control commands implemented: `PCM_INFO` (0x0100), `PCM_SET_PARAMS`
  (0x0101), `PCM_PREPARE` (0x0102), `PCM_RELEASE` (0x0103), `PCM_START`
  (0x0104), `PCM_STOP` (0x0105); status codes `S_OK`/`S_BAD_MSG`/`S_NOT_SUPP`/
  `S_IO_ERR`. Jack/chmap requests return `S_NOT_SUPP`.
- eventq: contract v1 defines no required events. The device model emits
  nothing unless the host queues events via `VirtioSnd::queue_event(...)` /
  `queue_jack_event(...)` (bounded FIFO, jack events deduplicated — verified in
  `snd.rs`). The browser bridge uses this to signal speaker (`jack_id=0`) and
  microphone (`jack_id=1`) connect/disconnect on ring attach/detach. The Win7
  driver parses events best-effort and always reposts buffers; the WaveRT
  path stays timer-driven by default.
- Interrupts: INTx is the contract baseline (ISR read-to-ack); the in-tree
  driver also opts into MSI/MSI-X (`MSISupported=1`). On Aero contract devices
  MSI-X is exclusive when enabled — a source whose MSI-X vector is
  unprogrammed (`0xFFFF`) or masked gets no interrupt at all, with no INTx
  fallback.

Packed struct layouts shared between driver and device model live in
`drivers/protocol/virtio/` (with Rust unit tests). Split-virtqueue guidance is in
[windows-drivers.md](./windows-drivers.md). Note the sound driver is **WDM**,
not KMDF — only virtio-input uses the framework.

## Snapshot/restore

- HDA: outer `aero-snapshot` `DEVICES` entry `DeviceId::HDA` (18), web kind
  `"audio.hda"`; inner `aero-io-snapshot` TLV `HDA0`. Schema:
  `crates/aero-io-snapshot/src/io/audio/state.rs` (`HdaControllerState`,
  `AudioWorkletRingState`); device hooks `HdaController::snapshot_state` /
  `restore_state` behind the `io-snapshot` feature in `crates/aero-audio`.
- virtio-snd: `DeviceId::VIRTIO_SND` (22), web kind `"audio.virtio_snd"`;
  captures virtio-pci transport + stream state + ring indices.
- Kind mapping lives in `apps/web/src/workers/vm_snapshot_wasm.ts` and
  `crates/aero-wasm/src/vm_snapshot_device_kind.rs` (both verified).
- What is captured: guest-visible device state (registers, CORB/RIRB, stream
  descriptors, codec pin/power state, DMA progress), host sample rates, and
  host ring **indices** (not contents).
- What is not: the `AudioContext`/`AudioWorkletNode` graph (recreated on
  restore), ring **contents** (cleared to silence), mic ring contents, and
  underrun/overrun counters (host telemetry). Host-time-derived audio clocks
  must be reset on resume so devices do not fast-forward by paused wall-clock
  time.
- Ring helpers: `WorkletBridge::snapshot_state()` / `restore_state()`
  (`crates/platform/src/audio/worklet_bridge.rs`),
  `apps/web/src/platform/audio_ring_restore.ts` (`restoreAudioWorkletRing()`).

## Tuning (`createAudioOutput` options)

Canonical entrypoint `apps/web/src/platform/audio.ts`. Key knobs and defaults:

| Knob | Default | Trade-off |
|------|---------|-----------|
| `startupPrefillFrames` | 512 | silence prefill at graph start; higher ⇒ fewer cold-start underruns, later first audible sample |
| `discardOnResume` | true | flush the ring on `AudioContext` resume (via `ring.reset` control message to the worklet) so suspended-time backlog is dropped instead of played late |
| `ringBufferFrames` | ~200 ms of capacity (derived from actual sample rate) | larger ⇒ fewer underruns, more worst-case latency |
| `latencyHint` | `"interactive"` | `"balanced"`/`"playback"` increase browser buffering |

Rules of thumb from the source docs: demo mode keeps prefill small
(0–512); VM mode uses larger prefill (2048–4096) and possibly a larger ring
(~400–500 ms) to tolerate IO-worker stalls; keep `discardOnResume: true` for
interactive use. `createAudioOutput` probes `AudioContext` with a
`webkitAudioContext` fallback and retries `AudioWorkletNode` construction
without `outputChannelCount` for browser compatibility.

Rough latency estimate:
`totalSeconds ≈ bufferLevelFrames / sampleRate + baseLatencySeconds + outputLatencySeconds`
(the latter two from `AudioContext.baseLatency` / `.outputLatency` when the
browser exposes them; surfaced via `audioOutput.getMetrics()`).

## Testing

- Unit: `cargo test -p aero-audio` (gating tests listed above);
  `crates/aero-io-snapshot/tests/state_roundtrip.rs` for snapshot round-trips.
- E2E (Playwright):
  - `tests/e2e/audio-worklet-hda-demo.spec.ts` — CPU-worker `HdaPlaybackDemo`
    keeps the AudioWorklet ring ~200 ms full using the real HDA model; asserts
    `AudioContext` running, write index advancing, bounded underruns, zero
    overruns.
  - `tests/e2e/audio-hda-pci-snapshot-resume.spec.ts` — full IO-worker PCI/MMIO
    path (config ports 0xCF8/0xCFC, BAR0 MMIO) plus snapshot/resume; asserts
    non-silent samples and no post-restore fast-forward.
  - `tests/e2e/audio-worklet-suspend-resume-discard.spec.ts` — suspend/resume
    triggers ring discard.
  - Harness note: the CPU worker publishes a demo framebuffer into guest RAM;
    audio harnesses must place CORB/RIRB/BDL/PCM scratch buffers disjoint from
    it (allocate from the end of guest RAM).
- Manual: `wiki/areas/testing.md` covers the Win7
  in-box-HDA checklist; `apps/web/serve-smoke-test.mjs` serves a standalone
  AudioWorklet smoke page (48 kHz tone + resample) with COOP/COEP headers.

## Legacy

- AC'97 is **not** part of the canonical stack. The one AC'97 device model this
  repository ever had belonged to the retired second device stack and went with
  it; the canonical audio decision covers HDA and virtio-snd, and applying that
  decision is why the model was not carried over. It is recoverable from
  `.attic/legacy-crates/` if that decision is ever revisited.
- The CPU-worker `HdaPlaybackDemo` (`vmRuntime=legacy`) is a test harness, not
  the production device path.

## Open issues / debt

- `vmRuntime=machine` does not yet expose guest audio devices through the
  browser worker runtime; audio device ownership is split between the legacy
  IO-worker stack and the canonical machine.
- virtio-snd and HDA cannot both be audible: the SPSC rings attach to exactly
  one device (HDA preferred); no explicit device-selection mechanism exists.
- Ring indices are snapshotted but contents are not, so restore always
  re-primes audio from silence (by design, but audible as a gap).

## Pointers

- Platform/firmware page: `wiki/areas/platform-and-firmware.md` (PCI BDF table,
  snapshot `DeviceId` registry, IRQ semantics).
- Decisions: `wiki/decisions/0010-canonical-audio-stack.md`,
  `wiki/decisions/0014-canonical-machine-stack.md`.
- Contracts (absorbed from `docs/`): AERO-W7-VIRTIO virtio contract,
  windows-device-contract, and split-virtqueue guidance →
  `wiki/areas/windows-drivers.md`; Win7 audio checklist →
  `wiki/areas/testing.md`.

*Absorbed from `audio.md` and `windows-drivers.md`
(2026-07); paths and constants verified against the tree at absorption time.*

## Device-model fixes applied (2026-07-27)

See `wiki/notes/bring-up-findings-divergence-and-sight-audits.md` §6 for details.

- **HDA widget TYPE field**: moved to bits [23:20] per Intel HDA spec (was in
  low bits → Win7's hdaudio.sys finds no Pin Complex TYPE=4 widgets → zero audio
  endpoints discovered → audio device never appears).
- **HDA SRST polarity**: stream runs with RUN=1, SRST=0 (was requiring both
  SRST=1 and RUN=1 — inverted from spec → DMA never runs).
- **HDA GCAP bit layout**: fixed to OSS[15:12], ISS[11:8], BSS[7:3], 64OK(bit0)
  per spec (was packed into low nibbles → wrong stream counts decoded by guest).
- **virtio-snd pcm_info struct**: this one was wrong and has been reverted. The
  structure is 32 bytes: `virtio_snd_info` is a single `le32 hda_fn_nid`, which
  leaves the following `le64` members naturally aligned. Widening the header to
  `le64` shifted every subsequent field by four bytes and added eight bytes per
  entry, which is the parse failure it was meant to avoid. See
  `wiki/notes/bring-up-findings-divergence-and-sight-audits.md`, correction to
  row 19.

## The host harnesses were still driving streams the pre-fix way (2026-07-29)

The 2026-07-27 pass fixed `SDnCTL.SRST` in the HDA device model: a stream runs on
RUN=1 with SRST=0, and a descriptor carrying both bits is held in reset. That was
correct, and it broke every caller in the tree, because all of them wrote
`SRST | RUN` in a single store — the convention the old model accepted.

Nothing noticed for two days. The Rust unit tests set stream state directly and
did not check that DMA moved, and the Playwright specs that would have caught it
run against `apps/web/src/wasm/`, whose contents were built on 2026-07-23. The
device model and the tests exercising it were, in effect, different programs.

Five call sites were still on the old convention:

| Where | What it drives |
| --- | --- |
| `apps/web/src/workers/cpu.worker.ts` | HDA PCI playback (SD#0) |
| `apps/web/src/workers/cpu.worker.ts` | synthetic capture (SD#1) |
| `apps/web/src/workers/io.worker.ts` | mic-capture harness (SD#1) |
| `crates/aero-wasm/src/lib.rs` | the two `HdaPlaybackDemo` stream setups |
| `crates/aero-wasm/src/hda_controller_bridge.rs` | two out-of-bounds DMA tests |

The last pair is worth calling out: those tests assert that a stream pointed at
an invalid DMA address does not panic. With SRST set the stream never ran, so
they were asserting nothing. They pass on their own merits now.

The MMIO callers were given the sequence the spec actually describes — assert
SRST, read back 1, deassert, read back 0, then program the descriptor and set RUN
— via `resetHdaStreamDescriptor` in `cpu.worker.ts` and an inline equivalent in
`io.worker.ts`. The two Rust demos assign the descriptor's register state
directly, so they simply no longer set SRST.

## `VirtioSndPlaybackDemo` was never part of the crate

`crates/aero-wasm/src/virtio_snd_playback_demo.rs` was added in January together
with its Playwright spec, and no `mod` declaration ever named it. Rust does not
warn about a file nothing includes, so the crate built, the workspace tests
passed, and the export simply did not exist.

The browser handles a missing export by logging `VirtioSndPlaybackDemo wasm
export is unavailable; skipping virtio-snd audio demo` and moving on, so the page
worked and the spec's ring never advanced. It only surfaced when the wasm was
rebuilt, because the stale package predated whatever tree last had the module
wired up.

`crates/aero-wasm/tests/every_source_file_is_reachable.rs` now fails on any file
under `src/` that no `mod` declaration names.

## Audio Subsystem

### Overview

Windows 7 uses **Intel HD Audio (HDA)** as the primary audio interface. Aero emulates guest audio devices (HDA + virtio-snd) and bridges them to the browser via **Web Audio / AudioWorklet**.

### Manual Windows 7 smoke test (in-box HDA driver)

To validate Windows 7 audio using the in-box HDA driver, use the manual checklist at:

- [`audio.md`](audio.md)

It covers Win7 boot, Device Manager enumeration (“High Definition Audio Controller” + “High Definition Audio Device”),
playback/recording validation, and the host-side metrics to capture (AudioWorklet ring buffer level + underrun/overrun counters).

Canonical implementation pointers (to avoid duplicated stacks):

- `crates/aero-audio/src/hda.rs` — canonical HDA device model (playback + capture) + codec/PCM glue.
- `crates/aero-audio/src/hda_pci.rs` — canonical PCI function wrapper for the HDA model (config space + BAR0 MMIO).
- `crates/aero-wasm/src/hda_controller_bridge.rs` — WASM-side bridge exported as `HdaControllerBridge` and used by the browser IO
  worker to expose HDA as a PCI/MMIO device (MMIO read/write, `process(frames)`, ring attachment).
- `crates/aero-virtio/src/devices/snd.rs` — canonical virtio-snd device model.
- `crates/aero-wasm/src/virtio_snd_pci_bridge.rs` — WASM-side bridge exported as `VirtioSndPciBridge` and used by the browser IO
  worker to expose virtio-snd as a virtio-pci device (BAR0 MMIO + `poll()`, ring attachment).
- `../specs/windows7-virtio-driver-contract.md` — definitive Windows 7 virtio device/transport contract (`AERO-W7-VIRTIO`, includes virtio-snd).
- `crates/platform/src/audio/worklet_bridge.rs` — playback `SharedArrayBuffer` ring layout + producer-side helper (`WorkletBridge`).
- `apps/web/src/platform/audio.ts` — Web Audio output setup + JS ring producer helpers.
- `apps/web/src/platform/audio-worklet-processor.js` — AudioWorklet playback ring consumer.
- `apps/web/src/audio/audio_frame_clock.ts` — deterministic time→audio-frame conversion helper used for device/worker tick scheduling (mirrors `crates/aero-audio/src/clock.rs`).
- `apps/web/src/audio/audio_worklet_ring.ts` + `apps/web/src/platform/audio_worklet_ring_layout.js` — playback ring header layout constants + helper math (shared between producers and the AudioWorklet consumer).
- `crates/platform/src/audio/mic_bridge.rs` — microphone `SharedArrayBuffer` ring layout + consumer-side helper (`MicBridge`).
- `apps/web/src/audio/mic_ring.js` + `apps/web/src/audio/mic-worklet-processor.js` — microphone ring helpers + AudioWorklet producer.
- `apps/web/vite.config.ts` + `vite.harness.config.ts` — emit AudioWorklet dependency assets (`mic_ring.js`, `audio_worklet_ring_layout.js`) because Vite does not follow ESM imports from worklet modules loaded via `audioWorklet.addModule(new URL(...))`.
- `apps/web/src/runtime/protocol.ts` + `apps/web/src/runtime/coordinator.ts` — ring buffer attachment messages (`SetAudioRingBufferMessage`, `SetMicrophoneRingBufferMessage`).
- `apps/web/src/workers/io.worker.ts` + `apps/web/src/io/*` — worker runtime PCI/MMIO device registration (IO worker owns the guest device model layer).
  - `apps/web/src/io/devices/hda.ts` — `HdaPciDevice` wrapper over `HdaControllerBridge` (MMIO + tick scheduling + ring attachment plumbing).
  - `apps/web/src/io/devices/virtio_snd.ts` — `VirtioSndPciDevice` wrapper over `VirtioSndPciBridge` (virtio-pci BAR0 MMIO + ring attachment plumbing).
  - `apps/web/src/workers/io_virtio_snd_init.ts` — IO worker virtio-snd init/registration helper.

The second device stack carried its own HDA and AC'97 models. It has been retired; the sample-rate,
sample-format, and channel-layout conversion it carried was the one part with no canonical
equivalent, and now lives in [`aero_audio::dsp`](../../crates/aero-audio/src/dsp/).

---

### Audio architecture

At a high level, audio flows like:

- Guest → (HDA/virtio-snd DMA) → device model → playback ring → AudioWorklet → speakers
- Microphone → mic ring → device model → guest capture DMA

---

### Worker runtime integration

This section describes the *canonical* browser runtime integration.

> Runtime note: the details below describe the legacy device-stack integration (`vmRuntime=legacy`), where guest audio devices
> live in the I/O worker. The canonical machine runtime (`vmRuntime=machine`) runs `api.Machine` in the CPU worker and does not
> currently expose guest audio devices via the I/O worker host stack.

#### Ownership model

- The **IO worker** owns the guest-visible device layer (PCI/MMIO/virtio) and is the intended home for guest audio devices (**HDA** and **virtio-snd**).
  - Both devices can be registered in IO-worker browser builds (depending on which WASM exports are available):
    - HDA: `HdaControllerBridge` (`crates/aero-wasm/src/hda_controller_bridge.rs`) + `HdaPciDevice` (`apps/web/src/io/devices/hda.ts`)
    - virtio-snd: `VirtioSndPciBridge` (`crates/aero-wasm/src/virtio_snd_pci_bridge.rs`) + `VirtioSndPciDevice` (`apps/web/src/io/devices/virtio_snd.ts`)
  - **Ring attachment policy** (SPSC rings): the IO worker attaches the host audio rings to **HDA when present**, and falls back to
    attaching them to **virtio-snd** only when HDA is unavailable (see `attachAudioRingBuffer` / `attachMicRingBuffer` in
    `apps/web/src/workers/io.worker.ts`).
- The **AudioWorkletProcessor** runs on the browser’s audio rendering thread.
- The **main thread** owns the browser audio graph (`AudioContext` + `AudioWorkletNode`), typically gated by user gesture.
- The **coordinator** forwards `SharedArrayBuffer` attachments to workers via `postMessage`.

#### Playback ring attachment (AudioWorklet output)

1. UI calls `createAudioOutput` (`apps/web/src/platform/audio.ts`), which:
    - allocates a playback `SharedArrayBuffer` ring,
    - loads `apps/web/src/platform/audio-worklet-processor.js`,
    - constructs an `AudioContext` (`AudioContext`/`webkitAudioContext` fallback) and applies a few startup/latency-smoothing policies (see `createAudioOutput` options below),
    - constructs an `AudioWorkletNode` with `processorOptions.ringBuffer = sab`.
2. The coordinator forwards the ring to the worker that will act as the **single producer** via `SetAudioRingBufferMessage`
    (`apps/web/src/runtime/protocol.ts`, policy + dispatch in `apps/web/src/runtime/coordinator.ts`).
    - The coordinator owns the attachment policy (`RingBufferOwner`) because the playback ring is SPSC (exactly one producer).
   - Default policy: CPU worker in demo mode (no disk), IO worker when running a real VM (disk present).
     - See `WorkerCoordinator.defaultAudioRingBufferOwner()` + `syncAudioRingBufferAttachments()`.
   - Optional override (use with care): `WorkerCoordinator.setAudioRingBufferOwner("cpu" | "io" | "none" | null)`.
     - Use `null` to clear an override and return to the default policy.
     - Note: `RingBufferOwner` includes `"both"` for compatibility, but the coordinator intentionally rejects it (throws) because it
       violates the SPSC contract (multi-producer access corrupts the ring indices).
   - `ringBuffer`: `SharedArrayBuffer | null` (null detaches)
   - `capacityFrames` / `channelCount`: out-of-band layout parameters
   - `dstSampleRate`: the *actual* `AudioContext.sampleRate`
3. The producer worker attaches the ring:
   - Generic WASM producer: `WorkletBridge.fromSharedBuffer(...)` (Rust: `crates/platform/src/audio/worklet_bridge.rs`; WASM export: `api.attach_worklet_bridge(...)`).
   - IO worker HDA path: `HdaPciDevice.setAudioRingBuffer(...)` (`apps/web/src/io/devices/hda.ts`) forwards to the WASM-side
     `HdaControllerBridge.attach_audio_ring(...)` + `set_output_rate_hz(...)` (`crates/aero-wasm/src/hda_controller_bridge.rs`).

#### Microphone ring attachment (AudioWorklet capture)

1. UI starts mic capture (`apps/web/src/audio/mic_capture.ts`), which:
   - allocates a mic `SharedArrayBuffer` ring,
   - starts `apps/web/src/audio/mic-worklet-processor.js` as the low-latency producer.
2. The coordinator forwards the mic ring via `SetMicrophoneRingBufferMessage`.
   - The coordinator owns the attachment policy (`RingBufferOwner`) because the mic ring is SPSC (exactly one consumer).
   - Default policy:
     - `vmRuntime=legacy`: CPU worker in demo mode, IO worker in VM mode.
     - `vmRuntime=machine`: CPU worker (canonical Machine) in VM mode.
     - See `WorkerCoordinator.defaultMicrophoneRingBufferOwner()` + `syncMicrophoneRingBufferAttachments()`.
   - Optional override (use with care): `WorkerCoordinator.setMicrophoneRingBufferOwner("cpu" | "io" | "none" | null)`.
     - Use `null` to clear an override and return to the default policy.
     - Note: `RingBufferOwner` includes `"both"` for compatibility, but the coordinator intentionally rejects it (throws) because it
       violates the SPSC contract (multiple consumers would advance `readPos` and effectively double-consume/drop samples).
   - `ringBuffer`: `SharedArrayBuffer | null`
   - `sampleRate`: the *actual* capture graph sample rate
3. The consumer worker consumes mic samples via `MicBridge.fromSharedBuffer(...)` (`crates/platform/src/audio/mic_bridge.rs`).
   - IO worker HDA path: the IO worker forwards the attachment into the WASM-side `HdaControllerBridge` so it can consume samples
     via an internal `MicBridge` while running the guest capture DMA path.

#### Device registration (PCI/MMIO)

In the legacy worker runtime (`vmRuntime=legacy`), guest-visible devices are registered on the IO worker PCI bus:

- Bus/device plumbing: `apps/web/src/io/device_manager.ts`, `apps/web/src/io/bus/pci.ts`, `apps/web/src/io/bus/mmio.ts`
- Worker wiring (legacy runtime, `vmRuntime=legacy`): `apps/web/src/workers/io.worker.ts` (calls `DeviceManager.registerPciDevice(...)`)
  - HDA PCI function wrapper: `apps/web/src/io/devices/hda.ts` (`HdaPciDevice`, backed by `HdaControllerBridge`).
    - Registration entrypoint: `maybeInitHdaDevice()` in `apps/web/src/workers/io.worker.ts`.
  - virtio-snd PCI function wrapper: `apps/web/src/io/devices/virtio_snd.ts` (`VirtioSndPciDevice`, backed by `VirtioSndPciBridge`).
    - Registration entrypoint: `tryInitVirtioSndDevice()` in `apps/web/src/workers/io_virtio_snd_init.ts` (invoked by `maybeInitVirtioSndDevice()` in `apps/web/src/workers/io.worker.ts`).
  - See `apps/web/src/io/devices/uhci.ts` for a concrete example of a WASM-backed PCI device wrapper (PIO + IRQ + tick scheduling).

Note: In `vmRuntime=machine`, guest audio devices live inside the canonical `api.Machine` runtime owned by
`apps/web/src/workers/machine_cpu.worker.ts`; the IO worker runs in host-only mode and does not register guest PCI devices.

#### Ring producer/consumer constraints (SPSC)

Both rings are **SPSC** (single-producer / single-consumer). Only the “owner” thread should mutate the relevant indices/counters:

- **Playback ring** (`worklet_bridge.rs`, `audio-worklet-processor.js`)
  - Producer-owned fields: `writeFrameIndex`, `overrunCount`
  - Consumer-owned fields: `readFrameIndex`, `underrunCount`
- **Microphone ring** (`mic_bridge.rs`, `mic_ring.js`)
  - Producer-owned fields: `writePos`, `droppedSamples`, `capacitySamples`
  - Consumer-owned fields: `readPos`

Violating SPSC (e.g. having both JS and WASM write to the playback ring) will corrupt indices and cause audible glitches.

#### Host-side telemetry (StatusIndex)

In addition to the raw ring header counters, the active producer worker publishes a few low-rate counters into the shared
status header (`apps/web/src/runtime/shared_layout.ts`):

- `StatusIndex.AudioBufferLevelFrames`
- `StatusIndex.AudioUnderrunCount`
- `StatusIndex.AudioOverrunCount`

These are useful for perf HUDs / smoke tests without needing direct access to the underlying `SharedArrayBuffer` ring.

---

### Browser demo + CI coverage

To validate the *real* HDA DMA path end-to-end in the browser (guest PCM DMA → HDA model → `WorkletBridge` → AudioWorklet),
the repo includes a small demo harness:

- **UI button**: click `#init-audio-hda-demo` (“Init audio output (HDA demo)”) in
  `apps/web/src/main.ts` — one host now, served by Playwright at
  `http://127.0.0.1:4173/`. This used to be a choice between two separate apps.
- **Implementation**:
  - In `vmRuntime=legacy`, the CPU worker (`apps/web/src/workers/cpu.worker.ts`) instantiates the WASM export `HdaPlaybackDemo` and keeps the AudioWorklet ring buffer ~200ms full.
  - The demo programs a looping guest PCM buffer + BDL and uses the *real* HDA device model (`aero_audio::hda::HdaController`) to generate output.
- **E2E test**: `tests/e2e/audio-worklet-hda-demo.spec.ts` asserts that:
   - `AudioContext` reaches `running`,
   - the ring buffer write index advances over time,
   - underruns stay bounded and overruns remain 0.

Note: this demo is a *test harness* for the HDA audio pipeline. In `vmRuntime=legacy`, the production VM device stack is
owned by the IO worker. In `vmRuntime=machine`, guest audio devices are owned by the Machine CPU worker and the IO worker runs
in host-only mode.

#### End-to-end IO-worker HDA PCI/MMIO device path

Legacy runtime note: this section applies to `vmRuntime=legacy` (IO-worker-owned guest devices). In `vmRuntime=machine`,
guest devices are owned by the Machine CPU worker.

The CPU-worker HDA demo above is useful for validating the core HDA audio model + ring-buffer plumbing, but it does **not**
exercise the *real* worker runtime device stack (PCI config space, BAR0 MMIO, IO-worker-owned HDA PCI function).

To validate that full path, the repo-root harness exposes:

- **UI button**: `#init-audio-hda-pci-device` (“Init audio output (HDA PCI device)”) in `apps/web/src/main.ts`
- **Implementation**:
  - The main thread allocates an AudioWorklet output ring buffer and attaches it to the **IO worker** via
    `WorkerCoordinator.setAudioRingBufferOwner("io")` + `setAudioRingBuffer(...)`.
  - The CPU worker programs the **IO worker's** HDA PCI function (8086:2668) using the real:
    - PCI config ports (0xCF8/0xCFC)
    - BAR0 MMIO reads/writes
  - The IO worker's WASM-backed `HdaPciDevice` then DMA-reads guest PCM and writes `f32` samples into the AudioWorklet ring.

Note: the CPU worker continuously publishes a shared framebuffer demo into guest RAM (see
`CPU_WORKER_DEMO_FRAMEBUFFER_OFFSET_BYTES` / `DEMO_FB_OFFSET`). Any audio harness that places guest-physical scratch buffers
(CORB/RIRB/BDL/PCM) in guest RAM must keep them disjoint from those regions (prefer allocating from the end of guest RAM),
otherwise the framebuffer publish loop will corrupt device state and cause flaky tests.
- **E2E test**: `tests/e2e/audio-hda-pci-snapshot-resume.spec.ts` asserts that:
  - ring read + write indices advance,
  - samples are **non-silent** (not just index movement),
  - underruns/overruns stay bounded, and
  - playback does not burst/fast-forward after snapshot restore.

---

### Playback: AudioWorklet output ring

#### Semantics

Playback uses a `SharedArrayBuffer` ring buffer consumed by an `AudioWorkletProcessor`.

- Indices are monotonic `u32` **frame counters** (wrap naturally at `2^32`) to avoid “read == write” ambiguity.
- Overrun/backpressure policy is **drop-new**:
  - the producer never advances the consumer-owned `readFrameIndex` to “make room”,
  - writes are truncated to the available free space,
  - dropped frames are counted in `overrunCount`.

Canonical semantics live in:

- `crates/platform/src/audio/worklet_bridge.rs` (WASM producer)
  - Re-exported from `crates/aero-audio/src/lib.rs` as `aero_audio::worklet_bridge`.
- `apps/web/src/platform/audio.ts` (JS producer used by demos/fallbacks)
- `apps/web/src/platform/audio-worklet-processor.js` (consumer)

#### Playback ring buffer layout

Header (`Uint32Array`, little-endian) + payload (`Float32Array`):

| Byte offset | Type | Meaning |
|------------:|------|---------|
| 0           | u32  | `readFrameIndex` (monotonic frame counter, consumer-owned) |
| 4           | u32  | `writeFrameIndex` (monotonic frame counter, producer-owned) |
| 8           | u32  | `underrunCount` (total missing output frames rendered as silence; wraps at 2^32) |
| 12          | u32  | `overrunCount` (frames dropped by producer due to buffer full; wraps at 2^32) |
| 16..        | f32[]| Interleaved PCM samples (`L0, R0, L1, R1, ...`) |

Important: `capacityFrames` and `channelCount` are passed out-of-band (they are *not* stored in the header) and must match
both the producer and the AudioWorklet `processorOptions`.

#### Sample rate mismatches

Browsers may ignore a requested `AudioContext` sample rate (Safari/iOS commonly runs at 44.1kHz). The AudioWorklet consumes
frames at `AudioContext.sampleRate`, so the device models must be configured to produce audio at that *actual* rate.

- HDA (`aero_audio::hda::HdaController`): set `output_rate_hz` to the output `AudioContext.sampleRate`.
- virtio-snd (`aero_virtio::devices::snd::VirtioSnd`): set `host_sample_rate_hz` to the output `AudioContext.sampleRate`.

#### `createAudioOutput` options (latency vs robustness)

`createAudioOutput` (`apps/web/src/platform/audio.ts`) is the canonical Web Audio output entrypoint. In addition to basic setup
(`sampleRate`, `latencyHint`, `ringBufferFrames`, etc.), it exposes a few tuning knobs to trade off:

- **Robustness** (tolerating IO worker stalls, slow startup, or browser suspensions), vs
- **Latency** (time from guest audio generation to speakers).

Key options (in addition to the existing `sampleRate`/`latencyHint`/`ringBufferFrames`/`ringBuffer`):

- `startupPrefillFrames?: number`
  - **What it does:** if the playback ring is empty at graph startup, pre-fills it with *silence* up to this many frames.
    (Default: `512` frames.)
    - Frames are **per-channel** (same unit as `ringBufferFrames`) and are clamped to the ring’s `capacityFrames`.
    - Avoid setting this too close to `ringBufferFrames` (capacity): a near-full ring leaves little headroom for real audio writes
      and can transiently increase backpressure/overrun counts at startup.
  - **Why it exists:** avoids an initial “startup underrun” window between `AudioWorkletNode` start and the first producer write,
    and gives slow-starting producers a small grace period.
  - **Trade-off:** increases time-to-first-audible-sample by roughly `startupPrefillFrames / AudioContext.sampleRate` seconds.
- `discardOnResume?: boolean`
  - **What it does:** when the `AudioContext` resumes after being suspended (tab backgrounding, iOS/Safari interruptions, etc.),
    discards any buffered playback frames by advancing the consumer `readFrameIndex` to the current `writeFrameIndex`
    (i.e. “flush the ring”).
    (Default: `true`.)
  - **Why it exists:** prevents *stale latency* where the VM kept producing while the browser was suspended; without discarding,
    the AudioWorklet would play back old buffered audio first, making the user hear audio seconds late after resume.
  - **Trade-off:** drops buffered frames across the suspension boundary (you get an audio discontinuity, but latency stays bounded).
  - **Implementation detail:** the discard happens on the **AudioWorklet consumer** (via a control message) so we do not violate
    the playback ring’s SPSC ownership rules (normally only the worklet advances `readFrameIndex`).
    - The discard is intentionally *not* applied to the **first** transition to `AudioContext.state === "running"` so that the
      startup silence prefill can still mask initial underruns.
    - If you need to flush explicitly (or you are in a browser that doesn’t reliably support `AudioContext` `statechange`
      listeners), `createAudioOutput` also exposes `audioOutput.discardBufferedFrames()` to manually request a ring reset.
    - Control message: `{ type: "ring.reset" }` posted to `AudioWorkletNode.port` (the worklet atomically applies
      `readFrameIndex := writeFrameIndex`).
  - **Worklet-side heuristic (optional):** the AudioWorkletProcessor also supports
    `processorOptions.discardOnResume === true` (default: `false`) to auto-reset the ring after detecting a large wall-clock gap
    between `process()` callbacks (a proxy for suspend/resume or extreme scheduling stalls). This is a best-effort self-healing
    mechanism for custom integrations that manually construct an `AudioWorkletNode`; `createAudioOutput` does **not** enable it.
  - **CI coverage:** `tests/e2e/audio-worklet-suspend-resume-discard.spec.ts` validates in a real Chromium browser that an
    `AudioContext` suspend/resume cycle triggers a prompt playback ring discard (prevents stale buffered playback after resume).

Related diagnostics knobs (not latency controls, but useful when tuning):

- `sendUnderrunMessages?: boolean` / `underrunMessageIntervalMs?: number`
  - When enabled, the AudioWorklet posts periodic `type: "underrun"` messages to the main thread for debugging/telemetry.
  - These are disabled by default because posting every render quantum can be expensive under persistent underrun.

##### AudioContext construction fallbacks (Safari/WebKit)

`createAudioOutput` must treat Web Audio as a runtime capability:

- It probes `globalThis.AudioContext`, and falls back to `globalThis.webkitAudioContext` when needed (Safari).
- Some WebKit/Safari variants are stricter about `AudioContext` constructor options; the implementation includes fallbacks so that
  “requested” settings (like `sampleRate`/`latencyHint`) do not hard-fail audio output initialization.
- Some browsers also differ in which `AudioWorkletNode` constructor options they accept; `createAudioOutput` retries node creation
  without `outputChannelCount` when needed for compatibility.
- Autoplay policies vary by browser: `createAudioOutput` makes a best-effort early `AudioContext.resume()`, but callers should
  still expect to call/await `audioOutput.resume()` from a user gesture handler (and may need to retry after a rejected resume).
- Always treat `audioOutput.context.sampleRate` as authoritative (Safari/iOS may ignore the requested sample rate).

##### Quick tuning summary

| Knob | Default | Primary trade-off | When to change |
|------|---------|-------------------|----------------|
| `startupPrefillFrames` | `512` | higher value ⇒ more startup robustness, more time-to-first-sound | VM mode / slow-starting producers |
| `discardOnResume` | `true` | `true` ⇒ drops buffered audio across suspend/resume to keep latency bounded | Almost always keep `true` for interactive UX |
| `ringBufferFrames` | ~200ms capacity (derived from actual sample rate) | larger ring ⇒ fewer underruns, higher worst-case latency | VM mode / IO-worker stalls |
| `latencyHint` | `"interactive"` | `"playback"`/`"balanced"` can increase buffering and stability | if underruns persist even with larger rings |

---

### Capture: microphone ring

Microphone capture is bridged from the browser to the guest via a `SharedArrayBuffer` ring buffer:

- **Main thread**: requests permission on explicit user action and manages lifecycle/UI state (`apps/web/src/audio/mic_capture.ts`).
- **AudioWorklet** (preferred): pulls mic PCM frames with low latency and writes them into the ring (`apps/web/src/audio/mic-worklet-processor.js`).
- **IO worker**: reads from the ring and feeds the guest capture device (HDA input pin or virtio-snd capture stream).

Rust bridge helpers live in `crates/platform/src/audio/mic_bridge.rs` and are re-exported as `aero_audio::mic_bridge`
(via `crates/aero-audio/src/lib.rs`).

#### Microphone ring buffer layout

The microphone ring buffer is a mono `Float32Array` backed by a `SharedArrayBuffer`:

| Byte offset | Type | Meaning |
|------------:|------|---------|
| 0           | u32  | `writePos` (monotonic sample counter) |
| 4           | u32  | `readPos` (monotonic sample counter) |
| 8           | u32  | `droppedSamples` (samples dropped due to buffer pressure; wraps at 2^32) |
| 12          | u32  | `capacitySamples` (samples in the data section; constant) |
| 16..        | f32[]| PCM samples, index = `(writePos % capacitySamples)` |

Backpressure policy for mic capture is **keep-latest** (drop the oldest part of the *current block* when partially writing)
so the guest sees the most recent microphone audio.

##### Attach/resume semantics (stale latency avoidance)

Microphone input is a host resource and is **not serialized in VM snapshots**. The AudioWorklet producer may continue writing
into the ring while the VM is snapshot-paused, or before the guest capture device has attached as the ring consumer.

To avoid replaying *stale* mic samples (which would manifest as large capture latency after resume), consumers discard any
already-buffered samples by advancing `readPos` to the current `writePos` (i.e. `readPos := writePos`) on:

- ring attachment (late attach while the producer is already running)
- snapshot resume / other pause boundaries where the producer may have continued writing while the VM was stopped

#### HDA capture exposure (guest)

The canonical `aero-audio` HDA model exposes one capture stream and a microphone pin widget:

- Stream DMA: `SD1` (input stream 0) DMA-writes captured PCM bytes into guest memory via BDL entries.
- Codec topology: an input converter widget (`NID 4`) plus a mic pin widget (`NID 5`) so Windows can enumerate a recording endpoint.

Host code provides microphone samples via `aero_audio::capture::AudioCaptureSource` (implemented for
`aero_platform::audio::mic_bridge::MicBridge` on wasm) and advances the device model via the HDA capture processing path.

#### HDA pin/power gating semantics (playback + capture)

The minimal HDA codec model enforces a subset of widget **power state** + **pin widget control** semantics that Windows uses to
mute endpoints. This primarily affects whether the guest hears audio / records audio, while keeping DMA timing consistent.

##### Playback gating (line-out)

Playback is forced to **silence** when any of the following are true:

- AFG power state (`NID 1`) is not `D0`
- output pin (`NID 3`) `pin_ctl == 0`
- output pin (`NID 3`) power state is not `D0`

This is implemented by applying 0 gain at the codec output stage (guest playback DMA still advances as normal).

Unit tests:

- [`crates/aero-audio/tests/hda_volume_mute.rs`](../../crates/aero-audio/tests/hda_volume_mute.rs)
  (`afg_power_state_d3_silences_output`, `pin_ctl_zero_silences_output`, `output_pin_power_state_d3_silences_output`)

##### Capture gating (microphone)

The capture stream DMA engine still advances, but the guest receives **silence** when any of the following are true:

- AFG power state (`NID 1`) is not `D0`
- mic pin (`NID 5`) power state is not `D0`
- mic pin (`NID 5`) `pin_ctl == 0`

In the capture-muted cases above, the device model must **not** consume microphone samples from the host ring
(`AudioCaptureSource`). (This avoids dropping mic audio while the guest endpoint is disabled.)

Unit tests:

- [`crates/aero-audio/tests/hda_capture_pin_gating.rs`](../../crates/aero-audio/tests/hda_capture_pin_gating.rs)
  (`capture_pin_ctl_zero_writes_silence_without_consuming`, `capture_pin_power_state_d3_writes_silence_without_consuming`,
  `capture_afg_power_state_d3_writes_silence_without_consuming`)

#### virtio-snd capture exposure (guest)

Guest-visible virtio-snd behaviour (stream ids, queues, formats) is specified by the
[`AERO-W7-VIRTIO` contract](../specs/windows7-virtio-driver-contract.md#34-virtio-snd-audio).

The canonical `aero-virtio` virtio-snd device model exposes an additional fixed-format capture stream:

- Stream id `1`, S16_LE mono @ 48kHz.
- Captured data is delivered to the guest via the virtio-snd RX queue (`VIRTIO_SND_QUEUE_RX`).

#### Capture constraints (echo/noise)

Capture uses `getUserMedia` audio constraints to expose user-facing toggles:

- `echoCancellation`
- `noiseSuppression`
- `autoGainControl`

These are applied when the stream is created (and may be updated later via `MediaStreamTrack.applyConstraints` when supported).

---

### Snapshot/Restore (Save States)

Audio snapshots must capture guest-visible progress (DMA positions, buffer state) while treating the Web Audio pipeline as a
host resource that may need reinitialization.

#### Outer snapshot identity

In the outer `aero-snapshot` `DEVICES` table, HDA audio state is stored under:

- `DeviceId::HDA` (`18`)
- JS kind string: `"audio.hda"` (worker snapshot glue)

`DeviceState.data` is an `aero-io-snapshot` TLV blob (`DEVICE_ID = HDA0`) and `DeviceState.version/flags` mirror the inner
`SnapshotVersion (major, minor)`.

#### What must be captured

- Guest-visible device state (HDA registers, CORB/RIRB, stream descriptors, codec verb-visible state, DMA progress, etc).
  - Includes codec pin/power gating state (AFG `power_state`, pin widget `pin_ctl`/`power_state`), which controls whether
    playback/capture are silenced and (for capture) whether the device model consumes host microphone samples.
- Host sample rates used by the device model (e.g. HDA `output_rate_hz` / `capture_sample_rate_hz`) so resampler state can be restored deterministically.
- Host-side ring **indices** (but not audio content):
   - playback ring `readFrameIndex` / `writeFrameIndex` + `capacityFrames`
   - helpers:
      - `WorkletBridge::snapshot_state()` / `WorkletBridge::restore_state()` (wasm; the real SAB-backed ring)
     - `InterleavedRingBuffer::snapshot_state()` (pure Rust helper used by unit tests)
     - `apps/web/src/platform/audio_ring_restore.ts`: `restoreAudioWorkletRing()` (JS helper used when restoring rings on the web side)
   - Note: ring underrun/overrun counters are host-side telemetry and are not part of the snapshot; restore intentionally leaves
     them untouched (and they may reset if the ring is recreated).

Implementation references:

- Snapshot schema: `crates/aero-io-snapshot/src/io/audio/state.rs` (`HdaControllerState`, `AudioWorkletRingState`).
- HDA device snapshot/restore: `crates/aero-audio/src/hda.rs` (`HdaController::snapshot_state` / `HdaController::restore_state`,
  behind the `io-snapshot` feature).
- Playback ring snapshot/restore: `crates/platform/src/audio/worklet_bridge.rs` (`WorkletBridge::snapshot_state` / `restore_state`).
- Roundtrip tests: `crates/aero-io-snapshot/tests/state_roundtrip.rs`.

#### Restore semantics / limitations

- The browser `AudioContext` / `AudioWorkletNode` is not serializable; on restore the host audio graph is recreated.
- Ring buffer **contents** are not restored. Producers clear the ring to silence on restore to avoid replaying stale samples.
- Host microphone rings are not serializable and may continue producing while the VM is paused; capture consumers are expected to
  discard stale data on attach/resume boundaries. If the guest capture endpoint is gated (pin/power state), the HDA model will DMA
  silence without consuming any host mic samples until the endpoint is re-enabled.
- Any host-time-derived audio clocks (e.g. `AudioFrameClock`-driven schedulers) must be reset on snapshot resume so devices do not
  "fast-forward" by wall-clock time spent paused during save/restore.
- virtio-snd snapshot/restore is supported in the browser runtime under kind `"audio.virtio_snd"` (`DeviceId::VIRTIO_SND = 22`).
  The snapshot captures virtio-pci transport state + virtio-snd stream state + AudioWorklet ring indices, but not host audio
  contents (rings are cleared to silence on restore).
- The goal is *guest-visible determinism*: after restore, Windows should see consistent HDA state and virtio-snd state (when
  present) and DMA position evolution.

---

### Latency management

- Ring buffer sizing defaults are derived from the *actual* `AudioContext.sampleRate`:
  - `apps/web/src/platform/audio.ts`: `getDefaultRingBufferFrames()` defaults to ~200ms of capacity.
- Producers should generally target a smaller steady-state fill level (tens of ms) and adapt if underruns occur:
  - `apps/web/src/platform/audio.ts`: `createAdaptiveRingBufferTarget()`.

#### Measuring/estimating latency

For rough tuning, you can break “audio latency” into:

- **Ring-buffer backlog** (producer → AudioWorklet): approximately
  `bufferLevelFrames / AudioContext.sampleRate` seconds.
- **Browser output pipeline latency** (AudioContext → speakers): best-effort fields exposed by some browsers:
  - `AudioContext.baseLatency` — estimated base latency of the audio graph (seconds)
  - `AudioContext.outputLatency` — estimated latency from the audio graph to the audio output device (seconds)

These `AudioContext` values are **browser/OS dependent** and may be missing. Aero exposes them (when available) via
`AudioOutputMetrics` returned from `audioOutput.getMetrics()`:

- `baseLatencySeconds`
- `outputLatencySeconds`

and also emits them as trace counters (`audio.baseLatencySeconds` / `audio.outputLatencySeconds`) when
`startAudioPerfSampling()` is enabled.

When available, a rough “total” playback latency estimate is:

```
totalSeconds ≈ (bufferLevelFrames / sampleRate) + baseLatencySeconds + outputLatencySeconds
```

Treat this as an **approximation**: browser scheduling, device buffering, and platform policies vary, and not all browsers expose
`outputLatency`.

#### Recommended defaults (demo mode vs VM mode)

Audio tuning is inherently workload-dependent, but the following presets are good starting points:

- **Demo mode** (repo harness / interactive demos; producer is typically the CPU worker):
  - keep `startupPrefillFrames` small to minimize “click → sound” delay:
    - `startupPrefillFrames: 0..512` (default is `512`, ~10.7ms @ 48kHz).
  - prefer `latencyHint: "interactive"` and a smaller steady-state buffer target.
  - `discardOnResume: true` is still recommended if you care about interactive latency after tab/background resumes.
- **VM mode** (real guest VM; producer is typically the IO worker and may stall on host IO/GC):
  - use a larger `startupPrefillFrames` so initial device init / IO-worker stalls don’t immediately underrun:
    - `startupPrefillFrames: 2048..4096` (~43–85ms @ 48kHz) is a reasonable starting range if you see cold-start underruns.
  - consider a less aggressive `latencyHint` (e.g. `"balanced"`/`"playback"`) and/or a larger ring capacity if you see frequent underruns:
    - default `ringBufferFrames` is ~200ms capacity (`sampleRate / 5`), but VM mode can justify ~400–500ms capacity
      if the IO worker frequently stalls.
  - enable `discardOnResume` to avoid resuming with a large backlog of buffered guest audio (stale latency).

Concrete examples:

```ts
// Demo mode: low startup latency, still avoids stale latency after tab resumes.
await createAudioOutput({
  latencyHint: "interactive",
  startupPrefillFrames: 0, // or 512 (default)
  discardOnResume: true,
});

// VM mode: tolerate IO-worker stalls (higher robustness), at the cost of more buffering.
await createAudioOutput({
  latencyHint: "balanced",
  startupPrefillFrames: 4096,
  discardOnResume: true,
});
```

---

### AC'97 fallback (legacy)

AC'97 is **not** part of the canonical Aero audio stack, and no AC'97 device model remains in the
tree. The canonical audio decision covers HDA and virtio-snd; the model that existed belonged to the
retired second device stack.

---

### Next steps

- See [Networking](networking.md) for network stack emulation.
- See [Browser APIs](web-host.md) for Web Audio details.
- See [Task Breakdown](../history/sprint-era-record.md) for audio tasks.

## Windows 7 audio smoke test (in-box HDA driver)

This is a **manual**, **reproducible** smoke test to validate that Windows 7’s **in-box HD Audio (Intel HDA) driver stack** works end-to-end in the Aero worker runtime:

- Windows enumerates the **PCI controller** (“High Definition Audio Controller”)
- Windows enumerates the **audio function** (“High Definition Audio Device”)
- **Playback** works (system sounds / WAV playback reaches the host speakers)
- **Recording** works (browser `getUserMedia` mic capture reaches the guest recording endpoint)

> Windows 7 uses the inbox `hdaudbus.sys` (bus) + `hdaudio.sys` (function) drivers.
> The goal of this smoke test is to confirm we do **not** need a custom driver just to get basic sound.

> Runtime note: this checklist currently applies to the legacy browser runtime (`vmRuntime=legacy`), where guest audio devices
> are hosted in the I/O worker. The canonical machine runtime (`vmRuntime=machine`) does not yet expose guest audio devices via
> the browser worker stack.

Important:

- This checklist is for the **baseline HDA device**.
  - In the browser runtime, the IO worker typically prefers **HDA** when it is present (virtio-snd may still be registered but is not
    the active ring-attached audio device).
  - If you are running a configuration that omits HDA and uses **virtio-snd** as the active guest audio device, this checklist won’t apply.

---

### Prerequisites (host/browser)

- Use a Chromium-based browser for initial bring-up (Chrome/Edge).
- The page must be **cross-origin isolated** (`COOP` + `COEP`) so `SharedArrayBuffer` works (needed for low-latency audio rings).
- **Host audio output** must be functional (speakers/headphones connected, not muted).
- **Host microphone** available (for capture test) and permission can be granted.

Quick sanity checks (DevTools Console):

```js
crossOriginIsolated;
typeof SharedArrayBuffer;
typeof AudioContext;
```

#### Windows 7 media

Per [`AGENTS.md`](../../AGENTS.md), a Win7 ISO is available on the agent machine:

```
/state/win7.iso
```

Do **not** commit or redistribute Windows media/images.

---

### Boot the Windows 7 test image

This section describes what to do in the Aero **web UI** (the exact page/controls may change over time; the intent is what matters).

Note: For the canonical Windows 7 boot/install storage topology (AHCI HDD + IDE/ATAPI CD-ROM), see
[`storage.md`](storage.md).

#### 1.1 Start the web app

From the repo root:

```bash
pnpm install --frozen-lockfile

# If this is a fresh checkout, build the wasm-pack outputs first (gitignored):
# - apps/web/src/wasm/pkg-single/
# - apps/web/src/wasm/pkg-threaded/
# - apps/web/src/wasm/pkg-*-gpu/
pnpm -C apps/web run wasm:build
#
# If this fails due to missing toolchains (wasm-pack / pinned nightly / rust-src), run:
#   just setup
# …then retry the build.

# Start the web UI (served from web/):
pnpm run dev:web
```

Open the printed local URL (usually `http://127.0.0.1:5173/`).

Notes:

- If the page errors with “Missing single-thread WASM package” / “Missing threaded WASM package”, you skipped the build step above.
  - Stop the dev server, run `pnpm -C apps/web run wasm:build`, then restart `pnpm run dev:web`.
- `pnpm run dev:web` runs the **web UI** under `apps/web/`, which includes the **Disks** + **Workers** panels used below.
- The repo-root harness (`pnpm run dev`) is primarily used by CI/Playwright and may not expose the full Win7 boot UI.

#### 1.2 Ensure audio can start (autoplay policy)

Most browsers require a **user gesture** to start audio output. Before expecting guest audio:

- In the UI, perform the user-gesture action that initializes **audio output** (AudioContext + AudioWorklet ring).
  - Some builds expose host-only demo buttons like **“Init audio output (test tone)”**. These are fine to satisfy autoplay policy,
    but they are **not** the guest Windows 7 audio path.
  - If the UI exposes **“Init audio output (worker tone)”**, that button is typically the *right* one for VM runs: it creates the
    AudioWorklet ring and attaches it to the worker audio device (HDA/virtio-snd). In **VM mode** the I/O worker becomes the producer,
    so the ring carries **guest audio** (despite the “tone” label).
  - If you accidentally start a demo tone, stop it before proceeding (DevTools Console):
    ```js
    await globalThis.__aeroAudioOutput?.close?.();
    await globalThis.__aeroAudioOutputWorker?.close?.();
    await globalThis.__aeroAudioOutputHdaDemo?.close?.();
    ```
- Confirm the host-side audio status shows:
  - `AudioContext: running` (not `suspended`)
  - Ring buffer counters are visible (see §3.2).

#### 1.3 Boot Windows 7

Use the UI’s **Disks** panel + **Workers** panel to boot Windows:

- In **Disks**:
  - Click **Import file…** and select the ISO:
    - Agent path: `/state/win7.iso` (see §Prerequisites)
  - Create/import a **Windows 7 system disk** as an HDD (`*.img`), *or* create a blank HDD and mount the ISO as a CD to install once.
  - Mount:
    - Use **Mount HDD** for the system disk
    - Use **Mount CD** for the Win7 ISO (the UI infers `*.iso` as `kind=cd`)
- In **Workers**:
  - Click **Start workers** (or **Start VM**).
- Wait for Windows to reach the desktop.

Boot device selection note (BIOS `DL`):

- When booting the **installer ISO**, ensure the VM is configured to boot from **CD0** (`DL=0xE0`) *before* reset/restart.
- For normal boots after installation, ensure the VM is configured to boot from **HDD0** (`DL=0x80`) *before* reset/restart.
- Aero’s legacy BIOS boot selection is still primarily driven by an explicit boot drive number (`DL`),
  but it also supports an optional “CD-first when present” policy (attempt CD0 when install media is
  attached, otherwise fall back to the configured boot drive). See
  [`storage.md`](storage.md#boot-flows-normative).

**If you are installing from ISO:** once Windows is installed, switch the boot drive to **HDD0** (`DL=0x80`) for subsequent boots. Unmounting the ISO/CD is optional but recommended to keep the test faster and more deterministic.

---

### Driver enumeration (Device Manager)

Inside the Windows 7 guest:

1. Open **Device Manager**:
   - `Start` → right-click `Computer` → `Manage` → `Device Manager`
2. Confirm the following entries exist and have **no yellow warning icon**:
   - **System devices** → `High Definition Audio Controller`
   - **Sound, video and game controllers** → `High Definition Audio Device`

3. Confirm Windows is using the **in-box Microsoft driver**:
   - Right-click `High Definition Audio Device` → **Properties** → **Driver** tab
   - Expected: **Driver Provider** is `Microsoft`
   - Optional: click **Driver Details** and confirm `hdaudio.sys` is present

4. Optional (CLI verification):

```cmd
wmic path Win32_SoundDevice get Name,Manufacturer,Status,PNPDeviceID
```

5. Optional (bus driver verification):
   - In Device Manager: **System devices** → `High Definition Audio Controller` → **Properties** → **Driver Details**
   - Expected: `hdaudbus.sys` is present (Win7 inbox HDA bus driver).

6. Optional (DxDiag report):

```cmd
dxdiag /t %TEMP%\\dxdiag.txt
notepad %TEMP%\\dxdiag.txt
```

Expected outcome:

- Windows should use the **Microsoft** inbox driver stack automatically.
- `Control Panel → Sound` should show at least one **Playback** device and at least one **Recording** device (names may vary, but should map to the HDA device).

Artifacts (for regression tracking):

- Capture a screenshot of **Device Manager** showing:
  - `High Definition Audio Controller`
  - `High Definition Audio Device`
  - (Optional) the Driver tab showing Provider=`Microsoft`
- If running via the Aero web UI, prefer using the **Workers** panel button **“Save screenshot (png)”** (exports the guest framebuffer).
  Otherwise use a normal OS screenshot.

---

### Playback test (system sounds / WAV)

#### 3.1 Trigger playback (guest)

In Windows 7:

1. Open `Control Panel → Sound` (or run `mmsys.cpl`).
2. **Playback tab**:
   - Select the default playback device (often `Speakers` / `High Definition Audio Device`).
   - (If needed) click **Set Default**.
   - Click **Test**.
   - Tip: if you don’t see any devices, right-click the empty area and enable:
     - **Show Disabled Devices**
     - **Show Disconnected Devices**

Alternative (also deterministic):

- `Control Panel → Sound → Sounds tab` → select any “Program Events” sound → click **Test**.

Alternative (deterministic WAV file path present on all Win7 installs):

- Run this in the guest (PowerShell):

```powershell
(New-Object Media.SoundPlayer "C:\\Windows\\Media\\tada.wav").PlaySync()
```

Expected outcome:

- You hear the test sound on the host speakers/headphones.
- Windows shows playback activity (level meter/volume mixer).
  - If you don’t see the green level meter move, check **Volume Mixer** and make sure the app/device is not muted.

Artifacts (for regression tracking):

- Capture a screenshot of `mmsys.cpl` → **Playback** tab (showing the default device).
  - Prefer the Aero UI **Workers → “Save screenshot (png)”** button when available.

#### 3.2 Observe host-side metrics (AudioWorklet ring buffer)

While the guest is producing sound, capture the host-side metrics:

**Required:**

- **Ring buffer level** stays non-zero while audio plays (buffer is being filled/consumed).
- **Underruns** do not rapidly increase.
- **Overruns** remain `0` (or do not increase).
- (Optional but useful) **readFrameIndex** and **writeFrameIndex** advance over time.

Suggested pass criteria (rule-of-thumb):

- Let playback run for ~10 seconds.
- `overrunCount` should stay at `0`.
- `underrunCount` should be `0`, or at least stop increasing after startup.
  - If you see a small non-zero value at startup, treat up to **128 underrun frames** (one render quantum) as “tolerable” but still worth tracking.

Tuning notes (latency vs robustness):

- The host-side Web Audio output is created via `createAudioOutput` (`apps/web/src/platform/audio.ts`), which exposes a few knobs that
  are useful when iterating on this smoke test:
  - `startupPrefillFrames`: pre-fills silence at startup (helps avoid startup underrun spam, but adds a small amount of initial latency).
  - `discardOnResume`: discards buffered playback frames on `AudioContext` resume to avoid resuming with “stale” audio latency after
    tab backgrounding/suspensions.
  - `ringBufferFrames` / `latencyHint`: increase robustness at the cost of higher buffering/latency.
- See [`audio.md`](audio.md#createaudiooutput-options-latency-vs-robustness) for details and suggested defaults (demo vs VM mode).

Where to look:

- If the web UI has an audio status box/HUD, record the values shown:
  - `bufferLevelFrames`
  - `underrunCount` (or `underrunFrames`)
  - `overrunCount` (or `overrunFrames`)
  - `sampleRate`
  - (Optional) `baseLatencySeconds` / `outputLatencySeconds` (if exposed by the browser)

If no UI exists, use DevTools Console to read the standard ring header (layout is documented in `audio.md`):

```js
// Find the active audio output object exposed by the host UI/runtime (name may vary by build).
const out =
  globalThis.__aeroAudioOutput ??
  globalThis.__aeroAudioOutputWorker ??
  globalThis.__aeroAudioOutputVirtioSndDemo ??
  globalThis.__aeroAudioOutputHdaDemo;

out?.getMetrics?.()

// Raw ring counters (u32). Prefer using the named views (`readIndex`, `writeIndex`, etc.)
// instead of hardcoding header offsets; the canonical layout is defined in:
//   - `apps/web/src/platform/audio_worklet_ring_layout.js`
//   - re-exported by `apps/web/src/audio/audio_worklet_ring.ts`
out?.ringBuffer && {
  read: Atomics.load(out.ringBuffer.readIndex, 0) >>> 0,
  write: Atomics.load(out.ringBuffer.writeIndex, 0) >>> 0,
  underruns: Atomics.load(out.ringBuffer.underrunCount, 0) >>> 0,
  overruns: Atomics.load(out.ringBuffer.overrunCount, 0) >>> 0,
};
```

Alternative (if the web UI exposes it):

- Use **Audio → “Export audio metrics (json)”** to download a JSON blob containing:
  - AudioWorklet `getMetrics()` + output ring indices/counters
  - Producer counters (buffer level + underruns/overruns) from the active audio producer worker (CPU demo vs IO guest device)
  - Microphone ring counters (when mic capture is active)
  - Host media device inventory + mic permission state (device IDs hashed)
  - Coordinator ring attachment policy snapshot (effective/default/override owner + current attachment state)
  - WorkerCoordinator snapshot (worker states + wasm variants + last fatal/nonfatal)
  - Effective config snapshot (with sensitive fields redacted)
 - Or use **Audio → “Export audio QA bundle (tar)”** to download a single archive containing:
    - `audio-metrics.json`
    - `manifest.json` (best-effort; list of files and byte sizes included in the tar)
    - `README.txt` (bundle overview + quick interpretation hints)
    - `aero-config.json` (best-effort; effective config snapshot with sensitive fields redacted)
    - `aero.version.json` (best-effort; build/version endpoint, when served)
    - `aero.version-meta.json` (best-effort; metadata for `aero.version.json`)
    - `host-media-devices.json` (best-effort; browser media device inventory + mic permission state; device IDs are hashed)
    - `workers.json` (best-effort; WorkerCoordinator snapshot: worker states + wasm variants + last fatal/nonfatal errors)
   - `serial.txt` (best-effort; guest serial output tail, if any)
   - `audio-output-*.wav` (best-effort; snapshot of currently-buffered output ring samples, usually a few hundred ms)
   - `audio-output-*.json` (best-effort; metadata for the output WAV snapshot: sample rate, frames captured, ring indices, and quick signal stats like RMS/peak)
   - `microphone-buffered.wav` (best-effort; snapshot of currently-buffered mic ring samples)
   - `microphone-buffered.json` (best-effort; metadata for the mic WAV snapshot: sample rate, samples captured, ring counters, quick signal stats like RMS/peak, and track debug info like backend/state/settings/constraints/capabilities with device IDs hashed)
   - `audio-samples.txt` (best-effort; one-line summary of any captured WAV snapshots, including RMS/peak dBFS estimates)
   - `hda-codec-state.json` (best-effort; requires the I/O worker)
   - `hda-controller-state.bin` (best-effort; deterministic snapshot bytes of the HDA controller+codec state; no guest RAM)
   - `hda-controller-state.json` (best-effort; metadata for `hda-controller-state.bin`)
   - `hda-tick-stats.json` (best-effort; IO-worker tick clamp counters for the HDA wrapper; useful even when tracing is disabled)
   - `virtio-snd-state.bin` (best-effort; deterministic snapshot bytes of the virtio-snd PCI function state, when present)
   - `virtio-snd-state.json` (best-effort; metadata for `virtio-snd-state.bin`)
   - `screenshot-*.png` (best-effort; requires the GPU worker)
   - `screenshot.json` (best-effort; metadata for the screenshot export)
   - `trace.json` (best-effort; Chrome trace export; includes worker thread metadata and any recorded spans/counters if tracing was enabled)
   - `trace-meta.json` (best-effort; metadata for `trace.json`)
   - `perf-hud.json` (best-effort; on-page perf HUD capture export, if available)
   - `perf-hud-meta.json` (best-effort; metadata for `perf-hud.json`)

 Quick interpretation tips:

- If `writeFrameIndex` is **not** increasing: the emulator side is not producing audio (guest DMA not progressing, HDA stream not running, etc).
- If `readFrameIndex` is **not** increasing: the AudioWorklet consumer is not running (AudioContext suspended, worklet not connected, etc).

#### 3.3 Observe guest DMA progress (if debug UI exists)

If the build exposes HDA stream debug state, confirm that **guest-visible DMA progress advances** while the sound plays:

- Output stream `LPIB` increases (and wraps modulo `CBL` for cyclic buffers)
- If the **position buffer** is enabled, its value changes over time

If DMA progress is stuck (e.g. `LPIB` never moves), the guest may “think” it is playing while the host produces silence.

---

### Recording test (microphone capture)

#### 4.1 Grant browser microphone permission (host)

In the Aero web UI:

1. Click **Start microphone** (or equivalent).
2. Approve the browser permission prompt (this is a `getUserMedia({ audio: … })` request).
3. If the UI shows mic ring stats, confirm they update (e.g. buffered samples increases, dropped samples stays low).

#### 4.2 Verify the guest recording endpoint (Windows)

In Windows 7:

1. Open `Control Panel → Sound → Recording` tab.
2. Confirm a recording device exists (typically `Microphone` / `High Definition Audio Device`).
   - Tip: if you don’t see any devices, right-click the empty area and enable:
     - **Show Disabled Devices**
     - **Show Disconnected Devices**
3. Speak into the host microphone and observe the **level meter** moves.

Optional end-to-end verification (more obvious than a level meter):

- Launch **Sound Recorder** in Windows, record ~3 seconds, and play it back.

Artifacts (for regression tracking):

- Capture a screenshot of `mmsys.cpl` → **Recording** tab (showing the mic device and level meter).
  - Prefer the Aero UI **Workers → “Save screenshot (png)”** button when available.

---

### Common failure modes + what to collect

#### A) Driver / enumeration failures

- **No “High Definition Audio Controller” in Device Manager**
  - Likely PCI wiring/identity issue (class code, BAR sizing, IRQ/MSI, device not on PCI bus)
- **Yellow bang / “Unknown device”**
  - Likely config space mismatch or missing required capabilities
  - Often appears as `Multimedia Audio Controller` under `Other devices`
- **Controller exists but no “High Definition Audio Device”**
  - HDA codec enumeration or CORB/RIRB verb path issue (bus driver loaded, function driver can’t bring up codec)

Collect:

- Screenshot of Device Manager showing the device tree + error icons.
- Device Properties → Details → “Hardware Ids” for the failing entry (copy text).
- Device Properties → Driver tab (Provider, Version, Driver Details).

#### B) Playback failures

- **No sound, but Windows shows playback activity**
  - Browser audio output not started (`AudioContext` is `suspended` / autoplay blocked)
  - Output ring underrunning (producer not keeping up)
  - Guest DMA progress stuck (stream not actually running)
- **No sound and no activity meter**
  - Windows output device may not be the default (set default in `mmsys.cpl`)
  - The app/tab may be muted:
    - Windows: `sndvol.exe` (Volume Mixer)
    - Chrome: tab mute / site mute
  - Host OS output device may be incorrect (headphones vs speakers)
- **No playback devices appear in `Control Panel → Sound`**
  - Driver enumeration may be incomplete, or Windows Audio services may be stopped
  - Check `services.msc`:
    - `Windows Audio`
    - `Windows Audio Endpoint Builder`
- **Clicks/stutter**
  - Frequent underruns (buffer too small or CPU stalls)
- **Overruns increasing**
  - Producer writing too fast or not respecting backpressure
- **Ring indices don’t move**
  - `writeFrameIndex` stuck: emulator isn’t writing to the ring (guest DMA / HDA stream / wiring issue)
  - `readFrameIndex` stuck: AudioWorklet not consuming (AudioContext suspended / node not connected)

Collect:

- Audio ring metrics: buffer level + underrun/overrun counters + `AudioContext.state` + `sampleRate`.
- (Optional) `baseLatencySeconds` / `outputLatencySeconds` from `out.getMetrics()` (if exposed by the browser); useful for comparing real host playback latency across platforms.
- If available: runtime/worker “producer” counters (how full the emulator-to-worklet ring is from the producer’s point of view).
- If available: buffered WAV snapshot summary (`audio-samples.txt` in the QA bundle) and/or `audio-output-*.json` signal stats:
  - `signal.rms` / `signal.peakAbs` and their dBFS estimates help quickly distinguish “ring contains silence” from “ring contains real audio but host output is muted/broken”.
- If available: guest HDA stream debug (`LPIB`, `CBL`, position buffer).
- Browser console logs (preserve timestamps; include `[cpu]`/`[io]` worker logs if present).
- If Perf tracing is enabled in the build, export a trace (it should include `audio.*` counters when the host is sampling audio metrics).
  - See: [`performance.md`](performance.md)

#### C) Microphone failures

- **No recording device in Windows**
  - Capture pin/stream not exposed or codec topology incomplete
- **Device exists but level meter never moves**
  - `getUserMedia` denied / not started, host mic muted, or capture ring not being drained into guest DMA
  - Browser may be blocking mic access:
    - Chrome: site icon → **Site settings** → Microphone → Allow
  - Another application may already be using the microphone exclusively (host OS dependent).

Collect:

- Browser permission status + any `getUserMedia` error shown in the console.
- If available: host media device inventory (from the QA bundle `host-media-devices.json`), including the `microphonePermissionState` and the list of detected `audioinput` devices (IDs are hashed).
- Mic ring stats (buffered/dropped) if available in UI.
- If using QA exports: capture backend from `microphone-buffered.json` (`backend=worklet` vs `backend=script`). `script` indicates a ScriptProcessorNode fallback (higher latency) and can explain timing-related capture issues.
- Optional: if the web UI exposes it, use:
  - **Audio → “Export HDA codec state (json)”** (downloads the same gating state without opening the I/O worker console).
  - **Audio → “Export HDA controller state (bin)”** (deterministic snapshot bytes; useful for low-level reproduction/debugging).
- Or use **Audio → “Export audio QA bundle (tar)”** to grab all relevant host-side artifacts in one file.
- **HDA codec gating state** (pin/power/amp) from the I/O worker DevTools console:
  ```js
  __aeroAudioHdaBridge?.codec_debug_state?.()
  ```
  - If `captureEnabled` is `false`, the failure is likely **gating** (pin widget control / power state / amp mute) rather than DMA/stream plumbing.
- Screenshot of the guest Recording tab.

#### D) Snapshot for bug reports (when available)

If the runtime exposes a snapshot/save-state UI, save a snapshot immediately after reproducing:

- Note the snapshot path (often `state/worker-vm-autosave.snap` in OPFS).
- Export it for sharing. Example DevTools snippet (OPFS):

```js
const root = await navigator.storage.getDirectory();
const fh = await root.getFileHandle("state/worker-vm-autosave.snap");
const file = await fh.getFile();
const url = URL.createObjectURL(file);
const a = document.createElement("a");
a.href = url;
a.download = "worker-vm-autosave.snap";
a.click();
```

---

### Appendix: what to include in a bug report

When reporting a Win7 audio regression, include (at minimum):

- **Build info**
  - Git commit SHA (or release version)
    - Recommended: open `/<origin>/aero.version.json` (same origin as the web UI) and attach the JSON.
    - Or in DevTools Console (if present): `__AERO_BUILD_INFO__`
  - Browser + version (e.g. Chrome 123)
  - OS + audio output device (optional but helpful)
- **Host capability checks**
  - `crossOriginIsolated` + `typeof SharedArrayBuffer` (see §Prerequisites)
  - Output `AudioContext.sampleRate` (from `out.getMetrics().sampleRate` if available)
- **Guest evidence**
  - Device Manager screenshot showing:
    - `High Definition Audio Controller`
    - `High Definition Audio Device`
  - Driver Provider on the audio function device (expected: `Microsoft`)
- **Runtime metrics/logs**
  - Ring buffer counters (buffer level + underruns + overruns)
  - (Optional) `baseLatencySeconds` / `outputLatencySeconds` from `out.getMetrics()` (if available)
  - Browser console log output during the repro (including worker-prefixed logs like `[cpu]`, `[io]` if present)
  - If tracing is available, a **trace export** taken during the repro (Perf HUD → Trace Start/Stop → Trace JSON)
    - See: [`performance.md`](performance.md)
  - Snapshot file exported after repro (if available)
- For driver binding issues: a snippet from `C:\Windows\inf\setupapi.dev.log` around the audio device’s Hardware ID can be extremely helpful.
