//! Modal for editing the theme, export directories, and MIDI settings.

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::ui::widgets::{ChooseOnePill, NumberSpinner};
use c7_core::device_config::{get_bank_dir, get_base_channel, get_export_folder, read_settings, update_setting};

const THEMES: [&str; 3] = ["Light", "System", "Dark"];

/// Applies the specified color scheme to Libadwaita's style manager.
pub(crate) fn apply_color_scheme(name: &str) {
    let manager = libadwaita::StyleManager::default();
    let scheme = match name {
        "light" => libadwaita::ColorScheme::ForceLight,
        "dark" => libadwaita::ColorScheme::ForceDark,
        _ => libadwaita::ColorScheme::Default,
    };
    manager.set_color_scheme(scheme);
}

/// Global application settings modal (gear icon in header bar).
pub(crate) struct SettingsModal {
    window: libadwaita::Window,
}

impl SettingsModal {
    /// Creates a new `SettingsModal`, initializing its appearance and layout.
    pub(crate) fn new(parent: &impl IsA<gtk4::Window>) -> Self {
        let window = libadwaita::Window::new();
        window.set_title(Some("Settings"));
        window.set_transient_for(Some(parent));
        window.set_modal(true);
        window.set_default_size(500, -1);
        window.set_resizable(false);

        let toolbar_view = libadwaita::ToolbarView::new();
        toolbar_view.add_top_bar(&libadwaita::HeaderBar::new());
        toolbar_view.set_content(Some(&build_content(&window)));
        window.set_content(Some(&toolbar_view));

        SettingsModal { window }
    }

    /// Presents the settings modal to the user.
    pub(crate) fn present(&self) {
        self.window.present();
    }
}

// ── Settings content ───────────────────────────────────────────────────────────────────────────────────────

/// Constructs the UI content for the settings window, including appearance and export options.
fn build_content(window: &libadwaita::Window) -> gtk4::Box {
    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    outer.set_margin_top(16);
    outer.set_margin_bottom(16);
    outer.set_margin_start(16);
    outer.set_margin_end(16);

    let settings = read_settings();

    // ── Appearance ─────────────────────────────────────────────────────────────────────────────────────────

    let appearance_group = libadwaita::PreferencesGroup::new();
    appearance_group.set_title("Appearance");

    let theme_row = libadwaita::ActionRow::new();
    theme_row.set_title("Theme");

    let saved = settings
        .get("theme")
        .and_then(|value| value.as_str())
        .unwrap_or("dark")
        .to_lowercase();

    let active_idx = THEMES.iter().position(|theme| theme.to_lowercase() == saved).unwrap_or(2);

    let theme_pill = ChooseOnePill::new(&THEMES, active_idx);
    theme_pill.widget().set_valign(gtk4::Align::Center);
    theme_pill.connect_changed(move |idx| {
        let name = THEMES[idx as usize].to_lowercase();

        update_setting("theme", serde_json::Value::String(name.clone()));
        apply_color_scheme(&name);
    });
    theme_row.add_suffix(theme_pill.widget());
    appearance_group.add(&theme_row);

    outer.append(&appearance_group);

    // ── Export ─────────────────────────────────────────────────────────────────────────────────────────────

    let export_group = libadwaita::PreferencesGroup::new();
    export_group.set_title("Export");

    // Starting folder for every file picker (save and load).
    let export_default = get_export_folder().to_string_lossy().into_owned();
    export_group.add(&make_directory_row(
        window,
        &settings,
        "Export Directory",
        "export_dir",
        &export_default,
    ));

    // Root of the structured "Save to Bank" library.
    let bank_default = get_bank_dir().to_string_lossy().into_owned();
    export_group.add(&make_directory_row(window, &settings, "Bank Directory", "bank_dir", &bank_default));

    outer.append(&export_group);

    // ── MIDI ───────────────────────────────────────────────────────────────────────────────────────────────

    let midi_group = libadwaita::PreferencesGroup::new();
    midi_group.set_title("MIDI");

    let channel_row = libadwaita::ActionRow::new();
    channel_row.set_title("Base Channel");
    channel_row.set_subtitle("Must match the device's own MIDI base channel.\nAlmost never needs to change.");

    let channel_spin = NumberSpinner::new(f64::from(get_base_channel() + 1), 1.0, 16.0, 1.0, "", 0);
    channel_spin.widget().set_valign(gtk4::Align::Center);
    {
        let channel_spin_c = channel_spin.clone();
        channel_spin.connect_value_changed(move || {
            let val = channel_spin_c.value().round().clamp(1.0, 16.0) as i64;
            update_setting("base_channel", serde_json::Value::Number(val.into()));
        });
    }
    channel_row.add_suffix(channel_spin.widget());
    midi_group.add(&channel_row);

    outer.append(&midi_group);

    // ── TurboMIDI ──────────────────────────────────────────────────────────────────────────────────────────

    let turbo_group = libadwaita::PreferencesGroup::new();
    turbo_group.set_title("TurboMIDI");

    let turbo_row = libadwaita::ActionRow::new();
    turbo_row.set_title("TurboMIDI speeds are automatically handled by the USB MIDI device. Nothing to do here.");

    turbo_group.add(&turbo_row);
    outer.append(&turbo_group);

    outer
}

/// Builds a directory-picker row bound to a settings `key`.
///
/// `default_display` is the effective path used when the key is unset (resolved by the caller from the live default).
/// It's shown with a "(default)" hint so the user knows nothing is saved yet but exports still have somewhere to go.
fn make_directory_row(
    window: &libadwaita::Window,
    settings_map: &serde_json::Map<String, serde_json::Value>,
    title: &str,
    key: &'static str,
    default_display: &str,
) -> libadwaita::ActionRow {
    let configured = settings_map
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|dir_str| !dir_str.is_empty());

    let row = libadwaita::ActionRow::new();
    row.set_title(title);
    match configured {
        Some(path) => row.set_subtitle(path),
        None => row.set_subtitle(&format!("{default_display}  (default)")),
    }

    let browse_btn = gtk4::Button::with_label("Browse...");
    browse_btn.set_valign(gtk4::Align::Center);
    {
        let row_c = row.clone();
        let window_c = window.clone();
        let title_str = title.to_string();
        browse_btn.connect_clicked(move |_| {
            on_browse(&window_c, &row_c, key, &title_str);
        });
    }
    row.add_suffix(&browse_btn);
    row
}

/// Opens a folder picker for `key`, seeding it with the currently saved path, and persists the chosen directory.
fn on_browse(window: &libadwaita::Window, directory_row: &libadwaita::ActionRow, key: &'static str, title: &str) {
    let dialog = gtk4::FileDialog::builder().title(format!("Select {title}")).build();
    let settings = read_settings();
    // Seed the dialog with the currently saved path if one exists.
    if let Some(path) = settings.get(key).and_then(|value| value.as_str())
        && !path.is_empty()
    {
        dialog.set_initial_folder(Some(&gio::File::for_path(path)));
    }
    let directory_row_c = directory_row.clone();
    dialog.select_folder(Some(window), None::<&gio::Cancellable>, move |result| {
        // Abort if folder selection failed or was cancelled.
        let Ok(folder) = result else { return };
        let Some(path) = folder.path() else { return };
        let path_str = path.to_string_lossy().into_owned();
        directory_row_c.set_subtitle(&path_str);
        update_setting(key, serde_json::Value::String(path_str));
    });
}
