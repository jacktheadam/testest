//! Windows 7 MBR HDD boot uses `66 60` PUSHAD around INT 13h AH=42h.
//!
//! A 16-bit-only PUSHA handler left that encoding as #UD, so RIP sat at the
//! relocated MBR's 0x659 forever and the VBR was never loaded. This drives the
//! shipped Machine + BIOS INT 13 EDD path through that exact sequence.

use aero_machine::{Machine, MachineConfig, RunExit};
use aero_storage::{MemBackend, RawDisk, VirtualDisk, SECTOR_SIZE};

fn run_until_halt_or_limit(m: &mut Machine, slices: usize) {
    for _ in 0..slices {
        match m.run_slice(20_000) {
            RunExit::Halted { .. } => return,
            RunExit::Completed { .. } => continue,
            other => panic!("unexpected exit: {other:?}"),
        }
    }
}

fn rel8(from_next: usize, to: usize) -> u8 {
    let diff = i32::try_from(to).unwrap() - i32::try_from(from_next).unwrap();
    assert!(
        (-128..=127).contains(&diff),
        "rel8 out of range from_next={from_next} to={to} diff={diff}"
    );
    diff as i8 as u8
}

/// Windows-shaped MBR: relocate to 0x600 (so the VBR can land at 0x7C00),
/// EDD check, `66 60` PUSHAD, AH=42 of LBA 1, 55AAh, far jump to the VBR.
fn windows_shaped_mbr() -> [u8; SECTOR_SIZE] {
    let mut s = [0u8; SECTOR_SIZE];
    let mut i = 0usize;
    let emit = |s: &mut [u8], i: &mut usize, b: &[u8]| {
        s[*i..*i + b.len()].copy_from_slice(b);
        *i += b.len();
    };

    // xor ax,ax / mov ss,ax / mov sp,7c00 / mov es,ax / mov ds,ax
    emit(&mut s, &mut i, &[0x33, 0xC0]);
    emit(&mut s, &mut i, &[0x8E, 0xD0]);
    emit(&mut s, &mut i, &[0xBC, 0x00, 0x7C]);
    emit(&mut s, &mut i, &[0x8E, 0xC0]);
    emit(&mut s, &mut i, &[0x8E, 0xD8]);
    // mov si,7c00 / mov di,600 / mov cx,200 / cld / rep movsb
    emit(&mut s, &mut i, &[0xBE, 0x00, 0x7C]);
    emit(&mut s, &mut i, &[0xBF, 0x00, 0x06]);
    emit(&mut s, &mut i, &[0xB9, 0x00, 0x02]);
    emit(&mut s, &mut i, &[0xFC, 0xF3, 0xA4]);
    // push ax / push 0x61c / retf  — continue at the relocated copy
    emit(&mut s, &mut i, &[0x50, 0x68, 0x1C, 0x06, 0xCB]);
    assert_eq!(i, 0x1C, "relocated entry must be 0x61C like Windows");

    // sti / mov bp, 0x7be / mov [bp], dl
    emit(&mut s, &mut i, &[0xFB]);
    emit(&mut s, &mut i, &[0xBD, 0xBE, 0x07]);
    emit(&mut s, &mut i, &[0x88, 0x56, 0x00]);
    emit(&mut s, &mut i, &[0xC6, 0x46, 0x11, 0x05]);
    emit(&mut s, &mut i, &[0xC6, 0x46, 0x10, 0x00]);
    emit(&mut s, &mut i, &[0xB4, 0x41, 0xBB, 0xAA, 0x55, 0xCD, 0x13]);

    s[i] = 0x72;
    i += 1;
    let jc_edd = i;
    i += 1;
    emit(&mut s, &mut i, &[0x81, 0xFB, 0x55, 0xAA]);
    s[i] = 0x75;
    i += 1;
    let jne_bx = i;
    i += 1;
    emit(&mut s, &mut i, &[0xF7, 0xC1, 0x01, 0x00]);
    s[i] = 0x74;
    i += 1;
    let jz_cx = i;
    i += 1;

    emit(&mut s, &mut i, &[0xFE, 0x46, 0x10]);
    emit(&mut s, &mut i, &[0x66, 0x60]); // PUSHAD at ~0x659-equivalent
    emit(&mut s, &mut i, &[0x66, 0x68, 0x00, 0x00, 0x00, 0x00]);
    emit(&mut s, &mut i, &[0x66, 0xFF, 0x76, 0x08]);
    emit(&mut s, &mut i, &[0x68, 0x00, 0x00]);
    emit(&mut s, &mut i, &[0x68, 0x00, 0x7C]);
    emit(&mut s, &mut i, &[0x68, 0x01, 0x00]);
    emit(&mut s, &mut i, &[0x68, 0x10, 0x00]);
    emit(&mut s, &mut i, &[0xB4, 0x42]);
    emit(&mut s, &mut i, &[0x8A, 0x56, 0x00]);
    emit(&mut s, &mut i, &[0x8B, 0xF4]);
    emit(&mut s, &mut i, &[0xCD, 0x13]);
    // Preserve CF across the DAP teardown, as the Windows MBR does with lahf/sahf.
    emit(&mut s, &mut i, &[0x9F, 0x83, 0xC4, 0x10, 0x9E]);
    emit(&mut s, &mut i, &[0x66, 0x61]); // POPAD

    s[i] = 0x72;
    i += 1;
    let jc_read = i;
    i += 1;
    emit(&mut s, &mut i, &[0x81, 0x3E, 0xFE, 0x7D, 0x55, 0xAA]);
    s[i] = 0x75;
    i += 1;
    let jne_sig = i;
    i += 1;
    emit(&mut s, &mut i, &[0xEA, 0x00, 0x7C, 0x00, 0x00]);

    let fail = i;
    emit(
        &mut s,
        &mut i,
        &[0xBA, 0xF8, 0x03, 0xB0, b'F', 0xEE, 0xFA, 0xF4],
    );

    s[jc_edd] = rel8(jc_edd + 1, fail);
    s[jne_bx] = rel8(jne_bx + 1, fail);
    s[jz_cx] = rel8(jz_cx + 1, fail);
    s[jc_read] = rel8(jc_read + 1, fail);
    s[jne_sig] = rel8(jne_sig + 1, fail);

    s[0x1BE] = 0x80;
    s[0x1BE + 4] = 0x07;
    s[0x1BE + 8..0x1BE + 12].copy_from_slice(&1u32.to_le_bytes());
    s[0x1BE + 12..0x1BE + 16].copy_from_slice(&1u32.to_le_bytes());
    s[510] = 0x55;
    s[511] = 0xAA;
    s
}

