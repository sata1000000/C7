//! Modal for exporting a kit or a single sound to a `.c7` file or the local bank.
//!
//! Owns the whole flow: the metadata prompt, the classification list it offers, and writing the result where the user chose.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use gio::Cancellable;
use gtk4::{self, Align, Box as GtkBox, Entry, Label, Orientation, glib};
use libadwaita::prelude::*;

use crate::ui::base_module::make_file_dialog;
use crate::ui::widgets::{ChooseOnePill, CustomDropdown};
use c7_core::c7_file_interfacing::{
    C7Data, C7Item, generate_c7_header, generate_export_filename, kit_blob_offset_or_die, md_sound_to_json, mnm_sound_to_json,
    wire_slot_to_section, write_c7_file,
};
use c7_core::device_config::{DeviceConfig, get_bank_dir};
use c7_core::kit::machine_list;
use c7_core::utils::{sanitize_filename, strip_illegal_filename_chars};

// ── Sound classification ───────────────────────────────────────────────────────────────────────────────────

/// Type options offered when exporting a Machinedrum sound.
const MD_SOUND_TYPES: &[&str] = &[
    "",
    "Other",
    "BD Bass Drum",
    "SD Snare Drum",
    "HT High Tom",
    "MT Mid Tom",
    "LT Low Tom",
    "CP Clap",
    "RS Rim Shot",
    "CB Cowbell",
    "CH Closed Hihat",
    "OH Open Hihat",
    "RC Ride Cymbal",
    "CC Crash Cymbal",
    "Machine",
];

/// Type options offered when exporting a Monomachine sound.
const MNM_SOUND_TYPES: &[&str] = &[
    "",
    "Other",
    "Bass",
    "Experiment",
    "Keys",
    "Lead",
    "Pad",
    "Percussion",
    "Sequence",
    "FX",
    "Template",
];

/// Picks the Type the prompt opens on, from the track's front-panel label or from the machine assigned to it.
///
/// Returns the index of the blank entry when the track gives no hint about what the sound is.
fn default_sound_type(dc: &DeviceConfig, bytes: &[u8], track_idx: usize) -> usize {
    if dc.is_device("MD") {
        // Strictly positional mapping following the Machinedrum front-panel labels.
        return *[
            2,  // 01-BD
            3,  // 02-SD
            4,  // 03-HT
            5,  // 04-MT
            6,  // 05-LT
            7,  // 06-CP
            8,  // 07-RS
            9,  // 08-CB
            10, // 09-CH
            11, // 10-OH
            12, // 11-RC
            13, // 12-CC
            14, 14, 14, 14, // 13-16: Machine
        ]
        .get(track_idx)
        .unwrap_or(&0);
    }

    // Monomachine has no fixed labels, so the assigned machine model is the only hint available.
    if !dc.is_device("MnM") {
        return 0;
    }
    let models_offset = kit_blob_offset_or_die(dc, "models");
    if bytes.len() <= models_offset {
        return 0;
    }
    let model = bytes[models_offset];
    let Some((_, machine_name)) = machine_list(dc).into_iter().find(|(machine_id, _)| *machine_id == model) else {
        return 0;
    };

    let machine_name = machine_name.to_uppercase();
    if machine_name == "DPRO-BBOX" {
        return type_index(MNM_SOUND_TYPES, "Percussion");
    }
    if machine_name.starts_with("FX-") {
        return type_index(MNM_SOUND_TYPES, "FX");
    }
    0
}

/// Finds where `name` sits in `options`, so a reordered list can't quietly change what a machine maps to.
///
/// Returns 0 if `name` isn't in the list.
fn type_index(options: &[&str], name: &str) -> usize {
    options.iter().position(|option| *option == name).unwrap_or(0)
}

// ── Export entry points ────────────────────────────────────────────────────────────────────────────────────

/// Prompts for a kit's metadata, then writes it wherever the user chose.
pub(crate) fn export_kit(parent: &impl IsA<gtk4::Widget>, dc: &Arc<DeviceConfig>, bytes: Vec<u8>, kit_slot: usize, kit_name: &str) {
    // The device reports "--" for an unnamed kit, which is not worth seeding the prompt with.
    let seed = kit_name.trim();
    let seed = if seed == "--" { "" } else { seed };

    let dc = Arc::clone(dc);
    let on_export = move |root: &gtk4::Widget, meta: ExportMeta| {
        let header = generate_c7_header("kit", &dc.device_short, None);
        let item = kit_c7_item(bytes.clone(), kit_slot, &meta);

        if meta.should_write_to_bank {
            write_to_bank(root, &get_bank_dir().join(&*dc.device_shorter), &meta.name, item, &header);
        } else {
            let filename = generate_export_filename(&dc.device_shorter, "Kit", &meta.name, None);
            write_to_chosen_file(root, "Kit", &filename, item, header);
        }
    };

    show_meta_modal(parent.as_ref(), "Kit", seed.to_string(), None, on_export);
}

