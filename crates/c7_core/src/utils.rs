//! General-purpose functions that aren't hardware-protocol or audio specific.

use std::sync::Arc;

use serde_json::Value;

/// Audio extensions C7 can decode, plus raw `.sds` sample dumps and `.syx` DigiPro waveforms.
pub const AUDIO_EXTS: &[&str] = &[".wav", ".aif", ".aiff", ".flac", ".ogg", ".mp3", ".wv", ".m4a", ".sds", ".syx"];
/// Image extensions accepted for image-to-wavetable conversion.
pub const IMAGE_EXTS: &[&str] = &[".png", ".jpg", ".jpeg", ".bmp", ".gif", ".tiff", ".tif", ".webp"];
/// SysEx / `C7` file extensions carrying instrument data.
pub const SYSEX_EXTS: &[&str] = &[".syx", ".sysex", ".c7"];

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Thread-safe status callback.
///
/// It takes a one-line status message and shows it to the user.
/// Each frontend decides what that means. The GTK app updates its status label, a CLI could print.
///
/// Cloneable and `Send`, so long-running transfers can report from background threads.
pub type StatusFn = Arc<dyn Fn(&str) + Send + Sync>;

/// Adds a `json_get()` method to `serde_json::Value` that walks a dot-separated path (e.g. "system.joystick.cc") through nested JSON.
///
/// It has to be a trait, not a plain function, because `Value` is a foreign type, and only a trait can attach a method to a foreign type.
///
/// Pair with the `as_*_or(_die)` helpers.
pub trait JsonPath {
    /// Returns the value at the dot-separated `path`.
    /// Returns `None` if any segment along the way is missing.
    fn json_get(&self, path: &str) -> Option<&Value>;
}

impl JsonPath for Value {
    /// Follows each JSON key in turn, bailing out with `None` the moment a segment is absent.
    ///
    /// Supports array indexing if the key parses to a usize.
    fn json_get(&self, path: &str) -> Option<&Value> {
        let mut current = self;
        for key in path.split('.') {
            if let Ok(idx) = key.parse::<usize>()
                && let Some(val) = current.get(idx)
            {
                current = val;
                continue;
            }
            current = current.get(key)?;
        }
        Some(current)
    }
}

