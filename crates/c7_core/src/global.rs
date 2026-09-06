//! Global settings domain: SysEx fetch/patch and scalar field layout.

use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::device_config::DeviceConfig;
use crate::midi::{MidiSender, PollableMidiInput};
use crate::sysex::{ELEKTRON_TYPE_BYTE, build_elektron_sysex, is_elektron_sysex, patch_global_checksum_14bit};
use crate::utils::{JsonPath, as_string_or, as_u64_or, as_u64_or_die};

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// A scalar field from `sysex_layout.global.fields` (array-shaped fields get their own table UI).
#[derive(Clone)]
pub struct GlobalParamField {
    pub fullname: String,
    pub offset: usize,
    pub min: i8,
    pub max: i8,
    pub mask: Option<u8>,
    pub shift: u8,
    pub display_offset: i8,
    pub labels: Option<Vec<(String, u8)>>,
}

impl GlobalParamField {
    /// Reads this field's value from `buffer`, offset by `base`.
    /// `base` is 0 for MnM's decoded buffer, or past the header/version/revision/slot preamble for MD's raw one.
    pub fn read(&self, buffer: &[u8], base: usize) -> u8 {
        let byte = buffer[base + self.offset];
        match self.mask {
            Some(mask_val) => (byte & mask_val) >> self.shift,
            None => byte,
        }
    }

    /// Computes the new byte, preserving other bits sharing it (e.g. MD's program-change mode/channel pair).
    pub fn masked_byte(&self, current: u8, val: u8) -> u8 {
        match self.mask {
            Some(mask_val) => (current & !mask_val) | ((val << self.shift) & mask_val),
            None => val,
        }
    }
}

/// Sends a Global-slot request and polls for the write-cmd response, giving up after 1 second.
pub fn request_global(midi_in: &PollableMidiInput, midi_out: &mut MidiSender, ch: u8, dc: &DeviceConfig, slot: u8) -> Option<Vec<u8>> {
    while midi_in.poll().is_some() {}

    let request_cmd = as_u64_or_die(dc.json_get("sysex_api.global.request_cmd")) as u8;
    let write_cmd = as_u64_or_die(dc.json_get("sysex_api.global.write_cmd")) as u8;
    let msg = build_elektron_sysex(dc.prod, ch, &[request_cmd, slot]);
    midi_out.sysex(&msg);

    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if let Some(raw) = midi_in.poll()
            && is_elektron_sysex(&raw, dc.prod)
            && raw.len() > 7
            && raw[ELEKTRON_TYPE_BYTE] == write_cmd
        {
            return Some(raw);
        }
        sleep(Duration::from_millis(5));
    }
    None
}

/// Patches one byte in a Global SysEx blob, adjusts the rolling checksum, and sends it.
///
/// Used by every single-byte Global write (seq mode, knob res, base channel, local control, etc.).
pub fn patch_global_byte(raw: &mut [u8], offset: usize, new_val: u8, midi_out: &mut MidiSender) {
    let old_val = raw[offset] as i32;
    patch_global_checksum_14bit(raw, old_val, new_val as i32);
    raw[offset] = new_val;
    midi_out.sysex(raw);
}

/// Collects the scalar fields of the Global payload, skipping array fields and `base_frequency`.
/// `base_frequency` is handled by each device's own master-tune field.
///
/// Offsets come back payload-relative, so a caller reading them out of a received message adds the payload's own offset first.
pub fn collect_global_scalar_fields(dc: &DeviceConfig) -> Vec<GlobalParamField> {
    dc.json_get("sysex_layout.global.fields.payload_structure")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(name, field)| {
            if name == "base_frequency" {
                return None;
            }
            if field.json_get("per_track").is_some() || field.json_get("size").is_some() || field.json_get("decoded_size").is_some() {
                return None;
            }
            let fullname = as_string_or(field.json_get("fullname"), name.as_str()).to_string();

            let offset = as_u64_or(field.json_get("offset"), 0) as usize;
            let min = field.json_get("min").and_then(serde_json::Value::as_i64).unwrap_or(0) as i8;
            let max = field.json_get("max").and_then(serde_json::Value::as_i64).unwrap_or(127) as i8;
            let mask = field
                .json_get("mask")
                .and_then(serde_json::Value::as_u64)
                .map(|mask_val| mask_val as u8);
            let shift = as_u64_or(field.json_get("shift"), 0) as u8;
            let display_offset = field.json_get("display_offset").and_then(serde_json::Value::as_i64).unwrap_or(0) as i8;

            let labels_key = as_string_or(field.json_get("values_ref"), name.as_str()).to_string();
            let labels = dc
                .json_get(&format!("sysex_layout.global.constants.{labels_key}"))
                .and_then(|val| val.as_array())
                .map(|array| {
                    array
                        .iter()
                        .enumerate()
                        .map(|(i, entry)| match entry.as_object() {
                            Some(_) => (
                                as_string_or(entry.json_get("label"), "").to_string(),
                                as_u64_or(entry.json_get("value"), i as u64) as u8,
                            ),
                            None => (as_string_or(Some(entry), "").to_string(), i as u8),
                        })
                        .collect::<Vec<_>>()
                });

            Some(GlobalParamField {
                fullname,
                offset,
                min,
                max,
                mask,
                shift,
                display_offset,
                labels,
            })
        })
        .collect()
}
