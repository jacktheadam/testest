use aero_cpu_core::assist::AssistContext;
use aero_cpu_core::interp::tier0::exec::{run_batch_with_assists, BatchExit};
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::state::{CpuMode, CR0_PE, SEG_ACCESS_DB, SEG_ACCESS_PRESENT};
use aero_cpu_core::CpuCore;
use aero_x86::Register;

// Build a raw GDT/LDT descriptor (same bit layout as descriptors_paging.rs).
#[allow(clippy::too_many_arguments)]
fn make_descriptor(
    base: u32,
    limit_raw: u32,
    typ: u8,
    s: bool,
    dpl: u8,
    present: bool,
    avl: bool,
    l: bool,
    db: bool,
    g: bool,
) -> u64 {
    let mut raw = 0u64;
    raw |= (limit_raw & 0xFFFF) as u64;
    raw |= ((base & 0xFFFF) as u64) << 16;
    raw |= (((base >> 16) & 0xFF) as u64) << 32;
    let access =
        (typ as u64) | ((s as u64) << 4) | (((dpl as u64) & 0x3) << 5) | ((present as u64) << 7);
    raw |= access << 40;
    raw |= (((limit_raw >> 16) & 0xF) as u64) << 48;
    let flags = (avl as u64) | ((l as u64) << 1) | ((db as u64) << 2) | ((g as u64) << 3);
    raw |= flags << 52;
    raw |= (((base >> 24) & 0xFF) as u64) << 56;
    raw
}

/// `66 cb` (RETFD) inside a 16-bit protected-mode code segment must pop a 4-byte
/// offset *and* a 4-byte CS selector slot: the operand-size prefix (0x66), not the
/// code-segment default size, determines the frame width.
///
/// Regression test for a bug where the protected-mode assist path derived the pop
/// width from `state.bitness()` (16) instead of the decoded operand size (32). The
/// old code split the 32-bit offset `0x401000` into `off=0x1000, cs=0x40` and far-"
/// returned" into a TSS selector, raising #GP — this is exactly what the Win7 boot
/// manager hit when transitioning from its 16-bit stub to 32-bit protected mode.
#[test]
fn retfd_32bit_frame_in_16bit_protected_mode() {
    let gdt_base = 0x1000u64;
    let cs_base = 0x20000u64;
    let esp0 = 0x8000u64;
    let target_sel = 0x20u16; // 32-bit code segment (GDT[4])
    let target_off = 0x401000u32;

    let mut bus = FlatTestBus::new(0x30000);

    // GDT[4]  = 0x20: 32-bit code, base 0, 4GiB.
    let code32 = make_descriptor(0, 0xFFFFF, 0xA, true, 0, true, false, false, true, true);
    // GDT[6]  = 0x30: 32-bit data, base 0, 4GiB.
    let data32 = make_descriptor(0, 0xFFFFF, 0x2, true, 0, true, false, false, true, true);
    // GDT[10] = 0x50: 16-bit code, base 0x20000 (the current CS).
    let code16 = make_descriptor(
        0x20000, 0xFFFF, 0xA, true, 0, true, false, false, false, false,
    );
    bus.load(gdt_base + 4 * 8, &code32.to_le_bytes());
    bus.load(gdt_base + 6 * 8, &data32.to_le_bytes());
    bus.load(gdt_base + 10 * 8, &code16.to_le_bytes());

    // `66 cb` = RETFD at the current (16-bit) CS base.
    bus.load(cs_base, &[0x66, 0xcb]);

    // RETFD frame on the stack: [offset:4][selector:4].
    bus.load(esp0, &target_off.to_le_bytes());
    bus.load(esp0 + 4, &(target_sel as u32).to_le_bytes());

    let mut cpu = CpuCore::new(CpuMode::Protected);
    cpu.state.control.cr0 = CR0_PE;
    cpu.state.update_mode();
    cpu.state.tables.gdtr.base = gdt_base;
    cpu.state.tables.gdtr.limit = (11 * 8 - 1) as u16;

    // Current CS: 16-bit code segment (D=0), base 0x20000.
    cpu.state.segments.cs.selector = 0x50;
    cpu.state.segments.cs.base = cs_base;
    cpu.state.segments.cs.limit = 0xFFFF;
    cpu.state.segments.cs.access = 0x9A; // present, S=1, type=code(exec/read), D=0
                                         // SS: 32-bit data segment, base 0.
    cpu.state.segments.ss.selector = 0x30;
    cpu.state.segments.ss.base = 0;
    cpu.state.segments.ss.limit = 0xFFFFF;
    cpu.state.segments.ss.access = 0x2 | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_DB;

    cpu.state.write_reg(Register::RSP, esp0);
    cpu.state.set_rip(0);

    let mut ctx = AssistContext::default();
    let res = run_batch_with_assists(&mut ctx, &mut cpu, &mut bus, 1);

    assert_eq!(res.executed, 1, "exit={:?}", res.exit);
    assert!(
        !matches!(res.exit, BatchExit::Exception(_)),
        "RETFD must not fault: {:?}",
        res.exit
    );
    // The whole point: the far return must land in the 32-bit segment at the full
    // 32-bit offset, not split the frame as if it were 16-bit.
    assert_eq!(cpu.state.segments.cs.selector, target_sel);
    assert_eq!(cpu.state.rip(), u64::from(target_off));
    assert_eq!(cpu.state.read_reg(Register::RSP), esp0 + 8);
    assert_eq!(
        cpu.state.bitness(),
        32,
        "target CS should be a 32-bit segment"
    );
}
