//! Modal for browsing saved sounds.
//!
//! Not reachable from the device menu. It opens from the `kit_editor` screen.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gio::prelude::*;
use glib::subclass::prelude::ObjectSubclassIsExt;
use gtk4::{
    self, Align, Box as GtkBox, Button, ColumnView, ColumnViewColumn, Entry, GestureClick, Label, ListBox, ListBoxRow, Orientation, Paned,
    Popover, ScrolledWindow, SelectionMode, Separator, SignalListItemFactory, SingleSelection, gdk, glib,
};
use libadwaita::prelude::*;

use c7_core::c7_file_interfacing::{
    C7Data, find_c7_item, md_sound_from_json, mnm_sound_from_json, read_c7_or_sysex_file, rename_c7_item, section_to_wire_slot,
};
use c7_core::device_config::{DeviceConfig, get_bank_dir};

const CATEGORY_ALL: &str = "All";
const CATEGORY_NONE: &str = "Uncategorized";

// ── Data model ─────────────────────────────────────────────────────────────────────────────────────────────

mod imp {
    use super::{Cell, RefCell, glib};
    use glib::prelude::*;
    use glib::subclass::prelude::*;

    /// `GObject` wrapper for one scanned sound file, required by `gio::ListStore`.
    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::SoundItem)]
    pub struct SoundItem {
        #[property(get, set)]
        pub name: RefCell<String>,
        #[property(get, set)]
        pub comment: RefCell<String>,
        #[property(get, set)]
        pub sound_type: RefCell<String>,
        #[property(get, set)]
        pub file_path: RefCell<String>,
        /// Reconstructed sound parameter blob.
        pub data: RefCell<Vec<u8>>,
        /// 0-indexed track slot this data was saved from, or `None` when the section name is unparsable.
        pub source_track: Cell<Option<usize>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SoundItem {
        const NAME: &'static str = "SoundItem";
        type Type = super::SoundItem;
    }

    impl ObjectImpl for SoundItem {
        /// Returns the list of properties for the `SoundItem` object.
        fn properties() -> &'static [glib::ParamSpec] {
            Self::derived_properties()
        }
        /// Sets the value of a property on the `SoundItem` object.
        fn set_property(&self, id: usize, val: &glib::Value, pspec: &glib::ParamSpec) {
            self.derived_set_property(id, val, pspec);
        }
        /// Retrieves the value of a property from the `SoundItem` object.
        fn property(&self, id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            self.derived_property(id, pspec)
        }
    }
}

glib::wrapper! {
    /// `GObject` wrapper for one scanned sound file, required by `gio::ListStore`.
pub struct SoundItem(ObjectSubclass<imp::SoundItem>);
}

impl SoundItem {
    /// Initializes a sound entry with its metadata and reconstructed parameter blob.
    pub fn new(name: &str, comment: &str, sound_type: &str, data: Vec<u8>, file_path: &str, source_track: Option<usize>) -> Self {
        let obj: Self = glib::Object::builder()
            .property("name", name)
            .property("comment", comment)
            .property("sound-type", sound_type)
            .property("file-path", file_path)
            .build();
        *obj.imp().data.borrow_mut() = data;
        obj.imp().source_track.set(source_track);
        obj
    }

    /// Returns the reconstructed parameter blob for this sound entry.
    pub fn data(&self) -> Vec<u8> {
        self.imp().data.borrow().clone()
    }
    /// Returns the 0-indexed track slot this data was saved from.
    fn source_track(&self) -> Option<usize> {
        self.imp().source_track.get()
    }
}

// ── Scanning ───────────────────────────────────────────────────────────────────────────────────────────────

/// Parses a C7 section name like `"sound_03"` into a 0-indexed track slot (2).
///
/// Returns `None` if the section name can't be parsed.
fn section_to_track_idx(section: &str) -> Option<usize> {
    section_to_wire_slot(section, "sound")
}

