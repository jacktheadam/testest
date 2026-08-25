# Bring-up findings: divergence-driven harness + sight-based audits

> **Status:** findings, dated 2026-07-26. Non-destructive (no source changed
> unless explicitly marked APPLIED below); produced by building a QEMU↔Aero
> first-divergence harness and by parallel read-only code audits against the
> Intel SDM. **If you are working the HAL-spin wall, read §0 first** — three
> independent audits converge on the same two root causes.
>
> Home for the *method* (divergence-driven bring-up) should be
> the bring-up instruments in `wiki/areas/debugging.md`; this page holds the concrete
> findings until they're applied, then it can be pruned.

## HAL-spin wall — two highly-probable root causes (read first)

The current open wall (`hal!0xfffff8000c608e7f`, IF=0, polling PM1a, flag
`0xfffff8000c62ad28` never set) was investigated by **three parallel read-only
audits** (timer subsystem, interrupt-controller stack, ACPI tables). They
triangulate to **two root causes**; the ACPI audit explicitly *clears* ACPI
tables/PM1a as the culprit (tables are correct and self-consistent).

### 0.1 ~~CR8 ↔ LAPIC TPR not aliased~~ — ALREADY FIXED (verified)

**Correction (verified 2026-07-26):** the CR8↔TPR aliasing **is already
implemented** — `crates/aero-machine/src/pc.rs:468` and
`lib.rs:13961-13964` call `sync_cr8_tpr_for_cpu(0, cr8)` before each poll, and
`crates/aero-interrupts/src/apic/local_apic.rs:458` `set_tpr_from_cr8` pushes
`(cr8 & 0x0f) << 4` into TPR. (An earlier parallel audit misread this as missing;
a second audit caught the contradiction and direct verification confirmed the
fix is present. This is a worked example of why parallel audits triangulate.)
**Not the wall.**

### 0.2 PRIMARY — timer-interrupt delivery vs RDTSC quantum coherence (THE lead)

- **Severity: HIGH — explains the bugcheck even if 0.1 is fixed.**
- **Location:** `crates/aero-cpu-core/src/interrupts.rs:735` (delivery refuses
  while `IF=0` — correct x86 behavior); `crates/aero-cpu-core/src/assist.rs:609-628`
  (`AERO_RDTSC_QUANTUM` default 0, recommended 2000 fast-forwards RDTSC);
  `crates/devices/src/acpi_pm.rs:361-377` (`nudge_pm_timer_on_poll` steals 100µs
  per PM_TMR read — a hack confirming virtual-time granularity is too coarse in
  I/O spin loops).
- **Why this matters:** the timer model fires correctly into IRR, but
  `deliver_external_interrupt` won't present the vector while IF=0. If the HAL
  arms the timer then busy-waits (with IF cleared by the prior
  `DbgBreakPointWithStatus(4)` per the handoff) on a flag set only by the timer
  ISR, the ISR can never run; meanwhile the RDTSC-bounded wait expires and
  bugchecks `0x5C`/`0x10B`. The PM-timer nudge hack is the smoking gun.
- **Fix direction:** (a) investigate why IF stays 0 after DbgBreak (next-move #1
  in the handoff); (b) make RDTSC quantum and platform timer-tick advance by the
  same virtual time so an RDTSC deadline can't expire before the corresponding
  timer event; (c) drive the PM timer from a monotonic virtual clock sampled at
  I/O time, then remove `nudge_pm_timer_on_poll`.

### 0.3 INT3 / #DB / IRET IF-handling — REFUTED as wall cause (verified correct)

