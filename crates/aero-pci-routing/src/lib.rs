#![no_std]

//! Canonical PCI INTx swizzle + PIRQ -> GSI routing helpers.
//!
//! Aero models PCI INTx routing using the conventional deterministic swizzle used by QEMU and
//! PC-compatible firmware:
//!
//! ```text
//! pirq = (device + pin_index) & 3
//! ```
//!
//! where:
//! - `device` is the PCI slot device number (0-31)
//! - `pin_index` is 0..3 for INTA..INTD
//!
//! The resulting PIRQ index is then mapped to a platform Global System Interrupt (GSI) via the
//! caller-provided `pirq_to_gsi` table.

/// Default Q35 APIC-mode PIRQ routing for Aero's root-bus device window.
///
/// QEMU's ICH9 maps the PIRQ E-H group used by root-bus slots 0-23 to dedicated IOAPIC GSIs
/// 20-23. Keeping PCI INTx out of the ISA range is required: in particular, GSI12 belongs to the
/// legacy PS/2 mouse and is advertised as edge-triggered, active-high, and exclusive.
pub const DEFAULT_PIRQ_TO_GSI: [u32; 4] = [20, 21, 22, 23];

/// Translate Aero's Q35 APIC PCI window to its compatibility PIC routing.
///
/// The legacy view is used only while the platform is in PIC mode. ACPI exposes the dedicated
/// GSI identity to an APIC-aware OS, so these compatibility IRQs do not participate in Windows'
/// APIC resource assignment.
#[inline]
pub const fn q35_legacy_pic_irq_for_gsi(gsi: u32) -> Option<u8> {
    if gsi >= 20 && gsi <= 23 {
        Some((gsi - 20) as u8 + 10)
    } else {
        None
    }
}

/// Computes the PIRQ index (0 = A, 1 = B, 2 = C, 3 = D) for a device/pin pair.
#[inline]
pub const fn pirq_index(device: u8, pin_index: u8) -> u8 {
    device.wrapping_add(pin_index) & 0x03
}

/// Returns the routed GSI for a device/pin pair.
#[inline]
pub const fn gsi_for_intx(pirq_to_gsi: [u32; 4], device: u8, pin_index: u8) -> u32 {
    pirq_to_gsi[pirq_index(device, pin_index) as usize]
}

/// Returns the value to program into the PCI config-space `Interrupt Line` register (0x3C).
///
/// - `interrupt_pin_cfg` is the PCI config-space `Interrupt Pin` register encoding:
///   0 = no interrupt pin, 1 = INTA#, 2 = INTB#, 3 = INTC#, 4 = INTD#.
/// - When `interrupt_pin_cfg == 0`, the conventional "unknown/unconnected" value 0xFF is returned.
#[inline]
pub fn irq_line_for_intx(pirq_to_gsi: [u32; 4], device: u8, interrupt_pin_cfg: u8) -> u8 {
    if interrupt_pin_cfg == 0 {
        return 0xFF;
    }
    let pin_index = interrupt_pin_cfg.wrapping_sub(1) & 0x03;
    let gsi = gsi_for_intx(pirq_to_gsi, device, pin_index);
    u8::try_from(gsi).unwrap_or(0xFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pirq_swizzle_matches_qemu_style_policy() {
        // Device 0: no swizzle.
        assert_eq!(pirq_index(0, 0), 0);
        assert_eq!(pirq_index(0, 1), 1);
        assert_eq!(pirq_index(0, 2), 2);
        assert_eq!(pirq_index(0, 3), 3);

        // Device 1: swizzled by one.
        assert_eq!(pirq_index(1, 0), 1);
        assert_eq!(pirq_index(1, 1), 2);
        assert_eq!(pirq_index(1, 2), 3);
        assert_eq!(pirq_index(1, 3), 0);

        // Device 4 wraps around.
        assert_eq!(pirq_index(4, 0), 0);
    }

    #[test]
    fn gsi_and_irq_line_helpers_follow_pirq_map() {
        let map = DEFAULT_PIRQ_TO_GSI;

        assert_eq!(gsi_for_intx(map, 0, 0), 20);
        assert_eq!(gsi_for_intx(map, 1, 0), 21);
        assert_eq!(gsi_for_intx(map, 2, 3), 21);

        // interrupt_pin_cfg: 1=INTA, 4=INTD.
        assert_eq!(irq_line_for_intx(map, 1, 1), 21);
        assert_eq!(irq_line_for_intx(map, 2, 4), 21);

        // interrupt_pin_cfg=0 => 0xFF sentinel.
        assert_eq!(irq_line_for_intx(map, 2, 0), 0xFF);
    }

    #[test]
    fn default_q35_routes_use_dedicated_apic_gsis() {
        assert_eq!(DEFAULT_PIRQ_TO_GSI, [20, 21, 22, 23]);
        assert!(
            DEFAULT_PIRQ_TO_GSI.iter().all(|gsi| !matches!(gsi, 1 | 12)),
            "PCI INTx must not overlap the fixed PS/2 keyboard/mouse IRQs"
        );
    }

    #[test]
    fn q35_apic_window_has_a_legacy_pic_compatibility_view() {
        assert_eq!(q35_legacy_pic_irq_for_gsi(20), Some(10));
        assert_eq!(q35_legacy_pic_irq_for_gsi(21), Some(11));
        assert_eq!(q35_legacy_pic_irq_for_gsi(22), Some(12));
        assert_eq!(q35_legacy_pic_irq_for_gsi(23), Some(13));
        assert_eq!(q35_legacy_pic_irq_for_gsi(12), None);
        assert_eq!(q35_legacy_pic_irq_for_gsi(24), None);
    }
}
