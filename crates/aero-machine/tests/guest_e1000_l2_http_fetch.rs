//! Guest-path e1000 RX of an HTTP response delivered via the L2 tunnel NET_RX ring.
//!
//! This drives the **shipped** `PcMachine` + `L2TunnelRingBackend` + `E1000Device` path:
//! host pushes an Ethernet frame containing an HTTP/1.1 200 body into the L2 RX ring,
//! `poll_network` pops it into the NIC, and guest RX DMA lands the HTTP bytes in guest RAM.
//!
//! Not a host-only codec unit test: the assertion is on **guest physical memory** after DMA.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use aero_ipc::ring::{PopError, RingBuffer};
use aero_l2_protocol::{encode_frame, L2_TUNNEL_MAGIC, L2_TUNNEL_TYPE_FRAME, L2_TUNNEL_VERSION};
use aero_machine::{PcMachine, PcMachineConfig};
use memory::MemoryBus as _;

/// Build a minimal Ethernet II frame (no IP/TCP) carrying an HTTP response as payload.
/// Win7's real stack would wrap HTTP in TCP/IP; here we prove the guest NIC DMA path for
/// the HTTP bytes that an L2 tunnel would deliver after host-side reassembly.
fn build_http_ethernet_frame(http_body: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + http_body.len());
    // dst MAC (guest)
    frame.extend_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    // src MAC (gateway)
    frame.extend_from_slice(&[0x52, 0x54, 0x00, 0xaa, 0xbb, 0xcc]);
    // ethertype: IPv4 placeholder (payload is raw HTTP for guest-buffer inspection)
    frame.extend_from_slice(&0x0800u16.to_be_bytes());
    frame.extend_from_slice(http_body);
    // pad to 60-byte minimum Ethernet frame if needed
    while frame.len() < 60 {
        frame.push(0);
    }
    frame
}

#[test]
fn guest_e1000_receives_l2_http_200_into_guest_rx_buffer() {
    let tx_ring = Arc::new(RingBuffer::new(32 * 1024));
    let rx_ring = Arc::new(RingBuffer::new(32 * 1024));

    let mut m = PcMachine::new_with_config(PcMachineConfig {
        ram_size_bytes: 4 * 1024 * 1024,
        enable_e1000: true,
        ..Default::default()
    })
    .expect("PcMachine with e1000");

    m.attach_l2_tunnel_rings(tx_ring.clone(), rx_ring.clone());

    let bdf = aero_devices::pci::profile::NIC_E1000_82540EM.bdf;

    // Enable MMIO + bus master.
    {
        let mut pci_cfg = m.platform_mut().pci_cfg.borrow_mut();
        let bus = pci_cfg.bus_mut();
        let cfg = bus
            .device_config_mut(bdf)
            .expect("E1000 present on PCI bus");
        cfg.set_command(0x2 | 0x4);
    }

    let bar0_base = {
        let mut pci_cfg = m.platform_mut().pci_cfg.borrow_mut();
        let bus = pci_cfg.bus_mut();
        bus.device_config(bdf)
            .and_then(|cfg| cfg.bar_range(0))
            .expect("E1000 BAR0")
            .base
    };

    // Guest RX ring + buffers (physical).
    let rx_ring_base = 0x3000u64;
    let rx_buf0 = 0x8000u64;
    let rx_buf1 = 0x9000u64;

    let http = b"HTTP/1.1 200 OK\r\n\
Content-Type: text/html\r\n\
Content-Length: 48\r\n\
\r\n\
<html><body>Aero in-guest L2 fetch OK</body></html>";
    let frame = build_http_ethernet_frame(http);

    // Also prove host L2 tunnel codec can wrap the same HTTP body (tunnel layer).
    let l2_wire = encode_frame(http).expect("encode_frame");
    assert_eq!(l2_wire[0], L2_TUNNEL_MAGIC);
    assert_eq!(l2_wire[1], L2_TUNNEL_VERSION);
    assert_eq!(l2_wire[2], L2_TUNNEL_TYPE_FRAME);

    let mut desc0 = [0u8; 16];
    desc0[0..8].copy_from_slice(&rx_buf0.to_le_bytes());
    let mut desc1 = [0u8; 16];
    desc1[0..8].copy_from_slice(&rx_buf1.to_le_bytes());

    let mem = &mut m.platform_mut().memory;
    mem.write_physical(rx_ring_base, &desc0);
    mem.write_physical(rx_ring_base + 16, &desc1);

    // Program RX.
    mem.write_u32(bar0_base + 0x2800, rx_ring_base as u32); // RDBAL
    mem.write_u32(bar0_base + 0x2804, 0); // RDBAH
    mem.write_u32(bar0_base + 0x2808, 16 * 2); // RDLEN
    mem.write_u32(bar0_base + 0x2810, 0); // RDH
    mem.write_u32(bar0_base + 0x2818, 1); // RDT
    mem.write_u32(bar0_base + 0x0100, 1 << 1); // RCTL.EN

    // Host → guest: push Ethernet frame into L2 NET_RX ring.
    rx_ring.try_push(&frame).expect("push NET_RX");

    // Before poll: guest buffer empty of HTTP.
    let mut before = vec![0u8; http.len()];
    m.platform_mut()
        .memory
        .read_physical(rx_buf0 + 14, &mut before); // skip eth header
    assert!(
        !before.windows(15).any(|w| w == b"HTTP/1.1 200 OK"),
        "guest RX buffer must not contain HTTP before poll"
    );

    m.poll_network();

    let stats = m.network_backend_l2_ring_stats().expect("L2 ring stats");
    assert_eq!(stats.rx_popped_frames, 1);
    assert_eq!(stats.rx_popped_bytes, frame.len() as u64);
    assert_eq!(stats.rx_dropped_oversize, 0);
    assert!(!stats.rx_broken);

    // Guest physical memory now holds the full Ethernet frame with HTTP payload.
    let mut out = vec![0u8; frame.len()];
    m.platform_mut().memory.read_physical(rx_buf0, &mut out);
    assert_eq!(&out[..frame.len().min(out.len())], &frame[..]);

    let payload = &out[14..14 + http.len()];
    assert_eq!(payload, http.as_slice());
    assert!(
        payload.windows(15).any(|w| w == b"HTTP/1.1 200 OK"),
        "guest memory must contain HTTP/1.1 200 after e1000 RX DMA"
    );
    assert!(
        payload
            .windows(b"Aero in-guest L2 fetch OK".len())
            .any(|w| w == b"Aero in-guest L2 fetch OK"),
        "guest memory must contain fetch body"
    );

    let desc0_after = m
        .platform_mut()
        .memory
        .read_physical_u128(rx_ring_base)
        .to_le_bytes();
    let length = u16::from_le_bytes(desc0_after[8..10].try_into().unwrap());
    let status = desc0_after[12];
    assert_eq!(length, frame.len() as u16);
    assert_eq!(status & 0x03, 0x03, "DD|EOP");

    assert_eq!(rx_ring.try_pop(), Err(PopError::Empty));

    // Host tunnel codec frame is independent proof of L2 wire format for the same body.
    assert!(l2_wire.len() > 4);
    assert_eq!(&l2_wire[4..4 + http.len()], http.as_slice());
}
