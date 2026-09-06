//! Headless kit manager: owns the live kit state and the hardware transfer engine.
//!
//! Holds the machine, master-FX, and per-track parameter state, and decides how edits reach the device as CC or SysEx.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use glib;

use crate::c7_file_interfacing::{
    apply_mnm_kit_globals, apply_sound_to_kit, extract_mnm_kit_globals, extract_sound_from_kit, kit_blob_offset, kit_blob_offset_or_die,
    kit_track_blob_len, normalize_mnm_workspace,
};
use crate::device_config::DeviceConfig;
use crate::midi::{MidiSender, PollableMidiInput, is_valid_port, run_midi_session};
use crate::sysex::{
    build_elektron_sysex, build_machine_assignment_payload, fetch_elektron_slot, patch_elektron_original_position,
    redirect_self_targeting_lfo, request_selected_track, request_status_param,
};
use crate::utils::{JsonPath, as_array_or, as_array_or_die, as_string_or, as_string_or_die, as_u64_or, as_u64_or_die};

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// One joystick axis from the device JSON (`system.joystick`).
///
/// Ranges are inherent to the transport, so the JSON only names the type.
#[derive(Clone)]
pub enum JoystickAxis {
    PitchBend,
    Cc(u8),
}

impl JoystickAxis {
    /// Wraps a final axis value in its MIDI message (pitch bend splits 14 bits across two data bytes, low first).
    fn encode_wire_message(&self, ch: u8, val: i32) -> Vec<u8> {
        match self {
            Self::PitchBend => vec![0xE0 | ch, (val & 0x7F) as u8, ((val >> 7) & 0x7F) as u8],
            Self::Cc(cc) => vec![0xB0 | ch, *cc, (val & 0x7F) as u8],
        }
    }

    /// Builds the message for a bipolar stick position (-1..1) around the transport's natural center (8192 for PB, 64 for CC).
    pub fn bipolar_message(&self, ch: u8, norm: f64) -> Vec<u8> {
        let val = match self {
            Self::PitchBend => ((norm * 8191.0).round() as i32 + 8192).clamp(0, 16383),
            Self::Cc(_) => ((norm * 63.0).round() as i32 + 64).clamp(0, 127),
        };
        self.encode_wire_message(ch, val)
    }

    /// Builds the message for a unipolar deflection (0..1) rising from the transport's resting value (center for PB, 0 for CC).
    pub fn unipolar_message(&self, ch: u8, norm: f64) -> Vec<u8> {
        let val = match self {
            Self::PitchBend => ((norm * 8191.0).round() as i32 + 8192).clamp(0, 16383),
            Self::Cc(_) => ((norm * 127.0).round() as i32).clamp(0, 127),
        };
        self.encode_wire_message(ch, val)
    }
}

/// Hardware command bytes and capability flags resolved once from the device JSON.
#[derive(Clone)]
pub struct HwConfig {
    pub prod: u8,
    pub has_auto_channel: bool,
    pub combo_offset: u32,
    pub status_query_cmd: u8,
    pub status_set_cmd: u8,
    pub status_reply_cmd: u8,
    pub status_kit_cmd: u8,
    pub status_audio_mode_cmd: Option<u8>,
    pub load_kit_cmd: u8,
    pub save_kit_cmd: u8,
    kit_request_cmd: u8,
    kit_dump_cmd: u8,
    kit_workspace_slot: u8,
    kit_workspace_extra: Vec<u8>,
    status_selected_track_cmd: Option<u8>,
    pub joystick_x: Option<JoystickAxis>,
    pub joystick_y_up: Option<JoystickAxis>,
    pub joystick_y_down: Option<JoystickAxis>,
    pub lfo_block_byte: u8,
}

