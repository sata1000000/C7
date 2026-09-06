//! Receive SDS samples from Machinedrum slots.

/*
Structurally identical to `receive_digipro.rs`, but with different section content objectives.
If you change ANY structural code in here, please change `receive_digipro.rs` to use the new standard too.
*/

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gio::prelude::*;
use gtk4::prelude::*;
use gtk4::{self, Align};

use crate::ui::base_module::{BaseModule, StatusFn, make_file_dialog};
use crate::ui::mixins::midi_listener::{ListenerState, MidiListenerMixin};
use crate::ui::widgets::{ChooseOnePill, NumberSpinner};
use c7_core::device_config::DeviceConfig;
use c7_core::midi::send_sysex;
use c7_core::sds::{is_sds_message, make_sds_confirmation, normalize_sds_data_packet, save_received_sample};

// ── Internal state ─────────────────────────────────────────────────────────────────────────────────────────

struct ReceiveSampleState {
    packets: Vec<Vec<u8>>,
    suggested_name: String,
}

/// Feature screen for capturing MMA SDS sample dumps from Elektron hardware.
pub(crate) struct ReceiveSampleScreen {
    pub root: gtk4::Box,
}

impl ReceiveSampleScreen {
    /// Initializes the sample reception screen.
    pub(crate) fn new(
        go_to_menu: impl Fn() + 'static,
        get_selected_midi: impl Fn() -> Option<String> + 'static,
        delay_spin: &NumberSpinner,
        active_config: &str,
    ) -> Self {
        let mut base = BaseModule::new();

        let dc = DeviceConfig::find_by_path_or_die(active_config);
        let device_short = dc.device_short.clone();
        let device_shorter = dc.device_shorter.clone();
        let sysex_header = dc.sysex_header.clone();
        let name_tag_byte = dc.sysex_command_byte("sample_name_tag", "block_byte");

        base.build_header("Receive Sample", go_to_menu);
        base.build_instructions_card(
            &[
                "1. Choose your desired output formats (WAV/SDS/C7) from the toggles below.",
                &format!("2. On the {device_short}, navigate to GLOBAL > FILE > SAMPLE MGR."),
                "3. Change the Mode to SEND and select the sample you want to extract.",
                &format!("4. Press YES on the {device_shorter}. The app is actively listening and will catch it automatically."),
                &format!("5. A save dialog will appear once the {device_short} finishes transmitting."),
            ],
            "Device Prep",
        );

        // Output format selector. Choose between WAV, raw SDS binary, or a `.c7` file.
        let format_pill = ChooseOnePill::new(&["Save as WAV", "Save as SDS", "Save as C7"], 0);
        format_pill.widget().set_halign(Align::Center);
        base.root.append(format_pill.widget());

        // Standard status line (separator + hidden progress bar + label) from `BaseModule`.
        base.build_status_area(false);

        let sample_state: Rc<RefCell<ReceiveSampleState>> = Rc::new(RefCell::new(ReceiveSampleState {
            packets: vec![],
            suggested_name: "SAMPLE".to_string(),
        }));

        // Thread-safe status setter shared by the listener callbacks and the save dialog.
        let update_status = base.status_updater();
        update_status("Status: Waiting to enter page...");

        // Lazy listener-state reference: populated after mixin construction so `on_midi_message()` can set fields on `ListenerState`.
        let lazy_ls: Rc<RefCell<Option<Rc<RefCell<ListenerState>>>>> = Rc::new(RefCell::new(None));

        let get_midi_rc: Rc<dyn Fn() -> Option<String>> = Rc::new(get_selected_midi);

        // ── on_midi_message ────────────────────────────────────────────────────────────────────────────────

        let on_midi_message = {
            let sample_state = Rc::clone(&sample_state);
            let update_status = Arc::clone(&update_status);
            let get_midi_rc = Rc::clone(&get_midi_rc);
            let lazy_ls = Rc::clone(&lazy_ls);
            move |data: Vec<u8>| {
                // Processes incoming MIDI traffic for Elektron name tags and standard SDS packets.
                let header_len = sysex_header.len();

                // Detect Elektron sample name tag packets.
                let is_name_tag = data.len() > header_len + 1
                    && data.get(..header_len) == Some(&sysex_header[..])
                    && data.get(header_len + 1) == Some(&name_tag_byte);

                if is_name_tag {
                    // Set suggested filename based on the received name tag.
                    let name_bytes = if data.len() > header_len + 3 {
                        &data[header_len + 3..data.len() - 1] // strip trailing F7
                    } else {
                        &[]
                    };

                    let name: String = name_bytes
                        .iter()
                        .filter_map(|&c| (32..=126).contains(&c).then_some(c as char))
                        .collect::<String>()
                        .trim()
                        .to_string();
                    if !name.is_empty() {
                        sample_state.borrow_mut().suggested_name = name;
                    }

                    mark_received(&lazy_ls);
                }
                // Detect and process SDS messages.
                else if is_sds_message(&data) {
                    let ch = data.get(2).copied().unwrap_or(0);
                    let sub_id = data.get(3).copied().unwrap_or(0);

                    // Handle SDS Dump Header (initialization of transfer).
                    if sub_id == 0x01 {
                        sample_state.borrow_mut().packets.push(data);
                        mark_received(&lazy_ls);
                        send_ack(&get_midi_rc, ch, 0);
                    }
                    // Handle SDS Data Packet (streaming sample data).
                    else if sub_id == 0x02 {
                        let packet_num = data.get(4).copied().unwrap_or(0);
                        // Normalize short packets to the 120-byte payload expected by some tools.
                        let normalized_packet = normalize_sds_data_packet(data);
                        let count = {
                            let mut state_mut = sample_state.borrow_mut();
                            state_mut.packets.push(normalized_packet);
                            state_mut.packets.len()
                        };
                        mark_received(&lazy_ls);
                        send_ack(&get_midi_rc, ch, packet_num);
                        // Provide UI feedback every 10 packets received.
                        if count % 10 == 0 {
                            update_status(&format!("Status: Receiving packet {count}..."));
                        }
                    }
                }
            }
        };

        // ── prompt_save flag ───────────────────────────────────────────────────────────────────────────────

        // `MidiListenerMixin` calls this when a 2.5-second silence is detected.
        // A polling timer on the main thread checks this flag and opens the dialog.
        let is_prompt_pending: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let prompt_save = {
            let is_prompt_pending = Rc::clone(&is_prompt_pending);
            move || {
                // Displays a native save dialog to store the captured sample data.
                is_prompt_pending.set(true);
            }
        };

        // ── `save_lock_delay` / `restore_delay` ────────────────────────────────────────────────────────────

        // `save_lock` disables the spinner on entry.
        // `restore` re-enables it on exit.
        let (save_lock_delay, restore_delay) = delay_spin.delay_lifecycle();

        // ── reset_state ────────────────────────────────────────────────────────────────────────────────────

        let reset_ls: Rc<RefCell<Option<Rc<RefCell<ListenerState>>>>> = Rc::new(RefCell::new(None));
        let reset_state = {
            let sample_state = Rc::clone(&sample_state);
            let update_status = Arc::clone(&update_status);
            let reset_ls = Rc::clone(&reset_ls);
            move || {
                // Clears buffers and resets flags to prepare for the next incoming sample.
                if let Some(listener_state) = reset_ls.borrow().as_ref() {
                    reset_capture(&sample_state, listener_state, &update_status);
                }
            }
        };

        let mixin = Rc::new(MidiListenerMixin::new(
            move || get_midi_rc(),
            on_midi_message,
            {
                let update_status = Arc::clone(&update_status);
                move |text: &str| update_status(text)
            },
            prompt_save,
            // `connected_status`: shown when a port successfully opens.
            |target_in: &str| format!("Status: Listening on '{target_in}'... Waiting for sample."),
            |_port: &str| {}, // open_extra_ports: not needed for sample receive
            save_lock_delay,
            restore_delay,
            reset_state,
        ));

        // Populate lazy listener-state references now that the mixin exists.
        let listener_state = mixin.listener_state();
        *lazy_ls.borrow_mut() = Some(Rc::clone(&listener_state));
        *reset_ls.borrow_mut() = Some(Rc::clone(&listener_state));

        // ── map / unmap lifecycle hooks ────────────────────────────────────────────────────────────────────

        {
            let mixin_c = Rc::clone(&mixin);
            base.root.connect_map(move |_| mixin_c.on_map());
        }
        {
            let mixin_c = Rc::clone(&mixin);
            base.root.connect_unmap(move |_| mixin_c.on_unmap());
        }

        // ── Prompt-save polling (100 ms) ───────────────────────────────────────────────────────────────────

        // The mixin's completion timer sets `is_prompt_pending`. This opens the dialog.
        {
            let is_prompt_pending = Rc::clone(&is_prompt_pending);
            let sample_state = Rc::clone(&sample_state);
            let update_status = Arc::clone(&update_status);
            let listener_state = Rc::clone(&listener_state);

            glib::timeout_add_local(Duration::from_millis(100), move || {
                if !is_prompt_pending.get() {
                    return glib::ControlFlow::Continue;
                }
                is_prompt_pending.set(false);

                // Snapshot captured data and format preference before opening the dialog.
                let (suggested_name, packets) = {
                    let state_ref = sample_state.borrow();
                    (state_ref.suggested_name.clone(), state_ref.packets.clone())
                };
                let should_save_wav = format_pill.active() == 0;
                let should_save_sds = format_pill.active() == 1;
                let should_save_c7 = format_pill.active() == 2;

                let sample_state = Rc::clone(&sample_state);
                let update_status = Arc::clone(&update_status);
                let listener_state = Rc::clone(&listener_state);
                let device_short = device_short.clone();

                let dialog = make_file_dialog("Save Sample As...", Some(&suggested_name));

                dialog.save(None::<&gtk4::Window>, None::<&gio::Cancellable>, move |result| {
                    // Processes the save dialog response and writes the sample to disk.
                    if let Ok(file) = result {
                        // Execute the save workflow if the user accepted the dialog.
                        if let Some(path) = file.path() {
                            update_status("Status: Saving...");
                            // Resolve user's format preference and write files.
                            let save_result = save_received_sample(
                                &packets,
                                &path,
                                should_save_wav,
                                should_save_sds,
                                should_save_c7,
                                &suggested_name,
                                &device_short,
                            );
                            match save_result {
                                Ok(saved_files) => update_status(&format!("Successfully saved: {}", saved_files.join(", "))),
                                // Reports file I/O or conversion errors to the status label.
                                Err(e) => update_status(&format!("Error: {e}")),
                            }
                            let sample_state = Rc::clone(&sample_state);
                            let update_status = Arc::clone(&update_status);
                            let listener_state = Rc::clone(&listener_state);
                            // Delay the reset so the user can read the result message.
                            glib::timeout_add_local_once(Duration::from_secs(4), move || {
                                reset_capture(&sample_state, &listener_state, &update_status);
                            });
                            return;
                        }
                    }
                    // Discard if the user cancelled the dialog.
                    update_status("Status: Discarded received sample.");
                    reset_capture(&sample_state, &listener_state, &update_status);
                });
                glib::ControlFlow::Continue
            });
        }

        ReceiveSampleScreen { root: base.root }
    }

