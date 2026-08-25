# Windows 7 virtio driver implementation

> How the four virtio guest drivers are built: the shared modern-PCI transport,
> interrupt handling and its failure modes, the DMA strategy, the audio
> miniport, and the per-device specifics for block, network, input, and sound —
> together with their test plans.
>
> The binding contract these drivers must satisfy is
> [windows7-virtio-driver-contract.md](windows7-virtio-driver-contract.md);
> the queue mechanics are in
> [windows7-virtqueue-split-ring.md](./windows7-virtqueue-split-ring.md).

## Goal

Enable Aero’s **virtio acceleration path** by making it straightforward to install Windows 7 drivers for:

- **virtio-blk** (storage) *(minimum deliverable)*
- **virtio-net** (network) *(minimum deliverable)*
- **virtio-input** (keyboard/mouse/tablet) *(best-effort; PS/2/USB HID remains fallback)*
- **virtio-snd** (audio) *(optional; HDA remains fallback; AC’97 is legacy-only)*

See also:

- [`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) — Aero’s definitive device/feature/transport contract.
- [`virtio/virtqueue-split-ring-win7.md`](windows7-virtio-driver-contract.md) — split-ring virtqueue implementation reference for Win7 KMDF drivers (descriptor mgmt, ordering/barriers, EVENT_IDX, indirect).
- [`16-windows7-driver-build-and-signing.md`](../areas/windows-drivers.md) — CI + local pipeline for building/cataloging/signing the in-tree Win7 driver packages (`drivers-win7.yml`).
- virtio-input end-to-end test plan (device model + Win7 driver + web runtime): [`test-plans/virtio-input.md`](../areas/windows-drivers.md)

This document defines:

- The chosen driver approach (and licensing rationale)
- The required on-disk packaging layout (`.inf` + `.sys` + `.cat`, plus any INF-referenced payload files such as `WdfCoInstaller*.dll`)
- A reproducible “drivers ISO” build flow for the emulator UX
- Windows 7 installation steps (Device Manager + `pnputil`)
- Win7 x64 test-signing mode flow + test certificate tooling
- Optional offline driver injection into Windows install images (DISM)

See also:

- `drivers/README.md` (how Aero builds/ships driver packs in practice)
- `../areas/windows-drivers.md` (end-user flow: Guest Tools, then switch to virtio)
- `../areas/windows-drivers.md` (detailed Win7 x64 offline servicing: cert injection + BCD test signing)
- `windows7-virtio-driver-contract.md` (Aero-specific virtio device IDs + contract)

---

## Packaging options (Guest Tools)

Aero can build Guest Tools media (`aero-guest-tools.iso` / `.zip`) from a few different driver sources:

1) **CI/release in-tree drivers (official artifacts)**
   - CI builds signed packages under `out/packages/**` and a public signing cert under `out/certs/`.
   - In GitHub Actions these are published as:
     - `win7-drivers-signed-packages` (`out/packages/**` + `out/certs/aero-test.cer`)
     - `aero-guest-tools` (`aero-guest-tools.iso/.zip` + manifest)
   - Scripts:
     - `drivers/build/package-guest-tools.ps1`
     - `drivers/scripts/make-guest-tools-from-ci.ps1` (convenience wrapper)
   - CI/release spec: `tools/packaging/specs/win7-signed.json`
   - Outputs:
     - `aero-guest-tools.iso`
     - `aero-guest-tools.zip`
     - `manifest.json`
     - `aero-guest-tools.manifest.json` (alias used by CI/release asset publishing)

2) **Upstream virtio-win** (`viostor`, `netkvm`, etc.) *(optional / compatibility)*
   - Script: `drivers/scripts/make-guest-tools-from-virtio-win.ps1`
   - Spec (via `-Profile`):
      - Default (`-Profile full`): `tools/packaging/specs/win7-virtio-full.json` (expects modern IDs for core devices; `AERO-W7-VIRTIO` v1 is modern-only; includes optional `vioinput`/`viosnd` when present for **both** x86 and amd64)
      - Optional (`-Profile minimal`): `tools/packaging/specs/win7-virtio-win.json` (storage+network only)
   - Device contract (for generated `config/devices.cmd`): `protocol-vectors/windows-device-contract-virtio-win.json`

3) **In-tree Aero virtio** (`aero_virtio_blk`, `aero_virtio_net`, etc.) *(local/dev)*
   - Script: `drivers/scripts/make-guest-tools-from-aero-virtio.ps1`
   - Spec: `tools/packaging/specs/win7-aero-virtio.json` (**modern-only** IDs; rejects virtio-pci transitional IDs at packaging time)
   - Device contract (for generated `config/devices.cmd` when using the CI packaging wrapper): `protocol-vectors/windows-device-contract.json`

See also:

- `../areas/windows-drivers.md` (specs, inputs/outputs, signing policy)
- `drivers/README.md` (CI artifacts + virtio-win alternative tooling)

## Chosen approach (what Aero ships): CI-built in-tree drivers

Aero’s official Windows 7 driver artifacts (driver bundles + Guest Tools media) are produced by CI from the in-repo driver sources and published via the `drivers-win7.yml` / `release-drivers-win7.yml` workflows.

The virtio-win packaging flow remains supported as an **alternative/compatibility** path (for example: to compare behavior/performance against upstream drivers or to bring your own WHQL/production-signed virtio stack).

### Why

Shipping CI-built in-tree drivers keeps Aero releases reproducible and ensures the packaged HWIDs/service-name contract stays aligned with the emulator + Guest Tools scripts.

The **virtio-win** packaging flow remains available as an optional compatibility/testing path.

### virtio-win compatibility mapping

When using the optional virtio-win flow, we target the **virtio-win** driver distribution (commonly shipped as `virtio-win.iso`) and specifically the packages:

| Aero device | Windows PCI HWID (Aero / `AERO-W7-VIRTIO` v1) | virtio-win package name (typical) |
|------------|----------------------------------------|-----------------------------------|
| virtio-net | `PCI\VEN_1AF4&DEV_1041&REV_01` | `NetKVM` (`netkvm.inf` / `netkvm.sys`) |
| virtio-blk | `PCI\VEN_1AF4&DEV_1042&REV_01` | `viostor` (`viostor.inf` / `viostor.sys`) |
| virtio-input | `PCI\VEN_1AF4&DEV_1052&REV_01` | `vioinput` (best-effort; Win7 package not present in all virtio-win releases) |
| virtio-snd | `PCI\VEN_1AF4&DEV_1059&REV_01` | `viosnd` (optional; Win7 package not present in all virtio-win releases) |

Notes:

- `VEN_1AF4` is the conventional VirtIO PCI vendor ID used by the upstream ecosystem.
- Aero’s virtio device contract is `AERO-W7-VIRTIO` (see `windows7-virtio-driver-contract.md`) and is **modern-only**:
  - virtio-pci vendor capabilities + BAR0 MMIO (no legacy I/O BAR)
  - PCI Revision ID `0x01`
  - device IDs in the virtio 1.0+ modern space (`0x1040 + <virtio device id>`)
- Many upstream virtio-win drivers also match the virtio-pci **transitional** ID range (the older `0x1000..` device IDs),
  but Aero contract v1 does not require those IDs. The web runtime can optionally expose virtio-net / virtio-input / virtio-snd as **transitional** or **legacy-only** devices (including the legacy I/O port BAR) via `virtioNetMode`, `virtioInputMode`, and `virtioSndMode`; see [`16-virtio-pci-legacy-transitional.md`](../areas/windows-drivers.md).
- The Aero contract major version is encoded in the PCI **Revision ID** (contract v1 = `REV_01`). QEMU virtio devices commonly enumerate as `REV_00` by default.
  - Aero’s in-tree Win7 virtio driver packages are revision-gated (`&REV_01`), and some drivers also validate the revision at runtime.
  - For QEMU-based testing with strict contract-v1 drivers, pass `x-pci-revision=0x01` on each `-device virtio-*-pci,...` arg (the Win7 host harness under `drivers/windows7/tests/host-harness/` does this automatically).

### Contract ↔ in-tree drivers ↔ Guest Tools config (virtio)

For Aero’s in-tree drivers and Guest Tools installer logic, the identifiers below must match **exactly**:

| Device | Contract PCI ID | In-tree driver INF | Windows service name | Guest Tools config |
|---|---|---|---|---|
| virtio-net | `1AF4:1041` (REV `0x01`) | `drivers/windows7/virtio-net/inf/aero_virtio_net.inf` | `aero_virtio_net` | `guest-tools/config/devices.cmd`: `AERO_VIRTIO_NET_SERVICE`, `AERO_VIRTIO_NET_HWIDS` |
| virtio-blk | `1AF4:1042` (REV `0x01`) | `drivers/windows7/virtio-blk/inf/aero_virtio_blk.inf` | `aero_virtio_blk` | `guest-tools/config/devices.cmd`: `AERO_VIRTIO_BLK_SERVICE`, `AERO_VIRTIO_BLK_HWIDS` |
| virtio-input | `1AF4:1052` (REV `0x01`) | `drivers/windows7/virtio-input/inf/aero_virtio_input.inf` | `aero_virtio_input` | `guest-tools/config/devices.cmd`: `AERO_VIRTIO_INPUT_SERVICE`, `AERO_VIRTIO_INPUT_HWIDS` |
| virtio-snd | `1AF4:1059` (REV `0x01`) | `drivers/windows7/virtio-snd/inf/aero_virtio_snd.inf` | `aero_virtio_snd` | `guest-tools/config/devices.cmd`: `AERO_VIRTIO_SND_SERVICE`, `AERO_VIRTIO_SND_HWIDS` |

Guest Tools uses:

- `AERO_VIRTIO_BLK_SERVICE` to configure the storage service as `BOOT_START` and to pre-seed `CriticalDeviceDatabase`.
- `AERO_VIRTIO_*_HWIDS` to enumerate the hardware IDs the installer should expect. For `AERO-W7-VIRTIO` v1, include `&REV_01` patterns for contract-major safety (the in-tree `aero_virtio_{blk,net,input,snd}.inf` files match `...&REV_01`, and some drivers also validate the revision at runtime).

Note: `guest-tools/config/devices.cmd` is generated from a Windows device contract JSON
(see `scripts/generate-guest-tools-devices-cmd.py`) and is regenerated during Guest Tools packaging
from the contract passed to the packager (`drivers/build/package-guest-tools.ps1 -WindowsDeviceContractPath`):

- `protocol-vectors/windows-device-contract.json` (canonical; Aero in-tree driver service names like `aero_virtio_blk` / `aero_virtio_net`)
- `protocol-vectors/windows-device-contract-virtio-win.json` (virtio-win; upstream service names like `viostor` / `netkvm`)

CI-style drift check (no rewrite):

```bash
python3 scripts/ci/gen-guest-tools-devices-cmd.py --check
```

Virtio-win Guest Tools builds must use the virtio-win contract so `guest-tools/setup.cmd` can
validate the boot-critical storage INF `AddService` name and pre-seed `CriticalDeviceDatabase`
without requiring `/skipstorage` while keeping Aero’s modern-only PCI HWID patterns (`REV_01`).

### Licensing policy (virtio-win-derived artifacts)

Aero aims for permissive licensing (MIT/Apache-2.0). Official CI/release artifacts ship the in-tree drivers from this repo under the project licenses. If you redistribute **virtio-win-derived** media (driver packs / Guest Tools built from `virtio-win.iso`), you must comply with virtio-win’s licensing/redistribution terms and ship the corresponding license texts/notices.

**Upstream reference points (to pin during implementation):**

- Driver sources: `kvm-guest-drivers-windows` (virtio-win project)
- Binary distribution: `virtio-win.iso` (virtio-win project)

In practice, virtio-win’s driver sources are typically distributed under a **BSD-style permissive license** (commonly BSD-3-Clause), which is compatible with Aero’s licensing goals, but Aero should still pin a version and ship the exact license texts for the specific artifacts it redistributes.

**Repository policy:**

- If Aero **vendors** virtio-win artifacts (or a source subtree), we must include:
  - the upstream license texts
  - a pinned upstream version/commit reference
  - a `THIRD_PARTY_NOTICES.md` attribution file (or equivalent redistribution notice document)
- If Aero chooses not to vendor binaries and instead requires users to supply them, we still document the flow, but Aero is no longer “shipping” the drivers.

This repo provides a **packaging + ISO build story** that works either way. The directory layout and tooling are designed so that later work can:

- vendor a pinned virtio-win subset directly into `drivers/virtio/prebuilt/`, or
- populate `drivers/virtio/prebuilt/` from an externally obtained virtio-win ISO.

Practical note: this repo generally avoids committing `.sys` driver binaries directly. Instead, it provides tooling to build/pin driver packs and emits installable artifacts via CI/release workflows.

---

## Packaging layout (virtio-win driver packs)

The virtio-win extraction tooling (`drivers/scripts/make-driver-pack.ps1` and related wrappers) uses the following layout under `drivers/virtio/`:

```
drivers/virtio/
  manifest.json
  THIRD_PARTY_NOTICES.md
  prebuilt/                  # where real driver files go
    win7/
      x86/
        viostor/
          viostor.inf
          viostor.sys
          viostor.cat
        netkvm/
          netkvm.inf
          netkvm.sys
          netkvm.cat
        vioinput/            # best-effort
        viosnd/              # optional
      amd64/                 # recommended (required for Win7 x64 Setup + x64 guests)
        viostor/
        netkvm/
        ...
  sample/                    # repo-owned placeholders used by CI/tests
    ...
```

Notes:

- For the virtio-win-derived pack, Aero only **requires** `viostor` (storage) + `netkvm` (network).
- `vioinput` and `viosnd` are optional and may be missing from some virtio-win versions. The packaging scripts handle this by default, but will only include them when they exist for **both** x86 and amd64; otherwise they’re omitted (unless `-StrictOptional` is used).
- CI/release Guest Tools uses **in-tree** driver packages and a different spec/driver naming (`virtio-blk`, `virtio-net`, ...). See `drivers/README.md`.

### Why we require `.inf` + `.sys` + `.cat`

- Windows installs PnP drivers via an **INF**.
- On x64, Windows requires kernel drivers to be **signed**; the signature is normally validated via the **catalog (`.cat`)** for the driver package.
- On Win7 x86, signature enforcement is generally off by default, but keeping `.cat` in the package makes the flow consistent.

---

## Building the “drivers ISO” (emulator UX integration)

Aero should expose a UX action like:

> “Mount driver ISO…” → select `aero-virtio-win7-drivers.iso`

The ISO is a simple ISO-9660/Joliet filesystem containing the `win7/` driver directories.

### Which ISO?

There are two related but distinct “driver ISO” concepts in Aero:

1) **Minimal virtio drivers ISO** (`aero-virtio-win7-drivers.iso`)
   - Contains only the extracted virtio driver packages under `win7/x86/...` and `win7/amd64/...`.
   - Intended for Windows Setup’s **Load Driver** flow (so Setup can see a virtio-blk boot disk).

2) **Aero Guest Tools ISO** (`aero-guest-tools.iso`)
   - Contains install scripts (`setup.cmd`), the driver payload under `drivers/x86/...` and `drivers/amd64/...`, and (optionally) certificate files under `certs/` depending on `manifest.json` `signing_policy`.
   - Intended for the recommended post-install flow:
       - install Win7 using baseline emulated devices (AHCI HDD + IDE/ATAPI CD-ROM, e1000)
         - see [`../areas/storage.md`](../areas/storage.md) for the canonical Win7 storage topology
         - boot selection note: Aero BIOS boot selection is still primarily driven by an explicit boot drive number (`DL`),
           but it also supports an optional “CD-first when present” policy (attempt CD0 when install media is attached, otherwise
           fall back to the configured boot drive).
           - installer ISO boot: `DL=0xE0` (CD0)
           - post-install boot: `DL=0x80` (HDD0)
           - see [`../areas/windows-drivers.md`](../areas/windows-drivers.md) and [storage boot flows](../areas/storage.md#boot-flows-normative)
       - mount Guest Tools ISO
       - run `setup.cmd`
       - switch VM devices to virtio and reboot

This document covers both; see the sections below.

### Tooling provided

- `tools/driver-iso/build.py` builds an ISO from a driver root directory and `drivers/virtio/manifest.json`.
- `tools/driver-iso/verify_iso.py` verifies the ISO contains the required INF files.

The builder prefers the deterministic in-tree Rust ISO writer (`aero_iso`) when `cargo` is available (or when `--backend rust` is used).
If Rust/cargo is unavailable, it falls back to external ISO tooling (Linux: `xorriso`/`genisoimage`/`mkisofs`; Windows: `oscdimg` or IMAPI).

For deterministic builds, set `--source-date-epoch` (or `SOURCE_DATE_EPOCH`) to a fixed value (for example, `0`).

The verifier (`verify_iso.py`) can list ISO contents using the in-tree Rust Joliet parser when `cargo` is available, and otherwise falls back to `pycdlib`/`xorriso`/PowerShell mounting depending on platform.

### Multi-arch driver ISOs are recommended (x86 + amd64)

Windows will only load kernel drivers that match the OS architecture:

- **Windows 7 x64 Setup** (“Load Driver”) requires **amd64** drivers.
- **Windows 7 x86 Setup** requires **x86** drivers.

To prevent accidentally producing an ISO that can’t be used for Win7 x64 installs, `tools/driver-iso/build.py`
and `tools/driver-iso/verify_iso.py` default to:

```text
--require-arch both
```

This requires the minimum driver set (at least `viostor` + `netkvm`) to be present for **both** architectures.

Example (from repo root):

```bash
python3 tools/driver-iso/build.py \
  --drivers-root drivers/virtio/prebuilt \
  --output dist/aero-virtio-win7-drivers.iso
```

### Building a single-arch ISO (x86-only or amd64-only)

If you intentionally want a single-arch ISO, pass `--require-arch`:

```bash
# Win7 x86-only drivers ISO (will NOT work for Win7 x64 installs)
python3 tools/driver-iso/build.py \
  --require-arch x86 \
  --drivers-root drivers/virtio/prebuilt \
  --output dist/aero-virtio-win7-drivers-x86.iso

# Win7 amd64-only drivers ISO
python3 tools/driver-iso/build.py \
  --require-arch amd64 \
  --drivers-root drivers/virtio/prebuilt \
  --output dist/aero-virtio-win7-drivers-amd64.iso
```

The verifier supports the same flag:

```bash
python3 tools/driver-iso/verify_iso.py \
  --require-arch x86 \
  --iso dist/aero-virtio-win7-drivers-x86.iso
```

To build a demo ISO from placeholders:

```bash
python3 tools/driver-iso/build.py \
  --drivers-root drivers/virtio/sample \
  --output dist/aero-virtio-win7-drivers-sample.iso
```

### Build an ISO from an upstream virtio-win ISO (optional / compatibility)

On Windows you can mount `virtio-win.iso` directly via `Mount-DiskImage`.

On Linux/macOS, you can:

- extract first with `tools/virtio-win/extract.py` and then pass `-VirtioWinRoot`, or
- run under `pwsh` and pass `-VirtioWinIso` (auto-extract fallback when `Mount-DiskImage` is unavailable or fails), or
- use the one-shot `.sh` wrappers under `drivers/scripts/`.

Quick one-liner wrapper (does both steps):

```powershell
powershell -ExecutionPolicy Bypass -File .\drivers\scripts\make-virtio-driver-iso.ps1 `
  -VirtioWinIso C:\path\to\virtio-win.iso `
  -OutIso .\dist\aero-virtio-win7-drivers.iso
```

For a deterministic ISO build (recommended), force the Rust backend and a fixed timestamp:

```powershell
powershell -ExecutionPolicy Bypass -File .\drivers\scripts\make-virtio-driver-iso.ps1 `
  -VirtioWinIso C:\path\to\virtio-win.iso `
  -OutIso .\dist\aero-virtio-win7-drivers.iso `
  -IsoBackend rust `
  -SourceDateEpoch 0
```

1) Extract a Win7 driver pack from the ISO:

```powershell
powershell -ExecutionPolicy Bypass -File .\drivers\scripts\make-driver-pack.ps1 `
  -VirtioWinIso C:\path\to\virtio-win.iso `
  -NoZip
```

By default this requires `viostor` + `netkvm` and attempts to include `vioinput` + `viosnd` best-effort.

Important: optional drivers are included only when present for **both** x86 and amd64; if an optional driver exists for only one architecture in the virtio-win source, it is omitted from both (with a warning) so Guest Tools packaging never ships a one-arch-only optional driver. To control this explicitly:

- Minimal pack: `-Drivers viostor,netkvm`
- Strict optional (fail if audio/input are missing): `-StrictOptional`

On Linux/macOS:

Option A (recommended): extract first, then use `-VirtioWinRoot`:

```bash
python3 tools/virtio-win/extract.py \
  --virtio-win-iso virtio-win.iso \
  --out-root /tmp/virtio-win-root

pwsh drivers/scripts/make-driver-pack.ps1 -VirtioWinRoot /tmp/virtio-win-root -NoZip
```

Option B: pass `-VirtioWinIso` directly under `pwsh` (auto-extract fallback on non-Windows when mounting is unavailable or fails):

```bash
pwsh drivers/scripts/make-driver-pack.ps1 -VirtioWinIso virtio-win.iso -NoZip
```

Notes:

- `tools/virtio-win/extract.py` prefers `7z` (install `p7zip-full` on Ubuntu/Debian or `p7zip` via Homebrew on macOS).
  If you don’t have `7z`, install `pycdlib` (`python3 -m pip install pycdlib`) and pass `--backend pycdlib`.
- `drivers/scripts/make-driver-pack.ps1` requires PowerShell 7 (`pwsh`) on non-Windows hosts.
- See `tools/virtio-win/README.md` for details (what is extracted, backends, provenance fields).
- Convenience: `bash ./drivers/scripts/make-driver-pack.sh` wraps the extraction + `pwsh` invocation into one command on Linux/macOS.
- Convenience: `bash ./drivers/scripts/make-virtio-driver-iso.sh` and `bash ./drivers/scripts/make-guest-tools-from-virtio-win.sh` provide one-shot wrappers for building the drivers ISO and Guest Tools media on Linux/macOS.

`tools/virtio-win/extract.py` also writes a machine-readable provenance file to:

- `/tmp/virtio-win-root/virtio-win-provenance.json`

`drivers/scripts/make-driver-pack.ps1` will ingest this file when present so the produced
driver pack `manifest.json` can record the original ISO hash/volume label even when using
`-VirtioWinRoot`.

The extractor also copies common root-level license/notice files (e.g. `LICENSE*`, `NOTICE*`,
`README*`) and small metadata files like `VERSION` into the extracted root so subsequent
packaging can propagate them (and derive a best-effort virtio-win version string) even on
non-Windows hosts.

This produces a staging directory (by default) at:

`drivers\out\aero-win7-driver-pack\`

The staging directory includes:

- `manifest.json` (provenance, including virtio-win ISO hash/volume label/version hints when available; also records which upstream license/notice files were copied when present)
- `THIRD_PARTY_NOTICES.md` (redistribution notices)
- `licenses/virtio-win/` (best-effort copy of upstream virtio-win license/notice files)

2) Build a mountable drivers ISO from that staging directory:

```powershell
python .\tools\driver-iso\build.py `
  --drivers-root .\drivers\out\aero-win7-driver-pack `
  --output .\dist\aero-virtio-win7-drivers.iso
```

Notes:

- `tools/driver-iso/build.py` prefers Rust/cargo (`--backend rust`) for deterministic ISO builds.
  If `cargo` is unavailable, you’ll need an external ISO authoring tool:
  - Linux/WSL: `xorriso` is easiest
  - Windows: `oscdimg.exe` (Windows ADK) is commonly used (or rely on the IMAPI fallback)

### Build `aero-guest-tools.iso` from a virtio-win ISO (optional / compatibility)

This produces a Guest Tools ISO that includes virtio drivers plus install scripts.

By default, the wrapper builds media with `signing_policy=none` (for WHQL/production-signed virtio-win drivers), so it does **not** require or inject any custom certificate files and `setup.cmd` will not prompt to enable Test Mode by default.

On a machine with Rust (`cargo`) installed:

```powershell
powershell -ExecutionPolicy Bypass -File .\drivers\scripts\make-guest-tools-from-virtio-win.ps1 `
  -VirtioWinIso C:\path\to\virtio-win.iso `
  -OutDir .\dist\guest-tools `
  -Version 0.0.0 `
  -BuildId local
```

By default, the wrapper uses `-Profile full` (includes optional Win7 audio/input drivers when present for **both** x86 and amd64; best-effort).

To build storage+network-only Guest Tools media, use:

```powershell
powershell -ExecutionPolicy Bypass -File .\drivers\scripts\make-guest-tools-from-virtio-win.ps1 `
  -VirtioWinIso C:\path\to\virtio-win.iso `
  -Profile minimal `
  -OutDir .\dist\guest-tools `
  -Version 0.0.0 `
  -BuildId local
```

If you are packaging **test-signed/custom-signed** drivers (not typical for virtio-win), you can override:

```powershell
powershell -ExecutionPolicy Bypass -File .\drivers\scripts\make-guest-tools-from-virtio-win.ps1 `
  -VirtioWinIso C:\path\to\virtio-win.iso `
  -Profile full `
  -OutDir .\dist\guest-tools `
  -Version 0.0.0 `
  -BuildId local `
  -SigningPolicy test
```

On Linux/macOS, you can either extract first and pass `-VirtioWinRoot`, pass `-VirtioWinIso` directly under `pwsh`
(auto-extract fallback when mounting is unavailable or fails), or use the `.sh` wrapper (`bash ./drivers/scripts/make-guest-tools-from-virtio-win.sh`):

```bash
python3 tools/virtio-win/extract.py \
  --virtio-win-iso virtio-win.iso \
  --out-root /tmp/virtio-win-root

pwsh drivers/scripts/make-guest-tools-from-virtio-win.ps1 \
  -VirtioWinRoot /tmp/virtio-win-root \
  -OutDir ./dist/guest-tools \
  -Version 0.0.0 \
  -BuildId local
```

This wrapper:

1. Extracts a Win7 driver pack from `virtio-win.iso` (using `drivers/scripts/make-driver-pack.ps1`).
2. Converts it into the input layout expected by the Rust Guest Tools packager.
3. Runs `tools/packaging/aero_packager/` with the selected packaging profile (default: `full`):
   - `-Profile full` (default): `tools/packaging/specs/win7-virtio-full.json`
      - required: `viostor` + `netkvm`
      - optional (included only if present for **both** x86 and amd64): `vioinput` + `viosnd`
   - `-Profile minimal`: `tools/packaging/specs/win7-virtio-win.json` (required: `viostor` + `netkvm`)

Advanced overrides:

- `-SpecPath` overrides the profile’s spec selection.
- `-Drivers` overrides the profile’s driver extraction list.

Outputs:

- `dist/guest-tools/aero-guest-tools.iso`
- `dist/guest-tools/aero-guest-tools.zip`
- `dist/guest-tools/manifest.json`

The Guest Tools ISO/zip root also includes `THIRD_PARTY_NOTICES.md`, and will include
upstream virtio-win license/notice files (if present) under `licenses/virtio-win/`
(including `driver-pack-manifest.json` for virtio-win ISO provenance).

---

## Installing on Windows 7

For most users, the recommended installation path is **Aero Guest Tools** (`aero-guest-tools.iso`) and `setup.cmd`
(`../areas/windows-drivers.md`). The steps below are primarily useful for:

- installing storage drivers during **Windows Setup** (“Load driver”), or
- manual troubleshooting / non-standard packaging flows.

### virtio-blk (storage) during Windows 7 setup (“Load driver”)

If the Windows installer can’t see the disk:

1. Boot the Windows 7 installer ISO.
2. When you reach “Where do you want to install Windows?”, choose **Load Driver**.
3. Attach one of the following driver media sources:
   - **CI/release driver bundles** (`win7-drivers`): mount `AeroVirtIO-Win7-<version>.iso` or attach `*-fat.vhd` as a secondary disk (see [`../areas/windows-drivers.md`](../areas/windows-drivers.md) and `INSTALL.txt` at the media root for the exact layout).
   - **virtio-win-derived drivers ISO** (optional/compatibility): mount `aero-virtio-win7-drivers.iso` built by `drivers/scripts/make-virtio-driver-iso.ps1`.
4. Select the storage driver INF for your media:
   - **Aero in-tree virtio-blk**: `aero_virtio_blk.inf` (find it under the attached media; the directory names differ between Guest Tools vs driver bundle artifacts).
   - **virtio-win virtio-blk**: `viostor.inf` (typically under `\win7\amd64\viostor\` or `\win7\x86\viostor\`).
5. The virtio disk should appear; continue installation.

### Post-install via Device Manager (net / input / snd)

1. Boot Windows.
2. Open **Device Manager**.
3. For each unknown device (virtio-net, virtio-input, virtio-snd):
   - Right click → **Update Driver Software…**
   - “Browse my computer for driver software”
   - Point it at the mounted driver media (Guest Tools ISO/zip, CI driver bundle ISO, or virtio-win-derived drivers ISO)
   - Enable “Include subfolders”

### Post-install via pnputil

Windows 7 includes `pnputil.exe` (limited compared to newer Windows):

```bat
REM Example: virtio-win-derived drivers ISO
pnputil -i -a D:\win7\amd64\viostor\viostor.inf
pnputil -i -a D:\win7\amd64\netkvm\netkvm.inf

REM Example: Aero Guest Tools media (in-tree drivers)
pnputil -i -a X:\drivers\amd64\aero_virtio_blk\aero_virtio_blk.inf
pnputil -i -a X:\drivers\amd64\aero_virtio_net\aero_virtio_net.inf
```

Replace `D:` / `X:` with your mounted media drive letter and use the correct architecture directory
(`x86` vs `amd64` / `x64`) for your guest.

If you’re using the FAT driver disk image instead, the layout is typically:

- `E:\x86\<driver>\*.inf`
- `E:\x64\<driver>\*.inf`

---

## Win7 x64: test-signing mode + test certificate tooling

Windows 7 x64 enforces driver signature checks. There are three practical scenarios:

1. **Using Aero CI/release driver artifacts (in-tree drivers, typically test-signed)**: requires trusting the signing certificate and enabling Test Signing (or using `nointegritychecks`, not recommended). Guest Tools media (`signing_policy=test`) guides this flow.
2. **Using upstream WHQL/production-signed virtio-win packages**: should install without enabling test mode.
3. **Using modified drivers / self-built drivers**: requires test signing mode (or a production code-signing certificate + cross-signing, which is out of scope).

### SHA-256 signatures (Win7 update requirement)

If the driver catalogs (`.cat`) are signed with **SHA-256**, Windows 7 needs **KB3033929** to validate those signatures. Without it, drivers may fail to load with **Code 52** (“Windows cannot verify the digital signature…”).

This is a common failure mode when using modern tooling to sign Win7 drivers.

### Enable test-signing mode (Win7 x64)

Run an elevated Command Prompt:

```bat
bcdedit /set testsigning on
shutdown /r /t 0
```

To disable:

```bat
bcdedit /set testsigning off
shutdown /r /t 0
```

### Generate a test certificate + sign drivers

See `tools/driver-signing/README.md` and scripts in `tools/driver-signing/`.

High-level process:

1. Create a code-signing certificate (self-signed is fine for test mode).
2. Import it into:
   - Trusted Root Certification Authorities
   - Trusted Publishers
3. Use the WDK `signtool.exe` to sign the `.cat` (and optionally `.sys`) files.

---

## Optional: offline injection into Windows install media (DISM)

If you want Windows Setup to “just work” with virtio storage without clicking “Load Driver”, inject drivers into `boot.wim` and `install.wim` using DISM on a Windows host:

1. Mount the image:
   - `dism /Mount-Wim /WimFile:X:\sources\boot.wim /Index:2 /MountDir:C:\mount\boot`
2. Add drivers:
   - `dism /Image:C:\mount\boot /Add-Driver /Driver:C:\drivers\win7\amd64\viostor /Recurse`
3. Commit/unmount:
   - `dism /Unmount-Wim /MountDir:C:\mount\boot /Commit`

Repeat for `install.wim` (pick the correct edition index).

### Important: test-signed drivers also require offline certificate trust

If the driver package you’re injecting is **test-signed** (common for development), you must also inject the public signing certificate into the offline certificate stores for **both**:

- `boot.wim` (WinPE / Setup index, typically 2)
- `install.wim` (the edition index you plan to install)

At minimum, inject into:

- `ROOT`
- `TrustedPublisher`

Recommended tooling (Windows host):

- End-to-end media servicing: `tools/windows/patch-win7-media.ps1`
- Offline hive injector: `tools/win-offline-cert-injector` (`win-offline-cert-injector --windows-dir <mount> --store ROOT --store TrustedPublisher <cert-file>...`)

For a much more complete, Win7 x64-focused servicing procedure (certificate injection + BCD template patching), see `../areas/windows-drivers.md` and the helper script `drivers/scripts/inject-win7-wim.ps1`.

---

## Manual verification checklist (Aero)

**Storage (virtio-blk):**

- [ ] Windows 7 detects the virtio disk during setup (with driver loaded).
- [ ] Installation completes and boots from the virtio disk.
- [ ] Disk is usable (create/copy large files; no I/O errors).

**Networking (virtio-net):**

- [ ] Windows 7 detects virtio NIC and installs the driver.
- [ ] Windows obtains a DHCP lease using Aero’s network stack.
- [ ] Basic connectivity works (ICMP/HTTP via Aero proxy).

**Input (virtio-input, best-effort):**

- [ ] Device appears and installs.
- [ ] Mouse movement is smooth (no PS/2-rate limitations).

**Sound (virtio-snd, optional):**

- [ ] Device appears and installs.
- [ ] Audio output works (system sounds).
- [ ] A recording endpoint exists (Control Panel → Sound → Recording) and capture works (may be silent if no host input source is available).

## Virtio Drivers (Windows 7 guest)

### Scope

This document captures the shared plumbing required to build **virtio 1.0 PCI “modern”** drivers for a **Windows 7** guest (typically KMDF/WDF). Device-specific drivers (virtio-blk, virtio-net, virtio-input, etc.) should reuse a common transport + virtqueue layer rather than reimplementing the spec repeatedly.

The goal is to describe what needs to exist *before* writing any device-specific virtio driver.

For the definitive Aero interoperability contract (virtio device IDs, required features, queue sizes, and transport rules), treat:

- [`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) (`AERO-W7-VIRTIO`)
- virtio-input end-to-end test plan (device model + Win7 driver + web runtime): [`../areas/windows-drivers.md`](../areas/windows-drivers.md)

as authoritative. If this document ever disagrees with the contract, the contract wins.

---

### Virtio 1.0 PCI modern transport

#### Capability discovery (PCI vendor-specific capabilities)

Virtio PCI devices expose their modern interface via **PCI vendor-specific capabilities** (capability ID `0x09`). Each capability is a `virtio_pci_cap` (or extension of it) that points at a sub-region of a BAR:

- `bar` – which BAR contains the region
- `offset` / `length` – byte range inside the BAR
- `cfg_type` – which virtio structure the region contains

For modern virtio 1.0 drivers, the important `cfg_type` values are:

| cfg_type | Name                       | Purpose |
| ---: | -------------------------- | ------- |
| 1 | Common configuration         | Feature negotiation, status, queue programming |
| 2 | Notify configuration         | Queue “doorbell” writes (kicks) |
| 3 | ISR status                   | Interrupt cause + acknowledgement (read-to-clear) |
| 4 | Device-specific configuration| Per-device config space (e.g., MAC, capacity) |
| 5 | PCI configuration access     | Optional/rarely needed |

Notes for Windows driver authors:

- In `EvtDevicePrepareHardware`, enumerate the device’s PCI capabilities (via bus interface / config space reads) and record the capabilities you need.
- Map BAR memory from the translated resource list (`CmResourceTypeMemory` / `CmResourceTypeMemoryLarge`) and compute each capability’s effective virtual address as `bar_va + cap.offset`.
  - On some x64 systems, PCI MMIO ranges (especially BARs above 4 GiB) can be reported as `CmResourceTypeMemoryLarge`. The `Length40/48/64` fields are stored in scaled units and must be decoded back to bytes (see `../areas/windows-drivers.md` for details).
- Multiple virtio capabilities can live in the same BAR; only map each BAR once.

##### Portable capability-list parser (hardware-free regression tests)

This repo includes a small **portable C99** module that implements the capability-list walk + `virtio_pci_cap` parsing logic, along with synthetic config-space unit tests that run on Linux CI:

- Parser: `drivers/win7/virtio/virtio-core/portable/virtio_pci_cap_parser.{h,c}`
- Tests: `drivers/win7/virtio/tests/virtio_pci_cap_parser_test.c`

Run locally:

```bash
bash ./drivers/win7/virtio/tests/build_and_run.sh
```

#### Required MMIO regions

A minimal modern virtio driver should expect to map these regions:

1. **Common config** (`cfg_type = 1`)
   - Contains `device_status`, `device_feature[_select]`, `driver_feature[_select]`, `num_queues`, and the queue programming registers.
2. **Notify area** (`cfg_type = 2`)
   - A write-only MMIO region used to notify (“kick”) a queue.
   - Uses `notify_off_multiplier` and `queue_notify_off` to compute the address to write.
3. **ISR status** (`cfg_type = 3`)
   - A single byte; reading it returns interrupt cause bits and clears them.
4. **Device config** (`cfg_type = 4`)
   - Device-specific structure defined by the device type (blk/net/input/etc.).

Many virtio drivers also want MSI-X resources (standard PCI MSI-X capability), but that is *not* a virtio vendor capability.

---

### Driver init sequence (status bits + feature negotiation)

Virtio devices use the `device_status` field in the common config. The important status bits are:

| Bit | Name | Meaning |
| ---: | ---- | ------- |
| 0x01 | ACKNOWLEDGE | Driver found the device |
| 0x02 | DRIVER | Driver knows how to drive the device |
| 0x04 | DRIVER_OK | Driver is fully set up |
| 0x08 | FEATURES_OK | Feature negotiation completed |
| 0x40 | DEVICE_NEEDS_RESET | Device hit an error and wants reset |
| 0x80 | FAILED | Driver gave up / fatal error |

Implementation-oriented sequence (modern transport):

1. **RESET**
   - Write `0` to `device_status`.
2. **ACK → DRIVER**
   - Write `ACKNOWLEDGE`, then `ACKNOWLEDGE | DRIVER`.
3. **FEATURES**
   - Read device features via:
     - write `device_feature_select = 0`, read `device_feature`
     - write `device_feature_select = 1`, read `device_feature`
   - Compute supported driver feature mask.
   - Write driver features via `driver_feature_select` + `driver_feature`.
4. **FEATURES_OK**
   - Set `FEATURES_OK` in `device_status`.
   - Read `device_status` back; if `FEATURES_OK` is not still set, the device rejected features → set `FAILED`.
5. **QUEUES**
   - Discover queue count via `num_queues`.
   - For each required queue:
     - program addresses for descriptor/avail/used rings
     - set `queue_size`, `queue_enable`, `queue_msix_vector` (if using MSI-X)
6. **DRIVER_OK**
   - Set `DRIVER_OK` once queues, interrupts, and device config are ready.

If at any point `DEVICE_NEEDS_RESET` is observed, the safe recovery path is to reset the device (write 0 to `device_status`) and restart initialization.

#### Config generation (safe device-config reads)

Modern virtio provides `config_generation` to let the driver read device-specific config atomically:

1. Read `gen0 = config_generation`.
2. Read the device-specific config structure (possibly multiple MMIO reads).
3. Read `gen1 = config_generation`.
4. If `gen0 != gen1`, retry.

This matters most for larger config structures (e.g., net config) or when the device can update config at runtime.

---

### Virtqueue split ring (virtio 1.0)

Virtio 1.0 drivers commonly use **split virtqueues** (descriptor table + avail ring + used ring) in guest memory.

For the detailed split-ring virtqueue implementation algorithms (descriptor free list + cookies, ordering/barriers, EVENT_IDX, indirect descriptors, and end-to-end virtio-input-style usage), see:

* [`virtio/virtqueue-split-ring-win7.md`](windows7-virtio-driver-contract.md)

#### Memory layout

A split virtqueue consists of:

1. **Descriptor table** (`virtq_desc[qsz]`)
   - 16 bytes each: `{ addr: u64, len: u32, flags: u16, next: u16 }`
2. **Avail ring**
   - `{ flags: u16, idx: u16, ring: u16[qsz], (used_event: u16 if EVENT_IDX) }`
3. **Used ring**
   - `{ flags: u16, idx: u16, ring: used_elem[qsz], (avail_event: u16 if EVENT_IDX) }`
   - `used_elem` is 8 bytes: `{ id: u32, len: u32 }`

Alignment requirements (practical rules):

- Descriptor table: 16-byte aligned (natural alignment works if the base is aligned).
- Avail ring: 2-byte aligned.
- Used ring: 4-byte aligned.

When packing into one allocation, round up the start of the used ring to a 4-byte boundary.

#### Why WDF common buffers are used on Windows 7

Virtqueue rings must live in **DMA-visible, nonpaged memory**:

- The device performs DMA reads of descriptors/avail and DMA writes to the used ring.
- Physical addresses of the rings are programmed into the common config (`queue_desc`, `queue_avail`, `queue_used`).

On Windows 7 KMDF, a practical approach is:

1. Create a DMA enabler (`WdfDmaEnablerCreate`) with a profile compatible with the device (typically scatter/gather).
2. Allocate per-queue ring memory as a **common buffer** (`WdfCommonBufferCreate`).
   - Common buffers are contiguous, nonpaged, and provide:
     - a CPU virtual address (for the driver to fill rings)
     - a device physical/logical address (to program the queue registers)

For request data buffers (e.g., block I/O payloads), device-specific drivers typically use WDF DMA transactions and build descriptor chains that reference those DMA-mapped buffers. The ring structures themselves still live in a common buffer.

#### Notify (“kick”) path

Modern virtio uses the notify capability plus per-queue notify offsets:

1. Read `queue_notify_off` from common config for the selected queue.
2. Compute:
   - `notify_addr = notify_base + queue_notify_off * notify_off_multiplier`
3. Write the queue index (usually a 16-bit value) to `notify_addr`.

Drivers typically “kick” only after updating the avail ring and performing appropriate ordering (e.g., memory barriers) so the device never sees a partially written descriptor chain.

---

### Interrupt handling (MSI-X and legacy INTx)

Virtio devices can signal interrupts via:

- **Legacy INTx** (pin-based, level-triggered)
- **MSI-X** (message-signaled, multiple vectors)

#### ISR status capability (read-to-clear)

The ISR status capability (`cfg_type = 3`) is a single byte:

- Bit 0: “queue interrupt”
- Bit 1: “device config changed”

Reading this byte acknowledges/clears the pending interrupt cause in the device (read-to-clear).

For **INTx** (level-triggered), reading this register in the ISR is required to deassert the line. For **MSI-X**, drivers typically do not rely on `isr_status` for ACK/routing (the message vector already identifies the source), but the register may still be useful as a fallback/debug signal.

#### MSI-X on Windows 7 (message-signaled interrupts)

When a PCI device is configured for MSI-X, Windows exposes one or more **message interrupt resources**. In WDF, this typically means:

- In `EvtDevicePrepareHardware`, identify `CmResourceTypeInterrupt` descriptors where the translated flags indicate a message interrupt.
- Create one `WDFINTERRUPT` per message (or whatever mapping strategy the driver uses), then associate:
  - one message vector with the device configuration change interrupt
  - one message vector per virtqueue (common for net with separate RX/TX)

Virtio’s side of MSI-X routing is programmed through `virtio_pci_common_cfg`:

- `msix_config` selects which MSI-X vector the device uses for config-change interrupts.
- `queue_msix_vector` (for a selected queue via `queue_select`) selects which MSI-X vector the device uses for that queue.
- Writing `0xFFFF` disables **MSI-X routing** for that interrupt source. Per the Aero Win7 virtio
  contract (`AERO-W7-VIRTIO` v1), when MSI-X is enabled at the PCI layer, MSI-X is **exclusive**:
  `0xFFFF` means interrupts for that source are **suppressed** (no MSI-X message and no INTx
  fallback). When MSI-X is disabled, devices deliver interrupts via **INTx + ISR** semantics.

If MSI-X resources are not available, the driver should fall back to a single INTx interrupt and use the ISR status byte to demultiplex causes.

#### Interrupt service flow

A typical WDF flow is:

1. **ISR**: do the minimum:
   - read ISR status (for INTx, always; for MSI-X, often still safe)
   - mask/unmask as needed
   - queue a DPC
2. **DPC**:
   - drain used ring entries for affected virtqueues
   - complete pending requests / indicate packets / report input events

Device-specific drivers should avoid doing heavy work in the ISR.

### See also (in-repo bring-up guides)

- WDM bring-up (caps + BAR mapping + queues + INTx): [`../areas/windows-drivers.md`](../areas/windows-drivers.md)
- Miniport bring-up (NDIS/StorPort): [`../areas/windows-drivers.md`](../areas/windows-drivers.md)
- KMDF interrupts guide (MSI-X vs INTx): [`../areas/windows-drivers.md`](../areas/windows-drivers.md)

## Virtio PCI: Legacy (0.9) + Transitional Devices

### Goal

Windows 7-era virtio drivers (notably older **virtio-win** builds) often expect the **virtio 0.9 “legacy” PCI transport** (I/O port BAR registers) or a **transitional device** that exposes *both* the legacy interface and the virtio 1.0+ PCI capability-based interface.

This document describes an **optional compatibility mode** for targeting upstream virtio-win driver bundles. It is **not** the default Aero Windows 7 virtio transport: [`AERO-W7-VIRTIO` v1](windows7-virtio-driver-contract.md) is modern-only (PCI capabilities + BAR0 MMIO) and explicitly does not require legacy/transitional I/O port transport.

To maximize compatibility with upstream driver bundles (especially older Windows 7-era virtio-win builds), Aero’s virtio devices may support:

- **Legacy virtio PCI transport** (virtio 0.9 register layout via an I/O port BAR), and/or
- **Transitional virtio PCI devices** that expose **both**:
  - legacy I/O port BAR registers, and
  - modern virtio 1.0+ PCI capabilities (common cfg/notify/isr/device cfg).

This document specifies the register layout and the behavioral rules needed to implement legacy and transitional virtio PCI devices in a way that keeps feature negotiation, queue setup, and interrupts consistent across the modern and legacy paths.

---

### Terminology

- **Legacy**: virtio 0.9 PCI transport using an **I/O port BAR** and **PFN-based** queue setup.
- **Modern**: virtio 1.0+ PCI transport using **vendor-specific PCI capabilities** and MMIO config structures.
- **Transitional**: a single PCI function that exposes **both legacy + modern** transports. The guest chooses which one to use based on what it probes and negotiates.

---

### Web runtime selection (web runtime)

The Aero web runtime defaults to **modern-only** virtio devices (Aero contract v1). To enable compatibility modes for upstream virtio-win bundles, select the transport explicitly per device:

#### virtio-net

- **Settings UI:** "Virtio-net mode"
- **Config:** `virtioNetMode: "modern" | "transitional" | "legacy"`
- **URL query override:** `?virtioNetMode=modern|transitional|legacy`

#### virtio-input (keyboard/mouse)

- **Settings UI:** "Virtio-input mode"
- **Config:** `virtioInputMode: "modern" | "transitional" | "legacy"`
- **URL query override:** `?virtioInputMode=modern|transitional|legacy`

#### virtio-snd (audio)

- **Settings UI:** "Virtio-snd mode"
- **Config:** `virtioSndMode: "modern" | "transitional" | "legacy"`
- **URL query override:** `?virtioSndMode=modern|transitional|legacy`

Notes:

- Changing any of these `virtio*Mode` values changes the guest-visible PCI device ID / BAR layout and requires a VM restart to take effect.
- `"legacy"` disables modern virtio-pci capabilities and exposes only the legacy I/O port register block.

---

### PCI Identification (Transitional vs Modern-only)

Virtio PCI functions are typically identified by:

- **PCI Vendor ID**: `0x1AF4` (virtio / Red Hat)
- **PCI Device ID**:
  - **Transitional IDs** (legacy-compatible): `0x1000..0x103F`
  - **Modern-only IDs**: `0x1040..0x107F`

> Exact device IDs depend on the virtio device type mapping used by the implementation. A common convention is:
> - transitional: `0x1000 + (device_type - 1)`
> - modern: `0x1040 + device_type`

For compatibility with **upstream virtio-win** driver packages (especially older Win7-era builds), presenting **transitional**
IDs is often the most compatible choice (because many older drivers probe/bind via the legacy I/O-port transport and the `0x1000..` transitional PCI ID range).

However, Aero’s own Windows 7 virtio contract v1 uses the **modern** virtio-pci ID space by default
(see `windows7-virtio-driver-contract.md` and `windows-device-contract.md`). Aero may still
expose transitional IDs/legacy I/O BARs as an optional compatibility mode, but drivers must not rely
on it unless explicitly stated by the contract.

---

### Legacy virtio PCI register block (virtio 0.9)

#### Placement

The legacy transport is presented via a **PCI I/O BAR**. The BAR maps a register block whose first 20 bytes are transport-defined registers; the remainder is **device-specific config**.

#### Register layout

All registers are **little-endian**.

| Offset | Size | Name | Access | Description |
|--------|------|------|--------|-------------|
| `0x00` | 32 | `HOST_FEATURES` | R | Device feature bits (legacy exposes **only bits 0..31**) |
| `0x04` | 32 | `GUEST_FEATURES` | W | Guest-selected features (legacy **only bits 0..31**) |
| `0x08` | 32 | `QUEUE_PFN` | R/W | PFN of the virtqueue for selected queue (`PFN * 4096 = guest phys addr`) |
| `0x0C` | 16 | `QUEUE_NUM` | R | Queue size (max entries) for selected queue. `0` means queue not available. |
| `0x0E` | 16 | `QUEUE_SEL` | W | Select virtqueue index |
| `0x10` | 16 | `QUEUE_NOTIFY` | W | Notify queue index (kicks the device) |
| `0x12` | 8  | `STATUS` | R/W | Device status state machine |
| `0x13` | 8  | `ISR_STATUS` | R (clear) | Interrupt status. Read returns bits and **clears** them. |
| `0x14` | …  | `DEVICE_CONFIG` | R/W | Device-specific config region |

##### Notes on access widths

Guests may perform 8/16/32-bit I/O to these offsets depending on driver and CPU type. The emulation should:

- accept naturally-aligned accesses, and
- be tolerant of sub-word accesses (e.g. 8-bit reads of `STATUS`).

---

### Legacy feature negotiation

Legacy exposes only **32 bits** of features via `HOST_FEATURES`/`GUEST_FEATURES`.

Implications for transitional devices:

- Modern-only feature bits (e.g. `VIRTIO_F_VERSION_1`, typically bit 32) are **not visible** to legacy drivers.
- Therefore, when the guest uses the legacy path, the device must behave as a **legacy virtio device** (0.9 semantics) and must not require `VIRTIO_F_VERSION_1`.

Recommended internal representation:

```rust
/// Device-supported features (full 64-bit set, even if legacy only exposes low 32).
device_features: u64,

/// Guest-accepted features for the *active* transport.
/// - Legacy path: upper 32 bits are forced to 0.
/// - Modern path: full 64-bit negotiation is allowed.
driver_features: u64,
```

---

### Legacy queue programming (PFN)

Legacy queue setup uses `QUEUE_PFN` rather than explicit descriptor/avail/used addresses.

Typical driver flow:

1. Write `QUEUE_SEL = qidx`
2. Read `QUEUE_NUM` (queue size); if `0`, queue doesn’t exist
3. Allocate a virtqueue ring in guest memory aligned to **4096**
4. Write `QUEUE_PFN = ring_guest_phys_addr / 4096`
5. Start operation; notify via `QUEUE_NOTIFY = qidx`

#### Deriving ring addresses from PFN

On the device side, `QUEUE_PFN` resolves to a single base address:

```text
ring_base = (QUEUE_PFN as u64) << 12
```

From `ring_base` and `QUEUE_NUM`, compute descriptor/avail/used addresses using the standard **vring** layout with **alignment = 4096** (often referred to as `VIRTIO_PCI_VRING_ALIGN` in OS code).

See also:

- [`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) — split-ring virtqueue layout/alignment rules and the driver-side algorithms for publishing/consuming entries (Win7 KMDF focus).

This allows the core virtqueue implementation to operate on the same internal representation regardless of transport:

```rust
struct QueueState {
    /// Max size exposed by device (`QUEUE_NUM` for legacy, `queue_size_max` for modern).
    max_size: u16,

    /// Size actually selected/enabled (legacy: always `max_size` once PFN != 0).
    size: u16,

    desc_addr: u64,
    avail_addr: u64,
    used_addr: u64,

    enabled: bool,
}
```

---

### Legacy device status state machine

The legacy `STATUS` register is shared conceptually with virtio 1.0+ status bits:

| Bit | Name | Meaning |
|-----|------|---------|
| `0x01` | `ACKNOWLEDGE` | Guest found the device |
| `0x02` | `DRIVER` | Guest knows how to drive the device |
| `0x04` | `DRIVER_OK` | Driver is fully set up; device may start processing queues |
| `0x08` | `FEATURES_OK` | (Virtio 1.0+) Feature negotiation complete |
| `0x40` | `DEVICE_NEEDS_RESET` | Device experienced an error and requires reset |
| `0x80` | `FAILED` | Guest gave up; device should stop |

Compatibility notes:

- Older legacy drivers may **not** use `FEATURES_OK`. Treat it as optional on the legacy path; don’t deadlock waiting for it.
- Writing `0` to `STATUS` is a full device reset: clear negotiated features, queue state, and pending interrupts.
- Aside from full reset (`0`), drivers typically only **set** bits. If a guest clears bits without resetting, the simplest compatible behavior is to ignore the clear.

---

### Legacy ISR status register (read clears)

`ISR_STATUS` is a one-byte register with “read-to-clear” semantics:

| Bit | Meaning |
|-----|---------|
| `0x01` | Queue interrupt (used ring update) |
| `0x02` | Device config changed |

Rules:

- Device sets bits when it needs to notify the guest.
- A guest read returns the current bitmask and then clears it.
- For **INTx**, IRQ deassertion should be tied to `ISR_STATUS` becoming 0 (and no other pending condition).

---

### Modern virtio PCI capabilities (virtio 1.0+ overview)

Modern virtio PCI uses vendor-specific PCI capabilities that point to MMIO structures:

- **Common configuration** (features, queue config, status)
- **ISR configuration** (same “read clears” semantics as legacy ISR)
- **Notify configuration** (per-queue notify address)
- **Device-specific configuration** (same content as legacy `DEVICE_CONFIG`)

Modern feature negotiation uses a **64-bit** feature set accessed via a select/data pair:

```text
device_feature_select (u32)  // 0 or 1
device_feature (u32)         // returns (device_features >> (select*32)) & 0xffff_ffff

driver_feature_select (u32)  // 0 or 1
driver_feature (u32)         // guest writes selected 32-bit word
```

For transitional devices:

- A modern driver must negotiate `VIRTIO_F_VERSION_1` (bit 32) and set `FEATURES_OK`.
- Legacy drivers cannot see bit 32 and will never set it; they should remain on legacy semantics.

---

### Transitional device behavior (legacy + modern at once)

#### High-level rule

A transitional virtio PCI function exposes both transports simultaneously. The guest can bind via:

- legacy I/O port registers, or
- modern capabilities.

The device must work correctly in either case.

#### Recommended “transport mode” gating

To avoid contradictory configuration (e.g., guest sets modern queue addresses while also programming legacy PFNs), treat the device as operating in one of these modes per reset cycle:

```rust
enum TransportMode {
    Unknown,
    Legacy,
    Modern,
}
```

Recommended selection rule:

- After a reset (`STATUS=0`), mode = `Unknown`.
- The first *write* that meaningfully configures the transport locks the mode:
  - writes to legacy `GUEST_FEATURES`, `QUEUE_PFN`, or legacy `STATUS` progression → `Legacy`
  - writes to modern common-cfg feature fields or modern queue address fields → `Modern`
- Once locked, ignore or reject configuration writes coming from the other transport until next reset.

This matches how real guests behave (they pick one driver stack).

#### Forcing legacy for testing

Provide a “disable modern” toggle (analogous to QEMU’s `disable-modern=on`) that:

- omits modern virtio PCI capabilities, and/or
- uses transitional/legacy device IDs only,

so that modern OSes are forced to bind via the legacy path. This is extremely useful for validating Windows 7 compatibility.

---

### Interrupts

#### Minimum: legacy INTx

To support legacy drivers, always support **PCI INTx** interrupts:

- Set `ISR_STATUS` bits on used-ring updates and config changes.
- Assert the PCI interrupt line while `ISR_STATUS != 0`.
- Deassert after the guest reads and clears `ISR_STATUS` (and no other conditions remain).

#### Recommended: MSI-X for modern virtio-net performance

MSI-X is optional for correctness but strongly recommended for performance, especially for virtio-net with multiple queues.

If MSI-X is implemented:

- allow a per-queue MSI-X vector (modern common config `queue_msix_vector`)
- allow a config-change MSI-X vector (modern common config `msix_config`)
- legacy drivers will ignore MSI-X and continue using INTx/ISR.

---

### Suggested implementation layout (VIO-CORE)

Suggested module layout:

```
src/io/virtio/
  pci_legacy.rs         // I/O port BAR, PFN queues, legacy ISR/status
  pci_modern.rs         // virtio 1.0+ PCI capabilities + common/notify/isr cfg
  pci_transitional.rs   // wraps both; mode gating; shared device core
  core.rs               // common virtio device/queue logic
```

Recommended abstraction boundary:

- `core.rs` owns:
  - negotiated features
  - queue state (desc/avail/used addresses, size, enable)
  - device-specific config blob
  - notification and interrupt generation hooks
- each PCI transport (`pci_legacy`, `pci_modern`) is a “front-end” that:
  - decodes guest register accesses, and
  - calls into the shared core to apply configuration / kick queues.

---

### Verification plan

#### Unit tests: legacy guest driver flow (transport-level)

Emulate a “pure legacy” driver interaction against the I/O-port BAR:

1. Read `HOST_FEATURES`
2. Write `GUEST_FEATURES`
3. Set `STATUS = ACKNOWLEDGE | DRIVER`
4. Configure queue 0:
   - `QUEUE_SEL = 0`
   - read `QUEUE_NUM`
   - write `QUEUE_PFN`
5. Set `STATUS = ACKNOWLEDGE | DRIVER | DRIVER_OK`
6. Place one descriptor chain in the ring and `QUEUE_NOTIFY = 0`
7. Device consumes descriptors, writes one used element, raises INTx, sets `ISR_STATUS |= 0x1`
8. Guest reads `ISR_STATUS` and verifies it clears

Pseudo-test (shape only):

```rust
#[test]
fn virtio_pci_legacy_queue_pfn_and_isr() {
    let mut dev = TestVirtioPciTransitional::new()
        .with_modern_disabled(true);

    let io = dev.legacy_io_base();

    // 1-2: feature negotiation
    let host_features = dev.io_read32(io + 0x00);
    dev.io_write32(io + 0x04, host_features & 0xffff_ffff);

    // 3: status progression
    dev.io_write8(io + 0x12, 0x01 | 0x02); // ACKNOWLEDGE | DRIVER

    // 4: queue setup via PFN
    dev.io_write16(io + 0x0E, 0); // QUEUE_SEL
    let qsz = dev.io_read16(io + 0x0C);
    assert!(qsz > 0);

    let ring_addr = dev.alloc_vring_legacy(qsz, /*align=*/4096);
    dev.io_write32(io + 0x08, (ring_addr >> 12) as u32); // QUEUE_PFN

    // 5: driver ok
    dev.io_write8(io + 0x12, 0x01 | 0x02 | 0x04);

    // 6: submit one request and notify
    dev.place_one_descriptor_chain_legacy(ring_addr, qsz);
    dev.io_write16(io + 0x10, 0); // QUEUE_NOTIFY

    // 7: device raises interrupt
    assert!(dev.intx_asserted());

    // 8: ISR read clears
    let isr = dev.io_read8(io + 0x13);
    assert_eq!(isr & 0x01, 0x01);
    assert!(!dev.intx_asserted());
}
```

#### Smoke tests (in-guest)

If/when a bootable guest harness exists:

- **Linux legacy bind test**:
  - present transitional virtio-blk and virtio-net
  - boot a kernel/config that will bind via legacy (or force legacy by disabling modern caps)
  - verify one block read/write and one net TX
- **Windows 7 virtio-win bind test**:
  - attach virtio-win ISO
  - verify that virtio-net/virtio-blk devices are detected and the driver binds successfully

## Virtio Input (virtio 1.1): Keyboard + Mouse (+ Tablet)

### Why virtio-input

PS/2 is simple but has limited throughput and higher per-event overhead. Once the guest has a virtio driver installed, **virtio-input** provides a fast paravirtual path for keyboard/mouse events with low latency and fewer emulated side effects.

See also:

- [`virtio/virtqueue-split-ring-win7.md`](windows7-virtio-driver-contract.md) — split-ring virtqueue implementation guide for Windows 7 KMDF drivers (descriptor mgmt, ordering/barriers, EVENT_IDX, indirect).
- [`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) — Aero’s definitive virtio device/feature/transport contract.
- [`virtio-input-test-plan.md`](../areas/windows-drivers.md) — end-to-end validation plan (Rust device-model tests, Win7 driver tests, web runtime routing).

