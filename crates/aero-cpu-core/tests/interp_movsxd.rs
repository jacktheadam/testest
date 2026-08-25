use aero_cpu_core::interp::tier0::exec::run_batch;
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::state::{
    CpuMode, CpuState, CR0_PE, CR0_PG, CR4_PAE, EFER_LME, SEG_ACCESS_L, SEG_ACCESS_PRESENT,
};
use aero_x86::Register;

// `48 63 f0` = movsxd rsi, eax (opcode 0x63 in 64-bit mode). Previously
// unhandled (only Movsx/Movzx were), so the Win7 x64 kernel hit #UD on it.
// Covers the sign-extension both ways.

fn long_state(code: &[u8]) -> (CpuState, FlatTestBus) {
    let mut bus = FlatTestBus::new(0x2000);
    bus.load(0, code);
    let mut state = CpuState::new(CpuMode::Long);
    state.control.cr0 = CR0_PE | CR0_PG;
    state.control.cr4 = CR4_PAE;
    state.msr.efer = EFER_LME;
    state.segments.cs.selector = 0x10;
    state.segments.cs.base = 0;
    state.segments.cs.limit = 0xFFFFF;
    state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_L;
    state.update_mode();
    assert_eq!(state.mode, CpuMode::Long);
    state.set_rip(0);
    (state, bus)
}

#[test]
fn movsxd_sign_extends_negative_dword() {
    let (mut state, mut bus) = long_state(&[0x48, 0x63, 0xF0]); // movsxd rsi, eax
    state.write_reg(Register::RAX, 0x8000_0000);
    let r = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r.executed, 1, "exit={:?}", r.exit);
    assert_eq!(state.read_reg(Register::RSI), 0xFFFF_FFFF_8000_0000);
}

#[test]
fn movsxd_zero_extends_positive_dword() {
    let (mut state, mut bus) = long_state(&[0x48, 0x63, 0xF0]); // movsxd rsi, eax
    state.write_reg(Register::RAX, 0x7FFF_FFFF);
    let r = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r.executed, 1, "exit={:?}", r.exit);
    assert_eq!(state.read_reg(Register::RSI), 0x0000_0000_7FFF_FFFF);
}

#[test]
fn movsxd_from_memory_source() {
    // movsxd rcx, dword [0x100]  (48 63 0c 25 00 01 00 00)
    let (mut state, mut bus) = long_state(&[0x48, 0x63, 0x0C, 0x25, 0x00, 0x01, 0x00, 0x00]);
    bus.load(0x100, &0xFFFF_FFFEu32.to_le_bytes());
    let r = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r.executed, 1, "exit={:?}", r.exit);
    assert_eq!(state.read_reg(Register::RCX), 0xFFFF_FFFF_FFFF_FFFE);
}
