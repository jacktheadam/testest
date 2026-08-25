# The sprint-era record

> **Historical.** Aero's entire codebase was written in eight days of massively
> parallel agent development, organised as eight workstreams against a
> milestone plan. This page preserves that plan, the workstream briefs, the
> milestone table, and the legal analysis of the era exactly as they stood, so
> the reasoning behind the tree's shape stays recoverable.
>
> None of it is normative. Milestone targets here were aspirations, never
> measurements; the current state is in
> [../state/repo-state-and-structure.md](../state/repo-state-and-structure.md).

## Overview

Aero development is organized into phases with clear milestones. This document outlines the timeline, deliverables, and success criteria for each phase.

---

## Project Timeline

```
┌─────────────────────────────────────────────────────────────────┐
│ Aero Development Timeline │
├─────────────────────────────────────────────────────────────────┤
│ │
│ 2024 │
│ ──── │
│ Q1 ████████ Phase 1: Foundation │
│ Q2 ████████ Phase 2: Core Emulation │
│ Q3 ████████ Phase 3: Graphics & I/O │
│ Q4 ████████ Phase 4: Windows 7 Compatibility │
│ │
│ 2025 │
│ ──── │
│ Q1 ████████ Phase 5: Performance Optimization │
│ Q2 ████████ Phase 6: Production Release │
│ │
└─────────────────────────────────────────────────────────────────┘
```

---

## Phase 1: Foundation (Months 1-3)

### Objectives
- Set up project infrastructure
- Implement basic CPU interpreter
- Create memory subsystem
- Build development tooling

### Deliverables

| Deliverable | Description | Success Criteria |
|-------------|-------------|------------------|
| Project scaffold | Rust/WASM project structure | Builds and runs in browser |
| CPU interpreter | Basic x86-64 decoder + interpreter | Passes instruction tests |
| Memory bus | Physical memory + MMIO routing | Read/write operations work |
| Basic BIOS | POST, memory detection, boot | Boots to boot sector |
| Test harness | Automated testing framework | CI/CD pipeline operational |

### Milestones

```
Week 1-2: Project setup, tooling, CI/CD
Week 3-4: x86-64 decoder implementation
Week 5-6: Basic interpreter (MOV, arithmetic, logic)
Week 7-8: Memory subsystem, paging basics
Week 9-10: Control flow instructions, interrupts
Week 11-12: BIOS POST, boot sector loading
```

### Exit Criteria
- [ ] Can boot FreeDOS from disk image
- [ ] >80% instruction decoder coverage
- [ ] All unit tests passing
- [ ] Documentation complete for Phase 1 components

---

## Phase 2: Core Emulation (Months 4-6)

### Objectives
- Complete CPU instruction set
- Implement JIT compiler (Tier 1)
- Add protected/long mode support
- Basic device models

### Deliverables

| Deliverable | Description | Success Criteria |
|-------------|-------------|------------------|
| Complete decoder | All x86-64 instructions | Decodes Windows 7 binaries |
| Baseline JIT | Tier 1 compilation | 10x faster than interpreter |
| Protected mode | GDT, LDT, segments | Mode switching works |
| Long mode | 64-bit execution | Windows boot loader runs |
| PIC/APIC | Interrupt controllers | IRQ delivery works |
| PIT/HPET | Timer devices | Accurate timing |
| PS/2 | Keyboard/mouse input | Basic input works |
| VGA | Text mode display | Boot messages visible |

### Milestones

```
Week 1-2: SSE/SSE2 instruction implementation
Week 3-4: Protected mode, segmentation
Week 5-6: Long mode, syscall/sysret
Week 7-8: JIT framework, basic block compilation
Week 9-10: Interrupt handling, APIC
Week 11-12: Timer devices, VGA text mode
```

### Exit Criteria
- [ ] Windows 7 boot loader executes
- [ ] Kernel begins loading
- [ ] JIT achieving >100 MIPS
- [ ] Input responsive

---

## Phase 3: Graphics & I/O (Months 7-9)

### Objectives
- WebGPU graphics backend
- DirectX 9 translation layer
- Storage subsystem
- Audio support

### Deliverables

| Deliverable | Description | Success Criteria |
|-------------|-------------|------------------|
| WebGPU renderer | Basic 2D/3D rendering | Framebuffer visible |
| VGA/SVGA | Graphics modes | Windows loading screen |
| DirectX 9 basic | D3D9 state machine | Simple apps render |
| AHCI controller | SATA emulation | Windows sees disk |
| OPFS backend | Large file storage | 40GB+ images supported |
| HD Audio | Basic audio output | System sounds play |
| E1000 | Network adapter | DHCP works |

### Milestones

```
Week 1-2: WebGPU initialization, basic rendering
Week 3-4: VGA graphics modes, SVGA
Week 5-6: AHCI controller, disk I/O
Week 7-8: DirectX 9 shader translation
Week 9-10: HD Audio, basic playback
Week 11-12: Network adapter, TCP/IP
```

### Exit Criteria
- [ ] Windows 7 completes installation
- [ ] Desktop renders (even if slow)
- [ ] Audio output working
- [ ] Network connectivity

---

## Phase 4: Windows 7 Compatibility (Months 10-12)

### Objectives
- Boot to usable desktop
- Run common applications
- DirectX 10/11 support
- USB basics

### Deliverables

| Deliverable | Description | Success Criteria |
|-------------|-------------|------------------|
| Aero glass | DWM compositor | Transparency effects |
| D3D9Ex compatibility | `Direct3DCreate9Ex`/`CreateDeviceEx`/`PresentEx` + present stats | DWM starts and keeps composition enabled |
| DirectX 10 | SM4 shaders | Modern apps render |
| DirectX 11 | Full translation | Games work |
| USB UHCI/EHCI | Basic USB | Keyboard/mouse |
| virtio drivers | Paravirtualized I/O | Major perf boost |
| Multi-core | SMP emulation | 2+ cores visible |

### Milestones

```
Week 1-2: Desktop usability fixes
Week 3-4: DirectX 10 shader translation
Week 5-6: Aero glass effects
Week 7-8: USB controller basics
Week 9-10: DirectX 11 support
Week 11-12: Virtio drivers, multi-core
```

### Exit Criteria
- [ ] Desktop fully usable
- [ ] 80% of top 100 apps work
- [ ] Frame rate ≥15 FPS
- [ ] No major crashes

---

## Phase 5: Performance Optimization (Months 13-15)

### Objectives
- Optimizing JIT (Tier 2)
- Graphics performance
- I/O optimization
- Memory efficiency

### Deliverables

| Deliverable | Description | Success Criteria |
|-------------|-------------|------------------|
| Tier 2 JIT | Optimizing compiler | 500+ MIPS |
| GPU batching | Draw call optimization | ≥30 FPS desktop |
| Storage cache | Sector caching | 100+ MB/s |
| Memory opt | Sparse allocation | <1.5x overhead |
| Profiler | Built-in profiling | Identifies bottlenecks |

### Milestones

```
Week 1-2: Profiling infrastructure
Week 3-4: JIT optimization passes
Week 5-6: GPU rendering optimization
Week 7-8: Storage prefetching, caching
Week 9-10: Memory optimization
Week 11-12: Final tuning, benchmarking
```

### Exit Criteria
- [ ] Boot time <60 seconds
- [ ] Desktop ≥30 FPS
- [ ] Storage ≥50 MB/s
- [ ] Memory overhead <1.5x

---

## Phase 6: Production Release (Months 16-18)

### Objectives
- Polish user experience
- Cross-browser testing
- Documentation
- Community launch

### Deliverables

| Deliverable | Description | Success Criteria |
|-------------|-------------|------------------|
| UI polish | Clean interface | Intuitive UX |
| Browser compat | Chrome/Firefox/Safari | Works everywhere |
| Documentation | User guides, API docs | Complete coverage |
| Website | Project landing page | Professional appearance |
| Demo | Live demo instance | Accessible to all |

### Milestones

```
Week 1-2: UI/UX improvements
Week 3-4: Cross-browser testing
Week 5-6: Documentation completion
Week 7-8: Website and demo
Week 9-10: Beta testing program
Week 11-12: Launch preparation, release
```

### Exit Criteria
- [ ] All tests passing
- [ ] No critical bugs
- [ ] Documentation complete
- [ ] Community feedback positive

---

## Success Metrics by Phase

| Phase | Primary Metric | Target |
|-------|---------------|--------|
| 1 | Instruction test pass rate | ≥95% |
| 2 | Instructions per second | ≥100 MIPS |
| 3 | Windows installation | Completes |
| 4 | Application compatibility | ≥80% |
| 5 | Desktop frame rate | ≥30 FPS |
| 6 | User satisfaction | ≥4/5 stars |

---

## Risk Mitigation

### Technical Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| JIT performance insufficient | Medium | High | Start optimization early |
| WebGPU browser support | Low | High | WebGL2 fallback |
| Memory limits | Medium | Medium | Sparse allocation |
| DirectX complexity | High | Medium | Prioritize D3D9 first |

### Schedule Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Scope creep | High | Medium | Strict scope control |
| Underestimation | Medium | High | Buffer time in each phase |
| Dependency delays | Low | Medium | Minimize external deps |

---

## Resource Requirements

### Team Composition (Estimated)

| Role | Count | Phase Focus |
|------|-------|-------------|
| CPU/Core Engineers | 4-6 | All phases |
| Graphics Engineers | 3-4 | Phase 3-5 |
| I/O Engineers | 2-3 | Phase 2-4 |
| Firmware Engineers | 1-2 | Phase 1-2 |
| Performance Engineers | 2-3 | Phase 5 |
| DevOps/Infra | 1-2 | All phases |

### Infrastructure

- CI/CD pipeline (GitHub Actions)
- Test servers with various browser configurations
- Storage for disk images and test data
- Network proxy servers for testing

---

## Review Checkpoints

### Monthly Reviews

- Progress against milestones
- Technical debt assessment
- Risk review
- Resource allocation

### Phase Gate Reviews

Before each phase transition:
- Exit criteria verification
- Stakeholder sign-off
- Next phase planning
- Lessons learned

---

## Next Steps

- See [Task Breakdown](./sprint-era-record.md) for detailed work items
- See [Testing Strategy](../areas/testing.md) for quality gates

## Task Breakdown & Work Organization

### Overview

This document breaks down Aero development into parallelizable work items. Tasks are organized by functional area, priority, and dependencies. This breakdown can inform how work might be distributed and parallelized.

### Windows Driver Implementation Notes

- [Virtio PCI (Modern) Interrupts on Windows 7 (KMDF)](../areas/windows-drivers.md)

---

### Suggested Work Organization

```
┌─────────────────────────────────────────────────────────────────┐
│ Aero Functional Areas │
├─────────────────────────────────────────────────────────────────┤
│ │
│ CORE │
│ ├── CPU-Decoder │
│ ├── CPU-Interpreter │
│ ├── CPU-JIT │
│ └── Memory │
│ │
│ GRAPHICS │
│ ├── VGA │
│ ├── DirectX-9 │
│ ├── DirectX-10/11 │
│ └── WebGPU-Backend │
│ │
│ I/O │
│ ├── Storage │
│ ├── Network │
│ ├── Audio │
│ └── Input │
│ │
│ FIRMWARE │
│ ├── BIOS │
│ ├── ACPI │
│ └── Device-Models │
│ │
│ PERFORMANCE │
│ ├── Profiling │
│ └── Optimization │
│ │
│ INFRASTRUCTURE │
│ ├── Build │
│ ├── Testing │
│ └── Browser-Compat │
│ │
│ SERVICE / DEPLOYMENT / SECURITY │
│ ├── Disk-Auth │
│ ├── Disk-Gateway │
│ ├── Upload/Import │
│ └── CDN (CloudFront) │
│ │
└─────────────────────────────────────────────────────────────────┘
```

---

### Interface Contracts

Components should adhere to these interfaces for integration:

#### CPU ↔ Memory Interface
 
```rust
// Canonical CPU bus trait used by `aero_cpu_core` Tier-0 + JIT.
//
// See: `aero_cpu_core::mem::CpuBus` (`crates/aero-cpu-core/src/mem.rs`)
//
// Note: this is intentionally abridged; the real trait also includes scalar
// reads/writes, bulk byte operations, `atomic_rmw` (write-intent semantics),
// and `preflight_write_bytes` (used to keep multi-byte writes fault-atomic).
pub trait CpuBus {
 /// Sync paging/MMU view with architectural state (CR0/CR3/CR4/EFER/CPL).
 fn sync(&mut self, state: &aero_cpu_core::state::CpuState) {}
 /// Invalidate a single translation (INVLPG).
 fn invlpg(&mut self, vaddr: u64) {}

 fn read_u8(&mut self, vaddr: u64) -> Result<u8, aero_cpu_core::Exception>;
 fn write_u8(&mut self, vaddr: u64, val: u8) -> Result<(), aero_cpu_core::Exception>;

 fn fetch(&mut self, vaddr: u64, max_len: usize) -> Result<[u8; 15], aero_cpu_core::Exception>;
 fn io_read(&mut self, port: u16, size: u32) -> Result<u64, aero_cpu_core::Exception>;
 fn io_write(
 &mut self,
 port: u16,
 size: u32,
 val: u64,
 ) -> Result<(), aero_cpu_core::Exception>;
}
```
 
#### CPU ↔ Device Interface
 
```rust
// Port IO is modeled directly on the CPU bus via `CpuBus::io_read/io_write`.
//
// Architectural interrupt/exception delivery is handled by `aero_cpu_core::CpuCore`
// (wrapper around `state::CpuState` + `interrupts::PendingEventState` + `time::TimeSource`).
// External interrupts are injected by queuing a vector in `PendingEventState` and delivered
// at instruction boundaries.
```

#### Graphics Interface

```rust
// `aero_gpu_vga::DisplayOutput` (implemented by `aero_gpu_vga::VgaDevice`).
pub trait DisplayOutput {
 fn get_framebuffer(&self) -> &[u32];
 fn get_resolution(&self) -> (u32, u32);
 fn present(&mut self);
}
```

Host-side AeroGPU command parsing/state is implemented as a concrete type:

- `aero_gpu::AeroGpuCommandProcessor` (see `crates/aero-gpu/src/command_processor.rs`)

---

### CORE Tasks

#### CPU-Decoder Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ------------------------------------------- | -------- | ------------ | ---------- |
| CD-001 | Implement prefix parsing (legacy, REX, VEX) | P0 | None | Medium |
| CD-002 | Implement 1-byte opcode table | P0 | CD-001 | High |
| CD-003 | Implement 2-byte opcode table (0F xx) | P0 | CD-001 | High |
| CD-004 | Implement 3-byte opcode tables | P1 | CD-003 | Medium |
| CD-005 | Implement ModR/M + SIB parsing | P0 | None | Medium |
| CD-006 | Implement displacement/immediate parsing | P0 | CD-005 | Low |
| CD-007 | Implement VEX/EVEX prefix handling | P1 | CD-001 | Medium |
| CD-008 | SSE instruction decoding | P0 | CD-003 | High |
| CD-009 | AVX instruction decoding | P2 | CD-007 | High |
| CD-010 | Decoder test suite | P0 | CD-002 | Medium |

#### CPU-Interpreter Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | -------------------------------------------- | -------- | -------------- | ---------- |
| CI-001 | Data movement instructions (MOV, PUSH, POP) | P0 | CD-002 | Medium |
| CI-002 | Arithmetic instructions (ADD, SUB, MUL, DIV) | P0 | CD-002 | High |
| CI-003 | Logical instructions (AND, OR, XOR, NOT) | P0 | CD-002 | Medium |
| CI-004 | Shift/rotate instructions | P0 | CD-002 | Medium |
| CI-005 | Control flow (JMP, CALL, RET, Jcc) | P0 | CD-002 | Medium |
| CI-006 | String instructions (MOVS, STOS, CMPS) | P0 | CD-002 | Medium |
| CI-007 | Bit manipulation (BT, BTS, BSF, BSR) | P1 | CD-002 | Medium |
| CI-008 | System instructions (INT, IRET, SYSCALL) | P0 | CD-002 | High |
| CI-009 | Privileged instructions (MOV CR/DR, LGDT) | P0 | CD-002 | Medium |
| CI-010 | x87 FPU instructions | P1 | CD-002 | Very High |
| CI-011 | SSE instructions (scalar) | P0 | CD-008 | High |
| CI-012 | SSE instructions (packed) | P0 | CD-008 | Very High |
| CI-013 | SSE2 instructions | P0 | CD-008 | High |
| CI-014 | SSE3/SSSE3 instructions | P1 | CI-012 | Medium |
| CI-015 | SSE4.1/4.2 instructions | P1 | CI-014 | Medium |
| CI-016 | Flag computation (lazy evaluation) | P0 | CI-002 | Medium |
| CI-017 | Interpreter test suite | P0 | CI-001..CI-009 | Very High |

#### CPU-JIT Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | --------------------------------------- | -------- | -------------- | ---------- |
| CJ-001 | Basic block detection | P0 | CD-010 | Medium |
| CJ-002 | IR (intermediate representation) design | P0 | None | High |
| CJ-003 | x86 → IR translation | P0 | CJ-001, CJ-002 | Very High |
| CJ-004 | IR → WASM code generation | P0 | CJ-002 | Very High |
| CJ-005 | Code cache management | P0 | CJ-004 | Medium |
| CJ-006 | Execution counter / hot path detection | P0 | None | Medium |
| CJ-007 | Baseline JIT (Tier 1) | P0 | CJ-003, CJ-004 | High |
| CJ-008 | Constant folding optimization | P1 | CJ-002 | Medium |
| CJ-009 | Dead code elimination | P1 | CJ-002 | Medium |
| CJ-010 | Common subexpression elimination | P1 | CJ-002 | Medium |
| CJ-011 | Flag elimination optimization | P1 | CJ-002 | Medium |
| CJ-012 | Register allocation | P1 | CJ-002 | High |
| CJ-013 | Optimizing JIT (Tier 2) | P1 | CJ-008..CJ-012 | Very High |
| CJ-014 | SIMD code generation | P1 | CJ-004 | High |
| CJ-015 | JIT test suite | P0 | CJ-007 | High |
| CJ-016 | Inline RAM loads/stores via JIT TLB fast-path | P0 | CJ-004, MM-012 | High |
| CJ-017 | MMIO/IO exits for JIT memory ops | P0 | CJ-016, MM-003 | Medium |

#### Memory Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ------------------------------------ | -------- | ------------ | ---------- |
| MM-001 | Physical memory allocation | P0 | None | Low |
| MM-002 | Memory bus routing | P0 | MM-001 | Medium |
| MM-003 | MMIO region management | P0 | MM-002 | Medium |
| MM-004 | 32-bit paging | P0 | MM-002 | Medium |
| MM-005 | PAE paging | P0 | MM-004 | Medium |
| MM-006 | 4-level paging (long mode) | P0 | MM-005 | Medium |
| MM-007 | TLB implementation | P0 | MM-006 | High |
| MM-008 | TLB invalidation (INVLPG, CR3 write) | P0 | MM-007 | Medium |
| MM-009 | Page fault handling | P0 | MM-006 | Medium |
| MM-010 | Sparse memory allocation | P1 | MM-001 | Medium |
| MM-011 | Memory test suite | P0 | MM-006 | Medium |
| MM-012 | JIT-visible TLB layout (stable offsets, packed entries) | P0 | MM-007 | Medium |
| MM-013 | `mmu_translate` helper for JIT (page walk + fill TLB) | P0 | MM-006..MM-012 | High |
| MM-014 | MMIO classification/epoch for JIT fast-path safety | P0 | MM-003, MM-012 | Medium |

---

### GRAPHICS Tasks

#### VGA Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | --------------------------- | -------- | ------------ | ---------- |
| VG-001 | VGA register emulation | P0 | None | High |
| VG-002 | Text mode rendering | P0 | VG-001 | Medium |
| VG-003 | Mode 13h (320x200x256) | P0 | VG-001 | Medium |
| VG-004 | Planar graphics modes | P1 | VG-001 | Medium |
| VG-005 | SVGA/VESA modes | P0 | VG-001 | High |
| VG-006 | VGA palette handling | P0 | VG-001 | Low |
| VG-007 | VGA DAC | P0 | VG-006 | Low |
| VG-008 | VGA BIOS interrupt handlers | P0 | VG-002 | Medium |

#### AeroGPU Tasks (Boot VGA + WDDM)

These tasks wire the generic VGA/VBE work into the **AeroGPU virtual PCI device** so Windows 7 can boot and install without a second “legacy VGA” adapter.

See: [AeroGPU Legacy VGA/VBE Compatibility](../areas/graphics.md)

| ID | Task | Priority | Dependencies | Complexity |
| -------------------- | -------------------------------------------------------------------- | -------- | ---------------------------- | ---------- |
| AeroGPU-EMU-DEV-001 | Base AeroGPU PCI device model (BARs, interrupts, MMIO register space) | P0 | DM-007, DM-008 | High |
| AeroGPU-EMU-DEV-002 | VGA legacy decode + VBE LFB modes + scanout handoff to WDDM | P0 | AeroGPU-EMU-DEV-001, VG-005 | High |
| AeroGPU-EMU-DEV-003 | WDDM scanout registers + present path (canvas) | P0 | AeroGPU-EMU-DEV-001 | High |

#### DirectX-9 Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ---------------------------- | -------- | -------------- | ---------- |
| D9-001 | DXBC bytecode parser | P0 | None | High |
| D9-002 | Shader model 2.0 translation | P0 | D9-001 | High |
| D9-003 | Shader model 3.0 translation | P0 | D9-002 | High |
| D9-004 | Vertex shader support | P0 | D9-002 | High |
| D9-005 | Pixel shader support | P0 | D9-002 | High |
| D9-006 | Render state translation | P0 | None | High |
| D9-007 | Texture format translation | P0 | None | Medium |
| D9-008 | Texture sampling | P0 | D9-007 | Medium |
| D9-009 | Render target management | P0 | None | Medium |
| D9-010 | Depth/stencil buffer | P0 | None | Medium |
| D9-011 | Blend state | P0 | None | Medium |
| D9-012 | D3D9 test suite | P0 | D9-001..D9-011 | High |
| D9-013 | D3D9Ex API surface (DWM path) | P0 | D9-009, D9-012 | High |
| D9-014 | Ex present stats + fences + shared surfaces | P0 | D9-013 | High |
| D9-015 | D3D9Ex test app + integration test | P0 | D9-014 | Medium |

#### DirectX-10/11 Tasks
 
| ID | Task | Priority | Dependencies | Complexity |
| ------ | --------------------------------------------------------------------- | -------- | ---------------------- | ---------- |
| D1-001 | Extend DXBC parser for SM4/SM5 (SHEX/SHDR + ISGN/OSGN/RDEF reflection) | P1 | D9-001 | High |
| D1-002 | Shader model 4.0 VS/PS translation (core ALU/flow/sampling) | P1 | D1-001 | High |
| D1-003 | Shader model 5.0 VS/PS translation (typed resources, integer ops) | P1 | D1-002 | High |
| D1-004 | Constant buffers (cbuffers) binding + dynamic update/renaming | P1 | WG-004, D1-001 | Medium |
| D1-005 | Resource views: SRV/RTV/DSV (textures + texture arrays) | P1 | WG-005 | High |
| D1-006 | Input layouts + semantic→location mapping + instancing step rate | P1 | D1-001, WG-002 | High |
| D1-007 | Blend/depth/rasterizer state objects | P1 | WG-002 | High |
| D1-008 | DrawIndexed/baseVertex + instancing + indirect draws | P1 | WG-002, WG-004 | Medium |
| D1-009 | Synchronization primitives (queries/event queries/fences) | P1 | WG-008 | Medium |
| D1-010 | Geometry shader support (lowering/emulation path) | P1 | D1-003 | High |
| D1-011 | Structured buffers + UAV support (storage buffers/textures) | P2 | D1-003, WG-004, WG-005 | High |
| D1-012 | Compute shaders + dispatch | P2 | D1-003, WG-003 | High |
| D1-013 | Tessellation shaders (HS/DS) emulation | P2 | D1-012 | Very High |
| D1-014 | D3D10/11 conformance suite (pixel compare scenes + perf sanity) | P1 | D1-002..D1-009 | High |

#### WebGPU Backend Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ---------------------------- | -------- | ------------ | ---------- |
| WG-001 | WebGPU device initialization | P0 | None | Low |
| WG-002 | Render pipeline creation | P0 | WG-001 | Medium |
| WG-003 | Compute pipeline creation | P1 | WG-001 | Medium |
| WG-004 | Buffer management | P0 | WG-001 | Medium |
| WG-005 | Texture management | P0 | WG-001 | Medium |
| WG-006 | WGSL shader library | P0 | None | High |
| WG-007 | Draw call batching | P1 | WG-002 | Medium |
| WG-008 | Framebuffer presentation | P0 | WG-002 | Medium |
| WG-009 | WebGL2 fallback | P2 | None | Very High |
| WG-010 | Persistent GPU cache (shader translations + reflection, IndexedDB/OPFS, versioned keys, LRU, telemetry, clear API) | P1 | WG-001 | Medium |

##### Implementation notes (WG-001..WG-009)

These are clarifying notes only (no ID/priority changes). See
[16 - Browser GPU Backends (WebGPU-first + WebGL2 Fallback)](../specs/browser-gpu-backends.md)
for the full browser backend design.

- **WG-001 (device initialization):**
 - Implement backend selection as `auto|webgpu|webgl2` with a user override.
 - Run GPU initialization inside the GPU worker using `OffscreenCanvas`.
 - Treat most WebGPU features as optional; negotiate from `adapter.features()`/limits.
- **WG-002 (render pipeline creation):**
 - Prefer pipeline caching keyed by translated state + shader IDs.
 - Keep a small “blit/present” pipeline always available (used by both backends).
- **WG-003 (compute pipeline creation):**
 - Only available in WebGPU mode; define CPU fallbacks for WebGL2 mode.
 - Gate all compute usage behind capability flags.
- **WG-004 (buffer management):**
 - Design for streaming updates (ring buffers/staging) rather than frequent map/unmap.
 - Avoid patterns that rely on storage buffers when targeting WebGL2 fallback.
- **WG-005 (texture management):**
 - Standardize on a small “portable” set of formats (RGBA8 + depth where possible).
 - Treat BCn/DXT as an optional fast-path; fall back to CPU decompression.
- **WG-006 (WGSL shader library):**
 - Keep a WebGL2-compatible shader subset for shared shaders (present/blit/debug).
 - Avoid WGSL features that cannot be lowered to GLSL ES 3.0.
- **WG-007 (draw call batching):**
 - Batch by pipeline/material state to reduce pipeline switches and bind updates.
 - Prefer command-buffer-friendly batching that maps cleanly to WebGPU.
- **WG-008 (framebuffer presentation):**
 - Implement “swapchain + blit”: render into an internal texture, then blit to canvas.
 - Handle resize by reconfiguring the surface and regenerating dependent resources.
- **WG-009 (WebGL2 fallback):**
 - Scope the fallback to framebuffer presentation + minimal render paths.
 - Document and enforce feature gaps (no compute, limited formats, restricted bindings).

---

### I/O Tasks

#### Storage Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | -------------------------- | -------- | ------------ | ---------- |
| ST-001 | IDE controller emulation | P0 | None | High |
| ST-002 | AHCI controller emulation | P0 | None | Very High |
| ST-003 | Disk image abstraction | P0 | None | Medium |
| ST-004 | OPFS backend | P0 | ST-003 | Medium |
| ST-005 | IndexedDB fallback backend | P1 | ST-003 | Medium |
| ST-006 | Sector caching | P1 | ST-003 | Medium |
| ST-007 | Sparse disk format | P1 | ST-003 | Medium |
| ST-008 | CD-ROM/ATAPI emulation | P0 | ST-001 | High |
| ST-009 | Virtio-blk driver (Win7) (see VIO-011) | P1 | VIO-001..VIO-003 | High |
| ST-010 | Storage test suite | P0 | ST-002 | Medium |

Note: IndexedDB-based storage is async and is not currently exposed as a synchronous
`aero_storage::StorageBackend` / `aero_storage::VirtualDisk`. See
[`../areas/storage.md`](../areas/storage.md) and
[`../areas/storage.md`](../areas/storage.md).

#### Network Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ------------------------ | -------- | ------------ | ---------- |
| NT-001 | E1000 NIC emulation | P0 | None | Very High |
| NT-002 | Packet receive/transmit | P0 | NT-001 | Medium |
| NT-003 | User-space network stack | P0 | None | High |
| NT-004 | DHCP client | P0 | NT-003 | Medium |
| NT-005 | DNS resolution (DoH) | P1 | None | Medium |
| NT-006 | WebSocket TCP proxy | P0 | None | Medium |
| NT-007 | WebRTC UDP proxy | P1 | None | High |
| NT-008 | Virtio-net driver (Win7) (see VIO-012) | P1 | VIO-001..VIO-003 | High |
| NT-009 | Network test suite | P0 | NT-001 | Medium |

Implementation references:

- Aero Gateway backend contract (TCP proxy + DoH): [`../specs/gateway-api.md`](../specs/gateway-api.md) (OpenAPI: [`services/gateway/openapi.yaml`](../../services/gateway/openapi.yaml))
- Gateway implementation: `services/gateway`
- UDP relay (WebRTC + WebSocket fallback, v1/v2 datagram framing): `proxy/webrtc-udp-relay` (protocol: [`proxy/webrtc-udp-relay/PROTOCOL.md`](../../proxy/webrtc-udp-relay/PROTOCOL.md))
 - DoS hardening: the relay configures pion/SCTP message-size caps to bound receive-side buffering/allocation before `DataChannel.OnMessage` runs. See `WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES` (SDP hint), `WEBRTC_SCTP_MAX_RECEIVE_BUFFER_BYTES` (hard cap), and `WEBRTC_SESSION_CONNECT_TIMEOUT` (session leak mitigation) in the relay README.
- Local development relay: `services/net-proxy/` (supports `/tcp`, `/tcp-mux`, `/udp`, plus DoH `/dns-query` + `/dns-json`)

#### Audio Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ---------------------------------- | -------- | ------------ | ---------- |
| AU-001 | HD Audio controller emulation | P0 | None | Very High |
| AU-002 | HDA codec emulation | P0 | AU-001 | High |
| AU-003 | Sample format conversion | P0 | None | Medium |
| AU-004 | AudioWorklet integration | P0 | None | Medium |
| AU-005 | Audio buffering/latency management | P0 | AU-004 | Medium |
| AU-006 | AC'97 fallback (legacy; `emulator/legacy-audio`) | P2 | None | High |
| AU-007 | Audio input (microphone) | P2 | AU-004 | Medium |
| AU-008 | Audio test suite | P0 | AU-001 | Medium |

