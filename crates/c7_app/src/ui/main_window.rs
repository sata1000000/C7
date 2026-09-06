//! The main application window.
//!
//! Owns the navigation stack, the device selector, and the MIDI port and delay controls in the top bar.
//! Feature screens are built on demand and torn down when the device changes.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk4::{
    self, Align, Button, FlowBox, Image, Justification, Label, Orientation, PolicyType, ScrolledWindow, SelectionMode, Stack,
    StackTransitionType, gio,
};
use libadwaita::prelude::*;
use libadwaita::{HeaderBar, ToolbarView};

use crate::ui::modals::about::AboutModal;
use crate::ui::modals::c7_utilities::C7UtilitiesModal;
use crate::ui::modals::settings::SettingsModal;
use crate::ui::screens::bridge_status::BridgeScreen;
use crate::ui::screens::device_selector::DeviceSelectorScreen;
use crate::ui::screens::firmware_tools::FirmwareToolsScreen;
use crate::ui::screens::kit_editor::KitEditorScreen;
use crate::ui::screens::librarian::LibrarianScreen;
use crate::ui::screens::midi_monitor::MidiMonitorScreen;
use crate::ui::screens::midi_tricks::MidiTricksScreen;
use crate::ui::screens::receive_digipro::ReceiveDigiproScreen;
use crate::ui::screens::receive_sample::ReceiveSampleScreen;
use crate::ui::screens::sysex_tricks::SysexTricksScreen;
use crate::ui::screens::upload_digipro::UploadDigiproScreen;
use crate::ui::screens::upload_sample::UploadSampleScreen;
use crate::ui::widgets::{CustomDropdown, NumberSpinner};
use c7_core::bridge_interfacing::start_bridge_server;
use c7_core::device_config::{DeviceConfig, read_settings, write_settings};
use c7_core::midi::{MANUAL_PORT, get_output_names, init_persistent_port, set_midi_delay_ms};
use c7_core::utils::as_string_or;

/// Mutable state shared across closures.
///
/// Holds no GTK widgets of its own.
struct AppState {
    active_config: Option<String>,
    device_shorter: String,
}

// ── Primary application window ─────────────────────────────────────────────────────────────────────────────

/// The primary window for the C7 application, managing the navigation stack and global settings.
pub(crate) struct SataC7App {
    window: libadwaita::ApplicationWindow,
}

impl SataC7App {
    /// Initializes the main window, UI components, and the navigation stack.
    pub(crate) fn new(app: &libadwaita::Application) -> Self {
        let window = libadwaita::ApplicationWindow::new(app);
        window.set_title(Some("Sata C7"));
        window.set_default_size(1050, 900);

        // ── Header bar ─────────────────────────────────────────────────────────────────────────────────────

        let header_bar = HeaderBar::new();

        // Information Button
        let info_btn = Button::from_icon_name("help-about-symbolic");
        info_btn.add_css_class("flat");
        info_btn.set_tooltip_text(Some("About C7"));
        header_bar.pack_end(&info_btn);

        // Utilities Button (on the left of info button)
        let utilities_btn = Button::from_icon_name("applications-engineering-symbolic");
        utilities_btn.add_css_class("flat");
        utilities_btn.set_tooltip_text(Some("C7 Utilities"));
        header_bar.pack_end(&utilities_btn);

        // Settings Button
        let settings_btn = Button::from_icon_name("preferences-system-symbolic");
        settings_btn.add_css_class("flat");
        settings_btn.set_tooltip_text(Some("Settings"));
        header_bar.pack_end(&settings_btn);

        // MIDI Monitor Button (only shown after a device is selected)
        let monitor_btn = Button::from_icon_name("utilities-terminal-symbolic");
        monitor_btn.add_css_class("flat");
        monitor_btn.set_tooltip_text(Some("MIDI Monitor"));
        monitor_btn.set_visible(false);
        header_bar.pack_end(&monitor_btn);

        // ── Toolbar view + content box ─────────────────────────────────────────────────────────────────────

        let toolbar_view = ToolbarView::new();
        toolbar_view.add_top_bar(&header_bar);

        let content_box = gtk4::Box::new(Orientation::Vertical, 15);
        content_box.set_margin_top(10);
        content_box.set_margin_bottom(15);
        content_box.set_margin_start(15);
        content_box.set_margin_end(15);

        toolbar_view.set_content(Some(&content_box));
        window.set_content(Some(&toolbar_view));

        // ── F11 = fullscreen ───────────────────────────────────────────────────────────────────────────────

        let toggle_fullscreen = gio::SimpleAction::new("toggle-fullscreen", None);
        {
            let window_c = window.clone();
            toggle_fullscreen.connect_activate(move |_, _| {
                window_c.set_fullscreened(!window_c.is_fullscreen());
            });
        }
        window.add_action(&toggle_fullscreen);
        app.set_accels_for_action("win.toggle-fullscreen", &["F11"]);

        // ── Navigation stack ───────────────────────────────────────────────────────────────────────────────

        let stack = Stack::new();
        stack.set_transition_type(StackTransitionType::SlideLeftRight);
        stack.set_transition_duration(300);
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);

