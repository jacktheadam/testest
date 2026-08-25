//! Bring-up: AeroGPU can report Bochs/QEMU Standard VGA PCI IDs and topology
//! so Win7's inbox `vgapnp.sys`/`framebuf` path can be compared directly with
//! QEMU. PE has no AeroGPU KMD for `A3A0:0001`; its generic `display.inf`
//! binding is class-code based rather than specific to `1234:1111`.
//!
//! Enable with `AERO_AEROGPU_STDVGA_IDS=1`. Under that mode BAR0 becomes a
//! 16 MiB framebuffer (Bochs layout) aliasing the VBE LFB.

use aero_devices::pci::PciBarDefinition;
use aero_machine::{Machine, MachineConfig, RunExit};
use std::sync::Mutex;

/// Env mutation must be serialized across tests in this file (parallel runners race).
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn machine_win7_graphics() -> Machine {
    let mut cfg = MachineConfig {
        ram_size_bytes: 2 * 1024 * 1024,
        enable_pc_platform: true,
        enable_aerogpu: true,
        enable_vga: false,
        enable_serial: false,
        enable_i8042: false,
        enable_a20_gate: false,
        enable_reset_ctrl: false,
        ..Default::default()
    };
    cfg.enable_ahci = false;
    cfg.enable_nvme = false;
    cfg.enable_ide = false;
    cfg.enable_virtio_blk = false;
    cfg.enable_uhci = false;
    cfg.enable_e1000 = false;
    cfg.enable_virtio_net = false;
    Machine::new(cfg).expect("machine")
}

fn vbe_mode_info_boot_sector(mode: u16) -> [u8; aero_storage::SECTOR_SIZE] {
    let mut sector = [0u8; aero_storage::SECTOR_SIZE];
    let [mode_lo, mode_hi] = mode.to_le_bytes();
    let program = [
        0x31, 0xC0, // xor ax,ax
        0x8E, 0xC0, // mov es,ax
        0xBF, 0x00, 0x05, // mov di,0500h
        0xB8, 0x01, 0x4F, // mov ax,4f01h
        0xB9, mode_lo, mode_hi, // mov cx,mode
        0xCD, 0x10, // int 10h
        0xF4, // hlt
    ];
    sector[..program.len()].copy_from_slice(&program);
    sector[510] = 0x55;
    sector[511] = 0xAA;
    sector
}

fn run_until_halt(machine: &mut Machine) {
    for _ in 0..100 {
        match machine.run_slice(10_000) {
            RunExit::Halted { .. } => return,
            RunExit::Completed { .. } => {}
            other => panic!("unexpected machine exit: {other:?}"),
        }
    }
    panic!("guest did not halt");
}

#[test]
fn aerogpu_default_identity_is_a3a0_0001() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Ensure env is off for this process (other tests may set it).
    std::env::remove_var("AERO_AEROGPU_STDVGA_IDS");
    let m = machine_win7_graphics();
    let bdf = m.aerogpu_bdf().expect("AeroGPU present at default IDs");
    let (id, bar0) = {
        let pci_cfg = m.pci_config_ports().expect("pci");
        let mut pci_cfg = pci_cfg.borrow_mut();
        let cfg = pci_cfg.bus_mut().device_config(bdf).unwrap();
        (cfg.vendor_device_id(), cfg.bar_definition(0))
    };
    assert_eq!(id.vendor_id, 0xa3a0);
    assert_eq!(id.device_id, 0x0001);
    assert_eq!(
        bar0,
        Some(PciBarDefinition::Mmio32 {
            size: 64 * 1024,
            prefetchable: false,
        })
    );
}