#### Input Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ------------------------ | -------- | -------------- | ---------- |
| IN-001 | PS/2 controller (i8042) | P0 | None | Medium |
| IN-002 | PS/2 keyboard | P0 | IN-001 | Medium |
| IN-003 | PS/2 mouse | P0 | IN-001 | Medium |
| IN-004 | Scancode translation | P0 | None | Medium |
| IN-005 | Browser event capture | P0 | None | Medium |
| IN-006 | Pointer Lock integration | P0 | IN-005 | Low |
| IN-007 | USB HID (keyboard) | P2 | None | Medium |
| IN-008 | USB HID (mouse) | P2 | None | Medium |
| IN-009 | Gamepad support | P2 | None | Medium |
| IN-010 | Input test suite | P0 | IN-001..IN-003 | Medium |
| IN-011 | Virtio-input (keyboard/mouse) device model (device config + event/status queues) | P1 | DM-008, VTP-002 | High |
| IN-012 | Windows 7 virtio-input (keyboard/mouse) KMDF HID minidriver (see VIO-010) | P1 | VIO-001..VIO-003 | Very High |
| IN-013 | HID report descriptor + keyboard/mouse mapping (see VIO-013) | P1 | IN-012 | High |
| IN-014 | Driver packaging/signing + installation docs (see VIO-014) | P1 | IN-012, IN-013 | Medium |
| IN-015 | Browser events → virtio-input events (EV_KEY/EV_REL + SYN) | P1 | IN-005..IN-006, IN-011 | Medium |
| IN-016 | Virtio-input (keyboard/mouse) functional test plan/tooling (see VIO-015) | P1 | IN-011..IN-015 | Medium |

#### Virtio Drivers (Windows 7 guest)

Virtio device-specific drivers (virtio-blk/net/input/etc.) should **not** each reinvent the transport layer. Any Windows 7 virtio driver work presumes shared foundations exist first:

- **Virtio 1.0 PCI modern transport** (capability discovery, BAR mapping, feature negotiation)
- **Virtqueue split-ring** implementation (descriptor/avail/used rings and DMA-safe memory)
- **Interrupt handling** (MSI-X and legacy INTx plumbing)

See `../areas/windows-drivers.md` for an implementation-oriented overview of these building blocks.

| ID | Task | Priority | Dependencies | Complexity |
| ------- | ------------------------------------------ | -------- | ------------------- | ---------- |
| VIO-001 | Virtio-pci modern transport library (shared) | P0 | None | High |
| VIO-002 | Virtqueue split-ring implementation (shared) | P0 | VIO-001 | Very High |
| VIO-003 | MSI-X + legacy interrupt plumbing (shared) | P0 | VIO-001 | High |
| VIO-010 | Virtio-input KMDF HID minidriver (Win7) (keyboard/mouse) (Input lane: IN-012) | P1 | VIO-001..VIO-003 | Very High |
| VIO-011 | Virtio-blk driver (Win7) (Storage lane: ST-009) | P1 | VIO-001..VIO-003 | High |
| VIO-012 | Virtio-net driver (Win7) (Network lane: NT-008) | P1 | VIO-001..VIO-003 | High |
| VIO-013 | Virtio-input HID report descriptor + key mapping (Input lane: IN-013) | P1 | VIO-010 | High |
| VIO-014 | Virtio-input packaging/signing + installation docs (Input lane: IN-014) | P1 | VIO-010, VIO-013 | Medium |
| VIO-015 | Virtio-input functional test plan/tooling (Input lane: IN-016) | P1 | VIO-010..VIO-014 | Medium |

#### Virtio PCI transport (device model / emulator side)

These tasks cover Aero’s virtio devices on the device-model side. The canonical virtio implementation
lives in `crates/aero-virtio` (used by `crates/aero-machine`); the legacy, emulator-local virtio stack
has been removed.

In addition to the modern virtio-pci transport used by Aero’s own Win7 drivers, we should also
support **legacy/transitional virtio-pci** for maximum compatibility with older virtio-win drivers.

See: [`16-virtio-pci-legacy-transitional.md`](../areas/windows-drivers.md)

| ID | Task | Priority | Dependencies | Complexity |
| ------- | ----------------------------------------------------------------------- | -------- | ----------------------- | ---------- |
| VTP-001 | Virtio core (virtqueue, feature negotiation, device status machine) | P0 | DM-007 | High |
| VTP-002 | Virtio PCI modern transport (virtio 1.0+ capabilities + MMIO layout) | P0 | VTP-001, DM-007 | High |
| VTP-003 | Virtio PCI legacy transport (virtio 0.9 I/O port BAR + PFN queues) | P0 | VTP-001, DM-007 | High |
| VTP-004 | Virtio PCI transitional device (expose both legacy + modern transports) | P0 | VTP-002, VTP-003 | Medium |
| VTP-005 | Legacy INTx wiring + ISR read-to-clear semantics | P0 | VTP-003 | Medium |
| VTP-006 | MSI-X support for virtio PCI (recommended for virtio-net performance) | P1 | VTP-002, DM-007 | High |
| VTP-007 | Unit tests: legacy guest flow (feature negotiation, PFN setup, notify) | P0 | VTP-003 | Medium |
| VTP-008 | Config option: disable modern caps (force legacy path for testing) | P1 | VTP-004 | Low |

---

### Windows Guest Tools & Paravirtual Driver Tasks

These tasks cover the **Windows-side packaging** needed for paravirtual devices to work reliably (especially boot-critical storage), plus the cross-team contract that prevents PCI ID drift.

**Source of truth (must stay in sync with emulator + drivers):**

- [`windows7-virtio-driver-contract.md`](../specs/windows7-virtio-driver-contract.md) (`AERO-W7-VIRTIO`, definitive for virtio devices)
- [`windows-device-contract.md`](../specs/windows-device-contract.md) / [`windows-device-contract.json`](../../protocol-vectors/windows-device-contract.json) (binding summary + manifest; must match `AERO-W7-VIRTIO` for virtio)

| ID | Task | Priority | Dependencies | Complexity |
| ------ | -------------------------------------------------------------------- | -------- | ------------ | ---------- |
| GT-001 | Define/maintain the Windows PCI device contract (IDs, BAR usage, INF) | P0 | None | Medium |
| GT-002 | Guest Tools installer consumes `windows-device-contract.json` | P0 | GT-001 | Medium |
| GT-003 | Seed `CriticalDeviceDatabase` for boot-critical storage (virtio-blk) | P0 | GT-002 | High |
| GT-004 | Ensure each driver INF models exactly match contract hardware IDs | P0 | GT-001 | Medium |
| GT-005 | Add emulator CI check: PCI IDs emitted match `windows-device-contract.json` | P1 | GT-001 | Medium |
| GT-006 | Versioning policy: bump contract version on breaking PCI/ABI changes | P1 | GT-001 | Low |

---

### FIRMWARE Tasks

> Note: the canonical BIOS/ACPI/machine stack is now implemented in-tree:
>
> - `crates/firmware::bios` (legacy BIOS HLE, ACPI/SMBIOS publication, ROM stubs)
> - `crates/aero-machine` (canonical integration layer; see the canonical machine stack decision)
>
> The tables below are primarily a historical breakdown and are **not** a reliable “what’s missing”
> list. Prefer looking at failing tests / open integration gaps (SMP, PCI routing hardening,
> snapshot determinism) rather than treating every row as unimplemented work.

#### BIOS Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ---------------------------- | -------- | -------------- | ---------- |
| BI-001 | POST sequence | P0 | None | Medium |
| BI-002 | Memory detection (E820) | P0 | BI-001 | Medium |
| BI-003 | Interrupt vector table setup | P0 | BI-001 | Low |
| BI-004 | BIOS data area setup | P0 | BI-001 | Low |
| BI-005 | INT 10h (video) | P0 | None | Medium |
| BI-006 | INT 13h (disk) | P0 | None | Medium |
| BI-007 | INT 15h (system) | P0 | None | Medium |
| BI-008 | INT 16h (keyboard) | P0 | None | Low |
| BI-009 | Boot device selection | P0 | BI-006 | Low |
| BI-010 | MBR/boot sector loading | P0 | BI-009 | Low |
| BI-011 | BIOS test suite | P0 | BI-001..BI-010 | Medium |

#### ACPI Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | -------------------------------------- | -------- | -------------- | ---------- |
| AC-001 | RSDP/RSDT/XSDT generation | P0 | None | Medium |
| AC-002 | FADT (Fixed ACPI Description Table) | P0 | AC-001 | Medium |
| AC-003 | MADT (Multiple APIC Description Table) | P0 | AC-001 | Medium |
| AC-004 | HPET table | P0 | AC-001 | Low |
| AC-005 | DSDT (AML bytecode) | P1 | AC-001 | High |
| AC-006 | Power management stubs | P1 | AC-002 | Medium |
| AC-007 | ACPI test suite | P0 | AC-001..AC-004 | Medium |

#### Device Models Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ------------------------ | -------- | -------------- | ---------- |
| DM-001 | PIC (8259A) | P0 | None | Medium |
| DM-002 | PIT (8254) | P0 | None | Medium |
| DM-003 | CMOS/RTC | P0 | None | Medium |
| DM-004 | Local APIC | P0 | None | High |
| DM-005 | I/O APIC | P0 | DM-004 | High |
| DM-006 | HPET | P0 | None | Medium |
| DM-007 | PCI configuration space | P0 | None | High |
| DM-008 | PCI device enumeration | P0 | DM-007 | Medium |
| DM-009 | DMA controller (8237) | P1 | None | Medium |
| DM-010 | Serial port (16550) | P2 | None | Medium |
| DM-011 | Device models test suite | P0 | DM-001..DM-006 | Medium |

---

### PERFORMANCE Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ---------------------------- | -------- | -------------- | ---------- |
| the performance HUD and JSON export | Profiling infrastructure | P0 | None | Medium |
| retired-instruction counting | Instruction counter | P0 | None | Low |
| the frame-timing metrics | Frame time tracking | P0 | None | Low |
| the storage benchmark harness | Memory usage tracking | P0 | None | Low |
| hot-path identification | Hot path identification | P1 | the performance HUD and JSON export | Medium |
| the JIT telemetry surface | JIT optimization analysis | P1 | CJ-015, the performance HUD and JSON export | Medium |
| graphics bottleneck analysis | Graphics bottleneck analysis | P1 | the performance HUD and JSON export, WG-001..WG-002 | Medium |
| the guest CPU benchmark suite | Benchmark suite | P0 | None | High |
| the checked-in benchmark baseline | Regression tracking | P0 | the guest CPU benchmark suite | Medium |

the guest CPU benchmark suite specifically includes a **guest CPU instruction throughput** microbenchmark suite (no OS images) with checksum validation, perf export integration, and a Playwright scenario. See: [Guest CPU Instruction Throughput Benchmarks ](../areas/performance.md).

---

### INFRASTRUCTURE Tasks

| ID | Task | Priority | Dependencies | Complexity |
| ------ | ------------------------------- | -------- | ------------ | ---------- |
| IF-001 | Project structure setup | P0 | None | Low |
| IF-002 | Rust/WASM build configuration | P0 | IF-001 | Medium |
| IF-003 | CI/CD pipeline (GitHub Actions) | P0 | IF-002 | Medium |
| IF-004 | Unit test framework | P0 | IF-001 | Low |
| IF-005 | Integration test framework | P0 | IF-004 | Medium |
| IF-006 | Browser test automation | P0 | IF-002 | High |
| IF-007 | Code coverage reporting | P1 | IF-004 | Low |
| IF-008 | Documentation generation | P1 | None | Low |
| IF-009 | Release automation | P1 | IF-003 | Medium |
| IF-010 | Performance regression CI | P1 | the guest CPU benchmark suite | Medium |

---

### SERVICE / DEPLOYMENT / SECURITY Tasks

These tasks cover the hosted-service components needed to support **user-provided disk images** (upload, storage, and streamed access) while meeting legal/security constraints.

| ID | Task | Priority | Dependencies | Complexity |
| ------ | --------------------------------------------------------------------- | -------- | -------------------- | ---------- |
| HS-001 | Disk image lifecycle + access control spec (`docs/17-*`) | P0 | None | Medium |
| HS-002 | Disk image streaming authentication spec (`docs/16-*`) | P0 | HS-001 | Medium |
| HS-003 | `disk-gateway` server (Range GET, auth verification, CORS) | P0 | HS-001, HS-002 | High |
| HS-004 | Upload/import pipeline (ingest, validate, store, attach to lifecycle) | P0 | HS-001 | High |
| HS-005 | CDN profiles (CloudFront) for disk streaming (`docs/deployment/*`) | P1 | HS-002, HS-003 | Medium |
| HS-006 | Hosted-service conformance tests (contract tests for gateway + auth) | P1 | HS-003, HS-004, HS-005 | Medium |
| HS-007 | Browser E2E tests for disk streaming + auth failures | P1 | HS-003, IF-006 | High |

---

### Dependency Graph (Simplified)

```
┌─────────────────────────────────────────────────────────────────┐
│ Task Dependencies │
├─────────────────────────────────────────────────────────────────┤
│ │
│ Phase 1: Foundation │
│ ┌──────────┐ │
│ │ IF-001/2 │──┬──▶ CD-001 ──▶ CD-002..009 │
│ └──────────┘ │ │ │
│ │ ▼ │
│ ├──▶ CI-001..017 │
│ │ │ │
│ │ ▼ │
│ └──▶ MM-001..006 │
│ │
│ Phase 2: Core │
│ ┌──────────┐ │
│ │CD-010 ───┼──▶ CJ-001..007 ──▶ CJ-008..015 │
│ └──────────┘ │
│ │
│ Phase 3: Graphics + I/O (can run in parallel) │
│ ┌──────────┐ ┌──────────┐ ┌──────────┐ │
│ │ VG-001 │ │ ST-001 │ │ NT-001 │ │
│ └────┬─────┘ └────┬─────┘ └────┬─────┘ │
│ │ │ │ │
│ ▼ ▼ ▼ │
│ ┌──────────┐ ┌──────────┐ ┌──────────┐ │
│ │ D9-001 │ │ AU-001 │ │ IN-001/11│ │
│ └────┬─────┘ └────┬─────┘ └────┬─────┘ │
│ │ │ │ │
│ ▼ ▼ ▼ │
│ ┌──────────┐ │
│ │ D1-001 │ │
│ └──────────┘ │
│ │
└─────────────────────────────────────────────────────────────────┘
```

---

### Parallel Execution Lanes

#### Maximum Parallelization

At any given time, these work streams can proceed independently:

**Phase 1 (8+ parallel lanes):**

1. Instruction decoder (CD-001..010)
2. Data movement instructions (CI-001)
3. Arithmetic instructions (CI-002)
4. Logical instructions (CI-003)
5. Memory bus (MM-001..003)
6. Paging (MM-004..006)
7. BIOS POST (BI-001..003)
8. Infrastructure (IF-001..005)

**Phase 2 (6+ parallel lanes):**

1. JIT framework (CJ-001..007)
2. SSE instructions (CI-011..015)
3. System instructions (CI-008..009)
4. TLB (MM-007..009)
5. Device models (DM-001..006)
6. ACPI tables (AC-001..004)

**Phase 3 (10+ parallel lanes):**

1. VGA (VG-001..008)
2. DirectX 9 parser (D9-001..003)
3. DirectX 9 state (D9-006..011)
4. WebGPU backend (WG-001..008)
5. AHCI (ST-001..002)
6. Storage cache (ST-006..007)
7. Network (NT-001..008)
8. Audio (AU-001..005)
9. Input (IN-001..006, IN-011..016)
10. Profiling (the performance HUD and JSON export..005)

---

### Suggested Work Principles

#### Interface-Driven Design

- Assign ownership by interface boundaries
- Clear ownership prevents conflicts
- Use defined interfaces to minimize cross-dependencies
- Mock dependencies early in development

#### Development Approach Suggestions

- **Understand Interface Contract First** - read the trait/interface definition
- **Test-Driven Development** - write tests before implementation
- **Document Edge Cases** - note undocumented behavior
- **Iterative Reviews** - get feedback early rather than waiting until completion

#### Priority Ordering

- P0 tasks before P1 - these are on the critical path
- Unblock other work streams first where possible

---

### Task Status Tracking

#### Status Definitions

| Status | Meaning |
| -------------- | ---------------------------- |
| 🔴 Not Started | Task not yet begun |
| 🟡 In Progress | Currently being worked on |
| 🟢 Complete | Implementation finished |
| ✅ Verified | Tests passing, code reviewed |
| 🔵 Blocked | Waiting on dependency |

#### Progress Template

```
## Week N Progress Report

### Core Team
- CD-001: ✅ Complete
- CD-002: 🟡 In progress (80%)
- CI-001: 🟢 Complete, awaiting review

### Graphics Team
- VG-001: 🟡 In progress (50%)
- D9-001: 🔴 Not started (blocked by CD-010)

### Blockers
- D9-001 blocked on instruction decoder completion
- NT-001 needs interface clarification

### Next Week Goals
- Complete CD-002, CD-003
- Begin CJ-001 (JIT framework)
```

---

### Coordination Points

#### Key Sync Topics

- Task status and blockers
- Dependency resolution
- Priority adjustments

#### Architecture Reviews

- Interface changes may require cross-functional review
- Performance-critical decisions
- Security considerations

---

*This document should be updated as tasks are completed and priorities shift.*

## Legal & Licensing Considerations

### Overview

Building a Windows 7 emulator involves significant legal considerations around intellectual property, software licensing, and distribution.

This document provides background and rationale. For the **repository’s
authoritative policies and templates**, see:

- [`../LEGAL.md`](./purged-material.md)
- [`../CONTRIBUTING.md`](./purged-material.md)
- [`../TRADEMARKS.md`](./purged-material.md)
- [`../DMCA_POLICY.md`](./purged-material.md)
- [`../SECURITY.md`](../history/purged-material.md)
- [`./LICENSE.md`](./sprint-era-record.md) (documentation licensing)

---

### Key Legal Areas

#### Emulator Legality

**General Principle:** Emulation itself is legal. Key precedents:

- **Sony v. Connectix (2000):** Ruled that reverse engineering for compatibility is fair use
- **Sega v. Accolade (1992):** Established that reverse engineering for interoperability is protected

**What We Can Do:**
- ✅ Build an emulator that runs x86/x64 code
- ✅ Implement hardware interfaces based on public specifications
- ✅ Reverse engineer undocumented behavior for compatibility

**What We Cannot Do:**
- ❌ Distribute Microsoft copyrighted code (Windows, BIOS, drivers)
- ❌ Use Microsoft trademarks without permission
- ❌ Circumvent DRM or copy protection

---

#### Windows 7 Licensing

##### End User License Agreement (EULA)

Windows 7 retail/OEM licenses permit:
- Installation on one physical computer
- Use with virtualization software on the licensed machine

**Key Considerations:**

1. **Users Must Supply Their Own License**
 - Aero does not include Windows 7
 - Users must have a valid Windows 7 license
 - License key validation happens within Windows itself

2. **Volume Licensing**
 - Enterprise customers may have VL agreements
 - Some VL agreements allow virtual instances
 - Users responsible for compliance

3. **Extended Security Updates (ESU)**
 - Windows 7 reached end of support January 2020
 - ESU available for enterprise through 2023
 - No longer receiving security updates

##### Distribution Model

```
┌─────────────────────────────────────────────────────────────────┐
│ Legal Distribution Model │
├─────────────────────────────────────────────────────────────────┤
│ │
│ Aero Project Provides: │
│ ┌─────────────────────────────────────────────────────────┐ │
│ │ • Emulator software (open source) │ │
│ │ • Custom BIOS (open source) │ │
│ │ • Documentation │ │
│ │ • Virtio drivers (open source) │ │
│ └─────────────────────────────────────────────────────────┘ │
│ │
│ User Must Provide: │
│ ┌─────────────────────────────────────────────────────────┐ │
│ │ • Windows 7 installation media (ISO) │ │
│ │ • Valid Windows 7 license key │ │
│ │ • Acceptance of Microsoft EULA │ │
│ └─────────────────────────────────────────────────────────┘ │
│ │
└─────────────────────────────────────────────────────────────────┘
```

---

#### BIOS/Firmware

##### Traditional BIOS

- IBM PC BIOS is copyrighted
- We **cannot** use or distribute original BIOS
- We **must** use open-source alternatives

##### Our Approach: Custom BIOS

```rust
// Aero uses a custom, open-source BIOS implementation
// Licensed under MIT/Apache-2.0 dual license

pub struct AeroBios {
 // No copyrighted code
 // Implements required functionality from scratch
 // Based on public specifications
}
```

**Open-Source BIOS Options:**
- SeaBIOS (LGPL v3)
- coreboot (GPL v2)
- Custom implementation (MIT/Apache-2.0)

**Recommendation:** Custom implementation to avoid GPL complications with WASM

---

#### Device Specifications

##### Public Specifications (Safe to Use)

| Component | Specification Source | License |
|-----------|---------------------|---------|
| x86/x64 CPU | Intel/AMD SDMs | Public documentation |
| VGA/SVGA | VESA specifications | Public standard |
| PS/2 | Public documentation | Public standard |
| PCI/PCIe | PCI-SIG specifications | Membership (specs public) |
| USB | USB-IF specifications | Public standard |
| AHCI/SATA | Intel/SATA-IO | Public documentation |
| HD Audio | Intel specification | Public documentation |
| ACPI | UEFI Forum | Public specification |

##### Potentially Problematic

| Component | Issue | Mitigation |
|-----------|-------|------------|
| DirectX | Proprietary API | Implement from public docs, no MS code |
| WDDM | Windows-specific | Implement custom GPU driver |
| NTFS | Patented | Don't implement in emulator (guest OS handles) |

---

#### Patent Considerations

##### x86 Patents

- Most fundamental x86 patents have expired
- AMD64 (x86-64) patents still active but cross-licensed
- Emulation for personal use generally not challenged

##### Video Codec Patents

If implementing video playback:
- H.264/AVC: Patent pool (licensing required for commercial)
- VP8/VP9: Royalty-free (Google)
- AV1: Royalty-free (Alliance for Open Media)

**Recommendation:** Use browser's native codec support via WebCodecs

##### Software Patents in Emulation

- No known blocking patents for x86 emulation
- JIT compilation techniques are well-established
- Memory virtualization techniques are public domain

---

#### Open Source Licensing

##### Aero License Selection

**Recommended: MIT OR Apache-2.0 dual license**

```
SPDX-License-Identifier: MIT OR Apache-2.0

Copyright (c) 2026 Aero Contributors

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software...
```

See ``../LICENSE-MIT``, ``../LICENSE-APACHE``,
and ``../NOTICE``.

**Rationale:**
- Permissive for commercial use
- Compatible with most other licenses
- No patent retaliation clauses (Apache-2.0 provides patent grant)
- No viral/copyleft requirements

##### Dependency Licenses

| Dependency | License | Compatible? |
|------------|---------|-------------|
| Rust stdlib | MIT/Apache-2.0 | ✅ Yes |
| wasm-bindgen | MIT/Apache-2.0 | ✅ Yes |
| WebGPU | W3C | ✅ Yes |
| SeaBIOS | LGPL v3 | ⚠️ Requires care |
| QEMU (reference) | GPL v2 | ❌ Cannot copy code |

---

#### Trademark Considerations

##### What We Cannot Use

- "Windows" trademark
- Microsoft logo
- Windows 7 product imagery
- "Designed for Windows" logos

##### What We Can Say

- "Compatible with Windows 7"
- "Runs Windows 7" (factual statement)
- "x86 PC emulator"

##### Project Naming

- ✅ "Aero" - Generic term, not trademarked in this context
- ❌ "Windows Emulator" - Implies Microsoft affiliation
- ❌ "Win7Emu" - Uses "Win" which could cause confusion

See [`../TRADEMARKS.md`](./purged-material.md) for repo-wide naming/branding
guidelines and suggested disclaimers.

---

#### Distribution Considerations

##### Source Code Distribution

```
Aero Repository Structure:
├── src/ # MIT/Apache-2.0
├── bios/ # MIT/Apache-2.0 (custom)
├── drivers/ # MIT/Apache-2.0 (virtio)
├── docs/ # CC-BY-4.0
└── tests/ # MIT/Apache-2.0
```

##### Binary Distribution
 
 - Pre-built WASM binaries: ✅ OK
 - Including Windows files: ❌ NOT OK
 - Including BIOS ROMs (not ours): ❌ NOT OK
 - Including Microsoft WDK redistributables (e.g. `WdfCoInstaller*.dll`): ⚠️ Only if explicitly opted-in and compliant with the applicable Microsoft redistribution license

##### Website/Service

If hosting Aero as a service:
- Do not provide Windows images
- Require users to upload their own
- Clear ToS requiring valid licenses
- No piracy facilitation
- Disk handling policy (upload privacy, ownership/sharing, persistence): see [Disk Image Lifecycle and Access Control](../areas/storage.md)
- Disk streaming access control (leases/tokens, `Range`, COOP/COEP): see [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md)

---

#### DMCA Considerations

##### Safe Harbor

As a platform provider, maintain DMCA safe harbor:
- Register DMCA agent
- Implement takedown procedures
- Don't have actual knowledge of infringement

See [`../DMCA_POLICY.md`](./purged-material.md) for a takedown/counter-notice
template and repeat-infringer posture.

##### Circumvention Concerns

The DMCA prohibits circumventing "technological protection measures":
- Windows activation: Don't bypass (let Windows handle it)
- Game DRM: Don't specifically circumvent
- Region locks: User responsibility

---

#### International Considerations

##### EU

- Generally permissive of interoperability
- Computer Programs Directive allows reverse engineering
- GDPR compliance if collecting user data

##### Other Jurisdictions

- Japan: Generally permissive of emulation
- Australia: Fair dealing provisions apply
- Check local laws for specific markets

---

### Compliance Checklist

#### Before Launch

- [ ] Legal review of codebase (no copyrighted code)
- [ ] Trademark clearance for project name
- [ ] License headers on all source files
- [ ] NOTICE file with attribution
- [ ] Terms of Service drafted
- [ ] Privacy Policy (if collecting data)
- [ ] DMCA agent registered

#### Ongoing

- [ ] Monitor for legal challenges
- [ ] Respond to takedown requests
- [ ] Update licenses as dependencies change
- [ ] Enforce dependency license/vulnerability policy in CI (e.g. `cargo-deny`, npm advisory scanning)
- [ ] Review user-contributed code

---

### Disclaimers

#### Required Disclaimers

```
Aero is an independent project and is not affiliated with, endorsed by,
or sponsored by Microsoft Corporation. Windows is a registered trademark
of Microsoft Corporation.

Users are responsible for ensuring they have valid licenses for any
software they run within Aero. The Aero project does not provide,
distribute, or facilitate access to Microsoft Windows or any other
copyrighted software.

Aero is provided "AS IS" without warranty of any kind.
```

---

### Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Microsoft takedown | Low | High | Clean-room implementation, no MS code |
| Patent claim | Very Low | High | Use established techniques |
| Trademark complaint | Low | Medium | Clear disclaimers, proper naming |
| DMCA abuse | Medium | Low | Proper safe harbor procedures |
| License violation | Low | Medium | Regular audits, clear policies |

---

### Recommendations

1. **Clean-Room Implementation**
 - Never copy or redistribute Microsoft proprietary source code (Windows, BIOS, etc.)
 - Microsoft driver *samples* may be usable if (and only if) they are under an OSI-permissive license (e.g., the MIT-licensed `Windows-driver-samples` repository) and required notices are preserved
 - See `drivers/windows7/LEGAL.md` for the Windows 7 virtio driver sourcing policy
 - Document all specifications used
 - Keep development logs

2. **Clear Boundaries**
 - Users provide their own Windows
 - No piracy facilitation
 - Educational/preservation focus

3. **Legal Consultation**
 - Consult IP attorney before major releases
 - Have legal review distribution model
 - Budget for potential legal defense

---

### Next Steps

- See [Project Milestones](./sprint-era-record.md) for development timeline
- See [Task Breakdown](./sprint-era-record.md) for implementation tasks

## Documentation License

Unless otherwise noted, the contents of the `docs/` directory are licensed
under the **Creative Commons Attribution 4.0 International** license
(**CC BY 4.0**).

- License summary: https://creativecommons.org/licenses/by/4.0/
- Full license text: https://creativecommons.org/licenses/by/4.0/legalcode

### Attribution

When redistributing or adapting documentation from `docs/`, provide
appropriate attribution (e.g., “Aero Contributors”) and indicate if changes
were made, as required by CC BY 4.0.

### Not software licensing

This license applies to documentation in `docs/` only.

Source code and other software in this repository is dual-licensed under
**MIT OR Apache-2.0** (see `../LICENSE-MIT` and `../LICENSE-APACHE`).

## Workstream Instructions

This directory contains the onboarding/task instructions for each parallel development
workstream. Start with [`AGENTS.md`](./purged-material.md) (operational guidance + interface
contracts), then choose the workstream you are working on:

| Workstream | File | Focus |
|------------|------|-------|
| **A: CPU/JIT** | [`cpu-jit.md`](./sprint-era-record.md) | CPU emulation, decoder, JIT, memory |
| **B: Graphics** | [`graphics.md`](./sprint-era-record.md) | VGA, DirectX 9/10/11, WebGPU |
| **C: Windows Drivers** | [`windows-drivers.md`](./sprint-era-record.md) | AeroGPU, virtio drivers |
| **D: Storage** | [`io-storage.md`](./sprint-era-record.md) | AHCI, NVMe, OPFS, streaming |
| **E: Network** | [`network.md`](./sprint-era-record.md) | E1000, L2 proxy, TCP/UDP |
| **F: USB/Input** | [`usb-input.md`](./sprint-era-record.md) | PS/2, USB HID, keyboard/mouse |
| **G: Audio** | [`audio.md`](./sprint-era-record.md) | HD Audio, AudioWorklet |
| **H: Integration** | [`integration.md`](./sprint-era-record.md) | BIOS, ACPI, PCI, boot |

For cross-workstream planning and a higher-level roadmap, see:

- [`project-history.md`](./sprint-era-record.md)
- [`../overview.md`](../overview.md)

## Workstream A: CPU & JIT