        let main_scroll = ScrolledWindow::new();
        main_scroll.set_vexpand(true);
        main_scroll.set_hexpand(true);
        main_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
        main_scroll.set_child(Some(&stack));

        // Pages grab focus when mapped.
        // Scroll-to-focus would yank the view toward them on every re-map (e.g. returning from the MIDI monitor).
        if let Some(viewport) = stack.parent().and_downcast::<gtk4::Viewport>() {
            viewport.set_scroll_to_focus(false);
        }

        // The monitor lives alongside the nav stack, not inside it.
        // A dedicated outer Stack with the same `SlideLeftRight` transition makes monitor open/close feel like page navigation.
        let monitor_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        monitor_container.set_vexpand(true);

        let view_stack = Stack::new();
        view_stack.set_transition_type(StackTransitionType::SlideLeftRight);
        view_stack.set_transition_duration(300);
        view_stack.set_vexpand(true);
        view_stack.set_hexpand(true);
        view_stack.set_hhomogeneous(false);
        view_stack.set_vhomogeneous(false);
        view_stack.add_named(&main_scroll, Some("content"));
        view_stack.add_named(&monitor_container, Some("monitor"));
        content_box.append(&view_stack);

        // ── Non-GTK mutable state ──────────────────────────────────────────────────────────────────────────

        let state = Rc::new(RefCell::new(AppState {
            active_config: None,
            device_shorter: String::new(),
        }));

        // ── Bridge monitor server ──────────────────────────────────────────────────────────────────────────

        // Started eagerly so plugins can connect before the user opens the Bridge page.
        //
        // This is purely observational. The plugins route MIDI traffic through the DAW.
        // This stream only feeds the Bridge page's I/O log.
        let bridge_source = start_bridge_server().expect("Bridge server could not start (port 7420 busy?)");
        let bridge_source_rc = Rc::new(RefCell::new(Some(bridge_source)));

        // ── Top bar: MIDI port dropdown + delay spinner ────────────────────────────────────────────────────

        let settings = read_settings();
        let midi_box = gtk4::Box::new(Orientation::Horizontal, 8);

        let midi_ports = get_output_names();

        // `MANUAL_PORT` ("DummyDevice") is always the first entry.
        // No hardware: virtual "C7" ports for hand-wiring on Linux/macOS (qjackctl), plus a dummy device for hardware-free development.
        // Its presence also means the list is never empty.
        let mut port_names: Vec<String> = vec![MANUAL_PORT.to_string()];
        port_names.extend(midi_ports.iter().cloned());