#[test]
fn aerogpu_stdvga_ids_report_bochs_1234_1111_and_bar0_framebuffer() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("AERO_AEROGPU_STDVGA_IDS", "1");
    let mut m = machine_win7_graphics();
    // Detect via aerogpu_bdf (accepts stdvga-compat IDs).
    let bdf = m
        .aerogpu_bdf()
        .expect("AeroGPU present under stdvga-compat IDs");
    let (id, class, command, bar0, bar1) = {
        let pci_cfg = m.pci_config_ports().expect("pci");
        let mut pci_cfg = pci_cfg.borrow_mut();
        let cfg = pci_cfg.bus_mut().device_config(bdf).unwrap();
        (
            cfg.vendor_device_id(),
            cfg.class_code(),
            cfg.command(),
            cfg.bar_definition(0),
            cfg.bar_definition(1),
        )
    };
    assert_eq!(id.vendor_id, aero_gpu_vga::VGA_PCI_VENDOR_ID);
    assert_eq!(id.device_id, aero_gpu_vga::VGA_PCI_DEVICE_ID);
    assert_eq!(class.class, 0x03);
    assert_eq!(class.subclass, 0x00);
    assert_eq!(
        command & 0x3,
        0x3,
        "VGA-compatible controllers need fixed legacy I/O and memory decode"
    );
    // Bochs Region 0 = 16 MiB framebuffer (inbox miniport primary surface).
    assert_eq!(
        bar0,
        Some(PciBarDefinition::Mmio32 {
            size: 16 * 1024 * 1024,
            prefetchable: true,
        })
    );
    // BAR1 cleared under STDVGA: dual 16+64 MiB framebuffer apertures correlated
    // with session 0xC000021A / STATUS_NO_MEMORY on cold VBE=BAR0.
    assert_eq!(bar1, None);
    // Bochs layout: VBE PhysBasePtr == BAR0 (framebuffer region start), not
    // BAR1+0x40000 interior — Eng system-PTE maps of the interior went NP.
    let bar0_base = m.aerogpu_bar0_base().expect("BAR0 programmed");
    if bar0_base != 0 {
        assert_eq!(
            m.vbe_lfb_base(),
            bar0_base,
            "STDVGA VBE PhysBasePtr must equal BAR0 base"
        );
    }

    // Exercise the interpreter-safe ROM implementation, not only the host
    // HLE state. The ROM is mapped before PCI enumeration, so its 4F01h path
    // must obtain the final BAR-backed PhysBasePtr from writable firmware
    // state rather than freezing the pre-POST default address.
    m.set_disk_image(vbe_mode_info_boot_sector(0x118).to_vec())
        .expect("boot sector");
    m.reset();
    run_until_halt(&mut m);
    assert_eq!(m.read_physical_u16(0x0500 + 18), 1024);
    assert_eq!(m.read_physical_u16(0x0500 + 20), 768);
    assert_eq!(
        u64::from(m.read_physical_u32(0x0500 + 40)),
        m.aerogpu_bar0_base().expect("BAR0 programmed"),
        "ROM VBE mode info must report the post-PCI STDVGA BAR0 base"
    );

    // Old snapshots predate the EBDA runtime field and restore zeros over it.
    // post_restore must republish the final BAR0 value after RAM is applied.
    const VBE_EBDA_LFB_BASE: u64 = 0x0009_F040;
    m.write_physical_u32(VBE_EBDA_LFB_BASE, 0);
    let snapshot = m.take_snapshot_full().expect("snapshot");
    let mut restored = machine_win7_graphics();
    restored
        .restore_snapshot_bytes(&snapshot)
        .expect("restore snapshot");
    assert_eq!(
        u64::from(restored.read_physical_u32(VBE_EBDA_LFB_BASE)),
        restored.aerogpu_bar0_base().expect("restored BAR0"),
        "snapshot restore must republish the ROM VBE runtime LFB after RAM"
    );

    // Clean up for other tests in this process.
    std::env::remove_var("AERO_AEROGPU_STDVGA_IDS");
}

/// Win7 PnP sizes every BAR by writing 0xFFFFFFFF. After POST, STDVGA BAR1
/// must still look unimplemented — otherwise the enumerator maps a second
/// framebuffer aperture (the 0xC000021A / NO_MEMORY interaction).
#[test]
fn stdvga_bar1_stays_unimplemented_through_sizing_probe_and_reset() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("AERO_AEROGPU_STDVGA_IDS", "1");
    let mut m = machine_win7_graphics();
    let bdf = m.aerogpu_bdf().expect("stdvga AeroGPU");

    {
        let pci_cfg = m.pci_config_ports().expect("pci");
        let mut pci_cfg = pci_cfg.borrow_mut();
        let cfg = pci_cfg.bus_mut().device_config_mut(bdf).unwrap();
        assert_eq!(cfg.bar_definition(1), None);
        assert_eq!(cfg.bar_range(1), None);
        cfg.write(0x14, 4, 0xFFFF_FFFF);
        assert_eq!(
            cfg.read(0x14, 4),
            0,
            "POST-assigned STDVGA BAR1 must not size as a live aperture"
        );
        cfg.write(0x14, 4, 0xe000_0000);
        assert_eq!(cfg.bar_range(1), None);
        assert_eq!(
            cfg.bar_range(0).map(|r| r.size),
            Some(16 * 1024 * 1024),
            "BAR0 must remain the 16MiB FB after a BAR1 probe"
        );
    }

    // Platform reset re-runs POST allocation; a stale mapped_bars entry
    // would re-hand BAR1 a window even with the definition cleared.
    m.reset();
    {
        let pci_cfg = m.pci_config_ports().expect("pci");
        let mut pci_cfg = pci_cfg.borrow_mut();
        let cfg = pci_cfg.bus_mut().device_config_mut(bdf).unwrap();
        assert_eq!(cfg.bar_definition(1), None);
        assert_eq!(cfg.bar_range(1), None);
        cfg.write(0x14, 4, 0xFFFF_FFFF);
        assert_eq!(cfg.read(0x14, 4), 0);
    }
    assert_eq!(
        m.vbe_lfb_base(),
        m.aerogpu_bar0_base().expect("BAR0 after reset"),
        "reset must keep PhysBasePtr on BAR0, not a resurrected BAR1 LFB"
    );

    std::env::remove_var("AERO_AEROGPU_STDVGA_IDS");
}

