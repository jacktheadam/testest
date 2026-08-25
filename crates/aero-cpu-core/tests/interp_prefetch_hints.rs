use aero_cpu_core::interp::tier0::exec::{run_batch, BatchExit};
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::state::{
    CpuMode, CpuState, CR0_PE, CR0_PG, CR4_PAE, EFER_LME, SEG_ACCESS_L, SEG_ACCESS_PRESENT,
};
use aero_x86::Register;

// PREFETCHh / PREFETCHW / reserved-NOP encodings are architecturally
// non-faulting hints: the memory operand is never dereferenced and the CPU must
// not raise. They execute as NOPs.
//
// Regression witness: the Win7 x64 kernel's prefetch memcpy loop hit #UD on
// `prefetchnta [rdx+rcx*4]` (0f 18 04 0a) at 1,673,734,232 inst of the install
// boot because only `Nop`/`Pause` were handled.

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
fn prefetchnta_sib_executes_as_nop() {
    // prefetchnta [rdx+rcx*4]  (0f 18 04 0a), the Win7 witness.
    let (mut state, mut bus) = long_state(&[0x0F, 0x18, 0x04, 0x0A]);
    state.write_reg(Register::RDX, 0x100);
    state.write_reg(Register::RCX, 4);
    let r = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r.executed, 1, "exit={:?}", r.exit);
    assert!(!matches!(r.exit, BatchExit::Exception(_)), "{:?}", r.exit);
    assert_eq!(state.rip(), 4);
}

#[test]
fn prefetchnta_sib_disp8_executes_as_nop() {
    // prefetchnta [rdx+rcx*4+0x40]  (0f 18 44 0a 40).
    let (mut state, mut bus) = long_state(&[0x0F, 0x18, 0x44, 0x0A, 0x40]);
    state.write_reg(Register::RDX, 0x100);
    state.write_reg(Register::RCX, 4);
    let r = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r.executed, 1, "exit={:?}", r.exit);
    assert_eq!(state.rip(), 5);
}

#[test]
fn prefetch_family_and_reserved_nop_execute_as_nops() {
    // prefetcht0 [rax] (0f 18 08), prefetcht1 (0f 18 10), prefetcht2 (0f 18 18),
    // prefetchw [rax] (0f 0d 08), reserved-nop 0f 19 00 — each advances RIP by
    // its own length and never faults, even with a bogus memory operand.
    let code = [
        0x0F, 0x18, 0x08, 0x0F, 0x18, 0x10, 0x0F, 0x18, 0x18, 0x0F, 0x0D, 0x08, 0x0F, 0x19, 0x00,
    ];
    let (mut state, mut bus) = long_state(&code);
    state.write_reg(Register::RAX, 0xFFFF_FFFF_FFFF_F000);
    let r = run_batch(&mut state, &mut bus, 5);
    assert_eq!(r.executed, 5, "exit={:?}", r.exit);
    assert!(!matches!(r.exit, BatchExit::Exception(_)), "{:?}", r.exit);
    assert_eq!(state.rip(), 15); // five 3-byte hints
}

#[test]
fn prefetch_does_not_dereference_memory() {
    // A garbage address must not fault: prefetch never reads its operand.
    let (mut state, mut bus) = long_state(&[0x0F, 0x18, 0x04, 0x0A]);
    state.write_reg(Register::RDX, 0xFFFF_FFFF_FFFF_FF00);
    state.write_reg(Register::RCX, 0x1000);
    let r = run_batch(&mut state, &mut bus, 1);
    assert!(!matches!(r.exit, BatchExit::Exception(_)), "{:?}", r.exit);
    assert_eq!(state.rip(), 4);
}
