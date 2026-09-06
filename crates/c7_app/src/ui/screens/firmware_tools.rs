//! Upload and receive device firmware, and ping for its identity.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use gio::prelude::*;
use gtk4::prelude::*;
use gtk4::{self, Align, Button, Label, Orientation, Separator};

use crate::ui::base_module::{BaseModule, ProgressFn, StatusFn, eta_string, make_file_dialog};
use crate::ui::file_checks::verify_device_match;
use c7_core::c7_file_interfacing::read_c7_or_sysex_file;
use c7_core::device_config::DeviceConfig;
use c7_core::firmware::{receive_firmware, request_os_version, upload_firmware};
use c7_core::midi::{get_midi_delay_ms, is_valid_port};
use c7_core::utils::{JsonPath, as_array_or_die, as_bool_or, as_string_or_die, now_compact_timestamp};

// ── Internal state ─────────────────────────────────────────────────────────────────────────────────────────

struct FirmwareToolsState {
    is_active: Arc<AtomicBool>,
    should_cancel: Arc<AtomicBool>,
    get_midi_rc: Rc<dyn Fn() -> Option<String>>,
    device_short: String,

    btn_upload: Option<Button>,
    btn_receive: Option<Button>,
    btn_ping: Option<Button>,
    version_label: Option<Label>,

    update_status: Option<StatusFn>,
    update_progress: Option<ProgressFn>,
}

/// Feature screen for firmware transfers.
pub(crate) struct FirmwareToolsScreen {
    pub root: gtk4::Box,
}

impl FirmwareToolsScreen {
    /// Initializes the firmware tools screen.
    pub(crate) fn new(
        go_to_menu: impl Fn() + 'static,
        get_selected_midi: impl Fn() -> Option<String> + 'static,
        active_config: &str,
    ) -> Self {
        let mut base = BaseModule::new();
        let dc = DeviceConfig::find_by_path_or_die(active_config);

        let state = Rc::new(RefCell::new(FirmwareToolsState {
            is_active: Arc::new(AtomicBool::new(false)),
            should_cancel: Arc::new(AtomicBool::new(false)),
            get_midi_rc: Rc::new(get_selected_midi),
            device_short: dc.device_short.clone(),
            btn_upload: None,
            btn_receive: None,
            btn_ping: None,
            version_label: None,
            update_status: None,
            update_progress: None,
        }));

        base.build_header("Firmware Tools", go_to_menu);

        if dc.has_gate("sysex_api.commands.identity_inquiry") {
            let detect_row = gtk4::Box::new(Orientation::Horizontal, 12);
            detect_row.set_margin_top(5);
            detect_row.set_margin_bottom(5);
            let btn = Button::with_label("Ping Machine Identity");
            let label = Label::new(Some("OS Version: Unknown"));
            label.add_css_class("monospace");
            detect_row.append(&btn);
            detect_row.append(&label);
            base.root.append(&detect_row);
            base.root.append(&Separator::new(Orientation::Horizontal));

            state.borrow_mut().btn_ping = Some(btn.clone());
            state.borrow_mut().version_label = Some(label);

            let state_c = Rc::clone(&state);
            btn.connect_clicked(move |_| state_c.borrow_mut().on_ping());
        }

        let all_links: Vec<serde_json::Value> = dc
            .json_get("firmware.links")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        let official: Vec<serde_json::Value> = all_links
            .iter()
            .filter(|link| as_bool_or(link.json_get("official"), false))
            .cloned()
            .collect();
        let unofficial: Vec<serde_json::Value> = all_links
            .iter()
            .filter(|link| !as_bool_or(link.json_get("official"), false))
            .cloned()
            .collect();

        let mut link_labels: Vec<Label> = Vec::new();
        if !official.is_empty() {
            link_labels.push(build_link_label("Official releases:", &official));
        }
        if !unofficial.is_empty() {
            link_labels.push(build_link_label("Unofficial releases:", &unofficial));
        }
        let link_refs: Vec<&Label> = link_labels.iter().collect();

        // A device that can't be flashed over MIDI leaves upload_instructions out of its JSON.
        let mut btn_upload = None;
        if dc.has_gate("firmware.upload_instructions") {
            let upload_steps = numbered_steps(&dc, "firmware.upload_instructions");
            let upload_refs: Vec<&str> = upload_steps.iter().map(String::as_str).collect();
            btn_upload = Some(build_section(
                &mut base,
                "Send MIDI Upgrade",
                &upload_refs,
                "Upload MIDI Upgrade File",
                &["suggested-action"],
                &link_refs,
            ));
        }

        // A device that can't send its own OS back leaves receive_instructions out of its JSON.
        let mut btn_receive = None;
        if dc.has_gate("firmware.receive_instructions") {
            // Nothing to divide from when the upload section above was skipped.
            if btn_upload.is_some() {
                base.root.append(&Separator::new(Orientation::Horizontal));
            }
            let receive_steps = numbered_steps(&dc, "firmware.receive_instructions");
            let receive_refs: Vec<&str> = receive_steps.iter().map(String::as_str).collect();
            btn_receive = Some(build_section(
                &mut base,
                "Receive MIDI Upgrade",
                &receive_refs,
                "Start Firmware Listener",
                &[],
                &[],
            ));
        }

        base.build_status_area(true);

        {
            let mut state_mut = state.borrow_mut();
            state_mut.btn_upload.clone_from(&btn_upload);
            state_mut.btn_receive.clone_from(&btn_receive);
            state_mut.update_status = Some(base.status_updater());
            state_mut.update_progress = Some(base.progress_updater());
        }

        if let Some(btn_upload) = &btn_upload {
            let state_upload = Rc::clone(&state);
            btn_upload.connect_clicked(move |_| state_upload.borrow_mut().on_upload());
        }

        if let Some(btn_receive) = &btn_receive {
            let state_receive = Rc::clone(&state);
            btn_receive.connect_clicked(move |_| state_receive.borrow_mut().on_receive());
        }

        FirmwareToolsScreen { root: base.root }
    }

