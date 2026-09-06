//! Toggle device settings over SysEx.
//!
//! Cards for live BPM, master tune, clock sync, machine assign, and track routing, each gated on the device JSON declaring its command.

/*
Structurally identical to `midi_tricks.rs`, but with different section content objectives.
If you change ANY structural code in here, please change `midi_tricks.rs` to use the new standard too.
*/

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use gtk4::{self, Align, Button, Grid, Orientation, Separator};
use serde_json::Value;

use crate::ui::base_module::{BaseModule, StatusFn, listen};
use crate::ui::widgets::{ChooseOnePill, CustomDropdown, CustomTextbox, NumberSpinner};
use c7_core::bpm_detector::BpmDetector;
use c7_core::device_config::{DeviceConfig, get_base_channel};
use c7_core::global::{GlobalParamField, collect_global_scalar_fields, patch_global_byte, request_global};
use c7_core::midi::{find_input_port, is_valid_port, run_midi_output, run_midi_session};
use c7_core::sysex::{
    ELEKTRON_PAYLOAD_START, build_elektron_sysex, build_machine_assignment_payload, decode_7bit, decode_rle7, encode_7bit, encode_rle7,
    filter_elektron_name, patch_global_checksum_14bit, pattern_slot_label, rebuild_rle7_dump, request_status_param, request_tempo,
    update_elektron_checksum,
};
use c7_core::utils::{JsonPath, as_array_or_die, as_bool_or_die, as_f64_or_die, as_string_or_die, as_string_vec_or_empty, as_u64_or_die};

/// Return type of `group_machines()`.
///
/// Holds the ordered family names, then a map from family to (slot, machine ID, display label).
type GroupedMachines = (Vec<String>, HashMap<String, Vec<(u8, String, String)>>);
/// Mutable shared map from machine family name to a list of machines.
///
/// Each entry holds (slot, machine ID, display label).
type MachineByFamily = Rc<RefCell<HashMap<String, Vec<(u8, String, String)>>>>;

// ── Immutable device config parsed once at construction ────────────────────────────────────────────────────

/// One row of the "Clock & Control Sync" card: a single bit in the Global's sync-flags byte.
#[derive(Clone)]
struct SyncFlagDef {
    bit: u8,
    is_inverted: bool,
    row_label: String,
    off_label: String,
    on_label: String,
}

/// Device configuration parsed once from the JSON, then shared (as `Arc<SysexTricksConfig>`) into every card.
#[derive(Clone)]
struct SysexTricksConfig {
    status_params: Vec<(u8, String, f64, f64)>,
    routing_outputs: Vec<(String, u8)>,
    routing_inputs: Vec<(String, u8)>,
    init_scopes: Vec<String>,
    machines: Vec<(u8, String)>,
    machine_order: HashMap<String, Vec<String>>,
    machine_family_order: Vec<String>,
    sync_flags: Vec<SyncFlagDef>,
}