fn marker_vbr() -> [u8; SECTOR_SIZE] {
    let mut sector = [0u8; SECTOR_SIZE];
    sector[..8].copy_from_slice(&[0xFA, 0xBA, 0xF8, 0x03, 0xB0, b'S', 0xEE, 0xF4]);
    sector[510] = 0x55;
    sector[511] = 0xAA;
    sector
}

#[test]
fn windows_mbr_pushad_int13_edd_loads_vbr() {
    let mut disk = RawDisk::create(MemBackend::new(), (2 * SECTOR_SIZE) as u64).unwrap();
    disk.write_sectors(0, &windows_shaped_mbr()).unwrap();
    disk.write_sectors(1, &marker_vbr()).unwrap();

    let mut m = Machine::new(MachineConfig {
        ram_size_bytes: 2 * 1024 * 1024,
        ..Default::default()
    })
    .unwrap();
    m.shared_disk().set_backend(Box::new(disk));
    m.reset();

    run_until_halt_or_limit(&mut m, 200);

    let rip = m.cpu().rip();
    assert_ne!(rip, 0x659, "RIP must leave the Windows MBR PUSHAD site");
    assert_eq!(
        m.take_serial_output(),
        vec![b'S'],
        "VBR must run after PUSHAD + INT 13h AH=42h (got RIP={rip:#x})"
    );
}
