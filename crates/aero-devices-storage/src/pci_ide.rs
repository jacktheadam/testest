//! PCI PIIX3-compatible IDE controller (legacy ports + Bus Master IDE DMA).
//!
//! This device model is designed for compatibility with:
//! - BIOS-era software using legacy IDE ports (0x1F0/0x3F6 and 0x170/0x376)
//! - Windows 7 `intelide.sys` / `atapi.sys` in PIIX3 legacy mode (prog-if 0x80,
//!   BMIDE BAR4 only; command blocks are hardwired, not PCI BARs)

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use aero_devices::irq::IrqLine;
use aero_devices::pci::profile::IDE_PIIX3;
use aero_devices::pci::{PciBarDefinition, PciConfigSpace, PciDevice};
use aero_io_snapshot::io::state::{IoSnapshot, SnapshotResult, SnapshotVersion};
use aero_io_snapshot::io::storage::state::{
    IdeAtaDeviceState, IdeChannelState, IdeControllerState, IdeDataMode, IdeDmaCommitState,
    IdeDmaDirection, IdeDmaRequestState, IdeDriveState, IdePioWriteState, IdePortMapState,
    IdeTaskFileState, IdeTransferKind, PciConfigSpaceState, MAX_IDE_DATA_BUFFER_BYTES,
};
use aero_platform::io::{IoPortBus, PortIoDevice};
use aero_storage::SECTOR_SIZE;
use memory::MemoryBus;

use crate::ata::AtaDrive;
use crate::atapi::{AtapiCdrom, IsoBackend, PacketResult};
use crate::busmaster::{BusMasterChannel, DmaCommit, DmaRequest};

const IDE_STATUS_BSY: u8 = 0x80;
const IDE_STATUS_DRDY: u8 = 0x40;
const IDE_STATUS_DSC: u8 = 0x10; // SEEK_STAT / Drive Seek Complete
const IDE_STATUS_DRQ: u8 = 0x08;
const IDE_STATUS_ERR: u8 = 0x01;

const IDE_CTRL_NIEN: u8 = 0x02;
const IDE_CTRL_SRST: u8 = 0x04;
const IDE_CTRL_HOB: u8 = 0x80;

const ATA_REG_DATA: u16 = 0;
const ATA_REG_ERROR_FEATURES: u16 = 1;
const ATA_REG_SECTOR_COUNT: u16 = 2;
const ATA_REG_LBA0: u16 = 3;
const ATA_REG_LBA1: u16 = 4;
const ATA_REG_LBA2: u16 = 5;
const ATA_REG_DEVICE: u16 = 6;
const ATA_REG_STATUS_COMMAND: u16 = 7;

const ATA_CTRL_ALT_STATUS_DEVICE_CTRL: u16 = 0;
const ATA_CTRL_DRIVE_ADDRESS: u16 = 1;

/// Legacy primary/secondary I/O port assignments.
#[derive(Debug, Clone, Copy)]
pub struct IdePortMap {
    pub cmd_base: u16,
    pub ctrl_base: u16,
}

pub const PRIMARY_PORTS: IdePortMap = IdePortMap {
    cmd_base: 0x1F0,
    ctrl_base: 0x3F6,
};

pub const SECONDARY_PORTS: IdePortMap = IdePortMap {
    cmd_base: 0x170,
    ctrl_base: 0x376,
};

fn ide_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("AERO_IDE_TRACE").is_some())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriveSelect {
    Master = 0,
    Slave = 1,
}

