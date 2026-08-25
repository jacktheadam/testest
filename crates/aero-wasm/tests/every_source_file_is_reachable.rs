//! Every file under `src/` must be reachable from the crate root.
//!
//! A `.rs` file that no `mod` declaration names is not part of the crate. Rust does not warn about
//! this — the file is simply never read — so an orphaned module compiles, tests green, and the
//! only symptom is a missing `#[wasm_bindgen]` export that the browser discovers at runtime and
//! usually handles by skipping the feature.
//!
//! That is exactly how `virtio_snd_playback_demo` was lost: added with its Playwright coverage,
//! never declared, and the resulting "wasm export is unavailable" warning left the demo silently
//! disabled while its test kept passing against a stale build artifact.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("read directory entry").path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_source_file_is_declared_as_a_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    let mut files = Vec::new();
    collect_rust_files(&src, &mut files);
    assert!(
        !files.is_empty(),
        "no sources found under {}",
        src.display()
    );

    // Every `mod name;` / `mod name {` written anywhere in the crate.
    let mut declared = BTreeSet::new();
    for file in &files {
        let text =
            fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for line in text.lines() {
            let line = line.trim();
            let rest = line
                .strip_prefix("pub mod ")
                .or_else(|| line.strip_prefix("pub(crate) mod "))
                .or_else(|| line.strip_prefix("mod "));
            let Some(rest) = rest else { continue };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                declared.insert(name);
            }
        }
    }

    let orphans: Vec<String> = files
        .iter()
        .filter(|file| {
            // `lib.rs` is the root and `mod.rs` is named by its directory, not its stem.
            let stem = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if stem == "lib" || stem == "main" {
                return false;
            }
            let name = if stem == "mod" {
                file.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string()
            } else {
                stem.to_string()
            };
            !name.is_empty() && !declared.contains(&name)
        })
        .map(|file| {
            file.strip_prefix(&src)
                .unwrap_or(file)
                .display()
                .to_string()
        })
        .collect();

    assert!(
        orphans.is_empty(),
        "these files are under src/ but no `mod` declaration names them, so they are not compiled \
         into the crate:\n  {}\n\nAdd a `mod` declaration (with the same `#[cfg]` as its siblings) \
         or move the file out of src/.",
        orphans.join("\n  "),
    );
}
