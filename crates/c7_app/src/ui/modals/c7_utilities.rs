//! Modal for batch processing `.c7` and `.syx` files.
//!
//! Loads multiple files, resolves slot conflicts, and exports consolidated bundles.
//! Slot conflicts are auto-resolved on save: the first occurrence keeps its slot, later duplicates move to the next free slot of that type.
//! Device mixing is rejected at load time.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::ui::base_module::make_file_dialog;
use crate::ui::file_checks::verify_device_match;
use crate::ui::widgets::{ChooseOnePill, CustomDropdown};
use c7_core::c7_file_interfacing::{
    C7Data, C7Item, device_family_from_file, generate_c7_header, generate_export_filename, item_fingerprint, item_sort_key,
    read_c7_or_sysex_file, resolve_conflicts, sort_c7_items, write_c7_file,
};
use c7_core::device_config::DeviceConfig;
use c7_core::utils::{cap_first_char, now_compact_timestamp};

// ── Internal state ─────────────────────────────────────────────────────────────────────────────────────────

struct LoadedFile {
    path: String,
    items: Vec<C7Item>,
}

struct ItemRow {
    item: C7Item,
    check: gtk4::CheckButton,
    conflict_combo: Option<CustomDropdown>,
}

struct C7UtilitiesState {
    loaded_files: Vec<LoadedFile>,
    item_rows: Vec<ItemRow>,
    loaded_device: Option<String>,
}

/// Secondary modal providing file-format utilities and diagnostic tools.
pub(crate) struct C7UtilitiesModal {
    window: libadwaita::Window,
}

impl C7UtilitiesModal {
    /// Creates the C7 Utilities modal, parented to the given main window.
    pub(crate) fn new(parent: &impl IsA<gtk4::Window>) -> Self {
        let window = libadwaita::Window::new();
        window.set_title(Some("C7 Utilities"));
        window.set_transient_for(Some(parent));
        window.set_modal(true);
        window.set_default_size(680, -1);

        let state = Rc::new(RefCell::new(C7UtilitiesState {
            loaded_files: Vec::new(),
            item_rows: Vec::new(),
            loaded_device: None,
        }));

        let toolbar_view = libadwaita::ToolbarView::new();
        toolbar_view.add_top_bar(&libadwaita::HeaderBar::new());
        toolbar_view.set_content(Some(&build_page(&window, &state)));
        window.set_content(Some(&toolbar_view));

        C7UtilitiesModal { window }
    }

    /// Displays the utilities window.
    pub(crate) fn present(&self) {
        self.window.present();
    }
}

// ── Page layout ────────────────────────────────────────────────────────────────────────────────────────────

