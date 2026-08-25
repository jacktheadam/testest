//! Guest-session HDA playback on `PcPlatform`: PCI HDA present, guest PCM + BDL in RAM,
//! DMA via controller, non-silent samples into the AudioWorklet ring.

#![cfg(not(target_arch = "wasm32"))]

use aero_audio::mem::MemoryAccess;
use aero_audio::worklet_bridge::InterleavedRingBuffer;
use aero_devices::pci::profile::HDA_ICH6;
use aero_devices::pci::{PCI_CFG_ADDR_PORT, PCI_CFG_DATA_PORT};
use aero_pc_platform::{PcPlatform, PcPlatformConfig};
use memory::MemoryBus as _;

fn cfg_addr(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | (offset as u32 & 0xFC)
}

fn write_cfg_u16(pc: &mut PcPlatform, bus: u8, device: u8, function: u8, offset: u8, value: u16) {
    pc.io.write(
        PCI_CFG_ADDR_PORT,
        4,
        cfg_addr(bus, device, function, offset),
    );
    pc.io.write(PCI_CFG_DATA_PORT, 2, u32::from(value));
}

fn read_cfg_u32(pc: &mut PcPlatform, bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    pc.io.write(
        PCI_CFG_ADDR_PORT,
        4,
        cfg_addr(bus, device, function, offset),
    );
    pc.io.read(PCI_CFG_DATA_PORT, 4)
}

fn read_hda_bar0_base(pc: &mut PcPlatform) -> u64 {
    let bdf = HDA_ICH6.bdf;
    let bar0 = read_cfg_u32(pc, bdf.bus, bdf.device, bdf.function, 0x10);
    u64::from(bar0 & 0xffff_fff0)
}

/// Guest RAM window for HDA DMA, implementing `aero_audio::mem::MemoryAccess`.
struct GuestRamWindow {
    base: u64,
    data: Vec<u8>,
}

impl MemoryAccess for GuestRamWindow {
    fn read_physical(&self, addr: u64, buf: &mut [u8]) {
        let off = (addr - self.base) as usize;
        buf.copy_from_slice(&self.data[off..off + buf.len()]);
    }

    fn write_physical(&mut self, addr: u64, buf: &[u8]) {
        let off = (addr - self.base) as usize;
        self.data[off..off + buf.len()].copy_from_slice(buf);
    }
}

#[test]
fn guest_session_hda_dma_tone_reaches_worklet_ring() {
    let mut pc = PcPlatform::new_with_config(
        4 * 1024 * 1024,
        PcPlatformConfig {
            enable_hda: true,
            ..Default::default()
        },
    );

    let bdf = HDA_ICH6.bdf;
    let bar0 = read_hda_bar0_base(&mut pc);
    assert_ne!(bar0, 0, "HDA BAR0 must be assigned");

    // Memory space + bus master (guest PCI program).
    write_cfg_u16(&mut pc, bdf.bus, bdf.device, bdf.function, 0x04, 0x0006);

    // CRST via MMIO GCTL.
    pc.memory.write_u32(bar0 + 0x08, 0x1);

    let hda = pc.hda.as_ref().expect("HDA device present").clone();

    // Codec setup (stream tag 1, 48 kHz 16-bit stereo).
    {
        let mut dev = hda.borrow_mut();
        let ctl = dev.controller_mut();
        let set_stream_ch = (0x706u32 << 8) | 0x10;
        ctl.codec_mut().execute_verb(2, set_stream_ch);
        let fmt_raw: u16 = (1 << 4) | 0x1;
        let set_fmt = (0x200u32 << 8) | (fmt_raw as u32);
        ctl.codec_mut().execute_verb(2, set_fmt);
    }

    let bdl_base = 0x10000u64;
    let pcm_base = 0x20000u64;
    let frames = 480usize;
    let bpf = 4usize;
    let pcm_len = frames * bpf;

    let freq = 440.0f32;
    let sr = 48_000.0f32;
    for n in 0..frames {
        let t = n as f32 / sr;
        let s = (2.0 * core::f32::consts::PI * freq * t).sin() * 0.5;
        let v = (s * i16::MAX as f32) as i16;
        let off = pcm_base + (n * bpf) as u64;
        pc.memory.write_u16(off, v as u16);
        pc.memory.write_u16(off + 2, v as u16);
    }

    pc.memory.write_u64(bdl_base, pcm_base);
    pc.memory.write_u32(bdl_base + 8, pcm_len as u32);
    pc.memory.write_u32(bdl_base + 12, 1);

    // Platform bus-master tick (proves DMA enable path).
    {
        let mut dev = hda.borrow_mut();
        let ctl = dev.controller_mut();
        let sd = ctl.stream_mut(0);
        sd.bdpl = bdl_base as u32;
        sd.bdpu = 0;
        sd.cbl = pcm_len as u32;
        sd.lvi = 0;
        sd.fmt = (1 << 4) | 0x1;
        sd.ctl = (1 << 1) | (1 << 2) | (1 << 20);
    }
    pc.process_hda(64);

    // Capture into worklet ring using a guest-RAM window as MemoryAccess.
    let window_base = 0x10000u64;
    let window_len = 0x20000usize;
    let mut window = vec![0u8; window_len];
    pc.memory.read_physical(window_base, &mut window);
    let mut guest = GuestRamWindow {
        base: window_base,
        data: window,
    };

    let mut ring = InterleavedRingBuffer::new(512, 2);
    {
        let mut dev = hda.borrow_mut();
        let ctl = dev.controller_mut();
        let sd = ctl.stream_mut(0);
        sd.bdpl = bdl_base as u32;
        sd.bdpu = 0;
        sd.cbl = pcm_len as u32;
        sd.lvi = 0;
        sd.fmt = (1 << 4) | 0x1;
        sd.ctl = (1 << 1) | (1 << 2) | (1 << 20);
        ctl.process_into(&mut guest, 256, &mut ring);
    }

    let mut out = vec![0.0f32; 256 * 2];
    let read = ring.read_interleaved(&mut out);
    assert_eq!(read, 256, "worklet ring should yield 256 frames");

    let peak = out.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(
        peak > 0.001,
        "guest-session HDA samples must be non-silent (peak={peak})"
    );

    // At least some samples should be clearly non-zero beyond numerical noise.
    let nonzero = out.iter().filter(|s| s.abs() > 0.001).count();
    assert!(
        nonzero > 16,
        "expected multiple non-silent samples, got {nonzero}"
    );
}
