//! Custom GTK4 widgets shared across feature screens.
//!
//! Each custom widget wraps one or more GTK primitives.
//! It exposes typed Rust methods (`connect_value_changed()`, etc.), not `GObject` string signals.

use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use cairo::{FontSlant, FontWeight, LineCap, LineJoin};
use gtk4::prelude::*;
use gtk4::{
    self, Align, Button, DrawingArea, EventControllerKey, EventControllerMotion, EventControllerScroll, EventControllerScrollFlags,
    GestureClick, GestureDrag, Image, Label, Orientation, PropagationPhase, ToggleButton, gdk, glib,
};

use crate::ui::drawing::{
    COLOR_BLUE, COLOR_BLUE_B, COLOR_BLUE_G, COLOR_BLUE_R, COLOR_GREEN_B, COLOR_GREEN_G, COLOR_GREEN_R, Corner, UI_CORNER_RADIUS,
    build_rounded_rect_path, draw_corner_badge_smaller,
};
use c7_core::sysex::filter_elektron_name;
use c7_core::utils::{format_value, pseudo_rand, round_to_digits};

/// Shared list of zero-argument event callbacks (e.g. spinner value committed).
type CbList = Rc<RefCell<Vec<Box<dyn Fn()>>>>;
/// Shared list of value-change callbacks.
///
/// Each callback takes (new value, is programmatic).
type ValueCbList = Rc<RefCell<Vec<Box<dyn Fn(i32, bool)>>>>;
/// Shared list of single-integer callbacks (e.g. note on/off, shape index changed).
type IntCbList = Rc<RefCell<Vec<Box<dyn Fn(i32)>>>>;

// ── CSS loaded once for `NumberSpinner` ────────────────────────────────────────────────────────────────────

