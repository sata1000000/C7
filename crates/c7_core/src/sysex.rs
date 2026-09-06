//! Functions for the Elektron SysEx codec.
//!
//! The request/response helpers (`fetch_elektron_slot()`, `request_*`) take `midi.rs`'s poller and sender instead of opening a port.
//! Callers run them inside a `run_midi_session()` closure, which owns the connection for as long as the exchange takes.

use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::device_config::DeviceConfig;
use crate::midi::{MidiSender, PollableMidiInput};
use crate::utils::{JsonPath, as_u64_or_die};

/// Elektron SysEx manufacturer ID.
pub const ELEKTRON_MFR: [u8; 4] = [0xF0, 0x00, 0x20, 0x3C];

// Byte offsets inside an Elektron SysEx header (`F0 00 20 3C prod ch type`).
pub const ELEKTRON_PROD_BYTE: usize = 4;
pub const ELEKTRON_CHANNEL_BYTE: usize = 5;
pub const ELEKTRON_TYPE_BYTE: usize = 6;

// First byte after the header. What it holds depends on the message: a version, an echoed query param, a slot, a subcommand.
pub const ELEKTRON_PAYLOAD_START: usize = 7;

/// Mask for the Elektron checksum, which is stored across two 7-bit MIDI SysEx bytes (high 7 bits + low 7 bits = 14 usable bits).
pub const ELEKTRON_CHECKSUM_MASK: u32 = 0x3FFF;

/// Characters allowed when writing kit/song names.
const ELEKTRON_ALLOWED_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789+-=&/().!?";

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Uppercases `input` and strips any character not in `ELEKTRON_ALLOWED_CHARS`.
///
/// Shared by anything that writes or previews an Elektron name field.
pub fn filter_elektron_name(input: &str) -> String {
    input
        .chars()
        .map(|char_val| char_val.to_uppercase().next().unwrap_or(char_val))
        .filter(|char_val| ELEKTRON_ALLOWED_CHARS.contains(*char_val))
        .collect()
}

/// Splits a binary blob into individual SysEx messages by searching for F0/F7 delimiters.
pub fn parse_sysex_file(data: &[u8]) -> Vec<Vec<u8>> {
    let mut msgs = Vec::new();
    let mut start = 0;
    while start < data.len() {
        // Locate the next F0 marker.
        let Some(f0_relative) = data[start..].iter().position(|&byte| byte == 0xF0) else {
            break;
        };
        let f0 = start + f0_relative;
        // Locate the matching F7 terminator.
        let Some(f7_relative) = data[f0..].iter().position(|&byte| byte == 0xF7) else {
            break;
        };
        let f7 = f0 + f7_relative;
        msgs.push(data[f0..=f7].to_vec());
        start = f7 + 1;
    }
    msgs
}

