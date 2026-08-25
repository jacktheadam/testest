//! COMISD / COMISS — scalar SSE compares that write EFLAGS.
//! Missing COMISD caused SYSTEM_SERVICE_EXCEPTION (0x3B) after LSL was fixed.

use aero_cpu_core::interp::tier0::exec::{run_batch_with_config, BatchExit};
use aero_cpu_core::interp::tier0::Tier0Config;
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::state::{
    CpuMode, CpuState, CR0_MP, CR0_NE, CR4_OSFXSR, FLAG_CF, FLAG_PF, FLAG_ZF,
};
use aero_cpu_core::CpuBus;
use aero_x86::Register;

fn setup_sse_long() -> (CpuState, FlatTestBus) {
    let mut state = CpuState::default();
    let bus = FlatTestBus::new(0x10000);
    state.mode = CpuMode::Long;
    // PE+PG already typical; enable SSE path.
    state.control.cr0 |= CR0_MP | CR0_NE;
    state.control.cr0 &= !0x4; // clear EM
    state.control.cr0 &= !0x8; // clear TS
    state.control.cr4 |= CR4_OSFXSR;
    state.sse.mxcsr = 0x1F80;
    (state, bus)
}

fn run_hlt(state: &mut CpuState, bus: &mut FlatTestBus) {
    let cfg = Tier0Config::default();
    for _ in 0..16 {
        let res = run_batch_with_config(&cfg, state, bus, 8);
        match res.exit {
            BatchExit::Halted => return,
            BatchExit::Completed | BatchExit::Branch => continue,
            other => panic!("unexpected {other:?}"),
        }
    }
    panic!("no HLT");
}

