use aero_cpu_core::interp::tier0::exec::{run_batch, BatchExit};
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{
    CpuMode, CpuState, CR0_PE, CR0_PG, CR4_PAE, EFER_LME, SEG_ACCESS_DB, SEG_ACCESS_L,
    SEG_ACCESS_PRESENT,
};
use aero_x86::Register;

// A near indirect CALL must read its target operand BEFORE pushing the return
// address. The previous order (push, then read) computed rsp-relative effective
// addresses against the already-decremented stack pointer, so `call [rsp+disp]`
// fetched the target one slot low (8 bytes in 64-bit, 4 in 32-bit).
//
// Regression witness: the Win7 x64 boot loader dispatches an import thunk with
// `call qword ptr [rsp+0x80]` (ff 94 24 80 00 00 00); it read `[rsp+0x78]` — a
// spilled argument pointer — and called into a data page.

fn long_state(bus_code: u64) -> (CpuState, FlatTestBus) {
    let bus = FlatTestBus::new(0x4000);
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
    state.set_rip(bus_code);
    (state, bus)
}

fn flat_protected_state(bus_code: u64) -> (CpuState, FlatTestBus) {
    let bus = FlatTestBus::new(0x4000);
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
    (state, bus)
}

#[test]
fn call_rm64_rsp_disp32_reads_target_before_push() {
    // call qword ptr [rsp+0x80]  (ff 94 24 80 00 00 00), the Win7 witness.
    let code = [0xFF, 0x94, 0x24, 0x80, 0x00, 0x00, 0x00];
    let (mut state, mut bus) = long_state(0x400);
    bus.load(0x400, &code);
    state.write_reg(Register::RSP, 0x1000);
    // Real target at [rsp+0x80]; decoy one slot low at [rsp+0x78].
    bus.load(0x1080, &0x2a00u64.to_le_bytes());
    bus.load(0x1078, &0xdeadbeefu64.to_le_bytes());

    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1, "exit={:?}", res.exit);
    assert!(
        !matches!(res.exit, BatchExit::Exception(_)),
        "{:?}",
        res.exit
    );
    assert_eq!(state.rip(), 0x2a00);
    assert_eq!(state.read_reg(Register::RSP), 0x1000 - 8);
    let mut tmp = [0u8; 8];
    bus.read_bytes(0x1000 - 8, &mut tmp).unwrap();
    assert_eq!(u64::from_le_bytes(tmp), 0x407); // next RIP
}

#[test]
fn call_rm64_rsp_sib_index_reads_target_before_push() {
    // call qword ptr [rsp+rax*8]  (ff 14 c4), rax = 2 → EA = rsp+16.
    let code = [0xFF, 0x14, 0xC4];
    let (mut state, mut bus) = long_state(0x400);
    bus.load(0x400, &code);
    state.write_reg(Register::RSP, 0x1000);
    state.write_reg(Register::RAX, 2);
    bus.load(0x1010, &0x2b00u64.to_le_bytes());
    bus.load(0x1008, &0xdeadbeefu64.to_le_bytes());

    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1, "exit={:?}", res.exit);
    assert!(
        !matches!(res.exit, BatchExit::Exception(_)),
        "{:?}",
        res.exit
    );
    assert_eq!(state.rip(), 0x2b00);
    assert_eq!(state.read_reg(Register::RSP), 0x1000 - 8);
}

#[test]
fn call_rm32_esp_disp8_reads_target_before_push() {
    // call dword ptr [esp+0x10]  (ff 54 24 10).
    let code = [0xFF, 0x54, 0x24, 0x10];
    let (mut state, mut bus) = flat_protected_state(0x400);
    bus.load(0x400, &code);
    state.write_reg(Register::RSP, 0x1000);
    bus.load(0x1010, &0x2c00u32.to_le_bytes());
    bus.load(0x100c, &0xdeadbeefu32.to_le_bytes());

    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1, "exit={:?}", res.exit);
    assert!(
        !matches!(res.exit, BatchExit::Exception(_)),
        "{:?}",
        res.exit
    );
    assert_eq!(state.rip(), 0x2c00);
    assert_eq!(state.read_reg(Register::RSP), 0x1000 - 4);
    let mut tmp = [0u8; 4];
    bus.read_bytes(0x1000 - 4, &mut tmp).unwrap();
    assert_eq!(u32::from_le_bytes(tmp), 0x404); // next EIP
}
