use aero_cpu_core::state::CpuMode;
use aero_devices::acpi_pm::DEFAULT_PM_TMR_BLK;
use aero_machine::{Machine, MachineConfig, RunExit};

const CODE_BASE: u64 = 0x2000;
const FIRST_VALUE: u64 = 0x0500;
const SECOND_VALUE: u64 = 0x0504;
const PM_TIMER_MASK_24BIT: u32 = 0x00ff_ffff;

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
fn adjacent_guest_pm_timer_reads_observe_one_shared_clock_instant() {
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
    let first_addr = (FIRST_VALUE as u16).to_le_bytes();
    let second_addr = (SECOND_VALUE as u16).to_le_bytes();
    let code = [
        0xba,
        timer_port[0],
        timer_port[1], // mov dx, PM_TMR
        0x66,
        0xed, // in eax, dx
        0x66,
        0xa3,
        first_addr[0],
        first_addr[1], // mov [FIRST_VALUE], eax
        0x66,
        0xed, // in eax, dx
        0x66,
        0xa3,
        second_addr[0],
        second_addr[1], // mov [SECOND_VALUE], eax
        0xf4,           // hlt
    ];
    machine.write_physical(CODE_BASE, &code);
    machine.write_physical_u32(FIRST_VALUE, u32::MAX);
    machine.write_physical_u32(SECOND_VALUE, u32::MAX);
    setup_real_mode_cpu(&mut machine);

    assert!(matches!(
        machine.run_slice(32),
        RunExit::Halted { executed: 6 }
    ));

    let first = machine.read_physical_u32(FIRST_VALUE) & PM_TIMER_MASK_24BIT;
    let second = machine.read_physical_u32(SECOND_VALUE) & PM_TIMER_MASK_24BIT;
    assert_eq!(
        second, first,
        "reading PM_TMR must sample the shared platform clock, not advance a private timer"
    );
}