/// Constructs the primary UI layout for item management and file selection.
fn build_page(window: &libadwaita::Window, state: &Rc<RefCell<C7UtilitiesState>>) -> gtk4::Box {
    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    outer.set_margin_top(16);
    outer.set_margin_bottom(16);
    outer.set_margin_start(16);
    outer.set_margin_end(16);

    let instructions = gtk4::Label::new(Some(
        "Load one or more .c7 or .syx files. \
         Select the items you want to keep, then save.",
    ));
    instructions.set_wrap(true);
    instructions.set_xalign(0.0);
    outer.append(&instructions);

    let add_btn = gtk4::Button::with_label("Add Files...");
    add_btn.add_css_class("pill");
    outer.append(&add_btn);

    // Loaded files list. Hidden until at least one file is added.
    let files_listbox = gtk4::ListBox::new();
    files_listbox.set_selection_mode(gtk4::SelectionMode::None);
    files_listbox.add_css_class("boxed-list");
    let files_scroll = gtk4::ScrolledWindow::new();
    files_scroll.set_child(Some(&files_listbox));
    files_scroll.set_propagate_natural_height(true);
    files_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    files_scroll.set_visible(false);
    outer.append(&files_scroll);

    // Select all/none. Hidden until items are present.
    let selection_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    selection_row.set_halign(gtk4::Align::Center);
    let all_btn = gtk4::Button::with_label("Select All");
    all_btn.add_css_class("pill");
    let none_btn = gtk4::Button::with_label("Select None");
    none_btn.add_css_class("pill");
    selection_row.append(&all_btn);
    selection_row.append(&none_btn);
    selection_row.set_visible(false);
    outer.append(&selection_row);

    // Item checklist. Hidden until items are present.
    let items_listbox = gtk4::ListBox::new();
    items_listbox.set_selection_mode(gtk4::SelectionMode::None);
    items_listbox.add_css_class("boxed-list");
    let items_scroll = gtk4::ScrolledWindow::new();
    items_scroll.set_child(Some(&items_listbox));
    items_scroll.set_propagate_natural_height(true);
    items_scroll.set_max_content_height(400);
    items_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    items_scroll.set_visible(false);
    outer.append(&items_scroll);

    // Output format toggle. Hidden until at least one file is loaded.
    let format_pill = ChooseOnePill::new(&["Save as .c7", "Save as .syx"], 0);
    format_pill.widget().set_halign(gtk4::Align::Center);
    format_pill.widget().set_visible(false);
    outer.append(format_pill.widget());

    let save_btn = gtk4::Button::with_label("Save...");
    save_btn.add_css_class("pill");
    save_btn.add_css_class("suggested-action");
    save_btn.set_sensitive(false);
    outer.append(&save_btn);

    let status_label = gtk4::Label::new(Some(""));
    status_label.set_xalign(0.0);
    status_label.set_wrap(true);
    status_label.add_css_class("dim-label");
    outer.append(&status_label);

    // ── Shared rebuild closure (Rc so it can be shared by multiple button callbacks) ───────────────────────

    let rebuild: Rc<dyn Fn()> = Rc::new({
        let state = Rc::clone(state);
        let format_pill_c = format_pill.clone();
        let save_btn_c = save_btn.clone();
        let status_label_c = status_label.clone();
        move || {
            rebuild_items_list(
                &state,
                &items_listbox,
                &items_scroll,
                &selection_row,
                &format_pill_c,
                &save_btn_c,
                &status_label_c,
            );
        }
    });

    // ── Select All / None ──────────────────────────────────────────────────────────────────────────────────

    {
        let state = Rc::clone(state);
        all_btn.connect_clicked(move |_| {
            for row in &state.borrow().item_rows {
                row.check.set_active(true);
            }
        });
    }
    {
        let state = Rc::clone(state);
        none_btn.connect_clicked(move |_| {
            for row in &state.borrow().item_rows {
                row.check.set_active(false);
            }
        });
    }

    // ── Add Files button ───────────────────────────────────────────────────────────────────────────────────

    {
        let state = Rc::clone(state);
        let window_c = window.clone();
        let status_label_c = status_label.clone();
        let rebuild = Rc::clone(&rebuild);
        add_btn.connect_clicked(move |_| {
            on_add_files(&window_c, &state, &files_listbox, &files_scroll, &status_label_c, &rebuild);
        });
    }

    // ── Save button ────────────────────────────────────────────────────────────────────────────────────────

    {
        let state = Rc::clone(state);
        let window_c = window.clone();
        save_btn.connect_clicked(move |_| {
            on_save(&window_c, &state, &format_pill, &status_label);
        });
    }

    outer
}

// ── File loading ───────────────────────────────────────────────────────────────────────────────────────────

/// Opens a file dialog to select `.c7` or `.syx` files.
fn on_add_files(
    window: &libadwaita::Window,
    state: &Rc<RefCell<C7UtilitiesState>>,
    files_listbox: &gtk4::ListBox,
    files_scroll: &gtk4::ScrolledWindow,
    status_label: &gtk4::Label,
    rebuild: &Rc<dyn Fn()>,
) {
    let dialog = make_file_dialog("Add Files", None);

    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    // Populate the file filters with supported extensions.
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

    let state = Rc::clone(state);
    let window_c = window.clone();
    let files_listbox_c = files_listbox.clone();
    let files_scroll_c = files_scroll.clone();
    let status_label_c = status_label.clone();
    let rebuild = Rc::clone(rebuild);
    dialog.open_multiple(Some(window), None::<&gio::Cancellable>, move |result| {
        // Silently fail if the dialog was closed without a selection.
        let Ok(file_list) = result else { return };
        on_files_chosen(
            &file_list,
            &state,
            &window_c,
            &files_listbox_c,
            &files_scroll_c,
            &status_label_c,
            &rebuild,
        );
    });
}

