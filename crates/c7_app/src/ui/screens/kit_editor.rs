//! Edit a kit's parameters on the device.
//!
//! Every parameter page is built from the device JSON, so knobs, combos, and shape widgets are per-device rather than hardcoded.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::Duration;

use gio::Cancellable;
use gtk4::{
    self, Align, Box as GtkBox, Button, CenterBox, CssProvider, EventControllerKey, FlowBox, FlowBoxChild, Label, MenuButton, Orientation,
    Popover, PositionType, Separator, ToggleButton, gdk, gio, glib,
};
use libadwaita::prelude::*;

use crate::ui::base_module::{BaseModule, make_file_dialog};
use crate::ui::file_checks::{does_file_type_match, verify_device_match};
use crate::ui::mixins::midi_listener::MidiListenerMixin;
use crate::ui::modals::sound_browser::SoundBrowserModal;
use crate::ui::modals::sound_export::{export_kit, export_sound};
use crate::ui::widgets::{
    CustomDropdown, JOYSTICK_BASE_RADIUS, Joystick, NumberSpinner, ParameterCombo, ParameterKnob, ParameterShape, PianoKeyboard,
};
use c7_core::c7_file_interfacing::{
    extract_master_fx, extract_sound_from_kit, find_c7_item, kit_blob_offset, kit_blob_offset_or_die, read_c7_or_sysex_file,
};
use c7_core::device_config::DeviceConfig;
use c7_core::dsp_utils::{note_name, semitone_to_pitch};
use c7_core::kit::{
    FetchResult, HwConfig, KitManager, MasterFxParam, MiscSyncRequest, apply_track_via_cc, extract_cc_meta, fetch_kit_for_display,
    machine_list, master_fx_list, resolve_auto_track, write_kit_to_workspace,
};
use c7_core::midi::{
    find_input_port, get_midi_delay_ms, is_valid_port, run_midi_output, run_midi_session, send_midi, send_midi_cc, send_sysex,
};
use c7_core::sysex::{
    ELEKTRON_PAYLOAD_START, ELEKTRON_TYPE_BYTE, build_elektron_sysex, build_machine_assignment_payload, extract_sysex_name,
    request_status_param,
};
use c7_core::utils::{
    JsonPath, as_array_or, as_array_or_die, as_bool_or, as_string_or, as_string_vec_or_empty, as_u64_or, as_u64_or_die, pseudo_rand,
};

/// Shared clipboard for copy/paste of a single track's sound.
///
/// Holds (SysEx blob, source track index).
type SoundClipboard = Rc<RefCell<Option<(Vec<u8>, usize)>>>;

/// Shared clipboard for copy/paste of a whole kit's SysEx blob.
type KitClipboard = Rc<RefCell<Option<Vec<u8>>>>;

/// Clears the in-flight-fetch flag when dropped, so an early return or a panic can't leave a fetch permanently marked as running.
struct FetchGuard(Arc<AtomicBool>);

impl Drop for FetchGuard {
    /// Clears the in-flight-fetch flag when the guard is dropped.
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Wraps a widget in a non-focusable `FlowBoxChild` and appends it to the given `FlowBox`.
fn add_to_flowbox<W: IsA<gtk4::Widget>>(flow_box: &FlowBox, widget: &W) {
    let child = FlowBoxChild::new();
    child.set_focusable(false);
    child.set_child(Some(widget));
    flow_box.insert(&child, -1);
}

static CSS_ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Initializes the CSS provider for the kit editor.
fn ensure_kit_editor_css() {
    CSS_ONCE.get_or_init(|| {
        let provider = CssProvider::new();
        provider.load_from_string(
            ".no-hover-fb flowboxchild:hover, .no-hover-fb flowboxchild:active { \
             background: transparent; box-shadow: none; outline: none; } \
             .dirty-kit-btn { color: @warning_color; background: alpha(@warning_color, 0.15); }",
        );
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(&display, &provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
}

/// Returns a mapping of physical keyboard keys to piano semitone offsets.
fn piano_key_map() -> HashMap<gdk::Key, i32> {
    let mut map = HashMap::new();
    map.insert(gdk::Key::a, -8);
    map.insert(gdk::Key::s, -6);
    map.insert(gdk::Key::d, -4);
    map.insert(gdk::Key::f, -3);
    map.insert(gdk::Key::g, -1);
    map.insert(gdk::Key::h, 1);
    map.insert(gdk::Key::j, 3);
    map.insert(gdk::Key::k, 4);
    map.insert(gdk::Key::l, 6);
    map.insert(gdk::Key::semicolon, 8);
    map.insert(gdk::Key::w, -7);
    map.insert(gdk::Key::e, -5);
    map.insert(gdk::Key::t, -2);
    map.insert(gdk::Key::y, 0);
    map.insert(gdk::Key::u, 2);
    map.insert(gdk::Key::o, 5);
    map.insert(gdk::Key::p, 7);
    map
}

/// Velocity used for notes fired by clicking the on-screen piano.
const PIANO_NOTE_VELOCITY: u8 = 100;

// ── Miscellaneous controls ─────────────────────────────────────────────────────────────────────────────────

/// One rendered Miscellaneous control, bound to the byte offsets its device JSON entry declared.
///
/// Most kinds read/write the extracted track blob in `track_states`.
/// Global kinds address `misc_kit_globals`, and `SharedSpinner` writes its byte into every track's row.
enum MiscControl {
    /// "OFF + tracks" dropdown (255 = OFF).
    ///
    /// With a resolved command byte it live-sends [cmd, track, value]. Without one it goes through the workspace sync.
    TrackCombo {
        blob_offset: usize,
        cmd: Option<u8>,
        combo: CustomDropdown,
    },
    Toggle {
        blob_offset: usize,
        btn: ToggleButton,
    },
    Spinner {
        blob_offset: usize,
        display_offset: i32,
        spin: NumberSpinner,
    },
    SharedSpinner {
        blob_offset: usize,
        display_offset: i32,
        spin: NumberSpinner,
    },
    /// Dropdown over a masked region of one byte. Options carry explicit byte values.
    MaskedCombo {
        blob_offset: usize,
        mask: u8,
        values: Vec<u8>,
        combo: CustomDropdown,
    },
    /// Page/Param/Depth triple of one assign matrix destination slot (depth is a two's-complement int8).
    AssignSlot {
        page_idx: usize,
        param_idx: usize,
        range_idx: usize,
        page_combo: CustomDropdown,
        param_combo: CustomDropdown,
        depth_spin: NumberSpinner,
    },
    GlobalCombo {
        global_idx: usize,
        combo: CustomDropdown,
    },
    GlobalSpinner {
        global_idx: usize,
        display_offset: i32,
        spin: NumberSpinner,
    },
}

/// The track the combo currently selects.
///
/// ALL and AUTO are already resolved to a concrete index by the time this is built.
#[derive(Clone, Copy)]
struct TrackSel {
    is_track_all: bool,
    is_track_auto: bool,
    track_idx: usize,
    state_idx: usize,
}

/// One rendered parameter control, in whichever form the param's JSON definition calls for.
#[derive(Clone)]
enum ParameterWidget {
    Knob(ParameterKnob),
    Combo(ParameterCombo),
    Shape(ParameterShape),
}

impl ParameterWidget {
    /// Sets the parameter value and optionally triggers the callback.
    fn set_value(&self, val: i32, should_trigger_cb: bool) {
        match self {
            Self::Knob(knob) => knob.set_value(val, should_trigger_cb),
            Self::Combo(combo) => combo.set_value(val, should_trigger_cb),
            Self::Shape(shape) => shape.set_value(val, should_trigger_cb),
        }
    }
    /// Returns the maximum value for the parameter.
    fn max_val(&self) -> i32 {
        match self {
            Self::Knob(knob) => knob.max_val(),
            Self::Combo(combo) => combo.max_idx(),
            Self::Shape(shape) => shape.max_idx(),
        }
    }
    /// Returns the default value for the parameter.
    fn default_val(&self) -> i32 {
        match self {
            Self::Knob(knob) => knob.default_val(),
            Self::Combo(combo) => combo.default_val(),
            Self::Shape(shape) => shape.default_val(),
        }
    }
    /// Returns the GTK widget for the parameter.
    fn widget(&self) -> gtk4::Widget {
        match self {
            Self::Knob(knob) => knob.widget().clone().upcast(),
            Self::Combo(combo) => combo.widget().clone().upcast(),
            Self::Shape(shape) => shape.widget().clone().upcast(),
        }
    }
}

// ── Internal state ─────────────────────────────────────────────────────────────────────────────────────────

struct KitEditorState {
    hw: Arc<HwConfig>,
    dc: Arc<DeviceConfig>,
    kit: Rc<KitManager>,
    get_midi_rc: Rc<dyn Fn() -> Option<String>>,
    root: GtkBox,

    /// Weak self-reference used by `schedule_piano_repeat()` to post a future callback without a circular Rc cycle.
    ///
    /// Set immediately after construction.
    state_wk: std::rc::Weak<RefCell<Option<KitEditorState>>>,

    auto_resolved_track: Cell<usize>,
    sound_clipboard: SoundClipboard,
    kit_clipboard: KitClipboard,

    synth_default_names: Vec<(String, String)>,
    machine_param_names: HashMap<u8, Vec<String>>,
    machine_list: Vec<(u8, String)>,
    is_updating_machine_combo: Rc<Cell<bool>>,
    machine_combo_loaded_idx: Cell<i32>,

    /// Maps each `cc_id` to its sequential param index (the order the manager's state rows and the knobs share).
    cc_knob_idx: HashMap<i32, usize>,

    keys_held: HashSet<gdk::Key>,
    is_piano_mouse_held: bool,
    /// Dedupes spacebar auto-repeat so holding space fires one trig per physical press.
    is_space_held: bool,
    piano_last_offset: i32,
    piano_repeat_src: Option<glib::SourceId>,
    is_poly_hw_updating: bool,
    is_poly_multi_saved: bool,

    track_combo: CustomDropdown,
    trigger_btn: Button,
    save_device_btn: Button,
    track_menu_btn: MenuButton,
    machine_apply_btn: Option<Button>,
    machine_combo: Option<CustomDropdown>,
    knobs: Vec<ParameterWidget>,
    master_knobs: Vec<(u8, u8, ParameterKnob)>,
    lfo_widgets: Vec<ParameterWidget>,
    piano: PianoKeyboard,
    joystick: Option<Joystick>,
    poly_hw_btn: Option<ToggleButton>,
    poly_btn: Option<ToggleButton>,
    piano_repeat_btn: ToggleButton,

    /// Miscellaneous section: kit params with no CC/knob path. The section box is desensitized when Track = ALL.
    misc_section: Option<GtkBox>,
    /// The rendered misc controls in JSON declaration order. Offsets and widget kinds come from `misc_pages`.
    misc_controls: Vec<MiscControl>,
    /// Assign matrix destination names per dest page (indexed 0-8).
    assign_dest_names: Vec<Vec<String>>,
    dynamic_machine_assign_idx: Option<usize>,
    lfo_dest_links: Vec<(ParameterCombo, ParameterCombo)>,
    /// Silences the misc widgets' change signals during programmatic repaints, mirroring `is_updating_machine_combo`.
    is_updating_misc: Rc<Cell<bool>>,

    is_kit_fetch_locked: Arc<AtomicBool>,
}

/// Feature screen for live real-time patch and parameter editing.
pub(crate) struct KitEditorScreen {
    pub root: gtk4::Box,
}

impl KitEditorScreen {
    /// Initializes the kit editor screen.
    pub(crate) fn new(
        go_to_menu: impl Fn() + 'static,
        get_selected_midi: impl Fn() -> Option<String> + 'static,
        active_config: &str,
    ) -> Self {
        ensure_kit_editor_css();
        let mut base = BaseModule::new();
        let root = base.root.clone();
        root.set_focusable(true);
        root.connect_map(|root_mapped| {
            root_mapped.grab_focus();
        });

        let dc = Arc::new(DeviceConfig::find_by_path_or_die(active_config));
        let pages = dc.json_get("pages.synth").and_then(|value| value.as_array());

        // The manager owns the device state and the hardware transfer engine. This screen renders from it.
        let kit_manager = KitManager::new(Arc::clone(&dc));
        let hw = Arc::clone(&kit_manager.hw);

        let mut synth_default_names = Vec::new();
        if let Some(pages) = pages
            && let Some(pg0) = pages.first()
            && let Some(params) = pg0.json_get("params").and_then(|value| value.as_array())
        {
            for (i, param) in params.iter().enumerate() {
                synth_default_names.push((
                    as_string_or(param.json_get("name"), &format!("CC {}", i + 1)).to_string(),
                    as_string_or(param.json_get("fullname"), &format!("Control {}", i + 1)).to_string(),
                ));
            }
        }

        let mut machine_param_names: HashMap<u8, Vec<String>> = HashMap::new();
        if let Some(machines) = dc.json_get("sysex_api.machines").and_then(|value| value.as_object()) {
            for family_variants in machines.values() {
                if let Some(variants) = family_variants.as_array() {
                    for variant in variants {
                        if let Some(id) = variant.json_get("id").and_then(serde_json::Value::as_u64) {
                            let param_names = as_string_vec_or_empty(variant.json_get("params"));
                            machine_param_names.insert(id as u8, param_names);
                        }
                    }
                }
            }
        }

        let machine_list = machine_list(&dc);

        // Maps each `cc_id` to its sequential param index (the same order the manager's state rows and the knobs use).
        let mut cc_knob_idx = HashMap::new();
        let mut knob_idx_counter = 0;
        let mut all_pages = Vec::new();
        if let Some(page_list) = pages {
            all_pages.extend(page_list.iter());
        }
        if let Some(lfo_pages) = dc.json_get("pages.lfo").and_then(|value| value.as_array()) {
            all_pages.extend(lfo_pages.iter());
        }
        for page in &all_pages {
            if let Some(params) = page.json_get("params").and_then(|value| value.as_array()) {
                for json_param in params {
                    if let Some(cc_id) = json_param.json_get("cc_id").and_then(serde_json::Value::as_i64) {
                        cc_knob_idx.insert(cc_id as i32, knob_idx_counter);
                    }
                    knob_idx_counter += 1;
                }
            }
        }

        let state_rc: Rc<RefCell<Option<KitEditorState>>> = Rc::new(RefCell::new(None));

        // ── Top Bar ────────────────────────────────────────────────────────────────────────────────────────

        let state_rc_c = Rc::clone(&state_rc);
        let go_back_rc = Rc::new(go_to_menu);
        let go_back_c = Rc::clone(&go_back_rc);
        let header = {
            let state_rc_c2 = Rc::clone(&state_rc_c);
            let go_back_c2 = Rc::clone(&go_back_c);
            base.build_header("Kit Editor", move || {
                if let Some(state_mut) = state_rc_c2.borrow_mut().as_mut() {
                    state_mut.cancel_piano_repeat();
                }
                go_back_c2();
            })
        };

        // UI definition for the refresh button
        let refresh_btn = Button::from_icon_name("view-refresh-symbolic");
        refresh_btn.set_focus_on_click(false);
        refresh_btn.set_tooltip_text(Some("Fetch workspace and sync UI"));
        {
            let state_rc_c = Rc::clone(&state_rc);
            refresh_btn.connect_clicked(move |_| {
                if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                    state_ref.on_refresh_ui_clicked();
                }
            });
        }

        // UI definition for the reload button
        let reload_btn = Button::with_label("Reload Kit");
        reload_btn.set_focus_on_click(false);
        reload_btn.set_tooltip_text(Some("Reload kit from saved slot"));
        reload_btn.add_css_class("destructive-action");
        {
            let state_rc_c = Rc::clone(&state_rc);
            reload_btn.connect_clicked(move |_| {
                if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                    state_ref.on_reload_kit_clicked();
                }
            });
        }

        let save_device_btn = Button::with_label("Save Kit");
        save_device_btn.set_focus_on_click(false);
        save_device_btn.set_tooltip_text(Some("Save the workspace to the device's devices kit"));
        {
            let state_rc_c = Rc::clone(&state_rc);
            save_device_btn.connect_clicked(move |_| {
                if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                    state_ref.on_save_device_clicked();
                }
            });
        }

        header.start.append(&refresh_btn);
        header.start.append(&reload_btn);
        header.start.append(&save_device_btn);

        // UI definition for the track selector dropdown
        let track_box = GtkBox::new(Orientation::Horizontal, 8);
        track_box.append(&Label::new(Some("Track:")));
        let track_combo = CustomDropdown::new(Some(0));
        track_combo.set_focus_on_click(false);
        track_combo.append_text("ALL");
        if hw.has_auto_channel {
            track_combo.append_text("AUTO");
        }

        if let Some(tracks) = dc.json_get("tracks").and_then(|value| value.as_array()) {
            for track in tracks {
                let name = as_string_or(track.json_get("trackname"), "");
                track_combo.append_text(name);
            }
        }
        track_combo.set_active(Some(1));
        {
            let state_rc_c = Rc::clone(&state_rc);
            track_combo.connect_changed(move |_| {
                if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                    state_ref.on_track_changed();
                }
            });
        }
        track_box.append(&*track_combo);

        // UI definition for the trigger button
        let trigger_btn = Button::with_label("Trig");
        trigger_btn.set_focus_on_click(false);
        {
            let state_rc_c = Rc::clone(&state_rc);
            trigger_btn.connect_clicked(move |_| {
                if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                    state_ref.trigger_current_track();
                }
            });
        }
        track_box.append(&trigger_btn);

        // UI definition for the track options popover
        let track_popover = Popover::new();
        track_popover.set_position(PositionType::Bottom);
        let menu_box = GtkBox::new(Orientation::Vertical, 0);
        menu_box.set_margin_top(4);
        menu_box.set_margin_bottom(4);
        menu_box.set_margin_start(4);
        menu_box.set_margin_end(4);