impl DriveSelect {
    fn from_device_reg(val: u8) -> Self {
        if (val & 0x10) != 0 {
            DriveSelect::Slave
        } else {
            DriveSelect::Master
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataMode {
    None,
    /// Device -> host (PIO IN) via data register.
    PioIn,
    /// Host -> device (PIO OUT) via data register.
    PioOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferKind {
    AtaPioRead,
    AtaPioWrite,
    Identify,
    AtapiPacket,
    AtapiPioIn,
}

#[derive(Debug, Clone, Default)]
struct TaskFile {
    features: u8,
    sector_count: u8,
    lba0: u8,
    lba1: u8,
    lba2: u8,
    device: u8,

    hob_features: u8,
    hob_sector_count: u8,
    hob_lba0: u8,
    hob_lba1: u8,
    hob_lba2: u8,

    pending_features_high: bool,
    pending_sector_count_high: bool,
    pending_lba0_high: bool,
    pending_lba1_high: bool,
    pending_lba2_high: bool,
}

impl TaskFile {
    fn read_reg(&self, reg: u16, hob: bool) -> u8 {
        match reg {
            ATA_REG_ERROR_FEATURES => {
                if hob {
                    self.hob_features
                } else {
                    self.features
                }
            }
            ATA_REG_SECTOR_COUNT => {
                if hob {
                    self.hob_sector_count
                } else {
                    self.sector_count
                }
            }
            ATA_REG_LBA0 => {
                if hob {
                    self.hob_lba0
                } else {
                    self.lba0
                }
            }
            ATA_REG_LBA1 => {
                if hob {
                    self.hob_lba1
                } else {
                    self.lba1
                }
            }
            ATA_REG_LBA2 => {
                if hob {
                    self.hob_lba2
                } else {
                    self.lba2
                }
            }
            ATA_REG_DEVICE => self.device,
            _ => 0,
        }
    }

    fn write_reg(&mut self, reg: u16, val: u8) {
        // QEMU/ATA: every write updates the visible register and the previous
        // value becomes the HOB. A single write is immediately readable (Win7
        // ataport reads back the ATAPI byte-count limit). A second write of the
        // same register (LBA48) leaves low=new, hob=previous.
        match reg {
            ATA_REG_ERROR_FEATURES => {
                self.hob_features = self.features;
                self.features = val;
                self.pending_features_high = false;
            }
            ATA_REG_SECTOR_COUNT => {
                self.hob_sector_count = self.sector_count;
                self.sector_count = val;
                self.pending_sector_count_high = false;
            }
            ATA_REG_LBA0 => {
                self.hob_lba0 = self.lba0;
                self.lba0 = val;
                self.pending_lba0_high = false;
            }
            ATA_REG_LBA1 => {
                self.hob_lba1 = self.lba1;
                self.lba1 = val;
                self.pending_lba1_high = false;
            }
            ATA_REG_LBA2 => {
                self.hob_lba2 = self.lba2;
                self.lba2 = val;
                self.pending_lba2_high = false;
            }
            ATA_REG_DEVICE => {
                self.device = val;
            }
            _ => {}
        }
    }

    fn normalize_for_command(&mut self, is_lba48: bool) {
        // Old snapshots could still have the pre-QEMU "first write lives in HOB"
        // pending flags set mid-command. Apply them once so restore stays coherent.
        if self.pending_features_high {
            self.features = self.hob_features;
            self.pending_features_high = false;
        }
        if self.pending_sector_count_high {
            self.sector_count = self.hob_sector_count;
            self.pending_sector_count_high = false;
        }
        if self.pending_lba0_high {
            self.lba0 = self.hob_lba0;
            self.pending_lba0_high = false;
        }
        if self.pending_lba1_high {
            self.lba1 = self.hob_lba1;
            self.pending_lba1_high = false;
        }
        if self.pending_lba2_high {
            self.lba2 = self.hob_lba2;
            self.pending_lba2_high = false;
        }

        if !is_lba48 {
            self.hob_features = 0;
            self.hob_sector_count = 0;
            self.hob_lba0 = 0;
            self.hob_lba1 = 0;
            self.hob_lba2 = 0;
        }
    }

    fn lba28(&self) -> u64 {
        let high4 = (self.device & 0x0F) as u64;
        (high4 << 24) | (self.lba2 as u64) << 16 | (self.lba1 as u64) << 8 | self.lba0 as u64
    }

    fn lba48(&self) -> u64 {
        (self.hob_lba2 as u64) << 40
            | (self.hob_lba1 as u64) << 32
            | (self.hob_lba0 as u64) << 24
            | (self.lba2 as u64) << 16
            | (self.lba1 as u64) << 8
            | self.lba0 as u64
    }

    fn sector_count28(&self) -> u16 {
        let c = self.sector_count as u16;
        if c == 0 {
            256
        } else {
            c
        }
    }

    fn sector_count48(&self) -> u32 {
        let c = ((self.hob_sector_count as u32) << 8) | self.sector_count as u32;
        if c == 0 {
            65536
        } else {
            c
        }
    }
}

enum IdeDevice {
    Ata(Box<AtaDrive>),
    Atapi(AtapiCdrom),
}

struct Channel {
    ports: IdePortMap,
    devices: [Option<IdeDevice>; 2],
    /// Guest-visible drive presence for each device slot.
    ///
    /// This is intentionally tracked separately from `devices`: during snapshot restore we drop
    /// host-side backends (e.g. `AtaDrive`) but still need the controller to behave as though the
    /// drive exists from the guest's perspective (taskfile/status reads, IRQ acking, etc).
    drive_present: [bool; 2],
    /// Guest-visible ATA device state that must survive snapshot restore even when the host-side
    /// disk backend is detached.
    ///
    /// This is particularly important for negotiated DMA/UDMA transfer modes set via
    /// `SET FEATURES (0xEF) / subcommand 0x03`.
    ata_state: [Option<IdeAtaDeviceState>; 2],

    tf: TaskFile,
    status: u8,
    error: u8,
    control: u8,

    data_mode: DataMode,
    transfer_kind: Option<TransferKind>,
    data: Vec<u8>,
    data_index: usize,

    irq_pending: bool,
    /// Optional pin driven immediately when `irq_pending` / nIEN changes.
    ///
    /// Win7 `ataport` ATAPI PIO is a two-IRQ protocol: data-in, then status.
    /// The status-phase edge is raised by the last data-port read *inside the
    /// same Tier-0 I/O batch* that acked the data-in IRQ. If we only push the
    /// pin at batch-boundary `poll_pci_intx_lines`, the PIC/IOAPIC never sees
    /// the 1→0→1 transition and the status IRQ is lost (DiscoverLuns INQUIRY
    /// times out; no `IDE\CdRom`). The older `ide.rs` model already drove the
    /// pin from `set_irq`. QEMU `ide_set_irq` does the same.
    irq_line: Option<Box<dyn IrqLine>>,

    // DMA requested by the currently-selected device and waiting for the Bus
    // Master engine to be started.
    pending_dma: Option<DmaRequest>,

    // Latched parameters for an in-flight ATA PIO write command.
    pio_write: Option<(u64, u64)>,
}

impl Channel {
    fn new(ports: IdePortMap) -> Self {
        Self {
            ports,
            devices: [None, None],
            drive_present: [false, false],
            ata_state: [None, None],
            tf: TaskFile::default(),
            status: IDE_STATUS_DRDY,
            error: 0,
            control: 0,
            data_mode: DataMode::None,
            transfer_kind: None,
            data: Vec::new(),
            data_index: 0,
            irq_pending: false,
            irq_line: None,
            pending_dma: None,
            pio_write: None,
        }
    }

    fn drive_address(&self) -> u8 {
        // ATA Drive Address (DADR) register at Control Block base + 1.
        //
        // The ATA/ATAPI spec defines DADR as the active-low versions of the drive/head select
        // lines:
        //   bits 7..6: 1
        //   bit 5: nDS1 (active low drive-select 1)
        //   bit 4: nDS0 (active low drive-select 0)
        //   bits 3..0: nHS3..nHS0 (active low head-select)
        //
        // Bus-high behavior (0xFF) for an absent selected device is handled by `read_ctrl_reg`
        // before this method is reached.
        let head = self.tf.device & 0x0F;
        let dev = self.selected_drive() as u8; // 0=master, 1=slave
        let n_ds0 = dev;
        let n_ds1 = dev ^ 1;
        0xC0 | (n_ds1 << 5) | (n_ds0 << 4) | ((!head) & 0x0F)
    }

    fn selected_drive(&self) -> DriveSelect {
        DriveSelect::from_device_reg(self.tf.device)
    }

    fn irq_pin_level(&self) -> bool {
        self.irq_pending && (self.control & IDE_CTRL_NIEN) == 0
    }

    fn sync_irq_pin(&self) {
        if let Some(line) = &self.irq_line {
            line.set_level(self.irq_pin_level());
        }
    }

    fn set_irq(&mut self) {
        // `nIEN` (bit 1 of the Device Control register) masks the interrupt *output*, but the
        // interrupt condition itself is still latched until the guest acknowledges it (typically
        // by reading the Status register).
        //
        // This matches how the simpler legacy IDE model in `src/ide.rs` behaves and avoids losing
        // interrupts if the guest temporarily disables them while polling.
        self.irq_pending = true;
        self.sync_irq_pin();
        if ide_trace_enabled() {
            let which = if self.ports.cmd_base == SECONDARY_PORTS.cmd_base {
                "sec"
            } else {
                "pri"
            };
            eprintln!(
                "AERO_IDE_TRACE: {which} irq set status={:#04x} ireason={:#04x} err={:#04x} nien={}",
                self.status,
                self.tf.sector_count,
                self.error,
                (self.control & IDE_CTRL_NIEN) != 0
            );
        }
    }

    fn clear_irq(&mut self) {
        self.irq_pending = false;
        self.sync_irq_pin();
    }

    /// QEMU `nsector = (nsector & ~7) | reason`: ATAPI interrupt reason lives in
    /// the low 3 bits; the rest of the Sector Count register is preserved.
    fn set_atapi_ireason(&mut self, reason: u8) {
        self.tf.sector_count = (self.tf.sector_count & !0x07) | (reason & 0x07);
    }

    fn reset(&mut self) {
        self.tf = TaskFile::default();
        self.status = IDE_STATUS_DRDY;
        self.error = 0;
        self.data_mode = DataMode::None;
        self.transfer_kind = None;
        self.data.clear();
        self.data_index = 0;
        self.pending_dma = None;
        self.pio_write = None;
        self.clear_irq();

        // Per ATA/ATAPI-7: after SRST deassertion, the selected device must
        // present its signature in the task file so the host can classify it:
        //   ATAPI CD-ROM: sector_count=0x01, lba_low=0x01, lba_mid=0x14, lba_high=0xEB
        //   ATA disk:     sector_count=0x01, lba_low=0x01, lba_mid=0x00, lba_high=0x00
        // Without this, Win7's atapi.sys/pciide.sys can't detect the CD-ROM
        // via SRST-based enumeration.
        self.tf.sector_count = 0x01;
        self.tf.lba0 = 0x01;
        if matches!(self.devices[0], Some(IdeDevice::Atapi(_))) {
            self.tf.lba1 = 0x14;
            self.tf.lba2 = 0xEB;
        }
    }

    fn apply_device_signature(&mut self, dev_idx: usize) {
        self.tf.sector_count = 0x01;
        self.tf.lba0 = 0x01;
        match self.devices[dev_idx].as_ref() {
            Some(IdeDevice::Atapi(_)) => {
                self.tf.lba1 = 0x14;
                self.tf.lba2 = 0xEB;
            }
            Some(IdeDevice::Ata(_)) => {
                self.tf.lba1 = 0x00;
                self.tf.lba2 = 0x00;
            }
            None => {
                self.tf.lba1 = 0x00;
                self.tf.lba2 = 0x00;
            }
        }
    }

    fn set_error(&mut self, err: u8) {
        self.error = err;
        self.status |= IDE_STATUS_ERR;
    }

    fn clear_error(&mut self) {
        self.error = 0;
        self.status &= !IDE_STATUS_ERR;
    }

    fn abort_command(&mut self, err: u8) {
        self.data_mode = DataMode::None;
        self.transfer_kind = None;
        self.data.clear();
        self.data_index = 0;
        self.pending_dma = None;
        self.pio_write = None;

        self.set_error(err);
        self.status &= !(IDE_STATUS_BSY | IDE_STATUS_DRQ);
        self.status |= IDE_STATUS_DRDY | IDE_STATUS_DSC;
        self.set_irq();
    }

    fn begin_pio_in(&mut self, kind: TransferKind, data: Vec<u8>) {
        self.data = data;
        self.data_index = 0;
        self.data_mode = DataMode::PioIn;
        self.transfer_kind = Some(kind);
        self.status &= !IDE_STATUS_BSY;
        self.status |= IDE_STATUS_DRQ | IDE_STATUS_DRDY | IDE_STATUS_DSC;
        self.clear_error();
        self.set_irq();
    }

    fn begin_pio_out(&mut self, kind: TransferKind, len: usize) {
        let Some(data) = try_alloc_zeroed(len) else {
            self.abort_command(0x04);
            return;
        };
        self.data = data;
        self.data_index = 0;
        self.data_mode = DataMode::PioOut;
        self.transfer_kind = Some(kind);
        self.status &= !IDE_STATUS_BSY;
        self.status |= IDE_STATUS_DRQ | IDE_STATUS_DRDY | IDE_STATUS_DSC;
        self.clear_error();
        // QEMU `cmd_packet` sets DRQ for the 12-byte CDB but does *not* raise
        // INTRQ. Win7 ataport polls that phase; an extra IRQ looks like a
        // completion and the first INQUIRY times out (~400M inst).
        if kind != TransferKind::AtapiPacket {
            self.set_irq();
        }
    }

    fn complete_non_data_command(&mut self) {
        self.data_mode = DataMode::None;
        self.transfer_kind = None;
        self.data.clear();
        self.data_index = 0;
        self.status &= !(IDE_STATUS_BSY | IDE_STATUS_DRQ);
        self.status |= IDE_STATUS_DRDY | IDE_STATUS_DSC;
        self.clear_error();
        self.set_irq();
    }

    fn data_in_u16(&mut self) -> u16 {
        if self.data_index >= self.data.len() {
            self.finish_data_phase();
            return 0;
        }
        let lo = self.data[self.data_index];
        let hi = self.data.get(self.data_index + 1).copied().unwrap_or(0);
        self.data_index += 2;
        if self.data_index >= self.data.len() {
            self.finish_data_phase();
        }
        u16::from_le_bytes([lo, hi])
    }

    fn data_in_u8(&mut self) -> u8 {
        if self.data_index >= self.data.len() {
            self.finish_data_phase();
            return 0;
        }
        let b = self.data[self.data_index];
        self.data_index += 1;
        if self.data_index >= self.data.len() {
            self.finish_data_phase();
        }
        b
    }

    fn data_in_u32(&mut self) -> u32 {
        let lo = self.data_in_u16() as u32;
        let hi = self.data_in_u16() as u32;
        lo | (hi << 16)
    }

    fn data_out_u8(&mut self, val: u8) {
        if self.data_index >= self.data.len() {
            // Ignore overflow.
            return;
        }
        self.data[self.data_index] = val;
        self.data_index += 1;
        if self.data_index >= self.data.len() {
            self.finish_data_phase();
        }
    }

    fn data_out_u16(&mut self, val: u16) {
        if self.data_index + 2 > self.data.len() {
            // Ignore overflow.
            return;
        }
        let b = val.to_le_bytes();
        self.data[self.data_index] = b[0];
        self.data[self.data_index + 1] = b[1];
        self.data_index += 2;
        if self.data_index >= self.data.len() {
            self.finish_data_phase();
        }
    }

    fn data_out_u32(&mut self, val: u32) {
        self.data_out_u16((val & 0xFFFF) as u16);
        self.data_out_u16((val >> 16) as u16);
    }

    fn finish_data_phase(&mut self) {
        match self.transfer_kind {
            Some(TransferKind::AtaPioWrite) => {
                let (lba, sectors) = self
                    .pio_write
                    .take()
                    .unwrap_or_else(|| (self.tf.lba28(), self.tf.sector_count28() as u64));
                let data = std::mem::take(&mut self.data);
                let idx = self.selected_drive() as usize;
                let ok = match self.devices[idx].as_mut() {
                    Some(IdeDevice::Ata(dev)) => ata_pio_write(dev, lba, sectors, &data).is_ok(),
                    _ => false,
                };
                if ok {
                    self.complete_non_data_command();
                } else {
                    self.abort_command(0x04);
                }
            }
            Some(TransferKind::AtapiPioIn) => {
                // Data phase complete; transition to status phase.
                // QEMU `ide_atapi_cmd_ok`: IO=1, CoD=1, preserve nsector[7:3].
                self.set_atapi_ireason(0x03);
                self.complete_non_data_command();
            }
            Some(TransferKind::AtapiPacket) => {
                let mut packet = [0u8; 12];
                packet.copy_from_slice(&self.data[..12]);
                let dma_requested = (self.tf.features & 0x01) != 0;
                let bcl = u16::from(self.tf.lba1) | (u16::from(self.tf.lba2) << 8);
                if ide_trace_enabled() {
                    eprintln!(
                        "AERO_IDE_TRACE: packet op={:#04x} {:02x}{:02x}{:02x}{:02x} dma={} bcl={bcl:#06x}",
                        packet[0], packet[1], packet[2], packet[3], packet[4],
                        dma_requested as u8
                    );
                }
                // QEMU `validate_bcl`: PIO data commands with a 0 byte-count
                // limit abort at the ATA layer. Win7's first INQUIRY is sent
                // with BCL=0; accepting it leaves DRQ set and the miniport
                // times out (~400M inst) instead of creating a CdRom PDO.
                if !dma_requested && bcl == 0 && atapi_packet_transfers_data(packet[0]) {
                    self.abort_command(0x04);
                    return;
                }
                let idx = self.selected_drive() as usize;
                let result = match self.devices[idx].as_mut() {
                    Some(IdeDevice::Atapi(dev)) => dev.handle_packet(&packet, dma_requested),
                    _ => {
                        self.abort_command(0x04);
                        return;
                    }
                };

                match result {
                    PacketResult::DataIn(mut buf) => {
                        // ATAPI uses sector_count as interrupt reason; IO=1, CoD=0.
                        // QEMU `ide_atapi_cmd_reply_end` preserves nsector[7:3].
                        self.set_atapi_ireason(0x02);
                        if bcl != 0 && buf.len() > bcl as usize {
                            buf.truncate(if bcl == 0xffff { 0xfffe } else { bcl } as usize);
                        }
                        let mut byte_count = buf.len().min(u16::MAX as usize) as u16;
                        if byte_count & 1 != 0 {
                            byte_count = byte_count.saturating_add(1);
                        }
                        self.tf.lba1 = (byte_count & 0xFF) as u8;
                        self.tf.lba2 = (byte_count >> 8) as u8;
                        if ide_trace_enabled() {
                            eprintln!(
                                "AERO_IDE_TRACE: pio_in len={} bcl={bcl:#06x} ireason={:#04x}",
                                buf.len(),
                                self.tf.sector_count
                            );
                        }
                        self.begin_pio_in(TransferKind::AtapiPioIn, buf);
                    }
                    PacketResult::NoDataSuccess => {
                        self.set_atapi_ireason(0x03); // IO=1, CoD=1 (status)
                        self.complete_non_data_command();
                    }
                    PacketResult::Error { sense_key, .. } => {
                        // QEMU `ide_atapi_cmd_error`: error = sense_key<<4,
                        // status = READY|ERR (no DSC), IREASON = IO|CoD.
                        self.set_atapi_ireason(0x03);
                        self.data_mode = DataMode::None;
                        self.transfer_kind = None;
                        self.data.clear();
                        self.data_index = 0;
                        self.pending_dma = None;
                        self.pio_write = None;
                        self.error = sense_key << 4;
                        self.status &= !(IDE_STATUS_BSY | IDE_STATUS_DRQ | IDE_STATUS_DSC);
                        self.status |= IDE_STATUS_DRDY | IDE_STATUS_ERR;
                        self.set_irq();
                    }
                    PacketResult::DmaIn(buf) => {
                        // Queue a DMA transfer; completion will raise IRQ.
                        self.set_atapi_ireason(0x02);
                        self.pending_dma = Some(DmaRequest::atapi_data_in(buf));
                        self.status &= !(IDE_STATUS_DRQ | IDE_STATUS_BSY);
                        self.status |= IDE_STATUS_DRDY | IDE_STATUS_DSC;
                        // Packet phase completed; DMA engine will move data.
                        self.data_mode = DataMode::None;
                        self.transfer_kind = None;
                        self.data.clear();
                        self.data_index = 0;
                    }
                }
            }
            Some(TransferKind::Identify) | Some(TransferKind::AtaPioRead) | None => {
                self.complete_non_data_command();
            }
        }
    }
}

/// A compatibility-first PCI IDE controller exposing legacy ports and Bus Master DMA.
pub struct IdeController {
    primary: Channel,
    secondary: Channel,
    bus_master_base: u16,
    bus_master: [BusMasterChannel; 2],
}

impl IdeController {
    pub fn new(bus_master_base: u16) -> Self {
        Self {
            primary: Channel::new(PRIMARY_PORTS),
            secondary: Channel::new(SECONDARY_PORTS),
            bus_master_base,
            bus_master: [BusMasterChannel::new(), BusMasterChannel::new()],
        }
    }
    /// Reset the controller's guest-visible register/state machine back to its power-on baseline,
    /// while preserving attached drives/media/backends.
    pub fn reset(&mut self) {
        // Reset each channel's task file / PIO state machine, but keep the attached device models.
        self.primary.reset();
        self.secondary.reset();

        // Reset host-controlled registers to their power-on defaults.
        //
        // Note: `Channel::reset()` is also used for the IDE software reset (SRST) edge, which does
        // not clear the device control register. For a controller-level reset (PCI/platform
        // reset), we want a full baseline.
        self.primary.control = 0;
        self.secondary.control = 0;

        // Reset Bus Master IDE registers, but preserve DMA capability bits derived from attached
        // devices.
        for chan in &mut self.bus_master {
            chan.reset();
        }
    }

    pub fn bus_master_base(&self) -> u16 {
        self.bus_master_base
    }

    pub fn set_bus_master_base(&mut self, base: u16) {
        self.bus_master_base = base;
    }

    pub fn attach_primary_master_ata(&mut self, mut drive: AtaDrive) {
        if let Some(state) = self.primary.ata_state[0].as_ref() {
            drive.restore_state(state);
        }
        self.primary.ata_state[0] = Some(drive.snapshot_state());
        self.primary.devices[0] = Some(IdeDevice::Ata(Box::new(drive)));
        self.primary.drive_present[0] = true;
        self.bus_master[0].set_drive_dma_capable(0, true);
    }

    pub fn attach_secondary_master_ata(&mut self, mut drive: AtaDrive) {
        if let Some(state) = self.secondary.ata_state[0].as_ref() {
            drive.restore_state(state);
        }
        self.secondary.ata_state[0] = Some(drive.snapshot_state());
        self.secondary.devices[0] = Some(IdeDevice::Ata(Box::new(drive)));
        self.secondary.drive_present[0] = true;
        self.bus_master[1].set_drive_dma_capable(0, true);
    }

    pub fn attach_primary_master_atapi(&mut self, dev: AtapiCdrom) {
        let dma = dev.supports_dma();
        self.primary.devices[0] = Some(IdeDevice::Atapi(dev));
        self.primary.drive_present[0] = true;
        self.bus_master[0].set_drive_dma_capable(0, dma);
    }

    pub fn attach_secondary_master_atapi(&mut self, dev: AtapiCdrom) {
        let dma = dev.supports_dma();
        self.secondary.devices[0] = Some(IdeDevice::Atapi(dev));
        self.secondary.drive_present[0] = true;
        self.bus_master[1].set_drive_dma_capable(0, dma);
    }

    /// Drive IRQ14/IRQ15 immediately when the latch changes.
    ///
    /// Pass clones of the same `PlatformIrqLine` objects used by
    /// `poll_pci_intx_lines` so the ref-counted GSI assert count stays
    /// coherent.
    pub fn attach_irq_lines(&mut self, irq14: Box<dyn IrqLine>, irq15: Box<dyn IrqLine>) {
        self.primary.irq_line = Some(irq14);
        self.secondary.irq_line = Some(irq15);
        self.primary.sync_irq_pin();
        self.secondary.sync_irq_pin();
    }

    /// Eject media from the secondary master ATAPI device (IDE secondary channel, drive 0).
    ///
    /// This preserves the presence of the CD-ROM device itself (it remains attached to the bus),
    /// but marks the tray open / no media. If the secondary master is not an ATAPI device, this is
    /// a no-op.
    pub fn eject_secondary_master_atapi_media(&mut self) {
        if let Some(IdeDevice::Atapi(dev)) = self.secondary.devices[0].as_mut() {
            dev.eject_media();
            // `supports_dma` is a device capability, not a property of the inserted media, but
            // keep the bus-master view coherent regardless.
            self.bus_master[1].set_drive_dma_capable(0, dev.supports_dma());
            self.secondary.drive_present[0] = true;
        }
    }

    /// Returns whether the secondary master ATAPI device (install media slot) reports media present
    /// from the guest's perspective.
    ///
    /// This is independent of whether a host ISO backend is currently attached: snapshot restore
    /// drops the backend but preserves `media_present`.
    pub fn secondary_master_atapi_media_present(&self) -> bool {
        match self.secondary.devices[0].as_ref() {
            Some(IdeDevice::Atapi(dev)) => dev.media_present(),
            _ => false,
        }
    }

    /// Re-attaches a host ISO backend to an existing ATAPI device without changing guest-visible
    /// media state.
    ///
    /// This is intended for snapshot restore: the controller snapshot restores the ATAPI device's
    /// internal state (tray/sense/media_changed) but drops the host backend reference.
    pub fn attach_primary_master_atapi_backend_for_restore(
        &mut self,
        backend: Box<dyn IsoBackend>,
    ) {
        match self.primary.devices[0].as_mut() {
            Some(IdeDevice::Atapi(dev)) => {
                dev.attach_backend_for_restore(backend);
                self.bus_master[0].set_drive_dma_capable(0, dev.supports_dma());
            }
            _ => {
                let dev = AtapiCdrom::new(Some(backend));
                let dma = dev.supports_dma();
                self.primary.devices[0] = Some(IdeDevice::Atapi(dev));
                self.bus_master[0].set_drive_dma_capable(0, dma);
            }
        }
        self.primary.drive_present[0] = true;
    }

    pub fn attach_secondary_master_atapi_backend_for_restore(
        &mut self,
        backend: Box<dyn IsoBackend>,
    ) {
        match self.secondary.devices[0].as_mut() {
            Some(IdeDevice::Atapi(dev)) => {
                dev.attach_backend_for_restore(backend);
                self.bus_master[1].set_drive_dma_capable(0, dev.supports_dma());
            }
            _ => {
                let dev = AtapiCdrom::new(Some(backend));
                let dma = dev.supports_dma();
                self.secondary.devices[0] = Some(IdeDevice::Atapi(dev));
                self.bus_master[1].set_drive_dma_capable(0, dma);
            }
        }
        self.secondary.drive_present[0] = true;
    }

    fn decode_bus_master(&self, port: u16) -> Option<(usize, u16)> {
        let base = self.bus_master_base;
        // Use wrapping arithmetic so pathological base addresses near `u16::MAX` can't trigger an
        // overflow panic when computing `base + len` under overflow-check builds (e.g. fuzzing),
        // and so the decode behaves like real 16-bit I/O port arithmetic (wrapping).
        let off = port.wrapping_sub(base);
        if off < 16 {
            let chan = (off / 8) as usize;
            let reg_off = off % 8;
            return Some((chan, reg_off));
        }
        None
    }

    pub fn io_read(&mut self, port: u16, size: u8) -> u32 {
        // Treat zero-sized accesses as true no-ops. (They are not representable by the x86 ISA,
        // but defensive callers may still attempt them.)
        if size == 0 {
            return 0;
        }

        // Command blocks.
        let off = port.wrapping_sub(self.primary.ports.cmd_base);
        if off < 8 {
            let reg = off;
            return Self::read_cmd_reg(&mut self.primary, reg, size);
        }
        let off = port.wrapping_sub(self.secondary.ports.cmd_base);
        if off < 8 {
            let reg = off;
            return Self::read_cmd_reg(&mut self.secondary, reg, size);
        }

        // Control blocks.
        let off = port.wrapping_sub(self.primary.ports.ctrl_base);
        if off < 2 {
            let reg = off;
            return Self::read_ctrl_reg(&mut self.primary, reg, size);
        }
        let off = port.wrapping_sub(self.secondary.ports.ctrl_base);
        if off < 2 {
            let reg = off;
            return Self::read_ctrl_reg(&mut self.secondary, reg, size);
        }

        // Bus master.
        if let Some((chan, reg_off)) = self.decode_bus_master(port) {
            return self.bus_master[chan].read(reg_off, size);
        }

        // Open bus.
        match size {
            1 => 0xFF,
            2 => 0xFFFF,
            4 => 0xFFFF_FFFF,
            _ => 0xFFFF_FFFF,
        }
    }

    pub fn io_write(&mut self, port: u16, size: u8, val: u32) {
        if size == 0 {
            return;
        }

        // Command blocks.
        let off = port.wrapping_sub(self.primary.ports.cmd_base);
        if off < 8 {
            let reg = off;
            Self::write_cmd_reg(&mut self.primary, reg, size, val);
            return;
        }
        let off = port.wrapping_sub(self.secondary.ports.cmd_base);
        if off < 8 {
            let reg = off;
            Self::write_cmd_reg(&mut self.secondary, reg, size, val);
            return;
        }

        // Control blocks.
        let off = port.wrapping_sub(self.primary.ports.ctrl_base);
        if off < 2 {
            let reg = off;
            Self::write_ctrl_reg(&mut self.primary, reg, val as u8);
            return;
        }
        let off = port.wrapping_sub(self.secondary.ports.ctrl_base);
        if off < 2 {
            let reg = off;
            Self::write_ctrl_reg(&mut self.secondary, reg, val as u8);
            return;
        }

        // Bus master.
        if let Some((chan, reg_off)) = self.decode_bus_master(port) {
            self.bus_master[chan].write(reg_off, size, val);
        }
    }

    fn read_cmd_reg(chan: &mut Channel, reg: u16, size: u8) -> u32 {
        let dev_idx = chan.selected_drive() as usize;
        // If the currently-selected drive is not present, float the bus high (all ones). This
        // matches common PATA probing logic where reading Status/AltStatus returns 0xFF when no
        // device responds.
        if !chan.drive_present[dev_idx] {
            return match size {
                1 => 0xFF,
                2 => 0xFFFF,
                4 => 0xFFFF_FFFF,
                _ => 0xFFFF_FFFF,
            };
        }

        match reg {
            ATA_REG_DATA => match size {
                1 => chan.data_in_u8() as u32,
                2 => chan.data_in_u16() as u32,
                4 => chan.data_in_u32(),
                _ => 0,
            },
            ATA_REG_ERROR_FEATURES => chan.error as u32,
            ATA_REG_SECTOR_COUNT | ATA_REG_LBA0 | ATA_REG_LBA1 | ATA_REG_LBA2 | ATA_REG_DEVICE => {
                let hob = (chan.control & IDE_CTRL_HOB) != 0;
                chan.tf.read_reg(reg, hob) as u32
            }
            ATA_REG_STATUS_COMMAND => {
                // Reading STATUS clears pending IRQ.
                chan.clear_irq();
                chan.status as u32
            }
            _ => 0,
        }
    }

    fn write_cmd_reg(chan: &mut Channel, reg: u16, size: u8, val: u32) {
        // Writes only affect the currently-selected device. If it is absent, ignore writes so a
        // guest probing for a slave device does not accidentally perturb the master device's
        // taskfile register image.
        //
        // Always honor writes to the Device/Head register because those are used to select a drive.
        let dev_idx = chan.selected_drive() as usize;
        if reg != ATA_REG_DEVICE && !chan.drive_present[dev_idx] {
            return;
        }

        match reg {
            ATA_REG_DATA => match size {
                1 => chan.data_out_u8(val as u8),
                2 => chan.data_out_u16(val as u16),
                4 => chan.data_out_u32(val),
                _ => {}
            },
            ATA_REG_ERROR_FEATURES
            | ATA_REG_SECTOR_COUNT
            | ATA_REG_LBA0
            | ATA_REG_LBA1
            | ATA_REG_LBA2
            | ATA_REG_DEVICE => {
                // QEMU `ide_ioport_write`: command-block writes other than the
                // command register are ignored while BSY or DRQ. Data is a
                // separate port. Leaving these live during ATAPI CDB/data
                // phases lets a leftover SET FEATURES count or a mid-DRQ BCL
                // write corrupt IREASON.
                if (chan.status & (IDE_STATUS_BSY | IDE_STATUS_DRQ)) != 0 {
                    return;
                }
                chan.tf.write_reg(reg, val as u8);
            }
            ATA_REG_STATUS_COMMAND => {
                Self::exec_command(chan, val as u8);
            }
            _ => {}
        }
    }

    fn read_ctrl_reg(chan: &mut Channel, reg: u16, size: u8) -> u32 {
        let dev_idx = chan.selected_drive() as usize;
        if !chan.drive_present[dev_idx] {
            return match size {
                1 => 0xFF,
                2 => 0xFFFF,
                4 => 0xFFFF_FFFF,
                _ => 0xFFFF_FFFF,
            };
        }

        match reg {
            ATA_CTRL_ALT_STATUS_DEVICE_CTRL => chan.status as u32,
            ATA_CTRL_DRIVE_ADDRESS => chan.drive_address() as u32,
            _ => 0,
        }
    }

    fn write_ctrl_reg(chan: &mut Channel, reg: u16, val: u8) {
        if reg != ATA_CTRL_ALT_STATUS_DEVICE_CTRL {
            return;
        }
        // Handle software reset edge (SRST going 0->1 then 1->0).
        let prev = chan.control;
        chan.control = val;
        if (prev & IDE_CTRL_SRST) == 0 && (val & IDE_CTRL_SRST) != 0 {
            if ide_trace_enabled() {
                let which = if chan.ports.cmd_base == SECONDARY_PORTS.cmd_base {
                    "sec"
                } else {
                    "pri"
                };
                eprintln!("AERO_IDE_TRACE: {which} SRST assert");
            }
            chan.reset();
        } else if (prev ^ val) & IDE_CTRL_NIEN != 0 {
            // nIEN masks the pin but not the latch. Drive the pin now so a
            // guest that clears nIEN mid-batch sees the pending edge.
            chan.sync_irq_pin();
        }
    }

    fn exec_command(chan: &mut Channel, cmd: u8) {
        if ide_trace_enabled() {
            let which = if chan.ports.cmd_base == SECONDARY_PORTS.cmd_base {
                "sec"
            } else {
                "pri"
            };
            eprintln!(
                "AERO_IDE_TRACE: {which} cmd={cmd:#04x} dev={:#04x} present={} status={:#04x} lba1={:#04x} lba2={:#04x}",
                chan.tf.device,
                chan.drive_present[chan.selected_drive() as usize],
                chan.status,
                chan.tf.lba1,
                chan.tf.lba2
            );
        }
        chan.status |= IDE_STATUS_BSY;
        chan.status &= !IDE_STATUS_DRQ;
        chan.clear_irq();

        let is_lba48 = matches!(cmd, 0x24 | 0x34 | 0x25 | 0x35 | 0xEA);
        chan.tf.normalize_for_command(is_lba48);

        let dev_idx = chan.selected_drive() as usize;

        match cmd {
            0xEC => {
                // IDENTIFY DEVICE
                match chan.devices[dev_idx].as_ref() {
                    Some(IdeDevice::Ata(_)) => {
                        let data = ata_identify_data(chan.devices[dev_idx].as_ref());
                        chan.begin_pio_in(TransferKind::Identify, data);
                    }
                    Some(IdeDevice::Atapi(_)) => {
                        // Per ATA/ATAPI spec, an ATAPI device aborts IDENTIFY DEVICE and leaves a
                        // signature in the LBA Mid/High registers so the guest can detect that a
                        // packet device is present.
                        chan.tf.sector_count = 0x01;
                        chan.tf.lba0 = 0x01;
                        chan.tf.lba1 = 0x14;
                        chan.tf.lba2 = 0xEB;
                        chan.abort_command(0x04);
                    }
                    None => {
                        chan.abort_command(0x04);
                    }
                }
            }
            0xA1 => {
                // IDENTIFY PACKET DEVICE
                let data = match chan.devices[dev_idx].as_ref() {
                    Some(IdeDevice::Atapi(dev)) => Some(dev.identify_packet_data()),
                    _ => None,
                };
                if let Some(data) = data {
                    chan.begin_pio_in(TransferKind::Identify, data);
                } else {
                    chan.abort_command(0x04);
                }
            }
            0x20 | 0x24 => {
                // READ SECTORS (PIO) / READ SECTORS EXT (PIO).
                let (lba, sectors) = if cmd == 0x24 {
                    (chan.tf.lba48(), chan.tf.sector_count48() as u64)
                } else {
                    (chan.tf.lba28(), chan.tf.sector_count28() as u64)
                };

                let res = match chan.devices[dev_idx].as_mut() {
                    Some(IdeDevice::Ata(dev)) => ata_pio_read(dev, lba, sectors).ok(),
                    _ => None,
                };

                if let Some(buf) = res {
                    chan.begin_pio_in(TransferKind::AtaPioRead, buf);
                } else {
                    chan.abort_command(0x04);
                }
            }
            0x30 | 0x34 => {
                // WRITE SECTORS (PIO) / WRITE SECTORS EXT (PIO).
                if matches!(chan.devices[dev_idx], Some(IdeDevice::Ata(_))) {
                    let (lba, sectors) = if cmd == 0x34 {
                        (chan.tf.lba48(), chan.tf.sector_count48() as u64)
                    } else {
                        (chan.tf.lba28(), chan.tf.sector_count28() as u64)
                    };

                    let byte_len = sectors
                        .checked_mul(SECTOR_SIZE as u64)
                        .and_then(|v| usize::try_from(v).ok())
                        .filter(|&v| v <= MAX_IDE_DATA_BUFFER_BYTES);

                    if let Some(byte_len) = byte_len {
                        chan.pio_write = Some((lba, sectors));
                        chan.begin_pio_out(TransferKind::AtaPioWrite, byte_len);
                    } else {
                        chan.abort_command(0x04);
                    }
                } else {
                    chan.abort_command(0x04);
                }
            }
            0xC8 | 0xCA | 0x25 | 0x35 => {
                // DMA commands.
                let is_write = matches!(cmd, 0xCA | 0x35);
                let (lba, sectors) = if matches!(cmd, 0x25 | 0x35) {
                    (chan.tf.lba48(), chan.tf.sector_count48() as u64)
                } else {
                    (chan.tf.lba28(), chan.tf.sector_count28() as u64)
                };

                let req = match chan.devices[dev_idx].as_mut() {
                    Some(IdeDevice::Ata(dev)) => {
                        if is_write {
                            let byte_len = sectors
                                .checked_mul(SECTOR_SIZE as u64)
                                .and_then(|v| usize::try_from(v).ok())
                                .filter(|&v| v <= MAX_IDE_DATA_BUFFER_BYTES);

                            byte_len.and_then(|len| {
                                try_alloc_zeroed(len)
                                    .map(|buf| DmaRequest::ata_write(buf, lba, sectors))
                            })
                        } else {
                            ata_pio_read(dev, lba, sectors)
                                .ok()
                                .map(DmaRequest::ata_read)
                        }
                    }
                    _ => None,
                };

                if let Some(req) = req {
                    chan.pending_dma = Some(req);
                    chan.status &= !IDE_STATUS_BSY;
                    chan.status |= IDE_STATUS_DRDY;
                } else {
                    chan.abort_command(0x04);
                }
            }
            0xE7 | 0xEA => {
                // FLUSH CACHE / FLUSH CACHE EXT
                let ok = match chan.devices[dev_idx].as_mut() {
                    Some(IdeDevice::Ata(dev)) => dev.flush().is_ok(),
                    _ => false,
                };
                if ok {
                    chan.complete_non_data_command();
                } else {
                    chan.abort_command(0x04);
                }
            }
            0x40 | 0x42 | 0x91 => {
                // READ VERIFY / READ VERIFY EXT / INITIALIZE DEVICE PARAMETERS.
                // No data phase. FormatEx issues VERIFY while preparing NTFS.
                if matches!(chan.devices[dev_idx], Some(IdeDevice::Ata(_))) {
                    chan.complete_non_data_command();
                } else {
                    chan.abort_command(0x04);
                }
            }
            0xEF => {
                // SET FEATURES
                let features = chan.tf.features;
                let sector_count = chan.tf.sector_count;
                if ide_trace_enabled() {
                    eprintln!(
                        "AERO_IDE_TRACE: set_features sub={features:#04x} count={sector_count:#04x}"
                    );
                }
                let ok = match chan.devices[dev_idx].as_mut() {
                    Some(IdeDevice::Ata(dev)) => {
                        match features {
                            0x02 => {
                                dev.set_write_cache_enabled(true);
                                true
                            }
                            0x82 => {
                                dev.set_write_cache_enabled(false);
                                true
                            }
                            0x03 => dev.set_transfer_mode_select(sector_count).is_ok(),
                            // Unknown SET FEATURES subcommands are treated as no-ops.
                            _ => true,
                        }
                    }
                    // ATAPI devices accept SET FEATURES (transfer mode negotiation)
                    // as a no-op success per typical ATAPI behavior. Without this,
                    // Win7's atapi.sys fails DMA mode negotiation and falls back to
                    // slow PIO for the CD-ROM.
                    Some(IdeDevice::Atapi(_)) => true,
                    _ => false,
                };
                if ok {
                    chan.complete_non_data_command();
                } else {
                    chan.abort_command(0x04);
                }
            }
            0x90 => {
                // EXECUTE DEVICE DIAGNOSTIC. Win7 atapi.sys uses this (not only
                // SRST) to classify PACKET vs ATA. ABRT here leaves ERR set and
                // the miniport drops the drive — no CdRom PDO.
                if matches!(chan.devices[dev_idx], Some(IdeDevice::Atapi(_)) | Some(IdeDevice::Ata(_)))
                {
                    chan.data_mode = DataMode::None;
                    chan.transfer_kind = None;
                    chan.data.clear();
                    chan.data_index = 0;
                    chan.pending_dma = None;
                    chan.pio_write = None;
                    chan.apply_device_signature(dev_idx);
                    chan.error = 0x01; // device 0 passed
                    if matches!(chan.devices[dev_idx], Some(IdeDevice::Atapi(_))) {
                        // ATA/ATAPI-6 9.10: packet devices return a clear
                        // status (READY not set) and do not raise INTRQ.
                        chan.status = 0;
                        chan.clear_irq();
                    } else {
                        chan.status &= !(IDE_STATUS_BSY | IDE_STATUS_DRQ | IDE_STATUS_ERR);
                        chan.status |= IDE_STATUS_DRDY | IDE_STATUS_DSC;
                        chan.set_irq();
                    }
                } else {
                    chan.abort_command(0x04);
                }
            }
            0x08 => {
                // DEVICE RESET — mandatory for PACKET devices (ATA/ATAPI-7).
                // Win7 atapi.sys issues this before IDENTIFY PACKET. An ABRT
                // here makes the miniport drop the drive (no CdRom PDO).
                if matches!(chan.devices[dev_idx], Some(IdeDevice::Atapi(_))) {
                    chan.data_mode = DataMode::None;
                    chan.transfer_kind = None;
                    chan.data.clear();
                    chan.data_index = 0;
                    chan.pending_dma = None;
                    chan.pio_write = None;
                    chan.tf.sector_count = 0x01;
                    chan.tf.lba0 = 0x01;
                    chan.tf.lba1 = 0x14;
                    chan.tf.lba2 = 0xEB;
                    // ATA8-ACS3: DEVICE RESET leaves status = 0 (READY clear)
                    // and does not assert INTRQ.
                    chan.status = 0;
                    chan.clear_error();
                    chan.clear_irq();
                } else {
                    chan.abort_command(0x04);
                }
            }
            0xA0 => {
                // ATAPI PACKET
                if matches!(chan.devices[dev_idx], Some(IdeDevice::Atapi(_))) {
                    // QEMU `cmd_packet` *assigns* nsector = ATAPI_INT_REASON_CD
                    // (1). A leftover SET FEATURES count (Win7 uses 0x0C = PIO
                    // mode 4) must not leak into IREASON; later reply_end
                    // preserves only bits 7:3 of this freshly assigned value.
                    chan.tf.sector_count = 0x01;
                    chan.begin_pio_out(TransferKind::AtapiPacket, 12);
                } else {
                    chan.abort_command(0x04);
                }
            }
            _ => {
                chan.abort_command(0x04);
            }
        }
    }

    /// Execute any pending Bus Master DMA transfers.
    ///
    /// A real device would DMA asynchronously. For emulator simplicity we
    /// complete transfers synchronously when the Bus Master start bit is set.
    pub fn tick(&mut self, mem: &mut dyn MemoryBus) {
        Self::tick_channel(&mut self.bus_master[0], &mut self.primary, mem);
        Self::tick_channel(&mut self.bus_master[1], &mut self.secondary, mem);
    }

    fn tick_channel(bm: &mut BusMasterChannel, chan: &mut Channel, mem: &mut dyn MemoryBus) {
        if !bm.is_started() {
            return;
        }
        let Some(mut req) = chan.pending_dma.take() else {
            return;
        };

        match bm.execute_dma(mem, &mut req) {
            Ok(()) => {
                let mut ok = true;
                // Commit writes after the DMA engine has pulled data from guest memory.
                if let Some(DmaCommit::AtaWrite { lba, sectors }) = req.commit.take() {
                    let dev_idx = chan.selected_drive() as usize;
                    ok = match chan.devices[dev_idx].as_mut() {
                        Some(IdeDevice::Ata(dev)) => {
                            ata_pio_write(dev, lba, sectors, &req.buffer).is_ok()
                        }
                        _ => false,
                    };
                }

                if ok {
                    bm.finish_success();
                    // For ATAPI DMA commands, transition to status phase (interrupt reason).
                    let dev_idx = chan.selected_drive() as usize;
                    if matches!(chan.devices[dev_idx], Some(IdeDevice::Atapi(_))) {
                        chan.set_atapi_ireason(0x03); // IO=1, CoD=1
                    }
                    chan.complete_non_data_command();
                } else {
                    bm.finish_error();
                    chan.abort_command(0x04);
                }
            }
            Err(_) => {
                bm.finish_error();
                // For ATAPI DMA commands, still transition to status phase.
                let dev_idx = chan.selected_drive() as usize;
                if matches!(chan.devices[dev_idx], Some(IdeDevice::Atapi(_))) {
                    chan.set_atapi_ireason(0x03);
                }
                chan.abort_command(0x04);
            }
        }
    }

    pub fn primary_irq_pending(&self) -> bool {
        self.primary.irq_pending && (self.primary.control & IDE_CTRL_NIEN) == 0
    }

    pub fn secondary_irq_pending(&self) -> bool {
        self.secondary.irq_pending && (self.secondary.control & IDE_CTRL_NIEN) == 0
    }
}

fn atapi_packet_transfers_data(opcode: u8) -> bool {
    // Mirror QEMU's NONDATA flag: these complete without a data phase.
    !matches!(opcode, 0x00 | 0x1B | 0x1E | 0xBB | 0x2B)
}

fn try_alloc_zeroed(len: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    buf.try_reserve_exact(len).ok()?;
    buf.resize(len, 0);
    Some(buf)
}

fn ata_identify_data(dev: Option<&IdeDevice>) -> Vec<u8> {
    match dev {
        Some(IdeDevice::Ata(d)) => d.identify_sector().to_vec(),
        _ => vec![0; SECTOR_SIZE],
    }
}

fn ata_pio_read(dev: &mut AtaDrive, lba: u64, sectors: u64) -> io::Result<Vec<u8>> {
    let byte_len = sectors
        .checked_mul(SECTOR_SIZE as u64)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "transfer too large"))?;
    if byte_len > MAX_IDE_DATA_BUFFER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "transfer too large",
        ));
    }
    let mut buf = try_alloc_zeroed(byte_len).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::OutOfMemory,
            "failed to allocate ATA PIO read buffer",
        )
    })?;
    dev.read_sectors(lba, &mut buf)?;
    Ok(buf)
}

