//! Functions for interfacing with the `.c7` JSON file format.
//!
//! Also handles `.syx`, and converts between raw SysEx blobs and the per-parameter JSON form.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;

use crate::device_config::{DeviceConfig, read_settings};
use crate::kit::master_fx_list;
use crate::sds::{compress_sds_to_b64, decode_flac_b64_to_sds, is_sds_message, sds_dump_header, sds_sample_number};
use crate::sysex::{
    ELEKTRON_PAYLOAD_START, ELEKTRON_PROD_BYTE, ELEKTRON_TYPE_BYTE, decode_7bit, decode_mnm_kit, encode_7bit, extract_sysex_name,
    parse_sysex_file, rebuild_rle7_dump, update_elektron_checksum,
};
use crate::utils::{
    JsonPath, as_array_or, as_string_or, as_string_or_die, as_u64_or, as_u64_or_die, bytes_to_hex_string, from_hex, now_compact_timestamp,
    now_iso_string, sanitize_filename, temp_path,
};

/// Logical data-type ordering for `sort_c7_items()`.
const TYPE_ORDER: &[&str] = &["global", "kit", "pattern", "song", "sample", "digipro", "sound"];

/// One kit field parsed from `sysex_layout.kit.fields`.
///
/// The device JSON's `sysex_layout.kit` section is the source of truth for where every kit field lives and how the track blob is built.
/// This code only interprets it.
///
/// `offset` addresses the field's section on the wire (MD, raw/7-bit sections) or in the decoded payload (MnM, whole-payload RLE+7-bit).
/// `base` is an extra offset inside a decoded section before the per-track stride applies.
/// Two fields share one section this way, like MD's trig/mute groups.
#[derive(Clone)]
struct KitField {
    offset: usize,
    base: usize,
    stride: usize,
    size: usize,
    kind: String,
    bits: usize,
    encoding: String,
    wire_len: usize,
}

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// A single parsed section from a `.c7` JSON file.
///
/// Mirrors the `item` dicts produced by `read_c7_file()` and consumed by `write_c7_file()`.
#[derive(Debug, Clone, Default)]
pub struct C7Item {
    /// JSON section name (e.g., "`kit_1`", "`pattern_3`", "c7").
    pub section: String,
    /// Binary SysEx blob, multi-chunk binary list, or base64 audio text.
    pub data: Option<C7Data>,
    /// All other string attributes (type, name, slot, format, device, description, etc).
    pub attrs: BTreeMap<String, String>,
}

/// Content variant for a `C7Item`.
#[derive(Debug, Clone)]
pub enum C7Data {
    /// Single contiguous binary blob (SysEx bytes, etc.).
    Binary(Vec<u8>),
    /// Pre-split list of chunks written as `data1`, `data2`, ... in the JSON.
    BinaryList(Vec<Vec<u8>>),
    /// Base64-encoded audio or other text content (format = "flac+base64").
    Text(String),
    /// Structured JSON object (e.g. per-parameter track data).
    Object(serde_json::Value),
}

impl C7Item {
    /// Returns the string value of any attribute by key.
    /// Returns the section name when key is "section".
    pub fn get(&self, key: &str) -> Option<&str> {
        match key {
            "section" => Some(&self.section),
            _ => self.attrs.get(key).map(String::as_str),
        }
    }

    /// Returns the type attribute ("kit", "pattern", etc.).
    /// Returns an empty string when absent.
    pub fn get_type(&self) -> &str {
        self.attrs.get("type").map_or("", String::as_str)
    }

    /// Returns the name attribute when present.
    pub fn get_name(&self) -> Option<&str> {
        self.attrs.get("name").map(String::as_str)
    }

    /// Returns the data as a binary slice.
    /// Returns `None` if data is absent or non-binary.
    pub fn get_data_bytes(&self) -> Option<&[u8]> {
        match &self.data {
            Some(C7Data::Binary(bytes)) => Some(bytes),
            _ => None,
        }
    }
}

/// Structured result from validation.
///
/// Carries split expected/actual strings so both the plain-text and dialog callers can format the message their own way.
pub enum ItemError {
    NoData,
    TypeMismatch { expected: String, found: String },
    DeviceMismatch { expected_device: String, file_device: String },
    CommandMismatch { expected_type: String, found_type: String },
}

/// Returns the set of sample slot indices (0-based) referenced by any track in a Machinedrum kit dump.
///
/// ROM machine model IDs 128-159 map to slots 0-31, and model IDs 176-191 map to slots 32-47.
pub fn md_kit_used_sample_slots(kit_sysex: &[u8], dc: &DeviceConfig) -> HashSet<usize> {
    let mut slots = HashSet::new();
    let models = kit_field_or_die(dc, "models");
    if kit_sysex.len() < models.offset + models.wire_len {
        return slots;
    }
    let decoded = decode_7bit(&kit_sysex[models.offset..models.offset + models.wire_len]);
    if decoded.len() < 64 {
        return slots;
    }
    slots.extend((0..16).filter_map(|track_idx| {
        let model_id = decoded[track_idx * 4 + 3]; // low byte (model ID) is last in big-endian pack32
        match model_id {
            128..=159 => Some((model_id - 128) as usize),
            176..=191 => Some((model_id - 176 + 32) as usize),
            _ => None,
        }
    }));
    slots
}

/// Determines sort order for `sort_c7_items()`.
pub fn item_sort_key(item: &C7Item) -> (usize, u32) {
    let item_type = item.get_type().to_lowercase();
    let rank = TYPE_ORDER.iter().position(|&t| t == item_type).unwrap_or(TYPE_ORDER.len());
    let slot = section_to_slot_num(&item.section, &item_type).unwrap_or(0);
    (rank, slot)
}

/// Searches through a collection of loaded items for a specific type or section.
pub fn find_c7_item<'a>(items: &'a [C7Item], target_type: Option<&str>, target_section: Option<&str>) -> Option<&'a C7Item> {
    items.iter().find(|item| {
        if let Some(t) = target_type
            && item.get_type() != t
        {
            return false;
        }
        if let Some(s) = target_section
            && item.section != s
        {
            return false;
        }
        item.data.is_some()
    })
}

/// Changes the internal display name of an item saved in a `.c7` JSON file.
pub fn rename_c7_item(path: impl AsRef<Path>, new_name: &str) {
    let path = path.as_ref();
    let Ok(content) = std::fs::read_to_string(path) else { return };
    let mut root: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(&content) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => return,
    };
    // Search for the first non-metadata section to apply the name change.
    let section_to_update = root.keys().find(|key| key.as_str() != "c7").cloned();
    if let Some(section) = section_to_update
        && let Some(obj) = root.get_mut(&section).and_then(|value| value.as_object_mut())
    {
        obj.insert("name".to_string(), serde_json::Value::String(new_name.to_string()));
    }
    let json = serde_json::to_string_pretty(&serde_json::Value::Object(root)).unwrap_or_default();
    let _ = std::fs::write(path, json);
}

/// Converts a `.c7` section label back into the 0-based hardware slot it targets.
pub fn section_to_wire_slot(section: &str, item_type: &str) -> Option<usize> {
    section_to_slot_num(section, item_type).map(|num| num.saturating_sub(1) as usize)
}

