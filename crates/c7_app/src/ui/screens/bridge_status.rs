//! Show connected bridge plugins and a live log of their traffic.
//!
//! It owns the monitor server's (`bridge_interfacing.rs`) event source for the app's lifetime and just renders what arrives.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::{self, Button, Label, Orientation, PolicyType, ScrolledWindow, Separator, TextView, WrapMode};
use libadwaita::prelude::*;

use crate::ui::base_module::{BaseModule, listen, make_file_dialog, update_status};
use c7_core::bridge_interfacing::{BRIDGE_PORT, BridgeEvent, BridgeEventSource};

/// Loaded once at import time.
///
/// `BridgeScreen` is only built once, but the guard keeps the pattern identical to the other CSS-owning screens.
static CSS_ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Loads the bridge screen's CSS exactly once, guarding with a `Once` so repeated widget instantiation doesn't stack duplicate providers.
fn ensure_bridge_css() {
    CSS_ONCE.get_or_init(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            "scrolledwindow.log-scroller { \
                background-color: #141414; \
                border-radius: 8px; \
                border: 1px solid rgba(255, 255, 255, 0.08); \
             } \
             textview.log-terminal, \
             textview.log-terminal text { \
                background-color: transparent; \
                color: #eeeeee; \
             }",
        );
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(&display, &provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
}

/// Maximum number of log lines kept in the scrolling parameter log.
const LOG_MAX_LINES: usize = 100;

/// Feature screen for C7 Bridge monitoring.
pub(crate) struct BridgeScreen {
    pub root: gtk4::Box,
}

impl BridgeScreen {
    /// Initializes the bridge screen.
    pub(crate) fn new(go_to_menu: impl Fn() + 'static, source: BridgeEventSource) -> Self {
        ensure_bridge_css();
        let base = BaseModule::new();
        base.root.set_valign(gtk4::Align::Fill);
        let header = base.build_header("C7 Bridge", go_to_menu);

        // Exports the bundled, matching-OS plugin to a user-chosen location.
        let download_btn = Button::with_label("Download C7 Bridge");
        download_btn.connect_clicked(|btn| {
            let src = bundled_clap_path();
            if !src.exists() {
                alert_dialog(btn, "C7 Bridge not found", "The plugin wasn't bundled with this build.");
                return;
            }
            let dialog = make_file_dialog("Download CLAP", Some("C7-Bridge.clap"));
            let btn = btn.clone();
            dialog.save(None::<&gtk4::Window>, None::<&gio::Cancellable>, move |result| {
                let Ok(file) = result else { return }; // dialog cancelled
                let Some(dest) = file.path() else { return };
                match copy_clap(&src, &dest) {
                    Ok(()) => alert_dialog(&btn, "CLAP exported", &format!("Saved to {}", dest.display())),
                    Err(e) => alert_dialog(&btn, "Export failed", &e.to_string()),
                }
            });
        });
        header.end.append(&download_btn);

        // ── Connection status card ─────────────────────────────────────────────────────────────────────────

        let conn_section = gtk4::Box::new(Orientation::Vertical, 8);
        conn_section.set_margin_top(5);
        conn_section.set_margin_bottom(5);

        let conn_title = Label::new(Some("Connections"));
        conn_title.set_xalign(0.0);
        conn_title.add_css_class("title-3");
        conn_section.append(&conn_title);

        let status_card = gtk4::Box::new(Orientation::Vertical, 0);
        status_card.add_css_class("card");

        let status_inner = gtk4::Box::new(Orientation::Vertical, 8);
        status_inner.set_margin_top(15);
        status_inner.set_margin_bottom(15);
        status_inner.set_margin_start(15);
        status_inner.set_margin_end(15);

        // Connected-plugin summary, written through the shared `update_status()` setter.
        let status_label = Label::new(None);
        status_label.set_xalign(0.0);
        status_label.set_hexpand(true);
        update_status(None, &status_label, "Waiting for plugins...");
        status_inner.append(&status_label);

        let port_label = Label::new(Some(&format!("Monitoring on 127.0.0.1:{BRIDGE_PORT}")));
        port_label.set_xalign(0.0);
        port_label.add_css_class("dim-label");
        status_inner.append(&port_label);

        status_card.append(&status_inner);
        conn_section.append(&status_card);
        base.root.append(&conn_section);

        base.root.append(&Separator::new(Orientation::Horizontal));

        // ── Parameter log ──────────────────────────────────────────────────────────────────────────────────

        let log_section = gtk4::Box::new(Orientation::Vertical, 8);
        log_section.set_margin_top(5);
        log_section.set_margin_bottom(5);
        log_section.set_vexpand(true);

        let log_title = Label::new(Some("I/O Logging"));
        log_title.set_xalign(0.0);
        log_title.add_css_class("title-3");
        log_section.append(&log_title);

        let scroll = ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_hexpand(true);
        scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
        scroll.add_css_class("log-scroller");

        let text_view = TextView::new();
        text_view.set_editable(false);
        text_view.set_wrap_mode(WrapMode::None);
        text_view.add_css_class("monospace");
        text_view.add_css_class("log-terminal");
        text_view.set_top_margin(12);
        text_view.set_bottom_margin(12);
        text_view.set_left_margin(12);
        text_view.set_right_margin(12);

        scroll.set_child(Some(&text_view));
        log_section.append(&scroll);
        base.root.append(&log_section);

        append_log("--- waiting for plugin ping ---", &text_view);

        // ── Event Attachment ───────────────────────────────────────────────────────────────────────────────

        // Owns `BridgeEventSource` (and its Receiver) for the app lifetime.
        // One entry per connected plugin instance. The same device can appear more than once (two DAW instances).

        let connected: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

        listen(source.rx, move |event| {
            match event {
                BridgeEvent::Connected { device, addr } => {
                    connected.borrow_mut().push(device.clone());
                    update_status(None, &status_label, &connection_summary(&connected.borrow()));
                    append_log(&format!("--- {device} plugin connected from {addr} ---"), &text_view);
                }
                BridgeEvent::Disconnected { device } => {
                    let mut list = connected.borrow_mut();
                    if let Some(pos) = list.iter().position(|d| *d == device) {
                        list.remove(pos);
                    }
                    update_status(None, &status_label, &connection_summary(&list));
                    drop(list);
                    append_log(&format!("--- {device} plugin disconnected ---"), &text_view);
                }
                BridgeEvent::CcParam { device, channel, cc, val } => {
                    append_log(&format!("CC   [{device}] ch={channel:02}  cc={cc:03}  val={val:03}"), &text_view);
                }
                BridgeEvent::SysexParam { device, block, param, val } => {
                    append_log(
                        &format!("SX   [{device}] blk=0x{block:02X}  p={param:02}  val={val:03}"),
                        &text_view,
                    );
                }
                BridgeEvent::Note {
                    device,
                    channel,
                    note,
                    velocity,
                } => {
                    append_log(
                        &format!("NOTE [{device}] ch={channel:02}  note={note:03} vel={velocity:03}"),
                        &text_view,
                    );
                }
                BridgeEvent::Machine { device, track, machine } => {
                    append_log(&format!("MACH [{device}] trk={track:02}  id={machine:03}"), &text_view);
                }
                BridgeEvent::NoteDropped { device, channel } => {
                    append_log(
                        &format!("WARN [{device}] Dropped a signal because it was sent invalid channel {channel}"),
                        &text_view,
                    );
                }
                BridgeEvent::PitchBend { device, channel, val } => {
                    append_log(&format!("PB   [{device}] ch={channel:02}  val={val:05}"), &text_view);
                }
            }
            glib::ControlFlow::Continue
        });

        BridgeScreen { root: base.root }
    }

    /// Returns the root widget for embedding in the navigation stack.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ── Module-level helpers ───────────────────────────────────────────────────────────────────────────────────

/// Formats the connected-plugin list for the status card.
fn connection_summary(connected: &[String]) -> String {
    if connected.is_empty() {
        "Waiting for plugins...".to_string()
    } else {
        format!("Connected: {}", connected.join(", "))
    }
}

/// Returns the absolute path to the bundled CLAP plugin.
///
/// `main.rs` anchors cwd to the data directory, so the plugin sits at `bridge/C7-Bridge.clap`.
/// A single file on Windows/Linux, a `.clap` bundle directory on macOS.
fn bundled_clap_path() -> PathBuf {
    std::env::current_dir().unwrap_or_default().join("bridge").join("C7-Bridge.clap")
}

/// Copies the bundled CLAP to `dest`, handling both a single-file plugin (Windows/Linux) and a macOS bundle directory.
fn copy_clap(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        copy_dir_all(src, dest)
    } else {
        std::fs::copy(src, dest).map(|_| ())
    }
}

/// Recursively copies a directory tree for the macOS `.clap` bundle.
///
/// `std::fs::copy` preserves permission bits, so the plugin binary keeps its executable flag.
fn copy_dir_all(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());

        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Presents a one-button alert dialog.
fn alert_dialog(parent: &impl IsA<gtk4::Widget>, heading: &str, body: &str) {
    let dialog = libadwaita::AlertDialog::builder().heading(heading).body(body).build();
    dialog.add_response("ok", "OK");
    dialog.present(Some(parent));
}

/// Appends a monospace log line to the `TextView`, deleting the oldest entry once `LOG_MAX_LINES` is reached.
fn append_log(text: &str, text_view: &TextView) {
    let buffer = text_view.buffer();
    let line = if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    };

    buffer.insert(&mut buffer.end_iter(), &line);

    // Limit line count.
    let line_count = buffer.line_count() as usize;
    if line_count > LOG_MAX_LINES
        && let Some(mut end) = buffer.iter_at_line((line_count - LOG_MAX_LINES) as i32)
    {
        let mut start = buffer.start_iter();
        buffer.delete(&mut start, &mut end);
    }

    // Auto-scroll to bottom.
    let mark = buffer.create_mark(None, &buffer.end_iter(), false);
    text_view.scroll_to_mark(&mark, 0.0, true, 0.0, 1.0);
    buffer.delete_mark(&mark);
}