impl HwConfig {
    /// Resolves the hardware config from a device JSON.
    pub fn from_device(dc: &DeviceConfig) -> Self {
        // AUTO track is available when the device JSON defines an `auto_channel`.
        let has_auto_channel = dc.has_gate("system.auto_channel");
        HwConfig {
            prod: dc.prod,
            has_auto_channel,
            combo_offset: if has_auto_channel { 2 } else { 1 },
            status_query_cmd: as_u64_or_die(dc.json_get("sysex_api.status.query_cmd")) as u8,
            status_set_cmd: as_u64_or_die(dc.json_get("sysex_api.status.set_cmd")) as u8,
            status_reply_cmd: as_u64_or_die(dc.json_get("sysex_api.status.reply_cmd")) as u8,
            status_kit_cmd: as_u64_or_die(dc.json_get("sysex_api.status.params.kit_number.cmd")) as u8,
            status_audio_mode_cmd: status_param_cmd(dc, "audio_mode"), // MnM only
            load_kit_cmd: as_u64_or_die(dc.json_get("sysex_api.navigation.load_kit.cmd")) as u8,
            save_kit_cmd: as_u64_or_die(dc.json_get("sysex_api.navigation.save_kit.cmd")) as u8,
            kit_request_cmd: as_u64_or_die(dc.json_get("sysex_api.kit.request_cmd")) as u8,
            kit_dump_cmd: as_u64_or_die(dc.json_get("sysex_api.kit.write_cmd")) as u8,
            kit_workspace_slot: as_u64_or_die(dc.json_get("sysex_api.kit.workspace_slot")) as u8,
            kit_workspace_extra: as_array_or(dc.json_get("sysex_api.kit.workspace_extra"), &[])
                .iter()
                .filter_map(|value| value.as_u64().map(|byte_val| byte_val as u8))
                .collect(),
            status_selected_track_cmd: status_param_cmd(dc, "selected_track"),
            joystick_x: parse_joy_axis(dc, "x"),
            joystick_y_up: parse_joy_axis(dc, "y_up"),
            joystick_y_down: parse_joy_axis(dc, "y_down"),
            lfo_block_byte: dc
                .json_get("track_lfo.sysex.block_byte")
                .and_then(|value| value.as_str())
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .unwrap_or(0),
        }
    }
}

/// One master-FX parameter, enumerated from the device JSON's `master_fx` section.
pub struct MasterFxParam {
    pub fx_name: String,
    pub fx_fullname: String,
    pub block_byte: u8,
    pub name: String,
    pub fullname: String,
    pub param_id: u8,
    pub default_val: i32,
    pub display_offset: i32,
    pub kit_offset: usize,
}

/// One workspace kit fetched from the device, tagged with the slot the device reported as current.
///
/// Most callers destructure `workspace` alone. `slot` exists for the ones that label the fetched kit in the UI.
pub struct FetchResult {
    pub slot: u8,
    pub workspace: Vec<u8>,
}

/// One debounced Miscellaneous workspace patch: which track, which blob offsets the UI owns, and any kit-global payloads.
pub struct MiscSyncRequest {
    pub track_idx: usize,
    pub owned_offsets: Vec<usize>,
    pub shared_params: Vec<(usize, u8)>,
    pub should_include_globals: bool,
}

/// Headless owner of the kit editor's device state and hardware transfers.
///
/// The UI renders from this state and reports edits into it. How those edits reach the device is decided here.
pub struct KitManager {
    pub hw: Arc<HwConfig>,
    pub dc: Arc<DeviceConfig>,

    /// Per-track parameter bytes as extracted by `extract_sound_from_kit()`.
    ///
    /// Row 16 backs the ALL selection.
    pub track_states: RefCell<Vec<Vec<u8>>>,
    /// Per-track LFO block state (MD SysEx LFO params).
    pub lfo_states: RefCell<Vec<Vec<u8>>>,
    /// Machine model ID per track.
    pub machine_models: RefCell<HashMap<usize, u8>>,
    /// Kit-global bytes backing the Global misc control.
    pub kit_globals: RefCell<Vec<u8>>,
    /// Kits fetched in the background land here, and the UI polls it and repaints.
    pub pending_kit: Arc<Mutex<Option<Vec<u8>>>>,
    /// Debounce source for the misc workspace patch.
    misc_sync_src: RefCell<Option<glib::SourceId>>,
}

