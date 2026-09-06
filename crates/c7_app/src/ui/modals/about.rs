//! Modal for showing the app version, developer name, and project links.

use std::sync::OnceLock;

use libadwaita::prelude::*;

/// Global cache for the resolved icon name.
static ICON_NAME: OnceLock<String> = OnceLock::new();

/// Registers C7's SVG icon with the GTK theme and returns its name.
fn resolve_icon_name() -> &'static str {
    ICON_NAME.get_or_init(|| {
        // Use C7's app ID for icon lookup when running as a Flatpak.
        if std::env::var("FLATPAK_ID").is_ok() {
            return "io.github.sata1000000.C7".to_string();
        }

        let temp_dir = std::env::temp_dir().join("c7_dev_icons");
        let apps_dir = temp_dir.join("hicolor").join("scalable").join("apps");

        std::fs::create_dir_all(&apps_dir).unwrap();
        std::fs::copy("icon.svg", apps_dir.join("c7.svg")).unwrap();

        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::IconTheme::for_display(&display).add_search_path(&temp_dir);
        }

        "c7".to_string()
    })
}

/// Namespace for presenting C7's About dialog.
pub(crate) struct AboutModal;

impl AboutModal {
    /// Creates and presents a Libadwaita About dialog.
    pub(crate) fn show(parent: &impl IsA<gtk4::Widget>) {
        let about = libadwaita::AboutDialog::builder()
            .application_name("C7")
            .application_icon(resolve_icon_name())
            .developer_name("sata1000000")
            .version(env!("CARGO_PKG_VERSION"))
            .website("https://github.com/sata1000000/C7")
            .issue_url("https://github.com/sata1000000/C7/issues")
            .build();

        // `AdwDialog` isn't a window, so it takes its parent at `present()`.
        about.present(Some(parent));
    }
}