fn ata_pio_write(dev: &mut AtaDrive, lba: u64, sectors: u64, data: &[u8]) -> io::Result<()> {
    let byte_len = sectors
        .checked_mul(SECTOR_SIZE as u64)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "transfer too large"))?;
    if byte_len > MAX_IDE_DATA_BUFFER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "transfer too large",
        ));
    }
    if data.len() < byte_len {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "short write buffer",
        ));
    }
    dev.write_sectors(lba, &data[..byte_len])?;
    Ok(())
}

/// Canonical PCI function wrapper for a PIIX3-compatible IDE controller.
pub struct Piix3IdePciDevice {
    pub controller: IdeController,
    config: PciConfigSpace,
}

impl Piix3IdePciDevice {
    pub const DEFAULT_BUS_MASTER_BASE: u16 = 0xC000;

    /// PIIX3 IDE Timing (IDETIM) registers: primary at 0x40, secondary at 0x42.
    /// Bit 15 is IDE Decode Enable. Firmware (and QEMU/KVM after the
    /// "initialize channels as enabled" fix) sets this so `intelide.sys`
    /// creates channel PDOs; a zero register means "channel disabled."
    const IDETIM_PRIMARY: u16 = 0x40;
    const IDETIM_SECONDARY: u16 = 0x42;
    const IDETIM_DECODE_ENABLE: u16 = 0x8000;

    pub fn new() -> Self {
        let mut config = IDE_PIIX3.build_config_space();

        // Only BAR4 (BMIDE) is a PCI BAR. Command/control blocks are hardwired
        // ISA compatibility ports, matching QEMU `piix3-ide`.
        config.set_bar_base(4, Self::DEFAULT_BUS_MASTER_BASE as u64);
        Self::enable_legacy_channels(&mut config);

        Self {
            controller: IdeController::new(Self::DEFAULT_BUS_MASTER_BASE),
            config,
        }
    }