        let port_combo = CustomDropdown::new(Some(0));
        port_combo.set_tooltip_text(Some("MIDI output device"));
        let last_port = as_string_or(settings.get("last_midi_port"), "").to_string();
        // Default to `MANUAL_PORT` (DummyDevice).
        let mut active_idx = 0;
        // Populates the MIDI port dropdown with available hardware ports.
        for (i, name) in port_names.iter().enumerate() {
            port_combo.append_text(name);
            if *name == last_port {
                active_idx = i as i32;
            }
        }
        port_combo.set_active(Some(active_idx as u32));

        if !last_port.is_empty() {
            let _ = init_persistent_port(&last_port);
        }
        midi_box.append(&*port_combo);

        let last_delay = settings.get("last_delay_ms").and_then(serde_json::Value::as_f64).unwrap_or(2.0);

        // Set MIDI rate bounds to `1ms` - `127ms`.
        //
        // Technically, a TM-1 can transfer slightly faster than `1ms`.
        // But since most users aren't using a TM-1, I don't want them to be confused when `0ms` breaks their system.
        let delay_spin = NumberSpinner::new(last_delay, 1.0, 127.0, 1.0, "AUTO", 0);
        delay_spin.set_tooltip_text("Inter-packet delay (ms)");
        delay_spin.set_disabled_tooltip("Closed-loop handshaking automatically manages pacing");

        // Seed the actor's pacing gap from the saved delay so sends are paced before the user ever touches the spinner.
        set_midi_delay_ms(delay_spin.value() as u8);
        midi_box.append(delay_spin.widget());

        midi_box.set_visible(false);
        header_bar.pack_start(&midi_box);

        // ── Persistent-settings closure (Rc so it can be shared by two connect_ calls) ─────────────────────

        let save_settings: Rc<dyn Fn()> = Rc::new({
            let port_combo_c = port_combo.clone();
            let delay_spin_c = delay_spin.clone();
            move || {
                let mut settings = read_settings();

                // While the delay spinner is insensitive (op in progress), keep the last saved value, not the zeroed-out spinner value.
                let delay_val = if delay_spin_c.root.is_sensitive() {
                    delay_spin_c.value()
                } else {
                    settings.get("last_delay_ms").and_then(serde_json::Value::as_f64).unwrap_or(2.0)
                };

                settings.insert(
                    "last_midi_port".to_string(),
                    serde_json::Value::String(port_combo_c.active_text().map(|text| text.to_string()).unwrap_or_default()),
                );
                settings.insert("last_delay_ms".to_string(), serde_json::Value::from(delay_val));
                write_settings(&settings);
            }
        });

        {
            let save_settings_c = Rc::clone(&save_settings);
            let port_combo_c = port_combo.clone();
            port_combo.connect_changed(move |_| {
                save_settings_c();
                if let Some(port) = port_combo_c.active_text() {
                    let name = port.to_string();
                    if !name.is_empty() {
                        let _ = init_persistent_port(&name);
                    }
                }
            });
        }
        {
            let save_settings_c = Rc::clone(&save_settings);
            let delay_spin_c = delay_spin.clone();
            delay_spin.connect_value_changed(move || {
                save_settings_c();
                // While the spinner is locked to AUTO (SDS screens disable it) send gap 0 so the ACK/NAK handshake owns pacing.
                // Otherwise the actor paces every send at the spinner value.
                let gap = if delay_spin_c.root.is_sensitive() {
                    delay_spin_c.value() as u8
                } else {
                    0
                };
                set_midi_delay_ms(gap);
            });
        }

        // ── `go_to_device_selector` (Rc so it can be captured by `on_device_selected`) ─────────────────────

