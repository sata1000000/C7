//! Manage device kits, patterns, songs, samples, and globals.
//!
//! Batch fetching, writing, and local backup/restore.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use gio::prelude::*;
use glib::subclass::prelude::*;
use gtk4::{
    self, Align, Box as GtkBox, Button, ColumnView, ColumnViewColumn, Label, MenuButton, Orientation, Popover, ScrolledWindow, Separator,
    SignalListItemFactory, SingleSelection, gdk, glib,
};
use libadwaita::prelude::*;

use crate::ui::base_module::{BaseModule, StatusFn, listen, make_file_dialog};
use crate::ui::file_checks::{does_file_type_match, verify_c7_item, verify_device_match};
use crate::ui::widgets::ChooseMultiPill;
use c7_core::c7_file_interfacing::{
    C7Data, C7Item, find_c7_item, generate_c7_header, generate_export_filename, md_kit_used_sample_slots, read_c7_or_sysex_file,
    section_to_wire_slot, wire_slot_to_section, write_c7_file,
};
use c7_core::device_config::{DeviceConfig, get_base_channel, read_settings, write_settings};
use c7_core::digipro::digipro_name;
use c7_core::midi::{
    find_input_port, is_valid_port, open_large_sysex_input, run_midi_output, run_midi_session, send_large_sysex, send_sysex,
};
use c7_core::pattern::blank_md_pattern;
use c7_core::sds::{
    build_sds_sample_packets, decode_flac_b64_to_sds, fetch_and_compress_sample, fetch_md_sample_names, probe_sds_slot,
    send_sds_sample_packets,
};
use c7_core::sysex::{
    ELEKTRON_TYPE_BYTE, build_elektron_sysex, extract_sysex_name, fetch_elektron_slot, is_elektron_sysex, is_empty_pattern,
    parse_sysex_file, patch_elektron_original_position, pattern_slot_label, update_elektron_checksum,
};
use c7_core::utils::{JsonPath, as_u64_or, as_u64_or_die, cap_first_char, now_compact_timestamp};

/// One undo snapshot.
///
/// Holds (slot blobs, selected slot indices, slot names).
type UndoSnapshot = (Vec<Option<Vec<u8>>>, HashSet<usize>, Vec<String>);

/// The column order used for whole-column iteration.
const ALL_ITEM_TYPES: [&str; 6] = ["kit", "pattern", "song", "sample", "digipro", "global"];

/// Placeholder name for a slot that hasn't been fetched from the device yet.
const UNFETCHED: &str = "--";

static CSS_ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Registers shared CSS providers for the Librarian UI if not already loaded.
fn ensure_librarian_css() {
    CSS_ONCE.get_or_init(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(
            ".dirty-row { background-color: rgba(255, 235, 59, 0.15); } \
             .rich-list row .librarian-actions { opacity: 0; } \
             .rich-list row:hover .librarian-actions { opacity: 1.0; }",
        );
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(&display, &provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
}

mod imp {
    use super::{Cell, RefCell, glib};
    use glib::prelude::*;
    use glib::subclass::prelude::*;

    /// Row model for a single Librarian slot (kit/pattern/song/sample/global).
    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::LibrarianItem)]
    pub struct LibrarianItem {
        #[property(get, set)]
        pub id_label: RefCell<String>,
        #[property(get, set)]
        pub name: RefCell<String>,
        #[property(get, set)]
        pub dirty: Cell<bool>,
        #[property(get, set)]
        pub position: Cell<u32>,
        pub dirty_h: RefCell<Option<glib::SignalHandlerId>>,
        pub name_h: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LibrarianItem {
        const NAME: &'static str = "LibrarianItem";
        type Type = super::LibrarianItem;
    }

    impl ObjectImpl for LibrarianItem {
        /// Returns the list of properties for the `LibrarianItem` object.
        fn properties() -> &'static [glib::ParamSpec] {
            Self::derived_properties()
        }
        /// Sets the value of a property on the `LibrarianItem` object.
        fn set_property(&self, id: usize, val: &glib::Value, pspec: &glib::ParamSpec) {
            self.derived_set_property(id, val, pspec);
        }
        /// Retrieves the value of a property from the `LibrarianItem` object.
        fn property(&self, id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            self.derived_property(id, pspec)
        }
    }
}

glib::wrapper! {
    /// Row model for a single Librarian slot (kit/pattern/song/sample/global).
    pub struct LibrarianItem(ObjectSubclass<imp::LibrarianItem>);
}

impl LibrarianItem {
    /// Initializes a librarian row item with display labels and state tracking.
    pub fn new(id_label: &str, name: &str) -> Self {
        glib::Object::builder()
            .property("id-label", id_label)
            .property("name", name)
            .property("dirty", false)
            .build()
    }
}

struct ColumnCtx {
    // Per-item-type in-memory state: blobs hold raw SysEx bytes (or `None`/str for samples).
    //
    // `Arc<Mutex<>>` allows background fetch/write threads to write slot-by-slot without holding the whole `LibrarianState` lock.
    // Only the individual column's data is locked.
    blobs: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
    original_blobs: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
    dirty: Arc<Mutex<HashSet<usize>>>,
    undo: Vec<UndoSnapshot>,

    first_slot: usize,
    slot_count: usize,
    display_offset: usize,
    request_cmd: u8,
    dump_cmd: u8,

    model: gio::ListStore,
    btn_fetch: Button,
    btn_write: Button,
    btn_undo: Button,
    vbox: GtkBox,
}

struct LibrarianState {
    dc: Arc<DeviceConfig>,

    /// Per-item-type `write_arm_type` byte from the device JSON, which `verify_c7_item()` checks SysEx command bytes against.
    write_arm_types: HashMap<String, u8>,
    /// Per-item-type column state, keyed by item type ("kit", "pattern", "song", …).
    cols: HashMap<String, ColumnCtx>,

    /// Ordered keys of columns the device actually has (count>0), so a visibility-pill index selects a column.
    column_keys: Vec<String>,

    btn_listen: Button,

    get_midi_rc: Rc<dyn Fn() -> Option<String>>,

    is_active: Arc<std::sync::atomic::AtomicBool>,
    cancel_listen: Arc<std::sync::atomic::AtomicBool>,
    listening_active: Arc<std::sync::atomic::AtomicBool>,
    listen_count: Arc<std::sync::atomic::AtomicUsize>,
    pending_unused_check: Arc<std::sync::atomic::AtomicBool>,

    /// Kept as a widget so file dialogs can anchor to the window via `.root()`.
    ///
    /// Status text is written through `update_status()`, not `set_label()` on this widget.
    status_label: Label,
    update_status: StatusFn,
    progress_bar: gtk4::ProgressBar,
    /// Channel sender for routing live SysEx from the listen thread to the main thread.
    listen_tx: Option<async_channel::Sender<Vec<u8>>>,
}

/// Feature screen for device data management.
pub(crate) struct LibrarianScreen {
    pub root: gtk4::Box,
}

impl LibrarianScreen {
    /// Initializes the Librarian screen.
    pub(crate) fn new(
        go_to_menu: impl Fn() + 'static,
        get_selected_midi: impl Fn() -> Option<String> + 'static,
        active_config: &str,
    ) -> Self {
        ensure_librarian_css();
        let mut base = BaseModule::new();
        let root = base.root.clone();
        let get_midi_rc = Rc::new(get_selected_midi);

        let dc = Arc::new(DeviceConfig::find_by_path_or_die(active_config));

        // Every column's slot count, first slot, display offset, and command bytes live under `sysex_api`.<item_type>.
        let parse_col = |key: &str| -> (usize, usize, usize, u8, u8) {
            (
                as_u64_or(dc.json_get(&format!("sysex_api.{key}.first_slot")), 0) as usize,
                as_u64_or(dc.json_get(&format!("sysex_api.{key}.slots")), 0) as usize,
                as_u64_or(dc.json_get(&format!("sysex_api.{key}.display_offset")), 0) as usize,
                as_u64_or(dc.json_get(&format!("sysex_api.{key}.request_cmd")), 0) as u8,
                as_u64_or(dc.json_get(&format!("sysex_api.{key}.write_cmd")), 0) as u8,
            )
        };

        let (kit_first, kit_count, kit_offset, kit_request, kit_dump) = parse_col("kit");
        let (pattern_first, pattern_count, pattern_offset, pattern_request, pattern_dump) = parse_col("pattern");
        let (song_first, song_count, song_offset, song_request, song_dump) = parse_col("song");
        let (sample_first, sample_count, sample_offset, sample_request, sample_dump) = parse_col("sample");
        let (digipro_first, digipro_count, digipro_offset, digipro_request, digipro_dump) = parse_col("digipro");
        let (global_first, global_count, global_offset, global_request, global_dump) = parse_col("global");

        /*
        Every sysex_api.<item_type> section is OPTIONAL.
        A device may omit any of them (like the Sidstation), so this must read defensively and skip absent sections.
        Do NOT "simplify" this to a direct index/unwrap; that crashes such devices.
        */

        let mut write_arm_types = HashMap::new();
        for item_type in ALL_ITEM_TYPES {
            if let Some(write_arm_type) = dc
                .json_get(&format!("sysex_api.{item_type}.write_arm_type"))
                .and_then(serde_json::Value::as_u64)
            {
                write_arm_types.insert(item_type.to_string(), write_arm_type as u8);
            }
        }

        base.build_header("Librarian", go_to_menu);

        let main_hbox = GtkBox::new(Orientation::Horizontal, 24);
        main_hbox.set_margin_top(16);
        main_hbox.set_margin_bottom(16);
        main_hbox.set_margin_start(16);
        main_hbox.set_margin_end(16);

        let state_rc: Rc<RefCell<Option<LibrarianState>>> = Rc::new(RefCell::new(None));
        let mut cols = HashMap::new();

        let mut build_col = |title: &str,
                             fetch_label: &str,
                             write_label: &str,
                             key: &str,
                             first: usize,
                             count: usize,
                             display_offset: usize,
                             request_cmd: u8,
                             dump_cmd: u8| {
            let vbox = GtkBox::new(Orientation::Vertical, 8);
            vbox.set_hexpand(true);
            let header = GtkBox::new(Orientation::Horizontal, 8);
            let title_label = Label::builder()
                .label(title)
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["title-3"])
                .build();
            let btn_undo = Button::builder()
                .icon_name("edit-undo-symbolic")
                .tooltip_text("Revert all changes")
                .css_classes(["flat"])
                .opacity(0.0)
                .sensitive(false)
                .build();
            {
                let state_rc_c = Rc::clone(&state_rc);
                let key_str = key.to_string();
                btn_undo.connect_clicked(move |_| {
                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                        state_mut.on_undo(&key_str);
                    }
                });
            }
            header.append(&title_label);
            header.append(&btn_undo);
            vbox.append(&header);

            let model = gio::ListStore::new::<LibrarianItem>();
            let selection = SingleSelection::builder()
                .model(&model)
                .autoselect(false)
                .can_unselect(true)
                .build();
            let column_view = ColumnView::builder()
                .model(&selection)
                .css_classes(["rich-list"])
                .show_row_separators(false)
                .show_column_separators(false)
                .reorderable(false)
                .build();

            // Column 1: drag handle + slot id label.
            let id_factory = SignalListItemFactory::new();
            let key_str = key.to_string();
            let state_rc_c = Rc::clone(&state_rc);
            id_factory.connect_setup(move |_, obj| {
                let list_item = obj.downcast_ref::<gtk4::ListItem>().unwrap();
                let hbox = GtkBox::new(Orientation::Horizontal, 8);
                hbox.set_margin_top(4);
                hbox.set_margin_bottom(4);
                hbox.set_margin_start(8);
                hbox.set_margin_end(4);
                let handle = gtk4::Image::from_icon_name("list-drag-handle-symbolic");
                handle.add_css_class("dim-label");
                let label = Label::builder().xalign(0.0).css_classes(["dim-label", "monospace"]).build();
                hbox.append(&handle);
                hbox.append(&label);

                let item_expr = gtk4::PropertyExpression::new(gtk4::ListItem::static_type(), gtk4::Expression::NONE, "item");
                let id_expr = gtk4::PropertyExpression::new(LibrarianItem::static_type(), Some(&item_expr), "id-label");
                id_expr.bind(&label, "label", Some(list_item));

                let drag_source = gtk4::DragSource::new();
                drag_source.set_actions(gdk::DragAction::MOVE);
                let list_item_ds = list_item.clone();

                drag_source.connect_prepare(move |_, _, _| {
                    let position = list_item_ds.position();
                    if position == gtk4::INVALID_LIST_POSITION {
                        return None;
                    }
                    Some(gdk::ContentProvider::for_value(&(position as i32).to_value()))
                });

                {
                    let hbox = hbox.clone();
                    drag_source.connect_drag_begin(move |_, _| hbox.set_opacity(0.4));
                }
                {
                    let hbox = hbox.clone();
                    drag_source.connect_drag_end(move |_, _, _| hbox.set_opacity(1.0));
                }
                hbox.add_controller(drag_source);

                let drop_target = gtk4::DropTarget::new(glib::Type::I32, gdk::DragAction::MOVE);
                {
                    let key_str = key_str.clone();
                    let state_rc_c2 = Rc::clone(&state_rc_c);
                    let list_item_dt = list_item.clone();
                    drop_target.connect_drop(move |_, val, _, _| {
                        let src = val.get::<i32>().unwrap() as usize;
                        let dst = list_item_dt.position();
                        if src != dst as usize
                            && dst != gtk4::INVALID_LIST_POSITION
                            && let Some(state_mut) = state_rc_c2.borrow_mut().as_mut()
                        {
                            state_mut.shift_rows(&key_str, src, dst as usize);
                        }
                        true
                    });
                }
                {
                    let hbox = hbox.clone();
                    drop_target.connect_enter(move |_, _, _| {
                        hbox.add_css_class("activatable");
                        gdk::DragAction::MOVE
                    });
                }
                {
                    let hbox = hbox.clone();
                    drop_target.connect_leave(move |_| hbox.remove_css_class("activatable"));
                }
                hbox.add_controller(drop_target);

                list_item.set_child(Some(&hbox));
            });

            id_factory.connect_bind(move |_, obj| {
                let list_item = obj.downcast_ref::<gtk4::ListItem>().unwrap();
                if let Some(item) = list_item.item().and_downcast::<LibrarianItem>() {
                    let child = list_item.child().unwrap();
                    // Update the position property for this slot.
                    item.set_position(list_item.position());

                    let row_box = child.parent().and_then(|p| p.parent()).unwrap();
                    let apply = move |dirty: bool| {
                        if dirty {
                            row_box.add_css_class("dirty-row");
                        } else {
                            row_box.remove_css_class("dirty-row");
                        }
                    };
                    apply(item.dirty());
                    let handle = item.connect_notify_local(Some("dirty"), move |item, _| apply(item.dirty()));
                    *item.imp().dirty_h.borrow_mut() = Some(handle);
                }
            });
            id_factory.connect_unbind(move |_, obj| {
                let list_item = obj.downcast_ref::<gtk4::ListItem>().unwrap();
                let child = list_item.child().unwrap();
                let row_box = child.parent().and_then(|p| p.parent()).unwrap();
                row_box.remove_css_class("dirty-row");
                if let Some(item) = list_item.item().and_downcast::<LibrarianItem>()
                    && let Some(handle) = item.imp().dirty_h.borrow_mut().take()
                {
                    item.disconnect(handle);
                }
            });
            let id_col = ColumnViewColumn::new(None, Some(id_factory));
            id_col.set_resizable(false);
            column_view.append_column(&id_col);