/// Processes the uploaded `.c7` and `.syx` files and loads them into the internal list.
fn on_files_chosen(
    file_list: &gio::ListModel,
    state: &Rc<RefCell<C7UtilitiesState>>,
    window: &libadwaita::Window,
    files_listbox: &gtk4::ListBox,
    files_scroll: &gtk4::ScrolledWindow,
    _status_label: &gtk4::Label,
    rebuild: &Rc<dyn Fn()>,
) {
    let num_files = file_list.n_items();
    if num_files == 0 {
        return;
    }

    let mut added = 0;

    // Iterate through the selected files and process each valid path.
    for i in 0..num_files {
        let Some(file) = file_list.item(i).and_downcast::<gio::File>() else {
            continue;
        };
        let Some(path) = file.path() else { continue };
        let path_str = path.to_string_lossy().to_string();

        // Skip files that have already been added to the session.
        if state.borrow().loaded_files.iter().any(|loaded_file| loaded_file.path == path_str) {
            continue;
        }

        // Attempt to parse and validate the file content.
        let loaded_device = state.borrow().loaded_device.clone();
        let items = load_file_items(&path_str);
        // Verify that the new file matches the device family of previously loaded files.
        if !verify_device_match(
            window,
            &path_str,
            &items,
            loaded_device.as_deref(),
            "You can't mix devices in one bundle.",
        ) {
            continue;
        }
        let file_device = device_family_from_file(&path_str, &items);
        let _row = build_file_row(&path_str, &items, state, files_listbox, files_scroll, rebuild);
        {
            let mut state_mut = state.borrow_mut();
            // Lock this window to this device family if this is the first identifiable file.
            if file_device.is_some() && state_mut.loaded_device.is_none() {
                state_mut.loaded_device = file_device;
            }
            state_mut.loaded_files.push(LoadedFile { path: path_str, items });
        }
        added += 1;
    }

    if added > 0 {
        files_scroll.set_visible(true);
        rebuild();
    }
}

/// Loads and parses a single `.c7` or `.syx` file into a list of C7 items.
fn load_file_items(path: &str) -> Vec<C7Item> {
    let all = read_c7_or_sysex_file(path);
    all.into_iter().filter(|item| item.section != "c7" && item.data.is_some()).collect()
}

/// Creates a UI row for the data in a loaded `.c7` or `.syx` file.
fn build_file_row(
    path: &str,
    items: &[C7Item],
    state: &Rc<RefCell<C7UtilitiesState>>,
    files_listbox: &gtk4::ListBox,
    files_scroll: &gtk4::ScrolledWindow,
    rebuild: &Rc<dyn Fn()>,
) -> libadwaita::ActionRow {
    let row = libadwaita::ActionRow::new();
    let filename = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    row.set_title(&filename);

    // Calculate item counts per type for the row subtitle.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for item in items {
        *counts.entry(item.get_type().to_string()).or_insert(0) += 1;
    }

    let subtitle = if counts.is_empty() {
        "No recognized items".to_string()
    } else {
        let mut parts: Vec<String> = counts.iter().map(|(item_type, count)| format!("{count} {item_type}(s)")).collect();
        parts.sort();
        parts.join(", ")
    };

    row.set_subtitle(&subtitle);

    let remove_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    remove_btn.add_css_class("flat");
    remove_btn.add_css_class("circular");
    remove_btn.set_valign(gtk4::Align::Center);
    row.add_suffix(&remove_btn);

    files_listbox.append(&row);

    // Remove button callback.
    {
        let state = Rc::clone(state);
        let path_str = path.to_string();
        let row_c = row.clone();
        let files_listbox_c = files_listbox.clone();
        let files_scroll_c = files_scroll.clone();
        let rebuild = Rc::clone(rebuild);
        remove_btn.connect_clicked(move |_| {
            files_listbox_c.remove(&row_c);
            {
                let mut state_mut = state.borrow_mut();
                state_mut.loaded_files.retain(|loaded_file| loaded_file.path != path_str);
                // Clear device lock when all files are cleared.
                if state_mut.loaded_files.is_empty() {
                    files_scroll_c.set_visible(false);
                    state_mut.loaded_device = None;
                }
            }
            rebuild();
        });
    }

    row
}

// ── Item checklist ─────────────────────────────────────────────────────────────────────────────────────────