> **⚠️ MANDATORY: Read and follow [`AGENTS.md`](./purged-material.md) in its entirety before starting any work.**
>
> AGENTS.md contains critical operational guidance including:
> - Defensive mindset (assume hostile/misbehaving code)
> - Resource limits and `safe-run.sh` usage
> - Windows 7 test ISO location (`/state/win7.iso`)
> - Interface contracts
> - Technology stack decisions
>
> **Failure to follow AGENTS.md will result in broken builds, OOM kills, and wasted effort.**

---

### Overview

This workstream owns the **CPU emulation engine**: the x86/x86-64 instruction decoder, interpreter (Tier 0), JIT compiler (Tier 1/2), and memory management unit (paging, TLB).

This is the **critical path** for performance. The emulator's speed is dominated by how fast the CPU can execute guest instructions.

---

### Key Crates & Directories

| Crate/Directory | Purpose |
|-----------------|---------|
| `crates/aero-cpu-core/` | CPU state, decoder, interpreter, exception handling |
| `crates/aero-cpu-decoder/` | Instruction decoding (separate from core for modularity) |
| `crates/aero-jit-x86/` | JIT compiler: x86 → IR → WASM |
| `crates/aero-mmu/` | Memory management unit, paging |
| `crates/aero-mem/` | Physical memory, bus routing |
| `crates/aero-x86/` | x86 architecture constants, helpers |

---

### Essential Documentation

**Must read:**

- [`../areas/cpu-and-jit.md`](../areas/cpu-and-jit.md) — CPU emulation design, tiered execution
- [`../areas/memory-and-paging.md`](../areas/memory-and-paging.md) — Paging, TLB, address translation
- [`../areas/performance.md`](../areas/performance.md) — JIT optimization strategies

**Reference:**

- [`../overview.md`](../overview.md) — System architecture
- [`../areas/performance.md`](../areas/performance.md) — CPU benchmarks 

---

### Interface Contract: CpuBus

The CPU communicates with memory and devices through `CpuBus`. This is the canonical interface:

```rust
// See: `crates/aero-cpu-core/src/mem.rs` (`aero_cpu_core::mem::CpuBus`)
//
// Notes:
// - Addresses are *linear* (paging translation is handled by the bus)
// - Operations return `Result` so the CPU can raise architectural faults (#PF, #GP)
// - The real trait also includes scalar reads/writes, bulk byte ops, atomic_rmw,
// and preflight_write_bytes for fault-atomic multi-byte writes
pub trait CpuBus {
 fn sync(&mut self, state: &aero_cpu_core::state::CpuState) {}
 fn invlpg(&mut self, vaddr: u64) {}

 fn read_u8(&mut self, vaddr: u64) -> Result<u8, aero_cpu_core::Exception>;
 fn write_u8(&mut self, vaddr: u64, val: u8) -> Result<(), aero_cpu_core::Exception>;

 fn fetch(&mut self, vaddr: u64, max_len: usize) -> Result<[u8; 15], aero_cpu_core::Exception>;
 fn io_read(&mut self, port: u16, size: u32) -> Result<u64, aero_cpu_core::Exception>;
 fn io_write(&mut self, port: u16, size: u32, val: u64) -> Result<(), aero_cpu_core::Exception>;
}
```

**Do not change this interface without cross-workstream review.** It affects every device model.

---

### Tasks

#### CPU-Decoder Tasks

| ID | Task | Priority | Dependencies | Complexity |
|----|------|----------|--------------|------------|
| CD-001 | Implement prefix parsing (legacy, REX, VEX) | P0 | None | Medium |
| CD-002 | Implement 1-byte opcode table | P0 | CD-001 | High |
| CD-003 | Implement 2-byte opcode table (0F xx) | P0 | CD-001 | High |
| CD-004 | Implement 3-byte opcode tables | P1 | CD-003 | Medium |
| CD-005 | Implement ModR/M + SIB parsing | P0 | None | Medium |
| CD-006 | Implement displacement/immediate parsing | P0 | CD-005 | Low |
| CD-007 | Implement VEX/EVEX prefix handling | P1 | CD-001 | Medium |
| CD-008 | SSE instruction decoding | P0 | CD-003 | High |
| CD-009 | AVX instruction decoding | P2 | CD-007 | High |
| CD-010 | Decoder test suite | P0 | CD-002 | Medium |

#### CPU-Interpreter Tasks

| ID | Task | Priority | Dependencies | Complexity |
|----|------|----------|--------------|------------|
| CI-001 | Data movement instructions (MOV, PUSH, POP) | P0 | CD-002 | Medium |
| CI-002 | Arithmetic instructions (ADD, SUB, MUL, DIV) | P0 | CD-002 | High |
| CI-003 | Logical instructions (AND, OR, XOR, NOT) | P0 | CD-002 | Medium |
| CI-004 | Shift/rotate instructions | P0 | CD-002 | Medium |
| CI-005 | Control flow (JMP, CALL, RET, Jcc) | P0 | CD-002 | Medium |
| CI-006 | String instructions (MOVS, STOS, CMPS) | P0 | CD-002 | Medium |
| CI-007 | Bit manipulation (BT, BTS, BSF, BSR) | P1 | CD-002 | Medium |
| CI-008 | System instructions (INT, IRET, SYSCALL) | P0 | CD-002 | High |
| CI-009 | Privileged instructions (MOV CR/DR, LGDT) | P0 | CD-002 | Medium |
| CI-010 | x87 FPU instructions | P1 | CD-002 | Very High |
| CI-011 | SSE instructions (scalar) | P0 | CD-008 | High |
| CI-012 | SSE instructions (packed) | P0 | CD-008 | Very High |
| CI-013 | SSE2 instructions | P0 | CD-008 | High |
| CI-014 | SSE3/SSSE3 instructions | P1 | CI-012 | Medium |
| CI-015 | SSE4.1/4.2 instructions | P1 | CI-014 | Medium |
| CI-016 | Flag computation (lazy evaluation) | P0 | CI-002 | Medium |
| CI-017 | Interpreter test suite | P0 | CI-001..CI-009 | Very High |

#### CPU-JIT Tasks

| ID | Task | Priority | Dependencies | Complexity |
|----|------|----------|--------------|------------|
| CJ-001 | Basic block detection | P0 | CD-010 | Medium |
| CJ-002 | IR (intermediate representation) design | P0 | None | High |
| CJ-003 | x86 → IR translation | P0 | CJ-001, CJ-002 | Very High |
| CJ-004 | IR → WASM code generation | P0 | CJ-002 | Very High |
| CJ-005 | Code cache management | P0 | CJ-004 | Medium |
| CJ-006 | Execution counter / hot path detection | P0 | None | Medium |
| CJ-007 | Baseline JIT (Tier 1) | P0 | CJ-003, CJ-004 | High |
| CJ-008 | Constant folding optimization | P1 | CJ-002 | Medium |
| CJ-009 | Dead code elimination | P1 | CJ-002 | Medium |
| CJ-010 | Common subexpression elimination | P1 | CJ-002 | Medium |
| CJ-011 | Flag elimination optimization | P1 | CJ-002 | Medium |
| CJ-012 | Register allocation | P1 | CJ-002 | High |
| CJ-013 | Optimizing JIT (Tier 2) | P1 | CJ-008..CJ-012 | Very High |
| CJ-014 | SIMD code generation | P1 | CJ-004 | High |
| CJ-015 | JIT test suite | P0 | CJ-007 | High |
| CJ-016 | Inline RAM loads/stores via JIT TLB fast-path | P0 | CJ-004, MM-012 | High |
| CJ-017 | MMIO/IO exits for JIT memory ops | P0 | CJ-016, MM-003 | Medium |

#### Memory Tasks

| ID | Task | Priority | Dependencies | Complexity |
|----|------|----------|--------------|------------|
| MM-001 | Physical memory allocation | P0 | None | Low |
| MM-002 | Memory bus routing | P0 | MM-001 | Medium |
| MM-003 | MMIO region management | P0 | MM-002 | Medium |
| MM-004 | 32-bit paging | P0 | MM-002 | Medium |
| MM-005 | PAE paging | P0 | MM-004 | Medium |
| MM-006 | 4-level paging (long mode) | P0 | MM-005 | Medium |
| MM-007 | TLB implementation | P0 | MM-006 | High |
| MM-008 | TLB invalidation (INVLPG, CR3 write) | P0 | MM-007 | Medium |
| MM-009 | Page fault handling | P0 | MM-006 | Medium |
| MM-010 | Sparse memory allocation | P1 | MM-001 | Medium |
| MM-011 | Memory test suite | P0 | MM-006 | Medium |
| MM-012 | JIT-visible TLB layout (stable offsets, packed entries) | P0 | MM-007 | Medium |
| MM-013 | `mmu_translate` helper for JIT (page walk + fill TLB) | P0 | MM-006..MM-012 | High |
| MM-014 | MMIO classification/epoch for JIT fast-path safety | P0 | MM-003, MM-012 | Medium |

---

### Performance Targets

| Metric | Target |
|--------|--------|
| Interpreter speed | Baseline (measure first) |
| Tier 1 JIT | ≥10x interpreter |
| Tier 2 JIT | ≥100 MIPS sustained |
| TLB hit rate | ≥95% for typical workloads |

---

### Coordination Points

#### Dependencies on Other Workstreams

- **Graphics (B)**: VGA register access goes through `CpuBus::io_read/io_write`
- **I/O (D, E, F, G)**: All device I/O goes through `CpuBus`
- **Integration (H)**: BIOS and ACPI depend on CPU modes working

#### What Other Workstreams Need From You

- Stable `CpuBus` interface
- Working protected mode and long mode
- Correct exception delivery (for device drivers)
- Interrupt injection API (for APIC)

---

### Testing

```bash
# Run CPU tests
bash ./scripts/safe-run.sh cargo test -p aero-cpu-core --locked
bash ./scripts/safe-run.sh cargo test -p aero-cpu-decoder --locked
# Note: `aero-jit-x86` can exceed safe-run's 10-minute default on cold caches.
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-jit-x86 --locked
bash ./scripts/safe-run.sh cargo test -p aero-mmu --locked

# Lint (treat warnings as errors)
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo clippy -p aero-jit-x86 --all-targets --all-features --locked -- -D warnings

# Focused smoke tests (fast, covers Tier-1 32-bit mode + the guest CPU benchmark suite payloads)
bash ./scripts/safe-run.sh cargo test -p aero-x86 --locked
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-jit-x86 --locked --test pf008_tier1_32
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-jit-x86 --locked --test pf008_tier1_32bit

# Run all tests (workspace-wide; first run can take >10 minutes in clean/contended sandboxes)
AERO_TIMEOUT=1800 bash ./scripts/safe-run.sh cargo test --locked
```

Use `cargo xtask conformance` (or `just test-conformance`) for differential instruction
semantics testing (`crates/conformance`, Aero vs reference backend).

```bash
# Recommended (uses scripts/safe-run.sh internally)
cargo xtask conformance --cases 512

# In contended/CI-like sandboxes, wrap the outer cargo invocation too (builds xtask under safe-run):
bash ./scripts/safe-run.sh cargo xtask conformance --cases 512 -- --nocapture
```

---

### Quick Start Checklist

1. ☐ Read [`AGENTS.md`](./purged-material.md) completely
2. ☐ Run `bash ./scripts/agent-env-setup.sh` and `source ./scripts/agent-env.sh`
3. ☐ Read [`../areas/cpu-and-jit.md`](../areas/cpu-and-jit.md)
4. ☐ Read [`../areas/memory-and-paging.md`](../areas/memory-and-paging.md)
5. ☐ Explore `crates/aero-cpu-core/src/` to understand current state
6. ☐ Run existing tests to establish baseline
7. ☐ Pick a task from the tables above and begin

---

*This workstream is the heart of the emulator. Performance here determines everything.*

## Workstream B: Graphics

> **⚠️ MANDATORY: Read and follow [`AGENTS.md`](./purged-material.md) in its entirety before starting any work.**
>
> AGENTS.md contains critical operational guidance including:
> - Defensive mindset (assume hostile/misbehaving code)
> - Resource limits and `safe-run.sh` usage
> - Windows 7 test ISO location (`/state/win7.iso`)
> - Interface contracts
> - Technology stack decisions
>
> **Failure to follow AGENTS.md will result in broken builds, OOM kills, and wasted effort.**

---

### Current status / what’s missing

Most of the “hard” graphics pieces already exist in-tree (with unit/integration tests). The main
remaining gap is **end-to-end Win7 WDDM validation on the canonical AeroGPU path**: `aero_machine`
exposes the correct PCI identity and has working boot-display fallbacks + scanout/vblank plumbing,
and BAR0 command execution is pluggable (submission bridge / in-process backend).

What’s still missing is validating the full “Win7 WDDM driver → ring submissions → `AEROGPU_CMD`
execution → fence completion → scanout present → browser canvas” loop against a real Win7 guest +
driver bring-up (plus any opcode/format gaps that fall out of that).

Key docs for that bring-up:

- [`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md) — canonical AeroGPU PCI IDs + current `aero_machine::Machine` status
- [`../areas/graphics.md`](../areas/graphics.md) — required VGA/VBE compatibility + scanout handoff model
- [`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md) — how `aero_machine` drives AeroGPU submission execution + fence forward progress (no-op bring-up vs submission bridge vs in-process backends)
- [`../specs/windows7-aerogpu-wddm-driver.md`](../specs/windows7-aerogpu-wddm-driver.md) — Win7 vblank/present timing contract (DWM/Aero stability)
- [`retirements.md`](retirements.md) — mapping from legacy “scratchpad task IDs” (SM3/DXBC/shared-surface) to in-tree implementations/tests (avoid duplicate work)

Quick reality check (as of this repo revision):

- ✅ Boot display (standalone VGA/VBE path): `MachineConfig::enable_vga=true` uses `crates/aero-gpu-vga/` and is wired into
 `crates/aero-machine/` (plus BIOS INT 10h handlers in `crates/firmware/`).
 - When `MachineConfig::enable_pc_platform=false`, `aero_machine` maps the VBE LFB MMIO aperture directly at
 the configured LFB base.
 - When `MachineConfig::enable_pc_platform=true`, `aero_machine` exposes a transitional Bochs/QEMU-compatible
 VGA PCI stub (currently `00:0c.0`, `1234:1111`) so the VBE LFB can be routed through its BAR0 inside the
 PCI MMIO window (BAR base assigned by BIOS POST / the PCI allocator unless pinned).
- ✅ Boot display (canonical browser machine): `MachineConfig::browser_defaults` (used by `crates/aero-wasm::Machine::new`)
 enables **AeroGPU** by default (`enable_aerogpu=true`, `enable_vga=false`), using AeroGPU's BAR1-backed VRAM
 plus legacy VGA/VBE decode for BIOS/boot display, and then handing off to WDDM scanout once claimed.
- ✅ Canonical AeroGPU identity in `aero_machine`: `MachineConfig::enable_aerogpu=true` (requires `enable_pc_platform=true`) /
 `MachineConfig::win7_graphics(...)`
 exposes `A3A0:0001` at `00:07.0` with **BAR1-backed VRAM** and VRAM-backed legacy VGA/VBE decode (`0xA0000..0xC0000`),
 **BAR0 MMIO + ring decode + fence/vblank/scanout regs**, and a backend boundary:
 - `Machine::aerogpu_drain_submissions` exposes newly-decoded submissions (`cmd_stream` + optional alloc table) for
 out-of-process execution.
 - `Machine::aerogpu_enable_submission_bridge` / `Machine::aerogpu_complete_fence` enable “external executor” semantics
 (fences require host completion). Vsync `PRESENT` fences are still paced by the device model: after the host reports
 completion, the fence becomes eligible to complete on the **next vblank** (Win7/WDDM timing contract).
 - Optional in-process backends exist for native/tests: `aerogpu_set_backend_immediate/null` (+ feature-gated
 `aerogpu_set_backend_wgpu`).
 - WDDM scanout presentation/boot→WDDM handoff is implemented in `Machine::display_present` and the shared
 `ScanoutState` publisher (`crates/aero-machine/src/{lib.rs,aerogpu.rs}`).
- ✅ AeroGPU ABI/protocol: `crates/aero-protocol/` (crate `aero-protocol`) contains Rust **and**
 TypeScript mirrors + ABI drift tests; it’s consumed by both Rust (`crates/aero-gpu/`, `crates/emulator/`)
 and the browser GPU worker (`apps/web/src/workers/`).
- ✅ Shared “device-side” AeroGPU implementation: `crates/aero-devices-gpu/` contains a portable PCI wrapper (BAR0 regs + BAR1 VRAM),
 ring/vblank/fence/scanout semantics, and a backend boundary for command execution (with unit + e2e tests).
- ✅ Wasm32 guardrails: `cargo xtask wasm-check` compile-checks `aero-devices-gpu`, `aero-machine`, and `aero-wasm` for
 `wasm32-unknown-unknown` (CI-friendly; does not require a JS runtime).
- ✅ Legacy/sandbox emulator wiring exists in `crates/emulator/` (reuses pieces of `aero-devices-gpu`), but it is not yet the canonical
 in-browser machine wiring.
- ✅ D3D9 + D3D11 translation: substantial implementations exist (`crates/aero-d3d9/`,
 `crates/aero-d3d11/`) with extensive host-side tests.
- ✅ WebGPU backend: `crates/aero-webgpu/` + `crates/aero-gpu/` provide WebGPU/wgpu-backed execution and present paths.
- 🚧 Remaining (P0): **validate the full Win7 driver bring-up and rendering loop** on the canonical browser machine:
 driver install → real ring submissions → `AEROGPU_CMD` execution in the GPU worker → fence completion →
 scanout present + vblank pacing (DWM stability).
 - Note: by default (no backend, submission bridge disabled), `aero_machine` completes fences without executing ACMD so guests can boot
 (with optional vblank pacing when vblank is active and the submission contains a vsync present).
 - Browser runtime robustness: the coordinator bounds buffered `aerogpu.submit` messages while the GPU worker is not READY, and
 may **force-complete fences** if a submission cannot be forwarded/executed (for example: queue overflow, postMessage failure, or GPU
 worker restart). This prevents guest deadlocks/TDRs at the cost of best-effort rendering in those failure modes.
 - Rust-side submission bridge tests: `bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_submission_bridge --locked`
 - Browser command-submission e2e: `bash ./scripts/safe-run.sh npm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu.spec.ts`
 - Browser vsync completion policy e2e: `bash ./scripts/safe-run.sh npm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu_vsync_completion.spec.ts`
 - Browser scanout smoke tests: `bash ./scripts/safe-run.sh npm run test:e2e -- tests/e2e/wddm_scanout_smoke.spec.ts`

### Overview

This workstream owns **graphics emulation**: VGA/VBE for boot, DirectX 9/10/11 translation for Windows applications, and the WebGPU/WebGL2 backend that renders to the browser canvas.

Graphics is what makes Windows 7 "usable." The Aero glass interface, DWM compositor, and all Windows applications depend on this workstream.

---

### Key Crates & Directories

| Crate/Directory | Purpose |
|-----------------|---------|
| `crates/aero-gpu/` | Core GPU abstraction, WebGPU backend |
| `crates/aero-gpu-vga/` | VGA/VBE mode emulation |
| `crates/aero-gpu-wasm/` | WASM bindings for GPU |
| `crates/aero-d3d9/` | DirectX 9 state machine and translation |
| `crates/legacy/aero-d3d9-shader/` | Legacy SM2/SM3 token-stream parser + disassembler (**reference-only**, not used by runtime) |
| `crates/aero-d3d11/` | DirectX 10/11 translation |
| `crates/aero-dxbc/` | DXBC bytecode parser (shared) |
| `crates/aero-webgpu/` | WebGPU abstraction layer |
| `crates/aero-protocol/` | **Canonical** AeroGPU ABI mirrors (Rust + TypeScript) |
| `crates/aero-devices-gpu/` | Shared “device-side” AeroGPU implementation (regs/ring/executor + portable PCI wrapper) |
| `crates/aero-machine/` | Canonical full-system machine (`aero_machine::Machine`) — supports both standalone VGA (`aero-gpu-vga`) and AeroGPU-owned legacy decode; browser defaults use AeroGPU |
| `crates/emulator/` | Legacy/sandbox integration surfaces for device models (reuses `aero-devices-gpu`; still used by some host-side tests) |
| `drivers/aerogpu/` | Windows 7 AeroGPU driver (KMD + UMD) |
| `apps/web/src/gpu/` + `apps/web/src/workers/` | TypeScript GPU runtime + GPU worker plumbing |

---

### Essential Documentation

**Must read:**