/// Walks the bank's {`device_shorter`}/ folder and returns a list of `SoundItem` objects.
///
/// Files that are unreadable or contain no sound item are skipped.
fn scan_sounds(device_shorter: &str) -> Vec<SoundItem> {
    let scan_root = get_bank_dir().join(device_shorter);
    if !scan_root.is_dir() {
        return vec![];
    }
    let mut entries = Vec::new();
    walk_for_sounds(&scan_root, &mut entries);
    entries
}

/// Recursively walks `dir`, collecting `SoundItem` objects from `.c7` files.
fn walk_for_sounds(dir: &Path, out: &mut Vec<SoundItem>) {
    let mut items: Vec<_> = match std::fs::read_dir(dir) {
        Ok(read_dir) => read_dir.filter_map(std::result::Result::ok).collect(),
        Err(_) => return,
    };
    items.sort_by_key(std::fs::DirEntry::file_name);
    for entry in items {
        let path = entry.path();
        if path.is_dir() {
            walk_for_sounds(&path, out);
            continue;
        }
        let extension = path
            .extension()
            .and_then(|extension_val| extension_val.to_str())
            .map(str::to_lowercase);
        let extension_str = extension.as_deref();
        if extension_str != Some("c7") && extension_str != Some("syx") && extension_str != Some("sysex") {
            continue;
        }
        let c7_items = read_c7_or_sysex_file(&path);
        let Some(item) = find_c7_item(&c7_items, Some("sound"), None) else {
            continue;
        };
        // Reconstruct binary blob from either raw Binary or structured Object (JSON param map).
        let data = match &item.data {
            Some(C7Data::Binary(bytes)) if !bytes.is_empty() => bytes.clone(),
            Some(C7Data::Object(json)) => {
                let device_name = c7_items
                    .iter()
                    .find(|item| item.section == "c7")
                    .and_then(|item| item.get("device"))
                    .unwrap_or("");
                let Some(dc) = DeviceConfig::find_by_name(device_name) else {
                    continue;
                };

                if dc.is_device("MnM") {
                    mnm_sound_from_json(json, &dc)
                } else {
                    md_sound_from_json(json, &dc)
                }
            }
            _ => continue,
        };
        let name = item.get("name").filter(|name_str| !name_str.is_empty()).map_or_else(
            || path.file_stem().and_then(|stem_str| stem_str.to_str()).unwrap_or("").to_string(),
            std::string::ToString::to_string,
        );
        let comment = item.get("comment").unwrap_or("").to_string();
        let sound_type = item.get("sound_type").unwrap_or("").to_string();
        let file_path = path.to_string_lossy().into_owned();
        let source_track = section_to_track_idx(&item.section);
        out.push(SoundItem::new(&name, &comment, &sound_type, data, &file_path, source_track));
    }
}

/// Backing state for the sound browser modal.
///
/// Holds the scanned entries, the category filter in force, and the widgets the browser updates.
struct SoundBrowserState {
    all_entries: Vec<SoundItem>,
    /// Filter per `category_list` row, in the same order; `None` = All, `Some("")` = Uncategorized, `Some(x)` = type x.
    row_filters: Vec<Option<String>>,
    current_category_filter: Option<String>,
    model: gio::ListStore,
    status_label: Label,
    category_list: ListBox,
    load_btn: Button,
    device_shorter: String,
}

/// Browser for saved sound `.c7` files.
///
/// Left panel: category list built from `sound_type` values.
/// Right panel: `ColumnView` list of matching sounds (name + comment).
pub(crate) struct SoundBrowserModal {
    pub window: libadwaita::Window,
    /// Holds the original sound bytes fetched before any preview so the browser can revert.
    pub original_data: Arc<Mutex<Option<Vec<u8>>>>,
}

impl SoundBrowserModal {
    /// Creates the sound browser modal.
    ///
    /// `on_load_cb` is called with the selected sound bytes when the user previews or loads a sound.
    pub(crate) fn new(
        parent: &impl IsA<gtk4::Widget>,
        device_shorter: &str,
        on_load_cb: impl Fn(Vec<u8>, Option<usize>) + 'static,
    ) -> Self {
        let window = libadwaita::Window::new();
        window.set_title(Some("Sound Browser"));
        window.set_default_size(680, 480);
        if let Some(root) = parent.root().and_downcast::<gtk4::Window>() {
            window.set_transient_for(Some(&root));
            window.set_modal(true);
        }

        let model = gio::ListStore::new::<SoundItem>();
        let selection = SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);