This repo implements virtio-input as a **single multi-function PCI device** (AERO-W7-VIRTIO contract v1) with two required functions and an optional third:

- Function 0: `Aero Virtio Keyboard` (`SUBSYS 0x0010`, `header_type = 0x80`)
- Function 1: `Aero Virtio Mouse` (relative pointer, `SUBSYS 0x0011`)
- (Optional) Function 2: `Aero Virtio Tablet` (absolute pointer / `EV_ABS`, `SUBSYS 0x0012`)

Each function is a standard virtio 1.1 device (`VIRTIO_ID_INPUT`) with its own virtqueues.

When installed with the in-tree Windows 7 driver(s), these PCI functions appear as **separate named devices** in Windows Device Manager (HIDClass):

- **Aero VirtIO Keyboard**
- **Aero VirtIO Mouse**
- (Optional) **Aero VirtIO Tablet** (requires `drivers/windows7/virtio-input/inf/aero_virtio_tablet.inf`)

---

### Device model overview

Virtio-input uses two virtqueues:

| Queue | Direction | Purpose |
|------:|-----------|---------|
| eventq | device → driver | Input events (`virtio_input_event`) |
| statusq | driver → device | Output events (e.g. LED state) |

Events use the Linux input ABI layout:

```text
struct virtio_input_event {
  le16 type;   // EV_KEY / EV_REL / EV_SYN / ...
  le16 code;   // KEY_* / BTN_* / REL_* / SYN_REPORT / ...
  le32 value;  // 1/0 for keys, deltas for REL_*, 0 for SYN_REPORT
};
```

The implementation emits a `EV_SYN / SYN_REPORT` event after each logical batch, matching the conventional input event stream format.

---

### Code map (where things live)

Virtio-input spans both the Rust device model and the browser runtime wiring. In the browser runtime, the integration path depends on
`vmRuntime`:

- **Virtio-input device model (Rust):** `crates/aero-virtio/src/devices/input.rs`
- **Browser runtime wiring (`vmRuntime="legacy"`):**
  - TypeScript PCI wrapper + event injection: `apps/web/src/io/devices/virtio_input.ts`
  - PCI registration helper (canonical BDF for the keyboard function): `apps/web/src/workers/io_virtio_input_register.ts`
  - IO worker integration + routing decisions: `apps/web/src/workers/io.worker.ts`
- **Canonical full-system machine wiring (used by `vmRuntime="machine"`):**
  - The canonical `aero_machine::Machine` supports virtio-input behind an explicit config flag:
    - `MachineConfig.enable_virtio_input = true` (requires `MachineConfig.enable_pc_platform = true`)
    - Fixed PCI BDFs:
      - `00:0A.0` — virtio-input keyboard (`aero_devices::pci::profile::VIRTIO_INPUT_KEYBOARD`)
      - `00:0A.1` — virtio-input mouse (`aero_devices::pci::profile::VIRTIO_INPUT_MOUSE`)
      - (Optional) `00:0A.2` — virtio-input tablet (`aero_devices::pci::profile::VIRTIO_INPUT_TABLET`, when attached)
    - Interrupts: both **INTx** (baseline) and **MSI-X** are supported.
      - MSI(-X) delivery is wired through `VirtioMsixInterruptSink` → `PlatformInterrupts::trigger_msi`
        (see `crates/aero-machine/src/lib.rs::VirtioMsixInterruptSink`).
      - Legacy INTx delivery is polled/routed via the machine’s PCI INTx sync loop (not via the virtio
        interrupt sink), so it remains deterministic across devices.
      - Regression test: `crates/aero-machine/tests/virtio_input_msix.rs`.
  - In the JS/WASM “single machine” API (`crates/aero-wasm::Machine`), virtio-input is opt-in at construction time:
    - `Machine.new_with_options(..., { enable_virtio_input: true })`
    - Injection APIs: `inject_virtio_key/rel/button/wheel` (Linux `evdev`-style codes)
    - Driver status probes: `virtio_input_keyboard_driver_ok()` / `virtio_input_mouse_driver_ok()`
  - Browser runtime integration lives in the machine CPU worker: `apps/web/src/workers/machine_cpu.worker.ts`.

---

### Config space queries (required by the virtio-input driver)

Virtio-input uses a small device-specific config region where the driver:

1. Writes `{select, subsel}`
2. Reads `size`
3. Reads `u.*` payload bytes

The implementation supports at least:

- `VIRTIO_INPUT_CFG_ID_NAME` (device name string)
- `VIRTIO_INPUT_CFG_ID_SERIAL` (string, currently `"0"`)
- `VIRTIO_INPUT_CFG_ID_DEVIDS` (`bustype/vendor/product/version`)
- `VIRTIO_INPUT_CFG_EV_BITS`:
  - `subsel = 0` → event type bitmap (varies by device kind; includes e.g. `EV_SYN`, `EV_KEY`, `EV_REL`, `EV_ABS`, `EV_LED`)
  - `subsel = EV_KEY` → supported key/button bitmap
  - `subsel = EV_REL` → supported rel bitmap (`REL_X`, `REL_Y`, `REL_WHEEL`, `REL_HWHEEL`)
  - `subsel = EV_ABS` → supported abs bitmap (`ABS_X`, `ABS_Y`, tablet only)
  - `subsel = EV_LED` → supported LED bitmap (`LED_*`, keyboard only)
- `VIRTIO_INPUT_CFG_ABS_INFO` (tablet only): axis range metadata for `ABS_X`/`ABS_Y` (used for scaling)

---

### Host/browser input integration

The capture layer (IN-CAPTURE) should be able to inject the same high-level input events into either:

- PS/2 (boot + early install)
- virtio-input (once guest driver is active)

Runtime routing is typically:

- **Auto mode**: PS/2 until the guest sets `DRIVER_OK` for the virtio-input device, then switch to virtio-input.
- Optional developer modes: PS/2 only, virtio only.

---

### Windows 7 driver (minimal test-signed approach)

Windows 7 has no in-box virtio-input driver. A minimal approach is to ship a custom, test-signed driver that:

1. Binds to the virtio-input PCI function (Aero Win7 contract v1 uses Vendor/Device `PCI\VEN_1AF4&DEV_1052` with PCI Revision ID `0x01` / `REV_01`).
2. Negotiates virtio features and sets `DRIVER_OK`.
3. Creates a HID keyboard + HID mouse interface for Windows by translating `virtio_input_event` streams into HID reports.
4. Optionally forwards LED state changes (Caps Lock / Num Lock / Scroll Lock) from Windows to `statusq`.
   - The contract requires that **all** `statusq` descriptors are consumed/completed (contents may be ignored).
   - The in-tree Win7 guest selftest can validate this end-to-end via `--test-input-led` (marker: `virtio-input-led`), and the
     host harness can enforce it with PowerShell `-WithInputLed` / Python `--with-input-led`.

Contract note:

- `AERO-W7-VIRTIO` v1 encodes the contract major version in the PCI Revision ID (`REV_01`).
- The in-tree Win7 virtio-input INFs are intentionally **revision-gated** (match only `...&REV_01` HWIDs), so QEMU-style
  `REV_00` virtio-input devices will not bind unless you override the revision (for example `x-pci-revision=0x01`).
  - The canonical keyboard/mouse INF (`drivers/windows7/virtio-input/inf/aero_virtio_input.inf`) includes:
    - subsystem-qualified keyboard/mouse HWIDs (`SUBSYS_0010` / `SUBSYS_0011`, both `&REV_01`) for distinct Device Manager names
      (**Aero VirtIO Keyboard** / **Aero VirtIO Mouse**), **and**
    - a strict revision-gated generic fallback HWID (no `SUBSYS`): `PCI\VEN_1AF4&DEV_1052&REV_01`
      (Device Manager name: **Aero VirtIO Input Device**) for environments where subsystem IDs are not exposed/recognized.
  - The repo also carries an optional legacy filename alias INF
    (`drivers/windows7/virtio-input/inf/virtio-input.inf.disabled`; rename to `virtio-input.inf` to enable) for basename
    compatibility with workflows/tools that still reference `virtio-input.inf`.
    - Policy: it is a **filename alias only**. From the first section header (`[Version]`) onward it must remain byte-for-byte
      identical to `aero_virtio_input.inf` (only the leading banner/comments may differ).
    - Enabling the alias does **not** change HWID matching behavior (it contains the same HWID set, including the fallback).
    - Guardrails:
      - `drivers/windows7/virtio-input/scripts/check-inf-alias.py`
      - `scripts/ci/check-windows7-virtio-contract-consistency.py`
  - Tablet devices bind via the separate tablet INF (`drivers/windows7/virtio-input/inf/aero_virtio_tablet.inf`,
    `SUBSYS_00121AF4`). That HWID is more specific, so it wins over the generic fallback when both driver packages are installed
    and the tablet subsystem ID is present. If the tablet INF is not installed (or the device does not expose the tablet
    subsystem ID), the generic fallback entry in `aero_virtio_input.inf` can also bind to tablet devices (but will use the
    generic device name).
  - Do not ship/install both `aero_virtio_input.inf` and the legacy alias `virtio-input.inf` at the same time (duplicate,
    overlapping INFs can lead to confusing PnP driver selection). Ship/install **only one** of the two basenames at a time.
- The driver also validates the Revision ID at runtime.

#### Installation flow (test signing)

1. **Enable test signing** (guest):
   - Run: `bcdedit /set testsigning on`
   - Reboot the VM
2. **Install the test certificate** used to sign the driver:
    - Import into **Trusted Root Certification Authorities**
    - Import into **Trusted Publishers**
    - For the in-tree Win7 driver, see `drivers/windows7/virtio-input/README.md` for helper scripts (`make-cert.ps1` / `install-test-cert.ps1`) and the full signing workflow.
3. **Install the driver**:
    - Device Manager → the virtio-input PCI device (often appears as “Unknown device”)
    - “Update driver” → “Have Disk…” → point at the driver `.inf`
4. **Verify**:
   - A new HID keyboard and HID mouse appear
   - The emulator can detect `DRIVER_OK` and switch input routing to virtio-input

#### In-tree driver source (this repo)

The canonical Windows 7 virtio-input driver source lives at:

- `drivers/windows7/virtio-input/` (INF: `inf/aero_virtio_input.inf`, service: `aero_virtio_input`)

The repo also carries an optional legacy filename alias INF (`inf/virtio-input.inf.disabled`; rename to `virtio-input.inf` to enable)
for compatibility with older workflows/tools that still reference `virtio-input.inf`. It is a filename-only alias and does
not change the set of matched hardware IDs.

Policy:

- `inf/aero_virtio_input.inf` includes both the subsystem-qualified keyboard/mouse model lines and the strict revision-gated
  generic fallback HWID (no `SUBSYS`): `PCI\VEN_1AF4&DEV_1052&REV_01`.
- The legacy alias INF is a **filename alias only**. From the first section header (`[Version]`) onward, it must remain
  byte-for-byte identical to `inf/aero_virtio_input.inf` (only the leading banner/comments may differ; see
  `drivers/windows7/virtio-input/scripts/check-inf-alias.py`).
- Enabling the alias does **not** change HWID matching behavior (it matches the same HWIDs as the canonical INF).
- Do not ship/install both basenames at the same time: ship/install **only one** of the two INF filenames to avoid duplicate,
  overlapping bindings.

#### Notes

- If you want the absolute smallest driver surface area for Windows 7, a KMDF driver that exposes a HID interface is typically the pragmatic choice.
- The status queue is optional for basic input, but supporting LED updates is useful for parity with PS/2 keyboard behavior.

## Virtio-snd (Paravirtual Audio) Device

This repository includes a minimal **virtio-snd** device model (`crates/aero-virtio`, `aero_virtio::devices::snd`) intended to be used as a high-performance alternative to full Intel HDA emulation once guest drivers exist.

> Runtime note: the browser integration details in this document describe the legacy worker device stack
> (`vmRuntime=legacy`), where guest devices are hosted in the I/O worker. In the canonical machine runtime
> (`vmRuntime=machine`), the I/O worker runs in a host-only stub mode and guest audio devices are not yet
> exposed via the browser worker runtime.

Note: the **browser worker runtime** (IO worker) wires up **HDA** as the default *active* guest audio device, but **virtio-snd is
also wired into the IO-worker PCI stack**:

- WASM bridge export: `crates/aero-wasm/src/virtio_snd_pci_bridge.rs` (`VirtioSndPciBridge`)
- TS PCI device wrapper: `apps/web/src/io/devices/virtio_snd.ts` (`VirtioSndPciDevice`)
- IO worker init/wiring: `apps/web/src/workers/io_virtio_snd_init.ts` + `apps/web/src/workers/io.worker.ts`

See also:

- [`virtio/virtqueue-split-ring-win7.md`](windows7-virtio-driver-contract.md) — split-ring virtqueue implementation guide for Windows 7 KMDF drivers (descriptor mgmt, ordering/barriers, EVENT_IDX, indirect).
- [`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) — Aero’s definitive virtio device/feature/transport contract.
- [`windows-device-contract.md`](windows-device-contract.md) — PCI ID + driver service naming contract (Aero).
- [`drivers/protocol/virtio/`](../../drivers/protocol/virtio/) — canonical `#[repr(C)]` message layouts (with Rust unit tests) shared between the guest driver and device model.

### Browser runtime status

Virtio-snd is instantiated in the browser by the **IO worker**:

1. `apps/web/src/workers/io.worker.ts` calls `tryInitVirtioSndDevice` (`apps/web/src/workers/io_virtio_snd_init.ts`) when the WASM export
   `VirtioSndPciBridge` is available.
2. `tryInitVirtioSndDevice` registers `VirtioSndPciDevice` (`apps/web/src/io/devices/virtio_snd.ts`) on the IO worker PCI bus.

#### Making virtio-snd the active (ring-attached) audio device

The host AudioWorklet rings are **SPSC**, so the IO worker attaches them to only one guest audio device at a time:

- Prefer **HDA** when `HdaControllerBridge` is available.
- Fall back to **virtio-snd** only when HDA is unavailable (e.g. a WASM build that omits HDA exports).

If both devices are registered, virtio-snd may still enumerate to the guest, but it will not produce/consume host audio unless an
explicit device-selection mechanism is added (detach HDA rings and attach virtio-snd rings).

Limitations (current):

- VM snapshot/restore is supported in the browser runtime under kind `"audio.virtio_snd"` (`DeviceId::VIRTIO_SND = 22`).
- Snapshots preserve guest-visible virtio-pci + stream state and AudioWorklet ring indices, but do not serialize host audio
  contents (rings are cleared to silence on restore).

Scope:

- **2 PCM streams**
  - Stream `0`: playback/output, **stereo (2ch)**, 48kHz, signed 16-bit little-endian (S16_LE)
  - Stream `1`: capture/input, **mono (1ch)**, 48kHz, signed 16-bit little-endian (S16_LE)
- **Split virtqueues** (virtio 1.2 subset)
- **Control queue** for stream discovery/configuration
- **TX queue** for PCM frame submission (playback)
- **RX queue** for PCM capture (recording)

### PCI identification (Aero)

Aero exposes virtio-snd as a virtio-pci device using the standard virtio vendor ID:

- Vendor ID: `0x1AF4`
- Device ID (canonical / default): `0x1059` (modern ID space: `0x1040 + VIRTIO_ID_SND (25)`)
- Subsystem Vendor ID: `0x1AF4`
- Subsystem Device ID: `0x0019` (`VIRTIO_ID_SND`)
- Revision ID: `0x01` (Aero Windows 7 virtio contract v1; see `windows7-virtio-driver-contract.md`)

Windows driver binding (Aero):

- The shipped Aero Win7 virtio-snd INF (`drivers/windows7/virtio-snd/inf/aero_virtio_snd.inf`) is intentionally strict and matches only:
  - `PCI\VEN_1AF4&DEV_1059&REV_01`
  - (Optional) `PCI\VEN_1AF4&DEV_1059&SUBSYS_00191AF4&REV_01` (commented out in the INF by default)

#### Contract v1 summary (AERO-W7-VIRTIO: virtio-snd)

Treat [`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) as authoritative.
This section is a convenience summary that MUST remain consistent with that contract.
The clean-room Win7 INF (`aero_virtio_snd.inf`) matches only `DEV_1059` (typically revision-gated as `&REV_01`), so if
Windows shows `DEV_1018` you must configure the hypervisor to expose a modern-only device (for example QEMU
`disable-legacy=on`).

- **Transport:** virtio-pci **modern-only** (PCI vendor-specific capabilities + **BAR0 MMIO**). No legacy I/O-port BARs.
- **Contract major version:** encoded in PCI Revision ID (`REV_01`).
- **Feature bits:** `VIRTIO_F_VERSION_1` + `VIRTIO_F_RING_INDIRECT_DESC` only.
- **Virtqueue sizes:** `controlq=64`, `eventq=64`, `txq=256`, `rxq=64`.
- **Streams (fixed-format):**
  - Stream `0`: render/playback, stereo (2ch), 48kHz, S16_LE
  - Stream `1`: capture/input, mono (1ch), 48kHz, S16_LE
- *(Optional/non-contract, Win7 driver behavior)*: if a virtio-snd implementation advertises additional formats/rates/channel
  counts in `PCM_INFO`, the in-tree Windows 7 driver can optionally expose additional formats to the Windows audio stack
  via dynamic WaveRT pin data ranges (while still requiring and preferring the contract-v1 baseline).

#### Event queue (eventq)

The virtio-snd specification defines `eventq` (queue index `1`) for asynchronous device → driver notifications.

Contract v1 does **not** define any required event messages (see `windows7-virtio-driver-contract.md` §3.4.2.1).

Driver behavior (Windows 7 in-tree `virtio-snd` driver):

- The driver posts a small bounded set of writable event buffers and keeps `eventq` running.
- The driver parses events best-effort and always reposts buffers.
- Known events include:
  - `JACK_*` (`JACK_CONNECTED` / `JACK_DISCONNECTED`): update fixed jack state used by the topology miniport.
  - `PCM_*` (`PCM_PERIOD_ELAPSED` / `PCM_XRUN`): optional audio-stream notifications.
    - Because contract v1 does not require `eventq`, the WaveRT software-DMA path remains **timer-driven** by default.
    - If `PCM_PERIOD_ELAPSED` events arrive, the driver may treat them as an **additional wakeup source** for the WaveRT period/DPC loop (best-effort), while coalescing duplicates to avoid double-signaling notifications.
    - If `PCM_XRUN` events arrive, the driver may attempt a best-effort stream recovery (typically `STOP`/`START`).

By default, Aero’s virtio-snd device model does not emit any `eventq` messages unless the host explicitly queues them (for example
via `VirtioSnd::queue_event(...)` / `VirtioSnd::queue_jack_event(...)`). The browser/WASM runtime uses this to emit jack
connect/disconnect events when host audio backends are attached/detached:

- Speaker jack (`jack_id = 0`): AudioWorklet output ring attach/detach (`VirtioSndPciBridge::set_audio_ring_buffer`)
- Microphone jack (`jack_id = 1`): mic capture ring attach/detach (`VirtioSndPciBridge::set_mic_ring_buffer`)

Implementation notes (device model):

- The device model keeps a **bounded** FIFO of pending event messages (currently capped at 256) to avoid unbounded host memory
  growth if a guest never posts/consumes event buffers.
- `queue_jack_event(...)` deduplicates redundant JACK state transitions within the pending FIFO (it will not enqueue repeated
  identical connected/disconnected events for the same jack ID).

Even when Aero emits no events (common in current browser builds):

- If the device completes an event buffer anyway (future extensions, or a buggy device model), the Windows 7 driver parses the standard
  8-byte virtio-snd event header (`type: u32` + `data: u32`, little-endian), dispatches known events best-effort, and ignores
  unknown/malformed events without crashing (buffers are always reposted).

The authoritative Windows driver-binding values are tracked in [`windows-device-contract.md`](windows-device-contract.md)
and [`protocol-vectors/windows-device-contract.json`](../../protocol-vectors/windows-device-contract.json).

If the device does not report the contract-v1 HWID `PCI\VEN_1AF4&DEV_1059&REV_01`, the shipped INF will not bind. For QEMU, pass:

```text
-device virtio-sound-pci,disable-legacy=on,x-pci-revision=0x01
```

For QEMU bring-up/regression (where the virtio-snd device may enumerate as transitional by default), the repo also
contains an opt-in compatibility driver package:

- `drivers/windows7/virtio-snd/inf/aero-virtio-snd-legacy.inf` (binds the transitional virtio-snd PCI ID `PCI\VEN_1AF4&DEV_1018`; installs service `aeroviosnd_legacy`)
- `virtiosnd_legacy.sys` (build with MSBuild `Configuration=Legacy`)

For older bring-up scenarios where a legacy **I/O-port** virtio-pci transport is required (not part of the Aero virtio
contract and not packaged by default), the repo also contains an opt-in I/O-port package:

- `drivers/windows7/virtio-snd/inf/aero-virtio-snd-ioport.inf` (binds `PCI\VEN_1AF4&DEV_1018&REV_00`; installs service `aeroviosnd_ioport`)
- `virtiosnd_ioport.sys` (build with MSBuild `virtio-snd-ioport-legacy.vcxproj`)

### Device Configuration

The device reports two PCM streams:

- `streams = 2`
- `jacks = 0` (**preferred**) or `jacks = 2` (**tolerated**, matches the driver’s fixed two-jack topology and enables optional JACK eventq notifications)
- `chmaps = 0`

### Supported Control Commands

The control virtqueue accepts requests prefixed by a 32-bit little-endian `code`. The response always starts with a 32-bit little-endian **status**.

The canonical packed layouts for all virtio-snd structs referenced below are defined in `drivers/protocol/virtio` (e.g. `VirtioSndPcmInfo`, `VirtioSndPcmSetParamsReq`, `VirtioSndPcmXferHdr`).

#### Status codes

- `VIRTIO_SND_S_OK = 0`
- `VIRTIO_SND_S_BAD_MSG = 1` (malformed request / invalid stream id)
- `VIRTIO_SND_S_NOT_SUPP = 2` (well-formed but unsupported operation/format)
- `VIRTIO_SND_S_IO_ERR = 3` (invalid state for the requested operation)

#### Implemented request codes

- `VIRTIO_SND_R_PCM_INFO (0x0100)`
  - Returns `virtio_snd_pcm_info` entries for streams `0` and/or `1` depending on the requested range.
- `VIRTIO_SND_R_PCM_SET_PARAMS (0x0101)`
  - Validates and stores parameters for stream id `0` or `1`.
  - Only accepts:
    - Stream `0` (playback): `{ channels = 2, format = S16_LE, rate = 48000 }`
    - Stream `1` (capture): `{ channels = 1, format = S16_LE, rate = 48000 }`
- `VIRTIO_SND_R_PCM_PREPARE (0x0102)`
  - Requires parameters to have been set.
- `VIRTIO_SND_R_PCM_START (0x0104)`
  - Requires the stream to have been prepared.
- `VIRTIO_SND_R_PCM_STOP (0x0105)`
  - Requires the stream to be running.
- `VIRTIO_SND_R_PCM_RELEASE (0x0103)`
  - Resets stream state back to `Idle`.

All jack/chmap requests return `VIRTIO_SND_S_NOT_SUPP`.

### TX Queue (Playback)

The TX virtqueue is used to submit PCM frames to the host.

The device expects a descriptor chain with:

1. One or more **out** descriptors containing:
   - An 8-byte header: `stream_id: u32` + `reserved: u32`
   - Followed by raw PCM bytes (interleaved S16_LE stereo)
2. At least one **in** descriptor containing an 8-byte response:
   - `status: u32`
   - `latency_bytes: u32` (currently `0`)

PCM writes are accepted only when the stream has been started; otherwise `VIRTIO_SND_S_IO_ERR` is returned.

### RX Queue (Capture)

The RX virtqueue is used by the guest to fetch captured PCM frames from the host.

The device expects a descriptor chain with:

1. One or more **out** descriptors containing:
   - An 8-byte header: `stream_id: u32` + `reserved: u32`
2. One or more **in** descriptors for PCM payload bytes (device writes captured PCM here)
   - Raw PCM bytes (S16_LE mono, 48kHz)
3. A final **in** descriptor containing an 8-byte response:
   - `status: u32`
   - `latency_bytes: u32` (currently `0`)

Captured reads are serviced only when the capture stream has been started; otherwise `VIRTIO_SND_S_IO_ERR` is returned.

If the host capture backend cannot provide enough samples to fill the payload buffers, the device writes silence for the missing samples and increments host-side underrun telemetry counters.

#### Capture sample source

In browser builds, captured samples are expected to come from the Web mic capture
ring buffer (`SharedArrayBuffer`) via `aero_platform::audio::mic_bridge::MicBridge` (re-exported as
`aero_audio::mic_bridge::MicBridge`).

#### Capture sample-rate conversion

The browser microphone capture graph runs at the owning `AudioContext.sampleRate` (which browsers may ignore; Safari/iOS often uses 44.1kHz). The virtio-snd guest-facing ABI is fixed at 48kHz S16_LE, so the RX/capture path resamples from the host capture rate to **48kHz** before encoding PCM payload bytes.

In `aero_virtio::devices::snd::VirtioSnd`, the host capture rate is tracked by `capture_sample_rate_hz` (defaults to `host_sample_rate_hz`).

#### AudioWorklet ring buffer layout

The AudioWorklet ring buffer used by the AU-WORKLET path uses **frame indices** (not sample indices).
The canonical layout is defined in:

- Rust: `crates/platform/src/audio/worklet_bridge.rs` (`aero_platform::audio::worklet_bridge`, re-exported by `aero_audio::worklet_bridge`)
- JS:
  - `apps/web/src/audio/audio_worklet_ring.ts` + `apps/web/src/platform/audio_worklet_ring_layout.js` (layout constants + helper math)
  - `apps/web/src/platform/audio.ts` (main-thread ring producer helpers + AudioWorkletNode wiring)
  - `apps/web/src/platform/audio-worklet-processor.js` (AudioWorklet consumer)

Layout (little-endian):

- u32 `readFrameIndex` (bytes 0..4)
- u32 `writeFrameIndex` (bytes 4..8)
- u32 `underrunCount` (bytes 8..12): total missing output frames rendered as silence due to underruns (wraps at 2^32)
- u32 `overrunCount` (bytes 12..16): frames dropped by the producer due to buffer full (wraps at 2^32)
- f32 `samples[]` (bytes 16..), interleaved by channel: `L0, R0, L1, R1, ...`

### Host sample rate and resampling

In the canonical Rust device model (`aero_virtio::devices::snd::VirtioSnd`), the guest contract is fixed at **48kHz** PCM, but the host Web Audio graph may run at a different sample rate. The device therefore performs sample-rate conversion in both directions:

#### TX / playback

TX PCM samples are:

1. decoded from interleaved S16_LE to interleaved `f32`
2. resampled from the guest contract rate (**48kHz**) to the host/output sample rate (typically `AudioContext.sampleRate`)
3. pushed into an `aero_audio::sink::AudioSink` (usually an AudioWorklet ring buffer producer).

#### RX / capture

RX PCM samples are:

1. read as mono `f32` from the capture backend (typically `MicBridge`) at the host/input sample rate
2. resampled from the host/input sample rate (`capture_sample_rate_hz`, defaulting to `host_sample_rate_hz`) to the guest contract rate (**48kHz**)
3. encoded as S16_LE and written into the guest RX payload buffers.

`host_sample_rate_hz` defaults to 48kHz, but can be configured via:

- `VirtioSnd::new_with_host_sample_rate(...)`
- `VirtioSnd::new_with_capture_and_host_sample_rate(...)`
- `VirtioSnd::set_host_sample_rate_hz(...)`

If the capture input rate differs from the playback/output rate, override it via:

- `VirtioSnd::set_capture_sample_rate_hz(...)`

### Windows 7 Driver Strategy

Windows 7 does not ship a virtio-snd driver. Expected options:

#### Option A: Custom test-signed WDM audio miniport

- Implement a WDM audio miniport (PortCls / WaveRT) that:
  - Enumerates as a standard Windows audio endpoint.
  - Uses the Aero **virtio-pci modern** transport (PCI vendor capabilities + BAR0 MMIO) and split virtqueues to:
    - Query stream capabilities (`PCM_INFO`)
    - Negotiate params (`PCM_SET_PARAMS`)
    - Start/stop streams and submit PCM buffers via the TX queue (playback) and RX queue (capture).
  - Works with PCI **INTx** (contract v1 baseline requires INTx; ISR status is read-to-ack to deassert the line).
  - Supports **MSI/MSI-X** (message-signaled interrupts). The in-tree Windows 7 driver package opts in via INF
    (`MSISupported=1`), prefers message interrupts when Windows grants them, programs virtio MSI-X routing
    (`msix_config`, `queue_msix_vector`), and verifies read-back.
    - On Aero contract devices, MSI-X is **exclusive** when enabled: if a virtio MSI-X selector is
      `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`) (or the MSI-X entry is masked/unprogrammed), interrupts for that source are
      **suppressed** (no MSI-X message and no INTx fallback). Drivers must not rely on “INTx fallback” unless MSI-X is
      actually disabled and INTx resources are in use.
  - Optional bring-up toggles (per-device registry, intended for early device-model bring-up):
    - `HKLM\SYSTEM\CurrentControlSet\Enum\<DeviceInstancePath>\Device Parameters\Parameters\AllowPollingOnly` (`REG_DWORD`)
      - `1`: allow polling-only mode if no usable interrupt resource can be connected (neither MSI/MSI-X nor INTx)
    - `HKLM\SYSTEM\CurrentControlSet\Enum\<DeviceInstancePath>\Device Parameters\Parameters\ForceNullBackend` (`REG_DWORD`)
      - `1`: force the silent null backend and allow `START_DEVICE` to succeed even when virtio transport bring-up fails
    - Find `<DeviceInstancePath>` via **Device Manager → device → Details → “Device instance path”**.
    - The shipped INFs seed these values with `FLG_ADDREG_NOCLOBBER` so explicit user overrides persist across reinstall/upgrade.
    - Backwards compatibility: older installs may store these values under the device/driver software key; the driver checks the per-device `Device Parameters` key first and falls back.
- Distribute as **test-signed**:
  - Enable test mode in the guest (`bcdedit /set testsigning on`).
  - Install the test certificate into the guest's trusted store.

This is the most controlled path and avoids licensing ambiguity.

In this repo, the in-tree implementation of this approach is:

- `drivers/windows7/virtio-snd/` (WDM PortCls + WaveRT audio driver)

See also:

- [`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) (definitive device/driver contract; transport + required features)
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) (WDM modern transport + interrupts bring-up guide)

#### Option B: Reuse open-source virtio-win (if license-compatible)

If an existing virtio-win `viosnd` driver supports Windows 7 and the project license is compatible, it could be reused to avoid writing a custom audio miniport.

This option requires a licensing review (see `../history/project-history.md`) and validation that the driver supports at
minimum the subset implemented here (fixed-format playback + capture streams, S16_LE @ 48kHz).

### Browser Smoke Test (AudioWorklet)

A standalone smoke test is provided under `apps/web/` to validate AudioWorklet playback and sample-rate handling.

It generates a 48kHz stereo tone and (if needed) linearly resamples it to the *actual* `AudioContext.sampleRate` before writing into the shared ring buffer. This prevents pitch-shift on browsers that ignore the requested sample rate (Safari/iOS commonly uses 44.1kHz).

Because it uses `SharedArrayBuffer`, it must be served with cross-origin isolation headers (COOP/COEP). A minimal local server is included:

```bash
node apps/web/serve-smoke-test.mjs
```

Then open:

```
http://localhost:8000/
```

## Virtio PCI modern transport bring-up (Windows 7, WDM)

This page describes the **WDM** (non-KMDF) bring-up flow for Aero
**virtio-pci modern** devices (Virtio 1.0+, PCI vendor capabilities + MMIO).

For WDM (non-KMDF) Windows 7 virtio drivers in this repo (for example
`virtio-snd`), the canonical transport implementation is:

- `drivers/windows/virtio/pci-modern/` (`VirtioPciModernTransport*`)

Note: Windows 7 **miniport** drivers (`virtio-blk`, `virtio-net`) use the
miniport-friendly shim under `drivers/windows7/virtio/common/` instead.

It is implemented in:

- `drivers/windows/virtio/pci-modern/virtio_pci_modern_transport.h`
- `drivers/windows/virtio/pci-modern/virtio_pci_modern_transport.c`

For the shared Windows 7 **INTx** helper (ISR read-to-ack + DPC dispatch), use:

- `drivers/windows7/virtio/common/include/virtio_pci_intx_wdm.h`
- `drivers/windows7/virtio/common/src/virtio_pci_intx_wdm.c`

For optional **MSI/MSI-X** (message-signaled) interrupts in WDM drivers, use:

- `drivers/windows7/virtio/common/include/virtio_pci_msix_wdm.h`
- `drivers/windows7/virtio/common/src/virtio_pci_msix_wdm.c`

This helper connects message interrupts via `IoConnectInterruptEx(CONNECT_MESSAGE_BASED)` and dispatches:

- message/vector 0: config callback
- when `MessageCount >= (1 + QueueCount)`: vectors 1..QueueCount drain queues 0..QueueCount-1
- otherwise: all queues are drained on vector 0.

For WDM drivers that want a single **INTx-or-message** connect/disconnect API, use:

- `drivers/windows7/virtio/common/include/virtio_pci_interrupts_wdm.h`
- `drivers/windows7/virtio/common/src/virtio_pci_interrupts_wdm.c`

This helper selects INTx vs MSI/MSI-X based on the translated interrupt descriptor and supports
a caller-controlled MessageId→(IsConfig/QueueIndex) routing table for message interrupts.

Note: For MSI/MSI-X, `IoConnectInterruptEx` requires a **PhysicalDeviceObject** (PDO). The combined helper
therefore takes both the driver's device object (typically the FDO) and the underlying PDO.

For the binding device/driver contract, see:
[`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md).

### Aero contract v1 transport expectations (device-model side)

Contract v1 (`AERO-W7-VIRTIO`, PCI Revision ID `0x01`) locks down the modern
transport to keep Windows 7 bring-up deterministic.

#### BAR0 MMIO

- **BAR0** is a **memory BAR** (MMIO), little-endian, size **>= 0x4000**.
- All required virtio configuration windows are in **BAR0** (contract v1 fixed
  layout).

Contract v1 fixed layout (all in BAR0):

| Capability | `cfg_type` | Offset | Minimum length |
|---|---:|---:|---:|
| `COMMON_CFG` | 1 | `0x0000` | `0x0100` |
| `NOTIFY_CFG` | 2 | `0x1000` | `0x0100` |
| `ISR_CFG` | 3 | `0x2000` | `0x0020` |
| `DEVICE_CFG` | 4 | `0x3000` | `0x0100` |

`NOTIFY_CFG.notify_off_multiplier` is required to be `4` by contract v1.

#### Required virtio vendor capabilities (PCI cap ID `0x09`)

PCI config space must contain a valid capability list with these virtio
vendor-specific capabilities:

- `VIRTIO_PCI_CAP_COMMON_CFG` (`cfg_type = 1`) → `common_cfg`
- `VIRTIO_PCI_CAP_NOTIFY_CFG` (`cfg_type = 2`) → notify doorbell region +
  `notify_off_multiplier`
- `VIRTIO_PCI_CAP_ISR_CFG` (`cfg_type = 3`) → ISR status register (read-to-ack)
- `VIRTIO_PCI_CAP_DEVICE_CFG` (`cfg_type = 4`) → device-specific config window

#### INTx + ISR read-to-ack semantics

Contract v1 requires **INTx**:

- The device asserts INTx when it sets any ISR cause bit.
- The driver **must read** the ISR status byte to **acknowledge** the interrupt
  and deassert the line.
- The ISR byte is **read-to-ack**; reading returns the pending cause bits and
  clears them.
- If the ISR byte reads as `0`, the interrupt is **not** for this device
  (important for shared vectors).

The helper `virtio_pci_intx_wdm` implements the canonical Windows 7 INTx pattern:
read ISR in the ISR (ack) and dispatch to driver callbacks at DPC level.

#### Required feature bits

All Aero modern devices require:

- `VIRTIO_F_VERSION_1` (bit 32) — always enforced by
  `VirtioPciModernTransportNegotiateFeatures`.
- `VIRTIO_F_RING_INDIRECT_DESC` (bit 28) — required by contract v1; strict-mode
  transport init/negotiation rejects devices that do not offer it.

### IRQL + locking notes

In practice:

- `VirtioPciModernTransportInit/Uninit` belong in `IRP_MN_START_DEVICE` and
  stop/remove handling and should be called at `PASSIVE_LEVEL` (they map/unmap
  MMIO and may query PCI config).
- `VirtioPciModernTransportNegotiateFeatures` should be called at
  `PASSIVE_LEVEL` (it performs a device reset handshake and may sleep/yield).
- Status/queue/config helpers are designed to be usable at `<= DISPATCH_LEVEL`
  (for example from a DPC), but queue programming is typically performed during
  start at `PASSIVE_LEVEL`.

Note: the transport reset helper is IRQL-aware. In kernel-mode builds it will
avoid long stalls at elevated IRQL by capping the busy-wait budget and returning
even if the device does not complete the reset handshake within that budget.

`common_cfg` contains selector registers (`device_feature_select`,
`driver_feature_select`, `queue_select`). The transport creates a per-device
spinlock (provided by the OS interface) and uses it internally to serialize
selector-based sequences.

### Canonical WDM PnP flow (modern transport + interrupts)

High-level sequencing for `IRP_MN_START_DEVICE` (omitting IRP forwarding boilerplate):

1. **Query bus interface** (e.g. `BUS_INTERFACE_STANDARD`) and read PCI config
2. **Locate BAR0** in the translated resource list and pass its translated
    physical address/length to `VirtioPciModernTransportInit`
3. **Negotiate features**
4. **Allocate and program virtqueues**
5. **Connect interrupts**:
   - Prefer MSI/MSI-X when the resource list contains `CM_RESOURCE_INTERRUPT_MESSAGE`:
      - Connect via `VirtioMsixConnect` (from `virtio_pci_msix_wdm`)
      - Program `common_cfg.msix_config` / `common_cfg.queue_msix_vector` using the helper's
        `ConfigVector` / `QueueVectors[]` (and the transport helpers
        `VirtioPciModernTransportSetConfigMsixVector` / `VirtioPciModernTransportSetQueueMsixVector`).
   - Fall back to legacy INTx using `VirtioIntxConnect` (from `virtio_pci_intx_wdm`).
   - Alternatively, use the combined helper `VirtioPciWdmInterruptConnect` (from `virtio_pci_interrupts_wdm`),
     which selects INTx vs MSI/MSI-X based on the translated interrupt descriptor.
6. **Set `DRIVER_OK`**

For stop/remove, disconnect interrupts first, reset the device, free queue resources,
then uninit the transport.

### Minimal pseudo-code (WDM START/STOP with the transport + INTx helper)

This snippet is intentionally simplified and omits DMA allocation details,
queue draining, and IRP forwarding.

```c
#include "virtio_pci_intx_wdm.h"
#include "virtio_pci_modern_transport.h"

typedef struct _DEVICE_CONTEXT {
    PDEVICE_OBJECT Self;
    PDEVICE_OBJECT Lower;

    VIRTIO_PCI_MODERN_OS_INTERFACE Os;
    VIRTIO_PCI_MODERN_TRANSPORT Pci;

    VIRTIO_INTX Intx;
} DEVICE_CONTEXT;

NTSTATUS StartDevice(_Inout_ DEVICE_CONTEXT *ctx,
                     _In_ PCM_RESOURCE_LIST raw,
                     _In_ PCM_RESOURCE_LIST translated)
{
    // 1) Fill ctx->Os callbacks:
    //    - PciRead8/16/32 via BUS_INTERFACE_STANDARD
    //    - MapMmio via MmMapIoSpace
    //    - StallUs via KeStallExecutionProcessor
    //    - SpinlockCreate/Acquire/Release via Ke* spinlocks
    //
    // 2) Find BAR0 translated physical address/length from CM_RESOURCE_LIST.
    UINT64 bar0_pa = /* ... */;
    UINT32 bar0_len = /* ... */;

    NTSTATUS st = VirtioPciModernTransportInit(&ctx->Pci,
                                              &ctx->Os,
                                              VIRTIO_PCI_MODERN_TRANSPORT_MODE_STRICT,
                                              bar0_pa,
                                              bar0_len);
    if (!NT_SUCCESS(st)) return st;

    UINT64 negotiated = 0;
    st = VirtioPciModernTransportNegotiateFeatures(&ctx->Pci,
                                                   /*Required=*/((UINT64)1u << 28), // INDIRECT_DESC
                                                   /*Wanted=*/0,
                                                   &negotiated);
    if (!NT_SUCCESS(st)) goto fail_uninit;

    // 3) Allocate and program queues...
    st = VirtioPciModernTransportSetupQueue(&ctx->Pci, /*q=*/0, /*desc_pa=*/0, /*avail_pa=*/0, /*used_pa=*/0);
    if (!NT_SUCCESS(st)) goto fail_reset;

    // 4) Connect INTx (translated interrupt resource + ctx->Pci.IsrStatus).
    st = VirtioIntxConnect(ctx->Self,
                           /*InterruptDescTranslated=*/NULL,
                           ctx->Pci.IsrStatus,
                           /*EvtConfigChange=*/NULL,
                           /*EvtQueueWork=*/NULL,
                           /*EvtDpc=*/NULL,
                           /*Cookie=*/ctx,
                           &ctx->Intx);
    if (!NT_SUCCESS(st)) goto fail_reset;

    VirtioPciModernTransportAddStatus(&ctx->Pci, VIRTIO_STATUS_DRIVER_OK);
    return STATUS_SUCCESS;

fail_reset:
    VirtioPciModernTransportResetDevice(&ctx->Pci);
fail_uninit:
    VirtioPciModernTransportUninit(&ctx->Pci);
    return st;
}

VOID StopDevice(_Inout_ DEVICE_CONTEXT *ctx)
{
    VirtioIntxDisconnect(&ctx->Intx);
    VirtioPciModernTransportResetDevice(&ctx->Pci);
    VirtioPciModernTransportUninit(&ctx->Pci);
}
```

Note: in strict mode `VirtioPciModernTransportInit` verifies that the BAR0
physical address passed to it matches the BAR0 base programmed in PCI config
space. If your driver stack reports different *raw* vs *translated* addresses,
pass the BAR0 bus address and translate inside your `MapMmio` callback.

### How this relates to other virtio code in this repo

#### `drivers/windows/virtio/pci-modern/` (portable modern transport)

`drivers/windows/virtio/pci-modern/` provides the canonical WDF-free virtio-pci modern transport used by Aero’s
Windows 7 **WDM** virtio drivers (for example `virtio-snd`). It:

- parses the PCI vendor capability list (contract §1.3),
- validates `RevisionID == 0x01`, and
- supports **STRICT** (enforce fixed offsets) vs **COMPAT** (accept relocated caps / 32-bit BAR0) policy.

Drivers integrate it by implementing the `VIRTIO_PCI_MODERN_OS_INTERFACE` callbacks (PCI config reads, BAR mapping,
stall, and selector-serialization lock).

#### `drivers/windows7/virtio/common/` (miniport-friendly Win7 helpers)

Windows 7 **miniport** drivers (`virtio-blk` / `virtio-net`) use the miniport-friendly modern transport shim in:

- `drivers/windows7/virtio/common/include/virtio_pci_modern_miniport.h`
- `drivers/windows7/virtio/common/src/virtio_pci_modern_miniport.c`

This shim is shaped for NDIS/StorPort: callers provide a mapped BAR0 pointer and a cached 256-byte PCI config snapshot,
so drivers do not need to implement `VIRTIO_PCI_MODERN_OS_INTERFACE`.

#### `drivers/win7/virtio/virtio-core/` (KMDF-centric modern transport)

`virtio-core` provides a more general virtio-pci modern discovery/mapping layer, primarily targeted at **KMDF** drivers.

It can be built without WDF (`VIRTIO_CORE_USE_WDF=0`) but uses a different device
abstraction (`VIRTIO_PCI_MODERN_DEVICE`) and provides its own init/mapping
helpers. For WDF-free drivers in this repo:

- WDM drivers typically use `drivers/windows/virtio/pci-modern/`.
- Miniport drivers typically use `drivers/windows7/virtio/common/`.

## Virtio PCI (Modern) Interrupts on Windows 7 (KMDF)

This document is a **hands-on implementation guide** for wiring up interrupt handling in a Windows 7 **KMDF** driver for a **virtio-pci modern** device (Virtio 1.0+ PCI capabilities). It covers both:

* **MSI-X (message-signaled)** interrupts (preferred).
* **Legacy INTx (line-based)** interrupts (fallback).

The intent is that a virtio-input (or any virtio-pci modern) driver can implement interrupts correctly **without additional research**.

See also:

* [`../virtio/virtqueue-split-ring-win7.md`](windows7-virtio-driver-contract.md) — split-ring virtqueue algorithms (descriptor mgmt, ordering/barriers, EVENT_IDX, indirect).
* [`../windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) — Aero’s definitive virtio device/feature/transport contract.

### Terminology / constants (virtio + Windows)

* **INTx**: legacy PCI line interrupt (level-triggered, frequently shared).
* **MSI-X message** (Windows): one delivered interrupt “message”. In virtio MSI-X programming, this corresponds to an **MSI-X table entry index**.
* Virtio vector sentinel:
  * `VIRTIO_PCI_MSI_NO_VECTOR` = `0xFFFF` (means “no vector assigned”).
* Virtio ISR Status register (`VIRTIO_PCI_CAP_ISR_CFG`, 1 byte, read-to-clear):
  * bit 0 (`0x01`): a queue has pending work
  * bit 1 (`0x02`): configuration change

### Enumerating interrupts in `EvtDevicePrepareHardware`

KMDF calls:

```c
NTSTATUS EvtDevicePrepareHardware(
    WDFDEVICE Device,
    WDFCMRESLIST ResourcesRaw,
    WDFCMRESLIST ResourcesTranslated
    );
```

You must locate interrupt resources in the translated list, determine whether you got MSI-X (message) or INTx (line), and capture the MSI-X message count.

#### Canonical helper (recommended)

Aero’s Windows virtio drivers use a shared helper that implements the patterns in this
document:

* `drivers/windows/virtio/kmdf/virtio_pci_interrupts.{c,h}`

Key entry points:

* `VirtioPciInterruptsPrepareHardware(...)` — enumerates resources and creates the KMDF `WDFINTERRUPT` objects (MSI-X or INTx).
* `VirtioPciInterruptsProgramMsixVectors(...)` — programs `common_cfg.msix_config` / `queue_msix_vector` using the mapping chosen at prepare time.
* `VirtioPciInterruptsQuiesce(...)` / `VirtioPciInterruptsResume(...)` — implements the MSI-X reset/quiesce sequencing described in section 4.1.

Example usage (simplified):

```c
// EvtDevicePrepareHardware
status = VirtioPciInterruptsPrepareHardware(
    Device,
    &ctx->Interrupts,
    ResourcesRaw,
    ResourcesTranslated,
    ctx->NumQueues,
    ctx->Pci.IsrStatus,
    ctx->Pci.CommonCfgLock, // serializes queue_select sequences
    VirtioConfigChanged,
    VirtioDrainQueue,
    ctx);

// EvtDeviceD0Entry (after each virtio reset)
status = VirtioPciInterruptsProgramMsixVectors(&ctx->Interrupts, ctx->Pci.CommonCfg);

// Reset/reconfigure path (PASSIVE_LEVEL)
status = VirtioPciInterruptsQuiesce(&ctx->Interrupts, ctx->Pci.CommonCfg);
// ... reset device + re-init queues ...
status = VirtioPciInterruptsResume(&ctx->Interrupts, ctx->Pci.CommonCfg);
```

#### Step-by-step: find `CmResourceTypeInterrupt`

1. Enumerate the **translated** resource list with `WdfCmResourceListGetCount/Descriptor`.
2. For each entry:
   * If `desc->Type == CmResourceTypeInterrupt`, it’s an interrupt resource.
3. Keep the **matching** raw descriptor at the same index (raw and translated lists are aligned by index for WDF).

#### Step-by-step: distinguish MSI-X vs INTx

For a `CmResourceTypeInterrupt` descriptor:

* **Message-signaled (MSI/MSI-X)** if `(desc->Flags & CM_RESOURCE_INTERRUPT_MESSAGE) != 0`
* **Line-based (INTx)** otherwise

Notes:
* On Windows 7, message-signaled delivery is typically **opt-in via INF** (see section 5).
* Prefer MSI-X if both appear (uncommon, but drivers should handle it defensively).

#### Step-by-step: obtain MSI-X message count (Win7 `CM_PARTIAL_RESOURCE_DESCRIPTOR`)

For message-signaled interrupts, Windows reports a single `CmResourceTypeInterrupt` descriptor whose union is **`u.MessageInterrupt`** (not `u.Interrupt`).

On Windows 7 WDK headers, the message count is:

```c
USHORT messageCount = descTranslated->u.MessageInterrupt.MessageCount;
```