impl KitManager {
    /// Creates the manager for a device, sizing the state tables from its JSON.
    pub fn new(dc: Arc<DeviceConfig>) -> Rc<Self> {
        let hw = Arc::new(HwConfig::from_device(&dc));
        // 150 bytes per row comfortably covers both the CC param count and the extended track blob of the supported devices.
        // 17 rows: 16 tracks plus row 16 backing the ALL selection.
        let track_states = vec![vec![0u8; 150]; 17];
        let lfo_params = as_array_or(dc.json_get("track_lfo.params"), &[]);
        let lfo_states = vec![vec![0u8; lfo_params.len().max(8)]; 17];
        Rc::new(KitManager {
            hw,
            dc,
            track_states: RefCell::new(track_states),
            lfo_states: RefCell::new(lfo_states),
            machine_models: RefCell::new(HashMap::new()),
            kit_globals: RefCell::new(vec![0; 4]),
            pending_kit: Arc::new(Mutex::new(None)),
            misc_sync_src: RefCell::new(None),
        })
    }

    /// Ingests a raw kit dump: refreshes every track's state row, the machine models, the MD LFO blocks, and the MnM kit globals.
    pub fn ingest_kit(&self, kit_raw: &[u8]) {
        let num_tracks = self
            .dc
            .json_get("tracks")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap();
        let mut track_states = self.track_states.borrow_mut();
        let mut lfo_states = self.lfo_states.borrow_mut();
        let mut machine_models = self.machine_models.borrow_mut();

        for track_idx in 0..num_tracks {
            let sound_bytes = extract_sound_from_kit(kit_raw, track_idx, &self.dc);
            if track_idx < track_states.len() {
                let num_cc = sound_bytes.len().min(track_states[track_idx].len());
                track_states[track_idx][..num_cc].copy_from_slice(&sound_bytes[..num_cc]);
            }

            if let Some(model_byte) = self
                .dc
                .json_get("system.machine_model_byte")
                .and_then(serde_json::Value::as_u64)
                .map(|val| val as usize)
                && sound_bytes.len() > model_byte
            {
                machine_models.insert(track_idx, sound_bytes[model_byte]);
            }

            if self.dc.is_device("MD") {
                // LFO params in the blob:
                //   [`lfos_offset..lfos_offset+5`] = destinationTrack, destinationParam, shape1, shape2, type  (→ `lfo_states[0..4]`)
                //   [21..23] = LFOS/LFOD/LFOM (speed, depth, mix)                       (→ `lfo_states[5..7]`)
                //
                // Speed/depth/mix sit in the standard params block, NOT in the LFO section.
                // The LFO section (36 bytes) only stores dest/shape/type + 31 bytes of oscillator engine state.
                // Speed is intentionally excluded from the `$52` SysEx encoding.
                if track_idx < lfo_states.len() {
                    let lfos_offset = kit_blob_offset(&self.dc, "lfos").unwrap();
                    let lfo_end = (lfos_offset + 5).min(sound_bytes.len());
                    for (i, &val) in sound_bytes[lfos_offset..lfo_end].iter().enumerate() {
                        if i < lfo_states[track_idx].len() {
                            lfo_states[track_idx][i] = val;
                        }
                    }
                    if sound_bytes.len() > 23 {
                        let row = &mut lfo_states[track_idx];
                        if row.len() > 5 {
                            row[5] = sound_bytes[21];
                        }
                        if row.len() > 6 {
                            row[6] = sound_bytes[22];
                        }
                        if row.len() > 7 {
                            row[7] = sound_bytes[23];
                        }
                    }
                }
            }
        }
        drop(track_states);

        if self.dc.is_device("MnM") {
            *self.kit_globals.borrow_mut() = extract_mnm_kit_globals(kit_raw, &self.dc);
        }
    }

