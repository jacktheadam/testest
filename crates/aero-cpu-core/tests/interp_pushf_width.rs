use aero_cpu_core::interp::tier0::exec::run_batch;
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{CpuMode, CpuState, RFLAGS_IF};
use aero_x86::Register;

#[test]
fn pushfd_popfd_32bit_width_in_16bit_mode() {
    // 66 9C = PUSHFD (push EFLAGS32) in 16-bit mode; 66 9D = POPFD.
    let code = [
        0x66, 0x9C, // pushfd
        0x66, 0x9D, // popfd
        0xF4, // hlt
    ];
    let mut bus = FlatTestBus::new(0x2000);
    bus.load(0, &code);
    let mut state = CpuState::new(CpuMode::Bit16);
    state.set_rip(0);
    state.write_reg(Register::SS, 0);
    state.write_reg(Register::SP, 0x1000);
    state.set_rflags(0x0202u64 | RFLAGS_IF);

    let r1 = run_batch(&mut state, &mut bus, 1);
    eprintln!(
        "exit={:?} executed={} rip={:#x}",
        r1.exit,
        r1.executed,
        state.rip()
    );
    assert_eq!(r1.executed, 1);
    // PUSHFD decrements SP by 4, not 2.
    assert_eq!(state.read_reg(Register::SP), 0x1000 - 4);
    let mut tmp = [0u8; 4];
    bus.read_bytes(0x1000 - 4, &mut tmp).unwrap();
    assert_eq!(u64::from(u32::from_le_bytes(tmp)), 0x0202u64 | RFLAGS_IF);

    let r2 = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r2.executed, 1);
    assert_eq!(state.read_reg(Register::SP), 0x1000);
    assert_eq!(
        state.rflags() & (0x0202u64 | RFLAGS_IF),
        0x0202u64 | RFLAGS_IF
    );
}

#[test]
fn pushf_popf_16bit_width_in_16bit_mode() {
    // 9C = PUSHF (16-bit) in 16-bit mode; 9D = POPF.
    let code = [0x9C, 0x9D, 0xF4];
    let mut bus = FlatTestBus::new(0x2000);
    bus.load(0, &code);
    let mut state = CpuState::new(CpuMode::Bit16);
    state.set_rip(0);
    state.write_reg(Register::SS, 0);
    state.write_reg(Register::SP, 0x1000);
    state.set_rflags(0x0202u64 | RFLAGS_IF);

    let r1 = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r1.executed, 1);
    assert_eq!(state.read_reg(Register::SP), 0x1000 - 2);
    let mut tmp = [0u8; 2];
    bus.read_bytes(0x1000 - 2, &mut tmp).unwrap();
    assert_eq!(u16::from_le_bytes(tmp), (0x0202u64 | RFLAGS_IF) as u16);

    let r2 = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r2.executed, 1);
    assert_eq!(state.read_reg(Register::SP), 0x1000);
}
