//! bcryptprimitives SHA-512 uses ROR/BSWAP/NOT that Tier-1 used to reject.
//!
//! Without those encodings the SHA-512 compress loop was split at every
//! `ror r64, imm8` and stayed in the interpreter for billions of guest
//! instructions (HMAC-SHA512 on plus2b-3600m).

#![cfg(debug_assertions)]

use aero_cpu_core::state::{CpuMode, CpuState};
use aero_jit_x86::tier1::ir::interp::{execute_block_cpu_state, ExecResult};
use aero_jit_x86::{discover_block, translate_block, BlockEndKind, BlockLimits};
use aero_x86::tier1::{InstKind, ShiftOp};
use aero_x86::Register;

mod tier1_common;
use tier1_common::SimpleBus;

const CODE: u64 = 0x1000;

#[test]
fn sha512_ror_bswap_not_stay_in_one_block() {
    // ror rax, 0x22 ; bswap r8 ; not rcx ; ret
    let bytes = [0x48, 0xc1, 0xc8, 0x22, 0x49, 0x0f, 0xc8, 0x48, 0xf7, 0xd1, 0xc3];
    let mut bus = SimpleBus::new(0x2000);
    bus.load(CODE, &bytes);
    let block = discover_block(&bus, CODE, BlockLimits::default());
    assert!(
        block.insts.iter().any(|i| matches!(
            i.kind,
            InstKind::Shift {
                op: ShiftOp::Ror,
                count: 0x22,
                ..
            }
        )),
        "must decode ROR, got {:#?}",
        block.insts
    );
    assert!(
        block
            .insts
            .iter()
            .any(|i| matches!(i.kind, InstKind::Bswap { .. })),
        "must decode BSWAP, got {:#?}",
        block.insts
    );
    assert!(
        block
            .insts
            .iter()
            .any(|i| matches!(i.kind, InstKind::Not { .. })),
        "must decode NOT, got {:#?}",
        block.insts
    );
    assert!(
        matches!(block.end_kind, BlockEndKind::Ret),
        "block must reach RET, not Invalid: {:?}",
        block.end_kind
    );
}

#[test]
fn sha512_ror64_via_ir_matches_rotate_right() {
    let bytes = [0x48, 0xc1, 0xc8, 0x29, 0xc3]; // ror rax, 41 ; ret
    let mut bus = SimpleBus::new(0x2000);
    bus.load(CODE, &bytes);
    let mut state = CpuState::new(CpuMode::Long);
    state.set_rip(CODE);
    let val = 0x6a09e667f3bcc908u64;
    state.write_reg(Register::RAX, val);
    let block = discover_block(&bus, CODE, BlockLimits::default());
    let ir = translate_block(&block);
    match execute_block_cpu_state(&ir, &mut state, &mut bus) {
        ExecResult::Continue | ExecResult::ExitToInterpreter { .. } => {}
    }
    assert_eq!(state.read_reg(Register::RAX), val.rotate_right(41));
}

#[test]
fn sha256_ror32_via_ir_matches_rotate_right() {
    // cryptsp SHA-256 is `ror r32, imm8`. IR interp must not use a 64-bit rotate.
    let bytes = [0xc1, 0xc8, 0x08, 0xc3]; // ror eax, 8 ; ret
    let mut bus = SimpleBus::new(0x2000);
    bus.load(CODE, &bytes);
    let mut state = CpuState::new(CpuMode::Long);
    state.set_rip(CODE);
    let val = 0x1234_5678u64;
    state.write_reg(Register::RAX, val);
    let block = discover_block(&bus, CODE, BlockLimits::default());
    let ir = translate_block(&block);
    match execute_block_cpu_state(&ir, &mut state, &mut bus) {
        ExecResult::Continue | ExecResult::ExitToInterpreter { .. } => {}
    }
    assert_eq!(
        state.read_reg(Register::RAX),
        0x7812_3456,
        "ror eax,8 of 0x12345678"
    );
}
