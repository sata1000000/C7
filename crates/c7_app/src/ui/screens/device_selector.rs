//! Choose which Elektron device to work with.

use std::path::Path;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{self, Align, FlowBox, Image, Label, Orientation, SelectionMode};

use c7_core::device_config::DeviceConfig;

// ── CSS helper ─────────────────────────────────────────────────────────────────────────────────────────────

/// Applies CSS to disable default hover and focus styles for `FlowBox` children.
fn suppress_flowboxchild_hover() {
    let css = gtk4::CssProvider::new();
    css.load_from_string(
        "flowboxchild { background: none; border: none; padding: 0; } \
         flowboxchild:hover, flowboxchild:focus { background: none; box-shadow: none; outline: none; }",
    );
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(&display, &css, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

/// UI screen for selecting a hardware device from the available registry.
pub(crate) struct DeviceSelectorScreen {
    root: gtk4::Box,
}

impl DeviceSelectorScreen {
    /// Initializes the device selector screen.
    ///
    /// `on_device_selected_cb` receives the config file path for the chosen device.
    pub(crate) fn new(on_device_selected_cb: impl Fn(String) + 'static) -> Self {
        let root = gtk4::Box::new(Orientation::Vertical, 32);
        root.set_halign(Align::Center);
        root.set_valign(Align::Center);
        root.set_vexpand(true);
        root.set_hexpand(true);

        let title = Label::new(Some("Select Your Device"));
        title.add_css_class("title-1");
        root.append(&title);

        let flowbox = FlowBox::new();
        flowbox.set_valign(Align::Start);
        flowbox.set_max_children_per_line(2);
        flowbox.set_selection_mode(SelectionMode::None);
        flowbox.set_row_spacing(16);
        flowbox.set_column_spacing(16);
        suppress_flowboxchild_hover();

        // Create and append a selection card for every registered device.
        let callback = Rc::new(on_device_selected_cb);
        for dc in DeviceConfig::get_all_configs() {
            let icon_path = dc.icon_path.clone().unwrap_or_default();
            let config_path = dc
                .config_path
                .clone()
                .map(|path_val| path_val.to_string_lossy().to_string())
                .unwrap_or_default();
            let btn = create_device_card(&dc.device_short, &icon_path);
            let callback_c = Rc::clone(&callback);
            btn.connect_clicked(move |_| callback_c(config_path.clone()));
            flowbox.insert(&btn, -1);
        }
        root.append(&flowbox);

        DeviceSelectorScreen { root }
    }

    /// Returns the root widget for embedding in the navigation stack.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ── Device card helper ─────────────────────────────────────────────────────────────────────────────────────

/// Constructs a clickable card widget for a specific device.
fn create_device_card(title_text: &str, icon_path: &Path) -> gtk4::Button {
    let btn = gtk4::Button::new();
    btn.set_size_request(260, 130);
    btn.add_css_class("card");

    let vbox = gtk4::Box::new(Orientation::Vertical, 0);
    vbox.set_halign(Align::Center);
    vbox.set_valign(Align::Center);

    // Determine the correct pixel size dynamically to enforce a uniform height.
    let aspect_ratio = get_svg_aspect_ratio(icon_path);
    let calculated_size = (42.074 * aspect_ratio) as i32;

    let icon_container = gtk4::Box::new(Orientation::Vertical, 0);
    icon_container.set_size_request(108, 108);

    let icon = Image::new();
    let icon_file = gio::File::for_path(icon_path);
    let file_icon = gio::FileIcon::new(&icon_file);
    icon.set_from_gicon(&file_icon);
    icon.set_pixel_size(calculated_size);
    icon.set_halign(Align::Center);
    icon.set_valign(Align::Center);
    icon.set_hexpand(true);
    icon.set_vexpand(true);
    icon_container.append(&icon);
    vbox.append(&icon_container);

    let title = Label::new(Some(title_text));
    title.add_css_class("title-3");

    // Pull the text up into the empty square bounds of the icon.
    let pull_css = gtk4::CssProvider::new();
    pull_css.load_from_string(".pull-up { margin-top: -21px; margin-bottom: 23px; }");
    gtk4::style_context_add_provider_for_display(&title.display(), &pull_css, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
    title.add_css_class("pull-up");

    vbox.append(&title);
    btn.set_child(Some(&vbox));
    btn
}

// ── SVG aspect-ratio helper ────────────────────────────────────────────────────────────────────────────────

/// Parses an SVG file to determine its aspect ratio from the viewBox.
fn get_svg_aspect_ratio(file_path: &Path) -> f64 {
    let Ok(text) = std::fs::read_to_string(file_path) else {
        return 1.0;
    };
    // Locate viewBox="...", the standard SVG attribute.
    let Some(viewbox_start) = text.find("viewBox") else {
        return 1.0;
    };
    let tail = &text[viewbox_start..];
    let Some(quote_start) = tail.find('"') else {
        return 1.0;
    };
    let inner = &tail[(quote_start + 1)..];
    let Some(quote_end) = inner.find('"') else {
        return 1.0;
    };
    let val = &inner[..quote_end];
    // Parse four space/comma-separated numbers: min-x min-y width height.
    let parts: Vec<f64> = val
        .split([' ', ','])
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<f64>().ok())
        .collect();

    if parts.len() >= 4 && parts[3] > 0.0 {
        parts[2] / parts[3]
    } else {
        1.0
    }
}