Store this count in your device context; you will use it to decide whether you can afford:

* **1 vector** for config + **1 vector per queue**, or
* the fallback “everything on vector 0” mapping (section 3).

#### Pseudocode: resource enumeration

```c
typedef struct _DEVICE_CONTEXT {
    // Interrupt “mode” selected from PnP resources.
    BOOLEAN UseMsix;

    // Number of MSI(-X) messages Windows granted for the device.
    USHORT MsixMessageCount; // 0 if not message-signaled

    // Indices into the CM resource lists.
    ULONG MsixResourceIndex; // valid if UseMsix
    ULONG IntxResourceIndex; // valid if !UseMsix

    // Interrupt objects (created from the resources above).
    WDFINTERRUPT IntxInterrupt;     // valid if !UseMsix
    WDFINTERRUPT* MsixInterrupts;   // array [MsixMessageCount] if UseMsix

    // Vector mapping policy for MSI-X.
    BOOLEAN MsixAllOnVector0; // TRUE when messages < (1 + NumQueues)

    // For INTx: stash ISR status bits for the DPC (ISR status is read-to-clear).
    volatile UCHAR PendingIsrStatus;
} DEVICE_CONTEXT;

NTSTATUS EvtDevicePrepareHardware(
    WDFDEVICE Device,
    WDFCMRESLIST Raw,
    WDFCMRESLIST Translated
    )
{
    DEVICE_CONTEXT* ctx = DeviceGetContext(Device);
    ctx->UseMsix = FALSE;
    ctx->MsixMessageCount = 0;

    ULONG count = WdfCmResourceListGetCount(Translated);
    for (ULONG i = 0; i < count; i++) {
        PCM_PARTIAL_RESOURCE_DESCRIPTOR t = WdfCmResourceListGetDescriptor(Translated, i);
        if (!t) continue;

        if (t->Type != CmResourceTypeInterrupt) continue;

        const BOOLEAN isMessage =
            ((t->Flags & CM_RESOURCE_INTERRUPT_MESSAGE) != 0);

        if (isMessage) {
            ctx->UseMsix = TRUE;
            ctx->MsixResourceIndex = i;
            ctx->MsixMessageCount = t->u.MessageInterrupt.MessageCount;
        } else {
            // Only keep INTx if we didn’t find MSI-X (prefer MSI-X).
            if (!ctx->UseMsix) {
                ctx->IntxResourceIndex = i;
            }
        }
    }

    return STATUS_SUCCESS;
}
```

### Creating KMDF interrupt objects

#### Recommended MSI-X strategy: *one `WDFINTERRUPT` per message*

For virtio, a “one interrupt object per vector” model is convenient because:

* Each MSI-X vector gets its own **ISR + DPC callback**.
* Each DPC can have its own **context** (queue index, etc).
* You avoid a big “switch(MessageID)” ISR, and you can choose to parallelize per-queue work.

In KMDF, you do this by calling `WdfInterruptCreate` **once per message** with:

* `WDF_INTERRUPT_CONFIG.InterruptRaw` / `.InterruptTranslated` pointing at the **same** message interrupt resource descriptor.
* `WDF_INTERRUPT_CONFIG.MessageNumber = i` for message `i`.

> The important detail: the CM resource list gives you one message interrupt descriptor with `MessageCount`. You reuse it for each `WdfInterruptCreate`, changing only `MessageNumber`.

#### INTx strategy: create a single `WDFINTERRUPT`

For INTx you create exactly one interrupt object, bound to the line-based descriptor. There is no message count, no `MessageNumber` to iterate.

#### Pseudocode: creating interrupts (MSI-X and INTx)

```c
typedef enum _VIRTIO_INTERRUPT_KIND {
    VirtioInterruptConfig,
    VirtioInterruptQueue,
} VIRTIO_INTERRUPT_KIND;

typedef struct _INTERRUPT_CONTEXT {
    DEVICE_CONTEXT* Device;
    VIRTIO_INTERRUPT_KIND Kind;
    ULONG QueueIndex;   // valid if Kind == VirtioInterruptQueue
    ULONG MessageNumber; // 0..(MessageCount-1)
} INTERRUPT_CONTEXT;

WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(INTERRUPT_CONTEXT, InterruptGetContext);

NTSTATUS CreateInterrupts(WDFDEVICE Device, WDFCMRESLIST Raw, WDFCMRESLIST Translated)
{
    DEVICE_CONTEXT* ctx = DeviceGetContext(Device);

    if (ctx->UseMsix) {
        PCM_PARTIAL_RESOURCE_DESCRIPTOR rawDesc =
            WdfCmResourceListGetDescriptor(Raw, ctx->MsixResourceIndex);
        PCM_PARTIAL_RESOURCE_DESCRIPTOR transDesc =
            WdfCmResourceListGetDescriptor(Translated, ctx->MsixResourceIndex);

        // Create one WDFINTERRUPT per message Windows granted (0..MessageCount-1).
        //
        // NOTE: You can create fewer (e.g. only 0..required-1) if you know you will
        // never program higher vectors, but creating one-per-message keeps the
        // bookkeeping straightforward.
        ctx->MsixInterrupts = (WDFINTERRUPT*)ExAllocatePoolWithTag(
            NonPagedPool,
            sizeof(WDFINTERRUPT) * ctx->MsixMessageCount,
            'xMsV');
        if (!ctx->MsixInterrupts) return STATUS_INSUFFICIENT_RESOURCES;
        RtlZeroMemory(ctx->MsixInterrupts, sizeof(WDFINTERRUPT) * ctx->MsixMessageCount);

        for (ULONG m = 0; m < ctx->MsixMessageCount; m++) {
            WDF_INTERRUPT_CONFIG icfg;
            WDF_INTERRUPT_CONFIG_INIT(&icfg, VirtioMsixIsr, VirtioMsixDpc);
            icfg.InterruptRaw = rawDesc;
            icfg.InterruptTranslated = transDesc;
            icfg.MessageNumber = m;

            WDF_OBJECT_ATTRIBUTES attr;
            WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attr, INTERRUPT_CONTEXT);
            attr.ParentObject = Device;

            WDFINTERRUPT interrupt;
            NTSTATUS status = WdfInterruptCreate(Device, &icfg, &attr, &interrupt);
            if (!NT_SUCCESS(status)) return status;
            ctx->MsixInterrupts[m] = interrupt;

            INTERRUPT_CONTEXT* ictx = InterruptGetContext(interrupt);
            ictx->Device = ctx;
            ictx->MessageNumber = m;
            if (m == 0) {
                // Recommended mapping: message 0 is used for virtio config changes.
                ictx->Kind = VirtioInterruptConfig;
            } else {
                // Recommended mapping: message 1..N used for queue 0..(N-1).
                ictx->Kind = VirtioInterruptQueue;
                ictx->QueueIndex = m - 1;
            }
        }

        return STATUS_SUCCESS;
    }

    // INTx: exactly one interrupt object
    PCM_PARTIAL_RESOURCE_DESCRIPTOR rawDesc =
        WdfCmResourceListGetDescriptor(Raw, ctx->IntxResourceIndex);
    PCM_PARTIAL_RESOURCE_DESCRIPTOR transDesc =
        WdfCmResourceListGetDescriptor(Translated, ctx->IntxResourceIndex);

    WDF_INTERRUPT_CONFIG icfg;
    WDF_INTERRUPT_CONFIG_INIT(&icfg, VirtioIntxIsr, VirtioIntxDpc);
    icfg.InterruptRaw = rawDesc;
    icfg.InterruptTranslated = transDesc;

    return WdfInterruptCreate(Device, &icfg, WDF_NO_OBJECT_ATTRIBUTES, &ctx->IntxInterrupt);
}
```

### Programming virtio MSI-X vectors (`msix_config`, `queue_msix_vector`)

Virtio-pci modern uses **device registers** (in `common_cfg`) to route interrupt sources to MSI-X vectors:

* `common_cfg->msix_config` routes **configuration-change** interrupts.
* `common_cfg->queue_msix_vector` routes a specific **queue’s** interrupts (requires setting `queue_select` first).

#### Mapping rule: Windows message number → virtio MSI-X vector

After creating a `WDFINTERRUPT` for message `i`, you can query:

```c
WDF_INTERRUPT_INFO info;
WDF_INTERRUPT_INFO_INIT(&info);
WdfInterruptGetInfo(Interrupt, &info);
ULONG message = info.MessageNumber; // 0-based
```

For MSI-X devices, `info.MessageNumber` is the **MSI-X table entry index** Windows assigned. That is the value you program into:

* `common_cfg->msix_config`
* `common_cfg->queue_msix_vector`

#### Required vectors vs available messages

Virtio typically wants:

* 1 vector for config, plus
* 1 vector per virtqueue you enable

So:

```
required = 1 + number_of_queues
```

If Windows grants fewer than `required` messages, use the required fallback mapping:

* Route **config + all queues** to **vector 0**.

This guarantees:

* interrupts still work,
* the driver doesn’t need partial/per-queue MSI-X logic, and
* DPC(0) can simply “handle everything”.

#### Read-back verification (device may return `VIRTIO_PCI_MSI_NO_VECTOR`)

After writing `msix_config` / `queue_msix_vector`, the driver should read back:

* If it reads back `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`), the device rejected the mapping (or MSI-X isn’t enabled).
* If it reads back something other than what you wrote, treat it as failure.

In either case, fall back to vector 0 mapping (or to INTx if you decide MSI-X is unusable).

#### Pseudocode: vector programming (with fallback)

```c
#ifndef VIRTIO_PCI_MSI_NO_VECTOR
#define VIRTIO_PCI_MSI_NO_VECTOR ((USHORT)0xFFFF)
#endif

static USHORT MsixVectorFromWdfInterrupt(_In_ WDFINTERRUPT Interrupt)
{
    WDF_INTERRUPT_INFO info;
    WDF_INTERRUPT_INFO_INIT(&info);
    WdfInterruptGetInfo(Interrupt, &info);

    // For message-signaled interrupts, MessageNumber is the MSI-X table entry.
    if (info.MessageNumber >= VIRTIO_PCI_MSI_NO_VECTOR) {
        return VIRTIO_PCI_MSI_NO_VECTOR;
    }
    return (USHORT)info.MessageNumber;
}

VOID ProgramVirtioMsixVectors(DEVICE_CONTEXT* ctx)
{
    if (!ctx->UseMsix || ctx->MsixMessageCount == 0) {
        // INTx path: nothing to program in common_cfg for routing.
        return;
    }

    const USHORT required = (USHORT)(1 + ctx->NumQueues);
    const BOOLEAN canDoPerQueue = (ctx->MsixMessageCount >= required);

    // Recommended mapping:
    //   message 0 -> virtio config
    //   message 1.. -> virtqueue 0..
    const USHORT configVec = MsixVectorFromWdfInterrupt(ctx->MsixInterrupts[0]);
    if (configVec == VIRTIO_PCI_MSI_NO_VECTOR) goto fallback_to_vec0;

    ctx->common_cfg->msix_config = configVec;
    if (ctx->common_cfg->msix_config != configVec) goto fallback_to_vec0;

    for (USHORT q = 0; q < ctx->NumQueues; q++) {
        USHORT qVec = configVec;
        if (canDoPerQueue) {
            qVec = MsixVectorFromWdfInterrupt(ctx->MsixInterrupts[q + 1]);
        }

        ctx->common_cfg->queue_select = q;
        ctx->common_cfg->queue_msix_vector = qVec;

        const USHORT readback = ctx->common_cfg->queue_msix_vector;
        if (readback == VIRTIO_PCI_MSI_NO_VECTOR || readback != qVec) {
            goto fallback_to_vec0;
        }
    }

    // Also verify msix_config didn’t get cleared/reset under us.
    if (ctx->common_cfg->msix_config == VIRTIO_PCI_MSI_NO_VECTOR) {
        goto fallback_to_vec0;
    }

    // Driver should record whether vector 0 is “shared” or dedicated.
    ctx->MsixAllOnVector0 = !canDoPerQueue;
    return;

fallback_to_vec0:
    // Required fallback: route config + all queues to vector 0.
    //
    // This assumes you created message 0 and it corresponds to MSI-X table entry 0.
    ctx->common_cfg->msix_config = 0;
    for (USHORT q = 0; q < ctx->NumQueues; q++) {
        ctx->common_cfg->queue_select = q;
        ctx->common_cfg->queue_msix_vector = 0;
    }
    ctx->MsixAllOnVector0 = TRUE;
}
```

**Important sequencing note:** a virtio **device reset** (writing `device_status = 0`) can clear `msix_config` / `queue_msix_vector` back to `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`). If your driver resets the device in `EvtDeviceD0Entry`, you must call `ProgramVirtioMsixVectors` **after** each reset (see pitfalls).

### ISR/DPC behavior: INTx vs MSI-X

#### INTx: ISR must read `isr_status` to ACK/deassert

INTx is level-triggered: if you do not deassert the interrupt line, you can get an **interrupt storm**.

For virtio-pci INTx, “ACK/deassert” is performed by reading the **ISR Status** register (read-to-clear). Therefore the INTx ISR should:

1. Read `isr_status` (MMIO read of 1 byte)
2. If the value is `0`, return `FALSE` (not your interrupt, important for shared lines)
3. Otherwise queue the DPC and return `TRUE`

Pseudocode:

```c
BOOLEAN VirtioIntxIsr(WDFINTERRUPT Interrupt, ULONG MessageID)
{
    UNREFERENCED_PARAMETER(MessageID);

    DEVICE_CONTEXT* ctx = DeviceGetContext(WdfInterruptGetDevice(Interrupt));

    // Read-to-clear; also deasserts the INTx line.
    UCHAR isr = READ_REGISTER_UCHAR(ctx->isr_status_reg);
    if (isr == 0) {
        return FALSE; // shared interrupt, not ours
    }

    // Optionally stash bits for the DPC since the register is now cleared.
    ctx->PendingIsrStatus |= isr;

    WdfInterruptQueueDpcForIsr(Interrupt);
    return TRUE;
}

VOID VirtioIntxDpc(WDFINTERRUPT Interrupt, WDFOBJECT AssociatedObject)
{
    UNREFERENCED_PARAMETER(AssociatedObject);

    DEVICE_CONTEXT* ctx = DeviceGetContext(WdfInterruptGetDevice(Interrupt));
    UCHAR isr = ctx->PendingIsrStatus;
    ctx->PendingIsrStatus = 0;

    if (isr & 0x02) {
        VirtioHandleConfigChange(ctx);
    }
    if (isr & 0x01) {
        VirtioServiceAllQueues(ctx);
    }
}
```

#### MSI-X: ISR should not depend on `isr_status`

With MSI-X:

* There is no level-triggered line to deassert.
* Windows delivers a specific message vector; use that to route work.
* You should not need `isr_status` for routing, and reading it can remove useful state if some other code expects it.

Pseudocode:

```c
BOOLEAN VirtioMsixIsr(WDFINTERRUPT Interrupt, ULONG MessageID)
{
    UNREFERENCED_PARAMETER(MessageID);
    WdfInterruptQueueDpcForIsr(Interrupt);
    return TRUE;
}

VOID VirtioMsixDpc(WDFINTERRUPT Interrupt, WDFOBJECT AssociatedObject)
{
    UNREFERENCED_PARAMETER(AssociatedObject);

    INTERRUPT_CONTEXT* ictx = InterruptGetContext(Interrupt);
    DEVICE_CONTEXT* ctx = ictx->Device;

    if (ctx->MsixAllOnVector0) {
        // Vector 0 is shared for config + all queues.
        VirtioHandleConfigChange(ctx);
        VirtioServiceAllQueues(ctx);
        return;
    }

    if (ictx->Kind == VirtioInterruptConfig) {
        VirtioHandleConfigChange(ctx);
    } else {
        VirtioServiceQueue(ctx, ictx->QueueIndex);
    }
}
```

### 4.1) MSI-X multi-vector concurrency model (and how to make it safe)

With MSI-X, Windows can deliver interrupts for different MSI-X messages to different CPUs. In KMDF, when you create **one `WDFINTERRUPT` per message**, each interrupt object has its own DPC — and **KMDF can run those DPCs concurrently**.

#### IRQL summary

* **ISR (`EvtInterruptIsr`)** runs at **DIRQL**:
  * keep it minimal: no heavy locks, no virtqueue draining, no reset work
  * typically just queues the DPC (`WdfInterruptQueueDpcForIsr`)
* **DPC (`EvtInterruptDpc`)** runs at **DISPATCH_LEVEL**:
  * this is where virtqueue used-ring draining/completions run
  * multiple DPCs (different MSI-X messages) may run in parallel
* **PnP/power/reset** paths run at **PASSIVE_LEVEL**:
  * `EvtDevicePrepareHardware`, `EvtDeviceD0Entry/D0Exit`, IOCTL threads, etc.
  * these can race with DPC code unless explicitly synchronized

#### What must be serialized (virtio-pci modern hazards)

1. **Per-virtqueue state + used-ring draining**
   * `last_used_idx`, in-flight descriptor tables, completion paths, etc.
   * must be protected against:
     * concurrent DPCs on other CPUs (other queues), and
     * submission/reset paths running outside the DPC
2. **Any sequence using `common_cfg.queue_select`**
   * `queue_select` is a single global register.
   * the sequence “write `queue_select`, then read/write queue-specific fields” must not interleave across threads.
3. **Device reset / MSI-X reprogramming vs active interrupts**
   * reset can clear vector registers back to `VIRTIO_PCI_MSI_NO_VECTOR`
   * interrupts must not run against partially initialized (or torn-down) queue state

#### Locking scheme (recommended)

If you want MSI-X parallelism, do **not** rely on implicit framework serialization.

Use explicit locks instead:

* **Per-queue spinlock** (one per virtqueue)
  * guards queue state + used-ring draining
  * queue DPC must acquire this lock before touching the queue
* **Global `common_cfg` spinlock**
  * guards any access that writes `common_cfg.queue_select` and then accesses queue-specific common_cfg fields (`queue_msix_vector`, `queue_enable`, `queue_notify_off`, …)

Lock ordering (to avoid deadlocks if a path needs both):

1. acquire `common_cfg` lock
2. acquire per-queue lock

#### KMDF: `WDF_INTERRUPT_CONFIG.AutomaticSerialization`

**Decision:** For MSI-X we intentionally set:

* `WDF_INTERRUPT_CONFIG.AutomaticSerialization = FALSE`

Rationale:

* With `AutomaticSerialization = TRUE`, KMDF often serializes ISR/DPC callbacks using the device synchronization scope, which can negate the performance benefit of per-queue MSI-X vectors.
* With it disabled, safety comes from the explicit spinlocks above.

#### Reset / reprogramming safety sequence (MSI-X)

At PASSIVE_LEVEL (e.g. device reset / D0Exit):

1. Set an atomic `ResetInProgress = TRUE` flag (DPCs bail out early).
2. Disable OS interrupt delivery (`WdfInterruptDisable` on each interrupt object).
3. Disable device-side routing:
   * program `msix_config = VIRTIO_PCI_MSI_NO_VECTOR`
   * for each queue: `queue_select = q; queue_msix_vector = VIRTIO_PCI_MSI_NO_VECTOR`
   * **Important (Aero Win7 contract):** when MSI-X is enabled, MSI-X is **exclusive**:
     `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`) suppresses interrupts for that source (**no MSI-X
     message and no INTx fallback**). This is useful for quiescing the device before reset/teardown.
4. Synchronize with in-flight DPC work:
   * acquire+release each queue lock once (waits for any running DPC to leave its critical section)
5. Reset / reinitialize device and queues.
6. Reprogram vectors (after reset) and verify read-back != `VIRTIO_PCI_MSI_NO_VECTOR`.
7. Re-enable OS interrupt delivery (`WdfInterruptEnable`).
8. Clear `ResetInProgress` only when queues are fully ready.

Concrete code implementing this model lives in:

* `drivers/windows/virtio/kmdf/virtio_pci_interrupts.c` (`VirtioPciInterruptsQuiesce` / `VirtioPciInterruptsResume`)

### Windows 7 INF: enabling MSI/MSI-X

On Windows 7, message-signaled interrupts are typically enabled through INF registry settings under the device’s hardware key.

Minimum required:

```inf
[MyDevice_Install.NT.HW]
AddReg = MyDevice_AddReg

[MyDevice_AddReg]
HKR, "Interrupt Management\MessageSignaledInterruptProperties", "MSISupported", 0x00010001, 1
```

Optional: request up to N messages (Windows may grant fewer):

```inf
HKR, "Interrupt Management\MessageSignaledInterruptProperties", "MessageNumberLimit", 0x00010001, 8
```

Notes:
* `0x00010001` = `REG_DWORD`.
* Without `MSISupported=1`, you should expect only INTx resources in `EvtDevicePrepareHardware`.
* Setting `MessageNumberLimit` higher than what the virtio device exposes in its MSI-X table cannot succeed; always handle “granted < requested” at runtime.

### Common pitfalls (checklist)

1. **INTx storm / hang**
   * Symptom: CPU stuck at high IRQL, ISR firing continuously.
   * Root cause: not reading `isr_status` in the INTx ISR (line never deasserts).
   * Fix: read `VIRTIO_PCI_CAP_ISR_CFG` in the ISR and return `FALSE` when it reads `0`.

2. **Using the wrong union fields for message interrupts**
   * Root cause: reading `desc->u.Interrupt.*` even when `CM_RESOURCE_INTERRUPT_MESSAGE` is set.
   * Fix: for message interrupts use `desc->u.MessageInterrupt.MessageCount` (Win7).

3. **Vector registers reset after virtio device reset**
   * Root cause: driver calls `device_status = 0` during init/recover but does not reprogram `msix_config` and `queue_msix_vector`.
   * Fix: call your MSI-X programming routine after every reset and before enabling queues/notifications.

4. **Assuming you always get (1 + numQueues) MSI-X messages**
   * Root cause: `MessageNumberLimit` is only a request; Windows can grant fewer messages.
   * Fix: implement the documented fallback mapping: config + all queues → vector 0.

5. **Concurrency surprises with “one WDFINTERRUPT per message”**
   * Multiple DPCs can run in parallel on different CPUs.
   * Per-interrupt locks (`WdfInterruptAcquireLock`) do **not** protect shared device/queue state across interrupts.
   * Fix: either:
     * add explicit locking around shared virtio state, or
     * configure the driver to serialize callbacks (device synchronization scope / interrupt serialization) and accept less parallelism.

## Virtio PCI “modern” interrupt bring-up on Windows 7 (MSI-X vs INTx)

This guide is a *validation/debug checklist* for getting interrupts working for a virtio-pci **modern** device on **Windows 7**.
It focuses on the common “everything enumerates but no interrupts” / “INTx storm” class of failures, and on proving (with WinDbg + DbgView + driver prints) what Windows actually configured.

See also:

* [`../virtio/virtqueue-split-ring-win7.md`](windows7-virtio-driver-contract.md) — split-ring virtqueue algorithms (descriptor mgmt, ordering/barriers, EVENT_IDX, indirect).
* [`../windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) — Aero’s definitive virtio device/feature/transport contract.

Assumptions:

- You have a kernel debugger (WinDbg) attached or can capture `DbgPrint` output with **DbgView**.
- Your driver is WDM or KMDF (examples use KMDF naming, but the resource concepts are WDM).
- Your device is virtio-pci modern (has a `common_cfg` with `msix_config` and `queue_msix_vector`).

---

### Quick mental model (what must line up)

For virtio-pci modern:

- Windows decides whether the device runs with **message-signaled interrupts** (MSI/MSI-X) or with **line-based INTx**.
- Your driver must:
  1. **Connect** whatever Windows assigned (INTx or MSI-X messages).
  2. If using MSI-X: **program virtio’s MSI-X vector selectors** (`common_cfg.msix_config`, `queue_msix_vector`) with the **MSI-X table entry indices** that correspond to the connected messages.
  3. If using INTx: **read the virtio ISR status register** in your ISR to deassert the line (or you’ll storm).

The debugging process below is about proving each link in that chain.

---

### Confirm which interrupt mode Windows assigned (MSI-X vs INTx)

#### 1.1 Device Manager → Resources tab (fast sanity check)

1. `devmgmt.msc` → find your device → **Properties** → **Resources**.
2. Look at **Interrupt Request** entries:
   - **INTx / line-based**: usually shows a small IRQ number (e.g. `IRQ 17`) and may be **shared** with other devices.
   - **Message signaled (MSI/MSI-X)**: typically shows one or *multiple* “Interrupt Request” entries with larger values (often shown in hex) and they’re usually **not shared**.

This is not perfect, but it’s a quick “are we even in the right mode?” check that works on Windows 7 without any special tooling.

#### 1.2 `aero-virtio-selftest.exe` IRQ markers (Aero guest tooling)

If you have access to Aero’s Windows 7 guest test tool (`aero-virtio-selftest.exe`), it can report which interrupt mode Windows actually assigned **without** WinDbg/DbgView:

- `virtio-blk-irq|INFO|mode=intx` *(cfgmgr32 / PnP-assigned IRQ resources)*
- `virtio-blk-irq|INFO|mode=msi|messages=<n>` *(cfgmgr32 / PnP-assigned IRQ resources)*
- `virtio-net-irq|INFO|mode=intx`
- `virtio-net-irq|INFO|mode=msi|messages=<n>`
- `virtio-input-irq|INFO|mode=intx`
- `virtio-input-irq|INFO|mode=msi|messages=<n>`
- `virtio-snd-irq|INFO|mode=intx`
- `virtio-snd-irq|INFO|mode=msix|messages=<n>|msix_config_vector=0x....|...` *(when the driver exposes the optional `\\.\aero_virtio_snd_diag` interface)*
  - Includes per-queue MSI-X routing (`msix_queue0_vector..msix_queue3_vector`) and diagnostic counters
    (`interrupt_count`, `dpc_count`, `drain0..drain3`).
- `virtio-snd-irq|INFO|mode=none|...` *(polling-only; no interrupt objects are connected)*
- `virtio-snd-irq|INFO|mode=msi|messages=<n>` *(fallback: message interrupts; does not distinguish MSI vs MSI-X)*

For `virtio-blk`, the selftest may also emit a richer miniport-IOCTL-derived line (best-effort; depends on the installed miniport contract):

- `virtio-blk-miniport-irq|INFO|mode=<intx|msi|unknown>|messages=<n>|message_count=<n>|msix_config_vector=0x....|msix_queue0_vector=0x....`

Notes:

- `mode=msi` means **message-signaled interrupts** (MSI or MSI-X). Most standalone markers do not attempt to distinguish MSI vs MSI-X, but some drivers may report `mode=msix` when additional vector routing diagnostics are available (for example via a device-specific diag interface). For virtio-blk, interpret non-`0xFFFF` `msix_*_vector` values as “MSI-X vectors are assigned” (and see the per-test `AERO_VIRTIO_SELFTEST|TEST|virtio-blk|...` marker for `irq_mode=msix` when available).
- `messages=<n>` / `message_count=<n>` is the number of interrupt messages Windows granted. Drivers must remain functional even when Windows grants fewer than requested (or only INTx).
- The tool logs to `C:\aero-virtio-selftest.log` and emits markers on stdout/COM1 for host-side parsing.

If you are using the in-tree Win7 QEMU harness (`drivers/windows7/tests/`), you can also ask QEMU to expose a larger MSI-X
table (requires QEMU virtio `vectors` property) and/or fail the harness when MSI-X is not enabled:

- Request a larger MSI-X table (requires QEMU virtio `vectors` property):
  - global: `-VirtioMsixVectors N` / `--virtio-msix-vectors N`
  - per device: `-Virtio{Net,Blk,Input,Snd}Vectors N` / `--virtio-{net,blk,input,snd}-vectors N`
- Require MSI-X (harness checks):
  - QMP MSI-X-enabled check (virtio-blk/net/snd):
    - PowerShell: `-RequireVirtio{Net,Blk,Snd}Msix` *(aliases: `-Require{Net,Blk,Snd}Msix`)*
    - Python: `--require-virtio-{net,blk,snd}-msix` *(aliases: `--require-{net,blk,snd}-msix`)*
    - For virtio-net, virtio-blk, and virtio-snd, the harness also requires the guest `virtio-*-msix` marker to report `mode=msix`
      (end-to-end validation: Windows actually assigned message interrupts and the driver programmed virtio MSI-X routing).
  - Guest marker check (virtio-input):
    - PowerShell: `-RequireVirtioInputMsix` *(alias: `-RequireInputMsix`)*
    - Python: `--require-virtio-input-msix` *(alias: `--require-input-msix`)*
    - Optional guest-side fail-fast: `aero-virtio-selftest.exe --require-input-msix` (or env var `AERO_VIRTIO_SELFTEST_REQUIRE_INPUT_MSIX=1`)
  - Optional guest-side fail-fast (make the *guest selftest* emit `RESULT|FAIL` when MSI/MSI-X expectations are not met):
    - virtio-blk: `aero-virtio-selftest.exe --expect-blk-msi` (or env var `AERO_VIRTIO_SELFTEST_EXPECT_BLK_MSI=1`)
    - virtio-net: `aero-virtio-selftest.exe --require-net-msix` (or env var `AERO_VIRTIO_SELFTEST_REQUIRE_NET_MSIX=1`)
    - virtio-snd: `aero-virtio-selftest.exe --require-snd-msix` (or env var `AERO_VIRTIO_SELFTEST_REQUIRE_SND_MSIX=1`)
    - virtio-input: `aero-virtio-selftest.exe --require-input-msix` (or env var `AERO_VIRTIO_SELFTEST_REQUIRE_INPUT_MSIX=1`)
    - When provisioning the guest scheduled task via `New-AeroWin7TestImage.ps1`, bake these in with:
      `-ExpectBlkMsi`, `-RequireNetMsix`, `-RequireSndMsix`, `-RequireInputMsix`.

See: [`drivers/windows7/tests/guest-selftest/README.md`](../decisions/README.md).

#### 1.3 Driver debug prints: detect `CM_RESOURCE_INTERRUPT_MESSAGE`

In `EvtDevicePrepareHardware` (KMDF) or `IRP_MN_START_DEVICE` (WDM), dump the translated resources. The key fact to print is whether the interrupt resource has the flag:

- `CM_RESOURCE_INTERRUPT_MESSAGE` set ⇒ **message-signaled** (MSI/MSI-X)
- not set ⇒ **INTx**

Also print the **message count** for message-signaled resources.

Minimal pseudo-code (WDM structs; names/fields are from WDK headers):

```c
static void DumpInterruptResource(_In_ PCM_PARTIAL_RESOURCE_DESCRIPTOR d)
{
    if (d->Type != CmResourceTypeInterrupt) return;

    const BOOLEAN isMessage = (d->Flags & CM_RESOURCE_INTERRUPT_MESSAGE) != 0;

    if (isMessage) {
        DbgPrintEx(DPFLTR_IHVDRIVER_ID, DPFLTR_INFO_LEVEL,
            "INT: MSI/MSI-X Level=%lu Vector=%lu Affinity=0x%Ix MessageCount=%u Flags=0x%x\n",
            d->u.MessageInterrupt.Level,
            d->u.MessageInterrupt.Vector,
            (ULONG_PTR)d->u.MessageInterrupt.Affinity,
            d->u.MessageInterrupt.MessageCount,
            d->Flags);
    } else {
        DbgPrintEx(DPFLTR_IHVDRIVER_ID, DPFLTR_INFO_LEVEL,
            "INT: INTx Level=%lu Vector=%lu Affinity=0x%Ix Flags=0x%x\n",
            d->u.Interrupt.Level,
            d->u.Interrupt.Vector,
            (ULONG_PTR)d->u.Interrupt.Affinity,
            d->Flags);
    }
}
```

What you’re looking for:

- If you think you’re using MSI-X but `CM_RESOURCE_INTERRUPT_MESSAGE` never appears, Windows gave you INTx.
- If `MessageCount` is 1 but you expected N queues + config, Windows only granted 1 message (often due to INF/registry policy or device capability limits).

#### 1.4 WinDbg: use `!irp` (PnP start IRP) and `!pci` (MSI-X enable bit)

##### `!irp` (resources as Windows handed them to the driver)

If you can break in your driver at `IRP_MJ_PNP / IRP_MN_START_DEVICE` (WDM) you can inspect the PnP IRP directly:

1. Break on your PnP dispatch routine (x64 calling convention: `rdx` is the IRP pointer).
2. In WinDbg:

```text
!irp @rdx
```

Look for pointers to `AllocatedResources` and `AllocatedResourcesTranslated`, then dump them:

```text
dt nt!_CM_RESOURCE_LIST <AllocatedResourcesTranslated>
```

Inside the resource list, find `CmResourceTypeInterrupt` descriptors and check their `Flags` for `CM_RESOURCE_INTERRUPT_MESSAGE` and any `MessageCount`.

If you’re KMDF-only and don’t have a PnP dispatch routine, the most practical equivalent is to log the resource descriptors from `EvtDevicePrepareHardware` (section 1.2) and use WinDbg to verify the PCI config (next section).

##### `!pci` (verify MSI-X capability and whether it is enabled)

WinDbg can show the device’s PCI capabilities, including whether MSI-X is *enabled*.

1. Determine the device’s BDF (Bus/Device/Function). Device Manager → **Location information** often shows something like “PCI bus X, device Y, function Z”.
2. In WinDbg:

```text
.load pci
!pcitree
!pci <bus> <device> <function>
```

In the `!pci` output, look for:

- An **MSI-X capability** (table size, table/BIR location).
- **MSI-X Enable** state.

If your driver believes it has MSI-X messages but `!pci` shows MSI-X disabled, something is inconsistent (often a driver that never successfully started/connected interrupts, or programming happening at the wrong time).

---

### Confirm MSI-X message count and message numbers actually connected

Windows “message interrupts” have two numbers that people often confuse:

- **MessageNumber** (what virtio wants): 0-based message ID / MSI-X table entry index.
- **Vector** (what the CPU sees): APIC/IDT vector used to deliver the interrupt.

For virtio, **you almost always want `MessageNumber`, not `Vector`.**

#### 2.1 Log what KMDF connected: `WdfInterruptGetInfo`

After each `WdfInterruptCreate`, call `WdfInterruptGetInfo` and print:

- `MessageSignaled`
- `MessageNumber`
- `Vector`
- `Irql`
- `Affinity`

Example:

```c
static void DumpWdfInterrupt(_In_ WDFINTERRUPT Interrupt)
{
    WDF_INTERRUPT_INFO info;
    WDF_INTERRUPT_INFO_INIT(&info);
    WdfInterruptGetInfo(Interrupt, &info);

    DbgPrintEx(DPFLTR_IHVDRIVER_ID, DPFLTR_INFO_LEVEL,
        "WDFINT: MessageSignaled=%u MessageNumber=%lu Vector=%lu Irql=%u Affinity=0x%Ix\n",
        info.MessageSignaled,
        info.MessageNumber,
        info.Vector,
        info.Irql,
        (ULONG_PTR)info.Affinity);
}
```

What “good” looks like:

- `MessageSignaled=1`
- You see message numbers `0..(N-1)` across your created interrupts (or at least the subset you requested/connected).
- `Vector` values will look “random”/unrelated (often high) — that’s expected.

#### 2.2 Cross-check with your ISR’s `MessageID`

KMDF’s ISR callback receives `MessageID`:

```c
BOOLEAN EvtInterruptIsr(WDFINTERRUPT Interrupt, ULONG MessageID);
```

For message-signaled interrupts, `MessageID` should match the interrupt’s `info.MessageNumber` (from `WdfInterruptGetInfo`). Logging this once (or sampling) is a good way to prove your ISR is running on the message you think it is.

---

### Virtio-specific MSI-X vector programming checks (the “read-back or it didn’t happen” rule)

Virtio-pci modern has explicit vector selectors:

- `common_cfg.msix_config` (u16): which MSI-X vector is used for **config change** interrupts.
- `common_cfg.queue_msix_vector` (u16, per-queue via `queue_select`): which MSI-X vector is used for **queue** interrupts.

These fields take an MSI-X **table entry index** (0-based). In Windows terms, that is typically the **MessageNumber**.

#### 3.1 Read back `common_cfg.msix_config`

After you decide which message will be the config interrupt:

1. Write `common_cfg.msix_config = <MessageNumber>`.
2. Read it back and log the value.

Expected:

- Read-back equals what you wrote.

If it reads back as `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`), see section 3.3.

#### 3.2 Read back each queue’s `queue_msix_vector`

For each queue you expect to generate interrupts:

1. `common_cfg.queue_select = <queue index>`
2. Write `common_cfg.queue_msix_vector = <MessageNumber>`
3. Read back and log.

Expected:

- Read-back equals what you wrote (or equals the shared vector if you intentionally share).

#### 3.3 What `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`) means (and when it’s expected)

`0xFFFF` is `VIRTIO_PCI_MSI_NO_VECTOR` (“no vector assigned”).

You should treat it as:

- **Expected** immediately after device reset (virtio resets vectors to “no vector”).
- **Expected** if Windows assigned you INTx and MSI-X is not enabled/usable.
- **A hard error** if you think you are in MSI-X mode and you just programmed a vector:
  - You wrote an out-of-range vector index (common when mistakenly using APIC `Vector`).
  - MSI-X isn’t enabled at the PCI layer.
  - The device has fewer MSI-X table entries than you assumed.

For queue interrupts specifically:

- **Virtio spec / QEMU-like behavior:** leaving `queue_msix_vector = VIRTIO_PCI_MSI_NO_VECTOR`
  (`0xFFFF`) disables MSI-X delivery for that queue (you’ll either poll or never see completions,
  depending on whether you have an INTx path wired).
- **Aero Win7 virtio contract behavior:** when MSI-X is enabled at the PCI layer, MSI-X is
  **exclusive**: if vectors are unassigned/unusable (`0xFFFF`, masked/unprogrammed entry, etc),
  interrupts for that source are **suppressed** (no MSI-X message and no INTx fallback). When
  MSI-X is disabled, devices deliver interrupts via **INTx + ISR** semantics (see
  `windows7-virtio-driver-contract.md` §1.8.4).

#### 3.4 Reset sequencing: MSI-X vectors must be re-programmed after *every* reset

Virtio feature negotiation typically involves a reset (`device_status = 0`) at least once during bring-up.

Common pitfall:

- Vectors are programmed once early, then a reset happens, vectors revert to
  `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`), and from that point onward **no MSI-X interrupts** ever
  fire. (On Aero contract devices, this typically means interrupts are suppressed until vectors
  are reprogrammed.)

Rule of thumb:

- Any code path that transitions the device through reset must also re-apply:
  - `msix_config`
  - every `queue_msix_vector` you rely on

…and verify via read-back.

---

### Common failure modes (symptoms → proof → fix)

#### 4.1 INTx storm because ISR status wasn’t read/cleared

**Symptoms**

- CPU pegged, system sluggish.
- ISR count increases extremely fast even when the device is “idle”.
- You see the ISR re-enter continuously (level-triggered line never deasserts).

**How to prove**

- Device Manager / driver resources show INTx (no `CM_RESOURCE_INTERRUPT_MESSAGE`).
- Your ISR never reads the virtio ISR status register (legacy ISR status capability).

**Fix**

- In INTx mode, always read the virtio ISR status register in the ISR as the *first* operation and treat it as the deassert/ack step.
- Avoid spamming `DbgPrint` from the ISR; use counters instead (section 5).

#### 4.2 No interrupts because MSI-X vectors weren’t programmed (or were lost after reset)

**Symptoms**

- Windows reports message interrupts (resources show `CM_RESOURCE_INTERRUPT_MESSAGE`, `WdfInterruptGetInfo.MessageSignaled=1`).
- Your queues make progress only if you poll; otherwise completions never trigger an ISR/DPC.
- The ISR count stays at 0 even while you know the device should interrupt.

**How to prove**

- Read back `common_cfg.msix_config` and/or `queue_msix_vector` and they are still
  `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`) after the device is “running”.
  - On Aero contract devices this means the device will **suppress interrupts** while MSI-X is
    enabled (no MSI-X message and no INTx fallback) until you successfully program MSI-X vectors.
- You can correlate this with a recent reset/status transition in your logs.

**Fix**

- Program `msix_config` and each `queue_msix_vector` *after* the last reset and before enabling/using the queues.
- Read back and assert they are not `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`).

#### 4.3 Wrong routing because the driver used APIC `Vector` instead of `MessageNumber`

**Symptoms**

- Driver logs show `Vector` values like `0xE1`, `0x93`, etc being written into virtio `msix_config` / `queue_msix_vector`.
- Read-back comes back `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`) (device rejected the index) or interrupts never arrive.

**How to prove**

- Your `WdfInterruptGetInfo` prints show something like:
  - `MessageNumber=0` (or small)
  - `Vector=0xE1` (large)
- Your virtio read-back shows `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`) after “programming”.

**Fix**

- Program virtio’s vector selectors with `MessageNumber` (MSI-X table entry index), not the CPU `Vector`.

---

### Recommended minimal instrumentation (what to log, and where)

Use logging that is actionable but won’t destroy the machine when things go wrong.
On Windows 7, **DbgView** is a practical way to capture `DbgPrintEx` output without kernel debugging attached:

- Run DbgView as Administrator
- Enable **Capture Kernel**

#### 5.1 At device start / `EvtDevicePrepareHardware`

Log once:

- Device identification (PCI location/BDF if you have it).
- Interrupt resources:
  - For each `CmResourceTypeInterrupt` descriptor:
    - whether `CM_RESOURCE_INTERRUPT_MESSAGE` is set
    - `MessageCount` (if message-based)
    - `Vector`, `Level`, `Affinity`

This answers: “Did Windows give me MSI-X or INTx, and how many messages?”

#### 5.2 After connecting interrupts (each `WdfInterruptCreate`)

Log once per interrupt object:

- `WdfInterruptGetInfo` fields: `MessageNumber`, `Vector`, `Irql`, `Affinity`, `MessageSignaled`

This answers: “Which message numbers did KMDF actually connect?”

#### 5.3 When programming virtio MSI-X vectors

For each write:

- What you wrote (`msix_config`, `queue_msix_vector`), and which message number you intended.
- The *read-back* value.

This answers: “Did the device accept my vector programming, or did it silently revert to `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`)?”

#### 5.4 In the ISR and DPC: counters, not prints

Keep per-interrupt counters:

- `isr_count[message]++`
- `dpc_count[message]++`

Log only:

- Periodically (e.g., once per second in a timer, or every 4096 interrupts), print a summary:
  - counts
  - last-seen `MessageID` (for message interrupts)
  - for INTx: last ISR status value read (optional, sample it)

This answers: “Are interrupts firing? Are DPCs scheduled? Is one vector storming?”

---

### Bring-up checklist (copy/paste)

1. **Device Manager → Resources**: verify whether you’re on MSI-X (message) or INTx.
2. Driver: dump translated interrupt resources; confirm `CM_RESOURCE_INTERRUPT_MESSAGE` and `MessageCount` if applicable.
3. Driver: for each `WDFINTERRUPT`, print `WdfInterruptGetInfo` and record **MessageNumber**.
4. WinDbg `!pci`: confirm MSI-X capability exists and MSI-X is enabled when you expect message interrupts.
5. Program virtio:
   - `common_cfg.msix_config = <MessageNumber>`
   - for each queue: `queue_msix_vector = <MessageNumber>`
6. Read back all virtio vector selectors:
   - if any are `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`) unexpectedly, fix before chasing anything else.
7. Verify ISR/DPC counters increment when generating known events (queue kick/completion).

## Windows 7 miniport guide: virtio-pci **modern** (PCI config access + BAR0 MMIO mapping)

This is a practical bring-up recipe for **virtio-pci modern** devices (Virtio 1.0+ “vendor capability” transport) in **Windows 7 miniports**:

* **NDIS 6.20** miniports (`MiniportInitializeEx` / `MiniportHaltEx`) — e.g. virtio-net
* **StorPort** miniports (`HwFindAdapter` / `HwInitialize`) — e.g. virtio-blk

It is intentionally **not KMDF/WDF**. The goal is to make it possible to implement:

1) reading a **256-byte PCI config snapshot** (for ID checks + vendor-cap parsing), and  
2) mapping **BAR0 MMIO** from the miniport’s resource list, and  
3) the **INTx ACK rule** for virtio (read-to-clear ISR byte).

### Shipping Win7 miniports: Win7 common miniport shim (`VirtioPciModernMiniport*`)

In this repo, the **shipping** Windows 7 miniport drivers:

* `drivers/windows7/virtio-net/` (NDIS 6.20 miniport)
* `drivers/windows7/virtio-blk/` (StorPort miniport)

use the Win7 common, WDF-free miniport shim:

* `drivers/windows7/virtio/common/include/virtio_pci_modern_miniport.h`
* `drivers/windows7/virtio/common/src/virtio_pci_modern_miniport.c`

The miniport shim is shaped for the NDIS/StorPort miniport model: the driver
does the OS-specific work up front, then passes two concrete inputs into the
shim:

* a **BAR0 MMIO mapping** (`Bar0Va` + `Bar0Length`), and
* a cached **256-byte PCI config snapshot** (for ID checks + vendor-cap parsing):
  * NDIS: `NdisMGetBusData(..., PCI_WHICHSPACE_CONFIG, cfg, 0, 256)`
  * StorPort: `StorPortGetBusData(..., PCIConfiguration, ..., cfg, 0, 256)`

The entry point is `VirtioPciModernMiniportInit`, which parses vendor
capabilities and fills a `VIRTIO_PCI_DEVICE` (COMMON/NOTIFY/ISR/DEVICE config
windows, plus helpers like `VirtioPciNegotiateFeatures`, `VirtioPciSetupQueue`,
`VirtioPciNotifyQueue`, and `VirtioPciReadIsr`).

Example:

```c
VIRTIO_PCI_DEVICE dev = {0};
NTSTATUS st = VirtioPciModernMiniportInit(&dev, bar0Va, bar0Len, cfg, sizeof(cfg));
if (!NT_SUCCESS(st)) return DEVICE_NOT_SUPPORTED;
```

This is what `drivers/windows7/virtio-net/src/aero_virtio_net.c` and
`drivers/windows7/virtio-blk/src/aero_virtio_blk.c` are wired against today.

### Alternative: generic OS-callback transport (`virtio_pci_modern_transport`)

This repo also contains a more generic, OS-callback based virtio-pci modern
transport implementation:

* `drivers/windows/virtio/pci-modern/virtio_pci_modern_transport.{c,h}`

It is not specific to miniports; you integrate it by implementing
`VIRTIO_PCI_MODERN_OS_INTERFACE` (PCI config reads, BAR mapping, stalls, and a
selector-serialization lock). This can be a better fit for other driver models
(WDM/KMDF) or codebases that want the transport layer to own BAR mapping.

The rest of this document explains the underlying mechanics (PCI config offsets,
BAR discovery/mapping, INTx ISR semantics) and is useful when wiring up either
transport or debugging contract failures.

Definitive contract for what Aero expects from virtio devices/drivers:

* [`../windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md)

