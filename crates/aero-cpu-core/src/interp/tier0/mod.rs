//! Tier-0 interpreter.
//!
//! This is the default interpreter for `aero_cpu_core` and operates directly on
//! the canonical JIT-ABI [`crate::state::CpuState`] + [`crate::mem::CpuBus`].
//! Higher tiers (JIT) build on the same state layout.

pub mod exec;

mod ops_alu;
mod ops_atomic;
mod ops_atomics;
mod ops_cf;
mod ops_data;
mod ops_fx;
mod ops_sse;
mod ops_string;
mod ops_x87;

use crate::cpuid::{CpuFeatureSet, CpuFeatures};
use crate::exception::{AssistReason, Exception};
use crate::fpu::FpKind;
use crate::linear_mem::{
    contiguous_masked_start, write_u16_wrapped, write_u32_wrapped, write_u64_wrapped,
};
use crate::mem::CpuBus;
use crate::state::{mask_bits, CpuState, CR0_EM, CR0_MP, CR0_NE, CR0_TS, CR4_OSFXSR};
use aero_x86::{DecodedInst, Mnemonic};

/// Configuration inputs for the Tier-0 interpreter.
///
/// Tier-0 executes directly against [`crate::state::CpuState`], so CPU-wide
/// knobs like CPUID feature reporting live outside the architectural state and
/// are plumbed in via this config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tier0Config {
    pub features: CpuFeatureSet,
    /// Whether a Tier-0 batch returns to its caller on a taken branch.
    ///
    /// Defaults to `true` (historical behaviour: every branch ends the batch).
    /// `Machine::run_slice` sets this to `false` so branch-dense boot code does not
    /// force a full device re-poll + paging-bus rebuild after nearly every branch
    /// (the dominant native-interpreter cost); see `exec.rs`. Callers that drive the
    /// batch as a "run until branch" loop (most tests) leave this `true`.
    pub exit_on_branch: bool,
}

impl Tier0Config {
    /// Construct a Tier-0 configuration from the guest-visible CPUID surface.
    ///
    /// Tier-0 instruction gating must match what `CPUID` advertises to the guest
    /// (via [`crate::assist::AssistContext`]). Use this helper to keep Tier-0
    /// coherent with the `CpuFeatures` policy.
    pub fn from_cpuid(features: &CpuFeatures) -> Self {
        Self {
            features: features.feature_set(),
            exit_on_branch: true,
        }
    }
}

impl Default for Tier0Config {
    fn default() -> Self {
        Self {
            // Tier-0 defaults to the minimum viable Win7 x86-64 profile.
            // Individual tests can override this to exercise optional features.
            features: CpuFeatureSet::win7_minimum(),
            exit_on_branch: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecOutcome {
    Continue,
    /// Like [`ExecOutcome::Continue`], but requests that the execution engine
    /// inhibit maskable interrupts for exactly one instruction (MOV SS / POP SS
    /// interrupt shadow semantics).
    ContinueInhibitInterrupts,
    Branch,
    Halt,
    Assist(AssistReason),
}

fn atomic_rmw_sized<B: CpuBus, R>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
    bits: u32,
    f: impl FnOnce(u64) -> (u64, R),
) -> Result<R, Exception> {
    let len = usize::try_from(bits / 8).map_err(|_| Exception::InvalidOpcode)?;
    if let Some(start) = contiguous_masked_start(state, addr, len) {
        return match bits {
            8 => bus.atomic_rmw::<u8, _>(start, |old| {
                let (new, ret) = f(old as u64);
                (new as u8, ret)
            }),
            16 => bus.atomic_rmw::<u16, _>(start, |old| {
                let (new, ret) = f(old as u64);
                (new as u16, ret)
            }),
            32 => bus.atomic_rmw::<u32, _>(start, |old| {
                let (new, ret) = f(old as u64);
                (new as u32, ret)
            }),
            64 => bus.atomic_rmw::<u64, _>(start, f),
            _ => Err(Exception::InvalidOpcode),
        };
    }

    // Wrapped (split) path: this is required when the linear address range wraps
    // (32-bit wrap in non-long modes, A20 alias wrap in real/v8086 with A20 off).
    // We cannot use `CpuBus::atomic_rmw` because it assumes a contiguous linear
    // range starting at `addr`.
    //
    // `atomic_rmw` also has write-intent semantics even when `new == old`. To
    // preserve that behavior here, read each byte through `CpuBus::atomic_rmw`
    // (so paging-aware busses perform write-intent translation/permission
    // checks) while still applying `CpuState::apply_a20` per byte.
    let mut old = 0u64;
    for i in 0..len {
        let byte_addr = state.apply_a20(addr.wrapping_add(i as u64));
        let b = bus.atomic_rmw::<u8, _>(byte_addr, |old| (old, old))?;
        old |= (b as u64) << (i * 8);
    }
    let mask = mask_bits(bits);
    old &= mask;

    let (new, ret) = f(old);
    let new = new & mask;
    if new != old {
        match bits {
            8 => bus.write_u8(state.apply_a20(addr), new as u8)?,
            16 => write_u16_wrapped(state, bus, addr, new as u16)?,
            32 => write_u32_wrapped(state, bus, addr, new as u32)?,
            64 => write_u64_wrapped(state, bus, addr, new)?,
            _ => return Err(Exception::InvalidOpcode),
        }
    }
    Ok(ret)
}

/// Which `ops_*` module owns a mnemonic.
///
/// Resolving this used to mean walking every module's `handles_mnemonic` in
/// order on every instruction: a `mov` paid five of those calls and an SSE op
/// paid all nine, each one re-deriving the same answer. The classification only
/// depends on the mnemonic, so it is computed once per mnemonic and memoised.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
enum OpsGroup {
    Atomics = 1,
    Cf,
    Atomic,
    Data,
    Alu,
    Fx,
    X87,
    /// `MOVSD`/`CMPSD` name both a string and an SSE instruction, so a mnemonic
    /// match is not enough: try the operand-aware string check, then SSE.
    StringThenSse,
    Sse,
    /// Handled by the tail `match` (privileged/system instructions and assists).
    Tail,
}

const OPS_GROUP_UNCLASSIFIED: u8 = 0;

impl OpsGroup {
    fn from_repr(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::Atomics,
            2 => Self::Cf,
            3 => Self::Atomic,
            4 => Self::Data,
            5 => Self::Alu,
            6 => Self::Fx,
            7 => Self::X87,
            8 => Self::StringThenSse,
            9 => Self::Sse,
            10 => Self::Tail,
            _ => return None,
        })
    }
}