        let mut track_action_btns = Vec::new();
        let actions = [
            ("Copy Sound", "copy_sound"),
            ("Paste Sound", "paste_sound"),
            ("Clear Sound", "clear_sound"),
            ("Export Sound", "save_sound"),
            ("Browse Sounds", "browse_sounds"),
            ("-", ""),
            ("Copy Kit", "copy_kit"),
            ("Paste Kit", "paste_kit"),
            ("Clear Kit", "clear_kit"),
            ("Export Kit", "save_kit"),
            ("Import Kit", "import_kit"),
        ];
        for (label, action) in actions {
            if label == "-" {
                let separator = Separator::new(Orientation::Horizontal);
                separator.set_margin_top(4);
                separator.set_margin_bottom(4);
                menu_box.append(&separator);
                continue;
            }
            let btn = Button::with_label(label);
            btn.add_css_class("flat");
            btn.set_halign(Align::Fill);
            btn.set_hexpand(true);
            let state_rc_c = Rc::clone(&state_rc);
            let track_popover_c = track_popover.clone();
            let action_str = action.to_string();
            btn.connect_clicked(move |_| {
                track_popover_c.popdown();
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.dispatch_menu_action(&action_str);
                }
            });
            menu_box.append(&btn);
            if action.ends_with("_sound") || action == "browse_sounds" {
                track_action_btns.push(btn);
            }
        }
        track_popover.set_child(Some(&menu_box));

        let track_menu_btn = MenuButton::new();
        track_menu_btn.set_popover(Some(&track_popover));
        track_box.append(&track_menu_btn);
        header.end.append(&track_box);

        let piano = PianoKeyboard::new(60);
        let mut _piano_octave = None;
        let mut _pb_knob = None;
        let mut poly_hw_btn = None;
        let mut poly_btn = None;
        let mut _base_note_spin = None;
        let mut _base_note_label = None;
        let joystick = if dc.has_gate("system.joystick") {
            Some(Joystick::new())
        } else {
            None
        };

        let piano_section = GtkBox::new(Orientation::Vertical, 8);
        piano_section.set_margin_top(4);
        piano_section.set_margin_bottom(4);
        piano_section.set_halign(Align::Center);
        let piano_title = Label::new(Some("Piano"));
        piano_title.add_css_class("title-3");
        piano_section.append(&piano_title);

        // UI definition for the Monomachine joystick
        if let Some(ref joystick_ref) = joystick {
            let joystick_row = GtkBox::new(Orientation::Horizontal, 8);
            joystick_row.set_halign(Align::Center);
            joystick_row.append(joystick_ref.widget());
            joystick_row.append(piano.widget());
            piano_section.append(&joystick_row);
            let state_rc_c2 = Rc::clone(&state_rc);
            joystick_ref.connect_moved(move |x, y| {
                if let Ok(guard) = state_rc_c2.try_borrow()
                    && let Some(ref state_ref) = *guard
                {
                    state_ref.on_joystick_moved(x, y);
                }
            });
        } else {
            piano_section.append(piano.widget());
        }

        let control_row = GtkBox::new(Orientation::Horizontal, 16);
        control_row.set_margin_top(4);
        control_row.set_halign(Align::Center);

        if dc.is_device("MnM") {
            let oct_col = GtkBox::new(Orientation::Vertical, 4);
            oct_col.set_valign(Align::Center);
            let octave_label = Label::new(Some("Octave"));
            octave_label.add_css_class("caption");
            oct_col.append(&octave_label);
            let spin = NumberSpinner::new(4.0, 0.0, 9.0, 1.0, "", 0);
            piano.set_base_note((4 + 1) * 12 + 8);
            let piano_c = piano.clone();
            let spin_c = spin.clone();
            spin.connect_value_changed(move || piano_c.set_base_note((spin_c.value() as i32 + 1) * 12 + 8));
            oct_col.append(spin.widget());
            control_row.append(&oct_col);
            _piano_octave = Some(spin);

            let state_rc_c_pb = Rc::clone(&state_rc);
            let pb_knob = ParameterKnob::new(
                "PB",
                "Pitch Bend",
                0u8,
                0,
                move |_, val, _| {
                    if let Some(state_ref) = state_rc_c_pb.borrow().as_ref() {
                        state_ref.on_pitch_bend_change(val);
                    }
                },
                0,
                -63,
                63,
                true,
                None,
            );
            pb_knob.set_spring(0);
            control_row.append(pb_knob.widget());
            _pb_knob = Some(pb_knob);

            let poly_hw_col = GtkBox::new(Orientation::Vertical, 4);
            poly_hw_col.set_valign(Align::Center);
            let poly_label = Label::new(Some("Poly"));
            poly_label.add_css_class("caption");
            poly_hw_col.append(&poly_label);
            let btn = ToggleButton::with_label("Off");
            let state_rc_c_poly = Rc::clone(&state_rc);
            btn.connect_toggled(move |_| {
                if let Some(state_ref) = state_rc_c_poly.borrow().as_ref() {
                    state_ref.on_poly_hw_toggled();
                }
            });
            poly_hw_col.append(&btn);
            control_row.append(&poly_hw_col);
            poly_hw_btn = Some(btn);

            let poly_col = GtkBox::new(Orientation::Vertical, 4);
            poly_col.set_valign(Align::Center);
            let multi_label = Label::new(Some("Multi"));
            multi_label.add_css_class("caption");
            poly_col.append(&multi_label);
            let btn = ToggleButton::with_label("Off");
            btn.connect_toggled(|toggled_btn| toggled_btn.set_label(if toggled_btn.is_active() { "On" } else { "Off" }));
            poly_col.append(&btn);
            control_row.append(&poly_col);
            poly_btn = Some(btn);
        }
        if dc.is_device("MD") {
            let base_col = GtkBox::new(Orientation::Vertical, 4);
            base_col.set_valign(Align::Center);
            let title_label = Label::new(Some("Base Note"));
            title_label.add_css_class("caption");
            base_col.append(&title_label);
            let spinner_row = GtkBox::new(Orientation::Horizontal, 6);
            let spin = NumberSpinner::new(60.0, 0.0, 127.0, 1.0, "", 0);
            let label = Label::new(Some(&note_name(60)));
            label.add_css_class("dim-label");
            let piano_c = piano.clone();
            let note_label_c = label.clone();
            let base_spin_c = spin.clone();
            spin.connect_value_changed(move || {
                let note_val = base_spin_c.value() as i32;
                note_label_c.set_label(&note_name(note_val));
                piano_c.set_base_note(note_val);
            });
            spinner_row.append(spin.widget());
            spinner_row.append(&label);
            base_col.append(&spinner_row);
            control_row.append(&base_col);
            _base_note_spin = Some(spin);
            _base_note_label = Some(label);
        }

        let repeat_col = GtkBox::new(Orientation::Vertical, 4);
        repeat_col.set_valign(Align::Center);
        let repeat_label = Label::new(Some("Repeat"));
        repeat_label.add_css_class("caption");
        repeat_col.append(&repeat_label);
        let piano_repeat_btn = ToggleButton::with_label("Off");
        piano_repeat_btn.set_tooltip_text(Some("Hold a key to retrigger at the delay interval"));
        piano_repeat_btn
            .connect_toggled(|repeat_toggle_btn| repeat_toggle_btn.set_label(if repeat_toggle_btn.is_active() { "On" } else { "Off" }));
        repeat_col.append(&piano_repeat_btn);
        control_row.append(&repeat_col);

        piano_section.append(&control_row);
        root.append(&piano_section);
        root.append(&Separator::new(Orientation::Horizontal));

        let key_controller = EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        {
            let state_rc_c = Rc::clone(&state_rc);
            key_controller.connect_key_pressed(move |_, keyval, _, _| {
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    return state_mut.on_piano_key_pressed(keyval);
                }
                glib::Propagation::Proceed
            });
        }
        {
            let state_rc_c = Rc::clone(&state_rc);
            key_controller.connect_key_released(move |_, keyval, _, _| {
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.on_piano_key_released(keyval);
                }
            });
        }
        root.add_controller(key_controller);

        {
            let state_rc_c = Rc::clone(&state_rc);
            piano.connect_note_pressed(move |offset| {
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.on_piano_press(offset);
                }
            });
        }
        {
            let state_rc_c = Rc::clone(&state_rc);
            piano.connect_note_released(move |offset| {
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.on_piano_release(offset);
                }
            });
        }

        let cc_section = GtkBox::new(Orientation::Vertical, 20);
        cc_section.set_margin_top(4);
        cc_section.set_margin_bottom(4);
        let synth_header = CenterBox::new();
        let synth_label = Label::new(Some("Synthesis"));
        synth_label.add_css_class("title-3");
        synth_header.set_center_widget(Some(&synth_label));

        let is_updating_machine_combo = Rc::new(Cell::new(false));

        let mut machine_apply_btn = None;
        let mut machine_combo = None;
        if !machine_list.is_empty() {
            let apply_box = GtkBox::new(Orientation::Horizontal, 6);
            let apply_btn = Button::with_label("Apply");
            apply_btn.add_css_class("suggested-action");
            apply_btn.set_visible(false);
            apply_btn.set_focus_on_click(false);
            {
                let state_rc_c = Rc::clone(&state_rc);
                apply_btn.connect_clicked(move |_| {
                    if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                        state_ref.apply_machine_to_track();
                    }
                });
            }
            apply_box.append(&apply_btn);
            machine_apply_btn = Some(apply_btn);

            let combo_box = CustomDropdown::new(Some(0));
            combo_box.set_size_request(130, -1);
            // Index 0 is a placeholder shown before a kit is loaded. `machine_list` entries start at index 1.
            combo_box.append_text("--");
            for (_, name) in &machine_list {
                combo_box.append_text(name);
            }
            {
                let state_rc_c = Rc::clone(&state_rc);
                let is_updating_combo_c = Rc::clone(&is_updating_machine_combo);
                combo_box.connect_changed(move |_| {
                    if is_updating_combo_c.get() {
                        return;
                    }
                    if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                        state_ref.on_machine_combo_changed();
                    }
                });
            }
            apply_box.append(&*combo_box);
            machine_combo = Some(combo_box);
            synth_header.set_end_widget(Some(&apply_box));
        }
        cc_section.append(&synth_header);

        let mut knobs = Vec::new();
        let mut master_knobs = Vec::new();
        let mut lfo_widgets = Vec::new();
        let mut lfo_dest_links = Vec::new();

        let build_card = |title: &str| -> (GtkBox, gtk4::Grid) {
            let card = GtkBox::new(Orientation::Vertical, 0);
            card.add_css_class("card");
            let inner = GtkBox::new(Orientation::Vertical, 16);
            inner.set_margin_top(20);
            inner.set_margin_bottom(20);
            inner.set_margin_start(20);
            inner.set_margin_end(20);
            let title_label = Label::new(Some(title));
            title_label.set_halign(Align::Center);
            title_label.add_css_class("title-4");
            inner.append(&title_label);
            let grid = gtk4::Grid::new();
            grid.set_column_spacing(24);
            grid.set_row_spacing(24);
            grid.set_halign(Align::Center);
            inner.append(&grid);
            card.append(&inner);
            (card, grid)
        };

        let param_grid = FlowBox::new();
        param_grid.set_selection_mode(gtk4::SelectionMode::None);
        param_grid.set_row_spacing(24);
        param_grid.set_column_spacing(24);
        param_grid.set_halign(Align::Center);
        param_grid.set_valign(Align::Start);
        param_grid.add_css_class("no-hover-fb");

        let make_knob_cb = |state_rc_c: Rc<RefCell<Option<KitEditorState>>>| {
            move |param_id: u8, val: i32, is_rand: bool| {
                if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                    state_ref.on_knob(param_id, val, is_rand);
                }
            }
        };
        let make_combo_cb = |state_rc_c: Rc<RefCell<Option<KitEditorState>>>| {
            move |param_id: u8, val: i32| {
                if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                    state_ref.on_knob(param_id, val, false);
                }
            }
        };
        let make_lfo_cb = |state_rc_c: Rc<RefCell<Option<KitEditorState>>>| {
            move |param_id: u8, val: i32, is_rand: bool| {
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.on_lfo_knob(param_id, val, is_rand);
                }
            }
        };
        let make_lfo_combo_cb = |state_rc_c: Rc<RefCell<Option<KitEditorState>>>| {
            move |param_id: u8, val: i32| {
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.on_lfo_knob(param_id, val, false);
                }
            }
        };

        if let Some(pages) = pages {
            let mut page_count = 0;
            for page in pages {
                let fullname = as_string_or(page.json_get("fullname"), "");
                let (card, grid) = build_card(fullname);
                let mut page_widget = None;
                let dest_widget: Option<ParameterCombo> = None;

                if let Some(params) = page.json_get("params").and_then(|value| value.as_array()) {
                    for (i, param) in params.iter().enumerate() {
                        let widget_type = as_string_or(param.json_get("widget"), "knob");
                        let cc_id = param.json_get("cc_id").and_then(serde_json::Value::as_i64).unwrap_or(-1) as i32;
                        let name = as_string_or(param.json_get("name"), "");
                        let fullname = as_string_or(param.json_get("fullname"), "");
                        let default_val = param.json_get("default_val").and_then(serde_json::Value::as_i64).unwrap_or(0) as i32;
                        let display_offset = param.json_get("display_offset").and_then(serde_json::Value::as_i64).unwrap_or(0) as i32;
                        let widget: ParameterWidget = if widget_type == "shape" {
                            let options: Option<Vec<String>> = param.json_get("options").and_then(|value| value.as_array()).map(|array| {
                                array
                                    .iter()
                                    .filter_map(|value| value.as_str().map(std::string::ToString::to_string))
                                    .collect()
                            });
                            ParameterWidget::Shape(ParameterShape::new(
                                name,
                                fullname,
                                cc_id as u8,
                                Some(default_val),
                                make_knob_cb(Rc::clone(&state_rc)),
                                options,
                                None,
                                None,
                            ))
                        } else if widget_type == "combo" {
                            let options_owned = as_string_vec_or_empty(param.json_get("options"));
                            let options: Vec<&str> = options_owned.iter().map(std::string::String::as_str).collect();
                            let combo_box = ParameterCombo::new(
                                name,
                                fullname,
                                cc_id as u8,
                                Some(default_val),
                                &options,
                                make_combo_cb(Rc::clone(&state_rc)),
                            );
                            if name == "PAGE" {
                                page_widget = Some(combo_box.clone());
                            }
                            ParameterWidget::Combo(combo_box)
                        } else {
                            ParameterWidget::Knob(ParameterKnob::new(
                                name,
                                fullname,
                                cc_id as u8,
                                default_val,
                                make_knob_cb(Rc::clone(&state_rc)),
                                display_offset,
                                0,
                                127,
                                false,
                                None,
                            ))
                        };

                        // Start uninitiated at 0, since combos/shapes would otherwise show `default_val`.
                        // `Some(default_val)` above only feeds the explicit "Default" action.
                        widget.set_value(0, false);
                        grid.attach(&widget.widget(), (i % 4) as i32, (i / 4) as i32, 1, 1);
                        knobs.push(widget);
                    }
                }
                if let (Some(page_widget), Some(dest_widget)) = (page_widget, dest_widget) {
                    lfo_dest_links.push((page_widget, dest_widget));
                }
                if grid.first_child().is_some() {
                    add_to_flowbox(&param_grid, &card);
                    page_count += 1;
                }
            }
            param_grid.set_max_children_per_line(page_count.clamp(1, 4) as u32);
        }
        cc_section.append(&param_grid);

        let create_action_row = |state_rc_c: Rc<RefCell<Option<KitEditorState>>>, target: &'static str, suffix: &str| -> FlowBox {
            let flow_box = FlowBox::new();
            flow_box.set_halign(Align::Center);
            flow_box.set_selection_mode(gtk4::SelectionMode::None);
            flow_box.set_column_spacing(12);
            flow_box.set_row_spacing(12);
            flow_box.add_css_class("no-hover-fb");
            let linked_box = GtkBox::new(Orientation::Horizontal, 0);
            linked_box.add_css_class("linked");
            for (action_label, action) in [("Minimize", "minimize"), ("Middle", "middle"), ("Maximize", "maximize")] {
                let action_btn = Button::with_label(&format!("{action_label} {suffix}"));
                let state_rc_c2 = Rc::clone(&state_rc_c);
                action_btn.connect_clicked(move |_| {
                    if let Some(state_mut) = state_rc_c2.borrow_mut().as_mut() {
                        state_mut.apply_mass_action(action, target);
                    }
                });
                linked_box.append(&action_btn);
            }
            add_to_flowbox(&flow_box, &linked_box);
            for (action_label, action) in [("Default", "default"), ("Randomize", "rand")] {
                let action_btn = Button::with_label(&format!("{action_label} {suffix}"));
                let state_rc_c2 = Rc::clone(&state_rc_c);
                action_btn.connect_clicked(move |_| {
                    if let Some(state_mut) = state_rc_c2.borrow_mut().as_mut() {
                        state_mut.apply_mass_action(action, target);
                    }
                });
                add_to_flowbox(&flow_box, &action_btn);
            }
            flow_box
        };

        cc_section.append(&create_action_row(Rc::clone(&state_rc), "ccs", "CCs"));
        root.append(&cc_section);

        let lfo_pages = dc.json_get("pages.lfo").and_then(|value| value.as_array());
        if let Some(lfo_pages) = lfo_pages
            && !lfo_pages.is_empty()
        {
            root.append(&Separator::new(Orientation::Horizontal));
            let lfo_cc_section = GtkBox::new(Orientation::Vertical, 20);
            lfo_cc_section.set_margin_top(4);
            lfo_cc_section.set_margin_bottom(4);
            let mod_label = Label::new(Some("Modulation"));
            mod_label.add_css_class("title-3");
            lfo_cc_section.append(&mod_label);
            let lfo_cc_grid = FlowBox::new();
            lfo_cc_grid.set_max_children_per_line(lfo_pages.len().clamp(1, 3) as u32);
            lfo_cc_grid.set_selection_mode(gtk4::SelectionMode::None);
            lfo_cc_grid.set_row_spacing(24);
            lfo_cc_grid.set_column_spacing(24);
            lfo_cc_grid.set_halign(Align::Center);
            lfo_cc_grid.set_valign(Align::Start);
            lfo_cc_grid.add_css_class("no-hover-fb");
            for page in lfo_pages {
                let fullname = as_string_or(page.json_get("fullname"), "");
                let (card, grid) = build_card(fullname);
                let mut page_widget = None;
                let dest_widget: Option<ParameterCombo> = None;

                if let Some(params) = page.json_get("params").and_then(|value| value.as_array()) {
                    for (i, param) in params.iter().enumerate() {
                        let widget_type = as_string_or(param.json_get("widget"), "knob");
                        let cc_id = param.json_get("cc_id").and_then(serde_json::Value::as_i64).unwrap_or(-1) as i32;
                        let name = as_string_or(param.json_get("name"), "");
                        let fullname = as_string_or(param.json_get("fullname"), "");
                        let default_val = param.json_get("default_val").and_then(serde_json::Value::as_i64).unwrap_or(0) as i32;
                        let display_offset = param.json_get("display_offset").and_then(serde_json::Value::as_i64).unwrap_or(0) as i32;

                        let widget = if widget_type == "shape" {
                            let options: Option<Vec<String>> = param
                                .json_get("options")
                                .and_then(|options| options.as_array())
                                .map(|array| {
                                    array
                                        .iter()
                                        .filter_map(|value| value.as_str().map(std::string::ToString::to_string))
                                        .collect()
                                })
                                // Fall back to the device's LFO shape set so waveform order and labels match the hardware.
                                // MnM orders shapes as TRI, ITRI, SAW, ISAW... The built-in index table follows MD order instead.
                                .or_else(|| {
                                    dc.json_get("lfo_shapes").and_then(|value| value.as_array()).map(|array| {
                                        array
                                            .iter()
                                            .filter_map(|shape| {
                                                shape
                                                    .json_get("name")
                                                    .and_then(|name_val| name_val.as_str())
                                                    .map(std::string::ToString::to_string)
                                            })
                                            .collect()
                                    })
                                });
                            ParameterWidget::Shape(ParameterShape::new(
                                name,
                                fullname,
                                cc_id as u8,
                                Some(default_val),
                                make_knob_cb(Rc::clone(&state_rc)),
                                options,
                                None,
                                None,
                            ))
                        } else if widget_type == "combo" {
                            let options_owned = as_string_vec_or_empty(param.json_get("options"));
                            let options: Vec<&str> = options_owned.iter().map(std::string::String::as_str).collect();
                            let combo_box = ParameterCombo::new(
                                name,
                                fullname,
                                cc_id as u8,
                                Some(default_val),
                                &options,
                                make_combo_cb(Rc::clone(&state_rc)),
                            );
                            if name == "PAGE" {
                                page_widget = Some(combo_box.clone());
                            }
                            ParameterWidget::Combo(combo_box)
                        } else {
                            ParameterWidget::Knob(ParameterKnob::new(
                                name,
                                fullname,
                                cc_id as u8,
                                default_val,
                                make_knob_cb(Rc::clone(&state_rc)),
                                display_offset,
                                0,
                                127,
                                false,
                                None,
                            ))
                        };

                        // Start uninitiated at 0, since the real values only arrive with the workspace kit pull.
                        widget.set_value(0, false);
                        grid.attach(&widget.widget(), (i % 4) as i32, (i / 4) as i32, 1, 1);
                        knobs.push(widget);
                    }
                }
                if let (Some(page_widget), Some(dest_widget)) = (page_widget, dest_widget) {
                    lfo_dest_links.push((page_widget, dest_widget));
                }
                add_to_flowbox(&lfo_cc_grid, &card);
            }

            lfo_cc_section.append(&lfo_cc_grid);
            lfo_cc_section.append(&create_action_row(Rc::clone(&state_rc), "mod_ccs", "CCs"));
            root.append(&lfo_cc_section);
        }

        let lfo_grid = FlowBox::new();
        lfo_grid.set_max_children_per_line(1);
        lfo_grid.set_selection_mode(gtk4::SelectionMode::None);
        lfo_grid.set_row_spacing(24);
        lfo_grid.set_column_spacing(24);
        lfo_grid.set_halign(Align::Center);
        lfo_grid.set_valign(Align::Start);
        lfo_grid.add_css_class("no-hover-fb");

        // Only the Machinedrum has a per-track LFO. Every other device omits `track_lfo` entirely.
        if dc.has_gate("track_lfo") {
            root.append(&Separator::new(Orientation::Horizontal));
            let lfo_section = GtkBox::new(Orientation::Vertical, 20);
            lfo_section.set_margin_top(4);
            lfo_section.set_margin_bottom(4);
            let track_lfo_label = Label::new(Some("Track LFO Configuration"));
            track_lfo_label.add_css_class("title-3");
            lfo_section.append(&track_lfo_label);
            lfo_section.append(&lfo_grid);
            let (card, grid) = build_card("Track LFO");
            let lfo_shapes: Vec<String> = dc
                .json_get("track_lfo.lfo_shapes")
                .and_then(|value| value.as_array())
                .map(|array| {
                    array
                        .iter()
                        .filter_map(|shape| {
                            shape
                                .json_get("name")
                                .and_then(|name_val| name_val.as_str())
                                .map(std::string::ToString::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some(params) = dc.json_get("track_lfo.params").and_then(|value| value.as_array()) {
                for (i, param) in params.iter().enumerate() {
                    let widget_type = as_string_or(param.json_get("widget"), "knob");
                    let default_val = param.json_get("default_val").and_then(serde_json::Value::as_i64).unwrap_or(0) as i32;
                    let param_id = param.json_get("index").and_then(serde_json::Value::as_i64).unwrap().max(0) as u8;
                    let name = as_string_or(param.json_get("name"), "");
                    let fullname = as_string_or(param.json_get("fullname"), "");
                    let mut options: Option<Vec<String>> = param.json_get("options").and_then(|options| options.as_array()).map(|array| {
                        array
                            .iter()
                            .filter_map(|value| value.as_str().map(std::string::ToString::to_string))
                            .collect()
                    });
                    if name == "TRACK" {
                        options = Some(
                            dc.json_get("tracks")
                                .and_then(|value| value.as_array())
                                .map(|array| {
                                    array
                                        .iter()
                                        .filter_map(|track| {
                                            track
                                                .json_get("trackname")
                                                .and_then(|name_val| name_val.as_str())
                                                .map(std::string::ToString::to_string)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                        );
                    }
                    if name == "PARAM" {
                        let mut param_names = Vec::new();
                        if let Some(pages) = pages {
                            for page in pages {
                                if let Some(params) = page.json_get("params").and_then(|value| value.as_array()) {
                                    param_names.extend(params.iter().filter_map(|param| {
                                        param
                                            .json_get("name")
                                            .and_then(|name_val| name_val.as_str())
                                            .map(std::string::ToString::to_string)
                                    }));
                                }
                            }
                        }
                        options = Some(param_names);
                    }

                    let widget = if widget_type == "shape" {
                        ParameterWidget::Shape(ParameterShape::new(
                            name,
                            fullname,
                            param_id,
                            Some(default_val),
                            make_lfo_cb(Rc::clone(&state_rc)),
                            options.or(Some(lfo_shapes.clone())),
                            None,
                            Some("LFO_"),
                        ))
                    } else if widget_type == "mix_shape" {
                        let state_rc_c = Rc::clone(&state_rc);
                        let get_shapes: Box<dyn Fn() -> (usize, usize)> =
                            Box::new(move || state_rc_c.borrow().as_ref().map_or((0, 0), KitEditorState::get_lfo_shapes));
                        // Pass the device shape names so the morph can resolve SHP1/SHP2 by name.
                        // (`get_shapes` returns indices into this list.)
                        // The same shape overrides the SHP widgets use so the morph matches them (e.g. RMP).
                        ParameterWidget::Shape(ParameterShape::new(
                            name,
                            fullname,
                            param_id,
                            Some(default_val),
                            make_lfo_cb(Rc::clone(&state_rc)),
                            Some(lfo_shapes.clone()),
                            Some(get_shapes),
                            Some("LFO_"),
                        ))
                    } else if let Some(options_owned) = options {
                        let options: Vec<&str> = options_owned.iter().map(std::string::String::as_str).collect();
                        ParameterWidget::Combo(ParameterCombo::new(
                            name,
                            fullname,
                            param_id,
                            Some(default_val),
                            &options,
                            make_lfo_combo_cb(Rc::clone(&state_rc)),
                        ))
                    } else {
                        ParameterWidget::Knob(ParameterKnob::new(
                            name,
                            fullname,
                            param_id,
                            default_val,
                            make_lfo_cb(Rc::clone(&state_rc)),
                            0,
                            0,
                            127,
                            false,
                            None,
                        ))
                    };

                    // Start uninitiated at 0, since the real values only arrive with the workspace kit pull.
                    widget.set_value(0, false);
                    grid.attach(&widget.widget(), (i % 4) as i32, (i / 4) as i32, 1, 1);
                    lfo_widgets.push(widget);
                }
            }
            add_to_flowbox(&lfo_grid, &card);
            lfo_section.append(&create_action_row(Rc::clone(&state_rc), "lfos", "LFO"));
            root.append(&lfo_section);
        }

        let master_grid = FlowBox::new();
        master_grid.set_selection_mode(gtk4::SelectionMode::None);
        master_grid.set_row_spacing(24);
        master_grid.set_column_spacing(24);
        master_grid.set_halign(Align::Center);
        master_grid.set_valign(Align::Start);
        master_grid.add_css_class("no-hover-fb");

        {
            let fx_params = master_fx_list(&dc);
            if !fx_params.is_empty() {
                root.append(&Separator::new(Orientation::Horizontal));
                let fx_section = GtkBox::new(Orientation::Vertical, 20);
                fx_section.set_margin_top(4);
                fx_section.set_margin_bottom(4);
                let master_fx_label = Label::new(Some("Master Effects"));
                master_fx_label.add_css_class("title-3");
                fx_section.append(&master_fx_label);
                fx_section.append(&master_grid);
                let fx_groups: Vec<&[MasterFxParam]> = fx_params.chunk_by(|a, b| a.fx_name == b.fx_name).collect();
                for group in &fx_groups {
                    let (card, grid) = build_card(&group[0].fx_fullname);
                    for (i, param) in group.iter().enumerate() {
                        let state_rc_c = Rc::clone(&state_rc);
                        let sysex_byte = param.block_byte;
                        let param_id = param.param_id;
                        let callback = move |param_id: u8, val: i32, _: bool| {
                            if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                                state_ref.on_master_knob(sysex_byte, param_id, val as u8);
                            }
                        };
                        let master_knob = ParameterKnob::new(
                            &param.name,
                            &param.fullname,
                            param_id,
                            param.default_val,
                            callback,
                            param.display_offset,
                            0,
                            127,
                            false,
                            None,
                        );
                        master_knob.set_value(0, false);
                        grid.attach(master_knob.widget(), (i % 4) as i32, (i / 4) as i32, 1, 1);
                        master_knobs.push((sysex_byte, param_id, master_knob));
                    }
                    add_to_flowbox(&master_grid, &card);
                }
                master_grid.set_max_children_per_line(fx_groups.len().clamp(1, 3) as u32);
                fx_section.append(&create_action_row(Rc::clone(&state_rc), "fx", "FX"));
                root.append(&fx_section);
            }
        }

        // ── Miscellaneous ──────────────────────────────────────────────────────────────────────────────────

        // Kit params with no knob/CC path.
        // The device JSON's `misc_pages` declares everything: cards, rows, widget kinds, and the byte offsets each widget edits.
        // The renderer below is fully generic, same as the pages param renderer. A new device only needs JSON.
        let misc_pages = as_array_or(dc.json_get("misc_pages"), &[]);
        let is_updating_misc = Rc::new(Cell::new(false));
        let mut misc_controls: Vec<MiscControl> = Vec::new();

        // Param names for the assign matrix dropdowns, indexed by dest page byte (0-8): PTCH, the four knob pages, the three LFOs, MIDI.
        // Index 1 (SYNT) holds generic names. The machine's own names override at display time.
        let mut assign_dest_names: Vec<Vec<String>> = Vec::new();
        let mut assign_page_options: Vec<String> = Vec::new();
        let mut dynamic_machine_assign_idx: Option<usize> = None;

        if let Some(targets) = dc.json_get("assign_targets").and_then(|value| value.as_array()) {
            for (target_idx, target) in targets.iter().enumerate() {
                assign_page_options.push(as_string_or(target.json_get("name"), "").to_string());
                if target
                    .json_get("dynamic_machine")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    dynamic_machine_assign_idx = Some(target_idx);
                }
                if let Some(options) = target.json_get("options").and_then(|value| value.as_array()) {
                    assign_dest_names.push(
                        options
                            .iter()
                            .filter_map(|option| option.as_str().map(std::string::ToString::to_string))
                            .collect(),
                    );
                } else if let Some(ref_path) = target.json_get("ref").and_then(|value| value.as_str()) {
                    let page_idx = target.json_get("index").and_then(serde_json::Value::as_i64).unwrap_or(0) as usize;
                    let mut params = Vec::new();
                    if let Some(page) = dc.json_get(&format!("{ref_path}.{page_idx}")) {
                        params = as_array_or(page.json_get("params"), &[])
                            .iter()
                            .map(|param| as_string_or(param.json_get("name"), "").to_string())
                            .collect();
                    }
                    assign_dest_names.push(params);
                } else {
                    assign_dest_names.push(Vec::new());
                }
            }
        }

        let track_names: Vec<String> = dc
            .json_get("tracks")
            .and_then(|value| value.as_array())
            .map(|array| {
                array
                    .iter()
                    .filter_map(|track| {
                        track
                            .json_get("trackname")
                            .and_then(|name| name.as_str())
                            .map(std::string::ToString::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let misc_grid = FlowBox::new();
        misc_grid.set_max_children_per_line(misc_pages.len().clamp(1, 3) as u32);
        misc_grid.set_selection_mode(gtk4::SelectionMode::None);
        misc_grid.set_row_spacing(24);
        misc_grid.set_column_spacing(24);
        misc_grid.set_halign(Align::Center);
        misc_grid.set_valign(Align::Start);
        misc_grid.add_css_class("no-hover-fb");

        // Wraps a widget in a vertical box with a caption label above it, matching the piano control row styling.
        let caption_col = |caption: &str, widget: &gtk4::Widget| -> GtkBox {
            let col = GtkBox::new(Orientation::Vertical, 4);
            let caption_label = Label::new(Some(caption));
            caption_label.add_css_class("caption");
            caption_label.add_css_class("monospace");
            col.append(&caption_label);
            col.append(widget);
            col
        };

        // Resolves a misc param's byte position in the extracted track blob from its "field" (+ optional "index") reference.
        // Field names come from `sysex_layout.kit.track_blob`, so the JSON never carries raw blob numbers.
        let blob_idx = |param: &serde_json::Value| -> usize {
            let field = as_string_or(param.json_get("field"), "");
            let field_offset = as_u64_or(param.json_get("index"), 0) as usize;
            kit_blob_offset_or_die(&dc, field) + field_offset
        };

        for page in misc_pages {
            let (card, grid) = build_card(as_string_or(page.json_get("name"), ""));
            for (row_idx, row_def) in as_array_or(page.json_get("rows"), &[]).iter().enumerate() {
                let row_box = GtkBox::new(Orientation::Horizontal, 16);
                row_box.set_halign(Align::Center);
                for param in row_def.as_array().map_or(&[][..], std::vec::Vec::as_slice) {
                    let name = as_string_or(param.json_get("name"), "");
                    // Index into `misc_controls`. The changed-signal closures use it to find their control again.
                    let control_idx = misc_controls.len();
                    match as_string_or(param.json_get("widget"), "") {
                        "track_combo" => {
                            let blob_offset = blob_idx(param);
                            let combo = CustomDropdown::new(Some(0));
                            combo.set_focus_on_click(false);
                            combo.append_text("OFF");
                            for track_name in &track_names {
                                combo.append_text(track_name);
                            }
                            combo.set_active(Some(0));
                            {
                                let state_rc_c = Rc::clone(&state_rc);
                                let is_updating_misc_c = Rc::clone(&is_updating_misc);
                                combo.connect_changed(move |sel_idx| {
                                    if is_updating_misc_c.get() {
                                        return;
                                    }
                                    let Some(sel_idx) = sel_idx else { return };
                                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                                        state_mut.on_misc_combo_changed(control_idx, sel_idx);
                                    }
                                });
                            }
                            row_box.append(&caption_col(name, combo.inner.upcast_ref()));
                            // The optional live command byte is referenced by JSON path so it stays single-sourced under `sysex_api`.
                            let cmd = param
                                .json_get("cmd_path")
                                .and_then(|value| value.as_str())
                                .and_then(|path| dc.json_get(path))
                                .and_then(serde_json::Value::as_u64)
                                .map(|value| value as u8);
                            misc_controls.push(MiscControl::TrackCombo { blob_offset, cmd, combo });
                        }
                        "toggle" => {
                            let blob_offset = blob_idx(param);
                            let btn = ToggleButton::with_label("Off");
                            btn.set_focus_on_click(false);
                            {
                                let state_rc_c = Rc::clone(&state_rc);
                                let is_updating_misc_c = Rc::clone(&is_updating_misc);
                                btn.connect_toggled(move |toggle_btn| {
                                    toggle_btn.set_label(if toggle_btn.is_active() { "On" } else { "Off" });
                                    if is_updating_misc_c.get() {
                                        return;
                                    }
                                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                                        state_mut.on_misc_toggle_changed(control_idx, toggle_btn.is_active());
                                    }
                                });
                            }
                            row_box.append(&caption_col(name, btn.upcast_ref()));
                            misc_controls.push(MiscControl::Toggle { blob_offset, btn });
                        }
                        kind @ ("spinner" | "shared_spinner" | "global_spinner") => {
                            // Global spinners address the kit-globals block by plain index. The others resolve a blob field offset.
                            // `display = stored + display_offset`, the same convention the knob pages use.
                            let display_offset = param.json_get("display_offset").and_then(serde_json::Value::as_i64).unwrap_or(0) as i32;
                            let min = param
                                .json_get("min")
                                .and_then(serde_json::Value::as_i64)
                                .unwrap_or(i64::from(display_offset)) as f64;
                            let max = param
                                .json_get("max")
                                .and_then(serde_json::Value::as_i64)
                                .unwrap_or(127 + i64::from(display_offset)) as f64;
                            let spin = NumberSpinner::new(min, min, max, 1.0, "", 0);
                            {
                                let state_rc_c = Rc::clone(&state_rc);
                                let is_updating_misc_c = Rc::clone(&is_updating_misc);
                                let spin_c = spin.clone();
                                spin.connect_value_changed(move || {
                                    if is_updating_misc_c.get() {
                                        return;
                                    }
                                    let spin_val = spin_c.value();
                                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                                        state_mut.on_misc_spin_changed(control_idx, spin_val);
                                    }
                                });
                            }
                            row_box.append(&caption_col(name, spin.widget().upcast_ref()));
                            misc_controls.push(match kind {
                                "shared_spinner" => MiscControl::SharedSpinner {
                                    blob_offset: blob_idx(param),
                                    display_offset,
                                    spin,
                                },
                                "global_spinner" => {
                                    let global_idx = as_u64_or(param.json_get("idx"), 0) as usize;
                                    MiscControl::GlobalSpinner {
                                        global_idx,
                                        display_offset,
                                        spin,
                                    }
                                }
                                _ => MiscControl::Spinner {
                                    blob_offset: blob_idx(param),
                                    display_offset,
                                    spin,
                                },
                            });
                        }
                        "masked_combo" => {
                            let blob_offset = blob_idx(param);
                            let mask = as_u64_or_die(param.json_get("mask")) as u8;
                            let combo = CustomDropdown::new(Some(0));
                            combo.set_focus_on_click(false);
                            let mut values = Vec::new();
                            for option in as_array_or(param.json_get("options"), &[]) {
                                combo.append_text(as_string_or(option.json_get("label"), ""));
                                values.push(as_u64_or(option.json_get("value"), 0) as u8);
                            }
                            combo.set_active(Some(0));
                            {
                                let state_rc_c = Rc::clone(&state_rc);
                                let is_updating_misc_c = Rc::clone(&is_updating_misc);
                                combo.connect_changed(move |sel_idx| {
                                    if is_updating_misc_c.get() {
                                        return;
                                    }
                                    let Some(sel_idx) = sel_idx else { return };
                                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                                        state_mut.on_misc_combo_changed(control_idx, sel_idx);
                                    }
                                });
                            }
                            row_box.append(&caption_col(name, combo.inner.upcast_ref()));
                            misc_controls.push(MiscControl::MaskedCombo {
                                blob_offset,
                                mask,
                                values,
                                combo,
                            });
                        }
                        "global_combo" => {
                            let global_idx = as_u64_or(param.json_get("idx"), 0) as usize;
                            let combo = CustomDropdown::new(Some(0));
                            combo.set_focus_on_click(false);
                            for option in as_array_or(param.json_get("options"), &[]) {
                                if let Some(label) = option.as_str() {
                                    combo.append_text(label);
                                }
                            }
                            combo.set_active(Some(0));
                            {
                                let state_rc_c = Rc::clone(&state_rc);
                                let is_updating_misc_c = Rc::clone(&is_updating_misc);
                                combo.connect_changed(move |sel_idx| {
                                    if is_updating_misc_c.get() {
                                        return;
                                    }
                                    let Some(sel_idx) = sel_idx else { return };
                                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                                        state_mut.on_misc_combo_changed(control_idx, sel_idx);
                                    }
                                });
                            }
                            row_box.append(&caption_col(name, combo.inner.upcast_ref()));
                            misc_controls.push(MiscControl::GlobalCombo { global_idx, combo });
                        }
                        "assign_slot" => {
                            // The three dest arrays are this widget's contract. Their blob positions come from the layout.
                            let slot = as_u64_or_die(param.json_get("slot")) as usize;
                            let page_field = as_string_or(param.json_get("page_field"), "destination_pages");
                            let param_field = as_string_or(param.json_get("param_field"), "destination_params");
                            let range_field = as_string_or(param.json_get("depth_field"), "destination_ranges");
                            let page_idx = kit_blob_offset(&dc, page_field).unwrap() + slot;
                            let param_idx = kit_blob_offset(&dc, param_field).unwrap() + slot;
                            let range_idx = kit_blob_offset(&dc, range_field).unwrap() + slot;

                            let page_combo = CustomDropdown::new(Some(0));
                            page_combo.set_focus_on_click(false);
                            for page_name in &assign_page_options {
                                page_combo.append_text(page_name);
                            }
                            page_combo.set_active(Some(0));
                            {
                                let state_rc_c = Rc::clone(&state_rc);
                                let is_updating_misc_c = Rc::clone(&is_updating_misc);
                                page_combo.connect_changed(move |sel_idx| {
                                    if is_updating_misc_c.get() {
                                        return;
                                    }
                                    let Some(sel_idx) = sel_idx else { return };
                                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                                        state_mut.on_assign_page_changed(control_idx, sel_idx);
                                    }
                                });
                            }
                            row_box.append(&caption_col("Page", page_combo.inner.upcast_ref()));

                            let param_combo = CustomDropdown::new(Some(0));
                            param_combo.set_focus_on_click(false);
                            for param_name in assign_dest_names.first().map_or(&[][..], std::vec::Vec::as_slice) {
                                param_combo.append_text(param_name);
                            }
                            param_combo.set_active(Some(0));
                            {
                                let state_rc_c = Rc::clone(&state_rc);
                                let is_updating_misc_c = Rc::clone(&is_updating_misc);
                                param_combo.connect_changed(move |sel_idx| {
                                    if is_updating_misc_c.get() {
                                        return;
                                    }
                                    let Some(sel_idx) = sel_idx else { return };
                                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                                        state_mut.on_assign_param_changed(control_idx, sel_idx);
                                    }
                                });
                            }
                            row_box.append(&caption_col("Param", param_combo.inner.upcast_ref()));

                            let depth_spin = NumberSpinner::new(0.0, -64.0, 63.0, 1.0, "", 0);
                            {
                                let state_rc_c = Rc::clone(&state_rc);
                                let is_updating_misc_c = Rc::clone(&is_updating_misc);
                                let depth_spin_c = depth_spin.clone();
                                depth_spin.connect_value_changed(move || {
                                    if is_updating_misc_c.get() {
                                        return;
                                    }
                                    let spin_val = depth_spin_c.value();
                                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                                        state_mut.on_assign_depth_changed(control_idx, spin_val);
                                    }
                                });
                            }
                            row_box.append(&caption_col("Depth", depth_spin.widget().upcast_ref()));

                            misc_controls.push(MiscControl::AssignSlot {
                                page_idx,
                                param_idx,
                                range_idx,
                                page_combo,
                                param_combo,
                                depth_spin,
                            });
                        }
                        _ => {}
                    }
                }
                grid.attach(&row_box, 0, row_idx as i32, 1, 1);
            }
            // Center the content so sparse cards don't hug their title when neighbors stretch the row.
            grid.set_vexpand(true);
            grid.set_valign(Align::Center);
            add_to_flowbox(&misc_grid, &card);
        }

        // Only mount the section when the device contributed cards (future devices may have neither).
        let misc_section = if misc_grid.first_child().is_some() {
            root.append(&Separator::new(Orientation::Horizontal));
            let section = GtkBox::new(Orientation::Vertical, 20);
            section.set_margin_top(4);
            section.set_margin_bottom(4);
            let misc_label = Label::new(Some("Miscellaneous"));
            misc_label.add_css_class("title-3");
            section.append(&misc_label);
            section.append(&misc_grid);
            root.append(&section);
            Some(section)
        } else {
            None
        };

        base.build_status_area(false);

        let get_midi_rc = Rc::new(get_selected_midi);
        let get_midi_for_mixin = Rc::clone(&get_midi_rc);

        let state_mut = KitEditorState {
            hw,
            dc: Arc::clone(&dc),
            kit: kit_manager,
            get_midi_rc,
            root: root.clone(),
            auto_resolved_track: Cell::new(0),
            sound_clipboard: Rc::new(RefCell::new(None::<(Vec<u8>, usize)>)),
            kit_clipboard: Rc::new(RefCell::new(None)),
            synth_default_names,
            machine_param_names,
            machine_list,
            is_updating_machine_combo: Rc::clone(&is_updating_machine_combo),
            machine_combo_loaded_idx: Cell::new(-1),
            cc_knob_idx,
            keys_held: HashSet::new(),
            is_piano_mouse_held: false,
            is_space_held: false,
            piano_last_offset: 0,
            piano_repeat_src: None,
            is_poly_hw_updating: false,
            is_poly_multi_saved: false,
            state_wk: std::rc::Weak::new(),
            save_device_btn,
            track_combo,
            trigger_btn,
            track_menu_btn,
            machine_apply_btn,
            machine_combo,
            knobs,
            master_knobs,
            lfo_widgets,
            piano,
            joystick,
            poly_hw_btn,
            poly_btn,
            piano_repeat_btn,
            misc_section,
            misc_controls,
            assign_dest_names,
            dynamic_machine_assign_idx,
            lfo_dest_links,
            is_updating_misc,
            is_kit_fetch_locked: Arc::new(AtomicBool::new(false)),
        };

        *state_rc.borrow_mut() = Some(state_mut);
        // Wire `state_wk` so `schedule_piano_repeat()` can post a deferred callback.
        if let Some(state_mut) = state_rc.borrow_mut().as_mut() {
            state_mut.state_wk = Rc::downgrade(&state_rc);
        }

        // `MidiListenerMixin` handles port polling (1000 ms) and message draining (20 ms).
        // `open_extra_ports` fires when a port successfully opens: queries audio mode (MnM only) and kicks off the initial kit fetch.
        let mixin = {
            let on_msg = {
                let state_rc_c = Rc::clone(&state_rc);
                move |data: Vec<u8>| {
                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                        state_mut.on_midi_message(&data);
                    }
                }
            };
            let open_extra = {
                let state_rc_c = Rc::clone(&state_rc);
                move |port: &str| {
                    if let Some(state_ref) = state_rc_c.borrow().as_ref() {
                        if let Some(audio_mode_cmd) = state_ref.hw.status_audio_mode_cmd {
                            send_sysex(
                                port,
                                &build_elektron_sysex(state_ref.hw.prod, 0, &[state_ref.hw.status_query_cmd, audio_mode_cmd]),
                            );
                        }
                        state_ref.load_kit_on_connect(port);
                    }
                }
            };
            Rc::new(MidiListenerMixin::new(
                move || get_midi_for_mixin(),
                on_msg,
                |_| {},
                || {},
                |port| format!("Listening on '{port}'..."),
                open_extra,
                || {},
                || {},
                || {},
            ))
        };
        {
            let mixin_c_map = Rc::clone(&mixin);
            base.root.connect_map(move |_| mixin_c_map.on_map());
        }
        {
            let mixin_c_unmap = Rc::clone(&mixin);
            base.root.connect_unmap(move |_| mixin_c_unmap.on_unmap());
        }
        let mixin_ls = mixin.listener_state();

        // Separate timer for `pending_kit`. Background kit fetches post results here for UI application.
        {
            let state_rc_c_poll = Rc::clone(&state_rc);
            let mixin_ls_c = Rc::clone(&mixin_ls);
            glib::timeout_add_local(Duration::from_millis(20), move || {
                if !mixin_ls_c.borrow().is_active_page {
                    return glib::ControlFlow::Break;
                }
                let pending = {
                    let guard = state_rc_c_poll.borrow();
                    match guard.as_ref() {
                        Some(state_ref) => state_ref.kit.pending_kit.try_lock().ok().and_then(|mut lock| lock.take()),
                        _ => {
                            return glib::ControlFlow::Break;
                        }
                    }
                };
                if let Some(raw) = pending
                    && let Some(state_mut) = state_rc_c_poll.borrow_mut().as_mut()
                {
                    state_mut.apply_kit_ui(&raw);
                }
                glib::ControlFlow::Continue
            });
        }

        Self { root }
    }

    /// Returns the root widget for embedding in the navigation stack.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

/// What a computer-keyboard key maps to.
///
/// Either a piano note offset or a joystick move.
enum KeyAction {
    Note(i32),
    Joy,
}

/// Maps a keyboard key to the piano note or joystick action it triggers, if any.
fn get_key_action(key: gdk::Key) -> Option<KeyAction> {
    if let Some(&offset) = piano_key_map().get(&key) {
        return Some(KeyAction::Note(offset));
    }
    match key {
        gdk::Key::Up | gdk::Key::Down | gdk::Key::Left | gdk::Key::Right => Some(KeyAction::Joy),
        _ => None,
    }
}

impl KitEditorState {
    // ── Kit load and save ──────────────────────────────────────────────────────────────────────────────────

    /// Re-fetches the kit from the device.
    fn on_refresh_ui_clicked(&self) {
        if let Some(port) = (self.get_midi_rc)()
            && is_valid_port(&port)
        {
            self.load_kit_on_connect(&port);
        }
    }

    /// Toggles the "dirty" styling on the Save button to reflect unsaved kit edits.
    fn set_kit_dirty(&self, is_dirty: bool) {
        if is_dirty {
            self.save_device_btn.add_css_class("dirty-kit-btn");
        } else {
            self.save_device_btn.remove_css_class("dirty-kit-btn");
        }
    }

    /// Saves the current workspace kit to its slot.
    fn on_save_device_clicked(&self) {
        let port_name = self.get_midi_rc.as_ref()().unwrap_or_default();
        if !is_valid_port(&port_name) {
            return;
        }

        let hw = Arc::clone(&self.hw);

        glib::spawn_future_local(async move {
            let _ = run_midi_session(&port_name, move |midi_in, outport| -> Result<(), String> {
                if let Some(slot) = request_status_param(
                    midi_in,
                    outport,
                    hw.prod,
                    0,
                    hw.status_query_cmd,
                    hw.status_reply_cmd,
                    hw.status_kit_cmd,
                    2.0,
                ) {
                    outport.sysex(&build_elektron_sysex(hw.prod, 0, &[hw.save_kit_cmd, slot & 0x7F]));
                    sleep(Duration::from_millis(100));
                }
                Ok(())
            })
            .await;
        });
        self.set_kit_dirty(false);
    }

    /// Reloads the kit from its saved slot.
    fn on_reload_kit_clicked(&self) {
        if let Some(port) = (self.get_midi_rc)()
            && is_valid_port(&port)
        {
            // Reloading discards the live tweaks, so the workspace matches the saved slot again.
            self.set_kit_dirty(false);
            let in_port = find_input_port(&port);
            let hw = Arc::clone(&self.hw);
            let dc = Arc::clone(&self.dc);
            let pending = Arc::clone(&self.kit.pending_kit);

            glib::spawn_future_local(async move {
                if in_port.is_some() {
                    let _ = run_midi_session(&port, move |midi_in, outport| -> Result<(), String> {
                        // Load the current kit fresh so the workspace matches the saved slot, then read it back for the UI.
                        if let Some(slot) = request_status_param(
                            midi_in,
                            outport,
                            hw.prod,
                            0,
                            hw.status_query_cmd,
                            hw.status_reply_cmd,
                            hw.status_kit_cmd,
                            2.0,
                        ) {
                            outport.sysex(&build_elektron_sysex(hw.prod, 0, &[hw.load_kit_cmd, slot & 0x7F]));
                            sleep(Duration::from_millis(100));
                        }

                        if let Some(FetchResult { workspace, .. }) = fetch_kit_for_display(midi_in, outport, &hw, &dc)
                            && let Ok(mut lock) = pending.lock()
                        {
                            *lock = Some(workspace);
                        }
                        Ok(())
                    })
                    .await;
                }
            });
        }
    }

    /// Automatically fetches the kit from the device upon establishing a MIDI connection.
    fn load_kit_on_connect(&self, port_name: &str) {
        if self.is_kit_fetch_locked.swap(true, Ordering::Relaxed) {
            return;
        }
        let is_fetch_locked = Arc::clone(&self.is_kit_fetch_locked);
        let in_port = find_input_port(port_name);
        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let pending = Arc::clone(&self.kit.pending_kit);
        let port = port_name.to_string();
        glib::spawn_future_local(async move {
            let _lock = FetchGuard(is_fetch_locked);
            if in_port.is_some() {
                let _ = run_midi_session(&port, move |midi_in, outport| -> Result<(), String> {
                    if let Some(FetchResult { workspace, .. }) = fetch_kit_for_display(midi_in, outport, &hw, &dc)
                        && let Ok(mut lock) = pending.lock()
                    {
                        *lock = Some(workspace);
                    }
                    Ok(())
                })
                .await;
            }
        });
    }

    // ── Incoming MIDI ──────────────────────────────────────────────────────────────────────────────────────

    /// Routes incoming MIDI messages to the appropriate UI update handler.
    ///
    /// Handles Elektron SysEx (LFO, status replies, master knobs) and MIDI CC/Note messages.
    fn on_midi_message(&mut self, msg: &[u8]) {
        // Elektron SysEx format: F0 00 20 3C [prod] [channel] [block] [param] [val] F7
        //                         0  1  2  3    4       5       6      7      8    9
        //
        // `msg[5]` = MIDI channel, not the block byte. That is a common off-by-one mistake.
        if msg.len() >= 9 && msg.starts_with(&self.dc.sysex_header) {
            let block_hex = msg[ELEKTRON_TYPE_BYTE];
            let param_id = msg[ELEKTRON_PAYLOAD_START];
            let val = msg[8];

            if self.dc.is_device("MnM") && block_hex == self.hw.status_reply_cmd && Some(param_id) == self.hw.status_audio_mode_cmd {
                let poly_on = val == 1;
                if let Some(ref btn) = self.poly_hw_btn {
                    btn.set_active(poly_on);
                }
                return;
            }

            // The workspace kit is fetched once on connect (`open_extra_ports`).
            // Status-kit replies are intentionally ignored here.
            // Re-requesting on every reply made the device spam workspace requests in a loop.
            if block_hex == self.hw.status_reply_cmd && param_id == self.hw.status_kit_cmd {
                return;
            }

            if block_hex == self.hw.lfo_block_byte {
                let track_idx = (param_id / 8) as usize;
                let param_idx = (param_id % 8) as usize;
                // Keep `lfo_states` in sync for all tracks so `on_copy_sound()` captures current values.
                // That includes speed/depth/mix (`param_idx` 5/6/7), which are also in the kit dump blob.
                {
                    let mut lfo_states = self.kit.lfo_states.borrow_mut();
                    if track_idx < lfo_states.len() && param_idx < lfo_states[track_idx].len() {
                        lfo_states[track_idx][param_idx] = val;
                    }
                }
                let current = self.track_combo.active().unwrap_or(0) as usize;
                if (current == track_idx + self.hw.combo_offset as usize || current == 0) && param_idx < self.lfo_widgets.len() {
                    self.lfo_widgets[param_idx].set_value(i32::from(val), false);
                }
            } else {
                for (knob_sysex_byte, knob_param_id, knob) in &self.master_knobs {
                    if *knob_sysex_byte == block_hex && *knob_param_id == param_id {
                        knob.set_value(i32::from(val), false);
                        break;
                    }
                }
            }
        } else if !msg.is_empty() {
            let status = msg[0];
            let kind = status & 0xF0;
            let ch = status & 0x0F;
            let accept_channels = self.get_piano_channels();

            if let Some(tracks) = self.dc.json_get("tracks").and_then(|value| value.as_array()) {
                if (kind == 0x90 || kind == 0x80) && self.hw.has_auto_channel {
                    let current = self.track_combo.active().unwrap_or(0);
                    if current == 1 {
                        for (i, track) in tracks.iter().enumerate() {
                            if ch == as_u64_or(track.json_get("channel"), 0) as u8 {
                                if i != self.auto_resolved_track.get() {
                                    self.auto_resolved_track.set(i);
                                    self.on_track_changed();
                                }
                                break;
                            }
                        }
                    }
                }

                // Only light piano keys from incoming MIDI if the device has a native keyboard that sends notes.
                if as_bool_or(self.dc.json_get("system.piano_keyboard_output"), false) && accept_channels.contains(&ch) && msg.len() >= 2 {
                    let note = msg[1];
                    let offset = i32::from(note) - self.piano.base_note();
                    if (-8..=8).contains(&offset) {
                        let is_active = kind == 0x90 && msg.len() >= 3 && msg[2] > 0;
                        self.piano.set_key_active(offset, is_active);
                    }
                }
            }
            // CC: reflect the matching knob on the active track. Shared with outbound reflection (piano pitch) via `reflect_cc()`.
            if kind == 0xB0 && msg.len() >= 3 {
                self.reflect_cc(ch, msg[1], msg[2]);
            }
        }
    }

    /// Reflects a CC (control, value) on channel `ch` onto its track param: updates the stored value and the knob, if that track is shown.
    ///
    /// Shared by inbound MIDI handling (`on_midi_message()`) and outbound reflection (piano pitch), so the UI mirrors both directions.
    fn reflect_cc(&mut self, ch: u8, control: u8, value: u8) {
        let Some(tracks) = self.dc.json_get("tracks").and_then(|value| value.as_array()) else {
            return;
        };
        for (track_idx, track) in tracks.iter().enumerate() {
            if ch != as_u64_or(track.json_get("channel"), 0) as u8 {
                continue;
            }
            let track_offset = as_u64_or(track.json_get("offset"), 0) as u8;
            let Some(&knob_idx) = self.cc_knob_idx.get(&(i32::from(control) - i32::from(track_offset))) else {
                continue;
            };
            let mut stored = i32::from(value);
            if knob_idx < self.knobs.len() {
                let max_val = self.knobs[knob_idx].max_val();
                if max_val > 0 && max_val < 127 {
                    stored = ((i32::from(value) * (max_val + 1)) / 128).min(max_val);
                }
            }

            let current = self.track_combo.active().unwrap_or(0) as usize;
            if (current == track_idx + self.hw.combo_offset as usize
                || current == 0
                || (self.hw.has_auto_channel && current == 1 && track_idx == self.auto_resolved_track.get()))
                && knob_idx < self.knobs.len()
            {
                self.knobs[knob_idx].set_value(stored, false);
            }

            let mut track_states = self.kit.track_states.borrow_mut();
            if track_idx < track_states.len() && knob_idx < track_states[track_idx].len() {
                track_states[track_idx][knob_idx] = stored as u8;
            }
            break;
        }
    }

    // ── Track and channel selection ────────────────────────────────────────────────────────────────────────

    /// Returns the current track selection state from the UI combo box.
    fn get_track_selection(&self) -> TrackSel {
        let combo_idx = self.track_combo.active().unwrap_or(0);
        let is_track_all = combo_idx == 0;
        let is_track_auto = self.hw.has_auto_channel && combo_idx == 1;
        let track_idx = if is_track_auto {
            self.auto_resolved_track.get()
        } else {
            (combo_idx as i32 - self.hw.combo_offset as i32).max(0) as usize
        };
        let state_idx = if is_track_all { 16 } else { track_idx };
        TrackSel {
            is_track_all,
            is_track_auto,
            track_idx,
            state_idx,
        }
    }

    /// Returns the MIDI channels that the piano should transmit on based on the current track selection.
    fn get_piano_channels(&self) -> Vec<u8> {
        let mut channels = Vec::new();
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            if let Some(tracks) = self.dc.json_get("tracks").and_then(|value| value.as_array()) {
                channels.extend(tracks.iter().map(|track| as_u64_or(track.json_get("channel"), 0) as u8));
            }
        } else if track_selection.is_track_auto {
            channels.push(as_u64_or(self.dc.json_get("system.auto_channel"), 8) as u8);
        } else if let Some(tracks) = self.dc.json_get("tracks").and_then(|value| value.as_array())
            && let Some(track) = tracks.get(track_selection.track_idx)
        {
            channels.push(as_u64_or(track.json_get("channel"), 0) as u8);
        }
        if channels.is_empty() {
            channels.push(0);
        }
        channels
    }

    // ── Piano, joystick, and pitch bend ────────────────────────────────────────────────────────────────────

    /// Handles physical keyboard key presses.
    fn on_piano_key_pressed(&mut self, keyval: gdk::Key) -> glib::Propagation {
        if keyval == gdk::Key::space {
            if !self.is_space_held {
                self.is_space_held = true;
                self.trigger_btn.set_state_flags(gtk4::StateFlags::ACTIVE, false);
                self.trigger_current_track();
            }
            return glib::Propagation::Stop;
        }

        let Some(action) = get_key_action(keyval) else {
            return glib::Propagation::Proceed;
        };
        if self.keys_held.contains(&keyval) {
            return glib::Propagation::Stop;
        }

        match action {
            KeyAction::Joy => {
                self.keys_held.insert(keyval);
                if let Some(ref joystick_ref) = self.joystick {
                    joystick_ref.stop_physics();
                    self.sync_joystick();
                }
            }
            KeyAction::Note(offset) => {
                if self.dc.is_device("MnM") && !self.poly_btn.as_ref().is_some_and(gtk4::prelude::ToggleButtonExt::is_active) {
                    self.clear_held_piano_notes();
                }
                self.keys_held.insert(keyval);
                self.piano_last_offset = offset;
                self.piano.set_key_active(offset, true);
                self.fire_piano_note(offset);
                if self.piano_repeat_btn.is_active() {
                    self.cancel_piano_repeat();
                    self.schedule_piano_repeat();
                }
            }
        }
        glib::Propagation::Stop
    }

    /// Handles physical keyboard key releases.
    fn on_piano_key_released(&mut self, keyval: gdk::Key) {
        if keyval == gdk::Key::space {
            self.is_space_held = false;
            self.trigger_btn.unset_state_flags(gtk4::StateFlags::ACTIVE);
            return;
        }

        if !self.keys_held.remove(&keyval) {
            return;
        }

        match get_key_action(keyval) {
            Some(KeyAction::Joy) => self.sync_joystick(),
            Some(KeyAction::Note(offset)) => {
                self.piano.set_key_active(offset, false);

                if self.dc.is_device("MnM") {
                    self.release_piano_note(Some(offset));
                } else if self.dc.is_device("MD") && !self.is_note_held() && !self.is_piano_mouse_held {
                    self.cancel_piano_repeat();
                    self.release_piano_note(Some(offset));
                }
            }
            _ => {}
        }
    }

    /// Returns `true` if any physical piano keys are currently held.
    fn is_note_held(&self) -> bool {
        self.keys_held.iter().any(|&key| piano_key_map().contains_key(&key))
    }

    /// Synchronizes the joystick position with the set of currently held arrow keys.
    fn sync_joystick(&self) {
        let Some(ref joystick_ref) = self.joystick else { return };
        let has_key = |key| self.keys_held.contains(&key);
        let dx = f64::from(i32::from(has_key(gdk::Key::Right)) - i32::from(has_key(gdk::Key::Left)));
        let dy = f64::from(i32::from(has_key(gdk::Key::Down)) - i32::from(has_key(gdk::Key::Up)));

        if dx == 0.0 && dy == 0.0 {
            joystick_ref.start_physics();
        } else {
            joystick_ref.set_knob(dx * JOYSTICK_BASE_RADIUS, dy * JOYSTICK_BASE_RADIUS);
            self.on_joystick_moved(dx, dy);
        }
    }

    /// Handles mouse clicks on the piano keys.
    fn on_piano_press(&mut self, offset: i32) {
        self.is_piano_mouse_held = true;
        self.piano_last_offset = offset;
        if self.dc.is_device("MnM") && !self.poly_btn.as_ref().is_some_and(gtk4::prelude::ToggleButtonExt::is_active) {
            self.clear_held_piano_notes();
        }
        self.piano.set_key_active(offset, true);
        self.fire_piano_note(offset);
        if self.piano_repeat_btn.is_active() {
            self.cancel_piano_repeat();
            self.schedule_piano_repeat();
        }
    }

    /// Handles mouse releases from the piano keys.
    fn on_piano_release(&mut self, offset: i32) {
        self.is_piano_mouse_held = false;
        self.piano.set_key_active(offset, false);

        if self.dc.is_device("MnM") {
            self.release_piano_note(Some(offset));
        } else if self.dc.is_device("MD") && !self.is_note_held() && !self.is_piano_mouse_held {
            self.cancel_piano_repeat();
            self.release_piano_note(Some(offset));
        }
    }

    /// Releases all currently held piano notes.
    fn clear_held_piano_notes(&mut self) {
        let notes: Vec<_> = self
            .keys_held
            .iter()
            .filter_map(|&key| {
                if let Some(KeyAction::Note(offset)) = get_key_action(key) {
                    Some((key, offset))
                } else {
                    None
                }
            })
            .collect();

        for (key, offset) in notes {
            self.piano.set_key_active(offset, false);
            self.release_piano_note(Some(offset));
            self.keys_held.remove(&key);
        }
        self.cancel_piano_repeat();
    }

    /// Sends MIDI Note On messages for a piano note.
    fn fire_piano_note(&mut self, offset: i32) {
        if let Some(port) = (self.get_midi_rc)() {
            if !is_valid_port(&port) {
                return;
            }

            if self.dc.is_device("MnM") {
                let note = (self.piano.base_note() + offset).clamp(0, 127) as u8;
                for ch in self.get_piano_channels() {
                    send_midi(&port, &[0x90 | ch, note, PIANO_NOTE_VELOCITY]);
                }
            } else if self.dc.is_device("MD") {
                let tracks = if self.get_track_selection().is_track_all {
                    as_array_or(self.dc.json_get("tracks"), &[]).to_vec()
                } else {
                    let track = self
                        .dc
                        .json_get("tracks")
                        .and_then(|value| value.as_array())
                        .and_then(|array| array.get(self.get_track_selection().track_idx))
                        .cloned();
                    track.map(|track| vec![track]).unwrap_or_default()
                };

                for track in tracks {
                    let ch = as_u64_or(track.json_get("channel"), 0) as u8;
                    let track_offset = as_u64_or(track.json_get("offset"), 0) as u8;
                    let note = as_u64_or(track.json_get("note"), 0) as u8;
                    // The Machinedrum has no per-note pitch, so the pitch knob is set by CC first and the note then plays at it.
                    let pitch_cc = as_u64_or(self.dc.json_get("pages.synth.0.params.0.cc_id"), 0) as u8 + track_offset;
                    let pitch_val = semitone_to_pitch(offset);
                    send_midi_cc(&port, ch, pitch_cc, pitch_val);
                    // Mirror the outgoing pitch onto its knob so the UI moves with what's played.
                    self.reflect_cc(ch, pitch_cc, pitch_val);
                    send_midi(&port, &[0x90 | ch, note, PIANO_NOTE_VELOCITY]);
                }
            }
        }
    }

    /// Sends MIDI Note Off messages for a piano note.
    fn release_piano_note(&self, offset: Option<i32>) {
        if let Some(port) = (self.get_midi_rc)() {
            if !is_valid_port(&port) {
                return;
            }
            let mut msgs = Vec::new();

            if self.dc.is_device("MnM") {
                if let Some(offset) = offset {
                    let note = (self.piano.base_note() + offset).clamp(0, 127) as u8;
                    for ch in self.get_piano_channels() {
                        msgs.push(vec![0x90 | ch, note, 0]);
                    }
                }
            } else if self.dc.is_device("MD") {
                let tracks = if self.get_track_selection().is_track_all {
                    as_array_or(self.dc.json_get("tracks"), &[]).to_vec()
                } else {
                    let track = self
                        .dc
                        .json_get("tracks")
                        .and_then(|value| value.as_array())
                        .and_then(|array| array.get(self.get_track_selection().track_idx))
                        .cloned();
                    track.map(|track| vec![track]).unwrap_or_default()
                };

                for track in tracks {
                    let ch = as_u64_or(track.json_get("channel"), 0) as u8;
                    let note = as_u64_or(track.json_get("note"), 0) as u8;
                    msgs.push(vec![0x80 | ch, note, 0]);
                }
            }

            for msg in &msgs {
                send_midi(&port, msg);
            }
        }
    }

    /// Schedules a repeating piano note trigger at the current delay interval.
    fn schedule_piano_repeat(&mut self) {
        let delay_ms = u64::from(get_midi_delay_ms()).max(1);
        let state_wk_c = self.state_wk.clone();
        let src = glib::timeout_add_local(Duration::from_millis(delay_ms), move || {
            let Some(state_rc_strong) = state_wk_c.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let mut state_opt = state_rc_strong.borrow_mut();
            if let Some(ref mut state_mut) = *state_opt {
                state_mut.piano_repeat_src = None;
                if (state_mut.is_note_held() || state_mut.is_piano_mouse_held) && state_mut.piano_repeat_btn.is_active() {
                    let offset = state_mut.piano_last_offset;
                    state_mut.fire_piano_note(offset);
                    state_mut.schedule_piano_repeat();
                }
            }
            glib::ControlFlow::Break
        });
        self.piano_repeat_src = Some(src);
    }

    /// Cancels any currently scheduled piano note repetition.
    fn cancel_piano_repeat(&mut self) {
        if let Some(src) = self.piano_repeat_src.take() {
            src.remove();
        }
    }

    /// Handles changes to the hardware polyphony setting for the Monomachine.
    fn on_poly_hw_toggled(&self) {
        if let Some(ref btn) = self.poly_hw_btn {
            let active = btn.is_active();
            btn.set_label(if active { "On" } else { "Off" });
            if active {
                if let Some(ref poly_btn_ref) = self.poly_btn {
                    poly_btn_ref.set_active(true);
                    poly_btn_ref.set_sensitive(false);
                }
            } else if let Some(ref poly_btn_ref) = self.poly_btn {
                poly_btn_ref.set_sensitive(true);
                poly_btn_ref.set_active(self.is_poly_multi_saved);
            }

            if self.is_poly_hw_updating {
                return;
            }

            if let Some(port) = (self.get_midi_rc)()
                && is_valid_port(&port)
            {
                let audio_mode_cmd = self.hw.status_audio_mode_cmd.unwrap();
                send_sysex(
                    &port,
                    &build_elektron_sysex(self.hw.prod, 0, &[self.hw.status_set_cmd, audio_mode_cmd, u8::from(active)]),
                );
            }
        }
    }

    /// Sends MIDI Pitch Bend messages based on the PB knob value.
    fn on_pitch_bend_change(&self, val: i32) {
        if let Some(port) = (self.get_midi_rc)() {
            if !is_valid_port(&port) {
                return;
            }
            let pitch = if val >= 0 {
                (f64::from(val) / 63.0 * 8191.0) as i32
            } else {
                (f64::from(val) / 63.0 * 8192.0) as i32
            };
            // Pitch bend is an unsigned 14-bit value with 8192 as its center.
            let pitch = pitch + 8192;
            let lsb = (pitch & 0x7F) as u8;
            let msb = ((pitch >> 7) & 0x7F) as u8;
            let mut msgs = Vec::new();
            for ch in self.get_piano_channels() {
                msgs.push(vec![0xE0 | ch, lsb, msb]);
            }
            for msg in &msgs {
                send_midi(&port, msg);
            }
        }
    }

    /// Sends the joystick movement as MIDI messages (Pitch Bend or CC).
    fn on_joystick_moved(&self, x: f64, y: f64) {
        if let Some(port) = (self.get_midi_rc)() {
            if !is_valid_port(&port) {
                return;
            }
            let mut msgs = Vec::new();
            let channels = self.get_piano_channels();
            // X is one bipolar axis. The JSON supplies its transport.
            if let Some(axis) = &self.hw.joystick_x {
                for ch in &channels {
                    msgs.push(axis.bipolar_message(*ch, x));
                }
            }
            // Y splits into two unipolar axes. The inactive direction is sent at rest so crossing the center releases it.
            if let (Some(up), Some(down)) = (&self.hw.joystick_y_up, &self.hw.joystick_y_down) {
                let (up_amt, down_amt) = if y < 0.0 { (-y, 0.0) } else { (0.0, y) };
                for ch in &channels {
                    msgs.push(up.unipolar_message(*ch, up_amt));
                    msgs.push(down.unipolar_message(*ch, down_amt));
                }
            }
            for msg in &msgs {
                send_midi(&port, msg);
            }
        }
    }

    // ── Parameter edits ────────────────────────────────────────────────────────────────────────────────────

    /// Sends MIDI CC messages when a knob is changed.
    fn on_knob(&self, param_id: u8, val: i32, is_rand: bool) {
        let port = (self.get_midi_rc)();
        if !is_valid_port(port.as_deref().unwrap_or("")) {
            return;
        }
        let track_selection = self.get_track_selection();
        let knob_idx = *self.cc_knob_idx.get(&i32::from(param_id)).unwrap_or(&9999);
        if knob_idx >= self.knobs.len() {
            return;
        }

        let knob_widget = &self.knobs[knob_idx];
        let max_val = knob_widget.max_val();
        let tracks_data = as_array_or(self.dc.json_get("tracks"), &[]);
        if tracks_data.is_empty() {
            return;
        }

        let mut track_msgs = Vec::new();
        let val_u8 = val as u8;
        let send_val = if max_val < 127 {
            ((f64::from(val_u8) + 0.5) * 128.0 / (f64::from(max_val) + 1.0)).min(127.0) as u8
        } else {
            val_u8
        };

        if track_selection.is_track_all {
            for (track_idx, track) in tracks_data.iter().enumerate() {
                let ch = as_u64_or(track.json_get("channel"), 0) as u8;
                let track_offset = as_u64_or(track.json_get("offset"), 0) as u8;
                let (track_val_u8, track_send_val) = if is_rand {
                    let rand_val = pseudo_rand(max_val as u32) as u8;
                    let rand_send_val = if max_val < 127 {
                        ((f64::from(rand_val) + 0.5) * 128.0 / (f64::from(max_val) + 1.0)).min(127.0) as u8
                    } else {
                        rand_val
                    };
                    (rand_val, rand_send_val)
                } else {
                    (val_u8, send_val)
                };
                let mut track_states = self.kit.track_states.borrow_mut();
                if let Some(row) = track_states.get_mut(track_idx)
                    && knob_idx < row.len()
                {
                    row[knob_idx] = track_val_u8;
                }
                if let Some(row) = track_states.get_mut(16)
                    && knob_idx < row.len()
                {
                    row[knob_idx] = track_val_u8;
                }
                track_msgs.push((ch, param_id.wrapping_add(track_offset), track_send_val));
            }
        } else {
            if let Some(row) = self.kit.track_states.borrow_mut().get_mut(track_selection.track_idx)
                && knob_idx < row.len()
            {
                row[knob_idx] = val_u8;
            }
            if track_selection.is_track_auto {
                track_msgs.push((as_u64_or(self.dc.json_get("system.auto_channel"), 8) as u8, param_id, send_val));
            } else {
                let track = &tracks_data[track_selection.track_idx];
                let ch = as_u64_or(track.json_get("channel"), 0) as u8;
                let track_offset = as_u64_or(track.json_get("offset"), 0) as u8;
                track_msgs.push((ch, param_id.wrapping_add(track_offset), send_val));
            }
        }

        self.set_kit_dirty(true);

        let delay_secs = if track_selection.is_track_all {
            f64::from(get_midi_delay_ms()) / 1000.0
        } else {
            0.0
        };

        self.update_lfo_dest_links();

        if let Some(port_name) = port {
            glib::spawn_future_local(async move {
                let _ = run_midi_output(&port_name, move |outport| -> Result<(), String> {
                    for (ch, cc, val) in track_msgs {
                        let msg = [0xb0 | (ch & 0x0f), cc & 0x7f, val & 0x7f];
                        outport.midi(&msg);
                        if delay_secs > 0.0 {
                            sleep(Duration::from_secs_f64(delay_secs));
                        }
                    }
                    Ok(())
                })
                .await;
            });
        }
    }

    /// Sends a SysEx message to update a master effect parameter.
    fn on_master_knob(&self, sysex_hex: u8, param_id: u8, val: u8) {
        self.set_kit_dirty(true);
        if let Some(port) = (self.get_midi_rc)()
            && is_valid_port(&port)
        {
            send_sysex(&port, &build_elektron_sysex(self.hw.prod, 0, &[sysex_hex, param_id, val]));
        }
    }

    /// Sends a SysEx message to update a track-level LFO parameter.
    fn on_lfo_knob(&mut self, param_idx: u8, val: i32, is_rand: bool) {
        self.set_kit_dirty(true);
        let track_selection = self.get_track_selection();
        let val_u8 = val as u8;

        let mut max_val = 127u32;
        if (param_idx as usize) < self.lfo_widgets.len() {
            max_val = self.lfo_widgets[param_idx as usize].max_val() as u32;
        }

        let mut track_val_u8s = [val_u8; 16];
        // Keep `lfo_states` in sync so `get_lfo_shapes()` returns current values for the SHMIX display.
        {
            let mut lfo_states = self.kit.lfo_states.borrow_mut();
            if track_selection.is_track_all {
                for (track_idx, row) in lfo_states.iter_mut().enumerate() {
                    if (param_idx as usize) < row.len() {
                        let track_val_u8 = if is_rand { pseudo_rand(max_val) as u8 } else { val_u8 };
                        row[param_idx as usize] = track_val_u8;
                        if track_idx < 16 {
                            track_val_u8s[track_idx] = track_val_u8;
                        }
                    }
                }
            } else if let Some(row) = lfo_states.get_mut(track_selection.track_idx)
                && (param_idx as usize) < row.len()
            {
                row[param_idx as usize] = val_u8;
                if track_selection.track_idx < 16 {
                    track_val_u8s[track_selection.track_idx] = val_u8;
                }
            }
        }

        // SHP1 (`param_idx`=2) or SHP2 (`param_idx`=3) changed. Redraw the SHMIX widget (index 7).
        if (param_idx == 2 || param_idx == 3)
            && self.lfo_widgets.len() > 7
            && let ParameterWidget::Shape(ref shape) = self.lfo_widgets[7]
        {
            shape.drawing.queue_draw();
        }

        self.update_lfo_dest_links();

        if let Some(port) = (self.get_midi_rc)() {
            if !is_valid_port(&port) {
                return;
            }
            let sysex_block = self
                .dc
                .json_get("track_lfo.sysex.block_byte")
                .and_then(|value| value.as_str())
                .and_then(|sysex_str| u8::from_str_radix(sysex_str, 16).ok())
                .unwrap();
            if sysex_block == 0 {
                return;
            }
            let mut msgs = Vec::new();
            if track_selection.is_track_all {
                for i in 0u8..16 {
                    msgs.push(build_elektron_sysex(
                        self.hw.prod,
                        0,
                        &[sysex_block, i.wrapping_mul(8).wrapping_add(param_idx), track_val_u8s[i as usize]],
                    ));
                }
            } else {
                msgs.push(build_elektron_sysex(
                    self.hw.prod,
                    0,
                    &[
                        sysex_block,
                        (track_selection.track_idx as u8).wrapping_mul(8).wrapping_add(param_idx),
                        val_u8,
                    ],
                ));
            }
            for msg in &msgs {
                send_sysex(&port, msg);
            }
        }
    }

    // ── Machine assignment ─────────────────────────────────────────────────────────────────────────────────

    /// Sends a machine assignment SysEx message to the hardware for the current track.
    fn apply_machine_to_track(&self) {
        if let Some(ref combo) = self.machine_combo {
            let track_selection = self.get_track_selection();
            if track_selection.is_track_all {
                return;
            }
            let combo_idx = combo.active().unwrap_or(0);
            if combo_idx == 0 {
                return;
            } // "--" placeholder, nothing to apply

            let machine_idx = (combo_idx - 1) as usize;
            if machine_idx < self.machine_list.len() {
                let machine_id = self.machine_list[machine_idx].0;
                let track_idx = if track_selection.is_track_auto {
                    -1
                } else {
                    track_selection.track_idx as i32
                };

                if let Some(port) = (self.get_midi_rc)()
                    && is_valid_port(&port)
                {
                    if let Some(ref btn) = self.machine_apply_btn {
                        btn.set_sensitive(false);
                    }
                    let hw = Arc::clone(&self.hw);
                    let dc = Arc::clone(&self.dc);
                    let btn_wk = glib::SendWeakRef::from(self.machine_apply_btn.as_ref().unwrap().downgrade());
                    let pending = Arc::clone(&self.kit.pending_kit);

                    glib::spawn_future_local(async move {
                        let _ = run_midi_session(&port, move |midi_in, outport| -> Result<(), String> {
                            let Some(target_track) = resolve_auto_track(track_idx, &hw, midi_in, outport) else {
                                return Ok(());
                            };
                            let payload = build_machine_assignment_payload(&dc, target_track as u8, machine_id, None);
                            outport.sysex(&build_elektron_sysex(hw.prod, 0, &payload));
                            sleep(Duration::from_millis(50));
                            if let Some(FetchResult { workspace: merged, .. }) = fetch_kit_for_display(midi_in, outport, &hw, &dc)
                                && let Ok(mut lock) = pending.lock()
                            {
                                *lock = Some(merged);
                            }
                            Ok(())
                        })
                        .await;
                        if let Some(apply_btn_ref) = btn_wk.upgrade() {
                            apply_btn_ref.set_visible(false);
                            apply_btn_ref.set_sensitive(true);
                        }
                    });
                }
            }
        }
    }

    /// Toggles the Apply button visibility and updates synthesis knob labels on machine combo change.
    fn on_machine_combo_changed(&self) {
        if self.is_updating_machine_combo.get() {
            return;
        }

        if let (Some(btn), Some(combo_box)) = (&self.machine_apply_btn, &self.machine_combo) {
            // Index 0 is the "--" placeholder, meaning no machine is selected.
            let combo_idx = combo_box.active().unwrap_or(0);
            if combo_idx == 0 {
                btn.set_visible(false);
                return;
            }

            let machine_idx = (combo_idx - 1) as usize;
            if machine_idx < self.machine_list.len() {
                let machine_id = self.machine_list[machine_idx].0;
                let track_selection = self.get_track_selection();
                if track_selection.is_track_auto {
                    btn.set_visible(self.machine_combo_loaded_idx.get() < 0 || combo_idx as i32 != self.machine_combo_loaded_idx.get());
                } else {
                    let current = self
                        .kit
                        .machine_models
                        .borrow()
                        .get(&track_selection.state_idx)
                        .copied()
                        .unwrap_or(255);
                    btn.set_visible(machine_id != current);
                }
                self.update_synthesis_knob_labels(Some(machine_id));
            } else {
                btn.set_visible(false);
            }
        }
    }

    // ── Track switching and repaint ────────────────────────────────────────────────────────────────────────

    /// Updates the UI parameter values and labels when the active track is changed.
    fn on_track_changed(&self) {
        let track_selection = self.get_track_selection();
        if let Some(ref combo_box) = self.machine_combo {
            combo_box.set_visible(!track_selection.is_track_all);
        }
        if let Some(ref btn) = self.machine_apply_btn
            && track_selection.is_track_all
        {
            btn.set_visible(false);
        }

        {
            let track_states = self.kit.track_states.borrow();
            if track_selection.state_idx < track_states.len() {
                for (knob_idx, &val) in track_states[track_selection.state_idx].iter().enumerate() {
                    if knob_idx >= self.knobs.len() {
                        continue;
                    }
                    // Bytes > 127 mean this slot on this track was never populated in the saved-kit format the workspace was loaded from.
                    // The slot is real and addressable. Twisting the knob will populate it, but right now there is no value to display.
                    //
                    // Gray the knob to communicate that, and zero the underlying value so a stray send doesn't transmit 127.
                    let knob_widget = self.knobs[knob_idx].widget();
                    if val <= 127 {
                        self.knobs[knob_idx].set_value(i32::from(val), false);
                        knob_widget.set_opacity(1.0);
                    } else {
                        self.knobs[knob_idx].set_value(0, false);
                        knob_widget.set_opacity(0.35);
                    }
                }
            }
            let lfo_states = self.kit.lfo_states.borrow();
            if track_selection.state_idx < lfo_states.len() {
                for (i, &val) in lfo_states[track_selection.state_idx].iter().enumerate() {
                    if i < self.lfo_widgets.len() {
                        self.lfo_widgets[i].set_value(i32::from(val), false);
                    }
                }
            }
        }
        self.update_synthesis_labels(track_selection.state_idx);
        self.update_misc_widgets(track_selection.state_idx);
    }

    /// Hands a raw kit SysEx dump to the kit manager, then refreshes everything rendered from its state.
    fn apply_kit_ui(&mut self, kit_raw: &[u8]) {
        self.kit.ingest_kit(kit_raw);
        // Master FX knobs: read values from the kit blob via `kit_sysex_offset` and match to knobs by (`block_byte`, `param_id`).
        if self.dc.is_device("MD") && !self.master_knobs.is_empty() {
            let fx_vals = extract_master_fx(kit_raw, &self.dc);
            for (sysex_block, param_id, val) in fx_vals {
                for (knob_sysex_byte, knob_param_id, knob) in &self.master_knobs {
                    if *knob_sysex_byte == sysex_block && *knob_param_id == param_id {
                        knob.set_value(i32::from(val), false);
                    }
                }
            }
        }

        self.on_track_changed();
    }

    /// Updates synthesis knob labels and visibility for a given machine model ID.
    ///
    /// Pass `None` to restore generic default names. Does not touch the machine combo.
    fn update_synthesis_knob_labels(&self, model_id: Option<u8>) {
        let num_knobs = as_array_or(self.dc.json_get("pages.synth.0.params"), &[]).len();
        if let Some(machine_id) = model_id
            && let Some(names) = self.machine_param_names.get(&machine_id)
        {
            for i in 0..num_knobs {
                if i >= self.knobs.len() {
                    break;
                }
                let has_param = i < names.len() && !names[i].is_empty();
                // Use opacity+sensitivity so the grid cell size stays fixed.
                self.knobs[i].widget().set_opacity(if has_param { 1.0 } else { 0.0 });
                self.knobs[i].widget().set_sensitive(has_param);
                if has_param && let ParameterWidget::Knob(ref knob_widget) = self.knobs[i] {
                    knob_widget.label.set_label(&names[i]);
                }
            }
            return;
        }

        for i in 0..num_knobs {
            if i >= self.knobs.len() {
                break;
            }
            self.knobs[i].widget().set_opacity(1.0);
            self.knobs[i].widget().set_sensitive(true);
            if let (Some((short, _)), ParameterWidget::Knob(knob_widget)) = (self.synth_default_names.get(i), &self.knobs[i]) {
                knob_widget.label.set_label(short);
            }
        }

        self.update_lfo_dest_links();
    }

    /// Syncs the machine combo and synthesis knob labels to the track at `state_idx`.
    fn update_synthesis_labels(&self, state_idx: usize) {
        if self.synth_default_names.is_empty() {
            return;
        }

        let model_id = if state_idx < 16 {
            self.kit.machine_models.borrow().get(&state_idx).copied()
        } else {
            None
        };

        // Always sync the combo, even if param names for this machine aren't found.
        // Index 0 is the "--" placeholder. Real `machine_list` entries start at index 1.
        if let Some(ref machine_combo_ref) = self.machine_combo {
            self.is_updating_machine_combo.set(true);
            let active_idx = model_id
                .and_then(|machine_id| {
                    self.machine_list
                        .iter()
                        .enumerate()
                        .find(|(_, entry)| entry.0 == machine_id)
                        .map(|(i, _)| i as u32 + 1)
                })
                .unwrap_or(0);
            machine_combo_ref.set_active(Some(active_idx));
            self.machine_combo_loaded_idx.set(active_idx as i32);
            self.is_updating_machine_combo.set(false);
        }

        if let Some(ref btn) = self.machine_apply_btn {
            btn.set_visible(false);
        }

        self.update_synthesis_knob_labels(model_id);
    }

    /// Sends a MIDI note-on/off to trigger the currently selected track on the hardware.
    fn trigger_current_track(&self) {
        if let Some(port) = (self.get_midi_rc)()
            && is_valid_port(&port)
        {
            let track_selection = self.get_track_selection();
            let tracks_data = as_array_or(self.dc.json_get("tracks"), &[]);
            if tracks_data.is_empty() {
                return;
            }

            // Build (channel, note) pairs to trigger, one per selected track.
            let track_pairs: Vec<(u8, u8)> = if track_selection.is_track_auto {
                vec![(
                    as_u64_or(self.dc.json_get("system.auto_channel"), 8) as u8,
                    self.piano.base_note() as u8,
                )]
            } else if track_selection.is_track_all {
                tracks_data
                    .iter()
                    .map(|track| {
                        let ch = as_u64_or(track.json_get("channel"), 0) as u8;
                        let note = as_u64_or(track.json_get("note"), 60) as u8;
                        (ch, note)
                    })
                    .collect()
            } else {
                tracks_data
                    .get(track_selection.track_idx)
                    .map(|track| {
                        let ch = as_u64_or(track.json_get("channel"), 0) as u8;
                        let note = as_u64_or(track.json_get("note"), 60) as u8;
                        vec![(ch, note)]
                    })
                    .unwrap_or_default()
            };

            if track_pairs.is_empty() {
                return;
            }

            let delay_secs = f64::from(get_midi_delay_ms()) / 1000.0;
            let is_track_all = track_selection.is_track_all;

            glib::spawn_future_local(async move {
                for (ch, note) in track_pairs {
                    let msg_on = [0x90 | (ch & 0x0F), note & 0x7F, 95u8];
                    let msg_off = [0x90 | (ch & 0x0F), note & 0x7F, 0u8];
                    send_midi(&port, &msg_on);
                    if delay_secs > 0.0 {
                        glib::timeout_future(Duration::from_secs_f64(delay_secs)).await;
                    }
                    send_midi(&port, &msg_off);
                    if is_track_all && delay_secs > 0.0 {
                        glib::timeout_future(Duration::from_secs_f64(delay_secs)).await;
                    }
                }
            });
        }
    }

    // ── Sound copy, paste, and export ──────────────────────────────────────────────────────────────────────

    /// Dispatches a menu action (copy/paste sound, clear, save, etc.) to the appropriate handler.
    fn dispatch_menu_action(&mut self, action: &str) {
        match action {
            "copy_sound" => self.on_copy_sound(),
            "paste_sound" => self.on_paste_sound(),
            "clear_sound" => self.apply_mass_action("default", "ccs"),
            "save_sound" => self.on_save_sound(),
            "browse_sounds" => self.on_browse_sounds(),
            "copy_kit" => self.on_copy_kit(),
            "paste_kit" => self.on_paste_kit(),
            "clear_kit" => {
                self.apply_mass_action("default", "ccs");
                self.apply_mass_action("default", "fx");
            }
            "save_kit" => self.on_save_kit(),
            "import_kit" => self.on_import_kit(),
            _ => {}
        }
    }

    /// Fetches the current track's sound from the device and stores it in the sound clipboard.
    fn on_copy_sound(&self) {
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            return;
        }
        let port = (self.get_midi_rc)();
        if !is_valid_port(port.as_deref().unwrap_or("")) {
            return;
        }

        self.track_menu_btn.set_sensitive(false);
        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let sound_clipboard_ref = Rc::clone(&self.sound_clipboard);
        let btn_wk = glib::SendWeakRef::from(self.track_menu_btn.downgrade());
        let track_idx = if track_selection.is_track_auto {
            -1
        } else {
            track_selection.track_idx as i32
        };

        glib::spawn_future_local(async move {
            let mut sound_result: Option<(Vec<u8>, usize)> = None;
            if let Some(port_name) = port {
                let result = run_midi_session(&port_name, move |midi_in, outport| -> Result<Option<(Vec<u8>, usize)>, String> {
                    let Some(resolved_track_idx) = resolve_auto_track(track_idx, &hw, midi_in, outport) else {
                        return Ok(None);
                    };

                    if let Some(FetchResult { workspace: raw, .. }) = fetch_kit_for_display(midi_in, outport, &hw, &dc) {
                        let bytes = extract_sound_from_kit(&raw, resolved_track_idx as usize, &dc);
                        Ok(Some((bytes, resolved_track_idx as usize)))
                    } else {
                        Ok(None)
                    }
                })
                .await;
                if let Ok(Some(entry)) = result {
                    sound_result = Some(entry);
                }
            }
            if let Some(entry) = sound_result {
                *sound_clipboard_ref.borrow_mut() = Some(entry);
            }
            if let Some(track_menu_btn_ref) = btn_wk.upgrade() {
                track_menu_btn_ref.set_sensitive(true);
            }
        });
    }

    /// Writes the sound clipboard to the selected track on the device.
    fn on_paste_sound(&self) {
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            return;
        }
        let sound_clipboard_ref = self.sound_clipboard.borrow().clone();
        let Some((sound_data, source_track)) = sound_clipboard_ref else {
            return;
        };
        let port = (self.get_midi_rc)();
        if !is_valid_port(port.as_deref().unwrap_or("")) {
            return;
        }

        self.track_menu_btn.set_sensitive(false);
        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let btn_wk = glib::SendWeakRef::from(self.track_menu_btn.downgrade());
        let track_idx = if track_selection.is_track_auto {
            -1
        } else {
            track_selection.track_idx as i32
        };
        let pending = Arc::clone(&self.kit.pending_kit);
        let (cc_ids, track_channel_offsets) = extract_cc_meta(&self.dc);

        glib::spawn_future_local(async move {
            if let Some(port_name) = port {
                let _ = run_midi_session(&port_name, move |midi_in, outport| -> Result<(), String> {
                    let Some(resolved_track_idx) = resolve_auto_track(track_idx, &hw, midi_in, outport) else {
                        return Ok(());
                    };
                    let kit = apply_track_via_cc(
                        &hw,
                        &dc,
                        midi_in,
                        outport,
                        &sound_data,
                        Some(source_track),
                        resolved_track_idx as usize,
                        &cc_ids,
                        &track_channel_offsets,
                    );
                    if let Some(kit) = kit
                        && let Ok(mut lock) = pending.lock()
                    {
                        *lock = Some(kit);
                    }
                    Ok(())
                })
                .await;
            }
            if let Some(track_menu_btn_ref) = btn_wk.upgrade() {
                track_menu_btn_ref.set_sensitive(true);
            }
        });
    }

    /// Fetches the current track's sound from the device, then shows the export metadata dialog.
    fn on_save_sound(&self) {
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            return;
        }
        let port = (self.get_midi_rc)();
        if !is_valid_port(port.as_deref().unwrap_or("")) {
            return;
        }

        self.track_menu_btn.set_sensitive(false);
        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let btn_wk = glib::SendWeakRef::from(self.track_menu_btn.downgrade());
        let track_idx = if track_selection.is_track_auto {
            -1
        } else {
            track_selection.track_idx as i32
        };
        let state_c = self.state_wk.upgrade().unwrap();

        glib::spawn_future_local(async move {
            let mut result = None;
            if let Some(port_name) = port {
                type SoundExportResult = Option<(Vec<u8>, i32, String)>;
                let session_result = run_midi_session(&port_name, move |midi_in, outport| -> Result<SoundExportResult, String> {
                    let Some(resolved_track_idx) = resolve_auto_track(track_idx, &hw, midi_in, outport) else {
                        return Ok(None);
                    };
                    if let Some(FetchResult { workspace: raw, .. }) = fetch_kit_for_display(midi_in, outport, &hw, &dc) {
                        let bytes = extract_sound_from_kit(&raw, resolved_track_idx as usize, &dc);
                        let kit_name = extract_sysex_name(&raw).unwrap_or_default();
                        Ok(Some((bytes, resolved_track_idx, kit_name)))
                    } else {
                        Ok(None)
                    }
                })
                .await;
                if let Ok(Some(session_result)) = session_result {
                    result = Some(session_result);
                }
            }
            if let Some((bytes, resolved_track_idx, kit_name)) = result
                && let Some(state_ref) = state_c.borrow().as_ref()
            {
                export_sound(&state_ref.root, &state_ref.dc, bytes, resolved_track_idx as usize, &kit_name);
            }
            if let Some(track_menu_btn_ref) = btn_wk.upgrade() {
                track_menu_btn_ref.set_sensitive(true);
            }
        });
    }

    /// Opens the sound preset browser window.
    fn on_browse_sounds(&self) {
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            return;
        }
        let port = (self.get_midi_rc)();
        let Some(port_name) = port else {
            return;
        };
        if !is_valid_port(&port_name) {
            return;
        }

        let state_wk_c = self.state_wk.clone();
        let port_name_c = port_name.clone();
        let on_load = move |bytes: Vec<u8>, source_track: Option<usize>| {
            if let Some(state_c) = state_wk_c.upgrade()
                && let Some(state_ref) = state_c.borrow().as_ref()
            {
                state_ref.paste_sound_data(bytes, source_track, &port_name_c);
            }
        };

        let browser = SoundBrowserModal::new(&self.root, &self.dc.device_shorter, on_load);
        let original_data_c = Arc::clone(&browser.original_data);
        browser.present();

        // Fetch original track data in background so Cancel can revert.
        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let track_idx = if track_selection.is_track_auto {
            -1
        } else {
            track_selection.track_idx as i32
        };
        glib::spawn_future_local(async move {
            let mut original = None;
            let result = run_midi_session(&port_name, move |midi_in, outport| -> Result<Option<Vec<u8>>, String> {
                let Some(resolved_track_idx) = resolve_auto_track(track_idx, &hw, midi_in, outport) else {
                    return Ok(None);
                };

                if let Some(FetchResult { workspace: raw, .. }) = fetch_kit_for_display(midi_in, outport, &hw, &dc) {
                    let bytes = extract_sound_from_kit(&raw, resolved_track_idx as usize, &dc);
                    Ok(Some(bytes))
                } else {
                    Ok(None)
                }
            })
            .await;
            if let Ok(Some(bytes)) = result {
                original = Some(bytes);
            }
            if let Some(bytes) = original {
                *original_data_c.lock().unwrap() = Some(bytes);
            }
        });
    }

    /// Pastes sound data into the current hardware workspace and refreshes the UI.
    fn paste_sound_data(&self, bytes: Vec<u8>, source_track: Option<usize>, port: &str) {
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            return;
        }
        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let track_idx = if track_selection.is_track_auto {
            -1
        } else {
            track_selection.track_idx as i32
        };
        let pending = Arc::clone(&self.kit.pending_kit);
        let port_name = port.to_string();
        let (cc_ids, track_channel_offsets) = extract_cc_meta(&self.dc);

        glib::spawn_future_local(async move {
            let _ = run_midi_session(&port_name, move |midi_in, outport| -> Result<(), String> {
                let Some(resolved_track_idx) = resolve_auto_track(track_idx, &hw, midi_in, outport) else {
                    return Ok(());
                };
                let kit = apply_track_via_cc(
                    &hw,
                    &dc,
                    midi_in,
                    outport,
                    &bytes,
                    source_track,
                    resolved_track_idx as usize,
                    &cc_ids,
                    &track_channel_offsets,
                );
                if let Some(kit) = kit
                    && let Ok(mut lock) = pending.lock()
                {
                    *lock = Some(kit);
                }
                Ok(())
            })
            .await;
        });
    }

    // ── Kit copy, paste, and export ────────────────────────────────────────────────────────────────────────

    /// Fetches the current kit from the device and stores it in the kit clipboard.
    fn on_copy_kit(&self) {
        let port = (self.get_midi_rc)();
        if !is_valid_port(port.as_deref().unwrap_or("")) {
            return;
        }

        self.track_menu_btn.set_sensitive(false);
        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let kit_clipboard_ref = Rc::clone(&self.kit_clipboard);
        let btn_wk = glib::SendWeakRef::from(self.track_menu_btn.downgrade());

        glib::spawn_future_local(async move {
            let mut kit_bytes = None;
            if let Some(port_name) = port {
                let result = run_midi_session(&port_name, move |midi_in, outport| -> Result<Option<Vec<u8>>, String> {
                    if let Some(FetchResult { workspace: raw, .. }) = fetch_kit_for_display(midi_in, outport, &hw, &dc) {
                        Ok(Some(raw))
                    } else {
                        Ok(None)
                    }
                })
                .await;
                if let Ok(Some(raw)) = result {
                    kit_bytes = Some(raw);
                }
            }
            if let Some(bytes) = kit_bytes {
                *kit_clipboard_ref.borrow_mut() = Some(bytes);
            }
            if let Some(track_menu_btn_ref) = btn_wk.upgrade() {
                track_menu_btn_ref.set_sensitive(true);
            }
        });
    }

    /// Writes the kit clipboard to the device's workspace.
    fn on_paste_kit(&self) {
        let bytes = self.kit_clipboard.borrow().clone();
        let Some(kit_data) = bytes else {
            return;
        };
        let port = (self.get_midi_rc)();
        if !is_valid_port(port.as_deref().unwrap_or("")) {
            return;
        }

        self.track_menu_btn.set_sensitive(false);
        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let btn_wk = glib::SendWeakRef::from(self.track_menu_btn.downgrade());
        let pending = Arc::clone(&self.kit.pending_kit);

        glib::spawn_future_local(async move {
            if let Some(port_name) = port {
                let _ = run_midi_session(&port_name, move |midi_in, outport| -> Result<(), String> {
                    write_kit_to_workspace(&hw, outport, &kit_data);
                    sleep(Duration::from_millis(100));
                    if let Some(FetchResult { workspace: refreshed, .. }) = fetch_kit_for_display(midi_in, outport, &hw, &dc)
                        && let Ok(mut lock) = pending.lock()
                    {
                        *lock = Some(refreshed);
                    }
                    Ok(())
                })
                .await;
            }
            if let Some(track_menu_btn_ref) = btn_wk.upgrade() {
                track_menu_btn_ref.set_sensitive(true);
            }
        });
    }

    /// Fetches the current kit from the device, then shows the export metadata dialog.
    fn on_save_kit(&self) {
        let port = (self.get_midi_rc)();
        if !is_valid_port(port.as_deref().unwrap_or("")) {
            return;
        }

        self.track_menu_btn.set_sensitive(false);
        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let btn_wk = glib::SendWeakRef::from(self.track_menu_btn.downgrade());

        let state_c = self.state_wk.upgrade().unwrap();

        glib::spawn_future_local(async move {
            let mut result = None;
            if let Some(port_name) = port {
                let session_result = run_midi_session(
                    &port_name,
                    move |midi_in, outport| -> Result<Option<(Vec<u8>, u8, String)>, String> {
                        if let Some(FetchResult { slot, workspace: raw, .. }) = fetch_kit_for_display(midi_in, outport, &hw, &dc) {
                            let kit_name = extract_sysex_name(&raw).unwrap_or_default();
                            Ok(Some((raw, slot, kit_name)))
                        } else {
                            Ok(None)
                        }
                    },
                )
                .await;
                if let Ok(Some(session_result)) = session_result {
                    result = Some(session_result);
                }
            }
            if let Some((bytes, slot, kit_name)) = result
                && let Some(state_ref) = state_c.borrow().as_ref()
            {
                export_kit(&state_ref.root, &state_ref.dc, bytes, slot as usize, &kit_name);
            }
            if let Some(track_menu_btn_ref) = btn_wk.upgrade() {
                track_menu_btn_ref.set_sensitive(true);
            }
        });
    }

    /// Presents a file dialog to import a kit from disk and upload it to the device.
    fn on_import_kit(&self) {
        let dialog = make_file_dialog("Open Kit File", None);
        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("Kit files (*.c7, *.syx)"));
        filter.add_pattern("*.c7");
        filter.add_pattern("*.syx");
        let filters = gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));

        let hw = Arc::clone(&self.hw);
        let dc = Arc::clone(&self.dc);
        let port = (self.get_midi_rc)();
        let pending = Arc::clone(&self.kit.pending_kit);
        let btn_wk = glib::SendWeakRef::from(self.track_menu_btn.downgrade());
        let root_wk = glib::SendWeakRef::from(self.root.downgrade());
        let (cc_ids, track_channel_offsets) = extract_cc_meta(&self.dc);

        let parent_window = self.root.root().and_then(|root_widget| root_widget.downcast::<gtk4::Window>().ok());
        dialog.open(parent_window.as_ref(), None::<&Cancellable>, move |result| {
            if let Ok(file) = result
                && let (Some(path), Some(port_name)) = (file.path(), port)
            {
                let items = read_c7_or_sysex_file(&path);
                if let Some(root) = root_wk.upgrade() {
                    // Reject files that don't contain kit data.
                    if !does_file_type_match(&root, path.to_str().unwrap_or(""), &items, "kit") {
                        return;
                    }
                    // Reject files exported from a different device family.
                    if !verify_device_match(&root, path.to_str().unwrap_or(""), &items, Some(&dc.device_short), "") {
                        return;
                    }
                }
                // Kits are stored as the raw SysEx dump (both `.c7` and `.syx` files parse to `C7Data`::Binary).
                if let Some(item) = find_c7_item(&items, Some("kit"), None) {
                    let kit_bytes = item.get_data_bytes().map(<[u8]>::to_vec);
                    if let Some(kit_data) = kit_bytes {
                        if let Some(track_menu_btn_ref) = btn_wk.upgrade() {
                            track_menu_btn_ref.set_sensitive(false);
                        }
                        glib::spawn_future_local(async move {
                            let _ = run_midi_session(&port_name, move |midi_in, outport| -> Result<(), String> {
                                let num_tracks = as_array_or_die(dc.json_get("tracks")).len();
                                for track_idx in 0..num_tracks {
                                    let sound_bytes = extract_sound_from_kit(&kit_data, track_idx, &dc);
                                    apply_track_via_cc(
                                        &hw,
                                        &dc,
                                        midi_in,
                                        outport,
                                        &sound_bytes,
                                        None,
                                        track_idx,
                                        &cc_ids,
                                        &track_channel_offsets,
                                    );
                                }
                                if dc.is_device("MD") {
                                    let fx_vals = extract_master_fx(&kit_data, &dc);
                                    for (sysex_block, param_id, val) in fx_vals {
                                        outport.sysex(&build_elektron_sysex(hw.prod, 0, &[sysex_block, param_id, val]));
                                        sleep(Duration::from_millis(2));
                                    }
                                }
                                if let Ok(mut lock) = pending.lock() {
                                    *lock = Some(kit_data);
                                }
                                Ok(())
                            })
                            .await;
                            if let Some(track_menu_btn_ref) = btn_wk.upgrade() {
                                track_menu_btn_ref.set_sensitive(true);
                            }
                        });
                    }
                }
            }
        });
    }

    // ── Mass actions and LFO shapes ────────────────────────────────────────────────────────────────────────

    /// Applies a batch operation (default, randomize, etc.) to a set of parameters or FX.
    fn apply_mass_action(&mut self, action: &str, target: &str) {
        self.set_kit_dirty(true);
        let port = (self.get_midi_rc)();
        if !port.as_deref().is_some_and(is_valid_port) {
            return;
        }
        let port = port.unwrap();
        let track_selection = self.get_track_selection();
        let mut msgs: Vec<Vec<u8>> = Vec::new();
        let tracks_data = as_array_or(self.dc.json_get("tracks"), &[]);

        let target_val = |action: &str, default_val: i32, max_val: i32| -> u8 {
            let target_val = match action {
                "default" => default_val,
                "minimize" => 0,
                "middle" => (max_val + 1) / 2,
                "maximize" => max_val,
                _ => pseudo_rand(max_val as u32) as i32,
            };
            target_val.clamp(0, 255) as u8
        };

        if target == "ccs" && !tracks_data.is_empty() {
            let track_indices: Vec<usize> = if track_selection.is_track_all {
                (0..tracks_data.len()).collect()
            } else {
                vec![track_selection.track_idx]
            };
            for &track_idx in &track_indices {
                let track = &tracks_data[track_idx];
                let ch = if track_selection.is_track_auto {
                    as_u64_or(self.dc.json_get("system.auto_channel"), 8) as u8
                } else {
                    as_u64_or(track.json_get("channel"), 0) as u8
                };
                let track_offset = as_u64_or(track.json_get("offset"), 0) as u8;
                let mut knob_idx = 0usize;
                for page in as_array_or(self.dc.json_get("pages.synth"), &[]) {
                    if let Some(params) = page.json_get("params").and_then(|value| value.as_array()) {
                        for json_param in params {
                            let default_val = json_param.json_get("default_val").and_then(serde_json::Value::as_i64);
                            // Skip params with no default when restoring defaults.
                            if action == "default" && default_val.is_none() {
                                knob_idx += 1;
                                continue;
                            }
                            let max_val = if knob_idx < self.knobs.len() {
                                self.knobs[knob_idx].max_val()
                            } else {
                                127
                            };
                            let val = target_val(action, default_val.unwrap_or(0) as i32, max_val);
                            {
                                let mut track_states = self.kit.track_states.borrow_mut();
                                if track_idx < track_states.len() && knob_idx < track_states[track_idx].len() {
                                    track_states[track_idx][knob_idx] = val;
                                }
                                if track_selection.is_track_all && track_states.len() > 16 && knob_idx < track_states[16].len() {
                                    track_states[16][knob_idx] = val;
                                }
                            }
                            if (track_selection.track_idx == track_idx || (track_selection.is_track_all && track_idx == 0))
                                && knob_idx < self.knobs.len()
                            {
                                self.knobs[knob_idx].set_value(i32::from(val), false);
                            }
                            let cc_id = as_u64_or(json_param.json_get("cc_id"), 0) as u8;
                            msgs.push(vec![
                                0xB0 | ch,
                                if track_selection.is_track_auto {
                                    cc_id
                                } else {
                                    cc_id + track_offset
                                },
                                val,
                            ]);
                            knob_idx += 1;
                        }
                    }
                }
            }
        } else if target == "mod_ccs" && !tracks_data.is_empty() {
            let track_indices: Vec<usize> = if track_selection.is_track_all {
                (0..tracks_data.len()).collect()
            } else {
                vec![track_selection.track_idx]
            };
            let lfo_pages = as_array_or(self.dc.json_get("pages.lfo"), &[]);
            if !lfo_pages.is_empty() {
                for &track_idx in &track_indices {
                    let track = &tracks_data[track_idx];
                    let ch = if track_selection.is_track_auto {
                        as_u64_or(self.dc.json_get("system.auto_channel"), 8) as u8
                    } else {
                        as_u64_or(track.json_get("channel"), 0) as u8
                    };
                    let track_offset = as_u64_or(track.json_get("offset"), 0) as u8;
                    for page in lfo_pages {
                        if let Some(params) = page.json_get("params").and_then(|value| value.as_array()) {
                            for json_param in params {
                                let cc_id = json_param.json_get("cc_id").and_then(serde_json::Value::as_i64).unwrap_or(-1) as i32;
                                let Some(&knob_idx) = self.cc_knob_idx.get(&cc_id) else {
                                    continue;
                                };
                                let max_val = if knob_idx < self.knobs.len() {
                                    self.knobs[knob_idx].max_val()
                                } else {
                                    127
                                };
                                let default_val =
                                    json_param.json_get("default_val").and_then(serde_json::Value::as_i64).unwrap_or(0) as i32;
                                let val = target_val(action, default_val, max_val);
                                {
                                    let mut track_states = self.kit.track_states.borrow_mut();
                                    if track_idx < track_states.len() && knob_idx < track_states[track_idx].len() {
                                        track_states[track_idx][knob_idx] = val;
                                    }
                                    if track_selection.is_track_all && track_states.len() > 16 && knob_idx < track_states[16].len() {
                                        track_states[16][knob_idx] = val;
                                    }
                                }
                                if (track_selection.track_idx == track_idx || (track_selection.is_track_all && track_idx == 0))
                                    && knob_idx < self.knobs.len()
                                {
                                    self.knobs[knob_idx].set_value(i32::from(val), false);
                                }
                                let cc_id = cc_id as u8;
                                msgs.push(vec![
                                    0xB0 | ch,
                                    if track_selection.is_track_auto {
                                        cc_id
                                    } else {
                                        cc_id + track_offset
                                    },
                                    val,
                                ]);
                            }
                        }
                    }
                }
            }
        } else if target == "lfos" {
            let sysex_block = self
                .dc
                .json_get("track_lfo.sysex.block_byte")
                .and_then(|value| value.as_str())
                .and_then(|sysex_str| u8::from_str_radix(sysex_str, 16).ok())
                .unwrap();
            if sysex_block == 0 {
                return;
            }
            let track_indices: Vec<usize> = if track_selection.is_track_all {
                (0..16).collect()
            } else {
                vec![track_selection.track_idx]
            };
            for &track_idx in &track_indices {
                for (param_idx, widget) in self.lfo_widgets.iter().enumerate() {
                    if param_idx == 0 && action == "rand" {
                        continue;
                    }
                    let max_val = widget.max_val();
                    // `param_idx` 0 is the TRACK selector. Its natural default is the track number itself.
                    let default_val = if param_idx == 0 && track_idx < 16 {
                        track_idx as i32
                    } else {
                        widget.default_val()
                    };
                    let val = target_val(action, default_val, max_val);
                    {
                        let mut lfo_states = self.kit.lfo_states.borrow_mut();
                        if track_idx < lfo_states.len() && param_idx < lfo_states[track_idx].len() {
                            lfo_states[track_idx][param_idx] = val;
                        }
                        if track_selection.is_track_all && lfo_states.len() > 16 && param_idx < lfo_states[16].len() {
                            lfo_states[16][param_idx] = val;
                        }
                    }
                    if track_selection.track_idx == track_idx || (track_selection.is_track_all && track_idx == 0) {
                        widget.set_value(i32::from(val), false);
                    }
                    msgs.push(build_elektron_sysex(
                        self.hw.prod,
                        0,
                        &[sysex_block, (track_idx as u8).wrapping_mul(8).wrapping_add(param_idx as u8), val],
                    ));
                }
            }
        } else if target == "fx" {
            for (sysex_block, param_id, knob) in &self.master_knobs {
                let val = target_val(action, knob.default_val(), 127);
                knob.set_value(i32::from(val), false);
                msgs.push(build_elektron_sysex(self.hw.prod, 0, &[*sysex_block, *param_id, val]));
            }
        }

        let delay_secs = f64::from(get_midi_delay_ms()) / 1000.0;
        glib::spawn_future_local(async move {
            let _ = run_midi_output(&port, move |outport| -> Result<(), String> {
                for msg in &msgs {
                    // `msgs` mixes SysEx (machine assign, LFO block) and raw CC pokes. Dispatch on the leading byte.
                    if msg.first() == Some(&0xF0) {
                        outport.sysex(msg);
                    } else {
                        outport.midi(msg);
                    }
                    if delay_secs > 0.0 {
                        sleep(Duration::from_secs_f64(delay_secs));
                    }
                }
                Ok(())
            })
            .await;
        });
    }

    /// Returns the active LFO shapes for the current track to update the SHMIX visualization.
    fn get_lfo_shapes(&self) -> (usize, usize) {
        let track_selection = self.get_track_selection();
        if track_selection.state_idx < 17 {
            let lfo_states = self.kit.lfo_states.borrow();
            let row = lfo_states.get(track_selection.state_idx).map_or(&[][..], std::vec::Vec::as_slice);
            (row.get(2).copied().unwrap_or(0) as usize, row.get(3).copied().unwrap_or(0) as usize)
        } else {
            (0, 0)
        }
    }

    // ── Miscellaneous controls ─────────────────────────────────────────────────────────────────────────────

    /// Repaints the Miscellaneous section from `track_states`, desensitized for ALL since these are per-track bytes.
    ///
    /// Skips the ">127 = uninitiated" gray-out used for the CC knobs, because 255 is real data (OFF).
    fn update_misc_widgets(&self, state_idx: usize) {
        let Some(section) = &self.misc_section else {
            return;
        };
        let is_track_all = state_idx >= 16;
        section.set_sensitive(!is_track_all);
        if is_track_all {
            return;
        }
        let track_states = self.kit.track_states.borrow();
        let globals = self.kit.kit_globals.borrow();
        let Some(row) = track_states.get(state_idx) else {
            return;
        };
        let track_count = as_array_or_die(self.dc.json_get("tracks")).len();

        self.is_updating_misc.set(true);
        for (control_idx, control) in self.misc_controls.iter().enumerate() {
            match control {
                MiscControl::TrackCombo { blob_offset, combo, .. } => {
                    // 255 is the stored OFF. Out-of-range values and self-references also read as OFF (matches the firmware).
                    let stored = row.get(*blob_offset).copied().unwrap_or(255) as usize;
                    let active = if stored >= track_count || stored == state_idx {
                        0
                    } else {
                        stored as u32 + 1
                    };
                    combo.set_active(Some(active));
                }
                MiscControl::Toggle { blob_offset, btn } => {
                    btn.set_active(row.get(*blob_offset).copied().unwrap_or(0) != 0);
                }
                // A `SharedSpinner`'s byte repeats in every track's blob, so the current row is as good as any.
                MiscControl::Spinner {
                    blob_offset,
                    display_offset,
                    spin,
                }
                | MiscControl::SharedSpinner {
                    blob_offset,
                    display_offset,
                    spin,
                } => {
                    spin.set_value(f64::from(row.get(*blob_offset).copied().unwrap_or(0)) + f64::from(*display_offset));
                }
                MiscControl::MaskedCombo {
                    blob_offset,
                    mask,
                    values,
                    combo,
                } => {
                    let masked = row.get(*blob_offset).copied().unwrap_or(0) & mask;
                    let active = values.iter().position(|&value| value & mask == masked).unwrap_or(0);
                    combo.set_active(Some(active as u32));
                }
                MiscControl::AssignSlot {
                    page_idx,
                    param_idx,
                    range_idx,
                    page_combo,
                    param_combo,
                    depth_spin,
                } => {
                    let page = (row.get(*page_idx).copied().unwrap_or(0) as usize).min(self.assign_dest_names.len().saturating_sub(1));
                    page_combo.set_active(Some(page as u32));
                    self.refill_assign_param_combo(control_idx, page, state_idx);
                    param_combo.set_active(Some(u32::from(row.get(*param_idx).copied().unwrap_or(0))));
                    depth_spin.set_value(f64::from(row.get(*range_idx).copied().unwrap_or(0) as i8));
                }
                MiscControl::GlobalCombo { global_idx, combo } => {
                    combo.set_active(Some(u32::from(globals.get(*global_idx).copied().unwrap_or(0))));
                }
                MiscControl::GlobalSpinner {
                    global_idx,
                    display_offset,
                    spin,
                } => {
                    spin.set_value(f64::from(globals.get(*global_idx).copied().unwrap_or(0)) + f64::from(*display_offset));
                }
            }
        }
        self.is_updating_misc.set(false);
    }

    /// Rebuilds one `AssignSlot`'s Param dropdown for the given dest page (page 1 = SYNT, which shows the machine's own param names).
    fn refill_assign_param_combo(&self, control_idx: usize, page_idx: usize, state_idx: usize) {
        let Some(MiscControl::AssignSlot { param_combo, .. }) = self.misc_controls.get(control_idx) else {
            return;
        };
        let mut names = self.assign_dest_names.get(page_idx).cloned().unwrap_or_default();
        if page_idx == 1 {
            let model = self.kit.machine_models.borrow().get(&state_idx).copied();
            if let Some(machine_names) = model.and_then(|machine_id| self.machine_param_names.get(&machine_id)) {
                for (i, name) in names.iter_mut().enumerate() {
                    if let Some(machine_name) = machine_names.get(i)
                        && !machine_name.is_empty()
                    {
                        name.clone_from(machine_name);
                    }
                }
            }
        }
        param_combo.remove_all();
        for name in &names {
            param_combo.append_text(name);
        }
    }

    /// Updates the options of the LFO DEST combos based on the current LFO PAGE selection.
    fn update_lfo_dest_links(&self) {
        for (page_cb, dest_cb) in &self.lfo_dest_links {
            let page_idx = page_cb.value() as usize;
            let mut names = self.assign_dest_names.get(page_idx).cloned().unwrap_or_default();
            if Some(page_idx) == self.dynamic_machine_assign_idx {
                let state_idx = self.track_combo.active().unwrap_or(0) as usize;
                let model = self.kit.machine_models.borrow().get(&state_idx).copied();
                if let Some(machine_names) = model.and_then(|machine_id| self.machine_param_names.get(&machine_id)) {
                    for (i, name) in names.iter_mut().enumerate() {
                        if let Some(machine_name) = machine_names.get(i)
                            && !machine_name.is_empty()
                        {
                            name.clone_from(machine_name);
                        }
                    }
                }
            }
            let options: Vec<&str> = names.iter().map(std::string::String::as_str).collect();
            dest_cb.set_options(&options);
        }
    }

    /// Handles a Miscellaneous dropdown change (track combos, masked combos, and global combos).
    fn on_misc_combo_changed(&mut self, control_idx: usize, sel_idx: u32) {
        enum MiscWriteTarget {
            Track { blob_offset: usize, cmd: Option<u8> },
            Masked { blob_offset: usize, mask: u8, value: u8 },
            Global { global_idx: usize },
        }
        let target = match self.misc_controls.get(control_idx) {
            Some(MiscControl::TrackCombo { blob_offset, cmd, .. }) => MiscWriteTarget::Track {
                blob_offset: *blob_offset,
                cmd: *cmd,
            },
            Some(MiscControl::MaskedCombo {
                blob_offset, mask, values, ..
            }) => MiscWriteTarget::Masked {
                blob_offset: *blob_offset,
                mask: *mask,
                value: values[sel_idx as usize],
            },
            Some(MiscControl::GlobalCombo { global_idx, .. }) => MiscWriteTarget::Global { global_idx: *global_idx },
            _ => return,
        };

        let track_selection = self.get_track_selection();
        match target {
            MiscWriteTarget::Track { blob_offset, cmd } => {
                if track_selection.is_track_all {
                    return;
                }
                let stored: u8 = if sel_idx == 0 { 255 } else { (sel_idx - 1) as u8 };
                if let Some(row) = self.kit.track_states.borrow_mut().get_mut(track_selection.track_idx)
                    && blob_offset < row.len()
                {
                    row[blob_offset] = stored;
                }
                match cmd {
                    // Dedicated command: live-send.
                    // OFF is stored as 255 but sent as 127 (7-bit wire_val, since the device reads any value > 15 as OFF).
                    Some(cmd) => {
                        if let Some(port) = (self.get_midi_rc)()
                            && is_valid_port(&port)
                        {
                            let wire_val = if stored > 15 { 127 } else { stored };
                            send_sysex(
                                &port,
                                &build_elektron_sysex(self.hw.prod, 0, &[cmd, track_selection.track_idx as u8 & 0x7F, wire_val]),
                            );
                        }
                    }
                    None => self.schedule_misc_sync(),
                }
            }
            MiscWriteTarget::Masked { blob_offset, mask, value } => {
                if track_selection.is_track_all {
                    return;
                }
                if let Some(row) = self.kit.track_states.borrow_mut().get_mut(track_selection.track_idx)
                    && blob_offset < row.len()
                {
                    row[blob_offset] = (row[blob_offset] & !mask) | (value & mask);
                }
                self.schedule_misc_sync();
            }
            MiscWriteTarget::Global { global_idx } => {
                {
                    let mut globals = self.kit.kit_globals.borrow_mut();
                    if global_idx < globals.len() {
                        globals[global_idx] = sel_idx as u8;
                    }
                }
                self.schedule_misc_sync();
            }
        }
        self.set_kit_dirty(true);
    }

    /// Handles a Miscellaneous flag toggle.
    fn on_misc_toggle_changed(&mut self, control_idx: usize, is_active: bool) {
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            return;
        }
        let Some(MiscControl::Toggle { blob_offset, .. }) = self.misc_controls.get(control_idx) else {
            return;
        };
        let blob_offset = *blob_offset;
        if let Some(row) = self.kit.track_states.borrow_mut().get_mut(track_selection.track_idx)
            && blob_offset < row.len()
        {
            row[blob_offset] = u8::from(is_active);
        }
        self.set_kit_dirty(true);
        self.schedule_misc_sync();
    }

    /// Handles a Miscellaneous spinner change (per-track, shared, and kit-global spinners).
    fn on_misc_spin_changed(&mut self, control_idx: usize, display_val: f64) {
        enum MiscWriteTarget {
            Track { blob_offset: usize },
            Shared { blob_offset: usize },
            Global { global_idx: usize },
        }
        let (target, display_offset) = match self.misc_controls.get(control_idx) {
            Some(MiscControl::Spinner {
                blob_offset,
                display_offset,
                ..
            }) => (MiscWriteTarget::Track { blob_offset: *blob_offset }, *display_offset),
            Some(MiscControl::SharedSpinner {
                blob_offset,
                display_offset,
                ..
            }) => (MiscWriteTarget::Shared { blob_offset: *blob_offset }, *display_offset),
            Some(MiscControl::GlobalSpinner {
                global_idx,
                display_offset,
                ..
            }) => (MiscWriteTarget::Global { global_idx: *global_idx }, *display_offset),
            _ => return,
        };
        let stored = ((display_val as i32) - display_offset).clamp(0, 127) as u8;

        let track_selection = self.get_track_selection();
        match target {
            MiscWriteTarget::Track { blob_offset } => {
                if track_selection.is_track_all {
                    return;
                }
                if let Some(row) = self.kit.track_states.borrow_mut().get_mut(track_selection.track_idx)
                    && blob_offset < row.len()
                {
                    row[blob_offset] = stored;
                }
            }
            // Shared bytes repeat in every track's blob, so all rows update together.
            MiscWriteTarget::Shared { blob_offset } => {
                for row in self.kit.track_states.borrow_mut().iter_mut() {
                    if blob_offset < row.len() {
                        row[blob_offset] = stored;
                    }
                }
            }
            MiscWriteTarget::Global { global_idx } => {
                let mut globals = self.kit.kit_globals.borrow_mut();
                if global_idx < globals.len() {
                    globals[global_idx] = stored;
                }
            }
        }
        self.set_kit_dirty(true);
        self.schedule_misc_sync();
    }

    /// Stores an assign Page edit, refreshes the Param names for the new page, and schedules the workspace sync.
    fn on_assign_page_changed(&mut self, control_idx: usize, sel_idx: u32) {
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            return;
        }
        let Some(MiscControl::AssignSlot { page_idx, param_idx, .. }) = self.misc_controls.get(control_idx) else {
            return;
        };
        let (page_idx, param_idx) = (*page_idx, *param_idx);

        if let Some(row) = self.kit.track_states.borrow_mut().get_mut(track_selection.track_idx)
            && page_idx < row.len()
        {
            row[page_idx] = sel_idx as u8;
        }

        // Refilling the Param dropdown fires its changed signal. The guard keeps it from re-entering the handler's machine_id borrow.
        self.is_updating_misc.set(true);
        self.refill_assign_param_combo(control_idx, sel_idx as usize, track_selection.track_idx);
        let param = self
            .kit
            .track_states
            .borrow()
            .get(track_selection.track_idx)
            .and_then(|row| row.get(param_idx))
            .copied()
            .unwrap_or(0);
        if let Some(MiscControl::AssignSlot { param_combo, .. }) = self.misc_controls.get(control_idx) {
            param_combo.set_active(Some(u32::from(param)));
        }

        self.is_updating_misc.set(false);
        self.set_kit_dirty(true);
        self.schedule_misc_sync();
    }

    /// Stores an assign Param edit and schedules the workspace sync.
    fn on_assign_param_changed(&mut self, control_idx: usize, sel_idx: u32) {
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            return;
        }
        let Some(MiscControl::AssignSlot { param_idx, .. }) = self.misc_controls.get(control_idx) else {
            return;
        };
        let param_idx = *param_idx;
        if let Some(row) = self.kit.track_states.borrow_mut().get_mut(track_selection.track_idx)
            && param_idx < row.len()
        {
            row[param_idx] = sel_idx as u8;
        }
        self.set_kit_dirty(true);
        self.schedule_misc_sync();
    }

    /// Stores an assign Depth edit (shown -64..+63, stored as two's-complement int8) and schedules the workspace sync.
    fn on_assign_depth_changed(&mut self, control_idx: usize, display_val: f64) {
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            return;
        }
        let Some(MiscControl::AssignSlot { range_idx, .. }) = self.misc_controls.get(control_idx) else {
            return;
        };
        let range_idx = *range_idx;
        let stored = (display_val as i32).clamp(-64, 63) as i8 as u8;
        if let Some(row) = self.kit.track_states.borrow_mut().get_mut(track_selection.track_idx)
            && range_idx < row.len()
        {
            row[range_idx] = stored;
        }
        self.set_kit_dirty(true);
        self.schedule_misc_sync();
    }

    /// Gathers what the misc controls own and hands the debounced workspace patch to the kit manager.
    ///
    /// The manager decides how (and whether) those bytes can reach this device.
    fn schedule_misc_sync(&self) {
        let track_selection = self.get_track_selection();
        if track_selection.is_track_all {
            return;
        }
        let mut owned_offsets: Vec<usize> = Vec::new();
        let mut shared_params: Vec<(usize, u8)> = Vec::new();
        let mut should_include_globals = false;

        {
            let track_states = self.kit.track_states.borrow();
            let row = track_states.get(track_selection.track_idx).map_or(&[][..], std::vec::Vec::as_slice);
            for control in &self.misc_controls {
                match control {
                    MiscControl::TrackCombo {
                        blob_offset, cmd: None, ..
                    }
                    | MiscControl::Toggle { blob_offset, .. }
                    | MiscControl::Spinner { blob_offset, .. }
                    | MiscControl::MaskedCombo { blob_offset, .. } => owned_offsets.push(*blob_offset),
                    MiscControl::TrackCombo { .. } => {}
                    MiscControl::AssignSlot {
                        page_idx,
                        param_idx,
                        range_idx,
                        ..
                    } => owned_offsets.extend([*page_idx, *param_idx, *range_idx]),
                    MiscControl::SharedSpinner { blob_offset, .. } => {
                        shared_params.push((*blob_offset, row.get(*blob_offset).copied().unwrap_or(0)));
                    }
                    MiscControl::GlobalCombo { .. } | MiscControl::GlobalSpinner { .. } => should_include_globals = true,
                }
            }
        }

        self.kit.schedule_misc_sync(
            (self.get_midi_rc)(),
            MiscSyncRequest {
                track_idx: track_selection.track_idx,
                owned_offsets,
                shared_params,
                should_include_globals,
            },
        );
    }
}