---

### What “virtio-pci modern” means in practice

Virtio 1.0+ PCI devices expose four required memory-mapped regions via **PCI vendor-specific capabilities** (cap ID `0x09`):

* **COMMON** config (`COMMON_CFG`) — feature negotiation, queue setup, device status
* **NOTIFY** config (`NOTIFY_CFG`) — where to write queue notifications
* **ISR** config (`ISR_CFG`) — a 1-byte, **read-to-clear** interrupt status register
* **DEVICE** config (`DEVICE_CFG`) — device-specific config struct (net/blk/etc)

On Windows 7, your miniport must:

* read PCI config space to find those vendor caps (or feed a parser that does),
* map the BAR(s) that contain those regions (this doc focuses on **BAR0**),
* treat the mapped BAR as **MMIO** (use `READ_REGISTER_*` / `WRITE_REGISTER_*`),
* for **INTx**, read the ISR byte in the interrupt routine to deassert the line.

---

### Common device identity checks (config space offsets)

Before attempting to parse capabilities or touch MMIO, validate the device identity from PCI config.

For Aero, the **PCI Revision ID encodes the virtio contract major version** (not “modern vs legacy” in the general virtio sense). Contract v1 uses `REV_01`.

* Vendor ID: `0x1AF4`
* Device ID:
  * `0x1041` = virtio-net
  * `0x1042` = virtio-blk
* **Revision ID**: `0x01` = `AERO-W7-VIRTIO` contract v1

Pseudocode helpers (avoid unaligned loads):

```c
static USHORT ReadLe16(const UCHAR* p) { return (USHORT)p[0] | ((USHORT)p[1] << 8); }
static ULONG  ReadLe32(const UCHAR* p) { return (ULONG)p[0] | ((ULONG)p[1] << 8) | ((ULONG)p[2] << 16) | ((ULONG)p[3] << 24); }

// Standard PCI config offsets
#define PCI_CFG_VENDOR_ID   0x00
#define PCI_CFG_DEVICE_ID   0x02
#define PCI_CFG_REVISION_ID 0x08
#define PCI_CFG_BAR0        0x10
```

Validation:

```c
USHORT vendor = ReadLe16(&cfg[PCI_CFG_VENDOR_ID]);
USHORT device = ReadLe16(&cfg[PCI_CFG_DEVICE_ID]);
UCHAR  rev    = cfg[PCI_CFG_REVISION_ID];

if (vendor != 0x1AF4) return DEVICE_NOT_SUPPORTED;
if (device != 0x1041 && device != 0x1042) return DEVICE_NOT_SUPPORTED;
if (rev != 0x01) return DEVICE_NOT_SUPPORTED; // not an AERO-W7-VIRTIO v1 device
```

---

### NDIS 6.20 miniport (`MiniportInitializeEx`): PCI config + BAR0 mapping

NDIS gives you the translated PnP resources in:

```c
PNDIS_RESOURCE_LIST res = MiniportInitParameters->AllocatedResources;
```

#### 2.1) Read a 256-byte PCI config snapshot (`NdisMGetBusData`)

For a PCI miniport (`Reg.InterfaceType = NdisInterfacePci`), you can read PCI config space with `NdisMGetBusData`.

Inputs you need:

* `MiniportAdapterHandle` (argument to `MiniportInitializeEx`)
* `WhichSpace = PCI_WHICHSPACE_CONFIG` (typically `0`)
* `Offset = 0`
* `Length = 256`

WinDDK 7600-compatible pseudocode:

```c
#ifndef PCI_WHICHSPACE_CONFIG
#define PCI_WHICHSPACE_CONFIG 0
#endif

UCHAR cfg[256];
RtlZeroMemory(cfg, sizeof(cfg));

ULONG bytesRead = NdisMGetBusData(
    MiniportAdapterHandle,
    PCI_WHICHSPACE_CONFIG,
    /*Buffer=*/cfg,
    /*Offset=*/0,
    /*Length=*/sizeof(cfg));

if (bytesRead != sizeof(cfg)) {
    return NDIS_STATUS_DEVICE_FAILED;
}
```

> If you later need to *write* config registers (rare for virtio modern), the symmetric API is `NdisMSetBusData`.

#### 2.2) Locate BAR0 as `CmResourceTypeMemory` / `CmResourceTypeMemoryLarge` in `PNDIS_RESOURCE_LIST`

`AllocatedResources` is a `PNDIS_RESOURCE_LIST` (NDIS typedef over `CM_PARTIAL_RESOURCE_LIST`).
Scan its `PartialDescriptors[]` for memory resources.

Most virtio-pci modern devices expose the main MMIO BAR as a `CmResourceTypeMemory` range.

On some **x64** systems, PCI MMIO ranges (especially BARs mapped **above 4 GiB**) can be reported as `CmResourceTypeMemoryLarge` instead. In that case, you must:

1. Select the correct union member (`u.Memory40/48/64`) based on `Flags` (`CM_RESOURCE_MEMORY_LARGE_40/48/64`), and
2. Decode the length back to bytes (the `Length40/48/64` fields are scaled units in the WDK).

Pseudocode (WinDDK 7600 compatible):

```c
typedef struct _ADAPTER {
    NDIS_HANDLE MiniportAdapterHandle;
    NDIS_PHYSICAL_ADDRESS Bar0Pa;
    ULONG Bar0Len;
    PUCHAR Bar0Va; // mapped VA for BAR0 MMIO
} ADAPTER;

static BOOLEAN GetMemoryRangeFromDescriptor(
    _In_  const CM_PARTIAL_RESOURCE_DESCRIPTOR* d,
    _Out_ NDIS_PHYSICAL_ADDRESS* OutPa,
    _Out_ ULONG* OutLen)
{
    if (d == NULL || OutPa == NULL || OutLen == NULL) {
        return FALSE;
    }

    if (d->Type == CmResourceTypeMemory) {
        *OutPa = d->u.Memory.Start;
        *OutLen = d->u.Memory.Length;
        return TRUE;
    }

    if (d->Type == CmResourceTypeMemoryLarge) {
        ULONGLONG lenBytes = 0;
        USHORT large = d->Flags & (CM_RESOURCE_MEMORY_LARGE_40 | CM_RESOURCE_MEMORY_LARGE_48 | CM_RESOURCE_MEMORY_LARGE_64);

        switch (large) {
            case CM_RESOURCE_MEMORY_LARGE_40:
                *OutPa = d->u.Memory40.Start;
                lenBytes = ((ULONGLONG)d->u.Memory40.Length40) << 8;   // 256B units
                break;
            case CM_RESOURCE_MEMORY_LARGE_48:
                *OutPa = d->u.Memory48.Start;
                lenBytes = ((ULONGLONG)d->u.Memory48.Length48) << 16;  // 64KiB units
                break;
            case CM_RESOURCE_MEMORY_LARGE_64:
                *OutPa = d->u.Memory64.Start;
                lenBytes = ((ULONGLONG)d->u.Memory64.Length64) << 32;  // 4GiB units
                break;
            default:
                return FALSE;
        }

        // NDIS maps an ULONG length; reject descriptors that decode beyond that.
        if (lenBytes > 0xFFFFFFFFull) {
            return FALSE;
        }

        *OutLen = (ULONG)lenBytes;
        return TRUE;
    }

    return FALSE;
}

static BOOLEAN GetMemoryRangeFromNdisResources(
    _In_  PNDIS_RESOURCE_LIST Resources,
    _Out_ NDIS_PHYSICAL_ADDRESS* OutPa,
    _Out_ ULONG* OutLen)
{
    ULONG i;
    for (i = 0; i < Resources->Count; i++) {
        PCM_PARTIAL_RESOURCE_DESCRIPTOR d = &Resources->PartialDescriptors[i];
        if (GetMemoryRangeFromDescriptor(d, OutPa, OutLen)) {
            return TRUE;
        }
    }
    return FALSE;
}
```

**Which memory descriptor is BAR0?**

* If your device has only one MMIO BAR, “first `CmResourceTypeMemory` wins” is usually enough.
* If there are multiple memory ranges, the robust approach is:
  1. read BAR0 from config space (offset `0x10`),
  2. mask off the low flag bits,
  3. pick the resource descriptor whose `Start` matches.

BAR0 masking pseudocode (32-bit memory BAR case):

```c
ULONG bar0_lo = ReadLe32(&cfg[PCI_CFG_BAR0]);
if (bar0_lo & 0x1) return DEVICE_HAS_IO_BAR0_NOT_MMIO;

ULONGLONG bar0_pa = (ULONGLONG)(bar0_lo & ~0xFULL); // 16-byte aligned for mem BARs
```

> If BAR0 is 64-bit (`(bar0_lo & 0x6) == 0x4`), BAR0 consumes BAR0+BAR1 and you must combine the high DWORD as well.

#### 2.3) Map BAR0 MMIO with `NdisMMapIoSpace` (and unmap in `MiniportHaltEx`)

Once you have `(Bar0Pa, Bar0Len)` from the resource list, map it:

```c
NDIS_STATUS status = NdisMMapIoSpace(
    (PVOID*)&Adapter->Bar0Va,
    Adapter->MiniportAdapterHandle,
    Adapter->Bar0Pa,
    Adapter->Bar0Len);

if (status != NDIS_STATUS_SUCCESS) {
    Adapter->Bar0Va = NULL;
    Adapter->Bar0Len = 0;
    return status;
}
```

Cleanup in `MiniportHaltEx`:

```c
if (Adapter->Bar0Va != NULL) {
    NdisMUnmapIoSpace(
        Adapter->MiniportAdapterHandle,
        Adapter->Bar0Va,
        Adapter->Bar0Len);
    Adapter->Bar0Va = NULL;
    Adapter->Bar0Len = 0;
}
```

At this point you have an MMIO base pointer:

```c
volatile UCHAR* bar0 = (volatile UCHAR*)Adapter->Bar0Va;
```

Do **not** use `READ_PORT_*` / `WRITE_PORT_*` on it.

---

### StorPort miniport (`HwFindAdapter`/`HwInitialize`): PCI config + BAR0 mapping

StorPort provides translated BAR information via:

```c
PPORT_CONFIGURATION_INFORMATION configInfo;
PACCESS_RANGE ranges = configInfo->AccessRanges;
```

#### 3.1) Map BAR0 MMIO from `AccessRanges[]` (`StorPortGetDeviceBase`)

For virtio modern, you want an **MMIO** access range:

* `range->RangeInMemory` must be `TRUE`
* map it with `InIoSpace = FALSE`

Pseudocode:

```c
PACCESS_RANGE bar0 = NULL;
ULONG i;

for (i = 0; i < configInfo->NumberOfAccessRanges; i++) {
    if (configInfo->AccessRanges[i].RangeLength == 0) continue;
    if (configInfo->AccessRanges[i].RangeInMemory) {
        bar0 = &configInfo->AccessRanges[i];
        break;
    }
}

if (bar0 == NULL) return SP_RETURN_NOT_FOUND;

PVOID bar0Va = StorPortGetDeviceBase(
    DeviceExtension,
    configInfo->AdapterInterfaceType,   // typically PCIBus
    configInfo->SystemIoBusNumber,
    bar0->RangeStart,
    bar0->RangeLength,
    /*InIoSpace=*/FALSE);               // FALSE = memory-mapped

if (bar0Va == NULL) return SP_RETURN_NOT_FOUND;
```

Save:

* `bar0->RangeStart` (physical)
* `bar0->RangeLength`
* `bar0Va` (virtual)

#### 3.2) Read PCI config space (`busInformation` and/or `StorPortGetBusData`)

In `HwFindAdapter`, StorPort passes a `busInformation` pointer which, for PCI, commonly points at a `PCI_COMMON_CONFIG` snapshot.

Two practical patterns:

##### Pattern A (fast path): use `busInformation` when it’s a full header

```c
PPCI_COMMON_CONFIG pci = (PPCI_COMMON_CONFIG)busInformation;
if (pci != NULL) {
    USHORT vendor = pci->VendorID;
    USHORT device = pci->DeviceID;
    UCHAR  rev    = pci->RevisionID;
    // ... validate IDs ...
}
```

This is enough for Vendor/Device/Revision checks, but it is not guaranteed to contain the full 256 bytes needed for capability parsing.

##### Pattern B (robust): always fetch 256 bytes with `StorPortGetBusData`

WinDDK 7600-compatible pseudocode:

```c
UCHAR cfg[256];
RtlZeroMemory(cfg, sizeof(cfg));

ULONG bytesRead = StorPortGetBusData(
    DeviceExtension,
    /*BusDataType=*/PCIConfiguration,
    /*SystemIoBusNumber=*/configInfo->SystemIoBusNumber,
    /*SlotNumber=*/configInfo->SlotNumber,
    /*Buffer=*/cfg,
    /*Offset=*/0,
    /*Length=*/sizeof(cfg));

if (bytesRead != sizeof(cfg)) return SP_RETURN_NOT_FOUND;
```

> Some WDKs expose `StorPortGetBusData` without an explicit `Offset` parameter (mirroring `HalGetBusData` vs `HalGetBusDataByOffset`).
> The intent is the same: you must obtain a stable 256-byte config snapshot for vendor-cap parsing.

---

### Parsing virtio vendor capabilities (COMMON/NOTIFY/ISR/DEVICE)

Once you have:

* `cfg[256]` — PCI config snapshot
* BAR base addresses (at minimum BAR0’s physical base), and
* BAR0 mapped as MMIO (`bar0Va`)

…you can parse virtio vendor caps and derive register pointers.

#### 4.1) Use the portable parser (recommended)

This repo includes a portable vendor-cap parser (C99, tested outside Windows):

* `drivers/win7/virtio/virtio-core/portable/virtio_pci_cap_parser.{h,c}`

The key API is:

```c
virtio_pci_cap_parse_result_t virtio_pci_cap_parse(
    const uint8_t *cfg_space,
    size_t cfg_space_len,
    const uint64_t bar_addrs[6],
    virtio_pci_parsed_caps_t *out_caps);
```

To feed it from a miniport, populate `bar_addrs[]` from your resource list.
If you only mapped BAR0, you can at least provide BAR0’s physical base:

```c
uint64_t bar_addrs[6] = {0};
bar_addrs[0] = (uint64_t)Bar0Pa.QuadPart; // NDIS: Adapter->Bar0Pa; StorPort: bar0->RangeStart

virtio_pci_parsed_caps_t parsed;
virtio_pci_cap_parse_result_t r = virtio_pci_cap_parse(cfg, sizeof(cfg), bar_addrs, &parsed);
if (r != VIRTIO_PCI_CAP_PARSE_OK) return DEVICE_FAILED;
```

If the parser reports that a required capability lives in a BAR other than 0, you must:

1. map that BAR as well (same technique as BAR0), and
2. use the correct `bar_addrs[bar]` and `bar_va[bar]` when forming pointers.

#### 4.2) Turn parsed offsets into mapped pointers (BAR0 case)

With BAR0 mapped at `bar0Va`:

```c
volatile UCHAR* bar0 = (volatile UCHAR*)bar0Va;

volatile struct virtio_pci_common_cfg* common =
    (volatile struct virtio_pci_common_cfg*)(bar0 + parsed.common_cfg.offset);

volatile UCHAR* isr_status =
    (volatile UCHAR*)(bar0 + parsed.isr_cfg.offset); // 1 byte, read-to-clear

volatile UCHAR* device_cfg =
    (volatile UCHAR*)(bar0 + parsed.device_cfg.offset);

volatile UCHAR* notify_base =
    (volatile UCHAR*)(bar0 + parsed.notify_cfg.offset);

ULONG notify_off_multiplier = parsed.notify_off_multiplier;
```

And when notifying a queue:

```c
// queue_notify_off is read from common_cfg after selecting the queue.
USHORT queue_notify_off = READ_REGISTER_USHORT(&common->queue_notify_off);

volatile USHORT* notify_addr =
    (volatile USHORT*)(notify_base + ((ULONG)queue_notify_off * notify_off_multiplier));

// Modern virtio-pci uses a 16-bit MMIO write; the value is the queue index.
WRITE_REGISTER_USHORT(notify_addr, QueueIndex);
```

---

### MMIO access rules (Win7)

For virtio-pci modern, the capability regions are **MMIO**, not port I/O.

Use:

* `READ_REGISTER_UCHAR/USHORT/ULONG`
* `WRITE_REGISTER_UCHAR/USHORT/ULONG`

Do **not** use:

* `READ_PORT_*` / `WRITE_PORT_*`

Rationale: the mapping returned by `NdisMMapIoSpace` / `StorPortGetDeviceBase(..., InIoSpace=FALSE)` is memory-mapped and must obey MMIO semantics.

---

### INTx interrupt ACK/deassert rule: read ISR status byte

Virtio’s legacy **INTx** line is **level-triggered**. The device deasserts the line only when the driver reads the 1-byte **ISR status** register (`ISR_CFG` capability).

Therefore:

1. In your interrupt routine, do a **1-byte MMIO read** of `isr_status`.
2. If it reads `0`, it wasn’t your interrupt (important on shared lines); return “not handled”.
3. Otherwise, you have ACKed/deasserted the line; schedule your DPC/notification and handle the bits.

Pseudocode (generic):

```c
// Runs at DIRQL in both NDIS and StorPort.
BOOLEAN VirtioIntxIsr(...)
{
    UCHAR isr = READ_REGISTER_UCHAR(isr_status); // read-to-clear + deassert
    if (isr == 0) {
        return FALSE; // shared interrupt, not ours
    }

    // Save isr for DPC because the register is now cleared.
    Adapter->PendingIsrStatus |= isr;

    QueueDpcOrDeferredWork();
    return TRUE;
}
```

* bit 0 (`0x01`): queue interrupt
* bit 1 (`0x02`): device config change interrupt

With **MSI-X**, there is no line to deassert; do not rely on `isr_status` reads for ACK.

---

### Minimal bring-up checklist (miniport)

1. Read 256 bytes of PCI config space.
2. Verify `(VendorID, DeviceID, RevisionID) == (0x1AF4, 0x1041/0x1042, 0x01)`.
3. Find BAR0 as a translated **memory** resource and map it as **MMIO**.
4. Parse virtio vendor caps (portable parser recommended) and compute MMIO pointers.
5. Access MMIO with `READ_REGISTER_*` / `WRITE_REGISTER_*`.
6. For INTx, **read ISR byte in the ISR** to ACK/deassert.

## Windows 7 `virtio-snd` PortCls + WaveRT driver design

This document is a **clean-room design reference** for a minimal Windows 7 audio driver for the repo’s [`virtio-snd`](../areas/windows-drivers.md) device model. It describes the **PortCls + WaveRT** surface (COM interfaces, KS descriptors, properties, and INF) required to enumerate functional render + capture endpoints in Windows 7.

The goal is to make the first bring-up deterministic: if the driver matches the shapes described here, Windows 7 should create “Speakers” (render) and “Microphone” (capture) endpoints and the audio engine should be able to transition streams to `RUN`.

Note: the current in-tree Windows 7 virtio-snd driver (`drivers/windows7/virtio-snd/`) supports both **render**
and **capture** streams per `AERO-W7-VIRTIO` v1 (stream id 0 via `txq`, stream id 1 via `rxq`). When a virtio-snd
implementation advertises a **superset** of capabilities in `PCM_INFO`, the driver can optionally expose additional
formats/rates/channel counts to Windows via dynamic WaveRT pin data ranges (while still requiring and preferring the
contract-v1 baseline).

### Scope / assumptions (minimum viable endpoint)

* **Windows target:** Windows 7 SP1 (x86/x64). The driver can be built with a Win7-era WinDDK (7600) layout or with newer WDKs (CI uses WDK10/MSBuild).
* **Device model:** `virtio-snd` PCI function (`PCI\VEN_1AF4&DEV_1059&REV_01`; Aero `AERO-W7-VIRTIO` v1).
* **Audio direction:** render + capture.
* **Streams:** contract v1 defines **2** virtio-snd streams:
  * Stream 0: playback/output (render)
  * Stream 1: capture/input (capture)
* **Format:** contract v1 baseline PCM:
  * render: **stereo (2ch), 48 kHz, signed 16-bit LE PCM (S16_LE)**
  * capture: **mono (1ch), 48 kHz, signed 16-bit LE PCM (S16_LE)**
  * *(Optional/non-contract)*: additional formats/rates/channels may be exposed when a device advertises them via
    `PCM_INFO`.
* **Mixing:** Windows audio engine (`audiodg.exe`) does mixing; the miniport is a single shared-mode endpoint.
* **Virtio transport:** virtio-pci **modern-only** (PCI vendor-specific capabilities + BAR0 MMIO).
* **Virtio feature bits:** `VIRTIO_F_VERSION_1` + `VIRTIO_F_RING_INDIRECT_DESC` only.
* **Virtqueues (contract v1):** `controlq=64`, `eventq=64`, `txq=256`, `rxq=64`.
* **Interrupts:** PCI **INTx** baseline (required by `AERO-W7-VIRTIO` v1). MSI/MSI-X is an optional enhancement; the in-tree driver prefers message interrupts when Windows grants them (INF opt-in), programs virtio MSI-X routing (`msix_config`, `queue_msix_vector`), and falls back to INTx if message interrupts cannot be used/programmed.

Authoritative virtio contract:

* [`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) (AERO-W7-VIRTIO v1)

Non-goals:

* Multiple endpoints per direction, sample rate conversion, offload, jack sensing, power management beyond “works”.

---

### Architecture overview

#### Windows audio stack (where this driver sits)

At a high level, the stack for a PCI `virtio-snd` endpoint looks like:

```
User apps (WASAPI / DirectSound / MME)
  ↓
Windows Audio Engine (audiodg.exe) + AudioSrv
  ↓
KS proxies (wdmaud.sys / sysaudio.sys / ks.sys)
  ↓
PortCls port driver (portcls.sys)
  ↓
Our adapter driver (virtio-snd PCI function driver)
  ↓
Miniports inside our driver:
  - WaveRT miniport (streaming render + capture pins)
  - Topology miniport (bridge pin + controls)
  ↓
virtio-pci transport + virtqueues
  ↓
Emulator device model (`../areas/windows-drivers.md`) → browser audio backend
```

#### Driver-internal decomposition

The **PortCls adapter driver** is a single `.sys` that:

1. Binds to the `virtio-snd` PCI function (PnP start/stop).
2. Initializes the virtio transport (BAR mapping, virtqueues, interrupts (MSI/MSI-X preferred with INTx fallback) or polling-only in bring-up mode).
3. Registers two PortCls subdevices:
   * `Wave` (WaveRT streaming filter factory)
   * `Topology` (topology filter factory)
4. Routes WaveRT stream operations into a **software-DMA loop** that submits PCM to the virtio-snd TX virtqueue (render) and posts buffers to the RX virtqueue (capture).

Most implementations share hardware state via an “adapter common” object referenced by both miniports:

```
           +---------------------+
           | AdapterCommon       |
           | - virtio state      |
           | - stream state      |
           | - DMA timer/DPC     |
           +----------+----------+
                      ^
      +---------------+---------------+
      |                               |
+-----+----------------+  +-----------+-----------+
| WaveRT miniport      |  | Topology miniport     |
| (IMiniportWaveRT)    |  | (IMiniportTopology)   |
+----------------------+  +-----------------------+
```

---

### Required COM interfaces and responsibilities (Win7 bring-up)

PortCls miniports are COM-style objects (kernel-mode, `IUnknown`-like). For Windows 7 WaveRT render/capture bring-up, the following interfaces are required.

#### 2.1 Base: `IUnknown`

All miniport and stream objects must implement:

* `QueryInterface`
* `AddRef`
* `Release`

Notes:

* Avoid pageable code in any method that can be called at `DISPATCH_LEVEL`.
* Keep lifetime rules simple: the port owns the miniport; the miniport owns streams; streams hold a ref to the adapter common.

#### 2.2 Base: `IMiniport`

Both the WaveRT and topology miniports implement `IMiniport` as a base.

**Win7-critical responsibilities:**

* **`Init`**: capture the `UnknownAdapter`/resource list, create or reference the shared adapter-common object, and store the port interface pointer (`IPortWaveRT` or `IPortTopology`).
* **`GetDescription`**: return the **PortCls/KS descriptor** for the subdevice (pins, nodes, connections, categories).
* **`DataRangeIntersection`**: answer `KSPROPERTY_PIN_DATAINTERSECTION` for the streaming pin (format negotiation).
  * For a fixed-format endpoint, this can be a strict matcher.
  * When exposing multiple formats/rates/channels, it typically returns a `KSDATAFORMAT_WAVEFORMATEXTENSIBLE`
    corresponding to the selected `KSDATARANGE_AUDIO` (and validates the request is compatible with it).

Why this matters: Windows uses `DataRangeIntersection` aggressively during endpoint construction. Returning “close enough” formats tends to cause user-mode format failures later (or silent format conversion you didn’t plan for).

#### 2.3 Wave: `IMiniportWaveRT`

The WaveRT miniport owns the streaming pin implementation.

**Win7 bring-up surface (must work):**

* **Stream creation (`NewStream`)**
  * Validate pin id (render or capture).
  * Validate data format (baseline fixed PCM formats, or a dynamically generated supported-format table).
  * Create and return an `IMiniportWaveRTStream` instance.
* **Hardware/stream capabilities**
  * Report that the stream is **render** or **capture** depending on pin id.
  * Provide consistent buffer alignment requirements (typically frame-aligned).

**Common implementation pattern:**

* `NewStream` creates a stream object configured with:
  * `FramesPerSecond` (sample rate)
  * `BlockAlign` (channels * bytes_per_sample)
  * ring buffer size in frames/bytes
  * notification “period” in frames

#### 2.4 Stream: `IMiniportWaveRTStream`

This is the most sensitive part of WaveRT. The Windows 7 audio engine expects the stream object to implement a coherent model of:

* **Buffer provisioning / mapping**
  * Provide (or coordinate) a cyclic DMA buffer that the audio engine can write into (render) or read from (capture).
  * For WaveRT this is typically a **locked kernel buffer** that PortCls maps into user mode.
* **Position reporting**
  * Report “play position” in bytes/frames monotonically while running.
  * Do not jump backwards except on STOP/RESET.
* **Notification event**
  * Accept a kernel event object from PortCls/user-mode and signal it at each period boundary.
  * If notifications do not fire, shared-mode audio often never starts (or glitches heavily).
* **State transitions**
  * Handle `KSSTATE_STOP`, `KSSTATE_ACQUIRE`, `KSSTATE_PAUSE`, `KSSTATE_RUN` transitions.
  * Bring-up ordering that tends to work:
    1. `STOP → ACQUIRE`: allocate buffer + init counters
    2. `ACQUIRE → PAUSE`: arm timers, prime virtio stream (`PREPARE`)
    3. `PAUSE → RUN`: send `START`, start periodic DPC submissions, begin advancing play cursor
    4. any `* → STOP`: stop DPC, send `STOP`/`RELEASE`, release resources

The exact method names in WDK vary slightly by WaveRT revision, but the above responsibilities map to:

* stream `SetState`
* stream `GetPosition` / “presentation position”
* stream “register notification event”
* stream “allocate/free buffer” (or callback invoked by the WaveRT port during `KSPROPERTY_RTAUDIO_BUFFER`)

#### 2.5 Topology: `IMiniportTopology`

Topology miniport is “controls + wiring” for the endpoint. For a minimal fixed-format endpoint (render + capture) it can be mostly static.

**Win7 bring-up surface (must work):**

* Expose a topology filter descriptor with:
  * a **render bridge pin** to connect to the wave filter
  * a **capture bridge pin** to connect to the wave filter
  * a **speaker node** and optionally a **microphone node** (recommended)
* Implement (or allow PortCls to implement) minimal properties:
  * `KSPROPERTY_AUDIO_CHANNEL_CONFIG` (report stereo on render, mono on capture)
  * optionally `KSPROPSETID_Jack` stubs (fixed “connected”)

---

### KS filter/pin/node descriptors and categories (concrete sketch)

The driver registers **two KS filter factories** via PortCls:

* WaveRT filter factory (streaming)
* Topology filter factory (controls/graph)

The descriptors below are a “shape reference”; the exact C structures are in WDK (`portcls.h`, `ks.h`, `ksmedia.h`). The important part is that the pin ids, categories, and data ranges are consistent.

#### 3.1 WaveRT filter (render + capture)

**Filter categories (must include):**

* `KSCATEGORY_AUDIO` — tells SysAudio/WDMAud “this is audio”.
* `KSCATEGORY_RENDER` — tells Windows this factory provides a render endpoint.
* `KSCATEGORY_CAPTURE` — tells Windows this factory provides a capture endpoint.
* `KSCATEGORY_REALTIME` — strongly recommended for WaveRT (low-latency path and endpoint heuristics).

**Pins:**

| Pin ID | Name (suggested) | Role | Dataflow | Communication | Exposed to user mode |
|-------:|------------------|------|----------|---------------|----------------------|
| 0 | `Render` | streaming render pin | `KSPIN_DATAFLOW_IN` | `KSPIN_COMMUNICATION_SINK` | yes (apps open this) |
| 1 | `Bridge` | render connection to topology filter | (usually `OUT`) | `KSPIN_COMMUNICATION_BRIDGE` | no |
| 2 | `Capture` | streaming capture pin | `KSPIN_DATAFLOW_OUT` | `KSPIN_COMMUNICATION_SOURCE` | yes (apps open this) |
| 3 | `BridgeCapture` | capture connection to topology filter | (usually `IN`) | `KSPIN_COMMUNICATION_BRIDGE` | no |

**Streaming pin data ranges:**

* MajorFormat: `KSDATAFORMAT_TYPE_AUDIO`
* SubFormat: `KSDATAFORMAT_SUBTYPE_PCM`
* Specifier: `KSDATAFORMAT_SPECIFIER_WAVEFORMATEX` (but return `WAVEFORMATEXTENSIBLE` from intersection)

For a fixed-format endpoint, `KSDATARANGE_AUDIO` should constrain:

* `MaximumChannels = MinimumChannels = 2`
* `MinimumBitsPerSample = MaximumBitsPerSample = 16`
* `MinimumSampleFrequency = MaximumSampleFrequency = 48000`

Capture pin data range is identical except:

* `MaximumChannels = MinimumChannels = 1`

**Why fixed ranges instead of “wildcards”:** the simplest stable bring-up is to avoid Windows picking unexpected formats.
Once stable, widen supported ranges deliberately (for example by generating pin data ranges from `PCM_INFO`).

##### WaveRT filter descriptor sketch (pseudo-C)

This is a concrete “shape” of the descriptor data that `IMiniport::GetDescription` should return for the WaveRT subdevice:

```c
// Filter categories (factory-level)
static const GUID* const kWaveCategories[] = {
    &KSCATEGORY_AUDIO,
    &KSCATEGORY_RENDER,
    &KSCATEGORY_CAPTURE,
    &KSCATEGORY_REALTIME,
};

// Supported streaming formats for pin 0 ("Render") and pin 2 ("Capture")
static const KSDATARANGE_AUDIO kRenderDataRanges[] = {
    {
        .DataRange = {
            .FormatSize = sizeof(KSDATARANGE_AUDIO),
            .Flags = 0,
            .SampleSize = 0,
            .MajorFormat = KSDATAFORMAT_TYPE_AUDIO,
            .SubFormat = KSDATAFORMAT_SUBTYPE_PCM,
            .Specifier = KSDATAFORMAT_SPECIFIER_WAVEFORMATEX,
        },
        .MaximumChannels = 2,
        .MinimumChannels = 2,
        .MaximumBitsPerSample = 16,
        .MinimumBitsPerSample = 16,
        .MaximumSampleFrequency = 48000,
        .MinimumSampleFrequency = 48000,
    },
};

static const KSDATARANGE_AUDIO kCaptureDataRanges[] = {
    {
        .DataRange = {
            .FormatSize = sizeof(KSDATARANGE_AUDIO),
            .Flags = 0,
            .SampleSize = 0,
            .MajorFormat = KSDATAFORMAT_TYPE_AUDIO,
            .SubFormat = KSDATAFORMAT_SUBTYPE_PCM,
            .Specifier = KSDATAFORMAT_SPECIFIER_WAVEFORMATEX,
        },
        .MaximumChannels = 1,
        .MinimumChannels = 1,
        .MaximumBitsPerSample = 16,
        .MinimumBitsPerSample = 16,
        .MaximumSampleFrequency = 48000,
        .MinimumSampleFrequency = 48000,
    },
};

// Pins: 0 = Render, 1 = Bridge, 2 = Capture, 3 = BridgeCapture
static const PCPIN_DESCRIPTOR kWavePins[] = {
    // Pin 0: render streaming pin (user visible)
    {
        .DataFlow = KSPIN_DATAFLOW_IN,
        .Communication = KSPIN_COMMUNICATION_SINK,
        .Category = &KSCATEGORY_AUDIO, // some drivers also set a pin category
        .Name = NULL,
        .DataRanges = (PKSDATARANGE)kRenderDataRanges,
        .DataRangesCount = ARRAYSIZE(kRenderDataRanges),
        .InstancesPossible = 1,
        .InstancesNecessary = 1,
    },
    // Pin 1: bridge pin (internal link to topology)
    {
        .DataFlow = KSPIN_DATAFLOW_OUT,
        .Communication = KSPIN_COMMUNICATION_BRIDGE,
        .Category = NULL,
        .Name = NULL,
        .DataRanges = NULL,
        .DataRangesCount = 0,
        .InstancesPossible = 1,
        .InstancesNecessary = 1,
    },
    // Pin 2: capture streaming pin (user visible)
    {
        .DataFlow = KSPIN_DATAFLOW_OUT,
        .Communication = KSPIN_COMMUNICATION_SOURCE,
        .Category = &KSCATEGORY_AUDIO,
        .Name = NULL,
        .DataRanges = (PKSDATARANGE)kCaptureDataRanges,
        .DataRangesCount = ARRAYSIZE(kCaptureDataRanges),
        .InstancesPossible = 1,
        .InstancesNecessary = 1,
    },
    // Pin 3: bridge pin for capture (internal link to topology)
    {
        .DataFlow = KSPIN_DATAFLOW_IN,
        .Communication = KSPIN_COMMUNICATION_BRIDGE,
        .Category = NULL,
        .Name = NULL,
        .DataRanges = NULL,
        .DataRangesCount = 0,
        .InstancesPossible = 1,
        .InstancesNecessary = 1,
    },
};

static const PCFILTER_DESCRIPTOR kWaveFilterDescriptor = {
    .Version = 1,
    .Flags = 0,
    .PinCount = ARRAYSIZE(kWavePins),
    .PinDescriptor = kWavePins,
    .CategoryCount = ARRAYSIZE(kWaveCategories),
    .Category = kWaveCategories,
    // .AutomationTable = ... (see section 4)
};
```

Exact field names differ between `PCPIN_DESCRIPTOR` variants (`PCPIN_DESCRIPTOR`, `PCPIN_DESCRIPTOR_EX`, etc.). The important part is: pin 0 advertises only the one PCM data range, and pin ids are stable.

#### 3.2 Topology filter (render endpoint wiring)

**Filter categories (must include):**

* `KSCATEGORY_TOPOLOGY`

**Pins and nodes (minimal recommended):**

| Pin ID | Name (suggested) | Role |
|-------:|------------------|------|
| 0 | `BridgeIn` | bridge pin to WaveRT filter bridge pin |
| 1 | `SpeakerOut` (optional but recommended) | “physical” output connector |

**Nodes (optional but recommended):**

* `KSNODETYPE_SPEAKER` — lets Windows label the endpoint as “Speakers” and enables standard speaker properties.

**Connections (example):**

* `BridgeIn` → `Speaker` node → `SpeakerOut`

If you omit `SpeakerOut`, keep at least:

* a bridge pin, and
* a speaker node connected to the bridge

In practice, “bridge-only topology” is fragile across Windows versions and tools; the extra output connector pin tends to make SysAudio’s graph building less surprising.

##### Topology filter descriptor sketch (pseudo-C)

```c
static const GUID* const kTopoCategories[] = {
    &KSCATEGORY_TOPOLOGY,
};

// Pins: 0 = BridgeIn, 1 = SpeakerOut (connector)
static const PCPIN_DESCRIPTOR kTopoPins[] = {
    // Pin 0: bridge from wave
    {
        .DataFlow = KSPIN_DATAFLOW_IN,
        .Communication = KSPIN_COMMUNICATION_BRIDGE,
        .Category = NULL,
        .Name = NULL,
        .DataRanges = NULL,
        .DataRangesCount = 0,
        .InstancesPossible = 1,
        .InstancesNecessary = 1,
    },
    // Pin 1: speaker connector pin (optional but recommended)
    {
        .DataFlow = KSPIN_DATAFLOW_OUT,
        .Communication = KSPIN_COMMUNICATION_NONE,
        .Category = &KSCATEGORY_AUDIO,
        .Name = NULL,
        .DataRanges = NULL,
        .DataRangesCount = 0,
        .InstancesPossible = 1,
        .InstancesNecessary = 1,
    },
};

static const PCNODE_DESCRIPTOR kTopoNodes[] = {
    // Node 0: speaker
    { .Type = &KSNODETYPE_SPEAKER },
};

// BridgeIn → Speaker node → SpeakerOut
static const KSTOPOLOGY_CONNECTION kTopoConnections[] = {
    // From filter pin 0 (BridgeIn) to speaker node 0.
    { .FromNode = KSFILTER_NODE, .FromPin = 0, .ToNode = 0, .ToPin = 0 },
    // From speaker node 0 back to filter pin 1 (SpeakerOut).
    { .FromNode = 0, .FromPin = 0, .ToNode = KSFILTER_NODE, .ToPin = 1 },
};

static const PCFILTER_DESCRIPTOR kTopoFilterDescriptor = {
    .Version = 1,
    .Flags = 0,
    .PinCount = ARRAYSIZE(kTopoPins),
    .PinDescriptor = kTopoPins,
    .NodeCount = ARRAYSIZE(kTopoNodes),
    .NodeDescriptor = kTopoNodes,
    .ConnectionCount = ARRAYSIZE(kTopoConnections),
    .ConnectionDescriptor = kTopoConnections,
    .CategoryCount = ARRAYSIZE(kTopoCategories),
    .Category = kTopoCategories,
    // .AutomationTable = ... (see section 4)
};
```

Again, treat this as a descriptor “shape” reference. The key is that the topology miniport provides a bridge pin (pin 0) and advertises a speaker-ish graph that SysAudio can reason about.

---

### Property sets / automation tables (minimum for Win7 stability)

This section lists the **property surface** that Windows 7 will exercise during endpoint enumeration and streaming.

#### 4.1 Pin/dataformat intersection (`KSPROPSETID_Pin`)

**Goal:** ensure format negotiation converges to *exactly* the one supported PCM format.

Minimum expected properties:

* `KSPROPERTY_PIN_DATARANGES` (GET)
  * Usually satisfied by the pin descriptor’s data ranges; KS will expose them.
* `KSPROPERTY_PIN_DATAINTERSECTION` (GET)
  * PortCls forwards to `IMiniport::DataRangeIntersection`.
  * **Implementation:** accept only the fixed `WAVEFORMATEXTENSIBLE` and return a full `KSDATAFORMAT_WAVEFORMATEXTENSIBLE`.

Stub strategy:

* Do not attempt to synthesize arbitrary intersections.
* If the caller provides `WAVEFORMATEX` vs `WAVEFORMATEXTENSIBLE`, you can either:
  * reject it (strict), or
  * accept it only if it describes the same PCM format and still return an extensible format.

#### 4.2 WaveRT buffer + position reporting (`KSPROPSETID_RtAudio`)

WaveRT relies on the `KSPROPSETID_RtAudio` property set to:

1. **Negotiate and map the cyclic buffer**.
2. **Configure a notification event**.
3. **Report position** with low overhead.

In practice, a minimal Win7 WaveRT render endpoint should be prepared to handle (directly or via the WaveRT port) at least:

* `KSPROPERTY_RTAUDIO_BUFFER` (GET) — allocate/describe the cyclic buffer and notification granularity.
* One of:
  * `KSPROPERTY_RTAUDIO_POSITIONREGISTER` (GET), or
  * `KSPROPERTY_RTAUDIO_POSITIONFUNCTION` (GET)
* A notification-event registration path (commonly surfaced as an RtAudio property that carries an event handle).

Commonly queried (nice to have; can often be fixed/stubbed):

* `KSPROPERTY_RTAUDIO_HWLATENCY` (GET)
* `KSPROPERTY_RTAUDIO_PRESENTATION_POSITION` (GET)

##### 4.2.1 Buffer property (allocation + mapping)

Windows issues a property request that effectively asks:

* “What is the cyclic buffer size and where is it?”
* “How many notifications per buffer?”

**Driver responsibilities:**

* Provide a kernel buffer that:
  * is nonpaged
  * is stable for the lifetime of the stream while in `ACQUIRE/PAUSE/RUN`
  * is aligned to frames (`BlockAlign`)
* Decide a buffer size policy (see section 5).
* Return the address/MDL information expected by the WaveRT port so it can map to user mode.

##### 4.2.2 Position reporting: register-based vs method-based

Windows 7 supports two practical patterns for WaveRT position:

* **Register-based** (`KSPROPERTY_RTAUDIO_POSITIONREGISTER`): expose a position register description (user-mode reads it directly).
* **Method-based** (`KSPROPERTY_RTAUDIO_POSITIONFUNCTION`): user-mode/port queries position through a kernel call path.

For a software device (virtio-snd) there is no real hardware register. The simplest stable approach is:

* implement **method-based** position reporting (always available)
* optionally also expose a “register” that points at a shared memory location updated by the driver (emulated register), if you want to reduce call overhead later

**What must be consistent:**

* Position must advance at 48 kHz while `RUN` and stop advancing otherwise.
* Reported position must be frame-accurate enough that the audio engine’s padding math doesn’t oscillate.

##### 4.2.3 Notification event

Windows will provide an event handle/object and expect it to be signaled at the configured period. In a software-DMA model, the DPC/timer loop is typically responsible for:

* setting the event each time the play cursor crosses the next period boundary
* clearing/coalescing appropriately (events are level-triggered)

Failure mode: if events never signal, shared-mode render often stays stuck in `PAUSE` or starves immediately.

#### 4.3 Topology channel config (`KSPROPSETID_Audio`)

Minimum property:

* `KSPROPERTY_AUDIO_CHANNEL_CONFIG` (GET/SET) on the appropriate topology node (speaker) or filter.

Recommended semantics for the minimal endpoint:

* GET: always return stereo speaker mask:
  * `SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT`
* SET: accept only that same value; otherwise return `STATUS_NOT_SUPPORTED` or `STATUS_INVALID_PARAMETER`

Why it matters: Windows control panel and the audio engine query/adjust channel config; returning inconsistent values can cause “speaker setup” UI to break or channel masks to be misapplied.

#### 4.4 Optional jack properties (`KSPROPSETID_Jack`)

These are not strictly required for audio to play, but Windows 7 UX and some apps probe them.

Minimal safe stubs (fixed values):

* `KSPROPERTY_JACK_DESCRIPTION` (GET)
  * return a single “jack” describing a speaker/line-out, always present
* `KSPROPERTY_JACK_DESCRIPTION2` (GET)
  * can be zeroed/defaulted if not supported
* `KSPROPERTY_JACK_CONTAINERID` (GET)
  * optional; return a stable GUID if implemented

Stub policy:

* If you don’t implement a jack property, return `STATUS_NOT_SUPPORTED` (not success with garbage).
* If you do implement, keep results stable across boots (container id especially).

---

### Timer/DPC software DMA model (WaveRT → virtio TX)

Because the virtio-snd device model is not a real bus-mastering audio controller, the WaveRT miniport behaves like a “software DMA” engine:

* user mode writes PCM into the WaveRT cyclic buffer
* the driver periodically copies/submits a period of frames to virtio-snd TX
* the driver advances a play cursor and signals notification events

#### 5.1 Period and buffer sizing policy (recommended defaults)

For a first bring-up, pick conservative values:

* **Period:** 10 ms
  * frames/period = `48000 * 10ms = 480` frames
  * bytes/period = `480 frames * 4 B/frame = 1920` bytes
* **Buffer:** 100 ms (10 periods)
  * frames/buffer = `4800`
  * bytes/buffer = `19200`

Rationale:

* 10 ms is a common Windows shared-mode period and keeps scheduling overhead manageable.
* 100 ms gives slack for occasional TX backpressure without immediate underruns.

You may later tune down (e.g., 3 ms) once stability is proven.

#### 5.2 Cursor math (ring buffer)

Maintain cursors in **frames** (not bytes) to avoid off-by-block-align mistakes:

* `bufferFrames` = total frames in the WaveRT cyclic buffer
* `playCursor` = next frame index the driver will submit to hardware (mod `bufferFrames`)
* `notificationCursor` = next frame index at which to signal the notification event (mod `bufferFrames`)

On each tick:

1. Determine `framesToSend` (usually `periodFrames`).
2. Read `framesToSend` frames from `WaveRtBuffer[playCursor..]` with wrap handling.
3. Submit those bytes to virtio TX.
4. Advance `playCursor = (playCursor + framesToSend) % bufferFrames`.
5. If `playCursor` crossed `notificationCursor`, signal event and advance `notificationCursor += periodFrames`.

#### 5.3 Notification signaling

The notification event is conceptually “the hardware consumed another period”. In a software device you have two choices:

* **Submit-driven:** signal when you successfully submit a period to virtio TX (simple, but conflates “accepted” with “played”).
* **Time-driven:** signal based on wall clock at 48 kHz, but clamp by submitted frames (more accurate, slightly more code).

For initial bring-up, submit-driven signaling is acceptable, as long as:

* you do not advance position without also “consuming” data, and
* you handle TX backpressure by *not* advancing/signaling.

#### 5.4 Backpressure when virtio TX is full

When the TX virtqueue has no free descriptors:

* Do **not** block in DPC.
* Do **not** advance `playCursor` (the device did not consume data).
* Keep the timer running; retry next tick.

Optional improvements (later):

* submit smaller chunks when descriptors are low
* keep a small “always available” silent buffer to avoid hard stalls
* expose glitch counters via debug output

#### 5.5 Optional: using `eventq` PCM events as an additional tick source

The virtio-snd specification defines asynchronous PCM notifications on `eventq`, including:

* `VIRTIO_SND_EVT_PCM_PERIOD_ELAPSED`
* `VIRTIO_SND_EVT_PCM_XRUN`

For the Aero Windows 7 virtio contract v1, **no `eventq` messages are required**, so a driver must not depend on these events for
correctness. The safe baseline is a timer-driven “software DMA” loop.

If `PCM_PERIOD_ELAPSED` events do arrive, a driver can treat them as a best-effort **additional wakeup** source for the same period/DPC
logic. When combining a timer tick with event-driven ticks, coalesce duplicates to avoid double-signaling the WaveRT notification event.

If `PCM_XRUN` events arrive, a driver can treat them as a hint that the device observed an underrun/overrun and attempt recovery at
`PASSIVE_LEVEL` (for example by issuing `PCM_STOP` / `PCM_START` and re-priming submission state).

---

### `virtio-snd` backend mapping

This driver maps WaveRT streams to the `virtio-snd` device model described in [`../areas/windows-drivers.md`](../areas/windows-drivers.md):

- render: `virtio-snd` stream id `0` (TX)
- capture: `virtio-snd` stream id `1` (RX)

#### 6.1 Control queue command sequence

The minimal state machine for stream id `0` (render) is:

1. **During device start (PnP START / adapter init):**
   * Optionally query `PCM_INFO` to confirm stream 0 exists.
2. **When the first WaveRT stream is created / format is committed:**
   * `PCM_SET_PARAMS` with:
     * `channels = 2`
     * `format = S16_LE`
     * `rate = 48000`
     * `period_bytes` / `buffer_bytes` consistent with section 5
3. **When transitioning to `PAUSE` (or `ACQUIRE → PAUSE`):**
   * `PCM_PREPARE`
4. **When transitioning to `RUN`:**
   * `PCM_START`
5. **When transitioning out of `RUN`:**
   * `PCM_STOP`
6. **When stream is closed or transitions to `STOP`:**
   * `PCM_RELEASE`

The capture stream (id `1`) uses the same control flow, but with:

* `channels = 1` (mono)
* capture buffers submitted via the virtio-snd RX queue (`rxq`)

#### 6.2 TX descriptor chain payload

Each TX submission is a virtqueue descriptor chain:

* OUT:
  1. `virtio_snd_pcm_xfer` header (8 bytes)
     * `stream_id: u32` (0)
     * `reserved: u32` (0)
  2. raw PCM bytes (interleaved stereo S16_LE)
* IN:
  1. `virtio_snd_pcm_status` (8 bytes)
     * `status: u32`
     * `latency_bytes: u32` (device model currently returns 0)

The miniport should treat non-OK statuses as a stream fault and transition to `STOP`.

#### 6.3 RX descriptor chain payload

Each RX submission is a virtqueue descriptor chain:

* OUT:
  1. `virtio_snd_pcm_xfer` header (8 bytes)
     * `stream_id: u32` (1)
     * `reserved: u32` (0)
* IN:
  1. raw PCM bytes (mono S16_LE) written by the device
  2. `virtio_snd_pcm_status` (8 bytes)
     * `status: u32`
     * `latency_bytes: u32` (device model currently returns 0)

The miniport should treat non-OK statuses as a stream fault and transition to `STOP`.

#### 6.4 IRQL constraints (what can run where)

Practical constraints for a stable Win7 driver:

* **Virtio control commands** (`PCM_SET_PARAMS`, `PREPARE`, `START`, `STOP`, `RELEASE`) should run at **`PASSIVE_LEVEL`** because they typically:
  * wait for a response
  * allocate/init buffers
  * may touch pageable code paths
* **TX submissions** can run at **`DISPATCH_LEVEL`** (DPC) *if*:
  * all buffers are nonpaged
  * virtqueue bookkeeping uses spin locks / interlocked ops only
  * no blocking waits occur

If your virtqueue implementation requires `PASSIVE_LEVEL` (e.g., uses KMDF DMA APIs that are passive-only), use a dedicated worker thread:

* DPC only schedules work (queues a work item) and updates cursors conservatively.
* Worker thread performs TX submissions and signals events.

---

### INF requirements (Windows 7 audio miniport installation)

Windows 7 enumerates WDM audio endpoints through `sysaudio.sys` + `wdmaud.sys` conventions. The INF must:

* Install a **PCI function driver service** for the device.
* Register **wave** and **topology** subdevices via `HKR,Drivers\...` keys.
* Register the **KS device interfaces** that SysAudio/MMDevice enumerates for render + capture.
* Include/need the standard KS and WDMAudio registration sections.

#### 7.1 Minimal INF outline (key directives)

At minimum:

* `Include=ks.inf, wdmaudio.inf`
* `Needs=KS.Registration, WDMAUDIO.Registration`

And an interfaces section that includes both directions, for example:

* `AddInterface = %KSCATEGORY_RENDER%,  %KSNAME_Wave%, AeroVirtioSnd.Wave.Interface`
* `AddInterface = %KSCATEGORY_CAPTURE%, %KSNAME_Wave%, AeroVirtioSnd.Capture.Interface`
* `AddInterface = %KSCATEGORY_TOPOLOGY%, %KSNAME_Topology%, AeroVirtioSnd.Topology.Interface`

And in `AddReg`:

* `HKR,,DevLoader,,*ntkern`
* `HKR,,NTMPDriver,,aero_virtio_snd.sys`
* `HKR,Drivers,SubClasses,, "wave,topology"`
* `HKR,Drivers\wave,Driver,,aero_virtio_snd.sys`
* `HKR,Drivers\wave,Description,,%AeroVirtioSnd.EndpointDesc%`
* `HKR,Drivers\topology,Driver,,aero_virtio_snd.sys`
* `HKR,Drivers\topology,Description,,%AeroVirtioSnd.TopologyDesc%`

If you want Windows 7 to allocate **message-signaled interrupts** (MSI/MSI-X), you must opt in via INF `HKR` keys under:

* `Interrupt Management\\MessageSignaledInterruptProperties`

The in-tree Aero virtio-snd INFs include this opt-in (recommended).

#### 7.2 Hardware IDs

Match at least:

* `PCI\VEN_1AF4&DEV_1059&REV_01` (virtio-snd, Aero contract v1)

For the Aero Windows 7 virtio contract (`AERO-W7-VIRTIO` v1), the in-tree Win7 `virtio-snd`
package is intentionally **revision-gated** and matches only:

* `PCI\VEN_1AF4&DEV_1059&REV_01`

Optionally add more specific matches for your emulator’s subsystem ids if used:

* `PCI\VEN_1AF4&DEV_1059&SUBSYS_XXXXXXXXYYYYYYYY&REV_01` (example placeholder)
* `PCI\VEN_1AF4&DEV_1059&SUBSYS_00191AF4&REV_01` (Aero contract v1 example; also present as a commented-out match in `aero_virtio_snd.inf`)

When testing under QEMU, note that virtio devices may default to `REV_00`. For strict Aero
contract-v1 binding you typically need:

* `-device virtio-sound-pci,disable-legacy=on,x-pci-revision=0x01`

#### 7.3 Worked INF example (AddReg excerpt)

This is an example fragment showing only the Win7-critical keys (not a complete INF):

```ini
; --- Registration glue so SysAudio/WDMAud bind correctly ---
[AeroVirtioSnd_Install.NT]
Include=ks.inf,wdmaudio.inf
Needs=KS.Registration,WDMAUDIO.Registration
CopyFiles=AeroVirtioSnd.CopyFiles
AddReg=AeroVirtioSnd.AddReg

[AeroVirtioSnd_Install.NT.Interfaces]
AddInterface=%KSCATEGORY_RENDER%,%KSNAME_Wave%,AeroVirtioSnd.Wave.Interface
AddInterface=%KSCATEGORY_CAPTURE%,%KSNAME_Wave%,AeroVirtioSnd.Capture.Interface
AddInterface=%KSCATEGORY_TOPOLOGY%,%KSNAME_Topology%,AeroVirtioSnd.Topology.Interface

[AeroVirtioSnd.AddReg]
HKR,,DevLoader,,*ntkern
HKR,,NTMPDriver,,aero_virtio_snd.sys

; Tell wdmaud/sysaudio which miniports exist in this driver.
HKR,Drivers,SubClasses,,"wave,topology"

; WaveRT subdevice
HKR,Drivers\wave,Driver,,aero_virtio_snd.sys
HKR,Drivers\wave,Description,,%AeroVirtioSnd.EndpointDesc%

; Topology subdevice
HKR,Drivers\topology,Driver,,aero_virtio_snd.sys
HKR,Drivers\topology,Description,,%AeroVirtioSnd.TopologyDesc%
```

Notes:

* `SubClasses` is a comma-separated string list. Keep the tokens lowercase (`wave`, `topology`) to match common tooling expectations.
* Real drivers often add `AssociatedFilters`, `FriendlyName`, `WaveRT` flags, etc. Start minimal, then add only when you know why.

#### 7.4 Optional: MSI/MSI-X opt-in (`Interrupt Management`)

Windows 7 typically allocates MSI/MSI-X only when explicitly requested via INF:

```inf
[AeroVirtioSnd_Install.NT.HW]
AddReg = AeroVirtioSnd_InterruptManagement_AddReg, AeroVirtioSnd_Parameters_AddReg

[AeroVirtioSnd_InterruptManagement_AddReg]
HKR, "Interrupt Management",,0x00000010
HKR, "Interrupt Management\\MessageSignaledInterruptProperties", MSISupported,        0x00010001, 1
; virtio-snd uses 4 virtqueues + a config interrupt = 5 vectors; request a little extra:
HKR, "Interrupt Management\\MessageSignaledInterruptProperties", MessageNumberLimit,  0x00010001, 8

; Per-device bring-up toggles (defaults):
[AeroVirtioSnd_Parameters_AddReg]
HKR,Parameters,,0x00000010
HKR,Parameters,ForceNullBackend,0x00010003,0
HKR,Parameters,AllowPollingOnly,0x00010003,0
```

Notes:

* `MessageNumberLimit` is a request; Windows may allocate fewer messages.
* `0x00010001` = `REG_DWORD`
* `0x00010003` = `REG_DWORD` + `FLG_ADDREG_NOCLOBBER` (do not overwrite an existing value; preserves per-device bring-up toggles across reinstalls/upgrades).
* `HKR` in a `.NT.HW` section is relative to the device instance’s **Device Parameters** key:
  * `HKLM\SYSTEM\CurrentControlSet\Enum\<DeviceInstancePath>\Device Parameters`
  * The bring-up toggles above therefore live under:
    * `...\Device Parameters\Parameters\ForceNullBackend`
    * `...\Device Parameters\Parameters\AllowPollingOnly`
  * Find `<DeviceInstancePath>` via **Device Manager → device → Details → “Device instance path”**.
  * Backwards compatibility: older installs may instead store these values under the per-device driver key; the driver checks the device key first and falls back.
* When message interrupts are used, drivers must still program virtio MSI-X routing (`msix_config`, `queue_msix_vector`).
  - On Aero contract devices, if MSI-X is enabled at the PCI layer but a virtio MSI-X selector remains
    `VIRTIO_PCI_MSI_NO_VECTOR` (`0xFFFF`) (or the MSI-X entry is masked/unprogrammed), interrupts for that source are
    **suppressed** (no MSI-X message and no INTx fallback).
  - Therefore, if virtio vector programming fails, drivers must not “wait for INTx”; they must either disable MSI-X and
    use INTx (if the platform/resources allow it) or treat the failure as fatal.

---

### Debugging checklist (bring-up and “no sound” triage)

#### 8.1 Endpoint enumeration

Checklist:

1. Device Manager:
   * Device appears under **Sound, video and game controllers**.
   * No Code 10 (start failed) / Code 52 (signature enforcement).
2. Sound control panel:
    * A playback device appears (often “Speakers”).
    * A recording device appears (often “Microphone”).
    * Default format lists (at least) 16-bit, 48000 Hz.

Where to look when it fails:

* `C:\Windows\inf\setupapi.dev.log` — INF processing, copy failures, signature issues.
* `C:\Windows\System32\drivers\` — confirm the `.sys` copied.
* Code 52: test-signing / certificate install is wrong (see driver signing docs).

#### 8.2 State transitions to `RUN`

Instrument (DbgPrint / WPP) the following points:

* miniport `Init`
* WaveRT `NewStream`
* stream `SetState` transitions
* control queue calls (`SET_PARAMS`, `PREPARE`, `START`, `STOP`, `RELEASE`)

Expected sequence on first playback:

1. stream created (format validated)
2. `SET_PARAMS`
3. `PREPARE`
4. `START`
5. periodic “tick” logs while `RUN`

#### 8.3 Periodic notification events

Symptoms if broken:

* playback starts then immediately stops
* audio engine stays in “silent” / no device activity
* apps report success but no sound

Confirm:

* event registration happened (log event pointer)
* event is signaled once per period while `RUN`
* play cursor advances monotonically

#### 8.4 TX submission counts

Confirm in logs:

* how many TX buffers were enqueued per second
  * for 10 ms period: ~100 submissions/sec
* how many completions are observed
* whether TX stalls due to “virtqueue full”

When TX stalls:

* verify interrupt/DPC handling for virtqueue completions
* verify the host/device model is consuming descriptors

#### 8.5 Useful tools

* **DbgView** for `DbgPrint` output (user-mode collection).
* **Kernel debugger (WinDbg/KD)** for:
  * breakpoints in `SetState`
  * inspecting KS objects and IRPs
* **KSStudio** (WDK) to inspect filters/pins/categories and validate the descriptor surface.

---

### Appendix A: Worked `WAVEFORMATEXTENSIBLE` example (stereo/48k/16-bit)

This is the canonical format the driver should accept and return from data intersection:

```c
// 2ch, 48kHz, 16-bit PCM (WAVEFORMATEXTENSIBLE)
static const WAVEFORMATEXTENSIBLE kVirtioSndWfx = {
    .Format = {
        .wFormatTag = WAVE_FORMAT_EXTENSIBLE,
        .nChannels = 2,
        .nSamplesPerSec = 48000,
        .nAvgBytesPerSec = 48000 * 2 * 2, // 192000
        .nBlockAlign = 2 * 2,             // 4 bytes per frame
        .wBitsPerSample = 16,
        .cbSize = 22,
    },
    .Samples = { .wValidBitsPerSample = 16 },
    .dwChannelMask = SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
    .SubFormat = KSDATAFORMAT_SUBTYPE_PCM,
};
```

If the caller asks for `WAVEFORMATEX` with the same values, you can either reject it (strict) or accept it and still return `WAVEFORMATEXTENSIBLE` (recommended for consistency).

## Virtqueue DMA Strategy (Windows 7 KMDF, virtio 1.0 PCI modern)

This document describes a **recommended DMA and memory-allocation strategy** for Aero’s Windows 7 KMDF virtio drivers when implementing **virtio 1.0 PCI “modern”** devices using the **split virtqueue** layout (descriptor table + avail ring + used ring).

See also:

* [`../../virtio/virtqueue-split-ring-win7.md`](windows7-virtio-driver-contract.md) — split-ring virtqueue algorithms (descriptor mgmt, ordering/barriers, EVENT_IDX, indirect).
* [`../../windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md) — Aero’s definitive virtio device/feature/transport contract.
* `drivers/windows/virtio/common/` — reference split virtqueue implementation (helpers like `VirtqSplitRingMemSize`, `VirtqSplitInit`, `VirtqSplitAddBuffer`, ...), plus a WDF-free PFN/MDL → SG builder (`virtio_sg_pfn.h/.c`).

The focus is on:

* Allocating virtqueue rings / indirect tables in DMA-safe memory.
* Mapping I/O buffers (IRP MDLs) into virtio descriptors robustly.
* Correct cache + ordering rules so the device and CPU agree on ring contents.

Non-goals:

* Full virtio feature negotiation walkthrough (except where it affects DMA: `INDIRECT_DESC`, `EVENT_IDX`).
* Packed virtqueues (virtio 1.1+).

---

### DMA enabler creation (profile selection, MaximumLength, SG limits)

#### 1.1 Preferred profile: 64-bit duplex, fallback to 32-bit duplex

Virtio queues frequently involve **both directions** at the driver level (e.g., virtio-blk read: header OUT, data IN, status IN). Even though an individual DMA transaction is still single-direction, using a **duplex** profile avoids artificial restrictions and is the right default for a driver that will have both read and write DMA activity.

Recommendation:

1. Try `WdfDmaProfileScatterGather64Duplex`
2. If unsupported, fall back to `WdfDmaProfileScatterGather32Duplex`

Notes:

* On a 32-bit OS build, everything is <4GiB anyway; using the 32-bit profile is typically fine.
* On systems with an IOMMU / DMA remapping, the “device address” you must give virtio is not necessarily the CPU physical address. **Use the DMA framework’s mapped addresses** (logical/bus addresses), not `MmGetPhysicalAddress`.

Pseudo-code:

```c
NTSTATUS CreateDmaEnabler(
    _In_ WDFDEVICE Device,
    _In_ size_t MaximumLength,
    _In_ ULONG MaxSgElements,
    _Out_ WDFDMAENABLER* DmaEnablerOut
    )
{
    NTSTATUS status;
    WDFDMAENABLER dma = NULL;
    WDF_DMA_ENABLER_CONFIG cfg;

    WDF_DMA_ENABLER_CONFIG_INIT(&cfg, WdfDmaProfileScatterGather64Duplex, MaximumLength);
    status = WdfDmaEnablerCreate(Device, &cfg, WDF_NO_OBJECT_ATTRIBUTES, &dma);

    if (!NT_SUCCESS(status)) {
        WDF_DMA_ENABLER_CONFIG_INIT(&cfg, WdfDmaProfileScatterGather32Duplex, MaximumLength);
        status = WdfDmaEnablerCreate(Device, &cfg, WDF_NO_OBJECT_ATTRIBUTES, &dma);
        if (!NT_SUCCESS(status)) {
            return status;
        }
    }

    // Clamp WDF’s SG list length so we can always represent it in virtio descriptors.
    WdfDmaEnablerSetMaximumScatterGatherElements(dma, MaxSgElements);

    *DmaEnablerOut = dma;
    return STATUS_SUCCESS;
}
```