    /// Debounces Miscellaneous edits into a single workspace patch, so spinner drags don't spam full kit writes.
    ///
    /// The latest request wins and executes ~500 ms after the last edit.
    pub fn schedule_misc_sync(self: &Rc<Self>, port: Option<String>, request: MiscSyncRequest) {
        if let Some(src) = self.misc_sync_src.borrow_mut().take() {
            src.remove();
        }
        let self_wk = Rc::downgrade(self);
        // The timeout closure is `FnMut`, so the one-shot payload is parked in a `RefCell` and taken on fire.
        let payload = RefCell::new(Some((port, request)));
        let src = glib::timeout_add_local(Duration::from_millis(500), move || {
            if let (Some(mgr), Some((port, request))) = (self_wk.upgrade(), payload.borrow_mut().take()) {
                *mgr.misc_sync_src.borrow_mut() = None;
                mgr.run_misc_sync(port, request);
            }
            glib::ControlFlow::Break
        });
        *self.misc_sync_src.borrow_mut() = Some(src);
    }

    /// Executes one Miscellaneous workspace patch.
    ///
    /// Fetch the live kit -> Splice the UI-owned offsets -> Patch kit globals -> Write back -> Queue the result for the UI.
    ///
    /// Only misc-owned offsets are spliced, so params, level, and model keep the freshly fetched values.
    fn run_misc_sync(&self, port: Option<String>, request: MiscSyncRequest) {
        // The patch codec below is Monomachine-specific. Devices whose misc params all live-send never schedule this.
        if !self.dc.is_device("MnM") {
            return;
        }
        let Some(port) = port else { return };
        if !is_valid_port(&port) {
            return;
        }
        let blob_len = kit_track_blob_len(&self.dc);
        let misc_state: Vec<u8> = {
            let states = self.track_states.borrow();
            let Some(row) = states.get(request.track_idx) else {
                return;
            };
            if row.len() < blob_len {
                return;
            }
            row[..blob_len].to_vec()
        };
        let kit_globals = if request.should_include_globals {
            Some(self.kit_globals.borrow().clone())
        } else {
            None
        };

        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let pending = Arc::clone(&self.pending_kit);
        let MiscSyncRequest {
            track_idx,
            owned_offsets,
            shared_params,
            ..
        } = request;

        glib::spawn_future_local(async move {
            let _ = run_midi_session(&port, move |midi_in, midi_out| -> Result<(), String> {
                let Some(FetchResult { workspace: kit, .. }) = fetch_kit_for_display(midi_in, midi_out, &hw, &dc) else {
                    return Ok(());
                };
                let mut blob = extract_sound_from_kit(&kit, track_idx, &dc);
                for &offset in &owned_offsets {
                    if offset < blob.len() && offset < misc_state.len() {
                        blob[offset] = misc_state[offset];
                    }
                }
                let mut patched = apply_sound_to_kit(&kit, &blob, track_idx, &dc);
                if kit_globals.is_some() || !shared_params.is_empty() {
                    patched = apply_mnm_kit_globals(&patched, kit_globals.as_deref().unwrap_or(&[]), &shared_params, &dc);
                }
                write_kit_to_workspace(&hw, midi_out, &patched);
                if let Ok(mut lock) = pending.lock() {
                    *lock = Some(patched);
                }
                Ok(())
            })
            .await;
        });
    }
}

/// Returns the device's assignable machines as (id, display name) pairs, sorted into the JSON's `machine_order` family ranking.
///
/// Empty when the device JSON declares no machines, which is also the "device has no machine concept" gate for callers.
pub fn machine_list(dc: &DeviceConfig) -> Vec<(u8, String)> {
    let mut machine_list: Vec<(u8, String)> = Vec::new();
    let Some(machines) = dc.json_get("sysex_api.machines") else {
        return machine_list;
    };

    let raw_list = as_array_or(machines.json_get("base"), &[]);
    for machine in raw_list {
        let obj = machine.as_object().unwrap();
        let id = obj["id"].as_u64().unwrap() as u8;
        let name = obj["name"].as_str().unwrap();
        machine_list.push((id, name.to_string()));
    }
    if let Some(machine_order) = machines.json_get("machine_order").and_then(|value| value.as_object()) {
        let family_order: Vec<String> = machine_order.keys().cloned().collect();
        machine_list.sort_by_key(|(_, name)| {
            let mut family_rank = 999;
            for (i, family) in family_order.iter().enumerate() {
                if name.starts_with(&format!("{family}-")) || name == family {
                    family_rank = i;
                    break;
                }
            }

            let machine_rank = if family_rank < 999 {
                let family = &family_order[family_rank];
                let short = if name.starts_with(&format!("{family}-")) {
                    &name[family.len() + 1..]
                } else {
                    name
                };
                machine_order[family]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|value| value.as_str() == Some(short))
                    .unwrap()
            } else {
                999
            };

            (family_rank, machine_rank)
        });
    }
    machine_list
}

