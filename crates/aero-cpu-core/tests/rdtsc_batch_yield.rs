//! `AERO_RDTSC_QUANTUM` recognizes tight timestamp polling loops without changing architectural
//! time. Tier-0 yields so the outer `Machine::run_slice` loop can tick RTC/PIT and poll IRQ8 before
//! a HAL RDTSC-bounded clock-sync wait expires mid-batch (0x5C / 0x10B).

use aero_cpu_core::assist::AssistContext;
use aero_cpu_core::interp::tier0::exec::{run_batch_cpu_core_with_assists, BatchExit};
use aero_cpu_core::interp::tier0::Tier0Config;
use aero_cpu_core::mem::FlatTestBus;
use aero_cpu_core::state::{CpuMode, RFLAGS_IF};
use aero_cpu_core::CpuCore;

const CODE_BASE: u64 = 0x2000;

#[test]
fn repeated_same_site_rdtsc_with_quantum_yields_batch_for_timer_poll() {
    // Force quantum for this process (OnceLock: set before first RDTSC assist).
    std::env::set_var("AERO_RDTSC_QUANTUM", "2000");

    let mut bus = FlatTestBus::new(0x10000);
    // Tight loop: RDTSC; JMP back to the same RDTSC.
    bus.load(CODE_BASE, &[0x0F, 0x31, 0xEB, 0xFC]);

    let mut cpu = CpuCore::new(CpuMode::Long);
    cpu.state.set_rip(CODE_BASE);
    cpu.state.set_flag(RFLAGS_IF, true);
    let mut ctx = AssistContext::default();
    let mut cfg = Tier0Config::from_cpuid(&ctx.features);
    cfg.exit_on_branch = false;

    let batch = run_batch_cpu_core_with_assists(&cfg, &mut ctx, &mut cpu, &mut bus, 64);
    assert_eq!(
        batch.exit,
        BatchExit::Branch,
        "RDTSC with quantum must yield the batch (got {:?}, executed={})",
        batch.exit,
        batch.executed
    );
    assert_eq!(
        batch.executed, 15,
        "seven warm-up iterations should remain exact before the eighth RDTSC accelerates"
    );
    assert_eq!(
        cpu.state.rip(),
        CODE_BASE + 2,
        "RIP should advance past the first RDTSC"
    );
    assert!(
        cpu.time.read_tsc() < 2000,
        "scheduler-yield optimization must not change the architectural TSC"
    );
}

#[test]
fn distinct_rdtsc_sites_do_not_fast_forward_or_yield() {
    std::env::set_var("AERO_RDTSC_QUANTUM", "2000");

    let mut bus = FlatTestBus::new(0x10000);
    let mut code = Vec::new();
    for _ in 0..16 {
        code.extend_from_slice(&[0x0F, 0x31]);
    }
    code.push(0xF4); // HLT
    bus.load(CODE_BASE, &code);

    let mut cpu = CpuCore::new(CpuMode::Long);
    cpu.state.set_rip(CODE_BASE);
    let mut ctx = AssistContext::default();
    let mut cfg = Tier0Config::from_cpuid(&ctx.features);
    cfg.exit_on_branch = false;

    let batch = run_batch_cpu_core_with_assists(&cfg, &mut ctx, &mut cpu, &mut bus, 64);
    assert_eq!(batch.exit, BatchExit::Halted);
    assert_eq!(batch.executed, 17);
    assert!(
        cpu.time.read_tsc() < 2000,
        "one-off timestamp/entropy samples at distinct sites must not receive the polling quantum"
    );
}

#[test]
fn tight_rdtsc_loop_with_interrupts_disabled_does_not_fast_forward() {
    std::env::set_var("AERO_RDTSC_QUANTUM", "2000");

    let mut bus = FlatTestBus::new(0x10000);
    bus.load(CODE_BASE, &[0x0F, 0x31, 0xEB, 0xFC]);

    let mut cpu = CpuCore::new(CpuMode::Long);
    cpu.state.set_rip(CODE_BASE);
    cpu.state.set_flag(RFLAGS_IF, false);
    let mut ctx = AssistContext::default();
    let mut cfg = Tier0Config::from_cpuid(&ctx.features);
    cfg.exit_on_branch = false;

    let batch = run_batch_cpu_core_with_assists(&cfg, &mut ctx, &mut cpu, &mut bus, 256);
    assert_eq!(batch.exit, BatchExit::Completed);
    assert_eq!(batch.executed, 256);
    assert!(
        cpu.time.read_tsc() < 2000,
        "IF=0 calibration/deadline loops must not receive an interrupt-wait scheduler yield"
    );
}
