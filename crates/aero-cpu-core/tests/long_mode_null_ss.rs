use aero_cpu_core::assist::AssistContext;
use aero_cpu_core::interp::tier0::exec::{run_batch_with_assists, BatchExit};
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::segmentation::{LoadReason, Seg};
use aero_cpu_core::state::{
    CpuMode, CpuState, CR0_PE, CR0_PG, CR4_PAE, EFER_LME, SEG_ACCESS_DB, SEG_ACCESS_L,
    SEG_ACCESS_PRESENT, SEG_ACCESS_UNUSABLE,
};
use aero_cpu_core::CpuCore;
use aero_cpu_core::{Exception, PagingBus};
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
        let start = paddr as usize;
        self.data[start..start + bytes.len()].copy_from_slice(bytes);
    }
}

impl MemoryBus for TestMemory {
    fn read_u8(&mut self, paddr: u64) -> u8 {
        self.data[paddr as usize]
    }
    fn read_u16(&mut self, paddr: u64) -> u16 {
        let off = paddr as usize;
        u16::from_le_bytes(self.data[off..off + 2].try_into().unwrap())
    }
    fn read_u32(&mut self, paddr: u64) -> u32 {
        let off = paddr as usize;
        u32::from_le_bytes(self.data[off..off + 4].try_into().unwrap())
    }
    fn read_u64(&mut self, paddr: u64) -> u64 {
        let off = paddr as usize;
        u64::from_le_bytes(self.data[off..off + 8].try_into().unwrap())
    }
    fn write_u8(&mut self, paddr: u64, value: u8) {
        self.data[paddr as usize] = value;
    }
    fn write_u16(&mut self, paddr: u64, value: u16) {
        let off = paddr as usize;
        self.data[off..off + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn write_u32(&mut self, paddr: u64, value: u32) {
        let off = paddr as usize;
        self.data[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn write_u64(&mut self, paddr: u64, value: u64) {
        let off = paddr as usize;
        self.data[off..off + 8].copy_from_slice(&value.to_le_bytes());
    }
}

fn long_mode_state() -> CpuState {
    let mut state = CpuState::new(CpuMode::Long);
    state.control.cr0 = CR0_PE | CR0_PG;
    state.control.cr4 = CR4_PAE;
    state.msr.efer = EFER_LME;
    // 64-bit code segment (L=1) so update_mode() reports Long.
    state.segments.cs.selector = 0x10;
    state.segments.cs.base = 0;
    state.segments.cs.limit = 0xFFFFF;
    state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_L;
    state.update_mode();
    assert_eq!(state.mode, CpuMode::Long);
    state
}

#[test]
fn load_ss_null_selector_is_allowed_in_long_mode() {
    let mut state = long_mode_state();
    let mut bus = FlatTestBus::new(0x1000);
    state
        .load_seg(&mut bus, Seg::SS, 0, LoadReason::Data)
        .expect("loading SS with a null selector must be allowed in 64-bit long mode");
    assert_eq!(state.segments.ss.selector, 0);
    assert!(
        state.segments.ss.access & SEG_ACCESS_UNUSABLE != 0,
        "null SS should be marked unusable"
    );
}

#[test]
fn load_ss_null_selector_faults_in_legacy_protected_mode() {
    let mut state = CpuState::new(CpuMode::Protected);
    state.control.cr0 = CR0_PE;
    state.segments.cs.selector = 0x20;
    state.segments.cs.base = 0;
    state.segments.cs.limit = 0xFFFFF;
    state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_DB;
    state.update_mode();
    assert_eq!(state.mode, CpuMode::Protected);

    let mut bus = FlatTestBus::new(0x1000);
    let err = state
        .load_seg(&mut bus, Seg::SS, 0, LoadReason::Data)
        .expect_err("loading SS with a null selector must #GP in legacy protected mode");
    assert!(matches!(err, Exception::GeneralProtection(_)), "{err:?}");
}

/// End-to-end: `mov ss, ax` with AX=0 in 64-bit long mode, as the Win7 kernel does
/// at its long-mode entry. Must not fault, and a subsequent `push` must still work
/// (64-bit stack uses RSP with SS base forced to 0). Verified against QEMU/KVM.
#[test]
fn mov_ss_ax_null_then_push_works_in_long_mode() {
    // Identity-map the low pages we use (code at 0x1000, stack at 0x8000) so paging
    // (required for long mode) resolves.
    let pml4 = 0x10000u64;
    let pdpt = 0x11000u64;
    let pd = 0x12000u64;
    let pt = 0x13000u64;
    let mut phys = TestMemory::new(0x20000);
    phys.write_u64(pml4, pdpt | PTE_P | PTE_RW | PTE_US);
    phys.write_u64(pdpt, pd | PTE_P | PTE_RW | PTE_US);
    phys.write_u64(pd, pt | PTE_P | PTE_RW | PTE_US);
    for i in 0..16u64 {
        phys.write_u64(pt + i * 8, (i * 0x1000) | PTE_P | PTE_RW | PTE_US);
    }
    // code at 0x1000:  66 8e d0 (mov ss,ax) ; 50 (push rax) ; f4 (hlt)
    phys.write_bytes(0x1000, &[0x66, 0x8e, 0xd0, 0x50, 0xF4]);

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
    cpu.state.update_mode();
    assert_eq!(cpu.state.mode, CpuMode::Long);

    cpu.state.set_rip(0x1000);
    cpu.state.write_reg(aero_x86::Register::RAX, 0);
    cpu.state.write_reg(aero_x86::Register::RSP, 0x8000);

    let mut ctx = AssistContext::default();
    let res = run_batch_with_assists(&mut ctx, &mut cpu, &mut bus, 3);
    assert_eq!(res.executed, 3, "exit={:?}", res.exit);
    assert!(
        !matches!(res.exit, BatchExit::Exception(_)),
        "mov ss,ax with AX=0 must not fault in long mode: {:?}",
        res.exit
    );
    assert_eq!(cpu.state.segments.ss.selector, 0);
    // push rax decremented RSP by 8 and wrote the (zero) value at base-0 + 0x7ff8.
    assert_eq!(cpu.state.read_reg(aero_x86::Register::RSP), 0x8000 - 8);
    assert!(cpu.state.halted, "should have executed the trailing HLT");
}