/// Extracts the name tag from a Machinedrum or Monomachine SysEx dump.
///
/// Returns `None` on invalid/unrecognized input.
/// Returns `Some("--")` for confirmed-empty slots.
pub fn extract_sysex_name(raw: &[u8]) -> Option<String> {
    if raw.is_empty() {
        return None;
    }

    // Locate the first F7 delimiter to isolate the initial SysEx message in a multi-message stream.
    let f7 = raw.iter().position(|&byte| byte == 0xF7)?;
    let first = &raw[..=f7];

    // Validate Elektron SysEx header and minimum length.
    if first.len() < 10 || first[0] != 0xF0 || first[1..4] != [0x00, 0x20, 0x3C] {
        return None;
    }

    let dc = DeviceConfig::find_by_prod(first[ELEKTRON_PROD_BYTE])?;
    let name_bytes: Vec<u8>;

    // Handling the Monomachine format.
    if dc.is_device("MnM") {
        let version = first[ELEKTRON_PAYLOAD_START];
        let packed_start = if version == 64 { 11 } else { 10 };
        if first.len() < packed_start + 8 + 5 {
            return None;
        }
        let packed = &first[packed_start..first.len() - 5];
        let unpacked = unpack_7bit(packed, 0);
        if unpacked.is_empty() {
            return None;
        }
        let decoded = decode_rle7(&unpacked, Some(11));
        if decoded.is_empty() || decoded[0] == 0x7F || decoded[0] == 0xFF {
            return Some("--".to_owned());
        }
        name_bytes = decoded[..11.min(decoded.len())].to_vec();
    }
    // Handling the Machinedrum format.
    else if dc.is_device("MD") {
        let name_offset = dc.layout_offset_or_die("kit", "name") as usize;
        let name_size = as_u64_or_die(dc.layout_field_or_die("kit", "name").json_get("size")) as usize;
        if first.len() < name_offset + name_size + 1 {
            return None;
        }
        if first[name_offset] == 0x7F {
            return Some("--".to_owned());
        }
        name_bytes = first[name_offset..name_offset + name_size].to_vec();
    }
    // Every other device. Only the MD and MnM name formats are handled here.
    else {
        return None;
    }

    let name_bytes: Vec<u8> = name_bytes.iter().copied().take_while(|&byte| byte != 0).collect();
    let chars: String = name_bytes.iter().filter_map(|&byte| elektron_lcd_char(byte)).collect();
    let trimmed = chars.trim().to_owned();
    Some(if trimmed.is_empty() { "--".to_owned() } else { trimmed })
}

/// Checks if a pattern data block represents an empty, unused slot.
///
/// "Empty" means no trigger steps are set and no param locks are active.
///
/// `raw[10]` is the MSB byte of the first 7-bit-encoded trig group, not a name field.
/// So the `0x7F` empty marker used for kits/songs does NOT apply here.
pub fn is_empty_pattern(raw: &[u8]) -> bool {
    if raw.len() < 11 {
        return true;
    }

    let device = find_device_from_pattern_dump(raw);

    // Inspection for Machinedrumm patterns.
    // Check for trig data and param locks existence.
    if raw.len() >= 84 && device == "MD" {
        // `raw[10..84]`: 74 wire bytes → 64 decoded bytes (16 tracks * 4 bytes each).
        // Any non-zero decoded byte means at least one trig step is set.
        let trig_decoded = decode_7bit(&raw[10..84]);
        if trig_decoded.iter().any(|&byte| byte != 0) {
            return false;
        }
        // `raw[182]` = numLockedRows.
        // If > 0, at least one param lock occupies a lock slot: step-level parameter automation, even with no trigger steps.
        if raw.len() > 182 && raw[182] != 0 {
            return false;
        }
        return true;
    }

    // Inspection for Monomachine patterns.
    // Most empty patterns still have a valid header but no sequencer data.
    if device == "MnM" {
        let version = raw[ELEKTRON_PAYLOAD_START];
        let packed_start = if version == 64 { 11 } else { 10 };
        let end = raw.len() - 5;
        if packed_start >= end {
            return true;
        }
        let packed = &raw[packed_start..end];
        let unpacked = unpack_7bit(packed, 0);
        if unpacked.is_empty() {
            return true;
        }
        // Only the 'used' counters (around 1367 bytes) matter here. Trig bits are in the first 432 bytes.
        let decoded = decode_rle7(&unpacked, Some(1367));
        if decoded.is_empty() {
            return true;
        }
        // Check trig bits (first 432 bytes)
        if decoded[..432.min(decoded.len())].iter().any(|&byte| byte != 0) {
            return false;
        }
        // Check "used" counters (MIDI notes, chord notes, locks)
        if decoded.len() >= 1367 {
            let midi_used = ((decoded[1362] as u16) << 8) | decoded[1363] as u16;
            let chord_used = decoded[1364];
            let locks_used = decoded[1366];
            if midi_used > 0 || chord_used > 0 || locks_used > 0 {
                return false;
            }
        }
        return true;
    }

    raw.len() < 20 || raw[10..20].iter().all(|&byte| byte == 0)
}

