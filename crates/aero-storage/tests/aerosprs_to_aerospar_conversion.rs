//! The legacy `AEROSPRS` format exists so old images stay readable, and the only thing that
//! usefully consumes it is the conversion to `AEROSPAR`. This exercises that path end to end:
//! write an image in the old format, convert it the way the `aerosparse_convert` binary does, and
//! read the result back through the current format.
//!
//! Reading the old format correctly is not enough on its own — the data has to survive the trip.

use aero_storage::{AeroSparseConfig, AeroSparseDisk, MemBackend, SparseDiskV1, VirtualDisk as _};

const SECTOR_SIZE: u32 = 512;
const TOTAL_SECTORS: u64 = 512;
const BLOCK_SIZE: u32 = 4096;

/// Distinct, offset-dependent bytes, so a copy that lands in the wrong place is visible rather
/// than masked by a repeating pattern.
fn pattern(offset: u64, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| ((offset as usize + i) * 31 % 251) as u8)
        .collect()
}

#[test]
fn legacy_image_converts_to_the_current_format_with_its_contents_intact() {
    let disk_size_bytes = TOTAL_SECTORS * u64::from(SECTOR_SIZE);

    // Writes chosen to cover the cases the conversion has to get right: the first block, a block
    // in the middle leaving a sparse gap either side, one that straddles a block boundary, and one
    // that ends exactly at the end of the disk.
    let writes: Vec<(u64, Vec<u8>)> = vec![
        (0, pattern(0, 600)),
        (u64::from(BLOCK_SIZE) * 3, pattern(3000, 1024)),
        (u64::from(BLOCK_SIZE) * 10 - 100, pattern(7777, 400)),
        (disk_size_bytes - 512, pattern(4242, 512)),
    ];

    let mut legacy = SparseDiskV1::create(
        MemBackend::default(),
        SECTOR_SIZE,
        TOTAL_SECTORS,
        BLOCK_SIZE,
    )
    .expect("create legacy image");
    for (offset, data) in &writes {
        legacy.write_at(*offset, data).expect("write legacy");
    }
    legacy.flush().expect("flush legacy");

    // Reopen from the same bytes, the way the converter sees an image it did not create.
    let mut legacy = SparseDiskV1::open(legacy.into_storage()).expect("reopen legacy image");

    let mut converted = AeroSparseDisk::create(
        MemBackend::default(),
        AeroSparseConfig {
            disk_size_bytes,
            block_size_bytes: BLOCK_SIZE,
        },
    )
    .expect("create converted image");

    let allocated: Vec<u64> = legacy
        .allocated_blocks()
        .map(|(logical_block, _physical)| logical_block)
        .collect();
    assert!(
        !allocated.is_empty(),
        "the legacy image should have allocated blocks to convert"
    );

    let mut buf = vec![0u8; BLOCK_SIZE as usize];
    for logical_block in allocated {
        let offset = logical_block * u64::from(BLOCK_SIZE);
        let len = u64::from(BLOCK_SIZE).min(disk_size_bytes - offset) as usize;
        legacy
            .read_at(offset, &mut buf[..len])
            .expect("read legacy");
        if buf[..len].iter().all(|b| *b == 0) {
            continue;
        }
        converted
            .write_at(offset, &buf[..len])
            .expect("write converted");
    }
    converted.flush().expect("flush converted");

    for (offset, data) in &writes {
        let mut got = vec![0u8; data.len()];
        converted
            .read_at(*offset, &mut got)
            .expect("read converted");
        assert_eq!(
            &got, data,
            "converted image differs from the legacy image at offset {offset}"
        );
    }

    // Everything not written stays zero — the conversion must not have smeared a block across the
    // sparse gaps it was supposed to leave alone.
    let mut gap = vec![0u8; 2048];
    converted
        .read_at(u64::from(BLOCK_SIZE) * 6, &mut gap)
        .expect("read gap");
    assert!(
        gap.iter().all(|b| *b == 0),
        "a region never written to the legacy image came back non-zero"
    );
}

#[test]
fn a_legacy_image_reads_back_what_was_written_across_block_boundaries() {
    let mut disk = SparseDiskV1::create(
        MemBackend::default(),
        SECTOR_SIZE,
        TOTAL_SECTORS,
        BLOCK_SIZE,
    )
    .expect("create legacy image");

    // Straddle three blocks in one write, then read it back in one read.
    let offset = u64::from(BLOCK_SIZE) - 7;
    let data = pattern(11, (BLOCK_SIZE as usize) * 2 + 64);
    disk.write_at(offset, &data).expect("write");

    let mut got = vec![0u8; data.len()];
    disk.read_at(offset, &mut got).expect("read");
    assert_eq!(got, data);
}