#[test]
fn comisd_equal_sets_zf() {
    let (mut state, mut bus) = setup_sse_long();
    // XMM0 = 1.0 f64, mem [0x3000] = 1.0 f64
    let one = 1.0f64.to_bits();
    state.sse.xmm[0] = u128::from(one);
    bus.write_u64(0x3000, one).unwrap();
    // COMISD xmm0, qword [0x3000]; HLT
    // 66 0F 2F 04 25 00 30 00 00  (comisd xmm0, [abs 0x3000]) — long mode abs
    // Simpler: load into xmm1 and comisd xmm0,xmm1
    // COMISD xmm0, xmm0  = 66 0F 2F C0
    bus.load(0x2000, &[0x66, 0x0F, 0x2F, 0xC0, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    assert!(state.get_flag(FLAG_ZF));
    assert!(!state.get_flag(FLAG_CF));
    assert!(!state.get_flag(FLAG_PF));
}

#[test]
fn comisd_less_sets_cf() {
    let (mut state, mut bus) = setup_sse_long();
    state.sse.xmm[0] = u128::from((-1.0f64).to_bits());
    state.sse.xmm[1] = u128::from((1.0f64).to_bits());
    // COMISD xmm0, xmm1 = 66 0F 2F C1
    bus.load(0x2000, &[0x66, 0x0F, 0x2F, 0xC1, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    assert!(state.get_flag(FLAG_CF));
    assert!(!state.get_flag(FLAG_ZF));
    assert!(!state.get_flag(FLAG_PF));
}

#[test]
fn xorpd_clears_xmm() {
    let (mut state, mut bus) = setup_sse_long();
    state.sse.xmm[0] = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210;
    // XORPD xmm0, xmm0 = 66 0F 57 C0
    bus.load(0x2000, &[0x66, 0x0F, 0x57, 0xC0, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    assert_eq!(state.sse.xmm[0], 0);
}

#[test]
fn cvtdq2pd_converts_two_i32_to_f64() {
    let (mut state, mut bus) = setup_sse_long();
    // XMM1 low 64 = [1, 2] as i32
    state.sse.xmm[1] = 1u128 | (2u128 << 32);
    // CVTDQ2PD xmm0, xmm1 = F3 0F E6 C1
    bus.load(0x2000, &[0xF3, 0x0F, 0xE6, 0xC1, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    let lo = state.sse.xmm[0] as u64;
    let hi = (state.sse.xmm[0] >> 64) as u64;
    assert_eq!(f64::from_bits(lo), 1.0);
    assert_eq!(f64::from_bits(hi), 2.0);
}

#[test]
fn cvtdq2ps_converts_packed_i32() {
    let (mut state, mut bus) = setup_sse_long();
    // XMM1 = [1, 2, 3, 4] as i32 lanes
    let src = (1u128) | (2u128 << 32) | (3u128 << 64) | (4u128 << 96);
    state.sse.xmm[1] = src;
    // CVTDQ2PS xmm0, xmm1 = 0F 5B C1
    bus.load(0x2000, &[0x0F, 0x5B, 0xC1, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    let out = state.sse.xmm[0];
    for (i, expect) in [1.0f32, 2.0, 3.0, 4.0].iter().enumerate() {
        let bits = ((out >> (i * 32)) & 0xFFFF_FFFF) as u32;
        assert_eq!(f32::from_bits(bits), *expect, "lane {i}");
    }
}

#[test]
fn cvtps2pd_converts_two_f32_to_f64() {
    // Win7 win32k site after winpeshl: 0F 5A C8 (CVTPS2PD xmm1, xmm0)
    let (mut state, mut bus) = setup_sse_long();
    let lo = 1.5f32.to_bits() as u128;
    let hi = (-2.25f32).to_bits() as u128;
    state.sse.xmm[0] = lo | (hi << 32);
    // CVTPS2PD xmm1, xmm0 = 0F 5A C8
    bus.load(0x2000, &[0x0F, 0x5A, 0xC8, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    let out = state.sse.xmm[1];
    assert_eq!(f64::from_bits(out as u64), 1.5);
    assert_eq!(f64::from_bits((out >> 64) as u64), -2.25);
}

#[test]
fn cvtpd2ps_converts_two_f64_to_f32() {
    let (mut state, mut bus) = setup_sse_long();
    state.sse.xmm[1] = u128::from(3.0f64.to_bits()) | (u128::from((-4.5f64).to_bits()) << 64);
    // CVTPD2PS xmm0, xmm1 = 66 0F 5A C1
    bus.load(0x2000, &[0x66, 0x0F, 0x5A, 0xC1, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    let out = state.sse.xmm[0];
    assert_eq!(f32::from_bits(out as u32), 3.0);
    assert_eq!(f32::from_bits(((out >> 32) & 0xFFFF_FFFF) as u32), -4.5);
    assert_eq!(out >> 64, 0);
}

#[test]
fn cvtsd2ss_guest_logonui_encoding_preserves_dest_high() {
    // AERO_LOG_UD from boot2-48b LogonUI: f2 0f 5a d0 = CVTSD2SS xmm2, xmm0.
    // Missing this SSE2 scalar convert killed LogonUI/spoolsv with 0xc000001d.
    let (mut state, mut bus) = setup_sse_long();
    state.sse.xmm[0] = u128::from(2.5f64.to_bits());
    let marker = 0xAAAA_AAAA_BBBB_BBBBu128 << 32;
    state.sse.xmm[2] = marker | 0x1111_1111;
    bus.load(0x2000, &[0xF2, 0x0F, 0x5A, 0xD0, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    let out = state.sse.xmm[2];
    assert_eq!(f32::from_bits(out as u32), 2.5);
    assert_eq!(out >> 32, marker >> 32);
}

#[test]
fn cvtss2sd_preserves_dest_high64() {
    let (mut state, mut bus) = setup_sse_long();
    state.sse.xmm[1] = 1.25f32.to_bits() as u128;
    let high = 0xFEED_FACE_DEAD_BEEFu128 << 64;
    state.sse.xmm[0] = high | 0x0123_4567_89AB_CDEF;
    // CVTSS2SD xmm0, xmm1 = F3 0F 5A C1
    bus.load(0x2000, &[0xF3, 0x0F, 0x5A, 0xC1, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    let out = state.sse.xmm[0];
    assert_eq!(f64::from_bits(out as u64), 1.25);
    assert_eq!(out >> 64, high >> 64);
}

#[test]
fn cvtsd2ss_memory_operand() {
    let (mut state, mut bus) = setup_sse_long();
    state.write_reg(Register::RAX, 0x3000);
    bus.write_u64(0x3000, (-8.0f64).to_bits()).unwrap();
    state.sse.xmm[0] = 0xCCCC_CCCC_CCCC_CCCCu128 << 32;
    // CVTSD2SS xmm0, qword [rax] = F2 0F 5A 00
    bus.load(0x2000, &[0xF2, 0x0F, 0x5A, 0x00, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    let out = state.sse.xmm[0];
    assert_eq!(f32::from_bits(out as u32), -8.0);
    assert_eq!(out >> 32, 0xCCCC_CCCC_CCCC_CCCC);
}

#[test]
fn comisd_mem_operand_matches_win7_site() {
    // Pattern from AERO_LOG_UD: 0f 2f 70 0c = COMISD xmm6, qword [rax+0xc]
    // (with 66 prefix on the full encoding; iced accepts 0F 2F as COMISD when
    // the 66 is present as part of the opcode form — actually COMISD is 66 0F 2F)
    let (mut state, mut bus) = setup_sse_long();
    state.sse.xmm[6] = u128::from((2.0f64).to_bits());
    state.write_reg(Register::RAX, 0x3000);
    bus.write_u64(0x3000 + 0xc, (3.0f64).to_bits()).unwrap();
    // 66 0F 2F 70 0C ; HLT
    bus.load(0x2000, &[0x66, 0x0F, 0x2F, 0x70, 0x0C, 0xF4]);
    state.set_rip(0x2000);
    run_hlt(&mut state, &mut bus);
    // 2.0 < 3.0 → CF=1
    assert!(state.get_flag(FLAG_CF));
    assert!(!state.get_flag(FLAG_ZF));
}
