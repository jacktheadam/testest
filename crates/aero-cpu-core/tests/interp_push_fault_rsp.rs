//! PUSH / CALL must leave RSP unchanged when the stack store #PF's.
//!
//! Previously `push` decremented RSP *before* writing. A stack-grow #PF on
//! CALL's return-address push then IRET-restarted the CALL with RSP already
//! lowered → double-push → /GS cookie check read the wrong slot →
//! STATUS_STACK_BUFFER_OVERRUN → Win7 SESSION5 (0x71).

use aero_cpu_core::interp::tier0::exec::{run_batch, BatchExit};
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{
    CpuMode, CpuState, CR0_PE, CR0_PG, CR4_PAE, EFER_LME, SEG_ACCESS_L, SEG_ACCESS_PRESENT,
};
use aero_cpu_core::Exception;
use aero_x86::Register;

/// Bus that #PF's write_u64 in a chosen half-open range (for stack-grow faults).
struct FaultingStackBus {
    inner: FlatTestBus,
    fault_lo: u64,
    fault_hi: u64,
}

impl FaultingStackBus {
    fn new(size: usize, fault_lo: u64, fault_hi: u64) -> Self {
        Self {
            inner: FlatTestBus::new(size),
            fault_lo,
            fault_hi,
        }
    }

    fn load(&mut self, addr: u64, bytes: &[u8]) {
        self.inner.load(addr, bytes);
    }

    fn should_fault(&self, vaddr: u64) -> bool {
        vaddr >= self.fault_lo && vaddr < self.fault_hi
    }
}

impl CpuBus for FaultingStackBus {
    fn read_u8(&mut self, vaddr: u64) -> Result<u8, Exception> {
        self.inner.read_u8(vaddr)
    }
    fn read_u16(&mut self, vaddr: u64) -> Result<u16, Exception> {
        self.inner.read_u16(vaddr)
    }
    fn read_u32(&mut self, vaddr: u64) -> Result<u32, Exception> {
        self.inner.read_u32(vaddr)
    }
    fn read_u64(&mut self, vaddr: u64) -> Result<u64, Exception> {
        self.inner.read_u64(vaddr)
    }
    fn read_u128(&mut self, vaddr: u64) -> Result<u128, Exception> {
        self.inner.read_u128(vaddr)
    }
    fn write_u8(&mut self, vaddr: u64, val: u8) -> Result<(), Exception> {
        if self.should_fault(vaddr) {
            return Err(Exception::PageFault {
                addr: vaddr,
                error_code: 0x2,
            });
        }
        self.inner.write_u8(vaddr, val)
    }
    fn write_u16(&mut self, vaddr: u64, val: u16) -> Result<(), Exception> {
        if self.should_fault(vaddr) {
            return Err(Exception::PageFault {
                addr: vaddr,
                error_code: 0x2,
            });
        }
        self.inner.write_u16(vaddr, val)
    }
    fn write_u32(&mut self, vaddr: u64, val: u32) -> Result<(), Exception> {
        if self.should_fault(vaddr) {
            return Err(Exception::PageFault {
                addr: vaddr,
                error_code: 0x2,
            });
        }
        self.inner.write_u32(vaddr, val)
    }
    fn write_u64(&mut self, vaddr: u64, val: u64) -> Result<(), Exception> {
        if self.should_fault(vaddr) {
            return Err(Exception::PageFault {
                addr: vaddr,
                error_code: 0x2,
            });
        }
        self.inner.write_u64(vaddr, val)
    }
    fn write_u128(&mut self, vaddr: u64, val: u128) -> Result<(), Exception> {
        if self.should_fault(vaddr) {
            return Err(Exception::PageFault {
                addr: vaddr,
                error_code: 0x2,
            });
        }
        self.inner.write_u128(vaddr, val)
    }
    fn fetch(&mut self, vaddr: u64, max_len: usize) -> Result<[u8; 15], Exception> {
        self.inner.fetch(vaddr, max_len)
    }
    fn io_read(&mut self, port: u16, size: u32) -> Result<u64, Exception> {
        self.inner.io_read(port, size)
    }
    fn io_write(&mut self, port: u16, size: u32, val: u64) -> Result<(), Exception> {
        self.inner.io_write(port, size, val)
    }
}

