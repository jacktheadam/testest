//! The original Aero sparse disk format, `AEROSPRS`.
//!
//! Superseded by `AEROSPAR` ([`crate::sparse`]), which is what new images use. This module stays
//! because images written in the old format still exist and would otherwise be unreadable: a format
//! nothing can open is data destroyed, however obsolete the format. It is complete enough to open,
//! read, write, and recover such an image, which is what conversion needs.
//!
//! The layout is a header, a single-record journal, an allocation table, then block data. The
//! journal exists so that allocating a block — which touches both the table and the data region —
//! survives a crash between the two writes: [`SparseDiskV1::open`] replays any record it finds
//! before the image is used.

//! Legacy Aero sparse disk format v1 (`AEROSPRS`).
//!
//! This format predates the canonical `AEROSPAR` sparse format implemented in `crates/aero-storage`
//! (`crate::AeroSparseDiskV1`).
//!
//! It is not used by the controller stack; new images are `AEROSPAR`. See
//! `wiki/areas/storage.md`.
use crate::backend::StorageBackend;
use crate::disk::{VirtualDisk, VirtualDiskSend};
use crate::error::{DiskError, Result};

const SPARSE_MAGIC: [u8; 8] = *b"AEROSPRS";
const SPARSE_VERSION: u32 = 1;
const HEADER_SIZE: u64 = 4096;
const JOURNAL_SIZE: u64 = 4096;
const JOURNAL_MAGIC: [u8; 4] = *b"JNL1";
// DoS guard: avoid allocating absurdly large in-memory allocation tables for untrusted images.
const MAX_TABLE_BYTES: u64 = 128 * 1024 * 1024; // 128 MiB
                                                // DoS guard: keep allocation blocks bounded. Extremely large block sizes can cause pathological
                                                // behavior (e.g. allocating a single block can require zero-filling gigabytes).
                                                //
                                                // This cap intentionally matches the one used by the canonical `AEROSPAR` format in
                                                // `crates/aero-storage` so legacy images cannot request more work per block than current ones.
const MAX_BLOCK_SIZE_BYTES: u32 = 64 * 1024 * 1024; // 64 MiB

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SparseHeaderV1 {
    pub sector_size: u32,
    pub block_size: u32,
    pub total_sectors: u64,
    pub table_offset: u64,
    pub table_entries: u64,
    pub journal_offset: u64,
    pub data_offset: u64,
}

impl SparseHeaderV1 {
    fn encode(&self) -> [u8; HEADER_SIZE as usize] {
        let mut buf = [0u8; HEADER_SIZE as usize];
        buf[..8].copy_from_slice(&SPARSE_MAGIC);
        buf[8..12].copy_from_slice(&SPARSE_VERSION.to_le_bytes());
        buf[12..16].copy_from_slice(&self.sector_size.to_le_bytes());
        buf[16..20].copy_from_slice(&self.block_size.to_le_bytes());
        buf[20..28].copy_from_slice(&self.total_sectors.to_le_bytes());
        buf[28..36].copy_from_slice(&self.table_offset.to_le_bytes());
        buf[36..44].copy_from_slice(&self.table_entries.to_le_bytes());
        buf[44..52].copy_from_slice(&self.journal_offset.to_le_bytes());
        buf[52..60].copy_from_slice(&self.data_offset.to_le_bytes());
        buf
    }

    pub(crate) fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < HEADER_SIZE as usize {
            return Err(DiskError::CorruptImage("sparse header truncated"));
        }
        if buf[..8] != SPARSE_MAGIC {
            return Err(DiskError::CorruptImage("sparse magic mismatch"));
        }
        let version = u32::from_le_bytes(
            buf[8..12]
                .try_into()
                .map_err(|_| DiskError::CorruptImage("sparse header truncated"))?,
        );
        if version != SPARSE_VERSION {
            return Err(DiskError::Unsupported("sparse version"));
        }
        let sector_size = u32::from_le_bytes(
            buf[12..16]
                .try_into()
                .map_err(|_| DiskError::CorruptImage("sparse header truncated"))?,
        );
        let block_size = u32::from_le_bytes(
            buf[16..20]
                .try_into()
                .map_err(|_| DiskError::CorruptImage("sparse header truncated"))?,
        );
        let total_sectors = u64::from_le_bytes(
            buf[20..28]
                .try_into()
                .map_err(|_| DiskError::CorruptImage("sparse header truncated"))?,
        );
        let table_offset = u64::from_le_bytes(
            buf[28..36]
                .try_into()
                .map_err(|_| DiskError::CorruptImage("sparse header truncated"))?,
        );
        let table_entries = u64::from_le_bytes(
            buf[36..44]
                .try_into()
                .map_err(|_| DiskError::CorruptImage("sparse header truncated"))?,
        );
        let journal_offset = u64::from_le_bytes(
            buf[44..52]
                .try_into()
                .map_err(|_| DiskError::CorruptImage("sparse header truncated"))?,
        );
        let data_offset = u64::from_le_bytes(
            buf[52..60]
                .try_into()
                .map_err(|_| DiskError::CorruptImage("sparse header truncated"))?,
        );

