use aero_cpu_core::interp::tier0::exec::{run_batch_with_config, BatchExit};
use aero_cpu_core::interp::tier0::Tier0Config;
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{
    CpuMode, CpuState, CR0_PE, CR0_PG, CR4_OSFXSR, CR4_PAE, EFER_LME, SEG_ACCESS_L,
    SEG_ACCESS_PRESENT,
};
use aero_x86::Register;

// MOVNTDQ (66 0F E7 /r): non-temporal store of 128-bit XMM → memory (16-byte
// aligned). Architecturally same as MOVDQA store for our model.
//
// Regression: Win7 x64 SSE2 non-temporal memcpy used
// `movntdq [rax+rcx], xmm0` at nt! ~0xfffff8000c0dd7a3 after the long-mode
// interrupt-frame fix, hitting #UD → KeBugCheck 0x1E / STATUS_ILLEGAL_INSTRUCTION.

fn long_sse_state(code: &[u8]) -> (CpuState, FlatTestBus, Tier0Config) {
    let mut bus = FlatTestBus::new(0x4000);
    bus.load(0x400, code);
    let mut state = CpuState::new(CpuMode::Long);
    state.control.cr0 = CR0_PE | CR0_PG;
    state.control.cr4 = CR4_PAE | CR4_OSFXSR;
    state.msr.efer = EFER_LME;
    state.segments.cs.selector = 0x10;
    state.segments.cs.base = 0;
    state.segments.cs.limit = 0xFFFFF;
    state.segments.cs.access = 0xA | 0x10 | SEG_ACCESS_PRESENT | SEG_ACCESS_L;
    state.update_mode();
    assert_eq!(state.mode, CpuMode::Long);
    state.set_rip(0x400);
    let cfg = Tier0Config::default();
    (state, bus, cfg)
}

#[test]
fn movntdq_stores_xmm_aligned() {
    // movntdq [rax+rcx], xmm0  (66 0f e7 04 08) — Win7 witness encoding.
    let (mut state, mut bus, cfg) = long_sse_state(&[0x66, 0x0F, 0xE7, 0x04, 0x08]);
    state.write_reg(Register::RAX, 0x1000);
    state.write_reg(Register::RCX, 0);
    let val = 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFFu128;
    state.sse.xmm[0] = val;

    let r = run_batch_with_config(&cfg, &mut state, &mut bus, 1);
    assert_eq!(r.executed, 1, "exit={:?}", r.exit);
    assert!(!matches!(r.exit, BatchExit::Exception(_)), "{:?}", r.exit);

    let mut tmp = [0u8; 16];
    bus.read_bytes(0x1000, &mut tmp).unwrap();
    assert_eq!(u128::from_le_bytes(tmp), val);
    assert_eq!(state.rip(), 0x405);
}
