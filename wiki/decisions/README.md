# Decisions

Standing architectural decisions. Each page records what was decided, the
reasoning and evidence behind it, and — as importantly — what it rules out.

A decision here is binding until a later decision supersedes it. Superseding
means writing the new decision and marking the old one as superseded from it;
it never means quietly editing the old one. A decision that no longer matches
the tree is a defect in one of the two.

Decisions are referred to by title, in words. The numbers are stable aliases for
cross-referencing and file ordering, not names — the house rule against coded
identifiers applies here too.

| Decision | Summary |
|---|---|
| [Repository layout](0001-repo-layout.md) | One Rust workspace plus a repo-root Vite application; the legacy stack is marked as such |
| [Cross-origin isolation](0002-cross-origin-isolation.md) | Serve under COOP/COEP so threads and shared memory are available at all |
| [Shared memory layout](0003-shared-memory-layout.md) | Several shared buffers rather than one, because wasm32 caps linear memory at 4 GiB |
| [WebAssembly build variants](0004-wasm-build-variants.md) | Build threaded and single-threaded variants; select at runtime |
| [AeroGPU PCI IDs and ABI](0005-aerogpu-pci-ids-and-abi.md) | `A3A0:0001` is the canonical identity, with a deprecation path for the bring-up ABI |
| [Node monorepo tooling](0006-node-monorepo-tooling.md) | One pnpm workspace, one lockfile, one workspace declaration |
| [Rust crate naming](0007-rust-crate-naming.md) | Naming conventions for workspace packages |
| [Canonical VM core](0008-canonical-vm-core.md) | `aero-machine` is the VM core crate |
| [Rust toolchain policy](0009-rust-toolchain-policy.md) | A pinned stable toolchain, with a pinned nightly only where threaded WebAssembly requires it |
| [Canonical audio stack](0010-canonical-audio-stack.md) | `aero-audio` plus `aero-virtio`; the legacy emulator audio path is gated off |
| [Cargo.lock policy](0012-cargo-lock-policy.md) | The lockfile is committed and builds are `--locked` |
| [Networking via an L2 tunnel](0013-networking-l2-tunnel.md) | Tunnel raw Ethernet to an unprivileged proxy rather than running a TCP/IP stack in the browser |
| [Canonical machine stack](0014-canonical-machine-stack.md) | `aero-machine` is the machine and VM stack |
| [Canonical USB stack](0015-canonical-usb-stack.md) | `crates/aero-usb` with `apps/web/src/usb/*` is the browser USB stack |
| [Windows 7 virtio driver naming](0016-win7-virtio-driver-naming.md) | Naming and layout for the guest virtio drivers |
| [Guest driver language](0017-guest-driver-language.md) | Windows guest drivers stay C/C++; Rust cannot reach the Windows 7 target |
| [Host language](0018-host-language.md) | A TypeScript shell over Rust workers; a full-Rust host was evaluated and rejected |

There is no decision numbered 0011; the number was never assigned.
