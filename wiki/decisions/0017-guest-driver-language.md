# Guest driver language

**Decision (0017).** Every Windows guest driver is written in C/C++. Rust is
rejected for the guest driver domain for as long as Windows 7 is the target.

What is actually in the tree: most drivers are **WDM** — virtio-blk, virtio-net,
virtio-snd and the AeroGPU kernel driver all declare `DriverType=WDM`. Only
virtio-input is **KMDF**, and it pins **KMDF 1.9**, not a later version. Kernel
projects build with the `WindowsKernelModeDriver10.0` toolset; the user-mode
drivers use `v143`.

## Context

Aero ships guest drivers into a Windows 7 SP1 guest: four virtio drivers and the
AeroGPU WDDM 1.1 display driver. The rest of the project is deliberately Rust and
TypeScript and nothing else, so a C/C++ domain is a real cost — it is a third
toolchain, Windows-only, and it cannot share code or tests with the emulator
core. "Windows has Rust driver APIs now" is a true statement in 2026, and it is
worth being precise about why it does not apply here.

## The build toolchain is not a preference

Targeting Windows 7 with a current kit is the constraint that shapes this
domain: 32-bit kernel-mode support was suspended from WDK 10.0.22000, and
Microsoft's guidance is that recent kits drop Windows 7 targeting.

**The repository does not currently match that guidance, and the discrepancy is
unresolved.** `drivers/build/install-wdk.ps1` pins
`-PreferredWdkKitVersion '10.0.22621.0'` and fetches it through winget at
provision time — there is no archived installer in the tree — while the drivers
declare `TargetVersion=Windows7`. Either the kit still targets Windows 7 in
practice, or this has never been proven, and it cannot be settled from here
because **no driver has ever been compiled**: the kit is Windows-only and the
work happens on headless Linux. Treat the exact kit version as
**[unverified]** until someone builds.

Sources: [Microsoft, building drivers for previous OS releases](https://techcommunity.microsoft.com/blog/windowsdriverdev/building-drivers-for-previous-os-releases-using-the-latest-windows-driver-kit-wd/4374910),
[WDK downloads](https://learn.microsoft.com/en-us/windows-hardware/drivers/other-wdk-downloads).

## Why Rust does not reach this target

Microsoft's `windows-drivers-rs` supports x64 and ARM64, Windows 10 2004 and
later, for WDM and KMDF-1.33-class kernel drivers — and even there the crates are
labelled not yet recommended for production. Against Aero's actual deliverable
(Windows 7 SP1, x86 *and* x64, KMDF 1.9, including a display driver) it fails on
several independent grounds, any one of which is disqualifying. Verified against
`main`:

**Kernel mode — hard-blocked.**

- No `_NT_TARGET_VERSION` support ([issue #149](https://github.com/microsoft/windows-drivers-rs/issues/149), open since May 2024).
  `wdk-build` always links the latest SDK and `wdk-alloc` uses `ExAllocatePool2`,
  which is Windows 10 2004+. The practical floor is therefore about Win10 2004.
- 32-bit x86 is not supported: `CpuArchitecture` is `Amd64 | Arm64` only
  ([issue #254](https://github.com/microsoft/windows-drivers-rs/issues/254), stalled).
- **No dxgkrnl bindings exist anywhere.** `wdk-sys` has no Display subset;
  windows-rs carries `D3DKMT*` thunks but zero `DxgkDdi*` or `DXGKRNL_INTERFACE`.
  No Rust WDDM driver has ever been published for any version of Windows.
- KMDF 1.33 is the tested default. Older library versions are generatable in
  principle but untested, with no known driver loading one
  ([issue #42](https://github.com/microsoft/windows-drivers-rs/issues/42)) — and
  this project needs 1.9.
- Hand-rolled Rust KMDF without the framework exists only as x64 hobby projects
  using `/KERNEL` link hacks; i686 kernel Rust is undocumented territory.

**User mode — soft-blocked, and the part worth watching.**

- Rust 1.78 raised all main `*-pc-windows-*` targets to a Windows 10 floor,
  including produced binaries ([announcement](https://blog.rust-lang.org/2024/02/26/Windows-7/)).
  Windows 7 survives only as `x86_64-win7-windows-msvc` and `i686-win7-windows-msvc`,
  both still **Tier 3**: nightly-only `-Z build-std`, no rustup builds, a single
  volunteer maintainer ([platform support](https://doc.rust-lang.org/nightly/rustc/platform-support/win7-windows-msvc.html)).
- windows-rs has **zero D3D user-mode DDI coverage** — verified by grepping the
  `windows` 0.62.2 crate: `D3DDDI_DEVICEFUNCS`, `D3D10/11DDI_DEVICEFUNCS`, the
  adapter functions and every `*DDIARG_*` are absent from the metadata. Bindings
  would be hand-written from WDK headers.
- The D3D10/11 UMD DDI is function-table-based rather than COM, so it is
  `#[repr(C)]`/`__stdcall`-friendly: a Rust UMD is *tedious but sound*, not
  blocked. There is, however, no precedent for any Rust graphics UMD, and an x64
  Windows install needs **both** x64 and x86 (WoW64) UMD builds — so the Tier 3
  32-bit toolchain problem applies even on x64-only installs.

## Consequences

- `drivers/` is a sealed C/C++ domain with its own build glue in PowerShell
  (`drivers/build/`), because WDK, MSBuild, `makecat` and `inf2cat` are all
  Windows-native. PowerShell is not a general scripting language here, but it is
  not confined to `drivers/` either — a handful of Windows-only helpers under
  `scripts/` and `tools/` use it for media patching, slipstreaming and
  certificate work, for the same reason.
- The guest driver stack cannot share types with the emulator core. The ABI is
  held together instead by the normative specs and by cross-language golden
  vectors — see [the AeroGPU device ABI](../specs/aerogpu-device-abi.md) and
  [the virtio driver contract](../specs/windows7-virtio-driver-contract.md).
- The KMD↔UMD private channel is language-agnostic by design, so a hybrid C
  kernel driver with a Rust user-mode driver stays available as a future path —
  for Windows 10+ SKUs, never for Windows 7.

## Revisit conditions

Reopen this decision only if **all three** of the following hold, or if the
project ever targets a Windows 10+ guest:

1. `_NT_TARGET_VERSION` support and x86 both land in `windows-drivers-rs`;
2. the Windows 7 Rust targets reach Tier 2;
3. someone ships a precedent Rust graphics user-mode driver.

Until then, a proposal to write a guest driver in Rust should be answered with
this page rather than re-litigated.
