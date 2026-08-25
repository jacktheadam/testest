use std::io;

use aero_io_snapshot::io::storage::state::{IdeAtapiDeviceState, MAX_IDE_DATA_BUFFER_BYTES};
use aero_storage::{DiskError, VirtualDisk, SECTOR_SIZE as ATA_SECTOR_SIZE};

/// Underlying `VirtualDisk` trait object used by ISO/CD backends.
///
/// `VirtualDisk` is conditionally `Send` via `aero_storage::VirtualDiskSend`:
/// - native: `dyn VirtualDisk` is `Send`
/// - wasm32: `dyn VirtualDisk` may be `!Send` (OPFS/JS-backed handles, etc.)
pub type IsoDisk = Box<dyn VirtualDisk>;

/// Read-only ISO9660 (or raw CD) backing store.
///
/// The IDE/ATAPI layer treats the image as a sequence of 2048-byte sectors.
///
/// # Canonical trait note
///
/// This trait is intentionally narrow (read-only, 2048-byte sectors) because it models an ATAPI
/// CD-ROM device rather than a general-purpose disk.
///
/// Most disk image code in this repo should use the canonical synchronous disk trait
/// [`aero_storage::VirtualDisk`]. For ISO images stored as a generic `VirtualDisk`, use
/// [`VirtualDiskIsoBackend`] to adapt into this ATAPI interface.
///
/// See `wiki/areas/storage.md`.
#[cfg(not(target_arch = "wasm32"))]
pub trait IsoBackend: Send {
    fn sector_count(&self) -> u32;
    fn read_sectors(&mut self, lba: u32, buf: &mut [u8]) -> io::Result<()>;
}

/// wasm32 variant of [`IsoBackend`].
///
/// The browser build supports non-`Send` disk backends (e.g. OPFS handles) that cannot safely
/// cross threads, so we avoid imposing a `Send` bound on ISO backends in wasm builds.
#[cfg(target_arch = "wasm32")]
pub trait IsoBackend {
    fn sector_count(&self) -> u32;
    fn read_sectors(&mut self, lba: u32, buf: &mut [u8]) -> io::Result<()>;
}

// Compile-time guard that we can implement ISO backends using !Send JS/DOM handles in wasm builds
// (e.g. OPFS-backed images in the browser/worker runtime).
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
mod wasm_send_bounds_check {
    use std::io;
    use std::rc::Rc;

    use aero_storage::VirtualDisk;

    use super::{AtapiCdrom, IsoBackend};

    struct NotSendIsoBackend(Rc<()>);

    impl IsoBackend for NotSendIsoBackend {
        fn sector_count(&self) -> u32 {
            0
        }

        fn read_sectors(&mut self, _lba: u32, buf: &mut [u8]) -> io::Result<()> {
            buf.fill(0);
            Ok(())
        }
    }

    fn _assert_accepts_non_send_virtual_disk(disk: Box<dyn VirtualDisk>) -> io::Result<AtapiCdrom> {
        AtapiCdrom::new_from_virtual_disk(disk)
    }
}

/// Adapter that exposes an [`aero_storage::VirtualDisk`] (byte-addressed) as an ATAPI/ISO9660
/// sector device (2048-byte sectors).
///
/// This is useful for attaching a disk image (e.g. a Windows install ISO stored in a generic
/// storage backend) as an ATAPI CD-ROM.
pub struct VirtualDiskIsoBackend {
    disk: IsoDisk,
    sector_count: u32,
}

impl VirtualDiskIsoBackend {
    pub fn new(disk: IsoDisk) -> io::Result<Self> {
        let capacity = disk.capacity_bytes();
        if !capacity.is_multiple_of(AtapiCdrom::SECTOR_SIZE as u64) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ISO disk capacity is not a multiple of 2048-byte sectors",
            ));
        }

        let sector_count = capacity / AtapiCdrom::SECTOR_SIZE as u64;
        let sector_count = u32::try_from(sector_count).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "ISO disk capacity exceeds 32-bit sector count limit",
            )
        })?;

        Ok(Self { disk, sector_count })
    }
}

impl IsoBackend for VirtualDiskIsoBackend {
    fn sector_count(&self) -> u32 {
        self.sector_count
    }

    fn read_sectors(&mut self, lba: u32, buf: &mut [u8]) -> io::Result<()> {
        if !buf.len().is_multiple_of(AtapiCdrom::SECTOR_SIZE) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unaligned buffer length",
            ));
        }

        let offset = u64::from(lba)
            .checked_mul(AtapiCdrom::SECTOR_SIZE as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "offset overflow"))?;

        self.disk.read_at(offset, buf).map_err(map_disk_error)
    }
}

const SENSE_NO_SENSE: u8 = 0x00;
const SENSE_NOT_READY: u8 = 0x02;
const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
const SENSE_UNIT_ATTENTION: u8 = 0x06;

fn try_alloc_zeroed(len: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    buf.try_reserve_exact(len).ok()?;
    buf.resize(len, 0);
    Some(buf)
}

const ASC_MEDIUM_NOT_PRESENT: u8 = 0x3A;
const ASC_MEDIUM_CHANGED: u8 = 0x28;
const ASC_INVALID_COMMAND: u8 = 0x20;
const ASC_INVALID_FIELD_IN_CDB: u8 = 0x24;