        if sector_size != 512 && sector_size != 4096 {
            return Err(DiskError::Unsupported("sector size (expected 512 or 4096)"));
        }
        if block_size == 0 {
            return Err(DiskError::CorruptImage("invalid block size"));
        }
        if !(block_size as u64).is_multiple_of(sector_size as u64) {
            return Err(DiskError::CorruptImage(
                "block size must be multiple of sector size",
            ));
        }
        if block_size > MAX_BLOCK_SIZE_BYTES {
            return Err(DiskError::Unsupported("block size too large"));
        }
        if total_sectors == 0 {
            return Err(DiskError::CorruptImage("total sectors is zero"));
        }

        // Validate size math so later LBA->byte offset computation can't overflow.
        let total_bytes = total_sectors
            .checked_mul(sector_size as u64)
            .ok_or(DiskError::CorruptImage("disk size overflow"))?;

        let block_size_u64 = block_size as u64;
        let expected_entries = total_bytes.div_ceil(block_size_u64);
        if table_entries != expected_entries {
            return Err(DiskError::CorruptImage("table entries mismatch"));
        }

        let table_bytes = table_entries
            .checked_mul(8)
            .ok_or(DiskError::CorruptImage("table size overflow"))?;
        if table_bytes > MAX_TABLE_BYTES {
            return Err(DiskError::Unsupported("allocation table too large"));
        }

        if table_offset < HEADER_SIZE {
            return Err(DiskError::CorruptImage("table offset invalid"));
        }
        if journal_offset < HEADER_SIZE {
            return Err(DiskError::CorruptImage("journal offset invalid"));
        }
        if data_offset < HEADER_SIZE {
            return Err(DiskError::CorruptImage("data offset invalid"));
        }
        if !table_offset.is_multiple_of(8) || !journal_offset.is_multiple_of(8) {
            return Err(DiskError::CorruptImage("metadata offset misaligned"));
        }

        let journal_end = journal_offset
            .checked_add(JOURNAL_SIZE)
            .ok_or(DiskError::CorruptImage("journal offset overflow"))?;
        if journal_end > table_offset {
            return Err(DiskError::CorruptImage("journal overlaps allocation table"));
        }

        let table_end = table_offset
            .checked_add(table_bytes)
            .ok_or(DiskError::CorruptImage("table offset overflow"))?;
        if table_end > data_offset {
            return Err(DiskError::CorruptImage(
                "allocation table overlaps data region",
            ));
        }
        if !data_offset.is_multiple_of(block_size_u64) {
            return Err(DiskError::CorruptImage("data offset not block aligned"));
        }

