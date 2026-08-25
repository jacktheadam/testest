# Windows 7 driver packaging and install media

> Everything between a compiled driver and a guest that boots with it: catalog
> generation and signing, package layout, the guest tools payload, offline
> image servicing, unattended install, boot configuration patching, the
> certificate blob format, and driver troubleshooting.

## Overview

This document collects practical notes for building, cataloging, and test-signing Windows drivers intended to run on Windows 7 SP1, and documents the CI scripts used to produce installable artifacts.

End-to-end, Aero’s Windows 7 driver pipeline is:

1. Build driver binaries (`.sys`) for **x86** and **x64**
2. Stage a driver package (INF + SYS + any coinstallers)
3. Generate catalogs (`.cat`) with `Inf2Cat`
4. Test-sign catalogs/binaries for development/CI
5. Bundle signed packages into distributable artifacts (driver bundle ZIP/ISO and/or Guest Tools media)
6. Install drivers on Windows 7 (post-install or during Windows Setup)

The repo provides PowerShell entrypoints that CI and local builds can share:

- `drivers/build/install-wdk.ps1` → locate/install toolchain components and write `out/toolchain.json`
- `drivers/build/validate-toolchain.ps1` → smoke-test that `Inf2Cat /os:7_X86,7_X64` works (catches runner/toolchain regressions early)
- `drivers/build/build-drivers.ps1` → build binaries into `out/drivers/`
- `drivers/build/build-aerogpu-dbgctl.ps1` → build AeroGPU dbgctl helper tool (required by CI packaging manifests that reference it)
- `drivers/build/make-catalogs.ps1` → stage packages + run `Inf2Cat` into `out/packages/`
- `drivers/build/sign-drivers.ps1` → create a test cert + sign `.sys`/`.cat` under `out/packages/` (signed catalogs cover INF-referenced payload files like user-mode DLLs)
- `drivers/build/package-drivers.ps1` → create `.zip` bundles and an optional `.iso` for “Load driver” installs
- `drivers/build/package-guest-tools.ps1` → build Guest Tools media (`aero-guest-tools.iso`/`.zip`) from signed packages using a packager spec (selects a subset of drivers)

These scripts are orchestrated in CI by:

- `.github/workflows/drivers-win7.yml` (**canonical** PR/push workflow; builds + catalogs + test-signs + packages)
  - Driver bundles: `win7-drivers` (from `out/artifacts/`)
  - Raw signed packages + cert: `win7-drivers-signed-packages` (from `out/packages/**` + `out/certs/aero-test.cer`)
  - Guest Tools media: `aero-guest-tools` (via `drivers/build/package-guest-tools.ps1`; ISO/zip/manifest)
- `.github/workflows/release-drivers-win7.yml` (tagged releases; publishes the packaged artifacts to GitHub Releases)

---

## Supported OS targets

- **Windows 7 SP1 x86 (32-bit)**
- **Windows 7 SP1 x64 (64-bit)**

Notes:

- Windows 7 x64 enforces kernel-mode signatures unless the machine is configured for **test signing**.
- Windows Server 2008 R2 shares the same kernel line (NT 6.1) and generally behaves the same for INF targeting/signature enforcement.

---

## Toolchain choice (CI and local)

### Why we validate the toolchain

Catalog generation is performed with `Inf2Cat.exe`. For Windows 7 we specifically need the OS tokens:

```
Inf2Cat /os:7_X86,7_X64
```

Not every Windows Kits / WDK release has historically accepted older `/os:` tokens, and CI runner images can change over time. A failing catalog-generation step is easy to miss until late in the build pipeline, so we validate it explicitly.

### Pinned Windows Kits version

CI pins the Windows Kits toolchain to:

- **Windows Kits 10.0.22621.0** (Windows 11 / Windows 10 22H2-era toolset)

The pin is implemented in `drivers/build/install-wdk.ps1` (which installs the Windows SDK/WDK via `winget` on CI if needed) and verified by `drivers/build/validate-toolchain.ps1`.

### Toolchain bootstrap (`drivers/build/install-wdk.ps1`)

`drivers/build/install-wdk.ps1` provisions a predictable toolchain for other scripts by writing:

- `out/toolchain.json`

This JSON includes MSBuild, Inf2Cat, signtool, and (when available) stampinf paths using common property names that other scripts understand (`MSBuild`, `Inf2CatPath`, `SignToolPath`, etc.).

### Toolchain validation (`drivers/build/validate-toolchain.ps1`)

From PowerShell:

```powershell
.\drivers\build\install-wdk.ps1
.\drivers\build\validate-toolchain.ps1
```

The scripts write logs/artifacts under:

- `out/toolchain.json` (resolved tool paths)
- `out/toolchain-validation/` (validation transcript + Inf2Cat output)

### CI workflow

The GitHub Actions workflow `.github/workflows/toolchain-win7-smoke.yml` runs on `windows-latest` and:

1. Resolves/installs the pinned toolchain
2. Prints tool versions (`Inf2Cat`, `signtool`, `stampinf`, `msbuild`)
3. Generates a minimal dummy driver package and runs `Inf2Cat /os:7_X86,7_X64`
4. Uploads the logs as workflow artifacts

---

## Local build instructions (PowerShell)

> These steps assume you are on a Windows host with PowerShell. Running in an elevated PowerShell is recommended (some certificate store operations may require it).

### Bootstrap (and optionally validate) the toolchain

```powershell
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force
.\drivers\build\install-wdk.ps1
.\drivers\build\validate-toolchain.ps1
```

### Build driver binaries (`.sys`)

#### CI driver selection (explicit opt-in via `ci-package.json`)

`drivers/build/build-drivers.ps1` only builds drivers that are explicitly marked as **CI-packaged** by placing a `ci-package.json` manifest at the *driver root* (for example `drivers/windows7/virtio-net/ci-package.json`).

`ci-package.json` is the explicit CI packaging gate: CI discovery starts from `drivers/**/ci-package.json` and only those drivers can flow into `out/drivers/`, `out/packages/`, and the final driver bundle artifacts. (Guest Tools media is further filtered by a separate packager spec; see below.)

The CI discovery rule is:

- Candidate driver roots are discovered from directories under `drivers/` that contain `ci-package.json`.
- Each candidate driver root must contain:
  - a build target: `<dirName>.sln` **or** exactly one `*.vcxproj`
  - at least one `*.inf` somewhere under its tree (excluding `obj/`, `out/`, `build/`, `target/`)
- MakeFileProj/Makefile wrapper projects (legacy WDK `build.exe`) are skipped by default.
  - Pass `-IncludeMakefileProjects` to opt in.
  - For mixed solutions (MSBuild projects + wrapper projects), CI builds only the non-wrapper `*.vcxproj` projects.

To add a new driver to CI packaging:

1. Copy `drivers/_template/ci-package.json` into your driver root as `ci-package.json`
2. Update the `$schema` relative path as needed
3. Replace the `infFiles` placeholder (or remove the `infFiles` key to enable CI auto-discovery of all `*.inf` files under the driver directory).
   - If the driver directory contains multiple INFs, an explicit `infFiles` allowlist is recommended to avoid packaging unrelated variants together.
4. Optionally set `wow64Files` if the x64 package needs specific 32-bit user-mode payload DLLs copied in from the x86 build output (WOW64 components).
   - Ensure WOW64 DLL names do not collide with 64-bit build output names, since WOW64 payloads are copied into the x64 package root.
5. Optionally include extra packaging assets/tools:
   - `additionalFiles` for non-binary files (README/license text, install scripts, extra `.inf` under subdirectories, etc).
   - `toolFiles` for user-mode helper tool binaries (`.exe`) checked into the driver source tree (explicit opt-in; `.exe` is intentionally disallowed in `additionalFiles`).

See also the examples under `drivers/_template/`:

- `ci-package.README.md` (field reference)
- `ci-package.json` (starter template)
- `ci-package.inf-wow64-example.json`
- `ci-package.tools-example.json`
- `ci-package.wdf-example.json`

> Note: CI only builds/stages drivers with `ci-package.json`; drivers without it are treated as dev/test and skipped.
>
> `drivers/win7/virtio/virtio-transport-test/` is a KMDF smoke-test driver and is intentionally **not** CI-packaged (no `ci-package.json`), so it does not ship in CI-produced driver bundles / Guest Tools artifacts. Its `virtio-transport-test.inf` intentionally binds a **non-contract** virtio PCI HWID (`PCI\VEN_1AF4&DEV_1040`) so it cannot steal binding from production virtio devices if you install it manually alongside other drivers.
> The virtio-input driver under `drivers/windows7/virtio-input/` is revision-gated to Aero contract v1 (`...&REV_01`).
> INF matching policy for virtio-input keyboard/mouse:
>
> - Canonical INF: `inf/aero_virtio_input.inf`
>   - Binds the subsystem-qualified keyboard/mouse contract v1 HWIDs (`SUBSYS_0010` / `SUBSYS_0011`, both `&REV_01`) for distinct
>     Device Manager names.
>   - Also includes a strict revision-gated generic fallback HWID (no `SUBSYS`): `PCI\VEN_1AF4&DEV_1052&REV_01`
>     (Device Manager name: **Aero VirtIO Input Device**) for environments where subsystem IDs are not exposed/recognized.
> - Optional legacy alias INF (disabled by default): `inf/virtio-input.inf.disabled` → rename to `inf/virtio-input.inf`
>   - Exists only for compatibility with workflows/tools that still reference `virtio-input.inf`.
>   - Policy: filename-only alias; from the first section header (`[Version]`) onward it must remain byte-for-byte identical to
>     `inf/aero_virtio_input.inf` (banner/comments above `[Version]` may differ). See
>     `drivers/windows7/virtio-input/scripts/check-inf-alias.py`.
>   - Because it is identical, enabling the alias does **not** change HWID matching behavior.
>
> Tablet devices bind via the separate `inf/aero_virtio_tablet.inf` (`SUBSYS_00121AF4`); that HWID is more specific than the generic
> fallback, so it wins when both packages are installed. If the tablet INF is not installed (or the device does not expose the tablet
> subsystem ID), the device may bind via the generic fallback and appear as **Aero VirtIO Input Device**.
>
> Avoid shipping/installing both basenames at once: overlapping driver packages can cause confusing driver selection. Prefer explicit
> `ci-package.json` `infFiles` allowlists so only one of the two INF basenames is packaged.

```powershell
.\drivers\build\build-drivers.ps1 -ToolchainJson .\out\toolchain.json
```

Outputs:

- Binaries staged under `out/drivers/<driver>/<arch>/...`
- MSBuild logs under `out/logs/drivers/` (including `.binlog` for deep debugging)

### Build AeroGPU dbgctl (required for CI-style packaging)

Some driver packages (notably `drivers/aerogpu`) ship auxiliary tooling alongside the driver package (for example, `aerogpu_dbgctl.exe`).

These tools should be treated as build outputs: build them, then copy them into `out/drivers/<driver>/<arch>/...` so `drivers/build/make-catalogs.ps1` will stage them into `out/packages/**`. Drivers that require such tools can enforce their presence via `requiredBuildOutputFiles` in `ci-package.json`.

`drivers/build/build-aerogpu-dbgctl.ps1` verifies the dbgctl build output at:

- `drivers/aerogpu/tools/win7_dbgctl/bin/aerogpu_dbgctl.exe`

and (when `out/drivers/aerogpu/<arch>/` exists, i.e. after `drivers/build/build-drivers.ps1`) copies it into:

- `out/drivers/aerogpu/x86/tools/win7_dbgctl/bin/aerogpu_dbgctl.exe`
- `out/drivers/aerogpu/x64/tools/win7_dbgctl/bin/aerogpu_dbgctl.exe`

When these signed packages are packaged into Guest Tools, dbgctl is shipped inside the AeroGPU driver directory at:

- `drivers/amd64/aerogpu/tools/win7_dbgctl/bin/aerogpu_dbgctl.exe`
- `drivers/x86/aerogpu/tools/win7_dbgctl/bin/aerogpu_dbgctl.exe`
  - Some Guest Tools builds also include a convenience copy in the optional top-level `tools/` payload (when present):
    - `tools/aerogpu_dbgctl.exe`
    - `tools/<arch>/aerogpu_dbgctl.exe`

Bitness policy:

- `aerogpu_dbgctl.exe` is intentionally built/shipped as an **x86 (32-bit)** tool.
- For the x64 driver/package tree we still ship the same x86 binary and run it under **WOW64**.
- CI enforces this by inspecting the PE header and requiring `IMAGE_FILE_MACHINE_I386 (0x014c)`.

Build it before catalog generation:

```powershell
.\drivers\build\build-aerogpu-dbgctl.ps1 -ToolchainJson .\out\toolchain.json
```

### Stage packages + generate catalogs (`Inf2Cat`)

`drivers/build/make-catalogs.ps1` stages driver packages under `out/packages/` by combining:

- packaging assets from `drivers/<driver>/` (INF files, optional coinstallers, etc), and
- built binaries from `out/drivers/<driver>/<arch>/`,

then runs `Inf2Cat` in each staged package directory.

Only drivers that opt into CI packaging via `drivers/<driver>/ci-package.json` are staged. The manifest can also constrain which INF files are included via `infFiles` (recommended when a driver directory contains multiple INFs).

```powershell
.\drivers\build\make-catalogs.ps1 -ToolchainJson .\out\toolchain.json
```

Outputs:

- Staged packages under `out/packages/<driver>/<arch>/`
- `.cat` files generated inside each package directory

### Test-sign the packages (`signtool`)

Default (max Win7 compatibility):

```powershell
.\drivers\build\sign-drivers.ps1 -ToolchainJson .\out\toolchain.json -Digest sha1
```

Dual signing (SHA-1 first, then append SHA-256):

```powershell
.\drivers\build\sign-drivers.ps1 -ToolchainJson .\out\toolchain.json -DualSign
```

Outputs:

- Signed `*.sys` and `*.cat` under `out/packages/**` (configurable via `-InputRoot`)
  - Note: `Inf2Cat` catalogs hash all INF-referenced files in the package (INF, SYS, DLL, etc). Windows PnP validates package contents against those hashes via the signed catalog, so Authenticode-signing user-mode DLLs individually is optional.
- Public cert (artifact-safe): `out/certs/aero-test.cer`
- Signing PFX (private key): `out/aero-test.pfx` (kept under `out/`, not `out/certs/`)

The script also verifies signatures:

- `.sys`: `signtool verify /kp /v`
- `.cat`: `signtool verify /v`

And imports the public cert into the current user **Trusted Root** and **Trusted Publishers** stores (and will also try LocalMachine stores when allowed) so verification works.

### Stable certificate (optional)

For releases you may want a consistent test certificate across runs. `drivers/build/sign-drivers.ps1` supports this by accepting a PFX via environment variables:

- `AERO_DRIVER_PFX_BASE64`: base64-encoded PFX bytes
- `AERO_DRIVER_PFX_PASSWORD`: PFX password

If both are set, the script uses the provided PFX instead of generating a new self-signed certificate, and still exports the public cert as `out/certs/aero-test.cer`.

### Package artifacts (ZIP + optional ISO)

Compatibility note: for maximum compatibility with unpatched Windows 7 when using `-Digest sha1` (or `-DualSign`), the *certificate itself* should also be SHA-1-signed (i.e. its signature algorithm should be `sha1RSA`/`sha1WithRSAEncryption`). The script will refuse to use a SHA-256-signed stable certificate for SHA-1 driver signing unless you pass `-AllowSha2CertFallback`.

```powershell
.\drivers\build\package-drivers.ps1 -InputRoot .\out\packages -CertPath .\out\certs\aero-test.cer -OutDir .\out\artifacts
```

Signing policy:

- Default: `-SigningPolicy test`
  - Requires `-CertPath` (defaults to `out/certs/aero-test.cer`).
  - Bundles `aero-test.cer` into the ZIP/ISO roots.
  - `INSTALL.txt` includes **test signing** and **certificate import** steps.
- For production/WHQL-signed drivers, use `-SigningPolicy production` (or `none`):
  - Does **not** require `-CertPath`.
  - Does **not** bundle any certificate files.
  - `INSTALL.txt` omits test-signing/certificate import steps.

Outputs:

- `out/artifacts/AeroVirtIO-Win7-<version>-x86.zip`
- `out/artifacts/AeroVirtIO-Win7-<version>-x64.zip`
- `out/artifacts/AeroVirtIO-Win7-<version>-bundle.zip`
- `out/artifacts/AeroVirtIO-Win7-<version>.iso` (unless `-NoIso` is used; requires Rust/cargo for deterministic builds by default; use `-LegacyIso` for Windows IMAPI2 which is **not** deterministic)
- `out/artifacts/AeroVirtIO-Win7-<version>-fat.vhd` (optional; when `-MakeFatImage` or `AERO_MAKE_FAT_IMAGE=1`; requires Windows + Administrator privileges; skipped unless `-FatImageStrict`)

Integrity manifests (default; disable with `-NoManifest`):

- `out/artifacts/AeroVirtIO-Win7-<version>-x86.manifest.json`
- `out/artifacts/AeroVirtIO-Win7-<version>-x64.manifest.json`
- `out/artifacts/AeroVirtIO-Win7-<version>-bundle.manifest.json`
- `out/artifacts/AeroVirtIO-Win7-<version>.manifest.json` (when ISO is produced)
- `out/artifacts/AeroVirtIO-Win7-<version>-fat.manifest.json` (when FAT VHD is produced)

Each `*.manifest.json` includes the artifact's `sha256`/`size`, the packaging `version`,
`signing_policy`, and (when present) `package.build_id` (defaults to the HEAD commit SHA), plus a
stable per-file hash list for mixed-media detection.

The packaged artifacts include:

- `INSTALL.txt` with the exact commands for `pnputil` installs (and, for `-SigningPolicy test`, test signing + certificate import)
- `aero-test.cer` *(SigningPolicy=test only)*

These driver bundle artifacts include **all** staged CI-packaged drivers under `out/packages/`. If you opt a dev/test driver into CI packaging, it will appear in the bundle artifacts; ensure its INF does not bind production HWIDs so it cannot steal device binding when multiple driver packages are present.

### Package Guest Tools media (ISO/ZIP) (optional)

`drivers/build/package-guest-tools.ps1` consumes the signed driver packages under `out/packages/` and produces the Guest Tools ISO/zip. Unlike the driver bundle ZIP/ISO, Guest Tools includes only the drivers selected by a packager spec (`-SpecPath`):

- **CI/release workflows:** `tools/packaging/specs/win7-signed.json`
- **Local default (when `-SpecPath` is omitted):** `tools/packaging/specs/win7-aero-guest-tools.json` (stricter HWID validation)

This means a driver can be CI-packaged (built + cataloged + signed) and appear in the driver bundle artifacts, but still be omitted from Guest Tools if it is not selected by the spec.

```powershell
.\drivers\build\package-guest-tools.ps1 -InputRoot .\out\packages -CertPath .\out\certs\aero-test.cer -OutDir .\out\artifacts -SpecPath .\tools\packaging\specs\win7-signed.json
```

Outputs:

- `out/artifacts/aero-guest-tools.iso`
- `out/artifacts/aero-guest-tools.zip`
- `out/artifacts/manifest.json`
- `out/artifacts/aero-guest-tools.manifest.json` (copy of `manifest.json`, used by CI/release asset publishing)

---

## Catalog generation (`Inf2Cat`)

### What the catalog is (and why it matters)

On Windows 7, a PnP driver package is typically considered “signed” when:

- The package contains a `.cat` file referenced by the INF, and
- The `.cat` is digitally signed, and
- The `.cat` contains hashes for every file in the package (INF, SYS, DLL, etc.)

**Any time you change any file in the package, you must regenerate and re-sign the catalog.**

Note: whether `Inf2Cat` includes *unreferenced* extra files found under the package directory tree in the catalog is a toolchain detail. CI’s Win7 toolchain smoke test (`drivers/build/validate-toolchain.ps1`) prints `INF2CAT_UNREFERENCED_FILE_HASHED=0|1` so toolchain updates don’t silently change this behaviour. In practice, treat staged package directories as immutable after catalog generation, and stage any auxiliary tools (e.g. dbgctl under `tools/`) before running `drivers/build/make-catalogs.ps1`.

### Required INF metadata

`Inf2Cat` expects certain INF metadata. At minimum, the `[Version]` section should contain:

```ini
[Version]
Signature   = "$WINDOWS NT$"
Class       = System
ClassGuid   = {4D36E97D-E325-11CE-BFC1-08002BE10318}
Provider    = %Aero%
DriverVer   = 01/01/2026,1.0.0.0
CatalogFile = aero-driver.cat
```

Key points:

- **`DriverVer` is required** and affects Windows’ driver ranking/selection.
- **`CatalogFile` must match the generated `.cat` filename** exactly.
  - Per-arch names like `CatalogFile.NTx86 = ...` / `CatalogFile.NTamd64 = ...` are also valid as long as the filenames match.

### Running Inf2Cat manually

Normally `drivers/build/make-catalogs.ps1` runs `Inf2Cat` for you, but for debugging it’s useful to know the raw invocation:

```cmd
Inf2Cat.exe /driver:"C:\path\to\driver-package" /os:7_X86,7_X64 /verbose
```

---

## Test signing model

### Why Aero uses test certificates

For development and CI artifacts we use a **test certificate** because:

- WHQL / Attestation signing is not available for iterative builds
- Development artifacts are intended for controlled environments (dev VMs, test images)
- It keeps the signing workflow self-contained and reproducible

### Enable test signing on Windows 7 x64

On the Windows 7 x64 machine (elevated Command Prompt):

```cmd
bcdedit /set {current} testsigning on
```

Reboot to apply. Verify with:

```cmd
bcdedit /enum {current}
```

### Install the test certificate (Root + TrustedPublisher)

Copy `aero-test.cer` to the Windows 7 machine, then run:

```cmd
certutil -addstore -f Root aero-test.cer
certutil -addstore -f TrustedPublisher aero-test.cer
```

---

## SHA-1 vs SHA-2 (Win7 compatibility)

### Default: SHA-1 for maximum out-of-box compatibility

For the broadest compatibility with “fresh” Windows 7 SP1 installs (especially offline VMs), we default to **SHA-1**:

- `signtool sign /fd sha1 ...`

### File digest (`/fd`) is not the whole story

Authenticode signing has two relevant hash/signature choices:

1. The **file digest** used in the Authenticode signature (`signtool sign /fd sha1|sha256`).
2. The **certificate’s own signature algorithm** (for a self-signed cert, the cert is signed by its own key).

On stock Windows 7 SP1 **without SHA-2 updates** (notably **KB3033929** and **KB4474419**), a common failure mode is:

- the driver/catalog is signed with `/fd sha1`, but
- the signing certificate is **SHA-256-signed**,

and Windows fails to validate the certificate chain because it cannot process SHA-2 signatures in certificates.

### `drivers/build/sign-drivers.ps1` fallback behaviour

Some CI runners refuse creating SHA-1-signed certificates. If SHA-1 certificate creation fails, `drivers/build/sign-drivers.ps1`:

- **fails by default**, or
- continues only if `-AllowSha2CertFallback` is provided, in which case it creates a SHA-256-signed certificate and prints a loud warning that **stock Win7 without KB3033929/KB4474419 may fail**.

### If we ever switch to SHA-256-only

If we sign with SHA-256 (`/fd sha256`) or if the certificate is SHA-256-signed, Windows 7 typically requires SHA-2 support updates such as:

- **KB3033929**
- **KB4474419**

### Optional strategy: dual-signing

If we need both:

- legacy Win7 compatibility (SHA-1), and
- stronger SHA-2 signatures for newer systems,

use `drivers/build/sign-drivers.ps1 -DualSign`, which signs twice:

1. SHA-1 signature first
2. Append SHA-256 signature (`signtool sign /as /fd sha256 ...`)

---

## WDK redistributables (WDF coinstaller)

Some Windows 7-era driver packages (especially KMDF-based ones) may require shipping a WDF coinstaller (`WdfCoInstaller*.dll`). This DLL is a **Microsoft WDK redistributable** with its own license terms.

Policy in this repo:

- CI does **not** include any WDK redistributables by default.
- Drivers that require a WDF coinstaller must declare it in `drivers/<driver>/ci-package.json`, and CI must be run with explicit opt-in:

```powershell
.\drivers\build\build-aerogpu-dbgctl.ps1 -ToolchainJson .\out\toolchain.json
.\drivers\build\make-catalogs.ps1 -ToolchainJson .\out\toolchain.json -IncludeWdfCoInstaller
```

See: `../areas/windows-drivers.md` and `../history/project-history.md`.

---

## Installing drivers on Windows 7

### Install on an already-installed Windows 7 system (`pnputil`)

On Windows 7, use `pnputil` from an elevated command prompt:

```cmd
pnputil -i -a C:\path\to\driver-package\aero-driver.inf
```

### Windows Setup (“Load driver”)

When installing Windows 7, you can load drivers during Setup:

1. Attach the generated driver ISO (`out/artifacts/...Win7-....iso`) to the VM, or copy the driver folder to removable media.
2. In Setup, click **Load driver**.
3. Browse to the folder containing the `.inf` for the correct architecture (`x86` vs `x64`).

---

## Troubleshooting

### Toolchain validation failures

- Run `.\drivers\build\validate-toolchain.ps1` and inspect `out/toolchain-validation/` for the exact `Inf2Cat` invocation and output.

### Build failures (`drivers/build/build-drivers.ps1` / MSBuild)

- Re-run `.\drivers\build\install-wdk.ps1` and confirm it locates MSBuild.
- Inspect:
  - `out/logs/drivers/<driver>-<arch>.msbuild.log`
  - `out/logs/drivers/<driver>-<arch>.msbuild.binlog`

### `Inf2Cat` / catalog failures

**Missing/invalid `DriverVer`**

- Fix: ensure `[Version]` contains a correctly formatted `DriverVer = MM/DD/YYYY,major.minor.build.revision`.

**“No files were found that could be cataloged”**

- Fix: ensure `drivers/build/make-catalogs.ps1` is running on a fully staged package (INF + SYS present) and that the INF references files that actually exist in the package.

**Hash mismatch at install time**

- Symptom: install fails with “The hash for the file is not present in the specified catalog file”.
- Fix: regenerate catalogs and re-sign after any binary change.

### Signing failures (`drivers/build/sign-drivers.ps1` / `signtool`)

**`New-SelfSignedCertificate` not available**

- Fix: install Windows’ PKI/Certificate tooling (PowerShell PKI cmdlets are expected on typical dev hosts).

**Code 52 on Win7 despite `/fd sha1`**

- Fix checklist:
  1. Confirm you actually produced a SHA-1-signed certificate (the script will refuse by default if it cannot).
  2. If you proceed with `-AllowSha2CertFallback`, install KB3033929/KB4474419 or use an updated Win7 image.

### Installation failures on Windows 7 x64

**Device Manager Code 52 (“Windows cannot verify the digital signature…”)**

- Fix checklist:
  1. Confirm test signing is enabled: `bcdedit /enum {current}` → `testsigning Yes`
  2. Confirm the certificate is installed into both `Root` and `TrustedPublisher`
  3. Confirm correct architecture packages (x86 vs x64)

**Where to look for details**

- `%WINDIR%\inf\setupapi.dev.log` contains detailed driver installation diagnostics (search for the INF name).

## Windows Driver Packaging, Catalog Generation, and WDK Redistributables

### Overview

For Windows 7 compatibility, some kernel-mode drivers built with **KMDF** (and some UMDF-based stacks) may require shipping a WDF coinstaller (`WdfCoInstaller*.dll`) alongside the driver package. This file is a **Microsoft WDK redistributable** and comes with its own license terms.

This repository’s CI is designed so that **no WDK redistributable binaries are included by default**. Including any Microsoft redistributables must be an explicit choice.

---

### Per-driver packaging manifest (`ci-package.json`)

Drivers intended to be built/packaged by CI must include a manifest:

`drivers/<driver>/ci-package.json`

Where `<driver>` is a path relative to `drivers/` (it may be nested, e.g. `drivers/windows7/virtio-snd/`).

Schema: `drivers/build/driver-package.schema.json`

This file is the **explicit CI opt-in gate**: the Win7 driver pipeline only builds/stages/packages
drivers that include `ci-package.json` at the driver root. This prevents accidentally shipping
dev/test drivers (or conflicting INFs that match the same HWIDs).

Supported fields:

- `$schema` (optional): JSON Schema reference for editor tooling (example: `"../../drivers/build/driver-package.schema.json"`). CI ignores this field. Update the relative path as needed for nested driver directories.
- `infFiles` (optional): explicit list of `.inf` files to stage (paths relative to the driver directory). If omitted, CI discovers all `.inf` files under the driver directory.
  - Use this for drivers that ship multiple INFs (feature variants, optional components) where staging all of them together is undesirable (e.g. multiple INFs with the same HWIDs).
  - If present, the list must be non-empty.
  - Paths must resolve under the driver directory (no absolute paths; cannot escape the driver directory).
  - CI stages selected INFs into the package root by file name; INF file names must be unique within the driver.
- `wow64Files` (optional): list of **file names** to copy from the driver’s **x86** build output into the **x64** staged package directory *before* INF stamping + Inf2Cat.
  - Intended for x64 driver packages that also need 32-bit user-mode components (WOW64 UMD DLLs).
  - Entries must be `.dll` file names.
  - Entries must be file names only (no path separators).
  - Requires x86 build outputs to be present (even if you are only generating/staging x64 packages).
  - WOW64 payloads are copied into the x64 package root; ensure the 32-bit DLL names do not collide with 64-bit build outputs (use distinct names such as a `_x64` suffix for 64-bit DLLs).
- `requiredBuildOutputFiles` (optional): list of file paths (relative to the per-arch build output directory under `out/drivers/<driver>/<arch>/`) that must exist after building.
  - CI will fail staging if any listed path is missing.
  - Paths must be relative (no absolute paths; cannot escape the build output directory).
  - Intended for auxiliary build products that should ship alongside the driver (for example, in-guest debug utilities).
- `additionalFiles` (optional): extra *non-binary* files to include (README/license text, install scripts, etc). Paths are relative to the driver directory (`drivers/<driver>/`) and must resolve under it (no absolute paths; cannot escape the driver directory).
  - CI refuses to include common binary extensions via `additionalFiles` (currently: `.sys`, `.dll`, `.exe`, `.cat`, `.msi`, `.cab`).
- `toolFiles` (optional): list of user-mode helper tool binaries to include in staged packages (paths relative to the driver directory).
  - Entries must have a `.exe` extension.
  - Paths must be relative to the driver directory (no absolute paths / drive letters / UNC roots), must not contain `..` segments, and must resolve under the driver directory.
  - CI copies each tool into the staged package directory preserving the relative path.
  - `.exe` is intentionally disallowed in `additionalFiles`; `toolFiles` is the explicit opt-in mechanism for shipping `.exe` alongside the driver.
- `wdfCoInstaller` (optional): declare that this driver needs the WDF coinstaller and which KMDF version/DLL name.
  - If `dllName` is omitted, CI derives it from `kmdfVersion` (e.g. `1.11` → `WdfCoInstaller01011.dll`).
  - If provided, `dllName` must be a simple filename like `WdfCoInstaller01011.dll` (not a path).

Example (explicit INF selection + WOW64 payload DLL in x64 package):

```json
{
  "$schema": "../../drivers/build/driver-package.schema.json",
  "infFiles": ["packaging/win7/mydriver.inf"],
  "wow64Files": ["mydriver_umd.dll"],
  "additionalFiles": ["README.md", "packaging/win7/install.cmd"]
}
```

For a real in-tree example (driver ships multiple INFs; manifest selects an explicit subset; plus WOW64 payload + helper scripts), see: `drivers/aerogpu/ci-package.json`.

Example (requires WDF coinstaller; `dllName` derived from `kmdfVersion`):

```json
{
  "$schema": "../../drivers/build/driver-package.schema.json",
  "wdfCoInstaller": {
    "kmdfVersion": "1.11"
  },
  "additionalFiles": ["README.md", "packaging/win7/install.cmd"]
}
```

Template examples are available under `drivers/_template/`:

- `ci-package.README.md` (field reference)
- `ci-package.json` (starter template; replace `infFiles` placeholder `REPLACE_ME.inf`, or remove `infFiles` to enable CI auto-discovery)
- `ci-package.inf-wow64-example.json`
- `ci-package.tools-example.json`
- `ci-package.wdf-example.json`

---

### Policy: WDK redistributables are **not included by default**

#### Default behavior

- `drivers/build/make-catalogs.ps1` stages driver packages and generates catalogs **without** copying any WDK redistributables.
- The pipeline intentionally **refuses** to package if it detects `WdfCoInstaller*.dll` checked into `drivers/<driver>/` (to prevent accidentally distributing Microsoft binaries).

#### Enabling WDF coinstaller inclusion (explicit opt-in)

To allow CI to copy `WdfCoInstaller*.dll` into staged packages (from the installed WDK redist directories):

1. The driver must declare `wdfCoInstaller` in `drivers/<driver>/ci-package.json`.
2. CI must run catalog generation with **explicit opt-in**:

