//! Builds the DAW-facing parameter set from a device JSON, and the MIDI messages those params emit.
//!
//! Every automatable device value becomes an `IntParam` whose change callback queues an `OutMsg`.

use std::sync::Arc;

use nih_plug::prelude::*;

use c7_core::bridge_interfacing::{MSG_MACHINE, MSG_SYSEX_PARAM};
use c7_core::device_config::DeviceConfig;
use c7_core::kit::{machine_list, master_fx_list};
use c7_core::sysex::{build_elektron_sysex, build_machine_assignment_payload};
use c7_core::utils::slug_string;
use c7_core::utils::{JsonPath, as_array_or, as_bool_or, as_string_or, as_u64_or};

/// A raw Elektron SysEx message as nih-plug output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawSysEx {
    data: [u8; 16],
    len: usize,
}

impl RawSysEx {
    /// Wraps a built SysEx message.
    fn new(msg: &[u8]) -> Self {
        let mut data = [0u8; 16];
        data[..msg.len()].copy_from_slice(msg);
        Self { data, len: msg.len() }
    }
}

impl SysExMessage for RawSysEx {
    type Buffer = [u8; 16];

    /// Hands nih-plug the raw bytes to place on the DAW's MIDI output.
    fn to_buffer(self) -> (Self::Buffer, usize) {
        (self.data, self.len)
    }

    /// The plugin emits SysEx but has no reason to parse any coming in.
    fn from_buffer(_buffer: &[u8]) -> Option<Self> {
        None
    }
}

/// One queued MIDI output event.
///
/// Param callbacks push these, and `process()` emits them to the DAW.
///
/// A knob names the track it belongs to rather than a MIDI channel, so every outgoing message resolves its channel in the same place.
/// SysEx carries its 4-byte monitor frame too, since the raw bytes alone don't say whether it was a master-FX edit or a machine assign.
pub(crate) enum OutMsg {
    Cc { track_idx: u8, cc: u8, val: u8 },
    Sysex { msg: RawSysEx, frame: [u8; 4] },
}

/// One track's note routing from the device JSON.
pub(crate) struct TrackRoute {
    channel: u8,
    note: Option<u8>,
}

/// The MIDI channel the track at `index` answers on, or `None` when the device has no track there.
///
/// A DAW channel addresses a track by position, not by the channel the hardware listens on.
/// Knobs, notes and pitch bend all resolve their outgoing channel through here, so none of them can drift from the others.
pub(crate) fn device_channel(tracks: &[TrackRoute], index: u8) -> Option<u8> {
    tracks.get(index as usize).map(|track| track.channel)
}

/// All plugin state shared with the DAW: the generated params plus the note-routing table.
///
/// `Params` is implemented by hand because the param set comes from the device JSON, and the derive macro only covers fixed struct fields.
pub(crate) struct BridgeParams {
    /// The generated device params.
    params: Box<[IntParam]>,
    /// Stable per-param ID strings (same index as `params`), which DAW project recall keys on.
    ids: Box<[String]>,
    /// CLAP module path per param (Like "Track 1/Synthesis").
    groups: Box<[String]>,
    /// Per-track routing, in JSON declaration order, indexed by the DAW's channel number.
    pub(crate) tracks: Vec<TrackRoute>,
    /// `true` when the tracks declare fixed trig notes (MD/AR).
    /// `false` when the channel picks the track, and the note is the pitch (MnM).
    pub(crate) has_trig_notes: bool,
    /// `true` when the device JSON says the hardware acts on incoming pitch bend.
    pub(crate) can_pitch_bend: bool,
}

unsafe impl Params for BridgeParams {
    /// Hands the DAW every generated param with its stable ID and module path.
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        (0..self.params.len())
            .map(|i| (self.ids[i].clone(), self.params[i].as_ptr(), self.groups[i].clone()))
            .collect()
    }
}

impl BridgeParams {
    /// Translates incoming DAW notes into the (channel, note) the device answers to, or `None` when nothing does.
    ///
    /// Trig-note devices (MD/AR) match the note against each track's fixed trig note and re-channel it.
    /// Pitched devices (MnM/A4) take the DAW channel as a track position and re-channel it. Each track is its own keyboard.
    pub(crate) fn route_note(&self, channel: u8, note: u8) -> Option<(u8, u8)> {
        if self.has_trig_notes {
            let track = self.tracks.iter().find(|track| track.note == Some(note))?;
            Some((track.channel, note))
        }
        // CLAP doesn't have a method to tell the DAW to only display the number of channels that the device supports.
        // Because of this, notes sent on invalid channels are filtered out.
        else {
            Some((device_channel(&self.tracks, channel)?, note))
        }
    }
}

