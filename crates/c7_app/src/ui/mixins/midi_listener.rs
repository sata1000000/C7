//! Shared MIDI input listening behavior for the receive-style screens.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::ui::base_module::listen;
use c7_core::midi::{PollableMidiInput, find_input_port, is_valid_port, open_large_sysex_input};

// ── Internal state ─────────────────────────────────────────────────────────────────────────────────────────

/// Per-mixin state visible to receive screens so they can set `is_receiving_data` / `last_packet_time`.
pub(crate) struct ListenerState {
    pub midi_in: Option<PollableMidiInput>,
    pub current_listen_port: Option<String>,
    pub is_active_page: bool,
    pub is_receiving_data: bool,
    pub last_packet_time: Instant,
}

impl ListenerState {
    /// Returns the default state for the MIDI listener.
    fn new() -> Self {
        ListenerState {
            midi_in: None,
            current_listen_port: None,
            is_active_page: false,
            is_receiving_data: false,
            last_packet_time: Instant::now(),
        }
    }
}

// ── MidiListenerMixin ──────────────────────────────────────────────────────────────────────────────────────

/// Map/unmap-driven MIDI input port tracker for receive-style screens.
///
/// Handles port polling, reconnect-on-change, and the completion timeout.
/// Interactive screens reuse only its safe-close logic via `close_midi_in()`.
///
/// Composition pattern: embed in a screen struct, call `on_map()`/`on_unmap()` from the widget's map/unmap signals.
/// Customize behavior via the callbacks passed to `new()`.
pub(crate) struct MidiListenerMixin {
    state: Rc<RefCell<ListenerState>>,
    get_midi_rc: Rc<dyn Fn() -> Option<String>>,
    on_midi_message: Rc<dyn Fn(Vec<u8>)>,
    update_status: Rc<dyn Fn(&str)>,
    prompt_save: Rc<dyn Fn()>,
    connected_status: Rc<dyn Fn(&str) -> String>,
    open_extra_ports: Rc<dyn Fn(&str)>,
    save_lock_delay: Rc<dyn Fn()>,
    restore_delay: Rc<dyn Fn()>,
    reset_state: Rc<dyn Fn()>,
}

impl MidiListenerMixin {
    /// Creates a new listener mixin with full callback customization.
    ///
    /// Pass a closure per override point to replace that point's default behavior.
    /// e.g. `|target_in| format!("Listening on {target_in}...")` for `connected_status`.
    pub(crate) fn new(
        get_midi_rc: impl Fn() -> Option<String> + 'static,
        on_midi_message: impl Fn(Vec<u8>) + 'static,
        update_status: impl Fn(&str) + 'static,
        prompt_save: impl Fn() + 'static,
        connected_status: impl Fn(&str) -> String + 'static,
        open_extra_ports: impl Fn(&str) + 'static,
        save_lock_delay: impl Fn() + 'static,
        restore_delay: impl Fn() + 'static,
        reset_state: impl Fn() + 'static,
    ) -> Self {
        MidiListenerMixin {
            state: Rc::new(RefCell::new(ListenerState::new())),
            get_midi_rc: Rc::new(get_midi_rc),
            on_midi_message: Rc::new(on_midi_message),
            update_status: Rc::new(update_status),
            prompt_save: Rc::new(prompt_save),
            connected_status: Rc::new(connected_status),
            open_extra_ports: Rc::new(open_extra_ports),
            save_lock_delay: Rc::new(save_lock_delay),
            restore_delay: Rc::new(restore_delay),
            reset_state: Rc::new(reset_state),
        }
    }

    // ── Lifecycle Management ───────────────────────────────────────────────────────────────────────────────

    /// Begins port polling and completion tracking.
    ///
    /// Called when the screen is mapped.
    pub(crate) fn on_map(&self) {
        self.state.borrow_mut().is_active_page = true;
        (self.save_lock_delay)();
        (self.update_status)("Status: Locating MIDI port...");
        self.start_port_poll_timer();
        self.start_completion_timer();
    }

    /// Halts polling and closes MIDI ports.
    ///
    /// Called when the screen is unmapped.
    pub(crate) fn on_unmap(&self) {
        self.state.borrow_mut().is_active_page = false;
        (self.restore_delay)();
        self.close_midi_in();
        (self.reset_state)();
    }