    /// Returns the root widget for embedding in the navigation stack.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

impl FirmwareToolsState {
    /// Handles the "Ping" button: queries the connected device's OS version and shows it in the label.
    fn on_ping(&mut self) {
        let port_name = match (self.get_midi_rc)() {
            Some(port_name) if is_valid_port(&port_name) => port_name,
            _ => {
                if let Some(label) = &self.version_label {
                    label.set_label("OS Version: No port selected");
                }
                return;
            }
        };

        if let Some(label) = &self.version_label {
            label.set_label("OS Version: Pinging...");
        }

        let version_wk = glib::SendWeakRef::from(self.version_label.as_ref().unwrap().downgrade());

        glib::spawn_future_local(async move {
            let result = request_os_version(&port_name).await;
            if let Some(label) = version_wk.upgrade() {
                label.set_label(&result.unwrap_or_else(|e| format!("OS Version: Error ({e})")));
            }
        });
    }

    /// Handles the "Upload" button: prompts for a firmware file, then streams it to the device with progress/cancel support.
    fn on_upload(&mut self) {
        if self.is_active.load(Ordering::Relaxed) {
            self.should_cancel.store(true, Ordering::Relaxed);
            return;
        }

        let dialog = make_file_dialog("Select Firmware SysEx File", None);
        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("Firmware files (*.syx)"));
        filter.add_pattern("*.syx");
        let filters = gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));

        let is_active = Arc::clone(&self.is_active);
        let should_cancel = Arc::clone(&self.should_cancel);
        let get_midi_rc = Rc::clone(&self.get_midi_rc);
        let device_short = self.device_short.clone();
        let upload_wk = self.btn_upload.as_ref().map(|btn| glib::SendWeakRef::from(btn.downgrade()));
        let receive_wk = self.btn_receive.as_ref().map(|btn| glib::SendWeakRef::from(btn.downgrade()));
        let ping_wk = self.btn_ping.as_ref().map(|btn| glib::SendWeakRef::from(btn.downgrade()));

        let update_status = self.update_status.clone().unwrap();
        let update_progress = self.update_progress.clone().unwrap();

        let progress_update_fn = move |text: String, frac: f64, start: Instant| {
            update_progress(&text, frac, &eta_string(start, frac));
        };

        dialog.open(None::<&gtk4::Window>, None::<&gio::Cancellable>, move |result| {
            let Ok(file) = result else { return };
            let Some(path) = file.path() else { return };
            let port_name = match get_midi_rc() {
                Some(port_name) if is_valid_port(&port_name) => port_name,
                _ => {
                    update_status("Error: No MIDI device selected.");
                    return;
                }
            };

            let items = read_c7_or_sysex_file(&path);
            if let Some(parent_window) = gtk4::Window::list_toplevels().into_iter().next()
                && !verify_device_match(&parent_window, &path.to_string_lossy(), &items, Some(&device_short), "")
            {
                return;
            }

            is_active.store(true, Ordering::Relaxed);
            should_cancel.store(false, Ordering::Relaxed);
            set_ui_state_upload_active(upload_wk.as_ref(), receive_wk.as_ref(), ping_wk.as_ref());

            let is_active_c = Arc::clone(&is_active);
            let cancel_op_c = Arc::clone(&should_cancel);
            let delay_ms_val = f64::from(get_midi_delay_ms());
            let start = Instant::now();
            let progress_update_fn_for_stream = progress_update_fn.clone();

            let (tx, rx) = async_channel::unbounded::<(String, f64, Instant)>();
            glib::spawn_future_local(async move {
                while let Ok((text, fraction, start)) = rx.recv().await {
                    progress_update_fn_for_stream(text, fraction, start);
                }
            });

            let progress_update_fn_for_finish = progress_update_fn.clone();
            glib::spawn_future_local(async move {
                let result = upload_firmware(port_name, path, delay_ms_val, cancel_op_c, tx, start).await;

                let text = match result {
                    Ok(()) if should_cancel.load(Ordering::Relaxed) => "Upload Cancelled.".to_string(),
                    Ok(()) => "Upload Complete! Verify on hardware screen.".to_string(),
                    Err(e) => format!("Error during upload: {e}"),
                };
                progress_update_fn_for_finish(text, 1.0, start);
                is_active_c.store(false, Ordering::Relaxed);
                set_ui_state_idle(upload_wk.as_ref(), receive_wk.as_ref(), ping_wk.as_ref());
            });
        });
    }

    /// Handles the "Receive" button: listens for an incoming firmware SysEx stream, then prompts to save it.
    fn on_receive(&mut self) {
        if self.is_active.load(Ordering::Relaxed) {
            self.should_cancel.store(true, Ordering::Relaxed);
            return;
        }

        let port_name = match (self.get_midi_rc)() {
            Some(port_name) if is_valid_port(&port_name) => port_name,
            _ => {
                if let Some(update_status) = &self.update_status {
                    update_status("Error: No MIDI device selected.");
                }
                return;
            }
        };

        self.is_active.store(true, Ordering::Relaxed);
        self.should_cancel.store(false, Ordering::Relaxed);
        if let Some(update_status) = &self.update_status {
            update_status("Status: Preparing OS dump...");
        }

        let upload_wk = self.btn_upload.as_ref().map(|btn| glib::SendWeakRef::from(btn.downgrade()));
        let receive_wk = self.btn_receive.as_ref().map(|btn| glib::SendWeakRef::from(btn.downgrade()));
        let ping_wk = self.btn_ping.as_ref().map(|btn| glib::SendWeakRef::from(btn.downgrade()));
        set_ui_state_receive_active(upload_wk.as_ref(), receive_wk.as_ref(), ping_wk.as_ref());

        let is_active = Arc::clone(&self.is_active);
        let should_cancel = Arc::clone(&self.should_cancel);
        let update_status = self.update_status.clone().unwrap();
        let device_short = self.device_short.clone();

        let (tx, rx) = async_channel::unbounded::<String>();
        let update_status_c = update_status.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                update_status_c(&msg);
            }
        });

        glib::spawn_future_local(async move {
            let result = receive_firmware(port_name, should_cancel.clone(), tx).await;
            is_active.store(false, Ordering::Relaxed);

            match result {
                Ok(buffer) if !buffer.is_empty() && !should_cancel.load(Ordering::Relaxed) => {
                    prompt_save_firmware(buffer, &device_short, upload_wk, receive_wk, ping_wk);
                }
                _ if should_cancel.load(Ordering::Relaxed) => {
                    update_status("Status: Receive Cancelled.");
                    set_ui_state_idle(upload_wk.as_ref(), receive_wk.as_ref(), ping_wk.as_ref());
                }
                _ => {
                    update_status("Status: Receive timed out or no data.");
                    set_ui_state_idle(upload_wk.as_ref(), receive_wk.as_ref(), ping_wk.as_ref());
                }
            }
        });
    }
}

