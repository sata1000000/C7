//! Trigger extras and utilities over MIDI CC.

/*
Structurally identical to `sysex_tricks.rs`, but with different section content objectives.
If you change ANY structural code in here, please change `sysex_tricks.rs` to use the new standard too.
*/

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{self, Align, Button, Grid, Orientation, ToggleButton};
use serde_json::Value;

use crate::ui::base_module::{BaseModule, StatusFn};
use c7_core::device_config::DeviceConfig;
use c7_core::midi::{is_valid_port, run_midi_output, send_midi_cc};
use c7_core::sysex::pattern_slot_label;
use c7_core::utils::{JsonPath, as_array_or, as_array_or_die, as_string_or, as_u64_or, as_u64_or_die};

// ── Constants & types ──────────────────────────────────────────────────────────────────────────────────────

/// CC for the "Sample Start" knob on ROM machines.
const MD_SAMPLE_START_CC: u8 = 20;

/// CC for the "Sample End" knob on ROM machines.
const MD_SAMPLE_END_CC: u8 = 21;

/// One track's mute or solo data.
///
/// Holds (track index, toggle button, channel, CC, engaged value, released value).
///
/// "Engaged" means muted on the mute card and soloed on the solo card.
type TrackToggleData = (usize, ToggleButton, u8, u8, u8, u8);

/// Feature screen providing miscellaneous extras and playful device utilities.
pub(crate) struct MidiTricksScreen {
    pub root: gtk4::Box,
    pub has_cards: bool,
}

// ── Module-level helpers ───────────────────────────────────────────────────────────────────────────────────

/// Resolves the selected MIDI port, or reports the standard error and returns `None`.
fn get_valid_port(get_midi: &Rc<dyn Fn() -> Option<String>>, update_status: &StatusFn) -> Option<String> {
    match get_midi() {
        Some(port_name) if is_valid_port(&port_name) => Some(port_name),
        _ => {
            update_status("Error: Select a valid MIDI port.");
            None
        }
    }
}

/// Sends the same mute or solo CC to every track and syncs button states without firing callbacks.
fn batch_track_toggle(
    track_data: &[TrackToggleData],
    is_engaged: bool,
    status_message: &str,
    get_midi: &Rc<dyn Fn() -> Option<String>>,
    update_status: &StatusFn,
    is_suppressing_toggle_cb: &Rc<Cell<bool>>,
) {
    let Some(port) = get_valid_port(get_midi, update_status) else {
        return;
    };
    let cc_msgs: Vec<(u8, u8, u8)> = track_data
        .iter()
        .map(|(_, _, ch, cc, engaged_val, released_val)| (*ch, *cc, if is_engaged { *engaged_val } else { *released_val }))
        .collect();
    for (ch, cc, val) in cc_msgs {
        send_midi_cc(&port, ch, cc, val);
    }
    // Update button states without triggering toggled callbacks.
    is_suppressing_toggle_cb.set(true);
    for (_, btn, _, _, _, _) in track_data {
        btn.set_active(is_engaged);
    }
    is_suppressing_toggle_cb.set(false);
    update_status(status_message);
}

impl MidiTricksScreen {
    /// Initializes the MIDI tricks screen.
    pub(crate) fn new(
        go_to_menu: impl Fn() + 'static,
        get_selected_midi: impl Fn() -> Option<String> + 'static,
        active_config: &str,
    ) -> Self {
        let mut base = BaseModule::new();
        let dc = Arc::new(DeviceConfig::find_by_path_or_die(active_config));
        let get_midi_rc: Rc<dyn Fn() -> Option<String>> = Rc::new(get_selected_midi);

        // Shared flag that suppresses mute- and solo-button toggled callbacks during batch operations.
        let is_suppressing_toggle_cb: Rc<Cell<bool>> = Rc::new(Cell::new(false));

        base.build_header("MIDI Tricks", go_to_menu);

        // The status line sits at the bottom (built at the end of `new()`), but every card's handlers below need to write to it now.
        // `deferred_status()` hands out an updater that does nothing at first.
        // It gets wired to the real label once `build_status_area()` has run.
        let (update_status, status_slot) = BaseModule::deferred_status();

        // ── Pattern Preview card ───────────────────────────────────────────────────────────────────────────

        if dc.has_gate("sysex_api.pattern.note_trigger") {
            let trigger_channel = as_u64_or_die(dc.json_get("sysex_api.pattern.note_trigger.channel")) as u8;
            let trigger_notes: Vec<u8> = as_array_or_die(dc.json_get("sysex_api.pattern.note_trigger.notes"))
                .iter()
                .map(|value| as_u64_or_die(Some(value)) as u8)
                .collect();

            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Pattern Preview").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label(format!(
                        "Play a bank A pattern once, then return to what was playing.\n\
                        This assumes your {} is using the factory default global map.",
                        dc.device_short
                    ))
                    .xalign(0.0)
                    .wrap(true)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let grid = Grid::builder().row_spacing(6).column_spacing(6).margin_top(4).build();