/// Runs all checks and returns a structured error.
/// Returns `None` on success.
pub fn validate_item(item: &C7Item, expected_type: Option<&str>, device_prod_byte: u8) -> Option<ItemError> {
    let data = match item.get_data_bytes() {
        Some(data) if !data.is_empty() => data,
        _ => return Some(ItemError::NoData),
    };

    let item_type = item.get_type().to_lowercase();
    if let Some(expected) = expected_type
        && item_type != expected.to_lowercase()
    {
        return Some(ItemError::TypeMismatch {
            expected: expected.to_string(),
            found: item_type,
        });
    }
    if item_type == "sample" {
        return None;
    }

    if data.len() < 5 || data[0] != 0xF0 || data[1..4] != [0x00, 0x20, 0x3C] {
        return None;
    }
    let file_prod = data[ELEKTRON_PROD_BYTE];
    if file_prod != device_prod_byte {
        return Some(ItemError::DeviceMismatch {
            expected_device: resolve_device_name_by_id(device_prod_byte),
            file_device: resolve_device_name_by_id(file_prod),
        });
    }

    if data.len() > 6 {
        let settings = read_settings();
        let active_config = settings
            .get("active_config")
            .and_then(|value| value.as_str())
            .map(std::string::ToString::to_string);
        if let Some(path) = active_config
            && let Some(expected_cmd) = lookup_registry_cmd_for_type(&path, &item_type)
            && data[ELEKTRON_TYPE_BYTE] as u64 != expected_cmd
        {
            return Some(ItemError::CommandMismatch {
                expected_type: item_type,
                found_type: identify_type_by_registry_cmd(file_prod, data[ELEKTRON_TYPE_BYTE]),
            });
        }
    }

    None
}

/// Returns the Monomachine kit-global block: polyphony mode, common timing, split key, and split track.
///
/// These sit at the tail of the decoded payload, so the block runs from `common_multimode` to the end.
pub fn extract_mnm_kit_globals(kit_sysex: &[u8], dc: &DeviceConfig) -> Vec<u8> {
    let globals_start = kit_field_or_die(dc, "common_multimode").offset;
    let (decoded, _) = decode_mnm_kit(
        kit_sysex,
        as_u64_or_die(dc.json_get("sysex_layout.kit.fields.payload.decoded_size")) as usize,
    );
    decoded[globals_start..].to_vec()
}

/// Writes the kit-global block and any shared per-track param bytes back into a Monomachine kit.
///
/// `shared_params` are (param offset, value) pairs repeated inside every track's param block (e.g. the multi envelope at 64..69).
pub fn apply_mnm_kit_globals(kit_sysex: &[u8], kit_globals: &[u8], shared_params: &[(usize, u8)], dc: &DeviceConfig) -> Vec<u8> {
    let globals_start = kit_field_or_die(dc, "common_multimode").offset;
    let params = kit_field_or_die(dc, "params");
    let num_tracks = as_array_or(dc.json_get("tracks"), &[]).len();
    let (mut decoded, decode_start) = decode_mnm_kit(
        kit_sysex,
        as_u64_or_die(dc.json_get("sysex_layout.kit.fields.payload.decoded_size")) as usize,
    );

    // `zip` stops at the shorter side, so a short workspace dump can't run past the payload.
    for (dst, &src) in decoded[globals_start..].iter_mut().zip(kit_globals) {
        *dst = src;
    }
    for track_idx in 0..num_tracks {
        for &(offset, val) in shared_params {
            if offset < params.stride {
                decoded[params.offset + track_idx * params.stride + offset] = val;
            }
        }
    }

    rebuild_rle7_dump(kit_sysex, &decoded, decode_start)
}

/// Re-emits a live-workspace kit dump (version `$40`) as an ordinary saved-slot dump (version `$02`).
pub fn normalize_mnm_workspace(workspace_sysex: &[u8], dc: &DeviceConfig) -> Vec<u8> {
    let decoded_size = as_u64_or_die(dc.json_get("sysex_layout.kit.fields.payload.decoded_size")) as usize;
    let (decoded, _) = decode_mnm_kit(workspace_sysex, decoded_size);
    // Saved-slot header prefix through origPos: the same bytes minus the workspace flag, with the version byte forced to `$02`.
    // `rebuild_rle7_dump()` reads only this prefix, then appends the payload.
    let mut header = workspace_sysex[..10].to_vec();
    header[ELEKTRON_PAYLOAD_START] = 2;
    rebuild_rle7_dump(&header, &decoded, 10)
}

/// Generates a fingerprint for an item to detect duplicates.
pub fn item_fingerprint(item: &C7Item) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    item.get_type().hash(&mut hasher);
    item.section.hash(&mut hasher);
    item.get_name().hash(&mut hasher);
    match &item.data {
        Some(C7Data::Binary(value)) => value.hash(&mut hasher),
        Some(C7Data::BinaryList(value)) => value.hash(&mut hasher),
        Some(C7Data::Text(text)) => text.hash(&mut hasher),
        Some(C7Data::Object(o)) => o.to_string().hash(&mut hasher),
        None => {}
    }
    hasher.finish()
}

/// Applies per-item slot-conflict resolution choices.
///
/// Choice value 0 = "Keep slot", value 1 = "Move to free slot".
/// Items without a choice (non-conflicting) always keep their slot.
pub fn resolve_conflicts(pairs: &[(&C7Item, Option<u32>)]) -> (Vec<C7Item>, usize) {
    let items: Vec<C7Item> = pairs.iter().map(|(item, _)| (*item).clone()).collect();
    let mut used_slots = collect_used_slots(&items);
    let mut resolved = Vec::new();
    let mut reassigned = 0usize;

    // Map over selected pairs and apply slot reassignment if requested.
    for (item, choice) in pairs {
        let move_slot = choice.is_some_and(|value| value == 1); // "Move to free slot"
        let mut new_item = (*item).clone();
        if move_slot {
            let item_type = item.get_type().to_string();
            let new_number = next_free_slot(&mut used_slots, &item_type);
            if let Some(new_section) = slot_number_to_section(&item_type, new_number) {
                new_item.section = new_section;
            }
            reassigned += 1;
        }
        resolved.push(new_item);
    }
    (resolved, reassigned)
}

/// Determines which device a file was created for, from its `.c7` header or the product-ID byte of its embedded SysEx.
pub fn device_family_from_file(path: &str, items: &[C7Item]) -> Option<String> {
    if path.to_lowercase().ends_with(".c7") {
        for item in read_c7_file(path) {
            if item.section == "c7" {
                // Attempt to resolve the device identity from the file header against the registry.
                let dc = DeviceConfig::find_by_name(item.get("device").unwrap_or(""));
                if let Some(dc) = dc {
                    return Some(dc.device_short);
                }
            }
        }
    }

    for item in items {
        if let Some(data) = item.get_data_bytes()
            && data.len() > 4
            && data[1..4] == [0x00, 0x20, 0x3C]
        {
            // Use MIDI Product ID byte to definitively identify the source hardware.
            return Some(resolve_device_name_by_id(data[ELEKTRON_PROD_BYTE]));
        }
    }

    None
}

