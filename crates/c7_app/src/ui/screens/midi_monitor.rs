//! Watch live MIDI and SysEx traffic.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{self, Align, Button, CheckButton, FlowBox, FlowBoxChild, Label, Orientation, ScrolledWindow, Separator, TextView, WrapMode};

use crate::ui::base_module::listen;
use c7_core::device_config::{read_settings, update_setting};
use c7_core::midi::{PollableMidiInput, find_input_port, is_valid_port, midi_tokens, open_large_sysex_input, register_outbound_callback};
use c7_core::utils::now_clock_string;

/// Loaded once at import time.
///
/// `MidiMonitorScreen` is recreated on every device selection, so loading inside `new()` would stack duplicate providers.
static CSS_ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Ensures monitor-specific CSS is loaded.
fn ensure_monitor_css() {
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

// ── Internal state ─────────────────────────────────────────────────────────────────────────────────────────

struct MidiMonitorState {
    midi_in: Option<PollableMidiInput>,
    is_listening: bool,
    last_sysex_data: Option<Vec<u8>>,
    last_midi_msg: Option<Vec<u8>>,
}

/// Feature screen displaying a live feed of incoming and outgoing MIDI messages.
pub(crate) struct MidiMonitorScreen {
    pub root: gtk4::Box,
}

impl MidiMonitorScreen {
    /// Initializes the MIDI monitor screen.
    pub(crate) fn new(go_to_menu: impl Fn() + 'static, get_selected_midi: impl Fn() -> Option<String> + 'static) -> Self {
        ensure_monitor_css();

        // FILL so the `ScrolledWindow` stretches to the window bottom.
        let root = gtk4::Box::new(Orientation::Vertical, 12);
        root.set_valign(Align::Fill);
        root.set_margin_top(15);
        root.set_margin_bottom(15);
        root.set_margin_start(15);
        root.set_margin_end(15);

        // ── Top bar ────────────────────────────────────────────────────────────────────────────────────────

        let top_bar = gtk4::CenterBox::new();
        top_bar.set_margin_bottom(8);

        let left_box = gtk4::Box::new(Orientation::Horizontal, 12);
        let back_btn = Button::with_label("← Back");
        back_btn.set_sensitive(false);
        {
            let back_btn_for_handler = back_btn.clone();
            // Disable immediately before navigating so the button can't be double-clicked and is correct if the monitor reopens.
            back_btn.connect_clicked(move |_| {
                back_btn_for_handler.set_sensitive(false);
                go_to_menu();
            });
        }
        left_box.append(&back_btn);
        top_bar.set_start_widget(Some(&left_box));

        top_bar.set_center_widget(Some(&{
            let title_label = Label::new(Some("SysEx & MIDI Monitor"));
            title_label.add_css_class("title-2");
            title_label
        }));

        root.append(&top_bar);
        root.append(&Separator::new(Orientation::Horizontal));

        let state: Rc<RefCell<MidiMonitorState>> = Rc::new(RefCell::new(MidiMonitorState {
            midi_in: None,
            is_listening: false,
            last_sysex_data: None,
            last_midi_msg: None,
        }));

        // ── Controls ───────────────────────────────────────────────────────────────────────────────────────

        let controls_box = gtk4::Box::new(Orientation::Vertical, 8);
        controls_box.set_margin_top(5);
        controls_box.set_margin_bottom(5);

        // Checkboxes are instance attributes so filter methods can read them.
        let filter_inbound_check = CheckButton::with_label("Filter Inbound");
        let filter_outbound_check = CheckButton::with_label("Filter Outbound");
        let filter_clock_check = CheckButton::with_label("Filter Clock");
        let filter_note_check = CheckButton::with_label("Filter Notes");
        let filter_cc_check = CheckButton::with_label("Filter CC");
        let filter_sysex_check = CheckButton::with_label("Filter SysEx");
        let auto_scroll_check = CheckButton::with_label("Scroll to Bottom");
        let wrap_check = CheckButton::with_label("Wrap Lines");

        // Load persisted settings or fall back to standard defaults.
        let settings = read_settings();
        let monitor_cfg = settings.get("midi_monitor").and_then(|value| value.as_object());
        let get_bool = |key: &str, default: bool| -> bool {
            monitor_cfg
                .and_then(|map| map.get(key))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(default)
        };

        filter_inbound_check.set_active(get_bool("filter_inbound", false));
        filter_outbound_check.set_active(get_bool("filter_outbound", true));
        filter_clock_check.set_active(get_bool("filter_clock", true));
        filter_note_check.set_active(get_bool("filter_note", false));
        filter_cc_check.set_active(get_bool("filter_cc", false));
        filter_sysex_check.set_active(get_bool("filter_sysex", false));
        auto_scroll_check.set_active(get_bool("scroll_to_bottom", true));
        wrap_check.set_active(get_bool("wrap_lines", false));

        // `FlowBox` keeps all checkboxes in one horizontal row. Limit raised above GTK's default of 7.
        let checkboxes_flow = FlowBox::new();
        checkboxes_flow.set_selection_mode(gtk4::SelectionMode::None);
        checkboxes_flow.set_column_spacing(12);
        checkboxes_flow.set_row_spacing(6);
        checkboxes_flow.set_max_children_per_line(12);

        for checkbox in [
            &filter_inbound_check,
            &filter_outbound_check,
            &filter_clock_check,
            &filter_note_check,
            &filter_cc_check,
            &filter_sysex_check,
            &auto_scroll_check,
            &wrap_check,
        ] {
            // `set_focusable(false)` on the wrapper keeps keyboard nav on the checkbox itself.
            let child = FlowBoxChild::new();
            child.set_focusable(false);
            child.set_child(Some(checkbox));
            checkboxes_flow.insert(&child, -1);
        }

        let listen_btn = Button::with_label("Start Listening");
        listen_btn.add_css_class("suggested-action");

        let clear_btn = Button::with_label("Clear Output");

        let buttons_box = gtk4::Box::new(Orientation::Horizontal, 8);
        buttons_box.append(&listen_btn);
        buttons_box.append(&clear_btn);

        controls_box.append(&checkboxes_flow);
        controls_box.append(&buttons_box);
        root.append(&controls_box);

        // ── Text Output Area ───────────────────────────────────────────────────────────────────────────────

        let scroll = ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_hexpand(true);
        scroll.set_has_frame(false);
        scroll.add_css_class("log-scroller");

        let text_view = TextView::new();
        text_view.set_editable(false);
        text_view.set_wrap_mode(if wrap_check.is_active() {
            WrapMode::WordChar
        } else {
            WrapMode::None
        });
        text_view.add_css_class("monospace");
        text_view.add_css_class("log-terminal");

        // Use internal text padding so the background color fills the rounded area.
        text_view.set_top_margin(12);
        text_view.set_bottom_margin(12);
        text_view.set_left_margin(12);
        text_view.set_right_margin(12);

        let buffer = text_view.buffer();
        buffer.create_tag(
            Some("sysex-diff"),
            &[
                ("foreground", &"#ff7043" as &dyn glib::prelude::ToValue),
                ("weight", &700i32 as &dyn glib::prelude::ToValue),
            ],
        );
        buffer.create_tag(
            Some("midi-diff"),
            &[
                ("foreground", &"#66bb6a" as &dyn glib::prelude::ToValue),
                ("weight", &700i32 as &dyn glib::prelude::ToValue),
            ],
        );

        scroll.set_child(Some(&text_view));
        root.append(&scroll);

        // ── Auto-scroll adjustment listener ────────────────────────────────────────────────────────────────

        {
            let adjustment = scroll.vadjustment();
            let auto_scroll_check = auto_scroll_check.clone();
            adjustment.connect_changed(move |adjustment| {
                if auto_scroll_check.is_active() {
                    adjustment.set_value(adjustment.upper() - adjustment.page_size());
                }
            });
        }

        let checkboxes = [
            (filter_inbound_check.clone(), "filter_inbound"),
            (filter_outbound_check.clone(), "filter_outbound"),
            (filter_clock_check.clone(), "filter_clock"),
            (filter_note_check.clone(), "filter_note"),
            (filter_cc_check.clone(), "filter_cc"),
            (filter_sysex_check.clone(), "filter_sysex"),
            (auto_scroll_check, "scroll_to_bottom"),
            (wrap_check.clone(), "wrap_lines"),
        ];

        // Helper closure to persist all filter states
        let save_settings = {
            let checkboxes = checkboxes.clone();
            move || {
                let mut monitor_map = serde_json::Map::new();
                for (checkbox, key) in &checkboxes {
                    monitor_map.insert(key.to_string(), serde_json::Value::Bool(checkbox.is_active()));
                }
                update_setting("midi_monitor", serde_json::Value::Object(monitor_map));
            }
        };

        // ── Wrap toggle ────────────────────────────────────────────────────────────────────────────────────

        {
            let text_view = text_view.clone();
            let save_settings = save_settings.clone();
            wrap_check.connect_toggled(move |btn| {
                // Updates the text view's wrap mode based on the user's checkbox selection.
                text_view.set_wrap_mode(if btn.is_active() { WrapMode::WordChar } else { WrapMode::None });
                save_settings();
            });
        }

        // Connect save settings helper to all remaining toggles.
        for (checkbox, key) in &checkboxes {
            if *key == "wrap_lines" {
                continue;
            }
            let save_settings = save_settings.clone();
            checkbox.connect_toggled(move |_| {
                save_settings();
            });
        }

        // ── Clear button ───────────────────────────────────────────────────────────────────────────────────

        {
            let buffer = text_view.buffer();
            let state = Rc::clone(&state);
            clear_btn.connect_clicked(move |_| {
                // Clears the terminal output buffer and resets the message diff history.
                buffer.set_text("");
                let mut state_mut = state.borrow_mut();
                state_mut.last_sysex_data = None;
                state_mut.last_midi_msg = None;
            });
        }

        // ── Outbound message tap ───────────────────────────────────────────────────────────────────────────

        // Flag that the outbound callback reads to skip sending when not listening.
        let is_forwarding = Arc::new(AtomicBool::new(false));

        {
            let (outbound_tx, outbound_rx) = async_channel::unbounded::<Vec<u8>>();
            let is_forwarding = Arc::clone(&is_forwarding);
            // Registers the global outbound tap. The callback stays registered for the life of the process.
            // `is_forwarding` gates whether messages are forwarded to the channel.
            register_outbound_callback(move |msg| {
                if is_forwarding.load(Ordering::Relaxed) {
                    let _ = outbound_tx.try_send(msg.to_vec());
                }
            });

            let state = Rc::clone(&state);
            let text_view = text_view.clone();
            let filter_clock_check = filter_clock_check.clone();
            let filter_note_check = filter_note_check.clone();
            let filter_cc_check = filter_cc_check.clone();
            let filter_sysex_check = filter_sysex_check.clone();

            // Attach the receiver event-driven: fires only when the callback sends a message.
            listen(outbound_rx, move |data| {
                if filter_outbound_check.is_active() {
                    return glib::ControlFlow::Continue;
                }
                if !passes_filter_raw(
                    &filter_clock_check,
                    &filter_note_check,
                    &filter_cc_check,
                    &filter_sysex_check,
                    &data,
                ) {
                    return glib::ControlFlow::Continue;
                }
                let is_sysex = data.first() == Some(&0xF0);
                if !is_sysex {
                    state.borrow_mut().last_sysex_data = None;
                }
                append_to_view(&state, &text_view, data, is_sysex, true);
                glib::ControlFlow::Continue
            });
        }

        // ── Listen button ──────────────────────────────────────────────────────────────────────────────────

        {
            let state = Rc::clone(&state);
            let is_forwarding = Arc::clone(&is_forwarding);
            let get_midi_rc = Rc::new(get_selected_midi);

            listen_btn.connect_clicked(move |btn| {
                if state.borrow().is_listening {
                    // Closes the active MIDI input port and stops message processing.
                    state.borrow_mut().midi_in = None;
                    state.borrow_mut().is_listening = false;
                    is_forwarding.store(false, Ordering::Relaxed);
                    update_listen_btn_ui(btn, false);
                    log_raw(&text_view, "--- Stopped Listening ---");
                } else {
                    // Attempts to open the selected MIDI input port and begin capturing.
                    let port_name = match get_midi_rc() {
                        Some(port_name) if is_valid_port(&port_name) => port_name,
                        _ => {
                            log_raw(&text_view, "--- Error: No target MIDI device selected in top bar ---");
                            return;
                        }
                    };

                    // `find_input_port()` guesses the input port from the output port's name.
                    // Fall back to `port_name` itself for devices that share one name for both directions.
                    let in_port = find_input_port(&port_name).unwrap_or_else(|| port_name.clone());
                    match open_large_sysex_input(&in_port) {
                        Ok(midi_in) => {
                            let rx = midi_in.rx.clone();
                            state.borrow_mut().midi_in = Some(midi_in);
                            state.borrow_mut().is_listening = true;
                            is_forwarding.store(true, Ordering::Relaxed);
                            update_listen_btn_ui(btn, true);
                            log_raw(&text_view, &format!("--- Listening to '{in_port}' ---"));

                            let state = Rc::clone(&state);
                            let text_view = text_view.clone();
                            let filter_inbound_check = filter_inbound_check.clone();
                            let filter_clock_check = filter_clock_check.clone();
                            let filter_note_check = filter_note_check.clone();
                            let filter_cc_check = filter_cc_check.clone();
                            let filter_sysex_check = filter_sysex_check.clone();

                            listen(rx, move |data| {
                                if !state.borrow().is_listening {
                                    return glib::ControlFlow::Break;
                                }

                                // Handles incoming MIDI messages by applying filters and logging.
                                if filter_inbound_check.is_active() {
                                    return glib::ControlFlow::Continue;
                                }
                                if !passes_filter_raw(
                                    &filter_clock_check,
                                    &filter_note_check,
                                    &filter_cc_check,
                                    &filter_sysex_check,
                                    &data,
                                ) {
                                    return glib::ControlFlow::Continue;
                                }
                                let is_sysex = data.first() == Some(&0xF0);
                                // Reset SysEx diff baseline on non-SysEx messages.
                                if !is_sysex {
                                    state.borrow_mut().last_sysex_data = None;
                                }
                                append_to_view(&state, &text_view, data, is_sysex, false);
                                glib::ControlFlow::Continue
                            });
                        }
                        Err(e) => {
                            log_raw(&text_view, &format!("--- Error opening port '{in_port}' as Input: {e} ---"));
                        }
                    }
                }
            });
        }

        {
            root.connect_map(move |_| {
                // Disable the button at the start of each entry, then re-enable it once the stack's 300 ms slide transition has finished.
                //
                // `idle_add_local` isn't enough on the GDK Win32 backend because `connect_map` fires during the transition, not after it.
                // One idle cycle still races the animation, and 350 ms > 300 ms transition duration.
                let back_btn = back_btn.clone();
                back_btn.set_sensitive(false);
                glib::timeout_add_local(Duration::from_millis(350), move || {
                    back_btn.set_sensitive(true);
                    glib::ControlFlow::Break
                });
            });
        }

        MidiMonitorScreen { root }
    }

    /// Returns the root widget for embedding in the navigation stack.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ── Module-level helpers ───────────────────────────────────────────────────────────────────────────────────

/// Updates the listener button's appearance and label to reflect current state.
fn update_listen_btn_ui(btn: &Button, is_active: bool) {
    if is_active {
        btn.set_label("Stop Listening");
        btn.remove_css_class("suggested-action");
        btn.add_css_class("destructive-action");
    } else {
        btn.set_label("Start Listening");
        btn.remove_css_class("destructive-action");
        btn.add_css_class("suggested-action");
    }
}

/// Appends a plain text line to the log.
fn log_raw(text_view: &TextView, text: &str) {
    let buffer = text_view.buffer();
    let line = if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    };
    buffer.insert(&mut buffer.end_iter(), &line);
}

/// Returns `false` if the message should be suppressed by the current filter checkboxes.
fn passes_filter_raw(
    filter_clock: &CheckButton,
    filter_note: &CheckButton,
    filter_cc: &CheckButton,
    filter_sysex: &CheckButton,
    data: &[u8],
) -> bool {
    if data.is_empty() {
        return true;
    }
    let first = data[0];
    if filter_clock.is_active() && (first == 0xF8 || first == 0xFE) {
        return false;
    }
    if filter_note.is_active() && (first & 0xF0 == 0x80 || first & 0xF0 == 0x90) {
        return false;
    }
    if filter_cc.is_active() && first & 0xF0 == 0xB0 {
        return false;
    }
    if filter_sysex.is_active() && first == 0xF0 {
        return false;
    }
    true
}

/// Inserts space-separated tokens into the buffer, tagging each run of tokens that differs from `previous_strings`.
///
/// Runs are merged into one tag application each, rather than one per token.
/// That keeps a multi-thousand-byte SysEx dump from costing thousands of individual `TextBuffer` mutations on the GTK main thread.
fn insert_diff_tokens(buffer: &gtk4::TextBuffer, current_strings: &[String], previous_strings: &[String], tag_name: &str) {
    // Token width varies, hex pairs for SysEx but whole words for MIDI, so each start has to be tracked rather than computed.
    let mut token_start = buffer.end_iter().offset();
    buffer.insert(&mut buffer.end_iter(), &current_strings.join(" "));

    let mut run_start = None;
    for (i, token) in current_strings.iter().enumerate() {
        let changed = i >= previous_strings.len() || *token != previous_strings[i];
        if changed && run_start.is_none() {
            run_start = Some(token_start);
        }
        if !changed && let Some(start) = run_start.take() {
            // `token_start` sits on the separator space that follows the run, so the run itself ends one character earlier.
            buffer.apply_tag_by_name(tag_name, &buffer.iter_at_offset(start), &buffer.iter_at_offset(token_start - 1));
        }
        token_start += token.chars().count() as i32 + 1;
    }

    if let Some(start) = run_start {
        buffer.apply_tag_by_name(tag_name, &buffer.iter_at_offset(start), &buffer.iter_at_offset(token_start - 1));
    }
}

/// Formats and appends a MIDI or SysEx message to the log with diff highlighting.
fn append_to_view(state: &Rc<RefCell<MidiMonitorState>>, text_view: &TextView, data: Vec<u8>, is_sysex: bool, is_out: bool) {
    let buffer = text_view.buffer();
    let timestamp = now_clock_string();

    if is_sysex {
        // Formats and appends a SysEx data dump to the log with byte-level diff highlighting.
        let dir = if is_out { "[OUT]" } else { "[IN ]" };
        let prefix = format!("[{timestamp}] {dir} SYSEX: ");

        // Strip F0/F7 framing to get the payload, then add it back so the full framed message is visible in the log.
        let payload: Vec<u8> = data
            .iter()
            .copied()
            .skip_while(|&byte| byte == 0xF0)
            .take_while(|&byte| byte != 0xF7)
            .collect();
        let current_bytes: Vec<u8> = std::iter::once(0xF0u8)
            .chain(payload.iter().copied())
            .chain(std::iter::once(0xF7u8))
            .collect();
        let current_strings: Vec<String> = current_bytes.iter().map(|byte| format!("{byte:02X}")).collect();

        let prev_data = state.borrow().last_sysex_data.clone();
        if let Some(prev) = prev_data {
            let previous_bytes: Vec<u8> = std::iter::once(0xF0u8)
                .chain(prev.iter().copied())
                .chain(std::iter::once(0xF7u8))
                .collect();
            let previous_strings: Vec<String> = previous_bytes.iter().map(|byte| format!("{byte:02X}")).collect();
            buffer.insert(&mut buffer.end_iter(), &prefix);
            insert_diff_tokens(&buffer, &current_strings, &previous_strings, "sysex-diff");
            buffer.insert(&mut buffer.end_iter(), "\n");
        } else {
            buffer.insert(&mut buffer.end_iter(), &format!("{prefix}{}\n", current_strings.join(" ")));
        }
        state.borrow_mut().last_sysex_data = Some(payload);
    } else {
        // Formats and appends a standard MIDI message to the log with diff highlighting.
        let dir = if is_out { "[OUT]" } else { "[IN] " };
        let prefix = format!("[{timestamp}] {dir} ");
        let current_tokens = midi_tokens(&data);

        let prev_data = state.borrow().last_midi_msg.clone();
        if let Some(prev) = prev_data {
            let previous_tokens = midi_tokens(&prev);
            buffer.insert(&mut buffer.end_iter(), &prefix);
            insert_diff_tokens(&buffer, &current_tokens, &previous_tokens, "midi-diff");
            buffer.insert(&mut buffer.end_iter(), "\n");
        } else {
            buffer.insert(&mut buffer.end_iter(), &format!("{prefix}{}\n", current_tokens.join(" ")));
        }

        state.borrow_mut().last_midi_msg = Some(data);
    }
}