    fn enable_legacy_channels(config: &mut PciConfigSpace) {
        config.write(Self::IDETIM_PRIMARY, 2, u32::from(Self::IDETIM_DECODE_ENABLE));
        config.write(Self::IDETIM_SECONDARY, 2, u32::from(Self::IDETIM_DECODE_ENABLE));
    }

    /// Reset device state back to its power-on baseline while preserving attached drives/media.
    pub fn reset(&mut self) {
        // Mirror PCI reset semantics: clear command register state (BAR programming is preserved).
        //
        // Note: We implement `PciDevice::reset` for this type by calling this method, so avoid
        // calling the trait method here (it would recurse).
        self.config.set_command(0);
        Self::enable_legacy_channels(&mut self.config);
        self.controller.reset();
    }

    pub fn bus_master_base(&self) -> u16 {
        self.config
            .bar_range(4)
            .and_then(|range| u16::try_from(range.base).ok())
            .unwrap_or_else(|| self.controller.bus_master_base())
    }

    fn sync_bus_master_base_from_config(&mut self) {
        let Some(range) = self.config.bar_range(4) else {
            return;
        };
        if range.base == 0 {
            return;
        }
        let Ok(base) = u16::try_from(range.base) else {
            return;
        };
        self.controller.set_bus_master_base(base);
    }

    pub fn tick(&mut self, mem: &mut dyn MemoryBus) {
        // Only allow the device to DMA when PCI Bus Mastering is enabled (PCI command bit 2).
        if (self.config.command() & (1 << 2)) == 0 {
            return;
        }
        self.controller.tick(mem);
    }

    pub fn io_read(&mut self, port: u16, size: u8) -> u32 {
        self.sync_bus_master_base_from_config();
        // IO space decode is gated by PCI command bit 0.
        if (self.config.command() & 0x1) == 0 {
            return match size {
                1 => 0xFF,
                2 => 0xFFFF,
                4 => 0xFFFF_FFFF,
                _ => 0xFFFF_FFFF,
            };
        }
        self.controller.io_read(port, size)
    }

    pub fn io_write(&mut self, port: u16, size: u8, value: u32) {
        self.sync_bus_master_base_from_config();
        if (self.config.command() & 0x1) == 0 {
            return;
        }
        self.controller.io_write(port, size, value)
    }

    pub fn attach_irq_lines(&mut self, irq14: Box<dyn IrqLine>, irq15: Box<dyn IrqLine>) {
        self.controller.attach_irq_lines(irq14, irq15);
    }

    pub fn snapshot_state(&self) -> IdeControllerState {
        fn snapshot_tf(tf: &TaskFile) -> IdeTaskFileState {
            IdeTaskFileState {
                features: tf.features,
                sector_count: tf.sector_count,
                lba0: tf.lba0,
                lba1: tf.lba1,
                lba2: tf.lba2,
                device: tf.device,
                hob_features: tf.hob_features,
                hob_sector_count: tf.hob_sector_count,
                hob_lba0: tf.hob_lba0,
                hob_lba1: tf.hob_lba1,
                hob_lba2: tf.hob_lba2,
                pending_features_high: tf.pending_features_high,
                pending_sector_count_high: tf.pending_sector_count_high,
                pending_lba0_high: tf.pending_lba0_high,
                pending_lba1_high: tf.pending_lba1_high,
                pending_lba2_high: tf.pending_lba2_high,
            }
        }

        fn snapshot_channel(chan: &Channel, bm: &BusMasterChannel, irq: u8) -> IdeChannelState {
            let data_mode = match chan.data_mode {
                DataMode::None => IdeDataMode::None,
                DataMode::PioIn => IdeDataMode::PioIn,
                DataMode::PioOut => IdeDataMode::PioOut,
            };
            let transfer_kind = chan.transfer_kind.map(|k| match k {
                TransferKind::AtaPioRead => IdeTransferKind::AtaPioRead,
                TransferKind::AtaPioWrite => IdeTransferKind::AtaPioWrite,
                TransferKind::Identify => IdeTransferKind::Identify,
                TransferKind::AtapiPacket => IdeTransferKind::AtapiPacket,
                TransferKind::AtapiPioIn => IdeTransferKind::AtapiPioIn,
            });

            let pending_dma = chan.pending_dma.as_ref().map(|req| IdeDmaRequestState {
                direction: match req.direction {
                    crate::busmaster::DmaDirection::ToMemory => IdeDmaDirection::ToMemory,
                    crate::busmaster::DmaDirection::FromMemory => IdeDmaDirection::FromMemory,
                },
                buffer: req.buffer.clone(),
                commit: req.commit.as_ref().map(|c| match c {
                    DmaCommit::AtaWrite { lba, sectors } => IdeDmaCommitState::AtaWrite {
                        lba: *lba,
                        sectors: *sectors,
                    },
                }),
            });

            let pio_write = chan
                .pio_write
                .map(|(lba, sectors)| IdePioWriteState { lba, sectors });

            let drives = core::array::from_fn(|idx| match chan.devices[idx].as_ref() {
                None => {
                    if chan.drive_present[idx] {
                        IdeDriveState::Ata(
                            chan.ata_state[idx]
                                .clone()
                                .unwrap_or(IdeAtaDeviceState { udma_mode: 2 }),
                        )
                    } else {
                        IdeDriveState::None
                    }
                }
                Some(IdeDevice::Ata(dev)) => IdeDriveState::Ata(dev.snapshot_state()),
                Some(IdeDevice::Atapi(dev)) => IdeDriveState::Atapi(dev.snapshot_state()),
            });

            IdeChannelState {
                ports: IdePortMapState {
                    cmd_base: chan.ports.cmd_base,
                    ctrl_base: chan.ports.ctrl_base,
                    irq,
                },
                tf: snapshot_tf(&chan.tf),
                status: chan.status,
                error: chan.error,
                control: chan.control,
                irq_pending: chan.irq_pending,
                data_mode,
                transfer_kind,
                data: chan.data.clone(),
                data_index: chan.data_index.min(u32::MAX as usize) as u32,
                pending_dma,
                pio_write,
                bus_master: bm.snapshot_state(),
                drives,
            }
        }

        // Snapshot the PCI config space and BAR state. Unimplemented BAR0–3 must
        // stay zero (not "I/O BAR at address 0") so a restore cannot resurrect
        // the phantom native windows Win7 mis-assigns.
        let pci_snap = self.config.snapshot_state();
        let mut regs = pci_snap.bytes;
        let bar_dword = |index: usize| -> u32 {
            match self.config.bar_definition(index as u8) {
                Some(PciBarDefinition::Io { .. }) => {
                    (pci_snap.bar_base[index] as u32 & 0xFFFF_FFFC) | 0x01
                }
                Some(_) => pci_snap.bar_base[index] as u32,
                None => 0,
            }
        };
        let bar0 = bar_dword(0);
        let bar1 = bar_dword(1);
        let bar2 = bar_dword(2);
        let bar3 = bar_dword(3);
        let bar4 = bar_dword(4);

        regs[0x10..0x14].copy_from_slice(&bar0.to_le_bytes());
        regs[0x14..0x18].copy_from_slice(&bar1.to_le_bytes());
        regs[0x18..0x1C].copy_from_slice(&bar2.to_le_bytes());
        regs[0x1C..0x20].copy_from_slice(&bar3.to_le_bytes());
        regs[0x20..0x24].copy_from_slice(&bar4.to_le_bytes());

        IdeControllerState {
            pci: PciConfigSpaceState {
                regs,
                bar0,
                bar1,
                bar2,
                bar3,
                bar4,
                bar0_probe: pci_snap.bar_probe[0],
                bar1_probe: pci_snap.bar_probe[1],
                bar2_probe: pci_snap.bar_probe[2],
                bar3_probe: pci_snap.bar_probe[3],
                bar4_probe: pci_snap.bar_probe[4],
                bus_master_base: self.controller.bus_master_base,
            },
            primary: snapshot_channel(&self.controller.primary, &self.controller.bus_master[0], 14),
            secondary: snapshot_channel(
                &self.controller.secondary,
                &self.controller.bus_master[1],
                15,
            ),
        }
    }

    pub fn restore_state(&mut self, state: &IdeControllerState) {
        fn restore_tf(tf: &mut TaskFile, state: &IdeTaskFileState) {
            tf.features = state.features;
            tf.sector_count = state.sector_count;
            tf.lba0 = state.lba0;
            tf.lba1 = state.lba1;
            tf.lba2 = state.lba2;
            tf.device = state.device;

            tf.hob_features = state.hob_features;
            tf.hob_sector_count = state.hob_sector_count;
            tf.hob_lba0 = state.hob_lba0;
            tf.hob_lba1 = state.hob_lba1;
            tf.hob_lba2 = state.hob_lba2;

            tf.pending_features_high = state.pending_features_high;
            tf.pending_sector_count_high = state.pending_sector_count_high;
            tf.pending_lba0_high = state.pending_lba0_high;
            tf.pending_lba1_high = state.pending_lba1_high;
            tf.pending_lba2_high = state.pending_lba2_high;
        }

        fn restore_channel(chan: &mut Channel, bm: &mut BusMasterChannel, state: &IdeChannelState) {
            chan.ports.cmd_base = state.ports.cmd_base;
            chan.ports.ctrl_base = state.ports.ctrl_base;

            restore_tf(&mut chan.tf, &state.tf);

            chan.status = state.status;
            chan.error = state.error;
            chan.control = state.control;
            chan.irq_pending = state.irq_pending;

            chan.data_mode = match state.data_mode {
                IdeDataMode::None => DataMode::None,
                IdeDataMode::PioIn => DataMode::PioIn,
                IdeDataMode::PioOut => DataMode::PioOut,
            };

            chan.transfer_kind = state.transfer_kind.map(|k| match k {
                IdeTransferKind::AtaPioRead => TransferKind::AtaPioRead,
                IdeTransferKind::AtaPioWrite => TransferKind::AtaPioWrite,
                IdeTransferKind::Identify => TransferKind::Identify,
                IdeTransferKind::AtapiPacket => TransferKind::AtapiPacket,
                IdeTransferKind::AtapiPioIn => TransferKind::AtapiPioIn,
            });

            chan.data = state.data.clone();
            chan.data_index = (state.data_index as usize).min(chan.data.len());

            chan.pio_write = state.pio_write.as_ref().map(|pw| (pw.lba, pw.sectors));

            chan.pending_dma = state.pending_dma.as_ref().map(|req| DmaRequest {
                direction: match req.direction {
                    IdeDmaDirection::ToMemory => crate::busmaster::DmaDirection::ToMemory,
                    IdeDmaDirection::FromMemory => crate::busmaster::DmaDirection::FromMemory,
                },
                buffer: req.buffer.clone(),
                commit: req.commit.as_ref().map(|c| match c {
                    IdeDmaCommitState::AtaWrite { lba, sectors } => DmaCommit::AtaWrite {
                        lba: *lba,
                        sectors: *sectors,
                    },
                }),
            });

            bm.restore_state(&state.bus_master);

            // Restore drive-visible state but drop any host-side backends. The platform must
            // re-attach disks/ISOs after restore.
            for slot in 0..2 {
                chan.devices[slot] = None;
                chan.drive_present[slot] = false;
                chan.ata_state[slot] = None;
                bm.set_drive_dma_capable(slot, false);
            }

            for slot in 0..2 {
                match &state.drives[slot] {
                    IdeDriveState::None => {}
                    IdeDriveState::Ata(_s) => {
                        // ATA backends are host-managed (snapshotted separately). Guests may still
                        // observe DMA capability bits, so restore them conservatively.
                        bm.set_drive_dma_capable(slot, true);
                        chan.drive_present[slot] = true;
                        chan.ata_state[slot] = Some(_s.clone());
                    }
                    IdeDriveState::Atapi(s) => {
                        let mut dev = AtapiCdrom::new(None);
                        dev.restore_state(s);
                        bm.set_drive_dma_capable(slot, dev.supports_dma());
                        chan.devices[slot] = Some(IdeDevice::Atapi(dev));
                        chan.drive_present[slot] = true;
                    }
                }
            }
        }

        // Restore PCI config space state.
        let mut bar_base = [0u64; 6];
        let mut bar_probe = [false; 6];
        bar_base[0] = u64::from(state.pci.bar0 & 0xFFFF_FFFC);
        bar_base[1] = u64::from(state.pci.bar1 & 0xFFFF_FFFC);
        bar_base[2] = u64::from(state.pci.bar2 & 0xFFFF_FFFC);
        bar_base[3] = u64::from(state.pci.bar3 & 0xFFFF_FFFC);
        bar_base[4] = u64::from(state.pci.bar4 & 0xFFFF_FFFC);
        bar_probe[0] = state.pci.bar0_probe;
        bar_probe[1] = state.pci.bar1_probe;
        bar_probe[2] = state.pci.bar2_probe;
        bar_probe[3] = state.pci.bar3_probe;
        bar_probe[4] = state.pci.bar4_probe;

        self.config
            .restore_state(&aero_devices::pci::PciConfigSpaceState {
                bytes: state.pci.regs,
                bar_base,
                bar_probe,
            });

        // Older snapshots leave IDETIM at 0 (channel decode off). intelide.sys
        // then creates no channel PDOs. Re-apply the firmware default so a
        // resume still looks like QEMU/KVM's enabled PIIX3.
        let primary = self.config.read(Self::IDETIM_PRIMARY, 2) as u16;
        let secondary = self.config.read(Self::IDETIM_SECONDARY, 2) as u16;
        if primary & Self::IDETIM_DECODE_ENABLE == 0 || secondary & Self::IDETIM_DECODE_ENABLE == 0 {
            Self::enable_legacy_channels(&mut self.config);
        }

        // Keep controller and config BAR4 decode consistent.
        self.controller.bus_master_base = state.pci.bus_master_base;

        restore_channel(
            &mut self.controller.primary,
            &mut self.controller.bus_master[0],
            &state.primary,
        );
        restore_channel(
            &mut self.controller.secondary,
            &mut self.controller.bus_master[1],
            &state.secondary,
        );
        self.controller.primary.sync_irq_pin();
        self.controller.secondary.sync_irq_pin();
    }
}

impl IoSnapshot for Piix3IdePciDevice {
    const DEVICE_ID: [u8; 4] = IdeControllerState::DEVICE_ID;
    const DEVICE_VERSION: SnapshotVersion = IdeControllerState::DEVICE_VERSION;

    fn save_state(&self) -> Vec<u8> {
        self.snapshot_state().save_state()
    }

    fn load_state(&mut self, bytes: &[u8]) -> SnapshotResult<()> {
        let mut state = IdeControllerState::default();
        state.load_state(bytes)?;
        self.restore_state(&state);
        Ok(())
    }
}

impl Default for Piix3IdePciDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl PciDevice for Piix3IdePciDevice {
    fn config(&self) -> &PciConfigSpace {
        &self.config
    }

    fn config_mut(&mut self) -> &mut PciConfigSpace {
        &mut self.config
    }

    fn reset(&mut self) {
        Piix3IdePciDevice::reset(self);
    }
}

pub type SharedPiix3IdePciDevice = Rc<RefCell<Piix3IdePciDevice>>;

/// Per-port `PortIoDevice` view into a shared PIIX3 IDE controller.
pub struct Piix3IdePort {
    ide: SharedPiix3IdePciDevice,
    port: u16,
}

impl Piix3IdePort {
    pub fn new(ide: SharedPiix3IdePciDevice, port: u16) -> Self {
        Self { ide, port }
    }
}

impl PortIoDevice for Piix3IdePort {
    fn read(&mut self, port: u16, size: u8) -> u32 {
        debug_assert_eq!(port, self.port);
        self.ide.borrow_mut().io_read(port, size)
    }

    fn write(&mut self, port: u16, size: u8, value: u32) {
        debug_assert_eq!(port, self.port);
        self.ide.borrow_mut().io_write(port, size, value);
    }
}