        let status_label = Label::builder()
            .label("Scanning...")
            .css_classes(["dim-label"])
            .halign(Align::Start)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        let category_list = ListBox::new();
        category_list.add_css_class("navigation-sidebar");
        category_list.set_selection_mode(SelectionMode::Single);
        let load_btn = Button::with_label("Load");
        load_btn.add_css_class("suggested-action");
        load_btn.set_sensitive(false);

        let on_load_rc: Rc<dyn Fn(Vec<u8>, Option<usize>)> = Rc::new(on_load_cb);
        let original_data: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));

        let state = Rc::new(RefCell::new(SoundBrowserState {
            all_entries: Vec::new(),
            row_filters: Vec::new(),
            current_category_filter: None,
            model,
            status_label: status_label.clone(),
            category_list: category_list.clone(),
            load_btn: load_btn.clone(),
            device_shorter: device_shorter.to_string(),
        }));

        let content = Self::build_content(
            &status_label,
            &category_list,
            &selection,
            &load_btn,
            &state,
            &on_load_rc,
            &original_data,
            &window,
        );

        let toolbar_view = libadwaita::ToolbarView::new();
        toolbar_view.add_top_bar(&libadwaita::HeaderBar::new());
        toolbar_view.set_content(Some(&content));
        window.set_content(Some(&toolbar_view));

        // Wire selection-changed for live preview on every selection change.
        {
            let state_c = Rc::clone(&state);
            let on_load_rc_c = Rc::clone(&on_load_rc);
            selection.connect_selection_changed(move |selection, _, _| {
                let entry = selection.selected_item().and_downcast::<SoundItem>();
                state_c.borrow().load_btn.set_sensitive(entry.is_some());
                if let Some(entry) = entry {
                    on_load_rc_c(entry.data(), entry.source_track());
                }
            });
        }

        // Defer directory scan so the window can render first.
        {
            let state_c = Rc::clone(&state);
            glib::spawn_future_local(async move {
                load_entries(&state_c);
            });
        }

        SoundBrowserModal { window, original_data }
    }

    /// Assembles the two-panel browser layout containing categories and sound lists.
    fn build_content(
        status_label: &Label,
        category_list: &ListBox,
        selection: &SingleSelection,
        load_btn: &Button,
        state_rc: &Rc<RefCell<SoundBrowserState>>,
        on_load_rc: &Rc<dyn Fn(Vec<u8>, Option<usize>)>,
        original_data: &Arc<Mutex<Option<Vec<u8>>>>,
        window: &libadwaita::Window,
    ) -> GtkBox {
        let outer = GtkBox::new(Orientation::Vertical, 0);
        outer.append(status_label);
        outer.append(&Separator::new(Orientation::Horizontal));

        // Two-panel split
        let paned = Paned::new(Orientation::Horizontal);
        paned.set_position(180);
        paned.set_wide_handle(false);
        paned.set_vexpand(true);

        // ── Left: category list ────────────────────────────────────────────────────────────────────────────

        let left_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .build();
        left_scroll.set_size_request(140, -1);
        {
            // Updates the sound list when a new category is selected from the sidebar.
            let state_rc = Rc::clone(state_rc);
            category_list.connect_row_selected(move |_, row| {
                if let Some(row) = row {
                    let row_idx = row.index() as usize;
                    // Look up the filter for this row using the stored parallel Vec.
                    let filter = state_rc.borrow().row_filters.get(row_idx).cloned().flatten();
                    state_rc.borrow_mut().current_category_filter.clone_from(&filter);
                    populate_sounds(&state_rc, filter.as_deref());
                }
            });
        }
        left_scroll.set_child(Some(category_list));
        paned.set_start_child(Some(&left_scroll));
        paned.set_resize_start_child(false);
        paned.set_shrink_start_child(false);

        // ── Right: sound `ColumnView` ──────────────────────────────────────────────────────────────────────

        let right_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .build();
        let column_view = ColumnView::new(Some(selection.clone()));
        column_view.add_css_class("rich-list");
        column_view.set_show_row_separators(true);
        column_view.set_show_column_separators(false);
        column_view.set_reorderable(false);
        {
            // Confirms the selection and closes the browser upon row activation (double-click).
            let state_rc = Rc::clone(state_rc);
            let window = window.clone();
            column_view.connect_activate(move |_, pos| {
                // The preview already fired on `selection-changed`. Activate only confirms and closes.
                if state_rc.borrow().model.item(pos).is_some() {
                    window.close();
                }
            });
        }

        // Name column (expands). Cells carry a right-click gesture.
        let name_factory = SignalListItemFactory::new();
        {
            let state_rc = Rc::clone(state_rc);
            let window = window.clone();
            name_factory.connect_setup(move |_, obj| {
                let item = obj.downcast_ref::<gtk4::ListItem>().unwrap();
                setup_name_cell(item, &state_rc, &window);
            });
        }
        name_factory.connect_bind(|_, obj| {
            let item = obj.downcast_ref::<gtk4::ListItem>().unwrap();
            bind_name_cell(item);
        });
        let name_col = ColumnViewColumn::new(Some("Name"), Some(name_factory));
        name_col.set_expand(true);
        column_view.append_column(&name_col);

        // Comment column (fixed width, dim).
        let comment_factory = SignalListItemFactory::new();
        comment_factory.connect_setup(|_, obj| {
            let label = Label::builder()
                .halign(Align::Start)
                .css_classes(["dim-label"])
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .margin_start(4)
                .margin_end(8)
                .margin_top(6)
                .margin_bottom(6)
                .build();
            let item = obj.downcast_ref::<gtk4::ListItem>().unwrap();
            item.set_child(Some(&label));
        });
        comment_factory.connect_bind(|_, obj| {
            let item = obj.downcast_ref::<gtk4::ListItem>().unwrap();
            if let Some(label) = item.child().and_downcast::<Label>() {
                let text = item
                    .item()
                    .and_downcast::<SoundItem>()
                    .map(|entry| entry.comment())
                    .unwrap_or_default();
                label.set_label(&text);
            }
        });
        let comment_col = ColumnViewColumn::new(Some("Comment"), Some(comment_factory));
        comment_col.set_fixed_width(180);
        column_view.append_column(&comment_col);

        right_scroll.set_child(Some(&column_view));
        paned.set_end_child(Some(&right_scroll));
        paned.set_resize_end_child(true);
        outer.append(&paned);
        outer.append(&Separator::new(Orientation::Horizontal));

        // Button row: Import Sound on left, Cancel/Load on right.
        let btn_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_top(10)
            .margin_bottom(10)
            .margin_start(12)
            .margin_end(12)
            .build();
        let import_btn = Button::with_label("Import Sound");
        {
            let state_rc = Rc::clone(state_rc);
            let on_load_rc = Rc::clone(on_load_rc);
            let window = window.clone();
            import_btn.connect_clicked(move |_| on_import_clicked(&state_rc, &on_load_rc, &window));
        }
        btn_row.append(&import_btn);
        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        btn_row.append(&spacer);

        let cancel_btn = Button::with_label("Cancel");
        {
            let on_load_rc = Rc::clone(on_load_rc);
            let original_data = Arc::clone(original_data);
            let window = window.clone();
            cancel_btn.connect_clicked(move |_| {
                // Discards changes, reverts to original state, and closes the browser.
                if let Ok(lock) = original_data.lock()
                    && let Some(data) = lock.clone()
                {
                    // Original data is already correctly configured, so no LFO patching is needed on revert.
                    on_load_rc(data, None);
                }
                window.close();
            });
        }
        btn_row.append(&cancel_btn);
        // Load button: preview already triggered on selection, so only confirm by closing.
        {
            let window = window.clone();
            load_btn.connect_clicked(move |_| window.close());
        }
        btn_row.append(load_btn);
        outer.append(&btn_row);
        outer
    }

    /// Presents the modal to the user.
    pub(crate) fn present(&self) {
        self.window.present();
    }
}