/// Returns a UTC ISO 8601 timestamp string ("`YYYY-MM-DDTHH:MM:SSZ`").
///
/// Used for time metadata in exported files.
pub fn now_iso_string() -> String {
    glib::DateTime::now_utc()
        .and_then(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Returns a compact timestamp string ("`YYYYMMDD_HHMMSS`").
///
/// Used for exported filenames.
pub fn now_compact_timestamp() -> String {
    glib::DateTime::now_utc()
        .and_then(|dt| dt.format("%Y%m%d_%H%M%S"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Returns the current local wall-clock time as "`HH:MM:SS`".
///
/// Used for timestamping live log lines.
pub fn now_clock_string() -> String {
    glib::DateTime::now_local()
        .and_then(|dt| dt.format("%H:%M:%S"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Returns a pseudo-random number in `0..=max`, inclusive.
///
/// Good enough for randomizing actions. Not good enough for cryptography.
pub fn pseudo_rand(max: u32) -> u32 {
    glib::random_int_range(0, max as i32 + 1) as u32
}

/// Returns the index of the first `byte` at or after `start`.
/// Returns `None` when it doesn't occur.
pub fn find_byte(data: &[u8], byte: u8, start: usize) -> Option<usize> {
    data[start..].iter().position(|&search_byte| search_byte == byte).map(|i| i + start)
}

/// Returns a unique temporary-file path.
///
/// The temporary file is formatted like: `<tmp_dir>/c7_<prefix>_<nanos>.<ext>`.
pub fn temp_path(prefix: &str, extension: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("c7_{prefix}_{}.{extension}", epoch_nanos()))
}

/// Parses a hexadecimal string into bytes.
///
/// Returns `None` if the string is odd-length or contains a non-hex character.
pub fn from_hex(hex_str: &str) -> Option<Vec<u8>> {
    let hex_str = hex_str.trim();
    if !hex_str.len().is_multiple_of(2) {
        return None;
    }
    (0..hex_str.len() / 2)
        .map(|i| u8::from_str_radix(&hex_str[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// Encodes a byte slice into a lowercase hexadecimal string.
pub fn bytes_to_hex_string(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut acc, &byte| {
        let _ = write!(acc, "{byte:02x}");
        acc
    })
}

/// Rounds a value to one decimal place.
pub fn round_to_one_decimal(val: f64) -> f64 {
    (val * 10.0).round() / 10.0
}

/// Sanitizes a string to be used safely as a filename, replacing spaces with underscores.
pub fn sanitize_filename(filename: &str) -> String {
    strip_illegal_filename_chars(filename).replace(' ', "_")
}

/// Lowercases a display name into a stable param-ID fragment (alphanumerics kept, everything else becomes '_').
pub fn slug_string(text: &str) -> String {
    text.chars()
        .map(|char_val| {
            if char_val.is_ascii_alphanumeric() {
                char_val.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Capitalizes the first character of a kind string ("kit" → "Kit").
pub fn cap_first_char(text: &str) -> String {
    let mut chars_iter = text.chars();
    chars_iter
        .next()
        .map(|first_char| first_char.to_uppercase().collect::<String>() + chars_iter.as_str())
        .unwrap_or_default()
}

/// Reads the array result of an optional JSON lookup.
///
/// Falls through to `default` if the lookup is a non-array value or missing.
pub fn as_array_or<'a>(val: Option<&'a Value>, default: &'a [Value]) -> &'a [Value] {
    val.and_then(Value::as_array).map_or(default, Vec::as_slice)
}

/// Reads the array result of an optional JSON lookup.
///
/// Use this only where the field is trusted to always be present.
///
/// # Panics
///
/// Panics if the lookup is missing or a non-array value.
#[track_caller]
pub fn as_array_or_die(val: Option<&Value>) -> &[Value] {
    val.and_then(Value::as_array)
        .map(Vec::as_slice)
        .expect("missing or non-array JSON field")
}

/// Reads the bool result of an optional JSON lookup.
///
/// Falls through to `default` if the lookup is a non-bool value or missing.
pub fn as_bool_or(val: Option<&Value>, default: bool) -> bool {
    val.and_then(Value::as_bool).unwrap_or(default)
}

/// Reads the bool result of an optional JSON lookup.
///
/// Use this only where the field is trusted to always be present.
///
/// # Panics
///
/// Panics if the lookup is missing or a non-bool value.
#[track_caller]
pub fn as_bool_or_die(val: Option<&Value>) -> bool {
    val.and_then(Value::as_bool).expect("missing or non-bool JSON field")
}

/// Reads the string result of an optional JSON lookup.
///
/// Falls through to `default` if the lookup is a non-string value or missing.
pub fn as_string_or<'a>(val: Option<&'a Value>, default: &'a str) -> &'a str {
    val.and_then(Value::as_str).unwrap_or(default)
}

/// Reads the string-array result of an optional JSON lookup.
///
/// Non-string entries are skipped, and a missing or non-array lookup yields an empty vec.
pub fn as_string_vec_or_empty(val: Option<&Value>) -> Vec<String> {
    val.and_then(Value::as_array)
        .map(|array| array.iter().filter_map(|entry| entry.as_str().map(ToString::to_string)).collect())
        .unwrap_or_default()
}

/// Reads the u64 result of an optional JSON lookup.
///
/// Falls through to `default` if the lookup is a non-numeric value or missing.
pub fn as_u64_or(val: Option<&Value>, default: u64) -> u64 {
    val.and_then(Value::as_u64).unwrap_or(default)
}

/// Reads the u64 result of an optional JSON lookup.
///
/// Use this only where the field is trusted to always be present.
///
/// # Panics
///
/// Panics if the lookup is missing or a non-numeric value.
#[track_caller]
pub fn as_u64_or_die(val: Option<&Value>) -> u64 {
    val.and_then(Value::as_u64).expect("missing or non-u64 JSON field")
}

/// Reads the f64 result of an optional JSON lookup.
///
/// Use this only where the field is trusted to always be present.
///
/// # Panics
///
/// Panics if the lookup is missing or a non-f64 value.
#[track_caller]
pub fn as_f64_or_die(val: Option<&Value>) -> f64 {
    val.and_then(Value::as_f64).expect("missing or non-f64 JSON field")
}

/// Reads the string result of an optional JSON lookup.
///
/// Use this only where the field is trusted to always be present.
///
/// # Panics
///
/// Panics if the lookup is missing or a non-string value.
#[track_caller]
pub fn as_string_or_die(val: Option<&Value>) -> &str {
    val.and_then(Value::as_str).expect("missing or non-string JSON field")
}

/// Formats a floating-point value to a string with a specified number of decimal digits.
pub fn format_value(val: f64, digits: i32) -> String {
    if digits == 0 {
        format!("{}", val as i64)
    } else {
        format!("{:.prec$}", val, prec = digits as usize)
    }
}

/// Rounds a floating-point value to a specified number of decimal digits.
pub fn round_to_digits(val: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    (val * factor).round() / factor
}

/// Returns the nanoseconds since the Unix epoch.
///
/// Used for temporary file naming.
pub fn epoch_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

/// Strips typical illegal characters in OS filenames, then trims surrounding whitespace.
pub fn strip_illegal_filename_chars(filename: &str) -> String {
    filename
        .chars()
        .filter(|&char_val| !r#"\/*?:"<>|"#.contains(char_val))
        .collect::<String>()
        .trim()
        .to_string()
}