/// Loads a file and parses it into a list of C7 items, supporting both JSON `.c7` files and raw `.syx` binary dumps.
pub fn read_c7_or_sysex_file(file_path: impl AsRef<Path>) -> Vec<C7Item> {
    let path = file_path.as_ref();
    let lower = path.to_string_lossy().to_lowercase();

    if lower.ends_with(".syx") || lower.ends_with(".sysex") {
        let Ok(raw) = std::fs::read(path) else { return Vec::new() };
        let msgs = parse_sysex_file(&raw);
        let prod = msgs.first().and_then(|msg| {
            if msg.len() > ELEKTRON_PROD_BYTE {
                Some(msg[ELEKTRON_PROD_BYTE])
            } else {
                None
            }
        });
        let device_shorter = prod
            .and_then(DeviceConfig::find_by_prod)
            .map_or_else(|| "UNK".to_string(), |dc| dc.device_shorter);

        let (_, items) = process_sysex_for_c7_export(&msgs, &device_shorter, None, None, Some(&|msg| extract_sysex_name(msg)));

        // `process_sysex_for_c7_export()` ignores SDS, and one sample spans a Dump Header plus every data packet after it.
        // Each run collapses into one `flac+base64` item: one item per packet would collide on the section name, losing all but the last.
        let mut out: Vec<C7Item> = Vec::with_capacity(items.len());
        let mut run = Vec::<u8>::new();
        let mut run_slot = 0usize;

        for (i, msg) in msgs.iter().enumerate() {
            if let Some(slot) = sds_sample_number(msg) {
                flush_sds_run(&mut out, &mut run, run_slot);
                run_slot = slot;
            } else if !is_sds_message(msg) {
                flush_sds_run(&mut out, &mut run, run_slot);
                out.push(items[i].clone());
                continue;
            }
            run.extend_from_slice(msg);
        }
        flush_sds_run(&mut out, &mut run, run_slot);

        return out;
    }

    read_c7_file(path)
}

/// Converts a raw Machinedrum track blob into a structured JSON object using the device specification.
///
/// The blob is a 67-byte extracted sound as produced by `extract_sound_from_kit()`.
/// Page sections use the page's `fullname` as the key and param `name` values as sub-keys.
/// The LFO section stores only the five params sent via SysEx (TRACK, PARAM, SHP1, SHP2, UPDTE).
/// SPEED/DEPTH/SHMIX live in the Routing page as LFOS/LFOD/LFOM and are already covered there.
pub fn md_sound_to_json(blob: &[u8], dc: &DeviceConfig) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    let levels_offset = kit_blob_offset_or_die(dc, "levels");
    // The model ID is the low byte of the field's big-endian 4-byte value.
    let model_id_offset = kit_blob_offset_or_die(dc, "models") + 3;

    let machine_id = if blob.len() > model_id_offset { blob[model_id_offset] } else { 0 };
    obj.insert("machine".to_string(), serde_json::Value::String(machine_id_to_name(dc, machine_id)));

    // Standalone pages (e.g. "Level") live at a named offset, not in the Synthesis/Effects/Routing byte range.
    let standalone_pages = as_array_or(dc.json_get("pages.standalone"), &[]);
    let mut standalone_pages_obj = serde_json::Map::new();
    for page in standalone_pages {
        let page_fullname = as_string_or_die(page.json_get("fullname"));
        let params = as_array_or(page.json_get("params"), &[]);
        let mut page_obj = serde_json::Map::new();
        for (i, param) in params.iter().enumerate() {
            let name = as_string_or_die(param.json_get("name"));
            let val: u8 = if levels_offset + i < blob.len() {
                blob[levels_offset + i]
            } else {
                0
            };
            page_obj.insert(name.to_string(), serde_json::Value::Number(val.into()));
        }
        standalone_pages_obj.insert(page_fullname.to_string(), serde_json::Value::Object(page_obj));
    }

    // Synthesis pages (3 × 8 = 24 params, stored consecutively at `blob[0..24]`)
    let mut byte_idx = 0usize;
    let synth_pages_obj = pages_to_json(as_array_or(dc.json_get("pages.synth"), &[]), blob, &mut byte_idx);

    let mut pages_obj = serde_json::Map::new();
    pages_obj.insert("standalone".to_string(), serde_json::Value::Object(standalone_pages_obj));
    pages_obj.insert("synth".to_string(), serde_json::Value::Object(synth_pages_obj));
    obj.insert("pages".to_string(), serde_json::Value::Object(pages_obj));

    // LFO: first 5 params only (TRACK, PARAM, SHP1, SHP2, UPDTE).
    // SPEED/DEPTH/SHMIX sit in the Routing page params and are omitted here.
    let lfos_offset = kit_blob_offset_or_die(dc, "lfos");
    let lfo_params = as_array_or(dc.json_get("track_lfo.params"), &[]);
    if !lfo_params.is_empty() {
        let mut lfo_obj = serde_json::Map::new();
        for (i, param) in lfo_params.iter().enumerate().take(5) {
            let name = as_string_or_die(param.json_get("name"));
            let val: u8 = if lfos_offset + i < blob.len() { blob[lfos_offset + i] } else { 0 };
            lfo_obj.insert(name.to_string(), serde_json::Value::Number(val.into()));
        }
        let mut track_lfo_obj = serde_json::Map::new();
        track_lfo_obj.insert("LFO".to_string(), serde_json::Value::Object(lfo_obj));
        obj.insert("track_lfo".to_string(), serde_json::Value::Object(track_lfo_obj));
    }

    serde_json::Value::Object(obj)
}

/// Reconstructs a raw Machinedrum sound blob from a structured JSON object.
///
/// Produces a 67-byte blob compatible with `apply_sound_to_kit()` and `apply_track_via_cc()`.
/// The 31 bytes of internal LFO oscillator state (`blob[34..65]`) are zeroed.
/// They aren't stored in the JSON because they can't be set via the public SysEx/CC API.
pub fn md_sound_from_json(json: &serde_json::Value, dc: &DeviceConfig) -> Vec<u8> {
    let mut blob = vec![0u8; kit_track_blob_len(dc)];
    // Params occupy the blob up to the level byte.
    let levels_offset = kit_blob_offset_or_die(dc, "levels");
    let model_id_offset = kit_blob_offset_or_die(dc, "models") + 3;
    let lfos_offset = kit_blob_offset_or_die(dc, "lfos");

    // Synthesis pages → `blob[0..24]`
    let mut byte_idx = 0usize;
    let synth_pages = as_array_or(dc.json_get("pages.synth"), &[]);
    pages_from_json(synth_pages, json, "pages.synth", &mut blob, levels_offset, &mut byte_idx);

    let standalone_pages = as_array_or(dc.json_get("pages.standalone"), &[]);
    for page in standalone_pages {
        let page_fullname = as_string_or_die(page.json_get("fullname"));
        let params = as_array_or(page.json_get("params"), &[]);
        for (i, param) in params.iter().enumerate() {
            let name = as_string_or_die(param.json_get("name"));
            let val = as_u64_or(json.json_get(&format!("pages.standalone.{page_fullname}.{name}")), 0) as u8;
            if levels_offset + i < blob.len() {
                blob[levels_offset + i] = val;
            }
        }
    }

    // Machine ID in the model field's last byte, matching `apply_track_via_cc()`.
    let machine_name = as_string_or(json.json_get("machine"), "");
    blob[model_id_offset] = machine_name_to_id(dc, machine_name);

    // LFO first 5 params. The remaining LFO bytes stay zero (internal oscillator state that has no public API).
    let lfo_params = as_array_or(dc.json_get("track_lfo.params"), &[]);
    for (i, param) in lfo_params.iter().enumerate().take(5) {
        let name = as_string_or_die(param.json_get("name"));
        blob[lfos_offset + i] = as_u64_or(json.json_get(&format!("track_lfo.LFO.{name}")), 0) as u8;
    }

    blob
}

/// Returns `(block_byte, param_id, value)` for every master FX parameter found in the kit blob.
///
/// Uses `kit_sysex_offset` from each `master_fx` entry in the device spec to locate the correct byte range in the kit dump.
pub fn extract_master_fx(kit_sysex: &[u8], dc: &DeviceConfig) -> Vec<(u8, u8, u8)> {
    master_fx_list(dc)
        .iter()
        .map(|fx_param| {
            let val = if fx_param.kit_offset < kit_sysex.len() {
                kit_sysex[fx_param.kit_offset]
            } else {
                0
            };
            (fx_param.block_byte, fx_param.param_id, val)
        })
        .collect()
}

