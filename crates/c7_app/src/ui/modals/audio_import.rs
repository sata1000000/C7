//! Dialog offering to trim silence off an audio file before it is imported.

use std::path::Path;

use gio::Cancellable;
use gtk4::{self, Align, Box as GtkBox, CheckButton, Orientation};
use libadwaita::prelude::*;

// ── Trim prompt ────────────────────────────────────────────────────────────────────────────────────────────

/// Presents the silence-trimming prompt for `path`, offering a checkbox for whichever ends hold silence.
///
/// `on_result` receives `Some((trim_front, trim_back))` once Load is pressed, or `None` if the user cancels.
pub(crate) fn show_trim_dialog(
    parent: &impl IsA<gtk4::Widget>,
    path: &str,
    front_seconds: f64,
    back_seconds: f64,
    on_result: impl FnOnce(Option<(bool, bool)>) + 'static,
) {
    let basename = Path::new(path).file_name().unwrap_or_default().to_string_lossy();
    let dialog = libadwaita::AlertDialog::builder()
        .heading("Load File")
        .body(format!("{basename} holds silence that can be cut before import."))
        .build();

    let content = GtkBox::new(Orientation::Vertical, 6);
    content.set_margin_top(4);
    content.set_margin_bottom(4);
    content.set_margin_start(16);
    content.set_margin_end(16);

    // Only the ends that actually hold silence get a checkbox, so a file quiet at one end offers one choice.
    let trim_front_chk = (front_seconds >= 0.01).then(|| {
        let check = CheckButton::with_label(&format!("Cut {front_seconds:.2}s of leading silence"));
        check.set_active(true);
        check.set_halign(Align::Start);
        content.append(&check);
        check
    });

    let trim_back_chk = (back_seconds >= 0.01).then(|| {
        let check = CheckButton::with_label(&format!("Cut {back_seconds:.2}s of trailing silence"));
        check.set_active(true);
        check.set_halign(Align::Start);
        content.append(&check);
        check
    });

    dialog.set_extra_child(Some(&content));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", "Load");
    dialog.set_response_appearance("ok", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");

    dialog.choose(Some(parent), Cancellable::NONE, move |response| {
        if response != "ok" {
            on_result(None);
            return;
        }
        let trim_front = trim_front_chk.as_ref().is_some_and(CheckButtonExt::is_active);
        let trim_back = trim_back_chk.as_ref().is_some_and(CheckButtonExt::is_active);
        on_result(Some((trim_front, trim_back)));
    });
}
