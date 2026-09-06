//! `BaseModule`: shared scaffold for all feature screens.
//!
//! Rust screens contain a `gtk4::Box` root widget rather than subclassing it.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use gtk4::prelude::*;
use gtk4::{self, Align, Button, Label, Orientation, ProgressBar, Separator, glib};

use c7_core::device_config::get_export_folder;

/// Thread-safe status setter handed out by [`BaseModule::status_updater`] (and `deferred_status()`).
///
/// It takes `&str` and marshals the update onto the GTK main thread. Callers include any "Status: " prefix themselves.
/// This is the single status entry point. The alias itself lives in `c7_core` so protocol operations there can take the same callback.
pub(crate) use c7_core::utils::StatusFn;

/// Thread-safe progress setter handed out by [`BaseModule::progress_updater`].
///
/// It takes (message, fraction, ETA) and routes to `show_progress_status()`.
pub(crate) type ProgressFn = Arc<dyn Fn(&str, f64, &str) + Send + Sync>;

/// Base scaffold for all feature screens.
///
/// Callers invoke `build_header()` and optionally `build_status_area()` during construction.
///
/// It contains a `gtk4::Box` root widget rather than subclassing it.
pub(crate) struct BaseModule {
    pub root: gtk4::Box,
    pub status_label: Option<Label>,
    pub progress_bar: Option<ProgressBar>,
    eta_label: Option<Label>,
    status_box: Option<gtk4::Box>,
    /// Feature cards appended so far, so a screen whose cards are all gated out can be left out of the menu.
    card_count: usize,
}

/// Returned by `build_header()`, holding the `start` and `end` boxes for extra widgets.
pub(crate) struct HeaderArea {
    pub start: gtk4::Box,
    pub end: gtk4::Box,
}

impl BaseModule {
    /// Initializes the component and its default state.
    pub(crate) fn new() -> Self {
        let root = gtk4::Box::new(Orientation::Vertical, 12);
        root.set_valign(Align::Start);
        root.set_margin_top(15);
        root.set_margin_bottom(15);
        root.set_margin_start(15);
        root.set_margin_end(15);

        BaseModule {
            root,
            status_label: None,
            progress_bar: None,
            eta_label: None,
            status_box: None,
            card_count: 0,
        }
    }

    /// Appends a header row and separator to the module.
    ///
    /// Returns a `HeaderArea` containing the `start` and `end` boxes for additional widgets.
    pub(crate) fn build_header(&self, title: &str, go_back_cb: impl Fn() + 'static) -> HeaderArea {
        let overlay = gtk4::Overlay::new();
        overlay.set_margin_bottom(8);

        let title_label = Label::new(Some(title));
        title_label.add_css_class("title-2");
        title_label.set_halign(Align::Center);
        title_label.set_valign(Align::Center);
        overlay.set_child(Some(&title_label));

        let start = gtk4::Box::new(Orientation::Horizontal, 8);
        start.set_halign(Align::Start);
        start.set_valign(Align::Center);

        let back_btn = Button::with_label("← Back");
        back_btn.connect_clicked(move |_| go_back_cb());
        start.append(&back_btn);
        overlay.add_overlay(&start);

        let end = gtk4::Box::new(Orientation::Horizontal, 8);
        end.set_halign(Align::End);
        end.set_valign(Align::Center);
        overlay.add_overlay(&end);

        self.root.append(&overlay);
        self.root.append(&Separator::new(Orientation::Horizontal));

        HeaderArea { start, end }
    }

