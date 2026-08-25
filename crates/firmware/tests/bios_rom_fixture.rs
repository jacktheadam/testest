use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` for integration tests in this package is `<repo>/crates/firmware`.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn bios_rom_fixture_matches_generator() {
    let fixture_path = repo_root().join("assets").join("bios.bin");
    let fixture = std::fs::read(&fixture_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", fixture_path.display()));

    let generated = firmware::bios::build_bios_rom();

    if fixture != generated {
        let min_len = fixture.len().min(generated.len());
        let first_diff = (0..min_len).find(|&i| fixture[i] != generated[i]);

        let details = match first_diff {
            Some(i) => format!(
                "first differing byte at 0x{i:04X}: fixture=0x{:02X}, generated=0x{:02X}",
                fixture[i], generated[i]
            ),
            None => format!(
                "length mismatch: fixture={} bytes, generated={} bytes",
                fixture.len(),
                generated.len()
            ),
        };

        panic!(
            "`assets/bios.bin` is out of date or has been modified ({details}).\n\
Regenerate with: cargo xtask bios-rom\n\
(or: cargo xtask fixtures, or: cargo run -p firmware --bin gen_bios_rom --locked)"
        );
    }
}

#[test]
fn bios_rom_publishes_a_stable_conventional_build_date() {
    let rom = firmware::bios::build_bios_rom();

    assert_eq!(&rom[0xFFF5..0xFFFD], b"06/23/99");
    assert_eq!(
        latest_windows_bios_date(&rom).as_deref(),
        Some("1999/06/23"),
        "the conventional build date must remain the newest date accepted by the Windows BIOS-ROM scanner"
    );
}

fn latest_windows_bios_date(bytes: &[u8]) -> Option<String> {
    let mut latest = None;

    for candidate in bytes.windows(8) {
        if candidate[2] != b'/'
            || candidate[5] != b'/'
            || !candidate[1].is_ascii_digit()
            || !candidate[3].is_ascii_digit()
            || !candidate[4].is_ascii_digit()
            || !candidate[6].is_ascii_digit()
            || !candidate[7].is_ascii_digit()
        {
            continue;
        }

        let month_tens = if candidate[0].is_ascii_digit() {
            candidate[0] - b'0'
        } else {
            0
        };
        let month = month_tens * 10 + candidate[1] - b'0';
        let day = (candidate[3] - b'0') * 10 + candidate[4] - b'0';
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            continue;
        }

        let short_year = (candidate[6] - b'0') * 10 + candidate[7] - b'0';
        let year = if short_year < 80 {
            2000 + u16::from(short_year)
        } else {
            1900 + u16::from(short_year)
        };
        let normalized = format!("{year:04}/{month:02}/{day:02}");
        if latest.as_ref().is_none_or(|old| normalized > *old) {
            latest = Some(normalized);
        }
    }

    latest
}