/// Converts a numerical slot index into a letter-number pattern label (e.g. "A01" or "B16").
pub fn pattern_slot_label(slot: usize) -> String {
    const BANK_START: char = 'A';
    const PER_BANK: usize = 16;

    let bank = char::from_u32(BANK_START as u32 + (slot / PER_BANK) as u32).unwrap();
    let number = (slot % PER_BANK) + 1;
    format!("{bank}{number:02}")
}

/// Patches an Elektron SysEx blob's Original Position (slot) byte and recalculates the 14-bit checksum.
pub fn patch_elektron_original_position(blob: &mut [u8], position_idx: usize, new_slot: u8) {
    let new_slot = new_slot & 0x7F;
    if blob[position_idx] == new_slot {
        return;
    }
    blob[position_idx] = new_slot;
    let len = blob.len();
    let checksum = elektron_checksum_14bit(blob);
    blob[len - 5] = ((checksum >> 7) & 0x7F) as u8;
    blob[len - 4] = (checksum & 0x7F) as u8;
}

/// Requests a specific hardware slot dump and blocks until it arrives.
pub fn fetch_elektron_slot(
    midi_in: &PollableMidiInput,
    midi_out: &mut MidiSender,
    prod: u8,
    ch: u8,
    request_cmd: u8,
    dump_cmd: u8,
    slot: u8,
    timeout_secs: f64,
    extra_payload: &[u8],
) -> Option<Vec<u8>> {
    // Clear stale messages before sending the request.
    while midi_in.poll().is_some() {}

    let mut payload = vec![request_cmd, slot & 0x7F];
    payload.extend_from_slice(extra_payload);
    midi_out.sysex(&build_elektron_sysex(prod, ch, &payload));

    let deadline = Instant::now() + Duration::from_secs_f64(timeout_secs);
    while Instant::now() < deadline {
        if let Some(raw) = midi_in.poll()
            && is_elektron_sysex(&raw, prod)
            && raw.len() > 7
            && raw[ELEKTRON_TYPE_BYTE] == dump_cmd
        {
            return Some(raw);
        }
        sleep(Duration::from_millis(5));
    }
    None
}

/// Queries the hardware for the exact tempo setting and returns it.
pub fn request_tempo(midi_in: &PollableMidiInput, midi_out: &mut MidiSender, dc: &DeviceConfig, ch: u8) -> Option<u32> {
    let query_cmd = as_u64_or_die(dc.json_get("sysex_api.status.query_cmd")) as u8;
    let reply_cmd = as_u64_or_die(dc.json_get("sysex_api.status.reply_cmd")) as u8;
    let bpm_query_cmd = as_u64_or_die(dc.json_get("sysex_api.bpm_query_cmd")) as u8;

    let msg = build_elektron_sysex(dc.prod, ch, &[query_cmd, bpm_query_cmd]);
    midi_out.sysex(&msg);
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if let Some(raw) = midi_in.poll()
            && raw.len() >= 11
            && raw[ELEKTRON_TYPE_BYTE] == reply_cmd
            && raw[ELEKTRON_PAYLOAD_START] == bpm_query_cmd
        {
            return Some(((raw[8] as u32) << 7) | (raw[9] as u32));
        }
        sleep(Duration::from_millis(5));
    }
    None
}

/// Requests the track the device has selected on its display.
///
/// Returns the track the device reports.
/// Returns `None` if the device names no selected-track status param.
pub fn request_selected_track(
    midi_in: &PollableMidiInput,
    midi_out: &mut MidiSender,
    prod: u8,
    ch: u8,
    status_query_cmd: u8,
    status_reply_cmd: u8,
    selected_track_cmd: Option<u8>,
) -> Option<u8> {
    let cmd = selected_track_cmd?;
    request_status_param(midi_in, midi_out, prod, ch, status_query_cmd, status_reply_cmd, cmd, 2.0)
}

