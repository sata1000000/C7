//! Generates one plugin type per enabled device at compile time.
//!
//! One `C7-Bridge.clap` carries every enabled device as its own plugin, each with its own CLAP ID and name.
//! A DAW expects one ID to mean one plugin implementation.

use std::fmt::Write;
use std::path::PathBuf;

/// Turns a device directory name into a Rust type name.
///
/// Strips non-alphanumerics, then uppercases the letter after each gap ("machinedrum" → "Machinedrum", "TM-1" → "TM1").
fn type_name(dir: &str) -> String {
    let mut out = String::new();
    let mut should_upper_next = true;
    for char in dir.chars() {
        if !char.is_ascii_alphanumeric() {
            should_upper_next = true;
            continue;
        }
        if should_upper_next {
            out.push(char.to_ascii_uppercase());
            should_upper_next = false;
        } else {
            out.push(char);
        }
    }

    out
}

/// Discovers every enabled device and writes the generated marker types + export call to `OUT_DIR/devices.rs`.
fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    let devices_dir = manifest.join("..").join("c7_core").join("devices");
    // Watch the directory itself so adding or removing a device JSON retriggers generation.
    println!("cargo:rerun-if-changed={}", devices_dir.display());

    // Sorted for a deterministic plugin order in the DAW's list.
    let mut dirs: Vec<_> = std::fs::read_dir(&devices_dir)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    dirs.sort_by_key(std::fs::DirEntry::file_name);

    let mut code = String::new();
    let mut exports: Vec<String> = Vec::new();

    for entry in dirs {
        let dir_name = entry.file_name().into_string().unwrap();
        let json_path = entry.path().join(format!("{dir_name}.json"));
        // `.json.disabled` and documentation-only directories aren't enabled devices.
        if !json_path.exists() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", json_path.display());

        let raw = std::fs::read_to_string(&json_path).unwrap();
        let data: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let device_short = data["device_short"].as_str().unwrap();

        let type_name = type_name(&dir_name);
        let absolute_json_path = json_path.canonicalize().unwrap().display().to_string().replace('\\', "/");
        writeln!(
            code,
            "declare_device!({type_name}, \"{dir_name}\", \"{device_short}\", \"{absolute_json_path}\");"
        )
        .unwrap();
        exports.push(format!("C7Bridge<{type_name}>"));
    }

    assert!(!exports.is_empty(), "no enabled devices found in c7_core/devices/");
    write!(code, "\nnih_export_clap!({});\n", exports.join(", ")).unwrap();

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("devices.rs");
    std::fs::write(out, code).unwrap();
}
