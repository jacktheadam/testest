use aero_cpu_core::interp::tier0::exec::{run_batch, BatchExit};
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{CpuMode, CpuState, CR0_PE, SEG_ACCESS_DB, SEG_ACCESS_PRESENT};
use aero_x86::Register;

// iced-x86 reports the memory operand of a near-indirect branch (`FF /4` jmp,
// `FF /2` call) with a `MemorySize::*Offset` size (e.g. `DwordOffset`), not the
// `UInt32` a data dword would use. The operand-width helper must accept the
// `*Offset` family; otherwise every jump/call table dispatch raises #UD.
//
// Regression test: the Win7 boot loader switches on a value with
// `jmp dword ptr [eax*4 + table]` and hit exactly this.

fn flat_protected_state(bus_code: u64) -> (CpuState, FlatTestBus) {
    let bus = FlatTestBus::new(0x500000);
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
fn jmp_rm32_jump_table_dword_offset() {
    // jmp dword ptr [eax*4 + 0x447663]  (ff 24 85 63 76 44 00), eax = 1.
    let code = [0xFF, 0x24, 0x85, 0x63, 0x76, 0x44, 0x00];
    let (mut state, mut bus) = flat_protected_state(0x446e1f);
    bus.load(0x446e1f, &code);
    // Table at 0x447663; entry[1] = 0x4099aa.
    bus.load(0x447663 + 4, &0x4099aau32.to_le_bytes());
    state.write_reg(Register::RAX, 1);

    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1, "exit={:?}", res.exit);
    assert!(
        !matches!(res.exit, BatchExit::Exception(_)),
        "{:?}",
        res.exit
    );
    assert_eq!(state.rip(), 0x4099aa);
}

#[test]
fn call_rm32_indirect_dword_offset_pushes_return() {
    // call dword ptr [0x447700]  (ff 15 00 77 44 00) — direct (no index), dword offset.
    let code = [0xFF, 0x15, 0x00, 0x77, 0x44, 0x00];
    let (mut state, mut bus) = flat_protected_state(0x400000);
    bus.load(0x400000, &code);
    bus.load(0x447700, &0x40555cu32.to_le_bytes());
    state.write_reg(Register::RSP, 0x8000);

    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1, "exit={:?}", res.exit);
    assert!(
        !matches!(res.exit, BatchExit::Exception(_)),
        "{:?}",
        res.exit
    );
    assert_eq!(state.rip(), 0x40555c);
    // Near call in 32-bit mode pushes the 4-byte return address (next IP).
    assert_eq!(state.read_reg(Register::RSP), 0x8000 - 4);
    let mut tmp = [0u8; 4];
    bus.read_bytes(0x8000 - 4, &mut tmp).unwrap();
    assert_eq!(u32::from_le_bytes(tmp), 0x400006);
}
