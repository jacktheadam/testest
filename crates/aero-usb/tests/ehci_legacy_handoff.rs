use aero_usb::ehci::regs::*;
use aero_usb::ehci::EhciController;

#[test]
fn ehci_legacy_handoff_bios_to_os() {
    let mut ehci = EhciController::new();

    // HCCPARAMS no longer advertises EECP (set to 0) because the USBLEGSUP
    // capability is served from MMIO, not PCI config space. Win7's usbehci.sys
    // reads extended caps from PCI config space, so advertising a non-zero EECP
    // would make it find a list terminator at config 0x40 and skip the handoff.
    let hccparams = ehci.mmio_read(REG_HCCPARAMS, 4);
    let eecp = ((hccparams >> HCCPARAMS_EECP_SHIFT) & 0xff) as u64;
    assert_eq!(eecp, 0, "EECP should be 0 (USBLEGSUP is MMIO-only)");

    // The USBLEGSUP MMIO register at offset 0x40 still functions for any driver
    // that accesses it directly via MMIO.
    let usblegsup = ehci.mmio_read(REG_USBLEGSUP, 4);
    assert_eq!(usblegsup & 0xff, USBLEGSUP_CAPID);
    assert_ne!(
        usblegsup & USBLEGSUP_BIOS_SEM,
        0,
        "BIOS should own EHCI on reset"
    );
    assert_eq!(
        usblegsup & USBLEGSUP_OS_SEM,
        0,
        "OS should not own EHCI on reset"
    );

    // Guest requests ownership.
    ehci.mmio_write(REG_USBLEGSUP, 4, USBLEGSUP_OS_SEM);

    let usblegsup = ehci.mmio_read(REG_USBLEGSUP, 4);
    assert_ne!(usblegsup & USBLEGSUP_OS_SEM, 0);
    assert_eq!(
        usblegsup & USBLEGSUP_BIOS_SEM,
        0,
        "BIOS-owned semaphore should clear once OS-owned is set"
    );
}