/// Enumerates every master-FX parameter from the device JSON.
///
/// Empty on devices with no `master_fx` section (MnM).
pub fn master_fx_list(dc: &DeviceConfig) -> Vec<MasterFxParam> {
    let fx_array = as_array_or(dc.json_get("master_fx"), &[]);
    if fx_array.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    for fx in fx_array {
        let fx_name = as_string_or(fx.json_get("name"), "").to_string();
        let fx_fullname = as_string_or(fx.json_get("fullname"), &fx_name).to_string();
        let block_byte = u8::from_str_radix(as_string_or_die(fx.json_get("sysex.block_byte")), 16).unwrap();
        for json_param in as_array_or(fx.json_get("params"), &[]) {
            let param_id = as_u64_or(json_param.json_get("param_id"), 0) as u8;
            let field_name = as_string_or_die(json_param.json_get("field"));
            let kit_offset = as_u64_or_die(dc.layout_field_or_die("kit", field_name).json_get("offset")) as usize;
            result.push(MasterFxParam {
                fx_name: fx_name.clone(),
                fx_fullname: fx_fullname.clone(),
                block_byte,
                name: as_string_or(json_param.json_get("name"), "").to_string(),
                fullname: as_string_or(json_param.json_get("fullname"), "").to_string(),
                param_id,
                default_val: json_param.json_get("default_val").and_then(serde_json::Value::as_i64).unwrap_or(0) as i32,
                display_offset: json_param
                    .json_get("display_offset")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0) as i32,
                kit_offset,
            });
        }
    }
    result
}

/// Resolves the AUTO-track placeholder (-1) to a concrete track index by querying the device for its currently selected track.
///
/// Returns the index unchanged when it is already concrete.
/// Returns `None` if the AUTO query gets no response (callers abort).
pub fn resolve_auto_track(track_idx: i32, hw: &HwConfig, midi_in: &PollableMidiInput, midi_out: &mut MidiSender) -> Option<i32> {
    if track_idx != -1 {
        return Some(track_idx);
    }
    request_selected_track(
        midi_in,
        midi_out,
        hw.prod,
        0,
        hw.status_query_cmd,
        hw.status_reply_cmd,
        hw.status_selected_track_cmd,
    )
    .map(|val| val as i32)
}

/// Writes a kit to the device's workspace slot so it immediately becomes the live kit.
pub fn write_kit_to_workspace(hw: &HwConfig, midi_out: &mut MidiSender, kit: &[u8]) {
    let mut kit_vec = kit.to_vec();
    patch_elektron_original_position(&mut kit_vec, 9, hw.kit_workspace_slot);
    midi_out.sysex(&kit_vec);
    sleep(Duration::from_millis(100));
}

/// Per-parameter CC addressing.
///
/// Holds (MIDI CC number, sequential byte offset into the track-data blob).
type CcBlobOffsets = Vec<(u8, usize)>;

/// Per-track MIDI addressing, indexed by track number.
///
/// Holds (MIDI channel, track offset).
type TrackChannelOffsets = Vec<(u8, u8)>;