fn long_state(code_at: u64, rsp: u64, bus: &mut FaultingStackBus) -> CpuState {
    let mut state = CpuState::new(CpuMode::Long);
    state.control.cr0 = CR0_PE | CR0_PG;
    state.control.cr4 = CR4_PAE;
    state.msr.efer = EFER_LME;
    state.segments.cs.selector = 0x10;
    state.segments.cs.base = 0;
    state.segments.cs.limit = 0xFFFFF;
    state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_L;
    state.segments.ss.selector = 0x18;
    state.segments.ss.base = 0;
    state.segments.ss.limit = 0xFFFFF;
    state.segments.ss.access = 0x2 | 0x10 | SEG_ACCESS_PRESENT;
    state.update_mode();
    assert_eq!(state.mode, CpuMode::Long);
    state.set_rip(code_at);
    state.write_reg(Register::RSP, rsp);
    let _ = bus;
    state
}

#[test]
fn call_near_stack_pf_leaves_rsp_unchanged() {
    // E8 rel32 CALL at 0x400 targeting 0x500. Stack top at 0x2000; the return
    // address store lands at 0x1FF8, which we mark as faulting.
    let code = [0xE8, 0xFB, 0x00, 0x00, 0x00]; // call +0x100 → 0x500
    let mut bus = FaultingStackBus::new(0x4000, 0x1FF0, 0x2000);
    bus.load(0x400, &code);
    bus.load(0x500, &[0xC3]); // ret at target (unused on the faulting attempt)
    let mut state = long_state(0x400, 0x2000, &mut bus);

    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(
        res.executed, 0,
        "CALL must not retire on stack #PF; exit={:?}",
        res.exit
    );
    assert!(
        matches!(
            res.exit,
            BatchExit::Exception(Exception::PageFault { addr: 0x1FF8, .. })
                | BatchExit::Exception(Exception::PageFault { .. })
        ),
        "expected stack #PF, got {:?}",
        res.exit
    );
    // Architecturally restartable: RIP still at CALL, RSP not decremented.
    assert_eq!(state.rip(), 0x400, "RIP must remain at the faulting CALL");
    assert_eq!(
        state.read_reg(Register::RSP),
        0x2000,
        "RSP must not be half-applied across a stack #PF"
    );
}

#[test]
fn push_rm64_stack_pf_leaves_rsp_unchanged() {
    // push rax (50) with RSP=0x2000; store at 0x1FF8 faults.
    let code = [0x50];
    let mut bus = FaultingStackBus::new(0x4000, 0x1FF0, 0x2000);
    bus.load(0x400, &code);
    let mut state = long_state(0x400, 0x2000, &mut bus);
    state.write_reg(Register::RAX, 0x1111_2222_3333_4444);

    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(
        res.executed, 0,
        "PUSH must not retire on stack #PF; exit={:?}",
        res.exit
    );
    assert!(
        matches!(res.exit, BatchExit::Exception(Exception::PageFault { .. })),
        "expected stack #PF, got {:?}",
        res.exit
    );
    assert_eq!(state.rip(), 0x400);
    assert_eq!(state.read_reg(Register::RSP), 0x2000);
}

#[test]
fn call_near_succeeds_when_stack_present() {
    let code = [0xE8, 0xFB, 0x00, 0x00, 0x00]; // call 0x500
                                               // No fault range (fault_lo == fault_hi).
    let mut bus = FaultingStackBus::new(0x4000, 0, 0);
    bus.load(0x400, &code);
    bus.load(0x500, &[0x90]); // nop
    let mut state = long_state(0x400, 0x2000, &mut bus);

    let res = run_batch(&mut state, &mut bus, 1);
    assert_eq!(res.executed, 1, "exit={:?}", res.exit);
    assert_eq!(state.rip(), 0x500);
    assert_eq!(state.read_reg(Register::RSP), 0x1FF8);
    let mut tmp = [0u8; 8];
    bus.read_bytes(0x1FF8, &mut tmp).unwrap();
    assert_eq!(u64::from_le_bytes(tmp), 0x405); // return address
}