- [`../areas/graphics.md`](../areas/graphics.md) — Canonical “what’s implemented vs missing” graphics status page
- [`retirements.md`](retirements.md) — scratchpad task ID → code/test mapping (SM3/DXBC/shared-surface); use this to avoid duplicating already-implemented work
- [`../areas/graphics.md`](../areas/graphics.md) — Graphics architecture overview
- [`../specs/direct3d-9ex-and-dwm.md`](../specs/direct3d-9ex-and-dwm.md) — D3D9Ex for DWM/Aero
- [`../specs/direct3d-10-11-translation.md`](../specs/direct3d-10-11-translation.md) — D3D10/11 details
- [`../areas/graphics.md`](../areas/graphics.md) — VGA/VBE boot compatibility
- [`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md) — AeroGPU PCI identity contract (A3A0:0001)
- [`../specs/windows7-aerogpu-wddm-driver.md`](../specs/windows7-aerogpu-wddm-driver.md) — Win7 vblank/present semantics (DWM)

**Reference:**

- [`../overview.md`](../overview.md) — System architecture
- [`../areas/web-host.md`](../areas/web-host.md) — WebGPU/WebGL2 browser integration

---

### Interface Contracts

#### Display Output

```rust
// `aero_gpu_vga::DisplayOutput` (implemented by `aero_gpu_vga::VgaDevice`).
pub trait DisplayOutput {
 fn get_framebuffer(&self) -> &[u32];
 fn get_resolution(&self) -> (u32, u32);
 fn present(&mut self);
}
```

In the canonical machine (`crates/aero-machine`), the host reads display output via:

- `Machine::display_present()`
- `Machine::display_framebuffer()` (RGBA8888)
- `Machine::display_resolution()`

#### Host-side AeroGPU command processing (Rust)

- Command stream parsing + Ex-facing state machine (fence/present bookkeeping): `crates/aero-gpu/src/{protocol.rs,command_processor.rs}`
- WebGPU-backed command execution:
 - D3D9: `crates/aero-gpu/src/aerogpu_d3d9_executor.rs`
 - D3D10/11: `crates/aero-d3d11/src/runtime/aerogpu_cmd_executor.rs`

#### AeroGPU Device ↔ Driver Protocol

The AeroGPU Windows driver communicates with the emulator via a shared protocol. See:
- `drivers/aerogpu/protocol/` — AeroGPU protocol headers (`aerogpu_pci.h`, `aerogpu_ring.h`, `aerogpu_cmd.h`)
- `crates/aero-protocol/aerogpu/` — Emulator-side mirrors (Rust + TypeScript)

Reference: `../specs/aerogpu-device-abi.md` (canonical AeroGPU VID/DID contract; note that the canonical
`aero_machine::Machine` can expose the AeroGPU PCI identity and BAR1-backed VRAM via
`MachineConfig::enable_aerogpu` (requires `enable_pc_platform=true`; mutually exclusive with `enable_vga`), and uses the standalone
`aero_gpu_vga` when `enable_vga=true`).

---

### Tasks

The tables below are meant to be an **onboarding map**: what already exists in-tree (with tests) and
what remains.

Legend:

- **Implemented** = exists in-tree and has at least unit/integration test coverage.
- **Partial** = exists, but is intentionally minimal/stubbed or has known gaps.
- **Remaining** = not implemented yet (or only exists as an out-of-tree doc/spec).

#### Boot display: VGA/VBE (`crates/aero-gpu-vga`)

Recommended end-to-end regression suite (device model + BIOS INT 10h + canonical machine wiring):

```bash
# Runs: cargo test -p aero-gpu-vga, cargo test -p firmware,
# and aero-machine boot-display integration tests (boot_int10_* + vga_* + aerogpu_legacy_* + bios_vga_sync)
# with safe-run isolation.
bash ./scripts/ci/run-vga-vbe-tests.sh
```

| ID | Status | Task | Where | How to test |
|----|--------|------|-------|-------------|
| VG-001 | Implemented | VGA register + legacy VRAM emulation (sequencer/CRTC/attribute/graphics + 0xA0000..0xBFFFF windows) | `crates/aero-gpu-vga/src/lib.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-gpu-vga --locked` |
| VG-002 | Implemented | Text mode rasterization (80x25) | `crates/aero-gpu-vga/src/lib.rs`, `crates/aero-gpu-vga/src/text_font.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-gpu-vga --locked` |
| VG-003 | Implemented | Mode 13h (320x200x256) chain-4 rendering | `crates/aero-gpu-vga/src/lib.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-gpu-vga --locked` |
| VG-004 | Partial | Planar graphics write modes + basic rasterization (enough for BIOS/boot) | `crates/aero-gpu-vga/src/lib.rs` (planar paths + tests) | `bash ./scripts/safe-run.sh cargo test -p aero-gpu-vga --locked` |
| VG-005 | Implemented | Bochs VBE (`VBE_DISPI`) linear framebuffer modes (LFB base configurable; legacy default `SVGA_LFB_BASE`) | `crates/aero-gpu-vga/src/lib.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-machine --test boot_int10_vbe_sets_mode --locked` |
| VG-006 | Implemented | Palette + DAC behavior (VGA ports `0x3C6..0x3C9`) | `crates/aero-gpu-vga/src/palette.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-gpu-vga --locked` |
| VG-007 | Implemented | Snapshot/restore (optional; behind `io-snapshot`) | `crates/aero-gpu-vga/src/snapshot.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-machine --test vga_snapshot_roundtrip --locked` |
| VG-008 | Implemented | BIOS INT 10h VGA + VBE entrypoints (real-mode boot) | `crates/firmware/src/bios/int10.rs`, `crates/firmware/src/bios/int10_vbe.rs` | `bash ./scripts/safe-run.sh cargo test -p firmware --test int10_vbe --locked` |

#### AeroGPU ABI/protocol (`crates/aero-protocol`, crate `aero-protocol`)

| ID | Status | Task | Where | How to test |
|----|--------|------|-------|-------------|
| AGPU-PROTO-001 | Implemented | Rust mirrors of `drivers/aerogpu/protocol/*.h` (PCI IDs, MMIO regs, ring ABI, command ABI) | `crates/aero-protocol/aerogpu/*.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-protocol --locked` |
| AGPU-PROTO-002 | Implemented | TypeScript mirrors + iterators/writers (consumed by `apps/web/src/workers/`) | `crates/aero-protocol/aerogpu/*.ts` | `bash ./scripts/safe-run.sh npm run test:protocol` |
| AGPU-PROTO-003 | Implemented | ABI drift / conformance tests (Rust + TS) | `crates/aero-protocol/tests/*` | `bash ./scripts/safe-run.sh cargo test -p aero-protocol --locked` and `bash ./scripts/safe-run.sh npm run test:protocol` |

#### AeroGPU device model + scanout plumbing (the real remaining work)

| ID | Status | Task | Where | How to test |
|----|--------|------|-------|-------------|
| AGPU-MACHINE-001 | Partial (in `crates/aero-machine/`) | `A3A0:0001` @ `00:07.0`: BAR1 VRAM + VRAM-backed legacy VGA/VBE decode + BIOS VBE LFB/text fallback; BAR0 MMIO device model (ring decode + fences/vblank/scanout/cursor + error-info) plus submission bridge + optional in-process backends (no integrated renderer by default) | `crates/aero-machine/src/{lib.rs,aerogpu.rs}` | `bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_ring_noop_fence --locked`<br>`bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_submission_bridge --locked`<br>`bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_immediate_backend_completes_fence --locked` |
| AGPU-DEV-001 | Implemented | Shared “device-side” AeroGPU implementation: regs/ring/executor + portable PCI wrapper (BAR0 regs + BAR1 VRAM), intended for reuse across hosts (future target for wiring into `aero_machine`). | `crates/aero-devices-gpu/src/{pci.rs,executor.rs,ring.rs,regs.rs,scanout.rs}` | `bash ./scripts/safe-run.sh cargo test -p aero-devices-gpu --locked` |
| AGPU-DEV-001a | Implemented (legacy integration surface) | Monolithic emulator AeroGPU device model still exists (duplicate PCI wrapper/integration code) and is used by some sandbox tests. | `crates/emulator/src/devices/pci/aerogpu.rs`, `crates/emulator/src/gpu_worker/*` | `bash ./scripts/safe-run.sh cargo test -p emulator --test aerogpu_device --locked` |
| AGPU-DEV-002 | Implemented (feature-gated) | wgpu-backed command execution backend used by end-to-end tests (D3D9-focused). | Backend: `crates/aero-devices-gpu/src/backend.rs` (`NativeAeroGpuBackend`) • E2E: `crates/aero-devices-gpu/tests/aerogpu_end_to_end.rs`, `crates/emulator/tests/aerogpu_end_to_end.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-devices-gpu --features wgpu-backend --test aerogpu_end_to_end --locked`<br>`bash ./scripts/safe-run.sh cargo test -p emulator --features aerogpu-native --test aerogpu_end_to_end --locked` |
| AGPU-WIRE-001 | **Remaining (P0)** | **Validate and harden end-to-end `AEROGPU_CMD` execution** on the canonical browser machine (Win7 driver → ring submissions → GPU worker execution → fence completion → scanout present). The submission bridge + GPU worker executors exist; the missing work is real Win7 bring-up validation + any opcode/format gaps that fall out. | Rust/wasm bridge: `crates/aero-wasm/src/lib.rs` (`aerogpu_drain_submissions`, `aerogpu_complete_fence`) • Runtime routing: `apps/web/src/runtime/coordinator.ts`, `apps/web/src/workers/{machine_cpu.worker.ts,gpu-worker.ts}` | `bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_submission_bridge --locked`<br>`bash ./scripts/safe-run.sh npm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu.spec.ts`<br>`bash ./scripts/safe-run.sh npm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu_vsync_completion.spec.ts` |
| AGPU-WIRE-002 | Implemented (in `crates/aero-machine/`) | Boot display → WDDM scanout handoff: once the guest successfully claims scanout0, `Machine::display_present` prefers the WDDM framebuffer over VBE/text. `SCANOUT0_ENABLE=0` blanks output but does not release WDDM ownership back to legacy (ownership remains sticky until VM reset). | `crates/aero-machine/src/lib.rs` (`display_present`, `display_present_aerogpu_scanout`) + `crates/aero-machine/src/aerogpu.rs` (`AEROGPU_MMIO_REG_SCANOUT0_ENABLE`) | `bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_scanout_handoff --locked`<br>`bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_scanout_disable_publishes_wddm_disabled --locked` |
| AGPU-WIRE-003 | Implemented (MVP) | Browser presentation path for WDDM scanout state exists (`SCANOUT_SOURCE_WDDM`). The GPU worker can present scanout from either guest RAM **or** the shared VRAM aperture (BAR1 backing) when `ScanoutState` is published with `source=WDDM` and a non-zero `base_paddr`. The canonical WASM machine (`crates/aero-wasm`) plumbs the shared `ScanoutState` into `aero_machine`. | Scanout contract: `apps/web/src/ipc/scanout_state.ts` + `crates/aero-shared/src/scanout_state.rs` • Publisher: `crates/aero-wasm/src/lib.rs` + `crates/aero-machine/src/{lib.rs,aerogpu.rs}` • Reader: `apps/web/src/workers/gpu-worker.ts` | `bash ./scripts/safe-run.sh npm run test:e2e -- tests/e2e/wddm_scanout_smoke.spec.ts`<br>`bash ./scripts/safe-run.sh npm run test:e2e -- tests/e2e/wddm_scanout_vram_smoke.spec.ts` |
| AGPU-WIRE-004 | **Remaining (P0)** | Validate Win7 vblank + vsynced present behavior against the documented contract (DWM stability) | Spec: `../specs/windows7-aerogpu-wddm-driver.md` • Guest tests: `drivers/aerogpu/tests/win7/*` | (Browser) `bash ./scripts/safe-run.sh npm run test:e2e -- tests/e2e/web/gpu_submit_aerogpu_vsync_completion.spec.ts`<br>In Win7 guest: `cd drivers\\aerogpu\\tests\\win7 && build_all_vs2010.cmd && run_all.cmd` |

#### DirectX 9 translation (`crates/aero-d3d9`)

| ID | Status | Task | Where | How to test |
|----|--------|------|-------|-------------|
| D9-001 | Implemented | DXBC container parsing helpers | `crates/aero-d3d9/src/dxbc/`, `crates/aero-dxbc/src/` | `bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --locked` |
| D9-002 | Implemented | SM2/SM3 decode → IR → WGSL generation | `crates/aero-d3d9/src/sm3/`, `crates/aero-d3d9/src/shader.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --locked` |
| D9-003 | Implemented | Fixed-function pipeline translation (FVF/TSS → generated WGSL) | `crates/aero-d3d9/src/fixed_function/` | `bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test fixed_function_wgsl_snapshots --locked` |
| D9-004 | Implemented | Resource model + runtime/state tracking (textures, samplers, RT/DS, eviction) | `crates/aero-d3d9/src/resources/`, `crates/aero-d3d9/src/runtime/`, `crates/aero-d3d9/src/state/` | `bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --locked` |
| D9-005 | Partial | D3D9Ex/DWM-facing semantics live in the **AeroGPU command processor** layer, not the translator | `crates/aero-gpu/src/command_processor.rs`, `../specs/direct3d-9ex-and-dwm.md` | `bash ./scripts/safe-run.sh cargo test -p aero-gpu --test aerogpu_ex_protocol --locked` |

#### DirectX 10/11 translation (`crates/aero-d3d11`)

| ID | Status | Task | Where | How to test |
|----|--------|------|-------|-------------|
| D11-001 | Implemented | SM4/SM5 decode + translation to WGSL for VS/PS/**CS** (FL10_0 bring-up + basic compute) | `crates/aero-d3d11/src/sm4/`, `crates/aero-d3d11/src/shader_translate.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test shader_translate --locked` |
| D11-002 | Implemented | WGPU-backed AeroGPU command executor (render/present **and compute pass/dispatch**) | `crates/aero-d3d11/src/runtime/` | `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_smoke --locked` |
| D11-003 | Partial | Geometry shaders: GEOMETRY stage + `stage_ex` plumbing + **compute prepass emulation** (wgpu has no native GS stage). Draws route through a compute prepass and then an indirect render pass; today there is a built-in synthetic expansion fallback (e.g. fixed triangles), plus list and adjacency-list GS prepass paths that can execute a translated SM4 GS DXBC subset (`emit`/`cut`, stream 0, `pointlist`/`linestrip`/`triangle_strip` output) for `PointList`, `LineList`, `TriangleList`, `LineListAdj`, and `TriangleListAdj` draws (both `Draw` and `DrawIndexed`). GS inputs are populated via vertex pulling and a minimal VS-as-compute feeding path (simple VS subset), with an IA-fill fallback for strict passthrough VS. HS/DS DXBC is not executed yet. | `crates/aero-d3d11/src/runtime/{aerogpu_cmd_executor.rs,gs_translate.rs,strip_to_list.rs}` • Design notes: `../specs/compute-expansion-emulation.md` | `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_compute_prepass_smoke --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_compute_prepass_primitive_id --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_compute_prepass_vertex_pulling --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelist_emits_triangle --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelist_emits_triangle --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelistadj_emits_triangle --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelistadj_emits_triangle --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test gs_translate --locked` |
| D11-003a | Implemented | Robustness/forward-compat: `stage_ex` GS plumbing acceptance + stage mismatch validation (regression test; GS coverage is tracked separately). | `crates/aero-d3d11/tests/aerogpu_cmd_geometry_shader_ignore.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_ignore --locked` |
| D11-003b | Partial | Execute guest GS DXBC via compute (Emit/Cut semantics, stream 0) and wire it into the compute prepass path. Supported IA input topologies (for both `Draw` and `DrawIndexed`): `PointList`, `LineList`, `TriangleList`, `LineListAdj`, `TriangleListAdj`. Other topologies that still route through the compute-prepass path (notably strip inputs + strip-adjacency, and patchlists used for tessellation bring-up) currently use synthetic expansion and do not execute guest GS DXBC. Remaining work includes broader topology coverage (`LineStrip`, `TriangleStrip`, `LineStripAdj`, `TriangleStripAdj`; tracked elsewhere), broader VS-as-compute feeding for GS inputs, and broader opcode/system-value/resource-binding coverage. | `crates/aero-d3d11/src/runtime/{gs_translate.rs,strip_to_list.rs,aerogpu_cmd_executor.rs}` • Tooling: `cargo run -p aero-d3d11 --bin dxbc_dump -- <gs_*.dxbc>` (opcode discovery / operand encodings) | `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test gs_translate --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_point_to_triangle --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_compute_prepass_instancing --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_restart_strip --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_pointlist_draw_indexed --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelist_vs_as_compute_feeds_gs_inputs --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelist_emits_triangle --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelist_emits_triangle --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelist_instance_step_rate --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_linelistadj_emits_triangle --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_trianglelistadj_emits_triangle --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_reads_srv_buffer_translated_prepass --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_texture_t0_translated_prepass --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_samples_texture_translated_prepass --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_translated_primitive_id --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_geometry_shader_translated_prepass_sv_primitive_id --locked`; `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_gs_instance_count --locked` |
| D11-004 | Remaining | Hull/Domain (tessellation) execution + UAV/structured buffers + broader SM5 parity (stage_ex bindings are plumbed, but HS/DS shader execution is not implemented yet; binding HS/DS currently causes draws to return a clear error; patchlist topologies without HS/DS route through placeholder compute-prepass scaffolding rather than real tessellation) | Start at: `crates/aero-d3d11/src/shader_translate.rs`, `crates/aero-d3d11/src/runtime/{aerogpu_cmd_executor.rs,execute.rs}` • ABI: `crates/aero-protocol/aerogpu/aerogpu_cmd.rs` (stage_ex fields) | Add tests under `crates/aero-d3d11/tests/` and run `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --locked` |

#### WebGPU/WebGL2 backend (`crates/aero-gpu`, `crates/aero-webgpu`, `crates/aero-gpu-wasm`)

| ID | Status | Task | Where | How to test |
|----|--------|------|-------|-------------|
| WG-001 | Implemented | WebGPU adapter/device init + feature/limit negotiation | `crates/aero-webgpu/src/webgpu.rs`, `crates/aero-webgpu/src/caps.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-webgpu --test webgpu_smoke --locked` |
| WG-002 | Implemented | wgpu-backed backend + shader/pipeline/resource helpers | `crates/aero-gpu/src/backend/wgpu_backend.rs`, `crates/aero-gpu/src/*` | `bash ./scripts/safe-run.sh cargo test -p aero-gpu --locked` |
| WG-003 | Partial | WebGL2 fallback is **present-only** today (no full D3D execution) | `crates/aero-gpu/src/backend/webgl2_present_backend.rs`, `apps/web/src/gpu/raw-webgl2-presenter.ts` | `bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test negotiated_features_gl --locked` |
| WG-004 | Partial | Persistent caching exists for **D3D9 shader translation artifacts**; pipeline cache is still in-memory | Rust: `crates/aero-d3d9/src/runtime/shader_cache.rs` • JS: `web/gpu-cache/persistent_cache.ts` | (Browser) `wasm-pack test --headless --chrome crates/aero-d3d9` |
| WG-005 | Implemented | WASM bindings used by the browser runtime | `crates/aero-gpu-wasm/src/lib.rs` | `bash ./scripts/safe-run.sh cargo test -p aero-gpu-wasm --locked` |

---

### Shader Translation Pipeline

```
DXBC Bytecode (SM2/3/4/5)
 ↓
aero-dxbc parser
 ↓
Internal IR
 ↓
WGSL Generation
 ↓
WebGPU Shader Module
 ↓
Browser GPU
```

Key considerations:
- DXBC is a register-based bytecode; WGSL is more structured
- Texture sampling semantics differ between D3D and WebGPU
- Coordinate system differences (D3D is top-left origin, WebGPU is bottom-left)

---

### Performance Targets

| Metric | Target |
|--------|--------|
| Desktop frame rate | ≥30 FPS with Aero enabled |
| Shader compilation | <100ms per shader (cached after first compile) |
| Draw call overhead | Batching should reduce by ≥50% |

---

### Coordination Points

#### Dependencies on Other Workstreams

- **CPU (A)**: VGA register access comes through `CpuBus::io_read/io_write`
- **Windows Drivers (C)**: AeroGPU KMD/UMD must match emulator device model
- **Integration (H)**: VGA BIOS must work for boot

#### What Other Workstreams Need From You

- Working VGA text mode for BIOS/boot
- Stable AeroGPU device model for driver development
- D3D9Ex surface for DWM compositor

---

### Testing

```bash
# Fast sanity: AeroGPU protocol + device model + aero-machine ring/fence plumbing
bash ./scripts/ci/run-aerogpu-tests.sh

# Run graphics tests
bash ./scripts/safe-run.sh cargo test -p aero-gpu-vga --locked
bash ./scripts/safe-run.sh cargo test -p aero-protocol --locked
bash ./scripts/safe-run.sh cargo test -p aero-devices-gpu --locked
bash ./scripts/safe-run.sh cargo test -p aero-gpu --locked
bash ./scripts/safe-run.sh cargo test -p aero-webgpu --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --locked
bash ./scripts/safe-run.sh cargo test -p aero-dxbc --locked

# WASM compatibility checks (browser runtime)
bash ./scripts/safe-run.sh cargo xtask wasm-check

# Run protocol TypeScript tests (Node test runner)
bash ./scripts/safe-run.sh npm run test:protocol

# Browser e2e: WDDM scanout state presentation (no Win7 guest; validates BGRX->RGBA + alpha policy)
bash ./scripts/safe-run.sh npm run test:e2e -- tests/e2e/wddm_scanout_smoke.spec.ts
bash ./scripts/safe-run.sh npm run test:e2e -- tests/e2e/wddm_scanout_vram_smoke.spec.ts

# Manual (interactive): WDDM scanout debug harness (toggle scanoutState source/base_paddr/pitch and XRGB alpha forcing)
# Open `/web/wddm-scanout-debug.html` under a COOP/COEP-enabled dev server (e.g. `npm run dev:harness`).

 # Run AeroGPU end-to-end command-execution tests (feature-gated; wgpu/WebGPU; may skip unless AERO_REQUIRE_WEBGPU=1)
 bash ./scripts/safe-run.sh cargo test -p aero-devices-gpu --features wgpu-backend --test aerogpu_end_to_end --locked
 bash ./scripts/safe-run.sh cargo test -p emulator --features aerogpu-native --test aerogpu_end_to_end --locked
 bash ./scripts/safe-run.sh cargo test -p aero-machine --features aerogpu-wgpu-backend --test aerogpu_wgpu_backend_smoke --locked
 
 # Run aero_machine AeroGPU boot display + BAR0 ring/fence + vblank/scanout plumbing smoke tests
 bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_vram_alias --locked
 bash ./scripts/safe-run.sh cargo test -p aero-machine --test boot_int10_aerogpu_vbe_115_sets_mode --locked
bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_ring_noop_fence --locked
bash ./scripts/safe-run.sh cargo test -p aero-machine --test aerogpu_bar0_mmio_vblank --locked

# Run D3D9 translator-focused tests (no GPU required)
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test vertex_decl_translate --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test sm3_wgsl --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test fixed_function_wgsl_snapshots --locked

# Run D3D9 WebGPU integration tests (wgpu/WebGPU; may skip unless AERO_REQUIRE_WEBGPU=1)
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test d3d9_fixed_function --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test d3d9_vertex_input --locked
bash ./scripts/safe-run.sh cargo test -p aero-d3d9 --test d3d9_blend_depth_stencil --locked

# Run D3D11 command-executor smoke test (wgpu/WebGPU; may skip unless AERO_REQUIRE_WEBGPU=1)
bash ./scripts/safe-run.sh cargo test -p aero-d3d11 --test aerogpu_cmd_smoke --locked
```

**Note:** GPU tests may be skipped on headless/GPU-less machines. Set `AERO_REQUIRE_WEBGPU=1` to force failure if no GPU is available.

If you need to validate CPU texture decompression fallbacks (or work around flaky driver/software-adapter compression paths), set `AERO_DISABLE_WGPU_TEXTURE_COMPRESSION=1` to force wgpu/WebGPU feature negotiation to avoid BC/ETC2/ASTC texture compression.

---

### Quick Start Checklist

1. ☐ Read [`AGENTS.md`](./purged-material.md) completely
2. ☐ Run `bash ./scripts/agent-env-setup.sh` and `source ./scripts/agent-env.sh`
3. ☐ Read [`../areas/graphics.md`](../areas/graphics.md)
4. ☐ Read [`../specs/direct3d-9ex-and-dwm.md`](../specs/direct3d-9ex-and-dwm.md)
5. ☐ Explore `crates/aero-gpu/src/` and `crates/aero-d3d9/src/`
6. ☐ Run existing tests to establish baseline
7. ☐ Pick a task from the tables above and begin

---

*Graphics makes Windows 7 beautiful. This is what users will see.*

## Workstream G: Audio

> **⚠️ MANDATORY: Read and follow [`AGENTS.md`](./purged-material.md) in its entirety before starting any work.**
>
> AGENTS.md contains critical operational guidance including:
> - Defensive mindset (assume hostile/misbehaving code)
> - Resource limits and `safe-run.sh` usage
> - Windows 7 test ISO location (`/state/win7.iso`)
> - Interface contracts
> - Technology stack decisions
>
> **Failure to follow AGENTS.md will result in broken builds, OOM kills, and wasted effort.**

---

### Overview

This workstream owns **audio emulation**: Intel HD Audio (HDA) controller + codec emulation, virtio-snd (optional paravirtual audio), and the Web Audio API integration that plays sound in the browser.

Audio is important for user experience but not on the critical boot path.

---

### Key Crates & Directories

| Crate/Directory | Purpose |
|-----------------|---------|
| `crates/aero-audio/` | Guest audio device models (HDA controller + codec + PCM helpers) |
| `crates/aero-virtio/` | Virtio device models (includes virtio-snd) |
| `crates/platform/src/audio/` | SharedArrayBuffer ring layouts + WASM bridges (`WorkletBridge`, `MicBridge`) |
| `crates/aero-wasm/` | wasm-pack exports used by the browser runtime (e.g. `HdaControllerBridge`, `VirtioSndPciBridge`, `attach_worklet_bridge`, `attach_mic_bridge`) |
| `apps/web/src/platform/` | Web Audio output setup + AudioWorklet consumer (`audio.ts`, `audio-worklet-processor.js`) |
| `apps/web/src/audio/` | Microphone capture UI + AudioWorklet producer (`mic_capture.ts`, `mic-worklet-processor.js`) |
| `apps/web/src/runtime/` | Coordinator↔worker control plane (ring buffer attachment messages + routing) |
| `apps/web/src/io/` + `apps/web/src/workers/io.worker.ts` | Worker runtime device stack (PCI/MMIO/virtio; IO worker owns guest audio devices) |

---

### Essential Documentation

**Must read:**

- [`../areas/audio.md`](../areas/audio.md) — Audio architecture

**Reference:**

- [`../areas/web-host.md`](../areas/web-host.md) — Web Audio API usage
- [`../overview.md`](../overview.md) — System architecture

---

### Tasks

#### Audio Tasks

| ID | Task | Priority | Dependencies | Complexity |
|----|------|----------|--------------|------------|
| AU-001 | HD Audio controller emulation | P0 | None | Very High |
| AU-002 | HDA codec emulation | P0 | AU-001 | High |
| AU-003 | Sample format conversion | P0 | None | Medium |
| AU-004 | AudioWorklet integration | P0 | None | Medium |
| AU-005 | Audio buffering/latency management | P0 | AU-004 | Medium |
| AU-006 | AC'97 fallback (legacy) | P2 | None | High |
| AU-007 | Audio input (microphone) | P2 | AU-004 | Medium |
| AU-008 | Audio test suite | P0 | AU-001 | Medium |

**Status (AU-004 / AU-008)**

- **AU-004 (AudioWorklet integration)**: AudioWorklet + SharedArrayBuffer rings exist:
 - `apps/web/src/platform/audio.ts`, `apps/web/src/platform/audio-worklet-processor.js`
 - `crates/platform/src/audio/worklet_bridge.rs`
 - The HDA model is integrated into the IO worker PCI/MMIO stack via:
 - `HdaControllerBridge` (`crates/aero-wasm/src/hda_controller_bridge.rs`)
 - `HdaPciDevice` (`apps/web/src/io/devices/hda.ts`)
 - Ring buffers are forwarded by the coordinator (`SetAudioRingBufferMessage` / `SetMicrophoneRingBufferMessage`).
 - virtio-snd exists as a Rust device model (`crates/aero-virtio/src/devices/snd.rs`) + Win7 driver contract, and **is wired into
 the browser IO worker** as a virtio-pci device:
 - WASM bridge export: `crates/aero-wasm/src/virtio_snd_pci_bridge.rs` (`VirtioSndPciBridge`)
 - TS PCI device wrapper: `apps/web/src/io/devices/virtio_snd.ts` (`VirtioSndPciDevice`)
 - IO worker init/wiring: `apps/web/src/workers/io_virtio_snd_init.ts` + `apps/web/src/workers/io.worker.ts`
 - **Ring attachment policy** (SPSC rings): the IO worker attaches the playback/mic rings to **HDA when present**, and falls back
 to attaching them to **virtio-snd** in WASM builds/configurations that omit HDA.
- **AU-008 (Audio test suite)**: E2E coverage includes:
 - AudioWorklet worker-tone smoke: `tests/e2e/audio-worklet-worker-tone.spec.ts`
 - Snapshot/resume regressions:
 - Worker-tone snapshot: `tests/e2e/audio-worklet-snapshot-resume.spec.ts`
 - IO-worker HDA PCI snapshot: `tests/e2e/audio-hda-pci-snapshot-resume.spec.ts`
 - Mic ring discard on resume: `tests/e2e/audio-mic-ring-snapshot-resume.spec.ts`
 - Playback ring discard on AudioContext suspend/resume: `tests/e2e/audio-worklet-suspend-resume-discard.spec.ts`
 - AudioWorklet + HDA demo (CPU worker harness): `tests/e2e/audio-worklet-hda-demo.spec.ts`

 Plus unit coverage for IO-worker HDA tick scheduling (`apps/web/src/io/devices/hda.test.ts`). Remaining work is full-VM guest-driver coverage (Windows playback/capture) once the end-to-end integration is exercised regularly.

---

### Audio Architecture

#### Data Flow

```
┌─────────────────────────────────────────────┐
│ Windows 7 Guest │
│ │ │
│ HD Audio Driver (hdaudio.sys) │
├─────────────────┼───────────────────────────┤
│ ▼ │
│ HDA Controller Emulation │ ← AU-001
│ │ │
│ ▼ │
│ HDA Codec Emulation │ ← AU-002
│ │ │
│ ▼ │
│ Sample Format Conversion │ ← AU-003
│ │ │
└─────────────────┼───────────────────────────┘
 │ SharedArrayBuffer ring buffer
 ▼
┌─────────────────────────────────────────────┐
│ Browser │
│ │ │
│ AudioWorklet Processor │ ← AU-004
│ │ │
│ ▼ │
│ Web Audio API │
│ │ │
│ ▼ │
│ System Audio Output │
└─────────────────────────────────────────────┘
```

#### Ring Buffer

Audio samples flow through a SharedArrayBuffer ring buffer:

```
Producer (Emulator) Consumer (AudioWorklet)
 │ │
 ▼ │
 ┌─────────────────────────────────────────┐ │
 │ [frames] [frames] [frames] [empty] │ │
 │ ↑ ↑ │ │
 │ readFrameIndex writeFrameIndex │ │
 └─────────────────────────────────────────┘ │
 │ ▼
 │ Pull ~128 frames
 │ per render quantum
 └───────────────────────────────────────
```

Key considerations:
- AudioWorklet runs on a separate high-priority thread
- Ring buffer must be lock-free (Atomics for pointers)
- Underflow handling (output silence, don't block)
- Sample rate conversion if needed (guest may use 44.1kHz, browser 48kHz)

---

### HD Audio Implementation Notes

Intel High Definition Audio is the standard Windows 7 audio controller. Key components:

1. **Controller Registers** — PCI BAR0 MMIO
2. **CORB/RIRB** — Command/Response ring buffers
3. **Stream Descriptors** — DMA buffer pointers
4. **Codec Nodes** — Audio widgets (DAC, ADC, mixer, etc.)

Windows 7 uses the inbox `hdaudio.sys` and `HdAudBus.sys` drivers.

Reference: Intel HDA specification (publicly available).

#### Pin/power gating semantics (playback + capture)

The `aero-audio` HDA codec model enforces a minimal subset of widget power + pin control semantics:

- **Playback is silenced** when:
 - AFG `power_state != D0`, OR
 - output pin `pin_ctl == 0`, OR
 - output pin `power_state != D0`.
- **Capture DMA still advances**, but the guest receives **silence** when:
 - AFG `power_state != D0`, OR
 - mic pin `power_state != D0`, OR
 - mic pin `pin_ctl == 0`.
 - In these capture-muted cases, the device model must **not** consume microphone samples from the host ring (so we don’t drop mic
 audio while the guest endpoint is disabled).

See [`../areas/audio.md`](../areas/audio.md) for the canonical write-up + unit test pointers.

---

### AudioWorklet Integration

Canonical implementation:

- Output setup (main thread): `apps/web/src/platform/audio.ts` (`createAudioOutput`)
- Ring consumer (AudioWorklet): `apps/web/src/platform/audio-worklet-processor.js`
- Ring layout/constants + helper math (TS): `apps/web/src/audio/audio_worklet_ring.ts`
- Ring layout/constants + helper math (layout-only, importable by AudioWorklet): `apps/web/src/platform/audio_worklet_ring_layout.js`
- Ring snapshot restore helper (JS): `apps/web/src/platform/audio_ring_restore.ts` (`restoreAudioWorkletRing`)
- Ring layout/constants + WASM producer bridge: `crates/platform/src/audio/worklet_bridge.rs`

Note: `createAudioOutput` includes a few robustness/latency controls that are important for real VM runs:

- `startupPrefillFrames` — optional startup silence prefill (reduces initial underrun spam / tolerates slow-starting producers)
- `discardOnResume` — discards buffered playback frames on `AudioContext` resume to avoid stale latency after suspends
- Safari/WebKit fallbacks (`webkitAudioContext`, constructor option retries, and `AudioWorkletNode` option compatibility retries)

See [`../areas/audio.md`](../areas/audio.md#createaudiooutput-options-latency-vs-robustness) for the canonical
write-up and suggested defaults (demo mode vs VM mode).

```typescript
// Simplified: AudioWorklet consumer for the SAB playback ring.
//
// Notes:
// - Indices are monotonic *frame counters* (u32 wrapping at 2^32), not modulo indices.
// - Samples are interleaved f32: [L0, R0, L1, R1, ...].
// - The consumer (AudioWorklet) increments the underrun counter by the number of
// missing frames it had to render as silence.
// The canonical implementation imports these from:
// - `apps/web/src/platform/audio_worklet_ring_layout.js` (layout-only, safe for AudioWorklet)
// - `apps/web/src/audio/audio_worklet_ring.ts` (TS helpers used by producers; re-exports the same constants)
import {
 READ_FRAME_INDEX,
 WRITE_FRAME_INDEX,
 UNDERRUN_COUNT_INDEX,
 HEADER_U32_LEN,
 HEADER_BYTES,
 framesAvailableClamped,
} from './audio_worklet_ring_layout.js';

class AeroAudioProcessor extends AudioWorkletProcessor {
 constructor(options) {
 super();
 const sab = options.processorOptions.ringBuffer;
 this.header = new Uint32Array(sab, 0, HEADER_U32_LEN);
 this.samples = new Float32Array(sab, HEADER_BYTES);
 this.channelCount = options.processorOptions.channelCount;
 this.capacityFrames = options.processorOptions.capacityFrames;

 // Canonical Aero worklet implementation also supports a small control channel:
 // the main thread may post `{ type: "ring.reset" }` to discard any buffered backlog
 // (`readFrameIndex := writeFrameIndex`) on resume.
 }

 process(_inputs, outputs) {
 const output = outputs[0];
 const framesNeeded = output[0].length;

 const read = Atomics.load(this.header, READ_FRAME_INDEX) >>> 0;
 const write = Atomics.load(this.header, WRITE_FRAME_INDEX) >>> 0;
 const available = framesAvailableClamped(read, write, this.capacityFrames);
 const framesToRead = Math.min(framesNeeded, available);

 // Copy framesToRead frames from `this.samples` into `output` (wrap-around omitted).
 // Zero-fill any missing frames and Atomics.add(UNDERRUN_COUNT_INDEX, missingFrames).

 Atomics.store(this.header, READ_FRAME_INDEX, read + framesToRead);
 return true;
 }
}
```

---

### Sample Format Conversion

Windows may output various formats:
- 16-bit signed integer (common)
- 24-bit signed integer
- 32-bit signed integer
- 32-bit float

Web Audio API requires 32-bit float in [-1.0, 1.0]:

```rust
// 16-bit signed to float
fn s16_to_f32(sample: i16) -> f32 {
 sample as f32 / 32768.0
}

// 32-bit signed to float
fn s32_to_f32(sample: i32) -> f32 {
 sample as f32 / 2147483648.0
}
```

---

### Coordination Points

#### Dependencies on Other Workstreams

- **CPU (A)**: guest MMIO/PIO operations must reach the worker runtime device stack (HDA/virtio-snd are exposed as PCI/MMIO devices).
- **Integration (H)**: IO worker PCI/MMIO registration + routing for guest audio devices (see `apps/web/src/workers/io.worker.ts`, `apps/web/src/io/*`).

#### What Other Workstreams Need From You

- Working audio for user experience testing
- System sounds for boot verification

---

### Testing

```bash
# Run audio tests
bash ./scripts/safe-run.sh cargo test -p aero-audio --locked

# Manual testing
# Boot Windows 7 and validate that the in-box HDA driver enumerates + plays/records audio:
# docs/testing/audio-windows7.md
```

Audio is hard to test automatically. Focus on:
- Controller initialization (no guest crash)
- Sample flow (ring buffer fills, doesn't overflow)
- Codec response to commands

---

### Quick Start Checklist

1. ☐ Read [`AGENTS.md`](./purged-material.md) completely
2. ☐ Run `bash ./scripts/agent-env-setup.sh` and `source ./scripts/agent-env.sh`
3. ☐ Read [`../areas/audio.md`](../areas/audio.md)
4. ☐ Explore `crates/aero-audio/src/`
5. ☐ Run existing tests to establish baseline
6. ☐ Pick a task from the tables above and begin

---

*Audio brings the emulator to life. System sounds tell you it's working.*

## Workstream H: Integration & Boot

> **⚠️ MANDATORY: Read and follow [`AGENTS.md`](./purged-material.md) in its entirety before starting any work.**
>
> AGENTS.md contains critical operational guidance including:
> - Defensive mindset (assume hostile/misbehaving code)
> - Resource limits and `safe-run.sh` usage
> - Windows 7 test ISO location (`/state/win7.iso`)
> - Interface contracts
> - Technology stack decisions
>
> **Failure to follow AGENTS.md will result in broken builds, OOM kills, and wasted effort.**

---

### Overview

This workstream owns **system integration**: BIOS, ACPI tables, device model wiring, PCI bus, interrupt controllers, timers, and the overall boot sequence.

This is the **coordination hub**. You wire together the work from all other workstreams and make the system boot.

### Current status (implementation reality)

#### Implemented (already in-tree)

- **Legacy BIOS (HLE) with INT dispatch via ROM stubs + `HLT` hypercall**:
 `crates/firmware/src/bios/mod.rs` (see module-level docs) + ROM generation in
 `crates/firmware/src/bios/rom.rs`.
- **POST + boot handoff** (IVT/BDA/EBDA init, A20 enable, then either MBR `0000:7C00` or El Torito
 no-emulation image depending on `BiosConfig::boot_drive`):
 `crates/firmware/src/bios/post.rs`, `crates/firmware/src/bios/ivt.rs`.
- **E820 map with PCI holes + >4 GiB high-memory remap**:
 `crates/firmware/src/bios/interrupts.rs::build_e820_map` and PC constants in
 `crates/aero-pc-constants/src/lib.rs`.
- **ACPI table generation + publication during POST**:
 `crates/aero-acpi/` (tables + AML DSDT) and BIOS integration in
 `crates/firmware/src/bios/acpi.rs`.
- **PCI core + ECAM mapping** (config ports + 256 MiB ECAM window at `0xB000_0000`):
 `crates/devices/src/pci/*` (core types), platform wiring in
 `crates/aero-pc-platform/src/lib.rs` and `crates/aero-machine/src/lib.rs::map_pc_platform_mmio_regions`.
- **PC interrupt/timer models wired in canonical platforms**:
 PIC (8259A), LAPIC + I/O APIC, PIT (8254), RTC/CMOS, HPET, ACPI PM/Sci, IMCR
 (see `crates/devices/src/*`, `crates/aero-pc-platform/src/lib.rs`,
 `crates/aero-machine/src/lib.rs`).
- **PCI MSI/MSI-X message delivery (for devices that opt in)**:
 `aero_platform::interrupts::msi` + `PlatformInterrupts::trigger_msi`; used today by
 (non-exhaustive):
 - AHCI (MSI) and NVMe (MSI + single-vector MSI-X) in both `aero-machine` and `aero-pc-platform`
 (see `crates/aero-machine/src/lib.rs::{process_ahci,process_nvme}` and
 `crates/aero-pc-platform/src/lib.rs::{process_ahci,process_nvme}`), and
 - xHCI (MSI + single-vector MSI-X) in `aero-pc-platform` when enabled (see
 `crates/devices/src/usb/xhci.rs` and `crates/devices/tests/xhci_msi_integration.rs`), and
 - virtio-pci MSI-X delivery in canonical integrations (virtio-blk in both stacks;
 virtio-net/virtio-input in `aero-machine`) via real virtio interrupt sinks plus MSI-X
 enable/function-mask mirroring in `VirtioPciBar0Mmio` (see VTP-009).
- **Snapshots + restore plumbing**:
 format + tooling in `crates/aero-snapshot/`, IO device state in `crates/aero-io-snapshot/`,
 canonical machine integration/tests in `crates/aero-machine/tests/*`.

#### Known major gaps / limitations (please don’t rediscover these)

- **SMP is still in bring-up (BSP-driven + partial AP execution; not a full SMP scheduler)**:
 `cpu_count` is **not** forced to 1: firmware publishes CPU topology via **ACPI MADT + SMBIOS**
 for `cpu_count >= 1`.
 - `aero_machine::Machine` includes basic SMP plumbing (per-vCPU LAPIC MMIO + INIT/SIPI bring-up +
 a bounded cooperative AP run loop inside `Machine::run_slice`; see
 `crates/aero-machine/tests/ap_tsc_sipi_sync.rs`, `lapic_mmio_per_vcpu.rs`,
 `ioapic_routes_to_apic1.rs`, `smp_lapic_timer_wakes_ap.rs`, and `smp_timer_irq_routed_to_ap.rs`.
 - `aero_machine::pc::PcMachine` / `aero_pc_platform::PcPlatform` remain **BSP-only execution**
 today; `cpu_count > 1` there is still primarily for firmware-table enumeration tests.
 Full SMP work remains substantial (stable multi-vCPU scheduling, AP↔BSP/AP IPI paths, and
 determinism/snapshot/time integration).
 - **Workaround (for real guest boots today):** keep `cpu_count = 1` and use snapshots for fast
 boot/dev workflows (see [`../specs/snapshot-format.md`](../specs/snapshot-format.md)).
 - **Progress tracker / plan:** [`../areas/cpu-and-jit.md`](../areas/cpu-and-jit.md)
 See [platform and firmware, SMP boot](../areas/platform-and-firmware.md#smp-boot-bsp--aps).
- **Virtio legacy INTx delivery is polling-based (MSI-X is wired end-to-end)**:
 - Transport MSI-X support (table/PBA + vector programming): `crates/aero-virtio/src/pci.rs`.
 - MSI-X enable/function-mask bits are mirrored into the virtio transport (`sync_virtio_msix_from_platform`,
 `VirtioPciBar0Mmio::{sync_pci_config,sync_pci_command}`); MSI delivery uses
 `VirtioPlatformInterruptSink` / `VirtioMsixInterruptSink`.
 - Coverage: `crates/aero-machine/tests/virtio_blk_msix.rs`,
 `crates/aero-machine/tests/virtio_input_msix.rs`,
 `crates/aero-machine/tests/machine_snapshot_preserves_msix_enable.rs`,
 `crates/aero-pc-platform/tests/pc_platform_virtio_blk_msix.rs`,
 `crates/aero-pc-platform/tests/pc_platform_virtio_blk_msix_snapshot.rs`.
- **NVMe MSI/MSI-X is implemented (but Win7 support is opt-in/experimental)**:
 `aero-devices-nvme` exposes MSI + MSI-X capabilities (currently single-vector MSI-X) and delivers
 message-signaled interrupts when enabled (see `crates/aero-devices-nvme/README.md`,
 `crates/aero-devices-nvme/tests/interrupts.rs`, `crates/aero-machine/tests/nvme_msix.rs`, and
 `pc_platform_nvme` tests).
 Note: Windows 7 has no in-box NVMe driver.
- **MSI/MSI-X delivery is LAPIC-based; in legacy PIC mode MSI vectors are not surfaced to the CPU**:
 `PlatformInterrupts::trigger_msi` decodes the MSI address/data and injects a fixed interrupt into
 the selected LAPIC(s) (destination ID `0xFF` broadcasts to all LAPICs; see
 `crates/platform/src/interrupts/msi.rs`,
 `crates/platform/src/interrupts/router.rs::{inject_fixed_for_apic,inject_fixed_broadcast}`, and
 `crates/platform/tests/smp_msi_routing.rs`, and
 `crates/devices/tests/msix_cpu_core_integration.rs`).
 Note: MSI injection intentionally bypasses `PlatformInterruptMode`, but while the platform is in
 **Legacy PIC mode** the vCPU interrupt polling path (`InterruptController::get_pending` /
 `PlatformInterrupts::{get_pending_for_cpu,get_pending_for_apic}`) consults the 8259 PIC instead of
 LAPIC IRR state, so MSI-delivered vectors will not be observed until the guest switches to APIC
 mode. Also ensure the guest leaves the LAPIC software-enabled (SVR[8]=1).
---

### Key Crates & Directories

The **canonical** integration stack is now fully in-tree: BIOS/ACPI are implemented in Rust, and the
machine wiring lives in `aero-machine` (see [Canonical machine stack](../decisions/0014-canonical-machine-stack.md)).
Older docs may still read like “bring your own BIOS binary” or like the machine layer is still TBD
— that is no longer accurate.

| Crate/Directory | Purpose |
|-----------------|---------|
| `crates/aero-machine/` | **Canonical machine integration layer** (the canonical machine stack decision): CPU + memory + devices + firmware; dispatches BIOS interrupt “hypercalls” |
| `crates/firmware/` | **Canonical legacy BIOS HLE** (`firmware::bios`): ROM stub generation + POST + INT services; publishes ACPI + SMBIOS |
| `crates/aero-acpi/` | ACPI table generator (used by `firmware::bios`) |
| `crates/devices/` | Device models (PCI, PIT/RTC/HPET, PIC port wrappers, AHCI/IDE, etc.) |
| `crates/platform/` / `crates/aero-pc-platform/` | Platform buses + PC/Q35-ish wiring helpers used by `aero-machine` |
| `crates/aero-interrupts/` | Interrupt controller models (PIC/APIC/I/O APIC) used by the platform layer |
| `crates/aero-smp/` | Deterministic SMP model (per-vCPU state + LAPIC/IPI delivery + scheduler + snapshot integration) used by the legacy `crates/emulator/` SMP path (not yet wired into the canonical `aero-machine` execution loop). |
| `crates/aero-timers/` / `crates/aero-time/` | Legacy timer stack (currently not used by the canonical `aero-machine` / `aero-pc-platform` wiring; prefer `crates/devices/*` + `crates/platform/src/interrupts/*`) |
| `crates/aero-snapshot/` | VM snapshot/restore format + helpers |
| `crates/emulator/` | Legacy/native emulator runtime + compat stack (not canonical VM wiring). Some legacy integration surfaces (e.g. sandbox AeroGPU PCI wrapper + executor wiring) still live here. See [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md). |
| `crates/aero-boot-tests/` | QEMU-based reference boot-test harness (registers workspace-root `tests/` boot suites as `[[test]]` targets; see [`../areas/testing.md`](../areas/testing.md)) |
| `assets/bios.bin` | **Generated fixture**: a 64 KiB ROM image built from `firmware::bios::build_bios_rom()` (not the runtime BIOS source). Regenerate with `cargo xtask bios-rom` (or `cargo xtask fixtures`). |
| `tests/fixtures/boot/` | Deterministic tiny boot fixtures generated by `cargo xtask fixtures` (CI enforces determinism via `cargo xtask fixtures --check`) |
| `tests/boot/basic_boot.rs`, `tests/boot_sector.rs`, `tests/freedos_boot.rs`, `tests/windows7_boot.rs` | QEMU-based reference boot tests (registered under `crates/aero-boot-tests`) |
| `scripts/prepare-freedos.sh` | Downloads + patches FreeDOS image into `test-images/` (required for `freedos_boot`) |
| `scripts/validate-acpi.sh` | Validates the checked-in AML tables under `crates/firmware/acpi/` using ACPICA `iasl` |

---

### Essential Documentation

**Must read:**

- [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md) — BIOS and ACPI
- [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md) — El Torito CD boot + INT 13h extensions (Win7 install media)
- [`../overview.md`](../overview.md) — System architecture
- [`../specs/snapshot-format.md`](../specs/snapshot-format.md) — Snapshot format
- [`../areas/storage.md`](../areas/storage.md) — Canonical Windows 7 storage topology (stable PCI BDFs + media attachment mapping + IRQ routing)
- [`../areas/testing.md`](../areas/testing.md) — How to run CI-equivalent tests locally (includes QEMU boot tests + fixtures)

**Reference:**

- [`../areas/testing.md`](../areas/testing.md) — Integration testing
- [`project-history.md`](./sprint-era-record.md) — Boot milestones
- [`../areas/debugging.md`](../areas/debugging.md) — Debug surfaces

---

### Tasks (status-aware)

Most `BI-*` / `AC-*` / `DM-*` items from the original project plan are now **implemented and covered
by tests** in the canonical stack. The tables below reflect current reality and point at the
relevant crates/tests.

Status legend:

- **Implemented**: in-tree + covered by unit/integration tests.
- **Partial**: implemented, but with a known limitation called out in the Notes/Pointers.
- **Open**: not implemented yet (or implemented in one integration layer but not the other).

#### BIOS Tasks

> Note: The canonical Windows 7 storage + boot-media topology (including the El Torito install
> flow) is defined in [`../areas/storage.md`](../areas/storage.md) and
> [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md). In the current BIOS, the boot
> path is **selected by the host** via `BiosConfig::boot_drive` (typically `0xE0` for install media,
> `0x80` for normal HDD boot).

| ID | Task | Status | Priority | Dependencies | Complexity | Pointers |
|----|------|--------|----------|--------------|------------|----------|
| BI-001 | POST sequence | Implemented | P0 | None | Medium | `crates/firmware/src/bios/post.rs` |
| BI-002 | Memory detection (E820) | Implemented | P0 | BI-001 | Medium | `crates/firmware/src/bios/interrupts.rs::build_e820_map` |
| BI-003 | Interrupt vector table setup | Implemented | P0 | BI-001 | Low | `crates/firmware/src/bios/ivt.rs` |
| BI-004 | BIOS data area setup | Implemented | P0 | BI-001 | Low | `crates/firmware/src/bios/ivt.rs::init_bda` |
| BI-005 | INT 10h (video) | Implemented | P0 | None | Medium | `crates/firmware/src/bios/int10.rs`, `int10_vbe.rs` |
| BI-006 | INT 13h (disk + EDD + CD-ROM + El Torito services) | Implemented | P0 | None | Medium | `crates/firmware/src/bios/interrupts.rs::handle_int13`, `crates/firmware/src/bios/eltorito.rs` (note: BIOS disk backend is currently read-only; write ops report write-protected) |
| BI-007 | INT 15h (system) | Implemented | P0 | None | Medium | `crates/firmware/src/bios/interrupts.rs::handle_int15` |
| BI-008 | INT 16h (keyboard) | Implemented | P0 | None | Low | `crates/firmware/src/bios/interrupts.rs::handle_int16` |
| BI-009 | Boot device selection (host-configured via `BiosConfig::boot_drive`) | Implemented | P0 | BI-006 | Low | `firmware::bios::BiosConfig::boot_drive`, `crates/firmware/src/bios/post.rs::boot` |
| BI-010 | Boot code loading (HDD MBR/VBR or El Torito no-emulation image) | Implemented | P0 | BI-009 | Low | `crates/firmware/src/bios/post.rs::boot_eltorito` |
| BI-011 | BIOS test suite | Implemented | P0 | BI-001..BI-010 | Medium | Run `cargo test -p firmware` (see tests in `crates/firmware/src/bios/*`) |

#### ACPI Tasks

| ID | Task | Status | Priority | Dependencies | Complexity | Pointers |
|----|------|--------|----------|--------------|------------|----------|
| AC-001 | RSDP/RSDT/XSDT generation | Implemented | P0 | None | Medium | `crates/aero-acpi/src/tables.rs`, `crates/firmware/src/bios/acpi.rs` |
| AC-002 | FADT (Fixed ACPI Description Table) | Implemented | P0 | AC-001 | Medium | `crates/aero-acpi/src/tables.rs` |
| AC-003 | MADT (Multiple APIC Description Table) | Implemented | P0 | AC-001 | Medium | `crates/aero-acpi/src/tables.rs` |
| AC-004 | HPET table | Implemented | P0 | AC-001 | Low | `crates/aero-acpi/src/tables.rs` |
| AC-005 | DSDT (AML bytecode) | Implemented (minimal) | P1 | AC-001 | High | `crates/aero-acpi/src/tables.rs` (AML builder) |
| AC-006 | Power management stubs | Implemented (ACPI PM device) | P1 | AC-002 | Medium | `crates/devices/src/acpi_pm.rs` + FADT/DSDT in `aero-acpi` |
| AC-007 | ACPI test suite | Implemented | P0 | AC-001..AC-006 | Medium | Run `cargo test -p aero-acpi` (see `crates/aero-acpi/tests/*`) |

#### Device Models Tasks

| ID | Task | Status | Priority | Dependencies | Complexity | Pointers |
|----|------|--------|----------|--------------|------------|----------|
| DM-001 | PIC (8259A) | Implemented | P0 | None | Medium | Core model: `crates/aero-interrupts/src/pic8259.rs`; port wrapper + platform registration: `crates/devices/src/pic8259.rs` |
| DM-002 | PIT (8254) | Implemented | P0 | None | Medium | `crates/devices/src/pit8254.rs` |
| DM-003 | CMOS/RTC | Implemented | P0 | None | Medium | `crates/devices/src/rtc_cmos.rs` |
| DM-004 | Local APIC | Implemented (multi-LAPIC topology; BSP-centric delivery) | P0 | None | High | Core model: `crates/aero-interrupts/src/apic/local_apic.rs`; routing glue in `crates/platform/src/interrupts/router.rs`; MMIO adapters in `crates/aero-machine/src/lib.rs` |
| DM-005 | I/O APIC | Implemented | P0 | DM-004 | High | Core model: `crates/aero-interrupts/src/apic/io_apic.rs`; routing glue in `crates/platform/src/interrupts/router.rs`; MMIO adapters in `crates/aero-machine/src/lib.rs` |
| DM-006 | HPET | Implemented | P0 | None | Medium | `crates/devices/src/hpet.rs` |
| DM-007 | PCI configuration space | Implemented | P0 | None | High | `crates/devices/src/pci/*` |
| DM-008 | PCI device enumeration | Implemented | P0 | DM-007 | Medium | `aero_devices::pci::bios_post` + `crates/aero-machine/tests/win7_storage_topology.rs` |
| DM-009 | DMA controller (8237) | Implemented (stub) | P1 | None | Medium | `crates/devices/src/dma.rs` |
| DM-010 | Serial port (16550) | Implemented | P2 | None | Medium | `crates/devices/src/serial.rs` |
| DM-011 | Device models test suite | Implemented | P0 | DM-001..DM-010 | Medium | `cargo test -p aero-devices`, `cargo test -p aero-pc-platform`, `cargo test -p aero-machine` |

#### Virtio PCI Transport Tasks

| ID | Task | Status | Priority | Dependencies | Complexity | Pointers |
|----|------|--------|----------|--------------|------------|----------|
| VTP-001 | Virtio core (virtqueue, feature negotiation) | Implemented | P0 | DM-007 | High | `crates/aero-virtio/src/queue.rs`, `crates/aero-virtio/src/devices/*` |
| VTP-002 | Virtio PCI modern transport | Implemented | P0 | VTP-001, DM-007 | High | `crates/aero-virtio/src/pci.rs` |
| VTP-003 | Virtio PCI legacy transport | Implemented | P0 | VTP-001, DM-007 | High | `crates/aero-virtio/src/pci.rs` |
| VTP-004 | Virtio PCI transitional device | Implemented | P0 | VTP-002, VTP-003 | Medium | `VirtioPciDevice::new_transitional` |
| VTP-005 | Legacy INTx wiring | Implemented | P0 | VTP-003 | Medium | `VirtioPciDevice::irq_level()` + platform INTx routers |
| VTP-006 | MSI-X support | Implemented | P1 | VTP-002, DM-007 | High | Transport MSI-X logic: `crates/aero-virtio/src/pci.rs` (`MsixCapability` + `InterruptSink::signal_msix`). Platform wiring: `crates/aero-pc-platform/src/lib.rs::VirtioPciBar0Mmio::sync_pci_config` + `sync_virtio_msix_from_platform`, and `crates/aero-machine/src/lib.rs::VirtioPciBar0Mmio::sync_pci_command` + `sync_virtio_msix_from_platform` (see tests `crates/aero-pc-platform/tests/pc_platform_virtio_blk_msix*.rs` and `crates/aero-machine/tests/virtio_blk_msix.rs`). |
| VTP-007 | Unit tests | Implemented | P0 | VTP-003 | Medium | `cargo test -p aero-virtio` (see `crates/aero-virtio/tests/*`) |
| VTP-008 | Config option: disable modern | Implemented | P1 | VTP-004 | Low | `VirtioPciOptions::{modern_only,legacy_only,transitional}` |
| VTP-009 | Wire virtio MSI/MSI-X into canonical machine/platform | Implemented | P1 | VTP-006 | High | `aero_pc_platform`: `VirtioPlatformInterruptSink` delivers `MsiMessage` into `PlatformInterrupts::trigger_msi`; `VirtioPciBar0Mmio::sync_pci_config` mirrors PCI command + MSI-X enable/mask into the virtio transport. `aero_machine`: virtio-net/virtio-blk/virtio-input use `VirtioMsixInterruptSink` to deliver `MsiMessage`; `VirtioPciBar0Mmio::sync_pci_command` mirrors PCI command + MSI-X enable/mask (see `crates/aero-machine/tests/virtio_blk_msix.rs` and `crates/aero-machine/tests/virtio_input_msix.rs`). `NoopVirtioInterruptSink` is only used for configurations/devices without interrupt delivery (i.e. `Machine` has no `PlatformInterrupts`). |

#### Canonical machine/platform gaps (actionable)

| ID | Task | Priority | Complexity | Notes / entry points |
|----|------|----------|------------|----------------------|
| MP-001 | SMP: run multiple vCPUs (make bring-up usable for real SMP guests) | P0 | Very High | `cpu_count > 1` is accepted and published via **ACPI MADT + SMBIOS**. `aero_machine::Machine` has basic SMP scaffolding (per-vCPU LAPIC MMIO, INIT/SIPI bring-up, and a cooperative AP run loop), but it is not yet a full SMP scheduler. Remaining work is robust multi-vCPU scheduling/execution (fairness/parallelism), AP↔AP/BSP IPI delivery from guest code, per-vCPU external interrupt injection, and snapshot/time determinism across multiple cores. |
| MP-002 | MSI/MSI-X: unify config-state mirroring in canonical PCI integrations | P1 | High | Message delivery exists (`PlatformInterrupts::trigger_msi`) and is used by AHCI (MSI), NVMe (MSI/MSI-X), xHCI (MSI), and virtio (MSI-X). The remaining integration pain is **keeping device-internal capability state coherent** with the canonical PCI config space (`PciConfigPorts`) (e.g. mirroring MSI/MSI-X enable/mask state into device models when the platform owns PCI config space). Snapshot regression: `crates/aero-machine/tests/machine_snapshot_preserves_msix_enable.rs`. |

If you are looking for impactful integration/boot work today, focus on:

- **SMP / multi-vCPU bring-up** (MP-001)
- **PCI routing hardening**: keep `aero_acpi` DSDT `_PRT`, PCI “Interrupt Line” programming, and the
 runtime INTx/MSI/MSI-X delivery model coherent and snapshot-safe.
- **Snapshot determinism & stability**: ensure device ordering, guest time, and interrupt state are
 reproducible across save/restore (see `../specs/snapshot-format.md` and `crates/aero-machine/tests/`).

---

### Boot Sequence

#### Phase 1: BIOS

```
Power On
 │
 ▼
POST (Power On Self Test)
 │
 ▼
Memory Detection (E820)
 │
 ▼
Interrupt Vector Table Setup
 │
 ▼
BIOS Data Area Setup
 │
 ▼
Boot Device Selection
 │
 ▼
Load boot code:
 - If `boot_drive` is `0xE0..=0xEF`: El Torito CD-ROM boot image (no-emulation) (Win7 install media; see `../areas/platform-and-firmware.md`)
 - Otherwise: MBR / boot sector (HDD/floppy; 512-byte sector)
 │
 ▼
Jump to loaded boot code
```

#### Phase 2: Boot Loader (Windows)

```
Boot Sector (bootmgr)
 │
 ▼
Switch to Protected Mode
 │
 ▼
Load winload.exe
 │
 ▼
Switch to Long Mode (64-bit)
 │
 ▼
Load ntoskrnl.exe + HAL
 │
 ▼
Kernel Initialization
 │
 ▼
Desktop (explorer.exe)
```

---

### Memory Map

PC/Q35 memory map that BIOS must report via E820 (source of truth:
`crates/firmware/src/bios/interrupts.rs::build_e820_map`, constants in
`crates/aero-pc-constants/src/lib.rs`):

```
0x0000_0000 - 0x0009_EFFF 636 KiB Conventional memory (usable)
0x0009_F000 - 0x0009_FFFF 4 KiB EBDA (reserved)
0x000A_0000 - 0x000F_FFFF 384 KiB VGA/BIOS/option ROM window (reserved)

0x0010_0000 - 0xB000_0000 ... Low RAM (usable; clamped to ECAM base)

0xB000_0000 - 0xC000_0000 256 MiB PCIe ECAM / MMCONFIG (reserved)
 - `aero_pc_constants::PCIE_ECAM_BASE = 0xB000_0000`
 - `PCIE_ECAM_SIZE = 0x1000_0000`

0xC000_0000 - 0x1_0000_0000 1 GiB PCI/MMIO hole (reserved; PCI BARs, APIC/HPET, etc.)

0x1_0000_0000 - ... ... High RAM remap (usable, only when RAM > 0xB000_0000)
 High RAM length = `total_ram - 0xB000_0000`
```

When the configured guest RAM size exceeds `PCIE_ECAM_BASE` (`0xB000_0000`), the BIOS reserves the
ECAM window (`0xB000_0000..0xC000_0000`) and the PCI/MMIO hole (`0xC000_0000..0x1_0000_0000`) in
the E820 map. To preserve the configured RAM size, the remainder is remapped above 4 GiB starting
at `0x1_0000_0000`.

This implies the emulator’s RAM backend must be **hole-aware**: guest RAM is not a single
contiguous `[0, total_ram)` region once PCI holes are modeled. Physical addresses in the reserved
holes must not hit RAM (and if not claimed by an MMIO device, should behave like open bus reads:
`0xFF` bytes / all-ones).

Implementation note: in the Rust VM core, this is modeled via `memory::MappedGuestMemory`
(`crates/memory/src/mapped.rs`) and is already applied by the canonical PC memory buses when
`ram_size > PCIE_ECAM_BASE` (see `crates/platform/src/memory.rs::MemoryBus::wrap_pc_high_memory`
and `crates/aero-machine/src/lib.rs::SystemMemory::new`).

---

### Interrupt Routing

#### Legacy (PIC)

```
IRQ 0 - PIT Timer
IRQ 1 - Keyboard
IRQ 2 - Cascade (PIC2)
IRQ 3 - Serial COM2
IRQ 4 - Serial COM1
IRQ 5 - LPT2 / Sound
IRQ 6 - Floppy
IRQ 7 - LPT1
IRQ 8 - RTC
IRQ 9 - Redirected IRQ2
IRQ 10 - Available
IRQ 11 - Available
IRQ 12 - PS/2 Mouse
IRQ 13 - FPU
IRQ 14 - Primary IDE
IRQ 15 - Secondary IDE
```

#### APIC Mode

Windows 7 prefers APIC. The MADT tells the OS about APIC configuration:
- Local APIC ID for each CPU
- I/O APIC address and GSI base
- Interrupt source overrides (e.g., IRQ0 → GSI2)

---

### PCI Device Enumeration

The canonical PCI layout (BDFs, IDs/class codes, and INTx routing) is defined in:

- [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md)
- `aero_devices::pci::profile` (source of truth for the constants used by tests/platform code)

**Important:** The canonical Windows 7 storage topology is normative and requires **ICH9 AHCI at
`00:02.0`** (see [`../areas/storage.md`](../areas/storage.md)). Do not
assign any other device to `00:02.0`.

#### Canonical bus 0 device numbers (when enabled)

This is the canonical BDF reservation map from `aero_devices::pci::profile` (used by tests and the
Windows driver/device contract). **Not every entry is currently wired into `aero_machine::Machine`**
yet; for the authoritative “what can the canonical machine expose today”, see
`crates/aero-machine/src/lib.rs::MachineConfig` (feature flags like `enable_ahci`, `enable_nvme`,
`enable_virtio_*`, `enable_ehci`, etc).

```
00:00.0 - Host bridge (Q35)
00:1f.0 - ISA/LPC bridge (ICH9)

00:01.0 - PIIX3 ISA bridge (multi-function; enables 00:01.1/00:01.2 discovery)
00:01.1 - PIIX3 IDE (Win7 install ISO attachment; legacy compat mode)
00:01.2 - PIIX3 UHCI (USB 1.1)

00:02.0 - ICH9 AHCI (Win7 OS disk; canonical and normative)
00:03.0 - NVMe (optional; off by default for Win7)
00:04.0 - HD Audio (ICH6)
00:05.0 - E1000 NIC
00:06.0 - RTL8139 NIC (alternate)
00:07.0 - AeroGPU display controller (reserved canonical BDF; `PCI\VEN_A3A0&DEV_0001`, see `../specs/aerogpu-device-abi.md`)
00:08.0 - virtio-net
00:09.0 - virtio-blk
00:0a.0 - virtio-input keyboard (multi-function)
00:0a.1 - virtio-input mouse
00:0b.0 - virtio-snd
00:0c.0 - VGA (stub) Bochs/QEMU “Standard VGA” (`1234:1111`; legacy VGA mode only)
00:0d.0 - xHCI (USB 3.x) controller (optional/experimental; Win7 has no in-box xHCI driver; see `../areas/usb-and-input.md`)
00:12.0 - EHCI (USB 2.0) controller (optional; Win7 in-box `usbehci.sys`; see `../areas/usb-and-input.md`)
```

#### Note on VGA / display today

With `MachineConfig::enable_vga=true` (and `enable_aerogpu=false`), the canonical
`aero_machine::Machine` uses `aero_gpu_vga` (VGA + Bochs VBE_DISPI) for boot display:

* Legacy VGA ports: `0x3B0..0x3DF` (includes both mono and color decode ranges; common subset `0x3C0..0x3DF`)
* VBE ports: `0x01CE/0x01CF`
* Legacy VRAM window: `0xA0000..0xBFFFF`
* SVGA linear framebuffer (LFB): base is **configurable** (historically defaulting to `0xE000_0000`
 via `aero_gpu_vga::SVGA_LFB_BASE`).
 - When `MachineConfig::enable_pc_platform=true`, the canonical machine also exposes a
 Bochs/QEMU-compatible “Standard VGA” PCI stub at `00:0c.0` (`1234:1111`) and routes the LFB
 through BAR0 inside the ACPI-reported PCI MMIO window. The BAR base is assigned by BIOS POST /
 the PCI resource allocator unless pinned via `MachineConfig::{vga_lfb_base,vga_vram_bar_base}`.
 - When `MachineConfig::enable_pc_platform=false`, the machine maps the LFB MMIO aperture directly
 at the configured base.

This legacy VGA/VBE boot-display path intentionally does *not* occupy `00:07.0`: that BDF is
reserved for the long-term AeroGPU WDDM device identity (`PCI\VEN_A3A0&DEV_0001`; see
[`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md) and
[`../areas/graphics.md`](../areas/graphics.md)).

With `MachineConfig::enable_aerogpu=true` (and `enable_vga=false`), the canonical machine exposes
the AeroGPU PCI identity (`A3A0:0001`) at `00:07.0` with the canonical BAR layout (BAR0 regs + BAR1
VRAM aperture) for stable Windows driver binding and enumeration. In `aero_machine` today BAR1 is
backed by a dedicated VRAM buffer and implements permissive legacy VGA decode (VGA port I/O +
VRAM-backed `0xA0000..0xBFFFF` window; see `../areas/graphics.md`).

BAR0 is implemented as a **minimal MMIO + ring/fence transport + submission decode/capture** surface.
In the default/no-backend mode, `aero_machine` completes fences without executing ACMD so the in-tree
Win7 KMD can initialize and make forward progress even when no executor is available.

Command execution can be supplied by host-side executors/backends:

- Browser runtime: out-of-process GPU worker execution via the WASM “submission bridge”
 (`Machine::aerogpu_drain_submissions` / `Machine::aerogpu_complete_fence`, exported from
 `crates/aero-wasm`).
- Native builds/tests: optional in-process backends (including a feature-gated headless wgpu backend
 via `Machine::aerogpu_set_backend_wgpu`).

`aero_machine` also implements scanout0 + vblank register storage (including vblank
counters/timestamps and IRQ semantics) and a host presentation path that can read/present the
guest-programmed scanout framebuffer via `Machine::display_present`.

Shared device-side building blocks (regs/ring/executor + reusable PCI wrapper) live in
`crates/aero-devices-gpu`, with a legacy/sandbox integration surface still in `crates/emulator`.

When AeroGPU owns the boot display path, firmware derives the VBE mode-info linear framebuffer base
from AeroGPU BAR1 (`PhysBasePtr = BAR1_BASE + 0x40000`, aka `VBE_LFB_OFFSET` in `aero_machine` /
`AEROGPU_PCI_BAR1_VBE_LFB_OFFSET_BYTES` in the protocol).

In this mode no transitional VGA PCI stub is installed.

The transitional VGA/VBE path is a boot-display stepping stone and does **not** implement the full
AeroGPU WDDM MMIO/ring protocol.

---

### Snapshot/Restore

Snapshots enable "instant boot":

1. Boot Windows 7 once (slow)
2. Save snapshot at desktop
3. Future sessions restore from snapshot (fast)

See [`../specs/snapshot-format.md`](../specs/snapshot-format.md) for format.

---

### Debugging

#### Serial Console

Enable serial output for debug messages:

```rust
// In BIOS or early boot code
fn debug_print(s: &str) {
 for b in s.bytes() {
 io_write(0x3F8, b as u64); // COM1
 }
}
```

#### State Inspection

The emulator exposes CPU/device state for debugging. See [`../areas/debugging.md`](../areas/debugging.md).

---

### Coordination Points

#### What You Need From Other Workstreams

- **CPU (A)**: Working CPU modes, interrupt delivery
- **Graphics (B)**: VGA text mode for boot messages
- **Storage (D)**: AHCI/IDE for disk boot
- **Input (F)**: Keyboard for BIOS interaction
- **Audio (G)**: HD Audio for system sounds

#### What Other Workstreams Need From You

- **All**: Working PCI bus, interrupt routing, timers
- **Drivers (C)**: Virtio PCI device models
- **Graphics (B)**: VGA BIOS INT 10h

---

### Testing

QEMU boot integration tests live under the workspace root `tests/` directory, but are registered
under the dedicated `aero-boot-tests` crate via `crates/aero-boot-tests/Cargo.toml` `[[test]]`
entries (e.g. `path = "../../tests/boot_sector.rs"`). Always run them via `-p aero-boot-tests`
(not `-p aero`).

```bash
# Regenerate/verify deterministic in-repo fixtures (BIOS ROM, ACPI DSDT, tiny boot images).
# CI runs `--check` and fails if any fixture is missing or out-of-date.
bash ./scripts/safe-run.sh cargo xtask fixtures
bash ./scripts/safe-run.sh cargo xtask fixtures --check

# Validate ACPI tables with ACPICA iasl (CI runs this in the `acpi-iasl` job).
# Requires `iasl` to be installed (ACPICA).
bash ./scripts/validate-acpi.sh

# Run BIOS tests
bash ./scripts/safe-run.sh cargo test -p firmware --locked

# Run ACPI table generator tests
bash ./scripts/safe-run.sh cargo test -p aero-acpi --locked

# Run device model tests
bash ./scripts/safe-run.sh cargo test -p aero-devices --locked
bash ./scripts/safe-run.sh cargo test -p aero-interrupts --locked
bash ./scripts/safe-run.sh cargo test -p aero-platform --locked

# Optional: legacy timer/time crates (not used by canonical machine/platform wiring today)
bash ./scripts/safe-run.sh cargo test -p aero-time --locked
bash ./scripts/safe-run.sh cargo test -p aero-timers --locked

# Run canonical integration tests (PCI wiring, snapshots, storage topologies, etc.)
bash ./scripts/safe-run.sh cargo test -p aero-pc-platform --locked
bash ./scripts/safe-run.sh cargo test -p aero-machine --locked
bash ./scripts/safe-run.sh cargo test -p aero-snapshot --locked
bash ./scripts/safe-run.sh cargo test -p aero-virtio --locked

# Boot tests (QEMU; requires qemu-system-i386 (or qemu-system-x86_64), mtools, unzip, curl)
# Note: the first `cargo test` in a clean/contended agent sandbox can take >10 minutes.
# If you hit safe-run timeouts during compilation, bump the timeout via AERO_TIMEOUT.
bash ./scripts/prepare-freedos.sh
# Ensure deterministic in-repo fixtures are present and up-to-date (boot sectors,
# BIOS ROM, ACPI DSDT, etc). CI enforces this via `cargo xtask fixtures --check`;
# no assembler toolchain required.
bash ./scripts/safe-run.sh cargo xtask fixtures --check
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-boot-tests --test boot_sector --locked
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-boot-tests --test freedos_boot --locked

# Full Windows 7 boot (local only; requires a user-supplied Windows 7 disk image)
bash ./scripts/prepare-windows7.sh
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-boot-tests --test windows7_boot --locked -- --ignored
```

---

### What’s missing to boot Windows 7? (checklist)

This checklist is intentionally **practical** and reflects what’s missing *in-repo* vs. what is
already implemented.

#### A) Boot an existing Win7 installation (HDD image)

- [x] BIOS POST + INT services + ACPI/SMBIOS publication (`crates/firmware`, `crates/aero-acpi`)
- [x] Canonical PC platform wiring (interrupt controllers, timers, PCI, storage) (`crates/aero-machine`, `crates/aero-pc-platform`)
- [ ] **A Windows 7 disk image** (local-only; not in repo):
 - Put it at `test-images/local/windows7.img`, or set `AERO_WINDOWS7_IMAGE=/path/to/windows7.img`
 - Then run: `cargo test -p aero-boot-tests --test windows7_boot --locked -- --ignored` (see `scripts/prepare-windows7.sh`)
- [ ] Optional but recommended: provide a golden screenshot at `test-images/local/windows7_login.png` (or `AERO_WINDOWS7_GOLDEN=...`)

#### B) Boot the Win7 installer from ISO (El Torito)

- [x] **BIOS El Torito no-emulation boot + INT 13h extensions** (what Win7 install ISOs use):
 see [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md) and `crates/firmware/src/bios/eltorito.rs`.
- [ ] **A Windows 7 install ISO** (local-only; not in repo):
 - In the agent sandbox this is usually available at `/state/win7.iso` (see `AGENTS.md`)
 - For slipstreaming drivers/certs/testsigning into install media, see `../areas/windows-drivers.md`
- [ ] Host config: attach the ISO as the IDE/ATAPI CD-ROM and set BIOS `boot_drive` to `0xE0..=0xEF` (commonly `0xE0`).
 Normal HDD boot uses `0x80`.

#### C) Important non-boot gaps

- [ ] SMP execution: SMP is still in bring-up. `aero_machine::Machine` has basic AP bring-up +
 cooperative AP execution, but it is not yet a full SMP scheduler. `aero_machine::pc::PcMachine` /
 `aero_pc_platform::PcPlatform` remain BSP-only execution today (MP-001).
- [ ] MSI/MSI-X hardening: message-signaled interrupts are wired for key devices (virtio, NVMe), but patterns/tests need to be generalized and made snapshot-safe as SMP lands (MP-002).

---

### Quick Start Checklist

1. ☐ Read [`AGENTS.md`](./purged-material.md) completely
2. ☐ Run `bash ./scripts/agent-env-setup.sh` and `source ./scripts/agent-env.sh`
3. ☐ Read [`../areas/platform-and-firmware.md`](../areas/platform-and-firmware.md)
4. ☐ Read [`../overview.md`](../overview.md)
5. ☐ Explore `crates/firmware/src/bios/`, `crates/aero-acpi/`, `crates/aero-machine/`, and `crates/aero-pc-platform/`
6. ☐ Regenerate/check deterministic in-repo fixtures: `bash ./scripts/safe-run.sh cargo xtask fixtures --check`
7. ☐ Run the in-process integration tests (`cargo test -p firmware`, `-p aero-machine`, `-p aero-pc-platform`, etc.)
8. ☐ Run QEMU reference boot tests (`boot_sector`, `freedos_boot`) after `bash ./scripts/prepare-freedos.sh`
9. ☐ Pick an item from the “Canonical machine/platform gaps” table (MP-001/MP-002) and begin

---

*Integration makes everything work together. You are the glue.*

## Workstream D: I/O & Storage

> **⚠️ MANDATORY: Read and follow [`AGENTS.md`](./purged-material.md) in its entirety before starting any work.**
>
> AGENTS.md contains critical operational guidance including:
> - Defensive mindset (assume hostile/misbehaving code)
> - Resource limits and `safe-run.sh` usage
> - Windows 7 test ISO location (`/state/win7.iso`)
> - Interface contracts
> - Technology stack decisions
>
> **Failure to follow AGENTS.md will result in broken builds, OOM kills, and wasted effort.**

---

### Overview

This workstream owns **storage emulation**: IDE/AHCI/NVMe disk controllers, disk image abstraction, and the browser storage backends (OPFS, IndexedDB).

Storage is on the **critical boot path**. Windows 7 cannot start without a working disk controller.

---

### Key Crates & Directories

| Crate/Directory | Purpose |
|-----------------|---------|
| `crates/aero-storage/` | Disk image abstraction, caching |
| `crates/aero-storage-server/` | Storage server for remote images |
| `crates/aero-devices-storage/` | IDE/AHCI controller emulation |
| `crates/aero-devices-nvme/` | NVMe controller emulation |
| `crates/aero-opfs/` | Origin Private File System backend |
| `crates/aero-http-range/` | HTTP Range request handling |
| `crates/st-idb/` | IndexedDB async storage backend |
| `apps/web/src/storage/` | TypeScript storage worker / host layer (orchestrates streaming/caching + hosts wasm backends) |

---

### Essential Documentation

**Must read:**

 - [`../areas/storage.md`](../areas/storage.md) — Storage architecture
 - [`../areas/storage.md`](../areas/storage.md) — Canonical Win7 storage topology (PCI BDFs + AHCI/IDE media mapping)
 - [`../areas/storage.md`](../areas/storage.md) — Canonical disk/backend traits + consolidation plan
 - [`../areas/storage.md`](../areas/storage.md) — IndexedDB (async) vs Rust controller (sync) integration plan
 - [`../areas/storage.md`](../areas/storage.md) — Remote disk streaming
 - [`../specs/chunked-disk-image-format.md`](../specs/chunked-disk-image-format.md) — Chunked format

**Reference:**

- [`../areas/storage.md`](../areas/storage.md) — CDN Range request behavior
- [`../areas/storage.md`](../areas/storage.md) — Auth for streaming
- [`../areas/storage.md`](../areas/storage.md) — Backend service

---

### Tasks

#### Storage Controller Tasks

| ID | Task | Priority | Dependencies | Complexity |
|----|------|----------|--------------|------------|
| ST-001 | IDE controller emulation | P0 | None | High |
| ST-002 | AHCI controller emulation | P0 | None | Very High |
| ST-003 | Disk image abstraction | P0 | None | Medium |
| ST-004 | OPFS backend | P0 | ST-003 | Medium |
| ST-005 | IndexedDB fallback backend | P1 | ST-003 | Medium |
| ST-006 | Sector caching | P1 | ST-003 | Medium |
| ST-007 | Sparse disk format | P1 | ST-003 | Medium |
| ST-008 | CD-ROM/ATAPI emulation | P0 | ST-001 | High |
| ST-009 | Virtio-blk device model | P1 | VTP-001..VTP-003 | High |
| ST-010 | Storage test suite | P0 | ST-002 | Medium |

Note: IndexedDB-based storage is async and is not currently exposed as a synchronous
`aero_storage::StorageBackend` / `aero_storage::VirtualDisk`. See
[`../areas/storage.md`](../areas/storage.md) and the canonical trait
mapping in [`../areas/storage.md`](../areas/storage.md).

#### NVMe Tasks

NVMe is an optional high-performance path. AHCI is sufficient for Windows 7.

| ID | Task | Priority | Dependencies | Complexity |
|----|------|----------|--------------|------------|
| NV-001 | NVMe controller registers | P2 | None | High |
| NV-002 | Admin queue implementation | P2 | NV-001 | High |
| NV-003 | I/O queue implementation | P2 | NV-002 | High |
| NV-004 | NVMe test suite | P2 | NV-003 | Medium |

---

### Storage Architecture

#### Layered Design

```
┌─────────────────────────────────────────────┐
│ Windows 7 Guest │
│ │ │
│ AHCI Driver / Virtio-blk Driver │
├─────────────────┼───────────────────────────┤
│ ▼ │
│ AHCI Controller Emulation │ ← ST-002
│ Virtio-blk Device Model │ ← ST-009
│ │ │
│ ▼ │
│ Disk Image Abstraction │ ← ST-003
│ │ │
│ ┌───────┴───────┐ │
│ ▼ ▼ │
│ OPFS Backend HTTP Range │ ← ST-004
│ (local) (remote streaming) │
└─────────────────────────────────────────────┘
```

#### Remote Streaming

For large disk images (20GB+), we stream sectors on demand:

```
Browser
 │
 ▼
HTTP Range GET → CDN → Origin (S3/R2/etc.)
 │
 ▼
Sector Cache (in-memory + OPFS)
 │
 ▼
AHCI Controller
```

Key considerations:
- HTTP Range requests for random access
- Sector-level caching to minimize network requests
- CORS and COEP headers for cross-origin isolation
- Authentication for private disk images

---

### AHCI Implementation Notes

AHCI (Advanced Host Controller Interface) is the SATA controller standard. Key components:

1. **HBA Memory Registers** — PCI BAR5 MMIO
2. **Port Registers** — Per-port command/status
3. **Command List** — Ring of command headers
4. **FIS (Frame Information Structure)** — Data transfer descriptors

Windows 7 uses the inbox `msahci.sys` driver.

Reference: AHCI 1.3.1 specification (publicly available from Intel).

---

### OPFS Backend

Origin Private File System provides fast, large file access in the browser.

In this repo, the primary OPFS backend implementation lives in Rust/wasm32 in
`crates/aero-opfs` (e.g. `aero_opfs::OpfsByteStorage` / `aero_opfs::OpfsBackend`),
which calls the underlying browser OPFS APIs via `wasm-bindgen`.

The TypeScript host layer is still responsible for wiring the worker runtime and may
orchestrate higher-level concerns like remote streaming and cache policy.

Underlying browser API (for reference):

```typescript
// Get OPFS root
const root = await navigator.storage.getDirectory();

// Create/open disk image file
const fileHandle = await root.getFileHandle('disk.img', { create: true });

// Get sync access handle for random access
const accessHandle = await fileHandle.createSyncAccessHandle();

// Read sector
const buffer = new ArrayBuffer(512);
accessHandle.read(buffer, { at: sectorOffset });

// Write sector
accessHandle.write(data, { at: sectorOffset });
```

Rust usage (wasm32):

```rust
use aero_opfs::OpfsByteStorage;
use aero_storage::StorageBackend;

let mut backend = OpfsByteStorage::open("disk.img", true).await?;

let mut sector = [0u8; 512];
backend.read_at(0, &mut sector)?;
backend.write_at(0, &sector)?;
backend.flush()?;
```

---

### Coordination Points

#### Dependencies on Other Workstreams

- **CPU (A)**: AHCI registers accessed via `CpuBus`
- **Integration (H)**: Controller must be wired into PCI bus

#### What Other Workstreams Need From You

- Working AHCI for Windows 7 boot
- CD-ROM for ISO booting
- Virtio-blk device model for driver development (C)

---

### Testing

QEMU boot integration tests live under the workspace root `tests/` directory, but are registered
under the dedicated `aero-boot-tests` crate via `crates/aero-boot-tests/Cargo.toml` `[[test]]`
entries (e.g. `path = "../../tests/freedos_boot.rs"`). Always run them via `-p aero-boot-tests`
(not `-p aero`).

```bash
# Run storage tests
bash ./scripts/safe-run.sh cargo test -p aero-storage --locked
bash ./scripts/safe-run.sh cargo test -p aero-devices-storage --locked
bash ./scripts/safe-run.sh cargo test -p aero-opfs --locked
bash ./scripts/safe-run.sh cargo test -p aero-io-snapshot --locked

# Snapshot layer should compile on wasm32 (e.g. for OPFS-backed disks).
bash ./scripts/safe-run.sh cargo check --target wasm32-unknown-unknown -p aero-io-snapshot --locked

# Compile-check selected crates for wasm32-unknown-unknown (no test runner required).
# This helps catch `Send`/threading bound regressions that break browser storage backends.
bash ./scripts/safe-run.sh cargo check --target wasm32-unknown-unknown -p aero-devices-storage -p aero-machine --locked

# Integration tests
# Note: the first run in a clean/contended agent sandbox can take >10 minutes to compile.
 AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-boot-tests --test freedos_boot --locked
```

---

### Quick Start Checklist

1. ☐ Read [`AGENTS.md`](./purged-material.md) completely
2. ☐ Run `bash ./scripts/agent-env-setup.sh` and `source ./scripts/agent-env.sh`
3. ☐ Read [`../areas/storage.md`](../areas/storage.md)
4. ☐ Explore `crates/aero-storage/src/` and `crates/aero-devices-storage/src/`
5. ☐ Run existing tests to establish baseline
6. ☐ Pick a task from the tables above and begin

---

*Storage is the foundation. Nothing works without a working disk.*

## Workstream E: Network & Proxy

> **⚠️ MANDATORY: Read and follow [`AGENTS.md`](./purged-material.md) in its entirety before starting any work.**
>
> AGENTS.md contains critical operational guidance including:
> - Defensive mindset (assume hostile/misbehaving code)
> - Resource limits and `safe-run.sh` usage
> - Windows 7 test ISO location (`/state/win7.iso`)
> - Interface contracts
> - Technology stack decisions
>
> **Failure to follow AGENTS.md will result in broken builds, OOM kills, and wasted effort.**

---

### Overview

This workstream owns **network emulation**: the E1000 NIC device model, virtio-net, and the external proxies that bridge guest network traffic to the real internet (TCP over WebSocket, UDP over WebRTC).

Network connectivity is important for Windows activation, updates, and general usability.

---

### Key Crates & Directories

| Crate/Directory | Purpose |
|-----------------|---------|
| `crates/aero-net-e1000/` | Intel E1000 NIC emulation |
| `crates/aero-net-pump/` | Bounded per-tick NIC↔backend frame pumping glue (shared integration logic) |
| `crates/aero-net-backend/` | Minimal host-side network backend trait + L2 tunnel backends (queue + NET_TX/NET_RX ring) |
| `crates/aero-net-stack/` | User-space TCP/IP stack |
| `crates/aero-l2-protocol/` | L2 frame protocol |
| `crates/aero-l2-proxy/` | L2 tunnel proxy |
| `proxy/` | Go-based network proxies |
| `proxy/webrtc-udp-relay/` | WebRTC UDP relay |
| `services/net-proxy/` | Local development proxy (TCP/UDP relay + DoH) |
| `services/gateway/` | Production gateway service |

---

### Essential Documentation

**Must read:**

- [`../areas/networking.md`](../areas/networking.md) — Network architecture
- [`../specs/gateway-api.md`](../specs/gateway-api.md) — Gateway API spec

**Reference:**

- [`proxy/webrtc-udp-relay/PROTOCOL.md`](../../proxy/webrtc-udp-relay/PROTOCOL.md) — UDP relay protocol
- [`../overview.md`](../overview.md) — System architecture

---

### Tasks

#### Network Device Tasks

| ID | Task | Priority | Dependencies | Complexity |
|----|------|----------|--------------|------------|
| NT-001 | E1000 NIC emulation | P0 | None | Very High |
| NT-002 | Packet receive/transmit | P0 | NT-001 | Medium |
| NT-003 | User-space network stack | P0 | None | High |
| NT-004 | DHCP client | P0 | NT-003 | Medium |
| NT-005 | DNS resolution (DoH) | P1 | None | Medium |
| NT-006 | WebSocket TCP proxy | P0 | None | Medium |
| NT-007 | WebRTC UDP proxy | P1 | None | High |
| NT-008 | Virtio-net device model | P1 | VTP-001..VTP-003 | High |
| NT-009 | Network test suite | P0 | NT-001 | Medium |

---

### Network Architecture

#### L2 Tunneling (Recommended)

The emulator forwards raw Ethernet frames to an external proxy that runs the TCP/IP stack:

```
┌─────────────────────────────────────────────┐
│ Windows 7 Guest │
│ │ │
│ E1000 / Virtio-net Driver │
├─────────────────┼───────────────────────────┤
│ ▼ │
│ E1000 Device Model │
│ Virtio-net Device Model │
│ │ │
│ ▼ │
│ L2 Frame Protocol │
│ │ │
└─────────────────┼───────────────────────────┘
 │ WebSocket
 ▼
┌─────────────────────────────────────────────┐
│ Aero Gateway │
│ │ │
│ L2 Proxy + TCP/IP Stack │
│ │ │
│ ┌───────┴───────┐ │
│ ▼ ▼ │
│ TCP Proxy UDP Relay │
│ (WebSocket) (WebRTC) │
└─────────────────────────────────────────────┘
```

#### Why L2 Tunneling?

- **Browser limitations**: Browsers cannot create raw sockets
- **Proxy handles stack**: TCP/IP stack runs in the proxy, not the browser
- **Security**: Proxy can enforce policies (rate limits, blocked destinations)

---

### E1000 Implementation Notes

Intel E1000 is a well-documented Gigabit Ethernet controller. Key components:

1. **PCI Configuration** — Standard PCI device
2. **Register Set** — MMIO BAR0
3. **TX Descriptor Ring** — Transmit packets
4. **RX Descriptor Ring** — Receive packets
5. **Interrupts** — MSI or INTx

Windows 7 has an inbox E1000 driver (`e1000325.sys`).

Reference: Intel 8254x PRM (publicly available).

---

### Proxy Architecture

#### TCP Proxy (WebSocket)

```
Browser Gateway
 │ │
 │ WebSocket CONNECT │
 │ ──────────────────────────────────▶│
 │ │
 │ TCP handshake │──▶ Real TCP connection
 │ ◀──────────────────────────────────│
 │ │
 │ Bidirectional data │
 │ ◀─────────────────────────────────▶│
```

#### UDP Relay (WebRTC)

```
Browser Gateway
 │ │
 │ WebRTC DataChannel │
 │ ──────────────────────────────────▶│
 │ │
 │ UDP frames (encapsulated) │──▶ Real UDP packets
 │ ◀─────────────────────────────────▶│
```

Inbound filtering note: `proxy/webrtc-udp-relay` defaults to `UDP_INBOUND_FILTER_MODE=address_and_port`
(only accept inbound UDP from remote address+port tuples the guest previously sent to). You can switch
to full-cone behavior with `UDP_INBOUND_FILTER_MODE=any` (**less safe**; see the relay README).

WebRTC DataChannel DoS hardening note: the relay configures pion/SCTP limits to prevent malicious peers
from sending extremely large WebRTC DataChannel messages that would otherwise be buffered/allocated
before `DataChannel.OnMessage` runs. Relevant knobs:

- `WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES` (SDP `a=max-message-size` hint; 0 = auto)
- `WEBRTC_SCTP_MAX_RECEIVE_BUFFER_BYTES` (hard receive-side cap; 0 = auto; must be ≥ `WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES` and ≥ `1500`)
- `WEBRTC_SESSION_CONNECT_TIMEOUT` (close sessions that never reach a connected state; prevents PeerConnection leaks; default `30s`)

---

### Aero Gateway

The production gateway (`services/gateway/`) provides:

- TCP proxy endpoint (`/tcp`)
- UDP relay (WebRTC signaling)
- DoH endpoint for DNS
- Health/readiness endpoints
- Rate limiting and access control

See [`../specs/gateway-api.md`](../specs/gateway-api.md) for the API spec.

---

### Local Development

For local development, use `services/net-proxy/`:

```bash
# From the repo root (npm workspaces)
npm ci

# Start the proxy (safe-by-default: only allows public/unicast targets)
npm -w services/net-proxy run dev

# Or: trusted local dev mode (allows localhost/private ranges for /tcp, /tcp-mux, /udp)
# AERO_PROXY_OPEN=1 npm -w services/net-proxy run dev
```

This provides a local proxy that the emulator (and the browser networking clients under `apps/web/src/net`) can connect to for testing:

- `GET /healthz`
- `GET|POST /dns-query` and `GET /dns-json` (DNS-over-HTTPS)
- `WS /tcp`, `WS /tcp-mux`, `WS /udp`

Notes:

- DoH endpoints are normal `fetch()` calls, so browser clients generally need them to be **same-origin** (or served with
 permissive CORS). The easiest local-dev approach is proxying `/dns-query` + `/dns-json` through Vite; alternatively,
 `net-proxy` supports an explicit CORS allowlist via `AERO_PROXY_DOH_CORS_ALLOW_ORIGINS` (see `services/net-proxy/README.md`).
- `net-proxy` DoH endpoints are intentionally lightweight and are **unauthenticated** (no session cookie) and not
 policy-filtered by `AERO_PROXY_OPEN` / `AERO_PROXY_ALLOW` (the policy applies to `/tcp`, `/tcp-mux`, `/udp`).

See [`services/net-proxy/README.md`](../decisions/README.md) for full details (allowlist policy, URL formats, and DoH examples).

---

### Coordination Points

#### Dependencies on Other Workstreams

- **CPU (A)**: NIC registers accessed via `CpuBus`
- **Integration (H)**: NIC must be wired into PCI bus

#### What Other Workstreams Need From You

- Working network for Windows activation/updates
- Virtio-net device model for driver development (C)

---

### Testing

```bash
# Run network tests
bash ./scripts/safe-run.sh cargo test -p aero-net-e1000 --locked
bash ./scripts/safe-run.sh cargo test -p aero-net-stack --locked
bash ./scripts/safe-run.sh cargo test -p aero-l2-proxy --locked

# Note: `aero-l2-proxy` can be slow to compile in shared/contended sandboxes.
# If `safe-run` times out, retry with a larger timeout and/or isolate Cargo state:
# AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo test -p aero-l2-proxy --locked
# AERO_ISOLATE_CARGO_HOME=1 bash ./scripts/safe-run.sh cargo test -p aero-l2-proxy --locked

# Run gateway tests
cd services/gateway
npm test
```

---

### Quick Start Checklist

1. ☐ Read [`AGENTS.md`](./purged-material.md) completely
2. ☐ Run `bash ./scripts/agent-env-setup.sh` and `source ./scripts/agent-env.sh`
3. ☐ Read [`../areas/networking.md`](../areas/networking.md)
4. ☐ Explore `crates/aero-net-e1000/src/` and `proxy/`
5. ☐ Run local proxy to test connectivity
6. ☐ Pick a task from the tables above and begin

---

*Network makes the emulator useful. Without it, Windows 7 is an island.*

## Workstream F: USB & Input

> **⚠️ MANDATORY: Read and follow [`AGENTS.md`](./purged-material.md) in its entirety before starting any work.**
>
> AGENTS.md contains critical operational guidance including:
> - Defensive mindset (assume hostile/misbehaving code)
> - Resource limits and `safe-run.sh` usage
> - Windows 7 test ISO location (`/state/win7.iso`)
> - Interface contracts
> - Technology stack decisions
>
> **Failure to follow AGENTS.md will result in broken builds, OOM kills, and wasted effort.**

---

### Overview

This workstream owns **input emulation** and **USB passthrough**: PS/2 keyboard/mouse, USB HID devices, UHCI/EHCI controllers, and browser-to-guest input event forwarding.

Input is essential for usability. Without keyboard/mouse, the emulator is unusable.

#### Two runtime shapes (important)

Aero supports input through two different integration styles:

- **Canonical full-system VM (`aero_machine::Machine`, exported to JS as `crates/aero-wasm::Machine`)**
 - One object owns CPU + devices.
 - Used by native tests and by the JS/WASM “single machine” API.
 - Input is injected directly via `Machine.inject_*`:
 - PS/2 (i8042): `inject_browser_key`, `inject_mouse_motion`, etc.
 - virtio-input (optional): `inject_virtio_key/rel/button/wheel` once enabled.
 - synthetic USB HID devices (keyboard/mouse/gamepad/consumer-control): `inject_usb_hid_*` (enabled by default for `new api.Machine(...)` in the WASM wrapper; native config is opt-in via `MachineConfig.enable_synthetic_usb_hid`).
 - Virtio-input is opt-in:
 - Native: `MachineConfig.enable_virtio_input = true` (requires `enable_pc_platform = true`).
 - JS/WASM: `api.Machine.new_with_options(..., { enable_virtio_input: true })`.
 - Canonical BDFs: `00:0A.0` (keyboard) and `00:0A.1` (mouse).
- **Browser worker runtime (production)**
 - Main thread captures browser events and batches them in `apps/web/src/input/*`.
 - The worker that injects input depends on `vmRuntime`:
 - `vmRuntime=legacy`: the **I/O worker** (`apps/web/src/workers/io.worker.ts`) owns guest device models and routes input to:
 - **virtio-input** (fast path, once the guest driver sets `DRIVER_OK`)
 - **synthetic USB HID devices behind the guest-visible USB controller** (when enabled/available; UHCI by default, with EHCI/xHCI fallbacks in some WASM builds)
 - **PS/2 i8042** fallback (via the `aero-devices-input` model / equivalents)
 - `vmRuntime=machine`: the **machine CPU worker** (`apps/web/src/workers/machine_cpu.worker.ts`) owns the canonical `api.Machine` instance and injects input directly (including backend selection/routing). The I/O worker runs in host-only stub mode and does not own guest input devices.

---

### Key Crates & Directories

| Crate/Directory | Purpose |
|-----------------|---------|
| `crates/aero-machine/` | Canonical full-system VM (`aero_machine::Machine`) |
| `crates/aero-wasm/` | WASM exports (`Machine`, virtio-input core, device bridges) |
| `crates/aero-usb/` | Canonical USB stack (the canonical USB stack decision) |
| `crates/aero-devices-input/` | PS/2 controller (i8042), keyboard, mouse |
| `apps/web/src/workers/io.worker.ts` | I/O worker routing (PS/2 vs USB HID vs virtio-input) |
| `apps/web/src/workers/machine_cpu.worker.ts` | Machine runtime input injection/routing (PS/2 vs USB HID vs virtio-input) |
| `apps/web/src/io/devices/` | Browser-side device models (i8042, UHCI, virtio-input, …) |
| `apps/web/src/usb/` | TypeScript USB broker and passthrough |
| `apps/web/src/input/` | Browser input event capture |

---

### Essential Documentation

**Must read:**

- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — Input architecture
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — USB HID usages and reports
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — EHCI (USB 2.0) emulation contracts (regs, root hub, IRQ, snapshot plan)
- [`../decisions/0015-canonical-usb-stack.md`](../decisions/0015-canonical-usb-stack.md) — USB stack design

**Reference:**

- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — Passthrough architecture
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — HID descriptor synthesis
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — WebUSB passthrough
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — xHCI (USB 3.x) controller emulation (in progress)
- [`../areas/usb-and-input.md`](../areas/usb-and-input.md) — Gamepad support

---

### Tasks

#### Input Device Tasks

| ID | Task | Priority | Dependencies | Complexity |
|----|------|----------|--------------|------------|
| IN-001 | PS/2 controller (i8042) | P0 | None | Medium |
| IN-002 | PS/2 keyboard | P0 | IN-001 | Medium |
| IN-003 | PS/2 mouse | P0 | IN-001 | Medium |
| IN-004 | Scancode translation | P0 | None | Medium |
| IN-005 | Browser event capture | P0 | None | Medium |
| IN-006 | Pointer Lock integration | P0 | IN-005 | Low |
| IN-007 | USB HID (keyboard) | P2 | None | Medium |
| IN-008 | USB HID (mouse) | P2 | None | Medium |
| IN-009 | Gamepad support | P2 | None | Medium |
| IN-010 | Input test suite | P0 | IN-001..IN-003 | Medium |
| IN-011 | Virtio-input device model | P1 | DM-008, VTP-002 | High |
| IN-012 | Windows 7 virtio-input driver | P1 | VIO-001..VIO-003 | Very High |
| IN-013 | HID report descriptor + mapping | P1 | IN-012 | High |
| IN-014 | Driver packaging/signing | P1 | IN-012, IN-013 | Medium |
| IN-015 | Browser → virtio-input events | P1 | IN-005, IN-011 | Medium |
| IN-016 | Virtio-input test plan | P1 | IN-011..IN-015 | Medium |

---

### Input Architecture

#### Canonical browser runtime input pipeline (main thread → input injector worker)

```
Browser DOM events
 → `apps/web/src/input/*` capture + batching
 → `postMessage({ type: "in:input-batch", buffer })`
 → input injector worker:
 - `vmRuntime=legacy`: `apps/web/src/workers/io.worker.ts`
 - `vmRuntime=machine`: `apps/web/src/workers/machine_cpu.worker.ts`
 (route + inject)
 → virtio-input (fast path) OR USB HID (guest-visible USB controller; UHCI by default) OR PS/2 i8042 (fallback)
 → Windows 7 guest input stacks
```

Routing policy (high level):

- **Keyboard:** virtio-input (when `DRIVER_OK`) → synthetic USB keyboard (once configured) → PS/2 i8042
- **Mouse:** virtio-input (when `DRIVER_OK`) → PS/2 until the synthetic USB mouse is configured → synthetic USB mouse (once configured; or if PS/2 is unavailable)
- **Gamepad:** synthetic USB gamepad (no PS/2 fallback)

#### USB HID devices behind the external hub (synthetic + passthrough)

The browser runtime can expose input as guest-visible USB HID devices in two ways:

- **Synthetic HID devices** (keyboard/mouse/gamepad/consumer-control) attached behind the external hub on root port 0 (see `apps/web/src/usb/uhci_external_hub.ts` and the attachment logic in `apps/web/src/workers/io.worker.ts`).
- **Physical device passthrough** via WebHID/WebUSB, bridged into the guest-visible USB controller topology (see `../areas/usb-and-input.md`).

Guest-visible topology (external hub on root port 0):

- root port 0: external hub (synthetic HID devices + WebHID passthrough)
- root port 1: reserved for WebUSB passthrough
- External hub ports:
 - ports 1..4 reserved for synthetic keyboard/mouse/gamepad/consumer-control
 - dynamic passthrough ports start at 5 (`UHCI_EXTERNAL_HUB_FIRST_DYNAMIC_PORT`)
- Note: when the external hub is hosted behind xHCI, hub port numbers must be <= **15** (xHCI Slot
 Context Route String encodes downstream hub ports as 4-bit values). The web runtime clamps hub port
 counts accordingly so the topology remains representable under xHCI.

Note: the canonical `aero_machine::Machine` only auto-attaches the external hub + synthetic HID
devices when `MachineConfig.enable_synthetic_usb_hid = true` (or via the WASM wrapper helper
`Machine.new_with_input_backends(..., enableSyntheticUsbHid=true)`). Enabling
`MachineConfig.enable_uhci` by itself only attaches the UHCI controller; hosts can still attach
device models explicitly via:
 - UHCI: `Machine.usb_attach_*` (when `MachineConfig.enable_uhci` is enabled)
 - EHCI: `Machine.usb_ehci_attach_*` (when `MachineConfig.enable_ehci` is enabled)
 - xHCI: `Machine.usb_xhci_attach_*` (when `MachineConfig.enable_xhci` is enabled)

---

### Scancode Translation

DOM `KeyboardEvent.code` is mapped to PS/2 **Set 2** scancode bytes via a single source-of-truth table:

- `tools/gen_scancodes/scancodes.json`

Generated outputs:

- `scancodes.ts` in `apps/web/src/input/` (browser capture)
- `crates/aero-devices-input/src/scancodes_generated.rs` (Rust/WASM)

Native Rust harnesses consume the mapping via `aero-devices-input` (there is no longer a separate generated harness copy).

In the web runtime, capture uses `ps2Set2ScancodeForCode` from `apps/web/src/input/scancode.ts`. On the Rust side, the canonical helper is `aero_devices_input::scancode::browser_code_to_set2_bytes`.

---

### USB Stack (aero-usb)

Per the canonical USB stack decision, `crates/aero-usb` is the **canonical USB stack**. It provides:

- UHCI controller emulation (USB 1.1, full/low-speed)
- EHCI bring-up (USB 2.0, high-speed; regs + root hub + minimal async/periodic schedule engines)
- xHCI bring-up (USB 3.x; in progress)
- USB device enumeration
- HID class driver
- Passthrough bridge for WebUSB/WebHID

```rust
// Simplified USB stack interface
pub trait UsbController {
 fn attach_device(&mut self, device: Box<dyn UsbDevice>);
 fn detach_device(&mut self, port: u8);
 fn process_frame(&mut self);
}

pub trait UsbDevice {
 fn handle_control(&mut self, setup: SetupPacket) -> ControlResult;
 fn handle_bulk_in(&mut self, endpoint: u8) -> BulkResult;
 fn handle_bulk_out(&mut self, endpoint: u8, data: &[u8]) -> BulkResult;
}
```

---

### Browser Event Capture

The canonical browser capture implementation lives in `apps/web/src/input/`:

- `apps/web/src/input/input_capture.ts`: installs listeners (focus/blur, Pointer Lock, keyboard/mouse/wheel, optional Gamepad polling) and flushes input at a fixed rate (default 125Hz).
- `apps/web/src/input/event_queue.ts`: packs events into an `Int32Array`-compatible `ArrayBuffer` and sends batches to the active input injector worker (`vmRuntime=legacy`: I/O worker; `vmRuntime=machine`: machine CPU worker).

The input injector worker (`io.worker.ts` in `vmRuntime=legacy`, `machine_cpu.worker.ts` in `vmRuntime=machine`) consumes the batches (`type: "in:input-batch"`), decodes `InputEventType`, and injects into the active backend (PS/2, USB HID, or virtio-input).

#### Input Diagnostics Panel (`?input=1`)

The main web UI can optionally show a live **input diagnostics** panel (useful for debugging stuck keys/buttons, backend switching, and input-batch latency).

- Enable: add `?input=1` (or any truthy `?input` value, including just `?input`) to the URL.
- Disable: `?input=0` or `?input=false`.

The panel reports (among other things):

- active keyboard/mouse backend (`ps2` / `usb` / `virtio`)
- held key count + held mouse button mask
- per-backend keyboard LED masks (USB / virtio / PS/2)
- input batch counters and latency stats (avg/EWMA/max)

---

### Coordination Points

#### Dependencies on Other Workstreams

- **CPU (A)**: i8042 registers accessed via `CpuBus::io_read/io_write`
- **Integration (H)**: USB controllers wired into PCI bus

#### What Other Workstreams Need From You

- Working keyboard/mouse for all other testing
- USB HID for more complex input scenarios

---

### Testing

```bash
# Run the USB/input-focused test suite (Rust + targeted web unit tests).
# (Assumes Node deps are installed; run `npm ci` from repo root if needed.)
# If your Node workspace entrypoint is `web/` (instead of the repo root), use:
# cargo xtask input --node-dir web
# cargo xtask input --web-dir web
# Note: by default this runs a focused subset of `aero-usb` tests (UHCI + external hub + EHCI +
# EHCI snapshot roundtrip + USB2 companion routing + WebUSB passthrough (UHCI + xHCI) + key HID
# snapshot compatibility/clamping tests + shared HID usage fixtures + xHCI bring-up smoke/reg-gating).
# Use `--usb-all` if you want to run the full `aero-usb` integration suite (all xHCI tests, etc).
cargo xtask input

# If you run Node tooling from `web/` directly (or your `node_modules/` live under `web/`),
# you can point `cargo xtask input` at that workspace:
cargo xtask input --node-dir web
# (Equivalent env var forms.)
AERO_NODE_DIR=web cargo xtask input
AERO_WEB_DIR=web cargo xtask input
WEB_DIR=web cargo xtask input

# Run only the Rust USB/input tests (skips Node + Playwright; does not require `node_modules`).
cargo xtask input --rust-only

# Run the full USB stack test suite (all `aero-usb` integration tests; can be slow).
cargo xtask input --usb-all

# Also run the canonical machine integration tests (snapshot + USB container wiring).
cargo xtask input --machine

# Also run the targeted WASM USB/input regression tests (runs in Node; does not require `node_modules`).
# Note: `cargo xtask input --wasm` enforces the Node.js *major* version from `.nvmrc` (CI baseline).
# If you need to bypass the check (unsupported Node; e.g. sandbox), you can run:
# AERO_ALLOW_UNSUPPORTED_NODE=1 cargo xtask input --wasm --rust-only
# but expect wasm-pack tooling to be flaky/hang in unsupported Node releases.
cargo xtask input --wasm --rust-only

# Also run the aero-wasm input integration smoke tests (public Machine API surface + backend wiring).
# This is a host-side Rust test suite (does not use wasm-pack) and can be run without `node_modules`.
cargo xtask input --rust-only --with-wasm

# Targeted WASM USB/input regression tests (run in Node).
wasm-pack test --node crates/aero-wasm --test webusb_uhci_bridge --locked
wasm-pack test --node crates/aero-wasm --test uhci_controller_topology --locked
wasm-pack test --node crates/aero-wasm --test uhci_runtime_webusb --locked
wasm-pack test --node crates/aero-wasm --test uhci_runtime_webusb_drain_actions --locked
wasm-pack test --node crates/aero-wasm --test uhci_runtime_topology --locked
wasm-pack test --node crates/aero-wasm --test uhci_runtime_external_hub --locked
wasm-pack test --node crates/aero-wasm --test uhci_runtime_snapshot_roundtrip --locked
wasm-pack test --node crates/aero-wasm --test ehci_controller_bridge_snapshot_roundtrip --locked
wasm-pack test --node crates/aero-wasm --test ehci_controller_topology --locked
wasm-pack test --node crates/aero-wasm --test webusb_ehci_passthrough_harness --locked
wasm-pack test --node crates/aero-wasm --test xhci_webusb_bridge --locked
wasm-pack test --node crates/aero-wasm --test xhci_controller_bridge --locked
wasm-pack test --node crates/aero-wasm --test xhci_controller_bridge_topology --locked
wasm-pack test --node crates/aero-wasm --test xhci_controller_bridge_webusb --locked
wasm-pack test --node crates/aero-wasm --test xhci_controller_topology --locked
wasm-pack test --node crates/aero-wasm --test xhci_topology --locked
wasm-pack test --node crates/aero-wasm --test xhci_step_frames_clamp --locked
wasm-pack test --node crates/aero-wasm --test xhci_step_frames_clamping --locked
wasm-pack test --node crates/aero-wasm --test xhci_bme_event_ring --locked
wasm-pack test --node crates/aero-wasm --test xhci_webusb_snapshot --locked
wasm-pack test --node crates/aero-wasm --test xhci_snapshot --locked
wasm-pack test --node crates/aero-wasm --test usb_bridge_snapshot_roundtrip --locked
wasm-pack test --node crates/aero-wasm --test usb_snapshot --locked
wasm-pack test --node crates/aero-wasm --test machine_input_injection_wasm --locked
wasm-pack test --node crates/aero-wasm --test wasm_machine_ps2_mouse --locked
wasm-pack test --node crates/aero-wasm --test usb_hid_bridge_keyboard_reports_wasm --locked
wasm-pack test --node crates/aero-wasm --test usb_hid_bridge_mouse_reports_wasm --locked
wasm-pack test --node crates/aero-wasm --test usb_hid_bridge_consumer_reports_wasm --locked
wasm-pack test --node crates/aero-wasm --test webhid_interrupt_out_policy_wasm --locked
wasm-pack test --node crates/aero-wasm --test webhid_report_descriptor_synthesis_wasm --locked

# Note: `wasm-pack test` currently builds *all* `crates/aero-wasm` integration tests, even if you
# pass `--test ...`. This means compile errors in unrelated WASM tests (e.g. other bridge tests)
# can still break this command.

# Canonical machine library tests (covers snapshot + USB container wiring).
cargo test -p aero-machine --lib --locked

# Canonical USB stack tests (catches UHCI/EHCI/xHCI regressions).
cargo test -p aero-usb --locked

# Lint: CI treats clippy warnings as errors (`-D warnings`), including in tests.
# If you're iterating on USB/input code, running these focused checks locally can save time:
cargo clippy -p aero-usb --tests --locked -- -D warnings
cargo clippy -p aero-devices-input --tests --locked -- -D warnings

# xHCI gotcha: transfer-ring execution is gated on `USBCMD.RUN`. If you're writing a unit test that
# rings xHCI doorbells and expects DMA to occur, make sure to set RUN first (see existing xHCI tests
# for the `ctrl.mmio_write(regs::REG_USBCMD, 4, u64::from(regs::USBCMD_RUN))` pattern).

# Optional: also run a small input-focused Playwright subset.
# (Defaults to Chromium + 1 worker; sets `AERO_WASM_PACKAGES=core` unless already configured.)
cargo xtask input --e2e

# If you're running in a constrained sandbox, consider using safe-run:
bash ./scripts/safe-run.sh cargo xtask input
# If your Node workspace is `web/`, you can also use:
AERO_NODE_DIR=web bash ./scripts/safe-run.sh cargo xtask input
# (Or: AERO_WEB_DIR=web / WEB_DIR=web)
bash ./scripts/safe-run.sh cargo xtask input --rust-only
bash ./scripts/safe-run.sh wasm-pack test --node crates/aero-wasm \
 --test webusb_uhci_bridge \
 --test uhci_controller_topology \
 --test uhci_runtime_webusb \
 --test uhci_runtime_webusb_drain_actions \
 --test uhci_runtime_topology \
 --test uhci_runtime_external_hub \
 --test uhci_runtime_snapshot_roundtrip \
 --test ehci_controller_bridge_snapshot_roundtrip \
 --test ehci_controller_topology \
 --test webusb_ehci_passthrough_harness \
 --test xhci_webusb_bridge \
 --test xhci_controller_bridge \
 --test xhci_controller_bridge_topology \
 --test xhci_controller_bridge_webusb \
 --test xhci_controller_topology \
 --test xhci_topology \
 --test xhci_step_frames_clamp \
 --test xhci_step_frames_clamping \
 --test xhci_bme_event_ring \
 --test xhci_webusb_snapshot \
 --test xhci_snapshot \
 --test usb_bridge_snapshot_roundtrip \
 --test usb_snapshot \
 --test machine_input_injection_wasm \
 --test wasm_machine_ps2_mouse \
 --test usb_hid_bridge_keyboard_reports_wasm \
 --test usb_hid_bridge_mouse_reports_wasm \
 --test usb_hid_bridge_consumer_reports_wasm \
 --test webhid_interrupt_out_policy_wasm \
 --test webhid_report_descriptor_synthesis_wasm \
 --locked

# Note: `safe-run.sh` defaults to a 10-minute timeout (`AERO_TIMEOUT=600`). On a cold build,
# `cargo xtask input` can exceed this, and `wasm-pack test` can be substantially slower (it may
# rebuild many targets even if you pass `--test ...`). Bump the timeout if you see a timeout kill:
AERO_TIMEOUT=1200 bash ./scripts/safe-run.sh cargo xtask input --rust-only
# For wasm-pack, 20 minutes is sometimes still not enough on a very cold build.
AERO_TIMEOUT=2400 bash ./scripts/safe-run.sh wasm-pack test --node crates/aero-wasm \
 --test webusb_uhci_bridge \
 --test uhci_controller_topology \
 --test uhci_runtime_webusb \
 --test uhci_runtime_webusb_drain_actions \
 --test uhci_runtime_topology \
 --test uhci_runtime_external_hub \
 --test uhci_runtime_snapshot_roundtrip \
 --test ehci_controller_bridge_snapshot_roundtrip \
 --test ehci_controller_topology \
 --test webusb_ehci_passthrough_harness \
 --test xhci_webusb_bridge \
 --test xhci_controller_bridge \
 --test xhci_controller_bridge_topology \
 --test xhci_controller_bridge_webusb \
 --test xhci_controller_topology \
 --test xhci_topology \
 --test xhci_step_frames_clamp \
 --test xhci_step_frames_clamping \
 --test xhci_bme_event_ring \
 --test xhci_webusb_snapshot \
 --test xhci_snapshot \
 --test usb_bridge_snapshot_roundtrip \
 --test usb_snapshot \
 --test machine_input_injection_wasm \
 --test wasm_machine_ps2_mouse \
 --test usb_hid_bridge_keyboard_reports_wasm \
 --test usb_hid_bridge_mouse_reports_wasm \
 --test usb_hid_bridge_consumer_reports_wasm \
 --test webhid_interrupt_out_policy_wasm \
 --test webhid_report_descriptor_synthesis_wasm \
 --locked

# You can also limit web wasm-pack builds to the core runtime package (useful for Playwright E2E):
AERO_WASM_PACKAGES=core npm -w web run wasm:build

# --- Manual / debugging (run pieces individually) ---

# Rust device-model tests
bash ./scripts/safe-run.sh cargo test -p aero-devices-input --locked
# Fast focused subset (matches `cargo xtask input` default; see `cargo xtask input --help` for the canonical list):
bash ./scripts/safe-run.sh cargo test -p aero-usb --locked \
 --test uhci \
 --test uhci_external_hub \
 --test ehci \
 --test ehci_ports \
 --test ehci_snapshot_roundtrip \
 --test usb2_companion_routing \
 --test usb2_port_mux_speed \
 --test usb2_port_mux_remote_wakeup \
 --test usb2_mux_non_owner_writes \
 --test hid_remote_wakeup \
 --test hid_idle_rate \
 --test webusb_passthrough_uhci \
 --test webusb_passthrough_speed \
 --test passthrough_validation \
 --test hid_builtin_snapshot \
 --test hid_composite_mouse_snapshot_compat \
 --test hid_configuration_snapshot_clamping \
 --test hid_consumer_control_snapshot_clamping \
 --test hid_gamepad_snapshot_clamping \
 --test hid_gamepad_report_fixture \
 --test hid_gamepad_report_clamping_fixture \
 --test hid_keyboard_snapshot_sanitization \
 --test hid_keyboard_leds \
 --test hid_mouse_report_generation \
 --test hid_mouse_snapshot_clamping \
 --test usb_hub_snapshot_configuration_clamping \
 --test attached_device_snapshot_address_clamping \
 --test hid_usage_keyboard_fixture \
 --test hid_usage_consumer_fixture \
 --test webhid_boot_interface \
 --test webhid_passthrough \
 --test webhid_report_descriptor_synthesis \
 --test xhci_enum_smoke \
 --test xhci_port_remote_wakeup \
 --test xhci_controller_webusb_ep0 \
 --test xhci_doorbell0 \
 --test xhci_stop_endpoint_unschedules \
 --test xhci_usbcmd_run_gates_transfers \
 --test xhci_webusb_passthrough
# Full USB suite:
bash ./scripts/safe-run.sh cargo test -p aero-usb --locked

# WASM integration sanity (routes input through the same public WASM APIs used by the web runtime).
bash ./scripts/safe-run.sh cargo test -p aero-wasm --locked --test machine_input_injection --test machine_input_backends --test machine_defaults_usb_hid --test webhid_report_descriptor_synthesis --test machine_virtio_input

# Web unit tests (full suite)
npm -w web run test:unit

# Playwright E2E suite (repo root)
npm run test:e2e
```

---

### Quick Start Checklist

1. ☐ Read [`AGENTS.md`](./purged-material.md) completely
2. ☐ Run `bash ./scripts/agent-env-setup.sh` and `source ./scripts/agent-env.sh`
3. ☐ Read [`../areas/usb-and-input.md`](../areas/usb-and-input.md)
4. ☐ Read [`../decisions/0015-canonical-usb-stack.md`](../decisions/0015-canonical-usb-stack.md)
5. ☐ Explore `crates/aero-devices-input/src/` and `crates/aero-usb/src/`
6. ☐ Run existing tests to establish baseline
7. ☐ Pick a task from the tables above and begin

---

*Input makes the emulator interactive. Without it, you're just watching a movie.*

## Workstream C: Windows Drivers

> **⚠️ MANDATORY: Read and follow [`AGENTS.md`](./purged-material.md) in its entirety before starting any work.**
>
> AGENTS.md contains critical operational guidance including:
> - Defensive mindset (assume hostile/misbehaving code)
> - Resource limits and `safe-run.sh` usage
> - Windows 7 test ISO location (`/state/win7.iso`)
> - Interface contracts
> - Technology stack decisions
>
> **Failure to follow AGENTS.md will result in broken builds, OOM kills, and wasted effort.**

---

### Overview

This workstream owns **Windows 7 guest drivers**: the AeroGPU display driver (WDDM KMD + UMD) and virtio paravirtualized drivers (virtio-blk, virtio-net, virtio-input, virtio-snd).

It also owns the **binding/packaging surface** for those drivers (HWID/service-name contracts + Guest Tools media) so the emulator, driver INFs, and installer scripts stay consistent.

These drivers run **inside the guest Windows 7** and communicate with the emulator's device models via PCI MMIO/IO ports and shared memory.

---

### Key Directories

| Directory | Purpose |
|-----------|---------|
| `drivers/aerogpu/` | AeroGPU WDDM driver (KMD + UMD) |
| `drivers/aerogpu/kmd/` | Kernel-mode driver |
| `drivers/aerogpu/umd/` | User-mode driver (D3D9/D3D10/D3D11 DDI) |
| `drivers/aerogpu/tests/win7/` | Guest-side AeroGPU validation suite (D3D9/D3D10/D3D11) |
| `drivers/windows7/` | Virtio drivers for Windows 7 |
| `drivers/windows7/virtio/common/` | Shared Win7 virtio glue (WDM/NDIS/StorPort shims + split virtqueue impl) |
| `drivers/windows7/virtio-blk/` | Block device driver |
| `drivers/windows7/virtio-net/` | Network driver |
| `drivers/windows7/virtio-input/` | HID input driver |
| `drivers/windows7/virtio-snd/` | Audio driver |
| `drivers/virtio/` | Virtio driver-pack/ISO layout surface (virtio-win compatibility tooling; `sample/` placeholders) |
| `drivers/scripts/` | Driver-pack + Guest Tools build/install scripts (`make-guest-tools-from-ci.ps1`, `make-driver-pack.ps1`, etc.) |
| `drivers/windows/virtio/` | Portable virtio helpers (shared with Win7 drivers; host-side tests build on Linux) |
| `drivers/windows7/tests/guest-selftest/` | `aero-virtio-selftest.exe` (runs inside Win7 guest; emits serial markers) |
| `drivers/windows7/tests/host-harness/` | QEMU host harness that runs the guest selftest and returns a deterministic PASS/FAIL |
| `drivers/win7/virtio/` | Win7 KMDF virtio scaffolding + capability parser tests (non-shipping test drivers) |
| `drivers/aerogpu/protocol/` | AeroGPU protocol headers (PCI IDs, MMIO register map, ring ABI, command stream) |
| `drivers/protocol/` | Protocol definitions (shared with emulator; currently used for virtio) |
| `drivers/protocol/virtio/` | Rust virtio protocol definitions + unit tests (`cargo test`) |
| `guest-tools/` | Guest tools packaging and installer |
| `docs/windows-device-contract.{md,json}` | Machine-readable PCI/INF/service binding contract consumed by CI + Guest Tools |
| `docs/windows-device-contract-virtio-win.json` | Optional virtio-win compatibility contract (used by virtio-win packaging flows) |
| `tools/device_contract_validator/` | Rust validator for the device contract (runs in CI) |
| `tools/packaging/` | Guest Tools packager + packaging specs (ISO/zip builder) |
| `tools/packaging/aero_packager/` | Deterministic Rust packager implementation + tests |
| `tools/guest-tools/` | Guest Tools config validation + linters (runs in CI) |
| `drivers/build/` | CI scripts for Win7 driver builds/signing/packaging (WDK provisioning, catalogs, signing) |

---

### Essential Documentation

**Must read:**

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — Windows driver development overview
- [`../specs/windows7-virtio-driver-contract.md`](../specs/windows7-virtio-driver-contract.md) — Virtio device contract
- [`../specs/windows-device-contract.md`](../specs/windows-device-contract.md) — Unified device/driver binding contract (virtio + AeroGPU)
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — MSI-X/INTx handling
- [`../specs/windows7-virtio-driver-contract.md`](../specs/windows7-virtio-driver-contract.md) — Virtqueue implementation

**Reference:**

- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — Build toolchain
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — Packaging and catalogs
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — Guest Tools packager specs/inputs/outputs (ISO/zip)
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — Virtio driver plumbing notes (transport/virtqueues)
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — Legacy/transitional virtio-pci notes (compatibility)
- [`../decisions/0016-win7-virtio-driver-naming.md`](../decisions/0016-win7-virtio-driver-naming.md) — Canonical in-tree Win7 virtio naming scheme (`aero_virtio_*`)
- [`drivers/README.md`](../decisions/README.md) — What CI actually ships (artifact names, release workflow, Guest Tools media)
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — Virtio driver packaging options (in-tree vs virtio-win)
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — virtio-input device model notes (keyboard/mouse) + contract mapping
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — virtio-input end-to-end test plan (device model ↔ driver ↔ harness ↔ web runtime)
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — virtio-snd device model notes + contract mapping (incl. transitional ID notes)
- [`../specs/windows7-aerogpu-validation.md`](../specs/windows7-aerogpu-validation.md) — AeroGPU stability checklist (TDR/vblank/perf debug playbook)
- [`../specs/windows7-d3d-umd-ddi.md`](../specs/windows7-d3d-umd-ddi.md) — Minimal D3D9Ex UMD implementation guide for enabling DWM/Aero on Win7
- [`../specs/windows7-d3d-umd-ddi.md`](../specs/windows7-d3d-umd-ddi.md) — Minimal D3D10/D3D11 UMD implementation guide (Win7)
- [`../specs/windows7-d3d-umd-ddi.md`](../specs/windows7-d3d-umd-ddi.md) — lightweight Win7 D3D9 UMD DDI call tracing (what DWM/test apps invoke; DebugView workflow)
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — End-to-end Win7 install + switch to virtio + AeroGPU
- [`../areas/windows-drivers.md`](../areas/windows-drivers.md) — Debugging tips
- [`../areas/graphics.md`](../areas/graphics.md) — AeroGPU VGA compat
- [`drivers/aerogpu/README.md`](../decisions/README.md) — AeroGPU build/CI entrypoint + key docs
- [`drivers/windows7/tests/README.md`](../decisions/README.md) — Win7 virtio selftest + harness overview (incl. virtio-snd notes)
- [`drivers/windows7/tests/host-harness/README.md`](../decisions/README.md) — Win7 virtio test harness (incl. virtio-snd wav capture)
- [`drivers/windows7/tests/guest-selftest/README.md`](../decisions/README.md) — Guest selftest details (incl. virtio-snd playback/capture/duplex)

---

### Device Contracts

**Critical:** The Windows drivers and emulator device models **must stay in sync**. The source of truth is:

- Virtio transport + feature contract: [`../specs/windows7-virtio-driver-contract.md`](../specs/windows7-virtio-driver-contract.md) (`AERO-W7-VIRTIO`)
- Unified device binding manifest: ``docs/windows-device-contract.json`` (+ human-readable [`../specs/windows-device-contract.md`](../specs/windows-device-contract.md))
 - Optional virtio-win compatibility manifest (used only for virtio-win-based packaging/scripts): ``docs/windows-device-contract-virtio-win.json``

Any change to PCI vendor/device IDs, BAR sizes, or feature bits requires:
1. Update the contract document
2. Update the emulator device model
3. Update the driver INF files
4. Update the unified binding manifest (`docs/windows-device-contract.json`) and regenerate Guest Tools `devices.cmd`
5. Coordinate with Graphics (B) and Integration (H) workstreams

#### CI guardrails (do not bypass)

These checks exist specifically to prevent “driver installs but doesn’t bind” regressions:

- Win7 driver build + packaging: ``.github/workflows/drivers-win7.yml``
 - Runs contract drift checks (docs ↔ INFs ↔ emulator PCI profiles)
 - Runs host-side unit tests for virtio common, virtio-snd protocol engines, and AeroGPU command stream encoding
- Repo-wide CI also runs some driver/Guest Tools guardrails (WDK macro guards, Guest Tools `devices.cmd` regeneration, D3D11 guest-memory import invariants): ``.github/workflows/ci.yml``
- Windows virtio contract wiring (manifest ↔ INFs ↔ emulator ↔ guest-tools): ``.github/workflows/windows-virtio-contract.yml``
- Device contract validator (JSON schema + invariants): ``.github/workflows/windows-device-contract.yml``
- Virtio protocol crate tests: ``.github/workflows/virtio-protocol.yml``
- Win7 virtio guest selftest build (x86 + x64 EXEs): ``.github/workflows/win7-virtio-selftest.yml``
- Win7 virtio QEMU harness (self-hosted; end-to-end guest run, incl. virtio-snd wav capture): ``.github/workflows/win7-virtio-harness.yml``
 - Use workflow inputs `with_virtio_input_events=true`, `with_virtio_input_wheel=true`, `with_virtio_input_media_keys=true`,
 `with_virtio_input_events_extended=true`, and/or `with_virtio_input_tablet_events=true` to enable the optional QMP injection-based
 end-to-end virtio-input tests.
 (Requires a guest image provisioned with `--test-input-events` for events/wheel, `--test-input-media-keys` for media keys, also
 `--test-input-events-extended` for the extended markers, and `--test-input-tablet-events` (alias: `--test-tablet-events`) for tablet.)
- Guest Tools packager + spec/config validation: ``.github/workflows/guest-tools-packager.yml``
- Guest Tools `devices.cmd` regeneration check (must match contract JSON): ``.github/workflows/guest-tools-devices-cmd.yml``
- Win7 toolchain smoke (WDK provisioning + `Inf2Cat /os:7_X86,7_X64`): ``.github/workflows/toolchain-win7-smoke.yml``
- virtio-win packaging smoke tests (optional flow, upstream virtio-win bundles): ``.github/workflows/virtio-win-packaging-smoke.yml``
- Sample virtio driver ISO build + smoke tests: ``.github/workflows/virtio-driver-iso.yml``
- Tagged release pipeline (publishes signed driver bundles + Guest Tools as GitHub Release assets): ``.github/workflows/release-drivers-win7.yml``
- Docs lint + contract/link checks (includes virtio contract consistency + share-token checks): ``.github/workflows/docs.yml``

Local equivalents for fast iteration:

```bash
# Virtio contract drift checks (docs ↔ INFs ↔ emulator PCI profiles + guest-tools specs)
python3 scripts/ci/check-windows7-virtio-contract-consistency.py
python3 scripts/ci/check-windows-virtio-contract.py --check

# Regenerate/check artifacts derived from docs/windows-device-contract.json
# (guest-tools/config/devices.cmd + docs/windows-device-contract-virtio-win.json)
python3 scripts/regen-windows-device-contract-artifacts.py
python3 scripts/regen-windows-device-contract-artifacts.py --check

# Optional: regenerate derived artifacts (only guest-tools/config/devices.cmd), then re-check
python3 scripts/ci/check-windows-virtio-contract.py --fix

# Ensure `guest-tools/config/devices.cmd` matches the contract JSON + generator
python3 scripts/ci/gen-guest-tools-devices-cmd.py --check

# Additional Win7 driver guardrails (fast, no VM required)
python3 scripts/ci/check-virtio-snd-vcxproj-sources.py
python3 scripts/ci/check-win7-virtqueue-split-headers.py
python3 scripts/ci/check-virtqueue-split-driver-builds.py
python3 scripts/ci/check-win7-virtio-header-collisions.py
python3 scripts/ci/check-win7-virtio-net-pci-config-access.py
python3 scripts/ci/check-win7-virtio-blk-no-duplicate-freeresources.py

# AeroGPU guardrails (fast, no VM required)
python3 scripts/ci/check-aerogpu-d3d9-def-stdcall.py
python3 scripts/ci/check-aerogpu-d3d10-def-stdcall.py
python3 scripts/ci/check-aerogpu-wdk-guards.py
python3 scripts/ci/check-aero-d3d11-guest-memory-imports.py
python3 scripts/ci/check-aerogpu-share-token-contract.py

# Repo layout guardrails (includes AeroGPU Win7 test-suite manifest/doc/fallback-list invariants)
bash scripts/ci/check-repo-layout.sh

# Ensure no duplicate virtio INFs/projects bind the same HWIDs (requires pwsh)
pwsh -NoProfile -ExecutionPolicy Bypass -File drivers/build/check-virtio-driver-uniqueness.ps1

# Device contract schema/invariants (same check as windows-device-contract.yml)
cargo run -p device-contract-validator --locked

# Rust virtio protocol unit tests (same as virtio-protocol.yml)
cargo test --locked --manifest-path drivers/protocol/virtio/Cargo.toml

# Guest Tools packager tests + spec/config validation
cargo test --locked --manifest-path tools/packaging/aero_packager/Cargo.toml
python3 tools/guest-tools/validate_config.py --spec tools/packaging/specs/win7-signed.json

# Ensure virtio-win Guest Tools docs + wrapper defaults stay in sync
python3 scripts/ci/check-virtio-win-guest-tools-docs.py

# Host-harness Python unit tests (wav verification + QEMU arg quoting)
python3 -m unittest discover -s drivers/windows7/tests/host-harness/tests -p 'test_*.py'

# Host-side C unit tests for virtio helpers and virtio-snd protocol engines
cmake -S . -B build-virtio-host-tests -DAERO_VIRTIO_BUILD_TESTS=ON -DAERO_AEROGPU_BUILD_TESTS=OFF -DCMAKE_BUILD_TYPE=Release
cmake --build build-virtio-host-tests
ctest --test-dir build-virtio-host-tests --output-on-failure

# Portable virtio PCI capability parser tests (virtio-core; no Windows/WDK required)
bash ./drivers/win7/virtio/tests/build_and_run.sh

# Host-side C++ unit tests for AeroGPU UMD helpers (command stream writer, submit buffer utils)
cmake -S . -B build-aerogpu-host-tests -DAERO_AEROGPU_BUILD_TESTS=ON -DAERO_VIRTIO_BUILD_TESTS=OFF -DCMAKE_BUILD_TYPE=Release
cmake --build build-aerogpu-host-tests
ctest --test-dir build-aerogpu-host-tests --output-on-failure --no-tests=error

# Windows-only: validate the installed WDK toolchain can generate Win7 catalogs
pwsh -NoProfile -ExecutionPolicy Bypass -File drivers/build/validate-toolchain.ps1
```

---

### Tasks

The tables below are meant to be an **onboarding map**: what already exists in-tree (with CI coverage), and what remains.

Legend:

- **Implemented** = present in-tree and wired into at least one CI workflow/guardrail.
- **Implemented (optional)** = present in-tree but not required by the v1 contract (typically compatibility/perf enhancements; emulator/device models may choose not to implement).
- **Partial** = present but explicitly minimal/stubbed; known follow-ups remain.
- **Remaining** = not implemented yet (or explicitly stubbed with TODO-level behavior).

#### Virtio Foundation + Guardrails (Shared)

| ID | Status | Task | Where | CI/Guardrails |
|----|--------|------|-------|---------------|
| VIO-001 | Implemented | Virtio-pci **modern** transport parser/library (cap discovery + BAR0 MMIO layout) | [`drivers/windows/virtio/pci-modern/README.md`](../decisions/README.md) | ``drivers-win7.yml`` (virtio host tests), [`check-windows7-virtio-contract-consistency.py`](../../scripts/ci/check-windows7-virtio-contract-consistency.py) |
| VIO-002 | Implemented | Split-ring virtqueue + SG helpers (portable) | [`drivers/windows/virtio/common/README.md`](../decisions/README.md) | ``drivers-win7.yml`` (virtio host tests), [`check-win7-virtio-header-collisions.py`](../../scripts/ci/check-win7-virtio-header-collisions.py) |
| VIO-003 | Implemented | Win7 virtio common glue (WDM/NDIS/StorPort shims + INTx) | [`drivers/windows7/virtio/common/README.md`](../decisions/README.md) | ``drivers-win7.yml`` (guardrails + host tests) |
| VIO-004 | Implemented | Rust virtio protocol definitions + tests | [`drivers/protocol/virtio/README.md`](../decisions/README.md) | ``virtio-protocol.yml`` |
| VIO-005 | Implemented | Win7 KMDF `virtio-core` transport (portable virtio PCI cap parser + optional Aero MMIO layout enforcement) | [`drivers/win7/virtio/virtio-core/README.md`](../decisions/README.md), [`drivers/win7/virtio/tests/README.md`](../decisions/README.md) | ``drivers-win7.yml`` (virtio host tests) |

#### Virtio Device Drivers

| ID | Status | Task | Where | Tests / docs to start with |
|----|--------|------|-------|----------------------------|
| VIO-010 | Implemented | virtio-input (KMDF HID minidriver; contract-v1 IDs; keyboard+mouse separation; report descriptor synthesis) | `drivers/windows7/virtio-input/` | README: [`drivers/windows7/virtio-input/README.md`](../decisions/README.md); unit tests: [`drivers/windows7/virtio-input/tests/README.md`](../decisions/README.md); guest selftest: [`drivers/windows7/tests/guest-selftest/README.md`](../decisions/README.md); harness event injection: [`drivers/windows7/tests/host-harness/README.md`](../decisions/README.md) |
| VIO-011 | Implemented | virtio-blk (StorPort miniport; contract-v1 IDs; boot-start capable; minimal feature set) | `drivers/windows7/virtio-blk/` | README: [`drivers/windows7/virtio-blk/README.md`](../decisions/README.md); guest coverage: [`drivers/windows7/tests/guest-selftest/README.md`](../decisions/README.md) |
| VIO-012 | Implemented | virtio-net (NDIS 6.20 miniport; contract-v1 IDs; minimal feature set) | `drivers/windows7/virtio-net/` | README: [`drivers/windows7/virtio-net/README.md`](../decisions/README.md); guest coverage: [`drivers/windows7/tests/guest-selftest/README.md`](../decisions/README.md) |
| VIO-016 | Implemented | **virtio-snd** (PortCls/WaveRT audio driver; contract v1 + optional transitional/QEMU package) | `drivers/windows7/virtio-snd/` | README: [`drivers/windows7/virtio-snd/README.md`](../decisions/README.md); design notes: [`drivers/windows7/virtio-snd/docs/design.md`](../../drivers/windows7/virtio-snd/docs/design.md) |
| VIO-017 | Implemented | virtio-snd host unit tests (protocol engines + integrated proto/SG tests) | `drivers/windows7/virtio-snd/tests/` | README: [`drivers/windows7/virtio-snd/README.md`](../decisions/README.md) (“Host unit tests” section); subset docs: [`drivers/windows7/virtio-snd/tests/host/README.md`](../decisions/README.md); CI: ``drivers-win7.yml`` |
| VIO-018 | Implemented | Win7 guest selftest coverage for virtio-snd **playback + capture + duplex** markers | `drivers/windows7/tests/guest-selftest/` | Guest tool docs: [`guest-selftest/README.md`](../decisions/README.md); CI build: ``win7-virtio-selftest.yml``; harness: ``win7-virtio-harness.yml`` (self-hosted) |
| VIO-019 | Implemented | Host harness: QEMU runner that parses guest selftest markers + optional wav non-silence verification | `drivers/windows7/tests/host-harness/` | Harness README: [`host-harness/README.md`](../decisions/README.md); unit tests: [`host-harness/tests/README.md`](../decisions/README.md) |
| VIO-020 | Implemented (optional) | Feature expansion for virtio devices (optional/non-contract):<br>- Optional MSI/MSI-X support (INTx still required) across Win7 `virtio-blk`/`virtio-net`/`virtio-input`/`virtio-snd`<br>- `virtio-net` offloads/TSO support<br>- `virtio-snd` `eventq` robustness (incl. JACK events) + multi-format PCM negotiation<br>- `virtio-input` expanded HID coverage (consumer/media keys, tablet/ABS pointer, extra buttons, horizontal wheel) + end-to-end harness injection tests | `drivers/windows7/virtio-blk/`<br>`drivers/windows7/virtio-net/`<br>`drivers/windows7/virtio-input/`<br>`drivers/windows7/virtio-snd/`<br>(optional) `drivers/windows/virtio/pci-modern/`<br>(optional) `drivers/windows7/tests/{guest-selftest,host-harness}/` | MSI-X/INTx: [`../areas/windows-drivers.md`](../areas/windows-drivers.md)<br>Optional features: [`../specs/windows7-virtio-driver-contract.md` (§3.5)](../specs/windows7-virtio-driver-contract.md#35-optionalcompatibility-features-non-normative)<br>E2E: [`guest-selftest/README.md`](../decisions/README.md) + [`host-harness/README.md`](../decisions/README.md) (IRQ markers, input-events/tablet-events)<br>Input injection tests: [`../areas/windows-drivers.md`](../areas/windows-drivers.md)<br>Driver READMEs: [`virtio-net`](../decisions/README.md), [`virtio-snd`](../decisions/README.md), [`virtio-input`](../decisions/README.md) |

#### Guest Tools Tasks

| ID | Status | Task | Where | CI/Guardrails |
|----|--------|------|-------|---------------|
| GT-001 | Implemented | Windows device contract docs + JSON manifest | Docs: [`windows-device-contract.md`](../specs/windows-device-contract.md) + ``windows-device-contract.json`` | ``windows-device-contract.yml`` (Rust validator) |
| GT-002 | Implemented | Guest Tools config generation (`guest-tools/config/devices.cmd`) from the contract | Generator: [`scripts/generate-guest-tools-devices-cmd.py`](../../scripts/generate-guest-tools-devices-cmd.py); output: [`guest-tools/config/devices.cmd`](../../guest-tools/config/devices.cmd) | ``.github/workflows/guest-tools-devices-cmd.yml``, [`gen-guest-tools-devices-cmd.py`](../../scripts/ci/gen-guest-tools-devices-cmd.py), ``.github/workflows/windows-virtio-contract.yml``, [`check-windows-virtio-contract.py`](../../scripts/ci/check-windows-virtio-contract.py) |
| GT-003 | Implemented | Guest Tools installer stages drivers, manages test-signing policy, and pre-seeds boot-critical virtio-blk (`CriticalDeviceDatabase`) | Installer: [`guest-tools/setup.cmd`](../../guest-tools/setup.cmd) | ``drivers-win7.yml`` (packages Guest Tools media) |
| GT-004 | Implemented | Enforce INF ↔ contract ↔ emulator consistency (virtio HWIDs, revision gating, service names) | [`check-windows7-virtio-contract-consistency.py`](../../scripts/ci/check-windows7-virtio-contract-consistency.py) | ``drivers-win7.yml`` (guardrails) |
| GT-005 | Implemented | Enforce virtio contract wiring (contract JSON ↔ INFs ↔ emulator PCI profiles ↔ guest-tools) | [`check-windows-virtio-contract.py`](../../scripts/ci/check-windows-virtio-contract.py) | ``windows-virtio-contract.yml`` |
| GT-006 | Implemented | Contract versioning + policy surfaced in docs and manifests | [`windows-device-contract.md`](../specs/windows-device-contract.md) | ``windows-device-contract.yml`` |
| GT-007 | Implemented | Guest Tools packager (ISO/zip) + spec/config validation | `tools/packaging/aero_packager/`, `tools/packaging/specs/`, `tools/guest-tools/` | ``guest-tools-packager.yml`` |

#### AeroGPU Driver Tasks

| ID | Status | Task | Where | Tests / docs to start with |
|----|--------|------|-------|----------------------------|
| AGPU-001 | Partial | AeroGPU KMD (WDDM 1.1 miniport) + D3D9Ex UMD bring-up (Aero composition + basic rendering) | `drivers/aerogpu/kmd/`, `drivers/aerogpu/umd/d3d9/` | UMD README (stub list): [`drivers/aerogpu/umd/d3d9/README.md`](../decisions/README.md); guest suite: [`drivers/aerogpu/tests/win7/README.md`](../decisions/README.md); host tests: `drivers/aerogpu/umd/d3d9/tests/` (CI: ``drivers-win7.yml``) |
| AGPU-002 | Implemented | D3D9Ex “currently stubbed DDIs” → real implementations (and decouple trace stub coverage via `TraceTestStub`) | `drivers/aerogpu/umd/d3d9/` | Stub checklist (now empty): [`d3d9/README.md#currently-stubbed-ddis`](../decisions/README.md#currently-stubbed-ddis); trace docs: [`../specs/windows7-d3d-umd-ddi.md`](../specs/windows7-d3d-umd-ddi.md); Win7 suite: [`drivers/aerogpu/tests/win7/`](../../drivers/aerogpu/tests/win7/) |
| AGPU-003 | Partial | D3D10/11 UMD feature expansion beyond “triangle” bring-up (pipeline/state coverage; Map/Unmap correctness; format support) | `drivers/aerogpu/umd/d3d10_11/` | README: [`drivers/aerogpu/umd/d3d10_11/README.md`](../decisions/README.md); checklist: [`../specs/windows7-d3d-umd-ddi.md`](../specs/windows7-d3d-umd-ddi.md); Map/Unmap notes: [`../specs/windows7-d3d-umd-ddi.md`](../specs/windows7-d3d-umd-ddi.md); host tests: `drivers/aerogpu/umd/d3d10_11/tests/` |
| AGPU-004 | Implemented | DXGI/D3D10/11 shared-resource interop (export/import shared surfaces, share-token plumbing) | `drivers/aerogpu/umd/d3d10_11/` | Design contract: [`../specs/aerogpu-device-abi.md`](../specs/aerogpu-device-abi.md); Win7 IPC tests: `d3d10_shared_surface_ipc`, `d3d10_1_shared_surface_ipc`, `d3d11_shared_surface_ipc` |
| AGPU-005 | Implemented | AeroGPU Win7 guest-side validation suite (D3D9/D3D10/D3D11 + vblank/fence/ring probes) | `drivers/aerogpu/tests/win7/` | [`drivers/aerogpu/tests/win7/README.md`](../decisions/README.md) |
| AGPU-006 | Implemented | DX11-capable AeroGPU package is staged by CI (`aerogpu_dx11.inf` + WOW64 `aerogpu_d3d10.dll`) | `drivers/aerogpu/ci-package.json`, `drivers/aerogpu/packaging/win7/` | Packaging notes: [`drivers/aerogpu/packaging/win7/README.md`](../decisions/README.md) (see “0) CI packages vs manual packaging”) |

---

### Driver Architecture

#### Virtio Driver Stack

```
┌──────────────────────────────────────────────────────┐
│ Windows 7 Guest │
├──────────────────────────────────────────────────────┤
│ aero_virtio_blk.sys aero_virtio_net.sys ... │ Device-specific drivers
│ │ │ │
│ └───────┬────────────┘ │
│ ▼ │
│ Shared in-driver virtio libs (not a separate .sys): │
│ - drivers/windows7/virtio/common/ │
│ - drivers/windows/virtio/{common,pci-modern}/ │
│ - drivers/win7/virtio/virtio-core/ │
│ │ │
├─────────────────┼────────────────────────────────────┤
│ ▼ │
│ PCI Bus (emulator) │
└──────────────────────────────────────────────────────┘
```

#### AeroGPU Driver Stack

```
┌─────────────────────────────────────────────┐
│ Windows 7 Guest │
├─────────────────────────────────────────────┤
│ D3D9/D3D10/D3D11 Runtime │
│ │ │
│ ▼ │
│ aerogpu_d3d9*.dll (+ optional aerogpu_d3d10*.dll) │ User-mode display drivers (UMDs)
│ │ │
│ ▼ │
│ aerogpu.sys (KMD) │ WDDM miniport driver
│ │ │
├─────────────────┼───────────────────────────┤
│ ▼ │
│ AeroGPU PCI Device (emulator) │
└─────────────────────────────────────────────┘
```

---

### Build Environment

**Requires Windows + WDK.** If developing on Linux/macOS, you'll need:
- Cross-compilation setup, OR
- Windows VM for driver builds, OR
- CI that builds on Windows

See [`../areas/windows-drivers.md`](../areas/windows-drivers.md) for toolchain setup.

```powershell
# Recommended (CI-like; builds + stages drivers under out/)
pwsh drivers/build/install-wdk.ps1
pwsh drivers/build/build-drivers.ps1 -ToolchainJson out/toolchain.json -Drivers aerogpu windows7/virtio-blk windows7/virtio-net windows7/virtio-input windows7/virtio-snd
pwsh drivers/build/build-aerogpu-dbgctl.ps1 -ToolchainJson out/toolchain.json # required when drivers/aerogpu/ci-package.json declares dbgctl via requiredBuildOutputFiles

# Note: CI only builds/packages drivers that explicitly opt in via `ci-package.json`
# under the driver directory (and have at least one `.inf`), to avoid accidentally
# shipping scaffolding/test drivers.
#
# If you need to ship extra packaging assets, use:
# - `additionalFiles` for non-binary files (README/license text, install scripts, etc)
# - `toolFiles` for repo-local `.exe` helper tools (explicit opt-in; `.exe` remains disallowed in additionalFiles)

# Optional: generate catalogs + test-sign + bundle artifacts (Guest Tools ISO/zip, etc.)
pwsh drivers/build/make-catalogs.ps1 -ToolchainJson out/toolchain.json
pwsh drivers/build/sign-drivers.ps1 -ToolchainJson out/toolchain.json
pwsh drivers/build/package-drivers.ps1
pwsh drivers/build/package-guest-tools.ps1 -SpecPath tools/packaging/specs/win7-signed.json

# On Windows with WDK installed:
cd drivers\aerogpu
msbuild aerogpu.sln /p:Configuration=Release /p:Platform=x64

# For virtio drivers (example: virtio-blk):
cd drivers\windows7\virtio-blk
msbuild aero_virtio_blk.vcxproj /p:Configuration=Release /p:Platform=x64
```

---

### Test Signing

Windows 7 requires signed drivers. For development:

1. Enable test signing: `bcdedit /set testsigning on`
2. Sign drivers with test certificate
3. Install test certificate in guest

See [`../areas/windows-drivers.md`](../areas/windows-drivers.md) for offline BCD patching.

---

### Coordination Points

#### Dependencies on Other Workstreams

- **Graphics (B)**: AeroGPU driver must match emulator device model
- **Integration (H)**: Device models must be wired into platform

#### What Other Workstreams Need From You

- Stable driver binaries for integration testing
- Updated INF files when device model changes
- Guest Tools installer for easy deployment

#### Cross-Workstream Contract Changes

**Any change to these requires coordination:**
- PCI Vendor/Device IDs
- BAR sizes or layouts
- Feature bits
- Command/status register formats

---

### Testing

Driver testing requires a running Windows 7 guest:

1. Boot Windows 7 in the emulator
2. Install test-signed drivers
3. Verify device appears in Device Manager
4. Run functional tests

For AeroGPU:
- Check display resolution changes
- Verify DWM composition (Aero glass)
- Run D3D9/D3D10/D3D11 test apps
- Run the in-tree Win7 validation suite (`drivers/aerogpu/tests/win7/`), preferably via `bin\\aerogpu_test_runner.exe` (see: [`drivers/aerogpu/tests/win7/README.md`](../decisions/README.md))

For virtio-blk:
- Verify disk appears in Disk Management
- Read/write test files
- Check performance (should be faster than emulated AHCI)

For virtio end-to-end regression testing (recommended):

- Guest selftest: `drivers/windows7/tests/guest-selftest/` (`aero-virtio-selftest.exe`)
- Host harness: `drivers/windows7/tests/host-harness/` (boots QEMU + parses serial markers)
 - Supports virtio-snd wav capture + non-silence verification when enabled.
 - See: [`drivers/windows7/tests/README.md`](../decisions/README.md)

Example (Linux/macOS/Windows host; Python harness):

```bash
python3 drivers/windows7/tests/host-harness/invoke_aero_virtio_win7_tests.py \
 --qemu-system qemu-system-x86_64 \
 --disk-image ./win7-aero-tests.qcow2 \
 --snapshot \
 --qemu-preflight-pci \
 --timeout-seconds 600
```

Example: require end-to-end virtio-input **event delivery** (host QMP injects a deterministic keyboard/mouse sequence):

```bash
python3 drivers/windows7/tests/host-harness/invoke_aero_virtio_win7_tests.py \
 --qemu-system qemu-system-x86_64 \
 --disk-image ./win7-aero-tests.qcow2 \
 --snapshot \
 --with-input-events \
 --timeout-seconds 600
```

Note: `--with-input-events` requires a guest image provisioned with virtio-input event testing enabled
(so the guest selftest runs with `--test-input-events` / env var).

When `--with-input-events` is set:

- If the guest emits `virtio-input-events|SKIP|flag_not_set`, the harness will fail
 (PowerShell: `VIRTIO_INPUT_EVENTS_SKIPPED`; Python: `FAIL: VIRTIO_INPUT_EVENTS_SKIPPED: ...`).
- If the end-to-end path is broken and the guest reports `virtio-input-events|FAIL|...`, the harness will fail
 (PowerShell: `VIRTIO_INPUT_EVENTS_FAILED`; Python: `FAIL: VIRTIO_INPUT_EVENTS_FAILED: ...`).
- If QMP input injection fails, the harness will fail
 (PowerShell: `QMP_INPUT_INJECT_FAILED`; Python: `FAIL: QMP_INPUT_INJECT_FAILED: ...`).
- If the guest selftest is too old/misconfigured and does not emit any `virtio-input-events` marker at all after
 completing `virtio-input`, the harness will fail early
 (PowerShell: `MISSING_VIRTIO_INPUT_EVENTS`; Python: `FAIL: MISSING_VIRTIO_INPUT_EVENTS: ...`).

Example: require end-to-end virtio-input **tablet (absolute pointer)** event delivery (host QMP injects a deterministic abs-move + click sequence):

```bash
python3 drivers/windows7/tests/host-harness/invoke_aero_virtio_win7_tests.py \
 --qemu-system qemu-system-x86_64 \
 --disk-image ./win7-aero-tests.qcow2 \
 --snapshot \
 --with-tablet-events \
 --timeout-seconds 600
```

Note: `--with-input-tablet-events` (alias: `--with-tablet-events`) requires a guest image provisioned with tablet event
testing enabled (so the guest selftest runs with `--test-input-tablet-events` (alias: `--test-tablet-events`) / env var
`AERO_VIRTIO_SELFTEST_TEST_INPUT_TABLET_EVENTS=1` / `AERO_VIRTIO_SELFTEST_TEST_TABLET_EVENTS=1`).

When `--with-input-tablet-events` / `--with-tablet-events` is set:

- If the guest emits `virtio-input-tablet-events|SKIP|flag_not_set`, the harness will fail
 (PowerShell: `VIRTIO_INPUT_TABLET_EVENTS_SKIPPED`; Python: `FAIL: VIRTIO_INPUT_TABLET_EVENTS_SKIPPED: ...`).
- If the end-to-end path is broken and the guest reports `virtio-input-tablet-events|FAIL|...`, the harness will fail
 (PowerShell: `VIRTIO_INPUT_TABLET_EVENTS_FAILED`; Python: `FAIL: VIRTIO_INPUT_TABLET_EVENTS_FAILED: ...`).
- If QMP tablet injection fails, the harness will fail
 (PowerShell: `QMP_INPUT_TABLET_INJECT_FAILED`; Python: `FAIL: QMP_INPUT_TABLET_INJECT_FAILED: ...`).
- If the guest selftest is too old/misconfigured and does not emit any `virtio-input-tablet-events` marker at all after
 completing `virtio-input`, the harness will fail early
 (PowerShell: `MISSING_VIRTIO_INPUT_TABLET_EVENTS`; Python: `FAIL: MISSING_VIRTIO_INPUT_TABLET_EVENTS: ...`).

See: [`drivers/windows7/tests/host-harness/README.md`](../decisions/README.md).

Example: attach virtio-snd and capture deterministic wav output + verify non-silence:

```bash
python3 drivers/windows7/tests/host-harness/invoke_aero_virtio_win7_tests.py \
 --qemu-system qemu-system-x86_64 \
 --disk-image ./win7-aero-tests.qcow2 \
 --snapshot \
 --with-virtio-snd \
 --virtio-snd-audio-backend wav \
 --virtio-snd-wav-path ./out/virtio-snd.wav \
 --virtio-snd-verify-wav \
 --timeout-seconds 600
```

Note: `--with-virtio-snd` requires a guest image with virtio-snd capture/duplex enabled. Newer guest selftest binaries run
capture + duplex by default when virtio-snd is present, but older binaries must be provisioned with `--test-snd-capture`
(or env var `AERO_VIRTIO_SELFTEST_TEST_SND_CAPTURE=1`). Otherwise the harness will fail on
`virtio-snd-duplex|SKIP|flag_not_set`. See: [`drivers/windows7/tests/host-harness/README.md`](../decisions/README.md).

Example: require the virtio-snd buffer limits stress test to PASS:

```bash
python3 drivers/windows7/tests/host-harness/invoke_aero_virtio_win7_tests.py \
 --qemu-system qemu-system-x86_64 \
 --disk-image ./win7-aero-tests.qcow2 \
 --snapshot \
 --with-virtio-snd \
 --with-snd-buffer-limits \
 --timeout-seconds 600
```

Note: `--with-snd-buffer-limits` requires a guest image provisioned with the selftest flag `--test-snd-buffer-limits`
(or env var `AERO_VIRTIO_SELFTEST_TEST_SND_BUFFER_LIMITS=1`) so the guest emits
`AERO_VIRTIO_SELFTEST|TEST|virtio-snd-buffer-limits|PASS/FAIL|...`. If the guest reports `SKIP|flag_not_set` or the
marker is missing entirely, the harness treats it as a hard failure.

---

### Quick Start Checklist

1. ☐ Read [`AGENTS.md`](./purged-material.md) completely
2. ☐ Read [`../areas/windows-drivers.md`](../areas/windows-drivers.md)
3. ☐ Read [`../specs/windows7-virtio-driver-contract.md`](../specs/windows7-virtio-driver-contract.md)
4. ☐ Read [`../specs/windows-device-contract.md`](../specs/windows-device-contract.md)
5. ☐ Read [`../areas/windows-drivers.md`](../areas/windows-drivers.md) (how the drivers are installed/switch-over order)
6. ☐ Read [`drivers/windows7/tests/README.md`](../decisions/README.md) (selftest + harness)
7. ☐ Read [`drivers/aerogpu/tests/win7/README.md`](../decisions/README.md) (AeroGPU guest validation suite)
8. ☐ Run contract checks locally (`python3 scripts/ci/check-windows7-virtio-contract-consistency.py`, `python3 scripts/ci/check-windows-virtio-contract.py --check`, `python3 scripts/ci/gen-guest-tools-devices-cmd.py --check`)
9. ☐ Set up Windows build environment (WDK)
10. ☐ Explore `drivers/aerogpu/` and `drivers/windows7/`
11. ☐ Build existing drivers to verify toolchain
12. ☐ Pick a task from the tables above and begin

---

*These drivers make the emulator fast. Virtio provides 10-100x speedup over full emulation.*

---