/// Saves a captured sample as a standalone `.c7` file, embedding the audio as FLAC+base64.
///
/// `sds_data` is the raw SDS binary, already consolidated from its individual packets.
pub fn write_sample_c7(file_path: impl AsRef<Path>, sds_data: &[u8], name: &str, device_short: &str) -> Result<(), String> {
    // Nothing upstream tracks the slot, so the device's own Dump Header is the only record of where the sample came from.
    let wire_slot = sds_dump_header(sds_data)
        .and_then(sds_sample_number)
        .ok_or("SDS: stream has no Dump Header")?;
    let b64 = compress_sds_to_b64(sds_data)?;
    let header = generate_c7_header("sample", device_short, None);
    let mut item = C7Item {
        section: wire_slot_to_section("sample", wire_slot),
        data: Some(C7Data::Text(b64)),
        attrs: BTreeMap::new(),
    };
    item.attrs.insert("type".to_string(), "sample".to_string());
    item.attrs.insert("format".to_string(), "flac+base64".to_string());
    item.attrs.insert("name".to_string(), name.to_string());
    write_c7_file(file_path, &[item], Some(&header));
    Ok(())
}

/// Saves a single DigiPro waveform to a `.c7` file with type 'digipro'.
///
/// `sysex`: the full `0x5D` DigiPro SysEx message for this waveform, stored as hex and otherwise unchanged.
pub fn write_digipro_c7(file_path: impl AsRef<Path>, sysex: &[u8], slot: u32, name: &str, device_shorter: &str) {
    let header = generate_c7_header("digipro", device_shorter, None);
    let mut item = C7Item {
        section: wire_slot_to_section("digipro", slot as usize),
        data: Some(C7Data::Binary(sysex.to_vec())),
        attrs: BTreeMap::new(),
    };
    item.attrs.insert("type".to_string(), "digipro".to_string());
    item.attrs.insert("format".to_string(), "sysex".to_string());
    item.attrs.insert("name".to_string(), name.to_string());
    write_c7_file(file_path, &[item], Some(&header));
}

/// Saves a sequence of DigiPro waveforms as a 'digipro_wavetable' `.c7` file.
///
/// `frames`: one full `0x5D` DigiPro SysEx message per frame. Stored as hex, SysEx-native (no decoded waveform).
/// Sections are numbered from 1 rather than from the slots the frames came from, since a wavetable has no fixed home on the device.
pub fn write_digipro_wavetable_c7(file_path: impl AsRef<Path>, frames: &[Vec<u8>], name: &str, device_shorter: &str) {
    let mut header = generate_c7_header("digipro_wavetable", device_shorter, None);
    header.insert("name".to_string(), name.to_string());

    let items: Vec<C7Item> = frames
        .iter()
        .enumerate()
        .map(|(i, sysex)| {
            let mut item = C7Item {
                section: wire_slot_to_section("digipro", i),
                data: Some(C7Data::Binary(sysex.clone())),
                attrs: BTreeMap::new(),
            };
            item.attrs.insert("type".to_string(), "digipro".to_string());
            item.attrs.insert("format".to_string(), "sysex".to_string());
            item.attrs.insert("name".to_string(), name.to_string());
            item
        })
        .collect();

    write_c7_file(file_path, &items, Some(&header));
}

/// Reads a 'digipro_wavetable' `.c7` file and returns `(slot, wave_u8)` tuples sorted by slot number.
///
/// Used to fill a sequential range of the 64 DigiPro slots on the Monomachine.
pub fn read_digipro_wavetable_c7(file_path: impl AsRef<Path>) -> Vec<(u32, Vec<u8>)> {
    let items = read_c7_file(file_path);
    let mut results: Vec<(u32, Vec<u8>)> = Vec::new();
    // Walking parsed structures: collecting and sorting wavetable frames.
    for item in items {
        if item.section == "c7" {
            continue;
        }
        if item.get_type() != "digipro" {
            continue;
        }
        let data = match item.get_data_bytes() {
            Some(data) if !data.is_empty() => data.to_vec(),
            // Skip items without valid binary data.
            _ => continue,
        };
        // Frame order comes from the section number, since a wavetable doesn't record the slots it was taken from.
        let Some(slot) = section_to_wire_slot(&item.section, "digipro") else {
            continue;
        };
        results.push((slot as u32, data));
    }
    results.sort_by_key(|(slot, _)| *slot);
    results
}

/// Expands a `.c7` file's embedded FLAC+base64 audio blob to a temp `.sds` file on disk.
///
/// Returns its path.
/// Returns `None` if the file has no audio.
pub fn expand_c7_to_sds(path: &str) -> Option<String> {
    let items = read_c7_file(path);

    let blob_b64 = items.into_iter().find_map(|item| {
        if item.get_type() != "sample" {
            return None;
        }
        if let Some(C7Data::Text(text)) = item.data {
            Some(text)
        } else {
            None
        }
    })?;

    let decoded = decode_flac_b64_to_sds(&blob_b64).ok()?;
    let tmp_path = temp_path("expand", "sds");
    std::fs::write(&tmp_path, decoded).ok()?;
    Some(tmp_path.to_string_lossy().to_string())
}

/// Sorts items by type, then by slot number.
pub fn sort_c7_items(mut items: Vec<C7Item>) -> Vec<C7Item> {
    items.sort_by_key(item_sort_key);
    items
}

/// Writes a sound blob back into a kit dump, reversing `extract_sound_from_kit()`.
///
/// Partial blobs are legal: fields beyond the blob's length keep the kit's existing bytes.
pub fn apply_sound_to_kit(kit_sysex: &[u8], sound_data: &[u8], track: usize, dc: &DeviceConfig) -> Vec<u8> {
    if is_kit_rle7(dc) {
        let (mut decoded, decode_start) = decode_mnm_kit(
            kit_sysex,
            as_u64_or_die(dc.json_get("sysex_layout.kit.fields.payload.decoded_size")) as usize,
        );

        let mut pos = 0usize;
        for name in kit_track_blob_fields(dc) {
            let field = kit_field_or_die(dc, &name);
            let len = kit_field_blob_len(&field);
            if pos + len > sound_data.len() {
                break;
            }
            match field.kind.as_str() {
                "bit_per_track" => {
                    let bit = 1u8 << track;
                    if sound_data[pos] != 0 {
                        decoded[field.offset] |= bit;
                    } else {
                        decoded[field.offset] &= !bit;
                    }
                }
                "bits_per_track" => write_track_bits(&mut decoded, &field, track, sound_data[pos]),
                _ => {
                    let start = field.offset + field.base + track * field.stride;
                    decoded[start..start + field.stride].copy_from_slice(&sound_data[pos..pos + field.stride]);
                }
            }
            pos += len;
        }

        rebuild_rle7_dump(kit_sysex, &decoded, decode_start)
    } else {
        let mut kit_sysex_vec = kit_sysex.to_vec();

        let mut pos = 0usize;
        for name in kit_track_blob_fields(dc) {
            let field = kit_field_or_die(dc, &name);
            let len = kit_field_blob_len(&field);
            if pos + len > sound_data.len() {
                break;
            }
            // Skip fields the dump is too short to hold, matching `extract_sound_from_kit()` so `pos` stays aligned with the blob.
            if kit_sysex_vec.len() < field.offset + field.wire_len {
                continue;
            }
            let start = field.base + track * field.stride;
            if field.encoding == "7bit" {
                let mut decoded = decode_7bit(&kit_sysex_vec[field.offset..field.offset + field.wire_len]);
                decoded[start..start + field.stride].copy_from_slice(&sound_data[pos..pos + field.stride]);
                let encoded = encode_7bit(&decoded);
                kit_sysex_vec[field.offset..field.offset + encoded.len()].copy_from_slice(&encoded);
            } else {
                kit_sysex_vec[field.offset + start..field.offset + start + field.stride]
                    .copy_from_slice(&sound_data[pos..pos + field.stride]);
            }
            pos += len;
        }

        update_elektron_checksum(&mut kit_sysex_vec);
        kit_sysex_vec
    }
}

