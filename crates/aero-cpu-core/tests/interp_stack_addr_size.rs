use aero_cpu_core::interp::tier0::exec::run_batch;
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{CpuMode, CpuState, CR0_PE, SEG_ACCESS_DB, SEG_ACCESS_PRESENT};
use aero_x86::Register;

// A 16-bit protected-mode code segment running with a 32-bit stack segment
// (SS.B = 1) must use ESP for stack operations. The Intel SDM takes the stack
// address size from the SS descriptor's B flag, *not* the CS D flag. This is the
// exact shape of the Win7 boot stub as it transitions to 32-bit protected mode;
// taking the width from CS.D masked the stack pointer to 16 bits and sent pushes
// to 0x1ff8 instead of 0x61ff8 (root cause of a long bring-up divergence).

fn state_16cs_32ss() -> CpuState {
    let mut state = CpuState::new(CpuMode::Protected);
    state.control.cr0 = CR0_PE;
    state.update_mode();
    // CS: 16-bit code segment (D = 0).
    state.segments.cs.selector = 0x50;
    state.segments.cs.base = 0;
    state.segments.cs.limit = 0xFFFF;
    state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT; // D=0
                                                                // SS: 32-bit data segment (B = 1).
    state.segments.ss.selector = 0x30;
    state.segments.ss.base = 0;
    state.segments.ss.limit = 0xFFFFF;
    state.segments.ss.access = 0x2 | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_DB; // B=1
    state
}

#[test]
fn stack_addr_size_comes_from_ss_b_bit_not_cs_d_bit() {
    let state = state_16cs_32ss();
    assert_eq!(state.bitness(), 16, "CS is a 16-bit code segment");
    assert_eq!(
        state.stack_ptr_bits(),
        32,
        "stack width must follow SS.B (32), not CS.D (16)"
    );
}

#[test]
fn push_uses_esp_when_cs_is_16bit_but_ss_is_32bit() {
    // mov esp, 0x00061ffc ; push edx  (both 0x66-prefixed => 32-bit operands)
    let code = [
        0x66, 0xBC, 0xFC, 0x1F, 0x06, 0x00, // mov esp, 0x00061ffc
        0x66, 0x52, // push edx
    ];
    let mut bus = FlatTestBus::new(0x70000);
    bus.load(0, &code);
    let mut state = state_16cs_32ss();
    state.set_rip(0);
    state.write_reg(Register::RDX, 0x25398);

    let r1 = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r1.executed, 1);
    assert_eq!(state.read_reg(Register::RSP) & 0xFFFF_FFFF, 0x61ffc);

    let r2 = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r2.executed, 1);
    // The push must decrement ESP by 4 and write to SS:0x61ff8, not SP:0x1ff8.
    assert_eq!(state.read_reg(Register::RSP) & 0xFFFF_FFFF, 0x61ff8);
    let mut tmp = [0u8; 4];
    bus.read_bytes(0x61ff8, &mut tmp).unwrap();
    assert_eq!(
        u32::from_le_bytes(tmp),
        0x25398,
        "push must write to 0x61ff8"
    );
    // And it must NOT have written to the 16-bit-masked 0x1ff8.
    let mut lo = [0u8; 4];
    bus.read_bytes(0x1ff8, &mut lo).unwrap();
    assert_ne!(u32::from_le_bytes(lo), 0x25398, "must not write to 0x1ff8");
}

#[test]
fn pop_reads_back_from_esp_not_sp() {
    // Seed a 32-bit stack value, then pop it in 16-bit PM with a 32-bit SS.
    let code = [0x66, 0x5A]; // pop edx (0x66 => 32-bit)
    let mut bus = FlatTestBus::new(0x70000);
    bus.load(0, &code);
    bus.load(0x61ff0, &0x00DEADBEu32.to_le_bytes());
    let mut state = state_16cs_32ss();
    state.set_rip(0);
    state.write_reg(Register::RSP, 0x61ff0);

    let r = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r.executed, 1);
    assert_eq!(state.read_reg(Register::RDX) & 0xFFFF_FFFF, 0xDEADBE);
    assert_eq!(state.read_reg(Register::RSP) & 0xFFFF_FFFF, 0x61ff4);
}
