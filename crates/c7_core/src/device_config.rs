//! Functions for device JSON loading and settings persistence.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde_json::Value;

use crate::utils::JsonPath;

/// A static cache of all parsed device JSON files, loaded exactly once.
static ALL_CONFIGS: LazyLock<Vec<DeviceConfig>> = LazyLock::new(|| {
    let mut results = Vec::new();
    // Packaged builds ship `devices/` next to the binary, which `main.rs` sets as the working directory.
    // In development the working directory is `crates/c7_app/` (`CARGO_MANIFEST_DIR`); the specs live one level up, in `c7_core`.
    let mut devices_dir = std::path::PathBuf::from("devices");
    if !devices_dir.exists() {
        devices_dir = std::path::PathBuf::from("../c7_core/devices");
    }
    if devices_dir.exists() {
        walk_json_files(&devices_dir, &mut results);
    }
    results
});

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Encapsulates the identity and capabilities of a specific hardware device.
#[derive(Clone)]
pub struct DeviceConfig {
    /// Raw device JSON data, kept for callers that need fields not promoted to struct members.
    pub device_json: Value,
    pub device_short: String,
    pub device_shorter: String,
    pub config_path: Option<PathBuf>,
    pub icon_path: Option<PathBuf>,
    pub sysex_header: Vec<u8>,
    pub prod: u8,
}

impl DeviceConfig {
    /// Initializes the configuration from raw device registry data.
    pub fn new(data: Value) -> Self {
        let device_short = data["device_short"].as_str().unwrap().to_string();
        let device_shorter = data["device_shorter"].as_str().unwrap().to_string();
        let sysex_header: Vec<u8> = data["sysex_header"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|value| value.as_u64().map(|byte_val| byte_val as u8))
            .collect();
        // `sysex_header[4]` is the product-ID byte used for device auto-detection.
        let prod = sysex_header[4];
        DeviceConfig {
            device_json: data,
            device_short,
            device_shorter,
            config_path: None,
            icon_path: None,
            sysex_header,
            prod,
        }
    }

    /// Same as `find_by_prod()`/`find_by_name()`, but keyed by config file path.
    ///
    /// # Panics
    ///
    /// Panics if the path isn't already indexed in the cache.
    pub fn find_by_path_or_die(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        ALL_CONFIGS
            .iter()
            .find(|dc| {
                dc.config_path.as_deref() == Some(path) || dc.config_path.as_ref().is_some_and(|config_path| config_path.ends_with(path))
            })
            .cloned()
            .unwrap_or_else(|| panic!("Device config not found in cache for path: {}", path.display()))
    }