            let name_factory = SignalListItemFactory::new();
            let key_str = key.to_string();
            let state_rc_c = Rc::clone(&state_rc);
            name_factory.connect_setup(move |_, obj| {
                let list_item = obj.downcast_ref::<gtk4::ListItem>().unwrap();
                let label = Label::builder()
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .margin_top(4)
                    .margin_bottom(4)
                    .margin_start(4)
                    .margin_end(8)
                    .build();
                let popover = Popover::builder().has_arrow(false).build();
                let actions_box = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .margin_top(4)
                    .margin_bottom(4)
                    .margin_start(4)
                    .margin_end(4)
                    .build();

                let build_btn = |icon: &str, text: &str, css: Option<&str>| -> Button {
                    let btn = Button::builder().halign(Align::Fill).build();
                    btn.add_css_class("flat");
                    if let Some(css) = css {
                        btn.add_css_class(css);
                    }
                    let row_box = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(8)
                        .margin_start(4)
                        .margin_end(8)
                        .margin_top(2)
                        .margin_bottom(2)
                        .build();
                    let image = gtk4::Image::from_icon_name(icon);
                    let label = Label::builder().label(text).xalign(0.0).build();
                    row_box.append(&image);
                    row_box.append(&label);
                    btn.set_child(Some(&row_box));
                    btn
                };

                let mut btn_load = None;
                let mut btn_rename = None;
                let item_type_cap = key_str.to_uppercase();
                if key_str != "global" {
                    btn_load = Some(build_btn(
                        "media-playback-start-symbolic",
                        &format!("Load {item_type_cap} on Device"),
                        None,
                    ));
                    actions_box.append(btn_load.as_ref().unwrap());
                }
                let btn_import = build_btn("document-open-symbolic", &format!("Import {item_type_cap}"), None);
                actions_box.append(&btn_import);
                let btn_export = build_btn("document-save-symbolic", &format!("Export {item_type_cap}"), None);
                actions_box.append(&btn_export);
                if key_str != "pattern" && key_str != "global" {
                    btn_rename = Some(build_btn("document-edit-symbolic", "Rename", None));
                    actions_box.append(btn_rename.as_ref().unwrap());
                }
                let btn_write_single = build_btn(
                    "document-send-symbolic",
                    &format!("Write {} to Device", cap_first_char(&key_str)),
                    Some("suggested-action"),
                );
                btn_write_single.set_visible(false);
                actions_box.append(&btn_write_single);
                actions_box.append(&Separator::new(Orientation::Horizontal));
                let btn_remove_ui = build_btn("edit-delete-symbolic", "Remove from UI", Some("destructive-action"));
                actions_box.append(&btn_remove_ui);
                let btn_delete_device = build_btn("edit-delete-symbolic", "Delete from Device", Some("destructive-action"));
                if key_str == "sample" || key_str == "digipro" || key_str == "global" {
                    btn_delete_device.set_sensitive(false);
                    btn_delete_device.set_tooltip_text(Some("Not supported for this data type"));
                }
                actions_box.append(&btn_delete_device);
                popover.set_child(Some(&actions_box));

                let menu_btn = MenuButton::builder()
                    .icon_name("view-more-symbolic")
                    .css_classes(["flat", "librarian-actions"])
                    .halign(Align::End)
                    .valign(Align::Center)
                    .margin_end(4)
                    .always_show_arrow(false)
                    .popover(&popover)
                    .build();
                let overlay = gtk4::Overlay::new();
                overlay.set_child(Some(&label));
                overlay.add_overlay(&menu_btn);
                overlay.set_measure_overlay(&menu_btn, false);

                let item_expr = gtk4::PropertyExpression::new(gtk4::ListItem::static_type(), gtk4::Expression::NONE, "item");
                let name_expr = gtk4::PropertyExpression::new(LibrarianItem::static_type(), Some(&item_expr), "name");
                name_expr.bind(&label, "label", Some(list_item));

                let dirty_expr = gtk4::PropertyExpression::new(LibrarianItem::static_type(), Some(&item_expr), "dirty");
                dirty_expr.bind(&btn_write_single, "visible", Some(list_item));

                let connect = |btn: &Button,
                               action: &'static str,
                               state_rc_c: Rc<RefCell<Option<LibrarianState>>>,
                               key_str: String,
                               list_item_c: gtk4::ListItem,
                               popover_c: Popover| {
                    btn.connect_clicked(move |_| {
                        popover_c.popdown();
                        let idx = list_item_c.position() as usize;
                        let state_rc_c2 = Rc::clone(&state_rc_c);
                        if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                            state_mut.dispatch_action(action, idx, &key_str, state_rc_c2);
                        }
                    });
                };

                if let Some(btn_load) = btn_load {
                    connect(
                        &btn_load,
                        "load",
                        Rc::clone(&state_rc_c),
                        key_str.clone(),
                        list_item.clone(),
                        popover.clone(),
                    );
                }
                connect(
                    &btn_import,
                    "import",
                    Rc::clone(&state_rc_c),
                    key_str.clone(),
                    list_item.clone(),
                    popover.clone(),
                );
                connect(
                    &btn_export,
                    "export",
                    Rc::clone(&state_rc_c),
                    key_str.clone(),
                    list_item.clone(),
                    popover.clone(),
                );

                if let Some(btn_rename) = btn_rename {
                    connect(
                        &btn_rename,
                        "rename",
                        Rc::clone(&state_rc_c),
                        key_str.clone(),
                        list_item.clone(),
                        popover.clone(),
                    );
                }
                connect(
                    &btn_write_single,
                    "write_single",
                    Rc::clone(&state_rc_c),
                    key_str.clone(),
                    list_item.clone(),
                    popover.clone(),
                );
                connect(
                    &btn_remove_ui,
                    "delete",
                    Rc::clone(&state_rc_c),
                    key_str.clone(),
                    list_item.clone(),
                    popover.clone(),
                );
                connect(
                    &btn_delete_device,
                    "delete_device",
                    Rc::clone(&state_rc_c),
                    key_str.clone(),
                    list_item.clone(),
                    popover,
                );

                list_item.set_child(Some(&overlay));
            });
            name_factory.connect_bind(move |_, obj| {
                let list_item = obj.downcast_ref::<gtk4::ListItem>().unwrap();
                if let Some(item) = list_item.item().and_downcast::<LibrarianItem>() {
                    let overlay = list_item.child().and_downcast::<gtk4::Overlay>().unwrap();
                    let label = overlay.child().and_downcast::<Label>().unwrap();
                    let apply = move |name: String| {
                        if name == UNFETCHED {
                            label.add_css_class("dim-label");
                        } else {
                            label.remove_css_class("dim-label");
                        }
                    };
                    apply(item.name());
                    let handle = item.connect_notify_local(Some("name"), move |item, _| apply(item.name()));
                    *item.imp().name_h.borrow_mut() = Some(handle);
                }
            });
            name_factory.connect_unbind(move |_, obj| {
                let list_item = obj.downcast_ref::<gtk4::ListItem>().unwrap();
                if let Some(item) = list_item.item().and_downcast::<LibrarianItem>()
                    && let Some(handle) = item.imp().name_h.borrow_mut().take()
                {
                    item.disconnect(handle);
                }
            });
            let name_col = ColumnViewColumn::new(None, Some(name_factory));
            name_col.set_expand(true);
            name_col.set_resizable(true);
            column_view.append_column(&name_col);

            if let Some(header) = column_view.first_child() {
                header.set_visible(false);
            }

            let scrolled_window = ScrolledWindow::builder()
                .child(&column_view)
                .vexpand(true)
                .min_content_height(420)
                .build();
            vbox.append(&scrolled_window);

            let btn_fetch = Button::builder().label(fetch_label).css_classes(["suggested-action"]).build();
            {
                let state_rc_c = Rc::clone(&state_rc);
                let key_str = key.to_string();
                btn_fetch.connect_clicked(move |btn| {
                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                        state_mut.on_fetch(btn, &key_str);
                    }
                });
            }
            vbox.append(&btn_fetch);

            let btn_write = Button::builder().label(write_label).sensitive(false).build();
            {
                let state_rc_c = Rc::clone(&state_rc);
                let key_str = key.to_string();
                btn_write.connect_clicked(move |btn| {
                    if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                        state_mut.on_write(btn, &key_str);
                    }
                });
            }
            vbox.append(&btn_write);

            let blobs: Arc<Mutex<Vec<Option<Vec<u8>>>>> = Arc::new(Mutex::new(vec![None; count]));
            let original: Arc<Mutex<Vec<Option<Vec<u8>>>>> = Arc::new(Mutex::new(vec![None; count]));
            let dirty: Arc<Mutex<HashSet<usize>>> = Arc::new(Mutex::new(HashSet::new()));

            cols.insert(
                key.to_string(),
                ColumnCtx {
                    blobs,
                    original_blobs: original,
                    dirty,
                    undo: Vec::new(),
                    first_slot: first,
                    slot_count: count,
                    display_offset,
                    request_cmd,
                    dump_cmd,
                    model,
                    btn_fetch,
                    btn_write,
                    btn_undo,
                    vbox: vbox.clone(),
                },
            );
            vbox
        };

        // Every column is built unconditionally (so `cols[...]` lookups never panic), but shown only when its `sysex_api` section exists.
        // This lets future devices (e.g. a Sidstation) omit features cleanly.
        // The range label is guarded against the count==0 underflow that an absent section would cause.
        let col_title = |name: &str, first: usize, count: usize, display_offset: usize| -> String {
            if count == 0 {
                name.to_string()
            } else {
                format!("{name}  ({:02}-{:02})", first + display_offset, first + count - 1 + display_offset)
            }
        };

        let kit_vbox = build_col(
            &col_title("Kits", kit_first, kit_count, kit_offset),
            "Fetch All Kits",
            "Write Kits to Device",
            "kit",
            kit_first,
            kit_count,
            kit_offset,
            kit_request,
            kit_dump,
        );
        if dc.has_gate("sysex_api.kit") {
            main_hbox.append(&kit_vbox);
        }

        let pattern_title = if pattern_count == 0 {
            "Patterns".to_string()
        } else {
            let first_label = pattern_slot_label(pattern_first);
            let last_label = pattern_slot_label(pattern_first + pattern_count - 1);
            format!("Patterns  ({first_label}-{last_label})")
        };

        let pattern_vbox = build_col(
            &pattern_title,
            "Fetch All Patterns",
            "Write Patterns to Device",
            "pattern",
            pattern_first,
            pattern_count,
            pattern_offset,
            pattern_request,
            pattern_dump,
        );
        if dc.has_gate("sysex_api.pattern") {
            main_hbox.append(&pattern_vbox);
        }

        let song_vbox = build_col(
            &col_title("Songs", song_first, song_count, song_offset),
            "Fetch All Songs",
            "Write Songs to Device",
            "song",
            song_first,
            song_count,
            song_offset,
            song_request,
            song_dump,
        );
        if dc.has_gate("sysex_api.song") {
            main_hbox.append(&song_vbox);
        }

        let sample_vbox = build_col(
            &col_title("Samples", sample_first, sample_count, sample_offset),
            "Fetch All Samples",
            "Write Samples to Device",
            "sample",
            sample_first,
            sample_count,
            sample_offset,
            sample_request,
            sample_dump,
        );
        if dc.has_gate("sysex_api.sample") {
            main_hbox.append(&sample_vbox);
        }

        let digipro_vbox = build_col(
            &col_title("DigiPro", digipro_first, digipro_count, digipro_offset),
            "Fetch All DigiPro",
            "Write DigiPro to Device",
            "digipro",
            digipro_first,
            digipro_count,
            digipro_offset,
            digipro_request,
            digipro_dump,
        );
        if dc.has_gate("sysex_api.digipro") {
            main_hbox.append(&digipro_vbox);
        }

        let global_vbox = build_col(
            &col_title("Globals", global_first, global_count, global_offset),
            "Fetch All Globals",
            "Write Globals to Device",
            "global",
            global_first,
            global_count,
            global_offset,
            global_request,
            global_dump,
        );
        if dc.has_gate("sysex_api.global") {
            main_hbox.append(&global_vbox);
        }

        base.root.append(&main_hbox);

        let backup_hbox = GtkBox::new(Orientation::Horizontal, 8);
        backup_hbox.set_margin_start(16);
        backup_hbox.set_margin_end(16);
        backup_hbox.set_margin_bottom(16);

        let btn_listen = Button::with_label("Listen");
        {
            let state_rc_c = Rc::clone(&state_rc);
            btn_listen.connect_clicked(move |_| {
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.on_listen_clicked();
                }
            });
        }
        backup_hbox.append(&btn_listen);

        let btn_import = Button::with_label("Import Backup (*.c7, *.syx)");
        {
            let state_rc_c = Rc::clone(&state_rc);
            btn_import.connect_clicked(move |_| {
                let state_rc_c2 = Rc::clone(&state_rc_c);
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.on_import_backup(state_rc_c2);
                }
            });
        }
        backup_hbox.append(&btn_import);

        let btn_export = Button::with_label("Export Backup (*.c7)");
        {
            let state_rc_c = Rc::clone(&state_rc);
            btn_export.connect_clicked(move |_| {
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.on_export_backup();
                }
            });
        }
        backup_hbox.append(&btn_export);

        backup_hbox.append(&GtkBox::builder().hexpand(true).build());

        let settings = read_settings();
        let device_shorter = &dc.device_shorter;
        let col_vis = settings
            .get("librarian_columns")
            .and_then(|value| value.json_get(device_shorter))
            .and_then(|value| value.as_object());

        // Only columns whose `sysex_api`.<item_type> section exists get a UI slot + visibility pill.
        let col_meta: [(&str, &str); 6] = [
            ("kit", "Kits"),
            ("pattern", "Patterns"),
            ("song", "Songs"),
            ("sample", "Samples"),
            ("digipro", "DigiPro"),
            ("global", "Globals"),
        ];

        let present_cols: Vec<(&str, &str)> = col_meta
            .iter()
            .filter(|(item_type, _)| dc.has_gate(&format!("sysex_api.{item_type}")))
            .map(|(item_type, label)| (*item_type, *label))
            .collect();

        let column_keys: Vec<String> = present_cols.iter().map(|(item_type, _)| item_type.to_string()).collect();
        let col_labels: Vec<&str> = present_cols.iter().map(|(_, label)| *label).collect();
        let mut action_idx: Vec<usize> = Vec::new();
        for (i, (item_type, _)) in present_cols.iter().enumerate() {
            let is_visible = col_vis
                .and_then(|value| value.get(*item_type))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(*item_type == "kit" || *item_type == "pattern");
            cols.get(*item_type).unwrap().vbox.set_visible(is_visible);
            if is_visible {
                action_idx.push(i);
            }
        }

        let col_pill = ChooseMultiPill::new(&col_labels, &action_idx);
        {
            let state_rc_c = Rc::clone(&state_rc);
            // `connect_changed()` callback is Fn(i32, bool). The two arguments are column index and new toggle state.
            col_pill.connect_changed(move |col_idx, is_visible| {
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.on_column_toggle(col_idx as usize, is_visible);
                }
            });
        }
        backup_hbox.append(col_pill.widget());
        base.root.append(&backup_hbox);

        base.build_status_area(false);

        // Channel for routing live SysEx from the listen thread to the main-thread state.
        let (listen_tx, listen_rx) = async_channel::unbounded::<Vec<u8>>();
        {
            let state_rc_c = Rc::clone(&state_rc);
            listen(listen_rx, move |raw| {
                if let Some(state_mut) = state_rc_c.borrow_mut().as_mut() {
                    state_mut.route_live_message(&raw);
                }
                glib::ControlFlow::Continue
            });

            // Dedicated low-frequency timer for the UI "unused" check.
            // Decoupled from MIDI reception to prevent redundant processing.
            let state_rc_c_timer = Rc::clone(&state_rc);
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if let Some(state_mut) = state_rc_c_timer.borrow_mut().as_mut()
                    && state_mut.pending_unused_check.swap(false, Ordering::Relaxed)
                {
                    state_mut.maybe_mark_unused_samples();
                }
                glib::ControlFlow::Continue
            });
        }

        let mut state_mut = LibrarianState {
            dc: Arc::clone(&dc),
            write_arm_types,
            cols,
            column_keys,
            btn_listen,
            get_midi_rc,
            is_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cancel_listen: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            listening_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            listen_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            pending_unused_check: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            status_label: base.status_label.clone().unwrap(),
            update_status: base.status_updater(),
            progress_bar: base.progress_bar.clone().unwrap(),
            listen_tx: Some(listen_tx),
        };
        state_mut.populate_initial_lists();
        *state_rc.borrow_mut() = Some(state_mut);

        Self { root }
    }

    /// Returns the root widget for embedding in the navigation stack.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

