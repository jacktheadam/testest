use aero_cpu_core::assist::AssistContext;
use aero_cpu_core::interp::tier0::exec::{run_batch_with_assists, BatchExit};
use aero_cpu_core::segmentation::{LoadReason, Seg};
use aero_cpu_core::state::{
    CpuMode, CpuState, CR0_PE, CR0_PG, CR4_PAE, EFER_LME, SEG_ACCESS_DB, SEG_ACCESS_L,
    SEG_ACCESS_PRESENT,
};
use aero_cpu_core::CpuCore;
use aero_cpu_core::PagingBus;
use aero_mmu::MemoryBus;
use core::convert::TryInto;

const PTE_P: u64 = 1 << 0;
const PTE_RW: u64 = 1 << 1;
const PTE_US: u64 = 1 << 2;

#[derive(Clone, Debug)]
struct TestMemory {
    data: Vec<u8>,
}
impl TestMemory {
    fn new(size: usize) -> Self {
        Self {
            data: vec![0; size],
        }
    }
    fn write_bytes(&mut self, paddr: u64, bytes: &[u8]) {
        let s = paddr as usize;
        self.data[s..s + bytes.len()].copy_from_slice(bytes);
    }
}
impl MemoryBus for TestMemory {
    fn read_u8(&mut self, paddr: u64) -> u8 {
        self.data[paddr as usize]
    }
    fn read_u16(&mut self, paddr: u64) -> u16 {
        let o = paddr as usize;
        u16::from_le_bytes(self.data[o..o + 2].try_into().unwrap())
    }
    fn read_u32(&mut self, paddr: u64) -> u32 {
        let o = paddr as usize;
        u32::from_le_bytes(self.data[o..o + 4].try_into().unwrap())
    }
    fn read_u64(&mut self, paddr: u64) -> u64 {
        let o = paddr as usize;
        u64::from_le_bytes(self.data[o..o + 8].try_into().unwrap())
    }
    fn write_u8(&mut self, paddr: u64, value: u8) {
        self.data[paddr as usize] = value;
    }
    fn write_u16(&mut self, paddr: u64, value: u16) {
        let o = paddr as usize;
        self.data[o..o + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn write_u32(&mut self, paddr: u64, value: u32) {
        let o = paddr as usize;
        self.data[o..o + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn write_u64(&mut self, paddr: u64, value: u64) {
        let o = paddr as usize;
        self.data[o..o + 8].copy_from_slice(&value.to_le_bytes());
    }
}

// The parameters are the descriptor's architectural fields, in the order the Intel manual lists
// them. Grouping them into a struct would put a layer between the call sites and the bit layout
// they are checking, which is the thing these tests are about.
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

fn long_mode_state_with_gdt(bus: &mut impl aero_cpu_core::mem::CpuBus, gdt_base: u64) -> CpuState {
    // GDT[2]=0x10: 64-bit code (current CS); GDT[4]=0x20: legacy 32-bit code (L=0).
    let code64 = make_descriptor(0, 0xFFFFF, 0xA, true, 0, true, false, true, false, true);
    let code32 = make_descriptor(0, 0xFFFFF, 0xA, true, 0, true, false, false, true, true);
    bus.write_u64(gdt_base + 2 * 8, code64).unwrap();
    bus.write_u64(gdt_base + 4 * 8, code32).unwrap();

    let mut state = CpuState::new(CpuMode::Long);
    state.control.cr0 = CR0_PE | CR0_PG;
    state.control.cr4 = CR4_PAE;
    state.msr.efer = EFER_LME;
    state.segments.cs.selector = 0x10;
    state.segments.cs.base = 0;
    state.segments.cs.limit = 0xFFFFF;
    state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_L;
    state.tables.gdtr.base = gdt_base;
    state.tables.gdtr.limit = (6 * 8 - 1) as u16;
    state.update_mode();
    assert_eq!(state.mode, CpuMode::Long);
    state
}

#[test]
fn far_transfer_to_legacy_code_segment_in_long_mode_enters_compatibility_mode() {
    let gdt_base = 0x1000u64;
    let mut bus = aero_cpu_core::mem::FlatTestBus::new(0x2000);
    let mut state = long_mode_state_with_gdt(&mut bus, gdt_base);

    // Far transfer (as RETF/ljmp does) to the legacy 32-bit code segment 0x20.
    state
        .load_seg(&mut bus, Seg::CS, 0x20, LoadReason::FarControlTransfer)
        .expect("far transfer to a legacy CS in long mode must be allowed (compatibility mode)");
    assert_eq!(state.segments.cs.selector, 0x20);
    // The mode must drop out of Long (compatibility mode is modelled as Protected).
    assert_eq!(state.mode, CpuMode::Protected);
    // ... and EFER.LME stays set so the MMU keeps 4-level long-mode paging.
    assert_ne!(state.msr.efer & EFER_LME, 0);
}

/// End-to-end: `retf` in long mode returning to a legacy 32-bit code segment, which
/// is the exact instruction the Win7 boot path faults on. Must not raise #GP.
#[test]
fn retfq_to_legacy_code_segment_in_long_mode_does_not_fault() {
    let pml4 = 0x10000u64;
    let pdpt = 0x11000u64;
    let pd = 0x12000u64;
    let pt = 0x13000u64;
    let gdt_base = 0x14000u64;
    let mut phys = TestMemory::new(0x20000);
    phys.write_u64(pml4, pdpt | PTE_P | PTE_RW | PTE_US);
    phys.write_u64(pdpt, pd | PTE_P | PTE_RW | PTE_US);
    phys.write_u64(pd, pt | PTE_P | PTE_RW | PTE_US);
    for i in 0..32u64 {
        phys.write_u64(pt + i * 8, (i * 0x1000) | PTE_P | PTE_RW | PTE_US);
    }
    // GDT[2]=0x10 64-bit code (current); GDT[4]=0x20 legacy 32-bit code.
    let code64 = make_descriptor(0, 0xFFFFF, 0xA, true, 0, true, false, true, false, true);
    let code32 = make_descriptor(0, 0xFFFFF, 0xA, true, 0, true, false, false, true, true);
    phys.write_u64(gdt_base + 2 * 8, code64);
    phys.write_u64(gdt_base + 4 * 8, code32);
    // code at 0x1000: 48 cb (retfq). Return frame at 0x8000: off=0x2000, sel=0x20.
    phys.write_bytes(0x1000, &[0x48, 0xcb]);
    phys.write_bytes(0x2000, &[0x90, 0x90, 0xF4]); // nops + hlt at the 32-bit target
    phys.write_u64(0x8000, 0x2000);
    phys.write_u64(0x8008, 0x20);

    let mut bus = PagingBus::new(phys);
    let mut cpu = CpuCore::new(CpuMode::Long);
    cpu.state.control.cr3 = pml4;
    cpu.state.control.cr0 = CR0_PE | CR0_PG;
    cpu.state.control.cr4 = CR4_PAE;
    cpu.state.msr.efer = EFER_LME;
    cpu.state.segments.cs.selector = 0x10;
    cpu.state.segments.cs.base = 0;
    cpu.state.segments.cs.limit = 0xFFFFF;
    cpu.state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_L;
    cpu.state.segments.ss.selector = 0x18;
    cpu.state.segments.ss.base = 0;
    cpu.state.segments.ss.limit = 0xFFFFF;
    cpu.state.segments.ss.access = 0x2 | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_DB;
    cpu.state.tables.gdtr.base = gdt_base;
    cpu.state.tables.gdtr.limit = (6 * 8 - 1) as u16;
    cpu.state.update_mode();
    assert_eq!(cpu.state.mode, CpuMode::Long);

    cpu.state.set_rip(0x1000);
    cpu.state.write_reg(aero_x86::Register::RSP, 0x8000);

    let mut ctx = AssistContext::default();
    let res = run_batch_with_assists(&mut ctx, &mut cpu, &mut bus, 1);
    assert_eq!(res.executed, 1, "exit={:?}", res.exit);
    assert!(
        !matches!(res.exit, BatchExit::Exception(_)),
        "retfq to a legacy code segment in long mode must not #GP: {:?}",
        res.exit
    );
    assert_eq!(cpu.state.segments.cs.selector, 0x20);
    assert_eq!(cpu.state.rip(), 0x2000);
    assert_eq!(cpu.state.mode, CpuMode::Protected);
}
