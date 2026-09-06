//! Compile-time device identity.
//!
//! `Device` is the trait every generated device marker type implements.
//! `declare_device!` builds one type per device from `build.rs`'s directory scan.
//! `plugin.rs` invokes it once per discovered device via the generated `include!`.

use c7_core::device_config::DeviceConfig;

/// One enabled device, generated as a marker type by `build.rs`.
///
/// Identity is compile-time so each generated plugin gets its own CLAP ID and name.
pub trait Device: 'static + Send + Sync {
    const CLAP_ID: &'static str;
    const NAME: &'static str;

    /// The device's baked JSON, parsed once on first use.
    fn config() -> &'static DeviceConfig;
}

macro_rules! declare_device {
    ($struct_name:ident, $id_suffix:expr, $display_short:expr, $json_path:expr) => {
        /// Marker type for the device, generated automatically via directory discovery.
        pub struct $struct_name;

        impl Device for $struct_name {
            const CLAP_ID: &'static str = concat!("com.sata.c7-bridge-", $id_suffix);
            const NAME: &'static str = concat!("C7 Bridge (", $display_short, ")");

            fn config() -> &'static ::c7_core::device_config::DeviceConfig {
                static CONFIG: std::sync::LazyLock<::c7_core::device_config::DeviceConfig> = std::sync::LazyLock::new(|| {
                    let raw = include_str!($json_path);
                    ::c7_core::device_config::DeviceConfig::new(serde_json::from_str(raw).unwrap())
                });
                &CONFIG
            }
        }
    };
}

pub(crate) use declare_device;