    /// Appends a progress bar and status label to the module.
    ///
    /// `with_eta=true` adds an ETA label in a horizontal status box (used by upload screens).
    /// The status box starts hidden. Show it when a transfer begins via `status_box.set_visible(true)`.
    pub(crate) fn build_status_area(&mut self, with_eta: bool) {
        // Some screens already end their last section with a separator, so only add one here if the last widget isn't already a separator.
        // That keeps a single bar above the status area instead of a double bar.
        let already_divided = self.root.last_child().is_some_and(|w| w.downcast::<Separator>().is_ok());
        if !already_divided {
            self.root.append(&Separator::new(Orientation::Horizontal));
        }

        let progress_bar = ProgressBar::new();
        progress_bar.set_visible(false);
        progress_bar.set_margin_top(4);
        self.root.append(&progress_bar);
        self.progress_bar = Some(progress_bar);

        if with_eta {
            let status_box = gtk4::Box::new(Orientation::Horizontal, 0);
            status_box.set_visible(false);

            let status_label = Label::new(Some(""));
            status_label.set_hexpand(true);
            status_label.set_halign(Align::Start);
            status_box.append(&status_label);

            let eta_label = Label::new(Some(""));
            eta_label.set_halign(Align::End);
            status_box.append(&eta_label);

            self.root.append(&status_box);
            self.status_label = Some(status_label);
            self.eta_label = Some(eta_label);
            self.status_box = Some(status_box);
        } else {
            let status_label = Label::new(Some(""));
            status_label.set_margin_top(4);
            status_label.set_halign(Align::Start);
            self.root.append(&status_label);
            self.status_label = Some(status_label);
        }
    }

    /// Appends a feature card and its trailing divider.
    ///
    /// Only cards added here count toward `has_cards()`, so a shared row appended straight to `root` can't mask an empty screen.
    pub(crate) fn append_card(&mut self, section: &gtk4::Box) {
        self.root.append(section);
        self.root.append(&Separator::new(Orientation::Horizontal));
        self.card_count += 1;
    }

    /// Returns `true` when at least one feature card passed its device gate.
    pub(crate) fn has_cards(&self) -> bool {
        self.card_count > 0
    }

    /// Assembles a bordered card with numbered steps.
    pub(crate) fn build_instructions_card(&self, steps: &[&str], title: &str) {
        let card = Self::instructions_card(steps, Some(title));
        card.set_margin_top(12);
        self.root.append(&card);
    }

    /// Returns the instructions card without appending it, for callers nesting one inside their own container.
    ///
    /// A `None` title suits a card sitting under a heading that already names it.
    pub(crate) fn instructions_card(steps: &[&str], title: Option<&str>) -> gtk4::Box {
        let card = gtk4::Box::new(Orientation::Vertical, 8);
        card.add_css_class("card");

        let inner = gtk4::Box::new(Orientation::Vertical, 8);
        inner.set_margin_top(15);
        inner.set_margin_bottom(15);
        inner.set_margin_start(15);
        inner.set_margin_end(15);

        if let Some(title) = title {
            let label = Label::new(Some(title));
            label.set_xalign(0.0);
            label.add_css_class("title-4");
            inner.append(&label);
        }

        for step in steps {
            let step_label = Label::new(Some(step));
            step_label.set_xalign(0.0);
            step_label.set_wrap(true);
            inner.append(&step_label);
        }

        card.append(&inner);
        card
    }

    /// Returns a thread-safe status setter to move into background threads.
    ///
    /// The closure is cloneable and `Send`, and schedules the label update back on the GTK main thread.
    /// It stops doing anything once the screen (and its label) is dropped, so a late worker message can't touch a freed widget.
    pub(crate) fn status_updater(&self) -> StatusFn {
        let label_wk = self.status_label.as_ref().map(|label| glib::SendWeakRef::from(label.downgrade()));
        let box_wk = self
            .status_box
            .as_ref()
            .map(|box_widget| glib::SendWeakRef::from(box_widget.downgrade()));
        Arc::new(move |msg: &str| {
            if let Some(Some(label)) = label_wk.as_ref().map(|w| w.upgrade()) {
                let box_now = box_wk.as_ref().and_then(|w| w.upgrade());
                update_status(box_now.as_ref(), &label, msg);
            }
        })
    }

    /// Returns a thread-safe progress setter for updating ETA and fraction.
    pub(crate) fn progress_updater(&self) -> ProgressFn {
        let label_wk = self.status_label.as_ref().map(|label| glib::SendWeakRef::from(label.downgrade()));
        let box_wk = self
            .status_box
            .as_ref()
            .map(|box_widget| glib::SendWeakRef::from(box_widget.downgrade()));
        let progress_wk = self
            .progress_bar
            .as_ref()
            .map(|progress| glib::SendWeakRef::from(progress.downgrade()));
        let eta_wk = self.eta_label.as_ref().map(|eta| glib::SendWeakRef::from(eta.downgrade()));

        Arc::new(move |msg: &str, frac: f64, eta: &str| {
            if let (Some(Some(label)), Some(Some(progress)), Some(Some(eta_label))) = (
                label_wk.as_ref().map(|w| w.upgrade()),
                progress_wk.as_ref().map(|w| w.upgrade()),
                eta_wk.as_ref().map(|w| w.upgrade()),
            ) {
                let box_now = box_wk.as_ref().and_then(|w| w.upgrade());
                if let Some(status_box) = box_now.as_ref() {
                    show_progress_status(status_box, &label, &progress, &eta_label, msg, frac, eta);
                }
            }
        })
    }