#[derive(Debug, Clone, Copy)]
struct Sense {
    key: u8,
    asc: u8,
    ascq: u8,
}

impl Sense {
    fn ok() -> Self {
        Self {
            key: SENSE_NO_SENSE,
            asc: 0,
            ascq: 0,
        }
    }
}

pub struct AtapiCdrom {
    backend: Option<Box<dyn IsoBackend>>,
    tray_open: bool,
    /// Guest-visible media presence (independent of whether the host ISO backend is currently
    /// attached).
    ///
    /// On snapshot restore, the device drops the host-side backend reference, but the guest-visible
    /// state must still round-trip. The platform is expected to re-attach the backend before
    /// resuming guest execution.
    media_present: bool,
    media_changed: bool,
    sense: Sense,
    supports_dma: bool,
}

impl AtapiCdrom {
    pub const SECTOR_SIZE: usize = 2048;

    pub fn new(backend: Option<Box<dyn IsoBackend>>) -> Self {
        let media_present = backend.is_some();
        let media_changed = media_present;
        Self {
            backend,
            tray_open: false,
            media_present,
            media_changed,
            sense: Sense::ok(),
            supports_dma: true,
        }
    }

    pub fn supports_dma(&self) -> bool {
        self.supports_dma
    }

    /// Guest-visible media presence (independent of whether the host ISO backend is attached).
    pub fn media_present(&self) -> bool {
        self.media_present
    }

    pub fn insert_media(&mut self, backend: Box<dyn IsoBackend>) {
        self.backend = Some(backend);
        self.tray_open = false;
        self.media_present = true;
        self.media_changed = true;
    }

    pub fn new_from_virtual_disk(disk: IsoDisk) -> io::Result<Self> {
        Ok(Self::new(Some(Box::new(VirtualDiskIsoBackend::new(disk)?))))
    }

    pub fn insert_virtual_disk(&mut self, disk: IsoDisk) -> io::Result<()> {
        self.insert_media(Box::new(VirtualDiskIsoBackend::new(disk)?));
        Ok(())
    }

    pub fn eject_media(&mut self) {
        self.backend = None;
        self.tray_open = true;
        self.media_present = false;
        self.media_changed = true;
    }

    /// Attach a host backend without changing any guest-visible media/tray state.
    ///
    /// This is intended for snapshot restore paths where the guest already "sees" a disc and we
    /// only need to re-establish the host-side backing store after deserialization.
    pub fn attach_backend_for_restore(&mut self, backend: Box<dyn IsoBackend>) {
        self.backend = Some(backend);
    }

    /// Detach the host backend without mutating guest-visible media state.
    pub fn detach_backend_for_restore(&mut self) {
        self.backend = None;
    }

    pub fn set_sense(&mut self, key: u8, asc: u8, ascq: u8) {
        self.sense = Sense { key, asc, ascq };
    }