        Ok(Self {
            sector_size,
            block_size,
            total_sectors,
            table_offset,
            table_entries,
            journal_offset,
            data_offset,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JournalRecord {
    state: u8,
    logical_block: u64,
    physical_offset: u64,
}

impl JournalRecord {
    fn empty() -> Self {
        Self {
            state: 0,
            logical_block: 0,
            physical_offset: 0,
        }
    }

    fn encode(&self) -> [u8; JOURNAL_SIZE as usize] {
        let mut buf = [0u8; JOURNAL_SIZE as usize];
        buf[..4].copy_from_slice(&JOURNAL_MAGIC);
        buf[4] = self.state;
        buf[8..16].copy_from_slice(&self.logical_block.to_le_bytes());
        buf[16..24].copy_from_slice(&self.physical_offset.to_le_bytes());
        buf
    }

    fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < JOURNAL_SIZE as usize {
            return Err(DiskError::CorruptImage("sparse journal truncated"));
        }
        if buf[..4] != JOURNAL_MAGIC {
            // Treat missing magic as empty journal for forward compatibility.
            return Ok(Self::empty());
        }
        let state = buf[4];
        let logical_block = u64::from_le_bytes(
            buf[8..16]
                .try_into()
                .map_err(|_| DiskError::CorruptImage("sparse journal truncated"))?,
        );
        let physical_offset = u64::from_le_bytes(
            buf[16..24]
                .try_into()
                .map_err(|_| DiskError::CorruptImage("sparse journal truncated"))?,
        );
        Ok(Self {
            state,
            logical_block,
            physical_offset,
        })
    }
}

pub struct SparseDiskV1<S> {
    storage: S,
    header: SparseHeaderV1,
    table: Vec<u64>,
}

impl<S: StorageBackend> SparseDiskV1<S> {
    pub fn create(
        mut storage: S,
        sector_size: u32,
        total_sectors: u64,
        block_size: u32,
    ) -> Result<Self> {
        if sector_size != 512 && sector_size != 4096 {
            return Err(DiskError::Unsupported("sector size (expected 512 or 4096)"));
        }
        if block_size == 0 || !(block_size as u64).is_multiple_of(sector_size as u64) {
            return Err(DiskError::Unsupported(
                "block size must be a multiple of sector size",
            ));
        }
        if block_size > MAX_BLOCK_SIZE_BYTES {
            return Err(DiskError::Unsupported("block size too large"));
        }

        let total_bytes = total_sectors
            .checked_mul(sector_size as u64)
            .ok_or(DiskError::Unsupported("disk size overflow"))?;
        let block_size_u64 = block_size as u64;
        let table_entries = total_bytes.div_ceil(block_size_u64);
        let table_bytes = table_entries
            .checked_mul(8)
            .ok_or(DiskError::Unsupported("table size overflow"))?;
        if table_bytes > MAX_TABLE_BYTES {
            return Err(DiskError::Unsupported("allocation table too large"));
        }

        let journal_offset = HEADER_SIZE;
        let table_offset = journal_offset
            .checked_add(JOURNAL_SIZE)
            .ok_or(DiskError::Unsupported("table offset overflow"))?;
        let table_end = table_offset
            .checked_add(table_bytes)
            .ok_or(DiskError::Unsupported("table offset overflow"))?;
        let data_offset = align_up(table_end, block_size_u64)?;

        let header = SparseHeaderV1 {
            sector_size,
            block_size,
            total_sectors,
            table_offset,
            table_entries,
            journal_offset,
            data_offset,
        };

        storage.write_at(0, &header.encode())?;
        storage.write_at(journal_offset, &JournalRecord::empty().encode())?;
        write_zeroes(&mut storage, table_offset, table_bytes)?;
        storage.set_len(data_offset)?;
        storage.flush()?;

        let table_entries_usize: usize = table_entries
            .try_into()
            .map_err(|_| DiskError::Unsupported("allocation table too large"))?;
        let mut table = Vec::new();
        table
            .try_reserve_exact(table_entries_usize)
            .map_err(|_| DiskError::QuotaExceeded)?;
        table.resize(table_entries_usize, 0);
        Ok(Self {
            storage,
            header,
            table,
        })
    }

    pub fn open(mut storage: S) -> Result<Self> {
        let mut header_buf = [0u8; HEADER_SIZE as usize];
        match storage.read_at(0, &mut header_buf) {
            Ok(()) => {}
            Err(DiskError::OutOfBounds { .. }) => {
                return Err(DiskError::CorruptImage("sparse header truncated"));
            }
            Err(e) => return Err(e),
        }
        let header = SparseHeaderV1::decode(&header_buf)?;

        let file_len = storage.len()?;
        if file_len < header.data_offset {
            return Err(DiskError::CorruptImage("sparse image truncated"));
        }

        let table_bytes = header
            .table_entries
            .checked_mul(8)
            .ok_or(DiskError::CorruptImage("table size overflow"))?;
        if table_bytes > MAX_TABLE_BYTES {
            return Err(DiskError::Unsupported("allocation table too large"));
        }
        let table_entries_usize: usize = header
            .table_entries
            .try_into()
            .map_err(|_| DiskError::Unsupported("allocation table too large"))?;

        // Read the allocation table without allocating an additional full-size temporary buffer.
        //
        // This keeps opens lightweight even for large-but-valid tables, and prevents aborting on
        // OOM for corrupt images that claim extreme table sizes.
        let mut table = Vec::new();
        table
            .try_reserve_exact(table_entries_usize)
            .map_err(|_| DiskError::QuotaExceeded)?;
        let table_bytes_usize: usize = table_bytes
            .try_into()
            .map_err(|_| DiskError::Unsupported("allocation table too large"))?;
        let mut buf = Vec::new();
        buf.try_reserve_exact(64 * 1024)
            .map_err(|_| DiskError::QuotaExceeded)?;
        buf.resize(64 * 1024, 0);
        let mut remaining = table_bytes_usize;
        let mut off = header.table_offset;
        while remaining > 0 {
            let read_len = remaining.min(buf.len());
            match storage.read_at(off, &mut buf[..read_len]) {
                Ok(()) => {}
                Err(DiskError::OutOfBounds { .. }) => {
                    return Err(DiskError::CorruptImage("allocation table out of bounds"));
                }
                Err(e) => return Err(e),
            }
            for chunk in buf[..read_len].chunks_exact(8) {
                let bytes: [u8; 8] = chunk
                    .try_into()
                    .map_err(|_| DiskError::CorruptImage("allocation table decode error"))?;
                table.push(u64::from_le_bytes(bytes));
            }
            off = off
                .checked_add(read_len as u64)
                .ok_or(DiskError::OffsetOverflow)?;
            remaining -= read_len;
        }

        let mut disk = Self {
            storage,
            header,
            table,
        };
        disk.recover_journal()?;
        Ok(disk)
    }

    pub fn header(&self) -> &SparseHeaderV1 {
        &self.header
    }

    pub fn allocated_blocks(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.table
            .iter()
            .enumerate()
            .filter_map(|(idx, &phys)| (phys != 0).then_some((idx as u64, phys)))
    }

    pub fn into_storage(self) -> S {
        self.storage
    }

    fn recover_journal(&mut self) -> Result<()> {
        let mut jbuf = [0u8; JOURNAL_SIZE as usize];
        self.storage
            .read_at(self.header.journal_offset, &mut jbuf)?;
        let record = JournalRecord::decode(&jbuf)?;
        if record.state == 0 {
            return Ok(());
        }
        if record.logical_block >= self.header.table_entries {
            return Err(DiskError::CorruptImage(
                "journal logical block out of range",
            ));
        }
        if record.physical_offset != 0
            && !record
                .physical_offset
                .is_multiple_of(self.header.block_size as u64)
        {
            return Err(DiskError::CorruptImage(
                "journal physical offset not aligned",
            ));
        }
        let idx: usize = record
            .logical_block
            .try_into()
            .map_err(|_| DiskError::CorruptImage("journal logical block out of range"))?;
        if idx >= self.table.len() {
            return Err(DiskError::CorruptImage(
                "journal logical block out of range",
            ));
        }
        let existing = self.table[idx];
        if existing != 0 && existing != record.physical_offset {
            return Err(DiskError::CorruptImage(
                "journal conflicts with allocation table",
            ));
        }
        self.table[idx] = record.physical_offset;
        self.write_table_entry(record.logical_block, record.physical_offset)?;

        // Clearing the journal is idempotent; if we crash before it lands on disk the record will
        // be replayed again on next open.
        self.storage
            .write_at(self.header.journal_offset, &JournalRecord::empty().encode())?;
        self.storage.flush()?;
        Ok(())
    }

    fn write_table_entry(&mut self, logical_block: u64, physical_offset: u64) -> Result<()> {
        let logical_block_bytes = logical_block
            .checked_mul(8)
            .ok_or(DiskError::Unsupported("table offset overflow"))?;
        let offset = self
            .header
            .table_offset
            .checked_add(logical_block_bytes)
            .ok_or(DiskError::Unsupported("table offset overflow"))?;
        self.storage
            .write_at(offset, &physical_offset.to_le_bytes())?;
        Ok(())
    }

    /// Total addressable bytes, independent of the [`VirtualDisk`] impl so the inherent helpers
    /// below can use it without carrying that trait's bounds.
    fn capacity(&self) -> u64 {
        self.header
            .total_sectors
            .saturating_mul(u64::from(self.header.sector_size))
    }

    /// Reject a byte range that runs past the end of the disk.
    fn check_range(&self, offset: u64, len: usize) -> Result<()> {
        let capacity = self.capacity();
        let end = offset
            .checked_add(len as u64)
            .ok_or(DiskError::OffsetOverflow)?;
        if end > capacity {
            return Err(DiskError::OutOfBounds {
                offset,
                len,
                capacity,
            });
        }
        Ok(())
    }

    /// Split the next chunk of a transfer at the block boundary it runs into.
    ///
    /// Returns the allocation-table index, the offset within that block, and how many bytes of the
    /// remaining transfer fall inside it.
    fn split_at_block(
        &self,
        at: u64,
        remaining: usize,
        block_size: u64,
    ) -> Result<(usize, usize, usize)> {
        let logical_block = at / block_size;
        let block_off = (at % block_size) as usize;
        let idx: usize = logical_block
            .try_into()
            .map_err(|_| DiskError::CorruptImage("logical block out of range"))?;
        if idx >= self.table.len() {
            return Err(DiskError::CorruptImage("logical block out of range"));
        }
        let to_copy = (block_size as usize)
            .saturating_sub(block_off)
            .min(remaining);
        Ok((idx, block_off, to_copy))
    }

    fn allocate_block(&mut self) -> Result<u64> {
        let block_size = self.header.block_size as u64;
        let mut len = self.storage.len()?;
        if len < self.header.data_offset {
            len = self.header.data_offset;
        }
        let offset = align_up(len, block_size)?;
        // Ensure the newly allocated block is fully zero-initialized so partial writes preserve
        // the semantics of unallocated blocks returning zero.
        write_zeroes(&mut self.storage, offset, block_size)?;
        Ok(offset)
    }
}

impl<S: StorageBackend + VirtualDiskSend> VirtualDisk for SparseDiskV1<S> {
    fn capacity_bytes(&self) -> u64 {
        self.capacity()
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.check_range(offset, buf.len())?;
        let block_size = u64::from(self.header.block_size);

        let mut remaining = buf;
        let mut at = offset;
        while !remaining.is_empty() {
            let (idx, block_off, to_copy) = self.split_at_block(at, remaining.len(), block_size)?;
            let physical = self.table[idx];

            if physical == 0 {
                // An unallocated block has never been written, and reads as zeros.
                remaining[..to_copy].fill(0);
            } else {
                let phys = physical
                    .checked_add(block_off as u64)
                    .ok_or(DiskError::CorruptImage("physical offset overflow"))?;
                self.storage.read_at(phys, &mut remaining[..to_copy])?;
            }

            remaining = &mut remaining[to_copy..];
            at += to_copy as u64;
        }
        Ok(())
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<()> {
        self.check_range(offset, buf.len())?;
        let block_size = u64::from(self.header.block_size);

        let mut remaining = buf;
        let mut at = offset;
        while !remaining.is_empty() {
            let (idx, block_off, to_copy) = self.split_at_block(at, remaining.len(), block_size)?;

            if self.table[idx] == 0 {
                // Writing zeros to a block that was never allocated leaves it sparse: the read
                // path already returns zeros for it, so allocating would only grow the image.
                if remaining[..to_copy].iter().all(|b| *b == 0) {
                    remaining = &remaining[to_copy..];
                    at += to_copy as u64;
                    continue;
                }

                let new_physical = self.allocate_block()?;
                let phys = new_physical
                    .checked_add(block_off as u64)
                    .ok_or(DiskError::CorruptImage("physical offset overflow"))?;
                self.storage.write_at(phys, &remaining[..to_copy])?;

                // Allocation spans two writes — the data region grew, and the table entry has to
                // point at it. The journal records the intent so that a crash between them can be
                // finished on the next open instead of losing the block.
                let logical_block = idx as u64;
                let rec = JournalRecord {
                    state: 1,
                    logical_block,
                    physical_offset: new_physical,
                };
                self.storage
                    .write_at(self.header.journal_offset, &rec.encode())?;
                self.storage.flush()?;

                self.table[idx] = new_physical;
                self.write_table_entry(logical_block, new_physical)?;
                self.storage.flush()?;

                self.storage
                    .write_at(self.header.journal_offset, &JournalRecord::empty().encode())?;
                // The table entry is the source of truth, so clearing the journal is not needed
                // for correctness. Doing it here keeps the next open from replaying a record that
                // has already been applied.
                self.storage.flush()?;
            } else {
                let phys = self.table[idx]
                    .checked_add(block_off as u64)
                    .ok_or(DiskError::CorruptImage("physical offset overflow"))?;
                self.storage.write_at(phys, &remaining[..to_copy])?;
            }

            remaining = &remaining[to_copy..];
            at += to_copy as u64;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.storage.flush()
    }
}

fn align_up(value: u64, align: u64) -> Result<u64> {
    if align == 0 {
        return Ok(value);
    }
    let rem = value % align;
    if rem == 0 {
        Ok(value)
    } else {
        value
            .checked_add(align - rem)
            .ok_or(DiskError::Unsupported("offset overflow"))
    }
}

fn write_zeroes<S: StorageBackend>(storage: &mut S, mut offset: u64, mut len: u64) -> Result<()> {
    const CHUNK: u64 = 64 * 1024;
    let buf = [0u8; CHUNK as usize];

    while len > 0 {
        let to_write = len.min(CHUNK);
        storage.write_at(offset, &buf[..to_write as usize])?;
        offset = offset
            .checked_add(to_write)
            .ok_or(DiskError::Unsupported("offset overflow"))?;
        len -= to_write;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SECTOR_SIZE: u32 = 512;
    const TEST_TOTAL_SECTORS: u64 = 128;
    const TEST_BLOCK_SIZE: u32 = 4096;

    #[derive(Default, Clone)]
    struct MemStorage {
        data: Vec<u8>,
    }

    impl StorageBackend for MemStorage {
        fn len(&mut self) -> crate::Result<u64> {
            Ok(self.data.len() as u64)
        }

        fn set_len(&mut self, len: u64) -> crate::Result<()> {
            let len_usize: usize = len
                .try_into()
                .map_err(|_| crate::DiskError::OffsetOverflow)?;
            self.data.resize(len_usize, 0);
            Ok(())
        }

        fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> crate::Result<()> {
            let offset_usize: usize = offset
                .try_into()
                .map_err(|_| crate::DiskError::OffsetOverflow)?;
            let end = offset_usize
                .checked_add(buf.len())
                .ok_or(crate::DiskError::OffsetOverflow)?;
            if end > self.data.len() {
                return Err(crate::DiskError::OutOfBounds {
                    offset,
                    len: buf.len(),
                    capacity: self.data.len() as u64,
                });
            }
            buf.copy_from_slice(&self.data[offset_usize..end]);
            Ok(())
        }

        fn write_at(&mut self, offset: u64, buf: &[u8]) -> crate::Result<()> {
            let offset_usize: usize = offset
                .try_into()
                .map_err(|_| crate::DiskError::OffsetOverflow)?;
            let end = offset_usize
                .checked_add(buf.len())
                .ok_or(crate::DiskError::OffsetOverflow)?;
            if end > self.data.len() {
                self.data.resize(end, 0);
            }
            self.data[offset_usize..end].copy_from_slice(buf);
            Ok(())
        }

        fn flush(&mut self) -> crate::Result<()> {
            Ok(())
        }
    }

    fn write_reference(reference: &mut [u8], lba: u64, data: &[u8]) {
        let start = (lba * TEST_SECTOR_SIZE as u64) as usize;
        let end = start + data.len();
        reference[start..end].copy_from_slice(data);
    }

    fn make_pattern(len: usize, seed: u8) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        for (idx, b) in buf.iter_mut().enumerate() {
            // Simple deterministic pattern; guaranteed not to be all-zero for seed != 0.
            *b = seed.wrapping_add((idx as u8).wrapping_mul(31));
        }
        buf
    }

    #[test]
    fn header_roundtrip() {
        let header = SparseHeaderV1 {
            sector_size: 512,
            block_size: 1024 * 1024,
            total_sectors: 1024,
            table_offset: 4096 + 4096,
            table_entries: 1,
            journal_offset: 4096,
            data_offset: 1024 * 1024,
        };
        let enc = header.encode();
        let dec = SparseHeaderV1::decode(&enc).unwrap();
        assert_eq!(header, dec);
    }

    #[test]
    fn decode_rejects_total_size_overflow() {
        let header = SparseHeaderV1 {
            sector_size: 512,
            block_size: 512,
            total_sectors: u64::MAX,
            table_offset: 8192,
            table_entries: 1,
            journal_offset: 4096,
            data_offset: 8192,
        };
        let enc = header.encode();
        let err = SparseHeaderV1::decode(&enc).unwrap_err();
        assert!(matches!(err, DiskError::CorruptImage("disk size overflow")));
    }

    #[test]
    fn decode_rejects_table_entries_mismatch() {
        // total_sectors=1024, sector_size=512 => 512KiB image. With block_size=1MiB the expected
        // table entries is 1; claim 2 and ensure the header is rejected.
        let header = SparseHeaderV1 {
            sector_size: 512,
            block_size: 1024 * 1024,
            total_sectors: 1024,
            table_offset: 8192,
            table_entries: 2,
            journal_offset: 4096,
            data_offset: 1024 * 1024,
        };
        let enc = header.encode();
        let err = SparseHeaderV1::decode(&enc).unwrap_err();
        assert!(matches!(
            err,
            DiskError::CorruptImage("table entries mismatch")
        ));
    }

    #[test]
    fn decode_rejects_allocation_table_too_large() {
        // Construct the smallest valid header that still exceeds the MAX_TABLE_BYTES guard.
        let table_entries = (MAX_TABLE_BYTES / 8) + 1;

        let header = SparseHeaderV1 {
            sector_size: 512,
            block_size: 512,
            total_sectors: table_entries,
            table_offset: 8192,
            table_entries,
            journal_offset: 4096,
            data_offset: 8192,
        };
        let enc = header.encode();
        let err = SparseHeaderV1::decode(&enc).unwrap_err();
        assert!(matches!(
            err,
            DiskError::Unsupported("allocation table too large")
        ));
    }

    #[test]
    fn decode_rejects_block_size_too_large() {
        let header = SparseHeaderV1 {
            sector_size: 512,
            block_size: MAX_BLOCK_SIZE_BYTES + 512,
            total_sectors: 1024,
            table_offset: 8192,
            table_entries: 1,
            journal_offset: 4096,
            data_offset: (MAX_BLOCK_SIZE_BYTES as u64) + 512,
        };
        let enc = header.encode();
        let err = SparseHeaderV1::decode(&enc).unwrap_err();
        assert!(matches!(
            err,
            DiskError::Unsupported("block size too large")
        ));
    }

    #[test]
    fn unallocated_reads_zero() {
        let storage = MemStorage::default();
        let mut disk = SparseDiskV1::create(
            storage,
            TEST_SECTOR_SIZE,
            TEST_TOTAL_SECTORS,
            TEST_BLOCK_SIZE,
        )
        .unwrap();
        let mut buf = vec![0xAA; 512 * 4];
        disk.read_sectors(0, &mut buf).unwrap();
        assert!(buf.iter().all(|b| *b == 0));
    }

    #[test]
    fn create_open_read_write_roundtrip() {
        let storage = MemStorage::default();
        let mut disk = SparseDiskV1::create(
            storage,
            TEST_SECTOR_SIZE,
            TEST_TOTAL_SECTORS,
            TEST_BLOCK_SIZE,
        )
        .unwrap();

        let disk_bytes = (TEST_TOTAL_SECTORS * TEST_SECTOR_SIZE as u64) as usize;
        let mut reference = vec![0u8; disk_bytes];

        // Within one block (block 0, sectors 1..3).
        let write1_lba = 1;
        let write1 = make_pattern(TEST_SECTOR_SIZE as usize * 2, 0x11);
        disk.write_sectors(write1_lba, &write1).unwrap();
        write_reference(&mut reference, write1_lba, &write1);

        // Across a block boundary (block 0 -> 1).
        //
        // block_size=4096 and sector_size=512 => 8 sectors per block, so lba=7 straddles the
        // boundary between block 0 and block 1.
        let write2_lba = 7;
        let write2 = make_pattern(TEST_SECTOR_SIZE as usize * 3, 0x55);
        disk.write_sectors(write2_lba, &write2).unwrap();
        write_reference(&mut reference, write2_lba, &write2);

        // Non-contiguous LBA (block 5).
        let write3_lba = 40;
        let write3 = make_pattern(TEST_SECTOR_SIZE as usize, 0xA3);
        disk.write_sectors(write3_lba, &write3).unwrap();
        write_reference(&mut reference, write3_lba, &write3);

        disk.flush().unwrap();

        let storage = disk.into_storage();
        let mut disk = SparseDiskV1::open(storage).unwrap();

        // Verify sparse semantics: unwritten blocks should read back as zeroes.
        let mut hole = vec![0xCC; TEST_SECTOR_SIZE as usize * 2];
        disk.read_sectors(20, &mut hole).unwrap();
        assert!(hole.iter().all(|b| *b == 0));

        // Verify the entire logical image matches our reference bytes.
        let mut read_back = vec![0u8; reference.len()];
        disk.read_sectors(0, &mut read_back).unwrap();
        assert_eq!(read_back, reference);

        // Sanity-check that only the touched logical blocks are allocated.
        let allocated: Vec<_> = disk.allocated_blocks().collect();
        assert_eq!(allocated.len(), 3);
    }

    #[test]
    fn journal_replay_on_open_updates_table_and_clears_journal() {
        let storage = MemStorage::default();
        let disk = SparseDiskV1::create(
            storage,
            TEST_SECTOR_SIZE,
            TEST_TOTAL_SECTORS,
            TEST_BLOCK_SIZE,
        )
        .unwrap();
        let mut storage = disk.into_storage();

        let mut header_buf = [0u8; HEADER_SIZE as usize];
        storage.read_at(0, &mut header_buf).unwrap();
        let header = SparseHeaderV1::decode(&header_buf).unwrap();

        let logical_block = 3u64;
        let physical_offset = header.data_offset;

        // Ensure the backing file contains the referenced physical block.
        storage
            .set_len(physical_offset + header.block_size as u64)
            .unwrap();
        storage
            .write_at(physical_offset, &vec![0x5Au8; header.block_size as usize])
            .unwrap();

        // Confirm the allocation table entry starts empty (we're simulating a crash after writing
        // the journal record but before committing the table entry).
        let mut table_entry = [0u8; 8];
        storage
            .read_at(header.table_offset + logical_block * 8, &mut table_entry)
            .unwrap();
        assert_eq!(u64::from_le_bytes(table_entry), 0);

        // Inject a journal record that should be replayed on open.
        let rec = JournalRecord {
            state: 1,
            logical_block,
            physical_offset,
        };
        storage
            .write_at(header.journal_offset, &rec.encode())
            .unwrap();

        let mut disk = SparseDiskV1::open(storage).unwrap();
        assert!(
            disk.allocated_blocks()
                .any(|(l, p)| l == logical_block && p == physical_offset),
            "expected journal record to be replayed into allocation table"
        );

        // Verify reads go through the replayed mapping.
        let sectors_per_block = header.block_size as u64 / header.sector_size as u64;
        let lba = logical_block * sectors_per_block;
        let mut buf = vec![0u8; header.sector_size as usize];
        disk.read_sectors(lba, &mut buf).unwrap();
        assert!(buf.iter().all(|b| *b == 0x5A));

        let header = *disk.header();
        let mut storage = disk.into_storage();

        // The allocation table should have been updated on disk.
        let mut table_entry = [0u8; 8];
        storage
            .read_at(header.table_offset + logical_block * 8, &mut table_entry)
            .unwrap();
        assert_eq!(u64::from_le_bytes(table_entry), physical_offset);

        // The journal should have been cleared.
        let mut jbuf = [0u8; JOURNAL_SIZE as usize];
        storage.read_at(header.journal_offset, &mut jbuf).unwrap();
        assert_eq!(jbuf, JournalRecord::empty().encode());

        // Reopen once more to ensure the cleared journal doesn't get re-applied and the mapping
        // persists via the table entry.
        let disk = SparseDiskV1::open(storage).unwrap();
        assert!(
            disk.allocated_blocks()
                .any(|(l, p)| l == logical_block && p == physical_offset),
            "expected allocation table entry to persist after journal replay"
        );
    }

    #[test]
    fn journal_conflict_detection() {
        let storage = MemStorage::default();
        let mut disk = SparseDiskV1::create(
            storage,
            TEST_SECTOR_SIZE,
            TEST_TOTAL_SECTORS,
            TEST_BLOCK_SIZE,
        )
        .unwrap();

        // Allocate logical block 0 normally.
        disk.write_sectors(0, &make_pattern(TEST_SECTOR_SIZE as usize, 0xF0))
            .unwrap();
        disk.flush().unwrap();

        let existing_phys = disk
            .allocated_blocks()
            .find(|(l, _)| *l == 0)
            .expect("logical block 0 should be allocated")
            .1;

        let header = *disk.header();
        let mut storage = disk.into_storage();

        // Inject a conflicting journal record for the same logical block, pointing at a different
        // physical offset.
        let conflicting_phys = existing_phys + header.block_size as u64;
        storage
            .set_len(conflicting_phys + header.block_size as u64)
            .unwrap();
        let rec = JournalRecord {
            state: 1,
            logical_block: 0,
            physical_offset: conflicting_phys,
        };
        storage
            .write_at(header.journal_offset, &rec.encode())
            .unwrap();

        let err = match SparseDiskV1::open(storage) {
            Ok(_) => panic!("expected open to fail due to journal/table conflict"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            DiskError::CorruptImage("journal conflicts with allocation table")
        ));
    }
}
