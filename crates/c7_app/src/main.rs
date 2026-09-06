//! Process startup.
//!
//! Resolves the platform's data paths, then hands off to the UI layer.

// Suppress the console window that Windows wants to spawn alongside the GUI.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod ui;

use glib::ExitCode;
use libadwaita::Application;
use libadwaita::prelude::*;

use c7_core::device_config;
use c7_core::utils::as_string_or;

const APP_ID: &str = "io.github.sata1000000.C7";

/// Main entry point for the C7 application.
fn main() -> ExitCode {
    // In development, `CARGO_MANIFEST_DIR` always points to `crates/c7_app/` regardless of where `cargo run` was invoked from.
    // In production, data lives next to the binary on Linux/Windows, or in `Contents/Resources/` on macOS.
    let root = if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        std::path::PathBuf::from(dir)
    } else {
        let exe = std::env::current_exe().unwrap();
        let exe_dir = exe.parent().unwrap();

        if cfg!(target_os = "macos") {
            exe_dir.join("../Resources")
        } else {
            exe_dir.to_path_buf()
        }
    };
    std::env::set_current_dir(root).unwrap();

    // On macOS, GTK's icon theme lookup uses `XDG_DATA_DIRS`.
    // The Rust bundle has to set this. Otherwise, GTK will fall back to Homebrew paths that likely don't exist on the machine.
    #[cfg(target_os = "macos")]
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let resources_share = exe_dir.join("../Resources/share");
            if resources_share.is_dir() {
                let existing = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
                let new_val = if existing.is_empty() {
                    resources_share.to_string_lossy().into_owned()
                } else {
                    format!("{}:{}", resources_share.to_string_lossy(), existing)
                };
                // SAFETY: no other threads exist yet, so no concurrent reader can race this write.
                unsafe {
                    std::env::set_var("XDG_DATA_DIRS", new_val);
                }
            }
        }
    }

    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(|app| {
        // Read the theme preference before the first window is drawn.
        let settings = device_config::read_settings();
        let theme = as_string_or(settings.get("theme"), "dark");
        ui::modals::settings::apply_color_scheme(theme);

        let window = ui::main_window::SataC7App::new(app);
        window.present();
    });

    app.run()
}
