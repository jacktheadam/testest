//! In-place editing of a REGF hive.
//!
//! # Status: written, unit-tested, and not wired in
//!
//! `patch_bcd_store` still uses the rebuild path. This module was built on the
//! theory that the rebuild was why bootmgr rejected patched media, and that
//! theory did not survive its controls: a hive edited in place — same file size,
//! 0.66% of bytes changed — is rejected exactly as the rebuilt one is, and QEMU
//! rejects both too, which rules out the emulator. The fault is somewhere in
//! what the elements say, not in how the hive is written. See kernel debugging
//! over COM1 in the wiki's debugging area.
//!
//! It is kept because it is the better shape for the job once that is fixed, and
//! because it does not pass the existing integration tests against the synthetic
//! fixture hive yet — wiring it in would have broken a working tool for no gain.
//!
//! # Why this exists
//!
//! The original implementation read a hive, rebuilt it through a hive *builder*,
//! and wrote the result. That produces a file which parses correctly — the
//! crate's own reader accepts it, and so do the tests — but which Windows Boot
//! Manager refuses. A Win7 install ISO whose `boot/bcd` has been round-tripped
//! that way faults in early boot at `rip=0x03ea`, executing interrupt-vector
//! table content: bootmgr read the store, something it needed was not there, and
//! it jumped into nothing.
//!
//! That was verified rather than guessed. Rewriting the store with *every* patch
//! flag turned off — a pure read-then-write with no semantic change — fails
//! identically, so the rewrite itself is what bootmgr rejects, not any element
//! this tool sets.
//!
//! # The approach
//!
//! Start from the original bytes and change as little as possible: allocate
//! cells out of the hive's existing free space, write the new records, link them
//! into their parent, and fix the header checksum. Everything bootmgr already
//! accepts stays byte-identical.
//!
//! There is room to do this. A Win7 `boot/bcd` is 262,144 bytes, of which the
//! declared bins occupy 24,576 — 11,680 allocated and 12,704 free, with
//! individual free cells as large as 4,064 bytes — and 233 KiB of the file sits
//! beyond the last bin, available for a new bin if the free cells run out.
//!
//! # Format notes
//!
//! Offsets stored inside a hive are relative to the end of the 4 KiB base block,
//! so an absolute file offset is `4096 + relative`. A cell's first four bytes are
//! a signed length: negative means allocated, positive means free, and the
//! magnitude includes the length field itself. Cells are 8-byte aligned.

use anyhow::{anyhow, bail, Result};

const BASE_BLOCK: usize = 4096;
const HBIN_HEADER: usize = 32;
const CELL_ALIGN: usize = 8;

/// Offsets within the base block.
mod base {
    pub const ROOT_CELL: usize = 36;
    pub const HBINS_SIZE: usize = 40;
    pub const CHECKSUM: usize = 508;
}

/// Offsets within an `nk` (key node) record, from the start of its cell payload.
mod nk {
    pub const SIGNATURE: usize = 0x00;
    pub const FLAGS: usize = 0x02;
    pub const LAST_WRITTEN: usize = 0x04;
    pub const PARENT: usize = 0x10;
    pub const SUBKEY_COUNT: usize = 0x14;
    pub const SUBKEYS_LIST: usize = 0x1C;
    pub const VALUES_COUNT: usize = 0x24;
    pub const VALUES_LIST: usize = 0x28;
    pub const SECURITY: usize = 0x2C;
    pub const CLASSNAME: usize = 0x30;
    pub const MAX_SUBKEY_NAME_LEN: usize = 0x34;
    pub const MAX_VALUE_NAME_LEN: usize = 0x3C;
    pub const MAX_VALUE_DATA_SIZE: usize = 0x40;
    pub const NAME_LENGTH: usize = 0x48;
    pub const CLASSNAME_LENGTH: usize = 0x4A;
    pub const HEADER_LEN: usize = 0x4C;
    /// Key name is stored as single-byte characters rather than UTF-16.
    pub const FLAG_COMP_NAME: u16 = 0x0020;
}