    /// Reads and parses one device JSON from disk, as `walk_json_files()` builds the `ALL_CONFIGS` cache.
    fn load_from_disk(path: &Path) -> Self {
        let contents = std::fs::read_to_string(path).unwrap();
        let data: Value = serde_json::from_str(&contents).unwrap();
        let mut instance = Self::new(data);
        instance.config_path = Some(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
        instance.icon_path = path.parent().map(|dir| {
            let absolute_dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            absolute_dir.join("logo-symbolic.svg")
        });
        instance
    }

    /// Yields `DeviceConfig` objects for all discovered device JSON files in the `devices/` registry.
    pub fn get_all_configs() -> impl Iterator<Item = DeviceConfig> {
        ALL_CONFIGS.iter().cloned()
    }

    /// Looks up a device by the product ID byte that Elektron puts in every SysEx header.
    ///
    /// Returns the matching `DeviceConfig`.
    /// Returns `None` if no enabled device uses that byte.
    pub fn find_by_prod(prod_id: u8) -> Option<DeviceConfig> {
        Self::get_all_configs().find(|dc| dc.prod == prod_id)
    }

    /// Reads a hex byte from `sysex_api.commands.<cmd_name>.<field>` in the device JSON.
    pub fn sysex_command_byte(&self, cmd_name: &str, field: &str) -> u8 {
        self.json_get(&format!("sysex_api.commands.{cmd_name}.{field}"))
            .and_then(|val| val.as_str())
            .and_then(|val_str| u8::from_str_radix(val_str.trim_start_matches("0x"), 16).ok())
            .unwrap()
    }

    /// Looks up a device by its `device_short` or `device_shorter` name.
    ///
    /// Returns the matching `DeviceConfig`.
    /// Returns `None` if the name is empty or unrecognized.
    pub fn find_by_name(name: &str) -> Option<DeviceConfig> {
        if name.is_empty() {
            return None;
        }
        let name = name.trim();
        Self::get_all_configs().find(|dc| dc.device_short == name || dc.device_shorter == name)
    }

    /// Walks a dot-separated path (e.g. "`sampler.native_bit_depth`") through the device's JSON.
    ///
    /// Returns the value at that path.
    /// Returns `None` if any segment is missing.
    ///
    /// Pair with `as_u64_or_die()` / `as_f64_or_die()` (util) for required reads.
    pub fn json_get(&self, path: &str) -> Option<&Value> {
        self.device_json.json_get(path)
    }

    /// Checks whether the device JSON activates a feature on at the given path.
    ///
    /// A device opts out of a feature by not having the path defined, or by locking the value with `null` or `false`.
    pub fn has_gate(&self, path: &str) -> bool {
        self.json_get(path)
            .is_some_and(|val| !val.is_null() && val.as_bool() != Some(false))
    }

    /// Finds a named field record in `sysex_layout.<space>.fields`.
    ///
    /// A field table keys its entries by name. Fields inside an encoded payload sit one level down, in the `payload_structure` beside it.
    pub fn layout_field(&self, space: &str, name: &str) -> Option<&Value> {
        let fields = self.json_get(&format!("sysex_layout.{space}.fields"))?;
        fields
            .json_get(name)
            .or_else(|| fields.json_get(&format!("payload_structure.{name}")))
    }

    /// Same as `layout_field()`, but for callers that require the field to exist.
    ///
    /// For callers already gated on the layout section existing, so a missing field means a malformed device JSON.
    ///
    /// # Panics
    ///
    /// Panics with the missing field's name if `sysex_layout.<space>.fields` has no `name` entry.
    pub fn layout_field_or_die(&self, space: &str, name: &str) -> &Value {
        self.layout_field(space, name)
            .unwrap_or_else(|| panic!("sysex_layout.{space}.fields missing '{name}'"))
    }

    /// Looks up a named field's offset in `sysex_layout.<space>.fields`.
    ///
    /// Field-table offsets count from the SysEx start byte.
    /// A `<name>_structure` restarts at zero, measured from wherever its parent field sits.
    fn layout_offset(&self, space: &str, name: &str) -> Option<i64> {
        self.layout_field(space, name)
            .and_then(|field| field.json_get("offset"))
            .and_then(Value::as_i64)
    }

    /// Same as `layout_offset()`, but for callers that require the field to exist.
    ///
    /// For callers already gated on the layout section existing, so a missing field means a malformed device JSON.
    ///
    /// # Panics
    ///
    /// Panics with the missing field's name if `sysex_layout.<space>.fields` has no `name` entry.
    pub fn layout_offset_or_die(&self, space: &str, name: &str) -> i64 {
        self.layout_offset(space, name)
            .unwrap_or_else(|| panic!("sysex_layout.{space}.fields missing '{name}'"))
    }

    /// Checks if the device matches the shorter name (e.g., "MD", "MnM", "SS").
    pub fn is_device(&self, shorter: &str) -> bool {
        self.device_shorter == shorter
    }
}

/// Returns the starting folder for all file pickers (save and load).
///
/// Defaults to the user's Downloads folder, overridden by `export_dir` when the user has set one.
pub fn get_export_folder() -> PathBuf {
    read_settings()
        .get("export_dir")
        .and_then(|value| value.as_str())
        .filter(|dir_str| !dir_str.is_empty())
        .map_or_else(downloads_path, PathBuf::from)
}

/// Returns the configured Elektron device base MIDI channel as a byte.
///
/// Ready to drop straight into a SysEx header (byte 5 of `F0 00 20 3C [prod] [ch] ...`).
/// Defaults to channel 1 (`0x00`), the factory setting almost nobody changes.
pub fn get_base_channel() -> u8 {
    read_settings()
        .get("base_channel")
        .and_then(Value::as_u64)
        .map_or(0, |val| val.clamp(1, 16) as u8 - 1)
}

/// Returns the root directory of the structured "Save to Bank" library.
///
/// Defaults to `<Downloads>/C7_Bank`, overridden by `bank_dir` when the user has set one.
pub fn get_bank_dir() -> PathBuf {
    read_settings()
        .get("bank_dir")
        .and_then(|value| value.as_str())
        .filter(|dir_str| !dir_str.is_empty())
        .map_or_else(|| downloads_path().join("C7_Bank"), PathBuf::from)
}

/// Updates a single setting value and immediately saves the changes to the file.
pub fn update_setting(key: &str, val: Value) {
    let mut settings = read_settings();
    settings.insert(key.to_string(), val);
    write_settings(&settings);
}

/// Loads the user's application settings from the configuration file.
///
/// Returns an empty map if the file is absent.
///
/// # Panics
///
/// Panics if the file contains invalid JSON.
pub fn read_settings() -> serde_json::Map<String, Value> {
    match std::fs::read_to_string(settings_file_path()) {
        Ok(json_str) => serde_json::from_str(&json_str).unwrap(),
        Err(_) => serde_json::Map::new(),
    }
}

/// Saves the settings map to the configuration file, ensuring the directory exists.
pub fn write_settings(settings: &serde_json::Map<String, Value>) {
    let path = settings_file_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(&path, json);
    }
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// Recursively walks `dir`, loading every `.json` file as a `DeviceConfig`.
///
/// The device list is sorted by alphabetical order.
fn walk_json_files(dir: &Path, out: &mut Vec<DeviceConfig>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<std::path::PathBuf> = entries.filter_map(Result::ok).map(|entry| entry.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            walk_json_files(&path, out);
        } else if path.extension().and_then(|extension_val| extension_val.to_str()) == Some("json") {
            out.push(DeviceConfig::load_from_disk(&path));
        }
    }
}

