use aero_cpu_core::interp::tier0::exec::{run_batch, BatchExit};
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{CpuMode, CpuState};
use aero_x86::Register;

#[test]
fn indirect_far_jump_reads_ptr16_16_from_memory() {
    // jmp far [0x0100]  (FF 2E 00 01) — IP = m16 at DS:0x0100, CS = m16 at DS:0x0102.
    let code = [0xFF, 0x2E, 0x00, 0x01];
    let mut bus = FlatTestBus::new(0x40000);
    bus.load(0x20000, &code);
    bus.write_bytes(0x20100, &[0x00, 0x40, 0x00, 0xF0]).unwrap();
    let mut state = CpuState::new(CpuMode::Bit16);
    state.write_reg(Register::CS, 0x2000);
    state.set_rip(0);
    state.write_reg(Register::DS, 0x2000);
    state.write_reg(Register::SS, 0);
    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1);
    assert_eq!(state.read_reg(Register::CS), 0xF000);
    assert_eq!(state.rip(), 0x4000);
}

#[test]
fn far_call_and_retf_roundtrip_in_real_mode() {
    // Layout (CS=0x1000 base 0x10000):
    //   0x10000: ff 1e 00 01   call far [0x0100]
    //   0x10004: f4            hlt (return address after RETF)
    // Target (CS=0xF000 base 0xF0000):
    //   0xF4000: cb            retf
    let mut bus = FlatTestBus::new(0x100000);
    bus.load(0x10000, &[0xFF, 0x1E, 0x00, 0x01, 0xF4]);
    bus.load(0xF4000, &[0xCB]);
    // Far pointer at DS:0x0100 → F000:4000.
    bus.write_bytes(0x10100, &[0x00, 0x40, 0x00, 0xF0]).unwrap();

    let mut state = CpuState::new(CpuMode::Bit16);
    state.write_reg(Register::CS, 0x1000);
    state.set_rip(0);
    state.write_reg(Register::DS, 0x1000);
    state.write_reg(Register::SS, 0);
    state.write_reg(Register::SP, 0x0800);

    // CALL FAR: pushes CS then next-IP, jumps to F000:4000.
    let r1 = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r1.executed, 1);
    assert_eq!(state.read_reg(Register::CS), 0xF000);
    assert_eq!(state.rip(), 0x4000);
    assert_eq!(state.read_reg(Register::SP), 0x0800 - 4);
    // Stack (low→high): ip=0x0004, cs=0x1000 (CS pushed first so IP is on top).
    let mut tmp = [0u8; 2];
    bus.read_bytes(0x07FC, &mut tmp).unwrap();
    assert_eq!(u16::from_le_bytes(tmp), 0x0004, "return IP at stack top");
    bus.read_bytes(0x07FE, &mut tmp).unwrap();
    assert_eq!(u16::from_le_bytes(tmp), 0x1000, "return CS below return IP");

    // RETF: pops IP then CS, returning to 1000:0004 (the HLT).
    let r2 = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r2.executed, 1);
    assert_eq!(state.read_reg(Register::CS), 0x1000);
    assert_eq!(state.rip(), 0x0004);
    assert_eq!(state.read_reg(Register::SP), 0x0800);

    let r3 = run_batch(&mut state, &mut bus, 1);
    assert!(matches!(r3.exit, BatchExit::Halted));
}