```powershell
.\drivers\build\make-catalogs.ps1 -IncludeWdfCoInstaller
```

The script will copy the requested `WdfCoInstaller*.dll` into `out/packages/<driver>/<arch>/` and logs the source path used.

---

### Catalog generation and signing

`drivers/build/make-catalogs.ps1` runs `Inf2Cat` in each staged package directory to generate `.cat` files. If a coinstaller DLL is present and referenced by the driver’s INF, Inf2Cat will hash it into the generated catalog.

#### Inf2Cat hashing scope (unreferenced extra files)

`Inf2Cat` is invoked against the *package directory* (`/driver:<dir>`), but catalog membership is driven by the INF: the generated `.cat` includes the **INF and INF-referenced payload files**.

Files that are present under the package directory but **not referenced by any INF** (for example helper tools) are **not** included in the catalog, and therefore are **not protected by the `.cat` signature**.

CI locks down this behaviour in `drivers/build/validate-toolchain.ps1` by staging a dummy package and an unreferenced sentinel payload file under a subdirectory (currently: `tools/win7_dbgctl/bin/aero_extra_payload_dbgctl.exe`), then dumping/scanning the generated `.cat` to ensure the file name is **absent**. The script prints:

- `INF2CAT_UNREFERENCED_FILE_HASHED=0` (expected; CI fails if this ever changes).