/// Resolves the user's Downloads directory, falling through to the home directory.
fn downloads_path() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Downloads).unwrap_or_else(glib::home_dir)
}

/// Path to the settings file (`settings.json`) inside the data directory.
fn settings_file_path() -> PathBuf {
    data_dir().join("settings.json")
}

/// Directory where all writable app data lives.
///
/// If running from a distributed bundle, the data lives in that bundle's writable location.
/// If running live from source, the data lives in the repo's `settings/` folder.
fn data_dir() -> PathBuf {
    bundle_data_dir().unwrap_or_else(|| PathBuf::from("settings"))
}

/// Locates the writable data directory for a packaged build.
///
/// Returns `Some(dir)` when running as a distributed bundle, identifying which one and where it keeps writable data.
/// Returns `None` when running live from source.
fn bundle_data_dir() -> Option<PathBuf> {
    // Linux Flatpak bundle (`build-linux.yml`):
    // Data lives under `XDG_CONFIG_HOME`.
    if std::env::var("FLATPAK_ID").is_ok() {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let config_dir = std::env::var("XDG_CONFIG_HOME").map_or_else(|_| PathBuf::from(home).join(".config"), PathBuf::from);
        return Some(config_dir.join("C7"));
    }

    // Windows .exe bundle (`build-windows.yml`):
    // Data lives in %APPDATA%\C7.
    #[cfg(target_os = "windows")]
    if std::env::var("CARGO_MANIFEST_DIR").is_err() {
        let appdata = std::env::var("APPDATA")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        return Some(PathBuf::from(appdata).join("C7"));
    }

    // macOS .app bundle (`build-macos.yml`):
    // The executable lives inside the .app.
    #[cfg(target_os = "macos")]
    if let Ok(exe) = std::env::current_exe() {
        if exe.to_string_lossy().contains(".app/") {
            if let Some(dir) = exe.parent() {
                return Some(dir.join("settings"));
            }
        }
    }

    None
}