/// Offsets within a `vk` (value) record.
mod vk {
    pub const SIGNATURE: usize = 0x00;
    pub const NAME_LENGTH: usize = 0x02;
    pub const DATA_SIZE: usize = 0x04;
    pub const DATA_OFFSET: usize = 0x08;
    pub const DATA_TYPE: usize = 0x0C;
    pub const FLAGS: usize = 0x10;
    pub const HEADER_LEN: usize = 0x14;
    /// Value name is stored as single-byte characters.
    pub const FLAG_COMP_NAME: u16 = 0x0001;
}

/// A hive held in memory, edited in place.
pub struct InPlaceHive {
    data: Vec<u8>,
}

impl InPlaceHive {
    pub fn new(data: Vec<u8>) -> Result<Self> {
        if data.len() < BASE_BLOCK + HBIN_HEADER {
            bail!("hive is too small to contain a base block and one bin");
        }
        if &data[0..4] != b"regf" {
            bail!("not a REGF hive (missing signature)");
        }
        Ok(Self { data })
    }

    pub fn into_bytes(mut self) -> Vec<u8> {
        self.update_checksum();
        self.data
    }

    fn read_u32(&self, at: usize) -> u32 {
        u32::from_le_bytes(self.data[at..at + 4].try_into().unwrap())
    }

