# Repository layout

**Decision (0001).** One Rust workspace for the emulator core and one Node workspace for everything TypeScript, in a single repository; predecessors are marked legacy rather than left to look current.

## The rule this layout serves

**One workspace per language, and as few languages as fit.** Every toolchain
taxes every future reader, gate and environment, so the target is Rust and
TypeScript plus one sealed C/C++ driver domain that
[cannot be avoided](0017-guest-driver-language.md). Within each language there is
exactly one workspace, one lockfile, and one dependency graph — no standalone
lockfiles, no per-directory tool kingdoms. Choose what a well-run project would
choose today, pinned exactly: no experimental machinery and no dependency without
a job.

## Context

Aero is split between:

- A performance-critical emulator core (Rust compiled to WebAssembly).
- A browser “host” application (TypeScript/HTML/CSS) that provides UI, wiring to browser APIs, workers, and deployment tooling.

We need a repo layout that:

- Keeps Rust crates modular and testable (workspace ergonomics).
- Keeps the browser app experience modern (fast dev server, HMR, bundling, worker support).
- Makes it straightforward to ship multiple WebAssembly build variants (threaded vs single-threaded).

## Decision

Adopt a **Rust workspace at the repository root** with a **Vite application under `apps/web/`** for the browser host:

- Root `Cargo.toml` declares `[workspace]` members for every Rust crate, with
  `[workspace.package]` and `[workspace.dependencies]` shared across them.
  `resolver = "3"` is deliberate and MSRV-aware. There is no `[workspace.lints]`
  table — shared lint configuration is a gap, not a feature.
- Rust crates live under `crates/` (the emulator core) and `tools/` (host-side
  CLIs), and produce WebAssembly artifacts the host consumes. `fuzz/` is a
  separate workspace only because cargo-fuzz requires it.
- The **canonical browser host** is `apps/web/`, the single Node workspace member
  that owns the shell, the runtime modules, the workers and the WASM build
  tooling.

This makes Rust builds and web builds first-class while still living in one
repository.

### The host tree was two trees, and is now one

This decision originally placed the canonical host at the repository root
(`index.html` + `src/`) and treated the separate `web/` tree as a subordinate
module directory.
That split turned out to be two partial implementations rather than an
application and its library, and both have since merged into `apps/web/`. The
merge is recorded in [what remains open](../state/repo-state-and-structure.md)
along with what it uncovered; the layout above is the current one.

What remains in `apps/web/` is two *shells* over one module library —
`index.html`, which continuous verification drives, and `bringup.html` for manual
Windows 7 bring-up. That is a much smaller thing than two implementations, and
the second goes when the first can do that job.

## Alternatives considered

1. **Single-package Rust repo with inlined web assets**
   - Pros: fewer moving parts.
   - Cons: poor frontend DX; difficult worker setup; awkward asset pipeline.

2. **Separate repositories (Rust core repo + web host repo)**
   - Pros: strong separation of concerns.
   - Cons: version skew risk; harder cross-cutting refactors; more CI complexity.

3. **Non-Vite tooling (Trunk / webpack / bespoke scripts)**
   - Pros: can work.
   - Cons: Vite has the best combination of worker ergonomics, speed, and mainstream familiarity.

## Consequences

- Contributors can work on Rust and the web host independently, but still in one repo.
- CI and developer tooling run the browser host from the repo root (`pnpm run dev`, `just dev`).
- WebAssembly artifacts become explicit build outputs that the repo-root app depends on (`apps/web/src/wasm/pkg-*`), which simplifies packaging and makes build variants feasible.