/// Rebuilds the UI item checklist from all currently loaded files.
fn rebuild_items_list(
    state: &Rc<RefCell<C7UtilitiesState>>,
    items_listbox: &gtk4::ListBox,
    items_scroll: &gtk4::ScrolledWindow,
    selection_row: &gtk4::Box,
    format_pill: &ChooseOnePill,
    save_btn: &gtk4::Button,
    status_label: &gtk4::Label,
) {
    // Clear all existing rows from the items listbox.
    while let Some(row) = items_listbox.row_at_index(0) {
        items_listbox.remove(&row);
    }

    let (multi_source, all_pairs, num_files, loaded_device) = {
        let state_ref = state.borrow();
        let multi = state_ref.loaded_files.len() > 1;
        let pairs: Vec<(C7Item, String)> = state_ref
            .loaded_files
            .iter()
            .flat_map(|loaded_file| loaded_file.items.iter().map(|item| (item.clone(), loaded_file.path.clone())))
            .collect();
        (multi, pairs, state_ref.loaded_files.len(), state_ref.loaded_device.clone())
    };

    // Separate items from source paths for sorting.
    let (all_items, source_paths): (Vec<C7Item>, Vec<String>) = all_pairs.into_iter().unzip();
    let sorted_indices = sort_indices_by_key(&all_items);

    // Drop exact duplicates (same type, slot, name, and data).
    let mut seen_fingerprints: HashSet<u64> = HashSet::new();
    let mut deduped: Vec<(C7Item, String)> = Vec::new();
    for idx in sorted_indices {
        let fingerprint = item_fingerprint(&all_items[idx]);
        if seen_fingerprints.insert(fingerprint) {
            deduped.push((all_items[idx].clone(), source_paths[idx].clone()));
        }
    }

    // Pre-count (type, section) occurrences so slot conflicts can be flagged.
    let mut slot_counts: HashMap<(String, String), usize> = HashMap::new();
    for (item, _) in &deduped {
        *slot_counts.entry((item.get_type().to_string(), item.section.clone())).or_insert(0) += 1;
    }
    let mut slot_seen: HashMap<(String, String), usize> = HashMap::new();

    let mut new_item_rows = Vec::new();

    // Map each item to a checklist row with conflict resolution controls.
    for (item, source) in deduped {
        let item_type = item.get_type().to_string();
        let section = item.section.clone();
        let name = item.get_name().unwrap_or("").to_string();

        let mut title = format!("{}  ·  {}", cap_first_char(&item_type), section);
        // Append name to title if it exists and isn't a placeholder.
        if !name.is_empty() && name != "--" {
            title = format!("{title}  ·  {name}");
        }

        let key = (item_type, section);
        let is_conflict = slot_counts.get(&key).copied().unwrap_or(0) > 1;

        // Tag row with occurrence index if a slot conflict exists.
        if is_conflict {
            let total = slot_counts[&key];
            let occurrence = slot_seen.entry(key.clone()).or_insert(0);
            *occurrence += 1;
            title = format!("{title}  ({} of {})", *occurrence, total);
        }

        let row = libadwaita::ActionRow::new();
        row.set_title(&title);
        // Show source filename if items are combined from multiple files.
        if multi_source && !source.is_empty() {
            let source_name = Path::new(&source)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            row.set_subtitle(&source_name);
        }

        let check = gtk4::CheckButton::new();
        check.set_active(true);
        check.set_valign(gtk4::Align::Center);
        row.add_prefix(&check);

        let conflict_combo: Option<CustomDropdown> = if is_conflict {
            let occurrence = slot_seen[&key];
            // Add resolution dropdown for items sharing a target slot.
            // First occurrence defaults to "Keep slot"; later ones to "Move to free slot".
            let combo = CustomDropdown::new(Some(0));
            combo.append_text("Keep slot");
            combo.append_text("Move to free slot");
            combo.set_active(Some(u32::from(occurrence != 1)));
            combo.set_valign(gtk4::Align::Center);
            row.add_suffix(&*combo);
            Some(combo)
        } else {
            None
        };

        items_listbox.append(&row);
        new_item_rows.push(ItemRow {
            item,
            check,
            conflict_combo,
        });
    }

    let has_items = !new_item_rows.is_empty();
    items_scroll.set_visible(has_items);
    selection_row.set_visible(has_items);
    format_pill.widget().set_visible(has_items);
    save_btn.set_sensitive(has_items);
    // A file that resolves to no device leaves `loaded_device` unset, collapsing the prefix to nothing.
    let device_prefix = loaded_device.map_or_else(String::new, |device| format!("{device} "));
    let label_text = format!("{num_files} {device_prefix}file(s) loaded  ·  {} item(s)", new_item_rows.len());
    status_label.set_label(if has_items { &label_text } else { "" });

    state.borrow_mut().item_rows = new_item_rows;
}