// ── Cell factories ─────────────────────────────────────────────────────────────────────────────────────────

/// Initializes the widget structure for a name cell in the sound `ColumnView`.
fn setup_name_cell(list_item: &gtk4::ListItem, state_rc: &Rc<RefCell<SoundBrowserState>>, window: &libadwaita::Window) {
    let cell_box = GtkBox::builder().orientation(Orientation::Horizontal).hexpand(true).build();
    let label = Label::builder()
        .halign(Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .margin_start(8)
        .margin_end(4)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    cell_box.append(&label);

    let gesture = GestureClick::new();
    gesture.set_button(3); // right-click only
    {
        let state_rc_c = Rc::clone(state_rc);
        let window_c = window.clone();
        let list_item_c = list_item.clone();
        gesture.connect_pressed(move |gesture, _n, x, y| {
            let entry = list_item_c.item().and_downcast::<SoundItem>();
            // Abort if the interaction was triggered on an invalid or uninitialized list entry.
            let Some(entry) = entry else { return };
            let Some(widget) = gesture.widget() else {
                return;
            };
            show_cell_context_menu(&widget, x, y, &entry, &state_rc_c, &window_c);
        });
    }
    cell_box.add_controller(gesture);
    list_item.set_child(Some(&cell_box));
}

/// Binds sound entry data to the name cell label and attaches event controllers.
fn bind_name_cell(list_item: &gtk4::ListItem) {
    let Some(cell_box) = list_item.child().and_downcast::<GtkBox>() else {
        return;
    };
    if let Some(label) = cell_box.first_child().and_downcast::<Label>() {
        let text = list_item
            .item()
            .and_downcast::<SoundItem>()
            .map(|entry| entry.name())
            .unwrap_or_default();
        label.set_label(&text);
    }
}

// ── Context menu (right-click) ─────────────────────────────────────────────────────────────────────────────

/// Presents a context menu (Rename/Delete) for the right-clicked sound entry.
fn show_cell_context_menu(
    widget: &gtk4::Widget,
    x: f64,
    y: f64,
    entry: &SoundItem,
    state_rc: &Rc<RefCell<SoundBrowserState>>,
    window: &libadwaita::Window,
) {
    let popover = Popover::new();
    popover.set_parent(widget);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

    let menu_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();
    menu_box.set_size_request(120, -1);

    let rename_btn = Button::with_label("Rename");
    rename_btn.add_css_class("flat");
    rename_btn.set_halign(Align::Fill);
    {
        let popover_c = popover.clone();
        let entry_c = entry.clone();
        let state_rc_c = Rc::clone(state_rc);
        let window_c = window.clone();
        rename_btn.connect_clicked(move |_| {
            popover_c.popdown();
            show_rename_modal(&entry_c, &state_rc_c, &window_c);
        });
    }
    menu_box.append(&rename_btn);

    let delete_btn = Button::with_label("Delete");
    delete_btn.add_css_class("flat");
    delete_btn.add_css_class("destructive-action");
    delete_btn.set_halign(Align::Fill);
    {
        let popover_c = popover.clone();
        let entry_c = entry.clone();
        let state_rc_c = Rc::clone(state_rc);
        delete_btn.connect_clicked(move |_| {
            popover_c.popdown();
            show_delete_dialog(&entry_c, &state_rc_c);
        });
    }
    menu_box.append(&delete_btn);

    popover.set_child(Some(&menu_box));
    popover.popup();
}

/// Presents a modal for updating the display name of a sound entry.
fn show_rename_modal(entry: &SoundItem, state_rc: &Rc<RefCell<SoundBrowserState>>, parent: &libadwaita::Window) {
    let dialog = libadwaita::AlertDialog::builder().heading("Rename Sound").build();
    let content = GtkBox::new(Orientation::Vertical, 6);
    content.set_margin_top(4);
    content.set_margin_bottom(4);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let name_entry = Entry::builder().text(entry.name()).activates_default(true).build();
    content.append(&name_entry);

    dialog.set_extra_child(Some(&content));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", "Rename");
    dialog.set_response_appearance("ok", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");

    let entry_c = entry.clone();
    let state_rc_c = Rc::clone(state_rc);
    dialog.choose(Some(parent), gio::Cancellable::NONE, move |response| {
        if response != "ok" {
            return;
        }
        let new_name = name_entry.text().trim().to_string();
        if !new_name.is_empty() && new_name != entry_c.name() {
            rename_c7_item(entry_c.file_path(), &new_name);
            entry_c.set_name(new_name);
            let filter = state_rc_c.borrow().current_category_filter.clone();
            populate_sounds(&state_rc_c, filter.as_deref());
        }
    });
}

/// Presents a confirmation dialog for permanent sound deletion.
fn show_delete_dialog(entry: &SoundItem, state_rc: &Rc<RefCell<SoundBrowserState>>) {
    let parent_window = state_rc.borrow().status_label.root().and_downcast::<gtk4::Window>();
    let dialog = libadwaita::AlertDialog::new(
        Some("Delete Sound?"),
        Some(&format!("\u{201C}{}\u{201D} will be permanently deleted from disk.", entry.name())),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", libadwaita::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    {
        let entry_c = entry.clone();
        let state_rc_c = Rc::clone(state_rc);
        dialog.connect_response(None, move |_, response| {
            if response != "delete" {
                return;
            }
            // Handles the deletion confirmation and removes the sound from disk and UI.
            let _ = std::fs::remove_file(entry_c.file_path());
            let file_path = entry_c.file_path();
            {
                let mut state_mut = state_rc_c.borrow_mut();
                state_mut.all_entries.retain(|entry| entry.file_path() != file_path);
                let count = state_mut.all_entries.len();

                let msg = if count > 0 {
                    format!("{} sound{} found", count, if count == 1 { "" } else { "s" })
                } else {
                    "No sounds found. Use 'Export Sound' to save sounds first".to_string()
                };
                state_mut.status_label.set_label(&msg);
            }
            rebuild_categories(&state_rc_c);
        });
    }
    dialog.present(parent_window.as_ref());
}

// ── Data loading ───────────────────────────────────────────────────────────────────────────────────────────

/// Triggers the directory scan and populates the internal entry list.
fn load_entries(state_rc: &Rc<RefCell<SoundBrowserState>>) {
    let device_shorter = state_rc.borrow().device_shorter.clone();
    let entries = scan_sounds(&device_shorter);
    let count = entries.len();
    {
        let mut state_mut = state_rc.borrow_mut();
        state_mut.all_entries = entries;

        let msg = if count > 0 {
            format!("{} sound{} found", count, if count == 1 { "" } else { "s" })
        } else {
            "No sounds found. Use 'Export Sound' to save sounds first".to_string()
        };

        state_mut.status_label.set_label(&msg);
    }
    rebuild_categories(state_rc);
}

/// Reconstructs the category sidebar based on the current set of sound entries.
fn rebuild_categories(state_rc: &Rc<RefCell<SoundBrowserState>>) {
    let category_list = state_rc.borrow().category_list.clone();

    // Remove all existing category rows.
    while let Some(child) = category_list.first_child() {
        category_list.remove(&child);
    }

    // Gather unique categories from the current entry set.
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for entry in &state_rc.borrow().all_entries {
        seen.insert(if entry.sound_type().is_empty() {
            CATEGORY_NONE.to_string()
        } else {
            entry.sound_type()
        });
    }

    let mut labels: Vec<String> = vec![CATEGORY_ALL.to_string()];
    labels.extend(seen);

    // Builds the category navigation list.
    let mut row_filters: Vec<Option<String>> = Vec::new();
    for label in &labels {
        let row = ListBoxRow::new();

        // `None` = All, `Some("")` = Uncategorized, `Some(x)` = type x.
        let filter: Option<String> = if label == CATEGORY_ALL {
            None
        } else if label == CATEGORY_NONE {
            Some(String::new())
        } else {
            Some(label.clone())
        };

        row_filters.push(filter);
        let row_label = Label::builder()
            .label(label.as_str())
            .halign(Align::Start)
            .margin_start(8)
            .margin_end(8)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        row.set_child(Some(&row_label));
        category_list.append(&row);
    }
    state_rc.borrow_mut().row_filters = row_filters;

    if let Some(first) = category_list.row_at_index(0) {
        category_list.select_row(Some(&first));
        populate_sounds(state_rc, None);
    }
}

/// Refills the sound list model applying the selected category filter.
fn populate_sounds(state_rc: &Rc<RefCell<SoundBrowserState>>, category_filter: Option<&str>) {
    let state_ref = state_rc.borrow();
    state_ref.model.remove_all();
    state_ref.load_btn.set_sensitive(false);
    // Filter the sound entries based on the user's selected category.
    for entry in &state_ref.all_entries {
        let entry_category = {
            let sound_type = entry.sound_type();
            if sound_type.is_empty() {
                CATEGORY_NONE.to_string()
            } else {
                sound_type
            }
        };
        // Apply category filtering logic.
        if let Some(filter) = category_filter {
            let expected = if filter.is_empty() { CATEGORY_NONE } else { filter };
            if entry_category != expected {
                continue;
            }
        }
        state_ref.model.append(entry);
    }
}

// ── Import from file ───────────────────────────────────────────────────────────────────────────────────────

/// Opens a file selector for importing an external `.c7` sound file.
fn on_import_clicked(state_rc: &Rc<RefCell<SoundBrowserState>>, on_load: &Rc<dyn Fn(Vec<u8>, Option<usize>)>, window: &libadwaita::Window) {
    let dialog = gtk4::FileDialog::new();
    dialog.set_title("Import Sound...");
    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    let file_filter = gtk4::FileFilter::new();
    file_filter.set_name(Some("C7 Backup (*.c7)"));
    file_filter.add_pattern("*.c7");
    file_filter.add_pattern("*.syx");
    filters.append(&file_filter);
    dialog.set_filters(Some(&filters));

    let parent_window = state_rc.borrow().status_label.root().and_downcast::<gtk4::Window>();
    let on_load_c = Rc::clone(on_load);
    let window_c = window.clone();
    dialog.open(parent_window.as_ref(), None::<&gio::Cancellable>, move |result| {
        // Processes the imported file, extracts sound data, and triggers a preview.
        let Ok(file) = result else { return }; // user cancelled
        let path = file.path().unwrap_or_default();
        let c7_items = read_c7_or_sysex_file(&path);
        match find_c7_item(&c7_items, Some("sound"), None) {
            None => show_import_error_dialog(&window_c, "No sound data found in this file."),
            Some(item) => match &item.data {
                Some(C7Data::Binary(bytes)) if !bytes.is_empty() => {
                    on_load_c(bytes.clone(), section_to_track_idx(&item.section));
                    window_c.close();
                }
                Some(C7Data::Object(json)) => {
                    let device_name = c7_items
                        .iter()
                        .find(|item| item.section == "c7")
                        .and_then(|item| item.get("device"))
                        .unwrap_or("");
                    match DeviceConfig::find_by_name(device_name) {
                        Some(dc) => {
                            let blob = if dc.is_device("MnM") {
                                mnm_sound_from_json(json, &dc)
                            } else {
                                md_sound_from_json(json, &dc)
                            };

                            on_load_c(blob, section_to_track_idx(&item.section));
                            window_c.close();
                        }
                        None => show_import_error_dialog(&window_c, "No sound data found in this file."),
                    }
                }
                _ => show_import_error_dialog(&window_c, "No sound data found in this file."),
            },
        }
    });
}

/// Presents an error dialog when a sound import operation fails.
fn show_import_error_dialog(parent: &libadwaita::Window, msg: &str) {
    let dialog = libadwaita::AlertDialog::new(Some("Cannot Import File"), Some(msg));
    dialog.add_response("ok", "OK");
    dialog.present(Some(parent));
}