// ── Module-level helpers ───────────────────────────────────────────────────────────────────────────────────

/// Numbers the steps a device declares at `path`.
///
/// Numbering is applied here rather than stored in the JSON, so inserting a step can't throw the sequence off.
fn numbered_steps(dc: &DeviceConfig, path: &str) -> Vec<String> {
    as_array_or_die(dc.json_get(path))
        .iter()
        .enumerate()
        .map(|(idx, step)| format!("{}. {}", idx + 1, as_string_or_die(Some(step))))
        .collect()
}

/// Builds one titled section (title, instructions card, extra widgets, action button) and appends it to `base`.
fn build_section(base: &mut BaseModule, title: &str, steps: &[&str], btn_label: &str, btn_cls: &[&str], extras: &[&Label]) -> Button {
    let section = gtk4::Box::new(Orientation::Vertical, 8);
    section.set_margin_top(5);
    section.set_margin_bottom(5);

    let title_label = Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.add_css_class("title-3");
    section.append(&title_label);

    if !steps.is_empty() {
        section.append(&BaseModule::instructions_card(steps, None));
    }

    for extra_widget in extras {
        section.append(*extra_widget);
    }

    let btn = Button::with_label(btn_label);
    btn.set_halign(Align::Start);
    for cls in btn_cls {
        btn.add_css_class(cls);
    }
    section.append(&btn);

    base.root.append(&section);
    btn
}