// ── Saving ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Starts the save process by presenting a save dialog.
///
/// Also determines the default filename and format.
fn on_save(window: &libadwaita::Window, state: &Rc<RefCell<C7UtilitiesState>>, format_pill: &ChooseOnePill, status_label: &gtk4::Label) {
    // Collect which item rows are selected (by index) and their conflict dropdown value.
    let selected: Vec<(usize, Option<u32>)> = {
        let state_ref = state.borrow();
        state_ref
            .item_rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                if row.check.is_active() {
                    // `active()` returns `Option<u32>`. `None` means no selection (treat as 0 = "Keep slot").
                    let combo_val = row.conflict_combo.as_ref().and_then(CustomDropdown::active);
                    Some((i, combo_val))
                } else {
                    None
                }
            })
            .collect()
    };

    if selected.is_empty() {
        status_label.set_label("No items selected.");
        return;
    }

    let file_format = format_pill.active();
    let extension = if file_format == 0 { ".c7" } else { ".syx" };

    // Build a default filename following the project naming scheme: `DEVICE_Item_Timestamp.c7`
    let default_name = {
        let state_ref = state.borrow();
        let device = state_ref.loaded_device.clone().unwrap_or_default();
        let dc = DeviceConfig::find_by_name(&device);
        let device_shorter = dc.map_or_else(|| "Unknown".to_string(), |dc| dc.device_shorter);
        let types: HashSet<String> = selected
            .iter()
            .map(|(i, _)| state_ref.item_rows[*i].item.get_type().to_string())
            .collect();

        let item_type = if types.len() == 1 {
            cap_first_char(types.iter().next().unwrap())
        } else {
            "Bundle".to_string()
        };

        let slot_label = now_compact_timestamp();
        let filename = generate_export_filename(&device_shorter, &item_type, &slot_label, None);
        let stem = filename.trim_end_matches(".c7").trim_end_matches(".syx").to_string();
        format!("{stem}{extension}")
    };

    let dialog = make_file_dialog("Save File", Some(&default_name));

    let state = Rc::clone(state);
    let status_label_c = status_label.clone();
    let extension_str = extension.to_string();
    dialog.save(Some(window), None::<&gio::Cancellable>, move |result| {
        // Silently fail if dialog was closed or path retrieval failed.
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let mut path_str = path.to_string_lossy().to_string();
        if !path_str.ends_with(&extension_str) {
            path_str.push_str(&extension_str);
        }
        save_selected_items(&state, &selected, file_format, &path_str, &status_label_c);
    });
}

/// Executes the file saving to disk.
fn save_selected_items(
    state: &Rc<RefCell<C7UtilitiesState>>,
    selected_indices: &[(usize, Option<u32>)],
    file_format: i32,
    path: &str,
    status_label: &gtk4::Label,
) {
    let (resolved, reassigned) = {
        let state_ref = state.borrow();
        let pairs: Vec<(&C7Item, Option<u32>)> = selected_indices
            .iter()
            .map(|(i, combo_val)| (&state_ref.item_rows[*i].item, *combo_val))
            .collect();
        resolve_conflicts(&pairs)
    };
    let sorted_items = sort_c7_items(resolved);

    let result: Result<(), String> = if file_format == 0 {
        // Handle native C7 bundle export.
        let loaded_device = state.borrow().loaded_device.clone().unwrap_or_else(|| "Unknown".to_string());
        let header = generate_c7_header("bundle", &loaded_device, None);
        write_c7_file(path, &sorted_items, Some(&header));
        Ok(())
    } else {
        // Handle raw SysEx export. Write raw SysEx bytes, skipping non-byte items.
        (|| {
            use std::io::Write;
            let mut out = std::fs::File::create(path).map_err(|e| e.to_string())?;
            // Serialize SysEx payloads into the output file.
            for item in &sorted_items {
                // Only commit binary data blocks to the `.syx` output.
                if let Some(C7Data::Binary(bytes)) = &item.data {
                    out.write_all(bytes).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        })()
    };

    let filename = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();

    match result {
        Ok(()) => {
            use std::fmt::Write;
            let mut msg = format!("Saved {} item(s) to {filename}", sorted_items.len());
            // Append reassignment statistics to the status message if conflicts were resolved.
            if reassigned > 0 {
                let _ = write!(msg, "  ({reassigned} slot conflict(s) auto-reassigned)");
            }
            status_label.set_label(&msg);
        }
        Err(e) => status_label.set_label(&format!("Error: {e}")),
    }
}

// ── Module-level helpers ───────────────────────────────────────────────────────────────────────────────────

/// Sorts items and returns their original indices in sorted order (for dedup with source tracking).
fn sort_indices_by_key(items: &[C7Item]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..items.len()).collect();
    indices.sort_by_key(|&i| item_sort_key(&items[i]));
    indices
}