/// Builds a CLAP module-path segment from `track_name`, prefixing it with "Track" only when the name doesn't already say so.
///
/// Some device tracknames already read "Track N" (A4/OT/SS/MnM).
fn track_group_label(track_name: &str) -> String {
    if track_name.starts_with("Track ") {
        track_name.to_string()
    } else {
        format!("Track {track_name}")
    }
}

/// Builds every knobbed param from the device JSON.
///
/// A machine-assign param, then the CC params from `pages.synth` and `pages.lfo`, then the master-FX SysEx params.
pub(crate) fn build_params(dc: &'static DeviceConfig, tx: &async_channel::Sender<OutMsg>) -> BridgeParams {
    let mut params = Vec::new();
    let mut ids = Vec::new();
    let mut groups = Vec::new();
    let mut tracks = Vec::new();

    let machines: Arc<Vec<(u8, String)>> = Arc::new(machine_list(dc));
    let can_pitch_bend = as_bool_or(dc.json_get("system.pitch_bend_input"), false);

    // Standalone-page CCs sit outside the track's CC block, numbering consecutively within a channel instead of stepping by `offset`.
    let pages: Vec<(&serde_json::Value, bool)> = as_array_or(dc.json_get("pages.standalone"), &[])
        .iter()
        .map(|page| (page, true))
        .chain(as_array_or(dc.json_get("pages.synth"), &[]).iter().map(|page| (page, false)))
        .chain(as_array_or(dc.json_get("pages.lfo"), &[]).iter().map(|page| (page, false)))
        .collect();

    let json_tracks = as_array_or(dc.json_get("tracks"), &[]);
    // A one-track device has nothing to tell apart, so naming the track in every param and group would only add noise.
    let has_one_track = json_tracks.len() == 1;

    let mut has_trig_notes = false;
    for (track_idx, track) in json_tracks.iter().enumerate() {
        let track_name = as_string_or(track.json_get("trackname"), "");
        let offset = as_u64_or(track.json_get("offset"), 0) as u8;
        let channel = as_u64_or(track.json_get("channel"), 0) as u8;
        let note = track.json_get("note").and_then(serde_json::Value::as_u64);
        has_trig_notes |= note.is_some();

        // A track's slot among those sharing its channel, which is how the MD numbers its packed Level CCs.
        let channel_slot = json_tracks[..track_idx]
            .iter()
            .filter(|other| as_u64_or(other.json_get("channel"), 0) as u8 == channel)
            .count() as u8;

        tracks.push(TrackRoute {
            channel,
            note: note.map(|note_val| note_val as u8),
        });

        if !machines.is_empty() {
            params.push(make_machine_param(track_name, track_idx as u8, &machines, dc, tx));
            ids.push(format!("{}_machine", slug_string(track_name)));
            groups.push(track_group_label(track_name));
        }

        for (page, is_standalone) in &pages {
            let page_fullname = as_string_or(page.json_get("fullname"), "");
            for json_param in as_array_or(page.json_get("params"), &[]) {
                // Params without a `cc_id` have no wire path, so there's nothing for the DAW to automate.
                let Some(cc) = json_param.json_get("cc_id").and_then(serde_json::Value::as_u64) else {
                    continue;
                };
                let name = as_string_or(json_param.json_get("name"), "");
                let param_name = if has_one_track {
                    name.to_string()
                } else {
                    format!("{track_name} {name}")
                };
                // Only combo params carry a list the DAW can name positions from.
                // A "shape" widget draws its names from elsewhere in the JSON, so it stays a plain 0-127 knob.
                let options: Vec<String> = if as_string_or(json_param.json_get("widget"), "") == "combo" {
                    as_array_or(json_param.json_get("options"), &[])
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                } else {
                    Vec::new()
                };
                let cc_step = if *is_standalone { channel_slot } else { offset };
                params.push(make_cc_param(
                    param_name,
                    track_idx as u8,
                    (cc as u8).wrapping_add(cc_step),
                    options,
                    tx,
                ));
                ids.push(format!(
                    "{}_{}_{}",
                    slug_string(track_name),
                    slug_string(page_fullname),
                    slug_string(name)
                ));
                groups.push(if has_one_track {
                    page_fullname.to_string()
                } else {
                    format!("{}/{page_fullname}", track_group_label(track_name))
                });
            }
        }
    }

    for master_fx_param in master_fx_list(dc) {
        params.push(make_master_fx_param(
            format!("{} {}", master_fx_param.fx_name, master_fx_param.name),
            dc.prod,
            master_fx_param.block_byte,
            master_fx_param.param_id,
            tx,
        ));
        ids.push(format!(
            "fx_{}_{}",
            slug_string(&master_fx_param.fx_name),
            slug_string(&master_fx_param.name)
        ));
        groups.push(format!("Master Effects/{}", master_fx_param.fx_fullname));
    }

    BridgeParams {
        params: params.into_boxed_slice(),
        ids: ids.into_boxed_slice(),
        groups: groups.into_boxed_slice(),
        tracks,
        has_trig_notes,
        can_pitch_bend,
    }
}

