//! Convert a legacy `AEROSPRS` sparse image to the current `AEROSPAR` format.
//!
//! Nothing writes `AEROSPRS` any more, but images in that format still exist, and the rest of the
//! stack only opens `AEROSPAR`. This is the bridge between the two.
//!
//! Only allocated blocks are read, and blocks that turn out to be entirely zero are skipped rather
//! than written, so a sparse image converts to a sparse image instead of a fully populated one.

#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("aerosparse_convert requires a filesystem and is not supported on wasm32");
    std::process::exit(2);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use std::env;
    use std::process::exit;

    use aero_storage::{
        AeroSparseConfig, AeroSparseDisk, FileBackend, SparseDiskV1, StorageBackend as _,
        VirtualDisk as _,
    };

    /// Report why the conversion cannot continue and stop.
    ///
    /// A partially written output is worse than none — it looks like a converted image — so every
    /// failure here ends the process rather than pressing on.
    fn fail(context: &str, err: impl std::fmt::Display) -> ! {
        eprintln!("aerosparse_convert: {context}: {err}");
        exit(1)
    }

    let mut args = env::args().skip(1);
    let (Some(input_path), Some(output_path), None) = (args.next(), args.next(), args.next())
    else {
        eprintln!("Usage: aerosparse_convert <input.aerosprs> <output.aerospar>");
        exit(2)
    };

    let mut input = FileBackend::open_read_only(&input_path)
        .unwrap_or_else(|e| fail("failed to open input", e));

    let mut magic = [0u8; 8];
    if let Err(e) = input.read_at(0, &mut magic) {
        fail("failed to read input magic", e);
    }
    if magic != *b"AEROSPRS" {
        eprintln!("aerosparse_convert: {input_path} is not an AEROSPRS image");
        exit(1);
    }

    let mut legacy =
        SparseDiskV1::open(input).unwrap_or_else(|e| fail("failed to open legacy image", e));

    let header = *legacy.header();
    let disk_size_bytes = header
        .total_sectors
        .checked_mul(u64::from(header.sector_size))
        .unwrap_or_else(|| {
            eprintln!("aerosparse_convert: legacy disk size overflows 64 bits");
            exit(1)
        });

    // `AEROSPAR` requires a power-of-two block size that is a multiple of 512. A legacy image that
    // already satisfies that keeps its block size, so allocated regions line up one-to-one and the
    // output stays as sparse as the input.
    let legacy_block_size = header.block_size;
    let block_size_bytes =
        if legacy_block_size.is_power_of_two() && legacy_block_size.is_multiple_of(512) {
            legacy_block_size
        } else {
            1024 * 1024
        };

    let out_backend =
        FileBackend::create(&output_path, 0).unwrap_or_else(|e| fail("failed to create output", e));
    let mut out = AeroSparseDisk::create(
        out_backend,
        AeroSparseConfig {
            disk_size_bytes,
            block_size_bytes,
        },
    )
    .unwrap_or_else(|e| fail("failed to create output image", e));

    let allocated: Vec<u64> = legacy
        .allocated_blocks()
        .map(|(logical_block, _physical)| logical_block)
        .collect();

    let legacy_block_bytes = u64::from(legacy_block_size);
    let mut buf = vec![0u8; legacy_block_size as usize];
    let mut copied = 0u64;

    for logical_block in allocated {
        let offset = logical_block
            .checked_mul(legacy_block_bytes)
            .unwrap_or_else(|| fail("offset overflow", format!("at block {logical_block}")));
        // The final block of an image whose size is not a whole number of blocks is short.
        let Some(remaining) = disk_size_bytes.checked_sub(offset) else {
            continue;
        };
        let len = legacy_block_bytes.min(remaining) as usize;

        if let Err(e) = legacy.read_at(offset, &mut buf[..len]) {
            fail(&format!("failed to read legacy block {logical_block}"), e);
        }
        // An allocated block may still hold nothing. Writing it would make the output image larger
        // than the input for no gain.
        if buf[..len].iter().all(|b| *b == 0) {
            continue;
        }
        if let Err(e) = out.write_at(offset, &buf[..len]) {
            fail(&format!("failed to write block {logical_block}"), e);
        }

        copied += 1;
        if copied.is_multiple_of(1024) {
            eprintln!("aerosparse_convert: copied {copied} blocks...");
        }
    }

    if let Err(e) = out.flush() {
        fail("failed to flush output", e);
    }
    eprintln!("aerosparse_convert: done, copied {copied} allocated blocks to {output_path}");
}