/// Builds the CC mapping tables for `apply_track_via_cc()` from the device config.
pub fn extract_cc_meta(dc: &DeviceConfig) -> (CcBlobOffsets, TrackChannelOffsets) {
    // Build (`cc_id`, `sequential_blob_index`) pairs. Params are stored sequentially in the blob with no gaps.
    // CC numbering gaps (e.g. MnM 64-71, 96-103) are NOT reflected in the binary layout.
    //
    // `pages.lfo` is chained after `pages.synth` for CC-transport devices (MnM).
    // For SysEx-transport devices (MD), `pages.lfo` is absent so the chain adds nothing.
    let pages = as_array_or(dc.json_get("pages.synth"), &[]);
    let lfo_pages = as_array_or(dc.json_get("pages.lfo"), &[]);
    let mut cc_ids: CcBlobOffsets = Vec::new();
    let mut byte_idx = 0usize;
    for page in pages.iter().chain(lfo_pages.iter()) {
        let params = as_array_or_die(page.json_get("params"));
        for json_param in params {
            let cc_id = as_u64_or_die(json_param.json_get("cc_id")) as u8;
            cc_ids.push((cc_id, byte_idx));
            byte_idx += 1;
        }
    }

    // `pages.standalone` (e.g. "Level") lives at its own fixed offset, not chained after `pages.synth`/`pages.lfo`.
    let levels_offset = kit_blob_offset_or_die(dc, "levels");
    let standalone_pages = as_array_or(dc.json_get("pages.standalone"), &[]);
    for page in standalone_pages {
        let params = as_array_or_die(page.json_get("params"));
        for (i, json_param) in params.iter().enumerate() {
            let cc_id = as_u64_or_die(json_param.json_get("cc_id")) as u8;
            cc_ids.push((cc_id, levels_offset + i));
        }
    }

    let track_channel_offsets: TrackChannelOffsets = dc
        .json_get("tracks")
        .and_then(|value| value.as_array())
        .map(|array| {
            array
                .iter()
                .map(|track| {
                    (
                        as_u64_or(track.json_get("channel"), 0) as u8,
                        as_u64_or(track.json_get("offset"), 0) as u8,
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    (cc_ids, track_channel_offsets)
}

/// Pastes a track to the hardware workspace by sending individual CC/SysEx parameter writes.
///
/// For MD, the kit-dump approach triggers the undo system (pre-paste state reverts on the second manual trigger).
/// CC + SysEx writes bypass the undo system and modify the live workspace directly.
/// For MnM, all params including LFO knobs are CC-accessible, so this path works for both devices.
///
/// `cc_ids` lists the CC numbers covering `pages.synth` (and `pages.lfo` for devices with transport="cc").
/// `track_channel_offsets` holds (channel, offset) per track index.
pub fn apply_track_via_cc(
    hw: &HwConfig,
    dc: &DeviceConfig,
    midi_in: &PollableMidiInput,
    midi_out: &mut MidiSender,
    sound_data: &[u8],
    src_track: Option<usize>,
    dest_track: usize,
    cc_ids: &CcBlobOffsets,
    track_channel_offsets: &TrackChannelOffsets,
) -> Option<Vec<u8>> {
    let mut sound_data_vec = sound_data.to_vec();
    redirect_self_targeting_lfo(&mut sound_data_vec, src_track, dest_track);

    let (ch, offset) = track_channel_offsets.get(dest_track).copied().unwrap_or((0, 0));

    // Assign the machine without resetting parameters, since the CC loop below sends every one anyway.
    // The JSON's `machine_model_byte` locates the machine ID in the device-specific blob layout.
    if let Some(model_byte) = dc
        .json_get("system.machine_model_byte")
        .and_then(serde_json::Value::as_u64)
        .map(|val| val as usize)
        && sound_data_vec.len() > model_byte
    {
        let machine_id = sound_data_vec[model_byte];
        let payload = build_machine_assignment_payload(dc, dest_track as u8, machine_id, None);
        let assign = build_elektron_sysex(hw.prod, 0, &payload);
        midi_out.sysex(&assign);
        sleep(Duration::from_millis(100));
    }

    // Params are stored sequentially in the blob. `blob_idx` is the sequential position built by `extract_cc_meta()`.
    // CC numbering gaps (e.g. MnM 64-71, 96-103) aren't reflected in the binary layout, so cc - `pitch_cc_base` can't be used as the index.
    for &(cc, blob_idx) in cc_ids {
        if blob_idx >= sound_data_vec.len() {
            continue;
        }
        midi_out.midi(&[0xB0 | ch, cc.wrapping_add(offset), sound_data_vec[blob_idx]]);
    }

    // LFO params for SysEx-transport devices (MD): TRACK/PARAM/SHP1/SHP2/UPDTE via SysEx block.
    // Speed/depth/mix sit in the Routing page params block and are already sent via the CC loop.
    //
    // For CC-transport devices (MnM), all LFO params are included in `cc_ids` above.
    if as_string_or(dc.json_get("track_lfo.transport"), "") == "sysex" {
        for param_idx in 0..5 {
            let byte_idx = 29 + param_idx;
            if byte_idx >= sound_data_vec.len() {
                break;
            }
            let sysex = build_elektron_sysex(
                hw.prod,
                0,
                &[
                    hw.lfo_block_byte,
                    (dest_track as u8).wrapping_mul(8).wrapping_add(param_idx as u8),
                    sound_data_vec[byte_idx],
                ],
            );
            midi_out.sysex(&sysex);
        }
    }

    sleep(Duration::from_millis(50));
    // The device applies these params to its live sound engine, but the workspace dump doesn't echo live CC tweaks.
    // So fetch the kit for the other tracks and splice in the bytes just sent for this one.
    let FetchResult { workspace: kit, .. } = fetch_kit_for_display(midi_in, midi_out, hw, dc)?;
    Some(apply_sound_to_kit(&kit, &sound_data_vec, dest_track, dc))
}

/// Fetches the live workspace kit for display/extraction: a status query for the current slot label plus a single workspace dump.
///
/// Both devices expose the working kit directly (MD via slot >= 64, MnM via the `$40` workspace flag), so no saved-slot merge is needed.
/// The MnM dump is normalized to the standard saved format so downstream code treats it as an ordinary kit.
pub fn fetch_kit_for_display(
    midi_in: &PollableMidiInput,
    midi_out: &mut MidiSender,
    hw: &HwConfig,
    dc: &DeviceConfig,
) -> Option<FetchResult> {
    let slot = request_status_param(
        midi_in,
        midi_out,
        hw.prod,
        0,
        hw.status_query_cmd,
        hw.status_reply_cmd,
        hw.status_kit_cmd,
        2.0,
    )
    .unwrap_or(0);
    let raw = fetch_elektron_slot(
        midi_in,
        midi_out,
        hw.prod,
        0,
        hw.kit_request_cmd,
        hw.kit_dump_cmd,
        hw.kit_workspace_slot,
        6.0,
        &hw.kit_workspace_extra,
    )?;
    let workspace = if dc.is_device("MnM") {
        normalize_mnm_workspace(&raw, dc)
    } else {
        raw
    };
    Some(FetchResult { slot, workspace })
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// Reads the `cmd` byte of one status param.
///
/// Returns `None` when the device declares no param under that key.
fn status_param_cmd(dc: &DeviceConfig, key: &str) -> Option<u8> {
    dc.json_get(&format!("sysex_api.status.params.{key}.cmd"))
        .and_then(serde_json::Value::as_u64)
        .map(|cmd| cmd as u8)
}

/// Parses one joystick axis from the device JSON.
///
/// Returns `None` when the axis is absent or has no recognized type.
fn parse_joy_axis(dc: &DeviceConfig, axis: &str) -> Option<JoystickAxis> {
    match as_string_or(dc.json_get(&format!("system.joystick.{axis}.type")), "") {
        "pitch_bend" => Some(JoystickAxis::PitchBend),
        "cc" => Some(JoystickAxis::Cc(
            as_u64_or_die(dc.json_get(&format!("system.joystick.{axis}.cc"))) as u8
        )),
        _ => None,
    }
}