/// Builds one CC param whose change callback queues its MIDI CC event.
///
/// Passing `options` turns the param into a stepped list that reads back as a name in the DAW.
/// The machine param does exactly this, showing "TRX-BD" instead of a number.
/// The kit editor's combo widgets send the selected position as the CC value, so the range narrows to match the number of options.
/// An empty `options` leaves the param spanning the full 0-127 CC range.
///
/// Starts (and DAW-resets) at 0, not the JSON `default_val`: the plugin can't know the device's real values.
/// Zero is the blank slate, matching the kit editor's uninitiated-at-zero knobs.
fn make_cc_param(name: String, track_idx: u8, cc: u8, options: Vec<String>, tx: &async_channel::Sender<OutMsg>) -> IntParam {
    let tx = tx.clone();
    let option_count = options.len();
    let max = if options.is_empty() { 127 } else { option_count as i32 - 1 };
    let param = IntParam::new(name, 0, IntRange::Linear { min: 0, max }).with_callback(Arc::new(move |val| {
        // The device splits the CC's 0-127 span evenly between a stepped param's options.
        // So the wire value is the middle of the option's band, not its position.
        let wire_val = if option_count == 0 {
            val.clamp(0, 127) as u8
        } else {
            ((f64::from(val.max(0)) + 0.5) * 128.0 / option_count as f64).min(127.0) as u8
        };
        let _ = tx.try_send(OutMsg::Cc {
            track_idx,
            cc,
            val: wire_val,
        });
    }));
    if options.is_empty() {
        return param;
    }

    let to_string = Arc::new(options);
    let to_value = Arc::clone(&to_string);
    param
        .with_value_to_string(Arc::new(move |val| {
            to_string.get(val.max(0) as usize).cloned().unwrap_or_else(|| val.to_string())
        }))
        .with_string_to_value(Arc::new(move |string_val| {
            to_value.iter().position(|option| option == string_val).map(|i| i as i32)
        }))
}

/// Builds one 0-127 master-FX param whose change callback queues its SysEx event.
fn make_master_fx_param(name: String, prod: u8, block: u8, param_id: u8, tx: &async_channel::Sender<OutMsg>) -> IntParam {
    let tx = tx.clone();
    IntParam::new(name, 0, IntRange::Linear { min: 0, max: 127 }).with_callback(Arc::new(move |val| {
        let val = val.clamp(0, 127) as u8;
        let msg = build_elektron_sysex(prod, 0, &[block, param_id, val]);
        let _ = tx.try_send(OutMsg::Sysex {
            msg: RawSysEx::new(&msg),
            frame: [MSG_SYSEX_PARAM, block, param_id, val],
        });
    }))
}

/// Builds a track's machine-assign param.
///
/// Value 0 is the "--" default (no assign), values 1..=n index the machine list.
///
/// The DAW renders the machine names through `value_to_string`, so its UI shows "TRX-BD" rather than a number.
fn make_machine_param(
    track_name: &str,
    track_idx: u8,
    machines: &Arc<Vec<(u8, String)>>,
    dc: &'static DeviceConfig,
    tx: &async_channel::Sender<OutMsg>,
) -> IntParam {
    let tx = tx.clone();
    let to_frame = Arc::clone(machines);
    let to_string = Arc::clone(machines);
    let to_value = Arc::clone(machines);
    IntParam::new(
        format!("{track_name} Machine"),
        0,
        IntRange::Linear {
            min: 0,
            max: to_frame.len() as i32,
        },
    )
    .with_value_to_string(Arc::new(move |val| {
        if val <= 0 {
            "--".to_string()
        } else {
            to_string[(val - 1) as usize].1.clone()
        }
    }))
    .with_string_to_value(Arc::new(move |string_val| {
        if string_val == "--" {
            Some(0)
        } else {
            to_value.iter().position(|(_, name)| name == string_val).map(|i| i as i32 + 1)
        }
    }))
    .with_callback(Arc::new(move |val| {
        // 0 is the placeholder. Selecting it assigns nothing, so a fresh instance stays hands-off until the user picks a machine.
        if val >= 1 {
            let machine_id = to_frame[(val - 1) as usize].0;
            let payload = build_machine_assignment_payload(dc, track_idx, machine_id, None);
            let msg = build_elektron_sysex(dc.prod, 0, &payload);
            let _ = tx.try_send(OutMsg::Sysex {
                msg: RawSysEx::new(&msg),
                frame: [MSG_MACHINE, track_idx, machine_id, 0],
            });
        }
    }))
}
