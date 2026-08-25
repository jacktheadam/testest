//! LAR / LSL assist — Win7 ntdll uses `LSL r32, r32` in user mode; missing
//! opcode was #UD → STATUS_ILLEGAL_INSTRUCTION → Session Manager BSOD
//! (`0xC000021A` / "Unhandled Exception in Session Manager").

use aero_cpu_core::assist::AssistContext;
use aero_cpu_core::interp::tier0::exec::{run_batch_cpu_core_with_assists, BatchExit};
use aero_cpu_core::interp::tier0::Tier0Config;
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::state::{CpuMode, RFLAGS_ZF};
use aero_cpu_core::{CpuBus, CpuCore};
use aero_x86::Register;

const CODE_BASE: u64 = 0x2000;
const GDT_BASE: u64 = 0x1000;

/// Protected 32-bit with a 3-entry GDT (null + code + data).
fn setup_cpu() -> (CpuCore, FlatTestBus) {
    let mut bus = FlatTestBus::new(0x10000);
    let mut cpu = CpuCore::new(CpuMode::Bit32);
    cpu.state.control.cr0 |= 1; // PE
    cpu.state.tables.gdtr.base = GDT_BASE;
    cpu.state.tables.gdtr.limit = 23;

    bus.write_u64(GDT_BASE, 0).unwrap(); // null
                                         // 0x08: code, P, DPL0, S=1 type=A, G=1, limit=FFFFF → effective FFFFFFFF
    bus.write_u64(GDT_BASE + 8, 0x00CF_9A00_0000_FFFF).unwrap();
    // 0x10: data, P, DPL0, S=1 type=2, G=0, limit=0x0FFF
    bus.write_u64(GDT_BASE + 16, 0x0040_9200_0000_0FFF).unwrap();

    cpu.state.segments.cs.selector = 0x08;
    cpu.state.segments.cs.base = 0;
    cpu.state.segments.cs.limit = 0xFFFF_FFFF;
    cpu.state.segments.ds.selector = 0x10;
    cpu.state.segments.ss.selector = 0x10;
    (cpu, bus)
}

fn run_until_hlt(cpu: &mut CpuCore, bus: &mut FlatTestBus) {
    let mut ctx = AssistContext::default();
    let cfg = Tier0Config::default();
    for _ in 0..32 {
        let res = run_batch_cpu_core_with_assists(&cfg, &mut ctx, cpu, bus, 16);
        match res.exit {
            BatchExit::Halted => return,
            BatchExit::Completed | BatchExit::Branch => continue,
            other => panic!("unexpected exit {other:?} at rip={:#x}", cpu.state.rip()),
        }
    }
    panic!("did not reach HLT");
}

#[test]
fn lsl_eax_eax_loads_g_limit_and_sets_zf() {
    let (mut cpu, mut bus) = setup_cpu();
    // LSL EAX, EAX (0F 03 C0); HLT — matches the ntdll sequence at 0x77b20065.
    bus.load(CODE_BASE, &[0x0F, 0x03, 0xC0, 0xF4]);
    cpu.state.set_rip(CODE_BASE);
    cpu.state.write_reg(Register::EAX, 0x08);

    run_until_hlt(&mut cpu, &mut bus);

    assert_eq!(
        cpu.state.read_reg(Register::EAX) & 0xFFFF_FFFF,
        0xFFFF_FFFF,
        "LSL of G=1 limit=FFFFF must yield FFFFFFFF"
    );
    assert!(cpu.state.get_flag(RFLAGS_ZF), "success sets ZF");
}

#[test]
fn lsl_data_segment_byte_limit() {
    let (mut cpu, mut bus) = setup_cpu();
    bus.load(CODE_BASE, &[0x0F, 0x03, 0xC0, 0xF4]);
    cpu.state.set_rip(CODE_BASE);
    cpu.state.write_reg(Register::EAX, 0x10); // data descriptor

    run_until_hlt(&mut cpu, &mut bus);

    assert_eq!(cpu.state.read_reg(Register::EAX) & 0xFFFF_FFFF, 0x0FFF);
    assert!(cpu.state.get_flag(RFLAGS_ZF));
}

#[test]
fn lsl_null_selector_clears_zf_leaves_dest() {
    let (mut cpu, mut bus) = setup_cpu();
    // LSL EBX, EAX; HLT
    bus.load(CODE_BASE, &[0x0F, 0x03, 0xD8, 0xF4]);
    cpu.state.set_rip(CODE_BASE);
    cpu.state.write_reg(Register::EAX, 0x00);
    cpu.state.write_reg(Register::EBX, 0xDEAD_BEEF);
    cpu.state.set_flag(RFLAGS_ZF, true);

    run_until_hlt(&mut cpu, &mut bus);

    assert!(!cpu.state.get_flag(RFLAGS_ZF), "null selector clears ZF");
    assert_eq!(
        cpu.state.read_reg(Register::EBX) & 0xFFFF_FFFF,
        0xDEAD_BEEF,
        "dest unchanged on failure"
    );
}
