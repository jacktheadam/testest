use aero_devices_input::I8042Controller;

#[test]
fn controller_without_a_keylock_reports_keyboard_unlocked() {
    let mut controller = I8042Controller::new();

    assert_ne!(
        controller.read_port(0x64) & 0x10,
        0,
        "an i8042 without a modeled keylock must report the keyboard as unlocked"
    );
}

#[test]
fn keyboard_device_write_reenables_disabled_port_and_exposes_ack() {
    let mut controller = I8042Controller::new();

    controller.write_port(0x64, 0xAD);
    controller.write_port(0x60, 0xF4);

    assert_ne!(
        controller.read_port(0x64) & 0x01,
        0,
        "the keyboard ACK must reach the output buffer"
    );
    assert_eq!(controller.read_port(0x60), 0xFA);
}

#[test]
fn mouse_device_write_reenables_disabled_port_and_exposes_aux_ack() {
    let mut controller = I8042Controller::new();

    controller.write_port(0x64, 0xA7);
    controller.write_port(0x64, 0xD4);
    controller.write_port(0x60, 0xF4);

    let status = controller.read_port(0x64);
    assert_ne!(
        status & 0x01,
        0,
        "the mouse ACK must reach the output buffer"
    );
    assert_ne!(status & 0x20, 0, "the ACK must be marked as AUX data");
    assert_eq!(controller.read_port(0x60), 0xFA);
}