#### 1.2 Choosing `MaximumLength` (per-transaction length)

`MaximumLength` is the **maximum number of bytes** that a single DMA transaction may map. Pick a value that:

* Covers the driver’s largest “one request” payload (or the largest MDL you will map in one go).
* Does not explode memory usage in WDF’s DMA map-register/bounce-buffer paths.
* Fits within the device’s limitations (e.g., virtio-blk `seg_max`/`size_max` if present).

Important behavioral note:

* If the request buffer exceeds `MaximumLength`, WDF can split the work into multiple DMA programming phases (multiple `EvtProgramDma` calls for the same transaction). That model doesn’t map cleanly to a “one virtio request = one descriptor chain” design, so prefer **`MaximumLength >= max_request_bytes`**.

Concrete recommendations:

* virtio-blk: choose `MaximumLength = min(OS_max_transfer, device_size_max_if_known)`; a conservative default is **1 MiB** per request.
* virtio-net: choose `MaximumLength` around typical packet aggregation sizes; **64 KiB** is a safe upper bound for a single packet buffer chain, but large-send/TSO paths may want more if supported.

The key is consistency with your descriptor budgeting (indirect table sizing) described below.

#### 1.3 Choosing `MaxSgElements` (`WdfDmaEnablerSetMaximumScatterGatherElements`)

If your virtqueue implementation can only represent **N** scatter-gather segments for “the data payload”, then you must clamp WDF to **at most N**, or you risk receiving an `SCATTER_GATHER_LIST` you cannot encode.

Concrete recommendation:

* Size the per-request indirect descriptor table to a fixed maximum (example below), and set:
  * `MaxSgElements = MaxIndirectDescriptorsForData`
* Additionally reserve descriptors for protocol headers/status that are not part of the data SG list.

Important behavioral note:

* If the MDL requires more than `MaxSgElements` segments under the platform’s DMA constraints, WDF can again split the DMA programming into multiple phases. For virtio queues, it’s usually better to **bound request sizes** so you never hit this.

Example budgeting for virtio-blk using one indirect table per request:

```
Indirect table layout:
  [0] = virtio-blk header (OUT, common buffer)
  [1..N] = data SG elements (IN for READ, OUT for WRITE, from WDF SG list)
  [N+1] = status byte (IN, common buffer)

MaxIndirectDescriptorsTotal = N + 2
MaxSgElements (WDF) = N
```

Pitfall: do not confuse **virtqueue ring size** (number of ring descriptors) with **scatter-gather element count**. With `INDIRECT_DESC`, each request consumes **1** ring descriptor, but still needs `N + overhead` indirect descriptors.

---

### Rings/indirect tables: WDF common buffers vs `MmAllocateContiguousMemory*`

#### Recommendation: prefer `WdfCommonBufferCreateWithConfig`

For virtqueue rings (desc/avail/used) and indirect descriptor tables, prefer **WDF common buffers** over raw `MmAllocateContiguousMemory*` allocations.

Why:

* **DMA adapter aware**: the allocation is made for the correct DMA adapter associated with the PCI device.
* **IOMMU / bounce support**: WDF can ensure the device can access the memory even if the platform requires mapping/bouncing. The returned address is a **device DMA address**.
* **Returns the correct address type**: `WdfCommonBufferGetAlignedLogicalAddress()` returns the bus/logical address that should be programmed into the virtio device. `MmAllocateContiguousMemory*` only gives you CPU-physical contiguity, not the device’s view.
* **Lifetime management**: the common buffer is a WDF object that can be parented to the device/queue object and automatically freed.

Avoid `MmAllocateContiguousMemory*` for production drivers because it:

* Encourages using `MmGetPhysicalAddress`/CPU physical addresses (wrong with IOMMU).
* Is not integrated with WDF’s DMA resource accounting.
* Tends to fail more often under memory pressure / fragmentation.

#### Recommended common-buffer configuration

* `CacheEnabled = FALSE` for rings/indirect tables (descriptor rings are small; correctness beats cache speed).
* `RequestedAlignment = PAGE_SIZE` to simplify virtqueue layout math and to satisfy all virtqueue alignment constraints trivially.

Pseudo-code:

```c
WDF_COMMON_BUFFER_CONFIG cbcfg;
WDF_COMMON_BUFFER_CONFIG_INIT(&cbcfg, /*PoolType*/ NonPagedPool);
cbcfg.CacheEnabled = FALSE;
cbcfg.RequestedAlignment = PAGE_SIZE;

status = WdfCommonBufferCreateWithConfig(
    DmaEnabler,
    RingBytes,
    WDF_NO_OBJECT_ATTRIBUTES,
    &cbcfg,
    &Vq->RingCommonBuffer);

Vq->RingVa = WdfCommonBufferGetAlignedVirtualAddress(Vq->RingCommonBuffer);
Vq->RingPa = WdfCommonBufferGetAlignedLogicalAddress(Vq->RingCommonBuffer); // device address
```

---

### Split virtqueue ring alignment + exact size formulas

Virtio split-ring layout has three regions:

1. `virtq_desc` table (array of descriptors)
2. `virtq_avail` ring (driver → device)
3. `virtq_used` ring (device → driver)

#### 3.1 Alignment requirements

* Descriptor table: **16-byte aligned**
* Avail ring: **2-byte aligned**
* Used ring: **4-byte aligned**

Recommendation:

* Allocate a single common buffer for the whole ring with **PAGE_SIZE alignment** and then compute offsets with `align_up()`. A page-aligned base is automatically aligned for 16/4/2.

#### 3.2 Size formulas (with/without `EVENT_IDX`)

Let:

* `qsz` = virtqueue size (`QueueSize`), in descriptors
* `event_idx` = negotiated `VIRTIO_F_RING_EVENT_IDX` (true/false)

Then:

**Descriptor table**

* Each `virtq_desc` is 16 bytes
* `desc_len = 16 * qsz`

**Avail ring**

* Fields: `u16 flags; u16 idx; u16 ring[qsz];` and optionally `u16 used_event;`
* `avail_len = 4 + (2 * qsz) + (event_idx ? 2 : 0)`

**Used ring**

* Each `virtq_used_elem` is 8 bytes (`u32 id; u32 len;`)
* Fields: `u16 flags; u16 idx; struct virtq_used_elem ring[qsz];` and optionally `u16 avail_event;`
* `used_len = 4 + (8 * qsz) + (event_idx ? 2 : 0)`

#### 3.3 Computing offsets (pseudo-code)

```c
static __forceinline size_t AlignUp(size_t x, size_t a)
{
    return (x + (a - 1)) & ~(a - 1);
}

void VirtqComputeLayout(
    _In_  USHORT qsz,
    _In_  BOOLEAN event_idx,
    _Out_ size_t* desc_off,
    _Out_ size_t* avail_off,
    _Out_ size_t* used_off,
    _Out_ size_t* total_bytes
    )
{
    size_t desc_len  = 16u * (size_t)qsz;
    size_t avail_len = 4u + 2u * (size_t)qsz + (event_idx ? 2u : 0u);
    size_t used_len  = 4u + 8u * (size_t)qsz + (event_idx ? 2u : 0u);

    *desc_off  = AlignUp(0, 16);
    *avail_off = AlignUp(*desc_off + desc_len, 2);
    *used_off  = AlignUp(*avail_off + avail_len, 4);

    // Optional but recommended: round up to a whole page for simpler debug and future growth.
    *total_bytes = AlignUp(*used_off + used_len, PAGE_SIZE);
}
```

Pitfall: Do not assume `avail_off + avail_len` is already 4-byte aligned (it often isn’t), so always `align_up()` before placing the used ring.

---

### Buffer mapping approaches (robust vs direct)

#### 4.1 Robust path: `WDFDMATRANSACTION` → `EvtProgramDma` → `SCATTER_GATHER_LIST`

Use this path for “real” DMA correctness:

* Works with IOMMU / DMA remapping.
* Works with devices limited to 32-bit addresses (WDF will bounce/map as needed).
* Gives you properly coalesced SG segments per the platform’s DMA constraints.

Recommended flow (per request):

1. Allocate/prepare a per-request context that owns:
   * A ring descriptor index (1 if using `INDIRECT_DESC`)
   * A slot in a common-buffer “indirect table pool”
   * A `WDFDMATRANSACTION` (for the data payload MDL)
2. Initialize the DMA transaction for the request’s main data MDL:
   * Direction depends on operation (virtio-blk READ = device writes into memory → `WdfDmaDirectionReadFromDevice`)
3. `WdfDmaTransactionExecute()` triggers `EvtProgramDma(...)` with an `SCATTER_GATHER_LIST`
4. In `EvtProgramDma`, build the virtio descriptor chain using:
   * Common-buffer header/status descriptors (small, fixed)
   * SG list elements for the data payload (variable)
5. Publish the head descriptor to the avail ring and notify the device.
6. **Keep the DMA transaction alive** until the device reports completion via the used ring, then call:
   * `WdfDmaTransactionDmaCompletedFinal(Transaction, 0 /* ignored */);`

Why “keep it alive”:

* If WDF had to allocate bounce buffers/map registers, the addresses given in the SG list are only valid while the transaction is active.
* Completing/finalizing the transaction too early can lead to device DMA into freed/reused mappings.

IRQL/locking note:

* `EvtProgramDma` can run at **DISPATCH_LEVEL**. Treat it like a DPC: no pageable code, no blocking waits, and avoid lock inversion with your completion path.

##### (Aero repo) reusable mapping helpers

The Aero repo includes a small, reusable helper layer that wraps the KMDF DMA transaction flow and converts the mapped
`SCATTER_GATHER_LIST` into a virtio-friendly `(addr,len)` list:

* `windows-drivers/virtio-kmdf/common/virtio_sg.h`
* `windows-drivers/virtio-kmdf/common/virtio_sg_wdfdma.c`

Key API:

* `VirtioWdfDmaStartMapping(...)` — starts the DMA transaction, builds `VIRTIO_SG_ELEM[]` from the `SCATTER_GATHER_LIST`,
  and keeps the transaction alive until completion.
* `VirtioWdfDmaCompleteAndRelease(...)` — call on virtqueue completion to finalize the DMA transaction and free resources.

On success, the returned `VIRTIO_WDFDMA_MAPPING` contains `mapping->Sg.Elems[0..mapping->Sg.Count-1]` (device/bus
addresses + lengths) ready to be copied into virtio descriptors/indirect tables.

Note: the helper is intentionally **single-shot** (one DMA programming callback must cover the entire buffer). Ensure your
DMA enabler’s `MaximumLength` and `MaxScatterGatherElements` are sized to match the largest request you will submit.

Pseudo-code outline:

```c
BOOLEAN EvtProgramDma(
    WDFDMATRANSACTION Transaction,
    WDFDEVICE Device,
    PVOID Context,
    WDF_DMA_DIRECTION Direction,
    PSCATTER_GATHER_LIST SgList
    )
{
    REQUEST_CTX* req = Context;

    // Build indirect table in common-buffer memory.
    VIRTQ_DESC* itbl = req->IndirectDescVa;
    ULONG n = 0;

    // 1) protocol header (common buffer, OUT)
    itbl[n++] = MakeDesc(req->HdrDmaAddr, sizeof(req->Hdr), /*write=*/FALSE);

    // 2) data payload (from WDF SG list; direction depends on request)
    for (ULONG i = 0; i < SgList->NumberOfElements; i++) {
        itbl[n++] = MakeDesc(SgList->Elements[i].Address, SgList->Elements[i].Length,
                             /*write=*/(Direction == WdfDmaDirectionReadFromDevice));
    }

    // 3) status byte (common buffer, IN)
    itbl[n++] = MakeDesc(req->StatusDmaAddr, 1, /*write=*/TRUE);

    // Ring descriptor points at the indirect table (device reads indirect table)
    Vq->Desc[req->RingDescIndex] =
        MakeIndirectDesc(req->IndirectTableDmaAddr, n * sizeof(VIRTQ_DESC));

    VirtqPublishAvail(Vq, req->RingDescIndex);
    return TRUE;
}
```

#### 4.2 Direct path (emulated-only): build SG from MDL PFN array

This approach is sometimes used in hypervisor/emulated environments where:

* The “device” effectively sees guest physical memory directly.
* There is no IOMMU remapping.
* You control the platform assumptions.

Strategy:

1. Use `MmGetMdlPfnArray(Mdl)` to obtain PFNs.
2. Coalesce physically-contiguous PFNs into `(Address, Length)` segments.
3. Use those addresses directly in virtio descriptors.

Pseudo-code for coalescing PFNs:

```c
PMDL mdl = ...;
PPFN_NUMBER pfns = MmGetMdlPfnArray(mdl);
ULONG pages = ADDRESS_AND_SIZE_TO_SPAN_PAGES(MmGetMdlVirtualAddress(mdl), MmGetMdlByteCount(mdl));
ULONG offset = MmGetMdlByteOffset(mdl);
ULONG remaining = MmGetMdlByteCount(mdl);

for (ULONG p = 0; p < pages; ) {
    PFN_NUMBER first = pfns[p];
    ULONG run = 1;
    while ((p + run) < pages && pfns[p + run] == first + run) {
        run++;
    }

    PHYSICAL_ADDRESS pa;
    pa.QuadPart = ((ULONGLONG)first << PAGE_SHIFT);

    size_t seg_len = (size_t)run * PAGE_SIZE;
    size_t seg_off = (p == 0) ? offset : 0;
    size_t seg_use = min(seg_len - seg_off, (size_t)remaining);

    EmitVirtioDesc(pa.QuadPart + seg_off, (ULONG)seg_use, ...);

    remaining -= (ULONG)seg_use;
    p += run;
}
```

Cache flushing:

* For **OUT** buffers (device reads from memory): flush CPU writes before notifying the device:
  * `KeFlushIoBuffers(mdl, FALSE /* ReadOperation */, TRUE /* DmaOperation */)`
* For **IN** buffers (device writes to memory): ensure dirty CPU cache lines won’t overwrite device data and invalidate CPU caches so reads see device writes:
  * `KeFlushIoBuffers(mdl, TRUE /* ReadOperation */, TRUE /* DmaOperation */)` before handing the buffer to the device, and again after completion if the CPU will read the data.

Pitfalls of the direct path (why it’s “emulated-only”):

* PFN-derived “physical” addresses are not valid device addresses with IOMMU/bounce.
 * On systems with 32-bit DMA limitations, the device may not be able to reach high PFNs.
 * You must do your own SG element limiting/coalescing; WDF normally handles this.

##### (Aero repo) direct MDL → PFN mapping helper

For drivers that intentionally use PFN-derived physical addresses (e.g., in fully emulated environments), Aero includes a
direct mapping helper that walks an MDL chain and emits per-page segments, coalescing physically-contiguous PFNs:

* `windows-drivers/virtio-kmdf/common/virtio_sg.h`
* `windows-drivers/virtio-kmdf/common/virtio_sg.c`
* WDF-free (WDM-friendly) variant: `drivers/windows/virtio/common/virtio_sg_pfn.h/.c`

Key API (both modules use the same function names):

* `VirtioSgMaxElemsForMdl(...)` — worst-case upper bound on element count (pages spanned).
* `VirtioSgBuildFromMdl(...)` — builds an SG array in caller-provided storage (no allocations; DISPATCH_LEVEL safe):
  * KMDF module: `VIRTIO_SG_ELEM[]`
  * WDF-free module: `VIRTQ_SG[]`

#### 4.3 Mixed-direction chains: recommended pattern (virtio-blk, virtio-net)

Virtio request chains typically contain a mix of:

* device-readable buffers (OUT, driver → device)
* device-writable buffers (IN, device → driver)

Instead of trying to represent an entire mixed chain via a single WDF DMA transaction:

**Recommended pattern**

* Allocate small, fixed-size per-request buffers (headers/status) from a **common-buffer slab**.
  * Always DMA-safe, device-addressable, and physically contiguous.
* Use a WDF DMA transaction **only for the bulk data payload**, which is naturally single-direction.

Examples:

* **virtio-blk READ**
  * header (OUT, common buffer)
  * data (IN, DMA transaction SG list)
  * status (IN, common buffer)
* **virtio-blk WRITE**
  * header (OUT, common buffer)
  * data (OUT, DMA transaction SG list)
  * status (IN, common buffer)
* **virtio-net TX**
  * virtio-net header (OUT, common buffer)
  * payload (OUT, DMA transaction SG list)
* **virtio-net RX**
  * virtio-net header (IN, common buffer)
  * payload buffers (IN, DMA transaction SG list, or pre-posted buffers)

#### 4.4 Indirect descriptors: make each request consume 1 ring descriptor

If `VIRTIO_F_RING_INDIRECT_DESC` is negotiated:

* Each in-flight request uses **one** entry in the main ring descriptor table.
* That ring descriptor points to a per-request indirect table in common-buffer memory.

Benefits:

* Simplifies “queue full” handling (max in-flight requests == ring size).
* Reduces ring descriptor pressure when a request has many SG segments.
* Keeps the main ring stable: a single `avail->ring[i] = head_desc` publish per request.

Pitfall: the indirect table memory must be described by a single `(addr,len)` in the ring descriptor, so allocate it from **physically contiguous, DMA-safe memory** (common buffer). Do not build indirect tables in nonpaged pool unless you also map them as a single contiguous DMA segment (which is usually not possible).

---

### Cache coherency and memory ordering (barriers)

#### 5.1 Common buffer caching (`CacheEnabled`)

* `CacheEnabled = FALSE` (uncached) is the safest default for rings/indirect tables.
  * Avoids subtle coherency issues on non-coherent platforms.
  * Rings are tiny, so performance impact is negligible.
* If `CacheEnabled = TRUE` (cached):
  * You must ensure the device sees descriptor writes promptly and that CPU sees device writes to used ring promptly.
  * In practice on x86 this is usually coherent, but the driver should not rely on it.
  * If in doubt, allocate rings uncached.

For MDL-mapped data buffers:

* Robust/WDF path: calling `WdfDmaTransactionDmaCompletedFinal` is part of the contract that makes CPU/device views consistent.
* Direct PFN path: use `KeFlushIoBuffers` as described above.

#### 5.2 Required barriers when publishing to the avail ring

Rule: the device must never observe `avail->idx` incremented before it can observe the descriptor contents and the corresponding `avail->ring[]` entry.

Pseudo-code:

```c
void VirtqPublishAvail(_Inout_ VIRTQ* vq, _In_ USHORT head_desc)
{
    USHORT idx = vq->avail->idx; // local snapshot

    vq->avail->ring[idx % vq->qsz] = head_desc;

    // Ensure all descriptor writes + ring entry write are globally visible
    // before we publish the new avail->idx.
    KeMemoryBarrier();

    vq->avail->idx = (USHORT)(idx + 1);

    // Ensure avail->idx is visible before ringing the doorbell/MMIO notify.
    KeMemoryBarrier();
    VirtioQueueNotify(vq);
}
```

If `EVENT_IDX` is negotiated, your notify decision is gated by `used_event` in the avail ring; ensure you use barriers around reading `used_event` and writing `avail->idx` per the virtio spec.

#### 5.3 Required barriers when consuming from the used ring

Rule: the driver must not read `used->ring[]` entries before it has observed the corresponding `used->idx`.

Pseudo-code:

```c
void VirtqConsumeUsed(_Inout_ VIRTQ* vq)
{
    USHORT used_idx = vq->used->idx;

    // Ensure subsequent reads of used->ring see entries associated with used_idx.
    KeMemoryBarrier();

    while (vq->last_used_idx != used_idx) {
        ULONG i = vq->last_used_idx % vq->qsz;
        struct virtq_used_elem e = vq->used->ring[i];

        vq->last_used_idx++;

        CompleteRequestById(e.id, e.len);
    }
}
```

---

### Cleanup requirements (PnP stop/start, resource rebalance)

#### 6.1 What to free in `EvtDeviceReleaseHardware` (and why)

Windows can stop/start a device without unloading the driver (PnP stop for rebalance, surprise remove, etc.). `EvtDeviceReleaseHardware` is where you must release resources that are tied to the current hardware resources / DMA adapter.

Free/delete in `EvtDeviceReleaseHardware`:

* Virtqueue ring common buffers (desc/avail/used allocation).
* Indirect table pool common buffers (if separate from ring).
* Header/status common-buffer slabs.
* WDF DMA objects that are adapter-specific:
  * `WDFDMAENABLER` (if created per hardware instance)
  * Any persistent DMA transactions tied to queues

Reason:

* The DMA adapter and its constraints can change after stop/start (different resources, different remapping behavior). Reusing old “device addresses” across a rebalance is a correctness bug.

#### 6.2 Quiescing outstanding DMA before freeing rings

Before freeing any ring/indirect-table memory, you must ensure:

* The virtio device is no longer executing DMA that references those addresses.
* All in-flight `WDFDMATRANSACTION`s have been finalized/completed so WDF can tear down mappings/bounce buffers safely.

Recommended quiesce sequence:

1. Stop submitting new requests (fail or queue them in software).
2. Disable interrupts / notifications for the queue.
3. Reset the virtio device / virtqueue (transport-specific), so the device will not DMA further.
4. Drain/cancel all outstanding requests:
   * For each in-flight request that has an active DMA transaction:
     * Call `WdfDmaTransactionDmaCompletedFinal(Transaction, 0);`
     * Then `WdfDmaTransactionRelease(Transaction);` (or delete the transaction object if one-shot)
     * Complete the WDFREQUEST with an error status (`STATUS_CANCELLED`/`STATUS_DEVICE_NOT_CONNECTED`).
5. Only then delete/free the common buffers.

Pitfall: freeing the ring common buffer while the device is still running can produce memory corruption that looks like “random” crashes (the device continues DMA into freed pages).

---

### Common pitfalls checklist

* **Using CPU physical addresses instead of DMA-mapped (logical) addresses**
  * Wrong: `MmGetPhysicalAddress(buffer)`
  * Right: `WdfCommonBufferGetAlignedLogicalAddress()` for common buffers; `SCATTER_GATHER_LIST->Elements[i].Address` for MDL buffers.
* **Queue full interactions with DMA callbacks**
  * Don’t start (`WdfDmaTransactionExecute`) a transaction if you cannot guarantee a ring slot (or an indirect-table slot). Otherwise `EvtProgramDma` fires when the queue is full and you have to either fail or defer while holding DMA resources.
* **Descriptor-count limits**
  * If the device or driver only supports N segments, clamp WDF with `WdfDmaEnablerSetMaximumScatterGatherElements`.
  * Ensure indirect tables have enough entries for `header + N + status`.
* **Indirect table contiguity**
  * The indirect table must be reachable via a single descriptor `(addr,len)`. Allocate it from a common buffer (or an equivalent DMA-safe contiguous allocation).
* **32-bit limitations**
  * If forced into 32-bit DMA, you must rely on the DMA framework (WDF) to map/bounce into reachable addresses. Do not assume “all RAM <4GiB”.
* **Cache / coherency bugs**
  * Use uncached common buffers for rings/indirect tables.
  * Use `KeMemoryBarrier()` around ring index publication/consumption.
  * If doing PFN-based direct mapping, flush with `KeFlushIoBuffers`.
* **EVENT_IDX gotchas**
  * The extra `used_event/avail_event` fields change structure sizes. Make sure your offset/size formulas match the negotiated feature set.

---

## Virtio-input end-to-end test plan (device model + Win7 driver + web runtime)

This is the single “do these steps” plan for validating **virtio-input** (keyboard + mouse) end-to-end:

1. Rust/device-model conformance (host-side)
2. Windows 7 driver unit tests (host-side, portable)
3. Windows 7 driver bring-up under QEMU (manual)
4. Windows 7 automated host harness (QEMU + guest selftest)
5. Web runtime validation (browser → input routing → virtio-input)

Authoritative interoperability contract (device model ↔ Win7 drivers):

- **[`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md)** (`AERO-W7-VIRTIO` v1)

Overview (device model behavior and motivation):

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md)

If a test fails, treat the contract as the source of truth; fix code or bump the contract version.

> Note: virtio-input is also available in the canonical full-system VM (`aero_machine::Machine`,
> either native or via `crates/aero-wasm::Machine`). You can do early bring-up there:
> configure the machine with `MachineConfig.enable_virtio_input = true` (native), or in JS/WASM:
> `api.Machine.new_with_options(..., { enable_virtio_input: true })`, and boot a Win7 image with
> the Aero virtio-input driver installed. This is complementary to the QEMU flows below (QEMU is
> still the fastest way to compare against a reference virtio implementation).
>
> Note: in `aero_machine::Machine`, virtio-input supports both INTx (baseline) and MSI-X. MSI-X
> delivery is wired through `VirtioMsixInterruptSink` → `PlatformInterrupts::trigger_msi` and is
> covered by `crates/aero-machine/tests/virtio_input_msix.rs`.
>
> **Windows images are not distributed.** This repo does not include proprietary Windows 7 images/ISOs.
> For QEMU/host-harness testing you must supply your own Win7 media or a locally prepared image.
> See [`../areas/testing.md`](../areas/testing.md) and [`../history/project-history.md`](../history/sprint-era-record.md).

---

### Rust / device-model tests (host-side)

#### 1.1 Run the virtio-input-focused tests (recommended)

From the repo root:

> Note (agent sandboxes / shared hosts): if you see `rustc` panics mentioning
> `WouldBlock` / `Resource temporarily unavailable` (for example `Unable to install ctrlc handler` or
> `called Result::unwrap() on an Err value: Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }`),
> re-run via `safe-run.sh` (recommended below). `safe-run.sh` defaults to `-j1` and will retry a few
> times. You can tune retries with `AERO_SAFE_RUN_RUSTC_RETRIES=...` and, if needed, raise the
> address-space cap with `AERO_MEM_LIMIT=...` (see `../areas/testing.md`).

```bash
cargo test -p aero-virtio --locked --test virtio_input
```

Recommended (enforces repo resource limits via `safe-run.sh`):

```bash
bash ./scripts/safe-run.sh cargo test -p aero-virtio --locked --test virtio_input
```

Alternative (name-filter based; useful when you don’t remember the test binary name; may match 0 tests):

```bash
# NOTE: `cargo test -- <pattern>` is a name filter. Always confirm the output says
# `running N tests` with N > 0; if you see `running 0 tests`, use `--test virtio_input`.
bash ./scripts/safe-run.sh cargo test -p aero-virtio --locked -- virtio_input

# Without safe-run.sh (no timeout / mem limit):
cargo test -p aero-virtio --locked -- virtio_input
```

Tip: if you see `running 0 tests`, prefer the explicit integration test invocation:

```bash
bash ./scripts/safe-run.sh cargo test -p aero-virtio --locked --test virtio_input
```

Primary coverage lives in:

- [`crates/aero-virtio/tests/virtio_input.rs`](../../crates/aero-virtio/tests/virtio_input.rs)

#### 1.2 Run contract-level virtio PCI checks that virtio-input depends on

These are not “virtio-input only”, but they lock down the **shared** virtio-pci contract that the Win7 driver stack depends on.

```bash
cargo test -p aero-virtio --locked --test win7_contract_queue_sizes
cargo test -p aero-virtio --locked --test win7_contract_ring_features
cargo test -p aero-virtio --locked --test win7_contract_dma_64bit
cargo test -p aero-virtio --locked --test pci_profile
cargo test -p aero-virtio --locked --test pci_bar0_mmio_access_sizes
cargo test -p aero-devices --locked --test pci_virtio_input_multifunction
cargo test -p aero-devices --locked --test windows_device_contract_virtio_input
cargo test -p aero-wasm --locked --test virtio_input_pci_device_core
```

Recommended (enforces repo resource limits via `safe-run.sh`):

```bash
bash ./scripts/safe-run.sh cargo test -p aero-virtio --locked --test win7_contract_queue_sizes
bash ./scripts/safe-run.sh cargo test -p aero-virtio --locked --test win7_contract_ring_features
bash ./scripts/safe-run.sh cargo test -p aero-virtio --locked --test win7_contract_dma_64bit
bash ./scripts/safe-run.sh cargo test -p aero-virtio --locked --test pci_profile
bash ./scripts/safe-run.sh cargo test -p aero-virtio --locked --test pci_bar0_mmio_access_sizes
bash ./scripts/safe-run.sh cargo test -p aero-devices --locked --test pci_virtio_input_multifunction
bash ./scripts/safe-run.sh cargo test -p aero-devices --locked --test windows_device_contract_virtio_input
bash ./scripts/safe-run.sh cargo test -p aero-wasm --locked --test virtio_input_pci_device_core
```

#### 1.3 What to expect (key invariants)

These expectations should match **exactly** what is specified in:
[`windows7-virtio-driver-contract.md`](windows7-virtio-driver-contract.md).

**PCI / virtio-pci modern layout**

- **BAR0 size** is **`0x4000`** bytes and is 64-bit MMIO (contract §1.2).
- PCI config space exposes a valid capability list (status bit 4 + cap ptr at `0x34`), containing the required virtio vendor caps:
  - `COMMON_CFG`, `NOTIFY_CFG`, `ISR_CFG`, `DEVICE_CFG` (contract §1.3).
- Aero contract v1 uses a fixed BAR0 capability layout (contract §1.4):
  - `COMMON_CFG` @ `0x0000`
  - `NOTIFY_CFG` @ `0x1000` (`notify_off_multiplier == 4`)
  - `ISR_CFG` @ `0x2000`
  - `DEVICE_CFG` @ `0x3000`

**virtio-input specifics**

- Device is exposed as a **single multi-function PCI device** (contract §3.3):
  - Vendor/Device: `1AF4:1052` (`VIRTIO_ID_INPUT`)
  - Revision ID: `0x01` (`REV_01`)
  - Keyboard:
    - function **0**
    - subsystem id `0x0010`
    - `header_type = 0x80` (multifunction bit set)
  - Mouse:
    - function **1**
    - subsystem id `0x0011`
  - Tablet (optional):
    - function **2**
    - subsystem id `0x0012`
- Each function exposes exactly **2 split virtqueues** (contract §3.3.2):
  - `eventq` (queue 0): **64**
  - `statusq` (queue 1): **64**
- `eventq` buffers complete with **`used.len = 8`** and contain exactly one `virtio_input_event` (contract §3.3.5).
- `statusq` buffers are always consumed/completed (contents may be ignored) (contract §3.3.5, statusq behavior).
- Device config selector behavior matches virtio-input spec + Aero requirements (contract §3.3.4):
  - `ID_NAME` returns:
    - `"Aero Virtio Keyboard"` / `"Aero Virtio Mouse"` / `"Aero Virtio Tablet"`
  - `ID_DEVIDS` returns BUS_VIRTUAL + virtio vendor + product id
  - `EV_BITS` bitmaps include required types/codes (keyboard vs mouse vs tablet differ).
    - Keyboard `EV_BITS` includes: `EV_SYN`, `EV_KEY`, `EV_LED` and at least `LED_NUML`/`LED_CAPSL`/`LED_SCROLLL` (optionally `LED_COMPOSE`/`LED_KANA`).
    - Mouse `EV_BITS` includes: `EV_SYN`, `EV_KEY`, `EV_REL`.
    - Tablet `EV_BITS` includes: `EV_SYN`, `EV_ABS` and `ABS_X`/`ABS_Y` (and `ABS_INFO` ranges for X/Y).

---

### Windows 7 driver unit tests (portable host-side)

The Win7 virtio-input driver contains several **portable C helpers** that can be tested on any host without the WDK:

- virtio-input → HID input report translation (`virtio_input_event` → HID reports)
- HID keyboard LED output report parsing (ReportID-prefix ambiguity)
- HID keyboard LED bitfield → virtio-input `EV_LED` statusq events

Key sources/tests:

- Translator: [`drivers/windows7/virtio-input/src/hid_translate.c`](../../drivers/windows7/virtio-input/src/hid_translate.c)
  - Test: [`drivers/windows7/virtio-input/tests/hid_translate_test.c`](../../drivers/windows7/virtio-input/tests/hid_translate_test.c)
- HID LED output report parsing: [`drivers/windows7/virtio-input/src/led_report_parse.c`](../../drivers/windows7/virtio-input/src/led_report_parse.c)
  - Test: [`drivers/windows7/virtio-input/tests/led_report_parse_test.c`](../../drivers/windows7/virtio-input/tests/led_report_parse_test.c)
- HID LED bitfield → virtio statusq events: [`drivers/windows7/virtio-input/src/led_translate.c`](../../drivers/windows7/virtio-input/src/led_translate.c)
  - Test: [`drivers/windows7/virtio-input/tests/led_translate_test.c`](../../drivers/windows7/virtio-input/tests/led_translate_test.c)
  - End-to-end parse+translate: [`drivers/windows7/virtio-input/tests/led_output_pipeline_test.c`](../../drivers/windows7/virtio-input/tests/led_output_pipeline_test.c)

See also: [`drivers/windows7/virtio-input/tests/README.md`](../decisions/README.md)

#### 2.1 Build + run (gcc / clang)

Run the full suite:

```bash
cd drivers/windows7/virtio-input
bash tests/run.sh
```

Or build a single test manually (from the repo root):

```bash
cd drivers/windows7/virtio-input/tests

# gcc
gcc -std=c11 -Wall -Wextra -Werror \
  -o /tmp/hid_translate_test \
  hid_translate_test.c ../src/hid_translate.c && /tmp/hid_translate_test

# clang (equivalent)
clang -std=c11 -Wall -Wextra -Werror \
  -o /tmp/hid_translate_test \
  hid_translate_test.c ../src/hid_translate.c && /tmp/hid_translate_test
```

Expected output:

```text
hid_translate_test: ok
```

#### 2.2 Explicit mapping check: F1..F12, NumLock, ScrollLock

The translator **must** map Linux `KEY_F1..KEY_F12` (virtio-input EV_KEY codes) to the correct HID keyboard usages (`0x3A..0x45`).

It must also map the lock keys used by Windows LED state:

- `KEY_NUMLOCK` → HID usage `0x53`
- `KEY_SCROLLLOCK` → HID usage `0x47`

This is required by the contract (virtio-input keyboard “minimum required supported key codes” includes **F1..F12**; contract §3.3.5).

The host-side unit test asserts these mappings. If this fails, fix the mapping in:

- `drivers/windows7/virtio-input/src/hid_translate.c`

---

### Windows 7 driver QEMU manual test (shortest path)

Full reference:

- [`drivers/windows7/virtio-input/tests/qemu/README.md`](../decisions/README.md)

#### 3.0 Build/package/sign the driver (host)

QEMU bring-up requires an installable driver package directory containing:

- `aero_virtio_input.inf`
- `aero_virtio_input.sys`
- `aero_virtio_input.cat` (recommended; required for signature verification)
- (optional, tablet / absolute pointer) `aero_virtio_tablet.inf`
- (optional, tablet / absolute pointer) `aero_virtio_tablet.cat`

See the canonical driver README for the full build + signing workflow and CI output paths:

- [`drivers/windows7/virtio-input/README.md`](../decisions/README.md)

If you built via CI scripts, the packaged outputs are typically staged under:

- `out/packages/windows7/virtio-input/x86/`
- `out/packages/windows7/virtio-input/x64/`

#### 3.1 Boot QEMU with virtio-input devices

Example (x64), keeping PS/2 enabled during installation so you don’t lose input:

```bash
qemu-system-x86_64 \
  -machine pc,accel=kvm \
  -m 4096 \
  -cpu qemu64 \
  -drive file=win7-x64.qcow2,if=ide,format=qcow2 \
  -device virtio-keyboard-pci,disable-legacy=on,x-pci-revision=0x01 \
  -device virtio-mouse-pci,disable-legacy=on,x-pci-revision=0x01 \
  -net nic,model=e1000 -net user