impl SysexTricksConfig {
    /// Parses the device JSON into typed fields once, so cards and threads read `cfg.field` instead of re-walking the raw JSON.
    fn from_dc(dc: &DeviceConfig) -> Self {
        let machine_id_and_name = |machine: &Value| -> (u8, String) {
            let id = as_u64_or_die(machine.json_get("id")) as u8;
            let name = as_string_or_die(machine.json_get("name")).to_string();
            (id, name)
        };

        let machines: Vec<(u8, String)> = dc
            .json_get("sysex_api.machines.base")
            .and_then(|value| value.as_array())
            .map(|array| array.iter().map(machine_id_and_name).collect())
            .unwrap_or_default();

        let machine_order_raw = dc.json_get("sysex_api.machines.machine_order").and_then(|value| value.as_object());
        let mut machine_order: HashMap<String, Vec<String>> = HashMap::new();
        let mut machine_family_order = Vec::new();
        if let Some(obj) = machine_order_raw {
            for (family_key, value) in obj {
                machine_family_order.push(family_key.clone());
                if let Some(array) = value.as_array() {
                    machine_order.insert(
                        family_key.clone(),
                        array
                            .iter()
                            .filter_map(|value| value.as_str().map(std::string::ToString::to_string))
                            .collect(),
                    );
                }
            }
        }

        let status_params = dc
            .json_get("sysex_api.status.params")
            .and_then(|value| value.as_object())
            .map(|params| {
                params
                    .values()
                    .map(|param| {
                        (
                            as_u64_or_die(param.json_get("cmd")) as u8,
                            as_string_or_die(param.json_get("fullname")).to_string(),
                            as_f64_or_die(param.json_get("min")),
                            as_f64_or_die(param.json_get("max")),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let raw_outputs = dc.json_get("sysex_api.track_routing.outputs").and_then(|value| value.as_array());
        let routing_outputs: Vec<(String, u8)> = raw_outputs
            .map(|array| {
                if array.first().and_then(|value| value.as_object()).is_some() {
                    array
                        .iter()
                        .map(|output| {
                            (
                                as_string_or_die(output.json_get("label")).to_string(),
                                as_u64_or_die(output.json_get("value")) as u8,
                            )
                        })
                        .collect()
                } else {
                    array
                        .iter()
                        .enumerate()
                        .map(|(i, value)| (as_string_or_die(Some(value)).to_string(), i as u8))
                        .collect()
                }
            })
            .unwrap_or_default();

        let routing_inputs: Vec<(String, u8)> = dc
            .json_get("sysex_api.track_routing.inputs")
            .and_then(|value| value.as_array())
            .map(|array| {
                array
                    .iter()
                    .map(|input| {
                        (
                            as_string_or_die(input.json_get("label")).to_string(),
                            as_u64_or_die(input.json_get("value")) as u8,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let init_scopes = as_string_vec_or_empty(dc.json_get("sysex_api.assign_machine.init_scopes"));

        let sync_flags: Vec<SyncFlagDef> = dc
            .json_get("sysex_api.global.sync_flags")
            .and_then(|value| value.as_array())
            .map(|array| {
                array
                    .iter()
                    .map(|flag| SyncFlagDef {
                        bit: as_u64_or_die(flag.json_get("bit")) as u8,
                        is_inverted: as_bool_or_die(flag.json_get("inverted")),
                        row_label: as_string_or_die(flag.json_get("label")).to_string(),
                        off_label: as_string_or_die(flag.json_get("off_label")).to_string(),
                        on_label: as_string_or_die(flag.json_get("on_label")).to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        SysexTricksConfig {
            status_params,
            routing_outputs,
            routing_inputs,
            init_scopes,
            machines,
            machine_order,
            machine_family_order,
            sync_flags,
        }
    }
}

// ── Pure helpers (no GTK) ──────────────────────────────────────────────────────────────────────────────────

/// Shared error string for all `request_global()` timeouts, so the wording stays consistent.
const NO_RESPONSE: &str = "No Response. Check Base Channel and Global Slot.";

/// One Global Parameters row's widget: a dropdown for enum fields, an On/Off pill for unlabeled 0-1 fields, a spinner otherwise.
enum GlobalParamWidget {
    Spinner(NumberSpinner),
    Dropdown(CustomDropdown),
    Pill(ChooseOnePill),
}

/// Groups a flat `(id, name)` machine list into `(families, by_family)`.
fn group_machines(list: &[(u8, String)]) -> GroupedMachines {
    let mut families: Vec<String> = Vec::new();
    let mut by_family: HashMap<String, Vec<(u8, String, String)>> = HashMap::new();
    for (id, name) in list {
        let (family, variant) = if let Some(pos) = name.rfind('-') {
            (name[..pos].to_string(), name[pos + 1..].to_string())
        } else {
            (name.clone(), name.clone())
        };

        if !by_family.contains_key(&family) {
            families.push(family.clone());
        }
        by_family.entry(family).or_default().push((*id, variant, name.clone()));
    }
    (families, by_family)
}

/// Feature screen for SysEx utilities: BPM, mode, routing, note-map, kit-name, and more.
pub(crate) struct SysexTricksScreen {
    pub root: gtk4::Box,
    pub has_cards: bool,
}

// ── Module-level helpers ───────────────────────────────────────────────────────────────────────────────────

/// Transmits a SysEx message to the hardware using the currently selected MIDI port.
async fn send_sysex_message(get_midi: &dyn Fn() -> Option<String>, prod: u8, ch: u8, payload: &[u8], update_status: &StatusFn) -> bool {
    let port = match get_midi() {
        Some(port_name) if is_valid_port(&port_name) => port_name,
        _ => {
            update_status("Error: Select a valid MIDI port.");
            return false;
        }
    };
    let msg = build_elektron_sysex(prod, ch, payload);
    match run_midi_output(&port, move |midi_out| {
        midi_out.sysex(&msg);
        Ok::<(), String>(())
    })
    .await
    {
        Ok(()) => true,
        Err(e) => {
            update_status(&format!("Error: {e}"));
            false
        }
    }
}

/// Resolves both input and output MIDI port names for bidirectional communication.
fn get_ports(get_midi: &dyn Fn() -> Option<String>, update_status: &StatusFn) -> Option<(String, String)> {
    let port = match get_midi() {
        Some(port_name) if is_valid_port(&port_name) => port_name,
        _ => {
            update_status("Error: Select a valid MIDI port.");
            return None;
        }
    };
    if let Some(in_port) = find_input_port(&port) {
        Some((port, in_port))
    } else {
        update_status("Error: could not find a matching MIDI input port.");
        None
    }
}

/// Updates each pill in the "Clock & Control Sync" card based on its bit in the sync byte.
fn apply_sync_flags(pill_data: &[(ChooseOnePill, u8, bool)], flags: u8) {
    for (pill, bit, is_inverted) in pill_data {
        let is_on = if *is_inverted { (flags & bit) == 0 } else { (flags & bit) != 0 };
        pill.set_active(usize::from(is_on));
    }
}

impl SysexTricksScreen {
    /// Initializes the SysEx tricks screen.
    pub(crate) fn new(
        go_to_menu: impl Fn() + 'static,
        get_selected_midi: impl Fn() -> Option<String> + 'static,
        active_config: &str,
    ) -> Self {
        let mut base = BaseModule::new();
        let dc = Arc::new(DeviceConfig::find_by_path_or_die(active_config));
        let cfg = Arc::new(SysexTricksConfig::from_dc(&dc));
        let get_midi_rc: Rc<dyn Fn() -> Option<String>> = Rc::new(get_selected_midi);

        base.build_header("SysEx Tricks", go_to_menu);

        // The status line sits at the bottom (built at the end of `new()`), but every card's handlers below need to write to it now.
        // `deferred_status()` hands out an updater that does nothing at first.
        // It gets wired to the real label once `build_status_area()` has run.
        let (update_status, status_slot) = BaseModule::deferred_status();

        // ── Shared top row ─────────────────────────────────────────────────────────────────────────────────

        let needs_fetch = dc.has_gate("sysex_layout.global") || dc.has_gate("sysex_api.master_tune");
        let (fetch_btn, shared_slot_spin) = {
            let row = gtk4::Box::new(Orientation::Horizontal, 8);
            let (fetch_button, sss) = if needs_fetch {
                let btn = Button::with_label("Fetch Current States");

                btn.set_tooltip_text(Some(if dc.has_gate("sysex_layout.global.fields.payload.encoding") {
                    "Reads the Global and updates the Master Tune spinner"
                } else {
                    "Reads the Global and updates All Global Parameters, Clock & Control Sync, and BPM spinner"
                }));

                row.append(&btn);
                let shared_slot_spin = NumberSpinner::new(1.0, 1.0, 8.0, 1.0, "", 0);
                shared_slot_spin.set_tooltip_text("Global slot to read when fetching all states");
                row.append(
                    &gtk4::Label::builder()
                        .label("Global Slot:")
                        .css_classes(["caption"])
                        .xalign(0.0)
                        .build(),
                );
                row.append(shared_slot_spin.widget());
                (Some(btn), Some(shared_slot_spin))
            } else {
                (None, None)
            };

            if needs_fetch {
                base.root.append(&row);
                base.root.append(&Separator::new(Orientation::Horizontal));
            }
            (fetch_button, sss)
        };

        // Field list + fetch channel shared with the All Global Parameters card below.
        let global_param_fields = Rc::new(collect_global_scalar_fields(&dc));
        let (all_params_fetch_tx, all_params_fetch_rx) = async_channel::unbounded::<Vec<u8>>();

        // ── All Global Parameters card (generic, JSON-driven) ──────────────────────────────────────────────

        // One row per single-value Global field, parallel to the per-field cards below. Fetch comes from `all_params_fetch_rx`.
        if dc.has_gate("sysex_layout.global") || dc.has_gate("sysex_api.master_tune") {
            let scalar_fields = (*global_param_fields).clone();
            if !scalar_fields.is_empty() {
                let section = gtk4::Box::new(Orientation::Vertical, 8);
                section.set_margin_top(5);
                section.set_margin_bottom(5);

                let title = gtk4::Label::builder().label("All Global Parameters").xalign(0.0).build();
                title.add_css_class("title-3");
                section.append(&title);

                section.append(
                    &gtk4::Label::builder()
                        .label(
                            "One row per single-value Global setting. \
                            Per-track and table settings aren't shown here.",
                        )
                        .xalign(0.0)
                        .css_classes(["dim-label"])
                        .wrap(true)
                        .build(),
                );

                let rows_box = gtk4::Box::new(Orientation::Vertical, 4);
                rows_box.set_margin_top(4);
                let mut rows: Vec<(GlobalParamField, GlobalParamWidget)> = Vec::new();
                for field in scalar_fields {
                    let row = gtk4::Box::new(Orientation::Horizontal, 10);
                    row.append(&gtk4::Label::builder().label(&field.fullname).xalign(0.0).width_request(220).build());
                    let widget = match &field.labels {
                        Some(labels) if !labels.is_empty() => {
                            let dropdown = CustomDropdown::new(Some(0));
                            for (l, _) in labels {
                                dropdown.append_text(l);
                            }
                            dropdown.set_active(Some(0));
                            row.append(&*dropdown);
                            GlobalParamWidget::Dropdown(dropdown)
                        }
                        _ if field.min == 0 && field.max == 1 => {
                            let pill = ChooseOnePill::new(&["Off", "On"], 0);
                            row.append(&pill.root);
                            GlobalParamWidget::Pill(pill)
                        }
                        _ => {
                            let lo = f64::from(field.min + field.display_offset);
                            let hi = f64::from(field.max + field.display_offset);
                            let spin = NumberSpinner::new(lo, lo, hi, 1.0, "", 0);
                            row.append(spin.widget());
                            GlobalParamWidget::Spinner(spin)
                        }
                    };
                    rows_box.append(&row);
                    rows.push((field, widget));
                }
                section.append(&rows_box);
                let rows = Rc::new(rows);

                let btn_row = gtk4::Box::new(Orientation::Horizontal, 8);
                btn_row.set_margin_top(8);
                let write_all_btn = Button::builder()
                    .label("Write All Params")
                    .css_classes(["suggested-action"])
                    .build();
                btn_row.append(&write_all_btn);
                section.append(&btn_row);

                let is_busy = Arc::new(AtomicBool::new(false));

                // Populated by the shared Fetch Current States session, not its own fetch button.
                {
                    let rows_c = Rc::clone(&rows);
                    listen(all_params_fetch_rx, move |vals| {
                        for (i, (field, widget)) in rows_c.iter().enumerate() {
                            let Some(field_val) = vals.get(i).copied() else {
                                continue;
                            };
                            match widget {
                                GlobalParamWidget::Spinner(spin) => spin.set_value(f64::from(field_val as i8 + field.display_offset)),
                                GlobalParamWidget::Dropdown(dropdown) => {
                                    if let Some(idx) = field
                                        .labels
                                        .as_ref()
                                        .and_then(|labels| labels.iter().position(|(_, val)| *val == field_val))
                                    {
                                        dropdown.set_active(Some(idx as u32));
                                    }
                                }
                                GlobalParamWidget::Pill(pill) => pill.set_active(usize::from(field_val != 0)),
                            }
                        }
                        glib::ControlFlow::Continue
                    });
                }

                // Reads every widget value on the main thread (not Send), then applies them in one session.
                {
                    let dc = Arc::clone(&dc);
                    let update_status = Arc::clone(&update_status);
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let shared_slot_spin_c = shared_slot_spin.clone();
                    let is_busy = Arc::clone(&is_busy);
                    let btn = write_all_btn.clone();
                    let rows_c = Rc::clone(&rows);
                    write_all_btn.connect_clicked(move |_| {
                        if is_busy.swap(true, Ordering::Relaxed) {
                            return;
                        }
                        let Some((out_port, _in_port)) = get_ports(&*get_midi_rc, &update_status) else {
                            is_busy.store(false, Ordering::Relaxed);
                            return;
                        };
                        let slot = shared_slot_spin_c.as_ref().unwrap().value() as u8 - 1;
                        let ch = get_base_channel();
                        btn.set_sensitive(false);
                        update_status(&format!("Status: Writing Global {}...", slot + 1));

                        let updates: Vec<(GlobalParamField, u8)> = rows_c
                            .iter()
                            .map(|(field, widget)| {
                                let val = match widget {
                                    GlobalParamWidget::Spinner(spin) => (spin.value() as i8 - field.display_offset) as u8,
                                    GlobalParamWidget::Dropdown(dropdown) => {
                                        let idx = dropdown.active().unwrap_or(0) as usize;
                                        field.labels.as_ref().and_then(|labels| labels.get(idx)).map_or(0, |(_, val)| *val)
                                    }
                                    GlobalParamWidget::Pill(pill) => u8::from(pill.active() == 1),
                                };
                                (field.clone(), val)
                            })
                            .collect();

                        let dc = Arc::clone(&dc);
                        let is_busy_c = Arc::clone(&is_busy);
                        let btn_c = btn.clone();
                        let update_status = Arc::clone(&update_status);
                        glib::spawn_future_local(async move {
                            let result = run_midi_session(&out_port, move |midi_in, midi_out| -> Result<String, String> {
                                let Some(mut raw) = request_global(midi_in, midi_out, ch, &dc, slot) else {
                                    return Err(NO_RESPONSE.to_string());
                                };
                                if dc.is_device("MnM") {
                                    let version = raw[ELEKTRON_PAYLOAD_START];
                                    let packed_start = if version == 64 { 11 } else { 10 };
                                    let packed = &raw[packed_start..raw.len() - 5];
                                    let unpacked = decode_7bit(packed);
                                    let mut decoded = decode_rle7(&unpacked, None);
                                    // Skipping out-of-range fields keeps the rebuilt dump the same length the device sent.
                                    let decoded_len = decoded.len();
                                    for (field, value) in updates.iter().filter(|(field, _)| field.offset < decoded_len) {
                                        let current_byte = decoded[field.offset];
                                        decoded[field.offset] = field.masked_byte(current_byte, *value);
                                    }
                                    let new_sysex = rebuild_rle7_dump(&raw, &decoded, packed_start);
                                    midi_out.sysex(&new_sysex);
                                } else {
                                    let payload_offset = dc.layout_offset_or_die("global", "payload") as usize;
                                    for (field, value) in &updates {
                                        let byte_offset = payload_offset + field.offset;
                                        let new_byte = field.masked_byte(raw[byte_offset], *value);
                                        patch_global_byte(&mut raw, byte_offset, new_byte, midi_out);
                                    }
                                }
                                Ok(format!("Status: Global {} written.", slot + 1))
                            })
                            .await;
                            is_busy_c.store(false, Ordering::Relaxed);
                            let msg = match result {
                                Ok(msg) => msg,
                                Err(e) => format!("Error: {e}"),
                            };
                            update_status(&msg);
                            btn_c.set_sensitive(true);
                        });
                    });
                }

                base.append_card(&section);
            }
        }

        // ── BPM card ───────────────────────────────────────────────────────────────────────────────────────

        let mut bpm_spin_opt: Option<NumberSpinner> = None;
        // The whole card exists to send the live-BPM command, so a device without one has nothing to show here.
        if dc.has_gate("sysex_api.bpm_live_cmd") {
            let bpm_min = as_f64_or_die(dc.json_get("sysex_api.global.bpm_min"));
            let bpm_max = as_f64_or_die(dc.json_get("sysex_api.global.bpm_max"));
            let bpm_spin = NumberSpinner::new(120.0, bpm_min, bpm_max, 1.0, "", 0);
            bpm_spin_opt = Some(bpm_spin.clone());
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("BPM Control").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            let live_cmd = as_u64_or_die(dc.json_get("sysex_api.bpm_live_cmd")) as u8;
            let bpm_persists = dc.layout_field("global", "bpm_msb").is_some();
            let mut description = "Set BPM live (takes effect immediately, no Global needed).".to_string();
            if bpm_persists {
                description += "\nOr patch the Global for a change that survives power-off.";
            }
            section.append(
                &gtk4::Label::builder()
                    .label(&description)
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let row = gtk4::Box::new(Orientation::Horizontal, 12);
            row.set_margin_top(4);

            let col_bpm = gtk4::Box::new(Orientation::Vertical, 4);
            col_bpm.append(&gtk4::Label::builder().label("BPM").xalign(0.0).css_classes(["caption"]).build());
            col_bpm.append(bpm_spin.widget());
            row.append(&col_bpm);

            section.append(&row);

            let btn_row = gtk4::Box::new(Orientation::Horizontal, 8);
            btn_row.set_margin_top(4);

            // Set Live button
            let live_btn = Button::builder().label("Set Live").css_classes(["suggested-action"]).build();
            live_btn.set_tooltip_text(Some("Takes effect immediately, not saved to Global"));
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                let bpm_spin_c = bpm_spin.clone();
                live_btn.connect_clicked(move |_| {
                    let bpm = bpm_spin_c.value() as u32;
                    let mult = as_f64_or_die(dc.json_get("sysex_api.global.bpm_multiplier")) as u32;
                    let val = bpm * mult;
                    let payload = vec![(val >> 7) as u8 & 0x7F, (val & 0x7F) as u8];
                    let mut full = vec![live_cmd];
                    full.extend_from_slice(&payload);
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(&*get_midi_rc, dc.prod, get_base_channel(), &full, &update_status_c).await {
                            update_status_c(&format!("Status: BPM set to {bpm} (live)."));
                        }
                    });
                });
            }
            btn_row.append(&live_btn);

            // Set Persistent button (only on devices whose Global actually stores BPM)
            if bpm_persists {
                let is_bpm_active = Arc::new(AtomicBool::new(false));
                let global_btn = Button::with_label("Set Persistent  (Global round-trip)");
                global_btn.set_tooltip_text(Some(
                    "Reads the Global, patches the tempo bytes, writes it back. Change survives power-off.",
                ));
                {
                    let update_status = Arc::clone(&update_status);
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let bpm_spin_c = bpm_spin.clone();
                    let shared_slot_spin_c = shared_slot_spin.clone();
                    let is_active = Arc::clone(&is_bpm_active);
                    let btn = global_btn.clone();
                    global_btn.connect_clicked(move |_| {
                        if is_active.swap(true, Ordering::Relaxed) {
                            return;
                        }
                        let Some((out_port, _in_port)) = get_ports(&*get_midi_rc, &update_status) else {
                            is_active.store(false, Ordering::Relaxed);
                            return;
                        };
                        let bpm = bpm_spin_c.value() as u32;
                        let slot = shared_slot_spin_c.as_ref().unwrap().value() as u8 - 1;
                        let ch = get_base_channel();
                        btn.set_sensitive(false);
                        update_status(&format!("Status: Requesting Global {}...", slot + 1));
                        let dc = Arc::clone(&dc);
                        let is_active_c = Arc::clone(&is_active);
                        let btn_c = btn.clone();
                        let update_status = Arc::clone(&update_status);
                        glib::spawn_future_local(async move {
                            let result = run_midi_session(&out_port, move |midi_in, midi_out| -> Result<String, String> {
                                match request_global(midi_in, midi_out, ch, &dc, slot) {
                                    None => Err(NO_RESPONSE.to_string()),
                                    Some(mut raw) => {
                                        let payload_offset = dc.layout_offset_or_die("global", "payload") as isize;
                                        let msb_offset = dc.layout_offset_or_die("global", "bpm_msb") as isize;
                                        let lsb_offset = dc.layout_offset_or_die("global", "bpm_lsb") as isize;
                                        let msb_idx = (payload_offset + msb_offset) as usize;
                                        let lsb_idx = (payload_offset + lsb_offset) as usize;
                                        let val = bpm * as_f64_or_die(dc.json_get("sysex_api.global.bpm_multiplier")) as u32;
                                        let (nb1, nb2) = ((val >> 7) as u8 & 0x7F, (val & 0x7F) as u8);
                                        let old_sum = i32::from(raw[msb_idx] + raw[lsb_idx]);
                                        patch_global_checksum_14bit(&mut raw, old_sum, i32::from(nb1 + nb2));
                                        raw[msb_idx] = nb1;
                                        raw[lsb_idx] = nb2;
                                        midi_out.sysex(&raw);
                                        Ok(format!("Status: BPM set to {bpm} (persistent)."))
                                    }
                                }
                            })
                            .await;
                            let msg = match result {
                                Ok(result) => result,
                                Err(e) => format!("Error: {e}"),
                            };
                            update_status(&msg);
                            btn_c.set_sensitive(true);
                            is_active_c.store(false, Ordering::Relaxed);
                        });
                    });
                }
                btn_row.append(&global_btn);
            }

            // Get BPM button (`has_bpm_query` only)
            if dc.has_gate("sysex_api.bpm_query_cmd") {
                let is_bpm_active = Arc::new(AtomicBool::new(false));
                let get_btn = Button::with_label("Get BPM");
                get_btn.set_tooltip_text(Some("Queries the current BPM from the device. Updates spinner immediately."));
                {
                    let update_status = Arc::clone(&update_status);
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let is_active = Arc::clone(&is_bpm_active);
                    let btn = get_btn.clone();
                    let bpm_spin_c = bpm_spin.clone();
                    get_btn.connect_clicked(move |_| {
                        if is_active.swap(true, Ordering::Relaxed) {
                            return;
                        }
                        let Some((out_port, _in_port)) = get_ports(&*get_midi_rc, &update_status) else {
                            is_active.store(false, Ordering::Relaxed);
                            return;
                        };
                        let ch = get_base_channel();
                        btn.set_sensitive(false);
                        update_status("Status: Querying BPM...");
                        let dc = Arc::clone(&dc);
                        let is_active_c = Arc::clone(&is_active);
                        let update_status = Arc::clone(&update_status);
                        let btn_c = btn.clone();
                        let bpm_spin_c2 = bpm_spin_c.clone();
                        glib::spawn_future_local(async move {
                            let result = run_midi_session(&out_port, move |midi_in, midi_out| -> Result<f64, String> {
                                match request_tempo(midi_in, midi_out, &dc, ch) {
                                    None => Err("No Response. Check Base Channel and MIDI port.".to_string()),
                                    Some(tick) => {
                                        let bpm_min = as_f64_or_die(dc.json_get("sysex_api.global.bpm_min")) as u32;
                                        let bpm_max = as_f64_or_die(dc.json_get("sysex_api.global.bpm_max")) as u32;
                                        let bpm_mult = as_f64_or_die(dc.json_get("sysex_api.global.bpm_multiplier")) as u32;
                                        let bpm = f64::from(bpm_min.max(bpm_max.min(tick / bpm_mult)));
                                        Ok(bpm)
                                    }
                                }
                            })
                            .await;
                            is_active_c.store(false, Ordering::Relaxed);
                            match result {
                                Ok(bpm) => {
                                    bpm_spin_c2.set_value(bpm);
                                    update_status(&format!("Status: BPM read: {}.", bpm as u32));
                                }
                                Err(e) => {
                                    update_status(&format!("Error: {e}"));
                                }
                            }
                            btn_c.set_sensitive(true);
                        });
                    });
                }
                btn_row.append(&get_btn);
            }

            section.append(&btn_row);
            base.append_card(&section);
        }

        let mut tune_spin_opt: Option<NumberSpinner> = None;
        // ── Master Tune card ───────────────────────────────────────────────────────────────────────────────

        if dc.has_gate("sysex_api.master_tune") {
            // The device stores tenths of Hz, so the layout's raw bounds scale down for display.
            let tune_multiplier = as_f64_or_die(dc.json_get("sysex_api.master_tune.multiplier"));
            let tune_min = as_f64_or_die(dc.json_get("sysex_layout.global.fields.payload_structure.base_frequency.min")) / tune_multiplier;
            let tune_max = as_f64_or_die(dc.json_get("sysex_layout.global.fields.payload_structure.base_frequency.max")) / tune_multiplier;
            let tune_default = as_f64_or_die(dc.json_get("sysex_api.master_tune.default_val"));
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Master Tune / Master Freq").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label("Read or write the master tuning frequency stored in the Global. Use Fetch Current States above to read the current value.")
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let col_hz = gtk4::Box::new(Orientation::Vertical, 4);
            col_hz.append(
                &gtk4::Label::builder()
                    .label("Frequency (Hz)")
                    .xalign(0.0)
                    .css_classes(["caption"])
                    .build(),
            );
            let tune_spin = NumberSpinner::new(tune_default, tune_min, tune_max, 0.1, "", 1);
            tune_spin_opt = Some(tune_spin.clone());
            col_hz.append(tune_spin.widget());
            let row = gtk4::Box::new(Orientation::Horizontal, 12);
            row.set_margin_top(4);
            row.append(&col_hz);
            section.append(&row);

            let is_tune_active = Arc::new(AtomicBool::new(false));
            let set_btn = Button::builder()
                .label("Set")
                .css_classes(["suggested-action"])
                .halign(Align::Start)
                .build();
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                let shared_slot_spin_c = shared_slot_spin.clone();
                let is_active = Arc::clone(&is_tune_active);
                let btn = set_btn.clone();
                set_btn.connect_clicked(move |_| {
                    if is_active.swap(true, Ordering::Relaxed) {
                        return;
                    }
                    let Some((out_port, _in_port)) = get_ports(&*get_midi_rc, &update_status) else {
                        is_active.store(false, Ordering::Relaxed);
                        return;
                    };
                    let hz = tune_spin.value();
                    let slot = shared_slot_spin_c.as_ref().unwrap().value() as u8 - 1;
                    let ch = get_base_channel();
                    btn.set_sensitive(false);
                    update_status(&format!("Status: Requesting Global {}...", slot + 1));
                    let dc = Arc::clone(&dc);
                    let is_active_c = Arc::clone(&is_active);
                    let update_status = Arc::clone(&update_status);
                    let btn_c = btn.clone();
                    glib::spawn_future_local(async move {
                        let result = run_midi_session(&out_port, move |midi_in, midi_out| -> Result<String, String> {
                            let Some(raw) = request_global(midi_in, midi_out, ch, &dc, slot) else {
                                return Err(NO_RESPONSE.to_string());
                            };
                            if !dc.has_gate("sysex_api.master_tune") {
                                return Err("Master Tune is not supported on this device.".to_string());
                            }
                            let freq_offset = dc.layout_offset_or_die("global", "base_frequency") as usize;
                            let version = raw[ELEKTRON_PAYLOAD_START];
                            let packed_start = if version == 64 { 11 } else { 10 };
                            let packed = &raw[packed_start..raw.len() - 5];
                            let unpacked = decode_7bit(packed);
                            if unpacked.is_empty() {
                                return Err("Failed to unpack Global data.".to_string());
                            }
                            let mut decoded = decode_rle7(&unpacked, None);
                            if decoded.len() < freq_offset + 4 {
                                decoded.resize(freq_offset + 4, 0);
                            }
                            let new_freq = (hz * as_f64_or_die(dc.json_get("sysex_api.master_tune.multiplier"))).round() as u32;
                            decoded[freq_offset..freq_offset + 4].copy_from_slice(&new_freq.to_be_bytes());
                            let rle_enc = encode_rle7(&decoded);
                            let encoded = encode_7bit(&rle_enc);
                            let mut new_syx: Vec<u8> = raw[..packed_start].to_vec();
                            new_syx.extend_from_slice(&encoded);
                            new_syx.extend_from_slice(&[0u8; 5]);
                            let last = new_syx.len() - 1;
                            new_syx[last] = 0xF7;
                            update_elektron_checksum(&mut new_syx);
                            midi_out.sysex(&new_syx);
                            Ok(format!("Status: Master Tune set to {hz:.1} Hz."))
                        })
                        .await;
                        let msg = match result {
                            Ok(result) => result,
                            Err(e) => format!("Error: {e}"),
                        };
                        update_status(&msg);
                        btn_c.set_sensitive(true);
                        is_active_c.store(false, Ordering::Relaxed);
                    });
                });
            }
            section.append(&set_btn);
            base.append_card(&section);
        }

        // Machinedrum's Sequencer Mode is a Global byte, covered by "All Global Parameters" below.
        // Monomachine's is a live status value instead, covered by the Status card.

        // ── Clock & Control Sync card (`sysex_layout.global` only) ─────────────────────────────────────────

        let clock_pill_data_opt: Option<Vec<(ChooseOnePill, u8, bool)>> =
            // The pills patch one byte of the received Global in place, which only lands while the payload is unencoded.
            if !dc.has_gate("sysex_layout.global.fields.payload.encoding") && !cfg.sync_flags.is_empty() {
                let section = gtk4::Box::new(Orientation::Vertical, 8);
                section.set_margin_top(5);
                section.set_margin_bottom(5);

                let title = gtk4::Label::builder().label("Clock & Control Sync").xalign(0.0).build();
                title.add_css_class("title-3");
                section.append(&title);

                section.append(
                    &gtk4::Label::builder()
                        .label(
                            "Configure MIDI clock and control routing in the Global. \n\
                            Each button reads the current Global, flips the relevant bit, and writes back. Other bits are preserved.",
                        )
                        .xalign(0.0)
                        .css_classes(["dim-label"])
                        .build(),
                );

                let grid = Grid::builder().row_spacing(6).column_spacing(8).margin_top(4).build();
                let is_clock_active = Arc::new(AtomicBool::new(false));

                // glib channel to update all clock pills at once (carries the whole sync byte).
                let (clock_tx, clock_rx) = async_channel::unbounded();

                let mut pill_data: Vec<(ChooseOnePill, u8, bool)> = Vec::new();
                for (row_idx, flag) in cfg.sync_flags.iter().enumerate() {
                    let pill = ChooseOnePill::new(&[&flag.off_label, &flag.on_label], 0);
                    let bit = flag.bit;
                    let is_inverted = flag.is_inverted;
                    let off_bit_val: u8 = if is_inverted { bit } else { 0 };
                    let on_bit_val: u8 = if is_inverted { 0 } else { bit };
                    let bit_vals = [off_bit_val, on_bit_val];
                    let off_msg = format!("Status: {} → {}.", flag.row_label, flag.off_label);
                    let on_msg = format!("Status: {} → {}.", flag.row_label, flag.on_label);
                    let status_msgs = [off_msg, on_msg];

                    let update_status = Arc::clone(&update_status);
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let shared_slot_spin_c = shared_slot_spin.clone();
                    let is_active = Arc::clone(&is_clock_active);
                    let all_pills_snapshot: Vec<ChooseOnePill> = pill_data.iter().map(|(p, _, _)| p.clone()).collect();
                    let pill_widget = pill.clone();
                    let tx = clock_tx.clone();

                    pill.connect_changed(move |i| {
                        let bit_val = bit_vals[i as usize];
                        let msg = status_msgs[i as usize].clone();
                        if is_active.swap(true, Ordering::Relaxed) {
                            return;
                        }
                        let Some((out_port, _in_port)) = get_ports(&*get_midi_rc, &update_status) else {
                            is_active.store(false, Ordering::Relaxed);
                            return;
                        };
                        let slot = shared_slot_spin_c.as_ref().unwrap().value() as u8 - 1;
                        let ch = get_base_channel();
                        // Disable all clock pills during update.
                        for pill in &all_pills_snapshot {
                            pill.root.set_sensitive(false);
                        }
                        pill_widget.root.set_sensitive(false);

                        update_status(&format!("Status: Requesting Global {}...", slot + 1));
                        let dc = Arc::clone(&dc);
                        let is_active_c = Arc::clone(&is_active);
                        let update_status = Arc::clone(&update_status);
                        let pill_c = pill_widget.clone();
                        let tx_c = tx.clone();
                        let snap_c = all_pills_snapshot.clone();
                        glib::spawn_future_local(async move {
                            let result = run_midi_session(&out_port, move |midi_in, midi_out| -> Result<(u8, String), String> {
                                let Some(mut raw) = request_global(midi_in, midi_out, ch, &dc, slot) else {
                                    return Err(NO_RESPONSE.to_string());
                                };
                                let payload_offset = dc.layout_offset_or_die("global", "payload") as isize;
                                let sync_offset = dc.layout_offset_or_die("global", "external_sync_flags") as isize;
                                let byte_offset = (payload_offset + sync_offset) as usize;
                                // Compute the new byte with the target bit set/cleared before patching.
                                let new_byte = (raw[byte_offset] & !bit) | bit_val;
                                patch_global_byte(&mut raw, byte_offset, new_byte, midi_out);
                                Ok((new_byte, msg))
                            })
                            .await;
                            is_active_c.store(false, Ordering::Relaxed);
                            match result {
                                Ok((new_byte, msg)) => {
                                    let _ = tx_c.try_send(new_byte);
                                    update_status(&msg);
                                }
                                Err(e) => {
                                    update_status(&format!("Error: {e}"));
                                }
                            }
                            pill_c.root.set_sensitive(true);
                            for pill in &snap_c {
                                pill.root.set_sensitive(true);
                            }
                        });
                    });

                    grid.attach(
                        &gtk4::Label::builder().label(format!("{}:", flag.row_label)).xalign(0.0).build(),
                        0,
                        row_idx as i32,
                        1,
                        1,
                    );
                    grid.attach(pill.widget(), 1, row_idx as i32, 1, 1);
                    pill_data.push((pill, bit, is_inverted));
                }

                // Attach clock channel receiver (captures `pill_data`).
                {
                    let pill_data_c = pill_data.clone();
                    listen(clock_rx, move |flags| {
                        apply_sync_flags(&pill_data_c, flags);
                        glib::ControlFlow::Continue
                    });
                }

                section.append(&grid);
                base.append_card(&section);
                Some(pill_data)
            } else {
                None
            };

        // ── Fetch All States wiring ────────────────────────────────────────────────────────────────────────

        if let Some(ref fetch_button) = fetch_btn {
            let (bpm_fetch_tx, bpm_fetch_rx) = async_channel::unbounded();
            let (tune_fetch_tx, tune_fetch_rx) = async_channel::unbounded();
            let (clock_fetch_tx, clock_fetch_rx) = async_channel::unbounded();

            if let Some(bpm_spin) = &bpm_spin_opt {
                let bpm_spin_c = bpm_spin.clone();
                listen(bpm_fetch_rx, move |value| {
                    bpm_spin_c.set_value(value);
                    glib::ControlFlow::Continue
                });
            }
            if let Some(tune_spin) = &tune_spin_opt {
                let tune_spin = tune_spin.clone();
                listen(tune_fetch_rx, move |value| {
                    tune_spin.set_value(value);
                    glib::ControlFlow::Continue
                });
            }
            if let Some(pill_data) = &clock_pill_data_opt {
                let pill_data = pill_data.clone();
                listen(clock_fetch_rx, move |flags| {
                    apply_sync_flags(&pill_data, flags);
                    glib::ControlFlow::Continue
                });
            }

            let is_active = Arc::new(AtomicBool::new(false));
            let btn = fetch_button.clone();
            let update_status = Arc::clone(&update_status);
            let get_midi_rc = Rc::clone(&get_midi_rc);
            let dc = Arc::clone(&dc);
            let shared_slot_spin_c = shared_slot_spin.clone();
            let global_param_fields = Rc::clone(&global_param_fields);
            fetch_button.connect_clicked(move |_| {
                if is_active.swap(true, Ordering::Relaxed) {
                    return;
                }
                let Some((out_port, _in_port)) = get_ports(&*get_midi_rc, &update_status) else {
                    is_active.store(false, Ordering::Relaxed);
                    return;
                };
                let slot = shared_slot_spin_c.as_ref().unwrap().value() as u8 - 1;
                let ch = get_base_channel();
                btn.set_sensitive(false);
                update_status(&format!("Status: Fetching Global {}...", slot + 1));
                let dc = Arc::clone(&dc);
                let is_active_c = Arc::clone(&is_active);
                let bpm_tx = bpm_fetch_tx.clone();
                let tune_tx = tune_fetch_tx.clone();
                let clk_tx = clock_fetch_tx.clone();
                let all_tx = all_params_fetch_tx.clone();
                // Owned Vec, not Rc: the background-thread closure must be Send.
                let all_fields: Vec<GlobalParamField> = (*global_param_fields).clone();
                let update_status = Arc::clone(&update_status);
                let btn_c = btn.clone();
                glib::spawn_future_local(async move {
                    let result = run_midi_session(&out_port, move |midi_in, midi_out| -> Result<String, String> {
                        let Some(raw) = request_global(midi_in, midi_out, ch, &dc, slot) else {
                            return Err(NO_RESPONSE.to_string());
                        };
                        if !dc.has_gate("sysex_layout.global.fields.payload.encoding") {
                            let payload_offset = dc.layout_offset_or_die("global", "payload") as isize;
                            if dc.layout_field("global", "bpm_msb").is_some() {
                                let msb_offset = dc.layout_offset_or_die("global", "bpm_msb") as isize;
                                let lsb_offset = dc.layout_offset_or_die("global", "bpm_lsb") as isize;
                                let bpm_raw = (u32::from(raw[(payload_offset + msb_offset) as usize]) << 7)
                                    | u32::from(raw[(payload_offset + lsb_offset) as usize]);
                                let bpm_min = as_f64_or_die(dc.json_get("sysex_api.global.bpm_min")) as u32;
                                let bpm_max = as_f64_or_die(dc.json_get("sysex_api.global.bpm_max")) as u32;
                                let bpm_mult = as_f64_or_die(dc.json_get("sysex_api.global.bpm_multiplier")) as u32;
                                let bpm = f64::from(bpm_min.max(bpm_max.min(bpm_raw / bpm_mult)));
                                let _ = bpm_tx.try_send(bpm);
                            }
                            if dc.layout_field("global", "external_sync_flags").is_some() {
                                let sync_offset = dc.layout_offset_or_die("global", "external_sync_flags") as isize;
                                let _ = clk_tx.try_send(raw[(payload_offset + sync_offset) as usize]);
                            }
                            let all_vals: Vec<u8> = all_fields.iter().map(|field| field.read(&raw, payload_offset as usize)).collect();
                            let _ = all_tx.try_send(all_vals);
                        } else if dc.has_gate("sysex_api.master_tune") {
                            let freq_offset = dc.layout_offset_or_die("global", "base_frequency") as usize;
                            let version = raw[ELEKTRON_PAYLOAD_START];
                            let packed_start = if version == 64 { 11 } else { 10 };
                            let packed = &raw[packed_start..raw.len() - 5];
                            let unpacked = decode_7bit(packed);
                            if !unpacked.is_empty() {
                                let decoded = decode_rle7(&unpacked, None);
                                if decoded.len() >= freq_offset + 4 {
                                    let base_frequency = u32::from_be_bytes(decoded[freq_offset..freq_offset + 4].try_into().unwrap());
                                    let multiplier = as_f64_or_die(dc.json_get("sysex_api.master_tune.multiplier"));
                                    let _ = tune_tx.try_send(f64::from(base_frequency) / multiplier);
                                }
                                // Fields run in ascending offset order, so a Global shorter than the layout just drops the trailing ones.
                                let all_vals: Vec<u8> = all_fields
                                    .iter()
                                    .take_while(|field| field.offset < decoded.len())
                                    .map(|field| field.read(&decoded, 0))
                                    .collect();
                                let _ = all_tx.try_send(all_vals);
                            }
                            let mut det = BpmDetector::new(None);
                            let deadline = Instant::now() + Duration::from_millis(1500);
                            while Instant::now() < deadline {
                                if let Some(msg) = midi_in.poll() {
                                    det.on_midi_message(&msg);
                                }
                                sleep(Duration::from_millis(2));
                            }
                            if let Some(bpm) = det.get_bpm() {
                                let _ = bpm_tx.try_send(bpm);
                            }
                        }
                        Ok(format!("Status: Global {} fetched. States updated.", slot + 1))
                    })
                    .await;
                    is_active_c.store(false, Ordering::Relaxed);
                    let msg = match result {
                        Ok(result) => result,
                        Err(e) => format!("Error: {e}"),
                    };
                    update_status(&msg);
                    btn_c.set_sensitive(true);
                });
            });

            // Entering the screen fetches the statuses.
            let fetch_on_map = fetch_button.clone();
            base.root.connect_map(move |_| fetch_on_map.emit_clicked());
        }

        // ── Status card ────────────────────────────────────────────────────────────────────────────────────

        if !cfg.status_params.is_empty() {
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Status").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label("Query or directly set a live status value without touching the Global.")
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let param_row = gtk4::Box::new(Orientation::Horizontal, 8);
            param_row.set_margin_top(4);
            param_row.append(&gtk4::Label::builder().label("Parameter:").xalign(0.0).build());
            let param_combo = CustomDropdown::new(Some(0));
            for (_, name, _, _) in &cfg.status_params {
                param_combo.append_text(name);
            }
            param_combo.set_active(Some(0));
            param_row.append(&*param_combo);
            section.append(&param_row);

            let val_row = gtk4::Box::new(Orientation::Horizontal, 8);
            val_row.append(&gtk4::Label::builder().label("Value  (for Set):").xalign(0.0).build());
            let val_spin = NumberSpinner::new(0.0, 0.0, 127.0, 1.0, "", 0);
            val_row.append(val_spin.widget());
            section.append(&val_row);

            {
                // Update `val_spin` range when param changes.
                let val_spin_c = val_spin.clone();
                let params = cfg.status_params.clone();
                let param_combo_widget = param_combo.clone();
                param_combo.connect_changed(move |_| {
                    let idx = param_combo_widget.active().unwrap_or(0) as usize;
                    if let Some((_, _, lo, hi)) = params.get(idx) {
                        val_spin_c.set_range(*lo, *hi);
                    }
                });
            }

            let btn_row = gtk4::Box::new(Orientation::Horizontal, 8);
            btn_row.set_margin_top(4);

            let query_btn = Button::builder().label("Query").css_classes(["suggested-action"]).build();
            let is_status_active = Arc::new(AtomicBool::new(false));
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let cfg = Arc::clone(&cfg);
                let dc = Arc::clone(&dc);
                let param_combo_widget = param_combo.clone();
                let is_active = Arc::clone(&is_status_active);
                let btn = query_btn.clone();
                query_btn.connect_clicked(move |_| {
                    if is_active.swap(true, Ordering::Relaxed) {
                        return;
                    }
                    let Some((out_port, _in_port)) = get_ports(&*get_midi_rc, &update_status) else {
                        is_active.store(false, Ordering::Relaxed);
                        return;
                    };
                    let idx = param_combo_widget.active().unwrap_or(0) as usize;
                    let Some(&(param_byte, ref param_name, _, _)) = cfg.status_params.get(idx) else {
                        is_active.store(false, Ordering::Relaxed);
                        return;
                    };
                    let param_name = param_name.clone();
                    let ch = get_base_channel();
                    btn.set_sensitive(false);
                    update_status(&format!("Status: Querying {param_name}..."));
                    let dc = Arc::clone(&dc);
                    let is_active_c = Arc::clone(&is_active);
                    let update_status = Arc::clone(&update_status);
                    let btn_c = btn.clone();
                    glib::spawn_future_local(async move {
                        let result = run_midi_session(&out_port, move |midi_in, midi_out| -> Result<String, String> {
                            let query_cmd = as_u64_or_die(dc.json_get("sysex_api.status.query_cmd")) as u8;
                            let reply_cmd = as_u64_or_die(dc.json_get("sysex_api.status.reply_cmd")) as u8;
                            match request_status_param(midi_in, midi_out, dc.prod, ch, query_cmd, reply_cmd, param_byte, 1.0) {
                                None => Err(format!("No response for {param_name} query.")),
                                Some(value) => Ok(format!("Status: {param_name} = {value}")),
                            }
                        })
                        .await;
                        is_active_c.store(false, Ordering::Relaxed);
                        let msg = match result {
                            Ok(result) => result,
                            Err(e) => format!("Error: {e}"),
                        };
                        update_status(&msg);
                        btn_c.set_sensitive(true);
                    });
                });
            }
            btn_row.append(&query_btn);

            let set_btn = Button::with_label("Set");
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let cfg = Arc::clone(&cfg);
                let dc = Arc::clone(&dc);
                set_btn.connect_clicked(move |_| {
                    let idx = param_combo.active().unwrap_or(0) as usize;
                    let Some(&(param_byte, ref param_name, _, _)) = cfg.status_params.get(idx) else {
                        return;
                    };
                    let param_name = param_name.clone();
                    let val = val_spin.value() as u8;
                    let set_cmd = as_u64_or_die(dc.json_get("sysex_api.status.set_cmd")) as u8;
                    let ch = get_base_channel();
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(&*get_midi_rc, dc.prod, ch, &[set_cmd, param_byte, val], &update_status_c).await {
                            update_status_c(&format!("Status: Set {param_name} = {val}."));
                        }
                    });
                });
            }
            btn_row.append(&set_btn);

            let ping_btn = Button::with_label("Ping");
            ping_btn.set_tooltip_text(Some("Send a bare presence ping with no parameter"));
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                ping_btn.connect_clicked(move |_| {
                    let query_cmd = as_u64_or_die(dc.json_get("sysex_api.status.query_cmd")) as u8;
                    let ch = get_base_channel();
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(&*get_midi_rc, dc.prod, ch, &[query_cmd], &update_status_c).await {
                            update_status_c("Status: Ping sent.");
                        }
                    });
                });
            }
            btn_row.append(&ping_btn);

            section.append(&btn_row);
            base.append_card(&section);
        }

        // ── Navigate card ──────────────────────────────────────────────────────────────────────────────────

        if dc.has_gate("sysex_api.navigation") {
            let pattern_max = as_f64_or_die(dc.json_get("sysex_api.navigation.load_pattern.max"));
            let kit_max = as_f64_or_die(dc.json_get("sysex_api.navigation.load_kit.max"));
            let song_max = as_f64_or_die(dc.json_get("sysex_api.navigation.load_song.max"));
            let load_pattern_cmd = as_u64_or_die(dc.json_get("sysex_api.navigation.load_pattern.cmd")) as u8;
            let load_kit_cmd = as_u64_or_die(dc.json_get("sysex_api.navigation.load_kit.cmd")) as u8;
            let save_kit_cmd = as_u64_or_die(dc.json_get("sysex_api.navigation.save_kit.cmd")) as u8;
            let load_song_cmd = as_u64_or_die(dc.json_get("sysex_api.navigation.load_song.cmd")) as u8;
            let save_song_cmd = as_u64_or_die(dc.json_get("sysex_api.navigation.save_song.cmd")) as u8;

            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Navigate").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label("Load or save patterns, kits, and songs directly via SysEx.".to_string())
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let grid = Grid::builder().row_spacing(8).column_spacing(12).margin_top(4).build();

            // Pattern row
            let pattern_spin = NumberSpinner::new(0.0, 0.0, pattern_max, 1.0, "", 0);
            pattern_spin.set_tooltip_text("A01=0, A02=1, ... B01=16, ...");
            grid.attach(
                &gtk4::Label::builder()
                    .label(format!("Pattern  (0-{}):", pattern_max as u32))
                    .xalign(0.0)
                    .build(),
                0,
                0,
                1,
                1,
            );
            grid.attach(pattern_spin.widget(), 1, 0, 1, 1);
            let load_pattern_btn = Button::with_label("Load");
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                load_pattern_btn.connect_clicked(move |_| {
                    let slot_val = pattern_spin.value() as u8;
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(
                            &*get_midi_rc,
                            dc.prod,
                            get_base_channel(),
                            &[load_pattern_cmd, slot_val],
                            &update_status_c,
                        )
                        .await
                        {
                            update_status_c(&format!(
                                "Status: Load Pattern {slot_val}  ({}).",
                                pattern_slot_label(slot_val as usize)
                            ));
                        }
                    });
                });
            }
            grid.attach(&load_pattern_btn, 2, 0, 1, 1);

            // Kit row
            let kit_spin = NumberSpinner::new(0.0, 0.0, kit_max, 1.0, "", 0);
            grid.attach(
                &gtk4::Label::builder()
                    .label(format!("Kit  (0-{}):", kit_max as u32))
                    .xalign(0.0)
                    .build(),
                0,
                1,
                1,
                1,
            );
            grid.attach(kit_spin.widget(), 1, 1, 1, 1);
            let load_kit_btn = Button::with_label("Load");
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                let kit_spin_c = kit_spin.clone();
                load_kit_btn.connect_clicked(move |_| {
                    let slot_val = kit_spin_c.value() as u8;
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(
                            &*get_midi_rc,
                            dc.prod,
                            get_base_channel(),
                            &[load_kit_cmd, slot_val],
                            &update_status_c,
                        )
                        .await
                        {
                            update_status_c(&format!("Status: Load Kit {slot_val}."));
                        }
                    });
                });
            }
            grid.attach(&load_kit_btn, 2, 1, 1, 1);
            let save_kit_btn = Button::with_label("Save");
            save_kit_btn.set_tooltip_text(Some("Save current kit to the slot shown"));
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                save_kit_btn.connect_clicked(move |_| {
                    let slot_val = kit_spin.value() as u8;
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(
                            &*get_midi_rc,
                            dc.prod,
                            get_base_channel(),
                            &[save_kit_cmd, slot_val],
                            &update_status_c,
                        )
                        .await
                        {
                            update_status_c(&format!("Status: Save Kit to slot {slot_val}."));
                        }
                    });
                });
            }
            grid.attach(&save_kit_btn, 3, 1, 1, 1);

            // Song row
            let song_spin = NumberSpinner::new(0.0, 0.0, song_max, 1.0, "", 0);
            grid.attach(
                &gtk4::Label::builder()
                    .label(format!("Song  (0-{}):", song_max as u32))
                    .xalign(0.0)
                    .build(),
                0,
                2,
                1,
                1,
            );
            grid.attach(song_spin.widget(), 1, 2, 1, 1);
            let load_song_btn = Button::with_label("Load");
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                let song_spin_c = song_spin.clone();
                load_song_btn.connect_clicked(move |_| {
                    let slot_val = song_spin_c.value() as u8;
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(
                            &*get_midi_rc,
                            dc.prod,
                            get_base_channel(),
                            &[load_song_cmd, slot_val],
                            &update_status_c,
                        )
                        .await
                        {
                            update_status_c(&format!("Status: Load Song {slot_val}."));
                        }
                    });
                });
            }
            grid.attach(&load_song_btn, 2, 2, 1, 1);
            let save_song_btn = Button::with_label("Save");
            save_song_btn.set_tooltip_text(Some("Save current song to the slot shown"));
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                save_song_btn.connect_clicked(move |_| {
                    let slot_val = song_spin.value() as u8;
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(
                            &*get_midi_rc,
                            dc.prod,
                            get_base_channel(),
                            &[save_song_cmd, slot_val],
                            &update_status_c,
                        )
                        .await
                        {
                            update_status_c(&format!("Status: Save Song to slot {slot_val}."));
                        }
                    });
                });
            }
            grid.attach(&save_song_btn, 3, 2, 1, 1);

            section.append(&grid);
            base.append_card(&section);
        }

        // ── Assign Machine card ────────────────────────────────────────────────────────────────────────────

        if dc.has_gate("sysex_api.assign_machine") {
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Assign Machine").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label(
                        "Assign a synthesis machine to a track. \n\
                        Init scope controls which parameters are reset.",
                    )
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let row = gtk4::Box::new(Orientation::Horizontal, 12);
            row.set_margin_top(4);

            let col_track = gtk4::Box::new(Orientation::Vertical, 4);
            col_track.append(&gtk4::Label::builder().label("Track").xalign(0.0).css_classes(["caption"]).build());
            let mach_track_spin = NumberSpinner::new(1.0, 1.0, as_array_or_die(dc.json_get("tracks")).len() as f64, 1.0, "", 0);
            col_track.append(mach_track_spin.widget());
            row.append(&col_track);

            let col_type = gtk4::Box::new(Orientation::Vertical, 4);
            col_type.append(&gtk4::Label::builder().label("Synth").xalign(0.0).css_classes(["caption"]).build());
            let family_combo = CustomDropdown::new(Some(0));
            family_combo.set_size_request(90, -1);
            col_type.append(&*family_combo);
            row.append(&col_type);

            let col_machine = gtk4::Box::new(Orientation::Vertical, 4);
            col_machine.append(&gtk4::Label::builder().label("Machine").xalign(0.0).css_classes(["caption"]).build());
            let variant_combo = CustomDropdown::new(Some(0));
            variant_combo.set_size_request(110, -1);
            col_machine.append(&*variant_combo);
            row.append(&col_machine);

            let col_init = gtk4::Box::new(Orientation::Vertical, 4);
            col_init.append(
                &gtk4::Label::builder()
                    .label("Init Scope")
                    .xalign(0.0)
                    .css_classes(["caption"])
                    .build(),
            );
            let init_combo = CustomDropdown::new(Some(0));
            for scope in &cfg.init_scopes {
                init_combo.append_text(scope);
            }
            init_combo.set_active(Some(0));
            col_init.append(&*init_combo);
            row.append(&col_init);
            section.append(&row);

            // Machine state shared across populate closures
            let machine_families: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
            let machine_by_family: MachineByFamily = Rc::new(RefCell::new(HashMap::new()));
            let machine_variants: Rc<RefCell<Vec<(u8, String, String)>>> = Rc::new(RefCell::new(Vec::new()));

            let populate_variants: Rc<dyn Fn()> = {
                let family_combo = family_combo.clone();
                let variant_combo = variant_combo.clone();
                let machine_families = Rc::clone(&machine_families);
                let machine_by_family: MachineByFamily = Rc::clone(&machine_by_family);
                let machine_variants = Rc::clone(&machine_variants);
                Rc::new(move || {
                    let idx = family_combo.active().unwrap_or(0) as usize;
                    let families = machine_families.borrow();
                    let by_family = machine_by_family.borrow();
                    let family = match families.get(idx) {
                        Some(family) => family.clone(),
                        None => return,
                    };
                    let variants = by_family.get(&family).cloned().unwrap_or_default();
                    (*machine_variants.borrow_mut()).clone_from(&variants);
                    variant_combo.remove_all();
                    for (_, _, full) in &variants {
                        variant_combo.append_text(full);
                    }
                    variant_combo.set_active(Some(0));
                })
            };

            let populate_families: Rc<dyn Fn()> = {
                let family_combo = family_combo.clone();
                let cfg = Arc::clone(&cfg);
                let machine_families = Rc::clone(&machine_families);
                let machine_by_family: MachineByFamily = Rc::clone(&machine_by_family);
                Rc::new(move || {
                    let (mut families, mut by_family) = group_machines(&cfg.machines);
                    if !cfg.machine_family_order.is_empty() {
                        let ordered: Vec<String> = cfg
                            .machine_family_order
                            .iter()
                            .filter(|family| by_family.contains_key(*family))
                            .cloned()
                            .collect();
                        let extra: Vec<String> = families.iter().filter(|family| !ordered.contains(family)).cloned().collect();
                        families = ordered;
                        families.extend(extra);
                    }
                    for (family, variants) in &mut by_family {
                        if let Some(order) = cfg.machine_order.get(family) {
                            variants.sort_by_key(|(_, value, _)| order.iter().position(|order_entry| order_entry == value).unwrap_or(999));
                        }
                    }
                    (*machine_families.borrow_mut()).clone_from(&families);
                    *machine_by_family.borrow_mut() = by_family;
                    family_combo.remove_all();
                    for family in &families {
                        family_combo.append_text(family);
                    }
                    family_combo.set_active(Some(0)); // triggers connect_changed → populate_variants
                })
            };

            // Wire signals: family change → variant change, model change → family change.
            {
                let populate_variants_c = Rc::clone(&populate_variants);
                family_combo.connect_changed(move |_| populate_variants_c());
            }
            populate_families(); // initial fill

            let assign_btn = Button::builder()
                .label("Assign Machine")
                .css_classes(["suggested-action"])
                .halign(Align::Start)
                .build();
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let cfg = Arc::clone(&cfg);
                let dc = Arc::clone(&dc);
                let machine_variants = Rc::clone(&machine_variants);
                assign_btn.connect_clicked(move |_| {
                    let var_idx = variant_combo.active().unwrap_or(0) as usize;
                    let variants = machine_variants.borrow();
                    let Some((machine_id_ref, _, machine_name_ref)) = variants.get(var_idx) else {
                        return;
                    };
                    let machine_id = *machine_id_ref;
                    let machine_name = machine_name_ref.clone();
                    drop(variants);
                    let track_idx = mach_track_spin.value() as u8 - 1;
                    let init_scope = init_combo.active().unwrap_or(0) as u8;
                    let ch = get_base_channel();
                    let init_name = cfg.init_scopes.get(init_scope as usize).cloned().unwrap_or_default();
                    let payload = build_machine_assignment_payload(&dc, track_idx, machine_id, Some(init_scope));
                    let extra = format!("  (init={init_name})");

                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(&*get_midi_rc, dc.prod, ch, &payload, &update_status_c).await {
                            update_status_c(&format!("Status: Track {} → {machine_name}{extra}.", track_idx + 1));
                        }
                    });
                });
            }
            section.append(&assign_btn);
            base.append_card(&section);
        }

        // ── Track Routing card ─────────────────────────────────────────────────────────────────────────────

        if !cfg.routing_outputs.is_empty() {
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Track Routing").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label("Assign a track's output bus and input source.")
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let row = gtk4::Box::new(Orientation::Horizontal, 12);
            row.set_margin_top(4);

            let col_track = gtk4::Box::new(Orientation::Vertical, 4);
            col_track.append(&gtk4::Label::builder().label("Track").xalign(0.0).css_classes(["caption"]).build());
            let routing_track_spin = NumberSpinner::new(1.0, 1.0, as_array_or_die(dc.json_get("tracks")).len() as f64, 1.0, "", 0);
            col_track.append(routing_track_spin.widget());
            row.append(&col_track);

            let col_bus = gtk4::Box::new(Orientation::Vertical, 4);
            col_bus.append(
                &gtk4::Label::builder()
                    .label("Output Bus")
                    .xalign(0.0)
                    .css_classes(["caption"])
                    .build(),
            );
            let bus_combo = CustomDropdown::new(Some(0));
            for (name, _) in &cfg.routing_outputs {
                bus_combo.append_text(name);
            }
            let default_out = as_u64_or_die(dc.json_get("sysex_api.track_routing.default_output")) as u32;
            bus_combo.set_active(Some(default_out));
            col_bus.append(&*bus_combo);
            row.append(&col_bus);

            let in_combo_opt: Option<CustomDropdown> = if cfg.routing_inputs.is_empty() {
                None
            } else {
                let col_in = gtk4::Box::new(Orientation::Vertical, 4);
                col_in.append(
                    &gtk4::Label::builder()
                        .label("Input Source")
                        .xalign(0.0)
                        .css_classes(["caption"])
                        .build(),
                );
                let input_combo = CustomDropdown::new(Some(0));
                for (name, _) in &cfg.routing_inputs {
                    input_combo.append_text(name);
                }
                let default_in = as_u64_or_die(dc.json_get("sysex_api.track_routing.default_input")) as u32;
                input_combo.set_active(Some(default_in));
                col_in.append(&*input_combo);
                row.append(&col_in);
                Some(input_combo)
            };

            section.append(&row);

            let routing_btn = Button::builder()
                .label("Set Routing")
                .css_classes(["suggested-action"])
                .halign(Align::Start)
                .build();
            {
                use std::fmt::Write;
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let cfg = Arc::clone(&cfg);
                let dc = Arc::clone(&dc);
                routing_btn.connect_clicked(move |_| {
                    let track_idx = routing_track_spin.value() as u8 - 1;
                    let out_idx = bus_combo.active().unwrap_or(0) as usize;
                    let (out_label, out_val) = cfg.routing_outputs.get(out_idx).cloned().unwrap_or_default();
                    let routing_cmd = as_u64_or_die(dc.json_get("sysex_api.track_routing.cmd")) as u8;
                    let ch = get_base_channel();
                    let mut payload = vec![routing_cmd, track_idx, out_val];
                    let mut status = format!("Track {} → {out_label}", track_idx + 1);
                    if let Some(ref input_combo) = in_combo_opt {
                        let in_idx = input_combo.active().unwrap_or(0) as usize;
                        if let Some((in_label, in_val)) = cfg.routing_inputs.get(in_idx) {
                            payload.push(*in_val);
                            let _ = write!(status, "  (input: {in_label})");
                        }
                    }
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(&*get_midi_rc, dc.prod, ch, &payload, &update_status_c).await {
                            update_status_c(&format!("Status: {status}."));
                        }
                    });
                });
            }
            section.append(&routing_btn);
            base.append_card(&section);
        }

        // ── Note Map card ──────────────────────────────────────────────────────────────────────────────────

        if dc.has_gate("sysex_api.note_map") {
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("MIDI Note Map").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label("Map a MIDI note to a track (0-15), or reset the entire note→track/pattern map to Elektron factory defaults.")
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let row = gtk4::Box::new(Orientation::Horizontal, 12);
            row.set_margin_top(4);
            let col_note = gtk4::Box::new(Orientation::Vertical, 4);
            col_note.append(
                &gtk4::Label::builder()
                    .label("MIDI Note")
                    .xalign(0.0)
                    .css_classes(["caption"])
                    .build(),
            );
            let note_spin = NumberSpinner::new(64.0, 0.0, 127.0, 1.0, "", 0);
            col_note.append(note_spin.widget());
            row.append(&col_note);
            let col_ntrack = gtk4::Box::new(Orientation::Vertical, 4);
            col_ntrack.append(
                &gtk4::Label::builder()
                    .label("Track (0-15)")
                    .xalign(0.0)
                    .css_classes(["caption"])
                    .build(),
            );
            let ntrack_spin = NumberSpinner::new(0.0, 0.0, 15.0, 1.0, "", 0);
            col_ntrack.append(ntrack_spin.widget());
            row.append(&col_ntrack);
            section.append(&row);

            let btn_row = gtk4::Box::new(Orientation::Horizontal, 8);
            btn_row.set_margin_top(4);
            let map_btn = Button::builder().label("Map Note").css_classes(["suggested-action"]).build();
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                map_btn.connect_clicked(move |_| {
                    let note = note_spin.value() as u8;
                    let track_idx = ntrack_spin.value() as u8;
                    let set_cmd = as_u64_or_die(dc.json_get("sysex_api.note_map.set_cmd")) as u8;
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(
                            &*get_midi_rc,
                            dc.prod,
                            get_base_channel(),
                            &[set_cmd, note, track_idx],
                            &update_status_c,
                        )
                        .await
                        {
                            update_status_c(&format!("Status: Note {note} → Track {}.", track_idx + 1));
                        }
                    });
                });
            }
            btn_row.append(&map_btn);
            let reset_btn = Button::builder().label("Reset All").css_classes(["destructive-action"]).build();
            reset_btn.set_tooltip_text(Some("Reset the entire note map to Elektron factory defaults"));
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                reset_btn.connect_clicked(move |_| {
                    let reset_cmd = as_u64_or_die(dc.json_get("sysex_api.note_map.reset_cmd")) as u8;
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(&*get_midi_rc, dc.prod, get_base_channel(), &[reset_cmd], &update_status_c).await {
                            update_status_c("Status: MIDI note map reset to factory defaults.");
                        }
                    });
                });
            }
            btn_row.append(&reset_btn);
            section.append(&btn_row);
            base.append_card(&section);
        }

        // ── Trig & Mute Groups card ────────────────────────────────────────────────────────────────────────

        if dc.has_gate("sysex_api.groups") {
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Trig & Mute Groups").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            section.append(
                &gtk4::Label::builder()
                    .label(
                        "Trig Group: when the trigger track fires, the target also fires. \n\
                        Mute Group: when the control track is muted, the muted track is also muted.",
                    )
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let grid = Grid::builder().row_spacing(8).column_spacing(12).margin_top(4).build();
            let trig_src = NumberSpinner::new(1.0, 1.0, 16.0, 1.0, "", 0);
            let trig_dst = NumberSpinner::new(2.0, 1.0, 16.0, 1.0, "", 0);
            grid.attach(&gtk4::Label::builder().label("Trigger Track:").xalign(0.0).build(), 0, 0, 1, 1);
            grid.attach(trig_src.widget(), 1, 0, 1, 1);
            grid.attach(&gtk4::Label::builder().label("Target Track:").xalign(0.0).build(), 2, 0, 1, 1);
            grid.attach(trig_dst.widget(), 3, 0, 1, 1);
            let trigger_btn = Button::builder().label("Set Trig Group").css_classes(["suggested-action"]).build();
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                trigger_btn.connect_clicked(move |_| {
                    let src = trig_src.value() as u8 - 1;
                    let dst = trig_dst.value() as u8 - 1;
                    let cmd = as_u64_or_die(dc.json_get("sysex_api.groups.trig_cmd")) as u8;
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(&*get_midi_rc, dc.prod, get_base_channel(), &[cmd, src, dst], &update_status_c).await {
                            update_status_c(&format!("Status: Trig Group: Track {} triggers Track {}.", src + 1, dst + 1));
                        }
                    });
                });
            }
            grid.attach(&trigger_btn, 4, 0, 1, 1);

            let mute_src = NumberSpinner::new(1.0, 1.0, 16.0, 1.0, "", 0);
            let mute_dst = NumberSpinner::new(2.0, 1.0, 16.0, 1.0, "", 0);
            grid.attach(&gtk4::Label::builder().label("Control Track:").xalign(0.0).build(), 0, 1, 1, 1);
            grid.attach(mute_src.widget(), 1, 1, 1, 1);
            grid.attach(&gtk4::Label::builder().label("Muted Track:").xalign(0.0).build(), 2, 1, 1, 1);
            grid.attach(mute_dst.widget(), 3, 1, 1, 1);
            let mute_btn = Button::builder().label("Set Mute Group").css_classes(["suggested-action"]).build();
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                mute_btn.connect_clicked(move |_| {
                    let src = mute_src.value() as u8 - 1;
                    let dst = mute_dst.value() as u8 - 1;
                    let cmd = as_u64_or_die(dc.json_get("sysex_api.groups.mute_cmd")) as u8;
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(&*get_midi_rc, dc.prod, get_base_channel(), &[cmd, src, dst], &update_status_c).await {
                            update_status_c(&format!(
                                "Status: Mute Group: Muting Track {} also mutes Track {}.",
                                src + 1,
                                dst + 1
                            ));
                        }
                    });
                });
            }
            grid.attach(&mute_btn, 4, 1, 1, 1);
            section.append(&grid);
            base.append_card(&section);
        }

        // ── Set Kit Name card ──────────────────────────────────────────────────────────────────────────────

        if dc.has_gate("sysex_api.set_kit_name") {
            let section = gtk4::Box::new(Orientation::Vertical, 8);
            section.set_margin_top(5);
            section.set_margin_bottom(5);

            let title = gtk4::Label::builder().label("Set Kit Name").xalign(0.0).build();
            title.add_css_class("title-3");
            section.append(&title);

            let name_length = as_u64_or_die(dc.json_get("sysex_api.set_kit_name.name_length")) as i32;
            section.append(
                &gtk4::Label::builder()
                    .label(format!(
                        "Set the current kit's name without a full dump/write cycle. Up to {name_length} characters, uppercase A-Z, 0-9, and space."
                    ))
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build(),
            );

            let row = gtk4::Box::new(Orientation::Horizontal, 8);
            row.set_margin_top(4);
            row.append(&gtk4::Label::builder().label("Name:").xalign(0.0).build());
            let name_entry = CustomTextbox::new();
            name_entry.set_max_length(name_length);
            name_entry.set_placeholder_text(Some("UP TO 11 CHARS"));
            name_entry.enable_elektron_filter();
            row.append(&*name_entry);
            section.append(&row);

            let name_btn = Button::builder()
                .label("Set Name")
                .css_classes(["suggested-action"])
                .halign(Align::Start)
                .build();
            {
                let update_status = Arc::clone(&update_status);
                let get_midi_rc = Rc::clone(&get_midi_rc);
                let dc = Arc::clone(&dc);
                name_btn.connect_clicked(move |_| {
                    let cmd = as_u64_or_die(dc.json_get("sysex_api.set_kit_name.cmd")) as u8;
                    let name_length_u = name_length as usize;
                    let raw_text: String = filter_elektron_name(&name_entry.text()).chars().take(name_length_u).collect();
                    let mut name_bytes: Vec<u8> = raw_text.into_bytes();
                    name_bytes.resize(name_length_u, 0x20);
                    let mut payload = vec![cmd];
                    payload.extend_from_slice(&name_bytes);
                    let get_midi_rc = Rc::clone(&get_midi_rc);
                    let dc = Arc::clone(&dc);
                    let update_status_c = Arc::clone(&update_status);
                    glib::spawn_future_local(async move {
                        if send_sysex_message(&*get_midi_rc, dc.prod, get_base_channel(), &payload, &update_status_c).await {
                            let displayed = String::from_utf8_lossy(&name_bytes).trim_end().to_string();
                            update_status_c(&format!("Status: Kit name set to \"{displayed}\"."));
                        }
                    });
                });
            }
            section.append(&name_btn);
            base.append_card(&section);
        }

        // ── Status area (appended last so it appears at the bottom) ────────────────────────────────────────

        // Arming the deferred slot connects the updater that every card captured above to this freshly built label.
        base.build_status_area(false);
        *status_slot.lock().unwrap() = Some(base.status_updater());
        update_status("Status: Ready.");

        let _ = (
            fetch_btn,
            shared_slot_spin,
            get_midi_rc,
            cfg,
            clock_pill_data_opt,
            tune_spin_opt,
            bpm_spin_opt,
        );

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