    fn write_u32(&mut self, at: usize, v: u32) {
        self.data[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn read_u16(&self, at: usize) -> u16 {
        u16::from_le_bytes(self.data[at..at + 2].try_into().unwrap())
    }

    fn write_u16(&mut self, at: usize, v: u16) {
        self.data[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn hbins_size(&self) -> usize {
        self.read_u32(base::HBINS_SIZE) as usize
    }

    /// Absolute file offset of a cell payload, given a hive-relative cell offset.
    fn cell_payload(&self, rel: u32) -> usize {
        BASE_BLOCK + rel as usize + 4
    }

    pub fn root_cell(&self) -> u32 {
        self.read_u32(base::ROOT_CELL)
    }

    /// The XOR-of-u32 checksum bootmgr validates before trusting the base block.
    fn update_checksum(&mut self) {
        let mut sum: u32 = 0;
        for chunk in self.data[..base::CHECKSUM].chunks_exact(4) {
            sum ^= u32::from_le_bytes(chunk.try_into().unwrap());
        }
        if sum == 0xFFFF_FFFF {
            sum = 0xFFFF_FFFE;
        }
        if sum == 0 {
            sum = 1;
        }
        self.write_u32(base::CHECKSUM, sum);
    }

    /// Allocate `need` payload bytes, returning a hive-relative cell offset.
    ///
    /// Prefers splitting an existing free cell; falls back to appending a bin
    /// when the file has room beyond the declared bins.
    fn alloc(&mut self, need: usize) -> Result<u32> {
        let want = align_up(need + 4, CELL_ALIGN);

        // First fit over the free cells, which keeps the edit local.
        let mut bin = BASE_BLOCK;
        let bins_end = BASE_BLOCK + self.hbins_size();
        while bin + HBIN_HEADER <= bins_end {
            if &self.data[bin..bin + 4] != b"hbin" {
                break;
            }
            let bin_size = self.read_u32(bin + 8) as usize;
            if bin_size == 0 {
                break;
            }
            let mut cur = bin + HBIN_HEADER;
            let end = (bin + bin_size).min(self.data.len());
            while cur + 4 <= end {
                let size = i32::from_le_bytes(self.data[cur..cur + 4].try_into().unwrap());
                if size == 0 {
                    break;
                }
                let len = size.unsigned_abs() as usize;
                if size > 0 && len >= want {
                    // Split when the remainder is big enough to be a usable cell.
                    let remainder = len - want;
                    if remainder >= CELL_ALIGN {
                        self.data[cur + want..cur + want + 4]
                            .copy_from_slice(&(remainder as i32).to_le_bytes());
                        self.data[cur..cur + 4].copy_from_slice(&(-(want as i32)).to_le_bytes());
                    } else {
                        self.data[cur..cur + 4].copy_from_slice(&(-(len as i32)).to_le_bytes());
                    }
                    let rel = (cur - BASE_BLOCK) as u32;
                    let payload = self.cell_payload(rel);
                    let payload_len = self.data[cur..].len().min(want) - 4;
                    self.data[payload..payload + payload_len].fill(0);
                    return Ok(rel);
                }
                cur += len;
            }
            bin += bin_size;
        }

        self.append_bin(want)
    }

    /// Add a new bin past the declared bins, if the file has room for it.
    fn append_bin(&mut self, want: usize) -> Result<u32> {
        const BIN_SIZE: usize = 4096;
        let bin_size = align_up(want + HBIN_HEADER, BIN_SIZE);
        let bin_off = BASE_BLOCK + self.hbins_size();

        if bin_off + bin_size > self.data.len() {
            bail!(
                "no room for a new bin: need {} bytes at offset {}, file is {}",
                bin_size,
                bin_off,
                self.data.len()
            );
        }

        self.data[bin_off..bin_off + bin_size].fill(0);
        self.data[bin_off..bin_off + 4].copy_from_slice(b"hbin");
        self.write_u32(bin_off + 4, (bin_off - BASE_BLOCK) as u32);
        self.write_u32(bin_off + 8, bin_size as u32);

        // One allocated cell, then the rest of the bin as a single free cell.
        let cell = bin_off + HBIN_HEADER;
        self.data[cell..cell + 4].copy_from_slice(&(-(want as i32)).to_le_bytes());
        let rest = bin_size - HBIN_HEADER - want;
        if rest >= CELL_ALIGN {
            let free = cell + want;
            self.data[free..free + 4].copy_from_slice(&(rest as i32).to_le_bytes());
        }

        let new_size = self.hbins_size() + bin_size;
        self.write_u32(base::HBINS_SIZE, new_size as u32);
        Ok((cell - BASE_BLOCK) as u32)
    }

    /// Find a direct subkey by name, case-insensitively.
    pub fn find_subkey(&self, parent: u32, name: &str) -> Option<u32> {
        self.subkey_offsets(parent)
            .into_iter()
            .find(|&child| self.key_name(child).eq_ignore_ascii_case(name))
    }

    fn key_name(&self, key: u32) -> String {
        let p = self.cell_payload(key);
        let len = self.read_u16(p + nk::NAME_LENGTH) as usize;
        let flags = self.read_u16(p + nk::FLAGS);
        let bytes = &self.data[p + nk::HEADER_LEN..p + nk::HEADER_LEN + len];
        if flags & nk::FLAG_COMP_NAME != 0 {
            bytes.iter().map(|&b| b as char).collect()
        } else {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
    }

    fn subkey_offsets(&self, parent: u32) -> Vec<u32> {
        let p = self.cell_payload(parent);
        let count = self.read_u32(p + nk::SUBKEY_COUNT);
        let list = self.read_u32(p + nk::SUBKEYS_LIST);
        if count == 0 || list == u32::MAX {
            return Vec::new();
        }
        let lp = self.cell_payload(list);
        let sig = &self.data[lp..lp + 2];
        let n = self.read_u16(lp + 2) as usize;
        let mut out = Vec::with_capacity(n);
        match sig {
            b"lf" | b"lh" => {
                for i in 0..n {
                    out.push(self.read_u32(lp + 4 + i * 8));
                }
            }
            b"li" => {
                for i in 0..n {
                    out.push(self.read_u32(lp + 4 + i * 4));
                }
            }
            b"ri" => {
                for i in 0..n {
                    let sub = self.read_u32(lp + 4 + i * 4);
                    out.extend(self.list_entries(sub));
                }
            }
            _ => {}
        }
        out
    }

    fn list_entries(&self, list: u32) -> Vec<u32> {
        let lp = self.cell_payload(list);
        let sig = &self.data[lp..lp + 2];
        let n = self.read_u16(lp + 2) as usize;
        let mut out = Vec::with_capacity(n);
        match sig {
            b"lf" | b"lh" => {
                for i in 0..n {
                    out.push(self.read_u32(lp + 4 + i * 8));
                }
            }
            b"li" => {
                for i in 0..n {
                    out.push(self.read_u32(lp + 4 + i * 4));
                }
            }
            _ => {}
        }
        out
    }

    /// Get or create a direct subkey, returning its cell offset.
    pub fn ensure_subkey(&mut self, parent: u32, name: &str) -> Result<u32> {
        if let Some(existing) = self.find_subkey(parent, name) {
            return Ok(existing);
        }

        let key = self.create_key_record(parent, name)?;
        self.link_subkey(parent, key, name)?;
        Ok(key)
    }

    fn create_key_record(&mut self, parent: u32, name: &str) -> Result<u32> {
        if !name.is_ascii() {
            bail!("only ASCII key names are supported ({name})");
        }
        let rel = self.alloc(nk::HEADER_LEN + name.len())?;
        let p = self.cell_payload(rel);

        // Inherit the parent's security descriptor and timestamp: a BCD store has
        // one descriptor shared by every key, and matching the parent keeps the
        // edit invisible to anything that compares them.
        let pp = self.cell_payload(parent);
        let security = self.read_u32(pp + nk::SECURITY);
        let timestamp = self.data[pp + nk::LAST_WRITTEN..pp + nk::LAST_WRITTEN + 8].to_vec();

        self.data[p + nk::SIGNATURE..p + nk::SIGNATURE + 2].copy_from_slice(b"nk");
        self.write_u16(p + nk::FLAGS, nk::FLAG_COMP_NAME);
        self.data[p + nk::LAST_WRITTEN..p + nk::LAST_WRITTEN + 8].copy_from_slice(&timestamp);
        self.write_u32(p + nk::PARENT, parent);
        self.write_u32(p + nk::SUBKEY_COUNT, 0);
        self.write_u32(p + nk::SUBKEYS_LIST, u32::MAX);
        self.write_u32(p + nk::VALUES_COUNT, 0);
        self.write_u32(p + nk::VALUES_LIST, u32::MAX);
        self.write_u32(p + nk::SECURITY, security);
        self.write_u32(p + nk::CLASSNAME, u32::MAX);
        self.write_u32(p + nk::MAX_SUBKEY_NAME_LEN, 0);
        self.write_u32(p + nk::MAX_VALUE_NAME_LEN, 0);
        self.write_u32(p + nk::MAX_VALUE_DATA_SIZE, 0);
        self.write_u16(p + nk::NAME_LENGTH, name.len() as u16);
        self.write_u16(p + nk::CLASSNAME_LENGTH, 0);
        self.data[p + nk::HEADER_LEN..p + nk::HEADER_LEN + name.len()]
            .copy_from_slice(name.as_bytes());
        Ok(rel)
    }

    /// Append a subkey to the parent's list, creating the list if absent.
    ///
    /// The list's hash type must match what is already there: an `lh` list stores
    /// a rolling hash of the name and an `lf` list stores its first four
    /// characters, and a lookup that disagrees with the stored hash will not find
    /// the key.
    fn link_subkey(&mut self, parent: u32, key: u32, name: &str) -> Result<()> {
        let pp = self.cell_payload(parent);
        let count = self.read_u32(pp + nk::SUBKEY_COUNT);
        let list = self.read_u32(pp + nk::SUBKEYS_LIST);

        let (sig, entries) = if count == 0 || list == u32::MAX {
            (*b"lf", Vec::new())
        } else {
            let lp = self.cell_payload(list);
            let sig: [u8; 2] = self.data[lp..lp + 2].try_into().unwrap();
            if &sig != b"lf" && &sig != b"lh" {
                bail!(
                    "unsupported subkey list type {:?}; refusing to edit in place",
                    String::from_utf8_lossy(&sig)
                );
            }
            let n = self.read_u16(lp + 2) as usize;
            let mut entries = Vec::with_capacity(n);
            for i in 0..n {
                entries.push((self.read_u32(lp + 4 + i * 8), self.read_u32(lp + 8 + i * 8)));
            }
            (sig, entries)
        };

        let hash = if &sig == b"lh" {
            lh_hash(name)
        } else {
            lf_hash(name)
        };

        // Keys are kept in name order; Windows binary-searches the list.
        let mut merged = entries;
        merged.push((key, hash));
        merged.sort_by(|a, b| {
            self.key_name(a.0)
                .to_ascii_uppercase()
                .cmp(&self.key_name(b.0).to_ascii_uppercase())
        });

        let new_list = self.alloc(4 + merged.len() * 8)?;
        let lp = self.cell_payload(new_list);
        self.data[lp..lp + 2].copy_from_slice(&sig);
        self.write_u16(lp + 2, merged.len() as u16);
        for (i, (off, h)) in merged.iter().enumerate() {
            self.write_u32(lp + 4 + i * 8, *off);
            self.write_u32(lp + 8 + i * 8, *h);
        }

        let pp = self.cell_payload(parent);
        self.write_u32(pp + nk::SUBKEY_COUNT, merged.len() as u32);
        self.write_u32(pp + nk::SUBKEYS_LIST, new_list);
        let max_name = self.read_u32(pp + nk::MAX_SUBKEY_NAME_LEN);
        if (name.len() as u32) > max_name {
            self.write_u32(pp + nk::MAX_SUBKEY_NAME_LEN, name.len() as u32);
        }
        Ok(())
    }

    /// Set a binary value on a key, replacing any existing value of that name.
    pub fn set_binary_value(&mut self, key: u32, name: &str, data: &[u8]) -> Result<()> {
        const REG_BINARY: u32 = 3;

        let existing = self.find_value(key, name);
        let data_cell = self.alloc(data.len())?;
        let dp = self.cell_payload(data_cell);
        self.data[dp..dp + data.len()].copy_from_slice(data);

        let value = match existing {
            Some(v) => v,
            None => {
                let v = self.alloc(vk::HEADER_LEN + name.len())?;
                self.append_value(key, v)?;
                v
            }
        };

        let vp = self.cell_payload(value);
        self.data[vp + vk::SIGNATURE..vp + vk::SIGNATURE + 2].copy_from_slice(b"vk");
        self.write_u16(vp + vk::NAME_LENGTH, name.len() as u16);
        self.write_u32(vp + vk::DATA_SIZE, data.len() as u32);
        self.write_u32(vp + vk::DATA_OFFSET, data_cell);
        self.write_u32(vp + vk::DATA_TYPE, REG_BINARY);
        self.write_u16(vp + vk::FLAGS, vk::FLAG_COMP_NAME);
        self.data[vp + vk::HEADER_LEN..vp + vk::HEADER_LEN + name.len()]
            .copy_from_slice(name.as_bytes());

        let kp = self.cell_payload(key);
        let max_name = self.read_u32(kp + nk::MAX_VALUE_NAME_LEN);
        if (name.len() as u32) > max_name {
            self.write_u32(kp + nk::MAX_VALUE_NAME_LEN, name.len() as u32);
        }
        let max_data = self.read_u32(kp + nk::MAX_VALUE_DATA_SIZE);
        if (data.len() as u32) > max_data {
            self.write_u32(kp + nk::MAX_VALUE_DATA_SIZE, data.len() as u32);
        }
        Ok(())
    }

    fn find_value(&self, key: u32, name: &str) -> Option<u32> {
        let kp = self.cell_payload(key);
        let count = self.read_u32(kp + nk::VALUES_COUNT);
        let list = self.read_u32(kp + nk::VALUES_LIST);
        if count == 0 || list == u32::MAX {
            return None;
        }
        let lp = self.cell_payload(list);
        for i in 0..count as usize {
            let v = self.read_u32(lp + i * 4);
            let vp = self.cell_payload(v);
            let len = self.read_u16(vp + vk::NAME_LENGTH) as usize;
            let got: String = self.data[vp + vk::HEADER_LEN..vp + vk::HEADER_LEN + len]
                .iter()
                .map(|&b| b as char)
                .collect();
            if got.eq_ignore_ascii_case(name) {
                return Some(v);
            }
        }
        None
    }

    fn append_value(&mut self, key: u32, value: u32) -> Result<()> {
        let kp = self.cell_payload(key);
        let count = self.read_u32(kp + nk::VALUES_COUNT);
        let list = self.read_u32(kp + nk::VALUES_LIST);

        let mut entries = Vec::new();
        if count != 0 && list != u32::MAX {
            let lp = self.cell_payload(list);
            for i in 0..count as usize {
                entries.push(self.read_u32(lp + i * 4));
            }
        }
        entries.push(value);

        let new_list = self.alloc(entries.len() * 4)?;
        let lp = self.cell_payload(new_list);
        for (i, off) in entries.iter().enumerate() {
            self.write_u32(lp + i * 4, *off);
        }

        let kp = self.cell_payload(key);
        self.write_u32(kp + nk::VALUES_COUNT, entries.len() as u32);
        self.write_u32(kp + nk::VALUES_LIST, new_list);
        Ok(())
    }

    /// Walk a backslash-separated path from the root, creating nothing.
    pub fn open_path(&self, path: &str) -> Option<u32> {
        let mut cur = self.root_cell();
        for seg in path.split('\\').filter(|s| !s.is_empty()) {
            cur = self.find_subkey(cur, seg)?;
        }
        Some(cur)
    }

    /// Walk a backslash-separated path from the root, creating missing keys.
    pub fn ensure_path(&mut self, path: &str) -> Result<u32> {
        let mut cur = self.root_cell();
        for seg in path.split('\\').filter(|s| !s.is_empty()) {
            cur = self
                .ensure_subkey(cur, seg)
                .map_err(|e| anyhow!("creating '{seg}' in '{path}': {e}"))?;
        }
        Ok(cur)
    }
}

fn align_up(v: usize, to: usize) -> usize {
    v.div_ceil(to) * to
}

/// `lf` lists store the first four characters of the name, uppercased.
fn lf_hash(name: &str) -> u32 {
    let mut buf = [0u8; 4];
    for (i, b) in name.bytes().take(4).enumerate() {
        buf[i] = b.to_ascii_uppercase();
    }
    u32::from_le_bytes(buf)
}

/// `lh` lists store a rolling hash over the uppercased name.
fn lh_hash(name: &str) -> u32 {
    let mut hash: u32 = 0;
    for b in name.bytes() {
        hash = hash
            .wrapping_mul(37)
            .wrapping_add(u32::from(b.to_ascii_uppercase()));
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lf_hash_uses_the_first_four_uppercased_characters() {
        assert_eq!(lf_hash("abcd"), u32::from_le_bytes(*b"ABCD"));
        assert_eq!(lf_hash("ab"), u32::from_le_bytes([b'A', b'B', 0, 0]));
        // Longer names are truncated rather than folded.
        assert_eq!(lf_hash("abcdef"), lf_hash("abcd"));
    }

    #[test]
    fn lh_hash_is_case_insensitive() {
        assert_eq!(lh_hash("Elements"), lh_hash("ELEMENTS"));
        assert_ne!(lh_hash("Elements"), lh_hash("Element"));
    }

    #[test]
    fn align_up_rounds_to_the_next_multiple() {
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(9, 8), 16);
    }
}