    pub fn identify_packet_data(&self) -> Vec<u8> {
        let mut words = [0u16; 256];

        // QEMU/ATA-ATAPI: protocol ATAPI, CD-ROM, removable, interrupt DRQ, 12-byte packets.
        // Win7 atapi.sys classifies the device from this word; 0x8580 (microprocessor
        // DRQ, no LBA in word 49) is enough for IDENTIFY to "work" but not to
        // publish an IDE\CdRom PDO.
        words[0] = 0x85C0;

        // Serial / firmware / model strings.
        write_ata_string(&mut words[10..20], "AEROCDROM000000000001", 20);
        write_ata_string(&mut words[23..27], "0.1", 8);
        write_ata_string(&mut words[27..47], "Aero ATAPI CD-ROM", 40);

        // Capabilities: IORDY + LBA + DMA. LBA (bit 9) is what atapi.sys
        // checks before creating a CD-ROM PDO. Match QEMU's DMA+LBA word.
        words[20] = 3;   // buffer type
        words[21] = 512; // cache size in sectors
        words[22] = 4;   // ECC bytes
        words[48] = 1;   // dword I/O (QEMU sets this on ATAPI too)
        words[49] = (1 << 11) | (1 << 9) | (1 << 8);

        // Word 53 validity bitmap: bit 0 = words 54-58, bit 1 = words 64-70
        // valid, bit 2 = word 88 valid.
        words[53] = 0x07;

        // Advertise MWDMA 0-2 and UDMA 0-5 as *supported* but not selected.
        // Claiming a UDMA mode is already selected makes Win7 ataport issue
        // the first INQUIRY as a DMA PACKET; we must honour that separately.
        words[62] = 0x07;
        words[63] = 0x07;

        // Words 64–70 are advertised valid. Win7 atapi.sys rejects a PACKET
        // device that claims them and then reports no PIO modes.
        words[64] = 0x0003; // PIO modes 3 and 4
        words[65] = 0x00B4;
        words[66] = 0x00B4;
        words[67] = 0x012C; // PIO cycle time without IORDY (QEMU)
        words[68] = 0x00B4; // PIO cycle time with IORDY
        words[71] = 30;
        words[72] = 30;

        words[80] = 0x1E; // ATA/ATAPI-1 through 4, QEMU
        words[82] = 1 << 4 | 1 << 9; // PACKET + DEVICE RESET
        words[83] = 1 << 14; // word 83 valid
        words[85] = 1 << 4 | 1 << 9;

        // Word 88: Ultra DMA 0-5 supported, none selected (SET FEATURES picks).
        words[88] = 0x3F;

        let mut out = vec![0u8; ATA_SECTOR_SIZE];
        for (i, w) in words.iter().enumerate() {
            out[i * 2..i * 2 + 2].copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    fn check_ready(&mut self) -> Result<(), PacketResult> {
        if self.media_changed {
            self.media_changed = false;
            self.set_sense(SENSE_UNIT_ATTENTION, ASC_MEDIUM_CHANGED, 0);
            return Err(PacketResult::Error {
                sense_key: SENSE_UNIT_ATTENTION,
                asc: ASC_MEDIUM_CHANGED,
                ascq: 0,
            });
        }
        if self.tray_open || !self.media_present || self.backend.is_none() {
            self.set_sense(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, 0);
            return Err(PacketResult::Error {
                sense_key: SENSE_NOT_READY,
                asc: ASC_MEDIUM_NOT_PRESENT,
                ascq: 0,
            });
        }
        Ok(())
    }

    pub fn handle_packet(&mut self, packet: &[u8; 12], dma_requested: bool) -> PacketResult {
        let opcode = packet[0];
        match opcode {
            0x12 => {
                // INQUIRY
                let alloc_len = packet[4] as usize;
                let data = self.inquiry_data();
                atapi_data_reply(dma_requested, data[..alloc_len.min(data.len())].to_vec())
            }
            0x00 => {
                // TEST UNIT READY
                match self.check_ready() {
                    Ok(()) => {
                        self.set_sense(SENSE_NO_SENSE, 0, 0);
                        PacketResult::NoDataSuccess
                    }
                    Err(e) => e,
                }
            }
            0x25 => {
                // READ CAPACITY(10)
                if let Err(e) = self.check_ready() {
                    return e;
                }
                let data = match self.read_capacity_10() {
                    Ok(d) => d,
                    Err(_) => {
                        self.set_sense(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, 0);
                        return PacketResult::Error {
                            sense_key: SENSE_NOT_READY,
                            asc: ASC_MEDIUM_NOT_PRESENT,
                            ascq: 0,
                        };
                    }
                };
                atapi_data_reply(dma_requested, data)
            }
            0x28 => {
                // READ(10)
                if let Err(e) = self.check_ready() {
                    return e;
                }
                let lba = u32::from_be_bytes([packet[2], packet[3], packet[4], packet[5]]);
                let blocks = u16::from_be_bytes([packet[7], packet[8]]) as u32;
                self.read_blocks(lba, blocks, dma_requested)
            }
            0xA8 => {
                // READ(12)
                if let Err(e) = self.check_ready() {
                    return e;
                }
                let lba = u32::from_be_bytes([packet[2], packet[3], packet[4], packet[5]]);
                let blocks = u32::from_be_bytes([packet[6], packet[7], packet[8], packet[9]]);
                self.read_blocks(lba, blocks, dma_requested)
            }
            0xBE => {
                // READ CD
                //
                // We support a conservative subset for data-disc use cases: reads of 2048-byte user
                // data sectors with no additional header/subchannel fields.
                if let Err(e) = self.check_ready() {
                    return e;
                }

                // Byte 1: Expected sector type (low 3 bits).
                //
                // Only accept types that map cleanly onto 2048-byte ISO sectors:
                // - 0x00: any (treat as 2048-byte user data)
                // - 0x02: Mode 1
                // - 0x04: Mode 2 Form 1
                let expected_sector_type = packet[1] & 0x07;
                match expected_sector_type {
                    0x00 | 0x02 | 0x04 => {}
                    _ => {
                        self.set_sense(SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0);
                        return PacketResult::Error {
                            sense_key: SENSE_ILLEGAL_REQUEST,
                            asc: ASC_INVALID_FIELD_IN_CDB,
                            ascq: 0,
                        };
                    }
                }

                // Byte 9: bitmask for which parts of the sector to return. We only support user
                // data (0x10) and require that no other bits are set.
                let sector_fields = packet[9];
                if sector_fields != 0x10 {
                    self.set_sense(SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0);
                    return PacketResult::Error {
                        sense_key: SENSE_ILLEGAL_REQUEST,
                        asc: ASC_INVALID_FIELD_IN_CDB,
                        ascq: 0,
                    };
                }

                // Byte 10: Sub-channel selection bits. We do not support subchannel reads.
                if packet[10] != 0 {
                    self.set_sense(SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0);
                    return PacketResult::Error {
                        sense_key: SENSE_ILLEGAL_REQUEST,
                        asc: ASC_INVALID_FIELD_IN_CDB,
                        ascq: 0,
                    };
                }

                let lba = u32::from_be_bytes([packet[2], packet[3], packet[4], packet[5]]);
                let blocks = (u32::from(packet[6]) << 16)
                    | (u32::from(packet[7]) << 8)
                    | u32::from(packet[8]);
                self.read_blocks(lba, blocks, dma_requested)
            }
            0x03 => {
                // REQUEST SENSE
                let alloc_len = packet[4] as usize;
                let sense = self.request_sense();
                // QEMU only drops a pending UNIT ATTENTION on REQUEST SENSE;
                // other sense stays until a later command replaces it.
                if self.sense.key == SENSE_UNIT_ATTENTION {
                    self.set_sense(SENSE_NO_SENSE, 0, 0);
                }
                atapi_data_reply(dma_requested, sense[..alloc_len.min(sense.len())].to_vec())
            }
            0x43 => {
                // READ TOC
                if let Err(e) = self.check_ready() {
                    return e;
                }
                let alloc_len = u16::from_be_bytes([packet[7], packet[8]]) as usize;
                let data = self.read_toc();
                atapi_data_reply(dma_requested, data[..alloc_len.min(data.len())].to_vec())
            }
            0x1B => {
                // START STOP UNIT (tray open/close/eject)
                let loej = (packet[4] & 0x02) != 0;
                let start = (packet[4] & 0x01) != 0;
                if loej && !start {
                    self.eject_media();
                } else if loej && start {
                    self.tray_open = false;
                }
                PacketResult::NoDataSuccess
            }
            0x1E => {
                // PREVENT ALLOW MEDIUM REMOVAL (no-op for our model).
                PacketResult::NoDataSuccess
            }
            0x46 => {
                // GET CONFIGURATION. QEMU flags this ALLOW_UA and does not
                // require media. Starting feature must be 0 (feature 0 is the
                // Profile List); anything else is ILLEGAL REQUEST, same as QEMU.
                let starting = u16::from_be_bytes([packet[2], packet[3]]);
                if starting != 0 {
                    self.set_sense(SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0);
                    return PacketResult::Error {
                        sense_key: SENSE_ILLEGAL_REQUEST,
                        asc: ASC_INVALID_FIELD_IN_CDB,
                        ascq: 0,
                    };
                }
                let alloc_len = u16::from_be_bytes([packet[7], packet[8]]) as usize;
                let data = self.get_configuration();
                atapi_data_reply(
                    dma_requested,
                    data[..alloc_len.min(data.len())].to_vec(),
                )
            }
            0x4A => {
                // GET EVENT STATUS NOTIFICATION.
                //
                // Windows' CD/DVD stack may use this to poll for media change and tray events.
                // We provide a minimal "no event" response and advertise only the media event
                // class.
                let request = packet[4];
                let alloc_len = u16::from_be_bytes([packet[7], packet[8]]) as usize;
                let data = self.get_event_status_notification(request);
                atapi_data_reply(dma_requested, data[..alloc_len.min(data.len())].to_vec())
            }
            0x51 => {
                // READ DISC INFORMATION.
                if let Err(e) = self.check_ready() {
                    return e;
                }
                let alloc_len = u16::from_be_bytes([packet[7], packet[8]]) as usize;
                let data = self.read_disc_information();
                atapi_data_reply(
                    dma_requested,
                    data[..alloc_len.min(data.len())].to_vec(),
                )
            }
            0x5A | 0x1A => {
                // MODE SENSE (10)/(6). QEMU rejects unknown pages (including
                // 0x1B) with ILLEGAL REQUEST; Win7 then asks for 0x2A.
                // Returning a dummy 0x1B page is worse than aborting: ataport
                // treats the garbage as "not a CD" after a successful INQUIRY.
                let page_code = packet[2] & 0x3F;
                let mut alloc_len = if opcode == 0x5A {
                    u16::from_be_bytes([packet[7], packet[8]]) as usize
                } else {
                    packet[4] as usize
                };
                if alloc_len == 0 {
                    alloc_len = if opcode == 0x5A { 256 } else { 256 };
                }
                match self.mode_sense(page_code, opcode == 0x5A) {
                    Some(data) => {
                        atapi_data_reply(dma_requested, data[..alloc_len.min(data.len())].to_vec())
                    }
                    None => {
                        self.set_sense(SENSE_ILLEGAL_REQUEST, ASC_INVALID_COMMAND, 0);
                        PacketResult::Error {
                            sense_key: SENSE_ILLEGAL_REQUEST,
                            asc: ASC_INVALID_COMMAND,
                            ascq: 0,
                        }
                    }
                }
            }
            _ => {
                self.set_sense(SENSE_ILLEGAL_REQUEST, ASC_INVALID_COMMAND, 0);
                PacketResult::Error {
                    sense_key: SENSE_ILLEGAL_REQUEST,
                    asc: ASC_INVALID_COMMAND,
                    ascq: 0,
                }
            }
        }
    }

    fn inquiry_data(&self) -> Vec<u8> {
        let mut data = vec![0u8; 36];
        data[0] = 0x05; // CD/DVD device
        data[1] = 0x80; // removable
        data[2] = 0x00; // match QEMU ATAPI inquiry (ANSI version)
        data[3] = 0x21; // ATAPI response format
        data[4] = (data.len() - 5) as u8;
        write_scsi_ascii(&mut data[8..16], b"AERO");
        write_scsi_ascii(&mut data[16..32], b"ATAPI CD-ROM");
        write_scsi_ascii(&mut data[32..36], b"0.1");
        data
    }

    fn request_sense(&self) -> Vec<u8> {
        let mut data = vec![0u8; 18];
        // QEMU sets the VALID bit (0x80) on the fixed-format sense header.
        data[0] = 0x70 | 0x80;
        data[2] = self.sense.key & 0x0F;
        data[7] = 10;
        data[12] = self.sense.asc;
        data[13] = self.sense.ascq;
        data
    }

    fn read_capacity_10(&mut self) -> io::Result<Vec<u8>> {
        let Some(backend) = self.backend.as_ref() else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no media"));
        };
        let sectors = backend.sector_count();
        let last_lba = sectors.saturating_sub(1);
        let mut out = vec![0u8; 8];
        out[..4].copy_from_slice(&last_lba.to_be_bytes());
        out[4..].copy_from_slice(&(Self::SECTOR_SIZE as u32).to_be_bytes());
        Ok(out)
    }

    fn read_blocks(&mut self, lba: u32, blocks: u32, dma_requested: bool) -> PacketResult {
        if blocks == 0 {
            self.set_sense(SENSE_NO_SENSE, 0, 0);
            return PacketResult::NoDataSuccess;
        }
        let len = match blocks
            .checked_mul(Self::SECTOR_SIZE as u32)
            .and_then(|v| usize::try_from(v).ok())
        {
            Some(v) => v,
            None => {
                self.set_sense(SENSE_ILLEGAL_REQUEST, 0x21, 0);
                return PacketResult::Error {
                    sense_key: SENSE_ILLEGAL_REQUEST,
                    asc: 0x21,
                    ascq: 0,
                };
            }
        };
        if len > MAX_IDE_DATA_BUFFER_BYTES {
            self.set_sense(SENSE_ILLEGAL_REQUEST, 0x21, 0);
            return PacketResult::Error {
                sense_key: SENSE_ILLEGAL_REQUEST,
                asc: 0x21,
                ascq: 0,
            };
        }
        let mut buf = match try_alloc_zeroed(len) {
            Some(buf) => buf,
            None => {
                // Surface allocation failures as a command error instead of aborting the entire
                // process on OOM.
                self.set_sense(SENSE_ILLEGAL_REQUEST, 0x21, 0);
                return PacketResult::Error {
                    sense_key: SENSE_ILLEGAL_REQUEST,
                    asc: 0x21,
                    ascq: 0,
                };
            }
        };
        let res = if let Some(backend) = self.backend.as_mut() {
            backend.read_sectors(lba, &mut buf)
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "no media"))
        };
        match res {
            Ok(()) => {
                self.set_sense(SENSE_NO_SENSE, 0, 0);
                if dma_requested {
                    PacketResult::DmaIn(buf)
                } else {
                    PacketResult::DataIn(buf)
                }
            }
            Err(_) => {
                self.set_sense(SENSE_ILLEGAL_REQUEST, 0x21, 0);
                PacketResult::Error {
                    sense_key: SENSE_ILLEGAL_REQUEST,
                    asc: 0x21,
                    ascq: 0,
                }
            }
        }
    }

    fn read_toc(&mut self) -> Vec<u8> {
        let sectors = self.backend.as_ref().map(|b| b.sector_count()).unwrap_or(0);
        let lead_out_lba = sectors;

        // Header (4 bytes) + 2 descriptors (track 1 + lead-out) = 4 + 16 = 20 bytes.
        let mut out = vec![0u8; 20];
        let data_len = (out.len() - 2) as u16;
        out[0..2].copy_from_slice(&data_len.to_be_bytes());
        out[2] = 1; // first track
        out[3] = 1; // last track

        // Track 1 descriptor.
        out[5] = 0x14; // ADR=1, control=4 (data track)
        out[6] = 0x01; // track number
        out[8..12].copy_from_slice(&0u32.to_be_bytes());

        // Lead-out descriptor (track number 0xAA).
        out[13] = 0x14;
        out[14] = 0xAA;
        out[16..20].copy_from_slice(&lead_out_lba.to_be_bytes());

        out
    }

    fn get_configuration(&self) -> Vec<u8> {
        // QEMU `cmd_get_configuration`: 8-byte feature header + feature 0
        // (Profile List) advertising DVD-ROM and CD-ROM. Current profile is
        // 0 with no media, CD-ROM for ≤700 MiB, DVD-ROM otherwise.
        const PROFILE_DVD_ROM: u16 = 0x0010;
        const PROFILE_CD_ROM: u16 = 0x0008;
        const CD_MAX_2048_SECTORS: u32 = 360_000;

        let current = if self.tray_open || !self.media_present || self.backend.is_none() {
            0
        } else {
            let sectors = self.backend.as_ref().map(|b| b.sector_count()).unwrap_or(0);
            if sectors > CD_MAX_2048_SECTORS {
                PROFILE_DVD_ROM
            } else {
                PROFILE_CD_ROM
            }
        };

        // header (8) + feature 0 header (4) + two 4-byte profiles
        let mut out = vec![0u8; 20];
        out[6..8].copy_from_slice(&current.to_be_bytes());
        out[10] = 0x03; // persistent | current
        out[11] = 8; // additional length: two profiles
        out[12..14].copy_from_slice(&PROFILE_DVD_ROM.to_be_bytes());
        out[14] = u8::from(current == PROFILE_DVD_ROM);
        out[16..18].copy_from_slice(&PROFILE_CD_ROM.to_be_bytes());
        out[18] = u8::from(current == PROFILE_CD_ROM);
        let data_len = (out.len() - 4) as u32;
        out[0..4].copy_from_slice(&data_len.to_be_bytes());
        out
    }

    fn get_event_status_notification(&self, request: u8) -> Vec<u8> {
        // We only advertise the "Media" event class.
        const EVENT_CLASS_MEDIA: u8 = 0x08;

        // If the guest requests event classes we don't implement, still return a valid header that
        // advertises the supported classes. (Windows probes supported classes first, then polls
        // only the Media class.)
        if request == 0 || (request & EVENT_CLASS_MEDIA) == 0 {
            // Return just the event header so the guest can learn which classes are supported.
            let mut out = vec![0u8; 4];
            // Event Data Length is the number of bytes following the first two bytes.
            out[0..2].copy_from_slice(&2u16.to_be_bytes());
            out[2] = 0; // no event class
            out[3] = EVENT_CLASS_MEDIA;
            return out;
        }

        let mut out = vec![0u8; 8];
        out[0..2].copy_from_slice(&6u16.to_be_bytes()); // bytes following this field
        out[2] = EVENT_CLASS_MEDIA;
        out[3] = EVENT_CLASS_MEDIA;
        // Event code 0 = no change. Provide basic media/tray status bits.
        out[4] = 0x00;
        let mut status = 0u8;
        if self.media_present {
            status |= 0x01;
        }
        if self.tray_open {
            status |= 0x02;
        }
        out[5] = status;
        out
    }

    fn read_disc_information(&self) -> Vec<u8> {
        // MMC "Disc Information" is variable-length. We return a fixed 34-byte payload
        // with conservative defaults that describe a finalized, read-only disc.
        let mut out = vec![0u8; 34];
        let data_len = (out.len() - 2) as u16;
        out[0..2].copy_from_slice(&data_len.to_be_bytes());
        // Disc status / last session status: report complete/finalized.
        out[2] = 0x0E;
        // First track number in last session.
        out[3] = 0x01;
        // Number of sessions (1).
        out[4] = 0x01;
        out
    }

    fn mode_sense(&self, page_code: u8, is_10: bool) -> Option<Vec<u8>> {
        let page = match page_code {
            0x01 => self.mode_page_01(),
            0x0D => self.mode_page_0d(),
            0x1A => self.mode_page_1a(),
            // 0x1D is Timeout & Protect (MMC). 0x1B is not a QEMU page —
            // leave it unimplemented so Win7 falls through to 0x2A.
            0x1D => self.mode_page_1b(),
            0x2A | 0x3F => self.mode_page_2a(),
            _ => return None,
        };
        Some(self.wrap_mode_page(&page, is_10))
    }

    fn wrap_mode_page(&self, page: &[u8], is_10: bool) -> Vec<u8> {
        if is_10 {
            let mut out = vec![0u8; 8 + page.len()];
            let mdl = (out.len() - 2) as u16;
            out[0..2].copy_from_slice(&mdl.to_be_bytes());
            // QEMU reports medium type 0x70 (non-present / door closed default).
            out[2] = 0x70;
            out[3] = 0x00;
            out[6..8].copy_from_slice(&0u16.to_be_bytes());
            out[8..].copy_from_slice(page);
            out
        } else {
            let mut out = vec![0u8; 4 + page.len()];
            out[0] = (out.len() - 1) as u8;
            out[1] = 0;
            out[2] = 0x80;
            out[3] = 0;
            out[4..].copy_from_slice(page);
            out
        }
    }

    fn mode_page_01(&self) -> Vec<u8> {
        // Read/write error recovery. 12-byte page, read retries only.
        vec![0x01, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    }

    fn mode_page_0d(&self) -> Vec<u8> {
        // CD-ROM device parameters (SFF-8020).
        vec![0x0D, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    }

    fn mode_page_1a(&self) -> Vec<u8> {
        // Power condition.
        vec![0x1A, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    }

    fn mode_page_1b(&self) -> Vec<u8> {
        // Timeout & protect / legacy CD-ROM page. Advertise a read-only disc.
        let mut page = vec![0u8; 12];
        page[0] = 0x1B;
        page[1] = (page.len() - 2) as u8;
        page[2] = 0x00;
        page[3] = 0x00;
        page
    }

    fn mode_page_2a(&self) -> Vec<u8> {
        // Mode page 0x2A: match QEMU's 22-byte capabilities page so Win7
        // ataport sees a normal MMC CD/DVD reader.
        let mut page = vec![0u8; 22];
        page[0] = 0x2A;
        page[1] = (page.len() - 2) as u8;
        page[2] = 0x3B; // CD-R/RW + DVD-ROM/R/RAM read
        page[3] = 0x00; // no write
        page[4] = 0x71; // audio play, mode2, multi-session, ISRC
        page[5] = 3 << 5;
        page[6] = (1 << 0) | (1 << 3) | (1 << 5); // lock, eject, tray
        page[8] = 0x02;
        page[9] = 0xC0; // 704 KB/s (4x) like QEMU
        page[12] = 0x02;
        page[13] = 0x00; // 512k buffer
        page[14] = 0x02;
        page[15] = 0xC0; // current 4x
        page
    }

    pub fn snapshot_state(&self) -> IdeAtapiDeviceState {
        IdeAtapiDeviceState {
            tray_open: self.tray_open,
            media_changed: self.media_changed,
            media_present: self.media_present,
            sense_key: self.sense.key,
            asc: self.sense.asc,
            ascq: self.sense.ascq,
        }
    }

    pub fn restore_state(&mut self, state: &IdeAtapiDeviceState) {
        self.tray_open = state.tray_open;
        self.media_present = state.media_present;
        self.media_changed = state.media_changed;
        self.sense = Sense {
            key: state.sense_key,
            asc: state.asc,
            ascq: state.ascq,
        };

        // The host ISO backend is treated as transient and must be re-attached by the platform
        // after restore (similar to `DiskLayerState::attach_backend`).
        self.backend = None;
    }
}

fn map_disk_error(err: DiskError) -> io::Error {
    io::Error::other(err)
}

#[derive(Debug)]
pub enum PacketResult {
    NoDataSuccess,
    DataIn(Vec<u8>),
    DmaIn(Vec<u8>),
    Error { sense_key: u8, asc: u8, ascq: u8 },
}

fn atapi_data_reply(dma_requested: bool, data: Vec<u8>) -> PacketResult {
    if dma_requested {
        PacketResult::DmaIn(data)
    } else {
        PacketResult::DataIn(data)
    }
}

fn write_scsi_ascii(dst: &mut [u8], src: &[u8]) {
    dst.fill(b' ');
    let copy_len = src.len().min(dst.len());
    dst[..copy_len].copy_from_slice(&src[..copy_len]);
}

fn write_ata_string(dst_words: &mut [u16], src: &str, byte_len: usize) {
    let mut bytes = vec![b' '; byte_len];
    let src_bytes = src.as_bytes();
    let copy_len = src_bytes.len().min(byte_len);
    bytes[..copy_len].copy_from_slice(&src_bytes[..copy_len]);

    for (i, word) in dst_words.iter_mut().enumerate() {
        let idx = i * 2;
        if idx + 1 >= bytes.len() {
            break;
        }
        *word = u16::from_be_bytes([bytes[idx], bytes[idx + 1]]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gesn_packet(request: u8, alloc_len: u16) -> [u8; 12] {
        let mut pkt = [0u8; 12];
        pkt[0] = 0x4A;
        pkt[4] = request;
        pkt[7..9].copy_from_slice(&alloc_len.to_be_bytes());
        pkt
    }

    fn request_sense_packet(alloc_len: u8) -> [u8; 12] {
        let mut pkt = [0u8; 12];
        pkt[0] = 0x03;
        pkt[4] = alloc_len;
        pkt
    }

    #[test]
    fn get_event_status_notification_succeeds_without_media_request_0() {
        let mut dev = AtapiCdrom::new(None);

        let pkt = gesn_packet(0, 0xFFFF);
        let PacketResult::DataIn(data) = dev.handle_packet(&pkt, false) else {
            panic!("expected DataIn for GET EVENT STATUS NOTIFICATION");
        };

        assert_eq!(data.len(), 4);
        assert_eq!(&data[0..2], &2u16.to_be_bytes());
        assert_eq!(data[2], 0x00, "notification class should be 0 (no event)");
        assert_eq!(
            data[3] & 0x08,
            0x08,
            "media event class should be advertised"
        );
    }

    #[test]
    fn get_event_status_notification_request_unsupported_class_returns_header_only() {
        let mut dev = AtapiCdrom::new(None);

        // Request an unsupported class (Operation Change). We should still succeed and return
        // the 4-byte header advertising the Media class.
        let pkt = gesn_packet(0x01, 0xFFFF);
        let PacketResult::DataIn(data) = dev.handle_packet(&pkt, false) else {
            panic!("expected DataIn for GET EVENT STATUS NOTIFICATION");
        };

        assert_eq!(data.len(), 4);
        assert_eq!(&data[0..2], &2u16.to_be_bytes());
        assert_eq!(data[2], 0x00, "notification class should be 0 (no event)");
        assert_eq!(
            data[3] & 0x08,
            0x08,
            "media event class should be advertised"
        );
    }

    #[test]
    fn mode_sense_page_1b_is_illegal_like_qemu_so_win7_falls_through_to_0x2a() {
        let mut dev = AtapiCdrom::new(None);
        let mut pkt = [0u8; 12];
        pkt[0] = 0x5A;
        pkt[1] = 0x08; // DBD, the exact Win7 atapi.sys encoding
        pkt[2] = 0x1B;
        pkt[7..9].copy_from_slice(&0x00FFu16.to_be_bytes());
        match dev.handle_packet(&pkt, false) {
            PacketResult::Error {
                sense_key: SENSE_ILLEGAL_REQUEST,
                ..
            } => {}
            other => panic!("QEMU rejects page 0x1B, got {other:?}"),
        }

        pkt[2] = 0x2A;
        let PacketResult::DataIn(data) = dev.handle_packet(&pkt, false) else {
            panic!("MODE SENSE page 0x2A must succeed without media");
        };
        assert!(data.len() >= 8 + 4, "header + page");
        assert_eq!(data[2], 0x70, "QEMU medium type");
        assert_eq!(data[8], 0x2A);
    }

    #[test]
    fn get_event_status_notification_succeeds_without_media_and_preserves_sense() {
        let mut dev = AtapiCdrom::new(None);
        dev.set_sense(0x05, 0xDE, 0xAD);

        let pkt = gesn_packet(0x08, 0xFFFF);
        let PacketResult::DataIn(data) = dev.handle_packet(&pkt, false) else {
            panic!("expected DataIn for GET EVENT STATUS NOTIFICATION");
        };

        assert_eq!(data.len(), 8);
        assert_eq!(&data[0..2], &6u16.to_be_bytes());
        assert_eq!(data[2], 0x08);
        assert_eq!(data[3] & 0x08, 0x08);
        assert_eq!(data[4], 0x00, "event code should be 'no change'");
        assert_eq!(data[5] & 0x01, 0x00, "media-present bit must be clear");
        assert_eq!(data[5] & 0x02, 0x00, "tray-open bit must be clear");

        // GET EVENT STATUS NOTIFICATION should not clobber current sense state.
        let req_sense = request_sense_packet(18);
        let PacketResult::DataIn(sense) = dev.handle_packet(&req_sense, false) else {
            panic!("expected DataIn for REQUEST SENSE");
        };
        assert_eq!(sense[2] & 0x0F, 0x05);
        assert_eq!(sense[12], 0xDE);
        assert_eq!(sense[13], 0xAD);
    }

    #[test]
    fn get_event_status_notification_reports_tray_open_bit() {
        let mut dev = AtapiCdrom::new(None);
        dev.eject_media();

        let pkt = gesn_packet(0x08, 0xFFFF);
        let PacketResult::DataIn(data) = dev.handle_packet(&pkt, false) else {
            panic!("expected DataIn for GET EVENT STATUS NOTIFICATION");
        };

        assert_eq!(data.len(), 8);
        assert_eq!(data[5] & 0x01, 0x00, "media-present bit must be clear");
        assert_eq!(data[5] & 0x02, 0x02, "tray-open bit must be set");
    }

    #[test]
    fn get_event_status_notification_respects_allocation_length() {
        let mut dev = AtapiCdrom::new(None);

        let pkt = gesn_packet(0x08, 4);
        let PacketResult::DataIn(data) = dev.handle_packet(&pkt, false) else {
            panic!("expected DataIn for GET EVENT STATUS NOTIFICATION");
        };

        assert_eq!(
            data.len(),
            4,
            "response must be truncated to allocation length"
        );
    }

    fn read_capacity_10_packet() -> [u8; 12] {
        let mut pkt = [0u8; 12];
        pkt[0] = 0x25;
        pkt
    }

    #[test]
    fn read_capacity_is_not_ready_without_media() {
        let mut dev = AtapiCdrom::new(None);

        let pkt = read_capacity_10_packet();
        let res = dev.handle_packet(&pkt, false);

        assert!(matches!(
            res,
            PacketResult::Error {
                sense_key: SENSE_NOT_READY,
                asc: ASC_MEDIUM_NOT_PRESENT,
                ascq: 0
            }
        ));
    }

    fn get_configuration_packet(starting_feature: u16, alloc_len: u16) -> [u8; 12] {
        let mut pkt = [0u8; 12];
        pkt[0] = 0x46;
        pkt[2..4].copy_from_slice(&starting_feature.to_be_bytes());
        pkt[7..9].copy_from_slice(&alloc_len.to_be_bytes());
        pkt
    }

    struct TestConfigIso {
        sectors: u32,
    }

    impl IsoBackend for TestConfigIso {
        fn sector_count(&self) -> u32 {
            self.sectors
        }

        fn read_sectors(&mut self, _lba: u32, buf: &mut [u8]) -> io::Result<()> {
            buf.fill(0);
            Ok(())
        }
    }

    #[test]
    fn get_configuration_matches_qemu_profile_list_without_media() {
        let mut dev = AtapiCdrom::new(None);
        let pkt = get_configuration_packet(0, 0xFFFF);
        let PacketResult::DataIn(data) = dev.handle_packet(&pkt, false) else {
            panic!("GET CONFIGURATION must succeed without media");
        };
        assert_eq!(data.len(), 20);
        assert_eq!(&data[0..4], &16u32.to_be_bytes());
        assert_eq!(&data[6..8], &0u16.to_be_bytes(), "no media → current profile 0");
        assert_eq!(data[10], 0x03, "feature 0 persistent|current");
        assert_eq!(data[11], 8);
        assert_eq!(&data[12..14], &0x0010u16.to_be_bytes());
        assert_eq!(data[14], 0, "DVD-ROM not current without media");
        assert_eq!(&data[16..18], &0x0008u16.to_be_bytes());
        assert_eq!(data[18], 0, "CD-ROM not current without media");
    }

    #[test]
    fn get_configuration_reports_dvd_profile_for_win7_sized_iso() {
        let mut dev = AtapiCdrom::new(Some(Box::new(TestConfigIso {
            // ~3 GiB Win7 install ISO is well above a 700 MiB CD.
            sectors: 1_600_000,
        })));
        let pkt = get_configuration_packet(0, 0xFFFF);
        let PacketResult::DataIn(data) = dev.handle_packet(&pkt, false) else {
            panic!("GET CONFIGURATION must succeed with media");
        };
        assert_eq!(&data[6..8], &0x0010u16.to_be_bytes());
        assert_eq!(data[14], 1, "DVD-ROM profile must be current");
        assert_eq!(data[18], 0);
    }

    #[test]
    fn get_configuration_rejects_nonzero_starting_feature_like_qemu() {
        let mut dev = AtapiCdrom::new(None);
        let pkt = get_configuration_packet(1, 0xFFFF);
        match dev.handle_packet(&pkt, false) {
            PacketResult::Error {
                sense_key: SENSE_ILLEGAL_REQUEST,
                asc: ASC_INVALID_FIELD_IN_CDB,
                ..
            } => {}
            other => panic!("QEMU rejects starting feature != 0, got {other:?}"),
        }
    }
}