/// The original dispatch chain, in its original order, as a pure function.
fn classify_mnemonic(mnem: Mnemonic) -> OpsGroup {
    if ops_atomics::handles_mnemonic(mnem) {
        OpsGroup::Atomics
    } else if ops_cf::handles_mnemonic(mnem) {
        OpsGroup::Cf
    } else if ops_atomic::handles_mnemonic(mnem) {
        OpsGroup::Atomic
    } else if ops_data::handles_mnemonic(mnem) {
        OpsGroup::Data
    } else if ops_alu::handles_mnemonic(mnem) {
        OpsGroup::Alu
    } else if ops_fx::handles_mnemonic(mnem) {
        OpsGroup::Fx
    } else if ops_x87::handles_mnemonic(mnem) {
        OpsGroup::X87
    } else if ops_string::handles_mnemonic(mnem) {
        OpsGroup::StringThenSse
    } else if ops_sse::handles_mnemonic(mnem) {
        OpsGroup::Sse
    } else {
        OpsGroup::Tail
    }
}

/// Comfortably above the iced mnemonic count; a mnemonic outside it falls back
/// to computing the group directly, so the bound is a memory cap, not a limit.
const OPS_GROUP_TABLE_LEN: usize = 4096;

static OPS_GROUP_TABLE: [core::sync::atomic::AtomicU8; OPS_GROUP_TABLE_LEN] =
    [const { core::sync::atomic::AtomicU8::new(OPS_GROUP_UNCLASSIFIED) }; OPS_GROUP_TABLE_LEN];

#[inline]
fn ops_group(mnem: Mnemonic) -> OpsGroup {
    use core::sync::atomic::Ordering;

    let idx = mnem as usize;
    let Some(slot) = OPS_GROUP_TABLE.get(idx) else {
        return classify_mnemonic(mnem);
    };
    if let Some(group) = OpsGroup::from_repr(slot.load(Ordering::Relaxed)) {
        return group;
    }
    // Racing threads compute the same value, so an unsynchronised store is fine.
    let group = classify_mnemonic(mnem);
    slot.store(group as u8, Ordering::Relaxed);
    group
}

