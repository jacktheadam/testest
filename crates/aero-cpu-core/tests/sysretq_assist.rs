//! SYSRETQ (REX.W form `48 0F 07`) must be handled like SYSRET.
//! iced-x86 maps it to `Mnemonic::Sysretq` (not `Sysret`). Win7 x64 only emits SYSRETQ.

use aero_cpu_core::assist::{handle_assist_decoded, AssistContext};
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::state::{gpr, CpuMode, CpuState, RFLAGS_IF};
use aero_cpu_core::time::TimeSource;
use aero_x86::decode;

#[test]
fn sysretq_returns_to_user_with_rcx_r11() {
    let mut state = CpuState::new(CpuMode::Long);
    state.segments.cs.selector = 0x10; // CPL0
    state.segments.ss.selector = 0x18;
    state.msr.efer |= 1; // SCE
                         // Windows-like STAR: kernel CS=0x10, user base=0x23 → user CS=0x33 SS=0x2B
    state.msr.star = 0x0023_0010_0000_0000;
    state.write_gpr64(gpr::RCX, 0x0000_0000_0040_1000);
    state.write_gpr64(gpr::R11, 0x202); // IF set
    state.set_rip(0xffff_f800_0000_1000);
    state.write_gpr64(gpr::RSP, 0xffff_f800_0000_2000);

    let bytes = [0x48u8, 0x0f, 0x07]; // SYSRETQ
    let decoded = decode(&bytes, 0xffff_f800_0000_1000, 64).expect("decode sysretq");
    assert!(
        format!("{:?}", decoded.instr.mnemonic())
            .to_lowercase()
            .contains("sysret"),
        "expected Sysret/Sysretq mnemonic, got {:?}",
        decoded.instr.mnemonic()
    );

    let mut bus = FlatTestBus::new(0x1000);
    let mut time = TimeSource::default();
    let mut ctx = AssistContext::default();
    handle_assist_decoded(&mut ctx, &mut time, &mut state, &mut bus, &decoded, false)
        .expect("sysretq assist");

    assert_eq!(state.rip(), 0x401000);
    assert_eq!(
        state.segments.cs.selector & 0x3,
        3,
        "CPL must be 3 after SYSRETQ"
    );
    assert_eq!(state.segments.cs.selector & !0x3, 0x30);
    assert_eq!(state.segments.ss.selector & !0x3, 0x28);
    assert_ne!(state.rflags() & RFLAGS_IF, 0);
    // SDM: fixed flat user CS cache — L=1 (64-bit), DPL=3, present.
    assert!(
        state.segments.cs.is_long(),
        "SYSRETQ must load CS.L=1 (64-bit user code)"
    );
    assert_eq!(state.segments.cs.dpl(), 3);
    assert!(state.segments.cs.is_present());
    // RFLAGS from R11 with SDM mask: RF/VM cleared, reserved bit1 set.
    assert_eq!(state.rflags() & 0x2, 0x2);
}
