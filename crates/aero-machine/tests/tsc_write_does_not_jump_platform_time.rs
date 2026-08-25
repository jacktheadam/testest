use aero_cpu_core::state::CpuMode;
use aero_devices::acpi_pm::DEFAULT_PM_TMR_BLK;
use aero_devices::clock::Clock;
use aero_machine::{Machine, MachineConfig, RunExit};

const CODE_BASE: u64 = 0x2000;

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

#[test]
fn writing_ia32_tsc_backward_does_not_jump_platform_time() {
    std::env::remove_var("AERO_PM_TIMER_POLL_QUANTUM");
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

    // mov ecx, IA32_TSC; xor eax,eax; xor edx,edx; wrmsr; hlt
    let code = [
        0x66, 0xb9, 0x10, 0x00, 0x00, 0x00, 0x66, 0x31, 0xc0, 0x66, 0x31, 0xd2, 0x0f, 0x30, 0xf4,
    ];
    machine.write_physical(CODE_BASE, &code);
    setup_real_mode_cpu(&mut machine);

    let cpu = machine.cpu_core_mut_by_index(0);
    cpu.time.set_tsc(0x1234_5678_9abc_def0);
    cpu.state.msr.tsc = cpu.time.read_tsc();

    let platform_before = machine.platform_clock().expect("platform clock").now_ns();
    let pm_before = machine.io_read(DEFAULT_PM_TMR_BLK, 4);

    let exit = machine.run_slice(16);
    assert!(matches!(exit, RunExit::Halted { executed: 5 }));

    let platform_after = machine.platform_clock().expect("platform clock").now_ns();
    let pm_after = machine.io_read(DEFAULT_PM_TMR_BLK, 4);

    assert!(
        platform_after.wrapping_sub(platform_before) <= 2,
        "five virtual cycles at 3 GHz must not become a near-2^64 platform-time jump"
    );
    assert_eq!(
        pm_after, pm_before,
        "a backward IA32_TSC write must not wrap or advance PM_TMR"
    );
}