/// Builds a label listing `links` as clickable markup, grouped under `prefix`.
fn build_link_label(prefix: &str, links: &[serde_json::Value]) -> Label {
    let label = Label::new(None);
    label.set_halign(Align::Start);

    let parts: Vec<String> = links
        .iter()
        .map(|link| {
            let url = as_string_or_die(link.json_get("url"));
            let label = as_string_or_die(link.json_get("label"));
            format!("<a href=\"{url}\">{label}</a>")
        })
        .collect();
    label.set_markup(&format!("{prefix}  {}", parts.join(", ")));

    label
}

/// Reflects an in-progress upload: disables Ping/Receive and turns Upload into a Cancel button.
fn set_ui_state_upload_active(
    upload_wk: Option<&glib::SendWeakRef<Button>>,
    receive_wk: Option<&glib::SendWeakRef<Button>>,
    ping_wk: Option<&glib::SendWeakRef<Button>>,
) {
    if let Some(btn) = ping_wk.and_then(|w| w.upgrade()) {
        btn.set_sensitive(false);
    }
    if let Some(btn) = upload_wk.and_then(|w| w.upgrade()) {
        btn.set_label("Cancel Operation");
        btn.add_css_class("destructive-action");
    }
    if let Some(btn) = receive_wk.and_then(|w| w.upgrade()) {
        btn.set_sensitive(false);
    }
}

/// Reflects an in-progress receive: disables Ping/Upload and turns Receive into a Cancel button.
fn set_ui_state_receive_active(
    upload_wk: Option<&glib::SendWeakRef<Button>>,
    receive_wk: Option<&glib::SendWeakRef<Button>>,
    ping_wk: Option<&glib::SendWeakRef<Button>>,
) {
    if let Some(btn) = ping_wk.and_then(|w| w.upgrade()) {
        btn.set_sensitive(false);
    }
    if let Some(btn) = upload_wk.and_then(|w| w.upgrade()) {
        btn.set_sensitive(false);
    }
    if let Some(btn) = receive_wk.and_then(|w| w.upgrade()) {
        btn.set_label("Cancel Operation");
        btn.add_css_class("destructive-action");
    }
}

/// Restores every button on the screen to its default label, sensitivity, and styling.
fn set_ui_state_idle(
    upload_wk: Option<&glib::SendWeakRef<Button>>,
    receive_wk: Option<&glib::SendWeakRef<Button>>,
    ping_wk: Option<&glib::SendWeakRef<Button>>,
) {
    if let Some(btn) = ping_wk.and_then(|w| w.upgrade()) {
        btn.set_sensitive(true);
    }
    if let Some(btn) = upload_wk.and_then(|w| w.upgrade()) {
        btn.set_label("Upload MIDI Upgrade File");
        btn.remove_css_class("destructive-action");
        btn.set_sensitive(true);
    }
    if let Some(btn) = receive_wk.and_then(|w| w.upgrade()) {
        btn.set_label("Start Firmware Listener");
        btn.remove_css_class("destructive-action");
        btn.set_sensitive(true);
    }
}

/// Opens a save dialog pre-filled with a device/date-stamped filename, then writes `data` to the chosen path.
fn prompt_save_firmware(
    data: Vec<u8>,
    device_short: &str,
    upload_wk: Option<glib::SendWeakRef<Button>>,
    receive_wk: Option<glib::SendWeakRef<Button>>,
    ping_wk: Option<glib::SendWeakRef<Button>>,
) {
    let name = format!("{device_short}_Firmware_{}.syx", now_compact_timestamp());
    let dialog = make_file_dialog("Save Firmware SysEx", Some(&name));

    dialog.save(None::<&gtk4::Window>, None::<&gio::Cancellable>, move |result| {
        if let Ok(file) = result
            && let Some(path) = file.path()
        {
            let _ = std::fs::write(&path, &data);
        }
        set_ui_state_idle(upload_wk.as_ref(), receive_wk.as_ref(), ping_wk.as_ref());
    });
}