fn exec_decoded<B: CpuBus>(
    cfg: &Tier0Config,
    state: &mut CpuState,
    bus: &mut B,
    decoded: &DecodedInst,
    next_ip: u64,
    addr_size_override: bool,
) -> Result<ExecOutcome, Exception> {
    let mnem = decoded.instr.mnemonic();
    if decoded.instr.has_lock_prefix() && !mnemonic_allows_lock_prefix(mnem) {
        return Err(Exception::InvalidOpcode);
    }
    match ops_group(mnem) {
        OpsGroup::Atomics => return ops_atomics::exec(state, bus, decoded, next_ip),
        OpsGroup::Cf => return ops_cf::exec(state, bus, decoded, next_ip),
        OpsGroup::Atomic => return ops_atomic::exec(state, bus, decoded, next_ip),
        OpsGroup::Data => return ops_data::exec(state, bus, decoded, next_ip),
        OpsGroup::Alu => return ops_alu::exec(state, bus, decoded, next_ip),
        OpsGroup::Fx => return ops_fx::exec(state, bus, decoded, next_ip),
        OpsGroup::X87 => return ops_x87::exec(state, bus, decoded, next_ip),
        OpsGroup::StringThenSse => {
            if ops_string::handles(&decoded.instr) {
                return ops_string::exec(state, bus, decoded, next_ip, addr_size_override);
            }
            if ops_sse::handles_mnemonic(mnem) {
                return ops_sse::exec(cfg, state, bus, decoded, next_ip);
            }
        }
        OpsGroup::Sse => return ops_sse::exec(cfg, state, bus, decoded, next_ip),
        OpsGroup::Tail => {}
    }

    match mnem {
        Mnemonic::Clts => {
            // CLTS is privileged (CPL0). In real mode `cpl()` is always 0.
            if state.cpl() != 0 {
                return Err(Exception::gp0());
            }
            state.control.cr0 &= !CR0_TS;
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Wait => {
            exec_wait(state)?;
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Hlt => {
            // HLT is privileged outside real mode.
            if state.cpl() != 0 {
                return Err(Exception::gp0());
            }
            Ok(ExecOutcome::Halt)
        }
        Mnemonic::In
        | Mnemonic::Out
        | Mnemonic::Insb
        | Mnemonic::Insw
        | Mnemonic::Insd
        | Mnemonic::Outsb
        | Mnemonic::Outsw
        | Mnemonic::Outsd => Ok(ExecOutcome::Assist(AssistReason::Io)),
        Mnemonic::Cpuid => Ok(ExecOutcome::Assist(AssistReason::Cpuid)),
        Mnemonic::Rdmsr | Mnemonic::Wrmsr => Ok(ExecOutcome::Assist(AssistReason::Msr)),
        Mnemonic::Int | Mnemonic::Int1 | Mnemonic::Int3 | Mnemonic::Into => {
            Ok(ExecOutcome::Assist(AssistReason::Interrupt))
        }
        Mnemonic::Iret | Mnemonic::Iretd | Mnemonic::Iretq => {
            Ok(ExecOutcome::Assist(AssistReason::Interrupt))
        }
        Mnemonic::Cli | Mnemonic::Sti => Ok(ExecOutcome::Assist(AssistReason::Interrupt)),
        // Privileged/system instructions that require additional CPU core state.
        Mnemonic::Lgdt
        | Mnemonic::Lidt
        | Mnemonic::Sgdt
        | Mnemonic::Sidt
        | Mnemonic::Ltr
        | Mnemonic::Str
        | Mnemonic::Lldt
        | Mnemonic::Sldt
        | Mnemonic::Lmsw
        | Mnemonic::Smsw
        | Mnemonic::Invlpg
        | Mnemonic::Wbinvd
        | Mnemonic::Invd
        | Mnemonic::Swapgs
        | Mnemonic::Syscall
        | Mnemonic::Sysret
        | Mnemonic::Sysretq // REX.W SYSRET (48 0F 07); iced treats separately from Sysret
        | Mnemonic::Sysenter
        | Mnemonic::Sysexit
        | Mnemonic::Rsm
        // LAR/LSL: unprivileged descriptor-table probes used by ntdll (e.g. LSL
        // EAX,EAX). Missing → #UD → STATUS_ILLEGAL_INSTRUCTION in smss.
        | Mnemonic::Lar
        | Mnemonic::Lsl => Ok(ExecOutcome::Assist(AssistReason::Privileged)),
        Mnemonic::Rdtsc
        | Mnemonic::Rdtscp
        | Mnemonic::Lfence
        | Mnemonic::Sfence
        | Mnemonic::Mfence => Ok(ExecOutcome::Assist(AssistReason::Unsupported)),
        _ => Err(Exception::InvalidOpcode),
    }
}

fn mnemonic_allows_lock_prefix(m: Mnemonic) -> bool {
    matches!(
        m,
        Mnemonic::Add
            | Mnemonic::Adc
            | Mnemonic::And
            | Mnemonic::Btc
            | Mnemonic::Btr
            | Mnemonic::Bts
            | Mnemonic::Cmpxchg
            | Mnemonic::Cmpxchg8b
            | Mnemonic::Cmpxchg16b
            | Mnemonic::Dec
            | Mnemonic::Inc
            | Mnemonic::Neg
            | Mnemonic::Not
            | Mnemonic::Or
            | Mnemonic::Sbb
            | Mnemonic::Sub
            | Mnemonic::Xadd
            | Mnemonic::Xchg
            | Mnemonic::Xor
    )
}

pub(super) fn check_fp_available(state: &CpuState, kind: FpKind) -> Result<(), Exception> {
    let cr0 = state.control.cr0;

    // Give #UD priority over #NM: if the ISA is disabled entirely, we do not
    // report it as a lazy-FPU trap.
    if (cr0 & CR0_EM) != 0 {
        return Err(Exception::InvalidOpcode);
    }

    if matches!(kind, FpKind::Sse) && (state.control.cr4 & CR4_OSFXSR) == 0 {
        return Err(Exception::InvalidOpcode);
    }

    if (cr0 & CR0_TS) != 0 {
        return Err(Exception::DeviceNotAvailable);
    }

    Ok(())
}

pub(super) fn exec_wait(state: &mut CpuState) -> Result<(), Exception> {
    let cr0 = state.control.cr0;

    if (cr0 & CR0_EM) != 0 {
        return Err(Exception::InvalidOpcode);
    }

    if (cr0 & CR0_MP) != 0 && (cr0 & CR0_TS) != 0 {
        return Err(Exception::DeviceNotAvailable);
    }

    if state.fpu.has_unmasked_exception() {
        if (cr0 & CR0_NE) != 0 {
            return Err(Exception::X87Fpu);
        }

        state.set_irq13_pending(true);
    }

    Ok(())
}

#[cfg(test)]
mod ops_group_tests {
    use super::*;

    /// The memo must agree with the chain it replaces, including on the second
    /// lookup that reads the cached value rather than recomputing it.
    #[test]
    fn memoised_group_matches_the_dispatch_chain() {
        for mnem in [
            Mnemonic::Mov,
            Mnemonic::Add,
            Mnemonic::Cmp,
            Mnemonic::Jmp,
            Mnemonic::Call,
            Mnemonic::Ret,
            Mnemonic::Push,
            Mnemonic::Xchg,
            Mnemonic::Cmpxchg,
            Mnemonic::Fld,
            Mnemonic::Fxsave,
            Mnemonic::Movsb,
            Mnemonic::Movsd,
            Mnemonic::Cmpsd,
            Mnemonic::Scasq,
            Mnemonic::Addsd,
            Mnemonic::Punpckhqdq,
            Mnemonic::Hlt,
            Mnemonic::Cpuid,
            Mnemonic::Lgdt,
        ] {
            let expected = classify_mnemonic(mnem);
            assert_eq!(ops_group(mnem), expected, "first lookup for {mnem:?}");
            assert_eq!(ops_group(mnem), expected, "cached lookup for {mnem:?}");
        }
    }

    /// `MOVSD`/`CMPSD` are shared between the string and SSE encodings, so they
    /// must land in the group that tries the operand-aware string check first.
    #[test]
    fn shared_string_and_sse_mnemonics_take_the_operand_aware_path() {
        assert_eq!(ops_group(Mnemonic::Movsd), OpsGroup::StringThenSse);
        assert_eq!(ops_group(Mnemonic::Cmpsd), OpsGroup::StringThenSse);
        // A string-only mnemonic uses the same group and simply falls through to
        // the tail when the operands are not string-shaped.
        assert_eq!(ops_group(Mnemonic::Movsb), OpsGroup::StringThenSse);
        // An SSE-only mnemonic never reaches the string check.
        assert_eq!(ops_group(Mnemonic::Addsd), OpsGroup::Sse);
    }

    /// A mnemonic beyond the memo table must still classify correctly.
    #[test]
    fn out_of_table_mnemonic_falls_back_to_direct_classification() {
        assert!(
            OPS_GROUP_TABLE_LEN > Mnemonic::Wbinvd as usize,
            "table should cover the real mnemonic range"
        );
        assert_eq!(classify_mnemonic(Mnemonic::Add), OpsGroup::Alu);
    }
}
