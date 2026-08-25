use aero_cpu_core::interp::tier0::exec::{run_batch, BatchExit};
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{CpuMode, CpuState};
use aero_x86::Register;

fn run_to_halt(state: &mut CpuState, bus: &mut FlatTestBus, max: u64) {
    let mut steps = 0;
    while steps < max {
        let res = run_batch(state, bus, 1024);
        steps += res.executed;
        match res.exit {
            BatchExit::Completed | BatchExit::Branch => continue,
            BatchExit::Halted => return,
            BatchExit::BiosInterrupt(vector) => panic!("unexpected BIOS interrupt: {vector:#x}"),
            BatchExit::Assist(r) => panic!("unexpected assist: {r:?}"),
            BatchExit::Exception(e) => panic!("unexpected exception: {e:?}"),
            BatchExit::CpuExit(e) => panic!("unexpected cpu exit: {e:?}"),
        }
    }
    panic!("program did not halt");
}

#[test]
fn enter_leave_16bit_level0_roundtrip() {
    // enter 0x20,0 ; hlt  — then a second program does `leave`.
    //
    // 16-bit ENTER imm16,0:
    //   push bp            (sp: 0x800 -> 0x7FE, [ss:0x7FE] = old bp)
    //   bp := sp           (bp = 0x7FE)
    //   sp -= 0x20         (sp = 0x7DE)
    let code = [
        0xC8, 0x20, 0x00, 0x00, // enter 0x20, 0
        0xC9, // leave
        0xF4, // hlt
    ];
    let mut bus = FlatTestBus::new(0x1000);
    bus.load(0, &code);
    let mut state = CpuState::new(CpuMode::Bit16);
    state.set_rip(0);
    state.write_reg(Register::SS, 0);
    state.write_reg(Register::SP, 0x800);
    state.write_reg(Register::BP, 0x1234);

    // Run just ENTER (3 instructions total; stop after 1).
    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1);
    assert_eq!(state.read_reg(Register::SP), 0x7DE);
    assert_eq!(state.read_reg(Register::BP), 0x7FE);
    // Saved frame pointer was pushed at ss:0x7FE.
    let mut tmp = [0u8; 2];
    bus.read_bytes(0x7FE, &mut tmp).unwrap();
    assert_eq!(u16::from_le_bytes(tmp), 0x1234);

    run_to_halt(&mut state, &mut bus, 10);
    // LEAVE: sp := bp (0x7FE); pop bp (bp = 0x1234, sp = 0x800).
    assert_eq!(state.read_reg(Register::BP), 0x1234);
    assert_eq!(state.read_reg(Register::SP), 0x800);
}

#[test]
fn enter_16bit_nesting_level2_pushes_frame_chain() {
    // 16-bit ENTER imm16,2 with an outer frame chain:
    //   outer bp at ss:0x0100 = 0xAAAA, and ss:0x00FE holds the "saved outer bp" 0xBBBB.
    // Steps:
    //   push bp            sp: 0x900 -> 0x8FE, [ss:0x8FE] = 0x0100
    //   for i in 1..2:     push [ss:0x00FE] = 0xBBBB  (sp: 0x8FE -> 0x8FC)
    //   push frame_temp    sp: 0x8FC -> 0x8FA, [ss:0x8FA] = 0x08FE
    //   bp := 0x8FE
    //   sp := 0x8FA - 0x10 = 0x8EA
    let code = [
        0xC8, 0x10, 0x00, 0x02, // enter 0x10, 2
        0xF4, // hlt
    ];
    let mut bus = FlatTestBus::new(0x1000);
    bus.load(0, &code);
    // Outer frame: bp chain lives at ss:0x00FE.
    bus.write_bytes(0x00FE, &0xBBBBu16.to_le_bytes()).unwrap();
    let mut state = CpuState::new(CpuMode::Bit16);
    state.set_rip(0);
    state.write_reg(Register::SS, 0);
    state.write_reg(Register::SP, 0x900);
    state.write_reg(Register::BP, 0x0100);

    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1);
    assert_eq!(state.read_reg(Register::SP), 0x8EA);
    assert_eq!(state.read_reg(Register::BP), 0x8FE);

    let mut tmp = [0u8; 2];
    bus.read_bytes(0x8FE, &mut tmp).unwrap();
    assert_eq!(u16::from_le_bytes(tmp), 0x0100, "old bp at frame bottom");
    bus.read_bytes(0x8FC, &mut tmp).unwrap();
    assert_eq!(
        u16::from_le_bytes(tmp),
        0xBBBB,
        "outer saved bp pushed from chain"
    );
    bus.read_bytes(0x8FA, &mut tmp).unwrap();
    assert_eq!(u16::from_le_bytes(tmp), 0x08FE, "frame_temp pushed last");

    run_to_halt(&mut state, &mut bus, 10);
}

#[test]
fn enter_leave_32bit_level0_roundtrip() {
    // 32-bit ENTER 0x40,0:
    //   push ebp   (esp: 0x1000 -> 0xFFC, [ss:0xFFC] = old ebp)
    //   ebp := esp (0xFFC)
    //   esp -= 0x40 (0xFBC)
    let code = [
        0xC8, 0x40, 0x00, 0x00, // enter 0x40, 0
        0xC9, // leave
        0xF4, // hlt
    ];
    let mut bus = FlatTestBus::new(0x2000);
    bus.load(0, &code);
    let mut state = CpuState::new(CpuMode::Bit32);
    state.set_rip(0);
    state.write_reg(Register::SS, 0);
    state.write_reg(Register::ESP, 0x1000);
    state.write_reg(Register::EBP, 0x00ABCDEF);

    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1);
    assert_eq!(state.read_reg(Register::ESP), 0xFBC);
    assert_eq!(state.read_reg(Register::EBP), 0xFFC);
    let mut tmp = [0u8; 4];
    bus.read_bytes(0xFFC, &mut tmp).unwrap();
    assert_eq!(u32::from_le_bytes(tmp), 0x00ABCDEF);

    run_to_halt(&mut state, &mut bus, 10);
    assert_eq!(state.read_reg(Register::EBP), 0x00ABCDEF);
    assert_eq!(state.read_reg(Register::ESP), 0x1000);
}