/// Adjusts the LFO destination track in a track blob when the LFO was targeting itself.
///
/// Pass `None` for `src` when no source is known (cancel/revert path, where no patching is needed).
pub fn redirect_self_targeting_lfo(blob: &mut [u8], src: Option<usize>, dst: usize) {
    let Some(src) = src else { return };
    if blob.len() > 29 && blob[29] as usize == src {
        blob[29] = dst as u8;
    }
}

/// Patches the 14-bit rolling checksum in a Global SysEx blob based on a value change.
pub fn patch_global_checksum_14bit(raw: &mut [u8], old_sum: i32, new_sum: i32) {
    let len = raw.len();
    let old_checksum = ((raw[len - 5] as i32) << 7) | (raw[len - 4] as i32);
    let new_checksum = old_checksum + (new_sum - old_sum);
    raw[len - 5] = ((new_checksum >> 7) & 0x7F) as u8;
    raw[len - 4] = (new_checksum & 0x7F) as u8;
}

/// Unpacks and decompresses a whole-payload RLE+7bit kit so its individual parts can be read.
pub fn decode_mnm_kit(kit_sysex: &[u8], decoded_size: usize) -> (Vec<u8>, usize) {
    let version = kit_sysex[ELEKTRON_PAYLOAD_START];
    let decode_start = if version == 64 { 11 } else { 10 };
    let tail_start = kit_sysex.len() - 5;
    let encoded = &kit_sysex[decode_start..tail_start];
    let seven_bit = decode_7bit(encoded);
    // Uncapped so an unofficial-firmware kit keeps its extra bytes.
    let mut decoded = decode_rle7(&seven_bit, None);
    // The live-workspace dump (version `$40`) omits its final kit-global byte (Split Track), so it decodes one byte short of spec.
    // Pad to `decoded_size` so every field offset stays valid.
    if decoded.len() < decoded_size {
        decoded.resize(decoded_size, 0);
    }
    (decoded, decode_start)
}

/// Re-encodes a decoded payload back through `rle7` then 7-bit, reassembles the SysEx message around it, and refreshes the checksum.
///
/// Works for any `rle7`-encoded dump type, Kit and Global included.
pub fn rebuild_rle7_dump(original_sysex: &[u8], decoded: &[u8], decode_start: usize) -> Vec<u8> {
    let rle_encoded = encode_rle7(decoded);
    let new_encoded = encode_7bit(&rle_encoded);
    let mut new_sysex = original_sysex[..decode_start].to_vec();
    new_sysex.extend_from_slice(&new_encoded);
    new_sysex.extend_from_slice(&[0u8; 5]);
    *new_sysex.last_mut().unwrap() = 0xF7;
    update_elektron_checksum(&mut new_sysex);
    new_sysex
}

/// Returns `true` if `raw` begins with the Elektron manufacturer header and the given product ID.
pub fn is_elektron_sysex(raw: &[u8], prod: u8) -> bool {
    raw.len() >= 5 && raw[0..5] == [0xF0, 0x00, 0x20, 0x3C, prod]
}

/// Decodes variable-length 7-bit MIDI SysEx data back into 8-bit bytes.
///
/// Same group layout as `unpack_7bit()`, but a partial group at the end of the stream is decoded rather than dropped.
pub fn decode_7bit(data: &[u8]) -> Vec<u8> {
    let data: Vec<u8> = data.iter().copied().filter(|&byte| byte < 0xF8 || byte == 0xF7).collect();
    let mut result = Vec::new();
    let len = data.len();
    let mut i = 0;
    // Decode the variable-length 7-bit sections back into standard bytes.
    while i < len {
        let mut bits = data[i] as u32;
        i += 1;
        let count = 7.min(len - i);
        for j in 0..count {
            bits <<= 1;
            result.push((data[i + j] & 0x7F) | ((bits & 0x80) as u8));
        }
        i += count;
    }
    result
}

