//! GTK mismatch dialog helpers for loaded files.

use std::path::Path;

use gtk4::prelude::IsA;
use libadwaita::prelude::*;

use c7_core::c7_file_interfacing::{C7Item, ItemError, device_family_from_file, validate_item};

/// Presents a warning dialog naming what was expected and what the file actually holds.
pub(crate) fn show_mismatch_dialog(
    parent: &impl IsA<gtk4::Widget>,
    expected: &str,
    actual: &str,
    filename: Option<&str>,
    extra_text: &str,
) {
    let mut body = match filename {
        Some(filename) => {
            let basename = Path::new(filename).file_name().and_then(|name| name.to_str()).unwrap_or(filename);
            format!("Expected {expected}, but {basename} contains {actual}.")
        }
        None => format!("Expected {expected}, but got {actual}."),
    };
    if !extra_text.is_empty() {
        body.push(' ');
        body.push_str(extra_text);
    }
    let dialog = libadwaita::AlertDialog::builder().heading("Mismatched File").body(&body).build();
    dialog.add_response("ok", "OK");
    dialog.present(Some(parent));
}

/// Checks a loaded file's device family against the connected device, showing a warning dialog on mismatch.
///
/// Returns `false` if the families diverge.
pub(crate) fn verify_device_match(
    parent: &impl IsA<gtk4::Widget>,
    filename: &str,
    items: &[C7Item],
    expected_device: Option<&str>,
    extra_text: &str,
) -> bool {
    let file_device = device_family_from_file(filename, items);
    if let (Some(file_device), Some(expected_device)) = (file_device.as_deref(), expected_device)
        && file_device != expected_device
    {
        show_mismatch_dialog(
            parent,
            &format!("a {expected_device} file"),
            &format!("{file_device} data"),
            Some(filename),
            extra_text,
        );
        return false;
    }
    true
}

/// Checks a loaded file's data type (kit, pattern, etc.) against what the screen expects, showing a warning dialog on mismatch.
///
/// Returns `false` on mismatch.
/// Returns `true` for any file extension this check doesn't cover.
pub(crate) fn does_file_type_match(parent: &impl IsA<gtk4::Widget>, filename: &str, items: &[C7Item], expected_type: &str) -> bool {
    let filename_lowercase = filename.to_lowercase();
    if !filename_lowercase.ends_with(".c7") && !filename_lowercase.ends_with(".syx") && !filename_lowercase.ends_with(".sysex") {
        return true;
    }
    for item in items {
        if item.section == "c7" {
            let file_type = item.get_type().to_lowercase();
            if !file_type.is_empty() && file_type != expected_type.to_lowercase() {
                show_mismatch_dialog(
                    parent,
                    &format!("a {expected_type} file"),
                    &format!("{file_type} data"),
                    Some(filename),
                    "",
                );
                return false;
            }
            return true;
        }
    }
    true
}

/// Validates one loaded item against the expected type and the connected device, showing a dialog for the first problem found.
///
/// Returns `false` if invalid.
pub(crate) fn verify_c7_item(
    parent: &impl IsA<gtk4::Widget>,
    item: &C7Item,
    expected_type: Option<&str>,
    device_prod_byte: u8,
    filename: Option<&str>,
) -> bool {
    match validate_item(item, expected_type, device_prod_byte) {
        None => true,
        Some(ItemError::NoData) => {
            let dialog = libadwaita::AlertDialog::builder()
                .heading("Mismatched File")
                .body("No SysEx data found in this file.")
                .build();
            dialog.add_response("ok", "OK");
            dialog.present(Some(parent));
            false
        }
        Some(ItemError::TypeMismatch { expected, found }) => {
            show_mismatch_dialog(parent, &format!("a {expected} file"), &format!("{found} data"), filename, "");
            false
        }
        Some(ItemError::DeviceMismatch {
            expected_device,
            file_device,
        }) => {
            show_mismatch_dialog(
                parent,
                &format!("a {expected_device} file"),
                &format!("{file_device} data"),
                filename,
                "",
            );
            false
        }
        Some(ItemError::CommandMismatch {
            expected_type, found_type, ..
        }) => {
            show_mismatch_dialog(
                parent,
                &format!("{expected_type} data"),
                &format!("{found_type} data"),
                filename,
                "",
            );
            false
        }
    }
}