```

Notes:

- `x-pci-revision=0x01` is required for the **Aero Win7 contract v1** (`REV_01`) drivers to bind.
- Stock QEMU virtio-input devices often report non-Aero `ID_NAME` strings (for example `QEMU Virtio Keyboard`).
  The in-tree Aero virtio-input driver is strict by default and may refuse to start (Code 10 / `STATUS_NOT_SUPPORTED`)
  unless you enable compat mode (`CompatIdName=1`) in the guest or use a contract-compliant virtio-input device model.
  See `drivers/windows7/virtio-input/docs/virtio-input-notes.md`.

#### 3.2 Install the test certificate + driver in the Win7 guest

Inside the guest:

1. Enable test signing (Admin CMD), then reboot:
   ```bat
   bcdedit /set testsigning on
   ```
2. Install the driver signing certificate used by your build into:
   - **Trusted Root Certification Authorities**
   - **Trusted Publishers**

   See `drivers/windows7/virtio-input/README.md` for the in-tree test-signing workflow (make-cert → install-test-cert → make-cat → sign-driver).
3. Install the driver via **Device Manager**:
   - Find the virtio-input PCI device(s) (often show as unknown before binding)
   - Update Driver → Have Disk… → point at the directory containing:
      - `aero_virtio_input.inf`
      - `aero_virtio_input.sys`
      - `aero_virtio_input.cat`

#### 3.3 Verify HID keyboard + mouse enumeration

In Device Manager after install/reboot:

- **Keyboards** contains a **HID Keyboard Device** (driver stack includes `kbdhid.sys`, `hidclass.sys`).
- **Mice and other pointing devices** contains a **HID-compliant mouse** (driver stack includes `mouhid.sys`, `hidclass.sys`).

(Optional but recommended) Run `hidtest.exe` as described in the QEMU README to validate raw input reports.

- Tool source/build instructions: [`drivers/windows7/virtio-input/tools/hidtest/README.md`](../decisions/README.md)

Tip: `hidtest.exe --counters` can help diagnose “input buffered while no pending READ_REPORT IRPs” behavior:

- `PendingRingDepth`/`PendingRingDrops`: READ_REPORT backlog in `DEVICE_CONTEXT.PendingReportRing[]` (primary buffering layer).
- Compare with `ReportRingDepth`/`ReportRingDrops`: translation-layer ring (`virtio_input_device.report_ring`).

Tip: `hidtest.exe --interrupt-info` / `--interrupt-info --json` / `--interrupt-info-json` can help diagnose the effective interrupt mode (INTx vs MSI-X),
message count granted by Windows, and MSI-X vector routing (config vs per-queue).

Tip: keyboard LED output reports exercise the **statusq** (driver → device) path. Useful probes:

- `hidtest.exe --keyboard --led-cycle` (cycles the 5 HID boot keyboard LED bits: Num/Caps/Scroll/Compose/Kana)
- `hidtest.exe --keyboard --led 0x1F` (sets all 5 bits)

To stress backpressure/coalescing, use `hidtest.exe --keyboard --led-spam N` (alternates `0` and `0x1F` by default; override the "on" mask via `--led 0xMASK` / `--led-hidd` / `--led-ioctl-set-output`) and watch `hidtest.exe --counters`.

Tip: `hidtest.exe --keyboard --state` / `--state --json` / `--state-json` shows the driver’s statusq/LED configuration (`StatusQActive`, `StatusQDropOnFull`, and `KeyboardLedSupportedMask`).

After the driver is installed and confirmed working, you can optionally disable PS/2 in QEMU (`-machine ...,i8042=off`) to ensure you are not accidentally testing the emulated PS/2 devices. Only do this once you have a known-good virtio-input driver; otherwise you may lose input in the guest.

#### 3.4 Expected pass/fail signals (Windows 7)

These are common “first look” signals when validating the driver end-to-end:

- **Before driver install (expected):**
  - Device Manager shows the virtio-input PCI function(s) under **Other devices** (often as “PCI Device”).
  - Typical state: **Code 28** (“drivers for this device are not installed”).
- **After successful install (expected):**
  - The virtio-input PCI function(s) bind to `aero_virtio_input.sys` and appear under **Human Interface Devices** (`HIDClass`).
  - When the guest exposes the Aero subsystem IDs, the two functions should have distinct names under **Human Interface Devices**:
    - **Aero VirtIO Keyboard** (`SUBSYS_00101AF4`)
    - **Aero VirtIO Mouse** (`SUBSYS_00111AF4`)
  - A HID keyboard and HID mouse are present and use the in-box HID stacks:
    - Keyboard: **Keyboards → HID Keyboard Device** (`kbdhid.sys`, `hidclass.sys`, `hidparse.sys`)
    - Mouse: **Mice and other pointing devices → HID-compliant mouse** (`mouhid.sys`, `hidclass.sys`, `hidparse.sys`)
- **Common failures:**
  - **Code 52**: Windows cannot verify the driver signature (test signing/cert install issue).
  - **Code 10**: device cannot start (often contract mismatch: wrong `REV`, wrong virtio-pci caps/layout, or wrong virtio-input `ID_NAME` strings).

For detailed troubleshooting (including QEMU-specific notes), see:

- [`drivers/windows7/virtio-input/tests/qemu/README.md`](../decisions/README.md)

---

### Windows 7 automated host harness (QEMU + guest selftest)

Full reference:

- [`drivers/windows7/tests/host-harness/README.md`](../decisions/README.md)

Optional self-hosted CI wrapper:

- ``.github/workflows/win7-virtio-harness.yml`` (`workflow_dispatch`)
  - Use workflow inputs:
    - `qemu_preflight_pci=true` to enable the optional QMP `query-pci` PCI ID preflight (fail fast on missing `REV_01`/wrong IDs)
    - To enable the optional QMP injection-based end-to-end virtio-input tests:
      - `with_virtio_input_events=true`
      - `with_virtio_input_wheel=true`
      - `with_virtio_input_media_keys=true`
      - `with_virtio_input_events_extended=true`
      - `with_virtio_input_tablet_events=true`
      - (Requires a guest image provisioned with `--test-input-events` for events/wheel, also `--test-input-media-keys` for media keys,
        also `--test-input-events-extended` for the extended markers, and `--test-input-tablet-events` (alias: `--test-tablet-events`)
        for tablet.)
    - To require the optional virtio-input LED/statusq smoke test:
      - `with_virtio_input_led=true`
      - (Requires a guest image provisioned with `--test-input-led` or env var `AERO_VIRTIO_SELFTEST_TEST_INPUT_LED=1`.)
    - To enable the optional virtio-blk runtime resize test (`virtio-blk-resize`):
      - `with_blk_resize=true`
      - optional: `blk_resize_delta_mib=<N>` to override the default growth delta (MiB)
      - (Requires a guest image provisioned with `--test-blk-resize` / env var `AERO_VIRTIO_SELFTEST_TEST_BLK_RESIZE=1`.)

#### 4.1 Basic invocation (PowerShell)

```powershell
pwsh ./drivers/windows7/tests/host-harness/Invoke-AeroVirtioWin7Tests.ps1 `
  -QemuSystem qemu-system-x86_64 `
  -DiskImagePath ./win7-aero-tests.qcow2 `
  -SerialLogPath ./win7-serial.log `
  -Snapshot `
  -TimeoutSeconds 600
```

#### 4.2 Basic invocation (Python, Linux-friendly)

```bash
python3 drivers/windows7/tests/host-harness/invoke_aero_virtio_win7_tests.py \
  --qemu-system qemu-system-x86_64 \
  --disk-image ./win7-aero-tests.qcow2 \
  --serial-log ./win7-serial.log \
  --timeout-seconds 600 \
  --snapshot
```

Success looks like:

- Harness exit code `0`
- Serial log contains `AERO_VIRTIO_SELFTEST|TEST|virtio-input|PASS`
- Serial log contains `AERO_VIRTIO_SELFTEST|TEST|virtio-input-bind|PASS` (verifies the underlying virtio-input PCI function(s) are bound to the expected driver service and have no PnP errors)

#### 4.3 Optional: MSI-X interrupt mode diagnostics / enforcement (`virtio-input-msix`)

The Aero contract v1 requires INTx and permits MSI-X as an optional enhancement. When debugging MSI-X enablement (or when
intentionally exercising MSI-X code paths under QEMU), the guest selftest may emit:

- `AERO_VIRTIO_SELFTEST|TEST|virtio-input-msix|PASS/FAIL/SKIP|mode=intx/msix/unknown|messages=<n>|mapping=...|used_vectors=<n>|config_vector=<n\|none>|queue0_vector=<n\|none>|queue1_vector=<n\|none>|...`
  - If the diagnostics IOCTL is unavailable/unsupported, the marker is emitted as `SKIP|reason=ioctl_not_supported|...`
    by default (or `FAIL|reason=...|...` when the guest selftest is provisioned with `--require-input-msix`).

To make MSI-X a **hard harness requirement** (end-to-end, guest-reported effective mode):

- PowerShell: `-RequireVirtioInputMsix` *(alias: `-RequireInputMsix`)*
- Python: `--require-virtio-input-msix` *(alias: `--require-input-msix`)*

Optional guest-side hard requirement (fail-fast; makes the guest selftest RESULT fail when `virtio-input-msix` reports `mode!=msix`):

- Guest selftest: `--require-input-msix` (or env var `AERO_VIRTIO_SELFTEST_REQUIRE_INPUT_MSIX=1`)
  - When provisioning via `New-AeroWin7TestImage.ps1`, use `-RequireInputMsix`.

To increase the chance that Windows grants enough MSI-X messages, request a larger MSI-X table size from QEMU
(requires QEMU virtio `vectors` property; the host harness fails fast if unsupported):

- PowerShell: `-VirtioInputVectors N` (or global `-VirtioMsixVectors N`)
- Python: `--virtio-input-vectors N` (or global `--virtio-msix-vectors N`)

#### 4.4 Optional: keyboard LED/statusq smoke test (guest HID output report write)

AERO-W7-VIRTIO v1 requires virtio-input to consume and complete all `statusq` descriptors (contents may be ignored).
The guest selftest can exercise this end-to-end by sending HID keyboard LED **output reports** (CapsLock/NumLock/ScrollLock),
which the driver translates into `EV_LED` events on `statusq`.

Guest marker:

- `AERO_VIRTIO_SELFTEST|TEST|virtio-input-led|PASS/FAIL/SKIP|...`

Guest image requirement:

- Provision the guest selftest to run with `--test-input-led` (or set guest env var `AERO_VIRTIO_SELFTEST_TEST_INPUT_LED=1`).

Host harness flags (marker requirement only; no QMP injection needed):

- PowerShell: `-WithInputLed`
- Python: `--with-input-led`

PowerShell:

```powershell
pwsh ./drivers/windows7/tests/host-harness/Invoke-AeroVirtioWin7Tests.ps1 `
  -QemuSystem qemu-system-x86_64 `
  -DiskImagePath ./win7-aero-tests.qcow2 `
  -WithInputLed `
  -TimeoutSeconds 600 `
  -Snapshot
```

Python:

```bash
python3 drivers/windows7/tests/host-harness/invoke_aero_virtio_win7_tests.py \
  --qemu-system qemu-system-x86_64 \
  --disk-image ./win7-aero-tests.qcow2 \
  --with-input-led \
  --timeout-seconds 600 \
  --snapshot
```

If the guest was not provisioned with `--test-input-led`, the guest will emit:
`AERO_VIRTIO_SELFTEST|TEST|virtio-input-led|SKIP|flag_not_set` and the harness will fail when `-WithInputLed` / `--with-input-led` is enabled
(PowerShell: `VIRTIO_INPUT_LED_SKIPPED`; Python: `FAIL: VIRTIO_INPUT_LED_SKIPPED: ...`).

If the guest reports `AERO_VIRTIO_SELFTEST|TEST|virtio-input-led|FAIL|...`, the harness fails
(PowerShell: `VIRTIO_INPUT_LED_FAILED`; Python: `FAIL: VIRTIO_INPUT_LED_FAILED: ...`).

If the guest selftest is too old (or misconfigured) and does not emit any `virtio-input-led` marker at all
(PASS/SKIP/FAIL) after completing `virtio-input`, the harness fails early
(PowerShell: `MISSING_VIRTIO_INPUT_LED`; Python: `FAIL: MISSING_VIRTIO_INPUT_LED: ...`).

#### 4.5 Optional: end-to-end input event delivery (QMP injection + guest HID report read)

The default guest marker `virtio-input` validates **enumeration** and the **HID report descriptor** contract (keyboard-only + mouse-only devices).
It does **not** prove that real virtio-input events (virtio queues → KMDF HID → user-mode) are delivered.

To validate actual event delivery deterministically, use the optional guest marker:

- `AERO_VIRTIO_SELFTEST|TEST|virtio-input-events|READY/PASS/FAIL/...`

Guest image requirement:

- Provision the guest selftest to run with `--test-input-events` (or set guest env var `AERO_VIRTIO_SELFTEST_TEST_INPUT_EVENTS=1`).
  - The host-harness README describes one way to do this via `New-AeroWin7TestImage.ps1 -TestInputEvents`.

PowerShell:

```powershell
pwsh ./drivers/windows7/tests/host-harness/Invoke-AeroVirtioWin7Tests.ps1 `
  -QemuSystem qemu-system-x86_64 `
  -DiskImagePath ./win7-aero-tests.qcow2 `
  -WithInputEvents `
  -TimeoutSeconds 600 `
  -Snapshot
```

Python:

```bash
python3 drivers/windows7/tests/host-harness/invoke_aero_virtio_win7_tests.py \
  --qemu-system qemu-system-x86_64 \
  --disk-image ./win7-aero-tests.qcow2 \
  --with-input-events \
  --timeout-seconds 600 \
  --snapshot
```

When enabled, the harness:

1. Waits for the guest readiness marker: `AERO_VIRTIO_SELFTEST|TEST|virtio-input-events|READY`
2. Injects a deterministic input sequence via QMP (prefers `input-send-event`, with backcompat fallbacks when unavailable):
   - keyboard: `'a'` press + release
   - mouse: relative move + left click
3. Requires the guest marker `AERO_VIRTIO_SELFTEST|TEST|virtio-input-events|PASS|...`

Expected signal:

- Guest serial contains `AERO_VIRTIO_SELFTEST|TEST|virtio-input-events|PASS|...`
- Host harness logs include a marker like:
  `AERO_VIRTIO_WIN7_HOST|VIRTIO_INPUT_EVENTS_INJECT|PASS|attempt=<n>|backend=<qmp_input_send_event|hmp_fallback>|kbd_mode=device/broadcast|mouse_mode=device/broadcast`
  - Note: The harness may retry injection a few times after `virtio-input-events|READY` to reduce timing flakiness.
    In that case you may see multiple `VIRTIO_INPUT_EVENTS_INJECT|PASS` lines (the marker includes `attempt=<n>` and `backend=...`).

If the guest was not provisioned with `--test-input-events`, the guest will emit:
`AERO_VIRTIO_SELFTEST|TEST|virtio-input-events|SKIP|flag_not_set` and the harness will fail when `-WithInputEvents` / `--with-input-events` is enabled
(PowerShell: `VIRTIO_INPUT_EVENTS_SKIPPED`; Python: `FAIL: VIRTIO_INPUT_EVENTS_SKIPPED: ...`).

If the guest reports `AERO_VIRTIO_SELFTEST|TEST|virtio-input-events|FAIL|...`, the harness fails
(PowerShell: `VIRTIO_INPUT_EVENTS_FAILED`; Python: `FAIL: VIRTIO_INPUT_EVENTS_FAILED: ...`).

If the guest selftest is too old (or otherwise misconfigured) and does not emit any `virtio-input-events` marker at all
(READY/SKIP/PASS/FAIL) after completing `virtio-input`, the harness fails early
(PowerShell: `MISSING_VIRTIO_INPUT_EVENTS`; Python: `FAIL: MISSING_VIRTIO_INPUT_EVENTS: ...`). Update/re-provision the guest selftest binary.

If QMP input injection fails (for example QMP is unreachable or the QEMU build does not support any supported input injection mechanism),
the harness fails (PowerShell: `QMP_INPUT_INJECT_FAILED`; Python: `FAIL: QMP_INPUT_INJECT_FAILED: ...`).

#### 4.5.1 Optional: end-to-end mouse wheel + horizontal wheel (QMP injection + guest HID report read)

The base `virtio-input-events` marker validates keyboard + relative mouse motion/click delivery.
To also validate **mouse scrolling**, the guest selftest emits an additional marker:

- `AERO_VIRTIO_SELFTEST|TEST|virtio-input-wheel|PASS/FAIL/SKIP|...`

Notes:

- The wheel marker is emitted as part of the `--test-input-events` flow; the guest does not need a separate flag.
- The host harness only requires `virtio-input-wheel|PASS` when the wheel injection flag is enabled.

Host harness flags:

- PowerShell: `-WithInputWheel` (aliases: `-WithVirtioInputWheel`, `-EnableVirtioInputWheel`)
- Python: `--with-input-wheel` (aliases: `--with-virtio-input-wheel`, `--require-virtio-input-wheel`, `--enable-virtio-input-wheel`)

When enabled, the harness:

1. Enables the base input-events injection (as if `-WithInputEvents` / `--with-input-events` were set)
2. Injects a deterministic scroll sequence via QMP `input-send-event`:
   - vertical wheel: `axis=wheel`, `value=+1` (with fallback `axis=vscroll` for alternate QEMU builds)
   - horizontal wheel: `axis=hscroll`, `value=-2` (with fallback `axis=hwheel` for older/alternate QEMU builds)
3. Requires the guest marker `AERO_VIRTIO_SELFTEST|TEST|virtio-input-wheel|PASS|...`

Note: The harness may retry injection a few times after `virtio-input-events|READY` to reduce timing flakiness. In that
case the guest may observe multiple injected scroll events; the wheel selftest is designed to handle this, and totals
may be multiples of the injected values.

If the running QEMU build rejects all tested axis name combinations (`wheel`/`vscroll` × `hscroll`/`hwheel`), the harness
fails with a clear error (upgrade QEMU or omit `-WithInputWheel` / `--with-input-wheel` (or aliases)).

#### 4.5.2 Optional: end-to-end Consumer Control (media keys) (QMP injection + guest HID report read)

This is the Consumer Control / media keys companion to `virtio-input-events` (keyboard + relative mouse).

Guest marker:

- `AERO_VIRTIO_SELFTEST|TEST|virtio-input-media-keys|READY/PASS/FAIL/...`

Guest image requirement:

- Provision the guest selftest to run with `--test-input-media-keys` (or set guest env var
  `AERO_VIRTIO_SELFTEST_TEST_INPUT_MEDIA_KEYS=1`).
  - The host-harness README describes one way to do this via `New-AeroWin7TestImage.ps1 -TestInputMediaKeys` (alias: `-TestMediaKeys`).

Host harness flags:

- PowerShell: `-WithInputMediaKeys` (aliases: `-WithVirtioInputMediaKeys`, `-EnableVirtioInputMediaKeys`)
- Python: `--with-input-media-keys` (aliases: `--with-virtio-input-media-keys`, `--enable-virtio-input-media-keys`)

When enabled, the harness:

1. Waits for the guest readiness marker: `AERO_VIRTIO_SELFTEST|TEST|virtio-input-media-keys|READY`
2. Injects a deterministic media key sequence via QMP (prefers `input-send-event`, with backcompat fallbacks when unavailable):
   - `qcode=volumeup` press + release
3. Requires the guest marker `AERO_VIRTIO_SELFTEST|TEST|virtio-input-media-keys|PASS|...`

Expected signal:

- Guest serial contains `AERO_VIRTIO_SELFTEST|TEST|virtio-input-media-keys|PASS|...`
- Host harness logs include a marker like:
  `AERO_VIRTIO_WIN7_HOST|VIRTIO_INPUT_MEDIA_KEYS_INJECT|PASS|attempt=<n>|backend=<qmp_input_send_event|hmp_fallback>|kbd_mode=device/broadcast`
  - Note: The harness may retry injection a few times after `virtio-input-media-keys|READY` to reduce timing flakiness.
    In that case you may see multiple `VIRTIO_INPUT_MEDIA_KEYS_INJECT|PASS` lines (the marker includes `attempt=<n>` and `backend=...`).

If the guest was not provisioned with `--test-input-media-keys`, the guest will emit:
`AERO_VIRTIO_SELFTEST|TEST|virtio-input-media-keys|SKIP|flag_not_set` and the harness will fail
(PowerShell: `VIRTIO_INPUT_MEDIA_KEYS_SKIPPED`; Python: `FAIL: VIRTIO_INPUT_MEDIA_KEYS_SKIPPED: ...`).

If QMP input injection fails (for example QMP is unreachable or the QEMU build does not support multimedia qcodes), the harness
fails (PowerShell: `QMP_MEDIA_KEYS_UNSUPPORTED`; Python: `FAIL: QMP_MEDIA_KEYS_UNSUPPORTED: ...`).

#### 4.5.3 Optional: extended virtio-input events (modifiers + extra buttons + per-feature wheel markers)

The base `virtio-input-events` marker validates basic keyboard + mouse motion/click delivery, and `virtio-input-wheel`
validates scrolling. To also validate additional HID report paths deterministically, the guest selftest can emit three
extra markers:

- `AERO_VIRTIO_SELFTEST|TEST|virtio-input-events-modifiers|PASS/FAIL/SKIP|...` (Shift/Ctrl/Alt + F1)
- `AERO_VIRTIO_SELFTEST|TEST|virtio-input-events-buttons|PASS/FAIL/SKIP|...` (mouse side/extra buttons)
- `AERO_VIRTIO_SELFTEST|TEST|virtio-input-events-wheel|PASS/FAIL/SKIP|...` (wheel + horizontal wheel)

Guest image requirement:

- Provision the guest selftest to run with `--test-input-events-extended` or set guest env var
  `AERO_VIRTIO_SELFTEST_TEST_INPUT_EVENTS_EXTENDED=1`.
  - If provisioning via `New-AeroWin7TestImage.ps1`, pass `-TestInputEventsExtended` (alias: `-TestInputEventsExtra`).
  - This is in addition to enabling the base `--test-input-events` flow; the harness will fail if `virtio-input-events`
    is skipped.

Host harness flags:

- PowerShell: `-WithInputEventsExtended` (alias: `-WithInputEventsExtra`)
- Python: `--with-input-events-extended` (alias: `--with-input-events-extra`)

When enabled, the harness:

1. Enables the base input-events injection (as if `-WithInputEvents` / `--with-input-events` were set)
2. Injects an extended deterministic sequence via QMP `input-send-event` (modifiers + side/extra buttons + wheel)
3. Requires all three `virtio-input-events-*` markers above to PASS

#### 4.6 Optional: end-to-end tablet (absolute pointer) event delivery (QMP injection + guest HID report read)

This is the tablet/absolute-pointer companion to `virtio-input-events` (keyboard + relative mouse).

Guest marker:

- `AERO_VIRTIO_SELFTEST|TEST|virtio-input-tablet-events|READY/PASS/FAIL/...`

Guest image requirement:

- Provision the guest selftest to run with `--test-input-tablet-events` (alias: `--test-tablet-events`) or set guest env
  var `AERO_VIRTIO_SELFTEST_TEST_INPUT_TABLET_EVENTS=1` / `AERO_VIRTIO_SELFTEST_TEST_TABLET_EVENTS=1`.
  - The host-harness README describes one way to do this via `New-AeroWin7TestImage.ps1 -TestInputTabletEvents` (alias: `-TestTabletEvents`).
- Ensure the guest has a virtio-input **tablet** driver installed and bound (so the tablet exposes a HID interface). For
  the in-tree Aero driver stack this is `drivers/windows7/virtio-input/inf/aero_virtio_tablet.inf` (it installs the
  shared `aero_virtio_input.sys` binary but matches the tablet HWID). When provisioning via `New-AeroWin7TestImage.ps1`,
  the tablet INF is installed by default when present; if you pass an explicit `-InfAllowList`, ensure it includes
  `aero_virtio_tablet.inf`.

PowerShell:

```powershell
pwsh ./drivers/windows7/tests/host-harness/Invoke-AeroVirtioWin7Tests.ps1 `
  -QemuSystem qemu-system-x86_64 `
  -DiskImagePath ./win7-aero-tests.qcow2 `
  -WithTabletEvents `
  -TimeoutSeconds 600 `
  -Snapshot
```

Python:

```bash
python3 drivers/windows7/tests/host-harness/invoke_aero_virtio_win7_tests.py \
  --qemu-system qemu-system-x86_64 \
  --disk-image ./win7-aero-tests.qcow2 \
  --with-tablet-events \
  --timeout-seconds 600 \
  --snapshot
```

To attach the `virtio-tablet-pci` device **without** QMP injection / marker enforcement (for example to validate
enumeration only), run the harness with:

- PowerShell: `-WithVirtioTablet`
- Python: `--with-virtio-tablet`

When enabled, the harness:

1. Waits for the guest readiness marker: `AERO_VIRTIO_SELFTEST|TEST|virtio-input-tablet-events|READY`
2. Injects a deterministic input sequence via QMP `input-send-event` (required; there is no widely-supported absolute-pointer fallback):
   - move to (0,0) (reset)
   - move to (10000,20000) (target)
   - left click down + up
3. Requires the guest marker `AERO_VIRTIO_SELFTEST|TEST|virtio-input-tablet-events|PASS|...`

Expected signal:

- Guest serial contains `AERO_VIRTIO_SELFTEST|TEST|virtio-input-tablet-events|PASS|...`
- Host harness logs include a marker like:
  `AERO_VIRTIO_WIN7_HOST|VIRTIO_INPUT_TABLET_EVENTS_INJECT|PASS|attempt=<n>|backend=<qmp_input_send_event>|tablet_mode=device/broadcast`

If the guest was not provisioned with `--test-input-tablet-events` / `--test-tablet-events`, the guest will emit:
`AERO_VIRTIO_SELFTEST|TEST|virtio-input-tablet-events|SKIP|flag_not_set` and the harness will fail when `-WithInputTabletEvents` / `-WithTabletEvents` / `--with-input-tablet-events` / `--with-tablet-events` is enabled
(PowerShell: `VIRTIO_INPUT_TABLET_EVENTS_SKIPPED`; Python: `FAIL: VIRTIO_INPUT_TABLET_EVENTS_SKIPPED: ...`).

If the guest reports `AERO_VIRTIO_SELFTEST|TEST|virtio-input-tablet-events|FAIL|...`, the harness fails
(PowerShell: `VIRTIO_INPUT_TABLET_EVENTS_FAILED`; Python: `FAIL: VIRTIO_INPUT_TABLET_EVENTS_FAILED: ...`).

If the guest selftest is too old (or otherwise misconfigured) and does not emit any `virtio-input-tablet-events` marker at all
(READY/SKIP/PASS/FAIL) after completing `virtio-input`, the harness fails early
(PowerShell: `MISSING_VIRTIO_INPUT_TABLET_EVENTS`; Python: `FAIL: MISSING_VIRTIO_INPUT_TABLET_EVENTS: ...`). Update/re-provision the guest selftest binary.

If QMP tablet injection fails, the harness fails
(PowerShell: `QMP_INPUT_TABLET_INJECT_FAILED`; Python: `FAIL: QMP_INPUT_TABLET_INJECT_FAILED: ...`).

---

### Web runtime validation (browser → virtio-input routing)

Goal: validate that browser keyboard/mouse events are routed through the correct virtual device:

- **before** the Win7 virtio-input driver sets `DRIVER_OK`: PS/2 or USB fallback
- **after** `DRIVER_OK`: virtio-input

#### 5.1 Bring up the web runtime

From the repo root:

```bash
# One-time setup (Node deps for the `apps/web/` workspace + repo-root harness):
pnpm install --frozen-lockfile

# Ensure the Rust→WASM packages exist (virtio-input uses the WASM device model).
# This builds any missing packages (single + threaded) via the `apps/web/` workspace scripts.
#
# If this fails due to missing `wasm-pack` / pinned Rust toolchains, the easiest fix is:
#   just setup
pnpm run wasm:ensure

cargo xtask web dev
```

Equivalent:

```bash
pnpm run dev
```

Note: when running the repo-root Vite harness (`cargo xtask web dev` / `pnpm run dev`), the web runtime is served at `/web/`.

Open it with a verbose log level (example):

```text
http://localhost:5173/web/?log=debug
```

Note: most multi-worker test/debug flows (SharedArrayBuffer, ring buffers, etc.) require `crossOriginIsolated` to be true (COOP/COEP headers).

#### 5.2 Validate virtio-input PCI exposure (IDs / caps / BAR0) in the browser runtime

Once virtio-input is wired into the web runtime PCI bus, validate the device is exposed exactly as required by:

- `windows7-virtio-driver-contract.md` (AERO-W7-VIRTIO v1), especially:
  - §1.3 PCI caps
  - §1.4 fixed BAR0 layout
  - §3.3 virtio-input

##### What to check (contract v1)

- Two PCI functions with **Vendor/Device** `1AF4:1052`:
  - function 0: keyboard (`SUBSYS 1AF4:0010`, `header_type = 0x80` multifunction)
  - function 1: mouse (`SUBSYS 1AF4:0011`)
- **Revision ID** `0x01` (`REV_01`) on both functions.
- **BAR0**: 64-bit MMIO, size `0x4000` bytes (via standard PCI BAR sizing probe).
- **Capability list** is present (PCI Status bit 4 set; cap pointer at `0x34`) and contains the required virtio vendor-specific capabilities:
  - `cfg_type = 1` COMMON
  - `cfg_type = 2` NOTIFY (must include `notify_off_multiplier = 4`)
  - `cfg_type = 3` ISR
  - `cfg_type = 4` DEVICE

##### How to check (practical approach)

If you're working on the **TypeScript PCI device implementation** itself (as opposed to the full runtime wiring), the fastest check is the unit test:

```bash
# Runs in the `apps/web/` workspace (Vitest).
pnpm -C apps/web run test:unit -- src/io/devices/virtio_input_pci.test.ts
```

Expected signal:

- **Pass:** test exits with code `0`.
- **Fail:** the assertion output will usually point directly at the drift (wrong IDs, wrong cap chain offsets, wrong BAR0 size, etc).

Additionally, to validate the **browser key mapping** used by virtio-input injection (DOM `KeyboardEvent.code` → HID usage → Linux `KEY_*` codes), run:

```bash
pnpm -C apps/web run test:unit -- src/io/devices/virtio_input_keymap.test.ts
```

This is a fast regression check that contract-required keys like **F1..F12**, `NumLock`, and `ScrollLock` map correctly end-to-end.

If you want an end-to-end **web runtime** smoke test (TypeScript PCI device wrapper + WASM bridge + BAR0 MMIO + virtqueue event injection), run:

```bash
pnpm -C apps/web run test:unit -- src/io/devices/virtio_input_pci_integration.test.ts
```

Note: this test will **SKIP** if the WASM packages are missing; run `pnpm run wasm:ensure` first.

Use the same “CPU ↔ IO worker” technique used by the existing PCI tests to read config space via PCI config mechanism #1 (ports `0xCF8`/`0xCFC`):

- [`tests/e2e/io_worker_i8042.spec.ts`](../../tests/e2e/io_worker_i8042.spec.ts) (see “PCI config + BAR-backed MMIO dispatch”)

At a high level:

1. From a CPU-context (or a small debug worker), write `0xCF8` to select a B/D/F + config register dword:

   ```text
   0x8000_0000 | (bus<<16) | (device<<11) | (function<<8) | (reg & 0xFC)
   ```

2. Read `0xCFC..0xCFF` to get the config dword/word/byte.
3. Scan bus 0 for `vendor_id == 0x1AF4` and `device_id == 0x1052`.
4. For BAR0 sizing:
   - write `0xFFFF_FFFF` to BAR0 low dword (and high dword for 64-bit BARs), then read back the mask and compute size.
   - expected size: `0x4000`.
5. Walk the PCI capability list (starting at config offset `0x34`) and confirm virtio vendor caps and their BAR/offset layout.
6. If you need to read actual virtio MMIO registers via BAR0 (e.g. to check `device_status` / `DRIVER_OK`), ensure PCI **memory space decoding** is enabled (PCI command bit1 = `0x2`).
   - The end-to-end PCI test in [`tests/e2e/io_worker_i8042.spec.ts`](../../tests/e2e/io_worker_i8042.spec.ts) includes an example of enabling mem decoding before issuing BAR-backed MMIO reads/writes.

Expected signal:

- **Pass:** the virtio-input keyboard and mouse functions enumerate and match the contract values above.
- **Fail:** any drift here usually means the Win7 driver won’t bind (INF is revision-gated) or won’t start (cap parsing/layout checks fail).

#### 5.3 Boot a Win7 image with the virtio-input driver installed

1. Boot a Windows 7 image that already has the Aero virtio-input driver installed and working (see §3).
2. In the Windows guest, confirm the virtio-input devices enumerate (Device Manager):
   - HID keyboard and HID mouse present (or at least the virtio-input PCI functions are present and the driver service is started).

#### 5.4 Verify routing switch happens at `DRIVER_OK`

In the **web runtime** (`vmRuntime=legacy`), input auto-routing is implemented by the IO worker:

- [`apps/web/src/workers/io.worker.ts`](../../apps/web/src/workers/io.worker.ts)
  - `maybeUpdateKeyboardInputBackend` / `maybeUpdateMouseInputBackend`
  - It prefers virtio-input only when `virtioInputKeyboard?.driverOk()` / `virtioInputMouse?.driverOk()` becomes true.

> Machine runtime note: in `vmRuntime=machine` the CPU worker runs the canonical `api.Machine` and receives input batches directly.
> The legacy IO-worker backend selection policy described in this section does not currently apply to machine runtime.

The backend selection policy is factored into a small pure helper (with unit tests), which is useful when refactoring routing behavior:

- [`apps/web/src/input/input_backend_selection.ts`](../../apps/web/src/input/input_backend_selection.ts)
- Unit test:
  ```bash
  pnpm -C apps/web run test:unit -- src/input/input_backend_selection.test.ts
  ```

Current (browser runtime) selection order, for reference:

- **Keyboard:** virtio-input (when `DRIVER_OK`) → synthetic USB keyboard (once the guest configures it) → PS/2.
- **Mouse:** PS/2 until the synthetic USB mouse is configured (to avoid duplicate active mice), then USB; virtio-input (when `DRIVER_OK`) always wins.
- Backend switching is **gated on held-state** (`keysHeld` / `buttonsHeld`): the worker will not switch while any key or mouse button is held down.

The `driverOk()` signal is derived from the virtio status register:

- [`apps/web/src/io/devices/virtio_input.ts`](../../apps/web/src/io/devices/virtio_input.ts) (`VirtioInputPciFunction::driverOk`)
  - Emits a one-time log when it flips true: `"[virtio-input] keyboard driver_ok"` / `"[virtio-input] mouse driver_ok"`.
- [`crates/aero-wasm/src/virtio_input_bridge.rs`](../../crates/aero-wasm/src/virtio_input_bridge.rs) (`VirtioInputPciDeviceCore::driver_ok`)
  - Reads `device_status` at BAR0+`0x14` and checks `VIRTIO_STATUS_DRIVER_OK` (`0x04`).

Validation steps:

1. While the guest is still booting (or before the virtio-input driver is installed), verify keyboard/mouse still work via the fallback path (PS/2 or USB).
2. After the driver is installed and the guest is fully booted, verify:
   - the driver reaches `DRIVER_OK` (virtio status bit 2, value `0x04`)
   - keyboard/mouse input continues working
3. Ensure no keys/buttons are held (held-state gating), then confirm that new input is being sent via virtio-input (not the fallback path).
   - Practically: after you see the `driver_ok` log(s), release all keys/buttons and generate a fresh input batch (e.g. press+release a key, move the mouse with no buttons held). The backend may not switch *immediately* if the guest hits `DRIVER_OK` mid-press.

If you need an explicit “device-ready” signal, `DRIVER_OK` is the virtio status bit `0x04` in the virtio common config `device_status` register (AERO-W7-VIRTIO v1 uses the common config at BAR0 + `0x0000`, and `device_status` at offset `0x14` within that common config block).

#### 5.5 Recommended debug signal (when validating routing)

For web runtime bring-up, the default debug signal is the one-time console log emitted when the guest sets `DRIVER_OK`:

- `"[virtio-input] keyboard driver_ok"` / `"[virtio-input] mouse driver_ok"` (see `VirtioInputPciFunction::driverOk()` in [`apps/web/src/io/devices/virtio_input.ts`](../../apps/web/src/io/devices/virtio_input.ts)).

Notes:

- This log is emitted via `console.info(...)`; make sure the browser console is showing **Info** logs.
- `DRIVER_OK` indicates the guest driver has finished virtio init, but the web runtime may delay switching until **no keys/buttons are held**.

If you need a *routing* signal (not just `DRIVER_OK`), add a one-line log when the backend changes in:

- [`apps/web/src/workers/io.worker.ts`](../../apps/web/src/workers/io.worker.ts) (`maybeUpdateKeyboardInputBackend` / `maybeUpdateMouseInputBackend`)

Note: the IO worker intentionally avoids switching backends while keys/buttons are held down to prevent “stuck key/button” states in the guest.

---

### Success criteria (summary)

You should be able to validate virtio-input without reading implementation details:

- Rust tests pass (virtio-input + contract-level virtio-pci invariants).
- Win7 driver translator unit test passes, including explicit **F1..F12** mapping.
- Win7 driver binds and enumerates HID keyboard + mouse under QEMU.
- Win7 host harness reports `virtio-input|PASS`.
- Web runtime routes input to virtio-input only after `DRIVER_OK`, with a clear debug signal when it flips.