/// Converts a raw Monomachine sound blob into a structured JSON object using the device specification.
///
/// The blob is a 112-byte extracted sound as produced by `extract_sound_from_kit()`.
/// Page sections use the page's `fullname` as the key and param `name` values as sub-keys.
pub fn mnm_sound_to_json(blob: &[u8], dc: &DeviceConfig) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    let levels_offset = kit_blob_offset_or_die(dc, "levels");
    let models_offset = kit_blob_offset_or_die(dc, "models");

    let model_id = if blob.len() > models_offset { blob[models_offset] } else { 0 };
    obj.insert("machine".to_string(), serde_json::Value::String(machine_id_to_name(dc, model_id)));

    // Standalone pages (e.g. "Level") live at a named offset, not in the synth/LFO page byte range.
    let standalone_pages = as_array_or(dc.json_get("pages.standalone"), &[]);
    let mut standalone_pages_obj = serde_json::Map::new();
    for page in standalone_pages {
        let page_fullname = as_string_or_die(page.json_get("fullname"));
        let params = as_array_or(page.json_get("params"), &[]);
        let mut page_obj = serde_json::Map::new();
        for (i, param) in params.iter().enumerate() {
            let name = as_string_or_die(param.json_get("name"));
            let val: u8 = if levels_offset + i < blob.len() {
                blob[levels_offset + i]
            } else {
                0
            };
            page_obj.insert(name.to_string(), serde_json::Value::Number(val.into()));
        }
        standalone_pages_obj.insert(page_fullname.to_string(), serde_json::Value::Object(page_obj));
    }

    // Params are stored sequentially in the blob with no gaps, spanning the synth pages then the LFO pages in definition order.
    // The CC numbering gaps (64-71, 96-103) are NOT reflected in the binary layout.
    let mut byte_idx = 0usize;
    let synth_pages_obj = pages_to_json(as_array_or(dc.json_get("pages.synth"), &[]), blob, &mut byte_idx);
    let lfo_pages_obj = pages_to_json(as_array_or(dc.json_get("pages.lfo"), &[]), blob, &mut byte_idx);

    let mut pages_obj = serde_json::Map::new();
    pages_obj.insert("standalone".to_string(), serde_json::Value::Object(standalone_pages_obj));
    pages_obj.insert("synth".to_string(), serde_json::Value::Object(synth_pages_obj));
    pages_obj.insert("lfo".to_string(), serde_json::Value::Object(lfo_pages_obj));
    obj.insert("pages".to_string(), serde_json::Value::Object(pages_obj));

    serde_json::Value::Object(obj)
}

/// Pulls one track's sound bytes out of a kit dump by walking the layout's `track_blob` field list.
pub fn extract_sound_from_kit(kit_sysex: &[u8], track: usize, dc: &DeviceConfig) -> Vec<u8> {
    let mut blob = Vec::new();
    if is_kit_rle7(dc) {
        let (decoded, _) = decode_mnm_kit(
            kit_sysex,
            as_u64_or_die(dc.json_get("sysex_layout.kit.fields.payload.decoded_size")) as usize,
        );

        for name in kit_track_blob_fields(dc) {
            let field = kit_field_or_die(dc, &name);
            match field.kind.as_str() {
                "bit_per_track" => blob.push((decoded[field.offset] >> track) & 1),
                "bits_per_track" => blob.push(read_track_bits(&decoded, &field, track)),
                _ => {
                    let start = field.offset + field.base + track * field.stride;
                    blob.extend_from_slice(&decoded[start..start + field.stride]);
                }
            }
        }
    } else {
        // Sectioned dump: each field addresses its own wire section, raw or 7-bit packed.
        for name in kit_track_blob_fields(dc) {
            let field = kit_field_or_die(dc, &name);
            // Skip fields the dump is too short to hold, since the JSON tracks newer firmware than the device may be running.
            if kit_sysex.len() < field.offset + field.wire_len {
                continue;
            }
            let start = field.base + track * field.stride;
            if field.encoding == "7bit" {
                let decoded = decode_7bit(&kit_sysex[field.offset..field.offset + field.wire_len]);
                blob.extend_from_slice(&decoded[start..start + field.stride]);
            } else {
                blob.extend_from_slice(&kit_sysex[field.offset + start..field.offset + start + field.stride]);
            }
        }
    }

    blob
}

/// Reconstructs a raw Monomachine sound blob from a structured JSON object.
///
/// Produces the core (flag-less) sound blob compatible with `apply_sound_to_kit()`.
pub fn mnm_sound_from_json(json: &serde_json::Value, dc: &DeviceConfig) -> Vec<u8> {
    let levels_offset = kit_blob_offset_or_die(dc, "levels");
    let models_offset = kit_blob_offset_or_die(dc, "models");
    let trig_offset = kit_blob_offset_or_die(dc, "trig_tracks");
    // Core blob only: the per-track flag bits carried after `trig_tracks` aren't serialized to JSON.
    let mut blob = vec![0u8; trig_offset + 1];

    let mut byte_idx = 0usize;
    let synth_pages = as_array_or(dc.json_get("pages.synth"), &[]);
    pages_from_json(synth_pages, json, "pages.synth", &mut blob, levels_offset, &mut byte_idx);
    let lfo_pages = as_array_or(dc.json_get("pages.lfo"), &[]);
    pages_from_json(lfo_pages, json, "pages.lfo", &mut blob, levels_offset, &mut byte_idx);

    let standalone_pages = as_array_or(dc.json_get("pages.standalone"), &[]);
    for page in standalone_pages {
        let page_fullname = as_string_or_die(page.json_get("fullname"));
        let params = as_array_or(page.json_get("params"), &[]);
        for (i, param) in params.iter().enumerate() {
            let name = as_string_or_die(param.json_get("name"));
            let val = as_u64_or(json.json_get(&format!("pages.standalone.{page_fullname}.{name}")), 0) as u8;
            if levels_offset + i < blob.len() {
                blob[levels_offset + i] = val;
            }
        }
    }

    let machine_name = as_string_or(json.json_get("machine"), "");
    blob[models_offset] = machine_name_to_id(dc, machine_name);

    blob
}

/// Returns the total byte length of the extracted track blob.
pub fn kit_track_blob_len(dc: &DeviceConfig) -> usize {
    kit_track_blob_fields(dc)
        .iter()
        .map(|field_name| kit_field_blob_len(&kit_field_or_die(dc, field_name)))
        .sum()
}

