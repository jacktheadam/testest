//! Regression: Win7 PAGELK executes `wbinvd` while programming MTRRs, immediately
//! before converting KiServiceTable. Missing WBINVD was #UD at
//! `0xfffff8000c2dd8f5` (~905k inst past the early-kernel MSR snap), blocking
//! SSDT init.

use aero_cpu_core::assist::{handle_assist, AssistContext};
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::state::{CpuMode, CpuState, CR0_PE, SEG_ACCESS_L, SEG_ACCESS_PRESENT};
use aero_cpu_core::time::TimeSource;
use aero_cpu_core::AssistReason;

const CODE: u64 = 0x1000;

#[test]
fn wbinvd_is_cpl0_nop() {
    let mut ctx = AssistContext::default();
    let mut state = CpuState::new(CpuMode::Bit64);
    state.control.cr0 |= CR0_PE;
    state.segments.cs.selector = 0x08; // CPL0
    state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_L;
    state.set_rip(CODE);
    let mut bus = FlatTestBus::new(0x2000);
    bus.load(CODE, &[0x0F, 0x09]); // WBINVD
    let mut time = TimeSource::default();
    handle_assist(
        &mut ctx,
        &mut time,
        &mut state,
        &mut bus,
        AssistReason::Privileged,
    )
    .expect("WBINVD at CPL0 must succeed");
    assert_eq!(state.rip(), CODE + 2);
}

#[test]
fn wbinvd_at_cpl3_raises_gp() {
    let mut ctx = AssistContext::default();
    let mut state = CpuState::new(CpuMode::Bit64);
    state.control.cr0 |= CR0_PE;
    state.segments.cs.selector = 0x1B; // RPL3
    state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_L;
    state.set_rip(CODE);
    let mut bus = FlatTestBus::new(0x2000);
    bus.load(CODE, &[0x0F, 0x09]);
    let mut time = TimeSource::default();
    let err = handle_assist(
        &mut ctx,
        &mut time,
        &mut state,
        &mut bus,
        AssistReason::Privileged,
    );
    assert!(err.is_err(), "WBINVD at CPL3 must #GP");
}