/// Encodes 8-bit bytes into 7-bit MIDI SysEx format (Elektron reversed-bit order).
///
/// The inverse of `unpack_7bit()`: bit 6 of the header byte carries the high bit of the first data byte, bit 5 the second, and so on.
/// Accepts any input length, including a partial trailing group.
pub fn encode_7bit(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity((data.len() / 7 + 1) * 8);
    let mut i = 0;
    while i < data.len() {
        let group = &data[i..data.len().min(i + 7)];
        let mut msb: u8 = 0;
        for (j, &byte) in group.iter().enumerate() {
            if byte & 0x80 != 0 {
                msb |= 1 << (6 - j);
            }
        }
        out.push(msb);
        for &byte in group {
            out.push(byte & 0x7F);
        }
        i += 7;
    }
    out
}

/// Requests one field of the device's live state, such as the loaded kit or the selected track.
///
/// The fields are the device JSON's `sysex_api.status.params`, and `param_cmd` picks which one.
///
/// Returns the value the device reports for that field.
/// Returns `None` if no reply arrives within `timeout_secs`.
pub fn request_status_param(
    midi_in: &PollableMidiInput,
    midi_out: &mut MidiSender,
    prod: u8,
    ch: u8,
    query_cmd: u8,
    reply_cmd: u8,
    param_cmd: u8,
    timeout_secs: f64,
) -> Option<u8> {
    while midi_in.poll().is_some() {}
    midi_out.sysex(&build_elektron_sysex(prod, ch, &[query_cmd, param_cmd]));

    let deadline = Instant::now() + Duration::from_secs_f64(timeout_secs);
    while Instant::now() < deadline {
        if let Some(raw) = midi_in.poll()
            && raw.len() >= 9
            && raw[ELEKTRON_PROD_BYTE] == prod
            && raw[ELEKTRON_TYPE_BYTE] == reply_cmd
            && raw[ELEKTRON_PAYLOAD_START] == param_cmd
        {
            return Some(raw[8]);
        }
        sleep(Duration::from_millis(5));
    }
    None
}

/// Collapses identical consecutive bytes into the `rle7` scheme.
///
/// Counterpart to `decode_rle7()`.
pub fn encode_rle7(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let byte = data[i];
        let mut run = 1usize;
        while i + run < data.len() && data[i + run] == byte && run < 127 {
            run += 1;
        }

        // Values >= `0x80` are reserved for RLE count markers.
        // Any actual data byte >= `0x80` (like negative mod depths) must be escaped as a run of 1.
        if run > 1 || byte & 0x80 != 0 {
            result.push(0x80 | run as u8);
            result.push(byte);
        } else {
            result.push(byte);
        }

        i += run;
    }
    result
}

/// Recalculates the 14-bit checksum and length fields in place.
pub fn update_elektron_checksum(kit: &mut [u8]) {
    let tail_start = kit.len() - 5;
    let checksum = elektron_checksum_14bit(kit);
    kit[tail_start] = ((checksum >> 7) & 0x7F) as u8;
    kit[tail_start + 1] = (checksum & 0x7F) as u8;
    let len = (tail_start - 5) as u32;
    kit[tail_start + 2] = ((len >> 7) & 0x7F) as u8;
    kit[tail_start + 3] = (len & 0x7F) as u8;
}

/// Decodes a stream of 7-bit packed MIDI data (Elektron reversed-bit order) back into 8-bit bytes.
///
/// Each group is 8 bytes: one header byte holding the high bit of the 7 data bytes that follow.
/// The header fills from bit 6 downward, so the first data byte takes bit 6, the second bit 5, and so on.
/// Only complete groups are processed, so partial trailing data is silently discarded.
fn unpack_7bit(packed: &[u8], start: usize) -> Vec<u8> {
    let mut result = Vec::new();
    let len = packed.len();
    let mut i = start;
    // Unpack the 7-bit MIDI payload back into its original 8-bit structure (the first byte holds the MSBs).
    while i + 7 < len {
        let mut bits = packed[i] as u32;
        for j in 0..7 {
            bits <<= 1;
            let byte = (packed[i + 1 + j] & 0x7F) | ((bits & 0x80) as u8);
            result.push(byte);
        }
        i += 8;
    }
    result
}