    /// Hands out a status updater that works before the real status label exists.
    ///
    /// Some screens wire up handlers early in `new()`, before `build_status_area()` creates the label.
    /// That's often called last, since the status line sits at the bottom.
    /// Those handlers need a `StatusFn` right away, so this returns one immediately, plus a slot.
    /// The updater does nothing until the slot is filled.
    /// That happens once the caller stores `status_updater()`'s result in it, after `build_status_area()` runs.
    pub(crate) fn deferred_status() -> (StatusFn, Arc<Mutex<Option<StatusFn>>>) {
        let slot: Arc<Mutex<Option<StatusFn>>> = Arc::new(Mutex::new(None));
        let updater: StatusFn = {
            let slot = Arc::clone(&slot);
            Arc::new(move |msg: &str| {
                if let Some(update_status) = slot.lock().unwrap().as_ref() {
                    update_status(msg);
                }
            })
        };
        (updater, slot)
    }
}

impl Default for BaseModule {
    /// Returns the default `BaseModule`.
    fn default() -> Self {
        Self::new()
    }
}

/// Estimates remaining time, formatted as `"MM:SS"`.
///
/// Returns an empty string if there's nothing to estimate yet, too little time has passed to measure a rate, or the operation is done.
pub(crate) fn eta_string(start: Instant, frac: f64) -> String {
    if frac <= 0.0 || frac >= 1.0 {
        return String::new();
    }
    let elapsed = start.elapsed().as_secs_f64();
    if elapsed <= 0.0 {
        return String::new();
    }
    let rate = frac / elapsed;
    let remaining_secs = (1.0 - frac) / rate;
    let mins = remaining_secs as u64 / 60;
    let secs = remaining_secs as u64 % 60;
    format!("{mins:02}:{secs:02}")
}

/// Writes a message to a status label as-is, revealing the status box if one is present.
///
/// Shared by `BaseModule::status_updater` and by screens that own their status label directly.
/// It doesn't add a "Status: " prefix. Callers include one themselves when they want it.
pub(crate) fn update_status(status_box: Option<&gtk4::Box>, status_label: &Label, msg: &str) {
    if let Some(box_widget) = status_box {
        box_widget.set_visible(true);
    }
    status_label.set_label(msg);
}

/// Displays a status message alongside numeric progress, showing the progress bar.
fn show_progress_status(
    status_box: &gtk4::Box,
    status_label: &Label,
    progress: &ProgressBar,
    eta_label: &Label,
    msg: &str,
    frac: f64,
    eta: &str,
) {
    status_box.set_visible(true);
    status_label.set_text(msg);
    progress.set_fraction(frac);
    progress.set_visible(true);
    let eta_text = if eta.is_empty() { String::new() } else { format!("ETA: {eta}") };
    eta_label.set_text(&eta_text);
}

/// Builds a `FileDialog` opening at the configured export folder.
///
/// Pass `initial_name: Some("foo.syx")` for save dialogs; `None` for open dialogs.
pub(crate) fn make_file_dialog(title: &str, initial_name: Option<&str>) -> gtk4::FileDialog {
    let dialog = gtk4::FileDialog::builder().title(title).build();
    dialog.set_initial_folder(Some(&gio::File::for_path(get_export_folder())));
    if let Some(name) = initial_name {
        dialog.set_initial_name(Some(name));
    }
    dialog
}

/// Bridges an `async_channel::Receiver` to the GTK main loop without polling.
///
/// Spawns an async task that awaits each message and invokes `f` on the GTK main thread when data arrives, until `f` returns `Break`.
pub(crate) fn listen<T: Send + 'static>(rx: async_channel::Receiver<T>, f: impl Fn(T) -> glib::ControlFlow + 'static) {
    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            if f(msg) == glib::ControlFlow::Break {
                break;
            }
        }
    });
}