/// Prompts for a sound's metadata, then writes it wherever the user chose.
///
/// `kit_name` names the kit the sound was pulled from, not the sound itself, so the exported filename can record where it came from.
pub(crate) fn export_sound(parent: &impl IsA<gtk4::Widget>, dc: &Arc<DeviceConfig>, bytes: Vec<u8>, track_idx: usize, kit_name: &str) {
    let type_options = if dc.is_device("MnM") { MNM_SOUND_TYPES } else { MD_SOUND_TYPES };
    let types = Some((type_options, default_sound_type(dc, &bytes, track_idx)));

    let dc = Arc::clone(dc);
    let kit_name = kit_name.to_string();
    let on_export = move |root: &gtk4::Widget, meta: ExportMeta| {
        let header = generate_c7_header("sound", &dc.device_short, None);
        let item = sound_c7_item(&bytes, track_idx, &dc, &meta);

        if meta.should_write_to_bank {
            // Each type gets its own folder under the bank, so an unclassified sound stays in the sounds folder itself.
            let sounds_dir = get_bank_dir().join(&*dc.device_shorter).join("sounds");
            let dir = match &meta.sound_type {
                Some(sound_type) => sounds_dir.join(strip_illegal_filename_chars(sound_type)),
                None => sounds_dir,
            };
            write_to_bank(root, &dir, &meta.name, item, &header);
        } else {
            let filename = generate_export_filename(&dc.device_shorter, "Sound", &kit_name, Some(&meta.name));
            write_to_chosen_file(root, "Sound", &filename, item, header);
        }
    };

    show_meta_modal(parent.as_ref(), "Sound", String::new(), types, on_export);
}

// ── Metadata prompt ────────────────────────────────────────────────────────────────────────────────────────

/// What the user filled in before pressing Export.
struct ExportMeta {
    /// Never empty. The prompt re-opens rather than reporting a blank name.
    name: String,
    comment: String,
    /// The chosen classification, or `None` for a kit, which has no Type row.
    sound_type: Option<String>,
    should_write_to_bank: bool,
}

/// Shows the "Kit Details" or "Sound Details" prompt, and opens it again if Export is pressed with the name left blank.
///
/// `parent` is a plain `Widget` instead of a generic one, because this function calls itself to open the prompt again.
fn show_meta_modal(
    parent: &gtk4::Widget,
    noun: &'static str,
    initial_name: String,
    types: Option<(&'static [&'static str], usize)>,
    on_export: impl FnOnce(&gtk4::Widget, ExportMeta) + 'static,
) {
    let dialog = libadwaita::AlertDialog::builder().heading(format!("{noun} Details")).build();
    let content = GtkBox::new(Orientation::Vertical, 6);
    content.set_margin_top(4);
    content.set_margin_bottom(4);
    content.set_margin_start(16);
    content.set_margin_end(16);

    content.append(&section_label("Name", 0));
    let name_entry = Entry::builder()
        .text(&initial_name)
        .placeholder_text(format!("{noun} name (required)..."))
        .activates_default(true)
        .build();
    content.append(&name_entry);

    content.append(&section_label("Comment", 4));
    let comment_entry = Entry::builder()
        .placeholder_text("Comment (optional)...")
        .activates_default(true)
        .build();
    content.append(&comment_entry);

    // Kits carry no classification, so the whole Type row is absent rather than shown empty.
    let type_row = types.map(|(type_options, type_default)| {
        content.append(&section_label("Type", 4));
        let type_combo = CustomDropdown::new(Some(type_default as i32));
        for option in type_options {
            type_combo.append_text(option);
        }
        type_combo.set_active(Some(type_default as u32));
        content.append(&*type_combo);

        // Only the "Other" entry lets the user name a type the list doesn't cover, so it hides for every fixed choice.
        let other_entry = Entry::builder()
            .placeholder_text("Custom type...")
            .visible(type_options[type_default] == "Other")
            .build();
        content.append(&other_entry);

        let other_entry_c = other_entry.clone();
        type_combo.connect_changed(move |idx| {
            if let Some(idx) = idx {
                other_entry_c.set_visible(type_options.get(idx as usize) == Some(&"Other"));
            }
        });

        (type_options, type_combo, other_entry)
    });

    content.append(&section_label("Export", 4));
    let export_pill = ChooseOnePill::new(&["Save to File", "Save to Bank"], 0);
    content.append(export_pill.widget());

    dialog.set_extra_child(Some(&content));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", "Export");
    dialog.set_response_appearance("ok", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");

    let parent_wk = glib::SendWeakRef::from(parent.downgrade());
    dialog.choose(Some(parent), Cancellable::NONE, move |response| {
        if response != "ok" {
            return;
        }

        // The prompt can outlive the screen that opened it, so a closed screen cancels the export.
        let Some(parent) = parent_wk.upgrade() else {
            return;
        };

        let name = name_entry.text().trim().to_string();
        if name.is_empty() {
            show_name_required_dialog(&parent, noun, initial_name, types, on_export);
            return;
        }

        // The blank entry at the top of each list is the user declining to classify, so it reports no type at all.
        let sound_type = type_row.and_then(|(type_options, type_combo, other_entry)| {
            let type_label = type_options.get(type_combo.active().unwrap_or(0) as usize).copied().unwrap_or("");
            let chosen = if type_label == "Other" {
                other_entry.text().to_string()
            } else {
                type_label.to_string()
            };
            (!chosen.is_empty()).then_some(chosen)
        });

        on_export(
            &parent,
            ExportMeta {
                name,
                comment: comment_entry.text().to_string(),
                sound_type,
                should_write_to_bank: export_pill.active() != 0,
            },
        );
    });
}

/// Presents a dialog explaining that a name is mandatory, then reopens the prompt once it's dismissed.
fn show_name_required_dialog(
    parent: &gtk4::Widget,
    noun: &'static str,
    initial_name: String,
    types: Option<(&'static [&'static str], usize)>,
    on_export: impl FnOnce(&gtk4::Widget, ExportMeta) + 'static,
) {
    let dialog = libadwaita::AlertDialog::builder()
        .heading("Name Required")
        .body(format!("You must enter a name for the {} before exporting.", noun.to_lowercase()))
        .build();
    dialog.add_response("ok", "OK");

    let parent_c = parent.clone();
    dialog.choose(Some(parent), Cancellable::NONE, move |_| {
        show_meta_modal(&parent_c, noun, initial_name, types, on_export);
    });
}

