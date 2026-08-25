# Host language

**Decision (0018).** The browser host is TypeScript over Rust workers. A
full-Rust host built on a WebAssembly UI framework — Leptos, Dioxus or Yew — was
evaluated in depth and rejected.

## Context

Aero's emulator core is Rust compiled to WebAssembly. It is a fair question why
the host around it is not also Rust: one language, shared types, no marshalling
at the boundary. The answer is specific to this application's shape, not a
general judgement about Rust on the web, so it is worth recording the reasoning
rather than the conclusion alone.

## Why a Rust UI framework buys nothing here

- **No framework has a WebGPU story.** Every one of them drops to raw `web-sys`
  and hands a canvas to `wgpu`. For a canvas-centric application the framework
  would therefore buy only the thin strip of DOM around the canvas — which is the
  part that was never the problem ([Leptos discussion #2245](https://github.com/leptos-rs/leptos/discussions/2245),
  [Dioxus issue #3725](https://github.com/DioxusLabs/dioxus/issues/3725)).
- **They assume a single-threaded main-thread module.** Aero's shape — shared
  memory, manually spawned worker WebAssembly modules, `-Z build-std` on nightly
  — is precisely where their risk concentrates, and is off their tested path
  ([discussion](https://users.rust-lang.org/t/wasm32-unknown-unknown-threads-wasm-bindgen/134010)).
- **"All Rust" was never actually attainable.** `AudioWorklet` cannot be
  expressed through `web-sys`; at least one JavaScript shim file is unavoidable
  regardless ([wasm-pack#689](https://github.com/rustwasm/wasm-pack/issues/689)).
- **Their value-add is irrelevant to this application.** Server-side rendering,
  hydration, islands, mobile and fullstack support are the features these
  frameworks compete on in 2026, and an emulator shell uses none of them.
  Debugging remains materially worse than TypeScript, and all three are still
  0.x with real bus-factor and roadmap risk. (Circulating claims that any of them
  reached 1.0 are content-farm fabrications — checked against docs.rs and GitHub
  and found false.)

The community verdict for canvas-centric, worker-heavy, thin-UI applications is
a TypeScript shell over Rust workers, which is what Aero already is. The decision
here is as much to *stop revisiting* this as to choose it.

## Consequences

- The browser host in `apps/web/` is TypeScript, and the boundary between it and
  the Rust core is the worker IPC protocol rather than a shared type system. That
  protocol is normative and specified in
  [the worker IPC protocol](../specs/worker-ipc-protocol.md); it is what pays for
  not having shared types.
- The browser-facing services — the gateway and the development relay — are
  TypeScript on Node, for the same consolidation reason. This is **not** a rule
  that host-side services must be TypeScript: `aero-l2-proxy` and
  `aero-storage-server` are Rust and stay Rust, because they sit next to the
  emulator's own types rather than next to the browser.
- Product UI stays DOM and CSS.

## The open option, if more Rust in the host is ever wanted

The high-value, low-risk move is **egui on wgpu for the debug and HUD layer** —
immediate-mode, rendered onto the wgpu surface that already exists via
`egui-wgpu`, snapshot-testable, and production-proven by Rerun. That is additive
and confined to developer-facing overlays. It is *not* scheduled, and it is not a
route to a Leptos or Dioxus shell rewrite, which remains rejected.