        let go_to_device_selector: Rc<dyn Fn()> = Rc::new({
            let stack_c = stack.clone();
            let view_stack_c = view_stack.clone();
            let midi_box_c = midi_box.clone();
            let monitor_btn_c = monitor_btn.clone();
            let info_btn_c = info_btn.clone();
            let utilities_btn_c = utilities_btn.clone();
            let settings_btn_c = settings_btn.clone();
            move || {
                view_stack_c.set_visible_child_name("content");
                midi_box_c.set_visible(false);
                monitor_btn_c.set_visible(false);
                info_btn_c.set_visible(true);
                utilities_btn_c.set_visible(true);
                settings_btn_c.set_visible(true);
                stack_c.set_visible_child_name("device_selector");
            }
        });

        // ── Header bar modal buttons ───────────────────────────────────────────────────────────────────────

        {
            let window_c = window.clone();
            info_btn.connect_clicked(move |_| {
                AboutModal::show(&window_c);
            });
        }

        {
            let window_c = window.clone();
            utilities_btn.connect_clicked(move |_| {
                C7UtilitiesModal::new(&window_c).present();
            });
        }

        {
            let window_c = window.clone();
            settings_btn.connect_clicked(move |_| {
                SettingsModal::new(&window_c).present();
            });
        }

        // ── MIDI Monitor button ────────────────────────────────────────────────────────────────────────────

        // Bound once here. Slides to the monitor view using the same `SlideLeftRight` transition as regular page navigation.
        // The monitor button is only visible after device selection, so the monitor widget is always ready by the time this fires.
        {
            let view_stack_c = view_stack.clone();
            monitor_btn.connect_clicked(move |btn| {
                view_stack_c.set_visible_child_name("monitor");
                btn.set_visible(false);
            });
        }

        // ── on_device_selected closure ─────────────────────────────────────────────────────────────────────