/// Snapshot taken while BAR1 is mid-probe must restore onto a fresh STDVGA
/// machine without turning BAR1 back into a 64MiB decode.
#[test]
fn stdvga_snapshot_restore_does_not_resurrect_bar1_after_probe() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("AERO_AEROGPU_STDVGA_IDS", "1");
    let mut m = machine_win7_graphics();
    let bdf = m.aerogpu_bdf().expect("stdvga AeroGPU");
    {
        let pci_cfg = m.pci_config_ports().expect("pci");
        let mut pci_cfg = pci_cfg.borrow_mut();
        let cfg = pci_cfg.bus_mut().device_config_mut(bdf).unwrap();
        cfg.write(0x14, 4, 0xFFFF_FFFF);
    }
    let snapshot = m.take_snapshot_full().expect("snapshot");
    let mut restored = machine_win7_graphics();
    restored
        .restore_snapshot_bytes(&snapshot)
        .expect("restore");
    let bdf = restored.aerogpu_bdf().expect("restored AeroGPU");
    {
        let pci_cfg = restored.pci_config_ports().expect("pci");
        let mut pci_cfg = pci_cfg.borrow_mut();
        let cfg = pci_cfg.bus_mut().device_config_mut(bdf).unwrap();
        assert_eq!(cfg.bar_definition(1), None);
        assert_eq!(cfg.bar_range(1), None);
        assert_eq!(
            cfg.read(0x14, 4),
            0,
            "restored STDVGA BAR1 must stay unused even if the snap caught a probe"
        );
    }
    std::env::remove_var("AERO_AEROGPU_STDVGA_IDS");
}

/// BAR0 LFB must still decode after BAR1 is gone. A store at the last dword
/// of the 16 MiB aperture must land; a store one dword past it must not
/// alias the framebuffer (old dual-BAR packing put BAR1 next door).
#[test]
fn stdvga_bar0_lfb_hits_last_dword_and_not_the_byte_past_the_aperture() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("AERO_AEROGPU_STDVGA_IDS", "1");
    let mut m = machine_win7_graphics();
    let bar0 = m.aerogpu_bar0_base().expect("BAR0");
    assert_ne!(bar0, 0);
    let bdf = m.aerogpu_bdf().expect("bdf");
    assert_eq!(
        m.pci_bar_base(bdf, aero_devices::pci::profile::AEROGPU_BAR1_VRAM_INDEX),
        None,
        "STDVGA must not assign BAR1"
    );

    let last = bar0 + (16 * 1024 * 1024) - 4;
    let past = bar0 + (16 * 1024 * 1024);
    m.write_physical_u32(last, 0xA1B2_C3D4);
    assert_eq!(
        m.read_physical_u32(last),
        0xA1B2_C3D4,
        "last BAR0 dword must be the LFB alias"
    );

    m.write_physical_u32(past, 0x5566_7788);
    assert_eq!(
        m.read_physical_u32(last),
        0xA1B2_C3D4,
        "store just past BAR0 must not alias the LFB"
    );

    // Command MEM decode off: a store must not land in VRAM. Re-enable and
    // read the LFB — the previous pixel must still be there.
    {
        let pci_cfg = m.pci_config_ports().expect("pci");
        let mut pci_cfg = pci_cfg.borrow_mut();
        pci_cfg.bus_mut().write_config(bdf, 0x04, 2, 0x0001); // I/O only
    }
    m.write_physical_u32(last, 0x1111_2222);
    {
        let pci_cfg = m.pci_config_ports().expect("pci");
        let mut pci_cfg = pci_cfg.borrow_mut();
        pci_cfg.bus_mut().write_config(bdf, 0x04, 2, 0x0003);
    }
    assert_eq!(
        m.read_physical_u32(last),
        0xA1B2_C3D4,
        "MEM decode off must not commit a BAR0 LFB store"
    );

    std::env::remove_var("AERO_AEROGPU_STDVGA_IDS");
}
