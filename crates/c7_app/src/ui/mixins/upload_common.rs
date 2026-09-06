//! Shared UI behaviors for the `upload_sample.rs` / `upload_digipro.rs` screens.

use gtk4::prelude::*;
use gtk4::{Entry, gdk, gio};

/// Wires a file-drop target on `root`, calling `on_load` with the dropped file's path.
pub(crate) fn wire_file_drop_target(root: &gtk4::Box, on_load: impl Fn(String) + 'static) {
    let drop_target = gtk4::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
    drop_target.connect_drop(move |_, val, _, _| {
        if let Ok(file) = val.get::<gio::File>()
            && let Some(path) = file.path()
        {
            on_load(path.to_string_lossy().to_string());
            return true;
        }
        false
    });
    root.add_controller(drop_target);
}

/// Returns the trimmed Entry text.
/// Returns `default` if it's empty.
pub(crate) fn name_or_default(entry: &Entry, default: &str) -> String {
    let trimmed = entry.text().trim().to_string();
    if trimmed.is_empty() { default.to_string() } else { trimmed }
}
