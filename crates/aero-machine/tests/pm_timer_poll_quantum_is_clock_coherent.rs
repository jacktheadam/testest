use aero_cpu_core::state::CpuMode;
use aero_devices::acpi_pm::DEFAULT_PM_TMR_BLK;
use aero_machine::{Machine, MachineConfig, RunExit};

const CODE_BASE: u64 = 0x2000;
const TSC0: u64 = 0x0500;
const PM0: u64 = 0x0504;
const TSC1: u64 = 0x0508;
const PM1: u64 = 0x050c;
const TSC2: u64 = 0x0510;
const PM_TIMER_MASK_24BIT: u32 = 0x00ff_ffff;
const POLL_QUANTUM_CYCLES: u32 = 300_000;

fn setup_real_mode_cpu(machine: &mut Machine) {
    let cpu = machine.cpu_core_mut_by_index(0);
    cpu.state.mode = CpuMode::Real;

    for segment in [
        &mut cpu.state.segments.cs,
        &mut cpu.state.segments.ds,
        &mut cpu.state.segments.es,
        &mut cpu.state.segments.ss,
        &mut cpu.state.segments.fs,
        &mut cpu.state.segments.gs,
    ] {
        segment.selector = 0;
        segment.base = 0;
        segment.limit = 0xffff;
        segment.access = 0;
    }

    cpu.state.set_stack_ptr(0x7000);
    cpu.state.set_rip(CODE_BASE);
    cpu.state.set_rflags(0x2);
    cpu.state.halted = false;
}

fn append_store_eax(code: &mut Vec<u8>, address: u64) {
    let address = (address as u16).to_le_bytes();
    code.extend_from_slice(&[0x66, 0xa3, address[0], address[1]]);
}

#[test]
fn pm_timer_poll_quantum_advances_tsc_and_platform_time_together() {
    // This integration test has its own process, so it initializes the machine's
    // cached poll quantum before any guest I/O read can observe it.
    std::env::set_var(
        "AERO_PM_TIMER_POLL_QUANTUM",
        POLL_QUANTUM_CYCLES.to_string(),
    );
    std::env::remove_var("AERO_RDTSC_QUANTUM");

    let cfg = MachineConfig {
        ram_size_bytes: 2 * 1024 * 1024,
        enable_pc_platform: true,
        enable_vga: false,
        enable_serial: false,
        enable_i8042: false,
        enable_a20_gate: false,
        enable_reset_ctrl: false,
        enable_e1000: false,
        enable_virtio_net: false,
        ..Default::default()
    };
    let mut machine = Machine::new(cfg).expect("machine");

    let timer_port = DEFAULT_PM_TMR_BLK.to_le_bytes();
    let mut code = Vec::new();
    code.extend_from_slice(&[0x0f, 0x31]); // rdtsc
    append_store_eax(&mut code, TSC0);
    code.extend_from_slice(&[0xba, timer_port[0], timer_port[1]]); // mov dx, PM_TMR
    code.extend_from_slice(&[0x66, 0xed]); // in eax, dx
    append_store_eax(&mut code, PM0);
    code.extend_from_slice(&[0x0f, 0x31]); // rdtsc
    append_store_eax(&mut code, TSC1);
    code.extend_from_slice(&[0xba, timer_port[0], timer_port[1]]); // mov dx, PM_TMR
    code.extend_from_slice(&[0x66, 0xed]); // in eax, dx
    append_store_eax(&mut code, PM1);
    code.extend_from_slice(&[0x0f, 0x31]); // rdtsc
    append_store_eax(&mut code, TSC2);
    code.push(0xf4); // hlt

    machine.write_physical(CODE_BASE, &code);
    setup_real_mode_cpu(&mut machine);

    assert!(matches!(machine.run_slice(64), RunExit::Halted { .. }));

    let tsc0 = machine.read_physical_u32(TSC0);
    let tsc1 = machine.read_physical_u32(TSC1);
    let tsc2 = machine.read_physical_u32(TSC2);
    let first_tsc_delta = tsc1.wrapping_sub(tsc0);
    let second_tsc_delta = tsc2.wrapping_sub(tsc1);
    for delta in [first_tsc_delta, second_tsc_delta] {
        assert!(
            (POLL_QUANTUM_CYCLES..=POLL_QUANTUM_CYCLES + 16).contains(&delta),
            "poll quantum was not reflected in TSC: delta={delta}"
        );
    }

    let pm0 = machine.read_physical_u32(PM0) & PM_TIMER_MASK_24BIT;
    let pm1 = machine.read_physical_u32(PM1) & PM_TIMER_MASK_24BIT;
    let pm_delta = pm1.wrapping_sub(pm0) & PM_TIMER_MASK_24BIT;
    assert!(
        (357..=359).contains(&pm_delta),
        "100us at 3.579545MHz should advance PM_TMR by about 358 ticks, got {pm_delta}"
    );
}