    // ── Port Management ────────────────────────────────────────────────────────────────────────────────────

    /// Safely closes the MIDI input port if it is currently open.
    fn close_midi_in(&self) {
        let mut state_mut = self.state.borrow_mut();
        // Dropping `midi_in` triggers midir's Drop impl which disconnects the port.
        state_mut.midi_in = None;
        state_mut.current_listen_port = None;
    }

    /// Returns an Rc clone of the shared listener state.
    ///
    /// Receive screens use this to update `is_receiving_data` and `last_packet_time`.
    pub(crate) fn listener_state(&self) -> Rc<RefCell<ListenerState>> {
        Rc::clone(&self.state)
    }

    // ── Port-poll timer (1 000 ms) ─────────────────────────────────────────────────────────────────────────

    /// Starts the periodic timer that polls for MIDI port changes.
    fn start_port_poll_timer(&self) {
        let state = Rc::clone(&self.state);
        let get_midi_rc = Rc::clone(&self.get_midi_rc);
        let update_status = Rc::clone(&self.update_status);
        let connected_status = Rc::clone(&self.connected_status);
        let open_extra_ports = Rc::clone(&self.open_extra_ports);
        let on_midi_message = Rc::clone(&self.on_midi_message);

        glib::timeout_add_local(Duration::from_secs(1), move || {
            if !state.borrow().is_active_page {
                return glib::ControlFlow::Break;
            }
            let port_out = get_midi_rc();
            if let Some(port) = port_out
                && is_valid_port(&port)
            {
                let current = state.borrow().current_listen_port.clone();
                if current.as_deref() != Some(port.as_str()) {
                    {
                        let mut state_mut = state.borrow_mut();
                        state_mut.midi_in = None;
                        state_mut.current_listen_port = None;
                    }
                    if let Some(target_in) = find_input_port(&port) {
                        match open_large_sysex_input(&target_in) {
                            Ok(midi_in) => {
                                let status = connected_status(&target_in);
                                // Clone the receiver before storing `midi_in`.
                                //
                                // Each new port connection creates a fresh channel, so attaching is always safe.
                                // The old task exits when its channel closes.
                                let rx = midi_in.rx.clone();
                                {
                                    let mut state_mut = state.borrow_mut();
                                    state_mut.midi_in = Some(midi_in);
                                    state_mut.current_listen_port = Some(port.clone());
                                }
                                update_status(&status);
                                open_extra_ports(&port);
                                let state_c = Rc::clone(&state);
                                let on_midi_message_c = Rc::clone(&on_midi_message);
                                listen(rx, move |data| {
                                    if !state_c.borrow().is_active_page {
                                        return glib::ControlFlow::Break;
                                    }
                                    on_midi_message_c(data);
                                    glib::ControlFlow::Continue
                                });
                            }
                            Err(e) => {
                                {
                                    let mut state_mut = state.borrow_mut();
                                    state_mut.midi_in = None;
                                    state_mut.current_listen_port = None;
                                }
                                update_status(&format!("Error: Failed to open ports: {e}"));
                            }
                        }
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    // ── Completion-detection timer (300 ms) ────────────────────────────────────────────────────────────────

    /// Starts the timer that detects when a transfer has finished.
    fn start_completion_timer(&self) {
        let state = Rc::clone(&self.state);
        let update_status = Rc::clone(&self.update_status);
        let prompt_save = Rc::clone(&self.prompt_save);

        glib::timeout_add_local(Duration::from_millis(300), move || {
            if !state.borrow().is_active_page {
                return glib::ControlFlow::Break;
            }
            let (receiving, elapsed) = {
                let state_ref = state.borrow();
                (state_ref.is_receiving_data, state_ref.last_packet_time.elapsed().as_secs_f64())
            };
            // Detects end of a data transfer based on the 2.5-second silence timeout.
            if receiving && elapsed > 2.5 {
                state.borrow_mut().is_receiving_data = false;
                update_status("Status: Transfer finished. Select save location...");
                prompt_save();
            }
            glib::ControlFlow::Continue
        });
    }
}
