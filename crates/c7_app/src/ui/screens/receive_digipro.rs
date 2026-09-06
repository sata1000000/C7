//! Receive DigiPro wavetables from Monomachine slots.

/*
Structurally identical to `receive_sample.rs`, but with different section content objectives.
If you change ANY structural code in here, please change `receive_sample.rs` to use the new standard too.
*/

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gio::prelude::*;
use gtk4::prelude::*;

use crate::ui::base_module::{BaseModule, StatusFn, make_file_dialog};
use crate::ui::mixins::midi_listener::{ListenerState, MidiListenerMixin};
use crate::ui::widgets::NumberSpinner;
use c7_core::device_config::DeviceConfig;
use c7_core::digipro::{digipro_name, is_digipro_packet};

// ── Internal state ─────────────────────────────────────────────────────────────────────────────────────────

struct ReceiveDigiproState {
    packets: Vec<Vec<u8>>,
    suggested_name: String,
}

/// Feature screen for downloading DigiPro wavetables from Monomachine slots.
pub(crate) struct ReceiveDigiproScreen {
    pub root: gtk4::Box,
}

impl ReceiveDigiproScreen {
    /// Initializes the DigiPro reception screen.
    pub(crate) fn new(
        go_to_menu: impl Fn() + 'static,
        get_selected_midi: impl Fn() -> Option<String> + 'static,
        delay_spin: &NumberSpinner,
        active_config: &str,
    ) -> Self {
        let mut base = BaseModule::new();

        let dc = Arc::new(DeviceConfig::find_by_path_or_die(active_config));
        let device_short = dc.device_short.clone();
        let device_shorter = dc.device_shorter.clone();
        let prod = dc.prod;
        let digipro_offset_slot = dc.layout_offset_or_die("digipro", "slot") as usize;

        base.build_header("Receive DigiPro", go_to_menu);
        base.build_instructions_card(
            &[
                &format!("1. On the {device_short}, navigate to GLOBAL > FILE > DIGIPRO MGR."),
                "2. Select SEND and choose the waveform slot you want to extract.",
                &format!("3. Press YES on the {device_shorter}. The app is actively listening and will catch it automatically."),
                &format!("4. A save dialog will appear once the {device_short} finishes transmitting."),
            ],
            "Device Prep",
        );

        // Standard status line (separator + hidden progress bar + label) from `BaseModule`.
        base.build_status_area(false);

        let digipro_state: Rc<RefCell<ReceiveDigiproState>> = Rc::new(RefCell::new(ReceiveDigiproState {
            packets: vec![],
            suggested_name: "DIGIPRO".to_string(),
        }));

        // Thread-safe status setter shared by the listener callbacks and the save dialog.
        let update_status = base.status_updater();
        update_status("Status: Waiting to enter page...");

        // Lazy listener-state reference: populated after mixin construction so `on_midi_message()` can set fields on `ListenerState`.
        let lazy_ls: Rc<RefCell<Option<Rc<RefCell<ListenerState>>>>> = Rc::new(RefCell::new(None));

        let get_midi_rc: Rc<dyn Fn() -> Option<String>> = Rc::new(get_selected_midi);

        // ── on_midi_message ────────────────────────────────────────────────────────────────────────────────

        let on_midi_message = {
            let digipro_state = Rc::clone(&digipro_state);
            let update_status = Arc::clone(&update_status);
            let lazy_ls = Rc::clone(&lazy_ls);
            let dc = Arc::clone(&dc);
            move |data: Vec<u8>| {
                // Only process SysEx messages matching this device's DigiPro dump format.
                if is_digipro_packet(&dc, &data) && data.get(4) == Some(&prod) {
                    let slot = data.get(digipro_offset_slot).copied().unwrap_or(0);
                    let mut state_mut = digipro_state.borrow_mut();
                    // Extract the suggested name from the first packet received.
                    if state_mut.packets.is_empty() {
                        let raw_name = digipro_name(&data, &dc);
                        state_mut.suggested_name = if raw_name.is_empty() {
                            format!("digipro_{slot:02}")
                        } else {
                            raw_name
                        };
                    }

                    state_mut.packets.push(data);
                    drop(state_mut);
                    update_status(&format!("Status: Receiving DigiPro slot {slot}..."));
                    // Mark the mixin's completion timer so it fires after 2.5 s of silence.
                    mark_received(&lazy_ls);
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
                // Displays a native save dialog to store the captured DigiPro data.
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
            let digipro_state = Rc::clone(&digipro_state);
            let update_status = Arc::clone(&update_status);
            let reset_ls = Rc::clone(&reset_ls);
            move || {
                // Clears buffers and resets flags to prepare for the next incoming waveform.
                if let Some(listener_state) = reset_ls.borrow().as_ref() {
                    reset_capture(&digipro_state, listener_state, &update_status);
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
            |target_in: &str| format!("Status: Listening on '{target_in}'... Waiting for DigiPro waveform."),
            |_port: &str| {}, // open_extra_ports: not needed for DigiPro receive
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
            let digipro_state = Rc::clone(&digipro_state);
            let update_status = Arc::clone(&update_status);
            let listener_state = Rc::clone(&listener_state);

            glib::timeout_add_local(Duration::from_millis(100), move || {
                if !is_prompt_pending.get() {
                    return glib::ControlFlow::Continue;
                }
                is_prompt_pending.set(false);

                // Snapshot captured data before opening the dialog.
                let (suggested_name, packets) = {
                    let state_ref = digipro_state.borrow();
                    (state_ref.suggested_name.clone(), state_ref.packets.clone())
                };

                let digipro_state = Rc::clone(&digipro_state);
                let update_status = Arc::clone(&update_status);
                let listener_state = Rc::clone(&listener_state);

                let dialog = make_file_dialog("Save DigiPro Waveform As...", Some(&suggested_name));

                dialog.save(None::<&gtk4::Window>, None::<&gio::Cancellable>, move |result| {
                    // Processes the save dialog response and writes the waveform to disk.
                    if let Ok(file) = result {
                        // Execute the save workflow if the user accepted the dialog.
                        if let Some(path) = file.path() {
                            update_status("Status: Saving...");
                            // DigiPro SysEx files are small enough to write on the main thread.
                            let syx_path = path.with_extension("syx");
                            let write_result: Result<(), String> = (|| {
                                use std::io::Write;
                                let mut file = std::fs::File::create(&syx_path).map_err(|e| e.to_string())?;
                                // Consolidate the captured SysEx packets into a single binary payload.
                                for packet in &packets {
                                    file.write_all(packet).map_err(|e| e.to_string())?;
                                }
                                Ok(())
                            })();
                            match write_result {
                                Ok(()) => {
                                    let name = syx_path
                                        .file_name()
                                        .map(|name| name.to_string_lossy().into_owned())
                                        .unwrap_or_default();
                                    update_status(&format!("Successfully saved: {name}"));
                                }
                                // Reports file I/O or conversion errors to the status label.
                                Err(e) => update_status(&format!("Error: {e}")),
                            }
                            let digipro_state = Rc::clone(&digipro_state);
                            let update_status = Arc::clone(&update_status);
                            let listener_state = Rc::clone(&listener_state);
                            // Delay the reset so the user can read the result message.
                            glib::timeout_add_local_once(Duration::from_secs(4), move || {
                                reset_capture(&digipro_state, &listener_state, &update_status);
                            });
                            return;
                        }
                    }
                    // Discard if the user cancelled the dialog.
                    update_status("Status: Discarded received DigiPro waveform.");
                    reset_capture(&digipro_state, &listener_state, &update_status);
                });
                glib::ControlFlow::Continue
            });
        }

        ReceiveDigiproScreen { root: base.root }
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

/// Clears the capture buffer and restores the listening status if the page is still active.
fn reset_capture(state: &Rc<RefCell<ReceiveDigiproState>>, listener_state: &Rc<RefCell<ListenerState>>, update_status: &StatusFn) {
    // Clears buffers and resets flags to prepare for the next incoming waveform.
    let mut state_mut = state.borrow_mut();
    state_mut.packets = vec![];
    state_mut.suggested_name = "DIGIPRO".to_string();
    drop(state_mut);
    // Restore listener status if the screen is still the active page.
    if let Some(port) = listener_state.borrow().current_listen_port.as_deref() {
        update_status(&format!("Status: Listening on '{port}'... Waiting for DigiPro waveform."));
    }
}