/// Decompresses the `rle7` scheme, which a device opts into through `sysex_layout.kit.encoding`.
///
/// Only the Monomachine declares it today.
///
/// A byte with bit 7 set encodes a run: (`0x80` | count) followed by the byte to repeat.
/// Raw values all sit below `0x80`, which is what leaves bit 7 free to act as the run marker.
///
/// If `count` is `Some(n)`, stops after producing n output bytes.
pub fn decode_rle7(data: &[u8], count: Option<usize>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() && count.is_none_or(|c| out.len() < c) {
        let byte = data[i];
        if byte & 0x80 != 0 {
            let repeat_count = (byte & 0x7F) as usize;
            i += 1;
            if i < data.len() {
                let repeat_byte = data[i];
                for _ in 0..repeat_count {
                    if count.is_none_or(|c| out.len() < c) {
                        out.push(repeat_byte);
                    }
                }
            }
        } else {
            out.push(byte);
        }
        i += 1;
    }
    out
}

/// Wraps a payload with the Elektron manufacturer header, producing a valid SysEx message.
pub fn build_elektron_sysex(prod: u8, ch: u8, payload: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(ELEKTRON_MFR.len() + 2 + payload.len() + 1);
    msg.extend_from_slice(&ELEKTRON_MFR);
    msg.push(prod);
    msg.push(ch);
    msg.extend_from_slice(payload);
    msg.push(0xF7);
    msg
}

/// Builds the machine assignment payload for one track.
pub fn build_machine_assignment_payload(dc: &DeviceConfig, track: u8, machine_id: u8, init_scope: Option<u8>) -> Vec<u8> {
    let assign_cmd = as_u64_or_die(dc.json_get("sysex_api.assign_machine.cmd")) as u8;
    let mut payload = vec![assign_cmd, track & 0x7F];

    // Devices that bank their machines split the ID into a 7-bit field plus the bank bit above it.
    // The Monomachine has no bank byte, so its init scope takes that slot and is always sent.
    // An `init_scope` of `None` leaves the trailing byte off, which assigns the machine without resetting its params.
    if dc.has_gate("sysex_api.assign_machine.has_bank_byte") {
        payload.push(machine_id & 0x7F);
        payload.push(machine_id >> 7);
        if let Some(init_scope) = init_scope {
            payload.push(init_scope);
        }
    } else {
        payload.push(machine_id);
        payload.push(init_scope.unwrap_or(0));
    }
    payload
}

/// Computes the 14-bit Elektron checksum over the payload region of a SysEx blob.
///
/// Covers `blob[9..blob.len()-5]`, the same window used by kits, patterns, and DigiPro messages.
fn elektron_checksum_14bit(blob: &[u8]) -> u32 {
    blob[9..blob.len() - 5].iter().map(|&byte| byte as u32).sum::<u32>() & ELEKTRON_CHECKSUM_MASK
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// Names the device that sent `raw`, but only when its type byte is that device's pattern dump command.
///
/// Returns the device's `device_short`.
/// Returns an empty string if the pattern dump is unrecognized.
fn find_device_from_pattern_dump(raw: &[u8]) -> String {
    let Some(dc) = DeviceConfig::find_by_prod(raw[ELEKTRON_PROD_BYTE]) else {
        return String::new();
    };
    let dump_cmd = dc.json_get("sysex_api.pattern.write_cmd").and_then(serde_json::Value::as_u64);
    if dump_cmd == Some(u64::from(raw[ELEKTRON_TYPE_BYTE])) {
        dc.device_shorter
    } else {
        String::new()
    }
}

/// Translates a byte value into a character that can be displayed on the hardware's screen.
fn elektron_lcd_char(byte: u8) -> Option<char> {
    // Return standard printable ASCII characters (Space through Z) directly.
    if (0x20..=0x5A).contains(&byte) {
        return Some(byte as char);
    }
    // For values above 'Z', wrap them back into the A-Z range (standard MD behavior).
    if byte > 0x5A {
        return Some(((byte - 0x41) % 26 + 0x41) as char);
    }
    None
}
