//! WinSetup LZX bitstream refill (expand grind, ~12% of early-expand samples):
//! - verbatim `0x7fefc0c3891`: `movzx edx,[r12+1]; movzx eax,[r12]; movsx ecx,dil; … add dil,0x10`
//! - aligned  `0x7fefc0c3ea7`: `movzx edx,[rbp+1]; movzx eax,[rbp]; … or r11d,edx; add dil,0x10`
//!
//! The fused Tier-0 path must match single-step through
//! `run_batch_cpu_core_with_assists`.

use aero_cpu_core::assist::AssistContext;
use aero_cpu_core::interp::tier0::exec::{run_batch_cpu_core_with_assists, BatchExit};
use aero_cpu_core::interp::tier0::Tier0Config;
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::state::{gpr, CpuMode};
use aero_cpu_core::CpuCore;

/// `encode_verbatim_block` @ +0x1b3891 (WinSetup.dll).
const VERBATIM: [u8; 34] = [
    0x41, 0x0f, 0xb6, 0x54, 0x24, 0x01, // movzx edx, byte [r12+1]
    0x41, 0x0f, 0xb6, 0x04, 0x24, // movzx eax, byte [r12]
    0x40, 0x0f, 0xbe, 0xcf, // movsx ecx, dil
    0xc1, 0xe2, 0x08, // shl edx, 8
    0xf7, 0xd9, // neg ecx
    0x49, 0x83, 0xc4, 0x02, // add r12, 2
    0x0b, 0xd0, // or edx, eax
    0xd3, 0xe2, // shl edx, cl
    0x0b, 0xda, // or ebx, edx
    0x40, 0x80, 0xc7, 0x10, // add dil, 0x10
];

/// `encode_aligned_block` @ +0x1b3ea7 (WinSetup.dll).
const ALIGNED: [u8; 32] = [
    0x0f, 0xb6, 0x55, 0x01, // movzx edx, byte [rbp+1]
    0x0f, 0xb6, 0x45, 0x00, // movzx eax, byte [rbp]
    0x40, 0x0f, 0xbe, 0xcf, // movsx ecx, dil
    0xc1, 0xe2, 0x08, // shl edx, 8
    0xf7, 0xd9, // neg ecx
    0x48, 0x83, 0xc5, 0x02, // add rbp, 2
    0x0b, 0xd0, // or edx, eax
    0xd3, 0xe2, // shl edx, cl
    0x44, 0x0b, 0xda, // or r11d, edx
    0x40, 0x80, 0xc7, 0x10, // add dil, 0x10
];

const CODE: u64 = 0x1000;
const STREAM: u64 = 0x2000;

fn run(
    cpu: &mut CpuCore,
    bus: &mut FlatTestBus,
    max: u64,
) -> aero_cpu_core::interp::tier0::exec::BatchResult {
    let mut ctx = AssistContext::default();
    let mut cfg = Tier0Config::from_cpuid(&ctx.features);
    cfg.exit_on_branch = false;
    run_batch_cpu_core_with_assists(&cfg, &mut ctx, cpu, bus, max)
}

fn oracle_vs_batch(mut setup: impl FnMut(&mut CpuCore, &mut FlatTestBus)) {
    let mut stepped_bus = FlatTestBus::new(0x4000);
    let mut stepped = CpuCore::new(CpuMode::Long);
    setup(&mut stepped, &mut stepped_bus);
    let mut step_executed = 0u64;
    for _ in 0..64 {
        let res = run(&mut stepped, &mut stepped_bus, 1);
        step_executed += res.executed;
        if matches!(res.exit, BatchExit::Halted) {
            break;
        }
    }

    let mut batched_bus = FlatTestBus::new(0x4000);
    let mut batched = CpuCore::new(CpuMode::Long);
    setup(&mut batched, &mut batched_bus);
    let batched_res = run(&mut batched, &mut batched_bus, 10_000);

    assert_eq!(batched.state.rip(), stepped.state.rip(), "rip");
    assert_eq!(
        batched.state.rflags() & 0x8d5,
        stepped.state.rflags() & 0x8d5,
        "status flags"
    );
    for r in 0..16 {
        assert_eq!(
            batched.state.read_gpr64(r),
            stepped.state.read_gpr64(r),
            "gpr {r}"
        );
    }
    assert_eq!(batched_bus.slice(0, 0x4000), stepped_bus.slice(0, 0x4000));
    assert_eq!(batched_res.executed, step_executed);
    assert!(
        batched_res.executed >= 10,
        "refill is 10 insns + HLT, got {}",
        batched_res.executed
    );
}

fn load_stream(bus: &mut FlatTestBus) {
    bus.load(STREAM, &[0x12, 0x34, 0x56, 0x78]);
}

#[test]
fn verbatim_bit_refill_batch_matches_single_step() {
    oracle_vs_batch(|cpu, bus| {
        bus.load(CODE, &VERBATIM);
        bus.load(CODE + VERBATIM.len() as u64, &[0xF4]);
        load_stream(bus);
        cpu.state.set_rip(CODE);
        cpu.state.write_gpr64(gpr::R12, STREAM);
        cpu.state.write_gpr64(gpr::RDI, 4);
        cpu.state.write_gpr64(gpr::RBX, 0x100);
        cpu.state.write_gpr64(gpr::RAX, 0);
        cpu.state.write_gpr64(gpr::RDX, 0);
        cpu.state.write_gpr64(gpr::RCX, 0);
    });
}

#[test]
fn aligned_bit_refill_batch_matches_single_step() {
    oracle_vs_batch(|cpu, bus| {
        bus.load(CODE, &ALIGNED);
        bus.load(CODE + ALIGNED.len() as u64, &[0xF4]);
        load_stream(bus);
        cpu.state.set_rip(CODE);
        cpu.state.write_gpr64(gpr::RBP, STREAM);
        cpu.state.write_gpr64(gpr::RDI, 4);
        cpu.state.write_gpr64(gpr::R11, 0x200);
        cpu.state.write_gpr64(gpr::RAX, 0);
        cpu.state.write_gpr64(gpr::RDX, 0);
        cpu.state.write_gpr64(gpr::RCX, 0);
    });
}

#[test]
fn verbatim_bit_refill_wraps_dil_and_matches_oracle() {
    oracle_vs_batch(|cpu, bus| {
        bus.load(CODE, &VERBATIM);
        bus.load(CODE + VERBATIM.len() as u64, &[0xF4]);
        load_stream(bus);
        cpu.state.set_rip(CODE);
        cpu.state.write_gpr64(gpr::R12, STREAM);
        cpu.state.write_gpr64(gpr::RDI, 0xf8);
        cpu.state.write_gpr64(gpr::RBX, 0);
        cpu.state.write_gpr64(gpr::RAX, 0);
        cpu.state.write_gpr64(gpr::RDX, 0);
        cpu.state.write_gpr64(gpr::RCX, 0);
    });
}