/// Builds one of the small captions that label a field, spaced `margin_top` pixels below whatever precedes it.
fn section_label(text: &str, margin_top: i32) -> Label {
    Label::builder()
        .label(text)
        .halign(Align::Start)
        .css_classes(vec!["caption".to_string()])
        .margin_top(margin_top)
        .build()
}

// ── Writing the file ───────────────────────────────────────────────────────────────────────────────────────

/// Packs one track's sound into a `C7Item` carrying the metadata the prompt collected.
fn sound_c7_item(bytes: &[u8], track_idx: usize, dc: &DeviceConfig, meta: &ExportMeta) -> C7Item {
    let sound_data_json = if dc.is_device("MnM") {
        mnm_sound_to_json(bytes, dc)
    } else {
        md_sound_to_json(bytes, dc)
    };
    let mut item = C7Item {
        section: wire_slot_to_section("sound", track_idx),
        data: Some(C7Data::Object(sound_data_json)),
        attrs: BTreeMap::new(),
    };

    item.attrs.insert("type".to_string(), "sound".to_string());
    item.attrs.insert("format".to_string(), "json".to_string());
    item.attrs.insert("name".to_string(), meta.name.clone());
    if !meta.comment.is_empty() {
        item.attrs.insert("comment".to_string(), meta.comment.clone());
    }
    if let Some(sound_type) = &meta.sound_type {
        item.attrs.insert("sound_type".to_string(), sound_type.clone());
    }
    item
}

/// Packs a kit into a `C7Item` carrying the metadata the prompt collected.
fn kit_c7_item(bytes: Vec<u8>, kit_slot: usize, meta: &ExportMeta) -> C7Item {
    // Kits are stored as the raw SysEx dump, same as the Librarian.
    let mut item = C7Item {
        section: wire_slot_to_section("kit", kit_slot),
        data: Some(C7Data::Binary(bytes)),
        attrs: BTreeMap::new(),
    };

    item.attrs.insert("type".to_string(), "kit".to_string());
    item.attrs.insert("format".to_string(), "sysex".to_string());
    item.attrs.insert("name".to_string(), meta.name.clone());
    if !meta.comment.is_empty() {
        item.attrs.insert("comment".to_string(), meta.comment.clone());
    }
    item
}

/// Writes `item` into the local bank at `dir`, reporting an error if that directory cannot be created.
fn write_to_bank(root: &impl IsA<gtk4::Widget>, dir: &Path, name: &str, item: C7Item, header: &BTreeMap<String, String>) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        let dialog = libadwaita::AlertDialog::builder()
            .heading("Export Error")
            .body(format!("Could not create directory:\n{e}"))
            .build();
        dialog.add_response("ok", "OK");
        dialog.present(Some(root));
        return;
    }

    let path = dir.join(format!("{}.c7", sanitize_filename(name)));
    write_c7_file(&path, &[item], Some(header));
}

/// Asks the user where to put `item`, seeding the save dialog with `filename`.
///
/// `noun` names the item in the dialog's title and file filter, capitalized as the user sees it.
fn write_to_chosen_file(root: &impl IsA<gtk4::Widget>, noun: &str, filename: &str, item: C7Item, header: BTreeMap<String, String>) {
    let dialog = make_file_dialog(&format!("Export {noun}"), Some(filename));
    let filter = gtk4::FileFilter::new();
    filter.set_name(Some(&format!("C7 {noun} files")));
    filter.add_pattern("*.c7");
    dialog.set_default_filter(Some(&filter));

    let parent_window = root.root().and_then(|root_widget| root_widget.downcast::<gtk4::Window>().ok());
    dialog.save(parent_window.as_ref(), None::<&Cancellable>, move |file_result| {
        if let Ok(file) = file_result
            && let Some(path) = file.path()
        {
            write_c7_file(path, &[item], Some(&header));
        }
    });
}