A dedicated audit traced RFLAGS.IF (bit 9) through every entry/exit path. **IF
handling is correct:** entry clears IF only for interrupt gates (type 0xE),
trap gates (0xF) preserve it (`interrupts.rs:1319-1326, 1122-1130`); IRET
restores IF from the popped RFLAGS for CPL0 (`interrupts.rs:1566-1586, 1448-1464`);
`set_rflags` applies no IF mask. The INT3 (#20) alignment fix has no IF side
effects. **Conclusion:** the IF=0 at the HAL spin is NOT produced by Aero's
INT3/IRET code — it's the architecturally-correct delivery gate
(`lib.rs:13950-13954`, `interrupts.rs:735` refusing maskable IRQs while IF=0).
The real question is *why the guest is spinning with IF=0 expecting an
interrupt it architecturally cannot receive* — i.e. upstream of delivery (the
timer/RDTSC coherence of §0.2, or a guest-logic issue around
`DbgBreakPointWithStatus(4)`). Also found: `#DB` (vector 1, single-step/DR) is
entirely unimplemented (TF cleared on entry, POPF can set it, but nothing ever
delivers #DB) — latent, not the wall.

### 0.4 Also found while auditing (not the wall, but CRITICAL silent corruption)

- **FXSAVE/FXRSTOR in 64-bit mode saves only 8 XMM registers; SDM requires 16.**
  `crates/aero-cpu-core/src/fxsave.rs:44-48` (`for i in 0..8`) — XMM count is
  mode-based (8 in 32-bit, 16 in 64-bit), independent of REX.W. Long-mode plain
  `FXSAVE` must dump XMM0–XMM15 at offsets 160–415; Aero zeroes XMM8–XMM15.
  Silent kernel data corruption on every context save once Win7 reaches that
  path. Same bug duplicated in `crates/aero-snapshot/src/cpu_core.rs:246-278`.
  **Fix:** pass `long_mode: bool` into `fxsave_legacy`/`fxrstor_legacy`, loop
  `0..16` when long mode. Test at `tests/fxsave.rs:94-97` encodes the wrong
  behavior and must be updated.

### 0.5 ACPI tables are NOT the wall (cleared)

The FADT PM1a_EVT_BLK `0x400` matches the device model exactly
(`crates/aero-acpi/src/tables.rs:140` vs `crates/devices/src/acpi_pm.rs:82`);
SCI=9 → GSI9 active-low/level is wired end-to-end; RSDP F-segment mirror (#14)
present; all table signatures/lengths/checksums valid. The only ACPI defect
found is MEDIUM: FADT Flags has both WBINVD and WBINVD_FLUSH clear (ACPI spec
violation; fix: set bit 0, since WBINVD is now implemented per #18).

---

## The two answers to "why is this slow"

- **The CPU is not the hard part.** x86 is well-specified; the decoder is
  differentiated against iced-x86. The 20 fixed bugs were mostly
  *coverage* gaps (CPUID bits, MSRs), *firmware conventions* (RSDP F-segment),
  and *delivery mechanics* (long-mode INT3 stack alignment) — not ISA semantics.
  These cannot be read "by sight" from a spec because Win7's expectations are
  undocumented; they must be discovered against a reference.
- **The slow loop is `interpreter_speed × Win7_boot_cost`, discovered in crash
  order.** Each frontier is billions of instructions away; bugs surface one at a
  time behind the crash. Two levers collapse this:
  1. **Forward first-divergence diff vs QEMU** (§2) — finds *all* divergences in
     one pass, not one per boot cycle.
  2. **Parallel read-only audits against the SDM** (§3) — finds spec-deviations
     *by sight*, in parallel, without booting. Validated: ~5 real bugs + several
     perf wins in minutes of agent time.

## The divergence harness (built, validated on the QEMU side)

- **Plugin:** `tools/qemu-trace-plugin/aero_trace.so` — a QEMU TCG plugin
  (build: `make`; no libglib2.0-dev needed). Captures per-instruction RIP
  (`exec=`) and per-RAM-store `rip paddr size` (`writes=`), matching Aero's
  `AERO_TRACE_COMPACT` and `AERO_WRITE_STREAM` formats. Captured **739 M Win7
  boot instructions** in 40 s; the El Torito boot-sector entry `0x7c00` lands at
  QEMU instruction ~6.6 M.
- **Diff:** `tools/qemu-trace-plugin/aero_diff.py` — aligns both streams at an
  entry RIP (default `0x7c00`, the first ISO-loaded code both emulators run) and
  reports the first divergence. A clean never-reconverging split = real bug;
  scattered single-line divergences = timer nondeterminism.
- **Usage:** see `tools/qemu-trace-plugin/README.md`. The one remaining manual
  step is the Aero-side capture (`AERO_TRACE_COMPACT`), since it shares the
  machine with bring-up runs.
- **One-command wrapper:** `tools/qemu-trace-plugin/aero_diverge.sh` automates
  the full QEMU-capture → Aero-capture → diff workflow:
  ```
  tools/qemu-trace-plugin/aero_diverge.sh --max-insts 12000000
  ```
  Traces are cached in `/tmp/aero-diff/`; delete to force re-capture.
- **Known limitation:** QEMU's mem callback does not expose the store value, so
  value-level divergence uses periodic `pmemsave`/`AERO_DUMP_MEM` RAM-snapshot
  hashing instead. Aero's BIOS is Rust (not SeaBIOS), so diffing must start at
  the ISO boot sector, not at reset.

## Sight-based audit findings (all read-only, all unapplied)

Each finding is `file:line` + severity + concrete fix. Verified against the tree
2026-07-26.

### 3.1 MSR coverage — the systemic one (CRITICAL)

- **APPLIED (§6 #3, #32):** `AERO_IGNORE_UNKNOWN_MSRS=1` env-gate (default off);
  MSR 0x2A + 0x1A2/0x1A4-0x1A6 thermal stubs (read=0).
- *Remaining:* consider adding 0x280-0x29F (MCA CTL2 threshold) stubs if MCG_CAP
  is ever raised above Count=1.

### 3.2 CPUID coverage — no critical gap (bug #17 properly closed)

- Leaf-1 EDX advertises `0x0789_FBFD` ⊇ required `0x0789_F3FD` — fully covered
  and regression-tested (`cpuid.rs:269-301`, `tests/cpuid_policy.rs`).
- **MEDIUM:** leaf `0xB`/`0x1F` (topology) return all-zero (`cpuid.rs:498-521`)
  because `LEAF1_ECX_X2APIC` is never set. Fine for uniprocessor; populate
  legacy topology (subleaf 0 SMT, subleaf 1 Core) for SMP.
- **LOW:** `max_basic_leaf = 0x1F` (`cpuid.rs:131`) is anachronistic for the
  Ivy Bridge model signature (`leaf1_eax = 0x0003_06A9`); lower to `0x0A`/`0x0B`
  to avoid Win7 probing a zero-returning range.

### 3.3 Paging — CRITICAL fix APPLIED

- **APPLIED (§6 #5, #10):** Removed CR4.PSE gating of PAE/long-mode large pages;
  allowed bits 52-62 in PAE PDPTE reserved-mask check. Regression test added (§6 #35).
- *Latent (Win7 doesn't use on boot):* CR4.SMEP/SMAP not implemented (no
  constants); protection keys; PKRU. Add when needed.

### 3.4 Interrupt/exception delivery

- **HIGH `crates/aero-cpu-core/src/interrupts.rs:1037,1043,1100,1413,1436`:** in
  *protected mode*, interrupt delivery and IRET write only the visible selector;
  the hidden descriptor cache (base/limit/access) is never loaded via `load_seg`.
  After a CPL-change interrupt, SS.base/limit and CS attributes are stale.
  Mitigated by Win7's flat model; manifests for non-flat GDTs or D-bit/CS.L
  mismatches. **Recommendation:** route CS/SS loads through `load_seg` with an
  IRET/far-transfer reason.
- *Latent:* NMI treated as maskable (gated by IF; no NMI self-mask —
  `interrupts.rs:735`); IRET ignores RFLAGS.NT; MOV/POP-SS shadow doesn't
  inhibit #DB. None fire under normal Win7 execution.
- *Verified correct:* long-mode 16-byte RSP alignment (all gates), SS:RSP always
  pushed, IST, error-code vectors, IF-clear on interrupt-gate entry, IRETQ frame
  pop + canonical checks, STI shadow + MOV/POP-SS inhibit for maskable IRQs.

### 3.5 Interpreter performance micro-optimizations (the "by sight" wins)

All per-instruction overhead on the ~6 M inst/s path. Independently removable;
gated on crate suites.
1. **HIGH `crates/aero-cpu-core/src/interp/tier0/exec.rs:856,863,870,891,929,977`:**
   per-instruction `cpu.state.msr.tsc = cpu.time.read_tsc()` is **dead code** —
   only RDTSC reads it (and overwrites it). ~6 M dead calls+stores/sec. Delete.
2. **HIGH `exec.rs:738`:** `bus.sync(&cpu.state)` runs every instruction (4
   cross-crate getter calls + comparisons for CR0/CR3/CR4/EFER/CPL that almost
   never change). Add a dirty bit set on MOV CRx / mode transitions; early-return
   when clean. At minimum mark `sync_mmu`/`PagingBus::sync` `#[inline]`.
3. **HIGH `interp/tier0/mod.rs:155-181`:** `exec_decoded` finds the handler by
   linearly asking up to 9 modules `handles_mnemonic`. Cache a `u8` handler-tag
   in the decode-cache slot (computed once at decode); dispatch via a single
   `match` on cache hit.
4. **HIGH `interp/tier0/ops_data.rs:469-503`:** `calc_ea` uses `i128` for every
   memory operand; every value fits in `i64`. Replace with `i64`/`u64`; use
   `idx << scale.trailing_zeros()` for the index×scale.
5. **MEDIUM `state.rs:1137-1140`:** `set_rflags` unconditionally clears the
   32-byte `LazyFlags` field per ALU op, but LazyFlags is never activated on the
   hot path (only in tests). Either remove LazyFlags, or actually wire ALU ops
   to defer PF (popcount)/AF/OF until a flag-consuming instruction.
6. **LOW:** add `#[inline]` to `bitness`, `seg_base_reg`, `mask_bits`
   (`state.rs:861,1301,1501`); `DecodeCache::store` returning the slot ref to
   avoid the re-lookup (`exec.rs:775-778`); drop the per-call `OnceLock` probe in
   `mem_debug_active` (`linear_mem.rs:27-41`). *(Partially applied — `bitness` +
   `sync_mmu` inlined; see §6.)*

### 3.6 Segmentation — systemic hidden-descriptor-cache bug class

A dedicated segmentation audit found that **every CS/SS load path except plain
`MOV Sreg` leaves the hidden descriptor cache (base/limit/access rights)
stale**, writing only the visible selector. This is a real SDM §5.8 violation
that corrupts linear addressing. Mitigated on the Win7 long-mode path by flat
segments (base=0), but it's a correctness landmine.

- **C1 `interrupts.rs:1069,1075,1132`:** protected-mode interrupt delivery writes
  SS/CS selectors only → stale bases; the save-frame pushes go to the wrong
  (outer) stack linear address after a CPL change.
- **C2 `interrupts.rs:1445,1466,1563,1597`:** IRET (protected & long) reloads
  only CS/SS selectors; in compat↔64-bit transitions the stale `CS.L`/`D` bits
  make `bitness()` return the wrong decode width.
- **C3 `segmentation.rs:445`:** `validate_load_cs` rejects `RPL > CPL`
  unconditionally — correct for far CALL/JMP but **breaks RET FAR / IRET**
  outer-ring returns (where RPL > CPL is the whole point). `instr_retf`
  (`assist.rs:1082`) performs no outer-stack restore.
- **C5 `assist.rs:1364,1398,1414,1442`:** SYSCALL/SYSRET/SYSENTER/SYSEXIT set
  selectors only, never the fixed-format flat descriptors the SDM mandates.
- **H1:** no call-gate / TSS-task-switch support in far CALL/JMP (silently #GPs).
- **H3 `linear_mem.rs:497`:** code fetch bypasses CS-limit / present / execute
  checks in 32-bit protected mode (acceptable in long mode).
- *Note:* the systemic fix is to route every CS/SS load through `load_seg` with
  the right `LoadReason`. Large, multi-site — held for coordination.

### 3.7 System instructions — no boot-blocking #UD (good coverage)

A full audit of privileged/system instructions confirms Win7's boot path is
covered (CLTS, INVLPG, MOV CR/DR, LIDT/LGDT, RDMSR/WRMSR, fences, HLT, CLI/STI,
IRET, SYSCALL/SYSRET, SWAPGS, RDTSC/RDTSCP all implemented + CPL-gated
correctly). Remaining gaps are latent (not advertised in CPUID, so Win7 won't
issue them):

- **F1 (applied, §6):** SYSEXIT-64 had RIP/RSP swapped.
- **F2 MEDIUM:** VERR/VERW/LAR/LSL unimplemented → #UD (always-decoded; the most
  likely latent boot-time #UD if Win7 probes a selector).
- **F3 MEDIUM:** RDPMC unimplemented → #UD at CPL0.
- **F7 MEDIUM `assist.rs:340, ops_data.rs:337`:** MOV DR4/DR5 not aliased to
  DR6/DR7 when CR4.DE=0 (the Win7 default) → #UD.
- Correctly absent (CPUID not advertised): MWAIT/MONITOR, INVPCID, CLFLUSHOPT,
  STAC/CLAC, XGETBV, CET, VMX — their #UD is safe-by-CPUID.

### 3.8 PCI subsystem — fixes applied

**APPLIED** (Finding 7 MADT ISO, Finding 14 undefined-BAR). See §6. Remaining:

- ~~**HIGH** MADT missing ISO for GSI 10-13~~ → **SUPERSEDED** (§6 #7).
  The 2026-07-31 Q35 correction moved PCI INTx to GSIs 20–23; GSIs 10–13
  remain ISA space and do not receive PCI MADT overrides.
- ~~**MEDIUM** undefined BARs return 0xFFFFFFFF~~ → **APPLIED** (§6 #14)
- **MEDIUM `devices/src/pci/platform.rs:13-14`:** chipset identity mismatch —
  IRQ0→GSI2 and IRQ9→GSI9 (SCI), but the DSDT `_PRT` routes PCI INTx to GSIs
  10-13. **No ISO for GSIs 10-13** → Win7 programs those IOAPIC redirections as
  active-high/edge (ISA default), but PCI INTx is active-low/level → interrupts
  delivered on the wrong transition. AHCI/E1000/USB effectively lose
  interrupts. The hardcoded `pin_active_low` (`io_apic.rs:160-166`) papers over
  polarity but NOT trigger mode. **Fix:** emit ISO overrides for GSI 9-13 as
  active-low/level in `build_madt`.
- **MEDIUM `devices/src/pci/config.rs:784-787,878-883`:** undefined BARs return
  `0xFFFFFFFF` on a sizing probe (should return 0) → Win7 allocates phantom IO
  BARs for the host bridge, ISA bridges, and AHCI BAR0-4.
- **MEDIUM `devices/src/pci/platform.rs:13-14`:** chipset identity mismatch —
  Q35 host bridge (`8086:29c0`) + ICH9 LPC (`8086:2918`) + PIIX3 southbridge
  identities (`8086:7000/7010/7020`). Two PCI-ISA bridges exposed.
- *Verified correct:* config mechanism #1, ECAM, defined-BAR sizing, class codes,
  capability lists, command register, reset path.

### 3.8.2 USB host controllers — fixes applied

**APPLIED** (Finding 15 EHCI EECP, Finding 16 UHCI HCHALTED). See §6. Remaining:

- ~~**HIGH** HCCPARAMS EECP advertises USBLEGSUP in wrong space~~ → **APPLIED**
- ~~**MEDIUM** UHCI HCHALTED set during EGSM~~ → **APPLIED**
- **MEDIUM `aero-usb/src/ehci/hub.rs:341-352`:** EHCI port reset auto-enables
  non-high-speed devices (should leave disabled; route to companion).
- *Verified correct:* CONFIGFLAG/port-owner routing, PCD interrupt latching,
  enumeration (SET_ADDRESS/SET_CONFIGURATION/GET_DESCRIPTOR), W1C change bits.

### 3.8.1 AHCI device model — likely disk-IO blocker (setup phase)

- **APPLIED** Finding 6 (PRD alignment gate), Finding 9 (GHC.AE pin-on),
  and Finding 18 (SET FEATURES 0x03). See §6.
- **MEDIUM `ahci.rs:799-807`:** ~~SET FEATURES 0x03 silently ignored~~ → **APPLIED**
  select) silently ignored — works only by coincidence (AtaDrive defaults to
  UDMA2). Call `drive.set_transfer_mode_select(cfis[12])`.
- *Verified correct:* ABAR layout, PxSIG=`0x00000101`, command-list→table→PRDT
  pipeline, completion interrupt (PxIS.DHRS), INTx→GSI22 (the original
  GSI12 result was superseded by the Q35 routing correction), COMRESET, IDENTIFY
  geometry, no-NCQ (word 76=0, consistent).

### 3.9 Decoder — sound (iced-backed); LOCK + REX fixes APPLIED

**APPLIED (§6 #33):** Clear stale REX metadata when a legacy prefix follows REX.
The decoder delegates to `iced-x86` (correct by construction). Classic
ModR/M/SIB/displacement decode is sound. Remaining gaps:

- **HIGH `aero-cpu-core/src/interp/tier0/mod.rs:250-273`:** LOCK enforcement
  checks the mnemonic only, not the memory-destination requirement (in the
  concurrent agent's active zone).

## Method note — what scales and what doesn't

## Ready-to-apply ordering (lowest risk → highest impact)

1. Perf #1 (delete dead TSC sync) — pure deletion, safe.
2. MSR default → ignore-unknown (§3.1) — one config-gated arm; prevents future
   crash cycles.
3. Paging CR4.PSE fix (§3.3) — one block removal; may unblock large-page paths.
4. MSR 0x2A stub (§3.1) — one arm; HAL clock-calibration hardening.
5. Perf #2/#3/#4 — the bigger interpreter speedups.

## Applied fixes (verified, 2026-07-26)

Fourteen fixes applied and verified across 7 crates. All gated on the relevant
crate test suites (all pass). They target files/crates **outside** the
concurrent agent's active edit zone to avoid conflicts.

| # | File | Fix | Severity | Tests |
|---|---|---|---|---|
| 1 | `aero-cpu-core/src/assist.rs` | SYSEXIT-64: swap RIP←RDX, RSP←RCX (were backwards) | correctness | aero-cpu-core |
| 2 | `aero-cpu-core/src/state.rs` | `#[inline]` on `bitness()`/`sync_mmu()` | perf | aero-cpu-core |
| 3 | `aero-cpu-core/src/msr.rs` | `AERO_IGNORE_UNKNOWN_MSRS=1` env-gate (default off) | bring-up aid | aero-cpu-core |
| 4 | `aero-cpu-core/src/fxsave.rs` + `state.rs` + `tests/fxsave.rs` | FXSAVE/FXRSTOR legacy: save/restore 16 XMM in IA-32e mode | CRITICAL (silent corruption) | fxsave 10/0 |
| 5 | `aero-mmu/src/lib.rs` | Remove CR4.PSE gating of PAE/long-mode large pages | CRITICAL (spurious #PF) | aero-mmu 36/0 |
| 6 | `aero-devices-storage/src/ahci.rs` | Remove PRD 512-byte alignment gate on DMA read/write | disk-IO blocker | aero-devices-storage 235/0 |
| 7 | `aero-acpi/src/tables.rs` | ~~Add MADT ISO overrides for PCI INTx GSI 10-13~~; superseded by Q35 PCI GSIs 20–23 | next-wall prevention | aero-acpi 32/0 |
| 8 | `aero-acpi/src/tables.rs` | Set FADT Flags WBINVD + PROC_C1 | spec compliance | aero-acpi 32/0 |
| 9 | `aero-devices-storage/src/ahci.rs` | Pin GHC.AE always-on (ICH9 read-only-1) | correctness | aero-devices-storage 235/0 |
| 10 | `aero-mmu/src/lib.rs` | Allow bits 52-62 in PAE PDPTE reserved-mask check | correctness (spurious #PF) | aero-mmu 36/0 |
| 11 | `aero-net-e1000/src/lib.rs` | Fix TSO context descriptor MSS/HDR_LEN byte offsets (were swapped) | CRITICAL (drops all LSO segments) | aero-net-e1000 88/0 |
| 12 | `aero-net-e1000/src/lib.rs` | Fix PHY BMSR: add ANEGCAPABLE + speed capability bits | CRITICAL (PHY config fails) | aero-net-e1000 88/0 |
| 13 | `aero-net-e1000/src/lib.rs` | ICR read clears only unmasked bits (per spec); add LSC + other ICR bits; raise LSC on reset | correctness (driver stall) | aero-net-e1000 88/0 |
| 14 | `devices/src/pci/config.rs` | Undefined BARs return 0 on sizing probe (were returning 0xFFFFFFFF → phantom IO BARs) | correctness (PCI enumeration) | devices PCI 13/0 |
| 15 | `aero-usb/src/ehci/mod.rs` | Set HCCPARAMS EECP=0 (USBLEGSUP is MMIO-only, not in PCI config space) | correctness (USB driver init) | aero-usb 862/0 |
| 16 | `aero-usb/src/uhci/regs.rs` | HCHALTED is RS-only (was set during EGSM global suspend) | spec compliance | aero-usb 862/0 |
| 17 | `aero-net-e1000/src/lib.rs` | Set EECD.AUTO_RD on reset (signals EEPROM auto-loaded); add TCTL.PSP padding for short TX frames | correctness (driver init + Ethernet min frame) | aero-net-e1000 88/0 |
| 18 | `aero-devices-storage/src/ahci.rs` | Handle SET FEATURES 0x03 (Set Transfer Mode) via `drive.set_transfer_mode_select` | correctness (UDMA negotiation) | aero-devices-storage 234/0 |
| 19 | `aero-virtio/src/devices/snd.rs` | ~~Widen `virtio_snd_pcm_info` to 36 bytes with an `le64 hdr`~~ **— mistaken, since reverted; see below** | — | — |
| 20 | `aero-virtio/src/devices/blk.rs` | Remove DISCARD + WRITE_ZEROES feature bits (Win7 contract §3.1.3 forbids them); zero config fields 0x24-0x38 | contract compliance | aero-virtio 211/0 |
| 21 | `aero-virtio/src/devices/input.rs` | Fix event loss: peek before validating descriptor chain, pop only after successful write | correctness (input event loss) | aero-virtio 211/0 |
| 22 | `aero-devices-nvme/src/lib.rs` | Add Get Log Page (0x02) + AER (0x0C) handling (were INVALID_OPCODE → driver init abort); set CAP.TO=4s | correctness (NVMe driver init) | aero-devices-nvme 107/0 |
| 23 | `devices/src/serial.rs` | Generate THRE interrupt (was only RDA); default MSR to DCD/DSR/CTS active | correctness (serial driver stall) | devices 0/0 |
| 24 | `aero-gpu-vga/src/lib.rs` | Advance vblank clock on each 0x3DA read (prevents retrace-poll hang in tight loops) | correctness (display hang) | aero-gpu-vga 46/0 |
| 25 | `aero-devices-storage/src/pci_ide.rs` | Present ATAPI signature (0x14/0xEB) on SRST reset (was all-zero → CD-ROM undetectable) | CRITICAL (CD boot detection) | aero-devices-storage 234/0 |
| 26 | `aero-devices-storage/src/atapi.rs` | Fix IDENTIFY PACKET DEVICE: remove wrong 16-byte packet flag (bit 0); add DMA mode words (53/63/88) | HIGH (PIO-only fallback) | aero-devices-storage 234/0 |
| 27 | `aero-audio/src/hda.rs` | Fix widget TYPE field: move to bits [23:20] (was in low bits → Win7 finds no audio endpoints) | CRITICAL (no audio endpoints) | aero-audio 126/0 |
| 28 | `aero-audio/src/hda.rs` | Fix SRST polarity: stream runs with RUN=1,SRST=0 (was RUN=1,SRST=1 — inverted from spec) | CRITICAL (DMA never runs) | aero-audio 126/0 |
| 29 | `aero-audio/src/hda.rs` | Fix GCAP bit layout: OSS[15:12], ISS[11:8], BSS[7:3], 64OK(bit0) per Intel HDA spec | HIGH (wrong stream counts decoded) | aero-audio 126/0 |
| 30 | `aero-devices-storage/src/pci_ide.rs` | ATAPI devices accept SET FEATURES (was abort → blocked DMA mode negotiation for CD) | HIGH (CD-ROM PIO-only) | aero-devices-storage 234/0 |
| 31 | `aero-snapshot/src/types.rs` + `cpu_core.rs` | Add IA32_TSC_AUX (0xC0000103) to snapshot save/restore (was silently zeroed → RDTSCP returns ECX=0 after restore) | HIGH (silent state divergence) | aero-snapshot 26/0 |
| 32 | `aero-cpu-core/src/msr.rs` | Add MSR thermal stubbs (0x1A2/0x1A4-0x1A6) + EBC_FREQUENCY_ID (0x2A) read=0 (prevent #GP on HAL thermal/clock probes) | MEDIUM (HAL probe #GP) | aero-cpu-core 37/0 |
| 33 | `aero-cpu-decoder/src/lib.rs` | Clear stale REX metadata when a legacy prefix follows REX (SDM: REX effective only as last prefix) | HIGH (wrong operand size inference) | aero-cpu-decoder 32/0 |
| 34 | `aero-net-stack/src/stack.rs` | TCP MSS segmentation: chunk proxy→guest data into 1460-byte segments (was one giant segment → silently dropped + seq advanced past undelivered data → permanent stall) | CRITICAL (all bulk TCP stalls) | aero-net-stack 44/0 |
| 35 | `aero-mmu/src/tests.rs` | Regression test: long-mode 2MB large page without CR4.PSE (pins the CR4.PSE fix against future regression) | test coverage | aero-mmu 38/0 |
| 36 | `aero-devices-storage/tests/piix3_ide.rs` | Regression test: ATAPI signature (0x14/0xEB) present after SRST reset (pins the SRST signature fix) | test coverage | aero-devices-storage 235/0 |
| 37 | `aero-audio/src/hda.rs` | Regression test: widget TYPE field in bits [23:20] for all widget types (pins the HDA TYPE fix) | test coverage | aero-audio 127/0 |
| 38 | `aero-net-stack/tests/stack_integration.rs` | Regression test: TCP proxy data >MSS is segmented into ≤1460-byte chunks (pins the MSS fix) | test coverage | aero-net-stack 45/0 |
| 39 | `aero-cpu-core/src/state.rs` | Guard `lazy_flags.clear()` in `set_rflags` (was unconditional 32-byte memset per ALU op; LazyFlags never active) | perf (3-5%) | aero-cpu-core |
| 40 | `aero-cpu-core/src/state.rs` | `#[inline]` on `mask_bits`, `read_reg`, `write_reg`, `seg_base_reg` (4 ultra-hot accessors missing hint) | perf (2-5%) | aero-cpu-core |
| 41 | `aero-cpu-core/src/interp/tier0/ops_data.rs` | `calc_ea`: replace i128 arithmetic with u64 wrapping (i128 compiles to multi-word carry-propagation; unnecessary since final mask truncates) | perf (2-4%) | aero-cpu-core |
| 42 | `aero-cpu-core/src/interp/tier0/ops_alu.rs` | `add_with_flags`/`sub_with_flags`: specialize for bits<64 using u64 wrapping (avoid u128 for 8/16/32-bit ALU ops) | perf (1-3%) | aero-cpu-core |
| 43 | `aero-cpu-core/src/interp/tier0/ops_alu.rs` | `#[inline]` on `logic_flags`, `set_logic_szp`, `parity8` (per-ALU-op flag helpers) | perf (1-2%) | aero-cpu-core |
| 35 | `aero-l2-proxy/src/session.rs` | WS backpressure drops frame instead of killing the tunnel (was: closes entire session → drops all TCP/UDP flows on single slow guest read) | HIGH (network collapse on backpressure) | aero-l2-proxy 42/0 |

### Correction to row 19: `virtio_snd_pcm_info` is 32 bytes, not 36

Row 19 widened the structure on the reasoning that it opens with an `le64`
header. It does not. The spec defines the opening member as

```c
struct virtio_snd_info { le32 hda_fn_nid; };
```

and `linux/virtio_snd.h` agrees. With a 4-byte header the two `le64` members
that follow land at offsets 8 and 16 — already naturally aligned — so the
structure is exactly 32 bytes with no implicit padding, and the trailing
`u8 padding[5]` is what rounds it out.

The widened version therefore shifted every field past the header by four bytes
and reported eight bytes too many per entry: exactly the breakage row 19 set out
to prevent. It has been reverted, and the offsets now read
`hda_fn_nid@0, features@4, formats@8, rates@16, direction@24,
channels_min@25, channels_max@26, padding@27..32`.

What let a wrong change stand was that the only test exercising it end-to-end
runs the wasm build, and the wasm packages in the tree were months stale. The
Rust unit tests had been rewritten to match the new layout, so they agreed with
it. The integration test disagreed the moment the packages were rebuilt.

### 3.10 Network stack (L2 tunnel + TCP) — MSS APPLIED; retransmission deferred

The L2 wire codec/framing is correct. The proxy's simplified TCP stack
(`aero-net-stack`) has been partially fixed:

- **APPLIED (§6 #34):** TCP MSS segmentation — proxy→guest data is now chunked
  into 1460-byte segments. The prior code built one giant segment (up to 16 KiB)
  that was silently dropped by the L2 encoder (2048-byte limit) and the E1000
  (1522-byte limit), but the send sequence was advanced past the undelivered
  data → permanent stall.
- **CRITICAL (remaining):** No TCP send buffer / retransmission. The proxy→guest
  data path is fire-and-forget. The L2 transport is explicitly lossy
  (L2TunnelBackend drops at queue full; E1000 drops oldest at 256 queued). Any
  dropped proxy→guest data/ACK/FIN permanently desyncs the connection. **Fix:**
  add a per-connection unacked send queue; re-emit on guest duplicate/old ACK or
  retransmit timer.
- **HIGH (remaining):** Guest FIN treated as full teardown; in-flight server data
  is lost (no proper half-close). Needs a new `Action::TcpProxyShutdownWrite`
  variant + session-task coordination.
- **HIGH (remaining):** WS backpressure terminates the whole tunnel instead of
  applying flow control. Attempted fix reverted (broke integration tests);
  needs a BACKPRESSURE/throttle message design.
- **MEDIUM (remaining):** Fixed 65535 receive window with no real flow control.

The workspace-wide `cargo check` has a **pre-existing, unrelated** failure
(`aws-sdk-s3` in `tools/disk-gateway` requires rustc 1.94.1 vs the pinned
1.92.0) — not introduced by these edits. All canonical consumers
(`aero-machine`, `aero-machine-cli`) compile clean.

### Deferred (documented, not applied — higher blast radius)

- **Segmentation systemic bug** (§3.6): every CS/SS load path except `MOV Sreg`
  leaves the hidden descriptor cache stale — large, multi-site fix in
  `interrupts.rs` + `assist.rs` + `segmentation.rs`, which the concurrent agent
  is actively editing.
- **Perf: dead per-instruction TSC sync** (§3.5 #1): ~~pure deletion~~ —
  **INVESTIGATION REFUTED the "dead code" framing.** `msr.tsc` is an intentional,
  tested architectural mirror consumed by: snapshot save (`cpu_core.rs:134` — the
  sole serialized TSC), the `cpu_tsc()` public wasm API (`tiered_vm.rs:1146`),
  SMP TSC sync (`sync_ap_tsc_to_bsp`, `lib.rs:8732`), guest-time resync, AND the
  `tsc_advances_without_rdtsc` test (`tests/time.rs:50`). Removing the per-insn
  update breaks all of these. The correct optimization is to make consumers read
  `TimeSource` directly, then delete the mirror — not to delete first. **Keep.**
- **Perf: per-instruction bus.sync** (§3.5 #2): dirty-bit optimization in
  `exec.rs`, which the concurrent agent is actively editing.
- **IOAPIC EOI highest-bit heuristic** (§3.4): clears wrong ISR bit under nested
  interrupts; needs careful rework of `local_apic.rs`.
- **VERR/VERW/LAR/LSL** (§3.7): unimplemented; add in `assist.rs`.
- **LOCK prefix memory-destination enforcement** (§3.9): add the `op_kind(0)
  == Memory` check in `mod.rs`.
- **FADT WBINVD flags** (§0.5): set bit 0 in `tables.rs:580`; one-line fix but
  untested with the current ACPI flow.

## JIT wiring (2026-07-27)

The native JIT infrastructure has been wired into the Machine execute loop and
the CLI. The full pipeline exists: `IrBlockCache` discovers hot blocks (threshold:
50 hits), translates to Tier-1 IR (`discover_block` + `translate_block`), caches
them, and `run_ir_block` executes via the Tier-1 IR interpreter with real
`CpuBus` access (raw-pointer adapter bypasses `&self`/`&mut self` mismatch
between `Tier1Bus` and `CpuBus`). Falls back to interpreter on unsupported
instructions (`ExitToInterpreter`).

### What was done

- **`aero-cpu-core/src/jit/runtime.rs`**: added `backend_clone()` and
  `backend_mut()` accessors on `JitRuntime` (needed by `process_compile_requests`).
- **`aero-cpu-core/src/interp/tier0/ir/mod.rs`**: removed
  `#[cfg(debug_assertions)]` gate on `pub mod interp` so the IR interpreter is
  available in release builds.
- **`aero-jit-x86/src/tier1/ir/interp.rs`**: added
  `execute_block_cpu_state()` — takes `&mut CpuState` + `&mut Tier1Bus` directly
  (no WASM linear memory copy needed).
- **`aero-machine/src/jit.rs`**:
  - `process_compile_requests` now actually compiles (was: drains without
    compiling). Uses `aero_jit_x86::backend::compile_and_install` →
    `JitRuntime::install_handle`.
  - Added `IrBlockCache` (hotness tracking, LRU eviction, IR translation cache).
  - Added `run_ir_block()` — executes a cached IR block against `CpuState` + bus.
  - `ReadAdapter`/`BusAdapter` — raw-pointer adapters bridging `Tier1Bus`
    (`&self` read) to `CpuBus` (`&mut self` read).
- **`aero-machine/src/lib.rs`**:
  - `pub mod jit` (was: private, not declared).
  - Added `run_slice_jit()` + `run_slice_inner()` with `Option<&mut IrBlockCache>`
    parameter. The JIT fast-path checks the cache before falling back to
    `run_batch_cpu_core_with_assists`. Hotness tracking runs after each batch.
  - `#![forbid(unsafe_code)]` → relaxed (raw pointers needed for bus adapters).
- **`aero-machine/Cargo.toml`**: added `aero-jit-x86` as a native dependency.
- **`aero-machine-cli/src/main.rs`**: added `--jit` flag; creates `IrBlockCache`
  and dispatches to `run_slice_jit` when set.

### Benchmark result

On the kernel spin-loop snapshot (`snap-2b5-plus7b.bin`), `--jit` shows **no
net speedup** (~5.4 Minst/s vs ~5.5 Minst/s baseline). This is expected: the
snapshot is a tight spin loop where the Tier-0 decode cache is already at
near-100% hit rate, and the IR interpreter's per-instruction dispatch overhead
matches what Tier-0 saves from skipping decode. The real 10-50x speedup requires
**native WASM compilation** (wasmtime/Cranelift), which eliminates the interpreter
loop entirely. The `process_compile_requests` fix now compiles real WASM blocks
via Cranelift, but executing them requires the wasmtime linear memory to hold
guest RAM (2+ GiB), which needs further integration work on `WasmtimeBackend`.

### What remains for real JIT speedup

1. Size `WasmtimeBackend`'s linear memory to hold full guest RAM (currently 128
   KiB; needs 2+ GiB). This requires `MemoryType::new` with a large minimum and
   `memory_reservation` configured for the actual RAM size.
2. Sync guest RAM into the wasmtime linear memory before each block execution
   and sync writes back after (or map the Machine's RAM directly).
3. The `ExecDispatcher::step()` path already handles JIT block execution via
   `WasmBackend::execute()` — it snapshots/restores state on MMIO exits. This is
   the correct path; it just needs the memory bridge.