            // Builds pattern trigger buttons for the first bank.
            for (i, &note) in trigger_notes.iter().enumerate() {
                let label = pattern_slot_label(i);
                let btn = Button::with_label(&label);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let update_status = Arc::clone(&update_status);
                btn.connect_clicked(move |_| {
                    // Sends a MIDI Note On/Off sequence to trigger a pattern on the hardware.
                    let Some(port) = get_valid_port(&get_midi_rc, &update_status) else {
                        return;
                    };
                    update_status(&format!("Status: Previewing {label} (Note {note})"));
                    let note_on = vec![0x90 | trigger_channel, note, 100u8];
                    let note_off = vec![0x80 | trigger_channel, note, 0u8];
                    glib::spawn_future_local(async move {
                        let _ = run_midi_output(&port, move |midi_out| {
                            midi_out.midi(&note_on);
                            sleep(Duration::from_millis(100));
                            midi_out.midi(&note_off);
                            Ok(())
                        })
                        .await;
                    });
                });
                grid.attach(&btn, (i % 8) as i32, (i / 8) as i32, 1, 1);
            }
            section.append(&grid);
            base.append_card(&section);
        }

        // ── Sample Slicer card ─────────────────────────────────────────────────────────────────────────────

        // The start/end CCs below are the Machinedrum's own numbers.
        if dc.is_device("MD") {
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Sample Slicer").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label(format!(
                        "Select the tracks that share the same sample. \n\
                        The slicer divides the sample equally across tracks by setting Sample Start (CC {MD_SAMPLE_START_CC}) and Sample End (CC {MD_SAMPLE_END_CC}) on each track."
                    ))
                    .xalign(0.0)
                    .wrap(true)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let grid = Grid::builder().row_spacing(6).column_spacing(6).margin_top(4).build();

            // Slicer track toggle buttons, keyed by track id.
            let mut slicer_btns: Vec<(usize, ToggleButton)> = vec![];
            // Builds track selection toggles for the sample slicer.
            for track in as_array_or(dc.json_get("tracks"), &[]) {
                let track_id = as_u64_or(track.json_get("id"), 0) as usize;
                let trackname = as_string_or(track.json_get("trackname"), "").to_string();
                let fullname = as_string_or(track.json_get("fullname"), "").to_string();
                let btn = ToggleButton::with_label(&trackname);
                btn.set_tooltip_text(Some(&fullname));
                grid.attach(&btn, (track_id % 8) as i32, (track_id / 8) as i32, 1, 1);
                slicer_btns.push((track_id, btn));
            }
            let slicer_btns = Rc::new(slicer_btns);
            section.append(&grid);

            let send_btn = Button::with_label("Send Slices");
            send_btn.add_css_class("suggested-action");
            send_btn.set_halign(Align::Start);
            let dc = Arc::clone(&dc);
            let slicer_btns = Rc::clone(&slicer_btns);
            let get_midi_rc = Rc::clone(&get_midi_rc);
            let update_status = Arc::clone(&update_status);
            send_btn.connect_clicked(move |_| {
                // Calculates and transmits MIDI CC messages to distribute sample slices across tracks.
                let Some(port) = get_valid_port(&get_midi_rc, &update_status) else {
                    return;
                };
                let mut selected: Vec<&Value> = as_array_or(dc.json_get("tracks"), &[])
                    .iter()
                    .filter(|track| {
                        let track_id = as_u64_or(track.json_get("id"), 0) as usize;
                        slicer_btns
                            .iter()
                            .find(|(id, _)| *id == track_id)
                            .is_some_and(|(_, btn)| btn.is_active())
                    })
                    .collect();
                selected.sort_by_key(|track| as_u64_or(track.json_get("id"), 0));
                if selected.len() < 2 {
                    update_status("Status: Select at least 2 tracks to slice.");
                    return;
                }
                let num_selected = selected.len();
                // Calculate slice start and end points for each selected track to prepare for playback.
                let mut cc_msgs: Vec<(u8, u8, u8)> = vec![];
                for (i, track) in selected.iter().enumerate() {
                    let ch = as_u64_or(track.json_get("channel"), 0) as u8;
                    let offset = as_u64_or(track.json_get("offset"), 0) as u8;
                    cc_msgs.push((ch, MD_SAMPLE_START_CC + offset, ((i * 128) / num_selected) as u8));
                    cc_msgs.push((
                        ch,
                        MD_SAMPLE_END_CC + offset,
                        (((i + 1) * 128) / num_selected).saturating_sub(1) as u8,
                    ));
                }
                for (ch, cc, val) in cc_msgs {
                    send_midi_cc(&port, ch, cc, val);
                }
                let names: Vec<String> = selected
                    .iter()
                    .filter_map(|track| track.json_get("name").and_then(|value| value.as_str()).map(String::from))
                    .collect();
                update_status(&format!("Status: Sliced {num_selected} tracks: {}", names.join(", ")));
            });
            section.append(&send_btn);
            base.append_card(&section);
        }

        // ── Track Mute card ────────────────────────────────────────────────────────────────────────────────

        if dc.has_gate("tracks.0.mute") && !as_array_or(dc.json_get("tracks"), &[]).is_empty() {
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Track Mute").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label(
                        "Sends the mute CC on the track's channel. \n\
                        Button state reflects what was last sent. Not the hardware's current state.",
                    )
                    .xalign(0.0)
                    .wrap(true)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let grid = Grid::builder().row_spacing(6).column_spacing(6).margin_top(4).build();

            let mut mute_data: Vec<TrackToggleData> = vec![];
            // Builds track mute toggle buttons.
            for track in as_array_or(dc.json_get("tracks"), &[]) {
                let track_id = as_u64_or(track.json_get("id"), 0) as usize;
                let trackname = as_string_or(track.json_get("trackname"), "").to_string();
                let fullname = as_string_or(track.json_get("fullname"), "").to_string();
                let ch = as_u64_or(track.json_get("channel"), 0) as u8;
                let mute_cc = as_u64_or(track.json_get("mute.cc"), 0) as u8;
                let muted_val = as_u64_or(track.json_get("mute.muted_value"), 0) as u8;
                let unmuted_val = as_u64_or(track.json_get("mute.unmuted_value"), 1) as u8;
                let btn = ToggleButton::with_label(&trackname);
                btn.set_tooltip_text(Some(&fullname));
                grid.attach(&btn, (track_id % 8) as i32, (track_id / 8) as i32, 1, 1);
                mute_data.push((track_id, btn, ch, mute_cc, muted_val, unmuted_val));
            }

            // Connect individual toggled callbacks.
            for (_, btn, ch, mute_cc, muted_val, unmuted_val) in &mute_data {
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let update_status = Arc::clone(&update_status);
                let is_suppressing_toggle_cb = Rc::clone(&is_suppressing_toggle_cb);
                let btn_name = btn.label().map(|label| label.to_string()).unwrap_or_default();
                let ch = *ch;
                let mute_cc = *mute_cc;
                let muted_val = *muted_val;
                let unmuted_val = *unmuted_val;
                btn.connect_toggled(move |toggled_btn| {
                    // Handles per-track mute toggling by sending the appropriate MIDI CC.
                    if is_suppressing_toggle_cb.get() {
                        return;
                    }
                    // Active button = track is muted.
                    // Inactive button = track is playing (unmuted).
                    let muted = toggled_btn.is_active();
                    let val = if muted { muted_val } else { unmuted_val };
                    let Some(port) = get_valid_port(&get_midi_rc, &update_status) else {
                        return;
                    };
                    send_midi_cc(&port, ch, mute_cc, val);
                    update_status(&format!("Status: {} {btn_name}", if muted { "Muted" } else { "Unmuted" }));
                });
            }
            section.append(&grid);

            let util_row = gtk4::Box::new(Orientation::Horizontal, 8);
            util_row.set_margin_top(4);

            let mute_data_rc: Rc<Vec<TrackToggleData>> = Rc::new(mute_data);

            let mute_all_btn = Button::with_label("Mute All");
            mute_all_btn.add_css_class("destructive-action");
            {
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let update_status = Arc::clone(&update_status);
                let is_suppressing_toggle_cb = Rc::clone(&is_suppressing_toggle_cb);
                let mute_data_rc = Rc::clone(&mute_data_rc);
                mute_all_btn.connect_clicked(move |_| {
                    // Batch mutes all tracks by iterating through the track configuration.
                    batch_track_toggle(
                        &mute_data_rc,
                        true,
                        "Status: Muted all tracks.",
                        &get_midi_rc,
                        &update_status,
                        &is_suppressing_toggle_cb,
                    );
                });
            }

            let unmute_all_btn = Button::with_label("Unmute All");
            unmute_all_btn.add_css_class("suggested-action");
            {
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let update_status = Arc::clone(&update_status);
                let is_suppressing_toggle_cb = Rc::clone(&is_suppressing_toggle_cb);
                let mute_data_rc = Rc::clone(&mute_data_rc);
                unmute_all_btn.connect_clicked(move |_| {
                    // Batch unmutes all tracks by iterating through the track configuration.
                    batch_track_toggle(
                        &mute_data_rc,
                        false,
                        "Status: Unmuted all tracks.",
                        &get_midi_rc,
                        &update_status,
                        &is_suppressing_toggle_cb,
                    );
                });
            }

            util_row.append(&mute_all_btn);
            util_row.append(&unmute_all_btn);
            section.append(&util_row);
            base.append_card(&section);
        }

        // ── Track Solo card ────────────────────────────────────────────────────────────────────────────────

        if dc.has_gate("tracks.0.solo") && !as_array_or(dc.json_get("tracks"), &[]).is_empty() {
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Track Solo").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label(
                        "Sends the solo CC on the track's channel. \n\
                        Button state reflects what was last sent. Not the hardware's current state.",
                    )
                    .xalign(0.0)
                    .wrap(true)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let grid = Grid::builder().row_spacing(6).column_spacing(6).margin_top(4).build();

            let mut solo_data: Vec<TrackToggleData> = vec![];
            // Builds track solo toggle buttons.
            for track in as_array_or(dc.json_get("tracks"), &[]) {
                let track_id = as_u64_or(track.json_get("id"), 0) as usize;
                let trackname = as_string_or(track.json_get("trackname"), "").to_string();
                let fullname = as_string_or(track.json_get("fullname"), "").to_string();
                let ch = as_u64_or(track.json_get("channel"), 0) as u8;
                let solo_cc = as_u64_or(track.json_get("solo.cc"), 0) as u8;
                let soloed_val = as_u64_or(track.json_get("solo.soloed_value"), 1) as u8;
                let unsoloed_val = as_u64_or(track.json_get("solo.unsoloed_value"), 0) as u8;
                let btn = ToggleButton::with_label(&trackname);
                btn.set_tooltip_text(Some(&fullname));
                grid.attach(&btn, (track_id % 8) as i32, (track_id / 8) as i32, 1, 1);
                solo_data.push((track_id, btn, ch, solo_cc, soloed_val, unsoloed_val));
            }

            // Connect individual toggled callbacks.
            for (_, btn, ch, solo_cc, soloed_val, unsoloed_val) in &solo_data {
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let update_status = Arc::clone(&update_status);
                let is_suppressing_toggle_cb = Rc::clone(&is_suppressing_toggle_cb);
                let btn_name = btn.label().map(|label| label.to_string()).unwrap_or_default();
                let ch = *ch;
                let solo_cc = *solo_cc;
                let soloed_val = *soloed_val;
                let unsoloed_val = *unsoloed_val;
                btn.connect_toggled(move |toggled_btn| {
                    // Handles per-track solo toggling by sending the appropriate MIDI CC.
                    if is_suppressing_toggle_cb.get() {
                        return;
                    }
                    // Active button = track is soloed.
                    // Inactive button = track isn't soloed.
                    //
                    // Several tracks can be soloed at once: each track has its own solo flag, not one CC naming a single soloed track.
                    let soloed = toggled_btn.is_active();
                    let val = if soloed { soloed_val } else { unsoloed_val };
                    let Some(port) = get_valid_port(&get_midi_rc, &update_status) else {
                        return;
                    };
                    send_midi_cc(&port, ch, solo_cc, val);
                    update_status(&format!("Status: {} {btn_name}", if soloed { "Soloed" } else { "Unsoloed" }));
                });
            }
            section.append(&grid);

            let util_row = gtk4::Box::new(Orientation::Horizontal, 8);
            util_row.set_margin_top(4);

            let solo_data_rc: Rc<Vec<TrackToggleData>> = Rc::new(solo_data);

            // No "Solo All" counterpart: soloing every track sounds identical to soloing none.
            let unsolo_all_btn = Button::with_label("Unsolo All");
            unsolo_all_btn.add_css_class("suggested-action");
            {
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let update_status = Arc::clone(&update_status);
                let is_suppressing_toggle_cb = Rc::clone(&is_suppressing_toggle_cb);
                let solo_data_rc = Rc::clone(&solo_data_rc);
                unsolo_all_btn.connect_clicked(move |_| {
                    // Batch unsolos all tracks by iterating through the track configuration.
                    batch_track_toggle(
                        &solo_data_rc,
                        false,
                        "Status: Unsoloed all tracks.",
                        &get_midi_rc,
                        &update_status,
                        &is_suppressing_toggle_cb,
                    );
                });
            }

            util_row.append(&unsolo_all_btn);
            section.append(&util_row);
            base.append_card(&section);
        }

        // ── Status area (appended last so it appears at the bottom) ────────────────────────────────────────

        // Arming the deferred slot connects the updater that every card captured above to this freshly built label.
        base.build_status_area(false);
        *status_slot.lock().unwrap() = Some(base.status_updater());
        update_status("Status: Ready.");

        let has_cards = base.has_cards();
        Self {
            root: base.root,
            has_cards,
        }
    }

    /// Returns the root widget for embedding in the navigation stack.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
