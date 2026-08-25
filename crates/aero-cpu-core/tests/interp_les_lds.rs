use aero_cpu_core::interp::tier0::exec::run_batch;
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{CpuMode, CpuState};
use aero_x86::Register;

#[test]
fn les_loads_far_pointer_in_real_mode() {
    // les ax, [0x0100]  (C4 06 00 01) — AX = m16 at DS:0x0100, ES = m16 at DS:0x0102.
    let code = [0xC4, 0x06, 0x00, 0x01, 0xF4];
    let mut bus = FlatTestBus::new(0x40000);
    bus.load(0, &code);
    // DS base = 0x2000 << 4 = 0x20000; far pointer stored at 0x20100.
    bus.write_bytes(0x20100, &[0x34, 0x12, 0x00, 0x30]).unwrap();
    let mut state = CpuState::new(CpuMode::Bit16);
    state.set_rip(0);
    state.write_reg(Register::DS, 0x2000);
    state.write_reg(Register::SS, 0);
    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1);
    assert_eq!(state.read_reg(Register::AX), 0x1234);
    assert_eq!(state.read_reg(Register::ES), 0x3000);
    assert_eq!(state.seg_base_reg(Register::ES), 0x30000);
}

#[test]
fn lds_loads_far_pointer_in_real_mode() {
    // lds ax, [0x0100]  (C5 06 00 01) — AX = m16 at DS:0x0100, DS = m16 at DS:0x0102.
    let code = [0xC5, 0x06, 0x00, 0x01, 0xF4];
    let mut bus = FlatTestBus::new(0x60000);
    bus.load(0, &code);
    bus.write_bytes(0x20100, &[0x78, 0x56, 0x00, 0x50]).unwrap();
    let mut state = CpuState::new(CpuMode::Bit16);
    state.set_rip(0);
    state.write_reg(Register::DS, 0x2000);
    state.write_reg(Register::SS, 0);
    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1);
    assert_eq!(state.read_reg(Register::AX), 0x5678);
    assert_eq!(state.read_reg(Register::DS), 0x5000);
    assert_eq!(state.seg_base_reg(Register::DS), 0x50000);
}