/// Writes project components to a `.c7` JSON file.
pub fn write_c7_file(file_path: impl AsRef<Path>, items: &[C7Item], metadata: Option<&BTreeMap<String, String>>) {
    let mut root = serde_json::Map::new();

    // Apply project metadata as "c7" header section of the JSON.
    if let Some(meta) = metadata {
        let mut meta_obj = serde_json::Map::new();
        let mut keys: Vec<&String> = meta.keys().collect();
        keys.sort_by_key(|key| attr_sort_weight(key));
        for key in keys {
            meta_obj.insert(key.clone(), serde_json::Value::String(meta[key].clone()));
        }
        root.insert("c7".to_string(), serde_json::Value::Object(meta_obj));
    }

    // Converts each item to a JSON object that's keyed by section name.
    for item in items {
        let mut obj = serde_json::Map::new();

        let mut keys: Vec<&String> = item.attrs.keys().collect();
        keys.sort_by_key(|key| attr_sort_weight(key));
        for key in keys {
            obj.insert(key.clone(), serde_json::Value::String(item.attrs[key].clone()));
        }

        // Convert binary payloads to hex strings, text, or nested objects.
        match &item.data {
            None => {}
            Some(C7Data::Text(text)) => {
                obj.insert("data".to_string(), serde_json::Value::String(text.clone()));
            }
            Some(C7Data::Object(value)) => {
                obj.insert("data".to_string(), value.clone());
            }
            Some(C7Data::BinaryList(chunks)) => {
                let array: Vec<serde_json::Value> = chunks
                    .iter()
                    .map(|chunk| serde_json::Value::String(bytes_to_hex_string(chunk)))
                    .collect();
                obj.insert("data".to_string(), serde_json::Value::Array(array));
            }
            Some(C7Data::Binary(bytes)) => {
                let msgs = parse_sysex_file(bytes);
                if msgs.len() > 1 {
                    let array: Vec<serde_json::Value> =
                        msgs.iter().map(|msg| serde_json::Value::String(bytes_to_hex_string(msg))).collect();
                    obj.insert("data".to_string(), serde_json::Value::Array(array));
                } else {
                    obj.insert("data".to_string(), serde_json::Value::String(bytes_to_hex_string(bytes)));
                }
            }
        }

        root.insert(item.section.clone(), serde_json::Value::Object(obj));
    }

    let json = serde_json::to_string_pretty(&serde_json::Value::Object(root)).unwrap_or_default();
    let _ = std::fs::write(file_path.as_ref(), json);
}

/// Reads a `.c7` JSON file and reassembles binary payloads for hardware transmission.
pub fn read_c7_file(file_path: impl AsRef<Path>) -> Vec<C7Item> {
    let Ok(content) = std::fs::read_to_string(file_path.as_ref()) else {
        return Vec::new();
    };
    let root: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(&content) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => return Vec::new(),
    };

    let mut items = Vec::new();
    for (section_name, section_val) in &root {
        let Some(obj) = section_val.as_object() else { continue };

        let mut item = C7Item {
            section: section_name.clone(),
            data: None,
            attrs: BTreeMap::new(),
        };

        for (key, val) in obj {
            if key == "data" {
                match val {
                    serde_json::Value::String(data_str) => {
                        let is_b64 = section_val
                            .json_get("format")
                            .and_then(|format_val| format_val.as_str())
                            .is_some_and(|format_val| format_val == "flac+base64");

                        if is_b64 {
                            item.data = Some(C7Data::Text(data_str.clone()));
                        } else {
                            match from_hex(data_str) {
                                Some(bytes) => item.data = Some(C7Data::Binary(bytes)),
                                None => item.data = Some(C7Data::Text(data_str.clone())),
                            }
                        }
                    }
                    serde_json::Value::Object(_) => {
                        // Structured parameter object (e.g. per-parameter track data).
                        item.data = Some(C7Data::Object(val.clone()));
                    }
                    serde_json::Value::Array(array) => {
                        // Multi-message binary: array of hex strings concatenated into one blob.
                        let all_bytes: Vec<u8> = array
                            .iter()
                            .filter_map(|value| value.as_str())
                            .filter_map(from_hex)
                            .flatten()
                            .collect();
                        item.data = Some(C7Data::Binary(all_bytes));
                    }
                    _ => {}
                }
            } else if let Some(val_str) = val.as_str() {
                item.attrs.insert(key.clone(), val_str.to_string());
            }
        }

        items.push(item);
    }

    items
}

/// Creates the informational JSON header for a new `.c7` file.
pub fn generate_c7_header(data_type: &str, device_short: &str, description: Option<&str>) -> BTreeMap<String, String> {
    // Look up the device so the header always stores its full name, turning "MnM" into "Monomachine".
    let device = DeviceConfig::find_by_name(device_short).map_or_else(|| device_short.to_string(), |dc| dc.device_short);

    let mut header = BTreeMap::new();
    header.insert("type".to_string(), data_type.to_string());
    header.insert("device".to_string(), device);
    header.insert("export_time".to_string(), now_iso_string());

    let description = description.or_else(|| {
        Some(match data_type {
            "global" => "System-wide settings",
            "kit" => "Machine assignments",
            "pattern" => "Sequencer pattern data",
            "song" => "Song mode arrangement data",
            "sample" => "Audio sample data",
            "bundle" => "Bundle containing multiple data types",
            "sound" => "Individual sound data extracted from a kit",
            "digipro" => "Single DigiPro waveform",
            "digipro_wavetable" => "DigiPro wavetable (Sequence of DigiPro frames)",
            _ => return None,
        })
    });
    if let Some(description) = description {
        header.insert("description".to_string(), description.to_string());
    }

    header
}

/// Same as `kit_blob_offset()`, but for callers that require the field to exist.
///
/// Use this where the field is trusted to always be part of the track blob.
///
/// # Panics
///
/// Panics with the missing field's name if `sysex_layout.kit.track_blob` has no `name` entry.
pub fn kit_blob_offset_or_die(dc: &DeviceConfig, name: &str) -> usize {
    kit_blob_offset(dc, name).unwrap_or_else(|| panic!("sysex_layout.kit.track_blob missing '{name}'"))
}

/// Builds the export filename for an item, sanitizing the slot label and name for the filesystem.
///
/// A typical output looks something like this:
/// `{device_shorter}_{item_type}_{safe_slot}_{safe_name}.c7`
pub fn generate_export_filename(device_shorter: &str, item_type: &str, slot_label: &str, item_name: Option<&str>) -> String {
    let safe_slot = {
        let sanitized = sanitize_filename(slot_label);
        if sanitized.is_empty() { "unknown".to_string() } else { sanitized }
    };
    if let Some(name) = item_name
        && !["--", ""].contains(&name)
    {
        let safe_name = sanitize_filename(name);
        if !safe_name.is_empty() {
            return format!("{device_shorter}_{item_type}_{safe_slot}_{safe_name}.c7");
        }
    }
    format!("{device_shorter}_{item_type}_{safe_slot}.c7")
}

/// Converts a section label (like "`pattern_17`") back into its 1-based `.c7` slot number.
fn section_to_slot_num(section: &str, item_type: &str) -> Option<u32> {
    section.strip_prefix(item_type)?.strip_prefix('_')?.parse().ok()
}

/// Builds the `.c7` section label for a 0-based hardware slot.
///
/// `.c7` slot numbers are 1-based for every item type, independent of how the device labels the slot on screen.
pub fn wire_slot_to_section(item_type: &str, wire_slot: usize) -> String {
    format!("{item_type}_{}", wire_slot + 1)
}