enum FetchUpdate {
    Status(String),
    ItemName(usize, String),
}

enum WriteUpdate {
    Status(String),
    Progress(f64),
    ItemDirty(usize, bool),
}

impl LibrarianState {
    // ── Column Visibility ──────────────────────────────────────────────────────────────────────────────────

    /// Shows or hides a column vbox when its pill button is toggled.
    fn on_column_toggle(&mut self, col_idx: usize, is_visible: bool) {
        // The pill has exactly one entry per present column, so `col_idx` always maps to a real column.
        let key = self.column_keys[col_idx].clone();
        self.cols[&key].vbox.set_visible(is_visible);
        self.save_column_visibility();
    }

    /// Persists column toggle states into `settings.json`, merging with existing keys.
    fn save_column_visibility(&self) {
        let mut settings = read_settings();
        let device_shorter = &self.dc.device_shorter;

        let mut columns_root = settings
            .get("librarian_columns")
            .and_then(|value| value.as_object())
            .cloned()
            .unwrap_or_default();

        let mut device_map = serde_json::Map::new();
        for key in &self.column_keys {
            device_map.insert(key.clone(), serde_json::Value::Bool(self.cols[key].vbox.is_visible()));
        }

        columns_root.insert(device_shorter.clone(), serde_json::Value::Object(device_map));
        settings.insert("librarian_columns".to_string(), serde_json::Value::Object(columns_root));
        write_settings(&settings);
    }

    // ── List population & DnD ──────────────────────────────────────────────────────────────────────────────

    /// Initializes the UI models with placeholder items based on device slot counts.
    fn populate_initial_lists(&mut self) {
        // Iterate in fixed order so list models always get populated. Empty audio columns (count 0) yield no rows.
        for item_type in ALL_ITEM_TYPES {
            let col = &self.cols[item_type];
            let count = col.slot_count;
            let first = col.first_slot;
            let offset = col.display_offset;
            let mut items = Vec::with_capacity(count);
            for idx in 0..count {
                let slot = first + idx;
                let (id_label, name_label) = match item_type {
                    "pattern" => (pattern_slot_label(slot), UNFETCHED.to_string()),
                    _ => (format!("{:02}", slot + offset), UNFETCHED.to_string()),
                };
                items.push(LibrarianItem::new(&id_label, &name_label).upcast::<glib::Object>());
            }
            col.model.splice(0, 0, &items);
        }
    }

    /// Shifts a row to a new index, cascading neighbors.
    ///
    /// Slot ids stay fixed.
    fn shift_rows(&mut self, item_type: &str, src: usize, dst: usize) {
        self.push_undo(item_type);
        let col = self.cols.get_mut(item_type).unwrap();
        {
            let mut blobs = col.blobs.lock().unwrap();
            let original = col.original_blobs.lock().unwrap();
            let mut dirty = col.dirty.lock().unwrap();
            let blob = blobs.remove(src);
            blobs.insert(dst, blob);

            let mut names: Vec<String> = (0..col.model.n_items())
                .map(|i| col.model.item(i).and_downcast::<LibrarianItem>().unwrap().name())
                .collect();
            let name = names.remove(src);
            names.insert(dst, name);

            for i in src.min(dst)..=src.max(dst) {
                let item = col.model.item(i as u32).and_downcast::<LibrarianItem>().unwrap();
                item.set_name(names[i].as_str());
                if blobs[i] == original[i] {
                    dirty.remove(&i);
                } else {
                    dirty.insert(i);
                }
                item.set_dirty(dirty.contains(&i));
            }
        }
        self.refresh_write_btn(item_type);
    }

    // ── Per-row Actions ────────────────────────────────────────────────────────────────────────────────────

