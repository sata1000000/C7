//! Functions for the Machinedrum pattern SysEx codec.
//!
//! MnM patterns aren't handled yet.
//! The MnM format uses a different RLE+7-bit encoding and a completely different trig/lock layout.

/*
C7 intentionally does not have a built-in pattern editor.

The SysEx parsing logic would be complicated and it's hyper-specific to each device.

C7 decides that GUI pattern editing is not worth it.
Especially because sequencing is already something that Elektron devices are great at. Just use the device.
*/

use crate::device_config::DeviceConfig;
use crate::sysex::update_elektron_checksum;

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Returns a blank MD pattern SysEx blob with all trigs, locks, and accents cleared.
///
/// Clears patterns directly by zeroing the data.
/// Preserves metadata (kit link, scale), matching the hardware's "Clear Pattern" state.
pub fn blank_md_pattern(dc: &DeviceConfig, raw: &[u8]) -> Vec<u8> {
    let mut raw_vec = raw.to_vec();

    // Force the length and lock count to their minimums.
    let lock_count = get_pattern_field(dc, raw_vec.len(), "lock_count").offset;
    let length_field = get_pattern_field(dc, raw_vec.len(), "pattern_length").offset;
    raw_vec[lock_count] = 0;
    raw_vec[length_field] = 16;

    // Zero out all 32-step trig and lock data.
    let zero_field = |vec: &mut [u8], name: &str| {
        let field = get_pattern_field(dc, vec.len(), name);
        if vec.len() >= field.offset + field.wire_len {
            vec[field.offset..field.offset + field.wire_len].fill(0);
        }
    };

    zero_field(&mut raw_vec, "trig_pattern_lo");
    zero_field(&mut raw_vec, "lock_pattern");
    zero_field(&mut raw_vec, "accent_pattern_lo");
    zero_field(&mut raw_vec, "locks_lo");
    zero_field(&mut raw_vec, "extra_pattern_lo");

    // Zero out all data inside steps 32-64 (if it exists).
    if raw_vec.len() > 3000 {
        zero_field(&mut raw_vec, "extra_pattern_hi");
    }

    update_elektron_checksum(&mut raw_vec);
    raw_vec
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

struct PatternField {
    offset: usize,
    wire_len: usize,
}

/// Finds the named field in `sysex_layout.pattern`, choosing `fields_32` vs `fields_64` by raw dump length.
fn get_pattern_field(dc: &DeviceConfig, raw_len: usize, name: &str) -> PatternField {
    let fields_key = if raw_len > 3000 { "fields_64" } else { "fields_32" };
    let field = dc.json_get(&format!("sysex_layout.pattern.{fields_key}.{name}")).unwrap();
    PatternField {
        offset: field["offset"].as_u64().unwrap() as usize,
        wire_len: field["wire_len"].as_u64().unwrap_or(0) as usize,
    }
}