/// Returns the byte offset of a named field inside the extracted track blob.
/// Returns `None` when the field isn't part of it.
pub fn kit_blob_offset(dc: &DeviceConfig, name: &str) -> Option<usize> {
    let mut pos = 0usize;
    for field_name in kit_track_blob_fields(dc) {
        let field = kit_field_or_die(dc, &field_name);
        if field_name == name {
            return Some(pos);
        }
        pos += kit_field_blob_len(&field);
    }
    None
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// Optional callback that tries to extract a human-readable name from a raw SysEx blob.
type ExtractNameFn<'a> = Option<&'a dyn Fn(&[u8]) -> Option<String>>;

/// Reads one blob byte per param across `pages`, advancing `byte_idx` as it goes.
///
/// Returns a map of page fullname to a map of param name to byte value.
///
/// Params reaching past the end of `blob` read as 0.
/// Callers thread one `byte_idx` through consecutive calls where the pages share a single run of bytes.
fn pages_to_json(pages: &[serde_json::Value], blob: &[u8], byte_idx: &mut usize) -> serde_json::Map<String, serde_json::Value> {
    let mut pages_obj = serde_json::Map::new();
    for page in pages {
        let page_fullname = as_string_or_die(page.json_get("fullname"));
        let mut page_obj = serde_json::Map::new();
        for param in as_array_or(page.json_get("params"), &[]) {
            let name = as_string_or_die(param.json_get("name"));
            let val: u8 = if *byte_idx < blob.len() { blob[*byte_idx] } else { 0 };
            page_obj.insert(name.to_string(), serde_json::Value::Number(val.into()));
            *byte_idx += 1;
        }
        pages_obj.insert(page_fullname.to_string(), serde_json::Value::Object(page_obj));
    }
    pages_obj
}

/// Writes one blob byte per param across `pages`, reading each value from `json` under `section`.
///
/// The inverse of `pages_to_json()`, so the two must walk `pages` in the same order.
/// Bytes at or past `limit` are skipped, keeping the params from overrunning the fields that follow them in the blob.
fn pages_from_json(
    pages: &[serde_json::Value],
    json: &serde_json::Value,
    section: &str,
    blob: &mut [u8],
    limit: usize,
    byte_idx: &mut usize,
) {
    for page in pages {
        let page_fullname = as_string_or_die(page.json_get("fullname"));
        for param in as_array_or(page.json_get("params"), &[]) {
            let name = as_string_or_die(param.json_get("name"));
            let val = as_u64_or(json.json_get(&format!("{section}.{page_fullname}.{name}")), 0) as u8;
            if *byte_idx < limit {
                blob[*byte_idx] = val;
            }
            *byte_idx += 1;
        }
    }
}

/// Compresses one accumulated SDS packet run into a `sample` item and clears the run.
///
/// Does nothing when the run is empty, so a caller can flush unconditionally between messages.
/// A run that fails to compress is dropped rather than written as a malformed section.
fn flush_sds_run(out: &mut Vec<C7Item>, run: &mut Vec<u8>, wire_slot: usize) {
    if run.is_empty() {
        return;
    }
    if let Ok(b64) = compress_sds_to_b64(run) {
        let mut item = C7Item {
            section: wire_slot_to_section("sample", wire_slot),
            data: Some(C7Data::Text(b64)),
            attrs: BTreeMap::new(),
        };
        item.attrs.insert("type".to_string(), "sample".to_string());
        item.attrs.insert("format".to_string(), "flac+base64".to_string());
        out.push(item);
    }
    run.clear();
}

/// Retrieves the device short name for a given MIDI product-ID byte from the registry.
fn resolve_device_name_by_id(prod_byte: u8) -> String {
    DeviceConfig::find_by_prod(prod_byte).map_or_else(|| format!("device 0x{prod_byte:02X}"), |dc| dc.device_short)
}

/// Looks up the expected SysEx command byte for a data type within a specific device registry file.
fn lookup_registry_cmd_for_type(device_path: &str, item_type: &str) -> Option<u64> {
    let dc = DeviceConfig::find_by_path_or_die(device_path);
    if !["global", "kit", "pattern", "song", "digipro"].contains(&item_type) {
        return None;
    }
    dc.json_get(&format!("sysex_api.{item_type}.write_cmd"))?.as_u64()
}

/// Converts a 1-based `.c7` slot number into its section label.
fn slot_number_to_section(item_type: &str, number: u32) -> Option<String> {
    match item_type {
        "kit" | "song" | "global" | "sample" | "digipro" | "sound" | "pattern" => Some(format!("{item_type}_{number}")),
        _ => None,
    }
}

/// Maps each item type to the set of slot numbers already occupied by that type.
fn collect_used_slots(items: &[C7Item]) -> HashMap<String, HashSet<u32>> {
    let mut used: HashMap<String, HashSet<u32>> = HashMap::new();

    // Walks though the slots to see which are occupied.
    for item in items {
        let item_type_str = item.get_type().to_string();
        if let Some(slot_num) = section_to_slot_num(&item.section, &item_type_str) {
            used.entry(item_type_str).or_default().insert(slot_num);
        }
    }
    used
}

/// Returns the lowest unused slot number for `item_type`, counting from 1, and marks it used so repeated calls keep advancing.
fn next_free_slot(used_by_type: &mut HashMap<String, HashSet<u32>>, item_type: &str) -> u32 {
    let used = used_by_type.entry(item_type.to_string()).or_default();
    let mut slot_num = 1u32;
    // Search for the lowest available integer slot index.
    while used.contains(&slot_num) {
        slot_num += 1;
    }
    used.insert(slot_num);
    slot_num
}

/// Examines raw SysEx messages, identifies their type, formats items for saving to a project file, and suggests a filename.
fn process_sysex_for_c7_export(
    msgs: &[Vec<u8>],
    device_shorter: &str,
    expected_item_type: Option<&str>,
    expected_slot_label: Option<&str>,
    extract_name_cb: ExtractNameFn<'_>,
) -> (String, Vec<C7Item>) {
    let (mut item_type, mut slot_label, mut extracted_name) = if msgs.len() > 1 {
        ("Bundle".to_string(), now_compact_timestamp(), None)
    } else {
        (
            expected_item_type.unwrap_or("Backup").to_string(),
            expected_slot_label.unwrap_or(&now_compact_timestamp()).to_string(),
            None,
        )
    };

    let mut items: Vec<C7Item> = Vec::new();

    // Walking parsed structures: identifying item types and slot mappings.
    for (idx, msg) in msgs.iter().enumerate() {
        let mut msg_item_type = "unknown".to_string();
        let mut item_name = format!("dump_{idx:03}");
        let mut item_extracted_name: Option<String> = None;

        // Process messages matching the Elektron SysEx header.
        if msg.len() > 9 && msg[1..4] == [0x00, 0x20, 0x3C] {
            let cmd = msg[ELEKTRON_TYPE_BYTE];
            let prod_byte = msg[ELEKTRON_PROD_BYTE];
            let slot = msg[9]; // origPos; byte 7 is the version marker, not a slot
            let section_slot = u32::from(slot) + 1; // `.c7` section labels are 1-based
            let resolved_type = identify_type_by_registry_cmd(prod_byte, cmd);

            // Route Global settings dumps.
            if resolved_type == "global" {
                msg_item_type = "global".to_string();
                item_name = if msgs.len() == 1 && expected_item_type == Some("Global") {
                    format!("global_{slot_label}")
                } else {
                    wire_slot_to_section("global", usize::from(slot))
                };
                if msgs.len() == 1 && expected_item_type.is_none() {
                    item_type = "Global".to_string();
                    slot_label = section_slot.to_string();
                }
            }
            // Route Kit dumps.
            else if resolved_type == "kit" {
                msg_item_type = "kit".to_string();
                item_name = if msgs.len() == 1 && expected_item_type == Some("Kit") {
                    format!("kit_{slot_label}")
                } else {
                    wire_slot_to_section("kit", usize::from(slot))
                };
                if let Some(cb) = extract_name_cb {
                    item_extracted_name = cb(msg);
                }
                if msgs.len() == 1 && expected_item_type.is_none() {
                    item_type = "Kit".to_string();
                    slot_label = section_slot.to_string();
                }
            }
            // Route Pattern dumps.
            else if resolved_type == "pattern" {
                msg_item_type = "pattern".to_string();
                item_name = if msgs.len() == 1 && expected_item_type == Some("Pattern") && expected_slot_label.is_some() {
                    format!("pattern_{slot_label}")
                } else {
                    wire_slot_to_section("pattern", usize::from(slot))
                };

                if msgs.len() == 1 && expected_item_type.is_none() {
                    item_type = "Pattern".to_string();
                    slot_label = section_slot.to_string();
                }
            }
            // Route Song dumps.
            else if resolved_type == "song" {
                msg_item_type = "song".to_string();
                item_name = if msgs.len() == 1 && expected_item_type == Some("Song") {
                    format!("song_{slot_label}")
                } else {
                    wire_slot_to_section("song", usize::from(slot))
                };

                if msgs.len() == 1 && expected_item_type.is_none() {
                    item_type = "Song".to_string();
                    slot_label = section_slot.to_string();
                }
            }
            // Route DigiPro dumps.
            else if resolved_type == "digipro" {
                msg_item_type = "digipro".to_string();
                item_name = wire_slot_to_section("digipro", usize::from(slot));
                if msgs.len() == 1 && expected_item_type.is_none() {
                    item_type = "DigiPro".to_string();
                    slot_label = section_slot.to_string();
                }
            }
        }

        let mut item = C7Item {
            section: item_name,
            data: Some(C7Data::Binary(msg.clone())),
            attrs: BTreeMap::new(),
        };
        item.attrs.insert("type".to_string(), msg_item_type);
        item.attrs.insert("format".to_string(), "sysex".to_string());
        if let Some(ref name) = item_extracted_name
            && name != "--"
        {
            item.attrs.insert("name".to_string(), name.clone());
        }

        if msgs.len() == 1
            && let Some(ref name) = item_extracted_name
        {
            extracted_name = Some(name.clone());
        }

        items.push(item);
    }

    let filename = generate_export_filename(device_shorter, &item_type, &slot_label, extracted_name.as_deref());
    (filename, items)
}

/// Reads one track's bit group out of a big-endian multi-byte bitfield (e.g. the MnM patchBusIn u16).
fn read_track_bits(decoded: &[u8], field: &KitField, track_idx: usize) -> u8 {
    let mut word = 0u64;
    for i in 0..field.size {
        word = (word << 8) | decoded[field.offset + i] as u64;
    }
    ((word >> (track_idx * field.bits)) & ((1u64 << field.bits) - 1)) as u8
}

/// Looks up a machine's unique SysEx ID from its name by searching all machine lists in the device spec.
fn machine_name_to_id(dc: &DeviceConfig, name: &str) -> u8 {
    let machines = dc.json_get("sysex_api.machines").unwrap().as_object().unwrap();
    for (key, list) in machines {
        if key == "machine_order" {
            continue;
        }
        let array = list.as_array().unwrap();
        for machine in array {
            if as_string_or(machine.json_get("name"), "") == name {
                return as_u64_or(machine.json_get("id"), 0) as u8;
            }
        }
    }
    0
}

/// Writes one track's bit group into a big-endian multi-byte bitfield.
fn write_track_bits(decoded: &mut [u8], field: &KitField, track_idx: usize, val: u8) {
    let mut word = 0u64;
    for i in 0..field.size {
        word = (word << 8) | decoded[field.offset + i] as u64;
    }
    let mask = ((1u64 << field.bits) - 1) << (track_idx * field.bits);
    word = (word & !mask) | (((val as u64) << (track_idx * field.bits)) & mask);
    for i in 0..field.size {
        decoded[field.offset + field.size - 1 - i] = (word >> (i * 8)) as u8;
    }
}

/// Returns `true` when the kit dump is one whole-payload RLE+7bit blob (MnM style) rather than sectioned (MD style).
///
/// A sectioned dump has no single `payload` field, so the lookup finds nothing, and the answer is false.
fn is_kit_rle7(dc: &DeviceConfig) -> bool {
    as_string_or(dc.json_get("sysex_layout.kit.fields.payload.encoding"), "") == "rle7"
}

/// Looks up a machine name from its numeric ID by searching all machine lists in the device spec.
fn machine_id_to_name(dc: &DeviceConfig, model_id: u8) -> String {
    let machines = dc.json_get("sysex_api.machines").unwrap().as_object().unwrap();
    for (key, list) in machines {
        if key == "machine_order" {
            continue;
        }
        let array = list.as_array().unwrap();
        for machine in array {
            if as_u64_or(machine.json_get("id"), 255) == model_id as u64 {
                return as_string_or(machine.json_get("name"), "").to_string();
            }
        }
    }
    String::new()
}

/// Identifies the data type of a SysEx message by scanning the registry for the command byte.
///
/// Returns the matching item type.
/// Returns `"unknown (0x<byte>)"` if no device or data type claims that command byte.
fn identify_type_by_registry_cmd(prod_byte: u8, cmd_byte: u8) -> String {
    // Find which data category uses this command byte as its write command.
    const ITEM_TYPES: [&str; 6] = ["global", "kit", "pattern", "song", "sample", "digipro"];

    // Elektron firmware images use command byte `0x7E`.
    // It's a fixed protocol command rather than a librarian dump, so it's matched here instead of via the device registry.
    if cmd_byte == 0x7E {
        return "firmware".to_string();
    }
    let Some(dc) = DeviceConfig::find_by_prod(prod_byte) else {
        return format!("unknown (0x{cmd_byte:02X})");
    };
    for item_type in ITEM_TYPES {
        let cmd = dc
            .json_get(&format!("sysex_api.{item_type}.write_cmd"))
            .and_then(serde_json::Value::as_u64);
        if cmd == Some(cmd_byte as u64) {
            return item_type.to_string();
        }
    }
    format!("unknown (0x{cmd_byte:02X})")
}

/// Determines the logical display order for JSON attributes within a section.
fn attr_sort_weight(key: &str) -> usize {
    const ORDER: &[&str] = &[
        "name",
        "type",
        "sound_type",
        "format",
        "device",
        "export_time",
        "description",
        "comment",
    ];
    ORDER.iter().position(|&k| k == key).unwrap_or(ORDER.len())
}

/// Looks up a named field in `sysex_layout.kit.fields`.
///
/// A device JSON that declares kit widgets or codecs without their layout is malformed.
///
/// # Panics
///
/// Panics if `sysex_layout.kit.fields` has no `name` entry.
fn kit_field_or_die(dc: &DeviceConfig, name: &str) -> KitField {
    let field = dc.layout_field_or_die("kit", name);
    KitField {
        offset: as_u64_or_die(field.json_get("offset")) as usize,
        base: as_u64_or(field.json_get("base"), 0) as usize,
        stride: as_u64_or(field.json_get("stride"), 1) as usize,
        size: as_u64_or(field.json_get("size"), 1) as usize,
        kind: as_string_or(field.json_get("kind"), "").to_string(),
        bits: as_u64_or(field.json_get("bits"), 1) as usize,
        encoding: as_string_or(field.json_get("encoding"), "").to_string(),
        wire_len: as_u64_or(field.json_get("wire_len"), 0) as usize,
    }
}

/// Returns the `track_blob` field order: which fields, in which order, make up the extracted track blob.
fn kit_track_blob_fields(dc: &DeviceConfig) -> Vec<String> {
    as_array_or(dc.json_get("sysex_layout.kit.track_blob"), &[])
        .iter()
        .filter_map(|json_val| json_val.as_str().map(std::string::ToString::to_string))
        .collect()
}

/// Bytes one field contributes to the extracted track blob (bit fields unpack to one byte).
fn kit_field_blob_len(field: &KitField) -> usize {
    if field.kind == "bit_per_track" || field.kind == "bits_per_track" {
        1
    } else {
        field.stride
    }
}