    /// Dispatches row-specific actions for a Librarian item.
    fn dispatch_action(&mut self, action: &str, row_idx: usize, item_type: &str, state_rc: Rc<RefCell<Option<LibrarianState>>>) {
        match action {
            "load" => {
                // Send a SysEx 'load slot' command so the device activates that slot.
                let port = (self.get_midi_rc)();
                if !is_valid_port(port.as_deref().unwrap_or("")) {
                    (self.update_status)("Error: Select a valid MIDI port.");
                    return;
                }
                let Some(cmd) = self
                    .dc
                    .json_get(&format!("sysex_api.navigation.load_{item_type}.cmd"))
                    .and_then(serde_json::Value::as_u64)
                else {
                    return;
                };
                let ch = get_base_channel();
                send_sysex(
                    &port.unwrap(),
                    &build_elektron_sysex(self.dc.prod, ch, &[cmd as u8, row_idx as u8 & 0x7F]),
                );
                (self.update_status)(&format!("Status: Loaded {item_type} {row_idx} on device."));
            }
            "write_single" => {
                let btn = self.cols[item_type].btn_write.clone();
                self.on_write(&btn, item_type);
            }
            "delete" => {
                // Clear a slot from local memory without marking it for device write.
                self.push_undo(item_type);
                let col = self.cols.get_mut(item_type).unwrap();
                col.blobs.lock().unwrap()[row_idx] = None;
                col.dirty.lock().unwrap().remove(&row_idx);
                col.model
                    .item(row_idx as u32)
                    .and_downcast::<LibrarianItem>()
                    .unwrap()
                    .set_name("--");
                if row_idx < col.model.n_items() as usize
                    && let Some(item) = col.model.item(row_idx as u32).and_downcast::<LibrarianItem>()
                {
                    item.set_dirty(false);
                }
                self.refresh_write_btn(item_type);
            }
            "delete_device" => self.delete_slot_from_device(row_idx, item_type),
            "rename" => {
                // Show an inline dialog to rename the display label of a slot.
                let current_name = self.cols[item_type]
                    .model
                    .item(row_idx as u32)
                    .and_downcast::<LibrarianItem>()
                    .map(|item| item.name())
                    .unwrap_or_default();
                let initial = if current_name == "--" { String::new() } else { current_name };
                let item_type_cap = cap_first_char(item_type);

                let dialog = libadwaita::AlertDialog::builder()
                    .heading(format!("Rename {item_type_cap}"))
                    .body("Enter a new display name:")
                    .build();
                let entry = gtk4::Entry::builder().text(&initial).build();
                entry.set_margin_start(16);
                entry.set_margin_end(16);
                entry.set_margin_top(4);
                entry.set_margin_bottom(4);
                dialog.set_extra_child(Some(&entry));
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("rename", "Rename");
                dialog.set_default_response(Some("rename"));
                dialog.set_response_appearance("rename", libadwaita::ResponseAppearance::Suggested);

                let item_type_str = item_type.to_string();
                dialog.connect_response(None, move |_, response| {
                    // Apply the new name if the input isn't empty.
                    if response == "rename" {
                        let new_name = entry.text().trim().to_string();
                        if !new_name.is_empty()
                            && let Some(state_mut) = state_rc.borrow_mut().as_mut()
                        {
                            state_mut.push_undo(&item_type_str);
                            if let Some(item) = state_mut
                                .cols
                                .get_mut(&item_type_str)
                                .unwrap()
                                .model
                                .item(row_idx as u32)
                                .and_downcast::<LibrarianItem>()
                            {
                                item.set_name(new_name.as_str());
                            }
                            state_mut
                                .cols
                                .get_mut(&item_type_str)
                                .unwrap()
                                .dirty
                                .lock()
                                .unwrap()
                                .insert(row_idx);
                            state_mut.refresh_write_btn(&item_type_str);
                        }
                    }
                });
                let parent = self.status_label.root().and_downcast::<gtk4::Window>();
                dialog.present(parent.as_ref());
            }
            "export" => {
                // Presents a save dialog to export a single Librarian item to disk.
                let blob_opt = self.cols[item_type].blobs.lock().unwrap()[row_idx].clone();
                let Some(blob) = blob_opt else {
                    (self.update_status)(&format!(
                        "Status: Cannot export: {} {} is empty.",
                        cap_first_char(item_type),
                        row_idx
                    ));
                    return;
                };
                // Reject empty kit/song slots (`0x7F` in the name's first byte).
                // Patterns don't have a name field, so `raw[10]` is data and shouldn't be checked.
                if matches!(item_type, "kit" | "song") && blob.len() > 10 && blob[10] == 0x7F {
                    (self.update_status)(&format!("Status: Cannot export: {} slot is empty.", cap_first_char(item_type)));
                    return;
                }

                let first = self.cols[item_type].first_slot;
                let slot = first + row_idx;
                let slot_label = self.item_type_slot_label(item_type, slot);

                let item_name: Option<String> = match item_type {
                    "kit" => extract_sysex_name(&blob).filter(|name| name.as_str() != "--"),
                    "song" => extract_sysex_name(&blob).filter(|name| name.as_str() != "--"),
                    "sample" => {
                        let item_name_opt = self.cols["sample"]
                            .model
                            .item(row_idx as u32)
                            .and_downcast::<LibrarianItem>()
                            .map(|item| item.name());
                        // Filter out auto-generated placeholder names.
                        item_name_opt.filter(|name| {
                            !(name.starts_with("SAMPLE_") || name.starts_with('S') && name.len() <= 3) && name.as_str() != "--"
                        })
                    }
                    _ => None,
                };

                let filename = generate_export_filename(&self.dc.device_shorter, item_type, &slot_label, item_name.as_deref());
                let dialog = make_file_dialog(&format!("Export {}", cap_first_char(item_type)), Some(&filename));

                let item_type_str = item_type.to_string();
                let dc_c = Arc::clone(&self.dc);
                let update_status = Arc::clone(&self.update_status);
                let port = (self.get_midi_rc)();
                let ch = get_base_channel();
                let parent = self.status_label.root().and_downcast::<gtk4::Window>();

                dialog.save(parent.as_ref(), None::<&gio::Cancellable>, move |result: Result<gio::File, _>| {
                    // Processes the native file dialog response for saving data to disk.
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        let path_str = path.to_string_lossy().into_owned();
                        if item_type_str == "sample" {
                            // Background thread: the slot is fetched and compressed live.
                            let dc_c2 = dc_c.clone();
                            let update_status_c2 = Arc::clone(&update_status);
                            let port_c = port.clone();
                            let slot_label_c = slot_label.clone();
                            glib::spawn_future_local(async move {
                                LibrarianState::export_sample(
                                    row_idx,
                                    path_str,
                                    slot_label_c,
                                    dc_c2,
                                    ch,
                                    slot as u8,
                                    port_c,
                                    update_status_c2,
                                )
                                .await;
                            });
                            return;
                        }
                        let global_meta = generate_c7_header(&item_type_str, &dc_c.device_short, None);
                        let mut attrs = BTreeMap::new();
                        attrs.insert("type".to_string(), item_type_str.clone());
                        attrs.insert("format".to_string(), "sysex".to_string());
                        if let Some(ref name) = item_name {
                            attrs.insert("name".to_string(), name.clone());
                        }
                        let section = wire_slot_to_section(&item_type_str, slot);
                        let item = C7Item {
                            section,
                            data: Some(C7Data::Binary(blob)),
                            attrs,
                        };
                        write_c7_file(&path_str, &[item], Some(&global_meta));
                        let basename = std::path::Path::new(&path_str)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        update_status(&format!("Status: Exported {item_type_str} to {basename}"));
                    }
                });
            }
            "import" => {
                // Presents an open dialog to import a single Librarian item from a `.c7` or `.syx` file.
                let item_type_cap = cap_first_char(item_type);
                let dialog = make_file_dialog(&format!("Import {item_type_cap}"), None);

                let item_type_str = item_type.to_string();
                let device_short = self.dc.device_short.clone();
                let prod = self.dc.prod;
                let first = self.cols[item_type].first_slot;
                let display_offset = self.cols[item_type].display_offset;
                let parent = self.status_label.root().and_downcast::<gtk4::Window>();

                let parent_window = parent.clone();
                dialog.open(parent.as_ref(), None::<&gio::Cancellable>, move |result: Result<gio::File, _>| {
                    // Processes the native file dialog response for loading data from disk.
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        let path_str = path.to_string_lossy().into_owned();
                        let items = read_c7_or_sysex_file(&path_str);
                        // Reject files meant for a different device or data type.
                        if let Some(ref parent_window) = parent_window {
                            if !verify_device_match(parent_window, &path_str, &items, Some(&device_short), "") {
                                return;
                            }
                            if !does_file_type_match(parent_window, &path_str, &items, &item_type_str) {
                                return;
                            }
                        }
                        let item = match find_c7_item(&items, Some(&item_type_str), None) {
                            Some(item) if item.data.is_some() => item.clone(),
                            _ => {
                                return;
                            }
                        };
                        if let Some(ref parent_window) = parent_window
                            && !verify_c7_item(parent_window, &item, Some(&item_type_str), prod, Some(&path_str))
                        {
                            return;
                        }
                        // Extract blob bytes. Handles both binary SysEx and flac+base64 text.
                        let blob: Vec<u8> = match &item.data {
                            Some(C7Data::Binary(b)) if !b.is_empty() => b.clone(),
                            Some(C7Data::Text(text)) if !text.is_empty() => text.as_bytes().to_vec(),
                            _ => {
                                return;
                            }
                        };
                        let basename = std::path::Path::new(&path_str)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        // Name resolution: use item name, SysEx name, or default label.
                        let name = match item_type_str.as_str() {
                            "kit" | "song" => extract_sysex_name(&blob).unwrap_or("--".into()),
                            "pattern" => {
                                let slot = first + row_idx;
                                if is_empty_pattern(&blob) {
                                    "--".to_string()
                                } else {
                                    pattern_slot_label(slot)
                                }
                            }
                            "global" => format!("Global {:02}", first + row_idx + display_offset),
                            _ => item.attrs.get("name").cloned().unwrap_or_else(|| format!("Sample {row_idx}")),
                        };
                        if let Some(state_mut) = state_rc.borrow_mut().as_mut() {
                            state_mut.push_undo(&item_type_str);
                            state_mut.cols.get_mut(&item_type_str).unwrap().blobs.lock().unwrap()[row_idx] = Some(blob);
                            state_mut
                                .cols
                                .get_mut(&item_type_str)
                                .unwrap()
                                .dirty
                                .lock()
                                .unwrap()
                                .insert(row_idx);
                            if let Some(librarian_item) = state_mut.cols[&item_type_str]
                                .model
                                .item(row_idx as u32)
                                .and_downcast::<LibrarianItem>()
                            {
                                librarian_item.set_name(name.as_str());
                            }
                            state_mut.refresh_write_btn(&item_type_str);
                            (state_mut.update_status)(&format!("Status: Imported {item_type_str} from {basename}"));
                        }
                    }
                });
            }
            _ => {}
        }
    }

    // ── Device deletion ────────────────────────────────────────────────────────────────────────────────────

    /// Writes an empty/blank blob to the given slot on the hardware, then removes it locally.
    ///
    /// MD kits and songs mark emptiness with the name's first byte. MD patterns are decoded, zeroed, and re-encoded.
    ///
    /// MnM is not yet supported.
    fn delete_slot_from_device(&mut self, row_idx: usize, item_type: &str) {
        let port = match (self.get_midi_rc)() {
            Some(port_name) if is_valid_port(&port_name) => port_name,
            _ => {
                (self.update_status)("Error: Select a valid MIDI port.");
                return;
            }
        };
        let Some(mut raw) = self.cols[item_type].blobs.lock().unwrap()[row_idx].clone() else {
            (self.update_status)(&format!("Status: Fetch {item_type}s first to delete from device."));
            return;
        };

        // Build the blank blob differently depending on the data type.
        //
        // MD kits and songs mark themselves empty with `0x7F` as the name's first byte, spaces for the rest.
        // Patterns have no name field. For MD patterns the blob is decoded, all step data zeroed, and re-encoded.
        //
        // Neither MD codec applies to MnM, whose kit/song/pattern payloads are RLE7-compressed rather than raw wire bytes.
        let emptied = if item_type == "pattern" {
            if !self.dc.is_device("MD") {
                // MnM pattern codec isn't yet implemented.
                (self.update_status)("Status: Delete from Device is not yet supported for MnM patterns.");
                return;
            }
            blank_md_pattern(&self.dc, &raw)
        } else if self.dc.is_device("MD") {
            // `sysex_layout.<item_type>.fields.name` gives the whole name field, not just its first byte.
            // `0x7F` (DEL) as the first byte marks a slot empty, but the rest of the old name has to go too, or it lingers behind it.
            let name_offset = self.dc.layout_offset_or_die(item_type, "name") as usize;
            let name_size = as_u64_or_die(self.dc.layout_field_or_die(item_type, "name").json_get("size")) as usize;
            if raw.len() >= name_offset + name_size {
                raw[name_offset] = 0x7F;
                for byte in &mut raw[name_offset + 1..name_offset + name_size] {
                    *byte = 0x20;
                }
            }
            update_elektron_checksum(&mut raw);
            raw
        } else {
            // MnM kit/song codec isn't yet implemented.
            (self.update_status)(&format!("Status: Delete from Device is not yet supported for MnM {item_type}s."));
            return;
        };

        let slot = (self.cols[item_type].first_slot + row_idx) as u8;
        let prod = self.dc.prod;
        let ch = get_base_channel();
        let dump_cmd = self.cols[item_type].dump_cmd;
        let arm_type = *self.write_arm_types.get(item_type).unwrap_or(&0);
        let write_arm_cmd = as_u64_or_die(self.dc.json_get("sysex_api.write_arm.cmd")) as u8;
        let item_type_str = item_type.to_string();
        let slot_label = if item_type == "pattern" {
            pattern_slot_label(self.cols[item_type].first_slot + row_idx)
        } else {
            format!("{:02}", slot as usize + self.cols[item_type].display_offset)
        };
        let update_status = Arc::clone(&self.update_status);
        let blobs_ref = self.cols[item_type].blobs.clone();
        let orig_ref = self.cols[item_type].original_blobs.clone();
        let dirty_ref = self.cols[item_type].dirty.clone();
        let model_wk = glib::SendWeakRef::from(self.cols[item_type].model.downgrade());

        let slot_label_c = slot_label.clone();
        let item_type_cap = cap_first_char(&item_type_str);
        update_status(&format!("Status: Deleting {item_type_cap} {slot_label_c} from device..."));

        glib::spawn_future_local(async move {
            let result: Result<(), String> = async {
                let mut packets = parse_sysex_file(&emptied);
                if packets.len() == 1 {
                    let packet = &mut packets[0];
                    if packet.len() > 10 && packet[ELEKTRON_TYPE_BYTE] == dump_cmd {
                        patch_elektron_original_position(packet, 9, slot); // byte 7 is the version marker, not a slot
                    }
                }

                if item_type_str == "pattern" {
                    // Patterns require `send_large_sysex()` to avoid ALSA sequencer segmentation gaps on MD.
                    // Its arm-then-blob logic (and the Linux rawmidi fast path) runs on a worker thread.
                    // So it can't live in a `run_midi_output()` closure.
                    let arm = build_elektron_sysex(prod, ch, &[write_arm_cmd, arm_type, slot & 0x7F, 0x01]);
                    for packet in packets {
                        send_large_sysex(&port, arm.clone(), packet, 0.0, 0.0).await;
                    }
                } else {
                    run_midi_output(&port, move |midi_out| -> Result<(), String> {
                        let arm = build_elektron_sysex(prod, ch, &[write_arm_cmd, arm_type, slot & 0x7F, 0x01]);
                        midi_out.sysex(&arm);
                        sleep(Duration::from_millis(300));
                        for packet in &packets {
                            midi_out.sysex(packet);
                            if packets.len() > 1 {
                                sleep(Duration::from_millis(50));
                            }
                        }
                        Ok(())
                    })
                    .await?;
                }

                Ok(())
            }
            .await;

            let msg = match result {
                Ok(()) => {
                    blobs_ref.lock().unwrap()[row_idx] = None;
                    orig_ref.lock().unwrap()[row_idx] = None;
                    dirty_ref.lock().unwrap().remove(&row_idx);
                    if let Some(model) = model_wk.upgrade()
                        && let Some(item) = model.item(row_idx as u32).and_downcast::<LibrarianItem>()
                    {
                        item.set_name("--");
                        item.set_dirty(false);
                    }
                    format!("Status: Deleted {} {} from device.", cap_first_char(&item_type_str), slot_label)
                }
                Err(e) => format!("Error deleting {item_type_str} {slot_label}: {e}"),
            };
            update_status(&msg);
        });
    }

    // ── Backup import / export ─────────────────────────────────────────────────────────────────────────────

    /// Presents a file selection dialog to import a full Librarian backup.
    fn on_import_backup(&mut self, state_rc: Rc<RefCell<Option<LibrarianState>>>) {
        let dialog = make_file_dialog("Import Bundle", None);

        // Populate file filters for the backup import dialog.
        let filters = gio::ListStore::new::<gtk4::FileFilter>();
        for (name, patterns) in [
            ("All Supported (*.c7, *.syx)", vec!["*.c7", "*.syx"]),
            ("C7 Backup (*.c7)", vec!["*.c7"]),
            ("SysEx (*.syx)", vec!["*.syx"]),
        ] {
            let file_filter = gtk4::FileFilter::new();
            file_filter.set_name(Some(name));
            for pattern in patterns {
                file_filter.add_pattern(pattern);
            }
            filters.append(&file_filter);
        }
        dialog.set_filters(Some(&filters));

        let device_short = self.dc.device_short.clone();
        let kit_first = self.cols["kit"].first_slot;
        let kit_count = self.cols["kit"].slot_count;
        let pattern_first = self.cols["pattern"].first_slot;
        let pattern_count = self.cols["pattern"].slot_count;
        let song_first = self.cols["song"].first_slot;
        let song_count = self.cols["song"].slot_count;
        let sample_first = self.cols["sample"].first_slot;
        let sample_count = self.cols["sample"].slot_count;
        let digipro_first = self.cols["digipro"].first_slot;
        let digipro_count = self.cols["digipro"].slot_count;
        let global_first = self.cols["global"].first_slot;
        let global_count = self.cols["global"].slot_count;
        let global_offset = self.cols["global"].display_offset;
        let parent = self.status_label.root().and_downcast::<gtk4::Window>();

        dialog.open(parent.as_ref(), None::<&gio::Cancellable>, move |result: Result<gio::File, _>| {
            // Processes the backup file selection and routes to the appropriate import logic.
            if let Ok(file) = result
                && let Some(path) = file.path()
            {
                let path_str = path.to_string_lossy().into_owned();
                // Route to specific SysEx backup importer for `.syx` files.
                if path_str.to_lowercase().ends_with(".syx") {
                    if let Some(state_mut) = state_rc.borrow_mut().as_mut() {
                        state_mut.import_syx_backup(&path_str);
                    }
                    return;
                }
                let items = read_c7_or_sysex_file(&path_str);
                let parent_window = gtk4::Window::list_toplevels().into_iter().next();
                if let Some(ref parent_window) = parent_window {
                    // Verify that the backup file matches the current hardware device.
                    if !verify_device_match(parent_window, &path_str, &items, Some(&device_short), "") {
                        return;
                    }
                }
                let mut imported: HashMap<&str, usize> = HashMap::new();
                if let Some(state_mut) = state_rc.borrow_mut().as_mut() {
                    // Prepare undo buffers for all data item_types before merging the backup.
                    for item_type in ALL_ITEM_TYPES {
                        state_mut.push_undo(item_type);
                    }
                    // Map each item in the backup to its matching Librarian slot.
                    for item in &items {
                        let section = item.section.as_str();
                        let item_type = item.get_type();
                        // Skip metadata rows and items without data payloads.
                        if section == "c7" {
                            continue;
                        }
                        // Extract blob bytes. Handles binary SysEx and flac+base64 text.
                        let blob: Vec<u8> = match &item.data {
                            Some(C7Data::Binary(b)) if !b.is_empty() => b.clone(),
                            Some(C7Data::Text(text)) if !text.is_empty() => text.as_bytes().to_vec(),
                            _ => continue,
                        };

                        // Handle Kit slot restoration.
                        if item_type == "kit" {
                            if let Some(wire_slot) = section_to_wire_slot(section, item_type) {
                                let row_idx = wire_slot.saturating_sub(kit_first);
                                if row_idx < kit_count {
                                    let name = extract_sysex_name(&blob).unwrap_or("--".into());
                                    if let Some(item) = state_mut.cols["kit"].model.item(row_idx as u32).and_downcast::<LibrarianItem>() {
                                        item.set_name(name.as_str());
                                    }
                                    state_mut.cols.get_mut("kit").unwrap().blobs.lock().unwrap()[row_idx] = Some(blob);
                                    state_mut.cols.get_mut("kit").unwrap().dirty.lock().unwrap().insert(row_idx);
                                    *imported.entry(item_type).or_default() += 1;
                                }
                            }
                        }
                        // Handle Sample slot restoration.
                        else if item_type == "sample" {
                            if let Some(wire_slot) = section_to_wire_slot(section, item_type) {
                                let row_idx = wire_slot.saturating_sub(sample_first);
                                if row_idx < sample_count {
                                    let name = item.attrs.get("name").cloned().unwrap_or_else(|| format!("Sample {row_idx}"));
                                    if let Some(item) = state_mut.cols["sample"].model.item(row_idx as u32).and_downcast::<LibrarianItem>()
                                    {
                                        item.set_name(name.as_str());
                                    }
                                    state_mut.cols.get_mut("sample").unwrap().blobs.lock().unwrap()[row_idx] = Some(blob);
                                    state_mut.cols.get_mut("sample").unwrap().dirty.lock().unwrap().insert(row_idx);
                                    *imported.entry(item_type).or_default() += 1;
                                }
                            }
                        }
                        // Handle DigiPro slot restoration.
                        else if item_type == "digipro" {
                            if let Some(wire_slot) = section_to_wire_slot(section, item_type) {
                                let row_idx = wire_slot.saturating_sub(digipro_first);
                                if row_idx < digipro_count {
                                    let name = item.attrs.get("name").cloned().unwrap_or_else(|| format!("DigiPro {row_idx}"));
                                    if let Some(item) = state_mut.cols["digipro"].model.item(row_idx as u32).and_downcast::<LibrarianItem>()
                                    {
                                        item.set_name(name.as_str());
                                    }
                                    state_mut.cols.get_mut("digipro").unwrap().blobs.lock().unwrap()[row_idx] = Some(blob);
                                    state_mut.cols.get_mut("digipro").unwrap().dirty.lock().unwrap().insert(row_idx);
                                    *imported.entry(item_type).or_default() += 1;
                                }
                            }
                        }
                        // Handle Pattern slot restoration.
                        else if item_type == "pattern" {
                            if let Some(wire_slot) = section_to_wire_slot(section, item_type) {
                                let row_idx = wire_slot.saturating_sub(pattern_first);
                                if row_idx < pattern_count {
                                    let slot = pattern_first + row_idx;
                                    let name = if is_empty_pattern(&blob) {
                                        "--".to_string()
                                    } else {
                                        pattern_slot_label(slot)
                                    };
                                    if let Some(item) = state_mut.cols["pattern"].model.item(row_idx as u32).and_downcast::<LibrarianItem>()
                                    {
                                        item.set_name(name.as_str());
                                    }
                                    state_mut.cols.get_mut("pattern").unwrap().blobs.lock().unwrap()[row_idx] = Some(blob);
                                    state_mut.cols.get_mut("pattern").unwrap().dirty.lock().unwrap().insert(row_idx);
                                    *imported.entry(item_type).or_default() += 1;
                                }
                            }
                        }
                        // Handle Song slot restoration.
                        else if item_type == "song" {
                            if let Some(wire_slot) = section_to_wire_slot(section, item_type) {
                                let row_idx = wire_slot.saturating_sub(song_first);
                                if row_idx < song_count {
                                    let name = extract_sysex_name(&blob).unwrap_or("--".into());
                                    if let Some(item) = state_mut.cols["song"].model.item(row_idx as u32).and_downcast::<LibrarianItem>() {
                                        item.set_name(name.as_str());
                                    }
                                    state_mut.cols.get_mut("song").unwrap().blobs.lock().unwrap()[row_idx] = Some(blob);
                                    state_mut.cols.get_mut("song").unwrap().dirty.lock().unwrap().insert(row_idx);
                                    *imported.entry(item_type).or_default() += 1;
                                }
                            }
                        }
                        // Handle Global settings restoration.
                        else if item_type == "global"
                            && let Some(wire_slot) = section_to_wire_slot(section, item_type)
                        {
                            let row_idx = wire_slot.saturating_sub(global_first);
                            if row_idx < global_count {
                                let slot_label = format!("Global {:02}", global_first + row_idx + global_offset);
                                if let Some(item) = state_mut.cols["global"].model.item(row_idx as u32).and_downcast::<LibrarianItem>() {
                                    item.set_name(slot_label.as_str());
                                }
                                state_mut.cols.get_mut("global").unwrap().blobs.lock().unwrap()[row_idx] = Some(blob);
                                state_mut.cols.get_mut("global").unwrap().dirty.lock().unwrap().insert(row_idx);
                                *imported.entry(item_type).or_default() += 1;
                            }
                        }
                    }
                    // Refresh UI state for all columns after the bulk import.
                    for item_type in ALL_ITEM_TYPES {
                        state_mut.refresh_write_btn(item_type);
                    }
                    // A `.c7` can carry any item type, so the report lists whichever columns this device actually has.
                    let parts: Vec<String> = ALL_ITEM_TYPES
                        .into_iter()
                        .filter(|item_type| state_mut.cols[*item_type].slot_count > 0)
                        .map(|item_type| format!("{} {item_type}s", imported.get(item_type).copied().unwrap_or(0)))
                        .collect();
                    (state_mut.update_status)(&format!("Imported backup: {}.", parts.join(", ")));
                }
            }
        });
    }

    /// Initializes a full Librarian backup export in a background thread.
    fn on_export_backup(&mut self) {
        let default_name = format!("{}_Bundle_{}.c7", self.dc.device_shorter, now_compact_timestamp());

        let dialog = make_file_dialog("Export Bundle", Some(&default_name));

        let dc = Arc::clone(&self.dc);
        let port = (self.get_midi_rc)();
        let ch = get_base_channel();
        let update_status = Arc::clone(&self.update_status);
        let kit_blobs = self.cols["kit"].blobs.clone();
        let kit_model = self.cols["kit"].model.clone();
        let pattern_blobs = self.cols["pattern"].blobs.clone();
        let pattern_model = self.cols["pattern"].model.clone();
        let song_blobs = self.cols["song"].blobs.clone();
        let song_model = self.cols["song"].model.clone();
        let sample_blobs = self.cols["sample"].blobs.clone();
        let sample_model = self.cols["sample"].model.clone();
        let digipro_blobs = self.cols["digipro"].blobs.clone();
        let digipro_model = self.cols["digipro"].model.clone();
        let global_blobs = self.cols["global"].blobs.clone();
        let parent = self.status_label.root().and_downcast::<gtk4::Window>();

        // Snapshot kit/pattern/song/global names from model (main thread only).
        let kit_names: Vec<String> = (0..kit_model.n_items())
            .map(|i| kit_model.item(i).and_downcast::<LibrarianItem>().unwrap().name())
            .collect();
        let pattern_names: Vec<String> = (0..pattern_model.n_items())
            .map(|i| pattern_model.item(i).and_downcast::<LibrarianItem>().unwrap().name())
            .collect();
        let song_names: Vec<String> = (0..song_model.n_items())
            .map(|i| song_model.item(i).and_downcast::<LibrarianItem>().unwrap().name())
            .collect();
        let sample_names: Vec<String> = (0..sample_model.n_items())
            .map(|i| sample_model.item(i).and_downcast::<LibrarianItem>().unwrap().name())
            .collect();
        let digipro_names: Vec<String> = (0..digipro_model.n_items())
            .map(|i| digipro_model.item(i).and_downcast::<LibrarianItem>().unwrap().name())
            .collect();

        dialog.save(parent.as_ref(), None::<&gio::Cancellable>, move |result: Result<gio::File, _>| {
            // Processes the native file dialog response for saving data to disk.
            if let Ok(file) = result
                && let Some(path) = file.path()
            {
                let path_str = path.to_string_lossy().into_owned();
                let update_status_c2 = Arc::clone(&update_status);
                let dc_c2 = Arc::clone(&dc);
                glib::spawn_future_local(async move {
                    LibrarianState::export_backup_c7(
                        path_str,
                        dc_c2,
                        port,
                        ch,
                        update_status_c2,
                        kit_blobs,
                        kit_names,
                        pattern_blobs,
                        pattern_names,
                        song_blobs,
                        song_names,
                        sample_blobs,
                        sample_names,
                        digipro_blobs,
                        digipro_names,
                        global_blobs,
                    )
                    .await;
                });
            }
        });
    }

    /// Toggles the passive SysEx listener on/off.
    fn on_listen_clicked(&mut self) {
        if self.listening_active.load(Ordering::Relaxed) {
            self.cancel_listen.store(true, Ordering::Relaxed);
            return;
        }
        let port = match (self.get_midi_rc)() {
            Some(port_name) if is_valid_port(&port_name) => port_name,
            _ => {
                (self.update_status)("Error: No MIDI port");
                return;
            }
        };
        let Some(in_port) = find_input_port(&port) else {
            (self.update_status)("Error: No MIDI input port found.");
            return;
        };
        for item_type in ALL_ITEM_TYPES {
            self.push_undo(item_type);
        }
        self.cancel_listen.store(false, Ordering::Relaxed);
        self.listen_count.store(0, Ordering::Relaxed);
        self.listening_active.store(true, Ordering::Relaxed);
        self.btn_listen.set_label("Stop Listening");
        self.btn_listen.add_css_class("destructive-action");
        (self.update_status)("Status: Listening for SysEx from hardware...");

        let cancel = Arc::clone(&self.cancel_listen);
        let active = Arc::clone(&self.listening_active);
        let tx = self.listen_tx.clone().unwrap();
        let update_status = Arc::clone(&self.update_status);
        let btn_wk = glib::SendWeakRef::from(self.btn_listen.downgrade());

        glib::spawn_future_local(async move {
            let result = open_large_sysex_input(&in_port);
            match result {
                Err(e) => {
                    update_status(&format!("Status: Listen error: {e}"));
                }
                Ok(midi_in) => {
                    while !cancel.load(Ordering::Relaxed) {
                        if let Some(raw) = midi_in.poll() {
                            if raw.len() >= 5 && raw[0] == 0xF0 {
                                let _ = tx.try_send(raw);
                            }
                        } else {
                            glib::timeout_future(Duration::from_millis(10)).await;
                        }
                    }
                }
            }
            active.store(false, Ordering::Relaxed);
            update_status("Status: Listening stopped.");
            if let Some(btn) = btn_wk.upgrade() {
                btn.set_label("Listen");
                btn.remove_css_class("destructive-action");
            }
        });
    }

    /// Routes a raw SysEx message received during passive listening into the appropriate column.
    ///
    /// Called on the main thread via the glib channel receiver.
    fn route_live_message(&mut self, raw: &[u8]) {
        // Only route Elektron SysEx for this device (product byte at `raw[4]`).
        if raw.len() < 10 || !is_elektron_sysex(raw, self.dc.prod) {
            return;
        }
        let cmd = raw[ELEKTRON_TYPE_BYTE];
        let slot = raw[9] as usize; // origPos; byte 7 is the version marker, not a slot
        let raw_vec = raw.to_vec();

        let mut is_routed = false;
        for item_type in ALL_ITEM_TYPES {
            // Copy scalars out to avoid holding a borrow into self.cols during the mutable update.
            let (col_first, col_count, col_dump, col_display_offset) = {
                let col = &self.cols[item_type];
                (col.first_slot, col.slot_count, col.dump_cmd, col.display_offset)
            };
            if cmd != col_dump {
                continue;
            }
            let Some(idx) = slot.checked_sub(col_first).filter(|&i| i < col_count) else {
                continue;
            };
            let display: Option<String> = match item_type {
                "kit" | "song" => extract_sysex_name(raw),
                "pattern" => {
                    if is_empty_pattern(raw) {
                        None
                    } else {
                        Some(pattern_slot_label(col_first + idx))
                    }
                }
                "global" => Some(format!("{:02}", slot + col_display_offset)),
                "digipro" => {
                    let raw_name = digipro_name(raw, &self.dc);
                    Some(if raw_name.is_empty() {
                        format!("DigiPro {:02}", slot + col_display_offset)
                    } else {
                        raw_name
                    })
                }
                // "sample" is left out on purpose.
                // MD's `sysex_api.sample.write_cmd` is 0, since samples go through the full SDS handshake instead of one dump command.
                // So `cmd != col_dump` above always skips this arm for "sample" before it would ever be reached.
                _ => None,
            };
            if let Some(name) = display {
                let col = self.cols.get_mut(item_type).unwrap();
                col.blobs.lock().unwrap()[idx] = Some(raw_vec);
                col.dirty.lock().unwrap().insert(idx);
                if let Some(item) = col.model.item(idx as u32).and_downcast::<LibrarianItem>() {
                    item.set_name(name.as_str());
                    item.set_dirty(true);
                }
                is_routed = true;
                break;
            }
        }
        if is_routed {
            let count = self.listen_count.fetch_add(1, Ordering::Relaxed) + 1;
            (self.update_status)(&format!("Status: Listening... {count} item(s) received"));
        }
    }

    // ── Fetch ──────────────────────────────────────────────────────────────────────────────────────────────

    /// Triggered by the 'Fetch All' buttons to begin capturing data from the hardware.
    fn on_fetch(&mut self, btn: &Button, item_type: &str) {
        if self.is_active.load(Ordering::Relaxed) {
            self.is_active.store(false, Ordering::Relaxed);
            btn.set_sensitive(false);
            (self.update_status)(&format!("Status: Cancelling {} fetch...", cap_first_char(item_type)));
            return;
        }
        let port = match (self.get_midi_rc)() {
            Some(port_name) if is_valid_port(&port_name) => port_name,
            _ => {
                (self.update_status)("Error: Select a valid MIDI port.");
                return;
            }
        };
        let Some(in_port) = find_input_port(&port) else {
            (self.update_status)("Error: Could not find matching input port.");
            return;
        };

        self.is_active.store(true, Ordering::Relaxed);
        {
            let col = self.cols.get_mut(item_type).unwrap();
            col.blobs.lock().unwrap().fill(None);
            col.original_blobs.lock().unwrap().fill(None);
            col.dirty.lock().unwrap().clear();
            col.undo.clear();
        }
        self.refresh_write_btn(item_type);
        let item_type_cap = cap_first_char(item_type);
        btn.set_label(&format!("Cancel {item_type_cap} Fetch"));
        btn.remove_css_class("suggested-action");
        btn.add_css_class("destructive-action");
        (self.update_status)(&format!("Status: Fetching {item_type_cap}s..."));
        // Disable all fetch buttons except this one while fetching.
        for iter_item_type in ALL_ITEM_TYPES {
            self.cols[iter_item_type].btn_fetch.set_sensitive(iter_item_type == item_type);
        }

        let col = &self.cols[item_type];
        let count = col.slot_count;
        let first = col.first_slot;
        let display_offset = col.display_offset;
        let request_cmd = col.request_cmd;
        let dump_cmd = col.dump_cmd;
        let blobs_ref = col.blobs.clone();
        let orig_ref = col.original_blobs.clone();
        let model_wk = glib::SendWeakRef::from(col.model.downgrade());
        let update_status = Arc::clone(&self.update_status);
        let btn_wk = glib::SendWeakRef::from(btn.downgrade());
        let all_fetch_wk: Vec<glib::SendWeakRef<Button>> = ALL_ITEM_TYPES
            .iter()
            .map(|&k| glib::SendWeakRef::from(self.cols[k].btn_fetch.downgrade()))
            .collect();
        let ch = get_base_channel();
        let prod = self.dc.prod;
        let active = Arc::clone(&self.is_active);
        let pending_check = Arc::clone(&self.pending_unused_check);
        let item_type_str = item_type.to_string();
        let timeout = match item_type {
            "pattern" => 6.0_f64,
            _ => 2.5,
        };
        let dc_c = Arc::clone(&self.dc);
        glib::spawn_future_local(async move {
            let (tx, rx) = async_channel::unbounded::<FetchUpdate>();
            let model_wk_c = model_wk.clone();
            let update_status_c2 = Arc::clone(&update_status);
            let update_status_c3 = Arc::clone(&update_status_c2);
            glib::spawn_future_local(async move {
                while let Ok(upd) = rx.recv().await {
                    match upd {
                        FetchUpdate::Status(msg) => update_status_c3(&msg),
                        FetchUpdate::ItemName(idx, display) => {
                            if let Some(model) = model_wk_c.upgrade()
                                && let Some(item) = model.item(idx as u32).and_downcast::<LibrarianItem>()
                            {
                                item.set_name(display.as_str());
                            }
                        }
                    }
                }
            });

            let active_c2 = active.clone();
            let blobs_ref_c2 = blobs_ref.clone();
            let dc_c2 = dc_c.clone();

            let fetch_result: Result<(), String> = async {
                if item_type_str == "sample" && dc_c2.is_device("MD") {
                    let tx_c = tx.clone();
                    run_midi_session(&port, move |midi_in, midi_out| -> Result<(), String> {
                        let _ = tx_c.send_blocking(FetchUpdate::Status("Fetching sample names...".to_string()));
                        let slots = fetch_md_sample_names(midi_in, midi_out, prod, ch, timeout);
                        if let Some(names) = slots {
                            for (idx, (name, occupied)) in names.iter().enumerate().take(count) {
                                if !active_c2.load(Ordering::Relaxed) {
                                    break;
                                }

                                let display = if *occupied && !name.is_empty() { name.clone() } else { "--".into() };

                                blobs_ref_c2.lock().unwrap()[idx] = if *occupied { Some(vec![]) } else { None };
                                let _ = tx_c.send_blocking(FetchUpdate::ItemName(idx, display));
                            }
                        } else {
                            for idx in 0..count {
                                if !active_c2.load(Ordering::Relaxed) {
                                    break;
                                }
                                let slot_str = format!("{:02}", first + idx + display_offset);
                                let _ = tx_c.send_blocking(FetchUpdate::Status(format!("Fetching Sample {slot_str}...")));
                                let (name_display, name_blob, _) =
                                    probe_sds_slot(midi_in, midi_out, (first + idx) as u8, ch, prod, timeout);
                                blobs_ref_c2.lock().unwrap()[idx] = name_blob;
                                let display = name_display.unwrap_or_else(|| "--".into());
                                let _ = tx_c.send_blocking(FetchUpdate::ItemName(idx, display));
                                sleep(Duration::from_millis(200));
                            }
                        }
                        Ok(())
                    })
                    .await?;
                } else {
                    let midi_in = open_large_sysex_input(&in_port)?;
                    let tx_c = tx.clone();
                    let item_type_str_c = item_type_str.clone();
                    run_midi_output(&port, move |midi_out| -> Result<(), String> {
                        for idx in 0..count {
                            if !active_c2.load(Ordering::Relaxed) {
                                break;
                            }
                            let slot = (first + idx) as u8;

                            let slot_label = if item_type_str_c == "pattern" {
                                pattern_slot_label(first + idx)
                            } else {
                                format!("{:02}", first + idx + display_offset)
                            };

                            let _ = tx_c.send_blocking(FetchUpdate::Status(format!("Fetching {item_type_str_c} {slot_label}...")));
                            let raw = fetch_elektron_slot(&midi_in, midi_out, prod, ch, request_cmd, dump_cmd, slot, timeout, &[]);
                            let display: String = match raw.as_deref() {
                                Some(raw) => match item_type_str_c.as_str() {
                                    "kit" | "song" => extract_sysex_name(raw).unwrap_or_else(|| "--".into()),
                                    "pattern" => {
                                        if is_empty_pattern(raw) {
                                            "--".into()
                                        } else {
                                            pattern_slot_label(first + idx)
                                        }
                                    }
                                    "global" => format!("{:02}", first + idx + display_offset),
                                    // Only reached for `digipro`.
                                    // `sample` fetches for the one device that has them (MD) are routed through the SDS branch above.
                                    _ => {
                                        let name_str = digipro_name(raw, &dc_c2);
                                        if name_str.is_empty() {
                                            format!("DigiPro {:02}", first + idx + display_offset)
                                        } else {
                                            name_str
                                        }
                                    }
                                },
                                None => {
                                    if item_type_str_c == "sample" {
                                        "--".into()
                                    } else {
                                        "<Timeout>".into()
                                    }
                                }
                            };
                            blobs_ref_c2.lock().unwrap()[idx] = raw;
                            let _ = tx_c.send_blocking(FetchUpdate::ItemName(idx, display));
                        }
                        Ok(())
                    })
                    .await?;
                }
                Ok(())
            }
            .await;

            (*orig_ref.lock().unwrap()).clone_from(&blobs_ref.lock().unwrap());
            active.store(false, Ordering::Relaxed);

            let msg = match fetch_result {
                Ok(()) => format!("Status: Finished fetching {item_type_cap}s."),
                Err(e) => format!("Error: {e}"),
            };
            update_status_c2(&msg);
            if let Some(btn) = btn_wk.upgrade() {
                btn.set_label(&format!("Fetch All {item_type_cap}s"));
                btn.remove_css_class("destructive-action");
                btn.add_css_class("suggested-action");
            }
            for fetch_btn_wk in &all_fetch_wk {
                if let Some(btn) = fetch_btn_wk.upgrade() {
                    btn.set_sensitive(true);
                }
            }
            if matches!(item_type_str.as_str(), "kit" | "sample") {
                pending_check.store(true, Ordering::Relaxed);
            }
        });
    }

    // ── Write ──────────────────────────────────────────────────────────────────────────────────────────────

    /// Triggered by the 'Write All' buttons to begin uploading modified data to the device.
    fn on_write(&mut self, btn: &Button, item_type: &str) {
        if self.is_active.load(Ordering::Relaxed) {
            self.is_active.store(false, Ordering::Relaxed);
            btn.set_label(&format!("Write {item_type}s to Device"));
            btn.remove_css_class("destructive-action");
            btn.set_sensitive(false);
            (self.update_status)(&format!("Status: Cancelling {} Write...", cap_first_char(item_type)));
            return;
        }
        let port = match (self.get_midi_rc)() {
            Some(port_name) if is_valid_port(&port_name) => port_name,
            _ => {
                (self.update_status)("Error: Select a valid MIDI port.");
                return;
            }
        };

        // The Monomachine strictly requires manual UI arming for Songs.
        let needs_arm_dialog = (self.dc.is_device("MnM") && item_type == "song") || (self.dc.is_device("MD") && item_type == "sample");

        if needs_arm_dialog {
            let item_type_cap = cap_first_char(item_type);
            let device_short = self.dc.device_short.clone();
            let device_shorter = self.dc.device_shorter.clone();

            let body = if self.dc.is_device("MnM") && item_type == "song" {
                format!(
                    "The {device_short} cannot automatically accept incoming songs in the background.\n\n\
                     1. On the Monomachine, go to GLOBAL > SYSEX RECV > SONG\n\
                     2. Select ORIG (or ANY) and press YES\n\n\
                     Wait until the Monomachine screen says \"WAITING\" before clicking OK."
                )
            } else {
                format!(
                    "The {device_short} cannot automatically accept incoming samples.\n\n\
                     1. On the {device_shorter}, go to GLOBAL > FILE > SAMPLE MGR\n\
                     2. Select RECV and press YES\n\
                     3. Select ANY slot and press YES (C7 routes them to the correct slots).\n\n\
                     Wait until the {device_shorter} screen says \"WAITING\" before clicking OK."
                )
            };

            let dialog = libadwaita::AlertDialog::new(Some(&format!("Arm Device for {item_type_cap} Upload")), Some(&body));
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("ok", "OK");
            dialog.set_default_response(Some("ok"));
            // The response handler captures Arc copies of the thread data so it can spawn the write thread once the user confirms arming.
            let blobs_ref = self.cols[item_type].blobs.clone();
            let orig_ref = self.cols[item_type].original_blobs.clone();
            let dirty_ref = self.cols[item_type].dirty.clone();
            let dirty_indices: Vec<usize> = {
                let dirty_lock = self.cols[item_type].dirty.lock().unwrap();
                let mut indices: Vec<usize> = dirty_lock.iter().copied().collect();
                indices.sort_unstable();
                indices
            };
            let slot_names: Vec<String> = dirty_indices
                .iter()
                .map(|&idx| {
                    self.cols[item_type]
                        .model
                        .item(idx as u32)
                        .and_downcast::<LibrarianItem>()
                        .map(|item| item.name())
                        .filter(|name| !name.is_empty() && name != "--")
                        .unwrap_or_else(|| format!("{:02}", self.cols[item_type].first_slot + idx + self.cols[item_type].display_offset))
                })
                .collect();
            let model_wk = glib::SendWeakRef::from(self.cols[item_type].model.downgrade());
            let update_status = Arc::clone(&self.update_status);
            let btn_wk = glib::SendWeakRef::from(self.cols[item_type].btn_write.downgrade());
            let progress_bar_wk = glib::SendWeakRef::from(self.progress_bar.downgrade());
            let all_fetch_wk: Vec<glib::SendWeakRef<Button>> = ALL_ITEM_TYPES
                .iter()
                .map(|&k| glib::SendWeakRef::from(self.cols[k].btn_fetch.downgrade()))
                .collect();
            let all_write_wk: Vec<glib::SendWeakRef<Button>> = ALL_ITEM_TYPES
                .iter()
                .filter(|&&k| k != item_type)
                .map(|&k| glib::SendWeakRef::from(self.cols[k].btn_write.downgrade()))
                .collect();
            let active = Arc::clone(&self.is_active);
            let ch = get_base_channel();
            let dc_c = Arc::clone(&self.dc);
            let item_type_str = item_type.to_string();

            if dirty_indices.is_empty() {
                (self.update_status)("Status: Nothing to write.");
                return;
            }

            // Show the arm dialog. Start the thread only when the user presses OK.
            let parent = self.status_label.root().and_downcast::<gtk4::Window>();
            dialog.connect_response(None, move |_, response| {
                if response != "ok" {
                    return;
                }
                active.store(true, Ordering::Relaxed);
                if let Some(progress_bar) = progress_bar_wk.upgrade() {
                    progress_bar.set_visible(true);
                    progress_bar.set_fraction(0.0);
                }
                if let Some(btn) = btn_wk.upgrade() {
                    btn.set_label("Cancel Operation");
                    btn.add_css_class("destructive-action");
                }
                for fetch_btn_wk in &all_fetch_wk {
                    if let Some(fetch_btn) = fetch_btn_wk.upgrade() {
                        fetch_btn.set_sensitive(false);
                    }
                }
                for write_btn_wk in &all_write_wk {
                    if let Some(write_btn) = write_btn_wk.upgrade()
                        && write_btn.label().map(|label| label.to_string()) != Some("Cancel Operation".to_string())
                    {
                        write_btn.set_sensitive(false);
                    }
                }

                let blobs_ref_c2 = blobs_ref.clone();
                let orig_ref_c2 = orig_ref.clone();
                let dirty_ref_c2 = dirty_ref.clone();
                let dirty = dirty_indices.clone();
                let slot_names_c = slot_names.clone();
                let model_wk = model_wk.clone();
                let update_status_c2 = Arc::clone(&update_status);
                let btn_wk = btn_wk.clone();
                let progress_bar_wk = progress_bar_wk.clone();
                let all_fetch_wk = all_fetch_wk.clone();
                let all_write_wk = all_write_wk.clone();
                let active_c2 = active.clone();
                let port_c = port.clone();
                let item_type_str_c = item_type_str.clone();
                let dc_c2 = Arc::clone(&dc_c);
                glib::spawn_future_local(async move {
                    Self::write_slots_to_device(
                        port_c,
                        blobs_ref_c2,
                        orig_ref_c2,
                        dirty_ref_c2,
                        dirty,
                        slot_names_c,
                        model_wk,
                        update_status_c2,
                        btn_wk,
                        progress_bar_wk,
                        all_fetch_wk,
                        all_write_wk,
                        active_c2,
                        ch,
                        dc_c2,
                        item_type_str_c,
                    )
                    .await;
                });
            });
            dialog.present(parent.as_ref());
            return;
        }

        // No arm dialog is needed. Collect dirty indices and start immediately.
        let dirty_indices: Vec<usize> = {
            let dirty_lock = self.cols[item_type].dirty.lock().unwrap();
            let mut indices: Vec<usize> = dirty_lock.iter().copied().collect();
            indices.sort_unstable();
            indices
        };
        if dirty_indices.is_empty() {
            (self.update_status)("Status: Nothing to write.");
            return;
        }

        self.is_active.store(true, Ordering::Relaxed);
        btn.set_label("Cancel Operation");
        btn.add_css_class("destructive-action");
        self.progress_bar.set_visible(true);
        self.progress_bar.set_fraction(0.0);
        for iter_item_type in ALL_ITEM_TYPES {
            self.cols[iter_item_type].btn_fetch.set_sensitive(false);
            self.cols[iter_item_type].btn_write.set_sensitive(iter_item_type == item_type);
        }

        let blobs_ref = self.cols[item_type].blobs.clone();
        let orig_ref = self.cols[item_type].original_blobs.clone();
        let dirty_ref = self.cols[item_type].dirty.clone();
        let slot_names: Vec<String> = dirty_indices
            .iter()
            .map(|&idx| {
                self.cols[item_type]
                    .model
                    .item(idx as u32)
                    .and_downcast::<LibrarianItem>()
                    .map(|item| item.name())
                    .filter(|name| !name.is_empty() && name != "--")
                    .unwrap_or_else(|| format!("{:02}", self.cols[item_type].first_slot + idx + self.cols[item_type].display_offset))
            })
            .collect();
        let model_wk = glib::SendWeakRef::from(self.cols[item_type].model.downgrade());
        let update_status = Arc::clone(&self.update_status);
        let btn_wk = glib::SendWeakRef::from(btn.downgrade());
        let all_fetch_wk: Vec<glib::SendWeakRef<Button>> = ALL_ITEM_TYPES
            .iter()
            .map(|&k| glib::SendWeakRef::from(self.cols[k].btn_fetch.downgrade()))
            .collect();
        let all_write_wk: Vec<glib::SendWeakRef<Button>> = ALL_ITEM_TYPES
            .iter()
            .filter(|&&k| k != item_type)
            .map(|&k| glib::SendWeakRef::from(self.cols[k].btn_write.downgrade()))
            .collect();
        let progress_bar_wk = glib::SendWeakRef::from(self.progress_bar.downgrade());
        let active = Arc::clone(&self.is_active);
        let ch = get_base_channel();
        let dc_c = Arc::clone(&self.dc);
        let item_type_str = item_type.to_string();

        glib::spawn_future_local(async move {
            Self::write_slots_to_device(
                port,
                blobs_ref,
                orig_ref,
                dirty_ref,
                dirty_indices,
                slot_names,
                model_wk,
                update_status,
                btn_wk,
                progress_bar_wk,
                all_fetch_wk,
                all_write_wk,
                active,
                ch,
                dc_c,
                item_type_str,
            )
            .await;
        });
    }

    // ── Unused-sample annotation ───────────────────────────────────────────────────────────────────────────

    /// Renames samples not referenced by any fetched kit to "SAMPLE (Unused)".
    ///
    /// Only runs for the MD and only when both kits and samples are fully fetched.
    fn maybe_mark_unused_samples(&self) {
        if !self.dc.is_device("MD") {
            return;
        }

        let sample_col = &self.cols["sample"];
        let kit_col = &self.cols["kit"];

        // Confirm all samples have been fetched (no placeholder names remain).
        let all_samples_fetched = (0..sample_col.model.n_items()).all(|i| {
            sample_col
                .model
                .item(i)
                .and_downcast::<LibrarianItem>()
                .is_none_or(|item| item.name() != UNFETCHED)
        });
        if !all_samples_fetched {
            return;
        }

        // Confirm all kits have been fetched.
        let all_kits_fetched = (0..kit_col.model.n_items()).all(|i| {
            kit_col
                .model
                .item(i)
                .and_downcast::<LibrarianItem>()
                .is_none_or(|item| item.name() != UNFETCHED)
        });
        if !all_kits_fetched {
            return;
        }

        // Collect every sample slot index referenced by any kit.
        let used_slots: HashSet<usize> = kit_col
            .blobs
            .lock()
            .unwrap()
            .iter()
            .filter_map(|blob| blob.as_ref())
            .flat_map(|blob| md_kit_used_sample_slots(blob, &self.dc))
            .collect();

        // Rename unreferenced samples that have actual data.
        let sample_blobs = sample_col.blobs.lock().unwrap();
        for idx in 0..sample_col.model.n_items() as usize {
            if sample_blobs[idx].is_none() {
                continue;
            }
            let Some(item) = sample_col.model.item(idx as u32).and_downcast::<LibrarianItem>() else {
                continue;
            };
            let name = item.name();
            if name == "--" || name.is_empty() {
                continue;
            }
            if !used_slots.contains(&idx) {
                let unused_name = format!("{name} (Unused)");
                item.set_name(unused_name.as_str());
            }
        }
    }

    // ── Reset UI Helpers ───────────────────────────────────────────────────────────────────────────────────

    /// Updates Write/Undo button states and repaints dirty-row highlights.
    fn refresh_write_btn(&self, item_type: &str) {
        let col = &self.cols[item_type];
        let dirty = col.dirty.lock().unwrap();
        let dirty_len = dirty.len();
        if dirty_len > 0 {
            col.btn_write.set_label(&format!("Write {dirty_len} {item_type}s to Device"));
            col.btn_write.set_sensitive(true);
        } else {
            col.btn_write.set_label(&format!("Write {item_type}s to Device"));
            col.btn_write.set_sensitive(false);
        }
        let has_undo = !col.undo.is_empty();
        col.btn_undo.set_opacity(if has_undo { 1.0 } else { 0.0 });
        col.btn_undo.set_sensitive(has_undo);
        for i in 0..col.model.n_items() {
            col.model
                .item(i)
                .and_downcast::<LibrarianItem>()
                .unwrap()
                .set_dirty(dirty.contains(&(i as usize)));
        }
    }

    /// Snapshots blobs, dirty set, and row labels before a mutation.
    fn push_undo(&mut self, item_type: &str) {
        let col = self.cols.get_mut(item_type).unwrap();
        let texts: Vec<String> = (0..col.model.n_items())
            .map(|i| col.model.item(i).and_downcast::<LibrarianItem>().unwrap().name())
            .collect();
        col.undo
            .push((col.blobs.lock().unwrap().clone(), col.dirty.lock().unwrap().clone(), texts));
    }

    /// Pops one undo step and restores blobs, dirty set, and row labels.
    fn on_undo(&mut self, item_type: &str) {
        let col = self.cols.get_mut(item_type).unwrap();
        if let Some((blobs, dirty, names)) = col.undo.pop() {
            *col.blobs.lock().unwrap() = blobs;
            *col.dirty.lock().unwrap() = dirty;
            for (i, txt) in names.iter().enumerate() {
                col.model
                    .item(i as u32)
                    .and_downcast::<LibrarianItem>()
                    .unwrap()
                    .set_name(txt.as_str());
            }
        }
        self.refresh_write_btn(item_type);
        (self.update_status)(&format!("Status: Undid last {item_type} change."));
    }

    // ── Slot label helpers ─────────────────────────────────────────────────────────────────────────────────

    /// Calculates the display label for a specific slot index based on its data type.
    fn item_type_slot_label(&self, item_type: &str, slot: usize) -> String {
        // Use specific bank/number labeling for patterns.
        if item_type == "pattern" {
            return pattern_slot_label(slot);
        }
        let display_offset = self.cols[item_type].display_offset;
        format!("{:02}", slot + display_offset)
    }

    // ── SysEx backup import ────────────────────────────────────────────────────────────────────────────────

    /// Parses a multi-message SysEx file and merges its contents into the local Librarian.
    fn import_syx_backup(&mut self, path: &str) {
        // Load the raw SysEx binary data from disk.
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                (self.update_status)(&format!("Error: Failed to read SYX: {e}"));
                return;
            }
        };
        let msgs = parse_sysex_file(&data);
        let items = read_c7_or_sysex_file(path);
        let parent_window = gtk4::Window::list_toplevels().into_iter().next();
        if let Some(ref parent_window) = parent_window
            && !verify_device_match(parent_window, path, &items, Some(&self.dc.device_short), "")
        {
            return;
        }

        let prod = self.dc.prod;
        let kit_dump = self.cols["kit"].dump_cmd;
        let pattern_dump = self.cols["pattern"].dump_cmd;
        let song_dump = self.cols["song"].dump_cmd;
        let global_dump = self.cols["global"].dump_cmd;
        let digipro_dump = self.cols["digipro"].dump_cmd;
        let mut imported: HashMap<&str, usize> = HashMap::new();

        // Create undo checkpoints for all data categories before merging SysEx messages.
        for item_type in ALL_ITEM_TYPES {
            self.push_undo(item_type);
        }

        // Iterate through all SysEx messages in the backup file and route them to slots.
        for raw in &msgs {
            // Skip messages that don't match the Elektron SysEx header format.
            if raw.len() < 10 || !is_elektron_sysex(raw, prod) {
                continue;
            }
            let cmd = raw[ELEKTRON_TYPE_BYTE];
            let slot = raw[9] as usize; // origPos; byte 7 is the version marker, not a slot
            let blob = raw.clone();

            // Map Kit dump messages to Librarian slots.
            if cmd == kit_dump {
                let kit_col = &self.cols["kit"];
                let row_idx = slot.wrapping_sub(kit_col.first_slot);
                // Verify the target slot is within the valid range.
                if row_idx < kit_col.slot_count {
                    let name = extract_sysex_name(&blob).unwrap_or("--".into());
                    // Skip empty kit slots in the backup.
                    if name == "--" {
                        continue;
                    }
                    if let Some(item) = kit_col.model.item(row_idx as u32).and_downcast::<LibrarianItem>() {
                        item.set_name(name.as_str());
                    }
                    kit_col.blobs.lock().unwrap()[row_idx] = Some(blob);
                    self.cols.get_mut("kit").unwrap().dirty.lock().unwrap().insert(row_idx);
                    *imported.entry("kit").or_default() += 1;
                }
            }
            // Map Pattern dump messages to Librarian slots.
            else if cmd == pattern_dump {
                let pattern_col = &self.cols["pattern"];
                let row_idx = slot.wrapping_sub(pattern_col.first_slot);
                // Verify the target slot is within the valid range.
                if row_idx < pattern_col.slot_count {
                    let name = if is_empty_pattern(&blob) {
                        "--".to_string()
                    } else {
                        pattern_slot_label(slot)
                    };
                    if let Some(item) = pattern_col.model.item(row_idx as u32).and_downcast::<LibrarianItem>() {
                        item.set_name(name.as_str());
                    }
                    pattern_col.blobs.lock().unwrap()[row_idx] = Some(blob);
                    self.cols.get_mut("pattern").unwrap().dirty.lock().unwrap().insert(row_idx);
                    *imported.entry("pattern").or_default() += 1;
                }
            }
            // Map Song dump messages to Librarian slots.
            else if cmd == song_dump {
                let song_col = &self.cols["song"];
                let row_idx = slot.wrapping_sub(song_col.first_slot);
                // Verify the target slot is within the valid range.
                if row_idx < song_col.slot_count {
                    let name = extract_sysex_name(&blob).unwrap_or("--".into());
                    // Skip empty song slots in the backup.
                    if name == "--" {
                        continue;
                    }
                    if let Some(item) = song_col.model.item(row_idx as u32).and_downcast::<LibrarianItem>() {
                        item.set_name(name.as_str());
                    }
                    song_col.blobs.lock().unwrap()[row_idx] = Some(blob);
                    self.cols.get_mut("song").unwrap().dirty.lock().unwrap().insert(row_idx);
                    *imported.entry("song").or_default() += 1;
                }
            }
            // Map Global settings messages to Librarian slots.
            else if cmd == global_dump {
                let global_col = &self.cols["global"];
                let row_idx = slot.wrapping_sub(global_col.first_slot);
                // Verify the target slot is within the valid range.
                if row_idx < global_col.slot_count {
                    let slot_label = format!("Global {:02}", slot + global_col.display_offset);
                    if let Some(item) = global_col.model.item(row_idx as u32).and_downcast::<LibrarianItem>() {
                        item.set_name(slot_label.as_str());
                    }
                    global_col.blobs.lock().unwrap()[row_idx] = Some(blob);
                    self.cols.get_mut("global").unwrap().dirty.lock().unwrap().insert(row_idx);
                    *imported.entry("global").or_default() += 1;
                }
            }
            // Map DigiPro waveform dump messages to Librarian slots.
            else if digipro_dump != 0 && cmd == digipro_dump {
                let digipro_col = &self.cols["digipro"];
                let row_idx = slot.wrapping_sub(digipro_col.first_slot);
                if row_idx < digipro_col.slot_count {
                    let raw_name = digipro_name(&blob, &self.dc);

                    let name = if raw_name.is_empty() {
                        format!("DigiPro {:02}", slot + digipro_col.display_offset)
                    } else {
                        raw_name
                    };

                    if let Some(item) = digipro_col.model.item(row_idx as u32).and_downcast::<LibrarianItem>() {
                        item.set_name(name.as_str());
                    }
                    digipro_col.blobs.lock().unwrap()[row_idx] = Some(blob);
                    self.cols.get_mut("digipro").unwrap().dirty.lock().unwrap().insert(row_idx);
                    *imported.entry("digipro").or_default() += 1;
                }
            }
            // Samples have no equivalent branch here.
            // MD's `sysex_api.sample.write_cmd` is 0 (samples go through the SDS handshake, not a single dump command).
        }

        // Refresh UI controls for each category after the bulk merge.
        for item_type in ALL_ITEM_TYPES {
            self.refresh_write_btn(item_type);
        }
        // Only types carrying a dump command can appear in a `.syx`, which is the same test the branches above match on.
        let parts: Vec<String> = ALL_ITEM_TYPES
            .into_iter()
            .filter(|item_type| self.cols[*item_type].dump_cmd != 0)
            .map(|item_type| format!("{} {item_type}s", imported.get(item_type).copied().unwrap_or(0)))
            .collect();
        (self.update_status)(&format!("Status: Imported SYX: {}.", parts.join(", ")));
    }

    // ── Write background thread ────────────────────────────────────────────────────────────────────────────

    /// Background thread body for all write operations (kits, patterns, songs, samples, DigiPro, globals).
    async fn write_slots_to_device(
        port: String,
        blobs_ref: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
        orig_ref: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
        dirty_ref: Arc<Mutex<HashSet<usize>>>,
        dirty: Vec<usize>,
        slot_names: Vec<String>,
        model_wk: glib::SendWeakRef<gio::ListStore>,
        update_status: StatusFn,
        btn_wk: glib::SendWeakRef<Button>,
        progress_bar_wk: glib::SendWeakRef<gtk4::ProgressBar>,
        all_fetch_wk: Vec<glib::SendWeakRef<Button>>,
        all_write_wk: Vec<glib::SendWeakRef<Button>>,
        active: Arc<std::sync::atomic::AtomicBool>,
        ch: u8,
        dc: Arc<DeviceConfig>,
        item_type: String,
    ) {
        let item_type_cap = cap_first_char(&item_type);
        let first_slot = as_u64_or(dc.json_get(&format!("sysex_api.{item_type}.first_slot")), 0) as usize;
        let display_offset = as_u64_or(dc.json_get(&format!("sysex_api.{item_type}.display_offset")), 0) as usize;
        let dump_cmd = as_u64_or(dc.json_get(&format!("sysex_api.{item_type}.write_cmd")), 0) as u8;
        let arm_type = as_u64_or(dc.json_get(&format!("sysex_api.{item_type}.write_arm_type")), 0) as u8;
        let write_arm_cmd = as_u64_or_die(dc.json_get("sysex_api.write_arm.cmd")) as u8;

        let total = dirty.len();
        let written = Arc::new(Mutex::new(Vec::new()));

        let (tx, rx) = async_channel::unbounded::<WriteUpdate>();
        let progress_bar_wk_main = progress_bar_wk.clone();
        let model_wk_main = model_wk.clone();
        let update_status_c = Arc::clone(&update_status);
        glib::spawn_future_local(async move {
            while let Ok(upd) = rx.recv().await {
                match upd {
                    WriteUpdate::Status(msg) => update_status_c(&msg),
                    WriteUpdate::Progress(p) => {
                        if let Some(progress_bar) = progress_bar_wk_main.upgrade() {
                            progress_bar.set_fraction(p);
                        }
                    }
                    WriteUpdate::ItemDirty(idx, dirty) => {
                        if let Some(model) = model_wk_main.upgrade()
                            && let Some(item) = model.item(idx as u32).and_downcast::<LibrarianItem>()
                        {
                            item.set_dirty(dirty);
                        }
                    }
                }
            }
        });

        let tx_status = tx.clone();
        let active_check = active.clone();
        let blobs_ref_c = blobs_ref.clone();
        let dirty_ref_c = dirty_ref.clone();
        let orig_ref_c = orig_ref.clone();
        let item_type_c = item_type.clone();
        let item_type_cap_c = item_type_cap.clone();

        let result: Result<(), String> = async {
            if item_type_c == "sample" || item_type_c == "digipro" {
                let written_slots = written.clone();
                run_midi_session(&port, move |midi_in, midi_out| -> Result<(), String> {
                    for (i, &idx) in dirty.iter().enumerate() {
                        if !active_check.load(Ordering::Relaxed) {
                            break;
                        }
                        let slot = (first_slot + idx) as u8;
                        let display_slot = slot as usize + display_offset;
                        let blob_opt = blobs_ref_c.lock().unwrap()[idx].clone();
                        let Some(blob) = blob_opt else { continue };
                        let frac = (i + 1) as f64 / total as f64;
                        let _ = tx_status.send_blocking(WriteUpdate::Status(format!(
                            "Status: Writing {item_type_cap_c} {display_slot:02} ({}/{total})...",
                            i + 1
                        )));
                        let _ = tx_status.send_blocking(WriteUpdate::Progress(frac));

                        if item_type_c == "digipro" {
                            midi_out.sysex(&blob);
                            written_slots.lock().unwrap().push(idx);
                            dirty_ref_c.lock().unwrap().remove(&idx);
                            orig_ref_c.lock().unwrap()[idx] = Some(blob);
                            let _ = tx_status.send_blocking(WriteUpdate::ItemDirty(idx, false));
                            sleep(Duration::from_millis(150));
                        } else {
                            let Ok(blob_str) = String::from_utf8(blob.clone()) else {
                                continue;
                            };
                            let sds_data = decode_flac_b64_to_sds(&blob_str)?;
                            let custom_name = slot_names.get(i).cloned().unwrap_or_else(|| format!("{display_slot:02}"));
                            let active_check_c2 = active_check.clone();
                            let cancel_cb = move || !active_check_c2.load(Ordering::Relaxed);
                            let packets = build_sds_sample_packets(&sds_data, slot, ch, &dc, &custom_name, Some(&cancel_cb));
                            if !active_check.load(Ordering::Relaxed) {
                                break;
                            }
                            let tx_prog = tx_status.clone();
                            let tx_stat = tx_status.clone();
                            let cancel_cb_send = || !active_check.load(Ordering::Relaxed);
                            let status_cb = move |msg: &str| {
                                let _ = tx_stat.send_blocking(WriteUpdate::Status(format!("Status: {msg}")));
                            };
                            let progress_cb = move |packet_idx: usize, total_packets: usize| {
                                let frac = if total_packets > 0 {
                                    packet_idx as f64 / total_packets as f64
                                } else {
                                    0.0
                                };
                                let _ = tx_prog.send_blocking(WriteUpdate::Progress(frac));
                            };
                            let ok = send_sds_sample_packets(
                                midi_in,
                                midi_out,
                                &packets,
                                Some(&cancel_cb_send),
                                Some(&status_cb),
                                Some(&progress_cb),
                                &custom_name,
                            );
                            if !ok {
                                let _ = tx_status.send_blocking(WriteUpdate::Status(format!(
                                    "Status: Transfer cancelled or failed for sample {display_slot:02}."
                                )));
                                break;
                            }
                            written_slots.lock().unwrap().push(idx);
                            dirty_ref_c.lock().unwrap().remove(&idx);
                            orig_ref_c.lock().unwrap()[idx] = Some(blob);
                            let _ = tx_status.send_blocking(WriteUpdate::ItemDirty(idx, false));
                            sleep(Duration::from_millis(200));
                            midi_in.flush();
                        }
                    }
                    Ok(())
                })
                .await?;
            } else if item_type_c == "pattern" {
                // Patterns require `send_large_sysex()` to avoid ALSA segmentation on MD.
                // That path runs on a worker thread (and writes rawmidi directly on Linux).
                // So the loop lives here, not in a `run_midi_output()` closure.
                for (i, &idx) in dirty.iter().enumerate() {
                    if !active_check.load(Ordering::Relaxed) {
                        break;
                    }
                    let slot = (first_slot + idx) as u8;
                    let blob_opt = blobs_ref_c.lock().unwrap()[idx].clone();
                    let Some(blob) = blob_opt else { continue };
                    let slot_label = pattern_slot_label(first_slot + idx);
                    let frac = (i + 1) as f64 / total as f64;
                    let _ = tx_status.send_blocking(WriteUpdate::Status(format!(
                        "Status: Writing Pattern {slot_label} ({}/{total})...",
                        i + 1
                    )));
                    let _ = tx_status.send_blocking(WriteUpdate::Progress(frac));

                    let mut packets = parse_sysex_file(&blob);
                    if packets.len() == 1 {
                        let packet = &mut packets[0];
                        if packet.len() > 10 && packet[ELEKTRON_TYPE_BYTE] == dump_cmd {
                            patch_elektron_original_position(packet, 9, slot);
                        }
                    }
                    let arm = build_elektron_sysex(dc.prod, ch, &[write_arm_cmd, arm_type, slot & 0x7F, 0x01]);
                    for packet in packets {
                        send_large_sysex(&port, arm.clone(), packet, 0.0, 0.0).await;
                    }

                    written.lock().unwrap().push(idx);
                    dirty_ref_c.lock().unwrap().remove(&idx);
                    orig_ref_c.lock().unwrap()[idx] = Some(blob);
                    let _ = tx_status.send_blocking(WriteUpdate::ItemDirty(idx, false));
                }
            } else {
                let written_for_output = written.clone();
                run_midi_output(&port, move |midi_out| -> Result<(), String> {
                    for (i, &idx) in dirty.iter().enumerate() {
                        if !active_check.load(Ordering::Relaxed) {
                            break;
                        }
                        let slot = (first_slot + idx) as u8;
                        let display_slot = slot as usize + display_offset;
                        let blob_opt = blobs_ref_c.lock().unwrap()[idx].clone();
                        let Some(blob) = blob_opt else { continue };
                        let frac = (i + 1) as f64 / total as f64;
                        let _ = tx_status.send_blocking(WriteUpdate::Status(format!(
                            "Status: Writing {item_type_cap_c} {display_slot:02} ({}/{total})...",
                            i + 1
                        )));
                        let _ = tx_status.send_blocking(WriteUpdate::Progress(frac));

                        let mut packets = parse_sysex_file(&blob);
                        if packets.len() == 1 {
                            let packet = &mut packets[0];
                            if packet.len() > 10 && packet[ELEKTRON_TYPE_BYTE] == dump_cmd {
                                patch_elektron_original_position(packet, 9, slot); // byte 7 is the version marker, not a slot
                            }
                        }

                        if item_type_c == "global" {
                            for packet in &packets {
                                midi_out.sysex(packet);
                                if packets.len() > 1 {
                                    sleep(Duration::from_millis(50));
                                }
                            }
                        } else {
                            let arm = build_elektron_sysex(dc.prod, ch, &[write_arm_cmd, arm_type, slot & 0x7F, 0x01]);
                            midi_out.sysex(&arm);
                            sleep(Duration::from_millis(300));
                            for packet in &packets {
                                midi_out.sysex(packet);
                                if packets.len() > 1 {
                                    sleep(Duration::from_millis(50));
                                }
                            }
                        }

                        written_for_output.lock().unwrap().push(idx);
                        dirty_ref_c.lock().unwrap().remove(&idx);
                        orig_ref_c.lock().unwrap()[idx] = Some(blob);
                        let _ = tx_status.send_blocking(WriteUpdate::ItemDirty(idx, false));
                        sleep(Duration::from_millis(200));
                    }
                    Ok(())
                })
                .await?;
            }
            Ok(())
        }
        .await;

        active.store(false, Ordering::Relaxed);
        let num_written = written.lock().unwrap().len();
        let num_remaining = total - num_written;
        let msg = match result {
            Ok(()) if num_written > 0 => {
                format!("Status: Wrote {num_written}/{total} {item_type}(s).")
            }
            Ok(()) => format!("Status: {item_type_cap} write cancelled or failed."),
            Err(e) => format!("Error writing {item_type}s: {e}"),
        };
        update_status(&msg);
        if let Some(progress_bar) = progress_bar_wk.upgrade() {
            progress_bar.set_visible(false);
            progress_bar.set_fraction(0.0);
        }
        if let Some(btn) = btn_wk.upgrade() {
            btn.remove_css_class("destructive-action");

            if num_remaining > 0 {
                btn.set_label(&format!("Write {num_remaining} {item_type}s to Device"));
                btn.set_sensitive(true);
            } else {
                btn.set_label(&format!("Write {item_type}s to Device"));
                btn.set_sensitive(false);
            }
        }
        for fetch_btn_wk in &all_fetch_wk {
            if let Some(fetch_btn) = fetch_btn_wk.upgrade() {
                fetch_btn.set_sensitive(true);
            }
        }
        for write_btn_wk in &all_write_wk {
            if let Some(write_btn) = write_btn_wk.upgrade() {
                write_btn.set_sensitive(true);
            }
        }
    }

    /// Fetches one sample slot from the device and writes it to a `.c7` file.
    async fn export_sample(
        _idx: usize,
        file_path: String,
        slot_label: String,
        dc: Arc<DeviceConfig>,
        ch: u8,
        slot: u8,
        port: Option<String>,
        update_status: StatusFn,
    ) {
        let name_fallback = format!("SAMPLE_{slot_label}");
        let global_meta = generate_c7_header("sample", &dc.device_short, None);

        let port_name = match port {
            Some(port_name) if is_valid_port(&port_name) => port_name,
            _ => {
                update_status("Error: Valid MIDI ports needed for sample export.");
                return;
            }
        };

        let (tx, rx) = async_channel::unbounded::<String>();
        let update_status_c = Arc::clone(&update_status);
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                update_status_c(&msg);
            }
        });

        let result = run_midi_session(&port_name, move |midi_in, midi_out| {
            let val = fetch_and_compress_sample(midi_in, midi_out, slot, &dc, ch, move |msg: &str| {
                let _ = tx.send_blocking(format!("Status: {msg}"));
            })
            .0;
            Ok(val)
        })
        .await;

        match result {
            Ok(Some(blob_b64)) => {
                let mut attrs = BTreeMap::new();
                attrs.insert("type".to_string(), "sample".to_string());
                attrs.insert("name".to_string(), name_fallback);
                attrs.insert("format".to_string(), "flac+base64".to_string());
                let section = wire_slot_to_section("sample", usize::from(slot));
                let item = C7Item {
                    section,
                    data: Some(C7Data::Text(blob_b64)),
                    attrs,
                };
                write_c7_file(&file_path, &[item], Some(&global_meta));
                let basename = std::path::Path::new(&file_path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                update_status(&format!("Status: Exported sample to {basename}"));
            }
            Ok(None) => {
                update_status(&format!("Error: No audio data received for sample {slot_label}."));
            }
            Err(e) => {
                update_status(&format!("Error: Failed to compress/save sample: {e}"));
            }
        }
    }

    /// Writes every already-fetched kit/pattern/song/sample/DigiPro/global blob to a single `.c7` backup file.
    async fn export_backup_c7(
        file_path: String,
        dc: Arc<DeviceConfig>,
        port: Option<String>,
        ch: u8,
        update_status: StatusFn,
        kit_blobs: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
        _kit_names: Vec<String>,
        pattern_blobs: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
        _pattern_names: Vec<String>,
        song_blobs: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
        _song_names: Vec<String>,
        sample_blobs: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
        sample_names: Vec<String>,
        digipro_blobs: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
        digipro_names: Vec<String>,
        global_blobs: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
    ) {
        let kit_first = as_u64_or(dc.json_get("sysex_api.kit.first_slot"), 0) as usize;
        let pattern_first = as_u64_or(dc.json_get("sysex_api.pattern.first_slot"), 0) as usize;
        let song_first = as_u64_or(dc.json_get("sysex_api.song.first_slot"), 0) as usize;
        let sample_first = as_u64_or(dc.json_get("sysex_api.sample.first_slot"), 0) as usize;
        let sample_offset = as_u64_or(dc.json_get("sysex_api.sample.display_offset"), 0) as usize;
        let digipro_first = as_u64_or(dc.json_get("sysex_api.digipro.first_slot"), 0) as usize;
        let global_first = as_u64_or(dc.json_get("sysex_api.global.first_slot"), 0) as usize;

        let (tx, rx) = async_channel::unbounded::<String>();
        let update_status_c = Arc::clone(&update_status);
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                update_status_c(&msg);
            }
        });

        let status_cb = {
            let tx = tx.clone();
            move |msg: &str| {
                let _ = tx.send_blocking(format!("Status: {msg}"));
            }
        };

        let sample_snap = sample_blobs.lock().unwrap().clone();
        let needs_ports = sample_snap.iter().any(std::option::Option::is_some);
        let mut should_open_session = false;
        if needs_ports && dc.is_device("MD") {
            if port.as_deref().is_some_and(is_valid_port) {
                should_open_session = true;
            } else {
                status_cb("Error: Valid MIDI ports needed for full sample backup.");
                return;
            }
        }

        let metadata = generate_c7_header("bundle", &dc.device_short, None);
        let mut items: Vec<C7Item> = Vec::new();
        let mut kits_exported = 0usize;
        let mut patterns_exported = 0usize;
        let mut songs_exported = 0usize;
        let mut samples_exported = 0usize;
        let mut globals_exported = 0usize;

        let kit_snap = kit_blobs.lock().unwrap().clone();
        for (idx, blob_opt) in kit_snap.iter().enumerate() {
            let Some(blob) = blob_opt else { continue };
            let name = extract_sysex_name(blob).unwrap_or("--".into());
            if name == "--" {
                continue;
            }
            let slot = kit_first + idx;
            let section = wire_slot_to_section("kit", slot);
            let mut attrs = BTreeMap::new();
            attrs.insert("type".to_string(), "kit".to_string());
            attrs.insert("format".to_string(), "sysex".to_string());
            if !name.is_empty() {
                attrs.insert("name".to_string(), name);
            }
            items.push(C7Item {
                section,
                data: Some(C7Data::Binary(blob.clone())),
                attrs,
            });
            kits_exported += 1;
        }

        let pattern_snap = pattern_blobs.lock().unwrap().clone();
        for (idx, blob_opt) in pattern_snap.iter().enumerate() {
            let Some(blob) = blob_opt else { continue };
            if is_empty_pattern(blob) {
                continue;
            }
            let slot = pattern_first + idx;
            let section = wire_slot_to_section("pattern", slot);
            let mut attrs = BTreeMap::new();
            attrs.insert("type".to_string(), "pattern".to_string());
            attrs.insert("format".to_string(), "sysex".to_string());
            items.push(C7Item {
                section,
                data: Some(C7Data::Binary(blob.clone())),
                attrs,
            });
            patterns_exported += 1;
        }

        let song_snap = song_blobs.lock().unwrap().clone();
        for (idx, blob_opt) in song_snap.iter().enumerate() {
            let Some(blob) = blob_opt else { continue };
            let name = extract_sysex_name(blob).unwrap_or("--".into());
            if name == "--" {
                continue;
            }
            let slot = song_first + idx;
            let section = wire_slot_to_section("song", slot);
            let mut attrs = BTreeMap::new();
            attrs.insert("type".to_string(), "song".to_string());
            attrs.insert("format".to_string(), "sysex".to_string());
            if !name.is_empty() {
                attrs.insert("name".to_string(), name);
            }
            items.push(C7Item {
                section,
                data: Some(C7Data::Binary(blob.clone())),
                attrs,
            });
            songs_exported += 1;
        }

        if should_open_session {
            let port_name = port.as_deref().unwrap_or("");
            let tx_c = tx.clone();
            let sample_names_c = sample_names.clone();
            let sample_snap_c = sample_snap.clone();

            match run_midi_session(port_name, move |midi_in, midi_out| -> Result<(Vec<C7Item>, usize), String> {
                let mut samples_exported = 0;
                for (idx, blob_opt) in sample_snap_c.iter().enumerate() {
                    let Some(_) = blob_opt else { continue };
                    let name = sample_names_c.get(idx).cloned().unwrap_or_default();
                    if name.is_empty() || name == "--" {
                        continue;
                    }
                    let slot = (sample_first + idx) as u8;
                    let slot_label = format!("{:02}", slot as usize + sample_offset);
                    let tx_c2 = tx_c.clone();
                    let cb = move |msg: &str| {
                        let _ = tx_c2.send_blocking(format!("Status: {msg}"));
                    };
                    let (blob_b64, _) = fetch_and_compress_sample(midi_in, midi_out, slot, &dc, ch, cb);

                    if let Some(b64) = blob_b64 {
                        let section = wire_slot_to_section("sample", usize::from(slot));
                        let mut attrs = BTreeMap::new();
                        attrs.insert("type".to_string(), "sample".to_string());
                        attrs.insert("name".to_string(), name);
                        attrs.insert("format".to_string(), "flac+base64".to_string());
                        items.push(C7Item {
                            section,
                            data: Some(C7Data::Text(b64)),
                            attrs,
                        });
                        samples_exported += 1;
                    } else {
                        let _ = tx_c.send_blocking(format!("Warning: No audio data for sample {slot_label}."));
                    }
                }
                Ok((items, samples_exported))
            })
            .await
            {
                Ok((new_items, new_samples)) => {
                    items = new_items;
                    samples_exported = new_samples;
                }
                Err(e) => {
                    status_cb(&format!("Backup export failed: {e}"));
                    return;
                }
            }
        } else {
            for (idx, blob_opt) in sample_snap.iter().enumerate() {
                let Some(blob) = blob_opt else { continue };
                let name = sample_names.get(idx).cloned().unwrap_or_default();
                if name.is_empty() || name == "--" {
                    continue;
                }
                let slot = sample_first + idx;
                let section = wire_slot_to_section("sample", slot);
                let mut attrs = BTreeMap::new();
                attrs.insert("type".to_string(), "sample".to_string());
                attrs.insert("name".to_string(), name);
                let (data, is_text) = if let Ok(text) = String::from_utf8(blob.clone()) {
                    (C7Data::Text(text), true)
                } else {
                    (C7Data::Binary(blob.clone()), false)
                };
                attrs.insert(
                    "format".to_string(),
                    if is_text { "flac+base64".to_string() } else { "sysex".to_string() },
                );
                items.push(C7Item {
                    section,
                    data: Some(data),
                    attrs,
                });
                samples_exported += 1;
            }
        }

        let digipro_snap = digipro_blobs.lock().unwrap().clone();
        for (idx, blob_opt) in digipro_snap.iter().enumerate() {
            let Some(blob) = blob_opt else { continue };
            let name = digipro_names.get(idx).cloned().unwrap_or_default();
            if name.is_empty() || name == "--" {
                continue;
            }
            let slot = digipro_first + idx;
            let section = wire_slot_to_section("digipro", slot);
            let mut attrs = BTreeMap::new();
            attrs.insert("type".to_string(), "digipro".to_string());
            attrs.insert("format".to_string(), "sysex".to_string());
            attrs.insert("name".to_string(), name);
            items.push(C7Item {
                section,
                data: Some(C7Data::Binary(blob.clone())),
                attrs,
            });
            samples_exported += 1;
        }

        let global_snap = global_blobs.lock().unwrap().clone();
        for (idx, blob_opt) in global_snap.iter().enumerate() {
            let Some(blob) = blob_opt else { continue };
            let slot = global_first + idx;
            let section = wire_slot_to_section("global", slot);
            let mut attrs = BTreeMap::new();
            attrs.insert("type".to_string(), "global".to_string());
            attrs.insert("format".to_string(), "sysex".to_string());
            items.push(C7Item {
                section,
                data: Some(C7Data::Binary(blob.clone())),
                attrs,
            });
            globals_exported += 1;
        }

        write_c7_file(&file_path, &items, Some(&metadata));
        let msg = format!(
            "Exported backup: {kits_exported} kits, {patterns_exported} patterns, \
            {songs_exported} songs, {samples_exported} samples, {globals_exported} globals."
        );
        status_cb(&msg);
    }
}