/// Ensures that the CSS for the `NumberSpinner` is loaded into the application.
fn ensure_spinner_css() {
    static LOADED: OnceLock<()> = OnceLock::new();
    LOADED.get_or_init(|| {
        let css = gtk4::CssProvider::new();
        css.load_from_string(
            ".delay-value  { min-width: 0; padding-left: 12px; padding-right: 12px; }\
             .delay-caret  { min-width: 0; min-height: 0; padding: 1px 8px;\
                             border-top-left-radius: 0; border-bottom-left-radius: 0;\
                             border-left-width: 0; }",
        );
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(&display, &css, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
}

// ── NumberSpinner ──────────────────────────────────────────────────────────────────────────────────────────
// Delay/value spinner built from a value-button + caret buttons.

struct SpinnerState {
    val: f64,
    lower: f64,
    upper: f64,
    step: f64,
    digits: i32,
    disabled_label: String,
    normal_tooltip: Option<String>,
    disabled_tooltip: Option<String>,
}

/// A delay/value spinner built from a label button + stacked caret buttons.
#[derive(Clone)]
pub(crate) struct NumberSpinner {
    pub root: gtk4::Box,
    value_btn: Button,
    plus_btn: Button,
    minus_btn: Button,
    state: Rc<RefCell<SpinnerState>>,
    cbs: CbList,
}

impl NumberSpinner {
    /// Initializes the spinner with a value range, step size, and UI elements.
    pub(crate) fn new(val: f64, lower: f64, upper: f64, step: f64, disabled_label: &str, digits: i32) -> Self {
        ensure_spinner_css();

        let root = gtk4::Box::new(Orientation::Horizontal, 0);
        root.set_valign(Align::Center);
        root.add_css_class("linked");

        let value_btn = Button::with_label(&format_value(val, digits));
        value_btn.set_focusable(true);
        value_btn.add_css_class("delay-value");
        if let Some(child) = value_btn.child()
            && let Ok(label) = child.downcast::<Label>()
        {
            label.set_margin_top(2);
        }
        root.append(&value_btn);

        let caret_box = gtk4::Box::new(Orientation::Vertical, 0);
        caret_box.add_css_class("linked");

        let plus_btn = Button::new();
        plus_btn.set_vexpand(true);
        plus_btn.set_focusable(false);
        let up_icon = Image::from_icon_name("pan-up-symbolic");
        up_icon.set_pixel_size(10);
        plus_btn.set_child(Some(&up_icon));
        plus_btn.add_css_class("delay-caret");
        caret_box.append(&plus_btn);

        let minus_btn = Button::new();
        minus_btn.set_vexpand(true);
        minus_btn.set_focusable(false);
        let down_icon = Image::from_icon_name("pan-down-symbolic");
        down_icon.set_pixel_size(10);
        minus_btn.set_child(Some(&down_icon));
        minus_btn.add_css_class("delay-caret");
        caret_box.append(&minus_btn);

        root.append(&caret_box);

        let state = Rc::new(RefCell::new(SpinnerState {
            val: val.clamp(lower, upper),
            lower,
            upper,
            step,
            digits,
            disabled_label: disabled_label.to_string(),
            normal_tooltip: None,
            disabled_tooltip: None,
        }));
        let cbs: CbList = Rc::new(RefCell::new(vec![]));

        let widget = NumberSpinner {
            root,
            value_btn,
            plus_btn,
            minus_btn,
            state,
            cbs,
        };

        // Wire up plus/minus buttons with press-and-hold acceleration.
        let plus_btn_c = widget.plus_btn.clone();
        let minus_btn_c = widget.minus_btn.clone();
        widget.wire_hold_btn(&plus_btn_c, 1.0);
        widget.wire_hold_btn(&minus_btn_c, -1.0);
        widget.wire_scroll();
        widget.update_carets();

        widget
    }

    /// Wires press-and-hold acceleration for the caret buttons.
    fn wire_hold_btn(&self, btn: &Button, direction: f64) {
        let state_c = Rc::clone(&self.state);
        let cbs_c = Rc::clone(&self.cbs);
        let value_btn_c = self.value_btn.clone();
        let self_c = self.clone();

        let do_step: Rc<dyn Fn(f64)> = Rc::new({
            let state_c2 = Rc::clone(&state_c);
            let cbs_c2 = Rc::clone(&cbs_c);
            move |mult: f64| {
                let (old_val, new_val, digits) = {
                    let state_ref = state_c2.borrow();
                    let step_val = round_to_digits(state_ref.val + direction * state_ref.step * mult, state_ref.digits.max(1));
                    (state_ref.val, step_val.clamp(state_ref.lower, state_ref.upper), state_ref.digits)
                };
                if (new_val - old_val).abs() > 1e-9 {
                    state_c2.borrow_mut().val = new_val;
                    value_btn_c.set_label(&format_value(new_val, digits));
                    self_c.update_carets();
                    for cb in cbs_c2.borrow().iter() {
                        cb();
                    }
                }
            }
        });

        // The running timer's `SourceId` is stored here.
        // Release/cancel calls `glib::source_remove`, which is synchronous, so no stale tick can fire after the button is released.
        let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let gesture = GestureClick::new();

        gesture.connect_pressed({
            let do_step_c = Rc::clone(&do_step);
            let timer_c = Rc::clone(&timer);
            move |_, _, _, _| {
                do_step_c(1.0);
                let do_step_c2 = Rc::clone(&do_step_c);
                let start = Instant::now();
                let src = glib::timeout_add_local(Duration::from_millis(80), move || {
                    let elapsed = start.elapsed().as_millis();
                    if elapsed < 400 {
                        return glib::ControlFlow::Continue;
                    }
                    let mult = if elapsed < 1500 {
                        1.0
                    } else if elapsed < 3000 {
                        5.0
                    } else {
                        10.0
                    };
                    do_step_c2(mult);
                    glib::ControlFlow::Continue
                });
                *timer_c.borrow_mut() = Some(src);
            }
        });
        let stop: Rc<dyn Fn()> = Rc::new({
            let timer_c = Rc::clone(&timer);
            move || {
                if let Some(src) = timer_c.borrow_mut().take() {
                    src.remove();
                }
            }
        });
        gesture.connect_released({
            let stop_c = Rc::clone(&stop);
            move |_, _, _, _| {
                stop_c();
            }
        });
        gesture.connect_cancel({
            let stop_c = Rc::clone(&stop);
            move |_, _| {
                stop_c();
            }
        });

        // Exception for if the mouse leaves the caret while it's being held.
        gesture.connect_unpaired_release({
            let stop_c = Rc::clone(&stop);
            move |_, _, _, _, _| {
                stop_c();
            }
        });
        gesture.connect_end({
            let stop_c = Rc::clone(&stop);
            move |_, _| {
                stop_c();
            }
        });

        btn.add_controller(gesture);
    }

    /// Wires the scroll wheel to step the value.
    fn wire_scroll(&self) {
        let state_c = Rc::clone(&self.state);
        let cbs_c = Rc::clone(&self.cbs);
        let value_btn_c = self.value_btn.clone();
        let self_c = self.clone();
        let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |_, _dx, dy| {
            let (current_val, step, lower, upper, digits) = {
                let state_ref = state_c.borrow();
                (state_ref.val, state_ref.step, state_ref.lower, state_ref.upper, state_ref.digits)
            };
            let delta = if dy > 0.0 { -step } else { step };
            let new_val = round_to_digits(current_val + delta, digits.max(1)).clamp(lower, upper);
            if (new_val - current_val).abs() > 1e-9 {
                state_c.borrow_mut().val = new_val;
                value_btn_c.set_label(&format_value(new_val, digits));
                self_c.update_carets();
                for cb in cbs_c.borrow().iter() {
                    cb();
                }
            }
            glib::Propagation::Stop
        });
        self.root.add_controller(scroll);
    }

    /// Returns the current value of the spinner.
    pub(crate) fn value(&self) -> f64 {
        self.state.borrow().val
    }

    /// Sets the current value of the spinner.
    pub(crate) fn set_value(&self, val: f64) {
        let (new_val, digits, old_val) = {
            let state_ref = self.state.borrow();
            let new_val = round_to_digits(val, state_ref.digits.max(1)).clamp(state_ref.lower, state_ref.upper);
            (new_val, state_ref.digits, state_ref.val)
        };
        self.state.borrow_mut().val = new_val;
        if self.root.is_sensitive() {
            self.value_btn.set_label(&format_value(new_val, digits));
        }
        self.update_carets();
        if (new_val - old_val).abs() > 1e-9 {
            for cb in self.cbs.borrow().iter() {
                cb();
            }
        }
    }

    /// Sets the lower and upper bounds of the spinner's value range.
    pub(crate) fn set_range(&self, lower: f64, upper: f64) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.lower = lower;
        state_mut.upper = upper;
        let clamped = state_mut.val.clamp(lower, upper);
        drop(state_mut);
        self.set_value(clamped);
    }

    /// Sets whether the spinner is sensitive to user input.
    pub(crate) fn set_sensitive(&self, sensitive: bool) {
        self.root.set_sensitive(sensitive);
        let state_ref = self.state.borrow();
        if sensitive {
            self.value_btn.set_label(&format_value(state_ref.val, state_ref.digits));
            if let Some(ref text) = state_ref.normal_tooltip {
                self.apply_tooltip(text);
            }
        }
        // Handling for if the spinner is disabled.
        else {
            self.value_btn.set_label(&state_ref.disabled_label);
            if let Some(ref text) = state_ref.disabled_tooltip {
                self.apply_tooltip(text);
            }
        }
        self.update_carets();
    }

    /// Sets the tooltip text for the spinner.
    pub(crate) fn set_tooltip_text(&self, text: &str) {
        self.state.borrow_mut().normal_tooltip = Some(text.to_string());
        self.apply_tooltip(text);
    }

    /// Sets the tooltip text to be displayed when the spinner is disabled.
    pub(crate) fn set_disabled_tooltip(&self, text: &str) {
        self.state.borrow_mut().disabled_tooltip = Some(text.to_string());
    }

    /// Returns a `(save_lock, restore)` closure pair for map/unmap lifecycle hooks.
    ///
    /// `save_lock()` captures the current value, disables the spinner, and zeroes it.
    /// `restore()` re-enables it and puts the saved value back.
    pub(crate) fn delay_lifecycle(&self) -> (impl Fn() + 'static + use<>, impl Fn() + 'static + use<>) {
        let saved = Rc::new(Cell::new(0.0_f64));
        let saved_c = Rc::clone(&saved);
        let self_c = self.clone();
        let save_lock = move || {
            saved_c.set(self_c.value());
            self_c.set_sensitive(false);
            self_c.set_value(0.0);
        };
        let saved_c = Rc::clone(&saved);
        let self_c = self.clone();
        let restore = move || {
            self_c.set_sensitive(true);
            self_c.set_value(saved_c.get());
        };
        (save_lock, restore)
    }

    /// Applies tooltip text to every sub-widget.
    fn apply_tooltip(&self, text: &str) {
        self.root.set_tooltip_text(Some(text));
        self.value_btn.set_tooltip_text(Some(text));
        self.plus_btn.set_tooltip_text(Some(text));
        self.minus_btn.set_tooltip_text(Some(text));
    }

    /// Registers a callback invoked whenever the value changes.
    pub(crate) fn connect_value_changed(&self, f: impl Fn() + 'static) {
        self.cbs.borrow_mut().push(Box::new(f));
    }

    /// Updates the opacity and targetability of the caret icons based on current value bounds.
    fn update_carets(&self) {
        let is_sensitive = self.root.is_sensitive();
        let (val, lower, upper) = {
            let state_ref = self.state.borrow();
            (state_ref.val, state_ref.lower, state_ref.upper)
        };

        let plus_active = !is_sensitive || (upper - val) > 1e-9;
        self.plus_btn.set_can_target(plus_active);
        if let Some(child) = self.plus_btn.child() {
            child.set_opacity(if plus_active { 1.0 } else { 0.3 });
        }

        let minus_active = !is_sensitive || (val - lower) > 1e-9;
        self.minus_btn.set_can_target(minus_active);
        if let Some(child) = self.minus_btn.child() {
            child.set_opacity(if minus_active { 1.0 } else { 0.3 });
        }
    }

    /// Returns the root `Box` widget.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ── ChooseOnePill ──────────────────────────────────────────────────────────────────────────────────────────
// Horizontal pill where exactly one entry is active at a time.

/// A horizontal pill of `ToggleButtons` where exactly one entry is active at a time.
#[derive(Clone)]
pub(crate) struct ChooseOnePill {
    pub root: gtk4::Box,
    btns: Vec<ToggleButton>,
    is_updating: Rc<Cell<bool>>,
    cbs: IntCbList,
}

impl ChooseOnePill {
    /// Initializes the single-choice pill with a set of labels and a default active index.
    pub(crate) fn new(labels: &[&str], active: usize) -> Self {
        let root = gtk4::Box::new(Orientation::Horizontal, 0);
        root.add_css_class("linked");
        let is_updating: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let cbs: IntCbList = Rc::new(RefCell::new(vec![]));
        let mut btns: Vec<ToggleButton> = Vec::new();

        // Creates toggle buttons and groups them for single-selection behavior.
        for (i, &label) in labels.iter().enumerate() {
            let btn = ToggleButton::with_label(label);
            if i > 0 {
                btn.set_group(Some(&btns[0]));
            }
            btns.push(btn.clone());
            root.append(&btn);
        }

        let widget = ChooseOnePill {
            root,
            btns,
            is_updating,
            cbs,
        };
        widget.set_active(active);

        for (i, btn) in widget.btns.iter().enumerate() {
            let updating_c = Rc::clone(&widget.is_updating);
            let cbs_c = Rc::clone(&widget.cbs);
            btn.connect_toggled(move |toggled_btn| {
                if !updating_c.get() && toggled_btn.is_active() {
                    for cb in cbs_c.borrow().iter() {
                        cb(i as i32);
                    }
                }
            });
        }
        widget
    }

    /// Returns the index of the currently active button.
    pub(crate) fn active(&self) -> i32 {
        self.btns
            .iter()
            .enumerate()
            .find(|(_, btn)| btn.is_active())
            .map_or(-1, |(i, _)| i as i32)
    }

    /// Sets the active button by index.
    pub(crate) fn set_active(&self, idx: usize) {
        if let Some(btn) = self.btns.get(idx) {
            self.is_updating.set(true);
            btn.set_active(true);
            self.is_updating.set(false);
        }
    }

    /// Grays out one choice, for when it can't apply to whatever is currently loaded.
    pub(crate) fn set_item_sensitive(&self, idx: usize, is_sensitive: bool) {
        if let Some(btn) = self.btns.get(idx) {
            btn.set_sensitive(is_sensitive);
        }
    }

    /// Connects a callback that fires when the active button changes.
    pub(crate) fn connect_changed(&self, f: impl Fn(i32) + 'static) {
        self.cbs.borrow_mut().push(Box::new(f));
    }

    /// Returns the root `Box` widget.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ── ChooseMultiPill ────────────────────────────────────────────────────────────────────────────────────────
// Pill of `ToggleButtons` where any count can be active.

/// A horizontal pill of `ToggleButtons` where any number can be active simultaneously.
#[derive(Clone)]
pub(crate) struct ChooseMultiPill {
    pub root: gtk4::Box,
    btns: Vec<ToggleButton>,
    is_updating: Rc<Cell<bool>>,
    cbs: ValueCbList,
}

impl ChooseMultiPill {
    /// Initializes the multi-toggle pill with a set of labels and initial active states.
    pub(crate) fn new(labels: &[&str], active_indices: &[usize]) -> Self {
        let root = gtk4::Box::new(Orientation::Horizontal, 0);
        root.add_css_class("linked");
        let is_updating: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let cbs: ValueCbList = Rc::new(RefCell::new(vec![]));
        let active_set: std::collections::HashSet<usize> = active_indices.iter().copied().collect();
        let mut btns = Vec::new();

        // Creates and connects a toggle button for each label.
        for (i, &label) in labels.iter().enumerate() {
            let btn = ToggleButton::with_label(label);
            btn.set_active(active_set.contains(&i));
            root.append(&btn);
            btns.push(btn);
        }

        let widget = ChooseMultiPill {
            root,
            btns,
            is_updating,
            cbs,
        };

        // Wire up toggled signals after all buttons are created.
        for (i, btn) in widget.btns.iter().enumerate() {
            let updating_c = Rc::clone(&widget.is_updating);
            let cbs_c = Rc::clone(&widget.cbs);
            let all_btns_c = widget.btns.clone();
            btn.connect_toggled(move |toggled_btn| {
                if updating_c.get() {
                    return;
                }
                if !toggled_btn.is_active() {
                    // Don't allow the last active button to be deactivated.
                    let has_active = all_btns_c.iter().any(gtk4::prelude::ToggleButtonExt::is_active);
                    if !has_active {
                        updating_c.set(true);
                        toggled_btn.set_active(true);
                        updating_c.set(false);
                        return;
                    }
                }
                for cb in cbs_c.borrow().iter() {
                    cb(i as i32, toggled_btn.is_active());
                }
            });
        }
        widget
    }

    /// Connects a callback that fires when any toggle button changes state.
    pub(crate) fn connect_changed(&self, f: impl Fn(i32, bool) + 'static) {
        self.cbs.borrow_mut().push(Box::new(f));
    }

    /// Returns the root `Box` widget.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ── CustomDropdown ─────────────────────────────────────────────────────────────────────────────────────────
// `DropDown` with scroll-cycling.
//
// If GTK4 ever adds type-to-jump to its Dropdown widget, it'd be a great addition here.

/// `DropDown` that always blocks scroll propagation and cycles entries on scroll.
///
/// When `default` is set, double-click or middle-click resets to default, and right-click jumps to a random entry.
#[derive(Clone)]
pub(crate) struct CustomDropdown {
    pub inner: gtk4::DropDown,
}

impl std::ops::Deref for CustomDropdown {
    type Target = gtk4::DropDown;
    /// Borrows the underlying `DropDown` widget.
    fn deref(&self) -> &gtk4::DropDown {
        &self.inner
    }
}

impl CustomDropdown {
    /// Initializes the dropdown, setting up scroll-cycling.
    pub(crate) fn new(default: Option<i32>) -> Self {
        let inner = gtk4::DropDown::new(Some(gtk4::StringList::new(&[])), None::<gtk4::Expression>);

        let widget = CustomDropdown { inner };
        widget.setup_scroll();
        widget.setup_click(default);
        widget
    }

    /// Appends a text entry to the dropdown.
    pub(crate) fn append_text(&self, text: &str) {
        if let Some(string_list) = self.inner.model().and_downcast::<gtk4::StringList>() {
            string_list.append(text);
        }
    }

    /// Removes all entries from the dropdown.
    pub(crate) fn remove_all(&self) {
        self.inner.set_model(Some(&gtk4::StringList::new(&[])));
    }

    /// Returns the selected index.
    /// Returns `None` if nothing is selected.
    pub(crate) fn active(&self) -> Option<u32> {
        let idx = self.inner.selected();
        if idx == gtk4::INVALID_LIST_POSITION { None } else { Some(idx) }
    }

    /// Sets the selected item by index (`None` clears the selection).
    pub(crate) fn set_active(&self, idx: Option<u32>) {
        self.inner.set_selected(idx.unwrap_or(gtk4::INVALID_LIST_POSITION));
    }

    /// Returns the text of the selected item, if any.
    pub(crate) fn active_text(&self) -> Option<glib::GString> {
        let idx = self.inner.selected();
        if idx == gtk4::INVALID_LIST_POSITION {
            return None;
        }
        self.inner
            .model()
            .and_downcast::<gtk4::StringList>()
            .and_then(|string_list| string_list.string(idx))
    }

    /// Connects a callback that fires when the selected item changes.
    ///
    /// The argument is the new index, or `None` if the selection was cleared.
    pub(crate) fn connect_changed<F: Fn(Option<u32>) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.inner.connect_selected_notify(move |dropdown| {
            let idx = dropdown.selected();
            f(if idx == gtk4::INVALID_LIST_POSITION { None } else { Some(idx) });
        })
    }

    /// Wires the scroll wheel to cycle through the dropdown items.
    fn setup_scroll(&self) {
        let inner_c = self.inner.clone();
        let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(PropagationPhase::Capture);
        scroll.connect_scroll(move |_, _dx, dy| {
            let num_items = inner_c.model().map_or(0, |m| m.n_items());
            if num_items > 0 {
                let selected_idx = inner_c.selected();
                let current_idx = if selected_idx == gtk4::INVALID_LIST_POSITION {
                    0
                } else {
                    selected_idx
                };
                let idx = if dy > 0.0 {
                    (current_idx + 1).min(num_items - 1)
                } else {
                    current_idx.saturating_sub(1)
                };
                inner_c.set_selected(idx);
            }
            glib::Propagation::Stop
        });
        self.inner.add_controller(scroll);
    }

    /// Wires double-click or middle-click (reset to default) and right-click (randomize) on the dropdown.
    fn setup_click(&self, default: Option<i32>) {
        let inner_c = self.inner.clone();
        let click = GestureClick::new();
        click.set_button(0);
        click.connect_pressed(move |gesture, num_presses, _x, _y| {
            inner_c.grab_focus();
            let btn = gesture.current_button();
            let num_items = inner_c.model().map_or(0, |m| m.n_items());

            if ((btn == 1 && num_presses == 2) || btn == 2) && default.is_some() {
                inner_c.set_selected(default.unwrap_or(0) as u32);
            } else if btn == 3 && num_items > 0 {
                inner_c.set_selected(pseudo_rand(num_items - 1));
            }
        });
        self.inner.add_controller(click);
    }
}

/// Entry that loses focus (and its cursor/selection highlight) when the user clicks elsewhere in the window or presses Escape.
///
/// Plain `gtk4::Entry` has no such behavior.
/// Focus only moves when another focusable widget claims it, so without this the highlight stays until the user clicks something.
#[derive(Clone)]
pub(crate) struct CustomTextbox {
    pub inner: gtk4::Entry,
}

impl std::ops::Deref for CustomTextbox {
    type Target = gtk4::Entry;
    /// Borrows the underlying Entry widget.
    fn deref(&self) -> &gtk4::Entry {
        &self.inner
    }
}

impl CustomTextbox {
    /// Creates a new `CustomTextbox` with click-outside/Escape defocus already wired up.
    pub(crate) fn new() -> Self {
        let inner = gtk4::Entry::new();
        let widget = CustomTextbox { inner };
        widget.setup_defocus();
        widget
    }

    /// Restricts live input to uppercase Elektron-allowed characters, preserving cursor position when characters are stripped.
    pub(crate) fn enable_elektron_filter(&self) {
        let entry_c = self.inner.clone();
        self.inner.connect_changed(move |entry| {
            let text = entry.text().to_string();
            let filtered = filter_elektron_name(&text);
            if filtered != text {
                let pos = entry.position();
                entry_c.set_text(&filtered);
                entry_c.set_position(pos.min(filtered.len() as i32));
            }
        });
    }

    /// Wires Escape and click-outside to clear focus.
    ///
    /// Escape is handled directly on the entry, since key events go to whichever widget is focused.
    /// Click-outside can't work that way: an entry-only controller never sees clicks elsewhere.
    /// So this waits for the entry to be mapped, then attaches a whole-window Capture-phase click listener instead.
    /// Clicking the entry itself is unaffected: its own bubble-phase click handler re-grabs focus right after, in the same gesture.
    fn setup_defocus(&self) {
        let key_controller = EventControllerKey::new();
        let entry_c = self.inner.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gtk4::gdk::Key::Escape {
                if let Some(root) = entry_c.root() {
                    root.set_focus(None::<&gtk4::Widget>);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.inner.add_controller(key_controller);

        let entry_for_map_handler = self.inner.clone();
        let is_wired = Rc::new(Cell::new(false));
        self.inner.connect_map(move |entry_mapped| {
            if is_wired.get() {
                return;
            }
            let Some(root) = entry_mapped.root() else {
                return;
            };
            is_wired.set(true);
            let click = GestureClick::new();
            click.set_propagation_phase(PropagationPhase::Capture);
            let entry_for_click_handler = entry_for_map_handler.clone();
            click.connect_pressed(move |_, _, _, _| {
                if entry_for_click_handler.has_focus()
                    && let Some(root) = entry_for_click_handler.root()
                {
                    root.set_focus(None::<&gtk4::Widget>);
                }
            });
            root.add_controller(click);
        });
    }
}

impl Default for CustomTextbox {
    /// Returns a new instance of `CustomTextbox` with default settings.
    fn default() -> Self {
        Self::new()
    }
}

// ── ParameterKnob ──────────────────────────────────────────────────────────────────────────────────────────
// Custom-drawn rotary knob with drag/scroll/click support.

/// Builds the short-name label shared above `ParameterKnob`, `ParameterCombo`, and `ParameterShape`.
fn build_parameter_label(short_name: &str, full_name: &str) -> Label {
    let label = Label::new(Some(short_name));
    label.add_css_class("monospace");
    label.set_tooltip_text(Some(full_name));
    label
}

struct KnobState {
    param_id: u8,
    val: i32,
    default_val: i32,
    display_offset: i32,
    min_val: i32,
    max_val: i32,
    is_bipolar: bool,
    max_seconds: Option<f64>,
    start_val: i32,
    snap_back_value: Option<i32>,
    on_change: Box<dyn Fn(u8, i32, bool)>,
}

impl KnobState {
    /// Returns the current value in seconds.
    /// Returns `None` on a knob with no `max_seconds` scale.
    fn value_seconds(&self) -> Option<f64> {
        let max_seconds = self.max_seconds?;
        let range = f64::from(self.max_val - self.min_val);
        Some(f64::from(self.val - self.min_val) / range * max_seconds)
    }
}

/// A custom-drawn rotary knob widget for managing MIDI parameters with drag and scroll support.
#[derive(Clone)]
pub(crate) struct ParameterKnob {
    pub root: gtk4::Box,
    pub drawing: DrawingArea,
    /// Short-name label shown above the knob, publicly settable for machine-specific renaming.
    pub label: Label,
    state: Rc<RefCell<KnobState>>,
}

impl ParameterKnob {
    /// Initializes the knob with scaling, defaults, and drawing configuration.
    pub(crate) fn new(
        short_name: &str,
        full_name: &str,
        param_id: u8,
        default_val: i32,
        on_change: impl Fn(u8, i32, bool) + 'static,
        display_offset: i32,
        min_val: i32,
        max_val: i32,
        is_bipolar: bool,
        max_seconds: Option<f64>,
    ) -> Self {
        let root = gtk4::Box::new(Orientation::Vertical, 6);
        let label = build_parameter_label(short_name, full_name);
        root.append(&label);

        let drawing = DrawingArea::new();
        drawing.set_size_request(56, 56);
        drawing.set_focusable(true);
        root.append(&drawing);

        let state = Rc::new(RefCell::new(KnobState {
            param_id,
            val: default_val,
            default_val,
            display_offset,
            min_val,
            max_val,
            is_bipolar,
            max_seconds,
            start_val: default_val,
            snap_back_value: None,
            on_change: Box::new(on_change),
        }));

        {
            let state_c = Rc::clone(&state);
            drawing.set_draw_func(move |da, cr, width, height| draw_knob(cr, width, height, &state_c.borrow(), da.color()));
        }

        let widget = ParameterKnob {
            root,
            drawing,
            label,
            state,
        };
        widget.wire_drag();
        widget.wire_scroll();
        widget.wire_click();
        widget
    }

    /// Wires the drag gesture to change the knob value.
    fn wire_drag(&self) {
        let state_c_begin = Rc::clone(&self.state);
        let drawing_c_begin = self.drawing.clone();
        let state_c_update = Rc::clone(&self.state);
        let drawing_c_update = self.drawing.clone();
        let state_c_end = Rc::clone(&self.state);
        let drawing_c_end = self.drawing.clone();

        let drag = GestureDrag::new();
        drag.connect_drag_begin(move |_, _x, _y| {
            drawing_c_begin.grab_focus();
            let current_val = state_c_begin.borrow().val;
            state_c_begin.borrow_mut().start_val = current_val;
        });

        drag.connect_drag_update(move |_, _ox, oy| {
            let start = f64::from(state_c_update.borrow().start_val);
            let new_f = start - (oy * 0.7);
            let new_val = new_f as i32;
            let (min, max) = {
                let state_ref = state_c_update.borrow();
                (state_ref.min_val, state_ref.max_val)
            };
            let clamped = new_val.clamp(min, max);
            if clamped != state_c_update.borrow().val {
                let param_id = state_c_update.borrow().param_id;
                state_c_update.borrow_mut().val = clamped;
                drawing_c_update.queue_draw();
                (state_c_update.borrow().on_change)(param_id, clamped, false);
            }
        });

        drag.connect_drag_end(move |_, _, _| {
            let snap_back = state_c_end.borrow().snap_back_value;
            if let Some(snap) = snap_back {
                let param_id = state_c_end.borrow().param_id;
                state_c_end.borrow_mut().val = snap;
                drawing_c_end.queue_draw();
                (state_c_end.borrow().on_change)(param_id, snap, false);
            }
        });
        self.drawing.add_controller(drag);
    }

    /// Snaps the knob back to `value` whenever the user releases the drag.
    pub(crate) fn set_spring(&self, val: i32) {
        self.state.borrow_mut().snap_back_value = Some(val);
    }

    /// Wires the scroll wheel to step the knob value.
    fn wire_scroll(&self) {
        let state_c = Rc::clone(&self.state);
        let drawing_c = self.drawing.clone();
        let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(PropagationPhase::Capture);
        scroll.connect_scroll(move |_, _dx, dy| {
            let delta = if dy > 0.0 { -2i32 } else { 2 };
            let (current_val, min, max, param_id) = {
                let state_ref = state_c.borrow();
                (state_ref.val, state_ref.min_val, state_ref.max_val, state_ref.param_id)
            };
            let new_val = (current_val + delta).clamp(min, max);
            if new_val != current_val {
                state_c.borrow_mut().val = new_val;
                drawing_c.queue_draw();
                (state_c.borrow().on_change)(param_id, new_val, false);
            }
            glib::Propagation::Stop
        });
        self.drawing.add_controller(scroll);
    }

    /// Wires double-click (reset to default), middle-click (center), and right-click (randomize) on the knob.
    fn wire_click(&self) {
        let state_c = Rc::clone(&self.state);
        let drawing_c = self.drawing.clone();
        let click = GestureClick::new();
        click.set_button(0);
        click.connect_pressed(move |gesture, num_presses, _x, _y| {
            drawing_c.grab_focus();
            let btn = gesture.current_button();
            let (default_val, is_bipolar, min, max, current_val, param_id) = {
                let state_ref = state_c.borrow();
                (
                    state_ref.default_val,
                    state_ref.is_bipolar,
                    state_ref.min_val,
                    state_ref.max_val,
                    state_ref.val,
                    state_ref.param_id,
                )
            };

            if btn == 1 && num_presses == 2 {
                let target = default_val;
                if target != current_val {
                    state_c.borrow_mut().val = target;
                    drawing_c.queue_draw();
                    (state_c.borrow().on_change)(param_id, target, false);
                }
            } else if btn == 2 {
                let target = if is_bipolar { 0 } else { 64 };
                if target != current_val {
                    state_c.borrow_mut().val = target;
                    drawing_c.queue_draw();
                    (state_c.borrow().on_change)(param_id, target, false);
                }
            } else if btn == 3 {
                let rand_val = (pseudo_rand((max - min) as u32) as i32 + min).clamp(min, max);
                state_c.borrow_mut().val = rand_val;
                drawing_c.queue_draw();
                (state_c.borrow().on_change)(param_id, rand_val, true);
            }
        });

        self.drawing.add_controller(click);
    }

    /// Sets the current value of the knob, optionally triggering the change callback.
    pub(crate) fn set_value(&self, val: i32, should_trigger_cb: bool) {
        let (new_val, current_val, param_id, _min, _max) = {
            let state_ref = self.state.borrow();
            let clamped_val = val.clamp(state_ref.min_val, state_ref.max_val);
            (clamped_val, state_ref.val, state_ref.param_id, state_ref.min_val, state_ref.max_val)
        };
        if new_val != current_val {
            self.state.borrow_mut().val = new_val;
            self.drawing.queue_draw();
            if should_trigger_cb {
                (self.state.borrow().on_change)(param_id, new_val, false);
            }
        }
    }

    /// Returns the root `Box` widget.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Returns the maximum allowable value for this knob.
    pub(crate) fn max_val(&self) -> i32 {
        self.state.borrow().max_val
    }

    /// Returns the default value for this knob.
    pub(crate) fn default_val(&self) -> i32 {
        self.state.borrow().default_val
    }
}

/// Renders the rotary knob using Cairo.
fn draw_knob(cr: &cairo::Context, width: i32, height: i32, state_ref: &KnobState, text_color: gdk::RGBA) {
    let cx = f64::from(width) / 2.0;
    let cy = f64::from(height) / 2.0;
    let radius = f64::from(width.min(height)) / 2.0 - 4.0;
    let start_angle = PI * 0.75;
    let end_angle = PI * 2.25;

    cr.set_line_width(5.0);
    cr.set_line_cap(LineCap::Round);
    cr.set_source_rgba(0.5, 0.5, 0.5, 0.2);
    cr.arc(cx, cy, radius, start_angle, end_angle);
    let _ = cr.stroke();

    let range_val = f64::from(state_ref.max_val - state_ref.min_val);
    let normalized = f64::from(state_ref.val - state_ref.min_val) / range_val;

    let val_angle = start_angle + normalized * (end_angle - start_angle);
    cr.set_source_rgba(COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B, 1.0);

    // Renders this way if the knob is bipolar.
    if state_ref.is_bipolar {
        let mid_angle = f64::midpoint(start_angle, end_angle);

        if val_angle > mid_angle {
            cr.arc(cx, cy, radius, mid_angle, val_angle);
        } else if val_angle < mid_angle {
            cr.arc_negative(cx, cy, radius, mid_angle, val_angle);
        }
    }
    // Renders a different way if the knob isn't bipolar.
    else {
        cr.arc(cx, cy, radius, start_angle, val_angle);
    }
    let _ = cr.stroke();

    // Thumb dot marking the current value.
    //
    // Used by both bipolar and single-direction knobs.
    let tx = cx + radius * val_angle.cos();
    let ty = cy + radius * val_angle.sin();
    cr.arc(tx, ty, 3.5, 0.0, 2.0 * PI);
    let _ = cr.fill();

    // Makes the inner-text change color.
    //
    // Uses the text coloring from the GTK theme.
    cr.set_source_color(&text_color);
    let font_size = if state_ref.max_seconds.is_some() { 12.0 } else { 13.0 };
    cr.set_font_size(font_size);
    cr.select_font_face("monospace", FontSlant::Normal, FontWeight::Bold);

    // Defines the number that lives inside of the knob.
    let display_text = if let Some(max_secs) = state_ref.max_seconds {
        let val_secs = state_ref.value_seconds().unwrap_or(0.0);
        if state_ref.val == state_ref.min_val {
            "0".to_string()
        } else if state_ref.val == state_ref.max_val {
            format!("{max_secs:.2}")
        } else {
            format!("{val_secs:.2}").trim_start_matches('0').to_string()
        }
    }
    // Bipolar knob:
    // The raw value with an explicit sign, a leading '+' on positives and the natural '-' on negatives.
    else if state_ref.is_bipolar {
        let raw_val = state_ref.val;
        if raw_val > 0 { format!("+{raw_val}") } else { format!("{raw_val}") }
    }
    // Standard knob:
    // The value shifted by `display_offset`, with a leading '+' when the offset makes it positive.
    else {
        let display_val = state_ref.val + state_ref.display_offset;
        let display_val_str = display_val.to_string();
        if state_ref.display_offset != 0 && display_val > 0 {
            format!("+{display_val_str}")
        } else {
            display_val_str
        }
    };

    // The part that draws the inner-knob number.
    if let Ok(ext) = cr.text_extents(&display_text) {
        cr.move_to(cx - ext.width() / 2.0, cy + ext.height() / 2.0);
        let _ = cr.show_text(&display_text);
    }
}

// ── ParameterCombo ─────────────────────────────────────────────────────────────────────────────────────────
// `CustomDropdown`-based parameter selector.

struct ComboState {
    param_id: u8,
    val: i32,
    default_val: i32,
    max_idx: i32,
    is_ignoring_cb: bool,
    on_change: Box<dyn Fn(u8, i32)>,
}

/// A dropdown-based parameter selector that synchronizes with hardware MIDI IDs.
#[derive(Clone)]
pub(crate) struct ParameterCombo {
    pub root: gtk4::Box,
    pub combo: CustomDropdown,
    state: Rc<RefCell<ComboState>>,
}

impl ParameterCombo {
    /// Initializes the combo box with labels, options, and a change callback.
    pub(crate) fn new(
        short_name: &str,
        full_name: &str,
        param_id: u8,
        default_val: Option<i32>,
        options: &[&str],
        on_change: impl Fn(u8, i32) + 'static,
    ) -> Self {
        let root = gtk4::Box::new(Orientation::Vertical, 6);
        root.append(&build_parameter_label(short_name, full_name));

        let combo = CustomDropdown::new(default_val);
        let default_val = default_val.unwrap_or(0);
        for option in options {
            combo.append_text(option);
        }
        combo.set_active(Some(0));
        root.append(&*combo);

        let state = Rc::new(RefCell::new(ComboState {
            param_id,
            val: 0,
            default_val,
            max_idx: options.len() as i32 - 1,
            is_ignoring_cb: false,
            on_change: Box::new(on_change),
        }));

        {
            let state_c = Rc::clone(&state);
            combo.connect_changed(move |idx| {
                if state_c.borrow().is_ignoring_cb {
                    return;
                }
                if let Some(val) = idx {
                    let val = val as i32;
                    let param_id = state_c.borrow().param_id;
                    state_c.borrow_mut().val = val;
                    (state_c.borrow().on_change)(param_id, val);
                }
            });
        }

        ParameterCombo { root, combo, state }
    }

    /// Sets the selected index of the combo box, optionally triggering the change callback.
    pub(crate) fn set_value(&self, val: i32, should_trigger_cb: bool) {
        let (max_idx, current_val) = {
            let state_ref = self.state.borrow();
            (state_ref.max_idx, state_ref.val)
        };
        // Do NOT clamp here: dependent combo boxes (like LFO DEST) need their target value during kit loads.
        // That happens before the PAGE combo fills in their options.
        // It will be clamped when `set_options()` is called.
        if val != current_val {
            self.state.borrow_mut().val = val;
            self.state.borrow_mut().is_ignoring_cb = !should_trigger_cb;
            if val <= max_idx {
                self.combo.set_active(Some(val as u32));
            }
            self.state.borrow_mut().is_ignoring_cb = false;
            if should_trigger_cb {
                let (param_id, value) = {
                    let state_ref = self.state.borrow();
                    (state_ref.param_id, state_ref.val)
                };
                (self.state.borrow().on_change)(param_id, value);
            }
        }
    }

    /// Sets the options dynamically.
    pub(crate) fn set_options(&self, options: &[&str]) {
        self.state.borrow_mut().is_ignoring_cb = true;
        self.combo.remove_all();
        for option in options {
            self.combo.append_text(option);
        }
        let new_max = (options.len() as i32 - 1).max(0);
        self.state.borrow_mut().max_idx = new_max;

        let current_val = self.state.borrow().val;
        let clamped = current_val.clamp(0, new_max);
        self.state.borrow_mut().val = clamped;
        self.combo.set_active(None);
        self.combo.set_active(Some(clamped as u32));
        self.state.borrow_mut().is_ignoring_cb = false;

        if clamped != current_val {
            let param_id = self.state.borrow().param_id;
            (self.state.borrow().on_change)(param_id, clamped);
        }
    }

    /// Returns the root `Box` widget.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Returns the current value.
    pub(crate) fn value(&self) -> i32 {
        self.state.borrow().val
    }

    /// Returns the maximum allowable index for this combo box.
    pub(crate) fn max_idx(&self) -> i32 {
        self.state.borrow().max_idx
    }

    /// Returns the default value for this combo box.
    pub(crate) fn default_val(&self) -> i32 {
        self.state.borrow().default_val
    }
}

// ── ParameterShape ─────────────────────────────────────────────────────────────────────────────────────────
// Custom drawing widget for LFO/oscillator waveform display.

type Vertices = Vec<(f64, f64)>;

/// Returns vertices of an exponential curve for waveform visualization.
fn exponential_vertices() -> Vertices {
    let mut vertices: Vertices = vec![(0.0, 8.0), (1.0, 8.0), (2.0, 7.0), (2.0, 0.0)];
    for i in 1..=16 {
        let fraction = f64::from(i) / 16.0;
        vertices.push((2.0 + 8.0 * fraction, 8.0 * (1.0 - (1.0 - fraction).powi(2))));
    }
    vertices.push((16.0, 8.0));
    vertices
}

/// Returns vertices of a sine for waveform visualization.
fn sine_vertices() -> Vertices {
    let mut vertices: Vertices = vec![(0.0, 4.0)];
    for i in 1..=32 {
        let x = f64::from(i) / 2.0;
        vertices.push((x, 4.0 - 4.0 * (PI * (x / 8.0)).sin()));
    }
    vertices
}

/// Vertically mirrors the provided vertices within an 8-unit high coordinate space.
fn mirror_vertices(vertices: &[(f64, f64)]) -> Vertices {
    vertices.iter().map(|&(x, y)| (x, 8.0 - y)).collect()
}

/// Returns standard vertices for named waveform shapes (TRI, SAW, SQR, etc.).
fn named_shape_vertices(name: &str) -> Option<Vertices> {
    match name {
        // Standard shapes:
        "TRI" => Some(vec![(0.0, 4.0), (4.0, 0.0), (12.0, 8.0), (16.0, 4.0)]),
        "SAW" => Some(vec![(0.0, 8.0), (4.0, 8.0), (12.0, 0.0), (16.0, 0.0)]),
        "SQR" => Some(vec![(0.0, 4.0), (0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (16.0, 8.0), (16.0, 4.0)]),
        "EXP" => Some(exponential_vertices()),
        "SINE" => Some(sine_vertices()),
        "RMP" => Some(vec![(0.0, 8.0), (8.0, 0.0), (8.0, 8.0), (16.0, 8.0)]),
        "RND" => Some(vec![
            (0.0, 2.0),
            (2.0, 2.0),
            (2.0, 5.0),
            (4.0, 5.0),
            (4.0, 6.0),
            (6.0, 6.0),
            (6.0, 1.0),
            (8.0, 1.0),
            (8.0, 4.0),
            (10.0, 4.0),
            (10.0, 2.0),
            (12.0, 2.0),
            (12.0, 8.0),
            (14.0, 8.0),
            (14.0, 5.0),
            (16.0, 5.0),
        ]),
        "PLS" => Some(vec![(0.0, 6.0), (8.0, 6.0), (8.0, 2.0), (16.0, 2.0)]),
        "HTRI" => Some(vec![(0.0, 8.0), (4.0, 4.0), (8.0, 8.0), (16.0, 8.0)]),

        // Inverted shape variants:
        "ITRI" => named_shape_vertices("TRI").map(|value| mirror_vertices(&value)),
        "ISAW" => named_shape_vertices("SAW").map(|value| mirror_vertices(&value)),
        "ISQR" => named_shape_vertices("SQR").map(|value| mirror_vertices(&value)),
        "IEXP" => named_shape_vertices("EXP").map(|value| mirror_vertices(&value)),
        "IRMP" => named_shape_vertices("RMP").map(|value| mirror_vertices(&value)),
        "IRND" => named_shape_vertices("RND").map(|value| mirror_vertices(&value)),
        "IPLS" => named_shape_vertices("PLS").map(|value| mirror_vertices(&value)),
        "IHTRI" => named_shape_vertices("HTRI").map(|value| mirror_vertices(&value)),

        // LFO shape variants:
        // MD developers thought it would be hilarious to make the LFO RMP shape different than on the MnM.
        "LFO_RMP" => Some(vec![(0.0, 8.0), (1.0, 8.0), (2.0, 7.0), (2.0, 0.0), (10.0, 8.0), (16.0, 8.0)]),

        // Unrecognized name yields `None`.
        // Structurally necessary for the "`LFO_X`" fallthrough to work.
        _ => None,
    }
}

/// Finds the vertex list variant for a shape wanted by the machine.
///
/// A name like 'RMP' resolves to '`LFO_RMP`' for the LFO, and falls back to the base shape without an LFO variant.
fn get_shape_vertices(val: usize, options: Option<&[String]>, prefix: &str) -> Vertices {
    let name = options.unwrap()[val].as_str();
    named_shape_vertices(&format!("{prefix}{name}"))
        .or_else(|| named_shape_vertices(name))
        .unwrap()
}

/// Returns the Y-coordinate of the line at a given X-coordinate.
///
/// Morph rendering samples both shapes at the same X before interpolating between them.
fn get_y_at_x(vertices: &[(f64, f64)], target_x: f64) -> f64 {
    let mut found_y = vertices[0].1;
    for i in 0..vertices.len().saturating_sub(1) {
        let (x1, y1) = vertices[i];
        let (x2, y2) = vertices[i + 1];
        if x1 <= target_x && target_x <= x2 {
            found_y = if x2 == x1 {
                y2
            } else {
                y1 + (target_x - x1) / (x2 - x1) * (y2 - y1)
            };
        }
    }
    found_y
}

struct ShapeState {
    short_name: String,
    param_id: u8,
    val: i32,
    default_val: i32,
    max_idx: i32,
    options: Vec<String>,
    get_shapes: Option<Rc<dyn Fn() -> (usize, usize)>>,

    /// Name prefix used when resolving shape vertices, like "`LFO_`" to pick LFO-specific variants.
    shape_prefix: String,
    start_val: i32,
    on_change: Box<dyn Fn(u8, i32, bool)>,
}

/// A custom drawing widget that visualizes LFO or oscillator waveforms with interactive morphing.
#[derive(Clone)]
pub(crate) struct ParameterShape {
    pub root: gtk4::Box,
    pub drawing: DrawingArea,
    state: Rc<RefCell<ShapeState>>,
}

impl ParameterShape {
    /// Initializes the shape widget, supporting both static lists and dynamic morphing lookups.
    pub(crate) fn new(
        short_name: &str,
        full_name: &str,
        param_id: u8,
        default_val: Option<i32>,
        on_change: impl Fn(u8, i32, bool) + 'static,
        options: Option<Vec<String>>,
        get_shapes: Option<Box<dyn Fn() -> (usize, usize)>>,
        shape_prefix: Option<&str>,
    ) -> Self {
        let root = gtk4::Box::new(Orientation::Vertical, 6);
        root.set_halign(Align::Center);
        root.append(&build_parameter_label(short_name, full_name));

        let drawing = DrawingArea::new();
        drawing.set_size_request(85, 45);
        drawing.set_focusable(true);
        root.append(&drawing);

        let default_val = default_val.unwrap_or(0);
        let max_idx = if get_shapes.is_some() {
            127
        } else {
            options.as_ref().map_or(10, |options| options.len() as i32 - 1)
        };

        let state = Rc::new(RefCell::new(ShapeState {
            short_name: short_name.to_string(),
            param_id,
            val: 0,
            default_val,
            max_idx,
            options: options.unwrap_or_default(),
            get_shapes: get_shapes.map(Rc::from),
            shape_prefix: shape_prefix.unwrap_or("").to_string(),
            start_val: 0,
            on_change: Box::new(on_change),
        }));

        {
            let state_c = Rc::clone(&state);
            drawing.set_draw_func(move |_, cr, width, height| draw_shape(cr, width, height, &state_c.borrow()));
        }

        let widget = ParameterShape { root, drawing, state };
        widget.wire_drag();
        widget.wire_scroll();
        widget.wire_click();
        widget
    }

    /// Wires the drag gesture to change the shape value.
    fn wire_drag(&self) {
        let state_c_begin = Rc::clone(&self.state);
        let drawing_c_begin = self.drawing.clone();
        let state_c_update = Rc::clone(&self.state);
        let drawing_c_update = self.drawing.clone();
        let drag = GestureDrag::new();

        drag.connect_drag_begin(move |_, _x, _y| {
            drawing_c_begin.grab_focus();
            let current_val = state_c_begin.borrow().val;
            state_c_begin.borrow_mut().start_val = current_val;
        });
        drag.connect_drag_update(move |_, _ox, oy| {
            let (start, max_idx) = {
                let state_ref = state_c_update.borrow();
                (f64::from(state_ref.start_val), state_ref.max_idx)
            };
            let scale = if max_idx < 20 { 0.1 } else { 0.7 };
            let new_val = (start - oy * scale) as i32;
            let clamped = new_val.clamp(0, max_idx);
            let old_val = state_c_update.borrow().val;
            if clamped != old_val {
                let param_id = state_c_update.borrow().param_id;
                state_c_update.borrow_mut().val = clamped;
                drawing_c_update.queue_draw();
                (state_c_update.borrow().on_change)(param_id, clamped, false);
            }
        });
        self.drawing.add_controller(drag);
    }

    /// Wires the scroll wheel to step the shape value.
    fn wire_scroll(&self) {
        let state_c = Rc::clone(&self.state);
        let drawing_c = self.drawing.clone();
        let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(PropagationPhase::Capture);
        scroll.connect_scroll(move |_, _dx, dy| {
            let (current_val, max_idx, param_id) = {
                let state_ref = state_c.borrow();
                (state_ref.val, state_ref.max_idx, state_ref.param_id)
            };
            let new_val = (current_val + if dy > 0.0 { -1 } else { 1 }).clamp(0, max_idx);
            if new_val != current_val {
                state_c.borrow_mut().val = new_val;
                drawing_c.queue_draw();
                (state_c.borrow().on_change)(param_id, new_val, false);
            }
            glib::Propagation::Stop
        });
        self.drawing.add_controller(scroll);
    }

    /// Wires double-click or middle-click (reset to default) and right-click (randomize) on the shape.
    fn wire_click(&self) {
        let state_c = Rc::clone(&self.state);
        let drawing_c = self.drawing.clone();
        let click = GestureClick::new();
        click.set_button(0);
        click.connect_pressed(move |gesture, num_presses, _x, _y| {
            drawing_c.grab_focus();
            let btn = gesture.current_button();
            let (default_val, max_idx, param_id, current_val) = {
                let state_ref = state_c.borrow();
                (state_ref.default_val, state_ref.max_idx, state_ref.param_id, state_ref.val)
            };

            if (btn == 1 && num_presses == 2) || btn == 2 {
                if default_val != current_val {
                    state_c.borrow_mut().val = default_val;
                    drawing_c.queue_draw();
                    (state_c.borrow().on_change)(param_id, default_val, false);
                }
            } else if btn == 3 {
                let rand_val = pseudo_rand(max_idx as u32) as i32;
                state_c.borrow_mut().val = rand_val;
                drawing_c.queue_draw();
                (state_c.borrow().on_change)(param_id, rand_val, true);
            }
        });

        self.drawing.add_controller(click);
    }

    /// Sets the value of the shape parameter, optionally triggering the change callback.
    pub(crate) fn set_value(&self, val: i32, should_trigger_cb: bool) {
        let (max_idx, current_val, param_id) = {
            let state_ref = self.state.borrow();
            (state_ref.max_idx, state_ref.val, state_ref.param_id)
        };
        let new_val = val.clamp(0, max_idx);
        if new_val != current_val {
            self.state.borrow_mut().val = new_val;
            self.drawing.queue_draw();
            if should_trigger_cb {
                (self.state.borrow().on_change)(param_id, new_val, false);
            }
        }
    }

    /// Returns the root `Box` widget.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Returns the maximum allowable index for this shape parameter.
    pub(crate) fn max_idx(&self) -> i32 {
        self.state.borrow().max_idx
    }

    /// Returns the default value for this shape parameter.
    pub(crate) fn default_val(&self) -> i32 {
        self.state.borrow().default_val
    }
}

/// Draws the black background and border behind the shape preview.
fn draw_shape_box_background(cr: &cairo::Context, width: i32, height: i32) {
    cr.set_source_rgb(0.02, 0.02, 0.02);
    cr.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
    let _ = cr.fill();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.3);
    cr.set_line_width(1.0);
    cr.rectangle(1.0, 1.0, f64::from(width) - 2.0, f64::from(height) - 2.0);
    let _ = cr.stroke();
}

/// Renders the waveform shape.
///
/// "Morph mode" interpolates between two waveforms, "Single mode" draws a single waveform.
fn draw_shape(cr: &cairo::Context, width: i32, height: i32, state_ref: &ShapeState) {
    draw_shape_box_background(cr, width, height);

    let pad = 7.0;
    let w_step = (f64::from(width) - pad * 2.0) / 16.0;
    let h_step = (f64::from(height) - pad * 2.0) / 8.0;
    let tx = |x: f64| pad + x * w_step;
    let ty = |y: f64| pad + y * h_step;

    let options_ref: Option<&[String]> = Some(state_ref.options.as_slice());

    // Morph mode:
    // Draws two waveforms and morphs between two.
    if let Some(ref get_shapes) = state_ref.get_shapes {
        draw_corner_badge_smaller(
            cr,
            f64::from(width),
            f64::from(height),
            Corner::TopRight,
            &state_ref.val.to_string(),
        );
        cr.set_line_width(4.5);
        cr.set_source_rgba(COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B, 1.0);
        cr.set_line_cap(LineCap::Round);
        cr.set_line_join(LineJoin::Round);
        let (shp1_idx, shp2_idx) = get_shapes();
        let vertices1 = get_shape_vertices(shp1_idx, options_ref, &state_ref.shape_prefix);
        let vertices2: Vertices = get_shape_vertices(shp2_idx, options_ref, &state_ref.shape_prefix)
            .into_iter()
            .map(|(x, y)| (x, 8.0 - y))
            .collect();
        let mix = f64::from(state_ref.val) / 127.0;
        cr.new_path();
        // Start at the first vertices' height (the bottom of a left vertical edge), then `line_to` every sample.
        // The x=0 sample jumps to the edge top, so the left edge of step shapes (SQR/RMP) is drawn instead of skipped.
        let start_y = vertices1[0].1 * (1.0 - mix) + vertices2[0].1 * mix;
        cr.move_to(tx(0.0), ty(start_y));

        // Interpolating between two shapes to create a morphed waveform.
        for i in 0..=256 {
            let x = (f64::from(i) / 256.0) * 16.0;
            let y = get_y_at_x(&vertices1, x) * (1.0 - mix) + get_y_at_x(&vertices2, x) * mix;
            cr.line_to(tx(x), ty(y));
        }
        let _ = cr.stroke();
    }
    // Single mode:
    // Draws a single named waveform rather than a morph between two.
    else {
        let shape_name = options_ref
            .and_then(|options| options.get(state_ref.val as usize))
            .map_or("", std::string::String::as_str);
        draw_corner_badge_smaller(cr, f64::from(width), f64::from(height), Corner::TopLeft, shape_name);
        draw_corner_badge_smaller(
            cr,
            f64::from(width),
            f64::from(height),
            Corner::TopRight,
            &state_ref.val.to_string(),
        );
        cr.set_line_width(4.5);
        cr.set_source_rgba(COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B, 1.0);
        cr.set_line_cap(LineCap::Round);
        cr.set_line_join(LineJoin::Round);
        let mut vertices = get_shape_vertices(state_ref.val as usize, options_ref, &state_ref.shape_prefix);
        if state_ref.short_name == "SHP2" {
            vertices = vertices.into_iter().map(|(x, y)| (x, 8.0 - y)).collect();
        }
        cr.new_path();
        if let Some(((first_px, first_py), rest)) = vertices.split_first() {
            cr.move_to(tx(*first_px), ty(*first_py));
            for (px, py) in rest {
                cr.line_to(tx(*px), ty(*py));
            }
        }
        let _ = cr.stroke();
    }
}

// ── PianoKeyboard ──────────────────────────────────────────────────────────────────────────────────────────
// Cairo piano spanning ±8 semitones from a base note.

const PIANO_NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const WHITE_OFFSETS: [i32; 10] = [-8, -6, -4, -3, -1, 1, 3, 4, 6, 8];
const BLACK_OFFSETS: [i32; 7] = [-7, -5, -2, 0, 2, 5, 7];
const BLACK_RIGHT_IDX: [i32; 7] = [1, 2, 4, 5, 6, 8, 9];
const WK_W: i32 = 57;
const WK_H: i32 = 120;
const BK_W: i32 = 36;
const BK_H: i32 = 75;

/// Returns the keyboard key label (A, W, S, etc.) associated with a semitone offset.
fn piano_key_label(offset: i32) -> &'static str {
    match offset {
        -8 => "A",
        -7 => "W",
        -6 => "S",
        -5 => "E",
        -4 => "D",
        -3 => "F",
        -2 => "T",
        -1 => "G",
        0 => "Y",
        1 => "H",
        2 => "U",
        3 => "J",
        4 => "K",
        5 => "O",
        6 => "L",
        7 => "P",
        8 => ";",
        _ => "",
    }
}

/// Returns a note name (like "C3") for a given MIDI note number.
fn piano_note_name(n: i32) -> String {
    format!("{}{}", PIANO_NOTE_NAMES[(n.rem_euclid(12)) as usize], n / 12 - 1)
}

/// Calculates the semitone offset at a given (x, y) coordinate relative to the piano widget.
fn piano_offset_at(x: f64, y: f64) -> Option<i32> {
    // Identify the semitone offset at the given (x, y) mouse coordinate.
    for (i, &offset) in BLACK_OFFSETS.iter().enumerate() {
        let black_key_x = BLACK_RIGHT_IDX[i] * WK_W - BK_W / 2;
        if x >= f64::from(black_key_x) && x < f64::from(black_key_x + BK_W) && y < f64::from(BK_H) {
            return Some(offset);
        }
    }
    let white_key_idx = (x / f64::from(WK_W)).floor() as usize;
    WHITE_OFFSETS.get(white_key_idx).copied()
}

struct PianoState {
    active: std::collections::HashSet<i32>,
    is_mouse_held: bool,
    mouse_offset: Option<i32>,
    base_note: i32,
}

/// Cairo piano keyboard spanning ±8 semitones from a base note.
#[derive(Clone)]
pub(crate) struct PianoKeyboard {
    pub drawing: DrawingArea,
    state: Rc<RefCell<PianoState>>,
    note_pressed_cbs: IntCbList,
    note_released_cbs: IntCbList,
}

impl PianoKeyboard {
    /// Initializes the piano keyboard widget with a base note and sizing.
    pub(crate) fn new(base_note: i32) -> Self {
        let drawing = DrawingArea::new();
        drawing.set_valign(gtk4::Align::Center);
        drawing.set_halign(gtk4::Align::Center);
        drawing.set_size_request(WHITE_OFFSETS.len() as i32 * WK_W, WK_H);
        drawing.set_margin_top(15);
        drawing.set_margin_bottom(10);

        let state = Rc::new(RefCell::new(PianoState {
            active: std::collections::HashSet::new(),
            is_mouse_held: false,
            mouse_offset: None,
            base_note,
        }));

        {
            let state_c = Rc::clone(&state);
            drawing.set_draw_func(move |_, cr, width, height| draw_piano(cr, width, height, &state_c.borrow()));
        }

        let widget = PianoKeyboard {
            drawing,
            state,
            note_pressed_cbs: Rc::new(RefCell::new(vec![])),
            note_released_cbs: Rc::new(RefCell::new(vec![])),
        };
        widget.wire_click();
        widget.wire_motion();
        widget
    }

    /// Fires all note-pressed callbacks with the given semitone offset.
    fn fire_pressed(&self, offset: i32) {
        for cb in self.note_pressed_cbs.borrow().iter() {
            cb(offset);
        }
    }
    /// Fires all note-released callbacks with the given semitone offset.
    fn fire_released(&self, offset: i32) {
        for cb in self.note_released_cbs.borrow().iter() {
            cb(offset);
        }
    }

    /// Wires mouse press/release to fire note-on and note-off events for the key under the cursor.
    fn wire_click(&self) {
        let state_c = Rc::clone(&self.state);
        let drawing_c = self.drawing.clone();
        let keyboard_c = self.clone();
        let click = GestureClick::new();
        click.connect_pressed(move |_, _n, x, y| {
            if let Some(offset) = piano_offset_at(x, y) {
                state_c.borrow_mut().is_mouse_held = true;
                state_c.borrow_mut().mouse_offset = Some(offset);
                keyboard_c.fire_pressed(offset);
                drawing_c.queue_draw();
            }
        });

        let state_c = Rc::clone(&self.state);
        let drawing_c = self.drawing.clone();
        let keyboard_c = self.clone();
        click.connect_released(move |_, _, _, _| {
            let offset = state_c.borrow().mouse_offset;
            if state_c.borrow().is_mouse_held {
                state_c.borrow_mut().is_mouse_held = false;
                if let Some(offset) = offset {
                    keyboard_c.fire_released(offset);
                    state_c.borrow_mut().mouse_offset = None;
                }
                drawing_c.queue_draw();
            }
        });

        self.drawing.add_controller(click);
    }

    /// Wires mouse motion so that dragging across keys fires note-off on the previous key and note-on on the new one.
    fn wire_motion(&self) {
        let state_c = Rc::clone(&self.state);
        let drawing_c = self.drawing.clone();
        let keyboard_c = self.clone();
        let motion = EventControllerMotion::new();
        motion.connect_motion(move |_, x, y| {
            if !state_c.borrow().is_mouse_held {
                return;
            }
            let offset = piano_offset_at(x, y);
            let prev = state_c.borrow().mouse_offset;
            if offset == prev {
                return;
            }
            if let Some(prev) = prev {
                keyboard_c.fire_released(prev);
            }
            state_c.borrow_mut().mouse_offset = offset;
            if let Some(offset) = offset {
                keyboard_c.fire_pressed(offset);
            }
            drawing_c.queue_draw();
        });
        self.drawing.add_controller(motion);
    }

    /// Marks a key by semitone offset as held or released.
    ///
    /// Used by the host to reflect incoming MIDI note-on/off visually.
    pub(crate) fn set_key_active(&self, offset: i32, is_active: bool) {
        if is_active {
            self.state.borrow_mut().active.insert(offset);
        } else {
            self.state.borrow_mut().active.remove(&offset);
        }
        self.drawing.queue_draw();
    }

    /// Returns the MIDI note number that the center key (offset 0) represents.
    pub(crate) fn base_note(&self) -> i32 {
        self.state.borrow().base_note
    }

    /// Sets the base MIDI note, shifting all key labels and note names accordingly.
    pub(crate) fn set_base_note(&self, note: i32) {
        self.state.borrow_mut().base_note = note;
        self.drawing.queue_draw();
    }

    /// Registers a callback fired with the semitone offset whenever a key is pressed.
    pub(crate) fn connect_note_pressed(&self, f: impl Fn(i32) + 'static) {
        self.note_pressed_cbs.borrow_mut().push(Box::new(f));
    }
    /// Registers a callback fired with the semitone offset whenever a key is released.
    pub(crate) fn connect_note_released(&self, f: impl Fn(i32) + 'static) {
        self.note_released_cbs.borrow_mut().push(Box::new(f));
    }

    /// Returns the underlying `DrawingArea` widget.
    pub(crate) fn widget(&self) -> &DrawingArea {
        &self.drawing
    }
}

/// Renders the white and black piano keys, highlighting active notes and labels.
fn draw_piano(cr: &cairo::Context, width: i32, height: i32, state_ref: &PianoState) {
    let width_f64 = f64::from(width);
    let height_f64 = f64::from(height);

    // Apply GTK rounded corner cutting to the entire widget.
    let _ = cr.save();
    build_rounded_rect_path(cr, 0.5, 0.5, width_f64 - 1.0, height_f64 - 1.0, UI_CORNER_RADIUS);
    cr.clip();

    // Render the white and black piano keys based on their semitone offsets.
    for (i, &offset) in WHITE_OFFSETS.iter().enumerate() {
        let x = i as i32 * WK_W;
        let active = state_ref.active.contains(&offset);
        cr.set_source_rgb(0.97, 0.97, 0.97);
        cr.rectangle(f64::from(x + 1), 1.0, f64::from(WK_W - 2), f64::from(WK_H - 2));

        // Set the color to (`COLOR_BLUE`) for when the user presses a white piano key.
        if active {
            let _ = cr.fill_preserve();
            cr.set_source_rgba(COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B, 0.7);
        }
        let _ = cr.fill();

        cr.set_source_rgba(0.0, 0.0, 0.0, 0.28);
        cr.set_line_width(1.0);
        cr.rectangle(f64::from(x) + 0.5, 0.5, f64::from(WK_W - 1), f64::from(WK_H - 1));
        let _ = cr.stroke();
        let label = piano_key_label(offset);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.40);
        cr.set_font_size(15.0);
        if let Ok(ext) = cr.text_extents(label) {
            cr.move_to(
                f64::from(x) + f64::from(WK_W) / 2.0 - ext.x_bearing() - ext.width() / 2.0,
                f64::from(WK_H - 33),
            );
            let _ = cr.show_text(label);
        }
        let note_name = piano_note_name(state_ref.base_note + offset);
        cr.set_source_rgba(0.0, 0.0, 0.0, if active { 0.55 } else { 0.32 });
        cr.set_font_size(12.0);
        if let Ok(note_ext) = cr.text_extents(&note_name) {
            cr.move_to(
                f64::from(x) + f64::from(WK_W) / 2.0 - note_ext.x_bearing() - note_ext.width() / 2.0,
                f64::from(WK_H - 12),
            );
            let _ = cr.show_text(&note_name);
        }
    }
    for (i, &offset) in BLACK_OFFSETS.iter().enumerate() {
        let black_key_x = BLACK_RIGHT_IDX[i] * WK_W - BK_W / 2;
        let active = state_ref.active.contains(&offset);
        cr.set_source_rgb(0.12, 0.12, 0.12);
        cr.rectangle(f64::from(black_key_x), 0.0, f64::from(BK_W), f64::from(BK_H));

        // Set the color to (`COLOR_BLUE`) for when the user presses a black piano key.
        if active {
            let _ = cr.fill_preserve();
            cr.set_source_rgba(COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B, 0.7);
        }
        let _ = cr.fill();

        cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
        cr.rectangle(f64::from(black_key_x + 3), 1.0, f64::from(BK_W - 6), 15.0);
        let _ = cr.fill();
        let label = piano_key_label(offset);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.55);
        cr.set_font_size(14.0);
        if let Ok(ext) = cr.text_extents(label) {
            cr.move_to(
                f64::from(black_key_x) + f64::from(BK_W) / 2.0 - ext.x_bearing() - ext.width() / 2.0,
                f64::from(BK_H - 27),
            );
            let _ = cr.show_text(label);
        }
        let note_name = piano_note_name(state_ref.base_note + offset);
        cr.set_source_rgba(1.0, 1.0, 1.0, if active { 0.80 } else { 0.45 });
        cr.set_font_size(11.0);
        if let Ok(note_ext) = cr.text_extents(&note_name) {
            cr.move_to(
                f64::from(black_key_x) + f64::from(BK_W) / 2.0 - note_ext.x_bearing() - note_ext.width() / 2.0,
                f64::from(BK_H - 8),
            );
            let _ = cr.show_text(&note_name);
        }
    }
    let _ = cr.restore();

    // Draw the rounded dark border around the entire piano widget.
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.28);
    cr.set_line_width(1.0);
    build_rounded_rect_path(cr, 0.5, 0.5, width_f64 - 1.0, height_f64 - 1.0, UI_CORNER_RADIUS);
    let _ = cr.stroke();
}

// ── Joystick ───────────────────────────────────────────────────────────────────────────────────────────────
// 2D spring-return joystick emitting normalized (x, y).

/// Each call returns how many seconds elapsed since the previous call, then resets so the next call measures from right now.
fn frame_delta_secs(now_us: i64, last_frame_us: &mut Option<i64>) -> f64 {
    let delta_time = last_frame_us.map_or(0.0, |last| ((now_us.saturating_sub(last)) as f64) / 1_000_000.0);
    *last_frame_us = Some(now_us);
    delta_time
}

/// Radius of the joystick's travel area, and the divisor that normalizes a deflection to [-1.0, 1.0].
pub(crate) const JOYSTICK_BASE_RADIUS: f64 = 45.0;
const JOYSTICK_KNOB_RADIUS: f64 = 17.0;
const JOYSTICK_STIFFNESS: f64 = 400.0;
const JOYSTICK_DAMPING: f64 = 25.0;

struct JoystickState {
    knob_x: f64,
    knob_y: f64,
    velocity_x: f64,
    velocity_y: f64,
    is_dragging: bool,
    tick_id: Option<gtk4::TickCallbackId>,
    last_frame_us: Option<i64>,
    moved_cbs: Vec<Box<dyn Fn(f64, f64)>>,
}

/// A 2D spring-return joystick.
///
/// Emits 'moved' with normalized (x, y) in [-1.0, 1.0].
#[derive(Clone)]
pub(crate) struct Joystick {
    pub drawing: DrawingArea,
    state: Rc<RefCell<JoystickState>>,
}

impl Joystick {
    /// Initializes the joystick with physics state, gesture controllers, and draw function.
    pub(crate) fn new() -> Self {
        let size = (JOYSTICK_BASE_RADIUS + JOYSTICK_KNOB_RADIUS) as i32 * 2;
        let drawing = DrawingArea::new();
        drawing.set_size_request(size, size);
        drawing.set_margin_top(15);
        drawing.set_margin_bottom(10);

        let state = Rc::new(RefCell::new(JoystickState {
            knob_x: 0.0,
            knob_y: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            is_dragging: false,
            tick_id: None,
            last_frame_us: None,
            moved_cbs: vec![],
        }));

        {
            let state_c = Rc::clone(&state);
            drawing.set_draw_func(move |da, cr, width, height| draw_joystick(da, cr, width, height, &state_c.borrow()));
        }

        let widget = Joystick { drawing, state };
        widget.wire_click();
        widget.wire_motion();

        let state_c = Rc::clone(&widget.state);
        widget.drawing.connect_unrealize(move |_| {
            if let Some(id) = state_c.borrow_mut().tick_id.take() {
                id.remove();
            }
        });

        widget
    }

    /// Moves the knob to the given offset from center, clamped to the base radius, and fires moved callbacks.
    pub(crate) fn set_knob(&self, dx: f64, dy: f64) {
        let new_x = dx.clamp(-JOYSTICK_BASE_RADIUS, JOYSTICK_BASE_RADIUS);
        let new_y = dy.clamp(-JOYSTICK_BASE_RADIUS, JOYSTICK_BASE_RADIUS);
        self.state.borrow_mut().knob_x = new_x;
        self.state.borrow_mut().knob_y = new_y;
        self.drawing.queue_draw();
        let (normalized_x, normalized_y) = (new_x / JOYSTICK_BASE_RADIUS, new_y / JOYSTICK_BASE_RADIUS);
        for cb in &self.state.borrow().moved_cbs {
            cb(normalized_x, normalized_y);
        }
    }

    /// Stops any running spring-return physics animation.
    pub(crate) fn stop_physics(&self) {
        if let Some(id) = self.state.borrow_mut().tick_id.take() {
            id.remove();
        }
    }

    /// Starts the spring-return physics simulation, driven by the widget's own frame clock rather than a fixed timer.
    ///
    /// `delta_time` is computed from real consecutive frame timestamps.
    /// That keeps the spring-back speed the same regardless of the display's actual refresh rate.
    pub(crate) fn start_physics(&self) {
        if self.state.borrow().tick_id.is_some() {
            return;
        }

        let state_c = Rc::clone(&self.state);
        let id = self.drawing.add_tick_callback(move |da, frame_clock| {
            let now_us = frame_clock.frame_time();
            let delta_time = frame_delta_secs(now_us, &mut state_c.borrow_mut().last_frame_us);
            let (normalized_x, normalized_y) = {
                let mut state_mut = state_c.borrow_mut();
                // Spring force toward origin, minus damping on velocity
                let force_x = -JOYSTICK_STIFFNESS * state_mut.knob_x - JOYSTICK_DAMPING * state_mut.velocity_x;
                let force_y = -JOYSTICK_STIFFNESS * state_mut.knob_y - JOYSTICK_DAMPING * state_mut.velocity_y;
                state_mut.velocity_x += force_x * delta_time;
                state_mut.knob_x += state_mut.velocity_x * delta_time;
                state_mut.velocity_y += force_y * delta_time;
                state_mut.knob_y += state_mut.velocity_y * delta_time;
                (state_mut.knob_x / JOYSTICK_BASE_RADIUS, state_mut.knob_y / JOYSTICK_BASE_RADIUS)
            };
            for cb in &state_c.borrow().moved_cbs {
                cb(normalized_x, normalized_y);
            }
            da.queue_draw();

            // Stop when both axes are effectively at rest.
            let (knob_x, knob_y, velocity_x, velocity_y) = {
                let state_ref = state_c.borrow();
                (state_ref.knob_x, state_ref.knob_y, state_ref.velocity_x, state_ref.velocity_y)
            };
            if knob_x.abs() < 0.4 && knob_y.abs() < 0.4 && velocity_x.abs() < 0.8 && velocity_y.abs() < 0.8 {
                {
                    let mut state_mut = state_c.borrow_mut();
                    state_mut.knob_x = 0.0;
                    state_mut.knob_y = 0.0;
                    state_mut.velocity_x = 0.0;
                    state_mut.velocity_y = 0.0;
                    state_mut.tick_id = None;
                    state_mut.last_frame_us = None;
                }
                for cb in &state_c.borrow().moved_cbs {
                    cb(0.0, 0.0);
                }
                da.queue_draw();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        self.state.borrow_mut().tick_id = Some(id);
    }

    /// Wires a mouse press inside the base to grab the knob, and its release to hand control back to the spring-return physics.
    fn wire_click(&self) {
        let state_c = Rc::clone(&self.state);
        let joystick_c = self.clone();
        let click = GestureClick::new();
        click.connect_pressed(move |_, _n, x, y| {
            let (canvas_width, canvas_height) = (f64::from(joystick_c.drawing.width()), f64::from(joystick_c.drawing.height()));
            let cx = canvas_width / 2.0;
            let cy = canvas_height / 2.0;
            if f64::hypot(x - cx, y - cy) <= JOYSTICK_BASE_RADIUS + JOYSTICK_KNOB_RADIUS {
                {
                    let mut state_mut = state_c.borrow_mut();
                    state_mut.is_dragging = true;
                    state_mut.velocity_x = 0.0;
                    state_mut.velocity_y = 0.0;
                }
                joystick_c.stop_physics();
                joystick_c.set_knob(x - cx, y - cy);
            }
        });

        let state_c = Rc::clone(&self.state);
        let joystick_c = self.clone();
        click.connect_released(move |_, _n, _x, _y| {
            if state_c.borrow().is_dragging {
                state_c.borrow_mut().is_dragging = false;
                joystick_c.start_physics();
            }
        });

        self.drawing.add_controller(click);
    }

    /// Wires mouse motion to move the knob, but only while a drag is already in progress.
    fn wire_motion(&self) {
        let state_c = Rc::clone(&self.state);
        let joystick_c = self.clone();
        let motion = EventControllerMotion::new();
        motion.connect_motion(move |_, x, y| {
            if !state_c.borrow().is_dragging {
                return;
            }
            let (canvas_width, canvas_height) = (f64::from(joystick_c.drawing.width()), f64::from(joystick_c.drawing.height()));
            joystick_c.set_knob(x - canvas_width / 2.0, y - canvas_height / 2.0);
        });
        self.drawing.add_controller(motion);
    }

    /// Registers a callback to be fired when a joystick movement occurs.
    pub(crate) fn connect_moved(&self, f: impl Fn(f64, f64) + 'static) {
        self.state.borrow_mut().moved_cbs.push(Box::new(f));
    }

    /// Returns the root `DrawingArea` widget.
    pub(crate) fn widget(&self) -> &DrawingArea {
        &self.drawing
    }
}

impl Default for Joystick {
    /// Returns a new instance of `Joystick`.
    fn default() -> Self {
        Self::new()
    }
}

/// Renders the joystick widget: base circle, axis lines, and the handle at its current position.
fn draw_joystick(da: &DrawingArea, cr: &cairo::Context, width: i32, height: i32, state_ref: &JoystickState) {
    let cx = f64::from(width) / 2.0;
    let cy = f64::from(height) / 2.0;
    let (red, green, blue) = COLOR_BLUE;

    // Queries the GTK theme's current text/foreground color.
    // By not drawing a background base, the joystick naturally uses the GTK window's actual background!
    let fg = da.color();

    // Crosshair guides (contrasting overlay adapting to theme)
    cr.set_line_width(0.8);
    cr.set_source_rgba(f64::from(fg.red()), f64::from(fg.green()), f64::from(fg.blue()), 0.1);
    cr.move_to(cx - JOYSTICK_BASE_RADIUS, cy);
    cr.line_to(cx + JOYSTICK_BASE_RADIUS, cy);
    let _ = cr.stroke();
    cr.move_to(cx, cy - JOYSTICK_BASE_RADIUS);
    cr.line_to(cx, cy + JOYSTICK_BASE_RADIUS);
    let _ = cr.stroke();

    // Base ring (contrasting outline adapting to theme)
    cr.set_line_width(1.5);
    cr.set_source_rgba(f64::from(fg.red()), f64::from(fg.green()), f64::from(fg.blue()), 0.3);
    cr.arc(cx, cy, JOYSTICK_BASE_RADIUS, 0.0, 2.0 * PI);
    let _ = cr.stroke();

    // Line from center to knob center
    if state_ref.knob_x != 0.0 || state_ref.knob_y != 0.0 {
        cr.set_line_width(1.0);
        cr.set_source_rgba(red, green, blue, 0.4);
        cr.move_to(cx, cy);
        cr.line_to(cx + state_ref.knob_x, cy + state_ref.knob_y);
        let _ = cr.stroke();
    }

    // Knob body
    let knob_center_x = cx + state_ref.knob_x;
    let knob_center_y = cy + state_ref.knob_y;
    cr.arc(knob_center_x, knob_center_y, JOYSTICK_KNOB_RADIUS, 0.0, 2.0 * PI);

    // Clear the lines underneath so the pure GTK window background shows through.
    cr.set_operator(cairo::Operator::Clear);
    let _ = cr.fill_preserve();
    cr.set_operator(cairo::Operator::Over);

    // GTK4 Adwaita buttons are natively only the foreground color at ~8% opacity.
    cr.set_source_rgba(f64::from(fg.red()), f64::from(fg.green()), f64::from(fg.blue()), 0.08);
    let _ = cr.fill_preserve();

    cr.set_line_width(1.8);
    cr.set_source_rgba(red, green, blue, 1.0);
    let _ = cr.stroke();

    // Center dot on knob
    cr.arc(knob_center_x, knob_center_y, 3.0, 0.0, 2.0 * PI);
    cr.set_source_rgba(red, green, blue, 0.9);
    let _ = cr.fill();
}

// ── RangeSlider ────────────────────────────────────────────────────────────────────────────────────────────
// Custom dual-thumb slider for selecting frames/ranges.

const RANGE_THUMB_RADIUS: f64 = 10.0;
const RANGE_TRACK_H: f64 = 5.0;
const RANGE_PAD_X: f64 = 12.0;

struct RangeSliderState {
    pos: f64,
    start: f64,
    end: f64,
    is_multi: bool,
    accent_color: Option<(f64, f64, f64)>,
    step: f64,
    drag_thumb: Option<String>,
    drag_start_val: f64,
    on_pos_changed: Option<Rc<dyn Fn(f64)>>,
    on_range_changed: Option<Rc<dyn Fn(f64, f64)>>,
}

impl Default for RangeSliderState {
    /// Returns the initial state: single-thumb mode, thumb at 0.5, full range [0,1].
    fn default() -> Self {
        RangeSliderState {
            pos: 0.5,
            start: 0.0,
            end: 1.0,
            is_multi: false,
            accent_color: None,
            step: 0.0,
            drag_thumb: None,
            drag_start_val: 0.0,
            on_pos_changed: None,
            on_range_changed: None,
        }
    }
}

/// Custom slider supporting single-thumb (single mode) and dual-thumb (multi mode).
#[derive(Clone)]
pub(crate) struct RangeSlider {
    pub drawing: DrawingArea,
    state: Rc<RefCell<RangeSliderState>>,
}

impl RangeSlider {
    /// Creates a new instance of `RangeSlider`.
    pub(crate) fn new() -> Self {
        let drawing = DrawingArea::new();
        drawing.set_hexpand(true);
        drawing.set_size_request(-1, 28);
        let state = Rc::new(RefCell::new(RangeSliderState::default()));
        {
            let state_c = Rc::clone(&state);
            drawing.set_draw_func(move |_, cr, width, height| draw_range_slider(cr, width, height, &state_c.borrow()));
        }
        let widget = RangeSlider { drawing, state };
        widget.wire_drag();
        widget
    }

    /// Sets the snapping step for the slider (0.0 for smooth).
    pub(crate) fn set_step(&self, step: f64) {
        self.state.borrow_mut().step = step;
    }

    /// Overrides the fill/thumb color, in either mode.
    ///
    /// Pass the RGB components in 0.0..1.0.
    pub(crate) fn set_accent_color(&self, red: f64, green: f64, blue: f64) {
        self.state.borrow_mut().accent_color = Some((red, green, blue));
        self.drawing.queue_draw();
    }

    /// Sets the current thumb position (single-mode only).
    pub(crate) fn set_pos(&self, val: f64) {
        self.state.borrow_mut().pos = val.clamp(0.0, 1.0);
        self.drawing.queue_draw();
    }

    /// Sets the active thumb positions [start, end] (multi-mode only).
    pub(crate) fn set_thumbs(&self, start: f64, end: f64) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.start = start.clamp(0.0, 1.0);
        state_mut.end = end.clamp(0.0, 1.0);
        drop(state_mut);
        self.drawing.queue_draw();
    }

    /// Toggles between single-thumb and dual-thumb (range) modes.
    pub(crate) fn set_multi(&self, is_multi: bool) {
        self.state.borrow_mut().is_multi = is_multi;
        self.drawing.queue_draw();
    }

    /// Returns the current thumb position.
    pub(crate) fn pos(&self) -> f64 {
        self.state.borrow().pos
    }
    /// Returns the current range start.
    pub(crate) fn start(&self) -> f64 {
        self.state.borrow().start
    }
    /// Returns the current range end.
    pub(crate) fn end(&self) -> f64 {
        self.state.borrow().end
    }

    /// Connects a callback for position changes.
    pub(crate) fn connect_pos_changed(&self, f: impl Fn(f64) + 'static) {
        self.state.borrow_mut().on_pos_changed = Some(Rc::new(f));
    }

    /// Connects a callback for range changes.
    pub(crate) fn connect_range_changed(&self, f: impl Fn(f64, f64) + 'static) {
        self.state.borrow_mut().on_range_changed = Some(Rc::new(f));
    }

    /// Returns the root `DrawingArea` widget.
    pub(crate) fn widget(&self) -> &DrawingArea {
        &self.drawing
    }

    /// Returns the track bounds in pixel coordinates.
    fn track_bounds_px(width: i32) -> (f64, f64) {
        (RANGE_PAD_X, (f64::from(width) - RANGE_PAD_X).max(RANGE_PAD_X + 1.0))
    }

    /// Converts a normalized value [0,1] to pixel X.
    fn to_px(val: f64, width: i32) -> f64 {
        let (x0, x1) = Self::track_bounds_px(width);
        x0 + val * (x1 - x0)
    }

    /// Converts a pixel X to a normalized value [0,1].
    fn from_px(px: f64, width: i32) -> f64 {
        let (x0, x1) = Self::track_bounds_px(width);
        ((px - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0)
    }

    /// Wires the drag gesture to move the slider thumbs.
    fn wire_drag(&self) {
        let state_c_begin = Rc::clone(&self.state);
        let drawing_c_begin = self.drawing.clone();
        let state_c_update = Rc::clone(&self.state);
        let drawing_c_update = self.drawing.clone();
        let state_c_end = Rc::clone(&self.state);
        let drag = GestureDrag::new();

        drag.connect_drag_begin(move |_, x, _y| {
            let width = drawing_c_begin.width();
            let state_ref = state_c_begin.borrow();

            let nearest = if state_ref.is_multi {
                let drag_pos = Self::from_px(x, width);
                let dist_start = (drag_pos - state_ref.start).abs();
                let dist_end = (drag_pos - state_ref.end).abs();
                if dist_start < dist_end {
                    "start"
                } else if dist_end < dist_start {
                    "end"
                } else if drag_pos >= 0.5 {
                    "start"
                } else {
                    "end"
                }
            } else {
                "pos"
            };

            let init_val = match nearest {
                "start" => state_ref.start,
                "end" => state_ref.end,
                _ => state_ref.pos, // "pos": single (non-multi) mode
            };
            drop(state_ref);
            state_c_begin.borrow_mut().drag_thumb = Some(nearest.to_string());
            state_c_begin.borrow_mut().drag_start_val = init_val;
        });

        drag.connect_drag_update(move |_, offset_x, _| {
            let width = drawing_c_update.width();
            if width < 1 {
                return;
            }
            let (x0, x1) = Self::track_bounds_px(width);
            let track_w = (x1 - x0).max(1.0);
            let mut state_mut = state_c_update.borrow_mut();
            let raw = state_mut.drag_start_val + offset_x / track_w;
            let step = state_mut.step;
            let snapped = if step > 0.0 { (raw / step).round() * step } else { raw }.clamp(0.0, 1.0);
            match state_mut.drag_thumb.as_deref() {
                Some("pos") => {
                    state_mut.pos = snapped;
                }
                Some("start") => {
                    state_mut.start = snapped.min(state_mut.end);
                }
                Some("end") => {
                    state_mut.end = snapped.max(state_mut.start);
                }
                _ => {}
            }
            let (multi, pos, start, end) = (state_mut.is_multi, state_mut.pos, state_mut.start, state_mut.end);
            let pos_cb = if multi { None } else { state_mut.on_pos_changed.clone() };
            let range_cb = if multi { state_mut.on_range_changed.clone() } else { None };
            drop(state_mut);
            if let Some(cb) = pos_cb {
                cb(pos);
            }
            if let Some(cb) = range_cb {
                cb(start, end);
            }
            drawing_c_update.queue_draw();
        });

        drag.connect_drag_end(move |_, _, _| {
            state_c_end.borrow_mut().drag_thumb = None;
        });
        self.drawing.add_controller(drag);
    }
}

impl Default for RangeSlider {
    /// Returns a new instance of `RangeSlider` with default settings.
    fn default() -> Self {
        Self::new()
    }
}

/// Renders the range slider track and thumbs.
fn draw_range_slider(cr: &cairo::Context, width: i32, height: i32, state_ref: &RangeSliderState) {
    let (x0, x1) = RangeSlider::track_bounds_px(width);
    let track_y = f64::from(height) / 2.0 - RANGE_TRACK_H / 2.0;
    cr.set_source_rgba(0.3, 0.3, 0.3, 1.0);
    fill_rounded_rect(cr, x0, track_y, x1 - x0, RANGE_TRACK_H, RANGE_TRACK_H / 2.0);

    // If the slider is a range of many, then do this:
    if state_ref.is_multi {
        let (red, green, blue) = state_ref.accent_color.unwrap_or((COLOR_GREEN_R, COLOR_GREEN_G, COLOR_GREEN_B));
        let start_pixel = RangeSlider::to_px(state_ref.start, width);
        let end_pixel = RangeSlider::to_px(state_ref.end, width);
        cr.set_source_rgba(red, green, blue, 0.35);
        fill_rounded_rect(
            cr,
            start_pixel,
            track_y,
            (end_pixel - start_pixel).max(0.0),
            RANGE_TRACK_H,
            RANGE_TRACK_H / 2.0,
        );
        for &thumb_pixel in &[start_pixel, end_pixel] {
            cr.set_source_rgba(red, green, blue, 1.0);
            cr.arc(thumb_pixel, f64::from(height) / 2.0, RANGE_THUMB_RADIUS, 0.0, 2.0 * PI);
            let _ = cr.fill();
        }
    }
    // If the slider is a range of one, then do this:
    else {
        let (red, green, blue) = state_ref.accent_color.unwrap_or((COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B));
        let thumb_pixel = RangeSlider::to_px(state_ref.pos, width);
        cr.set_source_rgba(red, green, blue, 0.35);
        fill_rounded_rect(cr, x0, track_y, (thumb_pixel - x0).max(0.0), RANGE_TRACK_H, RANGE_TRACK_H / 2.0);
        cr.set_source_rgba(red, green, blue, 1.0);
        cr.arc(thumb_pixel, f64::from(height) / 2.0, RANGE_THUMB_RADIUS, 0.0, 2.0 * PI);
        let _ = cr.fill();
    }
}

/// Draws the slider track segments.
fn fill_rounded_rect(cr: &cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    build_rounded_rect_path(cr, x, y, width, height, radius);
    let _ = cr.fill();
}