Practical rule: treat the staged package directory as immutable after catalog generation + signing. If you add, modify, or post-process any files that are hashed into the catalog (INF + INF-referenced payloads), you must regenerate and re-sign the `.cat`. Extra unreferenced tools are not cataloged, so changing them does not affect driver installation signature checks (but also isn't protected by the catalog; see below).

Signing is handled by `drivers/build/sign-drivers.ps1` (which uses `signtool` to sign `.sys` drivers and `.cat` catalogs):

```powershell
.\drivers\build\sign-drivers.ps1
```

#### What files are covered by the catalog signature?

`Inf2Cat` catalogs **cover the INF and the files referenced by the INF** (for example via `CopyFiles` / `SourceDisksFiles`).

Files that are merely *present in the package directory* but **not referenced by any INF** are **not** included in the catalog, and therefore are **not protected by the `.cat` signature**.

Implications:

- If you modify an extra tool binary like `aerogpu_dbgctl.exe` *after* the driver package has been signed, it **will not** break driver installation / PnP signature checks, because it is not part of the signed catalog in the first place.
- However, this also means the tool’s bytes are **not tamper-evident via the driver catalog**. If you need integrity/authenticity for such tools, you must:
  - Authenticode-sign the `.exe` separately, and/or
  - reference it from an INF (so Inf2Cat includes it in the catalog).

This repo locks down the current Inf2Cat behavior via a CI smoke test in `drivers/build/validate-toolchain.ps1`, which creates a dummy package with a distinctive unreferenced payload file and asserts that the filename does **not** appear in the generated `.cat`. If a future WDK update changes this behavior, CI will fail and the packaging assumptions/docs must be revisited.

---

### Licensing note (important)

WDK redistributables (including WDF coinstallers) are **Microsoft binaries** governed by Microsoft license terms. Before distributing any package that includes them:

- review the applicable Microsoft redistribution license(s),
- confirm the distribution model is compliant,
- and document the decision.

See also: `../history/project-history.md`.

## Driver Install Media (FAT Image)

Some Windows 7 installer flows (including WinPE and some emulators/VMs) make it easier to load drivers from a small FAT-formatted disk than from an ISO. To support this, CI can optionally produce a **mountable FAT32 disk image** containing the signed driver packages.

### Artifact

When enabled, driver packaging produces:

- `out/artifacts/AeroVirtIO-Win7-<version>-fat.vhd` (via `drivers/build/package-drivers.ps1 -MakeFatImage`)

This is a FAT32-formatted VHD containing:

- `aero-test.cer` *(SigningPolicy=test only)*
- `INSTALL.txt`
- `x86/` (signed 32-bit drivers, grouped by driver name)
- `x64/` (signed 64-bit drivers, grouped by driver name)

### Creating the FAT image locally

The image is created with built-in Windows tooling (DiskPart). It requires:

- Windows
- Administrator privileges (VHD attach/mount + formatting)

Run:

```powershell
pwsh drivers/build/package-drivers.ps1 -MakeFatImage
```

Notes:

- By default, `drivers/build/package-drivers.ps1` uses `-SigningPolicy test`, so the produced FAT image contains
  `aero-test.cer` and `INSTALL.txt` includes test-signing + cert import steps.
- For production/WHQL-signed drivers, pass `-SigningPolicy production` (or `none`) to omit the certificate
  and to generate `INSTALL.txt` without test-signing instructions.

In CI, you can enable FAT image creation without changing invocation by setting:

```text
AERO_MAKE_FAT_IMAGE=1
```

To make FAT image creation a hard requirement, pass:

- `drivers/build/package-drivers.ps1 -FatImageStrict` (packaging-level strictness), or
- `drivers/build/make-fat-image.ps1 -Strict` (image creation strictness)

To build a FAT image from an already-prepared directory (containing `INSTALL.txt`, `x86/`, `x64/`, and optionally `aero-test.cer` when using `-SigningPolicy test`), run:

```powershell
pwsh drivers/build/make-fat-image.ps1 -SourceDir <prepared-dir> -OutFile out/artifacts/aero-drivers-fat.vhd -SigningPolicy test
```

For production/WHQL-signed driver media (no bundled cert), pass:

```powershell
pwsh drivers/build/make-fat-image.ps1 -SourceDir <prepared-dir> -OutFile out/artifacts/aero-drivers-fat.vhd -SigningPolicy production
```

If your environment cannot create or mount VHDs, the script **skips FAT image creation** with a warning by default. To make this a hard failure, pass `-Strict`.

### Using it during Windows 7 Setup ("Load Driver")

Attach the VHD as a **secondary disk** in your VM/emulator (or mount it on the host and copy its contents to a FAT32 USB stick).

In Windows Setup:

1. Click **Load Driver**
2. Browse the attached disk
3. Select the correct architecture folder (`x86` or `x64`)
4. Pick the `.inf` for the driver you need (e.g., `x64\<driver>\*.inf`)

## Windows 7 Guest Tools (Aero)

This guide walks you through installing Windows 7 in Aero using the **baseline (fully emulated)** device profile first, then installing **Aero Guest Tools** to enable the **paravirtual (virtio + Aero GPU)** drivers.

> Aero does **not** include or distribute Windows. You must provide your own Windows 7 ISO and license.

If you are building from source / working on a PR, the GitHub Actions workflow
`.github/workflows/drivers-win7.yml` uploads an `aero-guest-tools` artifact containing:

- `aero-guest-tools.iso`
- `aero-guest-tools.zip`
- `manifest.json` (build metadata + SHA-256 hashes)
- `aero-guest-tools.manifest.json` (alias of `manifest.json` used by CI/release asset publishing)

### Quick start (overview)

1. Install Windows 7 SP1 using **baseline devices**: **AHCI (HDD) + IDE/ATAPI (CD-ROM) + e1000 + VGA**.
   - For the canonical Win7 install topology (AHCI HDD + IDE/ATAPI CD-ROM), see
     [`../areas/storage.md`](../areas/storage.md).
2. Mount `aero-guest-tools.iso` and run `setup.cmd` as Administrator.
3. Reboot once (still on baseline devices).
   - If you used `setup.cmd /skipstorage` (GPU-only / partial Guest Tools media), keep the boot disk on **AHCI** and skip step **4.1** (AHCI → virtio-blk). You can still switch other devices.
4. Switch devices in this order (reboot between each):
   1. **AHCI → virtio-blk**
   2. **e1000 → virtio-net**
   3. **VGA → Aero GPU**
   4. (Optional) **PS/2 → virtio-input**
   5. (Optional) **HDA → virtio-snd**
5. Run `verify.cmd` as Administrator and check `report.txt`.

> Note: Aero’s Windows 7 virtio device contract (`AERO-W7-VIRTIO` v1) encodes the contract major version in the PCI
> Revision ID (`REV_01`). Aero’s in-tree Win7 virtio driver packages are revision-gated (`&REV_01`), and some drivers also validate
> the revision at runtime, so if you are testing under QEMU or another
> VMM you may need to set `x-pci-revision=0x01` on the virtio devices (and preferably `disable-legacy=on`) for the
> drivers to bind.

If you are validating **virtio-input** specifically (device model + Win7 driver + web runtime routing), see:

- [`virtio-input-test-plan.md`](../areas/windows-drivers.md)

### Contents

- [Prerequisites](#prerequisites)
- [Step 1: Install Windows 7 using baseline devices](#step-1-install-windows-7-using-baseline-compatibility-devices)
- [Step 2: Mount `aero-guest-tools.iso`](#step-2-mount-aero-guest-toolsiso)
- [Step 3: Run `setup.cmd` as Administrator](#step-3-run-setupcmd-as-administrator)
- [What `setup.cmd` changes](#what-setupcmd-changes)
- [If `setup.cmd` fails: manual install](#if-setupcmd-fails-manual-install-advanced)
- [Step 4: Reboot (still on baseline devices)](#step-4-reboot-still-on-baseline-devices)
- [Step 5: Switch to virtio + Aero GPU](#step-5-switch-to-virtio--aero-gpu-recommended-order)
- [Step 6: Run `verify.cmd` / read `report.txt`](#step-6-run-verifycmd-and-interpret-reporttxt)
- [Rollback paths](#safe-rollback-path-if-virtio-blk-boot-fails)
- [Optional: uninstall Guest Tools](#optional-uninstall-guest-tools)
- [Optional: slipstream SHA-2 updates and drivers](#optional-slipstream-sha-2-updates-and-drivers-into-your-windows-7-iso)
- [Troubleshooting](../areas/windows-drivers.md)

### Prerequisites

#### What you need

- A Windows 7 **SP1** ISO for **x86 (32-bit)** or **x64 (64-bit)**.
  - Recommended: official Microsoft/MSDN/OEM media (unmodified).
  - Avoid “pre-activated”, “all-in-one”, or heavily modified ISOs (they frequently break servicing, signing, or boot).
- A Windows 7 product key/license that matches your ISO edition.
- `aero-guest-tools.iso` (or `aero-guest-tools.zip`) shipped with Aero builds/releases.
  - These artifacts are produced by CI from **signed driver packages** and include a `manifest.json` for integrity/version reporting.
  - If you're building from source/CI, see [`../areas/windows-drivers.md`](../areas/windows-drivers.md) for how the ISO/zip is produced from signed driver packages.
- Enough resources for the guest:
  - Disk: **30–40 GB** recommended for a comfortable Win7 install.
  - Memory: **2 GB** minimum (x86), **3–4 GB** recommended (x64).

#### Device profiles used in this guide

This guide intentionally starts with **baseline (fully emulated)** devices for maximum installer compatibility, then switches to **paravirtual** devices for performance after Guest Tools is installed.

The exact names vary by Aero version/UI, but the mapping is typically:

| Subsystem | Baseline (install/recovery) | Performance (after Guest Tools) | Notes |
| --- | --- | --- | --- |
| Storage | AHCI (SATA) | virtio-blk | Switch this first; easiest to brick boot if done too early. |
| Network | Intel e1000 | virtio-net | If networking breaks, switch back to e1000 and boot. |
| Graphics | VGA | Aero GPU | If you get a black screen, switch back to VGA and recover. |
| Input | PS/2 keyboard + mouse | virtio-input | Optional; switch last. If input breaks, switch back to PS/2. |
| Audio | HDA (Intel HD Audio) | virtio-snd *(optional)* | Optional; does not affect boot. If virtio-snd is not exposed/active in your runtime (browser runtime prefers HDA when present), keep HDA. See [`../areas/windows-drivers.md`](../areas/windows-drivers.md#browser-runtime-status). For baseline HDA validation, see [`../areas/audio.md`](../areas/audio.md). |

#### Why Windows 7 SP1 matters

Windows 7 RTM is missing years of fixes. SP1 significantly reduces installer and driver friction.

#### Supported Windows 7 ISOs / editions

Aero Guest Tools is intended for **Windows 7 SP1**:

- ✅ Windows 7 **SP1 x86** and **SP1 x64**
- ✅ Most editions should work (Home Premium / Professional / Ultimate / Enterprise), as long as your license matches the media.
- ❌ Windows 7 **RTM (no SP1)** is not recommended (higher chance of installer/driver/update failures).

#### SHA-256 / SHA-2 updates note (important for x64)

If Aero’s driver packages (or signing certificates) use **SHA-256 / SHA-2**, stock Windows 7 SP1 may require SHA-2-related updates such as **KB3033929** (and sometimes also **KB4474419**) to validate driver signatures.

- If the required SHA-2 updates are missing you may see **Device Manager → Code 52** (“Windows cannot verify the digital signature…”).
- You can install the required updates after Windows is installed, or **slipstream** them into your ISO (see the optional section at the end).

#### x86 vs x64 notes (driver signing and memory)

- **Windows 7 x86**
  - Does not enforce kernel-mode signature checks as strictly as x64, but you can still see warnings during driver install.
  - Practical benefit: can be easier to get started if you are troubleshooting signing issues.
- **Windows 7 x64**
  - Enforces kernel driver signature validation.
  - Aero Guest Tools media includes a `manifest.json` that describes signing expectations via `signing_policy`:
    - `test`: Guest Tools will install certificate(s) (from `certs\`) and may prompt to enable **Test Signing**.
    - `production`: drivers are production/WHQL-signed; no custom certificate or Test Signing is expected.
    - `none`: no signing expectations (development use).
  - You may still need SHA-2 updates (commonly **KB3033929**, sometimes **KB4474419**) if the driver catalogs/certificates are SHA-2-signed.

### Step 1: Install Windows 7 using baseline (compatibility) devices

Install Windows first using the baseline emulated devices (this avoids needing any third-party drivers during Windows Setup).

In Aero, create a new Windows 7 VM/guest and select the **baseline / compatibility** profile:

- **Storage:** SATA **AHCI**
  - The Windows installer ISO is typically exposed as an **ATAPI CD-ROM** on a **PIIX3 IDE**
    controller in the canonical Win7 topology; see [`../areas/storage.md`](../areas/storage.md).
- **Network:** Intel **e1000**
- **Graphics:** **VGA**
- **Input:** PS/2 keyboard + mouse (or Aero’s default input devices)
- **Audio:** HDA / Intel HD Audio (optional)

Boot drive selection note (BIOS `DL`):

- To boot the **installer ISO**, select **CD0** as the BIOS boot drive (`DL=0xE0`) *before* reset/restart.
- After installation (and for normal boots), select **HDD0** as the BIOS boot drive (`DL=0x80`) *before* reset/restart.
- Aero’s legacy BIOS boot selection is still primarily driven by an explicit boot drive number (`DL`),
  but it also supports an optional “CD-first when present” policy (attempt CD0 when install media is
  attached, otherwise fall back to the configured boot drive). See
  [storage boot flows](../areas/storage.md#boot-flows-normative).

Then:

1. Attach your Windows 7 ISO as the virtual CD/DVD.
2. Boot the VM and complete Windows Setup normally.
3. Confirm you can reach a stable desktop.

Recommended (but optional): take a snapshot/checkpoint here if your host environment supports it.

#### Optional (recommended for x64): install SHA-2 updates before Guest Tools

If you expect to use **SHA-256 / SHA-2-signed** driver packages, install the required SHA-2 updates (commonly **KB3033929**, and sometimes also **KB4474419**) while you are still on baseline devices (AHCI/IDE/e1000/VGA). This avoids confusing “unsigned driver” failures later.

See: [`../areas/windows-drivers.md`](../areas/windows-drivers.md#issue-missing-kb3033929-sha-256-signature-support)

### Step 2: Mount `aero-guest-tools.iso`

After you have a working Windows 7 desktop:

1. Eject/unmount the Windows installer ISO.
2. Mount/insert `aero-guest-tools.iso` as the virtual CD/DVD.
3. In Windows 7, open **Computer** and verify you see the CD drive.

If you were given `aero-guest-tools.zip` instead of an ISO, you can extract it on the host and copy the extracted folder into the VM (for example into `C:\AeroGuestTools\media\`) and run `setup.cmd` from there.

#### Third-party notices

Guest Tools media includes `THIRD_PARTY_NOTICES.md` at the ISO/zip root and may include
additional upstream license/notice texts under `licenses/virtio-win/` (when the media
was built from a virtio-win distribution). Review these files if you are redistributing
the media.

#### Where Guest Tools writes logs/reports

Regardless of whether you run Guest Tools from the mounted CD/DVD or from a copied folder, the scripts write their output to:

- `C:\AeroGuestTools\`

#### Optional: copy Guest Tools to the local disk

Running directly from the mounted CD/DVD is fine. Copying the files locally is optional, but can make it easier to re-run Guest Tools without re-mounting the ISO.

1. Create a folder such as `C:\AeroGuestTools\media\`
2. Copy all files from the Guest Tools CD into `C:\AeroGuestTools\media\`

### Step 3: Run `setup.cmd` as Administrator

1. Navigate to the Guest Tools folder:
   - Mounted CD/DVD (for example `X:\`), **or**
   - `C:\AeroGuestTools\media\` (if you copied the files locally)
2. Optional (recommended for automation / cautious installs): validate media integrity first (no system changes; does not require Administrator):
   - `setup.cmd /check /verify-media`
3. Right-click `setup.cmd` → **Run as administrator**.
4. Accept any UAC prompts.

During installation you may see driver install prompts:

- **Windows 7 x86:** Windows may warn about unsigned drivers. Choose **Install this driver software anyway** (only if you trust the Guest Tools you’re using).
- **Windows 7 x64:** Windows enforces kernel driver signatures. Guest Tools behavior is controlled by `manifest.json` `signing_policy`:
  - `test`: installs certificate(s) from `certs\` (when present) and may prompt to enable **Test Signing** so the drivers can load.
  - `production` / `none`: production/WHQL-signed drivers are expected; Guest Tools will not prompt to enable Test Signing by default.

When `setup.cmd` finishes, reboot Windows if prompted.

#### x64: `setup.cmd` signing policy (Test Signing / nointegritychecks)

Guest Tools media built by `tools/packaging/aero_packager` includes a `manifest.json` that describes signing expectations via `signing_policy`:

- `test`: intended for test-signed/custom-signed drivers.
  - The packager requires shipping certificate files under `certs\` and `setup.cmd` will install them.
  - On Windows 7 x64, `setup.cmd` may prompt to enable **Test Signing** (or enable it automatically under `/force`).
- `production`: intended for production/WHQL-signed drivers (no custom root cert, no Test Mode watermark expected).
  - `certs\` may be empty (or docs-only).
  - `setup.cmd` will not prompt to enable Test Signing by default.
- `none`: same as `production` for certificate/Test Signing behavior (development use).

If you are building your own Guest Tools media for WHQL/production-signed drivers, package with:

- `aero_packager --signing-policy production`

On Windows 7 x64, **test-signed** Guest Tools builds (`signing_policy=test`) may ask:

- `Enable Test Signing now (recommended for test-signed drivers)? [Y/N]`

If you are using test-signed/custom-signed drivers, choose **Y**. A reboot is required before the setting takes effect.

##### Override flags

Explicit command-line flags override the manifest:

- `setup.cmd /testsigning` or `setup.cmd /forcetestsigning` (enable Test Signing without prompting)
- `setup.cmd /nointegritychecks` or `setup.cmd /forcenointegritychecks` (enable `nointegritychecks` without prompting; **not recommended**)
- `setup.cmd /forcesigningpolicy:none|test|production` (override `manifest.json` `signing_policy`; legacy aliases: `testsigning`→`test`, `nointegritychecks`→`none`)

To keep the current boot policy unchanged, use:

- `setup.cmd /notestsigning` (skip changing the Test Signing state)

`setup.cmd /force` only implies `/testsigning` on x64 when `signing_policy=test`.

If `setup.cmd` fails or prints warnings, **do not** switch the boot disk to virtio-blk yet. Review:

- `C:\AeroGuestTools\install.log`

It is safe (and often recommended) to re-run `setup.cmd` after fixing the underlying problem.

#### `setup.cmd` output files

If you need to troubleshoot an installation, start by reviewing:

- `C:\AeroGuestTools\install.log`

Depending on the Guest Tools version, you may also see:

- `C:\AeroGuestTools\installed-driver-packages.txt`
- `C:\AeroGuestTools\installed-certs.txt`

#### Optional `setup.cmd` flags (advanced)

Guest Tools may support additional command-line flags. Common examples include:

- `setup.cmd /stageonly` (only stages drivers into the driver store)
- `setup.cmd /testsigning` / `setup.cmd /forcetestsigning` (x64: enable Test Signing without prompting)
- `setup.cmd /notestsigning` (x64: keep Test Signing state unchanged)
- `setup.cmd /nointegritychecks` / `setup.cmd /forcenointegritychecks` (x64: enable `nointegritychecks`; **not recommended**)
- `setup.cmd /forcesigningpolicy:none|test|production` (override `manifest.json` `signing_policy`; legacy aliases: `testsigning`→`test`, `nointegritychecks`→`none`)
- `setup.cmd /noreboot` (do not prompt for reboot/shutdown at the end)
- `setup.cmd /skipstorage` (alias: `/skip-storage`)  
  Skip boot-critical virtio-blk storage pre-seeding. Intended for partial Guest Tools payloads (for example AeroGPU-only development builds).  
  **Unsafe to switch the boot disk from AHCI → virtio-blk** until you later run `setup.cmd` again **without** `/skipstorage` (or manually replicate the registry/service steps).

To see the supported options for your build, you can also run:

- `setup.cmd /?`

If the Guest Tools media includes a `README.md`, consult it for the definitive list of supported flags for your build.

#### x64: “Test Mode” is expected if test signing is enabled

If Guest Tools enables test signing on Windows 7 x64, you may see a desktop watermark like:

- `Test Mode Windows 7 ...`

This is normal for test-signed drivers. Only disable test signing after you have confirmed you are using production-signed drivers (see the troubleshooting guide).

### What `setup.cmd` changes

The exact actions depend on the Guest Tools version, but the workflow generally includes:

#### Certificate store

Installs any certificate file(s) shipped under `certs\` (`*.cer`, `*.crt`, `*.p7b`) into the **Local Machine** certificate stores so Windows can trust Aero’s driver packages.

This step is policy-driven:

- If `signing_policy=test`, `setup.cmd` installs certificate file(s) from `certs\` (required for test-signed/custom-signed driver packages).
- If `signing_policy=production` or `signing_policy=none`, `setup.cmd` does **not** install certificates by default, even if `certs\` contains files (it warns and ignores them). Use `setup.cmd /installcerts` only for advanced/debug scenarios.

- **Trusted Root Certification Authorities**
- **Trusted Publishers**

#### Boot configuration (BCD)

Updates the boot configuration database via `bcdedit` when needed (based on `manifest.json` `signing_policy` or explicit override flags), for example:

- Enabling **Test Signing** (`testsigning on`) so test-signed kernel drivers load on Windows 7 x64 (typically only when `signing_policy=test`, unless you explicitly pass `/testsigning`).
- Optionally enabling `nointegritychecks` (disables signature enforcement entirely; **not recommended**).

Reboot is required after changing BCD settings.

#### Driver store / PnP staging

Stages the Aero drivers into the Windows driver store (so that when you later switch devices, Windows can bind the correct drivers automatically). Guest Tools typically:

- adds every `.inf` under `drivers\x86\` (on Win7 x86) or `drivers\amd64\` (on Win7 x64) using `pnputil`,
- and may attempt an immediate install for any matching devices present (unless `/stageonly` is used).

#### Registry / service configuration

Configures driver services and boot-critical settings (especially important for storage drivers), for example:

- Ensuring the **virtio storage** driver is set to start at boot when needed.
- Setting device/service parameters under `HKLM\SYSTEM\CurrentControlSet\Services\...`
- Pre-seeding `HKLM\SYSTEM\CurrentControlSet\Control\CriticalDeviceDatabase\...` entries for virtio-blk PCI IDs so the system can boot after switching **AHCI → virtio-blk**.

### If `setup.cmd` fails: manual install (advanced)

If `setup.cmd` fails (or you prefer to install components manually), you can typically do the same work yourself.

> The exact file names and folder layout inside `aero-guest-tools.iso` may vary by version. The commands below use common paths used by Aero Guest Tools (for example `X:\drivers\` and (when present) `X:\certs\`), but always prefer the layout on your media.

#### Import the driver signing certificate (Local Machine, if present/required)

If the Guest Tools media includes certificate file(s) (commonly under `X:\certs\` as `.cer`, `.crt`, or `.p7b`), import them.

If your Guest Tools media has `manifest.json` `signing_policy=production` or `signing_policy=none`, it should ship **no** certificate files and this step is typically unnecessary (WHQL/production-signed drivers).

From an elevated Command Prompt:

- `certutil -addstore -f Root X:\certs\your-cert.cer`
- `certutil -addstore -f TrustedPublisher X:\certs\your-cert.cer`

(`X:` is usually the Guest Tools CD drive letter.)

#### Enable test signing (Windows 7 x64, if required)

From an elevated Command Prompt:

- `bcdedit /set {current} testsigning on`
- Reboot

If you are using production-signed drivers, keep test signing off.

#### Stage/install drivers into the driver store

Use either `pnputil` (Windows 7 built-in) or DISM:

- Recommended: DISM (recursively add everything under the correct arch folder):
  - Windows 7 x64:
    - `dism /online /add-driver /driver:X:\drivers\amd64\ /recurse`
  - Windows 7 x86:
    - `dism /online /add-driver /driver:X:\drivers\x86\ /recurse`
- Alternative (if you prefer `pnputil`):
  - `pnputil -i -a X:\drivers\amd64\some-driver.inf` (x64) or `X:\drivers\x86\some-driver.inf` (x86)
  - Tip: the AeroGPU display driver INF is typically `aerogpu_dx11.inf` (DX11-capable). Some older/custom builds may also ship `aerogpu.inf` (DX9-only).
    In typical Guest Tools layouts it is:
    - `X:\drivers\amd64\aerogpu\aerogpu_dx11.inf` (x64)
    - `X:\drivers\x86\aerogpu\aerogpu_dx11.inf` (x86)
  - To bulk-install multiple INFs from an elevated Command Prompt:
    - Windows 7 x64:
      - `for /r "X:\drivers\amd64" %i in (*.inf) do pnputil -i -a "%i"`
    - Windows 7 x86:
      - `for /r "X:\drivers\x86" %i in (*.inf) do pnputil -i -a "%i"`
    - If you put this into a `.cmd` file, use `%%i` instead of `%i`:
      - Windows 7 x64:
        - `for /r "X:\drivers\amd64" %%i in (*.inf) do pnputil -i -a "%%i"`
      - Windows 7 x86:
        - `for /r "X:\drivers\x86" %%i in (*.inf) do pnputil -i -a "%%i"`

After staging, reboot once while still on baseline devices.

#### Pre-seed boot-critical virtio-blk storage (required before switching AHCI → virtio-blk)

If you plan to boot Windows from **virtio-blk**, you must also set up boot-critical storage plumbing (otherwise switching the boot disk commonly results in `0x0000007B INACCESSIBLE_BOOT_DEVICE`).

The safest approach is to get `setup.cmd` working, because it:

- configures the storage driver service as BOOT_START, and
- pre-seeds `HKLM\SYSTEM\CurrentControlSet\Control\CriticalDeviceDatabase\...` for the expected virtio-blk PCI IDs (based on `X:\config\devices.cmd`).

If you cannot run `setup.cmd`, do **not** switch the boot disk to virtio-blk until you have replicated those registry/service steps.

If you are using a **partial** Guest Tools build that does not include the virtio-blk storage driver (for example a GPU-only development build), you can still run:

- `setup.cmd /skipstorage`

to install certificates and stage the non-storage drivers. In that case, **leave the boot disk on AHCI** until you later re-run `setup.cmd` without `/skipstorage` using media that includes the virtio-blk driver (or you manually configure the service + CriticalDeviceDatabase keys).

### Step 4: Reboot (still on baseline devices)

After running Guest Tools, reboot once while still using baseline devices. This confirms the OS still boots normally before changing storage/network/display hardware.

Tip: `setup.cmd` may offer an interactive choice at the end (Reboot/Shutdown/No action). Choosing **Reboot** matches this step. If you choose **Shutdown**, consider booting once on baseline devices before you switch the boot disk to virtio-blk.

If `setup.cmd` enabled **Test Signing** or `nointegritychecks`, the reboot is required before those BCD settings take effect.

### Step 5: Switch to virtio + Aero GPU (recommended order)

To reduce the chance of an unrecoverable boot issue, switch devices **in stages** and verify Windows boots between each step.

#### Stage A: switch storage (AHCI → virtio-blk)

If you installed Guest Tools with `setup.cmd /skipstorage`, do **not** perform this stage yet. Leave the boot disk on **AHCI** until you re-run `setup.cmd` without `/skipstorage` using media that includes the virtio-blk driver.

1. Shut down Windows cleanly.
2. In Aero’s VM settings, switch the **system disk controller** from **AHCI** to **virtio-blk**.
3. Boot Windows.

Expected behavior:

- Windows boots to desktop.
- It may install new devices and ask for another reboot.

If you see firmware-level errors like “No bootable device” or “BOOTMGR is missing”, see:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md#issue-no-bootable-device-or-bootmgr-is-missing-after-switching-storage)

#### Stage B: switch networking (e1000 → virtio-net)

1. Shut down Windows.
2. Switch the network adapter from **e1000** to **virtio-net**.
3. Boot Windows and confirm networking works (optional).

If the virtio-net driver fails to bind and you lose networking, you can always switch back to **e1000** to regain connectivity while troubleshooting.

#### Stage C: switch graphics (VGA → Aero GPU)

1. Shut down Windows.
2. Switch graphics from **VGA** to **Aero GPU**.
3. Boot Windows.

Expected behavior:

- The screen may flicker as Windows binds the new display driver.
- You should be able to set higher resolutions.
- To enable the Aero Glass theme, you may need to select a Windows 7 theme in **Personalization** and/or run **Performance Information and Tools** once.
  - If “Aero” themes are unavailable, running `winsat formal` (from an elevated Command Prompt) often enables them after reboot.

If you get a black screen after switching to the Aero GPU, switch back to **VGA** and follow the recovery steps in:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md#issue-black-screen-after-switching-to-the-aero-gpu)

#### Stage D: switch input (PS/2 → virtio-input) (optional)

If your VM settings expose input devices separately (and you want the virtio input stack):

1. Shut down Windows.
2. Switch input from **PS/2** to **virtio-input**.
3. Boot Windows.

Notes:

- Aero’s in-tree Win7 virtio-input driver package (`aero_virtio_input.inf`) is **revision-gated** to the `AERO-W7-VIRTIO` v1 contract (`REV_01`).
  If your VMM exposes a `REV_00` virtio-input device (common in QEMU defaults), the driver will not bind; configure the device to report `REV_01`
  (for example `x-pci-revision=0x01`, ideally with `disable-legacy=on`).

If you lose keyboard/mouse input, power off and switch back to **PS/2**. Then troubleshoot:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md#issue-lost-keyboardmouse-after-switching-to-virtio-input)

#### Stage E: switch audio (HDA → virtio-snd) (optional)

Virtio audio does not affect boot, so treat it as an optional final step:

1. Shut down Windows.
2. Switch audio from **HDA** to **virtio-snd**.
3. Boot Windows and test audio:
   - Control Panel → **Sound** → **Playback** tab: confirm the virtio-snd output endpoint exists.
   - Control Panel → **Sound** → **Recording** tab: confirm the virtio-snd capture endpoint exists.

Note: this step depends on your VM/runtime actually exposing virtio-snd as the guest audio device. If your current runtime does not
expose virtio-snd (or does not make it the active ring-attached device), keep **HDA** enabled and skip this step.

Browser runtime note (`vmRuntime="legacy"`): virtio-snd can be registered, but the I/O worker currently attaches the host AudioWorklet
rings to **HDA when present** (SPSC policy). Virtio-snd becomes the active ring-attached audio device only in HDA-less builds (or with
an explicit host-side selection mechanism).

`vmRuntime="machine"` note: the canonical machine runtime does not currently expose guest audio devices to the host AudioWorklet stack,
so this virtio-snd/HDA selection behavior does not apply.

### Step 6: Run `verify.cmd` and interpret `report.txt`

After you can boot with virtio + Aero GPU:

1. Run `verify.cmd` from the Guest Tools media:
   - Mounted CD/DVD (for example `X:\verify.cmd`), or
   - `C:\AeroGuestTools\media\verify.cmd` (if you copied the files locally)
2. Right-click `verify.cmd` → **Run as administrator**
3. Open:
   - `C:\AeroGuestTools\report.txt`

Running `verify.cmd` typically writes:

- `C:\AeroGuestTools\report.txt` (human-readable)
- `C:\AeroGuestTools\report.json` (machine-readable)

At the end, `verify.cmd` prints an overall status:

- `Overall: PASS` (exit code 0)
- `Overall: WARN` (exit code 1)
- `Overall: FAIL` (exit code 2+)

#### Optional `verify.cmd` parameters (advanced)

Some Guest Tools builds support extra diagnostics flags, for example:

- `verify.cmd -PingTarget 192.168.0.1` (override the ping target)
- `verify.cmd -PlayTestSound` (attempt to play a test sound)

If `-PingTarget` is not provided, the script may attempt to ping the default gateway (if present).

Depending on your Guest Tools version, the report may include:

- Guest Tools build metadata (if `manifest.json` is present) so you can confirm which ISO/zip build you are using
- Guest Tools config (`config\devices.cmd`) contents (service name + expected PCI IDs), which affects boot-critical storage checks
- OS version and architecture (x86 vs x64)
- Whether **KB3033929** is installed (required for validating many SHA-256-signed driver catalogs on Windows 7)
- Whether signature enforcement is configured correctly (for example `testsigning` and/or `nointegritychecks`)
  - `nointegritychecks` disables signature validation entirely and is generally not recommended; prefer properly signed/test-signed drivers + the correct certificate/updates.
- Whether the Aero driver certificate(s) are installed into the expected certificate stores (**Local Machine** `Root` + `TrustedPublisher`)
- Device/driver binding status (Device Manager health) for:
  - virtio-blk storage
  - virtio-net networking
  - virtio-snd audio
  - virtio-input
  - Aero GPU / virtio-gpu graphics
- AeroGPU D3D9 UMD DLL placement (on Win7 x64 this includes the WOW64 UMD under `C:\Windows\SysWOW64\`, required for 32-bit D3D9 apps)

##### Note: how `verify.cmd` detects virtio-snd audio binding

The **Device Binding: Audio (virtio-snd)** check in `report.txt` is derived from `verify.ps1` scanning `Win32_PnPEntity` and attempting to match the virtio-snd PCI device by:

- the Windows **driver service name** bound to the device (preferred), and
- the expected virtio-snd PCI Hardware IDs from `config\devices.cmd` (fallback).

`verify.ps1` reads `AERO_VIRTIO_SND_SERVICE` from `config\devices.cmd` and checks it first, then falls back to common service names:

- `aero_virtio_snd` (Aero clean-room, canonical)
- `aeroviosnd` (legacy Aero clean-room)
- `aeroviosnd_legacy` (Aero QEMU compatibility package; transitional virtio-snd `PCI\VEN_1AF4&DEV_1018`)
- `aeroviosnd_ioport` (Aero legacy I/O-port bring-up package; transitional virtio-snd `PCI\VEN_1AF4&DEV_1018&REV_00`)
- `viosnd` (upstream virtio-win)
- `aerosnd`
- `virtiosnd`

If you are using a virtio-snd driver with a different service name, copy the Guest Tools media to a writable folder and edit `config\devices.cmd` to override:

```cmd
set "AERO_VIRTIO_SND_SERVICE=your-service-name"
```

Repo note: in this repository, `guest-tools/config/devices.cmd` is generated from `protocol-vectors/windows-device-contract.json` via `scripts/generate-guest-tools-devices-cmd.py`. Update the JSON manifest + regenerate rather than editing the repo copy directly.

CI-style drift check (no rewrite):

```bash
python3 scripts/ci/gen-guest-tools-devices-cmd.py --check
```

Missing virtio-snd devices are reported as **WARN** (audio is optional).
- Boot-critical storage readiness for switching AHCI → virtio-blk:
  - storage service `Start=0` (BOOT_START)
  - `CriticalDeviceDatabase` mappings for the expected virtio-blk PCI HWIDs (prevents `0x7B INACCESSIBLE_BOOT_DEVICE`)

If `report.txt` shows failures or warnings, see:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md)

### Safe rollback path (if virtio-blk boot fails)

If Windows fails to boot after switching to **virtio-blk** (common symptoms: boot loop or a BSOD like `0x0000007B INACCESSIBLE_BOOT_DEVICE`):

1. Power off the VM.
2. Switch the disk controller back to **AHCI** in Aero’s VM settings.
3. Boot Windows.
4. Re-run `setup.cmd` as Administrator and reboot once more.
5. Try switching to virtio-blk again (and avoid changing multiple device classes at once).

#### Rollback if virtio-net fails

If you lose networking after switching **e1000 → virtio-net**:

1. Power off the VM.
2. Switch the NIC back to **e1000**.
3. Boot Windows and troubleshoot virtio-net driver binding from a working desktop.

#### Rollback if virtio-input fails

If you lose keyboard/mouse input after switching **PS/2 → virtio-input**:

1. Power off the VM.
2. Switch input back to **PS/2**.
3. Boot Windows and troubleshoot virtio-input driver binding from a working desktop.

#### Rollback if Aero GPU fails

If you get a black/blank screen after switching **VGA → Aero GPU**:

1. Power off the VM.
2. Switch graphics back to **VGA**.
3. Boot Windows and follow the recovery steps in:
   - [`../areas/windows-drivers.md`](../areas/windows-drivers.md#issue-black-screen-after-switching-to-the-aero-gpu)

#### Rollback if virtio-snd fails

If audio stops working after switching **HDA → virtio-snd**, you can always switch back to **HDA**. Audio problems do not affect boot.

### Optional: uninstall Guest Tools

Guest Tools also includes `uninstall.cmd` for best-effort cleanup (useful for testing or reverting a VM back to baseline drivers).

1. Boot Windows.
2. (Recommended) If your boot disk is currently virtio-blk, switch back to **AHCI** first and boot successfully.
3. Run `uninstall.cmd` as Administrator from:
   - the mounted CD/DVD, or
   - `C:\AeroGuestTools\media\` (if you copied the files locally)
4. Review:
   - `C:\AeroGuestTools\uninstall.log`

Uninstall is best-effort and may not remove drivers that are currently in use.

Depending on the Guest Tools version, `uninstall.cmd` may also offer to disable Test Signing and/or `nointegritychecks` if they were enabled by Aero Guest Tools.

### Optional: Slipstream SHA-2 updates and drivers into your Windows 7 ISO

Slipstreaming is optional, but can reduce first-boot driver/signature problems (especially for offline installs).

**Rules:**

- Only modify ISOs you legally own.
- Do not redistribute the resulting ISO.

#### What you can slipstream

- **KB3033929** (SHA-256 signature support)
- **KB4474419** (additional SHA-2 code signing support; may require servicing stack updates depending on your base image)
- **KB4490628** (servicing stack update; a common prerequisite for installing newer updates like KB4474419)
- Aero driver `.inf` packages (virtio-blk/net and optionally Aero GPU)

#### High-level DISM approach (Windows host)

On a Windows 10/11 host (or a Windows VM), you can use DISM:

1. Copy ISO contents to a working folder (example: `C:\win7-iso\`).
2. Identify your `install.wim` index:
   - `dism /Get-WimInfo /WimFile:C:\win7-iso\sources\install.wim`
3. Mount `install.wim` (example index `1`):
   - `mkdir C:\wim\mount`
   - `dism /Mount-Wim /WimFile:C:\win7-iso\sources\install.wim /Index:1 /MountDir:C:\wim\mount`
4. Add the update packages you need (repeat `/Add-Package` per update).
   - Tip: when servicing stack updates are involved, add the SSU first (for example **KB4490628**), then add other updates.
   - Example:
     - `dism /Image:C:\wim\mount /Add-Package /PackagePath:C:\updates\KB4490628-x64.msu`
     - `dism /Image:C:\wim\mount /Add-Package /PackagePath:C:\updates\KB3033929-x64.msu`
     - `dism /Image:C:\wim\mount /Add-Package /PackagePath:C:\updates\KB4474419-x64.msu`
   - If DISM refuses the `.msu`, extract it first and add the `.cab` instead:
      - `expand -F:* C:\updates\KB3033929-x64.msu C:\updates\KB3033929\`
      - `dism /Image:C:\wim\mount /Add-Package /PackagePath:C:\updates\KB3033929\Windows6.1-KB3033929-x64.cab`
5. Add Aero drivers:
   - `dism /Image:C:\wim\mount /Add-Driver /Driver:C:\drivers\aero\ /Recurse`
6. Commit and unmount:
   - `dism /Unmount-Wim /MountDir:C:\wim\mount /Commit`

If you want Windows Setup itself to see a virtio-blk disk during installation, you must also add the storage driver to `sources\\boot.wim` (indexes 1 and 2).

Example (adding drivers to `boot.wim`):

1. Check boot indexes:
   - `dism /Get-WimInfo /WimFile:C:\win7-iso\sources\boot.wim`
2. Mount index 1 and add drivers:
   - `dism /Mount-Wim /WimFile:C:\win7-iso\sources\boot.wim /Index:1 /MountDir:C:\wim\mount`
   - `dism /Image:C:\wim\mount /Add-Driver /Driver:C:\drivers\aero\ /Recurse`
   - `dism /Unmount-Wim /MountDir:C:\wim\mount /Commit`
3. Repeat for index 2.

#### Rebuilding a bootable ISO (optional)

After modifying the WIM(s), you must rebuild a bootable ISO from your working folder. A common approach on Windows is `oscdimg` (Windows ADK):

- `oscdimg -m -o -u2 -udfver102 -bootdata:2#p0,e,bC:\win7-iso\boot\etfsboot.com#pEF,e,bC:\win7-iso\efi\microsoft\boot\efisys.bin C:\win7-iso C:\win7-slipstream.iso`

If your source ISO does not contain `efi\microsoft\boot\efisys.bin`, omit the UEFI boot entry.

For detailed recovery and switch-over pitfalls, see the troubleshooting guide:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md)

## Aero Guest Tools (Windows 7)

`guest-tools/` contains the **in-guest, offline** installer experience for Aero's Windows drivers ("Aero Guest Tools").

It is designed for the default workflow:

1. Install Windows 7 using safe/legacy emulated devices (AHCI HDD + IDE/ATAPI CD-ROM, e1000, HDA, PS/2, VGA).
   - See [`../areas/storage.md`](../areas/storage.md) for the canonical Win7 install/boot storage topology.
2. Mount the Guest Tools media and run `setup.cmd` inside the guest.
3. Power off / reboot and switch the VM devices to Aero virtio devices:
   - virtio-blk (storage)
   - virtio-net (network)
   - virtio-input (keyboard/mouse) *(optional)*
   - virtio-snd (audio) *(optional; browser runtime prefers HDA when present, so virtio-snd is active in HDA-less builds or with an explicit selection mechanism — keep HDA as fallback)*
   - Aero WDDM GPU
4. Boot and let Plug and Play bind the newly-present devices to the staged driver packages.

End-user guides:

- [`windows7-guest-tools.md`](../areas/windows-drivers.md)
- [`windows7-driver-troubleshooting.md`](../areas/windows-drivers.md)

See `guest-tools/README.md` for the authoritative ISO contents, script flags, and implementation details.

## Guest Tools Packaging (ISO + Zip)

This project distributes Windows drivers and helper scripts as a single, mountable **CD-ROM ISO** ("Aero Drivers / Guest Tools"), plus a `.zip` for manual extraction.

The packaging tool lives under:

- `tools/packaging/aero_packager/`

### CI/release packaging (from `out/packages` + `out/certs`)

Windows-driver CI produces:

- `out/packages/**` (staged + signed driver packages)
- `out/certs/aero-test.cer` (the *actual* signing certificate used for the driver catalogs)

To convert those CI outputs into the packager input layout and build the distributable Guest Tools media, use:

```powershell
pwsh -File drivers/build/package-guest-tools.ps1
```

Convenience wrapper (same behaviour, located alongside other driver scripts):

```powershell
pwsh -File drivers/scripts/make-guest-tools-from-ci.ps1
```

The convenience wrapper forwards most arguments to `drivers/build/package-guest-tools.ps1`, including:
`-SpecPath`, `-SigningPolicy`, and `-WindowsDeviceContractPath` (to override the device contract used
to generate the packaged `config/devices.cmd`).

By default this will:

- stage drivers into the layout expected by `aero_packager`:
  - `drivers/x86/<driver>/...`
  - `drivers/amd64/<driver>/...`
- map CI package roots (`out/packages/<driverRel>/{x86,x64}`) into stable Guest Tools-facing driver
  directory names (e.g. `drivers/aerogpu` → `aerogpu`, `windows7/virtio-blk` → `virtio-blk`, `windows7/virtio-net` → `virtio-net`)
- stage `guest-tools/` and normalize `certs/` based on signing policy:
  - `signing_policy=test`: inject `out/certs/aero-test.cer` (keeping `certs/README.md` if present)
  - `signing_policy=production|none`: do **not** inject certs (any existing `*.cer/*.crt/*.p7b` are stripped, leaving docs)
- produce:
  - `out/artifacts/aero-guest-tools.iso`
  - `out/artifacts/aero-guest-tools.zip`
  - `out/artifacts/manifest.json`
  - `out/artifacts/aero-guest-tools.manifest.json` (alias of `manifest.json` for CI/release asset naming)

#### Spec selection (CI vs local)

`drivers/build/package-guest-tools.ps1` uses a packaging spec (`-SpecPath`) to decide which driver
directories are allowed/required and which hardware IDs (HWIDs) to validate.

There are two common specs depending on whether you want to match CI/release behavior or
do stricter local validation:

| Spec | Typical use | Required drivers | Optional drivers | HWID validation |
|---|---|---|---|---|
| `tools/packaging/specs/win7-signed.json` | CI/release workflows (packaging from `out/packages` + `out/certs`) | `aerogpu`, `virtio-blk`, `virtio-net`, `virtio-input` | `virtio-snd` | Derives HWIDs from `devices.cmd` (no hardcoded regex list) |
| `tools/packaging/specs/win7-aero-guest-tools.json` | Local default (`drivers/build/package-guest-tools.ps1` with no `-SpecPath`) | `aerogpu`, `virtio-blk`, `virtio-net`, `virtio-input` | `virtio-snd` | Stricter HWID validation (pins virtio HWIDs in the spec; AeroGPU HWIDs via `devices.cmd`) |

Note: “required/optional drivers” here refers to **packaging validation** (what the ISO/zip is expected to contain), not whether a given device is required at runtime (for example PS/2/HDA remain fallbacks for optional devices).

To reproduce CI packaging locally (assuming you already have `out/packages/` + `out/certs/`):

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File drivers/build/package-guest-tools.ps1 -SpecPath tools/packaging/specs/win7-signed.json
```

### Inputs

#### Driver artifacts

The packager expects a directory containing two architecture subdirectories:

```
drivers/
  x86/
    <driver-name>/
      *.inf
      *.sys
      *.cat
      # Any auxiliary files referenced by the INF (e.g. WdfCoInstaller*.dll,
      # helper DLL/EXEs, manifests, etc) are preserved, with exclusions below.
  amd64/   (or `x64/` on input; the packaged output uses `amd64/`)
    <driver-name>/
      *.inf
      *.sys
      *.cat
      # Same as above.
```

##### Driver file inclusion / exclusions

When copying each `drivers/<arch>/<driver-name>/...` directory into the Guest Tools ISO/zip, the packager includes **all files** by default to keep Windows PnP driver packages installable (especially KMDF-based ones that require `WdfCoInstaller*.dll`).

It applies a small exclusion policy:

- Skipped by default (to avoid bloating artifacts with build outputs):
  - debug symbols: `*.pdb`, `*.ipdb`, `*.iobj`
  - build/link metadata: `*.obj`, `*.lib`, `*.exp`, `*.ilk`, `*.tlog`, `*.log`
  - source / project files: `*.c`, `*.cpp`, `*.h`, `*.sln`, `*.vcxproj`, etc
- **Refused (hard error)** to avoid leaking secrets:
  - private key material: `*.pfx`, `*.pvk`, `*.snk`, `*.key`, `*.pem` (case-insensitive)

The same private-key refusal applies to the `guest-tools/` input tree (e.g. `tools/`, `config/`, `certs/`, `licenses/`) as an extra safety net.

Files under `guest-tools/tools/` are filtered using the same inclusion/exclusion rules as driver
directories (for example `*.pdb` is excluded by default), and symlinks are refused.

Per-driver overrides can be configured in the packaging spec via `allow_extensions` and `allow_path_regexes`.

#### Guest Tools scripts / certs

The packager expects:

```
guest-tools/
  setup.cmd
  uninstall.cmd
  verify.cmd
  verify.ps1
  README.md
  THIRD_PARTY_NOTICES.md
  tools/          (optional; extra guest-side utilities; see filtering notes below)
    # Example: some Guest Tools builds include a convenience copy of `aerogpu_dbgctl.exe` under
    # `tools/aerogpu_dbgctl.exe` (or `tools/<arch>/aerogpu_dbgctl.exe`), in addition to the canonical
    # driver-packaged copy under `drivers/<arch>/aerogpu/tools/win7_dbgctl/bin/aerogpu_dbgctl.exe`.
  licenses/ (optional)
  config/
    README.md (optional)
    devices.cmd   (generated during packaging)
  certs/          (optional when signing_policy is production/none)
    README.md (optional but recommended)
    *.{cer,crt,p7b} (required for signing_policy=test; optional otherwise)
```

`config/devices.cmd` is generated during packaging from a Windows device contract JSON
(`--windows-device-contract` / `-WindowsDeviceContractPath`):

- `protocol-vectors/windows-device-contract.json` (canonical; in-tree Aero driver service names like `aero_virtio_blk` / `aero_virtio_net`)
- `protocol-vectors/windows-device-contract-virtio-win.json` (virtio-win; upstream service names like `viostor` / `netkvm` / `vioinput` / `viosnd`)

Virtio-win Guest Tools builds **must** use the virtio-win contract so `guest-tools/setup.cmd` can
validate the boot-critical storage INF `AddService` name and pre-seed registry state without
requiring `/skipstorage`.

Note:

- For **Aero** driver builds (in-tree virtio + AeroGPU), the contract’s `driver_service_name` values
  are expected to match the packaged INF `AddService` names (e.g. `aero_virtio_blk`).
- For **virtio-win** builds, pass the dedicated contract override
  `protocol-vectors/windows-device-contract-virtio-win.json` to `drivers/build/package-guest-tools.ps1 -WindowsDeviceContractPath`
  so the generated `devices.cmd` uses virtio-win service names (e.g. `viostor`, `netkvm`), while keeping
  Aero’s virtio PCI IDs/HWID patterns for boot-critical `CriticalDeviceDatabase` seeding.
  Do **not** edit the canonical `protocol-vectors/windows-device-contract.json` to virtio-win names.

### Outputs

The tool produces the following in the output directory:

- `aero-guest-tools.iso`
- `aero-guest-tools.zip`
- `manifest.json`
- `aero-guest-tools.manifest.json` (alias of `manifest.json`)

The CI wrapper script (`drivers/build/package-guest-tools.ps1`) also writes a copy of the manifest as
`aero-guest-tools.manifest.json` to avoid collisions when packaging into a shared artifact directory.

The packaged media also includes `THIRD_PARTY_NOTICES.md` at the ISO/zip root.

The ISO/zip root layout matches what `guest-tools/setup.cmd` expects:

```
/
  setup.cmd
  uninstall.cmd
  verify.cmd
  verify.ps1
  README.md
  THIRD_PARTY_NOTICES.md
  manifest.json
  config/
    devices.cmd
  certs/
    README.md (optional)
    *.{cer,crt,p7b} (optional)
  licenses/ (optional)
  tools/    (optional; extra guest-side utilities)
  drivers/
    x86/
      ...
    amd64/
      ...
```

When building Guest Tools from an upstream `virtio-win.iso` via
`drivers/scripts/make-guest-tools-from-virtio-win.ps1`, the wrapper will also attempt to
populate `licenses/virtio-win/` with upstream license/notice files (when present) and
include `driver-pack-manifest.json` for virtio-win ISO provenance.

### Packaging in-tree aero virtio drivers (aero_virtio_blk + aero_virtio_net)

If you built Aero's in-tree Windows 7 virtio drivers (`drivers/windows7/virtio-{blk,net}`) and
have a packager-style driver directory:

```
<DriverOutDir>/
  x86/aero_virtio_blk/*.{inf,sys,cat}
  x86/aero_virtio_net/*.{inf,sys,cat}
  amd64/aero_virtio_blk/*.{inf,sys,cat}   # (or x64/ instead of amd64/)
  amd64/aero_virtio_net/*.{inf,sys,cat}
```

You can build Guest Tools media directly using:

```powershell
powershell -ExecutionPolicy Bypass -File .\drivers\scripts\make-guest-tools-from-aero-virtio.ps1 `
  -DriverOutDir C:\path\to\driver-out `
  -OutDir .\dist\guest-tools `
  -Version 0.0.0 `
  -BuildId local
```

Signing policy notes:

- Default is `-SigningPolicy test` (`signing_policy=test` in `manifest.json`).
- Use `-SigningPolicy production` (or `none`) to omit `certs/*.{cer,crt,p7b}` from the packaged media (and avoid
  Test Signing prompts) when shipping WHQL/production-signed drivers.
- Use `-CertPath` to inject a different test root certificate into the staged `certs/` directory.

This uses the modern-only validation spec:

- `tools/packaging/specs/win7-aero-virtio.json`

### Running locally

#### CI-style flow (signed drivers → Guest Tools ISO/zip)

The repository ships a CI-friendly wrapper script that consumes the signed driver packages
produced by the Win7 driver pipeline and emits Guest Tools media into `out/artifacts/`:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File drivers/build/install-wdk.ps1
pwsh -NoProfile -ExecutionPolicy Bypass -File drivers/build/build-drivers.ps1 -ToolchainJson out/toolchain.json -RequireDrivers
pwsh -NoProfile -ExecutionPolicy Bypass -File drivers/build/build-aerogpu-dbgctl.ps1 -ToolchainJson out/toolchain.json
pwsh -NoProfile -ExecutionPolicy Bypass -File drivers/build/make-catalogs.ps1 -ToolchainJson out/toolchain.json
pwsh -NoProfile -ExecutionPolicy Bypass -File drivers/build/sign-drivers.ps1 -ToolchainJson out/toolchain.json

# Optional (also produces the standalone driver bundle ZIP/ISO/VHD artifacts):
pwsh -NoProfile -ExecutionPolicy Bypass -File drivers/build/package-drivers.ps1

# Guest Tools media (ISO + zip) built from the signed packages in out/packages/:
pwsh -NoProfile -ExecutionPolicy Bypass -File drivers/build/package-guest-tools.ps1
```

Notes:

- `drivers/build/package-guest-tools.ps1` stages drivers into the packager input layout (`x86/<driver>/...`, `amd64/<driver>/...`),
  copies `guest-tools/`, and (when `-SigningPolicy` resolves to `test`) injects `out/certs/aero-test.cer` into `certs/`
  so the resulting ISO matches the signed driver catalogs.
- The wrapper drives inclusion/validation via a packager spec (`-SpecPath`):
  - Local default: `tools/packaging/specs/win7-aero-guest-tools.json` (stricter HWID validation).
  - CI/release workflows: pass `tools/packaging/specs/win7-signed.json` (derives HWID patterns from `devices.cmd`; no hardcoded regex list).
- `config/devices.cmd` is generated by `aero_packager` from a Windows device contract JSON
  (default: `protocol-vectors/windows-device-contract.json`; override via `drivers/build/package-guest-tools.ps1 -WindowsDeviceContractPath`).
  Ensure the contract’s `virtio-blk.driver_service_name` matches the packaged driver’s INF `AddService` name so
  `setup.cmd` boot-critical pre-seeding aligns with the driver packages that are shipped.
  - In-tree Aero builds use `protocol-vectors/windows-device-contract.json`.
  - Virtio-win Guest Tools builds must use `protocol-vectors/windows-device-contract-virtio-win.json` so the packaged `devices.cmd`
    matches virtio-win’s `AddService` names (e.g. `viostor` / `netkvm`).
- `-InputRoot` defaults to `out/packages/`, but you can also point it at an extracted `*-bundle.zip` produced by
  `drivers/build/package-drivers.ps1` (or the `*-bundle.zip` file itself; the wrapper can auto-extract and auto-detect the layout).
- Determinism is controlled by `SOURCE_DATE_EPOCH` (or `-SourceDateEpoch`). When unset, the wrapper uses the HEAD commit
  timestamp to keep outputs stable for a given commit.
- When `-BuildId` is not provided, the wrapper defaults `build_id` in `manifest.json` to the HEAD commit SHA (or an
  equivalent commit SHA environment variable). This keeps the ISO/zip and `manifest.json` reproducible across CI reruns
  for the same inputs.

Outputs:

- `out/artifacts/aero-guest-tools.iso`
- `out/artifacts/aero-guest-tools.zip`
- `out/artifacts/manifest.json`
- `out/artifacts/aero-guest-tools.manifest.json`

#### Direct packager invocation (advanced)

```bash
cd tools/packaging/aero_packager

SOURCE_DATE_EPOCH=0 cargo run --release --locked -- \
  --drivers-dir /path/to/drivers \
  --guest-tools-dir /path/to/guest-tools \
  --spec /path/to/spec.json \
  --out-dir /path/to/out \
  --version 1.2.3 \
  --build-id local \
  --signing-policy test
```

#### Signing policy (test vs production vs none)

Guest Tools media includes a `manifest.json` that describes **signing expectations**:

- `signing_policy`: `test` | `production` | `none`
- `certs_required`: derived from `signing_policy` (currently `true` only for `test`)

This is consumed by `guest-tools/setup.cmd` / `verify.ps1` so they can behave appropriately:

- `test`: packager requires at least one cert file under `guest-tools/certs/` and setup will
  prompt to enable Test Signing on Windows 7 x64 (or enable it automatically under `/force`).
- `production`: packager allows `guest-tools/certs/` to contain only docs (or be empty) and setup
  will not prompt to enable Test Signing by default.
- `none`: same as `production` for certificate/Test Signing behavior (intended for development).

The packager default is `--signing-policy test` to preserve historical behavior.

For back-compat, the packager also accepts legacy aliases:

- `testsigning` / `test-signing` → `test`
- `nointegritychecks` → `none`

#### Building Guest Tools from an upstream virtio-win ISO (Win7 virtio drivers)

If you want the packaged Guest Tools ISO/zip to include the **virtio-win** drivers (viostor + NetKVM at minimum), use the wrapper script:

```powershell
powershell -ExecutionPolicy Bypass -File .\drivers\scripts\make-guest-tools-from-virtio-win.ps1 `
  -VirtioWinIso C:\path\to\virtio-win.iso `
  -OutDir .\dist\guest-tools `
  -Version 0.0.0 `
  -BuildId local
```

On Linux/macOS, you can run the same PowerShell wrapper under PowerShell 7 (`pwsh`):
it will automatically fall back to the cross-platform extractor when `Mount-DiskImage`
is unavailable (or fails to mount).

```bash
pwsh drivers/scripts/make-guest-tools-from-virtio-win.ps1 \
  -VirtioWinIso virtio-win.iso \
  -OutDir ./dist/guest-tools \
  -Version 0.0.0 \
  -BuildId local
```

Alternatively, you can extract first and pass `-VirtioWinRoot`:

```bash
python3 tools/virtio-win/extract.py --virtio-win-iso virtio-win.iso --out-root /tmp/virtio-win-root
pwsh drivers/scripts/make-guest-tools-from-virtio-win.ps1 -VirtioWinRoot /tmp/virtio-win-root -OutDir ./dist/guest-tools
```

Convenience wrapper (Linux/macOS): `bash ./drivers/scripts/make-guest-tools-from-virtio-win.sh`.

Note: The virtio-win wrapper uses `protocol-vectors/windows-device-contract-virtio-win.json` as the contract template and emits a
temporary contract override (service names derived from the extracted driver INFs) when calling the CI packager wrapper,
so the generated `config/devices.cmd` matches upstream virtio-win driver service names (`viostor`, `netkvm`, `vioinput`,
`viosnd`). This keeps `setup.cmd` boot-critical storage pre-seeding aligned with the packaged drivers.

`-Profile` controls both:

- the default driver set extracted from virtio-win (unless overridden by `-Drivers`), and
- the default packaging spec (unless overridden by `-SpecPath`).

Profiles (defaults):

- `full` (default):
  - `-Drivers @('viostor','netkvm','viosnd','vioinput')`
  - `-SpecPath tools/packaging/specs/win7-virtio-full.json` (optional `viosnd`/`vioinput` are best-effort and included only when present for **both** x86 and amd64)
- `minimal`:
  - `-Drivers @('viostor','netkvm')`
  - `-SpecPath tools/packaging/specs/win7-virtio-win.json`

To build storage+network-only Guest Tools media (no optional audio/input drivers), use `-Profile minimal`:

```powershell
powershell -ExecutionPolicy Bypass -File .\drivers\scripts\make-guest-tools-from-virtio-win.ps1 `
  -VirtioWinIso C:\path\to\virtio-win.iso `
  -Profile minimal `
  -OutDir .\dist\guest-tools `
  -Version 0.0.0 `
  -BuildId local
```

For advanced/custom validation, you can override the profile’s spec selection via `-SpecPath`.

Signing policy:

- The wrapper defaults to `-SigningPolicy none` (appropriate for WHQL/production-signed virtio-win drivers).
- If you are packaging test-signed/custom-signed drivers, override it (e.g. `-SigningPolicy test`).
  - Legacy alias accepted: `testsigning` (maps to `test`).

Notes:

- `-SpecPath` overrides the profile’s default spec selection.
- `-Drivers` overrides the profile’s default driver list.
- The wrapper generates a device contract override (based on `protocol-vectors/windows-device-contract-virtio-win.json`) so `setup.cmd`
  boot-critical storage pre-seeding uses the correct virtio-win storage `AddService` name (typically `viostor`).
- `-Profile full` does **not** enable `-StrictOptional` by default; missing `viosnd`/`vioinput` should remain best-effort unless strict mode is requested.

### Validation: required drivers + hardware IDs

Before producing any output, the packager verifies that:

- the output includes **only** driver directories listed in the packaging spec (prevents accidentally shipping stray/incomplete driver folders),
- each **required** driver is present for both `x86` and `amd64` (missing required drivers are fatal),
- each included driver (required + optional that are present) contains at least one `.inf`, `.sys`, and `.cat`,
- each included driver's `.inf` files contain the expected hardware IDs (regex match, case-insensitive) if provided,
- each included driver's `.inf` files reference only files that exist in the packaged driver directory (best-effort; validates common directives like `CopyFiles=`, `CopyINF=`, `SourceDisksFiles*`, and includes KMDF `WdfCoInstaller*.dll` sanity checks).

These checks are driven by a small JSON spec passed via `--spec`.

For Aero packaging profiles, the virtio HWID patterns are expected to match the **Aero virtio
contract v1** (virtio-pci **modern-only**). Transitional device IDs (the older virtio-pci
`0x1000..` device ID range) are intentionally not accepted in the in-repo specs.

#### Spec schema: required + optional drivers

The current schema uses a unified `drivers` list where each entry declares whether it is required:

```json
{
  "drivers": [
    {"name": "viostor", "required": true, "expected_hardware_ids": ["PCI\\\\VEN_1AF4&DEV_1042"]},
    {"name": "netkvm", "required": true, "expected_hardware_ids": ["PCI\\\\VEN_1AF4&DEV_1041"]},
    {"name": "vioinput", "required": false, "expected_hardware_ids": ["PCI\\\\VEN_1AF4&DEV_1052"]},
    {"name": "viosnd", "required": false, "expected_hardware_ids": ["PCI\\\\VEN_1AF4&DEV_1059"]}
  ]
}
```

If an optional driver is listed but missing from the input driver directory, the packager emits a warning and continues.

`expected_hardware_ids_from_devices_cmd_var` can be used instead of (or in addition to)
`expected_hardware_ids` to source expected HWIDs from `guest-tools/config/devices.cmd`. The packager
and config validator normalize these HWIDs down to the base `PCI\VEN_....&DEV_....` form before
validating that the driver INF matches.

Legacy specs using the older top-level `required_drivers` list are still accepted and treated as `required=true` entries.

### CI coverage (packager + config/spec drift)

GitHub Actions runs a dedicated workflow (`guest-tools-packager`) on PRs that touch Guest Tools
packaging inputs (`tools/packaging/**`, `guest-tools/**`, etc.). It covers:

- `cargo test --locked --manifest-path tools/packaging/aero_packager/Cargo.toml`
- A smoke packaging run that verifies the in-repo `guest-tools/` directory can be packaged (using the
  packager's dummy driver fixtures).
- A lightweight consistency check that ensures `guest-tools/config/devices.cmd` stays consistent with:
  - the Windows device contract (`protocol-vectors/windows-device-contract.json`) for the boot-critical virtio-blk
    storage service name and the exact virtio-blk/virtio-net PCI hardware IDs that Guest Tools seeds, and
  - the in-repo packaging specs (HWID regexes):
  - `win7-signed.json`
  - `win7-virtio-win.json`
  - `win7-virtio-full.json`
  - `win7-aero-guest-tools.json`
  - `win7-aero-virtio.json`

Separately, CI also runs the `Windows device contract` workflow (`.github/workflows/windows-device-contract.yml`),
which runs the Rust validator:

```bash
cargo run -p device-contract-validator --locked
```

This provides an additional guardrail that the **Windows device contract manifests** remain consistent with:

- Guest Tools config (`guest-tools/config/devices.cmd`)
- Packager specs (`tools/packaging/specs/*.json`)
- In-tree Win7 driver INFs (`drivers/**`)
- Emulator PCI ID constants (best-effort static checks)

You can run the same check locally:

```bash
python tools/guest-tools/validate_config.py --spec tools/packaging/specs/win7-signed.json
python tools/guest-tools/validate_config.py
python tools/guest-tools/validate_config.py --spec tools/packaging/specs/win7-virtio-full.json
python tools/guest-tools/validate_config.py --spec tools/packaging/specs/win7-aero-guest-tools.json
python tools/guest-tools/validate_config.py --spec tools/packaging/specs/win7-aero-virtio.json

# To validate the virtio-win contract variant, you must point the validator at a devices.cmd
# generated from that contract (or extracted from a virtio-win Guest Tools ZIP). The in-repo
# guest-tools/config/devices.cmd is generated from the canonical Aero contract.
python scripts/generate-guest-tools-devices-cmd.py --contract protocol-vectors/windows-device-contract-virtio-win.json --output /tmp/devices-virtio-win.cmd
python tools/guest-tools/validate_config.py --devices-cmd /tmp/devices-virtio-win.cmd --spec tools/packaging/specs/win7-virtio-win.json

# (Optional) If you are validating a devices.cmd copy that does not include a contract header,
# force the contract file explicitly:
# python tools/guest-tools/validate_config.py --devices-cmd /tmp/devices-virtio-win.cmd --windows-device-contract protocol-vectors/windows-device-contract-virtio-win.json --spec tools/packaging/specs/win7-virtio-win.json
```

## Windows 7 Install Media Servicing (WinPE/Setup) for Test-Signed Virtio Drivers

### Overview (why this exists)

Windows 7 **x64** enforces **kernel-mode driver signature enforcement (DSE)** very early in boot (boot-start drivers are verified/loaded by `winload.exe`/CI before most of the OS has started). If we want to boot/install Windows 7 using **test-signed** virtio storage/network drivers (e.g. virtio-blk/virtio-net) we must:

1. Ensure the boot environment is configured to allow test signatures (**`testsigning` in the BCD store**), and
2. Ensure the relevant OS environments trust the test certificate (**offline certificate injection into each image’s SOFTWARE hive**).

If either is missing, Windows Setup/WinPE may not load boot-critical drivers and installation can fail (e.g. disk not visible), or the installed OS can fail on first boot (e.g. `INACCESSIBLE_BOOT_DEVICE` when the storage driver is blocked).

This document answers, precisely:

- Which `boot.wim` and `install.wim` indices must be patched
- Where the offline registry hives and the BCD template live inside mounted images
- Repeatable `DISM` + `reg` + `bcdedit` commands to patch media

> Note: Windows 7 **x86** does not enforce kernel-mode driver signing in the same way as x64. This doc is primarily for **Windows 7 x64**.

See also:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) for the higher-level “what to patch” overview (ISO layout, ISO rebuild commands, validation checklist).

### Automated servicing (recommended)

For a repeatable, automated implementation of the steps in this document, use:

- [`tools/windows/patch-win7-media.ps1`](../../tools/windows/patch-win7-media.ps1)

It patches:

- Media BCD stores (`boot\BCD` and `efi\microsoft\boot\bcd` when present)
- Selected `boot.wim` + `install.wim` indices (driver injection optional)
- Offline certificate trust injection into each image’s SOFTWARE hive
- Offline `install.wim` `BCD-Template` so the installed OS inherits `testsigning`
- (Optional) Nested `winre.wim` inside each `install.wim` index (via `-PatchNestedWinRE`)

See [`tools/windows/README.md`](../decisions/README.md) for prerequisites and usage examples.

### Media variations (validated observations)

This repo cannot ship any Microsoft binaries (ISOs/WIMs/BCDs). The notes below only record *metadata and observed paths* relevant to tooling.

#### Validated vs assumed

Validated (empirically checked on real Windows 7 SP1 retail install media):

- `sources\boot.wim` contains 2 images (WinPE + Setup) and the install media boots the **Setup** image (BootIndex `2`).
- BIOS-mode install media BCD store is at `boot\BCD` and its WinPE loader(s) use `sources\boot.wim` as the ramdisk `device`/`osdevice`.
- x64 media contains a UEFI BCD store at `EFI\Microsoft\Boot\BCD` (casing varies by extraction tool; Windows is case-insensitive).
- `sources\install.wim` contains multiple edition images on multi-edition retail media.
- Each install image contains `Windows\System32\Config\BCD-Template`.
- Patching `BCD-Template` to set `testsigning on` results in an installed system whose `{current}` loader has `testsigning Yes` on first boot.

Assumed (not fully characterized across all OEM/custom media):

- OEM/custom install media may add extra `boot.wim` indices (e.g., recovery tools), additional BCD entries, or change which loader object is set as the boot default.
- Some single-edition retail ISOs ship an `install.wim` with only one image (instead of the multi-edition layout below).

Tooling should therefore:

- Never hardcode a single UEFI BCD location.
- Never assume `{default}` is the only loader object to patch.
- Prefer “patch all Windows Boot Loader objects that boot `boot.wim`” on install media, and “patch all Windows Boot Loader objects” in `BCD-Template`.

#### Compatibility matrix (observed)

| Media | `sources\boot.wim` images (`dism /Get-WimInfo`) | Booted WinPE image | BIOS BCD store | UEFI BCD store(s) | `sources\install.wim` images (`dism /Get-WimInfo`) | `BCD-Template` present | `BCD-Template` patch propagates to installed BCD |
|------|--------------------------------------------------|--------------------|----------------|-------------------|----------------------------------------------------|------------------------|--------------------------------------------------|
| Win7 SP1 x86 retail | 2: (1) Microsoft Windows PE (x86) (2) Microsoft Windows Setup (x86) | 2 | `boot\BCD` | (not present on the validated x86 retail media) | 5: Starter, Home Basic, Home Premium, Professional, Ultimate | Yes (all images) | Yes |
| Win7 SP1 x64 retail | 2: (1) Microsoft Windows PE (amd64) (2) Microsoft Windows Setup (amd64) | 2 | `boot\BCD` | `EFI\Microsoft\Boot\BCD` | 4: Home Basic, Home Premium, Professional, Ultimate | Yes (all images) | Yes |

Notes:

- The install-media BCD does **not** carry an obvious “WIM index” field; boot selection is controlled by the `boot.wim` **BootIndex** metadata. If you rebuild/export `boot.wim`, ensure the BootIndex remains correct or the media may boot the wrong image.
- When servicing `boot.wim` for installation-time drivers, **modify index 2** (Setup). Patch index 1 as well if you need recovery to load the same drivers/certs.
- When servicing `install.wim`, patch **all indices** so whichever edition the user installs will inherit the change.

---

### Mental model: what needs patching

There are three distinct “places” that matter for test-signed boot-start drivers:

1. **The bootable BCD store(s) on the installation media** (controls how WinPE/Setup is booted)
   - BIOS: `boot\BCD`
   - UEFI: `efi\microsoft\boot\bcd` (if present)
2. **`boot.wim`** (WinPE/Setup itself; needs the test certificate, and often the drivers)
3. **`install.wim`** (the installed OS image; needs the test certificate and a patched `BCD-Template`)

---

### `boot.wim` structure (WinPE/Setup)

#### Typical indices

On Windows 7 install media, `sources\boot.wim` usually contains:

- **Index 1**: Windows PE / WinRE (recovery tools; “Repair your computer”)
- **Index 2**: Windows Setup (the environment that runs `setup.exe`)

#### Which index must be patched (and why)

Install media typically boots whatever `boot.wim`’s **BootIndex** is set to (commonly **2**). That means:

- **Mandatory:** Patch **`boot.wim` Index 2** (Windows Setup), because it is what the ISO normally boots into.
- **Recommended:** Patch **`boot.wim` Index 1** as well if you want **WinRE/Recovery** to also be able to see disks/network using the same custom drivers.

To avoid guessing, read the BootIndex:

```bat
dism /Get-WimInfo /WimFile:C:\iso\sources\boot.wim
```

Look for `Boot Index : 2` (or similar) in the output.

---

### `install.wim` structure (installed OS)

`sources\install.wim` contains one image per edition (Starter/Home/Pro/Ultimate, etc). Example implications:

- If you always deploy a **single known edition**, patch only that **one index**.
- If you want to keep the ISO **multi-edition**, patch **all indices** so every edition boots with the same policy/certificate.

Determine the index list:

```bat
dism /Get-WimInfo /WimFile:C:\iso\sources\install.wim
```

#### Optional extra: nested WinRE (`winre.wim`)

Inside each `install.wim` image, Windows Recovery Environment is typically stored as:

`Windows\System32\Recovery\winre.wim`

If you need recovery to load the same test-signed storage/network drivers, you may also need to mount and patch that `winre.wim` (it is a WIM inside a WIM).

---

### Exact offline paths inside mounted images (what/where)

When a WIM is mounted to (say) `C:\mount\img`, these are the key files:

| Purpose | Path inside mounted image |
| --- | --- |
| SYSTEM hive (driver/service config, etc.) | `C:\mount\img\Windows\System32\Config\SYSTEM` |
| SOFTWARE hive (certificate stores live here) | `C:\mount\img\Windows\System32\Config\SOFTWARE` |
| BCD template used by Setup/`bcdboot` | `C:\mount\img\Windows\System32\Config\BCD-Template` |

---

### Certificate injection (offline) — how it works

Windows “Local Machine” certificate stores are registry-backed under the SOFTWARE hive. Conceptually:

- Store location: `HKLM\SOFTWARE\Microsoft\SystemCertificates\<STORE>\Certificates`
- Each certificate is a subkey named by its **SHA-1 thumbprint** (no spaces, typically uppercase)
- The certificate entry is stored as one or more values (typically a `REG_BINARY` value named **`Blob`**)
  written by CryptoAPI's registry-backed cert store provider (**not guaranteed to be raw DER**)

For test-signed kernel drivers, it is common to install the test certificate into both:

- `ROOT` (Trusted Root Certification Authorities)
- `TrustedPublisher` (Trusted Publishers)

Offline, that becomes:

- `HKLM\<OFFLINE_SOFTWARE>\Microsoft\SystemCertificates\ROOT\Certificates\<thumbprint>\Blob`
- `HKLM\<OFFLINE_SOFTWARE>\Microsoft\SystemCertificates\TrustedPublisher\Certificates\<thumbprint>\Blob`

#### Recommended: inject a certificate into an offline-mounted image using CryptoAPI

Rather than hand-writing registry values, use the Windows-native `tools/win-offline-cert-injector`,
which loads the offline SOFTWARE hive and uses CryptoAPI to create the exact registry-backed store entry.

Run from an elevated PowerShell prompt:

```powershell
$MountDir = "C:\mount\boot2"  # change per image
$CertPath = "C:\certs\aero-test.cer"  # CI output: out/certs/aero-test.cer

# Build once (or use a prebuilt binary)
cd tools\win-offline-cert-injector
cargo build --release --locked

.\target\release\win-offline-cert-injector.exe `
  --windows-dir $MountDir `
  --store ROOT --store TrustedPublisher `
  --cert $CertPath
```

---

### BCD edits required (WinPE + installed OS)

#### Patch the **media boot BCD store(s)** (required for WinPE/Setup)

Changing `startnet.cmd` / `winpeshl.ini` is **not sufficient** for driver signature enforcement: boot-start drivers are validated before those scripts run. The setting must be present in the **BCD store used to boot WinPE**.

On the extracted ISO contents (example `C:\iso\...`):

```bat
:: BIOS boot path (always on Win7 media)
bcdedit /store C:\iso\boot\BCD /set {default} testsigning on

:: Optional: disables integrity checks entirely (lab use only)
:: bcdedit /store C:\iso\boot\BCD /set {default} nointegritychecks on

:: UEFI boot path (only if your ISO contains it)
if exist C:\iso\efi\microsoft\boot\bcd (
  bcdedit /store C:\iso\efi\microsoft\boot\bcd /set {default} testsigning on
)
```

Notes:

- Prefer `testsigning on` when using test-signed drivers + injected cert.
- Use `nointegritychecks on` only when you understand the security implications and are in a controlled test environment.

#### Patch the **`BCD-Template` inside `install.wim`** (required for the installed OS)

Windows Setup creates the installed OS boot store using the template at:

`Windows\System32\Config\BCD-Template`

Patch that template **inside each `install.wim` index you intend to deploy** so newly-created boot stores inherit `testsigning`:

```bat
:: After mounting install.wim index N to C:\mount\installN
bcdedit /store C:\mount\installN\Windows\System32\Config\BCD-Template /set {default} testsigning on

:: Optional (lab only)
:: bcdedit /store C:\mount\installN\Windows\System32\Config\BCD-Template /set {default} nointegritychecks on
```

If `bcdedit` complains that `{default}` is not found, enumerate and pick the Windows Boot Loader identifier:

```bat
bcdedit /store C:\mount\installN\Windows\System32\Config\BCD-Template /enum all
```

---

### Repeatable servicing workflow (DISM/reg/bcdedit)

This is an end-to-end “do it every time” sequence. Assumes you have already extracted the ISO to `C:\iso`.

#### Inspect WIM indices (don’t guess)

```bat
dism /Get-WimInfo /WimFile:C:\iso\sources\boot.wim
dism /Get-WimInfo /WimFile:C:\iso\sources\install.wim
```

#### Patch ISO boot BCD stores (WinPE/Setup)

```bat
bcdedit /store C:\iso\boot\BCD /set {default} testsigning on
if exist C:\iso\efi\microsoft\boot\bcd (
  bcdedit /store C:\iso\efi\microsoft\boot\bcd /set {default} testsigning on
)
```

#### Patch `boot.wim` (WinPE/Setup)

Mount, inject cert (and optionally drivers), commit.

```bat
md C:\mount\boot2
dism /Mount-Wim /WimFile:C:\iso\sources\boot.wim /Index:2 /MountDir:C:\mount\boot2

:: Inject test cert (see PowerShell snippet above; set $MountDir=C:\mount\boot2)
:: Optional: add virtio drivers to WinPE/Setup
:: dism /Image:C:\mount\boot2 /Add-Driver /Driver:C:\drivers\virtio\win7\amd64 /Recurse

dism /Unmount-Wim /MountDir:C:\mount\boot2 /Commit
rd C:\mount\boot2
```

Optional (recommended for recovery):

```bat
md C:\mount\boot1
dism /Mount-Wim /WimFile:C:\iso\sources\boot.wim /Index:1 /MountDir:C:\mount\boot1

:: Inject test cert (set $MountDir=C:\mount\boot1)
:: Optional: add recovery drivers

dism /Unmount-Wim /MountDir:C:\mount\boot1 /Commit
rd C:\mount\boot1
```

#### Patch `install.wim` (installed OS image)

Repeat for each edition index you want to support:

```bat
md C:\mount\installN
dism /Mount-Wim /WimFile:C:\iso\sources\install.wim /Index:<N> /MountDir:C:\mount\installN

:: Inject test cert (set $MountDir=C:\mount\installN)

:: Patch BCD template so the installed OS boots with testsigning
bcdedit /store C:\mount\installN\Windows\System32\Config\BCD-Template /set {default} testsigning on

:: Optional: add virtio drivers into the installed OS image
:: dism /Image:C:\mount\installN /Add-Driver /Driver:C:\drivers\virtio\win7\amd64 /Recurse

dism /Unmount-Wim /MountDir:C:\mount\installN /Commit
rd C:\mount\installN
```

#### (Optional) Patch nested `winre.wim` inside `install.wim`

If you need recovery to understand the same storage/network devices:

1. Mount `install.wim` index N.
2. Copy `Windows\System32\Recovery\winre.wim` out to a working path.
3. Mount and patch `winre.wim` (it is usually a single-index WinPE image).
4. Replace it back into the mounted `install.wim` image.
5. Commit/unmount the `install.wim`.

---

### WinPE caveats (common pitfalls)

- **Editing `startnet.cmd` / `winpeshl.ini` does not bypass DSE** for boot-start drivers. Those scripts run *after* the kernel has already decided which boot-start drivers it will load.
- For WinPE/Setup, the critical policy is the **BCD store on the media** (`boot\BCD` / `efi\...\bcd`), not a setting inside the WIM.
- For the installed OS, patching **`BCD-Template`** inside `install.wim` is how you get `testsigning` set *before the first boot* of the deployed system.

---

### Verification checklist (don’t ship a broken ISO)

#### Confirm WIM indices + BootIndex

```bat
dism /Get-WimInfo /WimFile:C:\iso\sources\boot.wim
dism /Get-WimInfo /WimFile:C:\iso\sources\install.wim
```

#### Confirm BCD settings (media)

```bat
bcdedit /store C:\iso\boot\BCD /enum {default}

:: If present
bcdedit /store C:\iso\efi\microsoft\boot\bcd /enum {default}
```

Look for:

- `testsigning                Yes`
- (optional) `nointegritychecks          Yes`

#### Confirm certificate presence in an offline image

```bat
reg load HKLM\OFFSOFT C:\mount\boot2\Windows\System32\Config\SOFTWARE
reg query "HKLM\OFFSOFT\Microsoft\SystemCertificates\ROOT\Certificates"
reg query "HKLM\OFFSOFT\Microsoft\SystemCertificates\TrustedPublisher\Certificates"
reg unload HKLM\OFFSOFT
```

You should see a subkey whose name matches your certificate thumbprint (no spaces).

## Windows 7 Offline Media Patcher (testsigning + certificate injection)

This repository includes a Windows-only helper that patches a **user-provided, extracted Windows 7 SP1 ISO** directory so it can boot and install using **test-signed drivers** (most importantly: boot-critical storage drivers).

Recommended script:

- `tools/windows/patch-win7-media.ps1` (see [`tools/windows/README.md`](../decisions/README.md))

The patcher **does not ship any Microsoft binaries/images**; it only edits the extracted ISO files you point it at.

It can:

- Enable `testsigning` (and optionally `nointegritychecks`) in the install-media BCD store(s)
- Patch `BCD-Template` inside `install.wim` so the installed OS inherits the same boot policy
- Inject a public signing certificate into offline `ROOT` + `TrustedPublisher` (by default) inside:
  - `boot.wim` (WinPE/Setup)
  - `install.wim` (installed OS)
- Optionally inject drivers from a directory containing `.inf` files (via `-DriversPath`)

See also:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) (background + manual workflow)
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) (auditable, longer-form guide)

---

### Prerequisites

- Windows 10/11 host (recommended)
- Run from an **elevated** PowerShell prompt (Administrator)
- PowerShell 5.1+ (Windows PowerShell or PowerShell 7)
- Built-in tools:
  - `dism.exe`
  - `bcdedit.exe`
  - `attrib.exe`
  - `reg.exe`
- `win-offline-cert-injector.exe` (build once from `tools/win-offline-cert-injector/`)

```powershell
cd tools\win-offline-cert-injector
cargo build --release --locked
```

---

### Usage

1) Extract a Windows 7 SP1 ISO to a folder (example: `C:\win7-iso\`).

2) Run the patch script as Administrator.

Example (CI-style test-signed drivers + cert):

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\windows\patch-win7-media.ps1 `
  -MediaRoot C:\win7-iso `
  -CertPath  .\out\certs\aero-test.cer `
  -DriversPath .\out\packages
```

Example (patch signing policy + cert trust only; no driver injection):

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\windows\patch-win7-media.ps1 `
  -MediaRoot C:\win7-iso `
  -CertPath  C:\path\to\driver-test.cer
```

---

### Optional flags

- Enable `nointegritychecks` as well (**not recommended**; only for lab bring-up):

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\windows\patch-win7-media.ps1 `
  -MediaRoot C:\win7-iso `
  -CertPath  C:\path\to\driver-test.cer `
  -EnableNoIntegrityChecks
```

- Patch only `boot.wim` Setup image (index 2):

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\windows\patch-win7-media.ps1 `
  -MediaRoot C:\win7-iso `
  -CertPath  C:\path\to\driver-test.cer `
  -BootWimIndices 2
```

- Patch a subset of `install.wim` indices:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\windows\patch-win7-media.ps1 `
  -MediaRoot C:\win7-iso `
  -CertPath  C:\path\to\driver-test.cer `
  -InstallWimIndices "1,4"
```

- Inject the certificate into additional stores (example: `TrustedPeople` + `CA`):

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\windows\patch-win7-media.ps1 `
  -MediaRoot C:\win7-iso `
  -CertPath  C:\path\to\driver-test.cer `
  -CertStores ROOT,CA,TrustedPublisher,TrustedPeople
```

---

### What gets patched (high level)

#### BCD stores on the extracted ISO folder

Patched via `bcdedit /store`:

- `boot\BCD` (BIOS/CSM)
- `efi\microsoft\boot\bcd` (UEFI, if present)

The script enables:

- `testsigning on`
- `nointegritychecks on` only when `-EnableNoIntegrityChecks` is passed

#### Offline registry hives inside WIM images (certificate trust)

For each selected mounted WIM index, the script calls `win-offline-cert-injector` to inject the certificate into the offline `SOFTWARE` hive under:

- `Microsoft\SystemCertificates\ROOT`
- `Microsoft\SystemCertificates\TrustedPublisher`

#### `BCD-Template` inside `install.wim` (installed OS boot policy)

Patched via `bcdedit /store`:

- `<MountDir>\Windows\System32\Config\BCD-Template`

---

### Verification

From the patched ISO folder on the host:

```powershell
bcdedit /store C:\win7-iso\boot\BCD /enum {default}
if (Test-Path C:\win7-iso\efi\microsoft\boot\bcd) {
  bcdedit /store C:\win7-iso\efi\microsoft\boot\bcd /enum {default}
}
```

In WinPE/Setup or the installed OS:

```cmd
bcdedit /enum {current}
certutil -store Root
certutil -store TrustedPublisher
```

---

### Notes / Safety

- The script **modifies files in place**. Work on a copy of your extracted ISO directory if you want to preserve the original.
- If a run is interrupted, you may need to clean up stuck WIM mounts manually (`dism /Cleanup-Wim`).

## Windows 7 SP1 Unattended Install (Driver Injection + Post-Install Scripting)

This project needs a repeatable, zero-touch Windows 7 SP1 install path where:

- Users supply their own Windows 7 SP1 ISO (x86 or x64).
- Aero supplies only **open-source drivers** (virtio storage/NIC, virtual GPU, etc.) and **scripts**.

This document focuses on the *plumbing* required for:

1. Loading drivers early enough for Windows Setup to see the disk/network.
2. Staging additional drivers into the offline OS so they’re available on first boot.
3. Running scripts during setup to enable test-signing and install drivers.

See also the reference templates in [`guest-tools/unattend/`](../../guest-tools/unattend/).

For the broader install-media preparation workflow (ISO layout, what must be patched in WIM/BCD/registry hives, ISO rebuild commands), see:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md)

For a step-by-step **validation + troubleshooting playbook** on real Win7 SP1 installs (logs, `%configsetroot%` verification, `$OEM$` copy behavior, `SetupComplete.cmd` checks), see:
 
* [`../areas/windows-drivers.md`](../areas/windows-drivers.md)

---

### Windows 7 setup pass order (what runs when)

Windows 7 unattended setup is driven by passes in `autounattend.xml`. The high-level flow for a clean install looks like:

1. **`windowsPE`** (WinPE / Windows Setup environment)
   - Disk partitioning / formatting
   - Choosing which image to apply from `install.wim`
   - Accepting EULA
   - **Loading setup-critical drivers** (storage, NIC)
2. **`offlineServicing`** (offline OS image on disk, not booted yet)
   - **Staging drivers into the offline OS driver store**
   - (Also used for offline package/feature injection, if needed)
3. **Reboot** into the newly applied OS
4. **`specialize`** (first boot configuration; runs as **SYSTEM**)
   - Computer- and machine-level configuration
   - **`RunSynchronous`** hooks (SYSTEM) for early scripting
5. **`oobeSystem`** (OOBE / first-run; some parts run as **SYSTEM**, some as the first user)
   - User creation / OOBE suppression
   - **`FirstLogonCommands`** hooks (user context)
6. **`%WINDIR%\\Setup\\Scripts\\SetupComplete.cmd`** and first logon
   - `SetupComplete.cmd` runs **once** as **SYSTEM** near the end of setup (before the first logon)
   - First logon triggers `FirstLogonCommands` (if configured)

> Note: The exact boundaries between “end of setup”, `SetupComplete.cmd`, and “first logon” can be confusing. The practical takeaway is: if you need **SYSTEM context without depending on a user session**, prefer `specialize` / `SetupComplete.cmd`.

---

### Driver injection (WinPE vs. offline OS)

Windows 7 has two different unattend components for drivers:

- **WinPE/setup environment (setup-critical):** `Microsoft-Windows-PnpCustomizationsWinPE` (`windowsPE`)
- **Offline OS staging (post-apply):** `Microsoft-Windows-PnpCustomizationsNonWinPE` (`offlineServicing`)

These solve different problems and are often used together.

#### Loading setup-critical drivers in WinPE (`windowsPE`)

**Component:** `Microsoft-Windows-PnpCustomizationsWinPE`  
**Pass:** `windowsPE`  
**Purpose:** Make drivers available *while Windows Setup is running in WinPE*.

This is the right place for drivers needed for:

- Storage controller access (so Setup can see the target disk)
- Network access (if you plan to pull content from the network during setup)

**Key setting:**

`DriverPaths/PathAndCredentials/Path` — a list of directories containing `.inf` driver packages.

Typical pattern (see templates):

- Put storage/NIC drivers under `Drivers\WinPE\<arch>\...`
- Point `DriverPaths` at that directory

> Verify on real Win7 setup: whether `DriverPaths` is scanned recursively for `.inf` files vs. only the top directory can vary across tooling and versions. To avoid surprises, keep drivers in a shallow directory structure or add multiple `PathAndCredentials` entries.

#### Staging drivers into the offline OS (`offlineServicing`)

**Component:** `Microsoft-Windows-PnpCustomizationsNonWinPE`  
**Pass:** `offlineServicing`  
**Purpose:** Add driver packages to the *offline* Windows installation on disk.

This is analogous to doing `DISM /Add-Driver` against the target `Windows` directory: the drivers are placed into the offline driver store so that on the first boot Windows can install them when it enumerates hardware.

Typical uses:

- GPU drivers
- Virtio drivers that are not strictly required for WinPE setup, but should be present in the installed OS
- Any device driver you want Windows to “just find” after first boot (INF-based packages)

> Important: This only stages **INF-based** drivers. If a vendor driver is only distributed as an interactive installer (`.exe` / `.msi`), it won’t be installed by `offlineServicing`—use `SetupComplete.cmd` or another scripting hook instead.

---

### Script execution hooks (where to run what)

Windows 7 gives multiple places to run commands; the main difference is **timing** and **security context**.

#### Hook A: `specialize` → `Microsoft-Windows-Deployment` → `RunSynchronous` (SYSTEM)

- Runs on first boot after the image is applied.
- Runs as **LocalSystem**.
- Good for machine-level changes and for copying files into `%WINDIR%` locations.

Common uses in this project:

- Copy `SetupComplete.cmd` from installation/config media onto the installed OS.
- Copy drivers/certs to a stable path on the system drive.
- Enable test-signing early (though it still requires a reboot to take effect).

#### Hook B: `oobeSystem` → `Microsoft-Windows-Shell-Setup` → `FirstLogonCommands` (user)

- Runs when a user logs in for the first time.
- Runs in the **user context** (which may or may not be an administrator).

Use this for tasks that require a user profile or per-user configuration. For fully unattended usage, pair it with `AutoLogon` or ensure OOBE creates/logs into the intended account automatically.

#### Hook C: `%WINDIR%\\Setup\\Scripts\\SetupComplete.cmd` (SYSTEM, runs once)

- If present, Windows Setup runs it near the end of setup.
- Runs as **LocalSystem**.
- Runs **once** (per install).

This is often the easiest place to:

- Import certificates into LocalMachine stores
- Install INF drivers via `pnputil` (Win7 has `pnputil.exe`; exact flags differ by OS version—verify on Win7 SP1)
- Trigger a reboot (for example after enabling test signing)

#### Recommended: use Aero's unattended scripts

This repo includes Win7 SP1-compatible post-install automation scripts that:

- Import an optional test certificate
- Enable test signing (`bcdedit /set testsigning on`)
- Install all `*.inf` packages under a `Drivers\` folder via `pnputil`
- Avoid reboot loops via marker files and self-deleting scheduled tasks

See:

- [`guest-tools/unattend/scripts/`](../../guest-tools/unattend/scripts/)
- [`guest-tools/unattend/scripts/README.md`](../decisions/README.md)
- [`guest-tools/unattend/scripts/SetupComplete.cmd`](../../guest-tools/unattend/scripts/SetupComplete.cmd)
- [`guest-tools/unattend/scripts/InstallDriversOnce.cmd`](../../guest-tools/unattend/scripts/InstallDriversOnce.cmd)

#### Example: `SetupComplete.cmd` skeleton

If you ship a config ISO that includes `Scripts\\SetupComplete.cmd`, you can use it to enable test signing, trust your signing certificate, and stage/install drivers.

Minimal example (treat this as a starting point and **verify on real Win7 SP1**):

```bat
@echo off
setlocal EnableExtensions

set LOG=%WINDIR%\Temp\Aero-SetupComplete.log
echo [%DATE% %TIME%] SetupComplete starting>>"%LOG%"

REM If you used UseConfigurationSet=true, Setup may provide %configsetroot%.
REM For robustness, copy payloads to C:\Aero\ during specialize (or use the
REM production scripts in guest-tools/unattend/scripts/, which do this by default).
set SRC=%configsetroot%
if not defined configsetroot (
  echo [%DATE% %TIME%] WARNING: configsetroot is not set>>"%LOG%"
)

REM Enable test signing (requires reboot to take effect)
bcdedit /set testsigning on>>"%LOG%" 2>&1

REM Trust the driver signing certificate (optional; adjust file name/store as needed)
set CERT=%SRC%\Cert\aero-test.cer
if not exist "%CERT%" set CERT=%SRC%\Cert\aero_test.cer
if not exist "%CERT%" set CERT=%SRC%\Cert\aero-test-root.cer
if not exist "%CERT%" set CERT=%SRC%\Certs\AeroTestRoot.cer
if exist "%CERT%" (
  certutil -addstore -f Root "%CERT%">>"%LOG%" 2>&1
  certutil -addstore -f TrustedPublisher "%CERT%">>"%LOG%" 2>&1
)

REM Stage/install INF drivers (verify pnputil flags on Win7 SP1)
REM - Staging only: pnputil -a <inf>
REM - Stage + install: pnputil -i -a <inf>
if exist "%SRC%\Drivers\Offline\amd64" (
  for /r "%SRC%\Drivers\Offline\amd64" %%I in (*.inf) do (
    pnputil -i -a "%%I">>"%LOG%" 2>&1
  )
)

REM Reboot if you enabled test signing (otherwise you can remove this)
shutdown /r /t 0
```

> Note: For x86 installs, change the driver folder to `...\Offline\x86`. For robustness, copy needed drivers/certs from `%configsetroot%` to a stable location (for example `C:\Aero\`) during the `specialize` pass. The production `SetupComplete.cmd` in `guest-tools/unattend/scripts/` performs this copy by default.

---

### Test-signing for Windows 7 x64 (test-signed drivers)

Windows 7 x64 enforces kernel-mode driver signing. If Aero’s drivers are test-signed (common for internal/open-source builds), you typically need both:

1. Windows booted with **test signing enabled**, and
2. The **signing certificate trusted** by the local machine.

#### SHA-2 update note (KB3033929 / KB4474419)

If your driver packages (or the signing certificate itself) use **SHA-256** signatures, stock Windows 7 SP1 may be unable to validate them until SHA-2 support updates are installed (commonly **KB3033929** and **KB4474419**).

If you see signature/trust failures (for example Device Manager **Code 52**), see:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md#issue-missing-kb3033929-sha-256-signature-support)

#### Enable test signing (requires reboot)

Run as Administrator (SYSTEM is fine):

```bat
bcdedit /set testsigning on
```

This setting does **not** take effect until you reboot.

#### Import the signing certificate (LocalMachine)

Import into both `Root` and `TrustedPublisher`:

```bat
certutil -addstore -f Root "%configsetroot%\\Cert\\aero-test.cer"
certutil -addstore -f TrustedPublisher "%configsetroot%\\Cert\\aero-test.cer"
```

The unattended scripts in this repo accept several common certificate file names:

- `Cert\\aero-test.cer` (preferred; matches CI output)
- `Cert\\aero_test.cer` (accepted)
- `Cert\\aero-test-root.cer`
- `Certs\\AeroTestRoot.cer` (legacy)

> Verify on real Win7 setup: the best store(s) depend on how the drivers are signed (cross-signed vs. test-signed). The above is a common baseline for test-signed driver packages.

#### Suggested sequencing

If you’re enabling test signing during setup:

1. Run `bcdedit /set testsigning on`
2. Copy/import the cert
3. Reboot
4. Install drivers (or let staged drivers install automatically)

In practice, steps 1–3 fit well in `SetupComplete.cmd`, followed by `shutdown /r /t 0`.

---

### Recommended packaging strategy (no Windows file redistribution): “config ISO”

The recommended approach is to ship a **separate, tiny “config ISO”** (or USB image) that contains only:

- `autounattend.xml`
- driver `.inf` packages
- scripts (`SetupComplete.cmd`, etc.)
- certificates (optional)

This avoids redistributing any Microsoft files and makes the process reproducible.

> Note: If you only need drivers for an interactive install (Windows Setup → **Load Driver**),
> CI can also produce a small FAT32 “driver disk” (`*-fat.vhd`) that you attach as a secondary disk.
> See: [`../areas/windows-drivers.md`](../areas/windows-drivers.md).

#### Expected config media layout

The reference templates assume a layout like:

```
<config-media-root>/
  autounattend.xml
  Drivers/
    WinPE/
      amd64/   (storage/NIC drivers needed by WinPE/Setup)
      x86/
    Offline/
      amd64/   (drivers to stage into installed OS)
      x86/
  Scripts/
    SetupComplete.cmd
    InstallDriversOnce.cmd
    FirstLogon.cmd        (optional)
  Cert/
    aero-test.cer         (optional; preferred; matches CI output)
    aero_test.cer         (optional; accepted)
    aero-test-root.cer    (optional; accepted)
  Certs/
    AeroTestRoot.cer      (optional; accepted for compatibility)
```

#### Keeping access to config content after the first reboot

**Setting:** `Microsoft-Windows-Setup` → `UseConfigurationSet=true`

When enabled, Setup treats the media containing `autounattend.xml` as a *configuration set* and copies its contents onto the target disk so later passes can still access them (commonly via `%configsetroot%`).

> Verify on real Win7 setup: the exact copy location and the lifetime/availability of `%configsetroot%` varies across Windows versions and deployment scenarios. For robustness, keep the config ISO attached through first boot.
>
> The reference templates (and Aero's Win7 unattended scripts) can stage the payload into `C:\Aero\` during `specialize` so later phases don't depend on removable/config media remaining attached.

---

### Advanced strategy: slipstream drivers into `boot.wim` / `install.wim`

For advanced users (or for edge cases where you can’t rely on external config media), you can inject drivers directly into the Windows images:

- `boot.wim` (the WinPE/Setup environment)
- `install.wim` (the actual OS image)

High-level workflow (Windows host with DISM):

1. Mount `boot.wim` (typically index 2: “Microsoft Windows Setup”)
2. `dism /image:<mount> /add-driver /driver:<path> [/recurse]`
3. Unmount/commit
4. Repeat for `install.wim` (the index you plan to install)
5. Rebuild the ISO

Linux-friendly tooling: `wimlib-imagex` can mount/edit WIMs, but the exact commands depend on your environment.

Tradeoffs:

- More complex and error-prone
- Must be repeated for each ISO/edition
- Modifies Microsoft media (still fine for personal use, but changes your reproducibility story)

For Aero’s use case, the config ISO + unattend driver paths is the preferred baseline.

## Windows 7 SP1 Install Media Preparation (Slipstreaming)

### Overview

Aero cannot distribute Microsoft Windows binaries. Users (or developers) must supply their own **Windows 7 SP1** installation ISO and prepare it locally.

This document describes an **auditable, reproducible** process for preparing a Win7 SP1 install image that:

- Loads **Aero storage drivers** during Windows Setup (WinPE), so disks are visible.
- Installs Aero drivers into the **installed OS image** so Windows can boot after installation.
- Configures **driver signature policy** (prefer test-signing; fallback to disabling integrity checks).
- Installs the **Aero test root certificate** into WinPE and the installed OS so test-signed drivers are trusted.

Note: If you are using Aero’s **baseline** Windows 7 install topology (AHCI HDD + IDE/ATAPI CD-ROM),
Windows Setup should be able to see the disk using Windows 7’s in-box AHCI driver. This document is
primarily needed when you want to install/boot using **paravirtual** or otherwise non-inbox storage
devices (e.g. virtio-blk). For the baseline topology details (canonical PCI BDFs, attachment mapping,
and interrupt routing), see [`../areas/storage.md`](../areas/storage.md).

This is written to be executable manually today, and to serve as a reference for future automation (see `tools/win7-slipstream/templates/`).

Related references in this repo:

- `../areas/windows-drivers.md` (more detailed, Windows-first DISM/reg/bcd workflows)
- `../areas/windows-drivers.md` (unattended install details and hooks)
- `../areas/windows-drivers.md` (practical validation/troubleshooting playbook for unattended installs with a separate config ISO)
- `../areas/windows-drivers.md` (BCD internals and robust offline patching strategy)
- `guest-tools/unattend/` (ready-to-edit `autounattend.xml` templates and Win7-compatible post-install scripts)
- `tools/win7-slipstream/patches/README.md` (auditable `.reg` patches for offline BCD + SOFTWARE hives)
- `tools/bcd_patch/` (cross-platform CLI to patch BCD stores without `bcdedit.exe`)
- `tools/win-offline-cert-injector/` (Windows-native CLI for injecting certs into offline SOFTWARE hives)
- `tools/windows/patch-win7-media.ps1` (Windows-only helper that applies the same kinds of patches programmatically)

---

### Supported input ISOs (expected layout)

This process assumes a Windows 7 **SP1** ISO (x86 or x64) with the standard layout:

- `sources/boot.wim` (WinPE + Windows Setup)
- `sources/install.wim` (the OS images / editions)
- BIOS boot BCD store: `boot/BCD`
- UEFI boot BCD store (x64 media typically): `efi/microsoft/boot/BCD`

Notes:

- Some OEM media may have a different layout or additional boot entries; always validate the structure first.
- x86 media may not include `efi/…` at all (BIOS-only).
- Aero driver injection must match the ISO architecture (x86 drivers for x86 media; amd64 drivers for x64 media).
- On case-sensitive host filesystems (Linux/macOS), extracted ISO file paths may differ in case (for example `efi/microsoft/boot/bcd`). Treat these paths as **case-insensitive identifiers**, and verify the actual extracted filenames before running commands that reference them.

#### Quick structure checks

On any platform:

```sh
# Confirm WIM files exist
test -f sources/boot.wim && test -f sources/install.wim

# Optional: list WIM indexes (requires wimlib)
wimlib-imagex info sources/boot.wim
wimlib-imagex info sources/install.wim
```

---

### What must be patched (minimum)

To boot and install Windows 7 with Aero’s custom kernel-mode drivers (especially storage, and likely graphics later), the install media typically needs all of the following:

1. **WinPE (boot.wim) must contain the required drivers**
   - At minimum: storage driver(s) so Setup can see the disk.
   - Recommended: inject into both indexes of `boot.wim` (WinPE + Setup).

2. **Install image (install.wim) must contain the required drivers**
   - At minimum: storage driver(s) so the installed OS can boot.
   - Recommended: inject into the specific edition index you will install, or all indexes if unsure.

3. **WinPE boot BCD settings on the ISO**
   - Patch both `boot/BCD` (BIOS) and `efi/microsoft/boot/BCD` (UEFI if present).
   - Enable the required signature mode for WinPE so Setup can load Aero drivers:
     - Preferred: `testsigning on` (with Aero test root cert installed in WinPE).
     - Fallback (emulator-only): `nointegritychecks on` (security tradeoff; see below).

4. **Installed OS boot settings**
   - Prefer patching the image’s BCD template at:
     - `install.wim:<index>\Windows\System32\config\BCD-Template`
   - This ensures newly installed Windows boots with the same signature mode as required for Aero drivers.

5. **Certificate trust (offline registry)**
   - Install Aero’s test root certificate into the **offline SOFTWARE hive** for:
     - WinPE image(s): `boot.wim:<index>\Windows\System32\config\SOFTWARE`
     - Installed OS image(s): `install.wim:<index>\Windows\System32\config\SOFTWARE`
   - This enables the OS to trust **test-signed** Aero drivers from first boot.

---

### Driver injection strategies

#### Strategy A (Windows host): full offline injection with DISM (recommended)

Use Windows DISM + bcdedit to inject drivers/certs directly into `boot.wim` and `install.wim`, and patch BCD stores.

Pros:
- Most direct and well-supported.
- Easy to validate using DISM.

Cons:
- Requires Windows (or Windows VM) with DISM available.

#### Strategy B (cross-platform host): unattend-based injection (drivers staged on media)

Stage drivers on the install media and use `autounattend.xml` to:

- Point WinPE (`PnpCustomizationsWinPE`) at driver directories on the media.
- Point offline servicing (`PnpCustomizationsNonWinPE`) at driver directories to stage into the installed image.

Pros:
- Works on Linux/macOS without DISM for *driver injection*.
- Keeps changes visible as files on the media.

Cons:
- You still need to handle signature policy + certificate trust.
- For boot-critical drivers, you still must ensure the installed OS trusts the signing cert from first boot.

Ready-to-edit unattend examples live at `guest-tools/unattend/`. Golden-reference templates (placeholders intended for future tooling) live at `tools/win7-slipstream/templates/`.

---

### Driver signature strategy

#### Preferred: test-signed drivers + Aero test root cert + testsigning enabled

This is the best balance for emulator development:

- Drivers are cryptographically signed (auditability).
- Windows is explicitly placed into test mode.
- Trust is limited to your test root cert (instead of “trust everything”).

Requirements:

- All Aero kernel-mode drivers should be signed with an Aero test certificate.
- The corresponding test root cert must be present in:
  - `LocalMachine\\Root` (Trusted Root Certification Authorities)
  - `LocalMachine\\TrustedPublisher` (Trusted Publishers)
- BCD must have `testsigning on` (WinPE and installed OS).

#### Fallback: disable integrity checks (`nointegritychecks`)

This is **not recommended** except for controlled environments (e.g., inside the Aero emulator during early bring-up):

- Disables kernel driver signature enforcement.
- Makes it much easier for malicious/unintended drivers to load.

Requirements:

- BCD must have `nointegritychecks on`.
- (Certificate injection is optional, but still recommended for later tightening.)

---

### Recommended media-side file layout (for auditing)

There are two common layouts, depending on whether you are directly modifying the Windows ISO or supplying a separate “config media” ISO/USB for unattend.

#### Option A: single `aero/` directory on the Windows ISO

Keep Aero additions under a single directory in the ISO root (example):

```
aero/
  certs/
    aero-test.cer
  drivers/
    winpe/        # storage drivers needed during Setup
    system/       # drivers to be present in installed OS
  scripts/
    SetupComplete.cmd
    FirstLogon.ps1
```

Even when using DISM to inject drivers, keeping a copy of the exact inputs on the ISO is useful for auditing and later debugging.

#### Option B: `%configsetroot%` “configuration set” layout (recommended for unattend workflows)

If you use the repo’s architecture-specific templates in `guest-tools/unattend/`, they assume a config-media layout like:

```
<config-media-root>/
  autounattend.xml
  Drivers/
    WinPE/
      amd64/
      x86/
    Offline/
      amd64/
      x86/
  Scripts/
    SetupComplete.cmd
    InstallDriversOnce.cmd
  Cert/
    aero-test.cer
```

See `guest-tools/unattend/README.md` for the full expected structure and payload-location fallbacks.

---

### Strategy A: Windows-only offline injection (DISM + bcdedit)

#### Prerequisites

- Fast path: if you just want a repeatable one-command workflow (drivers + offline cert trust + BCD patching), use:
  - `tools/windows/patch-win7-media.ps1` (see `tools/windows/README.md`)

- Example:
  ```powershell
  pwsh .\tools\windows\patch-win7-media.ps1 `
    -MediaRoot C:\win7-slipstream\iso `
    -CertPath  C:\certs\aero-test.cer `
    -DriversPath C:\aero\drivers\win7 `
    -InstallWimIndices all
  ```

- Windows 10/11 host (or Windows VM) with:
  - DISM (`dism.exe`) available (built-in).
  - Optional: Windows ADK `oscdimg.exe` for ISO rebuild.
- Your Aero driver `.inf` directories for the correct architecture.
- Aero test root certificate file (DER or Base64 is fine).
  - CI output: `out/certs/aero-test.cer`

#### Copy ISO contents to a working directory

Mount the ISO and copy its contents:

```powershell
$IsoDrive = "E:"                       # mounted ISO
$WorkDir  = "C:\\win7-slipstream\\iso"
New-Item -ItemType Directory -Force $WorkDir | Out-Null
robocopy "$IsoDrive\\" "$WorkDir\\" /E
```

Note: Some ISO extraction/copy methods mark files as read-only. If you are doing the manual workflow below (not using `patch-win7-media.ps1`), ensure files are writable before patching:

```powershell
attrib -r "$WorkDir\\sources\\boot.wim"
attrib -r "$WorkDir\\sources\\install.wim"
attrib -r "$WorkDir\\boot\\BCD"
if (Test-Path "$WorkDir\\efi\\microsoft\\boot\\BCD") { attrib -r "$WorkDir\\efi\\microsoft\\boot\\BCD" }
```

#### Inject drivers into `sources/boot.wim`

`boot.wim` usually contains two indexes:

- Index 1: “Windows PE”
- Index 2: “Windows Setup”

List indexes:

```powershell
dism /Get-WimInfo /WimFile:"$WorkDir\\sources\\boot.wim"
```

Mount, add drivers, commit for each index:

```powershell
$Mount = "C:\\win7-slipstream\\mount"
New-Item -ItemType Directory -Force $Mount | Out-Null

$WinPeDrivers = "C:\\aero\\drivers\\winpe"   # directory containing .inf files

foreach ($Index in 1,2) {
  dism /Mount-Wim /WimFile:"$WorkDir\\sources\\boot.wim" /Index:$Index /MountDir:"$Mount"
  dism /Image:"$Mount" /Add-Driver /Driver:"$WinPeDrivers" /Recurse
  dism /Unmount-Wim /MountDir:"$Mount" /Commit
}
```

#### Inject drivers into `sources/install.wim`

List available editions (indexes):

```powershell
dism /Get-WimInfo /WimFile:"$WorkDir\\sources\\install.wim"
```

Inject drivers into the edition(s) you plan to install. If unsure, inject into all:

```powershell
$OsDrivers = "C:\\aero\\drivers\\system"

# Example: inject into index 1 only. Repeat for each desired index.
$Index = 1
dism /Mount-Wim /WimFile:"$WorkDir\\sources\\install.wim" /Index:$Index /MountDir:"$Mount"
dism /Image:"$Mount" /Add-Driver /Driver:"$OsDrivers" /Recurse
dism /Unmount-Wim /MountDir:"$Mount" /Commit
```

#### Install Aero test root cert into offline SOFTWARE hives (WinPE + installed OS)

Windows stores LocalMachine certificate stores under the SOFTWARE hive at:

- `…\\Microsoft\\SystemCertificates\\ROOT\\Certificates\\<thumbprint>`
- `…\\Microsoft\\SystemCertificates\\TrustedPublisher\\Certificates\\<thumbprint>`

The key name is the certificate **SHA-1 thumbprint of the certificate DER bytes** (uppercase hex, no spaces).

Note: the `Blob` value stored under `SystemCertificates` is written by CryptoAPI and is **not guaranteed to be raw DER**.
Recommended tooling (preferred over manual registry editing):

```powershell
cd tools\win-offline-cert-injector
cargo build --release --locked

$Cert = "C:\\aero\\certs\\aero-test.cer"

# After mounting a WIM index to $Mount:
.\target\release\win-offline-cert-injector.exe --windows-dir "$Mount" --store ROOT --store TrustedPublisher "$Cert"
```

#### Patch WinPE boot BCD on the ISO (BIOS + UEFI)

Enable test-signing (preferred):

```powershell
bcdedit /store "$WorkDir\\boot\\BCD" /set {default} testsigning on

if (Test-Path "$WorkDir\\efi\\microsoft\\boot\\BCD") {
  bcdedit /store "$WorkDir\\efi\\microsoft\\boot\\BCD" /set {default} testsigning on
}
```

Fallback (emulator-only): disable integrity checks:

```powershell
bcdedit /store "$WorkDir\\boot\\BCD" /set {default} nointegritychecks on
if (Test-Path "$WorkDir\\efi\\microsoft\\boot\\BCD") {
  bcdedit /store "$WorkDir\\efi\\microsoft\\boot\\BCD" /set {default} nointegritychecks on
}
```

If `{default}` does not exist (some OEM media), enumerate and set the correct “Windows Setup” loader entry:

```powershell
bcdedit /store "$WorkDir\\boot\\BCD" /enum all
```

#### Patch installed OS boot policy via `BCD-Template`

Mount an `install.wim` index and patch:

```powershell
$Index = 1
dism /Mount-Wim /WimFile:"$WorkDir\\sources\\install.wim" /Index:$Index /MountDir:"$Mount"

bcdedit /store "$Mount\\Windows\\System32\\config\\BCD-Template" /set {default} testsigning on

dism /Unmount-Wim /MountDir:"$Mount" /Commit
```

As with the ISO BCD, use `/enum all` to locate the correct loader identifier if needed.

#### Add `autounattend.xml` and setup scripts (optional but recommended)

See `tools/win7-slipstream/templates/README.md` for where these files must live on the ISO.

#### Rebuild the ISO (Windows: oscdimg)

If you have the Windows ADK installed (for `oscdimg.exe`):

```powershell
$OutIso = "C:\\win7-slipstream\\win7-aero.iso"

# Dual BIOS+UEFI boot if the source ISO supports it.
# Uses boot sectors from the extracted ISO tree (do not download these from elsewhere).
oscdimg -m -o -u2 -udfver102 `
  -bootdata:2#p0,e,b"$WorkDir\\boot\\etfsboot.com"#pEF,e,b"$WorkDir\\efi\\microsoft\\boot\\efisys.bin" `
  "$WorkDir" "$OutIso"
```

If your ISO is BIOS-only, you can omit the UEFI boot entry.

##### Aero BIOS boot note (El Torito + DL drive number)

Aero’s legacy BIOS boots Windows install media via an **El Torito no-emulation** boot entry (the `...,e,...` entries in `oscdimg -bootdata`, and `-no-emul-boot` in `xorriso`).
When booting from the first CD/ISO, the BIOS provides the boot drive number in `DL` as **`0xE0`**; when booting from the HDD instead, `DL` is **`0x80`**.
In Aero, this `DL` value is selected by the host via `BiosConfig::boot_drive` / `MachineConfig::boot_drive` (or `Machine::set_boot_drive(...)` + `reset()`).
For host convenience, firmware also supports an optional “CD-first when present” policy: keep the
configured `boot_drive` as the HDD (typically `0x80`) and set `boot_from_cd_if_present=true` +
`cd_boot_drive=0xE0` so BIOS attempts to boot from the ISO when attached, then falls back to HDD.
For the canonical optical attachment point (PIIX3 IDE **secondary master ATAPI**), see [`../areas/storage.md`](../areas/storage.md).
For INT 13h CD-extension expectations (including `AH=41h/42h/48h` for `DL=0xE0`), see
[`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md).

---

### Strategy B: Cross-platform driver injection (autounattend + xorriso)

This strategy avoids DISM for driver injection, but still requires you to address:

- Signature mode (WinPE BCD + installed OS boot policy).
- Certificate trust (WinPE + installed OS offline SOFTWARE hives).

On Linux/macOS you can generally do the file/WIM manipulations with open-source tools, and (if needed) run the BCD-editing parts inside a Windows VM.

Tip: You can keep Aero content on a separate “config media” ISO (drivers/certs/scripts + `autounattend.xml`) and attach it alongside the Windows ISO. Windows Setup scans attached media for `autounattend.xml`, and the repo’s templates use `%configsetroot%` for stable paths. This keeps the Windows ISO’s file tree cleaner, while still requiring offline patching of BCD/certs inside the Windows WIMs for boot-critical drivers.

If you only need an interactive “Load Driver” disk (no unattend/config scripts), CI can also optionally produce a small FAT32 driver disk (`*-fat.vhd`); see [`../areas/windows-drivers.md`](../areas/windows-drivers.md).

#### Prerequisites

- `xorriso` (ISO rebuild)
- `wimlib-imagex` (optional; for validation and offline hive edits)
- A tool to extract the ISO:
  - `bsdtar`, `7z`, or `xorriso -osirrox on -indev … -extract / …`

#### Extract ISO

Example using `7z`:

```sh
mkdir -p iso-root
7z x -oiso-root Win7SP1.iso
```

#### Stage Aero drivers + certs on the ISO tree

```sh
mkdir -p iso-root/aero/{certs,drivers/winpe,drivers/system,scripts}
cp /path/to/aero-test.cer iso-root/aero/certs/
cp -R /path/to/winpe-driver-inf-dirs/* iso-root/aero/drivers/winpe/
cp -R /path/to/system-driver-inf-dirs/* iso-root/aero/drivers/system/
```

#### Add an `autounattend.xml`

Copy an `autounattend.xml` template to the ISO root and edit as needed. Options in this repo:

- Ready-to-edit, architecture-specific templates:
  - `guest-tools/unattend/autounattend_amd64.xml`
  - `guest-tools/unattend/autounattend_x86.xml`
- Golden-reference templates (placeholders intended for future tooling):
  - `tools/win7-slipstream/templates/autounattend.drivers-only.xml`
  - `tools/win7-slipstream/templates/autounattend.full.xml`

Windows Setup scans the root of removable media / install media for `autounattend.xml`.

If your unattend references `%configsetroot%` (recommended for stable paths), ensure the `Microsoft-Windows-Setup` component sets:

```xml
<UseConfigurationSet>true</UseConfigurationSet>
```

#### Add setup scripts (optional but recommended)

For post-install automation (install certs, enable test mode, install drivers), the repo includes Win7-compatible scripts at:

- `guest-tools/unattend/scripts/`

You can also start from the simpler golden-reference templates in `tools/win7-slipstream/templates/` (see its README for `$OEM$` placement).

#### Handle signature mode + certificate trust

Even with unattend-based driver paths, you must still ensure WinPE and the installed OS can load Aero drivers:

- Patch ISO BCD (`boot/BCD` and `efi/microsoft/boot/BCD`) to enable `testsigning` (preferred) or `nointegritychecks` (fallback).
- Patch `BCD-Template` inside the target install.wim index so the installed OS boots with the same signing policy.
- Install Aero’s test root cert into offline SOFTWARE hives for boot.wim + install.wim.

If you don’t have Windows tooling available, these steps can still be done cross-platform by editing the offline registry hives directly (see `tools/win7-slipstream/patches/README.md` and the example below).

##### Example: offline certificate + BCD patching on Linux/macOS (wimlib + hivexregedit)

If you prefer not to use Windows tooling for certificate injection, you can edit the offline SOFTWARE hive directly.

This example mounts `boot.wim` index 2 read-write, generates a `.reg` certificate patch, merges it into the hive, then commits.

Prerequisites:

- `wimlib-imagex` (FUSE-based mounting; on macOS you may need macFUSE)
- `hivexregedit`

Note: `cert-to-reg.py` writes the certificate’s raw DER bytes into the `Blob` registry value. This often works, but the CryptoAPI registry-backed cert store format is **not guaranteed** to be raw DER across all environments. For the most portable patch, generate the `.reg` on Windows using `tools/win-certstore-regblob-export` (or inject directly using `tools/win-offline-cert-injector`), then apply it cross-platform with `hivexregedit`. See `tools/win7-slipstream/patches/README.md`.

```sh
CERT_PATH="iso-root/aero/certs/aero-test.cer"

# Generate `aero-cert.reg` on a Windows machine (once) using CryptoAPI, then copy it here:
#   win-certstore-regblob-export --store ROOT --store TrustedPublisher --format reg --reg-hklm-subkey SOFTWARE "$CERT_PATH" > aero-cert.reg
#
# (Fallback: tools/win7-slipstream/scripts/cert-to-reg.py can generate a best-effort .reg from raw DER,
# but it may not match CryptoAPI's exact registry-backed `Blob` representation.)

# Mount boot.wim index 2 (Windows Setup) read-write.
mkdir -p mnt
wimlib-imagex mount iso-root/sources/boot.wim 2 mnt --read-write

hivexregedit --merge --prefix 'HKEY_LOCAL_MACHINE\SOFTWARE' mnt/Windows/System32/config/SOFTWARE aero-cert.reg
wimlib-imagex unmount mnt --commit
```

Repeat this for:

- `boot.wim` index 1 and 2
- each `install.wim` index you will install (or all indexes if unsure)

You can also patch BCD stores cross-platform.

Preferred (more robust): use the repo’s cross-platform BCD patcher (`tools/bcd_patch/`), which patches multiple objects (global settings, loader settings, OS loader entries) rather than relying on inheritance quirks:

```sh
# Patch extracted ISO BCD stores (boot/BCD + efi/microsoft/boot/BCD if present).
cargo run --locked -p bcd-patch -- win7-tree --root iso-root --nointegritychecks off

# Patch BCD-Template inside a mounted install.wim index (run once per index you care about).
# Example mount root: mnt-install
cargo run --locked -p bcd-patch -- win7-tree --root mnt-install --nointegritychecks off
```

Alternative: patch via auditable `.reg` files + `hivexregedit` (see `tools/win7-slipstream/patches/README.md` for details):

```sh
hivexregedit --merge --prefix 'HKEY_LOCAL_MACHINE\BCD' iso-root/boot/BCD tools/win7-slipstream/patches/bcd-testsigning.reg

# If your ISO has a UEFI BCD store too:
hivexregedit --merge --prefix 'HKEY_LOCAL_MACHINE\BCD' iso-root/efi/microsoft/boot/BCD tools/win7-slipstream/patches/bcd-testsigning.reg
```

On Linux/macOS, ensure the `iso-root/...` paths match the actual case of the extracted files (some ISOs use `bcd` instead of `BCD`).

#### Rebuild the ISO (Linux/macOS: xorriso)

Example command (dual BIOS+UEFI boot):

```sh
xorriso -as mkisofs \
  -iso-level 3 -udf -J -joliet-long -D -N \
  -volid "WIN7_AERO" \
  -b boot/etfsboot.com \
  -no-emul-boot -boot-load-size 8 -boot-info-table \
  -eltorito-alt-boot \
  -e efi/microsoft/boot/efisys.bin \
  -no-emul-boot \
  -o win7-aero.iso \
  iso-root
```

Note: Aero boots install media via **El Torito no-emulation** entries; the boot drive number passed
in `DL` is **`0xE0`** for the first CD-ROM (HDD0 is **`0x80`**). See
[`../areas/storage.md`](../areas/storage.md) and
[`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md).

Important:

- Use UDF-capable output (`-udf`, `-iso-level 3`) because `install.wim` can exceed 4GB.
- Boot images (`boot/etfsboot.com`, `efi/…/efisys.bin`) must come from the user’s original ISO tree.
- `-boot-info-table` is an optional bootloader-side patch used by some toolchains; BIOS does not
  consume it (see [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md)).

---

### Reproducibility & auditing recommendations

For a reproducible and reviewable slipstream process:

- Record input ISO hash (e.g., SHA256) and the output ISO hash.
- Record:
  - WIM indexes modified (boot.wim indexes, install.wim index list).
  - Driver directories injected / staged (and their hashes).
  - Certificate thumbprint installed.
  - BCD flags set (`testsigning`, `nointegritychecks`) for both WinPE and BCD-Template.
- Keep all Aero-specific additions under `aero/` on the ISO for easy review.
- If you care about byte-for-byte ISO reproducibility, normalize timestamps in the staging tree before ISO creation and keep a stable file ordering.

---

### Validation checklist (no Windows redistribution required)

#### Structure checks

- [ ] ISO tree contains `sources/boot.wim` and `sources/install.wim`
- [ ] ISO tree contains `boot/BCD`
- [ ] If UEFI boot expected: `efi/microsoft/boot/BCD` exists

#### WIM checks (with wimlib or DISM)

- [ ] `boot.wim` has expected indexes (usually 1 and 2)
- [ ] `install.wim` has expected edition indexes
- [ ] Drivers present:
  - Windows: `dism /Image:<mount> /Get-Drivers`
  - Cross-platform: mount with wimlib and verify files under `Windows\\System32\\DriverStore`

#### BCD checks

- [ ] `boot/BCD` WinPE loader has `testsigning` enabled (preferred) or `nointegritychecks` enabled (fallback)
- [ ] If present: `efi/microsoft/boot/BCD` matches
- [ ] `install.wim:<index>\\Windows\\System32\\config\\BCD-Template` has the same signing policy

#### Certificate trust checks (offline)

For each patched image (boot.wim indexes + install.wim index):

- [ ] `Windows\\System32\\config\\SOFTWARE` contains:
  - `Microsoft\\SystemCertificates\\ROOT\\Certificates\\<thumbprint>`
  - `Microsoft\\SystemCertificates\\TrustedPublisher\\Certificates\\<thumbprint>`

#### Optional smoke test (user-run)

Users can locally test their prepared ISO without sharing any Windows binaries:

- Boot the ISO in QEMU/VirtualBox.
- Attach a virtual disk that requires the Aero storage driver (for example, a virtio disk).
- Confirm Windows Setup can see the disk and proceed to partition/format.

Example (QEMU, x64):

```sh
qemu-system-x86_64 \
  -m 4096 \
  -cdrom win7-aero.iso \
  -drive file=win7.qcow2,if=none,id=drive0,format=qcow2 \
  -device virtio-blk-pci,drive=drive0,disable-legacy=on,x-pci-revision=0x01 \
  -boot d
```

## Windows 7 Unattended Install Validation & Troubleshooting

This document is a practical validation + debugging playbook for doing a **real Windows 7 SP1 installation** in a VM using:

* **CD0**: the official Windows 7 SP1 installer ISO
* **CD1**: an Aero “config ISO” containing `autounattend.xml` plus any drivers/scripts/payload

It is written to answer (by *verification*, not assumptions) the open questions around:

* whether `%configsetroot%` is available when booting with a separate config ISO,
* how Windows Setup treats a secondary CD/DVD (“config ISO”) during unattended install, and
* whether `$OEM$` is processed/copied when `$OEM$` is on that separate media.

Where behavior varies by hypervisor or media type, this guide uses **“expected”** language and provides concrete steps to prove what happened from logs and on-screen state.

See also (related docs in this repo):

* [`../areas/windows-drivers.md`](../areas/windows-drivers.md) (how to structure `autounattend.xml`, driver injection passes, and setup scripting hooks)
* [`../areas/windows-drivers.md`](../areas/windows-drivers.md) (the broader “what to patch” install-media preparation workflow: WIM/BCD/certs + ISO rebuild + validation)
* [`../areas/windows-drivers.md`](../areas/windows-drivers.md) (post-install driver signing/trust failures, Device Manager codes, `setupapi.dev.log` triage)

---

### Prerequisites

* A Windows 7 SP1 ISO (x86 or x64).
* An Aero config ISO (CD1) that contains at minimum:
  * `\autounattend.xml` (root of the ISO)
  * A unique marker file at the root, e.g. `\AERO_CONFIG.MEDIA` (recommended for drive discovery)
  * Any payload you expect to be copied/used (drivers, scripts, certificates, etc)
* A VM with:
  * 2+ GB RAM (4 GB preferred for x64)
  * 2 vCPUs
  * 25+ GB virtual disk
  * BIOS/Legacy boot (Win7 supports UEFI in some configs, but keep this simple)

---

### End-to-end validation checklist (Win7 SP1 in a VM)

#### A. VM + media setup (pre-boot)

1. Create a new VM and attach storage:
   - **Hard disk**: blank, 25+ GB
   - **CD0**: Windows 7 SP1 ISO
   - **CD1**: Aero config ISO (contains `autounattend.xml`)

2. Ensure the VM is configured to boot from **CD/DVD** (so Windows Setup boots).
   - Aero note: BIOS boot selection is driven by the BIOS boot drive number (`DL`) consumed during
     POST/boot.
     - Simple (direct CD boot): set the boot drive to the first CD-ROM (**`DL=0xE0`**) and reset.
       - Rust: `machine.set_boot_drive(0xE0); machine.reset();`
       - JS/wasm: `machine.set_boot_drive(0xE0); machine.reset();`
     - Recommended (CD-first when present): keep the configured HDD boot drive as the fallback
       (**`DL=0x80`**) and enable the firmware “CD-first when present” policy so BIOS boots the ISO
       when it is attached, but still boots from HDD after ejecting it.
       - Rust:
         `machine.set_boot_drive(0x80); machine.set_cd_boot_drive(0xE0); machine.set_boot_from_cd_if_present(true); machine.reset();`
       - JS/wasm:
         `machine.set_boot_drive(0x80); machine.set_cd_boot_drive(0xE0); machine.set_boot_from_cd_if_present(true); machine.reset();`

3. (Recommended) Make CD1 easy to identify:
   - ISO volume label: `AERO-CONFIG` (or similar)
   - Root marker file: `AERO_CONFIG.MEDIA`

Why: Windows Setup/WinPE drive letters are not stable across hypervisors, and CD0/CD1 can swap.

---

#### B. Boot + prove Setup is fully unattended

Boot the VM. A fully unattended flow should show **no prompts** for the items below.

##### Unattended proof points

Check off each item as it happens:

- [ ] **No “Press any key to boot from CD/DVD…” stalls** (if your hypervisor requires a keypress, this is fine; it’s outside Windows Setup).
- [ ] **No language/keyboard prompt** (or it is auto-accepted without interaction).
- [ ] **No EULA prompt** (EULA is accepted automatically).
- [ ] **No “Which version of Windows do you want to install?” prompt** (image selection is automatic).
- [ ] **No disk selection/partitioning UI** (disk partitioning is automatic).
- [ ] Setup proceeds directly through “Expanding Windows files”, “Installing features”, “Completing installation”, then reboots.
- [ ] **No OOBE user creation prompts** (username/computer name/timezone/etc are auto-configured).

If any prompt appears, skip to [Failure modes + fixes](#4-failure-modes--fixes) and to [Where to look for logs](#2-where-to-look-for-logs).

---

##### Optional: post-install sanity checks (edition + partitioning)

These checks help prove that unattended *selection* worked (correct edition/arch) and that unattended *disk config* did what you intended.

**Confirm edition + architecture**

```bat
wmic os get caption,osarchitecture,version
```

You should see the expected edition (for example “Windows 7 Professional”) and architecture (“32-bit” / “64-bit”).

**Confirm partition layout**

Interactive:

```bat
diskpart
list disk
list vol
exit
```

Non-interactive one-liner (convenient for copy/paste):

```bat
(echo list disk & echo list vol & echo exit) | diskpart
```

#### C. Prove scripts ran (SetupComplete, testsigning, scheduled task lifecycle, drivers)

After the first boot to the desktop (or to the login screen, depending on your unattend), validate in this order.

##### SetupComplete executed

`SetupComplete.cmd` is executed by Windows Setup at the end of installation, from:

* `%WINDIR%\Setup\Scripts\SetupComplete.cmd`

Validation steps:

1. Open an elevated command prompt (or run as Administrator).
2. Confirm the file exists:

```bat
dir "%WINDIR%\Setup\Scripts\SetupComplete.cmd"
```

3. Confirm the Aero script logs/markers exist (expected, if your scripts create them):

```bat
dir C:\Windows\Temp\aero-setup.log
dir C:\Windows\Temp\aero-driver-install.log
```

4. (Optional) Confirm Setup attempted to execute `SetupComplete.cmd` by searching Setup logs:

```bat
findstr /i /c:"setupcomplete" C:\Windows\Panther\setupact.log
```

What “good” looks like:

* `C:\Windows\Temp\aero-setup.log` exists and includes a timestamp and a line indicating it ran under `SYSTEM`.
* If driver install is separated into a second phase, `C:\Windows\Temp\aero-driver-install.log` exists as well.

If `SetupComplete.cmd` is missing or logs are missing, see [SetupComplete not running](#setupcomplete-not-running).

---

##### testsigning enabled

If your workflow relies on test-signed drivers, the system should have testsigning enabled.

Run:

```bat
bcdedit /enum {current}
```

Expected snippet:

```text
Windows Boot Loader
-------------------
identifier              {current}
...
testsigning             Yes
```

Notes:

* If `testsigning` is missing or `No`, either testsigning wasn’t set, it was set on the wrong BCD entry, or a reboot is still required.
* In Win7, enabling testsigning typically requires a reboot before unsigned/test-signed drivers will load reliably.

---

##### Scheduled task created and then removed

Some unattended flows create a startup scheduled task to finish post-install steps (e.g. driver install after the first boot) and remove it once complete.

Validation steps:

1. Immediately after reaching the desktop the first time, list tasks and look for “Aero”:

```bat
schtasks /query /fo LIST /v | findstr /i aero
```

2. Reboot once and run the same command again.

Expected behavior (example):

* First boot: an Aero-related task exists (name varies by implementation).
* After the post-install step completes: the task is deleted and no longer appears in `schtasks /query`.

If you don’t know the exact task name, use this as a broad check and rely on your custom logs (e.g. `aero-setup.log`) to print the created task name explicitly.

If tasks persist forever or you get reboot loops, see [Reboot loops / task never deleted](#reboot-loops--task-never-deleted).

---

##### Drivers installed

How you validate driver installation depends on whether the VM hardware matches your drivers (virtio devices in QEMU, synthetic devices in Hyper-V, etc). The goal here is to prove *the installation mechanism worked* and to capture why it didn’t when it fails.

**A) Validate via your own logs (recommended)**

Check for logged `pnputil` output in:

* `C:\Windows\Temp\aero-driver-install.log`

You want to see successful add/install messages, e.g.:

```text
Processing inf :  netkvm.inf
Driver package added successfully.
Driver package installed successfully.
```

**B) Validate via `pnputil` driver store enumeration**

Run:

```bat
pnputil -e
```

Look for your driver provider/name in the output (for custom Aero drivers, consider using a unique provider string to make this grep-able).

**C) Validate in Device Manager**

* Run `devmgmt.msc`
* Confirm:
  * No “Unknown device” entries remain for the devices your drivers target.
  * The expected driver is bound to the expected device (Properties → Driver tab).

If installation fails, jump to [Unsigned/test-signed driver install blocked](#unsignedtest-signed-driver-install-blocked) and consult `setupapi.dev.log` in [Log locations](#2-where-to-look-for-logs).

---

### Where to look for logs

These are the first places to look when unattended install does not behave as expected.

#### Windows Setup logs (unattend parsing, config-set detection, pass execution)

During setup (WinPE), logs are written to the WinPE RAM disk:

* `X:\Windows\Panther\setupact.log`
* `X:\Windows\Panther\setuperr.log`
* `X:\Windows\Panther\UnattendGC\setupact.log`
* `X:\Windows\Panther\UnattendGC\setuperr.log`
* `X:\Windows\Panther\UnattendGC\diagerr.xml` (unattend parse/validation errors, when present)
* `X:\Windows\Panther\UnattendGC\diagwrn.xml` (unattend parse/validation warnings, when present)

After Windows is installed, logs are copied to:

* `C:\Windows\Panther\setupact.log`
* `C:\Windows\Panther\setuperr.log`
* `C:\Windows\Panther\UnattendGC\setupact.log`
* `C:\Windows\Panther\UnattendGC\setuperr.log`
* `C:\Windows\Panther\UnattendGC\diagerr.xml` (unattend parse/validation errors, when present)
* `C:\Windows\Panther\UnattendGC\diagwrn.xml` (unattend parse/validation warnings, when present)
* (If setup failed/rolled back) `C:\Windows\Panther\Rollback\setupact.log`
* (If setup failed/rolled back) `C:\Windows\Panther\Rollback\setuperr.log`

What each proves:

* `setupact.log`: the canonical timeline of Windows Setup actions (disk config, image apply, reboots, pass transitions).
* `setuperr.log`: errors only (start here when something failed).
* `UnattendGC\setupact.log`: unattend processing and “generated catalog” / “unattend gather” behavior. This is often where you’ll see:
  * which unattend file was selected,
  * which passes were applied,
  * and whether a configuration set was detected.

Useful searches:

```bat
findstr /i /c:"unattend" C:\Windows\Panther\setupact.log
findstr /i /c:"autounattend" C:\Windows\Panther\setupact.log
findstr /i /c:"configset" C:\Windows\Panther\setupact.log
findstr /i /c:"configsetroot" C:\Windows\Panther\setupact.log
findstr /i /c:"unattend" C:\Windows\Panther\UnattendGC\setupact.log
findstr /i /c:"setupcomplete" C:\Windows\Panther\setupact.log
```

---

#### Driver install logs (PnP binding, signature enforcement, INF parsing)

* `C:\Windows\inf\setupapi.dev.log`

What it proves:

* Every device driver bind attempt, including:
  * which INF matched,
  * why a match was rejected (signature, rank, missing files),
  * whether the driver installed successfully.

Useful searches:

```bat
findstr /i /c:"!!!" C:\Windows\inf\setupapi.dev.log
findstr /i /c:"failed" C:\Windows\inf\setupapi.dev.log
findstr /i /c:".inf" C:\Windows\inf\setupapi.dev.log
```

`!!!` lines are the high-signal failure markers in `setupapi.dev.log`.

---

#### Custom Aero logs (scripts and post-install automation)

These paths are recommended because they are writable during setup and easy to collect:

* `C:\Windows\Temp\aero-setup.log` (SetupComplete + general automation)
* `C:\Windows\Temp\aero-driver-install.log` (driver-specific install output)

What they should prove (minimum):

* Which phase ran (SetupComplete vs first-logon vs scheduled task).
* Which media path was used (e.g. `%configsetroot%` value, or discovered drive letter).
* Output of key commands (`bcdedit`, `pnputil`, `schtasks`), including exit codes.

---

### Debug actions during setup (Shift+F10)

At almost any Windows Setup screen, press:

* `Shift+F10` → opens `cmd.exe` (WinPE command prompt)

This is the fastest way to prove whether CD1 is visible and whether `%configsetroot%` exists.

#### A. Enumerate disks/volumes and identify CD0 vs CD1

List volumes:

```bat
wmic logicaldisk get name,description,filesystem
```

Optional (include volume label for easier CD0/CD1 identification):

```bat
wmic logicaldisk get name,description,filesystem,volumename
```

Then inspect likely drive letters:

```bat
dir C:\
dir D:\
dir E:\
dir F:\
```

If you want to brute-force scan drive letters (interactive `cmd.exe`):

```bat
for %D in (C D E F G H I J K L M N O P Q R S T U V W X Y Z) do @echo ==== %D ==== & @dir %D:\ 2>nul
```

Notes:

* In WinPE, the Windows installer environment is usually `X:\`.
* The target OS partition may not be `C:` yet, depending on when you check.
* CD drives are commonly `D:`/`E:` but can vary.

#### B. Confirm the Aero config ISO is present

Once you believe you found CD1, check for your marker file and `autounattend.xml`:

```bat
dir <CD1>:\AERO_CONFIG.MEDIA
dir <CD1>:\autounattend.xml
```

If you did not include a marker file, add one; it makes every debugging step easier.

#### C. Print and interpret environment variables (especially `%configsetroot%`)

Dump environment:

```bat
set
```

Then specifically:

```bat
echo configsetroot=[%configsetroot%]
set configset
```

Interpretation:

* **Expected (if Windows Setup treats CD1 as a “configuration set” root):**
  * `%configsetroot%` is a non-empty path to the *configuration set root*.
  * Depending on how Setup handled the media, it may point to:
    * the original removable/optical media (for example `D:\` / `E:\`), **or**
    * a copied/staged location on a local disk.
  * Prove what it points to by running:

```bat
echo configsetroot=[%configsetroot%]
dir "%configsetroot%"
```

  * If you use a marker file (recommended), also test:

```bat
dir "%configsetroot%\AERO_CONFIG.MEDIA"
```
* **If empty:**
  * Windows Setup may still be using `autounattend.xml`, but it may **not** consider the media a configuration set.
  * In that case, `$OEM$` copy behavior may differ (see [Open questions](#open-questions)).

#### D. Check live setup logs before reboot

Open logs in Notepad (works in WinPE):

```bat
notepad X:\Windows\Panther\setupact.log
notepad X:\Windows\Panther\setuperr.log
```

Search within the log for:

* `unattend`
* `autounattend`
* `ConfigSet`
* `ConfigSetRoot`

This is the fastest way to prove which unattend file was selected and whether a config set was detected.

---

### Failure modes + fixes

#### `autounattend.xml` not picked up

Symptoms:

* Setup shows prompts (EULA, edition selection, disk selection) that should be automated.

What to check:

1. Confirm the file is exactly at the root of the intended media:
   * `\autounattend.xml` (root)
2. Confirm Windows Setup can read the media (WinPE `dir` works).
3. Confirm Setup logs mention the file:
   * Search `setupact.log`/`UnattendGC\setupact.log` for `autounattend`.

Common causes / fixes:

* Wrong filename (`unattend.xml` instead of `autounattend.xml` for removable-media discovery).
* Wrong placement (nested folder instead of root).
* Wrong media type / discovery behavior:
  * **Expected (but not guaranteed):** Windows Setup scans *some* removable/optical media for `autounattend.xml`, but the exact search order varies by Windows version and environment.
  * If you place `autounattend.xml` on **CD1** and Setup behaves as if it never saw it, verify in `X:\Windows\Panther\setupact.log` / `UnattendGC\setupact.log` whether Setup enumerated that drive for answer files.
  * If CD1 is not scanned in your environment, the practical fixes are:
    - Put `autounattend.xml` on a **USB** device image instead (often treated as removable media), or
      - Rebuild the Windows install ISO so `autounattend.xml` is on **CD0** (customized install media), or
      - Ensure your workflow does not depend on “CD1 answer file discovery” and instead uses a slipstream/patcher approach:
      - [`../areas/windows-drivers.md`](../areas/windows-drivers.md) (`tools/windows/patch-win7-media.ps1`, Windows-first)
      - [`../areas/windows-drivers.md`](../areas/windows-drivers.md) (manual, auditable slipstreaming)
      - [`tools/win7-slipstream/README.md`](../decisions/README.md) (automated slipstreaming tool)
* The hypervisor attaches CD1 too late (attach before boot).
* Multiple unattend files present on multiple devices; Setup may pick an unexpected one.

---

#### Driver paths not found / quoting issues

Symptoms:

* Your driver-install script logs “file not found” or `pnputil` fails to open the INF.
* `setupapi.dev.log` shows missing source files.

What to check:

* In WinPE and in the installed OS, confirm the actual drive letter and paths.
* Avoid spaces and quoting ambiguity in paths on the ISO.

Fix patterns:

* Prefer a layout like `\Drivers\<vendor>\<arch>\*.inf` with no spaces.
* Log the resolved path before invoking `pnputil`.

---

#### Unsigned/test-signed driver install blocked

Symptoms:

* `pnputil` reports failure.
* Device Manager shows the device but refuses the driver.
* `setupapi.dev.log` contains signature enforcement failures.

What to check:

1. Confirm testsigning state:

```bat
bcdedit /enum {current}
```

2. Check `C:\Windows\inf\setupapi.dev.log` for `!!!` lines around your INF name.

Typical fixes:

* Ensure testsigning is enabled **and** the machine rebooted after setting it.
* If using a test certificate, import it into the correct stores (TrustedPublisher and/or Root) before installing drivers:

```bat
certutil -addstore -f TrustedPublisher C:\Aero\certs\aero-test.cer
certutil -addstore -f Root C:\Aero\certs\aero-test.cer
```

Notes:

* Exact certificate requirements depend on how the driver was signed.
* Don’t guess: the error reason will be in `setupapi.dev.log`.

---

#### SetupComplete not running

Symptoms:

* No `C:\Windows\Temp\aero-setup.log`
* No scheduled task created
* No post-install behavior happened (testsigning unchanged, no drivers installed)

What to check:

1. Confirm the file exists where Windows expects it:

```bat
dir "%WINDIR%\Setup\Scripts\SetupComplete.cmd"
```

2. Confirm `$OEM$` content was copied (if you rely on `$OEM$`):
   * `$OEM$\$$\Setup\Scripts\SetupComplete.cmd` should become:
     * `C:\Windows\Setup\Scripts\SetupComplete.cmd`

3. Check Setup logs:
   * `C:\Windows\Panther\setupact.log`
   * `C:\Windows\Panther\setuperr.log`

Common root causes / fixes:

* `$OEM$` wasn’t processed from CD1 (see [open questions](#open-questions)).
* The file path inside `$OEM$` is wrong (must be exactly `$$\Setup\Scripts\SetupComplete.cmd`).
* Script ran but failed immediately; make the script write a first-line log entry before doing anything else.
* If `$OEM$` processing is unreliable in your environment, don’t depend on it for `SetupComplete.cmd` delivery:
  * Put `SetupComplete.cmd` on your config media and copy it into `%WINDIR%\Setup\Scripts\` via a `specialize` `RunSynchronous` command in `autounattend.xml`.
  * Reference implementations exist in the repo’s templates; see:
    * [`../areas/windows-drivers.md`](../areas/windows-drivers.md)
    * [`guest-tools/unattend/`](../../guest-tools/unattend/)

---

#### Reboot loops / task never deleted

Symptoms:

* VM keeps rebooting or repeatedly runs the same post-install step.
* Scheduled task persists across reboots.

What to check:

* Your script must create a durable “done” marker (file or registry) and check it before doing work.
* Ensure the scheduled task deletes itself after success:

```bat
schtasks /delete /tn "<TaskName>" /f
```

Fix patterns:

* Use a marker file such as `C:\Aero\postinstall.done` or `C:\Windows\Temp\aero-postinstall.done`.
* Log every branch decision (“marker present: skipping”, “marker missing: running”).
* Log `schtasks` output and `%ERRORLEVEL%`.

---

### Explicit notes on the open questions (config-set, config ISO, $OEM$)

#### Open questions

This section is intentionally a **test plan**: it tells you exactly how to determine what Windows Setup actually did in your environment.

##### Question A: Is `%configsetroot%` available when using a separate config ISO (CD1)?

**Expected (but not guaranteed):** if Windows Setup recognizes CD1 as a *configuration set*, it sets `%configsetroot%` during setup.

How to verify:

1. During setup, press `Shift+F10`.
2. Run:

```bat
echo configsetroot=[%configsetroot%]
```

3. Record the result and correlate with logs:

* `X:\Windows\Panther\setupact.log`
* `X:\Windows\Panther\setuperr.log`

Search for:

* `ConfigSet`
* `ConfigSetRoot`

Interpretation:

* If `%configsetroot%` is set, **confirm it actually contains your config payload**:

```bat
dir "%configsetroot%"
dir "%configsetroot%\AERO_CONFIG.MEDIA"
```

* If the marker exists there (or you can see your expected folders), you can use `%configsetroot%` as your primary reference to find drivers/payload during setup.
* If `%configsetroot%` is set but the marker is missing, it may be pointing at a copied/staged config-set location or to a different device than you expect. Don’t guess:
  * Use the drive-letter scan in [Debug actions](#3-debug-actions-during-setup-shiftf10) to locate the real CD1.
  * Optionally, after install (once you know which drive is the OS partition), search the system drive for your marker to discover where Setup copied the config set:

```bat
where /r C:\ AERO_CONFIG.MEDIA
rem If `where` is not available (some WinPE environments), use:
dir /s /b C:\AERO_CONFIG.MEDIA 2>nul
```

* If `%configsetroot%` is empty, do **not** rely on it; treat CD1 as “just another CD drive” and use drive-letter discovery (see [Fallback](#fallback-approach-if-configsetroot--oem-are-unreliable)).

---

##### Question B: Does Windows Setup process/copy `$OEM$` when `$OEM$` lives on CD1?

**Expected (but not guaranteed):** if CD1 is a recognized configuration set, `$OEM$` should be processed similarly to `$OEM$` on the main install media.

How to verify:

1. Put an unmistakable file under `$OEM$` in your config ISO, for example:
   * `$OEM$\$1\Aero\oem-proof.txt` (should copy to `C:\Aero\oem-proof.txt`)
   * `$OEM$\$$\Setup\Scripts\SetupComplete.cmd` (should copy to `%WINDIR%\Setup\Scripts\SetupComplete.cmd`)
2. After install completes, verify:

```bat
dir C:\Aero\oem-proof.txt
dir "%WINDIR%\Setup\Scripts\SetupComplete.cmd"
```

Interpretation:

* If these files exist, `$OEM$` copy happened.
* If not, `$OEM$` was not processed from CD1 in your environment. In that case, your `SetupComplete.cmd` will not exist unless you deliver it some other way.

---

##### Question C: Does Windows Setup use `autounattend.xml` from CD1 while *not* treating it as a config set?

This is a common “partial success” scenario: Setup finds `autounattend.xml` and applies many settings, but does not set `%configsetroot%` and does not copy `$OEM$`.

How to verify:

* Prompts are automated (so unattend parsing worked), but:
  * `%configsetroot%` is empty during setup, and/or
  * `$OEM$` outputs are missing after install.

Corroborate in logs:

* `C:\Windows\Panther\UnattendGC\setupact.log` should still show which unattend file was used.
* You may see evidence of unattend parsing without config set detection.

---

##### Question D: Does Windows Setup scan a secondary CD/DVD (CD1) for `autounattend.xml`?

**Expected (but not guaranteed):** Windows Setup will pick up `\autounattend.xml` from some removable/optical media, but the exact search order (and whether it scans a “second CD”) can vary by environment.

How to verify (minimal experiment):

1. Ensure **CD0** is the stock Windows 7 ISO (no answer file).
2. Put `\autounattend.xml` only on **CD1** and attach it before boot.
3. Boot and observe:
   * If Setup is fully unattended, CD1 answer file discovery worked in that environment.
   * If Setup shows prompts (edition selection, disk selection, EULA), it likely did not scan CD1 for the answer file.
4. Corroborate in logs:
   * In WinPE (Shift+F10): `notepad X:\Windows\Panther\setupact.log`
   * Search for `autounattend.xml` and/or the device path it was loaded from.

Practical fixes if CD1 is not scanned:

* Put `autounattend.xml` on a **USB** device image (often treated as removable media and more consistently scanned), or
* Patch/rebuild the install ISO so `autounattend.xml` is on **CD0**, or
* Use a slipstream/media patcher flow so Setup does not depend on “secondary CD answer file discovery”:
  * [`../areas/windows-drivers.md`](../areas/windows-drivers.md)
  * [`../areas/windows-drivers.md`](../areas/windows-drivers.md)
  * [`tools/win7-slipstream/README.md`](../decisions/README.md)

##### Fallback approach (if `%configsetroot%` / `$OEM$` are unreliable)

If you find that CD1 is not treated as a configuration set on your hypervisor/media, the robust approach is:

1. **Make CD1 self-identifying** via a marker file (root): `AERO_CONFIG.MEDIA`.
2. **Scan drive letters at runtime** to locate CD1.
3. **Copy payload to a stable local path** early, e.g. `C:\Aero\` (so later phases don’t depend on CD drive letters).

Example drive-discovery snippet (batch, suitable for `SetupComplete.cmd`):

```bat
setlocal enabledelayedexpansion

set "AERO_MEDIA="
for %%D in (C D E F G H I J K L M N O P Q R S T U V W X Y Z) do (
  if exist "%%D:\AERO_CONFIG.MEDIA" set "AERO_MEDIA=%%D:"
)

if not defined AERO_MEDIA (
  echo ERROR: Aero config media not found. >> C:\Windows\Temp\aero-setup.log
  exit /b 1
)

echo Found Aero media at %AERO_MEDIA% >> C:\Windows\Temp\aero-setup.log

rem Example: copy payload to C:\Aero
md C:\Aero 2>nul
xcopy "%AERO_MEDIA%\Payload" C:\Aero\ /E /I /H /Y >> C:\Windows\Temp\aero-setup.log 2>&1
```

Key idea:

* **Do not** build a flow that only works if `%configsetroot%` is set or only works if `$OEM$` is copied from CD1.
* Instead, design the automation to succeed with either:
  * config-set semantics (when available), or
  * simple “find the CD by marker and copy from it” semantics (always available when the CD is attached).

## Windows 7 BCD offline patching (testsigning / nointegritychecks)

This project builds and boots Windows 7 images in automated tests. For those images to work with
our performance-critical custom drivers (virtio storage/network/etc.), we patch **Boot
Configuration Data (BCD)** stores **offline** (i.e. by editing the BCD files directly, not by
running `bcdedit` inside a running Windows install).

The goal of the patch is to make Windows boot with:

- **`testsigning` enabled** (allow loading *test-signed* kernel-mode code), and/or
- **`nointegritychecks` enabled** (disable kernel-mode integrity checks / signature enforcement).

This document exists so we do **not** rely on “heuristics + confirm later” when patching a
boot-critical registry hive.

> Safety: Always take a backup copy of the BCD store before patching. A corrupted BCD can prevent
> a system or image from booting.

---

### Motivation

Windows 7 **x64** enforces kernel-mode code signing. In practice:

- Our custom drivers (virtio-blk/virtio-net/virtio-gpu, etc.) will often be **unsigned** during
  development, or only **test-signed**.
- Without `testsigning` and/or `nointegritychecks`, Windows 7 x64 will refuse to load those
  drivers, which can prevent the system from booting (storage driver), networking from working,
  or graphics acceleration from loading.

Patching the BCD stores **before boot** makes test images deterministic and avoids having to
manually toggle boot options inside a guest OS.

---

### Which files to patch (Win7)

We patch **all BCD stores that can participate in the boot flow** for our test images. In
practice that means:

#### Extracted ISO (installation media / WinPE)

Patch both of these (they are used for different firmware boot paths):

- `boot/BCD` (BIOS/CSM boot path)
- `efi/microsoft/boot/BCD` (UEFI boot path)

Note: ISO extractors and non-Windows filesystems may change the case of these
paths (e.g. `EFI/Microsoft/Boot/bcd`). Implementations should treat the path
lookup as case-insensitive.

Also note that on Windows these files are often marked hidden/system/read-only.
An offline patcher must ensure the files are writable before attempting to write
them back.

#### Extracted OS image (installed OS template)

Patch:

- `Windows/System32/Config/BCD-Template`

`BCD-Template` is the registry-hive template Windows Setup uses when creating the installed
system’s BCD store (e.g. the eventual `\\Boot\\BCD`). If we don’t patch the template, an installed
image can “lose” the settings even if the installer media BCD was patched.

---

### BCD internals (minimum needed for offline patching)

BCD stores are **registry hives** in standard `REGF` format (the same on-disk format used for
`SYSTEM`, `SOFTWARE`, etc.). This is why offline patching can be implemented as “edit a registry
hive file”.

The minimum structure you need to know is:

```
<root>
  Objects
    {GUID}
      Elements
        <8-hex element type>        (key name, 8 hex digits, no `0x` prefix)
          Element                   (value name)
```

Concretely, the offline patcher usually writes values like:

```
Objects\{<object-guid>}\Elements\16000049\Element
Objects\{<object-guid>}\Elements\16000048\Element
```

Notes:

- The `{GUID}` directory names are the object identifiers (what `bcdedit` displays as `{...}`
  entries).
- Each element is keyed by its **32-bit element type ID**, rendered as **8 hex digits**
  (zero-padded) for the subkey name.
- The element payload is stored in the `Element` value under that element-type key.

#### Boolean encoding (Win7)

Element data in BCD hives is stored as `REG_BINARY`. For the Win7 boolean elements used by this
tool, the simplest working encoding is:

```
[u32 element_type (LE)] [u32 data_len (LE)] [data...]
```

For these Win7 boolean elements, `data_len` is 4 and the payload is a little-endian u32:

- enabled: `01 00 00 00`
- disabled: `00 00 00 00`

The two boolean element types we care about are:

| Element type ID | Elements subkey | `bcdedit` name | BCD element constant name |
| --- | --- | --- | --- |
| `0x16000048` | `16000048` | `nointegritychecks` | `BcdLibraryBoolean_DisableIntegrityChecks` (`DisableIntegrityChecks`) |
| `0x16000049` | `16000049` | `testsigning` | `BcdLibraryBoolean_AllowPrereleaseSignatures` (`AllowPrereleaseSignatures`) |

These are **Library Boolean** elements (the `0x16xxxxxx` element type range).

So the full `Element` blob is typically 12 bytes, for example:

- `testsigning` (`0x16000049`, enabled):
  - `49 00 00 16  04 00 00 00  01 00 00 00`
- `nointegritychecks` (`0x16000048`, enabled):
  - `48 00 00 16  04 00 00 00  01 00 00 00`

This matches the audited offline `.reg` patches under `tools/win7-slipstream/patches/`.

Note: the BCD element record encoding is not documented as a public stability guarantee, so an
offline patcher should validate its assumptions against at least one real Win7 BCD store (e.g. by
loading it with `reg.exe load` and inspecting an existing `Elements\\16xxxxxx\\Element` value), then
write values in the same format.

---

### Well-known object GUIDs (Win7)

These are the canonical GUIDs behind `bcdedit` aliases:

| `bcdedit` alias                | GUID                                  | Notes |
|--------------------------------|---------------------------------------|------|
| `{bootmgr}`                    | `9dea862c-5cdd-4e70-acc1-f32b344d4795` | Boot Manager object. Used to find `default`/`displayorder` entries. |
| `{globalsettings}`             | `7ea2e1ac-2e61-4728-aaa3-896d9d0a9f0e` | Global “library” settings inherited by many objects. |
| `{bootloadersettings}`         | `6efb52bf-1766-41db-a6b3-0ee5eff72bd7` | Template inherited by Windows Boot Loader entries. |
| `{resumeloadersettings}`       | `1afa9c49-16ab-4a5c-901b-212802da9460` | Template inherited by Windows Resume Loader entries. |
| `{memdiag}` (commonly present) | `b2721d73-1db4-4c62-bf78-c548a880142d` | Windows Memory Diagnostic. Usually present in boot menu display order. |
| `{ntldr}` (optional)           | `466f5a88-0af2-4f76-9038-095b170dc21c` | Legacy NTLDR entry, only present if created. |

#### Reference: audited `.reg` patches in this repo

If you need a concrete, known-good offline patch format, see:

- `tools/win7-slipstream/patches/bcd-testsigning.reg`
- `tools/win7-slipstream/patches/bcd-nointegritychecks.reg`

They patch the well-known settings objects by GUID (see table above) by writing the `Element`
value directly.

Object GUID key names are **case-insensitive**. Some tools show GUIDs with braces (`{...}`) and
some without; offline patching should tolerate both.

---

### Element type IDs used by the offline patcher

BCD “elements” are identified by a 32-bit `BCD_ELEMENT_TYPE` number. In the registry hive, each
element is stored as a subkey under `Elements` named as **8-digit hex** (e.g. `16000048`).

Only the element types actively used by the patcher are listed here:

| Hex element type | Meaning / `bcdedit` name | Data kind  | Used for |
|------------------|--------------------------|-----------|----------|
| `0x16000048`     | `nointegritychecks`      | Boolean   | Disables kernel-mode code integrity checks. |
| `0x16000049`     | `testsigning`            | Boolean   | Allows test-signed / prerelease signatures. |
| `0x12000002`     | `applicationpath`        | String    | Identifies loader objects (`winload.*`, `winresume.*`). |
| `0x23000003`     | `{bootmgr} default`      | Object    | GUID of the default boot entry. |
| `0x24000001`     | `{bootmgr} displayorder` | ObjectList | Ordered list of boot entries shown in the boot menu. |

---

### Deterministic object selection (what gets patched)

To make the setting effective across the different boot paths Windows 7 uses, patch multiple
objects when present:

1. `{globalsettings}`
2. `{bootloadersettings}`
3. `{resumeloadersettings}` (if present)
4. All OS/resume loader entries (`winload*`, `winresume*`) discovered by `applicationpath`
5. Additionally, objects referenced by `{bootmgr}`’s `displayorder`/`default` are included when
   those elements exist.

Note on object identifiers: `bcdedit` supports symbolic names like `{default}` and `{bootmgr}`.
Offline patchers work directly with the REGF hive and therefore typically patch objects by their
GUID keys under `Objects\\{GUID}`. Patching the well-known settings objects and all `winload*`
entries avoids having to resolve store-specific aliases like `{default}`.

#### Locating OS loader entries programmatically (offline)

On Windows 7, OS loader objects include an application path element that points at `winload.exe`
(BIOS) or `winload.efi` (UEFI). Resume loader objects similarly reference `winresume.*`.

To find them offline:

- Enumerate `Objects\\{GUID}` under the BCD hive.
- For each object, look for the element:
  - `BcdLibraryString_ApplicationPath` (`0x12000002`)
  - (i.e. `Objects\\{GUID}\\Elements\\12000002\\Element`)
- Decode the element’s string value and check whether it contains `winload` or `winresume`
  (case-insensitive substring match is sufficient in practice).

In practice, `BcdLibraryString_ApplicationPath` is a Windows path such as
`\\Windows\\system32\\winload.exe` / `winload.efi`. If you don’t want to fully decode the
element, scanning the raw bytes for the UTF-16LE substring `w\0i\0n\0l\0o\0a\0d\0` is a pragmatic
approach.

---

### Verification steps (developers with Win7 media)

#### Enumerate objects using `bcdedit`

To verify a patched store from a Windows host (no VM required), use `bcdedit` against the file
directly:

```bat
bcdedit /store <path-to-BCD> /enum all /v
```

In the output, confirm the relevant entries contain:

- `testsigning              Yes`
- `nointegritychecks        Yes`

If you specifically patched the settings objects (recommended), you can also check them directly:

```bat
bcdedit /store <path-to-BCD> /enum {globalsettings} /v
bcdedit /store <path-to-BCD> /enum {bootloadersettings} /v
```

For a low-level check of the exact bytes written, you can load the store as a hive (requires an
elevated prompt) and query the element value directly:

```bat
reg load HKLM\BCD <path-to-BCD>
reg query HKLM\BCD\Objects\{7ea2e1ac-2e61-4728-aaa3-896d9d0a9f0e}\Elements\16000049 /v Element
reg query HKLM\BCD\Objects\{7ea2e1ac-2e61-4728-aaa3-896d9d0a9f0e}\Elements\16000048 /v Element
reg unload HKLM\BCD
```

If those settings show up as `No` or are missing entirely, the offline patch did not apply to the
object(s) Windows is actually booting through (most commonly: only patching one of the ISO BCD
stores, or patching settings objects but not the OS loader objects themselves).

#### Load the BCD hive and inspect object/element keys

These steps let you confirm both GUID presence and element type IDs on a real Windows 7 BCD store
without committing any binaries to the repo.

Load the hive into a temporary registry key:

```bat
reg.exe load HKLM\BCD_OFFLINE X:\Boot\BCD
```

List well-known object keys (GUIDs):

```bat
reg.exe query HKLM\BCD_OFFLINE\Objects
```

Inspect element type subkeys for an object (example: `{bootloadersettings}`):

```bat
reg.exe query HKLM\BCD_OFFLINE\Objects\{6efb52bf-1766-41db-a6b3-0ee5eff72bd7}\Elements
```

You should see element subkeys named like `16000048`, `16000049`, etc, matching the IDs above.

Unload when done:

```bat
reg.exe unload HKLM\BCD_OFFLINE
```

---

### Implementation linkage

The canonical constants live in:

- `tools/bcd_patch/src/constants.rs`

The patcher’s selection strategy is implemented (and unit tested) in:

- `tools/bcd_patch/src/lib.rs` (`select_target_objects`)

## Windows 7 CryptoAPI registry certificate `Blob` format (byte-level)

Windows 7 / WinPE system certificate stores persisted in the registry under:

`HKLM\SOFTWARE\Microsoft\SystemCertificates\<STORE>\Certificates\<SHA1>\Blob`

store each certificate as a `REG_BINARY` value called `Blob`.

Per-user system stores use the same format under:

`HKCU\SOFTWARE\Microsoft\SystemCertificates\<STORE>\Certificates\<SHA1>\Blob`

This document specifies the **exact** binary layout of that `Blob` value on
Windows 7, and how it relates to CryptoAPI serialization APIs.

### Ground truth / relationship to CryptoAPI

On Windows 7, the registry provider uses the same serialized form produced by:

- `CertSerializeCertificateStoreElement()` (for a `PCCERT_CONTEXT`)

and the resulting bytes can be round-tripped back into a store with:

- `CertAddSerializedElementToStore()`

The included harness (`tools/win-blob-dump/`) prints and validates:

1. The raw output of `CertSerializeCertificateStoreElement()`
2. A byte-for-byte comparison with the registry `Blob` value for the same cert:
   - `HKCU\...` always (current user store)
   - `HKLM\...` when run with sufficient privileges (local machine store)

> Note: Some *container* formats (e.g. a serialized store file created via
> `CertSaveStore(CERT_STORE_SAVE_AS_STORE, ...)`) may wrap each element with an
> additional record header (length, context type, etc). The registry `Blob`
> value is just the per-certificate element bytes, not the outer container
> framing.

### High-level shape

The blob is **not** only `[dwEncodingType][cbCert][DER]`.

It is:

1. A fixed header (`dwCertEncodingType`, `cbCertEncoded`)
2. The DER-encoded certificate bytes
3. A persisted **property section** (count + property entries)

All integer fields are **little-endian**.

### Struct layout (packed, with 4-byte alignment rules)

```c
// Little-endian.
//
// Note: "alignment" here refers to *padding inside the serialized blob*,
// not in-memory struct padding. Windows aligns the start of the property
// section and each subsequent property entry to a 4-byte boundary.

struct Win7RegistryCertBlob {
    u32 dwCertEncodingType; // Usually 0x00010001 (X509_ASN_ENCODING | PKCS_7_ASN_ENCODING)
    u32 cbCertEncoded;      // Length in bytes of pbCertEncoded (DER)
    u8  pbCertEncoded[cbCertEncoded];
    u8  pad0[(4 - (cbCertEncoded % 4)) % 4]; // zero bytes

    u32 cProperties;        // Count of persisted properties following
    Win7SerializedProperty properties[cProperties];
};

struct Win7SerializedProperty {
    u32 dwPropId;   // CERT_*_PROP_ID
    u32 cbValue;    // Length in bytes of value[] (not including padding)
    u8  value[cbValue];
    u8  pad[(4 - (cbValue % 4)) % 4]; // zero bytes
};
```

#### Notes / invariants

- `dwCertEncodingType` is the same encoding value you would pass to
  `CertCreateCertificateContext()`.
  - In practice Windows uses `X509_ASN_ENCODING | PKCS_7_ASN_ENCODING` (`0x00010001`)
    for system stores.
- `pbCertEncoded` is the **raw DER certificate**, exactly as imported.
- `cProperties` counts only the properties that CryptoAPI considers **persistable**
  for that context at the time it is written.
- `dwPropId` ordering is the order produced by `CertEnumCertificateContextProperties`
  on Win7 (observed to be ascending numeric order in practice).
- The blob uses **4-byte padding** (0x00 bytes) to align the following DWORD:
  - after the certificate DER
  - after each property value

### Padding / alignment examples

#### Padding after DER (`cbCertEncoded % 4 != 0`)

Example certificate (unaligned length):

- File: `tools/win-blob-dump/examples/example_unaligned_cert.der`
- `cbCertEncoded = 506 (0x01fa)`

The next DWORD (`cProperties`) starts at the next 4-byte boundary, so after the
DER bytes there are 2 bytes of `0x00` padding:

```text
... (last 16 bytes of DER) ...
000001f2: 74 55 c0 65 5b ee f2 b3 5f 72 4f 89 52 9a f5 15 |tU.e[..._rO.R...|
00000202: 00 00 00 00 00 00                               |......|
          ^^ ^^
          pad0 = 2 bytes
                ^^^^^^^^^^^
                cProperties = 0 (DWORD)
```

#### Padding after a property value (`cbValue % 4 != 0`)

Example: a `CERT_FRIENDLY_NAME_PROP_ID` value of `"Aero\0"` has:

- `cbValue = 10 (0x0a)` bytes (UTF-16LE)
- followed by 2 bytes of padding to align the next property header to a DWORD boundary

```text
00000224: 01 00 00 00 0b 00 00 00 0a 00 00 00 41 00 65 00 |............A.e.|
          ^^^^^^^^^^^
          cProperties = 1
                      ^^^^^^^^^^^  ^^^^^^^^^^^
                      propId = 11  cbValue = 10
                                            ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                                            UTF-16LE "Aero\0"
00000234: 72 00 6f 00 00 00 00 00                         |r.o.....|
                    ^^^^^^
                    terminator (00 00)
                          ^^ ^^
                          padding (2 bytes)
```

### Persisted properties: what you actually see

The persisted properties depend on how the cert was added/imported, but common
examples include:

| Property | ID | Typical meaning |
|---|---:|---|
| `CERT_KEY_PROV_INFO_PROP_ID` | 2 | Links the cert to a legacy CryptoAPI private key container (CSP) |
| `CERT_FRIENDLY_NAME_PROP_ID` | 11 | User-facing display name (UTF-16LE string, typically NUL-terminated) |
| `CERT_ARCHIVED_PROP_ID` | 19 | Whether a cert is archived in the store |

Property **values** are stored as opaque byte blobs; most are the same as
returned by `CertGetCertificateContextProperty()`, but some property IDs (notably
`CERT_KEY_PROV_INFO_PROP_ID`) use an internal serialized form suitable for
persistence (i.e., not raw process pointers).

#### `CERT_FRIENDLY_NAME_PROP_ID` (11)

- Value is a UTF-16LE string (typically NUL-terminated).
- `cbValue` is the byte length of the UTF-16LE buffer stored in the blob.

#### `CERT_KEY_PROV_INFO_PROP_ID` (2)

The `CERT_KEY_PROV_INFO` property is documented as a `CRYPT_KEY_PROV_INFO`
structure containing pointers, but the persisted bytes inside the registry
`Blob` must be architecture-independent.

The included harness prints a heuristic decode of the property value that
matches an **offset-based** serialization (32-bit offsets from the start of the
property value) of the form:

```c
// Little-endian, offsets are relative to the start of this value blob.
// Strings are UTF-16LE and typically NUL-terminated.
struct Win7PersistedCryptKeyProvInfo {
    u32 offContainerName;     // -> wchar_t[]
    u32 offProvName;          // -> wchar_t[]
    u32 dwProvType;
    u32 dwFlags;
    u32 cProvParam;
    u32 offProvParamArray;    // -> Win7PersistedCryptKeyProvParam[cProvParam] (or 0 if none)
    u32 dwKeySpec;
    // ...variable data (strings, params, param data)...
};

struct Win7PersistedCryptKeyProvParam {
    u32 dwParam;
    u32 offData;              // -> u8[cbData]
    u32 cbData;
    u32 dwFlags;
};
```

If you need to generate byte-identical `CERT_KEY_PROV_INFO_PROP_ID` payloads,
run the harness on Win7 and use the printed offsets/bytes as the reference.

### Annotated hexdumps (from the harness)

The harness (`tools/win-blob-dump/win_blob_dump.c`) prints full hexdumps.
Below are full hexdumps for a small, reproducible example certificate, laid out
according to the format described in this document (i.e. these are the expected
bytes for that input).

Example certificate:

- File: `tools/win-blob-dump/examples/example_cert.der`
- SHA1 (thumbprint / registry key name): `FDA7D93129AF9CE5317A0FA9CD466FB562A3982C`
- `cbCertEncoded = 540 (0x21c)`

#### Example A: no extra properties (`cProperties = 0`)

Notes:

- `dwCertEncodingType = 0x00010001` (`X509_ASN_ENCODING | PKCS_7_ASN_ENCODING`)
- `cbCertEncoded = 0x0000021c`
- Since `cbCertEncoded % 4 == 0`, there is **no** padding after DER before `cProperties`.

```text
00000000: 01 00 01 00 1c 02 00 00 30 82 02 18 30 82 01 81 |........0...0...|
00000010: a0 03 02 01 02 02 14 27 ea 81 00 1b f1 cd 55 50 |.......'......UP|
00000020: 68 8b d0 8a 3f b4 af ab b3 59 31 30 0d 06 09 2a |h...?....Y10...*|
00000030: 86 48 86 f7 0d 01 01 0b 05 00 30 1e 31 1c 30 1a |.H........0.1.0.|
00000040: 06 03 55 04 03 0c 13 41 65 72 6f 42 6c 6f 62 44 |..U....AeroBlobD|
00000050: 75 6d 70 45 78 61 6d 70 6c 65 30 1e 17 0d 32 36 |umpExample0...26|
00000060: 30 31 31 30 31 32 30 35 34 32 5a 17 0d 33 36 30 |0110120542Z..360|
00000070: 31 30 38 31 32 30 35 34 32 5a 30 1e 31 1c 30 1a |108120542Z0.1.0.|
00000080: 06 03 55 04 03 0c 13 41 65 72 6f 42 6c 6f 62 44 |..U....AeroBlobD|
00000090: 75 6d 70 45 78 61 6d 70 6c 65 30 81 9f 30 0d 06 |umpExample0..0..|
000000a0: 09 2a 86 48 86 f7 0d 01 01 01 05 00 03 81 8d 00 |.*.H............|
000000b0: 30 81 89 02 81 81 00 f5 a7 65 2c e8 85 f2 0b ad |0........e,.....|
000000c0: 5f b4 a9 ae f4 eb ba 3a ef 2e 81 e0 de cf 31 54 |_......:......1T|
000000d0: d4 4e 3c 22 59 01 0c 67 ba af e1 ee 0c b6 55 fd |.N<"Y..g......U.|
000000e0: 1c 5c 51 53 ee 5c ef cf 04 9e e7 36 f7 ab c5 98 |.\QS.\.....6....|
000000f0: a5 e4 e7 d1 3e 2d 96 00 7a d5 c6 cd 13 e1 83 05 |....>-..z.......|
00000100: 16 cb af 5d 77 2e ba 0f 2b 00 7c 12 d1 2e 4a 79 |...]w...+.|...Jy|
00000110: 68 14 3d 34 15 63 94 f3 e5 71 b0 be 60 d4 01 c1 |h.=4.c...q..`...|
00000120: c6 8d cc d6 4b 7f c4 ce 91 6b a6 9d 4a c2 c0 c0 |....K....k..J...|
00000130: 25 ba b3 12 19 3a a7 02 03 01 00 01 a3 53 30 51 |%....:.......S0Q|
00000140: 30 1d 06 03 55 1d 0e 04 16 04 14 8d 61 90 7f 9f |0...U.......a...|
00000150: 0b fa 51 23 b5 14 40 8a 69 11 67 e9 2e bc f4 30 |..Q#..@.i.g....0|
00000160: 1f 06 03 55 1d 23 04 18 30 16 80 14 8d 61 90 7f |...U.#..0....a..|
00000170: 9f 0b fa 51 23 b5 14 40 8a 69 11 67 e9 2e bc f4 |...Q#..@.i.g....|
00000180: 30 0f 06 03 55 1d 13 01 01 ff 04 05 30 03 01 01 |0...U.......0...|
00000190: ff 30 0d 06 09 2a 86 48 86 f7 0d 01 01 0b 05 00 |.0...*.H........|
000001a0: 03 81 81 00 df 96 50 6f 7c 89 1b f4 60 17 be 64 |......Po|...`..d|
000001b0: af d1 65 86 1c 08 a9 2f b3 20 fe 0e 57 07 f3 c0 |..e..../. ..W...|
000001c0: ed 90 71 03 f0 49 14 42 7c 1d 60 7b 4f 1a ce a6 |..q..I.B|.`{O...|
000001d0: 49 f9 60 0a a5 37 18 76 6f 79 ae 19 75 6d 56 0f |I.`..7.voy..umV.|
000001e0: 3c 03 3d 32 0d dd bd a0 0a 84 7f 54 76 fd 8e 00 |<.=2.......Tv...|
000001f0: 3a 6f 68 71 25 f9 6b e2 39 ff 3b 4b ac 9d 92 0a |:ohq%.k.9.;K....|
00000200: 57 14 33 0c d4 44 24 9f cf 52 a2 37 0d 73 26 bc |W.3..D$..R.7.s&.|
00000210: ab 1e 27 ef a3 50 39 f3 a8 6b d6 db c5 d0 17 03 |..'..P9..k......|
00000220: fb 5a 5d db 00 00 00 00                         |.Z].....|
```

#### Example B: with a persisted property (`FriendlyName`, `cProperties = 1`)

Notes:

- `cProperties = 1`
- One property entry is appended:
  - `dwPropId = 11` (`CERT_FRIENDLY_NAME_PROP_ID`)
  - `cbValue = 0x28`
  - Value is UTF-16LE `"AeroBlobDumpExample\0"`

```text
00000000: 01 00 01 00 1c 02 00 00 30 82 02 18 30 82 01 81 |........0...0...|
00000010: a0 03 02 01 02 02 14 27 ea 81 00 1b f1 cd 55 50 |.......'......UP|
00000020: 68 8b d0 8a 3f b4 af ab b3 59 31 30 0d 06 09 2a |h...?....Y10...*|
00000030: 86 48 86 f7 0d 01 01 0b 05 00 30 1e 31 1c 30 1a |.H........0.1.0.|
00000040: 06 03 55 04 03 0c 13 41 65 72 6f 42 6c 6f 62 44 |..U....AeroBlobD|
00000050: 75 6d 70 45 78 61 6d 70 6c 65 30 1e 17 0d 32 36 |umpExample0...26|
00000060: 30 31 31 30 31 32 30 35 34 32 5a 17 0d 33 36 30 |0110120542Z..360|
00000070: 31 30 38 31 32 30 35 34 32 5a 30 1e 31 1c 30 1a |108120542Z0.1.0.|
00000080: 06 03 55 04 03 0c 13 41 65 72 6f 42 6c 6f 62 44 |..U....AeroBlobD|
00000090: 75 6d 70 45 78 61 6d 70 6c 65 30 81 9f 30 0d 06 |umpExample0..0..|
000000a0: 09 2a 86 48 86 f7 0d 01 01 01 05 00 03 81 8d 00 |.*.H............|
000000b0: 30 81 89 02 81 81 00 f5 a7 65 2c e8 85 f2 0b ad |0........e,.....|
000000c0: 5f b4 a9 ae f4 eb ba 3a ef 2e 81 e0 de cf 31 54 |_......:......1T|
000000d0: d4 4e 3c 22 59 01 0c 67 ba af e1 ee 0c b6 55 fd |.N<"Y..g......U.|
000000e0: 1c 5c 51 53 ee 5c ef cf 04 9e e7 36 f7 ab c5 98 |.\QS.\.....6....|
000000f0: a5 e4 e7 d1 3e 2d 96 00 7a d5 c6 cd 13 e1 83 05 |....>-..z.......|
00000100: 16 cb af 5d 77 2e ba 0f 2b 00 7c 12 d1 2e 4a 79 |...]w...+.|...Jy|
00000110: 68 14 3d 34 15 63 94 f3 e5 71 b0 be 60 d4 01 c1 |h.=4.c...q..`...|
00000120: c6 8d cc d6 4b 7f c4 ce 91 6b a6 9d 4a c2 c0 c0 |....K....k..J...|
00000130: 25 ba b3 12 19 3a a7 02 03 01 00 01 a3 53 30 51 |%....:.......S0Q|
00000140: 30 1d 06 03 55 1d 0e 04 16 04 14 8d 61 90 7f 9f |0...U.......a...|
00000150: 0b fa 51 23 b5 14 40 8a 69 11 67 e9 2e bc f4 30 |..Q#..@.i.g....0|
00000160: 1f 06 03 55 1d 23 04 18 30 16 80 14 8d 61 90 7f |...U.#..0....a..|
00000170: 9f 0b fa 51 23 b5 14 40 8a 69 11 67 e9 2e bc f4 |...Q#..@.i.g....|
00000180: 30 0f 06 03 55 1d 13 01 01 ff 04 05 30 03 01 01 |0...U.......0...|
00000190: ff 30 0d 06 09 2a 86 48 86 f7 0d 01 01 0b 05 00 |.0...*.H........|
000001a0: 03 81 81 00 df 96 50 6f 7c 89 1b f4 60 17 be 64 |......Po|...`..d|
000001b0: af d1 65 86 1c 08 a9 2f b3 20 fe 0e 57 07 f3 c0 |..e..../. ..W...|
000001c0: ed 90 71 03 f0 49 14 42 7c 1d 60 7b 4f 1a ce a6 |..q..I.B|.`{O...|
000001d0: 49 f9 60 0a a5 37 18 76 6f 79 ae 19 75 6d 56 0f |I.`..7.voy..umV.|
000001e0: 3c 03 3d 32 0d dd bd a0 0a 84 7f 54 76 fd 8e 00 |<.=2.......Tv...|
000001f0: 3a 6f 68 71 25 f9 6b e2 39 ff 3b 4b ac 9d 92 0a |:ohq%.k.9.;K....|
00000200: 57 14 33 0c d4 44 24 9f cf 52 a2 37 0d 73 26 bc |W.3..D$..R.7.s&.|
00000210: ab 1e 27 ef a3 50 39 f3 a8 6b d6 db c5 d0 17 03 |..'..P9..k......|
00000220: fb 5a 5d db 01 00 00 00 0b 00 00 00 28 00 00 00 |.Z].........(...|
00000230: 41 00 65 00 72 00 6f 00 42 00 6c 00 6f 00 62 00 |A.e.r.o.B.l.o.b.|
00000240: 44 00 75 00 6d 00 70 00 45 00 78 00 61 00 6d 00 |D.u.m.p.E.x.a.m.|
00000250: 70 00 6c 00 65 00 00 00                         |p.l.e...|
```

### How to generate byte-identical blobs outside Windows

To generate a Win7-compatible registry `Blob` for a cert:

1. Write the 8-byte header (`dwCertEncodingType`, `cbCertEncoded`).
2. Append the DER bytes.
3. Append `pad0` to a 4-byte boundary.
4. Append `cProperties` (DWORD).
5. For each persisted property:
   1. Append `dwPropId` (DWORD)
   2. Append `cbValue` (DWORD)
   3. Append `value` bytes
   4. Append zero padding to a 4-byte boundary

The main remaining complexity is producing byte-identical `value` bytes for
properties like `CERT_KEY_PROV_INFO_PROP_ID`. The harness is designed to make
this visible by printing the serialized property payloads.

## Windows 7 Driver Troubleshooting (Aero Guest Tools)

This document covers common Windows 7 issues after installing **Aero Guest Tools** and switching the VM from baseline emulated devices (**AHCI/IDE/e1000/VGA**) to paravirtual devices (**virtio + Aero GPU**).

If you have not installed Guest Tools yet, start here:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md)
- For the canonical Windows 7 boot/install storage topology (AHCI HDD + IDE/ATAPI CD-ROM), see
  [`../areas/storage.md`](../areas/storage.md).

### Before you start: quick triage checklist

1. **Don’t keep rebooting** if you hit a boot loop or `0x7B` BSOD after switching storage. Power off and use the rollback path.
2. Collect `report.txt` by running `verify.cmd` as Administrator and opening `C:\AeroGuestTools\report.txt`. Pay special attention to any `Code 52` (signing/trust) or `Code 28` (driver not installed) device errors.
   - Also note the Guest Tools `signing_policy` (from `manifest.json`) and the **Signature Mode (BCDEdit)** check; together they tell you whether Test Signing is expected.
3. Confirm you’re using drivers that match your OS:
   - Windows 7 **x86** requires x86 drivers.
   - Windows 7 **x64** requires x64 drivers. (32-bit drivers cannot load.)
4. If you changed multiple VM devices at once (storage + GPU + network), consider rolling back and switching **one class at a time** so failures are easier to isolate.
5. Confirm the guest **date/time** is correct. If the clock is far off, Windows may treat certificates as “not yet valid” or “expired” and driver signature validation can fail.

### Quick links by symptom

- Driver signature / trust failures:
  - [Device Manager Code 52 (signature and trust failures)](#issue-device-manager-code-52-signature-and-trust-failures)
  - [Catalog hash mismatch (hash not present in specified catalog file)](#issue-catalog-hash-mismatch-hash-not-present-in-specified-catalog-file)
  - [Guest Tools media integrity check fails (manifest hash mismatch)](#issue-guest-tools-media-integrity-check-fails-manifest-hash-mismatch)
  - [Missing KB3033929 (SHA-256 signature support)](#issue-missing-kb3033929-sha-256-signature-support)
- Driver installed but not working:
  - [Device Manager Code 28 (drivers not installed)](#issue-device-manager-code-28-drivers-not-installed)
  - [Device Manager Code 10 (device cannot start)](#issue-device-manager-code-10-device-cannot-start)
  - [Virtio device not found or Unknown device after switching](#issue-virtio-device-not-found-or-unknown-device-after-switching)
  - [Lost keyboard/mouse after switching to virtio-input](#issue-lost-keyboardmouse-after-switching-to-virtio-input)
- Boot failures after switching storage:
  - [Storage controller switch gotchas (boot loops, 0x7B)](#issue-storage-controller-switch-gotchas-boot-loops-0x7b)
  - [No bootable device or BOOTMGR is missing after switching storage](#issue-no-bootable-device-or-bootmgr-is-missing-after-switching-storage)
- Windows Setup disk detection issues:
  - [Windows Setup can't see a virtio-blk disk](#issue-windows-setup-cant-see-a-virtio-blk-disk-slipstream-installs)
- Display issues after switching to the Aero GPU:
  - [Black screen after switching to the Aero GPU](#issue-black-screen-after-switching-to-the-aero-gpu)
  - [Aero theme not available (stuck in basic graphics mode)](#issue-aero-theme-not-available-stuck-in-basic-graphics-mode)
  - [Allocation failures (E_OUTOFMEMORY)](#issue-allocation-failures-e_outofmemory)
  - [32-bit D3D9 apps fail on Windows 7 x64 (missing WOW64 UMD)](#issue-32-bit-d3d9-apps-fail-on-windows-7-x64-missing-wow64-umd)
  - [32-bit D3D11 apps fail on Windows 7 x64 (missing WOW64 D3D10/11 UMD)](#issue-32-bit-d3d11-apps-fail-on-windows-7-x64-missing-wow64-d3d1011-umd)
- Guest Tools installation problems:
  - [`setup.cmd` fails (won't run)](#issue-setupcmd-fails-wont-run)
  - [Safe Mode recovery tips](#safe-mode-recovery-tips)
- Expected behavior:
  - [Test Mode watermark on the desktop (x64)](#issue-test-mode-watermark-on-the-desktop-x64)
- Diagnostics:
  - [Collecting useful logs](#collecting-useful-logs)
  - [Dumping the last AeroGPU submission (cmd stream and alloc table)](#dumping-the-last-aerogpu-submission-cmd-stream-and-alloc-table)
  - [Finding device Hardware IDs](#finding-device-hardware-ids)
  - [Capturing BSOD stop codes](#capturing-bsod-stop-codes)

### Collecting useful logs

If you need to debug driver install failures, these are the most useful artifacts to gather:

- `C:\AeroGuestTools\report.txt` (from `verify.cmd`)
- `C:\AeroGuestTools\report.json` (from `verify.cmd`, machine-readable)
- `C:\AeroGuestTools\install.log` (from `setup.cmd`)
- `C:\AeroGuestTools\uninstall.log` (from `uninstall.cmd`, if used)
- Device Manager → device → **Properties**:
  - **General** tab (error code)
  - **Events** tab (device install/start failures)
- Driver installation log:
  - `C:\Windows\inf\setupapi.dev.log`
    - Tip: open it and search for the device’s Hardware ID or the `.inf` name.

### Dumping the last AeroGPU submission (cmd stream and alloc table)

If you hit a **GPU hang**, **TDR**, or **incorrect rendering** and need to debug what command stream the guest last submitted (without attaching WinDbg), capture the last submission’s binary blobs and decode them on the host.

This produces one or more small, shareable artifacts:

- `cmd.bin`: the raw AeroGPU cmd stream for the submission (`cmd_gpa` region).
- `alloc.bin` (or `cmd.bin.alloc_table.bin`): the raw alloc table for the submission (`alloc_table_gpa` region, when present; AGPU only).
- `cmd.bin.txt`: a small text summary (ring index, fence, GPAs/sizes).

Also capture a one-shot dbgctl snapshot (recommended):

- `aerogpu_dbgctl.exe --status` (or `--status --json=C:\status.json`)
  - Includes fences/ring state and the most recent **latched device error** (`Last error:`) when supported (ABI 1.3+ / `AEROGPU_IRQ_ERROR`).

#### Guest (Windows 7): dump the last submission
  
Run this inside the guest as soon as possible after reproducing:

From a default Aero Guest Tools ISO/zip mount (often `X:`), run the command that matches your guest OS:

- Win7 x64:

```bat
X:\drivers\amd64\aerogpu\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe --dump-last-cmd --out C:\cmd.bin
```

- Win7 x86:

```bat
X:\drivers\x86\aerogpu\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe --dump-last-cmd --out C:\cmd.bin
```

Equivalent newer spelling (same behavior; accepted by newer dbgctl builds; assumes you're running from the dbgctl directory or have it on `PATH`):

```bat
aerogpu_dbgctl.exe --dump-last-submit --cmd-out C:\cmd.bin
```

Notes on legacy spellings (kept for compatibility with older dbgctl builds):

- For `aerogpu_dbgctl.exe`, `--dump-last-cmd` is an alias for `--dump-last-submit`.
- For `aerogpu_dbgctl.exe --dump-last-submit`, `--out` is an alias for `--cmd-out`.

For `aerogpu_dbgctl.exe --dump-last-submit`, if you want a stable alloc-table output filename (`C:\alloc.bin`) instead of the default `C:\cmd.bin.alloc_table.bin` (AGPU only), set `--alloc-out` (only supported when dumping a single submission, i.e. `--count 1`):

```bat
:: Replace <GuestToolsDrive> with the drive letter of the mounted Guest Tools ISO/zip (e.g. D).
:: Win7 x64:
cd /d <GuestToolsDrive>:\drivers\amd64\aerogpu\tools\win7_dbgctl\bin
:: Win7 x86:
:: cd /d <GuestToolsDrive>:\drivers\x86\aerogpu\tools\win7_dbgctl\bin

aerogpu_dbgctl.exe --dump-last-submit --cmd-out C:\cmd.bin --alloc-out C:\alloc.bin
```

Then copy `C:\cmd.bin`, `C:\cmd.bin.txt`, and any alloc-table dump that `aerogpu_dbgctl` produced (`C:\alloc.bin` if you passed `--alloc-out`, or `C:\cmd.bin.alloc_table.bin` when present) to the host machine (shared folder, ISO, whatever is convenient).

Notes:

- On Win7 x64, `aerogpu_dbgctl.exe` is intentionally an **x86 (32-bit)** binary and runs under **WOW64**
  (even when invoked from `X:\drivers\amd64\...`), so you can run it directly from the mounted media without editing `PATH`.
- This requires the installed KMD to allow the debug-only `AEROGPU_ESCAPE_OP_READ_GPA` escape.
  - If `READ_GPA` is not enabled/authorized, dbgctl will fail with `STATUS_NOT_SUPPORTED` (`0xC00000BB`).
  - To enable it, set (and reboot/restart the driver):  
    `HKLM\SYSTEM\CurrentControlSet\Services\aerogpu\Parameters\EnableReadGpaEscape = 1` (REG_DWORD)  
    and run dbgctl as a privileged user (Administrator and/or `SeDebugPrivilege`).
- If `aerogpu_dbgctl` refuses to dump due to the default size cap (1 MiB), re-run with `--force`:
  - `aerogpu_dbgctl.exe --dump-last-submit --cmd-out C:\cmd.bin --alloc-out C:\alloc.bin --force`
- For `aerogpu_dbgctl.exe --dump-last-submit`, to capture an older submission (for example if the newest submit is a tiny no-op), use `--index-from-tail`:
  - `aerogpu_dbgctl.exe --dump-last-submit --index-from-tail 1 --cmd-out C:\prev_cmd.bin --alloc-out C:\prev_alloc.bin`
- For `aerogpu_dbgctl.exe --dump-last-submit`, to dump multiple recent submissions in one run, use `--count N` (writes one output per submission, like `cmd_0.bin`, `cmd_1.bin`, ...).
  - Note: for `aerogpu_dbgctl --dump-last-submit`, `--alloc-out` is only supported when dumping a single submission (`--count 1`). When dumping multiple submissions, `aerogpu_dbgctl` writes alloc tables (when present) to `<cmd_path>.alloc_table.bin` next to each dumped cmd stream.
  - `aerogpu_dbgctl.exe --dump-last-submit --count 4 --cmd-out C:\cmd.bin`
- For `aerogpu_dbgctl.exe`, if your build uses multiple rings, select the ring with `--ring-id N` (default is 0).

#### Host: decode the submission

From the repo root on the host:

If `alloc.bin` exists (or you have `cmd.bin.alloc_table.bin`), decode the full submission (replace the `--alloc` path as needed):

```bash
cargo run -p aero-gpu-trace-replay -- decode-submit --cmd cmd.bin --alloc alloc.bin
```

To inspect the alloc table itself (alloc_id → gpa/size/flags), run (replace the `alloc.bin` path as needed):

```bash
cargo run -p aero-gpu-trace-replay -- decode-alloc-table alloc.bin
```

If there is no alloc table dump (common on legacy ring formats), skip `decode-submit` and use `decode-cmd-stream` below.

#### Optional: list opcodes directly (`decode-cmd-stream`)

You can also decode just the cmd stream to get a stable per-packet opcode listing:

```bash
cargo run -p aero-gpu-trace-replay -- decode-cmd-stream cmd.bin
```

Tip: pass `--strict` to fail on unknown opcodes instead of printing them as `Unknown`:

```bash
cargo run -p aero-gpu-trace-replay -- decode-cmd-stream --strict cmd.bin
```

### Finding device Hardware IDs

If you are manually binding a driver (or filing a bug), the **Hardware IDs** are the most useful identifier.

1. Open **Device Manager**.
2. Right-click the device → **Properties**.
3. Open the **Details** tab.
4. Select **Hardware Ids**.

You can use these IDs to:

- confirm the VM is presenting the device you think it is,
- verify you are installing the correct driver package (especially x86 vs x64 and device class),
- search `setupapi.dev.log` to see why a driver did (or didn’t) bind.

### Capturing BSOD stop codes

If Windows blue-screens and immediately reboots, you lose the most important clue (the stop code).

To force the BSOD to stay on screen:

1. Reboot.
2. Press **F8** before the Windows logo appears.
3. Select **Disable automatic restart on system failure**.
4. Reboot again and reproduce the failure; note the stop code (for example `0x0000007B`).

### Safe rollback path (storage boot failure)

If Windows fails to boot after switching the system disk from **AHCI → virtio-blk**:

1. Power off the VM.
2. Switch the disk controller back to **AHCI**.
3. Boot Windows (it should boot again).
4. Re-run `setup.cmd` as Administrator and reboot once on AHCI.
5. Try switching to virtio-blk again.

Why this works: Windows can only boot from a storage controller if its driver is installed and configured as boot-critical. Going back to AHCI restores the known-good boot path so you can fix the driver configuration from inside Windows.

If you ran `setup.cmd /skipstorage` (check for `C:\AeroGuestTools\storage-preseed.skipped.txt`), storage pre-seeding was intentionally skipped. In that case, do **not** switch the boot disk to virtio-blk until you re-run `setup.cmd` **without** `/skipstorage` using Guest Tools media that includes the virtio-blk storage driver.

Tip: in `report.txt`, check:

- **virtio-blk Storage Service**: should show the configured storage service with `Start=0 (BOOT_START)`
- **virtio-blk Boot Critical Registry**: should show no missing/mismatched `CriticalDeviceDatabase` keys

### Issue: Device Manager Code 52 (signature and trust failures)

**Symptom**

- Device Manager shows a yellow warning icon and:
  - `Windows cannot verify the digital signature for the drivers required for this device. (Code 52)`

**Common causes**

- **Windows 7 x64** is not in **Test Mode** but the drivers are test-signed.
- The Aero driver signing certificate was not installed into the correct certificate stores.
- Windows 7 is missing **KB3033929**, so it cannot validate **SHA-256** signatures.
- You installed the wrong-architecture driver package (x86 vs x64).
  - Windows 7 **x86**: drivers can install with warnings, but you can still end up with Code 52 if the package is malformed or not trusted as expected.
- The guest clock is incorrect, so certificate validity checks fail.

#### Fix steps

1. **Confirm signature mode (x64):**
   - Run `verify.cmd` and check:
     - `signing_policy` (from the Guest Tools `manifest.json`)
     - **Signature Mode (BCDEdit)** (`testsigning` / `nointegritychecks`)
   - General guidance:
     - If `signing_policy=test`: ensure `testsigning` is **on**.
     - If `signing_policy=production` (WHQL/prod-signed drivers): ensure `testsigning` is **off**.
       - `verify.cmd` will warn if production builds are running in Test Mode.
   - Open an elevated Command Prompt and run:
      - `bcdedit /enum {current}`
   - Look for `testsigning Yes`.
   - If needed, enable or disable it:
      - `bcdedit /set {current} testsigning on`
      - or: `bcdedit /set {current} testsigning off`
      - Reboot.

2. **Confirm the driver signing certificate is installed (recommended for test-signed/custom-signed drivers):**
    - Run `certlm.msc` (Local Computer certificate manager).
    - Check:
      - **Trusted Root Certification Authorities → Certificates**
      - **Trusted Publishers → Certificates**
    - If the certificate is missing, re-run `setup.cmd` as Administrator.
    - Note: If you are using WHQL/production-signed drivers and your Guest Tools media has `signing_policy=production` (or `none`), the media may not ship any `certs\*.cer/*.crt/*.p7b`, and installing a custom certificate is typically unnecessary.

3. **Check KB3033929 (SHA-256 support):**
   - See the KB3033929 section below.

4. **Reinstall the driver:**
   - Re-run `setup.cmd` as Administrator.
   - Or in Device Manager:
     - Right-click the device → **Update Driver Software…**
     - Choose **Browse my computer for driver software**
     - Browse to your Guest Tools driver folder

5. **Confirm the driver package is staged in the driver store (optional but useful):**
   - In an elevated Command Prompt:
     - `pnputil -e`
   - Look for the published name (`oemXX.inf`) associated with the Aero/virtio devices.
   - If you re-run `setup.cmd`, it should stage any missing packages automatically.

#### One-time bypass (not recommended as the primary path)

On Windows 7 x64 you can sometimes boot once with driver signature enforcement disabled:

1. Reboot.
2. Press **F8** before Windows starts.
3. Select **Disable Driver Signature Enforcement**.

This only affects that one boot. For a repeatable setup, prefer installing properly signed/test-signed drivers and configuring test signing as required.

### Issue: Catalog hash mismatch (hash not present in specified catalog file)

**Symptom**

During driver installation (or on boot), Windows reports an error like:

- `The hash for the file is not present in the specified catalog file. The file is likely corrupt or the victim of tampering.`

**Common causes**

- The Guest Tools media is corrupted or incomplete.
- The `.cat` file does not match the `.sys`/`.inf` (wrong driver set or mixed versions).
- Signature validation is failing (for example: incorrect system time, missing KB3033929 for SHA-256).

**Fix**

1. Verify the guest clock/date/time is correct.
2. Ensure KB3033929 is installed if your drivers are SHA-256-signed.
3. Replace the Guest Tools ISO with a fresh copy (don’t mix driver folders across versions).
4. (Optional) Validate the new media before installing:
   - `setup.cmd /check /verify-media`
5. Re-run `setup.cmd` as Administrator (or use the manual install fallback).

### Issue: Guest Tools media integrity check fails (manifest hash mismatch)

This issue is specific to the Guest Tools diagnostics output (it’s detected by `verify.cmd`), but it usually causes driver install failures that look like catalog/signature problems.

**Symptom**

- `verify.cmd` reports **FAIL** in **Guest Tools Media Integrity (manifest.json)** and lists:
  - missing files, and/or
  - SHA-256 hash mismatches for files on the Guest Tools media.

**Common causes**

- The ISO/zip was corrupted in transit.
- Only part of the ISO contents were copied/extracted.
- Files from two different Guest Tools releases were mixed together (for example, overwriting `drivers\` but keeping an older `manifest.json`).

**Fix**

1. Replace the Guest Tools ISO/zip with a fresh copy.
2. Ensure you copy/extract **the entire media root** (including `drivers\`, `config\`, and `certs\` when present/required by `manifest.json` `signing_policy`).
3. (Optional) Validate the new media before installing:
   - `setup.cmd /check /verify-media`
4. Re-run `setup.cmd` as Administrator after replacing the media.

### Issue: Missing KB3033929 (SHA-256 signature support)

Windows 7 needs KB3033929 to validate many SHA-256 signatures. Without it, drivers that are correctly signed may still appear “unsigned”.

**How to check if it’s installed**

- Control Panel → Programs and Features → View installed updates → search for `KB3033929`
- Or in an elevated Command Prompt:
  - `wmic qfe | find "3033929"`

**How to fix**

1. Download the correct KB3033929 `.msu` for your architecture on a host machine:
   - Windows 7 x86 → x86 update
   - Windows 7 x64 → x64 update
2. Copy the `.msu` into the VM (ISO, network, or shared folder).
3. Run the `.msu` inside the VM and reboot.

#### Related SHA-2 updates (sometimes required)

Depending on how your driver packages and certificates are signed, stock Windows 7 SP1 may also require additional SHA-2 updates (for example **KB4474419**).

You can check for these updates with:

- `wmic qfe | find "4474419"`
- `wmic qfe | find "4490628"` (common servicing stack prerequisite for installing newer updates)

If KB3033929/KB4474419 fails to install, ensure you are on Windows 7 SP1 and install the required servicing stack updates first.

#### Recommended signing algorithm policy (for compatibility)

If you are producing or selecting driver packages for Windows 7:

- **Best out-of-box compatibility:** SHA-1-signed catalogs (works on a fresh SP1 install).
- **SHA-256-only signing:** requires KB3033929 (and users frequently don’t have it offline).
- **Practical approach:** provide a path that works both ways:
  - Ensure Guest Tools clearly tells users when KB3033929 is required, and/or
  - Provide a SHA-1-signed fallback driver set for offline installs.

### Issue: Storage controller switch gotchas (boot loops, 0x7B)

**Symptom**

- After switching AHCI → virtio-blk, Windows:
  - reboots repeatedly, or
  - BSODs with `0x0000007B INACCESSIBLE_BOOT_DEVICE`

**Why it happens**

Windows is booting from a disk controller whose driver is not installed or not configured as a boot-start driver. This is the most common failure mode when switching storage controllers.

**Fix**

1. Use the **Safe rollback path** (back to AHCI).
2. From the working AHCI boot:
   - Re-run `setup.cmd` as Administrator.
   - Reboot once (still on AHCI) to let Windows finish driver staging.
3. Switch to virtio-blk again.

If you installed Guest Tools with `setup.cmd /skipstorage`, storage pre-seeding was skipped by design. Re-run `setup.cmd` without `/skipstorage` using media that includes the virtio-blk driver before attempting to boot from virtio-blk.

**Tip: change one device class at a time**

Do storage first, then network, then GPU. If you change storage + GPU simultaneously and the guest can’t boot or can’t display, recovery becomes much harder.

#### Advanced: confirm boot-critical virtio-blk pre-seeding

If you can boot on AHCI but consistently get `0x7B` on virtio-blk, check that Guest Tools actually completed the boot-critical storage setup:

1. Review `C:\AeroGuestTools\install.log` and look for a section like “Preparing boot-critical virtio-blk storage plumbing…”.
2. Confirm the virtio-blk storage driver service is configured as boot-start:
   - Registry:
     - `HKLM\SYSTEM\CurrentControlSet\Services\<storage-service>`
   - Expected values:
     - `Start = 0` (BOOT_START)
     - `ImagePath = system32\drivers\<driver>.sys`
   - The exact service name and expected PCI IDs are defined by the Guest Tools media in `config\devices.cmd`.
3. Confirm CriticalDeviceDatabase entries exist for the expected virtio-blk PCI IDs:
   - `HKLM\SYSTEM\CurrentControlSet\Control\CriticalDeviceDatabase\PCI#VEN_....`
   - These keys map the PCI ID to the storage service so Windows can load the driver early enough to mount the boot volume.

If these entries are missing, re-run `setup.cmd` as Administrator and reboot once on AHCI before switching back to virtio-blk.

### Issue: No bootable device or BOOTMGR is missing after switching storage

**Symptom**

- The VM firmware shows an error like:
  - `No bootable device`, or
  - `BOOTMGR is missing`

**Common causes**

- The disk image is not attached after the hardware profile change.
- The VM is trying to boot from the wrong device (for example, CD/DVD with no media).
- The disk/controller change did not actually map the existing system disk to the new controller.

**Fix**

1. Power off the VM.
2. Verify the system disk image is still attached.
3. Verify the VM is configured to boot from the disk.
   - Aero note: in Aero’s BIOS, this corresponds to booting from the first HDD (`DL=0x80`). Ensure
     the host/runtime sets the boot drive accordingly (e.g. `Machine::set_boot_drive(0x80)` then
     `reset()`).
4. If you can’t quickly resolve it, switch back to the known-good **AHCI** storage controller and boot Windows, then retry the switch to virtio-blk.

### Issue: Virtio device not found or Unknown device after switching

**Symptom**

- After switching to virtio-net or virtio-blk, Windows shows:
  - “Unknown device” in Device Manager, or
  - no working network adapter, or
  - the disk/controller isn’t using the expected driver

**Fix**

1. Confirm the VM is actually configured to use the virtio device (on the host side).
2. In Windows:
   - Open Device Manager → Action → Scan for hardware changes.
3. If the device still shows as unknown:
   - Right-click → Update Driver Software…
   - Browse to the Guest Tools driver folder and ensure you’re selecting the correct architecture.
4. If installation is blocked by signatures, resolve Code 52 first.

### Issue: Lost keyboard/mouse after switching to virtio-input

**Symptom**

- After switching input devices to **virtio-input**, Windows boots but you have no working keyboard/mouse.

**Fix**

1. Power off the VM.
2. Switch input back to **PS/2**.
3. Boot Windows.
4. Re-run `setup.cmd` as Administrator (so the virtio-input driver package is staged).
5. Verify the virtio-input PCI device matches the Aero Win7 virtio contract v1 identity:
    - In Device Manager → the virtio-input PCI device → Properties → Details → **Hardware Ids**
    - The list should include a contract v1 **revision-gated** ID (`REV_01`), such as:
      - `PCI\VEN_1AF4&DEV_1052&REV_01`
      - (Windows will also list less-specific variants such as `PCI\VEN_1AF4&DEV_1052`; this is normal.)
    - If the device exposes Aero subsystem IDs, the list will also include more specific variants, for example:
      - `PCI\VEN_1AF4&DEV_1052&SUBSYS_00101AF4&REV_01` *(keyboard)*, or
      - `PCI\VEN_1AF4&DEV_1052&SUBSYS_00111AF4&REV_01` *(mouse)*
      - When those subsystem IDs are present, Windows will prefer the more specific match so the devices show up as
        **Aero VirtIO Keyboard** / **Aero VirtIO Mouse**.
    - Note: the canonical in-tree virtio-input INF (`aero_virtio_input.inf`) includes:
      - subsystem-qualified keyboard/mouse model lines (`SUBSYS_0010` / `SUBSYS_0011`) for distinct Device Manager names, and
      - the strict revision-gated generic fallback model line (no `SUBSYS`): `PCI\VEN_1AF4&DEV_1052&REV_01`.
      If your virtio-input PCI device does **not** expose Aero subsystem IDs, Windows can still bind via the fallback entry
      (Device Manager name: **Aero VirtIO Input Device**).
    - If Windows still does not bind the driver, check:
      - the device reports `REV_01` (not `REV_00`), and
      - the driver package is staged/installed (re-run `setup.cmd`), and
      - signing/trust issues (Code 52 / KB3033929 / correct clock), and
      - you are installing the correct architecture (x86 vs x64).
    - Tablet devices bind via the separate tablet INF (`aero_virtio_tablet.inf`,
      `PCI\VEN_1AF4&DEV_1052&SUBSYS_00121AF4&REV_01`). That match is more specific than the generic fallback, so it wins when
      both packages are installed and it matches. If the tablet INF is not installed, the generic fallback entry in
      `aero_virtio_input.inf` can also bind to tablet devices (but will use the generic device name).
    - Legacy INF basename note: `virtio-input.inf.disabled` is a disabled-by-default **filename-only alias** for
      `aero_virtio_input.inf`. You may locally rename it to `virtio-input.inf` if a workflow/tool expects that basename.
      - Sync policy: from the first section header (`[Version]`) onward it must be strictly byte-for-byte identical to
        `aero_virtio_input.inf` (banner/comments may differ). See `drivers/windows7/virtio-input/scripts/check-inf-alias.py`.
      - Because it is identical, enabling the alias does **not** change HWID matching behavior (and is not required for
        fallback binding).
      - Avoid installing both basenames at the same time (duplicate packages can cause confusing driver store state/selection).
    - If the device reports `REV_00`, the in-tree Aero virtio-input INFs will not bind; ensure your emulator/QEMU config sets
      `x-pci-revision=0x01` (and preferably `disable-legacy=on`).
6. If Device Manager shows signing or driver errors for the input device, resolve them first (Code 52 / Code 28 / Code 10), then switch back to virtio-input.

For the consolidated end-to-end virtio-input validation plan (Rust device model + Win7 driver + web runtime routing), see:

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md)

### Issue: Device Manager Code 28 (drivers not installed)

**Symptom**

- Device Manager shows:
  - `The drivers for this device are not installed. (Code 28)`

**Fix**

1. Run `setup.cmd` as Administrator again (it should stage the missing drivers).
2. Or install the driver manually:
   - Right-click the device → **Update Driver Software…**
   - **Browse my computer for driver software**
   - Point it at your Guest Tools driver folder.

### Issue: Device Manager Code 10 (device cannot start)

**Symptom**

- Device Manager shows:
  - `This device cannot start. (Code 10)`

**Common causes**

- Wrong driver (x86 vs x64, or the wrong device class).
- Driver is present but blocked by signing/trust (sometimes appears as Code 10 or Code 52 depending on the device).
- Incomplete/mismatched driver package (mixed versions).
- Driver bound but refused to start due to a contract/runtime mismatch (for example a virtio device reporting the wrong `REV_..`).

**Fix**

1. Check signature/trust first (Code 52 section, KB3033929, correct clock).
2. Re-run `setup.cmd` as Administrator.
3. If you recently changed multiple VM devices, roll back and switch one device class at a time to isolate the failure.

### Issue: Windows Setup can't see a virtio-blk disk (slipstream installs)

This only applies if you are attempting to install Windows directly onto **virtio-blk** during Windows Setup.

**Symptom**

- Windows Setup shows “Where do you want to install Windows?” but no disks are listed.

**Cause**

- The virtio-blk storage driver is not available in the Windows Setup environment (`boot.wim`).

**Fix**

- Either:
  - Install Windows using baseline **AHCI** first (recommended), then switch to virtio-blk after running Guest Tools, **or**
  - Attach a driver media disk and use **Load Driver** during Windows Setup:
     - Drivers ISO: browse `drivers\...\x86\` or `drivers\...\x64\` as appropriate
     - FAT driver disk (`*-fat.vhd`): browse `x86\` or `x64\` (see [`../areas/windows-drivers.md`](../areas/windows-drivers.md))
     - Then select the storage driver `.inf` and continue installation, **or**
  - Slipstream the virtio-blk driver into `sources\boot.wim` (indexes 1 and 2) and rebuild the ISO.

### Issue: `setup.cmd` fails (won't run)

**Common symptoms**

- Double-clicking does nothing.
- You see “Access is denied” / “The requested operation requires elevation”.
- You see a console window that closes immediately.

**Fix**

1. Run Guest Tools from:
   - the mounted CD/DVD (for example `X:\setup.cmd`), **or**
   - a local copy (recommended: `C:\AeroGuestTools\media\setup.cmd`)
2. Right-click `setup.cmd` → **Run as administrator**.
3. If it still fails, run it from an elevated Command Prompt so you can read the output:
   - Start menu → type `cmd` → right-click **cmd.exe** → Run as administrator
   - `cd /d X:\` (or `cd /d C:\AeroGuestTools\media`)
   - `setup.cmd`
   - Review `C:\AeroGuestTools\install.log` afterwards.
4. If the script is incompatible with your build or you need a fallback, use the manual install steps in the Guest Tools guide:
   - [`../areas/windows-drivers.md`](../areas/windows-drivers.md#if-setupcmd-fails-manual-install-advanced)

### Issue: Black screen after switching to the Aero GPU

**Symptom**

- After switching **VGA → Aero GPU**, Windows appears to boot but the display is blank/black, or you cannot reach a usable desktop.

**Fix / recovery**

1. Power off the VM.
2. Switch graphics back to **VGA** in the VM settings.
3. Boot Windows.
4. Check Device Manager for the Aero GPU device status:
    - If you see Code 52, fix signing/trust first.
    - If you see an unknown device, reinstall drivers (run `setup.cmd` as Administrator).
5. Try switching to Aero GPU again.

If you must keep the Aero GPU selected while recovering, use Safe Mode (below) since it typically avoids loading third-party display drivers.

#### Advanced diagnostics (scanout / cursor state)

If the OS boots far enough that you can run tools (local console preferred; RDP may change the active display path), dump the scanout state:

`aerogpu_dbgctl.exe` is shipped under the AeroGPU driver directory in packaged outputs:

- Guest Tools ISO/zip (often mounted as `X:`):
 - x64: `X:\drivers\amd64\aerogpu\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe`
 - x86: `X:\drivers\x86\aerogpu\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe`
 - Optional top-level tools payload (when present): `X:\tools\aerogpu_dbgctl.exe` (or under `X:\tools\<arch>\aerogpu_dbgctl.exe`)
- CI-staged packages (host-side): `out\packages\aerogpu\x64\tools\win7_dbgctl\bin\aerogpu_dbgctl.exe` (and `...\x86\...`)

Example (Guest Tools ISO/zip often mounted as `X:`; replace `X:` with your actual drive letter):

```bat
cd /d X:\drivers\amd64\aerogpu\tools\win7_dbgctl\bin
aerogpu_dbgctl.exe --status
```

In the commands below, `aerogpu_dbgctl.exe` assumes you are running from the directory containing dbgctl (otherwise replace it with a full path).

- `aerogpu_dbgctl.exe --status`
  - Captures a combined snapshot (device/ABI + fences + ring0 + scanout0 + cursor + vblank + CreateAllocation trace summary).

- `aerogpu_dbgctl.exe --query-scanout`
  - Confirms whether scanout is enabled, the current mode (`width/height/pitch`), and whether a framebuffer GPA is programmed.
  - Useful for diagnosing blank output caused by mode/pitch mismatches or a missing scanout surface address.

- `aerogpu_dbgctl.exe --dump-scanout-bmp C:\\scanout.bmp`
  - Dumps the scanout framebuffer to an uncompressed 32bpp BMP (requires the installed KMD to allow the debug-only `AEROGPU_ESCAPE_OP_READ_GPA` escape; see `drivers/aerogpu/tools/win7_dbgctl/README.md`).
  - Useful when the guest “seems alive” but the screen is blank/corrupted and you need a pixel artifact without relying on host-side capture.

- `aerogpu_dbgctl.exe --dump-scanout-png C:\\scanout.png`
  - Same as `--dump-scanout-bmp`, but writes a PNG (RGBA8).
  - Note: dbgctl’s built-in PNG encoder uses stored (uncompressed) deflate blocks for simplicity, so the PNG may be slightly **larger** than the BMP.

- `aerogpu_dbgctl.exe --query-cursor`
  - Dumps the hardware cursor MMIO state (`CURSOR_*` registers): enable, position/hotspot, size/format/pitch, and the cursor framebuffer GPA.
  - Useful when the desktop is running but the cursor is missing/stuck/off-screen.

- `aerogpu_dbgctl.exe --dump-cursor-bmp C:\\cursor.bmp`
  - Dumps the current cursor image to an uncompressed 32bpp BMP (requires the installed KMD to allow the debug-only `AEROGPU_ESCAPE_OP_READ_GPA` escape; see `drivers/aerogpu/tools/win7_dbgctl/README.md`).
  - Useful for debugging cursor image/pitch/fb_gpa issues without relying on host-side capture.

- `aerogpu_dbgctl.exe --dump-cursor-png C:\\cursor.png`
  - Same as `--dump-cursor-bmp`, but writes a PNG (RGBA8; preserves alpha).

If you have the Win7 guest-side validation suite available, you can also run:

- `drivers\\aerogpu\\tests\\win7\\bin\\scanout_state_sanity.exe`
  - Validates that the KMD cached mode matches the MMIO scanout registers and the desktop resolution (helps catch broken `DxgkDdiCommitVidPn` mode caching).

- `drivers\\aerogpu\\tests\\win7\\bin\\cursor_state_sanity.exe`
  - Moves the cursor, sets a custom cursor shape, and validates cursor MMIO state via `AEROGPU_ESCAPE_OP_QUERY_CURSOR`.
  - Note: this test is only meaningful on a local console session; it will skip under RDP unless `--allow-remote` is passed (in which case it still skips).

#### Alternative recovery options (if the OS boots but the screen is unusable)

- Try the boot menu option:
  - **F8** → **Enable low-resolution video (640x480)**
- Or force VGA/base video mode via BCD (from a working boot, typically while still on VGA):
  - Enable:
    - `bcdedit /set {current} basevideo yes`
    - Reboot and retry with the Aero GPU selected
  - Disable (after recovery):
    - `bcdedit /deletevalue {current} basevideo`

### Issue: Aero theme not available (stuck in basic graphics mode)

**Symptoms**

- Only “Windows 7 Basic” / classic themes are available.
- Resolution options are limited (often 800×600) or color depth is wrong.

**Fix**

1. Confirm the Aero GPU driver is actually loaded:
   - Device Manager → Display adapters should show the Aero GPU device without warnings.
2. Run the Windows Experience Index assessment (often enables Aero):
   - Open an elevated Command Prompt and run: `winsat formal`
   - Reboot.
3. Then select an Aero theme:
   - Desktop right-click → Personalize → pick a theme under **Aero Themes**.

### Issue: 32-bit D3D9 apps fail on Windows 7 x64 (missing WOW64 UMD)

**Symptoms**

- 64-bit D3D apps work (or the desktop is usable), but **32-bit** D3D9 apps fail to start or fail to create a device.
- Common errors include failures from `Direct3DCreate9` / `CreateDevice` in 32-bit apps.

**Why it happens**

On Windows 7 x64, the display driver package must install **both**:

- a 64-bit D3D9 UMD to `C:\Windows\System32\` (despite the name, `System32` is the **64-bit** system directory on x64), and
- a 32-bit (WOW64) D3D9 UMD to `C:\Windows\SysWOW64\` (`SysWOW64` holds the **32-bit** system DLLs on x64).

If the `SysWOW64` UMD is missing, **32-bit apps will not be able to use D3D9** even though 64-bit apps may work.

**Fix**

1. Confirm the expected UMD files exist on the guest:
    - `C:\Windows\System32\aerogpu_d3d9_x64.dll`
    - `C:\Windows\SysWOW64\aerogpu_d3d9.dll`
    - Tip: `verify.cmd` reports this under **AeroGPU D3D9 UMD DLL placement**.
2. Run the guest-side D3D validation suite (recommended) to confirm the *runtime* actually loads the correct UMD DLL:
    - `drivers\aerogpu\tests\win7\run_all.cmd --require-umd`
    - Or just the D3D9 test:
      - `drivers\aerogpu\tests\win7\bin\d3d9ex_triangle.exe --require-umd`
    - The test output should include the resolved UMD path. For a 32-bit test binary on a Win7 x64 guest it should be tagged as `(WOW64)` and typically resolve to `C:\Windows\SysWOW64\aerogpu_d3d9.dll`.
3. If the `SysWOW64` DLL is missing, reinstall using the supported AeroGPU Win7 package:
    - `drivers/aerogpu/packaging/win7/README.md`
    - Ensure your build/staging workflow includes the WOW64 UMD in the **x64** package:
      - If you are using CI-produced packages, `out/packages/aerogpu/x64/` should contain both `aerogpu_d3d9_x64.dll` and `aerogpu_d3d9.dll`.
      - If you are staging from a repo-local build, use `drivers\aerogpu\build\stage_packaging_win7.cmd fre x64`.
4. Reboot the guest after reinstalling the display driver.

### Issue: 32-bit D3D11 apps fail on Windows 7 x64 (missing WOW64 D3D10/11 UMD)

**Symptoms**

- 64-bit D3D10/D3D11 apps work, but **32-bit** D3D10/D3D11 apps fail to start or fail to create a device.
- Common failures show up in 32-bit apps calling `D3D10CreateDevice*` / `D3D11CreateDevice*` (often `E_FAIL` / `DXGI_ERROR_UNSUPPORTED`), or the app may crash during device creation if the runtime can’t load the expected UMD.

**Why it happens**

If you install the DX11-capable AeroGPU driver package (`aerogpu_dx11.inf`) on Windows 7 x64, the package must install **both**:

- a 64-bit D3D10/11 UMD to `C:\Windows\System32\`:
  - `C:\Windows\System32\aerogpu_d3d10_x64.dll`
- a 32-bit (WOW64) D3D10/11 UMD to `C:\Windows\SysWOW64\`:
  - `C:\Windows\SysWOW64\aerogpu_d3d10.dll`

The UMD filenames are also registered in the adapter’s registry key:

- `UserModeDriverName = "aerogpu_d3d10_x64.dll"` (native x64)
- `UserModeDriverNameWow = "aerogpu_d3d10.dll"` (WOW64 x86)

If the WOW64 UMD is missing or not registered, **32-bit D3D10/D3D11 apps will not be able to use AeroGPU** even though 64-bit apps may work.

**Fix**

1. Confirm the expected UMD files exist on the guest:
   - `C:\Windows\System32\aerogpu_d3d10_x64.dll`
   - `C:\Windows\SysWOW64\aerogpu_d3d10.dll`
   - Tip: `verify.cmd` reports this under **AeroGPU D3D10/11 UMD DLL placement** (if any D3D10/11 UMD DLLs are detected).
2. Confirm the UMD registry values:
   - From a DX11-capable driver package, run:
     - `drivers\aerogpu\packaging\win7\verify_umd_registration.cmd dx11`
   - This prints and validates `UserModeDriverName` / `UserModeDriverNameWow`.
3. Run the guest-side D3D validation suite (recommended) to confirm the runtime loads the correct UMD DLL:
   - `drivers\aerogpu\tests\win7\run_all.cmd --require-umd`
   - Or just the D3D11 test:
     - `drivers\aerogpu\tests\win7\bin\d3d11_triangle.exe --require-umd`
4. Reboot the guest after reinstalling the display driver.

### Issue: Allocation failures (E_OUTOFMEMORY)

**Symptoms**

- D3D9/D3D10/D3D11 apps fail to create resources (textures, buffers, swapchain backbuffers), often returning:
  - `E_OUTOFMEMORY`
  - `D3DERR_OUTOFVIDEOMEMORY`
- The failures may occur “too early” (for example, after allocating only a few large textures) even though the guest still has free RAM.

**Why it happens**

AeroGPU is a **system-memory-backed** WDDM adapter (no dedicated VRAM). (The device may still expose
BAR1 as a legacy VGA/VBE compatibility aperture.) Even so, the Windows 7 graphics kernel (`dxgkrnl`) enforces
a per-adapter **segment budget** based on what the KMD reports as “non-local” memory.

The AeroGPU Win7 KMD defaults this budget to **512 MB** for bring-up. Some workloads legitimately need a larger budget, otherwise
allocations can fail due to the budget limit rather than actual guest memory exhaustion.

**Fix: increase the segment budget hint (`NonLocalMemorySizeMB`)**

Set the AeroGPU device registry parameter:

- **Key:** `HKR\Parameters\NonLocalMemorySizeMB`
- **Type:** `REG_DWORD`
- **Unit:** MB
- **Default:** 512
- **Clamped:** min 128; max 2048 on x64; max 1024 on x86

Recommended starting points:

- **Win7 x64:** 1024–2048 (depending on guest RAM and workload)
- **Win7 x86:** 256–1024 (larger values are clamped to 1024)

Important: this is a **budget hint** (system-RAM-backed), not dedicated VRAM. It does not “create VRAM”; it only changes what the
driver reports to dxgkrnl. Setting it too high can increase guest RAM consumption and paging pressure under heavy workloads.

#### How to set it (Win7)

1. Find the AeroGPU adapter driver key:
   - Device Manager → Display adapters → AeroGPU → Properties → Details → select **Driver key**.
   - It typically looks like:
     - `HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\000X\`
2. Create/open a `Parameters` subkey and set `NonLocalMemorySizeMB`. Example (replace `000X`):

```bat
reg add "HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\000X\Parameters" ^
  /v NonLocalMemorySizeMB /t REG_DWORD /d 2048 /f
```

3. Reboot the guest (or disable/enable the AeroGPU device) for the new budget to take effect.

Quick validation:

- If you have the guest-side AeroGPU validation suite available, run `drivers\\aerogpu\\tests\\win7\\bin\\segment_budget_sanity.exe`
  to confirm the updated `NonLocalMemorySize` is visible from user mode (it queries `D3DKMTQueryAdapterInfo(GETSEGMENTGROUPSIZE)`
  and prints the segment budget in MiB).
- Re-run `verify.cmd` and check `C:\AeroGuestTools\report.txt` / `report.json` for the AeroGPU `NonLocalMemorySizeMB` value
  (to confirm the override is present and what value is configured).

If you need to revert, delete the value:

```bat
reg delete "HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\000X\Parameters" ^
  /v NonLocalMemorySizeMB /f
```

For the canonical KMD-side behavior and rationale, see: `drivers/aerogpu/kmd/README.md`.

### Safe Mode recovery tips

Safe Mode is useful if the system boots but a driver (commonly display) causes instability.

#### Option A: Use F8 at boot (legacy boot menu)

If your VM can send F8 early enough during boot:

1. Reboot.
2. Press **F8** repeatedly before the Windows logo appears.
3. Choose **Safe Mode**.

#### Option B: Force Safe Mode via `bcdedit` (more reliable)

From a working boot (typically while still on AHCI):

1. Open an elevated Command Prompt.
2. Enable Safe Mode:
   - `bcdedit /set {current} safeboot minimal`
3. Shut down, apply the hardware change (virtio/GPU), and boot.

After you recover, disable Safe Mode:

- `bcdedit /deletevalue {current} safeboot`

#### If you got “stuck in Safe Mode”

If you set `safeboot minimal` and forget to remove it, Windows will continue to boot into Safe Mode every time.

Fix (from an elevated Command Prompt):

- `bcdedit /deletevalue {current} safeboot`
- Reboot

#### Other useful F8 boot options (Windows 7)

- **Last Known Good Configuration (advanced)**: rolls back to the last driver/service configuration that successfully reached the logon desktop.
- **Enable Boot Logging**: writes `C:\Windows\ntbtlog.txt`, which can help identify which driver loads last before a hang/boot failure.

### Issue: Test Mode watermark on the desktop (x64)

If test signing is enabled, Windows 7 x64 shows a “Test Mode” watermark. This is expected if you are using test-signed drivers.

Only disable test signing if you are sure you have production-signed drivers installed and loading (for example, `verify.cmd` reports `signing_policy=production`):

- Disable:
  - `bcdedit /set {current} testsigning off`
  - Reboot

If you disable it too early, the drivers may stop loading and devices may fall back to “unknown” or Code 52.

---
