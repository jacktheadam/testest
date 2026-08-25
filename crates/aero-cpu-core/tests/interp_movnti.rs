use aero_cpu_core::interp::tier0::exec::{run_batch, BatchExit};
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{
    CpuMode, CpuState, CR0_PE, CR0_PG, CR4_PAE, EFER_LME, SEG_ACCESS_DB, SEG_ACCESS_L,
    SEG_ACCESS_PRESENT,
};
use aero_x86::Register;

// MOVNTI (0F C3 /r) is a non-temporal store: architecturally a plain 32/64-bit
// store with no alignment or ordering requirements; the NT hint is not
// architectural state.
//
// Regression witness: the Win7 x64 kernel's non-temporal memcpy loop stored
// with `movnti [r9-8], r15` (4c 0f c3 49 f8) at 1,673,734,395 inst of the
// install boot and hit #UD.

fn long_state(code: &[u8]) -> (CpuState, FlatTestBus) {
    let mut bus = FlatTestBus::new(0x2000);
    bus.load(0x400, code);
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
    state.set_rip(0x400);
    (state, bus)
}

#[test]
fn movnti_rm64_stores_qword() {
    // movnti [rcx-8], r9  (4c 0f c3 49 f8), the Win7 witness.
    let (mut state, mut bus) = long_state(&[0x4C, 0x0F, 0xC3, 0x49, 0xF8]);
    state.write_reg(Register::RCX, 0x1008);
    state.write_reg(Register::R9, 0x1122_3344_5566_7788);
    let r = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r.executed, 1, "exit={:?}", r.exit);
    assert!(!matches!(r.exit, BatchExit::Exception(_)), "{:?}", r.exit);
    let mut tmp = [0u8; 8];
    bus.read_bytes(0x1000, &mut tmp).unwrap();
    assert_eq!(u64::from_le_bytes(tmp), 0x1122_3344_5566_7788);
    assert_eq!(state.rip(), 0x405);
}

#[test]
fn movnti_m32_stores_dword_unaligned() {
    // movnti [eax], ecx  (0f c3 08) in 32-bit protected mode, unaligned dest.
    let bus_code = 0x400u64;
    let mut bus = FlatTestBus::new(0x2000);
    bus.load(bus_code, &[0x0F, 0xC3, 0x08]);
    let mut state = CpuState::new(CpuMode::Protected);
    state.control.cr0 = CR0_PE;
    state.update_mode();
    state.segments.cs.selector = 0x20;
    state.segments.cs.base = 0;
    state.segments.cs.limit = 0xFFFFF;
    state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_DB;
    state.segments.ds.selector = 0x30;
    state.segments.ds.base = 0;
    state.segments.ds.limit = 0xFFFFF;
    state.segments.ds.access = 0x2 | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_DB;
    state.segments.ss = state.segments.ds;
    state.set_rip(bus_code);
    state.write_reg(Register::RAX, 0x1003);
    state.write_reg(Register::RCX, 0xAABB_CCDD);
    let r = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r.executed, 1, "exit={:?}", r.exit);
    assert!(!matches!(r.exit, BatchExit::Exception(_)), "{:?}", r.exit);
    let mut tmp = [0u8; 4];
    bus.read_bytes(0x1003, &mut tmp).unwrap();
    assert_eq!(u32::from_le_bytes(tmp), 0xAABB_CCDD);
    assert_eq!(state.rip(), 0x403);
}