/// Register the PIIX3 IDE controller's legacy ports + Bus Master IDE BAR on an [`IoPortBus`].
pub fn register_piix3_ide_ports(bus: &mut IoPortBus, ide: SharedPiix3IdePciDevice) {
    // Primary command block (0x1F0..=0x1F7).
    for port in PRIMARY_PORTS.cmd_base..PRIMARY_PORTS.cmd_base + 8 {
        bus.register(port, Box::new(Piix3IdePort::new(ide.clone(), port)));
    }
    // Primary control block (alt-status/dev-ctl + drive address): 0x3F6..=0x3F7.
    for port in PRIMARY_PORTS.ctrl_base..PRIMARY_PORTS.ctrl_base + 2 {
        bus.register(port, Box::new(Piix3IdePort::new(ide.clone(), port)));
    }

    // Secondary command block (0x170..=0x177).
    for port in SECONDARY_PORTS.cmd_base..SECONDARY_PORTS.cmd_base + 8 {
        bus.register(port, Box::new(Piix3IdePort::new(ide.clone(), port)));
    }
    // Secondary control block: 0x376..=0x377.
    for port in SECONDARY_PORTS.ctrl_base..SECONDARY_PORTS.ctrl_base + 2 {
        bus.register(port, Box::new(Piix3IdePort::new(ide.clone(), port)));
    }

    // Bus Master IDE (BAR4): 16 bytes, both channels.
    let bm_base = ide.borrow().bus_master_base();
    // Use saturating arithmetic so a pathological BAR base near `u16::MAX` can't overflow/panic
    // under overflow-check builds (e.g. fuzzing). If the region would extend past 0xFFFF, we only
    // register ports up to `u16::MAX` (there is no wraparound in the x86 I/O port space).
    for port in bm_base..=bm_base.saturating_add(15) {
        bus.register(port, Box::new(Piix3IdePort::new(ide.clone(), port)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aero_storage::{MemBackend, RawDisk, VirtualDisk};
    use memory::{Bus, MemoryBus};

    struct TestIsoBackend {
        image: Vec<u8>,
    }

    impl IsoBackend for TestIsoBackend {
        fn sector_count(&self) -> u32 {
            let sectors = self.image.len() / AtapiCdrom::SECTOR_SIZE;
            sectors as u32
        }

        fn read_sectors(&mut self, lba: u32, buf: &mut [u8]) -> io::Result<()> {
            if !buf.len().is_multiple_of(AtapiCdrom::SECTOR_SIZE) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unaligned buffer length",
                ));
            }

            let start = usize::try_from(lba)
                .ok()
                .and_then(|v| v.checked_mul(AtapiCdrom::SECTOR_SIZE))
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "offset overflow"))?;
            let end = start
                .checked_add(buf.len())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "offset overflow"))?;
            if end > self.image.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "read beyond end of ISO image",
                ));
            }

            buf.copy_from_slice(&self.image[start..end]);
            Ok(())
        }
    }

    #[test]
    fn port_io_size0_is_noop() {
        let mut ctl = IdeController::new(0xFFF0);

        // Pretend an ATA master is present so Status reads would normally clear IRQ state.
        ctl.primary.drive_present[0] = true;
        ctl.primary.irq_pending = true;

        let status_port = ctl.primary.ports.cmd_base + ATA_REG_STATUS_COMMAND;
        let device_port = ctl.primary.ports.cmd_base + ATA_REG_DEVICE;

        // Sanity: a normal status read clears IRQ.
        let _ = ctl.io_read(status_port, 1);
        assert!(
            !ctl.primary.irq_pending,
            "sanity check failed: STATUS read should clear IRQ"
        );

        // Re-assert and ensure a size-0 read is a true no-op.
        ctl.primary.irq_pending = true;
        assert_eq!(ctl.io_read(status_port, 0), 0);
        assert!(
            ctl.primary.irq_pending,
            "size-0 STATUS read must not clear IRQ"
        );

        // Size-0 writes must not update registers (e.g. drive select).
        let before_device = ctl.primary.tf.device;
        ctl.io_write(device_port, 0, 0xE0);
        assert_eq!(
            ctl.primary.tf.device, before_device,
            "size-0 write must not update taskfile registers"
        );
    }

    #[test]
    fn ide_controller_reset_clears_control_registers_and_bus_master_state() {
        let mut ctl = IdeController::new(0xFFF0);

        // Seed device control registers; `Channel::reset()` does not clear these, so the
        // controller-level reset must.
        ctl.primary.control = 0xAA;
        ctl.secondary.control = 0x55;

        // Seed Bus Master register blocks with non-zero command + PRD pointers.
        ctl.bus_master[0].set_drive_dma_capable(0, true);
        ctl.bus_master[1].set_drive_dma_capable(1, true);
        ctl.bus_master[0].write(0, 1, 0x09);
        ctl.bus_master[0].write(4, 4, 0x1234_5678);
        ctl.bus_master[1].write(0, 1, 0x09);
        ctl.bus_master[1].write(4, 4, 0x8765_4321);

        ctl.reset();

        assert_eq!(ctl.primary.control, 0);
        assert_eq!(ctl.secondary.control, 0);

        assert_eq!(ctl.bus_master[0].read(0, 1), 0);
        assert_eq!(ctl.bus_master[0].read(4, 4), 0);
        assert_eq!(ctl.bus_master[1].read(0, 1), 0);
        assert_eq!(ctl.bus_master[1].read(4, 4), 0);

        // DMA capability bits should remain stable across controller resets.
        assert_eq!(ctl.bus_master[0].read(2, 1) & (1 << 5), 1 << 5);
        assert_eq!(ctl.bus_master[1].read(2, 1) & (1 << 6), 1 << 6);
    }

    #[test]
    fn pio_out_allocation_failure_aborts_command_instead_of_panicking() {
        let mut chan = Channel::new(PRIMARY_PORTS);

        // Seed in-flight state that should get cleared by `abort_command`.
        chan.status = IDE_STATUS_BSY | IDE_STATUS_DRQ;
        chan.data_mode = DataMode::PioIn;
        chan.transfer_kind = Some(TransferKind::AtaPioRead);
        chan.data = vec![1, 2, 3];
        chan.data_index = 1;
        chan.irq_pending = false;
        chan.pending_dma = Some(DmaRequest::ata_read(vec![0xAA]));
        chan.pio_write = Some((0x1234, 1));

        // Use a length that deterministically fails `try_reserve_exact` with a capacity overflow,
        // without actually attempting to allocate an enormous buffer.
        chan.begin_pio_out(TransferKind::AtaPioWrite, usize::MAX);

        assert_eq!(chan.data_mode, DataMode::None);
        assert_eq!(chan.transfer_kind, None);
        assert!(chan.data.is_empty());
        assert_eq!(chan.data_index, 0);
        assert!(chan.pending_dma.is_none());
        assert!(chan.pio_write.is_none());

        assert_eq!(chan.error, 0x04);
        assert_ne!(chan.status & IDE_STATUS_ERR, 0);
        assert_ne!(chan.status & IDE_STATUS_DRDY, 0);
        assert_eq!(chan.status & IDE_STATUS_BSY, 0);
        assert_eq!(chan.status & IDE_STATUS_DRQ, 0);
        assert!(chan.irq_pending);
    }

    #[test]
    fn drive_address_reports_master_present_slave_absent() {
        let mut ctl = IdeController::new(0xFFF0);
        ctl.primary.drive_present[0] = true;
        ctl.primary.drive_present[1] = false;

        let drive_addr_port = ctl.primary.ports.ctrl_base + ATA_CTRL_DRIVE_ADDRESS;
        let val = ctl.io_read(drive_addr_port, 1) as u8;

        // Master selected, head=0.
        assert_eq!(val, 0xEF);
    }

    #[test]
    fn drive_address_slave_selected_without_device_floats_bus_high() {
        let mut ctl = IdeController::new(0xFFF0);
        ctl.primary.drive_present[0] = true;
        ctl.primary.drive_present[1] = false;

        let device_port = ctl.primary.ports.cmd_base + ATA_REG_DEVICE;
        // Select the slave drive (bit 4). Use a realistic value as written by many guests.
        ctl.io_write(device_port, 1, 0xB0);

        let drive_addr_port = ctl.primary.ports.ctrl_base + ATA_CTRL_DRIVE_ADDRESS;
        let val = ctl.io_read(drive_addr_port, 1) as u8;
        assert_eq!(val, 0xFF);

        // Other control reads should still float high when the selected device is absent.
        let alt_status_port = ctl.primary.ports.ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;
        assert_eq!(ctl.io_read(alt_status_port, 1) as u8, 0xFF);
    }

    #[test]
    fn drive_address_reports_both_master_and_slave_present() {
        let mut ctl = IdeController::new(0xFFF0);
        ctl.primary.drive_present[0] = true;
        ctl.primary.drive_present[1] = true;

        // Select slave, head=0.
        let device_port = ctl.primary.ports.cmd_base + ATA_REG_DEVICE;
        ctl.io_write(device_port, 1, 0xB0);

        let drive_addr_port = ctl.primary.ports.ctrl_base + ATA_CTRL_DRIVE_ADDRESS;
        let val = ctl.io_read(drive_addr_port, 1) as u8;

        assert_eq!(val, 0xDF);
    }

    #[test]
    fn drive_address_floats_bus_high_when_channel_has_no_devices() {
        let mut ctl = IdeController::new(0xFFF0);
        assert!(!ctl.primary.drive_present[0]);
        assert!(!ctl.primary.drive_present[1]);

        let drive_addr_port = ctl.primary.ports.ctrl_base + ATA_CTRL_DRIVE_ADDRESS;
        assert_eq!(ctl.io_read(drive_addr_port, 1) as u8, 0xFF);
        assert_eq!(ctl.io_read(drive_addr_port, 2) as u16, 0xFFFF);
        assert_eq!(ctl.io_read(drive_addr_port, 4), 0xFFFF_FFFF);
    }

    #[test]
    fn drive_address_reflects_head_bits_from_device_reg() {
        let mut ctl = IdeController::new(0xFFF0);
        // Mark both drives present so DADR can be read for both master+slave selections.
        ctl.primary.drive_present[0] = true;
        ctl.primary.drive_present[1] = true;

        let device_port = ctl.primary.ports.cmd_base + ATA_REG_DEVICE;
        // Select master with head=7 (low nibble).
        let head = 7u8;
        ctl.io_write(device_port, 1, u32::from(0xE0 | head));

        let drive_addr_port = ctl.primary.ports.ctrl_base + ATA_CTRL_DRIVE_ADDRESS;
        let val = ctl.io_read(drive_addr_port, 1) as u8;
        // DADR encodes the active-low drive/head select lines (nDS1/nDS0/nHS3..nHS0).
        assert_eq!(val & 0xC0, 0xC0);
        assert_eq!(val & 0x30, 0x20, "master selected => nDS1=1, nDS0=0");
        assert_eq!(val & 0x0F, (!head) & 0x0F);

        // Select slave with head=5.
        let head = 5u8;
        ctl.io_write(device_port, 1, u32::from(0xF0 | head));
        let val = ctl.io_read(drive_addr_port, 1) as u8;
        assert_eq!(val & 0xC0, 0xC0);
        assert_eq!(val & 0x30, 0x10, "slave selected => nDS1=0, nDS0=1");
        assert_eq!(val & 0x0F, (!head) & 0x0F);
    }

    #[test]
    fn drive_address_read_does_not_clear_irq_or_mutate_taskfile() {
        let mut ctl = IdeController::new(0xFFF0);
        ctl.primary.drive_present[0] = true;

        let device_port = ctl.primary.ports.cmd_base + ATA_REG_DEVICE;
        // Select master, head=7.
        ctl.io_write(device_port, 1, 0xE7);

        ctl.primary.irq_pending = true;
        let before_device = ctl.primary.tf.device;

        let drive_addr_port = ctl.primary.ports.ctrl_base + ATA_CTRL_DRIVE_ADDRESS;
        let first = ctl.io_read(drive_addr_port, 1);
        let second = ctl.io_read(drive_addr_port, 1);

        assert_eq!(first, second, "DADR reads must be stable");
        assert_eq!(
            first as u8, 0xE8,
            "sanity check failed: master head=7 should yield 0xE8"
        );
        assert_eq!(
            ctl.primary.tf.device, before_device,
            "DADR read must not mutate taskfile registers"
        );
        assert!(
            ctl.primary.irq_pending,
            "DADR read must not clear the channel IRQ latch"
        );
    }

    #[test]
    fn atapi_read10_uses_bus_master_dma_and_raises_irq() {
        const BM_BASE: u16 = 0xC000;
        const PRD_BASE: u64 = 0x1000;
        const BUF_BASE: u64 = 0x2000;

        let mut expected = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        for (i, b) in expected.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }

        let mut cd = AtapiCdrom::new(Some(Box::new(TestIsoBackend {
            image: expected.clone(),
        })));

        // The model reports Unit Attention on the first command after media insertion (similar to
        // real hardware). Clear it so the subsequent READ(10) succeeds.
        let tur = [0u8; 12];
        let _ = cd.handle_packet(&tur, false);

        let mut ctl = IdeController::new(BM_BASE);
        ctl.attach_primary_master_atapi(cd);

        // Guest RAM: PRD table + data buffer.
        let mut mem = Bus::new(0x10_000);

        // Single-entry PRD pointing at a 2048-byte guest buffer with EOT set.
        mem.write_u32(PRD_BASE, BUF_BASE as u32);
        mem.write_u16(PRD_BASE + 4, AtapiCdrom::SECTOR_SIZE as u16);
        mem.write_u16(PRD_BASE + 6, 0x8000);

        // Program Bus Master IDE registers: PRD pointer + direction=ToMemory + start.
        ctl.io_write(BM_BASE + 4, 4, PRD_BASE as u32);
        ctl.io_write(BM_BASE, 1, 0x09);

        // Issue ATAPI PACKET command with the DMA bit set in Features.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        let ctrl_base = PRIMARY_PORTS.ctrl_base;

        let data_port = cmd_base + ATA_REG_DATA;
        let features_port = cmd_base + ATA_REG_ERROR_FEATURES;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let lba2_port = cmd_base + ATA_REG_LBA2;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;
        let alt_status_port = ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;

        // Select primary master.
        ctl.io_write(device_port, 1, 0xA0);
        // Request DMA for PACKET transfers (Features bit 0).
        ctl.io_write(features_port, 1, 0x01);
        // Byte count (2048) for the command (LBA Mid/High).
        ctl.io_write(lba1_port, 1, 0x00);
        ctl.io_write(lba2_port, 1, 0x08);

        ctl.io_write(command_port, 1, 0xA0);

        // PACKET phase asserts DRQ so the host can write the CDB. QEMU does
        // not raise INTRQ here; Win7 ataport polls.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(st & IDE_STATUS_DRQ, 0);
        assert_ne!(st & IDE_STATUS_DRDY, 0);
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert!(
            !ctl.primary_irq_pending(),
            "PACKET CDB phase must not raise IRQ (QEMU/Win7)"
        );

        // Build an ATAPI READ(10) packet for LBA=0, blocks=1.
        let mut pkt = [0u8; 12];
        pkt[0] = 0x28; // READ(10)
        pkt[7..9].copy_from_slice(&1u16.to_be_bytes());

        // Write the packet via the data register (PIO-out, 16-bit words).
        for chunk in pkt.chunks_exact(2) {
            let w = u16::from_le_bytes([chunk[0], chunk[1]]);
            ctl.io_write(data_port, 2, w as u32);
        }

        // After the packet phase, the device should have queued a DMA request rather than entering
        // a PIO data-in phase.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_eq!(st & IDE_STATUS_DRQ, 0);
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert_ne!(st & IDE_STATUS_DRDY, 0);
        assert!(!ctl.primary_irq_pending());
        assert!(ctl.primary.pending_dma.is_some());

        // Run the synchronous DMA engine.
        ctl.tick(&mut mem);

        // DMA should have written the expected 2048-byte sector into guest memory.
        let mut actual = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        mem.read_physical(BUF_BASE, &mut actual);
        assert_eq!(actual, expected);

        // Controller should end in status phase: DRQ cleared and IRQ pending.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_eq!(st & IDE_STATUS_DRQ, 0);
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert_eq!(st & IDE_STATUS_ERR, 0);
        assert!(ctl.primary_irq_pending());

        // Bus Master status should indicate interrupt and no error.
        let bm_st = ctl.io_read(BM_BASE + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x04, "bus master status: {bm_st:#04x}");
        assert_ne!(bm_st & (1 << 5), 0, "DMA capability bit should be set");
    }

    #[test]
    fn atapi_dma_prd_missing_eot_aborts_command_and_sets_bm_error() {
        const BM_BASE: u16 = 0xC000;
        const PRD_BASE: u64 = 0x1000;
        const BUF_BASE: u64 = 0x2000;

        let mut expected = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        for (i, b) in expected.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }

        let mut cd = AtapiCdrom::new(Some(Box::new(TestIsoBackend {
            image: expected.clone(),
        })));
        // Clear Unit Attention.
        let tur = [0u8; 12];
        let _ = cd.handle_packet(&tur, false);

        let mut ctl = IdeController::new(BM_BASE);
        ctl.attach_primary_master_atapi(cd);

        let mut mem = Bus::new(0x10_000);

        // Malformed PRD: one entry long enough to cover the transfer but missing the EOT bit.
        mem.write_u32(PRD_BASE, BUF_BASE as u32);
        mem.write_u16(PRD_BASE + 4, AtapiCdrom::SECTOR_SIZE as u16);
        mem.write_u16(PRD_BASE + 6, 0x0000);

        // Seed the destination buffer so we can verify whether DMA wrote anything.
        mem.write_physical(BUF_BASE, &vec![0xFFu8; AtapiCdrom::SECTOR_SIZE]);

        // Program bus master registers: PRD pointer + direction=ToMemory + start.
        ctl.io_write(BM_BASE + 4, 4, PRD_BASE as u32);
        ctl.io_write(BM_BASE, 1, 0x09);

        // Issue ATAPI PACKET command with DMA enabled in Features.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        let ctrl_base = PRIMARY_PORTS.ctrl_base;

        let data_port = cmd_base + ATA_REG_DATA;
        let features_port = cmd_base + ATA_REG_ERROR_FEATURES;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let lba2_port = cmd_base + ATA_REG_LBA2;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;
        let alt_status_port = ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;

        ctl.io_write(device_port, 1, 0xA0);
        ctl.io_write(features_port, 1, 0x01); // DMA
        ctl.io_write(lba1_port, 1, 0x00);
        ctl.io_write(lba2_port, 1, 0x08); // 2048-byte packet byte count
        ctl.io_write(command_port, 1, 0xA0); // PACKET

        // PACKET phase asserts DRQ so the host can write the CDB. No INTRQ.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(st & IDE_STATUS_DRQ, 0);
        assert_ne!(st & IDE_STATUS_DRDY, 0);
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert!(!ctl.primary_irq_pending());

        // READ(10) packet (LBA=0, blocks=1).
        let mut pkt = [0u8; 12];
        pkt[0] = 0x28;
        pkt[7..9].copy_from_slice(&1u16.to_be_bytes());
        for chunk in pkt.chunks_exact(2) {
            let w = u16::from_le_bytes([chunk[0], chunk[1]]);
            ctl.io_write(data_port, 2, w as u32);
        }

        assert!(ctl.primary.pending_dma.is_some());

        // Run DMA; it should error due to malformed PRD table.
        ctl.tick(&mut mem);

        assert!(
            ctl.primary_irq_pending(),
            "IRQ should be pending after DMA error"
        );
        let bm_st = ctl.io_read(BM_BASE + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06, "BMIDE status should have IRQ+ERR set");

        let err = ctl.io_read(cmd_base + ATA_REG_ERROR_FEATURES, 1) as u8;
        assert_eq!(err, 0x04, "expected ABRT after DMA failure");

        // ATAPI uses Sector Count as interrupt reason; errors should still transition to status
        // phase (IO=1, CoD=1).
        let irq_reason = ctl.io_read(cmd_base + ATA_REG_SECTOR_COUNT, 1) as u8;
        assert_eq!(
            irq_reason, 0x03,
            "expected ATAPI status phase after DMA failure"
        );

        // Use ALT_STATUS so we don't accidentally clear the interrupt.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(
            st & IDE_STATUS_ERR,
            0,
            "ERR bit should be set after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_BSY,
            0,
            "BSY should be clear after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_DRQ,
            0,
            "DRQ should be clear after DMA failure"
        );
        assert_ne!(
            st & IDE_STATUS_DRDY,
            0,
            "DRDY should be set after DMA failure"
        );

        // Even though the PRD table is malformed, the DMA engine should still have written the
        // data before detecting the missing EOT bit.
        let mut actual = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        mem.read_physical(BUF_BASE, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn atapi_dma_error_on_secondary_channel_sets_secondary_bm_status_and_irq_only() {
        const BM_BASE: u16 = 0xC000;
        const PRD_BASE: u64 = 0x1000;
        const BUF_BASE: u64 = 0x2000;

        let mut expected = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        for (i, b) in expected.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }

        let mut cd = AtapiCdrom::new(Some(Box::new(TestIsoBackend {
            image: expected.clone(),
        })));
        // Clear Unit Attention.
        let tur = [0u8; 12];
        let _ = cd.handle_packet(&tur, false);

        let mut ctl = IdeController::new(BM_BASE);
        ctl.attach_secondary_master_atapi(cd);

        let mut mem = Bus::new(0x10_000);

        // Malformed PRD: one entry long enough to cover the transfer but missing the EOT bit.
        mem.write_u32(PRD_BASE, BUF_BASE as u32);
        mem.write_u16(PRD_BASE + 4, AtapiCdrom::SECTOR_SIZE as u16);
        mem.write_u16(PRD_BASE + 6, 0x0000);

        // Program secondary bus master registers: PRD pointer + direction=ToMemory + start.
        ctl.io_write(BM_BASE + 8 + 4, 4, PRD_BASE as u32);
        ctl.io_write(BM_BASE + 8, 1, 0x09);

        // Issue ATAPI PACKET command with DMA enabled in Features on the secondary channel.
        let cmd_base = SECONDARY_PORTS.cmd_base;
        let ctrl_base = SECONDARY_PORTS.ctrl_base;

        let data_port = cmd_base + ATA_REG_DATA;
        let features_port = cmd_base + ATA_REG_ERROR_FEATURES;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let lba2_port = cmd_base + ATA_REG_LBA2;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;
        let alt_status_port = ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;

        ctl.io_write(device_port, 1, 0xA0);
        ctl.io_write(features_port, 1, 0x01); // DMA
        ctl.io_write(lba1_port, 1, 0x00);
        ctl.io_write(lba2_port, 1, 0x08); // 2048-byte packet byte count
        ctl.io_write(command_port, 1, 0xA0); // PACKET

        // PACKET phase asserts DRQ so the host can write the CDB. No INTRQ.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(st & IDE_STATUS_DRQ, 0);
        assert_ne!(st & IDE_STATUS_DRDY, 0);
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert!(!ctl.secondary_irq_pending());
        assert!(!ctl.primary_irq_pending());

        // READ(10) packet (LBA=0, blocks=1).
        let mut pkt = [0u8; 12];
        pkt[0] = 0x28;
        pkt[7..9].copy_from_slice(&1u16.to_be_bytes());
        for chunk in pkt.chunks_exact(2) {
            let w = u16::from_le_bytes([chunk[0], chunk[1]]);
            ctl.io_write(data_port, 2, w as u32);
        }

        assert!(ctl.secondary.pending_dma.is_some());

        // Run DMA; it should error due to malformed PRD table.
        ctl.tick(&mut mem);

        assert!(ctl.secondary_irq_pending());
        assert!(!ctl.primary_irq_pending());

        let bm_st_primary = ctl.io_read(BM_BASE + 2, 1) as u8;
        assert_eq!(
            bm_st_primary & 0x07,
            0,
            "primary BMIDE status should be unaffected"
        );

        let bm_st_secondary = ctl.io_read(BM_BASE + 8 + 2, 1) as u8;
        assert_eq!(
            bm_st_secondary & 0x07,
            0x06,
            "secondary BMIDE status should have IRQ+ERR set"
        );
        assert_ne!(
            bm_st_secondary & (1 << 5),
            0,
            "secondary DMA capability bit should be set"
        );

        // ATAPI uses Sector Count as interrupt reason; errors should transition to status phase.
        let irq_reason = ctl.io_read(cmd_base + ATA_REG_SECTOR_COUNT, 1) as u8;
        assert_eq!(irq_reason, 0x03);

        // Even though the PRD table is malformed, the DMA engine should still have written the
        // data before detecting the missing EOT bit.
        let mut actual = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        mem.read_physical(BUF_BASE, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn atapi_dma_error_irq_is_latched_while_nien_is_set_and_surfaces_after_reenable() {
        const BM_BASE: u16 = 0xC000;
        const PRD_BASE: u64 = 0x1000;
        const BUF_BASE: u64 = 0x2000;

        let mut expected = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        for (i, b) in expected.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }

        let mut cd = AtapiCdrom::new(Some(Box::new(TestIsoBackend {
            image: expected.clone(),
        })));
        // Clear Unit Attention.
        let tur = [0u8; 12];
        let _ = cd.handle_packet(&tur, false);

        let mut ctl = IdeController::new(BM_BASE);
        ctl.attach_primary_master_atapi(cd);

        let mut mem = Bus::new(0x10_000);

        // Malformed PRD: one entry long enough to cover the transfer but missing the EOT bit.
        mem.write_u32(PRD_BASE, BUF_BASE as u32);
        mem.write_u16(PRD_BASE + 4, AtapiCdrom::SECTOR_SIZE as u16);
        mem.write_u16(PRD_BASE + 6, 0x0000);

        // Seed the destination buffer.
        mem.write_physical(BUF_BASE, &vec![0xFFu8; AtapiCdrom::SECTOR_SIZE]);

        // Program bus master registers: PRD pointer + direction=ToMemory + start.
        ctl.io_write(BM_BASE + 4, 4, PRD_BASE as u32);
        ctl.io_write(BM_BASE, 1, 0x09);

        // Issue ATAPI PACKET command with DMA enabled in Features.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        let ctrl_base = PRIMARY_PORTS.ctrl_base;

        let data_port = cmd_base + ATA_REG_DATA;
        let features_port = cmd_base + ATA_REG_ERROR_FEATURES;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let lba2_port = cmd_base + ATA_REG_LBA2;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;
        let alt_status_port = ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;

        ctl.io_write(device_port, 1, 0xA0);
        ctl.io_write(features_port, 1, 0x01); // DMA
        ctl.io_write(lba1_port, 1, 0x00);
        ctl.io_write(lba2_port, 1, 0x08); // 2048-byte packet byte count
        ctl.io_write(command_port, 1, 0xA0); // PACKET

        // PACKET phase asserts DRQ so the host can write the CDB. No INTRQ.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(st & IDE_STATUS_DRQ, 0);
        assert_ne!(st & IDE_STATUS_DRDY, 0);
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert!(!ctl.primary_irq_pending());

        // Build an ATAPI READ(10) packet for LBA=0, blocks=1.
        let mut pkt = [0u8; 12];
        pkt[0] = 0x28;
        pkt[7..9].copy_from_slice(&1u16.to_be_bytes());
        for chunk in pkt.chunks_exact(2) {
            let w = u16::from_le_bytes([chunk[0], chunk[1]]);
            ctl.io_write(data_port, 2, w as u32);
        }

        // Mask IDE interrupts before running DMA; completion should latch irq_pending.
        let ctrl_port = ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;
        ctl.io_write(ctrl_port, 1, u32::from(IDE_CTRL_NIEN));

        ctl.tick(&mut mem);

        assert!(
            ctl.primary.irq_pending,
            "DMA error should set irq_pending even when nIEN masks output"
        );
        assert!(!ctl.primary_irq_pending());

        // Re-enable interrupts and ensure the pending IRQ now surfaces.
        ctl.io_write(ctrl_port, 1, 0);
        assert!(ctl.primary_irq_pending());

        // Reading STATUS acknowledges/clears it.
        let _ = ctl.io_read(command_port, 1);
        assert!(!ctl.primary_irq_pending());

        // Use ALT_STATUS so we don't clear the IRQ again by accident.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(st & IDE_STATUS_ERR, 0);

        let bm_st = ctl.io_read(BM_BASE + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06);
    }

    #[test]
    fn atapi_dma_error_irq_can_be_acknowledged_while_nien_is_set() {
        const BM_BASE: u16 = 0xC000;
        const PRD_BASE: u64 = 0x1000;
        const BUF_BASE: u64 = 0x2000;

        let mut expected = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        for (i, b) in expected.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }

        let mut cd = AtapiCdrom::new(Some(Box::new(TestIsoBackend {
            image: expected.clone(),
        })));
        // Clear Unit Attention.
        let tur = [0u8; 12];
        let _ = cd.handle_packet(&tur, false);

        let mut ctl = IdeController::new(BM_BASE);
        ctl.attach_primary_master_atapi(cd);

        let mut mem = Bus::new(0x10_000);

        // Malformed PRD: one entry long enough to cover the transfer but missing the EOT bit.
        mem.write_u32(PRD_BASE, BUF_BASE as u32);
        mem.write_u16(PRD_BASE + 4, AtapiCdrom::SECTOR_SIZE as u16);
        mem.write_u16(PRD_BASE + 6, 0x0000);

        // Program bus master registers: PRD pointer + direction=ToMemory + start.
        ctl.io_write(BM_BASE + 4, 4, PRD_BASE as u32);
        ctl.io_write(BM_BASE, 1, 0x09);

        // Issue ATAPI PACKET command with DMA enabled in Features.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        let ctrl_base = PRIMARY_PORTS.ctrl_base;

        let data_port = cmd_base + ATA_REG_DATA;
        let features_port = cmd_base + ATA_REG_ERROR_FEATURES;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let lba2_port = cmd_base + ATA_REG_LBA2;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;
        let alt_status_port = ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;

        ctl.io_write(device_port, 1, 0xA0);
        ctl.io_write(features_port, 1, 0x01); // DMA
        ctl.io_write(lba1_port, 1, 0x00);
        ctl.io_write(lba2_port, 1, 0x08); // 2048-byte packet byte count
        ctl.io_write(command_port, 1, 0xA0); // PACKET

        // Acknowledge the PACKET-phase IRQ.
        let _ = ctl.io_read(command_port, 1);

        // READ(10) packet (LBA=0, blocks=1).
        let mut pkt = [0u8; 12];
        pkt[0] = 0x28;
        pkt[7..9].copy_from_slice(&1u16.to_be_bytes());
        for chunk in pkt.chunks_exact(2) {
            let w = u16::from_le_bytes([chunk[0], chunk[1]]);
            ctl.io_write(data_port, 2, w as u32);
        }

        // Mask interrupts before running DMA.
        let ctrl_port = ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;
        ctl.io_write(ctrl_port, 1, u32::from(IDE_CTRL_NIEN));

        ctl.tick(&mut mem);

        assert!(ctl.primary.irq_pending);
        assert!(!ctl.primary_irq_pending());

        // Guest polls by reading STATUS even while interrupts are masked; this should acknowledge
        // and clear the pending interrupt condition.
        let _ = ctl.io_read(command_port, 1);
        assert!(!ctl.primary.irq_pending);

        // Re-enabling interrupts should *not* surface an interrupt now that the guest has acked it.
        ctl.io_write(ctrl_port, 1, 0);
        assert!(!ctl.primary_irq_pending());

        // Use ALT_STATUS so we don't accidentally clear IRQ state again.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(st & IDE_STATUS_ERR, 0);

        // Bus master status remains set until the guest clears it explicitly.
        let bm_st = ctl.io_read(BM_BASE + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06);
    }

    #[test]
    fn atapi_dma_prd_too_short_aborts_command_and_partially_transfers_data() {
        const BM_BASE: u16 = 0xC000;
        const PRD_BASE: u64 = 0x1000;
        const BUF_BASE: u64 = 0x2000;

        let mut expected = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        for (i, b) in expected.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }

        let mut cd = AtapiCdrom::new(Some(Box::new(TestIsoBackend {
            image: expected.clone(),
        })));
        // Clear Unit Attention.
        let tur = [0u8; 12];
        let _ = cd.handle_packet(&tur, false);

        let mut ctl = IdeController::new(BM_BASE);
        ctl.attach_primary_master_atapi(cd);

        let mut mem = Bus::new(0x10_000);

        // Malformed PRD: EOT is set but the entry is too short to cover the 2048-byte transfer.
        const PRD_LEN: u16 = 1024;
        mem.write_u32(PRD_BASE, BUF_BASE as u32);
        mem.write_u16(PRD_BASE + 4, PRD_LEN);
        mem.write_u16(PRD_BASE + 6, 0x8000);

        // Seed the destination buffer; the DMA transfer should update only the first PRD_LEN bytes.
        mem.write_physical(BUF_BASE, &vec![0xFFu8; AtapiCdrom::SECTOR_SIZE]);

        // Program bus master registers: PRD pointer + direction=ToMemory + start.
        ctl.io_write(BM_BASE + 4, 4, PRD_BASE as u32);
        ctl.io_write(BM_BASE, 1, 0x09);

        // Issue ATAPI PACKET command with DMA enabled in Features.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        let ctrl_base = PRIMARY_PORTS.ctrl_base;

        let data_port = cmd_base + ATA_REG_DATA;
        let features_port = cmd_base + ATA_REG_ERROR_FEATURES;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let lba2_port = cmd_base + ATA_REG_LBA2;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;
        let alt_status_port = ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;

        ctl.io_write(device_port, 1, 0xA0);
        ctl.io_write(features_port, 1, 0x01); // DMA
        ctl.io_write(lba1_port, 1, 0x00);
        ctl.io_write(lba2_port, 1, 0x08); // 2048-byte packet byte count
        ctl.io_write(command_port, 1, 0xA0); // PACKET

        // PACKET phase asserts DRQ so the host can write the CDB. No INTRQ.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(st & IDE_STATUS_DRQ, 0);
        assert_ne!(st & IDE_STATUS_DRDY, 0);
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert!(!ctl.primary_irq_pending());

        // READ(10) packet (LBA=0, blocks=1).
        let mut pkt = [0u8; 12];
        pkt[0] = 0x28;
        pkt[7..9].copy_from_slice(&1u16.to_be_bytes());
        for chunk in pkt.chunks_exact(2) {
            let w = u16::from_le_bytes([chunk[0], chunk[1]]);
            ctl.io_write(data_port, 2, w as u32);
        }

        assert!(ctl.primary.pending_dma.is_some());

        // Run DMA; it should error due to the PRD being too short.
        ctl.tick(&mut mem);

        assert!(
            ctl.primary_irq_pending(),
            "IRQ should be pending after DMA error"
        );
        assert!(
            ctl.primary.pending_dma.is_none(),
            "DMA request should be consumed on error"
        );

        let bm_st = ctl.io_read(BM_BASE + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06, "BMIDE status should have IRQ+ERR set");

        let err = ctl.io_read(cmd_base + ATA_REG_ERROR_FEATURES, 1) as u8;
        assert_eq!(err, 0x04, "expected ABRT after DMA failure");

        let irq_reason = ctl.io_read(cmd_base + ATA_REG_SECTOR_COUNT, 1) as u8;
        assert_eq!(
            irq_reason, 0x03,
            "expected ATAPI status phase after DMA failure"
        );

        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(
            st & IDE_STATUS_ERR,
            0,
            "ERR bit should be set after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_BSY,
            0,
            "BSY should be clear after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_DRQ,
            0,
            "DRQ should be clear after DMA failure"
        );
        assert_ne!(
            st & IDE_STATUS_DRDY,
            0,
            "DRDY should be set after DMA failure"
        );

        // Verify partial transfer semantics.
        let mut actual = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        mem.read_physical(BUF_BASE, &mut actual);
        let prd_len = PRD_LEN as usize;
        assert_eq!(&actual[..prd_len], &expected[..prd_len]);
        assert!(
            actual[prd_len..].iter().all(|&b| b == 0xFF),
            "expected remaining bytes to remain untouched"
        );
    }

    #[test]
    fn atapi_dma_direction_mismatch_aborts_command_and_does_not_transfer_data() {
        const BM_BASE: u16 = 0xC000;
        const PRD_BASE: u64 = 0x1000;
        const BUF_BASE: u64 = 0x2000;

        let mut expected = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        for (i, b) in expected.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }

        let mut cd = AtapiCdrom::new(Some(Box::new(TestIsoBackend {
            image: expected.clone(),
        })));
        // Clear Unit Attention.
        let tur = [0u8; 12];
        let _ = cd.handle_packet(&tur, false);

        let mut ctl = IdeController::new(BM_BASE);
        ctl.attach_primary_master_atapi(cd);

        let mut mem = Bus::new(0x10_000);

        // Valid PRD entry covering the entire transfer (EOT set).
        mem.write_u32(PRD_BASE, BUF_BASE as u32);
        mem.write_u16(PRD_BASE + 4, AtapiCdrom::SECTOR_SIZE as u16);
        mem.write_u16(PRD_BASE + 6, 0x8000);

        // Seed the destination buffer; a direction mismatch should prevent any DMA writes.
        mem.write_physical(BUF_BASE, &vec![0xFFu8; AtapiCdrom::SECTOR_SIZE]);

        // Program bus master registers with *wrong* direction: FromMemory (bit3 clear) + start.
        ctl.io_write(BM_BASE + 4, 4, PRD_BASE as u32);
        ctl.io_write(BM_BASE, 1, 0x01);

        // Issue ATAPI PACKET command with DMA enabled in Features.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        let ctrl_base = PRIMARY_PORTS.ctrl_base;

        let data_port = cmd_base + ATA_REG_DATA;
        let features_port = cmd_base + ATA_REG_ERROR_FEATURES;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let lba2_port = cmd_base + ATA_REG_LBA2;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;
        let alt_status_port = ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;

        ctl.io_write(device_port, 1, 0xA0);
        ctl.io_write(features_port, 1, 0x01); // DMA
        ctl.io_write(lba1_port, 1, 0x00);
        ctl.io_write(lba2_port, 1, 0x08); // 2048-byte packet byte count
        ctl.io_write(command_port, 1, 0xA0); // PACKET

        // PACKET phase asserts DRQ so the host can write the CDB. No INTRQ.
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(st & IDE_STATUS_DRQ, 0);
        assert_ne!(st & IDE_STATUS_DRDY, 0);
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert!(!ctl.primary_irq_pending());

        // READ(10) packet (LBA=0, blocks=1).
        let mut pkt = [0u8; 12];
        pkt[0] = 0x28;
        pkt[7..9].copy_from_slice(&1u16.to_be_bytes());
        for chunk in pkt.chunks_exact(2) {
            let w = u16::from_le_bytes([chunk[0], chunk[1]]);
            ctl.io_write(data_port, 2, w as u32);
        }

        assert!(ctl.primary.pending_dma.is_some());

        // Run DMA; it should error due to direction mismatch.
        ctl.tick(&mut mem);

        assert!(
            ctl.primary_irq_pending(),
            "IRQ should be pending after DMA error"
        );
        assert!(
            ctl.primary.pending_dma.is_none(),
            "DMA request should be consumed on error"
        );

        let bm_st = ctl.io_read(BM_BASE + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06, "BMIDE status should have IRQ+ERR set");

        let err = ctl.io_read(cmd_base + ATA_REG_ERROR_FEATURES, 1) as u8;
        assert_eq!(err, 0x04, "expected ABRT after DMA failure");

        let irq_reason = ctl.io_read(cmd_base + ATA_REG_SECTOR_COUNT, 1) as u8;
        assert_eq!(
            irq_reason, 0x03,
            "expected ATAPI status phase after DMA failure"
        );

        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(
            st & IDE_STATUS_ERR,
            0,
            "ERR bit should be set after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_BSY,
            0,
            "BSY should be clear after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_DRQ,
            0,
            "DRQ should be clear after DMA failure"
        );
        assert_ne!(
            st & IDE_STATUS_DRDY,
            0,
            "DRDY should be set after DMA failure"
        );

        let mut actual = vec![0u8; AtapiCdrom::SECTOR_SIZE];
        mem.read_physical(BUF_BASE, &mut actual);
        assert!(
            actual.iter().all(|&b| b == 0xFF),
            "direction mismatch should prevent any DMA writes"
        );
    }

    fn setup_primary_ata_controller() -> IdeController {
        let capacity = SECTOR_SIZE as u64;
        let disk = RawDisk::create(MemBackend::new(), capacity).unwrap();
        let mut ctl = IdeController::new(0xFFF0);
        ctl.attach_primary_master_ata(AtaDrive::new(Box::new(disk)).unwrap());
        ctl
    }

    fn setup_primary_ata_controller_with_sector0(fill: u8) -> IdeController {
        let capacity = SECTOR_SIZE as u64;
        let mut disk = RawDisk::create(MemBackend::new(), capacity).unwrap();
        disk.write_sectors(0, &vec![fill; SECTOR_SIZE]).unwrap();
        let mut ctl = IdeController::new(0xFFF0);
        ctl.attach_primary_master_ata(AtaDrive::new(Box::new(disk)).unwrap());
        ctl
    }

    fn read_primary_sector0_via_pio(ctl: &mut IdeController) -> Vec<u8> {
        let cmd_base = PRIMARY_PORTS.cmd_base;

        // READ SECTORS for LBA 0, 1 sector.
        ctl.io_write(cmd_base + ATA_REG_DEVICE, 1, 0xE0);
        ctl.io_write(cmd_base + ATA_REG_SECTOR_COUNT, 1, 1);
        ctl.io_write(cmd_base + ATA_REG_LBA0, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA1, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA2, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_STATUS_COMMAND, 1, 0x20);

        let mut out = vec![0u8; SECTOR_SIZE];
        for i in 0..(SECTOR_SIZE / 2) {
            let w = ctl.io_read(cmd_base + ATA_REG_DATA, 2) as u16;
            out[i * 2..i * 2 + 2].copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    #[test]
    fn ata_dma_prd_missing_eot_aborts_command_and_signals_interrupt() {
        let mut ctl = setup_primary_ata_controller();
        let mut mem = Bus::new(0x8000);

        let prd_addr: u64 = 0x1000;
        let dma_buf: u64 = 0x2000;

        // PRD entry with no EOT flag (malformed), but long enough to cover the transfer.
        mem.write_u32(prd_addr, dma_buf as u32);
        mem.write_u16(prd_addr + 4, SECTOR_SIZE as u16);
        mem.write_u16(prd_addr + 6, 0x0000);

        let bm_base = ctl.bus_master_base();
        ctl.io_write(bm_base + 4, 4, prd_addr as u32);

        // READ DMA for LBA 0, 1 sector.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        ctl.io_write(cmd_base + ATA_REG_DEVICE, 1, 0xE0);
        ctl.io_write(cmd_base + ATA_REG_SECTOR_COUNT, 1, 1);
        ctl.io_write(cmd_base + ATA_REG_LBA0, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA1, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA2, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_STATUS_COMMAND, 1, 0xC8);

        // Start bus master (direction = device -> memory).
        ctl.io_write(bm_base, 1, 0x09);
        ctl.tick(&mut mem);

        // IDE channel should abort the command and raise an interrupt.
        assert!(
            ctl.primary_irq_pending(),
            "IRQ should be pending after DMA error"
        );

        let err = ctl.io_read(cmd_base + ATA_REG_ERROR_FEATURES, 1) as u8;
        assert_eq!(err, 0x04, "expected ABRT after DMA failure");

        // Use ALT_STATUS so we don't accidentally clear the interrupt.
        let st = ctl.io_read(PRIMARY_PORTS.ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL, 1) as u8;
        assert_ne!(
            st & IDE_STATUS_ERR,
            0,
            "ERR bit should be set after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_BSY,
            0,
            "BSY should be clear after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_DRQ,
            0,
            "DRQ should be clear after DMA failure"
        );
        assert_ne!(
            st & IDE_STATUS_DRDY,
            0,
            "DRDY should be set after DMA failure"
        );

        // Bus Master status should indicate interrupt + error, and clear ACTIVE.
        let bm_st = ctl.io_read(bm_base + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06, "BMIDE status should have IRQ+ERR set");

        // Reading STATUS should acknowledge/clear the IDE interrupt latch.
        let _ = ctl.io_read(cmd_base + ATA_REG_STATUS_COMMAND, 1);
        assert!(
            !ctl.primary_irq_pending(),
            "IRQ should clear when the guest reads STATUS"
        );

        // Reading STATUS does not clear the Bus Master status bits; guests clear those explicitly.
        let bm_st_after = ctl.io_read(bm_base + 2, 1) as u8;
        assert_eq!(
            bm_st_after & 0x07,
            0x06,
            "BMIDE status should remain set until guest clears it"
        );
    }

    #[test]
    fn ata_dma_error_irq_is_latched_while_nien_is_set_and_surfaces_after_reenable() {
        let mut ctl = setup_primary_ata_controller();
        let mut mem = Bus::new(0x8000);

        let prd_addr: u64 = 0x1000;
        let dma_buf: u64 = 0x2000;

        // Malformed PRD entry with no EOT flag, but long enough to cover the transfer.
        mem.write_u32(prd_addr, dma_buf as u32);
        mem.write_u16(prd_addr + 4, SECTOR_SIZE as u16);
        mem.write_u16(prd_addr + 6, 0x0000);

        let bm_base = ctl.bus_master_base();
        ctl.io_write(bm_base + 4, 4, prd_addr as u32);

        // Mask IDE interrupts via Device Control (nIEN=1).
        let ctrl_port = PRIMARY_PORTS.ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;
        ctl.io_write(ctrl_port, 1, u32::from(IDE_CTRL_NIEN));

        // READ DMA for LBA 0, 1 sector.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        ctl.io_write(cmd_base + ATA_REG_DEVICE, 1, 0xE0);
        ctl.io_write(cmd_base + ATA_REG_SECTOR_COUNT, 1, 1);
        ctl.io_write(cmd_base + ATA_REG_LBA0, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA1, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA2, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_STATUS_COMMAND, 1, 0xC8);

        ctl.io_write(bm_base, 1, 0x09);
        ctl.tick(&mut mem);

        // The interrupt should be latched internally but masked from the output.
        assert!(
            ctl.primary.irq_pending,
            "DMA error should set irq_pending even when nIEN masks output"
        );
        assert!(
            !ctl.primary_irq_pending(),
            "nIEN should mask the primary IRQ output"
        );

        // Re-enable interrupts; the pending IRQ should now surface.
        ctl.io_write(ctrl_port, 1, 0);
        assert!(
            ctl.primary_irq_pending(),
            "IRQ should surface after re-enabling"
        );

        // Reading STATUS acknowledges/clears the latch.
        let _ = ctl.io_read(cmd_base + ATA_REG_STATUS_COMMAND, 1);
        assert!(
            !ctl.primary_irq_pending(),
            "IRQ should clear when the guest reads STATUS"
        );
        assert!(!ctl.primary.irq_pending);

        // Bus Master status should still show IRQ+ERR until guest clears it explicitly.
        let bm_st = ctl.io_read(bm_base + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06);
    }

    #[test]
    fn ata_dma_error_irq_can_be_acknowledged_while_nien_is_set() {
        let mut ctl = setup_primary_ata_controller();
        let mut mem = Bus::new(0x8000);

        let prd_addr: u64 = 0x1000;
        let dma_buf: u64 = 0x2000;

        // Malformed PRD entry with no EOT flag (but long enough to cover the transfer).
        mem.write_u32(prd_addr, dma_buf as u32);
        mem.write_u16(prd_addr + 4, SECTOR_SIZE as u16);
        mem.write_u16(prd_addr + 6, 0x0000);

        let bm_base = ctl.bus_master_base();
        ctl.io_write(bm_base + 4, 4, prd_addr as u32);

        // Mask IDE interrupts via Device Control (nIEN=1).
        let ctrl_port = PRIMARY_PORTS.ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;
        ctl.io_write(ctrl_port, 1, u32::from(IDE_CTRL_NIEN));

        // READ DMA for LBA 0, 1 sector.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        ctl.io_write(cmd_base + ATA_REG_DEVICE, 1, 0xE0);
        ctl.io_write(cmd_base + ATA_REG_SECTOR_COUNT, 1, 1);
        ctl.io_write(cmd_base + ATA_REG_LBA0, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA1, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA2, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_STATUS_COMMAND, 1, 0xC8);

        ctl.io_write(bm_base, 1, 0x09);
        ctl.tick(&mut mem);

        assert!(ctl.primary.irq_pending);
        assert!(!ctl.primary_irq_pending());

        // Guest polls by reading STATUS even while interrupts are masked; this should acknowledge
        // and clear the pending interrupt condition.
        let _ = ctl.io_read(cmd_base + ATA_REG_STATUS_COMMAND, 1);
        assert!(!ctl.primary.irq_pending);

        // Re-enabling interrupts should *not* surface an interrupt now that the guest has acked it.
        ctl.io_write(ctrl_port, 1, 0);
        assert!(!ctl.primary_irq_pending());

        // Bus master status remains set until the guest clears it explicitly.
        let bm_st = ctl.io_read(bm_base + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06);
    }

    #[test]
    fn ata_dma_prd_too_short_aborts_command_and_partially_transfers_data() {
        let mut ctl = setup_primary_ata_controller();
        let mut mem = Bus::new(0x8000);

        let prd_addr: u64 = 0x1000;
        let dma_buf: u64 = 0x2000;

        // Single PRD entry with EOT set, but only 256 bytes. This is too short for a 512-byte
        // sector transfer.
        mem.write_u32(prd_addr, dma_buf as u32);
        mem.write_u16(prd_addr + 4, 256);
        mem.write_u16(prd_addr + 6, 0x8000);

        // Seed the DMA destination with a non-zero pattern so we can verify that only the covered
        // portion of the buffer was updated before we detected the malformed PRD list.
        mem.write_physical(dma_buf, &vec![0xFFu8; SECTOR_SIZE]);

        let bm_base = ctl.bus_master_base();
        ctl.io_write(bm_base + 4, 4, prd_addr as u32);

        // READ DMA for LBA 0, 1 sector.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        ctl.io_write(cmd_base + ATA_REG_DEVICE, 1, 0xE0);
        ctl.io_write(cmd_base + ATA_REG_SECTOR_COUNT, 1, 1);
        ctl.io_write(cmd_base + ATA_REG_LBA0, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA1, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA2, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_STATUS_COMMAND, 1, 0xC8);

        // Start bus master (direction = device -> memory).
        ctl.io_write(bm_base, 1, 0x09);
        ctl.tick(&mut mem);

        // IDE channel should abort the command and raise an interrupt.
        assert!(
            ctl.primary_irq_pending(),
            "IRQ should be pending after DMA error"
        );

        let err = ctl.io_read(cmd_base + ATA_REG_ERROR_FEATURES, 1) as u8;
        assert_eq!(err, 0x04, "expected ABRT after DMA failure");

        // Bus Master status should indicate interrupt + error, and clear ACTIVE.
        let bm_st = ctl.io_read(bm_base + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06, "BMIDE status should have IRQ+ERR set");

        // The transfer should have updated only the first 256 bytes.
        let mut got = vec![0u8; SECTOR_SIZE];
        mem.read_physical(dma_buf, &mut got);
        assert!(
            got[..256].iter().all(|&b| b == 0),
            "expected first 256 bytes to be transferred from disk"
        );
        assert!(
            got[256..].iter().all(|&b| b == 0xFF),
            "expected remaining bytes to remain untouched"
        );
    }

    #[test]
    fn ata_dma_direction_mismatch_aborts_command_and_sets_bm_error() {
        let mut ctl = setup_primary_ata_controller();
        let mut mem = Bus::new(0x8000);

        let prd_addr: u64 = 0x1000;
        let dma_buf: u64 = 0x2000;

        // Seed the destination buffer with a non-zero pattern; a direction mismatch should prevent
        // any DMA from occurring, leaving this data untouched.
        mem.write_physical(dma_buf, &vec![0xFFu8; SECTOR_SIZE]);

        // Valid PRD entry: one 512-byte segment, EOT set.
        mem.write_u32(prd_addr, dma_buf as u32);
        mem.write_u16(prd_addr + 4, SECTOR_SIZE as u16);
        mem.write_u16(prd_addr + 6, 0x8000);

        let bm_base = ctl.bus_master_base();
        ctl.io_write(bm_base + 4, 4, prd_addr as u32);

        // READ DMA for LBA 0, 1 sector (device -> memory request).
        let cmd_base = PRIMARY_PORTS.cmd_base;
        ctl.io_write(cmd_base + ATA_REG_DEVICE, 1, 0xE0);
        ctl.io_write(cmd_base + ATA_REG_SECTOR_COUNT, 1, 1);
        ctl.io_write(cmd_base + ATA_REG_LBA0, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA1, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA2, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_STATUS_COMMAND, 1, 0xC8);

        // Start bus master with direction bit cleared (from memory), which mismatches the request.
        ctl.io_write(bm_base, 1, 0x01);
        ctl.tick(&mut mem);

        assert!(
            ctl.primary_irq_pending(),
            "IRQ should be pending after DMA error"
        );

        let err = ctl.io_read(cmd_base + ATA_REG_ERROR_FEATURES, 1) as u8;
        assert_eq!(err, 0x04, "expected ABRT after DMA failure");

        let st = ctl.io_read(PRIMARY_PORTS.ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL, 1) as u8;
        assert_ne!(
            st & IDE_STATUS_ERR,
            0,
            "ERR bit should be set after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_BSY,
            0,
            "BSY should be clear after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_DRQ,
            0,
            "DRQ should be clear after DMA failure"
        );
        assert_ne!(
            st & IDE_STATUS_DRDY,
            0,
            "DRDY should be set after DMA failure"
        );

        let bm_st = ctl.io_read(bm_base + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06, "BMIDE status should have IRQ+ERR set");

        let mut got = vec![0u8; SECTOR_SIZE];
        mem.read_physical(dma_buf, &mut got);
        assert!(
            got.iter().all(|&b| b == 0xFF),
            "direction mismatch should prevent any DMA writes"
        );
    }

    #[test]
    fn ata_write_dma_prd_missing_eot_aborts_command_and_does_not_write_disk() {
        let mut ctl = setup_primary_ata_controller_with_sector0(0x11);
        let mut mem = Bus::new(0x8000);

        let prd_addr: u64 = 0x1000;
        let dma_buf: u64 = 0x2000;

        // One PRD entry without EOT (malformed), but long enough to cover the sector.
        mem.write_u32(prd_addr, dma_buf as u32);
        mem.write_u16(prd_addr + 4, SECTOR_SIZE as u16);
        mem.write_u16(prd_addr + 6, 0x0000);

        // Fill the guest DMA source buffer with a recognizable pattern.
        mem.write_physical(dma_buf, &vec![0xA5u8; SECTOR_SIZE]);

        let bm_base = ctl.bus_master_base();
        ctl.io_write(bm_base + 4, 4, prd_addr as u32);

        // WRITE DMA for LBA 0, 1 sector.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        ctl.io_write(cmd_base + ATA_REG_DEVICE, 1, 0xE0);
        ctl.io_write(cmd_base + ATA_REG_SECTOR_COUNT, 1, 1);
        ctl.io_write(cmd_base + ATA_REG_LBA0, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA1, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA2, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_STATUS_COMMAND, 1, 0xCA);

        // Start bus master (direction = from memory).
        ctl.io_write(bm_base, 1, 0x01);
        ctl.tick(&mut mem);

        assert!(
            ctl.primary_irq_pending(),
            "IRQ should be pending after DMA error"
        );
        let bm_st = ctl.io_read(bm_base + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06, "BMIDE status should have IRQ+ERR set");

        let err = ctl.io_read(cmd_base + ATA_REG_ERROR_FEATURES, 1) as u8;
        assert_eq!(err, 0x04, "expected ABRT after DMA failure");

        let st = ctl.io_read(PRIMARY_PORTS.ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL, 1) as u8;
        assert_ne!(st & IDE_STATUS_ERR, 0);
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert_eq!(st & IDE_STATUS_DRQ, 0);
        assert_ne!(st & IDE_STATUS_DRDY, 0);

        // Clear the IRQ latch so the subsequent PIO read starts cleanly.
        let _ = ctl.io_read(cmd_base + ATA_REG_STATUS_COMMAND, 1);

        // The disk should not have been modified by a malformed PRD list (no commit should occur).
        let got = read_primary_sector0_via_pio(&mut ctl);
        assert_eq!(got, vec![0x11u8; SECTOR_SIZE]);
    }

    #[test]
    fn ata_write_dma_prd_too_short_aborts_command_and_does_not_write_disk() {
        let mut ctl = setup_primary_ata_controller_with_sector0(0x11);
        let mut mem = Bus::new(0x8000);

        let prd_addr: u64 = 0x1000;
        let dma_buf: u64 = 0x2000;

        // Single 256-byte PRD with EOT, but the request is 512 bytes.
        mem.write_u32(prd_addr, dma_buf as u32);
        mem.write_u16(prd_addr + 4, 256);
        mem.write_u16(prd_addr + 6, 0x8000);
        mem.write_physical(dma_buf, &vec![0xA5u8; SECTOR_SIZE]);

        let bm_base = ctl.bus_master_base();
        ctl.io_write(bm_base + 4, 4, prd_addr as u32);

        // WRITE DMA for LBA 0, 1 sector.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        ctl.io_write(cmd_base + ATA_REG_DEVICE, 1, 0xE0);
        ctl.io_write(cmd_base + ATA_REG_SECTOR_COUNT, 1, 1);
        ctl.io_write(cmd_base + ATA_REG_LBA0, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA1, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA2, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_STATUS_COMMAND, 1, 0xCA);

        ctl.io_write(bm_base, 1, 0x01);
        ctl.tick(&mut mem);

        assert!(
            ctl.primary_irq_pending(),
            "IRQ should be pending after DMA error"
        );
        let bm_st = ctl.io_read(bm_base + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06, "BMIDE status should have IRQ+ERR set");

        let err = ctl.io_read(cmd_base + ATA_REG_ERROR_FEATURES, 1) as u8;
        assert_eq!(err, 0x04, "expected ABRT after DMA failure");

        let st = ctl.io_read(PRIMARY_PORTS.ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL, 1) as u8;
        assert_ne!(
            st & IDE_STATUS_ERR,
            0,
            "ERR bit should be set after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_BSY,
            0,
            "BSY should be clear after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_DRQ,
            0,
            "DRQ should be clear after DMA failure"
        );
        assert_ne!(
            st & IDE_STATUS_DRDY,
            0,
            "DRDY should be set after DMA failure"
        );

        let _ = ctl.io_read(cmd_base + ATA_REG_STATUS_COMMAND, 1);
        let got = read_primary_sector0_via_pio(&mut ctl);
        assert_eq!(got, vec![0x11u8; SECTOR_SIZE]);
    }

    #[test]
    fn ata_write_dma_direction_mismatch_aborts_command_and_does_not_write_disk() {
        let mut ctl = setup_primary_ata_controller_with_sector0(0x11);
        let mut mem = Bus::new(0x8000);

        let prd_addr: u64 = 0x1000;
        let dma_buf: u64 = 0x2000;

        // Valid PRD entry: one 512-byte segment, EOT set.
        mem.write_u32(prd_addr, dma_buf as u32);
        mem.write_u16(prd_addr + 4, SECTOR_SIZE as u16);
        mem.write_u16(prd_addr + 6, 0x8000);
        mem.write_physical(dma_buf, &vec![0xA5u8; SECTOR_SIZE]);

        let bm_base = ctl.bus_master_base();
        ctl.io_write(bm_base + 4, 4, prd_addr as u32);

        // WRITE DMA for LBA 0, 1 sector (guest memory -> device).
        let cmd_base = PRIMARY_PORTS.cmd_base;
        ctl.io_write(cmd_base + ATA_REG_DEVICE, 1, 0xE0);
        ctl.io_write(cmd_base + ATA_REG_SECTOR_COUNT, 1, 1);
        ctl.io_write(cmd_base + ATA_REG_LBA0, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA1, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA2, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_STATUS_COMMAND, 1, 0xCA);

        // Start bus master with direction=ToMemory (bit3 set), which mismatches a WRITE DMA.
        ctl.io_write(bm_base, 1, 0x09);
        ctl.tick(&mut mem);

        assert!(
            ctl.primary_irq_pending(),
            "IRQ should be pending after DMA error"
        );
        let bm_st = ctl.io_read(bm_base + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06, "BMIDE status should have IRQ+ERR set");

        let err = ctl.io_read(cmd_base + ATA_REG_ERROR_FEATURES, 1) as u8;
        assert_eq!(err, 0x04, "expected ABRT after DMA failure");

        let st = ctl.io_read(PRIMARY_PORTS.ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL, 1) as u8;
        assert_ne!(
            st & IDE_STATUS_ERR,
            0,
            "ERR bit should be set after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_BSY,
            0,
            "BSY should be clear after DMA failure"
        );
        assert_eq!(
            st & IDE_STATUS_DRQ,
            0,
            "DRQ should be clear after DMA failure"
        );
        assert_ne!(
            st & IDE_STATUS_DRDY,
            0,
            "DRDY should be set after DMA failure"
        );

        let _ = ctl.io_read(cmd_base + ATA_REG_STATUS_COMMAND, 1);
        let got = read_primary_sector0_via_pio(&mut ctl);
        assert_eq!(got, vec![0x11u8; SECTOR_SIZE]);
    }

    #[test]
    fn ata_write_dma_commit_failure_aborts_command_and_does_not_write_disk() {
        // Use a single-sector disk so any write to a larger LBA will fail during the commit phase.
        let mut ctl = setup_primary_ata_controller_with_sector0(0x11);
        let mut mem = Bus::new(0x8000);

        let prd_addr: u64 = 0x1000;
        let dma_buf: u64 = 0x2000;

        // Valid PRD entry: one 512-byte segment, EOT set.
        mem.write_u32(prd_addr, dma_buf as u32);
        mem.write_u16(prd_addr + 4, SECTOR_SIZE as u16);
        mem.write_u16(prd_addr + 6, 0x8000);
        mem.write_physical(dma_buf, &vec![0xA5u8; SECTOR_SIZE]);

        let bm_base = ctl.bus_master_base();
        ctl.io_write(bm_base + 4, 4, prd_addr as u32);

        // WRITE DMA for out-of-bounds LBA 10.
        let cmd_base = PRIMARY_PORTS.cmd_base;
        ctl.io_write(cmd_base + ATA_REG_DEVICE, 1, 0xE0);
        ctl.io_write(cmd_base + ATA_REG_SECTOR_COUNT, 1, 1);
        ctl.io_write(cmd_base + ATA_REG_LBA0, 1, 10);
        ctl.io_write(cmd_base + ATA_REG_LBA1, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_LBA2, 1, 0);
        ctl.io_write(cmd_base + ATA_REG_STATUS_COMMAND, 1, 0xCA);

        // Start bus master (direction = from memory).
        ctl.io_write(bm_base, 1, 0x01);
        ctl.tick(&mut mem);

        assert!(
            ctl.primary_irq_pending(),
            "IRQ should be pending after DMA error"
        );
        let bm_st = ctl.io_read(bm_base + 2, 1) as u8;
        assert_eq!(bm_st & 0x07, 0x06, "BMIDE status should have IRQ+ERR set");

        let err = ctl.io_read(cmd_base + ATA_REG_ERROR_FEATURES, 1) as u8;
        assert_eq!(err, 0x04, "expected ABRT after DMA failure");

        // Use ALT_STATUS so we don't accidentally clear the interrupt.
        let st = ctl.io_read(PRIMARY_PORTS.ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL, 1) as u8;
        assert_ne!(st & IDE_STATUS_ERR, 0);
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert_eq!(st & IDE_STATUS_DRQ, 0);
        assert_ne!(st & IDE_STATUS_DRDY, 0);

        // Clear the IRQ latch so the subsequent PIO read starts cleanly.
        let _ = ctl.io_read(cmd_base + ATA_REG_STATUS_COMMAND, 1);

        // The failed commit must not modify the disk.
        let got = read_primary_sector0_via_pio(&mut ctl);
        assert_eq!(got, vec![0x11u8; SECTOR_SIZE]);
    }

    #[test]
    fn drive_address_works_for_atapi_devices_on_secondary_channel() {
        let mut ctl = IdeController::new(0xFFF0);

        ctl.attach_secondary_master_atapi(AtapiCdrom::new(None));

        let device_port = ctl.secondary.ports.cmd_base + ATA_REG_DEVICE;
        let drive_addr_port = ctl.secondary.ports.ctrl_base + ATA_CTRL_DRIVE_ADDRESS;

        // Select master. (Value written is conventional for master select; Drive Address should
        // ignore LBA/CHS mode and just reflect head/device selection.)
        ctl.io_write(device_port, 1, 0xA0);
        assert_eq!(ctl.io_read(drive_addr_port, 1) as u8, 0xEF);
    }

    #[test]
    fn win7_ataport_inquiry_pio_uses_qemu_ireason_and_no_cdb_irq() {
        // Exact sequence Win7 atapi.sys uses during IdeIssueInquiry:
        // PACKET 0xA0, poll DRQ (no INTRQ), write 12-byte CDB, then the data
        // phase must present IREASON=IO, BCL=0x24, DRQ+IRQ so ProcessInterrupt
        // copies the INQUIRY buffer and completes the IRB as SUCCESS.
        let mut ctl = IdeController::new(0xC000);
        ctl.attach_secondary_master_atapi(AtapiCdrom::new(None));

        let cmd_base = SECONDARY_PORTS.cmd_base;
        let ctrl_base = SECONDARY_PORTS.ctrl_base;
        let data_port = cmd_base + ATA_REG_DATA;
        let features_port = cmd_base + ATA_REG_ERROR_FEATURES;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let lba2_port = cmd_base + ATA_REG_LBA2;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;
        let ireason_port = cmd_base + ATA_REG_SECTOR_COUNT;
        let alt_status_port = ctrl_base + ATA_CTRL_ALT_STATUS_DEVICE_CTRL;

        ctl.io_write(device_port, 1, 0xA0);
        ctl.io_write(features_port, 1, 0x00); // PIO
        ctl.io_write(lba1_port, 1, 0x24);
        ctl.io_write(lba2_port, 1, 0x00);
        ctl.io_write(command_port, 1, 0xA0);

        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(st & IDE_STATUS_DRQ, 0, "CDB phase must assert DRQ");
        assert_eq!(st & IDE_STATUS_BSY, 0);
        assert!(
            !ctl.secondary_irq_pending(),
            "CDB phase must not raise IRQ"
        );
        assert_eq!(
            ctl.io_read(ireason_port, 1) as u8 & 0x07,
            0x01,
            "CDB phase IREASON is CoD, !IO"
        );

        let mut pkt = [0u8; 12];
        pkt[0] = 0x12;
        pkt[4] = 0x24;
        for chunk in pkt.chunks_exact(2) {
            let w = u16::from_le_bytes([chunk[0], chunk[1]]);
            ctl.io_write(data_port, 2, w as u32);
        }

        assert!(
            ctl.secondary_irq_pending(),
            "INQUIRY data phase must raise IRQ"
        );
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_ne!(st & IDE_STATUS_DRQ, 0);
        assert_eq!(st & IDE_STATUS_ERR, 0);
        assert_eq!(
            ctl.io_read(ireason_port, 1) as u8 & 0x07,
            0x02,
            "data phase IREASON is IO, !CoD"
        );
        let bcl = ctl.io_read(lba1_port, 1) as u16 | ((ctl.io_read(lba2_port, 1) as u16) << 8);
        assert_eq!(bcl, 0x24);

        let mut inquiry = [0u8; 36];
        for i in 0..18 {
            let w = ctl.io_read(data_port, 2) as u16;
            inquiry[i * 2] = w as u8;
            inquiry[i * 2 + 1] = (w >> 8) as u8;
        }
        assert_eq!(inquiry[0], 0x05, "PDT = CD/DVD");
        assert_eq!(inquiry[1], 0x80, "removable");
        assert_eq!(inquiry[3], 0x21, "ATAPI response format");

        // Last word of the PIO read must drop DRQ, set IREASON=IO|CoD, and
        // raise the completion IRQ — Win7 ProcessInterrupt then marks SUCCESS.
        assert!(ctl.secondary_irq_pending(), "status phase must raise IRQ");
        let st = ctl.io_read(alt_status_port, 1) as u8;
        assert_eq!(st & IDE_STATUS_DRQ, 0);
        assert_eq!(st & IDE_STATUS_ERR, 0);
        assert_eq!(
            ctl.io_read(ireason_port, 1) as u8 & 0x07,
            0x03,
            "status phase IREASON is IO|CoD"
        );
    }

    #[test]
    fn atapi_inquiry_status_ack_drops_then_reasserts_irq_line_without_a_poll() {
        // ataport ProcessInterrupt: read STATUS (ack data-in), then `rep insw`
        // the 36-byte INQUIRY. The last word raises the status-phase IRQ.
        // That whole sequence is one Tier-0 I/O batch, so a poll-only pin
        // never goes 1→0→1 and DiscoverLuns times out.
        use crate::bus::TestIrqLine;

        let mut ctl = IdeController::new(0xC000);
        let irq14 = TestIrqLine::default();
        let irq15 = TestIrqLine::default();
        ctl.attach_irq_lines(Box::new(irq14.clone()), Box::new(irq15.clone()));
        ctl.attach_secondary_master_atapi(AtapiCdrom::new(None));

        let cmd_base = SECONDARY_PORTS.cmd_base;
        let data_port = cmd_base + ATA_REG_DATA;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;

        ctl.io_write(device_port, 1, 0xA0);
        ctl.io_write(lba1_port, 1, 0x24);
        ctl.io_write(command_port, 1, 0xA0);
        let mut pkt = [0u8; 12];
        pkt[0] = 0x12;
        pkt[4] = 0x24;
        for chunk in pkt.chunks_exact(2) {
            let w = u16::from_le_bytes([chunk[0], chunk[1]]);
            ctl.io_write(data_port, 2, w as u32);
        }
        assert!(irq15.level(), "data-in must raise IRQ15 immediately");
        assert_eq!(irq15.transitions(), vec![true]);

        let _ = ctl.io_read(command_port, 1);
        assert!(
            !irq15.level(),
            "STATUS read must drop the pin before the PIO drain"
        );
        assert_eq!(irq15.transitions(), vec![true, false]);

        for _ in 0..18 {
            let _ = ctl.io_read(data_port, 2);
        }
        assert!(
            irq15.level(),
            "last INQUIRY word must raise the status-phase edge immediately"
        );
        assert_eq!(irq15.transitions(), vec![true, false, true]);
        assert!(!irq14.level(), "primary pin must stay quiet");
    }

    #[test]
    fn taskfile_writes_ignored_while_drq_like_qemu() {
        let mut ctl = IdeController::new(0xC000);
        ctl.attach_secondary_master_atapi(AtapiCdrom::new(None));

        let cmd_base = SECONDARY_PORTS.cmd_base;
        let count_port = cmd_base + ATA_REG_SECTOR_COUNT;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;

        ctl.io_write(device_port, 1, 0xA0);
        ctl.io_write(lba1_port, 1, 0x24);
        ctl.io_write(command_port, 1, 0xA0);

        let ireason = ctl.io_read(count_port, 1) as u8;
        let bcl_lo = ctl.io_read(lba1_port, 1) as u8;
        ctl.io_write(count_port, 1, 0x0C);
        ctl.io_write(lba1_port, 1, 0x00);
        assert_eq!(
            ctl.io_read(count_port, 1) as u8,
            ireason,
            "Sector Count (IREASON) must be frozen while DRQ"
        );
        assert_eq!(
            ctl.io_read(lba1_port, 1) as u8,
            bcl_lo,
            "BCL must be frozen while DRQ"
        );
    }

    #[test]
    fn packet_ireason_is_assigned_not_ored_so_set_features_count_does_not_leak() {
        // Live Win7: SET FEATURES 0x03 / count=0x0C (PIO mode 4), then PACKET
        // INQUIRY. QEMU cmd_packet assigns nsector=1; preserving 0x0C produced
        // IREASON 0x0A/0x0B on the next INQUIRY.
        let mut ctl = IdeController::new(0xC000);
        ctl.attach_secondary_master_atapi(AtapiCdrom::new(None));

        let cmd_base = SECONDARY_PORTS.cmd_base;
        let features_port = cmd_base + ATA_REG_ERROR_FEATURES;
        let count_port = cmd_base + ATA_REG_SECTOR_COUNT;
        let device_port = cmd_base + ATA_REG_DEVICE;
        let command_port = cmd_base + ATA_REG_STATUS_COMMAND;
        let lba1_port = cmd_base + ATA_REG_LBA1;
        let lba2_port = cmd_base + ATA_REG_LBA2;
        let data_port = cmd_base + ATA_REG_DATA;

        ctl.io_write(device_port, 1, 0xA0);
        ctl.io_write(features_port, 1, 0x03);
        ctl.io_write(count_port, 1, 0x0C);
        ctl.io_write(command_port, 1, 0xEF);
        let _ = ctl.io_read(command_port, 1); // ack IRQ

        ctl.io_write(features_port, 1, 0x00);
        ctl.io_write(lba1_port, 1, 0x24);
        ctl.io_write(lba2_port, 1, 0x00);
        ctl.io_write(command_port, 1, 0xA0);
        assert_eq!(
            ctl.io_read(count_port, 1) as u8,
            0x01,
            "PACKET must assign IREASON=1, not OR leftover 0x0C"
        );

        let mut pkt = [0u8; 12];
        pkt[0] = 0x12;
        pkt[4] = 0x24;
        for chunk in pkt.chunks_exact(2) {
            let w = u16::from_le_bytes([chunk[0], chunk[1]]);
            ctl.io_write(data_port, 2, w as u32);
        }
        assert_eq!(ctl.io_read(count_port, 1) as u8, 0x02);
    }
}