        let on_device_selected = {
            let state_c = Rc::clone(&state);
            let stack_c = stack.clone();
            let go_to_device_selector_c = Rc::clone(&go_to_device_selector);
            let bridge_source_rc_c = Rc::clone(&bridge_source_rc);
            move |config_path: String| {
                let dc = Arc::new(DeviceConfig::find_by_path_or_die(&config_path));
                {
                    let mut state_mut = state_c.borrow_mut();
                    state_mut.active_config = Some(config_path.clone());
                    state_mut.device_shorter.clone_from(&dc.device_shorter);
                }

                midi_box.set_visible(true);
                monitor_btn.set_visible(true);
                info_btn.set_visible(false);
                utilities_btn.set_visible(false);
                settings_btn.set_visible(false);

                // Clear the navigation stack of dynamic pages before rebuilding the view.
                //
                // `bridge_connect` is intentionally excluded.
                // It owns the app-lifetime bridge event stream (takeable only once) and has no device-specific content.
                let dynamic_pages = [
                    "main_menu",
                    "upload_sample",
                    "receive_sample",
                    "upload_digipro",
                    "receive_digipro",
                    "kit_editor",
                    "midi_tricks",
                    "sysex_tricks",
                    "librarian",
                    "firmware_tools",
                ];
                for page_name in dynamic_pages {
                    if let Some(page) = stack_c.child_by_name(page_name) {
                        stack_c.remove(&page);
                    }
                }

                // If the monitor was open when the device switched, slide back to the nav view so the new device's main menu is visible.
                view_stack.set_visible_child_name("content");

                // ── Shared callback factories ──────────────────────────────────────────────────────────────

                // Each factory returns a fresh closure wrapping the same underlying Rc.
                let go_to_menu_rc: Rc<dyn Fn()> = Rc::new({
                    let stack_c2 = stack_c.clone();
                    move || stack_c2.set_visible_child_name("main_menu")
                });
                let go_to_menu_factory = move || {
                    let go_to_menu_rc = Rc::clone(&go_to_menu_rc);
                    move || go_to_menu_rc()
                };

                let get_selected_midi_rc: Rc<dyn Fn() -> Option<String>> = Rc::new({
                    let port_combo_c = port_combo.clone();
                    move || -> Option<String> { port_combo_c.active_text().map(|text| text.to_string()) }
                });
                let get_selected_midi_factory = move || {
                    let get_selected_midi_rc = Rc::clone(&get_selected_midi_rc);
                    move || get_selected_midi_rc()
                };
                let get_midi_for_monitor = get_selected_midi_factory();

                // Both tricks screens are built up front, because their menu cards depend on whether any of their own cards passed a gate.
                let midi_tricks = MidiTricksScreen::new(go_to_menu_factory(), get_selected_midi_factory(), &config_path);
                let has_midi_tricks = midi_tricks.has_cards;

                let sysex_tricks = SysexTricksScreen::new(go_to_menu_factory(), get_selected_midi_factory(), &config_path);
                let has_sysex_tricks = sysex_tricks.has_cards;

                let build_screen: Rc<dyn Fn(&str)> = Rc::new({
                    let stack_c2 = stack_c.clone();
                    let delay_spin_c = delay_spin.clone();
                    let bridge_source_rc_c2 = Rc::clone(&bridge_source_rc_c);
                    move |target: &str| {
                        if stack_c2.child_by_name(target).is_some() {
                            return;
                        }

                        match target {
                            "upload_sample" => {
                                let screen =
                                    UploadSampleScreen::new(go_to_menu_factory(), get_selected_midi_factory(), &delay_spin_c, &config_path);
                                stack_c2.add_named(screen.widget(), Some("upload_sample"));
                            }
                            "receive_sample" => {
                                let screen = ReceiveSampleScreen::new(
                                    go_to_menu_factory(),
                                    get_selected_midi_factory(),
                                    &delay_spin_c,
                                    &config_path,
                                );
                                stack_c2.add_named(screen.widget(), Some("receive_sample"));
                            }
                            "upload_digipro" => {
                                let screen = UploadDigiproScreen::new(
                                    go_to_menu_factory(),
                                    get_selected_midi_factory(),
                                    &delay_spin_c,
                                    &config_path,
                                );
                                stack_c2.add_named(screen.widget(), Some("upload_digipro"));
                            }
                            "receive_digipro" => {
                                let screen = ReceiveDigiproScreen::new(
                                    go_to_menu_factory(),
                                    get_selected_midi_factory(),
                                    &delay_spin_c,
                                    &config_path,
                                );
                                stack_c2.add_named(screen.widget(), Some("receive_digipro"));
                            }
                            "kit_editor" => {
                                let screen = KitEditorScreen::new(go_to_menu_factory(), get_selected_midi_factory(), &config_path);
                                stack_c2.add_named(screen.widget(), Some("kit_editor"));
                            }
                            "librarian" => {
                                let screen = LibrarianScreen::new(go_to_menu_factory(), get_selected_midi_factory(), &config_path);
                                stack_c2.add_named(screen.widget(), Some("librarian"));
                            }
                            "firmware_tools" => {
                                let screen = FirmwareToolsScreen::new(go_to_menu_factory(), get_selected_midi_factory(), &config_path);
                                stack_c2.add_named(screen.widget(), Some("firmware_tools"));
                            }
                            "bridge_connect" => {
                                let src = bridge_source_rc_c2.borrow_mut().take().unwrap();
                                let screen = BridgeScreen::new(go_to_menu_factory(), src);
                                stack_c2.add_named(screen.widget(), Some("bridge_connect"));
                            }
                            _ => {}
                        }
                    }
                });

                // ── Build and register the main menu ───────────────────────────────────────────────────────

                let menu_widget = build_main_menu(
                    &dc,
                    &stack_c,
                    {
                        let go_to_device_selector_c2 = Rc::clone(&go_to_device_selector_c);
                        move || go_to_device_selector_c2()
                    },
                    has_midi_tricks,
                    has_sysex_tricks,
                    {
                        let build_screen = Rc::clone(&build_screen);
                        move |target_name| build_screen(target_name)
                    },
                );
                stack_c.add_named(&menu_widget, Some("main_menu"));
                stack_c.set_visible_child_name("main_menu");

                // GtkStack takes the slide direction from child order, so these register after "main_menu".
                if has_midi_tricks {
                    stack_c.add_named(midi_tricks.widget(), Some("midi_tricks"));
                }
                if has_sysex_tricks {
                    stack_c.add_named(sysex_tricks.widget(), Some("sysex_tricks"));
                }

                // ── Build the MIDI monitor for this device session ─────────────────────────────────────────

                // Clear any widget from a previous device session.
                while let Some(child) = monitor_container.first_child() {
                    monitor_container.remove(&child);
                }
                let monitor = MidiMonitorScreen::new(
                    {
                        let view_stack_c = view_stack.clone();
                        let monitor_btn_c = monitor_btn.clone();
                        let main_scroll_c = main_scroll.clone();
                        move || {
                            view_stack_c.set_visible_child_name("content");
                            monitor_btn_c.set_visible(true);
                            // Snap back to the top so returning from the monitor looks like a fresh page entry.
                            main_scroll_c.vadjustment().set_value(0.0);
                        }
                    },
                    get_midi_for_monitor,
                );
                monitor_container.append(monitor.widget());
            }
        };

