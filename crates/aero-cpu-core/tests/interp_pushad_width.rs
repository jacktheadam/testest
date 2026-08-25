//! PUSHAD/POPAD operand size must follow the 66-prefix mnemonic, not CS bitness.
//!
//! Windows 7's relocated MBR sits at 0000:0659 on `66 60` (PUSHAD) before every
//! INT 13h extended read. Handling only `Mnemonic::Pusha` and using
//! `state.bitness()` made that encoding #UD in 16-bit real mode, so RIP never
//! left 0x659 and HDD boot could not load the VBR.

use aero_cpu_core::interp::tier0::exec::run_batch;
use aero_cpu_core::mem::{CpuBus, FlatTestBus};
use aero_cpu_core::state::{CpuMode, CpuState};
use aero_x86::Register;

#[test]
fn pushad_66_60_in_16bit_mode_is_32bit_not_ud() {
    // 66 60 = PUSHAD (push EAX..EDI) in 16-bit mode; 66 61 = POPAD.
    let code = [
        0x66, 0x60, // pushad
        0x66, 0x61, // popad
        0xF4, // hlt
    ];
    let mut bus = FlatTestBus::new(0x8000);
    bus.load(0, &code);
    let mut state = CpuState::new(CpuMode::Bit16);
    state.set_rip(0);
    state.write_reg(Register::SS, 0);
    state.write_reg(Register::SP, 0x7C00);
    state.write_reg(Register::EAX, 0xA1A1_A1A1);
    state.write_reg(Register::ECX, 0xC2C2_C2C2);
    state.write_reg(Register::EDX, 0xD3D3_D3D3);
    state.write_reg(Register::EBX, 0xB4B4_B4B4);
    state.write_reg(Register::EBP, 0xE5E5_E5E5);
    state.write_reg(Register::ESI, 0x5656_5656);
    state.write_reg(Register::EDI, 0x7777_7777);

    let r1 = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r1.executed, 1, "66 60 must retire, not #UD");
    assert_eq!(state.rip(), 2);
    // PUSHAD decrements SP by 32, not 16 (PUSHA) and not 0 (#UD restart).
    assert_eq!(state.read_reg(Register::SP), 0x7C00 - 32);

    let mut image = [0u8; 32];
    bus.read_bytes(0x7C00 - 32, &mut image).unwrap();
    // Push order: EAX, ECX, EDX, EBX, original ESP, EBP, ESI, EDI (high addr first).
    assert_eq!(&image[28..32], &0xA1A1_A1A1u32.to_le_bytes());
    assert_eq!(&image[24..28], &0xC2C2_C2C2u32.to_le_bytes());
    assert_eq!(&image[20..24], &0xD3D3_D3D3u32.to_le_bytes());
    assert_eq!(&image[16..20], &0xB4B4_B4B4u32.to_le_bytes());
    assert_eq!(&image[12..16], &0x0000_7C00u32.to_le_bytes());
    assert_eq!(&image[8..12], &0xE5E5_E5E5u32.to_le_bytes());
    assert_eq!(&image[4..8], &0x5656_5656u32.to_le_bytes());
    assert_eq!(&image[0..4], &0x7777_7777u32.to_le_bytes());

    state.write_reg(Register::EAX, 0);
    state.write_reg(Register::EDI, 0);
    let r2 = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r2.executed, 1);
    assert_eq!(state.read_reg(Register::SP), 0x7C00);
    assert_eq!(state.read_reg(Register::EAX), 0xA1A1_A1A1);
    assert_eq!(state.read_reg(Register::EDI), 0x7777_7777);
}

#[test]
fn pusha_without_66_in_16bit_mode_stays_16bit() {
    let code = [0x60, 0xF4];
    let mut bus = FlatTestBus::new(0x2000);
    bus.load(0, &code);
    let mut state = CpuState::new(CpuMode::Bit16);
    state.set_rip(0);
    state.write_reg(Register::SS, 0);
    state.write_reg(Register::SP, 0x1000);
    state.write_reg(Register::AX, 0x1111);

    let r = run_batch(&mut state, &mut bus, 1);
    assert_eq!(r.executed, 1);
    assert_eq!(state.read_reg(Register::SP), 0x1000 - 16);
}