    /// Returns the root widget for embedding in the navigation stack.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ── Module-level helpers ───────────────────────────────────────────────────────────────────────────────────

/// Sets `is_receiving_data=true` and resets `last_packet_time` in the mixin's `ListenerState`.
fn mark_received(lazy_ls: &Rc<RefCell<Option<Rc<RefCell<ListenerState>>>>>) {
    if let Some(listener_state) = lazy_ls.borrow().as_ref() {
        let mut state_mut = listener_state.borrow_mut();
        state_mut.is_receiving_data = true;
        state_mut.last_packet_time = Instant::now();
    }
}

/// Transmits an SDS ACK message to the device to confirm packet reception.
fn send_ack(get_midi: &Rc<dyn Fn() -> Option<String>>, ch: u8, packet_num: u8) {
    // Skip if no port is selected (e.g. device disconnected mid-transfer).
    if let Some(port) = get_midi() {
        send_sysex(&port, &make_sds_confirmation(ch, packet_num));
    }
}

/// Clears the capture buffer and restores the listening status if the page is still active.
fn reset_capture(state: &Rc<RefCell<ReceiveSampleState>>, listener_state: &Rc<RefCell<ListenerState>>, update_status: &StatusFn) {
    // Clears buffers and resets flags to prepare for the next incoming sample.
    let mut state_mut = state.borrow_mut();
    state_mut.packets = vec![];
    state_mut.suggested_name = "SAMPLE".to_string();
    drop(state_mut);
    // Restore listener status if the screen is still the active page.
    if let Some(port) = listener_state.borrow().current_listen_port.as_deref() {
        update_status(&format!("Status: Listening on '{port}'... Waiting for sample."));
    }
}