        // ── Initial page: device selector ──────────────────────────────────────────────────────────────────

        let device_selected = DeviceSelectorScreen::new(on_device_selected);
        stack.add_named(device_selected.widget(), Some("device_selector"));
        stack.set_visible_child_name("device_selector");

        SataC7App { window }
    }

    /// Presents the application window to the user.
    pub(crate) fn present(&self) {
        self.window.present();
    }
}

// ── Module-level helpers ───────────────────────────────────────────────────────────────────────────────────

/// Builds the primary feature navigation menu for the selected device.
///
/// Returns the top-level box ready to be added to the stack as "`main_menu`".
///
/// `has_midi_tricks` and `has_sysex_tricks` come from the pre-built screens, since neither card can be gated on a single device JSON key.
fn build_main_menu(
    dc: &DeviceConfig,
    stack: &Stack,
    go_to_device_selector: impl Fn() + 'static,
    has_midi_tricks: bool,
    has_sysex_tricks: bool,
    build_screen: impl Fn(&str) + Clone + 'static,
) -> gtk4::Box {
    let menu_box = gtk4::Box::new(Orientation::Vertical, 16);
    menu_box.set_margin_top(32);
    menu_box.set_margin_bottom(32);
    menu_box.set_margin_start(32);
    menu_box.set_margin_end(32);
    menu_box.set_valign(Align::Start);
    menu_box.set_halign(Align::Center);

    let title_row = gtk4::CenterBox::new();
    title_row.set_valign(Align::Center);

    let switch_btn = Button::from_icon_name("go-previous-symbolic");
    switch_btn.set_tooltip_text(Some("Switch Device"));
    switch_btn.set_valign(Align::Center);
    switch_btn.connect_clicked(move |_| go_to_device_selector());
    title_row.set_start_widget(Some(&switch_btn));

    let title = Label::new(Some(&dc.device_short));
    title.add_css_class("title-1");
    title_row.set_center_widget(Some(&title));

    menu_box.append(&title_row);

    let flowbox = FlowBox::new();
    flowbox.set_valign(Align::Start);
    flowbox.set_max_children_per_line(3);
    flowbox.set_selection_mode(SelectionMode::None);
    flowbox.set_row_spacing(16);
    flowbox.set_column_spacing(16);

    // Builds the card buttons for the main menu flowbox.
    // Most cards are gated on device capabilities.

    if dc.has_gate("sysex_api.sample") {
        flowbox.insert(
            &create_card_button(
                "audio-x-generic",
                "Upload Sample",
                "Send audio to slots",
                "upload_sample",
                stack,
                build_screen.clone(),
            ),
            -1,
        );
    }
    if dc.has_gate("sysex_api.sample.can_dump") {
        flowbox.insert(
            &create_card_button(
                "audio-x-generic",
                "Receive Sample",
                "Dump audio from slots",
                "receive_sample",
                stack,
                build_screen.clone(),
            ),
            -1,
        );
    }
    if dc.has_gate("sysex_api.digipro") {
        flowbox.insert(
            &create_card_button(
                "audio-x-generic",
                "Upload DigiPro",
                "Send waveforms to slots",
                "upload_digipro",
                stack,
                build_screen.clone(),
            ),
            -1,
        );
        flowbox.insert(
            &create_card_button(
                "audio-x-generic",
                "Receive DigiPro",
                "Dump waveforms from slots",
                "receive_digipro",
                stack,
                build_screen.clone(),
            ),
            -1,
        );
    }
    if dc.has_gate("sysex_api.kit.workspace_slot") {
        flowbox.insert(
            &create_card_button(
                "audio-input-microphone-symbolic",
                "Kit Editor",
                "Edit sounds live",
                "kit_editor",
                stack,
                build_screen.clone(),
            ),
            -1,
        );
    }
    if has_midi_tricks {
        flowbox.insert(
            &create_card_button(
                "input-dialpad-symbolic",
                "MIDI Tricks",
                "Control tracks live",
                "midi_tricks",
                stack,
                build_screen.clone(),
            ),
            -1,
        );
    }
    if has_sysex_tricks {
        flowbox.insert(
            &create_card_button(
                "emblem-system-symbolic",
                "SysEx Tricks",
                "Change global settings",
                "sysex_tricks",
                stack,
                build_screen.clone(),
            ),
            -1,
        );
    }
    if dc.has_gate("sysex_api.librarian") {
        flowbox.insert(
            &create_card_button(
                "drive-harddisk-symbolic",
                "Librarian",
                "Back up device data",
                "librarian",
                stack,
                build_screen.clone(),
            ),
            -1,
        );
    }
    flowbox.insert(
        &create_card_button(
            "software-update-available-symbolic",
            "Firmware Tools",
            "Send OS update files",
            "firmware_tools",
            stack,
            build_screen.clone(),
        ),
        -1,
    );
    if dc.has_gate("pages.synth") {
        flowbox.insert(
            &create_card_button(
                "network-transmit-receive-symbolic",
                "C7 Bridge",
                "Control from your DAW",
                "bridge_connect",
                stack,
                build_screen,
            ),
            -1,
        );
    }

    menu_box.append(&flowbox);
    menu_box
}

