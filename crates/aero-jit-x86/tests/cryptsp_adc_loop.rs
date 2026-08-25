//! cryptsp 0xc0a0 (64-bit limb add) must JIT as a single ADC carry chain.
//!
//! Without ADC in Tier-1, the loop was split at `adc r10, 0` and the 4096-bit
//! RSA path in Win7 cryptsp produced a garbage PKCS#1 block under `--jit`.

#![cfg(debug_assertions)]

use aero_cpu_core::state::{CpuMode, CpuState};
use aero_jit_x86::tier1::ir::interp::{execute_block_cpu_state, ExecResult};
use aero_jit_x86::{discover_block, translate_block, BlockEndKind, BlockLimits, Tier1Bus};
use aero_types::Width;
use aero_x86::tier1::{AluOp, InstKind};
use aero_x86::Register;

mod tier1_common;
use tier1_common::SimpleBus;

/// cryptsp `0xc0a0` limb-add (dest = a + b, carry in RAX).
const C0A0: &[u8] = &[
    0x45, 0x0b, 0xc9, 0x74, 0x41, 0x4a, 0x8d, 0x0c, 0xc9, 0x4a, 0x8d, 0x14, 0xca, 0x4f, 0x8d,
    0x04, 0xc8, 0x49, 0xf7, 0xd9, 0x4d, 0x33, 0xd2, 0x66, 0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x4a, 0x8b, 0x04, 0xca, 0x49, 0x03, 0xc2, 0x41, 0xba, 0x00, 0x00, 0x00, 0x00,
    0x49, 0x83, 0xd2, 0x00, 0x4b, 0x03, 0x04, 0xc8, 0x49, 0x83, 0xd2, 0x00, 0x4a, 0x89, 0x04,
    0xc9, 0x49, 0xff, 0xc1, 0x75, 0xde, 0x49, 0x8b, 0xc2, 0xc3,
];

const CODE: u64 = 0x1000;
const LOOP: u64 = CODE + 0x20; // 0xc0c0 - 0xc0a0
const DEST: u64 = 0x4000;
const SRC_A: u64 = 0x5000;
const SRC_B: u64 = 0x6000;

#[test]
fn cryptsp_c0a0_loop_decodes_adc_and_stays_in_one_block() {
    let mut bus = SimpleBus::new(0x8000);
    bus.load(CODE, C0A0);
    let block = discover_block(&bus, LOOP, BlockLimits::default());
    assert!(
        block.insts.iter().any(|i| matches!(
            i.kind,
            InstKind::Alu {
                op: AluOp::Adc,
                ..
            }
        )),
        "loop must include ADC, got {:#?}",
        block.insts
    );
    assert!(
        matches!(block.end_kind, BlockEndKind::Jcc),
        "loop must end at JNE, not Invalid: {:?}",
        block.end_kind
    );
}

#[test]
fn cryptsp_c0a0_64_limb_all_ones_matches_reference_via_ir() {
    const N: usize = 64;
    let mut bus = SimpleBus::new(0x8000);
    bus.load(CODE, C0A0);
    for i in 0..N {
        let off = (i * 8) as u64;
        bus.write(DEST + off, Width::W64, 0);
        bus.write(SRC_A + off, Width::W64, u64::MAX);
        bus.write(SRC_B + off, Width::W64, u64::MAX);
    }

    let nbytes = (N * 8) as i64;
    let mut state = CpuState::new(CpuMode::Long);
    state.set_rip(LOOP);
    state.write_reg(Register::RCX, DEST + nbytes as u64);
    state.write_reg(Register::RDX, SRC_A + nbytes as u64);
    state.write_reg(Register::R8, SRC_B + nbytes as u64);
    // The loop at 0xc0c0 expects r9 already negated.
    state.write_reg(Register::R9, (-nbytes) as u64);
    state.write_reg(Register::R10, 0);

    let block = discover_block(&bus, LOOP, BlockLimits::default());
    let ir = translate_block(&block);

    let mut steps = 0u32;
    loop {
        steps += 1;
        assert!(steps < 10_000, "loop did not terminate");
        match execute_block_cpu_state(&ir, &mut state, &mut bus) {
            ExecResult::Continue => {
                if state.rip() != LOOP {
                    break;
                }
            }
            ExecResult::ExitToInterpreter { next_rip } => {
                panic!("ADC loop still exits to interpreter at {next_rip:#x}");
            }
        }
    }

    // dest[i] = MAX+MAX = 0xFFFFFFFFFFFFFFFE plus carry-in from previous.
    // all-ones + all-ones: dest = [0xFFFFFFFFFFFFFFFE, 0xFFFFFFFFFFFFFFFF, ...] carry 1
    let mut expect = vec![0u64; N];
    let mut carry = 0u128;
    for i in 0..N {
        let t = (u64::MAX as u128) + (u64::MAX as u128) + carry;
        expect[i] = t as u64;
        carry = t >> 64;
    }
    for i in 0..N {
        let got = bus.read(DEST + (i * 8) as u64, Width::W64);
        assert_eq!(got, expect[i], "limb {i}");
    }
    assert_eq!(state.read_reg(Register::R10), carry as u64);
}