/// Helper to construct a standardized UI card button for the main menu.
fn create_card_button(
    icon_name: &str,
    title_text: &str,
    description_text: &str,
    target_name: &str,
    stack: &Stack,
    build_screen: impl Fn(&str) + 'static,
) -> Button {
    let btn = Button::new();
    btn.set_size_request(260, 160);
    btn.add_css_class("card");

    let vbox = gtk4::Box::new(Orientation::Vertical, 12);
    vbox.set_halign(Align::Center);
    vbox.set_valign(Align::Center);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let icon = Image::from_icon_name(icon_name);
    icon.set_pixel_size(48);
    vbox.append(&icon);

    let text_box = gtk4::Box::new(Orientation::Vertical, 4);
    text_box.set_halign(Align::Center);

    let title = Label::new(Some(title_text));
    title.add_css_class("title-3");
    title.set_wrap(true);
    title.set_justify(Justification::Center);
    text_box.append(&title);

    let description = Label::new(Some(description_text));
    description.add_css_class("dim-label");
    description.set_wrap(true);
    description.set_max_width_chars(25);
    description.set_justify(Justification::Center);
    text_box.append(&description);

    vbox.append(&text_box);
    btn.set_child(Some(&vbox));

    let target = target_name.to_string();
    let stack_c = stack.clone();
    btn.connect_clicked(move |_| {
        build_screen(&target);
        stack_c.set_visible_child_name(&target);
    });
    btn
}
