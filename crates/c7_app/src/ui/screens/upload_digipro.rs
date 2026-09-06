//! Send DigiPro wavetables to Monomachine slots.
//!
//! Load → visualize the frame and 3D wavetable → Send to Device or Save to File.

/*
Structurally identical to `upload_sample.rs`, but with different section content objectives.
If you change ANY structural code in here, please change `upload_sample.rs` to use the new standard too.
*/

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use cairo::LineJoin;
use gtk4::prelude::*;
use gtk4::{
    self, Align, Box as GtkBox, Button, CheckButton, DrawingArea, Entry, EventControllerScroll, EventControllerScrollFlags, FileFilter,
    FlowBox, GestureClick, GestureDrag, Orientation, SelectionMode, ToggleButton, gio, glib,
};

use crate::ui::base_module::{BaseModule, ProgressFn, StatusFn, eta_string};
use crate::ui::drawing::{
    COLOR_BLUE_B, COLOR_BLUE_G, COLOR_BLUE_R, COLOR_GREEN_B, COLOR_GREEN_G, COLOR_GREEN_R, COLOR_PURPLE_B, COLOR_PURPLE_G, COLOR_PURPLE_R,
    Corner, clip_rounded, draw_corner_badge,
};
use crate::ui::file_checks::show_mismatch_dialog;
use crate::ui::mixins::upload_common::{name_or_default, wire_file_drop_target};
use crate::ui::modals::audio_import::show_trim_dialog;
use crate::ui::widgets::{ChooseOnePill, CustomTextbox, NumberSpinner, RangeSlider};
use c7_core::c7_file_interfacing::{read_c7_file, write_digipro_c7, write_digipro_wavetable_c7};
use c7_core::device_config::{DeviceConfig, get_base_channel, get_export_folder};
use c7_core::digipro::{contains_digipro, i16_to_digipro_sysex};
use c7_core::dsp_utils::detect_silence;
use c7_core::midi::{is_valid_port, run_midi_output};
use c7_core::utils::{AUDIO_EXTS, IMAGE_EXTS, JsonPath, SYSEX_EXTS, as_u64_or_die};
use c7_core::wavetable_presets::{DEFAULT_PRESET, PRESETS};
use c7_core::wavetable_utils::{FrameMode, center_table, load_wavetable_file, normalize_table, sysex_to_digipro_wavetable};

// ── Display decimation ─────────────────────────────────────────────────────────────────────────────────────

/// Samples per frame used for on-screen rendering.
///
/// The device payload stays at each frame's full resolution.
/// The views draw this many points instead, which already exceeds the pixel width a single frame ever occupies on screen.
/// Every view decimates through the same helper, so they always agree.
const DISPLAY_WAVE_LEN: usize = 256;

/// Downsamples one full-resolution frame to `DISPLAY_WAVE_LEN` evenly spaced points for display.
///
/// The first and last samples are preserved so the drawn curve spans the frame edge-to-edge.
/// This is render-only.
/// The full-resolution frame in `UploadDigiproState`, which is what gets sent to and saved for the device, is never modified.
fn decimate_frame(wave: &[i16]) -> Vec<i16> {
    if wave.len() <= DISPLAY_WAVE_LEN {
        return wave.to_vec();
    }
    let (last_src, last_dst) = (wave.len() - 1, DISPLAY_WAVE_LEN - 1);
    // Integer index map: output j maps to source j*`last_src`/`last_dst`, so j=0 -> 0 and j=last -> last.
    (0..DISPLAY_WAVE_LEN).map(|j| wave[j * last_src / last_dst]).collect()
}

/// Drawing state for the left panel.
///
/// Holds one wave normally, and the whole table in Multi mode.
struct WaveformViewState {
    wave: Vec<i16>,
    waves: Vec<Vec<i16>>,
    is_multi_mode: bool,
    line_width: f64,
}

impl Default for WaveformViewState {
    /// Returns the default state for the waveform view.
    fn default() -> Self {
        WaveformViewState {
            wave: vec![0; DISPLAY_WAVE_LEN],
            waves: Vec::new(),
            is_multi_mode: false,
            line_width: 3.7,
        }
    }
}

/// Display of the currently selected waveform frame.
///
/// Single mode: blue line for the one frame at the slider position.
/// Multi mode: onion-skin stack in green, first wave solid, following waves fading linearly to `MIN_ALPHA` at the back of the selection.
#[derive(Clone)]
struct WaveformView {
    drawing: DrawingArea,
    state: Rc<RefCell<WaveformViewState>>,
}

impl WaveformView {
    /// Creates a new waveform view.
    pub(crate) fn new() -> Self {
        let drawing = DrawingArea::new();
        drawing.set_hexpand(true);
        let state = Rc::new(RefCell::new(WaveformViewState::default()));
        {
            let state_c = Rc::clone(&state);
            drawing.set_draw_func(move |_, cr, w, h| draw_waveform(cr, w, h, &state_c.borrow()));
        }
        WaveformView { drawing, state }
    }

    /// Updates the display to show the given waveform frame in single mode.
    fn set_wave(&self, wave: &[i16]) {
        self.state.borrow_mut().wave = decimate_frame(wave);
        self.drawing.queue_draw();
    }

    /// Toggles whether to draw the single wave or the multi-frame onion skin.
    fn set_multi_mode(&self, is_enabled: bool) {
        self.state.borrow_mut().is_multi_mode = is_enabled;
        self.drawing.queue_draw();
    }

    /// Provides the list of frames for the onion-skin multi-mode view.
    fn set_waves(&self, waves: &[Vec<i16>]) {
        let waves: Vec<Vec<i16>> = waves.iter().map(|w| decimate_frame(w)).collect();
        let mut state_mut = self.state.borrow_mut();
        if let Some(first) = waves.first() {
            state_mut.wave.clone_from(first);
        }
        state_mut.waves = waves;
        drop(state_mut);
        self.drawing.queue_draw();
    }

    /// Returns the drawing area widget.
    pub(crate) fn widget(&self) -> &DrawingArea {
        &self.drawing
    }
}

impl Default for WaveformView {
    /// Returns the default waveform view.
    fn default() -> Self {
        Self::new()
    }
}

/// Draws a single waveform frame by interpolating linearly between its samples.
fn draw_wave_line(
    cr: &cairo::Context,
    width: i32,
    height: i32,
    data: &[i16],
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
    line_width: f64,
) {
    cr.set_source_rgba(red, green, blue, alpha);
    cr.set_line_width(line_width);
    cr.set_line_join(LineJoin::Round);
    cr.new_path();
    // Map the frame's samples to the horizontal pixel grid using linear interpolation.
    for px in 0..width {
        let sample_pos = (f64::from(px) / f64::from((width - 1).max(1))) * (data.len() - 1) as f64;
        let idx_lower = sample_pos as usize;
        let idx_upper = (idx_lower + 1).min(data.len() - 1);
        let fraction = sample_pos - idx_lower as f64;
        let interp_val = f64::from(data[idx_lower]) * (1.0 - fraction) + f64::from(data[idx_upper]) * fraction;
        let y = f64::from(height) * (0.5 - (interp_val / 32768.0) * 0.5);
        if px == 0 {
            cr.move_to(f64::from(px), y);
        } else {
            cr.line_to(f64::from(px), y);
        }
    }
    let _ = cr.stroke();
}

/// Draws the waveform to the given Cairo context.
fn draw_waveform(cr: &cairo::Context, width: i32, height: i32, state_ref: &WaveformViewState) {
    clip_rounded(cr, f64::from(width), f64::from(height));
    cr.set_source_rgb(0.0, 0.0, 0.0);
    let _ = cr.paint();
    if width < 2 || height < 2 {
        return;
    }

    if state_ref.is_multi_mode && !state_ref.waves.is_empty() {
        // Onion-skin: draw back-to-front so `waves[0]` (front of selection) lands on top.
        let num_waves = state_ref.waves.len();
        // Draw back-to-front onion skins for the multi-frame wavetable preview.
        for i in (0..num_waves).rev() {
            // i=0 → alpha 1.0 (solid front);  i=n-1 → alpha 0.0 (fully transparent back).
            let alpha = 1.0 - (i as f64 / (num_waves - 1).max(1) as f64);
            draw_wave_line(
                cr,
                width,
                height,
                &state_ref.waves[i],
                COLOR_GREEN_R,
                COLOR_GREEN_G,
                COLOR_GREEN_B,
                alpha,
                state_ref.line_width,
            );
        }
    } else {
        draw_wave_line(
            cr,
            width,
            height,
            &state_ref.wave,
            COLOR_BLUE_R,
            COLOR_BLUE_G,
            COLOR_BLUE_B,
            1.0,
            state_ref.line_width,
        );
    }
}

// ── Right panel: 3D Multi waveform display ─────────────────────────────────────────────────────────────────

/// Brightness multiplier for the sheet drawn under a single loaded waveform.
///
/// Dims the waveform's own color to about a third, so the outline on top still stands out.
const SURFACE_SHADE: f64 = 0.32;

struct WavetableViewState {
    table: Vec<Vec<i16>>,
    scrub_position: f64,
    is_multi_mode: bool,
    range_start: f64,
    range_end: f64,
    canvas_width: f64,
    canvas_height: f64,
    yaw: f64,
    pitch: f64,
    amplitude: f64,

    alpha_back: f64,
    alpha_front: f64,
    ghost_line_width: f64,
    selected_line_width: f64,
    drag_yaw: f64,
    drag_pitch: f64,
    is_fit_dirty: bool,
    fit_offset_x: f64,
    fit_offset_y: f64,
    fit_scale: f64,
}

impl Default for WavetableViewState {
    /// Returns the default state for the wavetable view.
    fn default() -> Self {
        WavetableViewState {
            table: Vec::new(),
            scrub_position: 0.5,
            is_multi_mode: false,
            range_start: 0.0,
            range_end: 1.0,
            canvas_width: 0.74,
            canvas_height: 0.74,
            yaw: 0.185,
            pitch: 0.419,
            amplitude: 0.370,
            alpha_back: 0.080,
            alpha_front: 0.500,
            ghost_line_width: 2.00,
            selected_line_width: 3.70,
            drag_yaw: 0.0,
            drag_pitch: 0.0,
            is_fit_dirty: true,
            fit_offset_x: 0.0,
            fit_offset_y: 0.0,
            fit_scale: 1.0,
        }
    }
}

/// 3D waterfall display of the full wavetable.
///
/// Single mode: selected frame drawn last (on top) in blue; all others dim purple.
/// Multi mode: selected range drawn last in boosted green; all others dim purple.
#[derive(Clone)]
struct WavetableView {
    drawing: DrawingArea,
    state: Rc<RefCell<WavetableViewState>>,
}

impl WavetableView {
    /// Creates a new wavetable view.
    pub(crate) fn new() -> Self {
        let drawing = DrawingArea::new();
        drawing.set_hexpand(true);
        drawing.set_focusable(true);
        drawing.set_cursor_from_name(Some("grab"));
        let state = Rc::new(RefCell::new(WavetableViewState::default()));
        {
            let state_c = Rc::clone(&state);
            drawing.set_draw_func(move |_, cr, w, h| draw_wavetable(cr, w, h, &mut state_c.borrow_mut()));
        }
        {
            let state_c = Rc::clone(&state);
            drawing.connect_resize(move |_, _, _| {
                state_c.borrow_mut().is_fit_dirty = true;
            });
        }
        let widget = WavetableView { drawing, state };
        widget.wire_yaw_drag();
        widget.wire_pitch_drag();
        widget.wire_middle_click();
        widget.wire_scroll();
        widget
    }

    /// Toggles between single-frame and multi-frame modes.
    fn set_multi_mode(&self, is_enabled: bool) {
        self.state.borrow_mut().is_multi_mode = is_enabled;
        self.drawing.queue_draw();
    }

    /// Sets the start and end positions for the multi-frame range.
    pub(crate) fn set_range(&self, start_pos: f64, end_pos: f64) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.range_start = start_pos.clamp(0.0, 1.0);
        state_mut.range_end = end_pos.clamp(0.0, 1.0);
        drop(state_mut);
        self.drawing.queue_draw();
    }

    /// Sets the wavetable data.
    fn set_table(&self, table: &[Vec<i16>]) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.table = table.iter().map(|w| decimate_frame(w)).collect();
        state_mut.is_fit_dirty = true;
        drop(state_mut);
        self.drawing.queue_draw();
    }

    /// Sets the current position in the wavetable.
    pub(crate) fn set_position(&self, pos: f64) {
        self.state.borrow_mut().scrub_position = pos.clamp(0.0, 1.0);
        self.drawing.queue_draw();
    }

    /// Returns the drawing area widget.
    pub(crate) fn widget(&self) -> &DrawingArea {
        &self.drawing
    }

    /// Wires left-drag to rotate the view horizontally.
    fn wire_yaw_drag(&self) {
        let state_c_begin = Rc::clone(&self.state);
        let drawing_c_begin = self.drawing.clone();
        let state_c_update = Rc::clone(&self.state);
        let drawing_c_update = self.drawing.clone();
        let drawing_c_end = self.drawing.clone();
        let drag = GestureDrag::new();
        drag.set_button(1);

        drag.connect_drag_begin(move |_, _, _| {
            let yaw = state_c_begin.borrow().yaw;
            state_c_begin.borrow_mut().drag_yaw = yaw;
            drawing_c_begin.set_cursor_from_name(Some("grabbing"));
        });
        drag.connect_drag_update(move |_, offset_x, _| {
            let width = drawing_c_update.width();
            if width < 1 {
                return;
            }
            let base = state_c_update.borrow().drag_yaw;
            state_c_update.borrow_mut().yaw = base + offset_x / f64::from(width) * std::f64::consts::PI * 2.0;
            drawing_c_update.queue_draw();
        });
        drag.connect_drag_end(move |_, _, _| {
            drawing_c_end.set_cursor_from_name(Some("grab"));
        });
        self.drawing.add_controller(drag);
    }

    /// Wires right-drag to tilt the view vertically.
    fn wire_pitch_drag(&self) {
        let state_c_begin = Rc::clone(&self.state);
        let drawing_c_begin = self.drawing.clone();
        let state_c_update = Rc::clone(&self.state);
        let drawing_c_update = self.drawing.clone();
        let drawing_c_end = self.drawing.clone();
        let drag = GestureDrag::new();
        drag.set_button(3);

        drag.connect_drag_begin(move |_, _, _| {
            let pitch = state_c_begin.borrow().pitch;
            state_c_begin.borrow_mut().drag_pitch = pitch;
            drawing_c_begin.set_cursor_from_name(Some("grabbing"));
        });
        drag.connect_drag_update(move |_, _, offset_y| {
            let height = drawing_c_update.height();
            if height < 1 {
                return;
            }
            let half_pi = std::f64::consts::FRAC_PI_2;
            let base = state_c_update.borrow().drag_pitch;
            // Right click drag up/down -> change pitch. Negative `offset_y` is dragging up -> tilt up.
            let raw = base - offset_y / f64::from(height) * std::f64::consts::PI;
            state_c_update.borrow_mut().pitch = raw.clamp(-half_pi, half_pi);
            drawing_c_update.queue_draw();
        });
        drag.connect_drag_end(move |_, _, _| {
            drawing_c_end.set_cursor_from_name(Some("grab"));
        });
        self.drawing.add_controller(drag);
    }

    /// Wires middle-click to reset the view to its default yaw, tilt, and zoom.
    fn wire_middle_click(&self) {
        let state = Rc::clone(&self.state);
        let drawing_c = self.drawing.clone();
        let click = GestureClick::new();
        click.set_button(2);
        click.connect_pressed(move |_, _, _, _| {
            let mut state_mut = state.borrow_mut();
            state_mut.canvas_width = 0.74;
            state_mut.canvas_height = 0.74;
            state_mut.yaw = 0.185;
            state_mut.pitch = 0.419;
            state_mut.amplitude = 0.370;
            state_mut.is_fit_dirty = true;
            drop(state_mut);
            drawing_c.queue_draw();
        });
        self.drawing.add_controller(click);
    }

    /// Wires the scroll wheel to zoom the view.
    fn wire_scroll(&self) {
        let state = Rc::clone(&self.state);
        let drawing_c = self.drawing.clone();
        let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |_, _dx, dy| {
            // 5% zoom per scroll tick. dy>0 = scroll down = zoom out.
            let factor = 1.0 - dy * 0.05;
            let mut state_mut = state.borrow_mut();
            state_mut.canvas_width = (state_mut.canvas_width * factor).clamp(0.5, 4.0);
            state_mut.canvas_height = (state_mut.canvas_height * factor).clamp(0.5, 4.0);
            drop(state_mut);
            drawing_c.queue_draw();
            glib::Propagation::Stop
        });
        self.drawing.add_controller(scroll);
    }
}

impl Default for WavetableView {
    /// Returns the default wavetable view.
    fn default() -> Self {
        Self::new()
    }
}

/// Draws the 3D wavetable waterfall to the given Cairo context.
fn draw_wavetable(cr: &cairo::Context, width: i32, height: i32, state_mut: &mut WavetableViewState) {
    clip_rounded(cr, f64::from(width), f64::from(height));
    cr.set_source_rgb(0.0, 0.0, 0.0);
    let _ = cr.paint();
    let table_len = state_mut.table.len();
    if table_len == 0 || width < 2 || height < 2 {
        return;
    }
    let wave_length = state_mut.table[0].len();
    if wave_length == 0 {
        return;
    }

    let selected_frame_idx: i64;
    let range_set: std::collections::HashSet<usize>;

    if state_mut.is_multi_mode {
        let start_idx = (state_mut.range_start * (table_len - 1) as f64).round() as usize;
        let end_idx = (state_mut.range_end * (table_len - 1) as f64).round() as usize;
        range_set = (start_idx..=end_idx).collect();
        selected_frame_idx = -1;
    } else {
        selected_frame_idx = (state_mut.scrub_position * (table_len - 1) as f64).round() as i64;
        range_set = std::collections::HashSet::new();
    }

    let draw_width = f64::from(width) * state_mut.canvas_width;
    let draw_height = f64::from(height) * state_mut.canvas_height;
    let amplitude = draw_height * state_mut.amplitude;
    let cos_yaw = state_mut.yaw.cos();
    let sin_yaw = state_mut.yaw.sin();
    let cos_pitch = state_mut.pitch.cos();
    let sin_pitch = state_mut.pitch.sin();

    // On first draw after a table/resize: sample the bounding box.
    // Then compute centering offsets and a scale-down factor (≤1) to avoid clipping.
    // Held fixed until the next table/resize change, so interactive yaw/pitch/zoom feels natural.
    if state_mut.is_fit_dirty {
        state_mut.is_fit_dirty = false;
        let step = (table_len / 20).max(1);
        let mut raw_xs: Vec<f64> = Vec::new();
        let mut raw_ys: Vec<f64> = Vec::new();
        // A lone frame is drawn as a sheet spanning the full depth, so both of its edges have to be inside the fit box.
        let fit_samples: Vec<(usize, f64)> = if table_len == 1 {
            vec![(0, 0.0), (0, 1.0)]
        } else {
            (0..table_len)
                .step_by(step)
                .map(|frame_idx| (frame_idx, frame_idx as f64 / (table_len - 1) as f64))
                .collect()
        };
        // Sample 3D vertices to compute an optimal fit scale and offset.
        for (frame_idx, normalized_pos) in fit_samples {
            let z_base = (normalized_pos - 0.5) * draw_width;
            let wave = &state_mut.table[frame_idx];
            for sample_idx in (0..wave.len()).step_by(4) {
                let fraction = sample_idx as f64 / (wave.len() - 1) as f64;
                let x_base = (fraction - 0.5) * draw_width;
                let y_base = -(f64::from(wave[sample_idx]) / 32768.0) * amplitude;
                let y_pos = y_base * cos_pitch - z_base * sin_pitch;
                let z_pos = y_base * sin_pitch + z_base * cos_pitch;
                let x_yaw = x_base * cos_yaw + z_pos * sin_yaw;
                raw_xs.push(x_yaw);
                raw_ys.push(y_pos);
            }
        }

        if !raw_ys.is_empty() && !raw_xs.is_empty() {
            let x_lo = raw_xs.iter().copied().fold(f64::INFINITY, f64::min);
            let x_hi = raw_xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let y_lo = raw_ys.iter().copied().fold(f64::INFINITY, f64::min);
            let y_hi = raw_ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let content_w = (x_hi - x_lo).max(1.0);
            let content_h = (y_hi - y_lo).max(1.0);
            let margin_x = 0.04 * f64::from(width);
            let margin_y = 0.04 * f64::from(height);
            let scale_x = (f64::from(width) - 2.0 * margin_x) / content_w;
            let scale_y = (f64::from(height) - 2.0 * margin_y) / content_h;
            state_mut.fit_scale = scale_x.min(scale_y).min(1.0);
            state_mut.fit_offset_x = -f64::midpoint(x_lo, x_hi) * state_mut.fit_scale;
            state_mut.fit_offset_y = -f64::midpoint(y_lo, y_hi) * state_mut.fit_scale;
        } else {
            state_mut.fit_scale = 1.0;
            state_mut.fit_offset_x = 0.0;
            state_mut.fit_offset_y = 0.0;
        }
    }

    let center_x = f64::from(width) / 2.0 + state_mut.fit_offset_x;
    let center_y = f64::from(height) / 2.0 + state_mut.fit_offset_y;
    let fit_scale = state_mut.fit_scale;

    // A lone frame has no neighbors to stack against, so extrude it into one filled sheet instead of leaving a floating line.
    if table_len == 1 {
        let wave = &state_mut.table[0];
        if wave.is_empty() {
            return;
        }

        // Projects the frame at a depth position, `0.0` being the front edge of the table and `1.0` the back.
        let edge_at = |time_fraction: f64| -> Vec<(f64, f64)> {
            let z_base = (time_fraction - 0.5) * draw_width;
            wave.iter()
                .enumerate()
                .map(|(sample_idx, &sample)| {
                    let fraction = sample_idx as f64 / (wave.len() - 1) as f64;
                    let x_base = (fraction - 0.5) * draw_width;
                    let y_base = -(f64::from(sample) / 32768.0) * amplitude;
                    let y_pos = y_base * cos_pitch - z_base * sin_pitch;
                    let z_pos = y_base * sin_pitch + z_base * cos_pitch;
                    let x_yaw = x_base * cos_yaw + z_pos * sin_yaw;
                    (center_x + x_yaw * fit_scale, center_y + y_pos * fit_scale)
                })
                .collect()
        };

        let front = edge_at(0.0);
        let back = edge_at(1.0);

        // A single frame is always the selected one, and `update_multi_sensitivity()` keeps multi mode out of reach here.
        let (red, green, blue) = (COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B);

        // The front and back edges cross at most view angles, so one path closed around both fills as a bowtie.
        // Filling per quad avoids that, and stroking each one keeps neighbors from leaving antialiased seams.
        cr.set_source_rgb(red * SURFACE_SHADE, green * SURFACE_SHADE, blue * SURFACE_SHADE);
        cr.set_line_width(1.0);
        for (quad_idx, (&front_a, &back_a)) in front.iter().zip(&back).enumerate().take(front.len() - 1) {
            cr.new_path();
            cr.move_to(front_a.0, front_a.1);
            cr.line_to(front[quad_idx + 1].0, front[quad_idx + 1].1);
            cr.line_to(back[quad_idx + 1].0, back[quad_idx + 1].1);
            cr.line_to(back_a.0, back_a.1);
            cr.close_path();
            let _ = cr.fill_preserve();
            let _ = cr.stroke();
        }

        // Outline the front edge so the sheet keeps a defined leading contour.
        cr.new_path();
        cr.move_to(front[0].0, front[0].1);
        for &(x, y) in &front[1..] {
            cr.line_to(x, y);
        }
        cr.set_source_rgb(red, green, blue);
        cr.set_line_width(state_mut.selected_line_width);
        let _ = cr.stroke();
        return;
    }

    // Depth sorting (Painter's algorithm): furthest first
    let frame_depth = |frame_idx: usize| -> f64 {
        let time_fraction = if table_len > 1 {
            frame_idx as f64 / (table_len - 1) as f64
        } else {
            0.0
        };
        let z_base = (time_fraction - 0.5) * draw_width;
        z_base * cos_yaw * cos_pitch
    };
    let mut order: Vec<usize> = (0..table_len).collect();
    order.sort_by(|&a, &b| frame_depth(b).partial_cmp(&frame_depth(a)).unwrap_or(std::cmp::Ordering::Equal));

    // Project and draw each wavetable frame in depth-sorted order.
    for frame_idx in order {
        let time_fraction = if table_len > 1 {
            frame_idx as f64 / (table_len - 1) as f64
        } else {
            0.0
        };
        let z_base = (time_fraction - 0.5) * draw_width;
        let view_depth = (time_fraction - 0.5) * cos_yaw * cos_pitch + 0.5;

        // Perceptually linear decay (square root of remaining depth):
        let depth_factor = (1.0 - view_depth).max(0.0).sqrt();

        let (wave_red, wave_green, wave_blue, wave_alpha, wave_line_width) = if state_mut.is_multi_mode {
            if range_set.contains(&frame_idx) {
                let alpha = {
                    let a_val = state_mut.alpha_back + (state_mut.alpha_front - state_mut.alpha_back) * depth_factor;
                    (a_val * 2.5).min(1.0)
                };
                let line_width = (state_mut.ghost_line_width + 0.3) * (0.35 + 0.75 * depth_factor);
                (COLOR_GREEN_R, COLOR_GREEN_G, COLOR_GREEN_B, alpha, line_width)
            } else {
                let alpha = state_mut.alpha_back + (state_mut.alpha_front - state_mut.alpha_back) * depth_factor;
                let line_width = state_mut.ghost_line_width * (0.35 + 0.75 * depth_factor);
                (COLOR_PURPLE_R, COLOR_PURPLE_G, COLOR_PURPLE_B, alpha, line_width)
            }
        } else if frame_idx == selected_frame_idx as usize && selected_frame_idx >= 0 {
            (COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B, 1.0, state_mut.selected_line_width)
        } else {
            let alpha = state_mut.alpha_back + (state_mut.alpha_front - state_mut.alpha_back) * depth_factor;
            let line_width = state_mut.ghost_line_width * (0.35 + 0.75 * depth_factor);
            (COLOR_PURPLE_R, COLOR_PURPLE_G, COLOR_PURPLE_B, alpha, line_width)
        };

        let wave = &state_mut.table[frame_idx];
        if wave.is_empty() {
            continue;
        }

        // Stroke Waveform with depth-cued color and width:
        cr.set_source_rgba(wave_red, wave_green, wave_blue, wave_alpha);
        cr.set_line_width(wave_line_width);
        cr.new_path();
        for (sample_idx, &sample) in wave.iter().enumerate() {
            let fraction = sample_idx as f64 / (wave.len() - 1) as f64;
            let x_base = (fraction - 0.5) * draw_width;
            let y_base = -(f64::from(sample) / 32768.0) * amplitude;
            let y_pos = y_base * cos_pitch - z_base * sin_pitch;
            let z_pos = y_base * sin_pitch + z_base * cos_pitch;
            let x_yaw = x_base * cos_yaw + z_pos * sin_yaw;

            let draw_x = center_x + x_yaw * state_mut.fit_scale;
            let draw_y = center_y + y_pos * state_mut.fit_scale;

            if sample_idx == 0 {
                cr.move_to(draw_x, draw_y);
            } else {
                cr.line_to(draw_x, draw_y);
            }
        }
        let _ = cr.stroke();
    }

    // Frame-count badge (bottom-right corner, only in multi mode).
    if state_mut.is_multi_mode && table_len > 0 {
        let count = ((state_mut.range_end * (table_len - 1) as f64).round() as i64
            - (state_mut.range_start * (table_len - 1) as f64).round() as i64
            + 1)
        .max(1);
        draw_corner_badge(cr, f64::from(width), f64::from(height), Corner::BottomRight, &count.to_string());
    }
}

// ── Internal state ─────────────────────────────────────────────────────────────────────────────────────────

struct UploadDigiproState {
    /// Processed (norm/`dc_check_c` applied), the version that gets sent, saved, and displayed.
    wavetable: Vec<Vec<i16>>,
    /// Pre-norm/`dc_check_c`, the source of truth for reactive toggle re-processing.
    raw_wavetable: Vec<Vec<i16>>,
    is_multi_mode: bool,
    saved_single_pos: f64,
    saved_range: Option<(f64, f64)>,
    dc: Arc<DeviceConfig>,
    get_midi_rc: Rc<dyn Fn() -> Option<String>>,
    /// Last loaded audio file and the trims it was loaded with, so changing the frame mode can re-cut it.
    source: Option<(String, bool, bool)>,
}

/// Feature screen for uploading DigiPro waves to Monomachine slots.
pub(crate) struct UploadDigiproScreen {
    pub root: gtk4::Box,
}

impl UploadDigiproScreen {
    /// Initializes the DigiPro upload screen.
    pub(crate) fn new(
        go_to_menu: impl Fn() + 'static,
        get_selected_midi: impl Fn() -> Option<String> + 'static,
        _delay_spin: &NumberSpinner,
        active_config: &str,
    ) -> Self {
        let mut base = BaseModule::new();

        let dc = Arc::new(DeviceConfig::find_by_path_or_die(active_config));

        let slot_min = as_u64_or_die(dc.json_get("sysex_layout.digipro.fields.slot.min")) as u8;
        let slot_max = as_u64_or_die(dc.json_get("sysex_layout.digipro.fields.slot.max")) as u8;
        let name_size = as_u64_or_die(dc.json_get("sysex_layout.digipro.fields.name.size")) as u16;

        // Samples per waveform frame, from the device JSON.
        let wave_length = as_u64_or_die(dc.json_get("digipro.wave_length")) as usize;

        let wavetable = DEFAULT_PRESET(wave_length);

        // ── Header ─────────────────────────────────────────────────────────────────────────────────────────

        let header = base.build_header("Upload DigiPro", go_to_menu);

        let multi_btn = ToggleButton::with_label("Multi");
        multi_btn.set_tooltip_text(Some("Multi mode: select a range of frames"));
        header.end.append(&multi_btn);

        // ── Two display panels ─────────────────────────────────────────────────────────────────────────────

        let panels = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .hexpand(true)
            .build();
        panels.set_size_request(-1, 200);

        let wave_view = WaveformView::new();
        let table_view = WavetableView::new();
        panels.append(wave_view.widget());
        panels.append(table_view.widget());
        base.root.append(&panels);

        // Panel resize: keep panels at 2:1 aspect per panel.
        {
            wave_view.widget().connect_resize(move |_, width, height| {
                let target_h = (width / 2).max(80);
                if height != target_h {
                    panels.set_size_request(-1, target_h);
                }
            });
        }

        // ── Range slider ───────────────────────────────────────────────────────────────────────────────────

        let range_slider = RangeSlider::new();
        let table_len = wavetable.len();
        let step = 1.0 / (table_len - 1).max(1) as f64;
        range_slider.set_step(step);
        range_slider.set_pos(0.5);
        base.root.append(range_slider.widget());

        // ── Normalize / Center toggles ─────────────────────────────────────────────────────────────────────

        let checks_box = GtkBox::new(Orientation::Horizontal, 8);
        checks_box.set_margin_start(4);
        checks_box.set_margin_top(4);

        let dc_check = CheckButton::with_label("Center Waveforms");
        dc_check.set_tooltip_text(Some("Shifts each waveform frame to be centered around the midpoint"));
        dc_check.set_active(false);
        checks_box.append(&dc_check);

        let norm_check = CheckButton::with_label("Maximize Volume");
        norm_check.set_active(false);
        checks_box.append(&norm_check);

        // How the loaded audio was cut into frames. Hidden until a load reports one, since the other file types decide it themselves.
        let mode_pill = ChooseOnePill::new(&["Single", "Pitch", "Fixed"], 0);
        mode_pill.widget().set_halign(Align::End);
        mode_pill.widget().set_hexpand(true);
        mode_pill.widget().set_visible(false);
        checks_box.append(mode_pill.widget());

        base.root.append(&checks_box);

        // Seed both panels with the default wavetable at the center position.
        table_view.set_table(&wavetable);
        let mid_frame_idx = ((table_len - 1) as f64 * 0.5).round() as usize;
        wave_view.set_wave(&wavetable[mid_frame_idx.min(table_len - 1)]);
        table_view.set_position(0.5);

        // Holds the active wavetable and upload settings.
        let state = Rc::new(RefCell::new(UploadDigiproState {
            wavetable: wavetable.clone(),
            raw_wavetable: wavetable,
            is_multi_mode: false,
            saved_single_pos: 0.5,
            saved_range: None,
            dc: Arc::clone(&dc),
            get_midi_rc: Rc::new(get_selected_midi),
            source: None,
        }));

        // ── Toggle reactions: re-process `raw_wavetable` and refresh views immediately ─────────────────────

        {
            let state_c = Rc::clone(&state);
            let wave_view_c = wave_view.clone();
            let table_view_c = table_view.clone();
            let range_slider_c = range_slider.clone();
            let norm_check_c = norm_check.clone();
            let dc_check_c = dc_check.clone();
            norm_check.connect_toggled(move |_| {
                apply_toggles_and_refresh(&state_c, &wave_view_c, &table_view_c, &range_slider_c, &norm_check_c, &dc_check_c);
            });
        }
        {
            let state_c = Rc::clone(&state);
            let wave_view_c = wave_view.clone();
            let table_view_c = table_view.clone();
            let range_slider_c = range_slider.clone();
            let norm_check_c = norm_check.clone();
            let dc_check_c = dc_check.clone();
            dc_check.connect_toggled(move |_| {
                apply_toggles_and_refresh(&state_c, &wave_view_c, &table_view_c, &range_slider_c, &norm_check_c, &dc_check_c);
            });
        }

        // ── Instructions card ──────────────────────────────────────────────────────────────────────────────

        let device_short = dc.device_short.clone();
        let device_shorter = dc.device_shorter.clone();
        base.build_instructions_card(
            &[
                &format!("1. On the {device_short}, navigate to GLOBAL > FILE > DIGIPRO MGR."),
                "2. Select RECEIVE, choose ORG mode, and press YES.",
                &format!("3. The {device_shorter} screen should now show \"WAITING\"."),
                "4. Drag the slider to select a frame, set Slot and Name, then click 'Send to Device'.",
                "   In ORG mode, the Slot field controls which slot receives the waveform.",
            ],
            "Device Prep",
        );

        // ── Presets row ────────────────────────────────────────────────────────────────────────────────────

        let presets_flow = FlowBox::new();
        presets_flow.set_selection_mode(SelectionMode::None);
        presets_flow.set_max_children_per_line(PRESETS.len() as u32);
        presets_flow.set_hexpand(true);
        presets_flow.set_column_spacing(6);
        presets_flow.set_row_spacing(6);
        presets_flow.set_margin_bottom(8);

        // Creates shortcut buttons for internal wavetable presets.
        for &(name, func) in PRESETS {
            let btn = Button::with_label(name);
            let state_c = Rc::clone(&state);
            let wave_view_c = wave_view.clone();
            let table_view_c = table_view.clone();
            let range_slider_c = range_slider.clone();
            let multi_btn_c = multi_btn.clone();
            let mode_pill_c = mode_pill.clone();
            let norm_check_c = norm_check.clone();
            let dc_check_c = dc_check.clone();
            btn.connect_clicked(move |_| {
                let table = func(as_u64_or_die(state_c.borrow().dc.json_get("digipro.wave_length")) as usize);
                let table_len = table.len();
                update_multi_sensitivity(&multi_btn_c, table_len > 1);

                // A generated preset replaces whatever was loaded, so there is no longer a file to re-cut.
                state_c.borrow_mut().source = None;
                mode_pill_c.widget().set_visible(false);

                // `raw_wavetable` must be updated before `update_toggle_sensitivity()`.
                // Unchecking a box fires `connect_toggled` -> `apply_toggles_and_refresh()`, which reads `raw_wavetable` from state.
                // Without this, it would see the previous preset's data.
                let mut state_mut = state_c.borrow_mut();
                state_mut.wavetable.clone_from(&table);
                state_mut.raw_wavetable.clone_from(&table);
                let multi = state_mut.is_multi_mode;
                drop(state_mut);
                range_slider_c.set_step(1.0 / (table_len - 1).max(1) as f64);
                table_view_c.set_table(&table);

                if multi {
                    let start = range_slider_c.start();
                    let end = range_slider_c.end();
                    let start_idx = ((table_len - 1) as f64 * start).round() as usize;
                    let end_frame_idx = ((table_len - 1) as f64 * end).round() as usize;
                    wave_view_c.set_waves(&table[start_idx..=end_frame_idx.min(table_len - 1)]);
                    table_view_c.set_range(start, end);
                } else {
                    let pos = range_slider_c.pos();
                    let frame_idx = ((table_len - 1) as f64 * pos).round() as usize;
                    wave_view_c.set_wave(&table[frame_idx.min(table_len - 1)]);
                    table_view_c.set_position(pos);
                }

                update_toggle_sensitivity(&table, &norm_check_c, &dc_check_c);
            });
            presets_flow.insert(&btn, -1);
        }
        base.root.append(&presets_flow);

        // ── Controls row ───────────────────────────────────────────────────────────────────────────────────

        let control_row = GtkBox::builder().spacing(8).build();

        let load_btn = Button::with_label("Load from File...");
        control_row.append(&load_btn);

        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        control_row.append(&spacer);

        let slot_label = gtk4::Label::builder().label("Slot:").valign(Align::Center).build();
        // Spinner is 1-indexed. The device's own slot range (from `sysex_layout.digipro`) is 0-indexed.
        let slot_spin = NumberSpinner::new(
            f64::from(slot_min + 1),
            f64::from(slot_min + 1),
            f64::from(slot_max + 1),
            1.0,
            "",
            0,
        );

        let slot_end_label = gtk4::Label::builder()
            .label("End Slot:")
            .valign(Align::Center)
            .margin_start(4)
            .build();
        slot_end_label.set_visible(false);

        let slot_end_val = Button::new();
        slot_end_val.add_css_class("delay-value");
        slot_end_val.set_focusable(false);
        slot_end_val.set_valign(Align::Center);
        slot_end_val.set_visible(false);

        let name_label = gtk4::Label::builder().label("Name:").valign(Align::Center).build();
        let name_entry = CustomTextbox::new();
        name_entry.enable_elektron_filter();
        name_entry.set_max_length(i32::from(name_size));
        name_entry.set_text("WAVE");
        name_entry.set_width_chars(i32::from(name_size));
        name_entry.set_max_width_chars(i32::from(name_size) + 1);
        name_entry.set_valign(Align::Center);
        name_entry.set_tooltip_text(Some(&format!("Up to {name_size} characters; stored uppercase in the device")));

        control_row.append(&slot_label);
        control_row.append(slot_spin.widget());
        control_row.append(&slot_end_label);
        control_row.append(&slot_end_val);
        control_row.append(&name_label);
        control_row.append(&*name_entry);

        let save_btn = Button::with_label("Save to File");
        save_btn.set_valign(Align::Center);
        control_row.append(&save_btn);

        let send_btn = Button::with_label("Send to Device");
        send_btn.add_css_class("suggested-action");
        send_btn.set_valign(Align::Center);
        control_row.append(&send_btn);
        base.root.append(&control_row);

        base.build_status_area(true);
        let update_status = base.status_updater();
        let update_progress = base.progress_updater();

        // ── Frame mode: re-cut the loaded audio when the choice changes ────────────────────────────────────

        {
            let state_c = Rc::clone(&state);
            let wave_view_c = wave_view.clone();
            let table_view_c = table_view.clone();
            let range_slider_c = range_slider.clone();
            let multi_btn_c = multi_btn.clone();
            let mode_pill_c = mode_pill.clone();
            let update_status_c = Arc::clone(&update_status);
            let update_progress_c = Arc::clone(&update_progress);
            let norm_check_c = norm_check.clone();
            let dc_check_c = dc_check.clone();
            mode_pill.connect_changed(move |idx| {
                let Some((path, trim_front, trim_back)) = state_c.borrow().source.clone() else {
                    return;
                };
                load_file_bg(
                    &path,
                    trim_front,
                    trim_back,
                    frame_mode_from_index(idx),
                    &norm_check_c,
                    &dc_check_c,
                    &wave_view_c,
                    &table_view_c,
                    &range_slider_c,
                    &multi_btn_c,
                    &mode_pill_c,
                    &update_status_c,
                    &update_progress_c,
                    &state_c,
                );
            });
        }

        // ── Unified load dispatcher (used by both the drop target and Load button) ─────────────────────────

        let on_load: Rc<dyn Fn(String)> = {
            let wave_view_c = wave_view.clone();
            let table_view_c = table_view.clone();
            let range_slider_c = range_slider.clone();
            let multi_btn_c = multi_btn.clone();
            let mode_pill_c = mode_pill.clone();
            let update_status_c = Arc::clone(&update_status);
            let update_progress_c = Arc::clone(&update_progress);
            let state_c = Rc::clone(&state);
            let norm_check_c = norm_check.clone();
            let dc_check_c = dc_check.clone();
            Rc::new(move |path: String| {
                dispatch_load(
                    &path,
                    &wave_view_c,
                    &table_view_c,
                    &range_slider_c,
                    &multi_btn_c,
                    &mode_pill_c,
                    &update_status_c,
                    &update_progress_c,
                    &norm_check_c,
                    &dc_check_c,
                    &state_c,
                );
            })
        };

        // ── Drop target ────────────────────────────────────────────────────────────────────────────────────

        {
            let on_load_c = Rc::clone(&on_load);
            wire_file_drop_target(&base.root, move |path| on_load_c(path));
        }

        // ── Slot spin change ───────────────────────────────────────────────────────────────────────────────

        {
            let state_c = Rc::clone(&state);
            let range_slider_c = range_slider.clone();
            let slot_end_val_c = slot_end_val.clone();
            let slot_spin_c = slot_spin.clone();
            slot_spin.connect_value_changed(move || {
                let state_ref = state_c.borrow();
                if state_ref.is_multi_mode {
                    update_slot_range_label(
                        slot_spin_c.value() as u8,
                        &state_ref.wavetable,
                        &range_slider_c,
                        as_u64_or_die(state_ref.dc.layout_field_or_die("digipro", "slot").json_get("max")) as u8 + 1,
                        &slot_end_val_c,
                    );
                }
            });
        }

        // ── Range slider pos callback ──────────────────────────────────────────────────────────────────────

        {
            let wave_view_c = wave_view.clone();
            let table_view_c = table_view.clone();
            let state_c = Rc::clone(&state);
            range_slider.connect_pos_changed(move |pos| {
                let state_ref = state_c.borrow();
                let wave_count = state_ref.wavetable.len();
                if wave_count == 0 {
                    return;
                }
                let frame_idx = ((wave_count - 1) as f64 * pos).round() as usize;
                wave_view_c.set_wave(&state_ref.wavetable[frame_idx.min(wave_count - 1)]);
                table_view_c.set_position(pos);
            });
        }

        // ── Range slider range callback ────────────────────────────────────────────────────────────────────

        {
            let wave_view_c = wave_view.clone();
            let table_view_c = table_view.clone();
            let state_c = Rc::clone(&state);
            let range_slider_c = range_slider.clone();
            let slot_spin_c = slot_spin.clone();
            let slot_end_val_c = slot_end_val.clone();
            range_slider.connect_range_changed(move |start, end| {
                let state_ref = state_c.borrow();
                let table_len = state_ref.wavetable.len();
                let start_idx = ((table_len - 1) as f64 * start).round() as usize;
                let end_frame_idx = ((table_len - 1) as f64 * end).round() as usize;
                wave_view_c.set_waves(&state_ref.wavetable[start_idx..=end_frame_idx.min(table_len - 1)]);
                table_view_c.set_range(start, end);
                update_slot_range_label(
                    slot_spin_c.value() as u8,
                    &state_ref.wavetable,
                    &range_slider_c,
                    as_u64_or_die(state_ref.dc.layout_field_or_die("digipro", "slot").json_get("max")) as u8 + 1,
                    &slot_end_val_c,
                );
            });
        }

        // ── Multi toggle ───────────────────────────────────────────────────────────────────────────────────

        {
            let state_c = Rc::clone(&state);
            let range_slider_c = range_slider.clone();
            let send_btn_c = send_btn.clone();
            let slot_spin_c = slot_spin.clone();
            multi_btn.connect_toggled(move |btn| {
                let multi = btn.is_active();
                let mut state_mut = state_c.borrow_mut();
                let table_len = state_mut.wavetable.len();
                if table_len > 0 {
                    state_mut.is_multi_mode = multi;
                }

                if multi {
                    state_mut.saved_single_pos = range_slider_c.pos();
                    let (start, end) = if let Some((start, end)) = state_mut.saved_range {
                        (start, end)
                    } else {
                        let pos = range_slider_c.pos();
                        ((pos - 0.25).max(0.0), (pos + 0.25).min(1.0))
                    };
                    drop(state_mut);
                    range_slider_c.set_thumbs(start, end);
                    range_slider_c.set_multi(true);
                    wave_view.set_multi_mode(true);
                    table_view.set_multi_mode(true);
                    send_btn_c.set_sensitive(true);
                    send_btn_c.set_tooltip_text(Some("Multi mode: sequentially send each waveform to the hardware"));
                    slot_label.set_label("Start Slot:");
                    slot_end_label.set_visible(true);
                    slot_end_val.set_visible(true);
                    let state_ref = state_c.borrow();
                    update_slot_range_label(
                        slot_spin_c.value() as u8,
                        &state_ref.wavetable,
                        &range_slider_c,
                        as_u64_or_die(state_ref.dc.layout_field_or_die("digipro", "slot").json_get("max")) as u8 + 1,
                        &slot_end_val,
                    );
                    let start_idx = ((table_len - 1) as f64 * start).round() as usize;
                    let end_frame_idx = ((table_len - 1) as f64 * end).round() as usize;
                    wave_view.set_waves(&state_ref.wavetable[start_idx..=end_frame_idx.min(table_len - 1)]);
                    table_view.set_range(start, end);
                } else {
                    state_mut.saved_range = Some((range_slider_c.start(), range_slider_c.end()));
                    let saved_pos = state_mut.saved_single_pos;
                    drop(state_mut);
                    range_slider_c.set_multi(false);
                    range_slider_c.set_pos(saved_pos);
                    wave_view.set_multi_mode(false);
                    table_view.set_multi_mode(false);
                    send_btn_c.set_sensitive(true);
                    send_btn_c.set_tooltip_text(None);
                    slot_label.set_label("Slot:");
                    slot_end_label.set_visible(false);
                    slot_end_val.set_visible(false);
                    let state_ref = state_c.borrow();
                    let frame_idx = ((table_len - 1) as f64 * saved_pos).round() as usize;
                    wave_view.set_wave(&state_ref.wavetable[frame_idx.min(table_len - 1)]);
                    table_view.set_position(saved_pos);
                }
            });
        }

        // ── Load button ────────────────────────────────────────────────────────────────────────────────────

        {
            let on_load_c = Rc::clone(&on_load);
            let root_c = base.root.clone();
            load_btn.connect_clicked(move |_| {
                on_load_clicked(&root_c, Rc::clone(&on_load_c));
            });
        }

        // ── Save button ────────────────────────────────────────────────────────────────────────────────────

        {
            let state = Rc::clone(&state);
            let range_slider_c = range_slider.clone();
            let name_entry_c = name_entry.clone();
            let slot_spin_c = slot_spin.clone();
            let update_status_c = Arc::clone(&update_status);
            let update_progress_c = Arc::clone(&update_progress);
            let root_c = base.root.clone();
            save_btn.connect_clicked(move |_| {
                on_save_clicked(
                    &root_c,
                    &state,
                    &range_slider_c,
                    &name_entry_c,
                    &slot_spin_c,
                    &update_status_c,
                    &update_progress_c,
                );
            });
        }

        // ── Send button ────────────────────────────────────────────────────────────────────────────────────

        {
            let state = Rc::clone(&state);
            let update_status_c = Arc::clone(&update_status);
            let update_progress_c = Arc::clone(&update_progress);
            let btn = send_btn.clone();
            send_btn.connect_clicked(move |_| {
                on_send_clicked(
                    &state,
                    &range_slider,
                    &name_entry,
                    &slot_spin,
                    &update_status_c,
                    &update_progress_c,
                    &btn,
                );
            });
        }

        // Set initial toggle sensitivity based on the default preset.
        update_toggle_sensitivity(&state.borrow().raw_wavetable.clone(), &norm_check, &dc_check);

        // Clear focus on background click.
        {
            let root_c = base.root.clone();
            let bg_click = GestureClick::new();
            bg_click.connect_pressed(move |_, _, _, _| {
                if let Some(root) = root_c.root() {
                    root.set_focus(None::<&gtk4::Widget>);
                }
            });
            base.root.add_controller(bg_click);
        }

        UploadDigiproScreen { root: base.root }
    }

    /// Returns the root widget for embedding in the navigation stack.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ── File loading ───────────────────────────────────────────────────────────────────────────────────────────

/// Routes files to the appropriate loader based on extension.
fn dispatch_load(
    path: &str,
    wave_view: &WaveformView,
    table_view: &WavetableView,
    range_slider: &RangeSlider,
    multi_btn: &ToggleButton,
    mode_pill: &ChooseOnePill,
    update_status: &StatusFn,
    update_progress: &ProgressFn,
    norm_check: &CheckButton,
    dc_check: &CheckButton,
    state: &Rc<RefCell<UploadDigiproState>>,
) {
    let lower = path.to_lowercase();

    // Reject SysEx/c7 files that aren't DigiPro waveform data.
    if lower.ends_with(".syx") || lower.ends_with(".sysex") {
        let dc = Arc::clone(&state.borrow().dc);
        let is_digipro = std::fs::read(path).ok().is_some_and(|bytes| contains_digipro(&dc, &bytes));
        if !is_digipro {
            show_mismatch_dialog(
                wave_view.widget(),
                "a DigiPro waveform (.syx)",
                "an unrecognized SysEx file",
                Some(path),
                "",
            );
            return;
        }
    } else if lower.ends_with(".c7") {
        // `write_c7_file()` only emits the "c7" header when metadata is passed, so a headerless file has no type and is allowed through.
        let items = read_c7_file(path);
        let c7_type = items
            .iter()
            .find(|item| item.section == "c7")
            .map(|item| item.get_type().to_lowercase())
            .unwrap_or_default();
        if !c7_type.is_empty() && c7_type != "digipro" && c7_type != "digipro_wavetable" {
            show_mismatch_dialog(
                wave_view.widget(),
                "a DigiPro or DigiPro Wavetable file",
                &format!("{c7_type} data"),
                Some(path),
                "",
            );
            return;
        }
    }

    if SYSEX_EXTS.iter().any(|ext| lower.ends_with(ext)) {
        analyze_sysex_then_load(
            path,
            wave_view,
            table_view,
            range_slider,
            multi_btn,
            mode_pill,
            update_status,
            update_progress,
            norm_check,
            dc_check,
            state,
        );
    } else {
        // Both audio and images go through the silence analyzer. Images will detect none and load directly.
        analyze_then_show_modal(
            path,
            wave_view,
            table_view,
            range_slider,
            multi_btn,
            mode_pill,
            update_status,
            update_progress,
            norm_check,
            dc_check,
            state,
        );
    }
}

/// Decodes a `.c7`/`.syx`/`.sds` file on a background thread and applies the main-window Normalize/Center toggles directly.
fn analyze_sysex_then_load(
    path: &str,
    wave_view: &WaveformView,
    table_view: &WavetableView,
    range_slider: &RangeSlider,
    multi_btn: &ToggleButton,
    mode_pill: &ChooseOnePill,
    update_status: &StatusFn,
    update_progress: &ProgressFn,
    norm_check: &CheckButton,
    dc_check: &CheckButton,
    state: &Rc<RefCell<UploadDigiproState>>,
) {
    update_progress("Loading...", 0.0, "");
    let path_str = path.to_string();
    let wave_view_c = wave_view.clone();
    let table_view_c = table_view.clone();
    let range_slider_c = range_slider.clone();
    let multi_btn_c = multi_btn.clone();
    let mode_pill_c = mode_pill.clone();
    let update_status_c = Arc::clone(update_status);
    let update_progress_c = Arc::clone(update_progress);
    let state_c = Rc::clone(state);
    let norm_check_c = norm_check.clone();
    let dc_check_c = dc_check.clone();

    glib::spawn_future_local(async move {
        let (wave_length, dc) = {
            let state_ref = state_c.borrow();
            (
                as_u64_or_die(state_ref.dc.json_get("digipro.wave_length")) as usize,
                Arc::clone(&state_ref.dc),
            )
        };
        let result = sysex_to_digipro_wavetable(&dc, &path_str, wave_length);
        match result {
            Ok(raw) => {
                let label = std::path::Path::new(&path_str)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let mut table = raw.clone();
                if dc_check_c.is_active() {
                    table = center_table(table);
                }
                if norm_check_c.is_active() {
                    table = normalize_table(table);
                }
                apply_table(
                    &table,
                    &raw,
                    &label,
                    &state_c,
                    &wave_view_c,
                    &table_view_c,
                    &range_slider_c,
                    &multi_btn_c,
                    &update_status_c,
                    &update_progress_c,
                );
                update_toggle_sensitivity(&raw, &norm_check_c, &dc_check_c);

                // A SysEx wavetable brings its own framing, so the frame-mode pill has nothing to offer here.
                state_c.borrow_mut().source = None;
                mode_pill_c.widget().set_visible(false);
            }
            Err(e) => {
                update_status_c(&format!("Error: {e}"));
            }
        }
    });
}

/// Detects silence on a background thread and shows the clipping-suggestions modal only when silence is found.
///
/// Normalize and Center state comes from the main-window `CheckButtons`.
fn analyze_then_show_modal(
    path: &str,
    wave_view: &WaveformView,
    table_view: &WavetableView,
    range_slider: &RangeSlider,
    multi_btn: &ToggleButton,
    mode_pill: &ChooseOnePill,
    update_status: &StatusFn,
    update_progress: &ProgressFn,
    norm_check: &CheckButton,
    dc_check: &CheckButton,
    state: &Rc<RefCell<UploadDigiproState>>,
) {
    update_progress("Analyzing...", 0.0, "");
    let path_str = path.to_string();
    let wave_view_c = wave_view.clone();
    let table_view_c = table_view.clone();
    let range_slider_c = range_slider.clone();
    let multi_btn_c = multi_btn.clone();
    let mode_pill_c = mode_pill.clone();
    let update_status_c = Arc::clone(update_status);
    let update_progress_c = Arc::clone(update_progress);
    let state_c = Rc::clone(state);
    let norm_check_c = norm_check.clone();
    let dc_check_c = dc_check.clone();

    glib::spawn_future_local(async move {
        let (front_seconds, back_seconds) = detect_silence(&path_str);

        if front_seconds < 0.01 && back_seconds < 0.01 {
            // No silence to trim. Load directly using main-window checkbox states.
            load_file_bg(
                &path_str,
                false,
                false,
                FrameMode::Auto,
                &norm_check_c,
                &dc_check_c,
                &wave_view_c,
                &table_view_c,
                &range_slider_c,
                &multi_btn_c,
                &mode_pill_c,
                &update_status_c,
                &update_progress_c,
                &state_c,
            );
        } else {
            let path_for_load = path_str.clone();
            // Cloned up front because the callback below takes ownership of `wave_view_c`.
            let parent_widget = wave_view_c.widget().clone();
            show_trim_dialog(&parent_widget, &path_str, front_seconds, back_seconds, move |choice| {
                let Some((should_trim_front, should_trim_back)) = choice else {
                    update_status_c("Status: Cancelled.");
                    return;
                };
                load_file_bg(
                    &path_for_load,
                    should_trim_front,
                    should_trim_back,
                    FrameMode::Auto,
                    &norm_check_c,
                    &dc_check_c,
                    &wave_view_c,
                    &table_view_c,
                    &range_slider_c,
                    &multi_btn_c,
                    &mode_pill_c,
                    &update_status_c,
                    &update_progress_c,
                    &state_c,
                );
            });
        }
    });
}

/// Decodes a file into the wavetable (runs in background thread, idles back to UI).
///
/// Loads without `norm/dc_check_c` so the raw table is preserved for reactive toggle re-processing.
fn load_file_bg(
    path: &str,
    should_trim_front: bool,
    should_trim_back: bool,
    mode: FrameMode,
    norm_check: &CheckButton,
    dc_check: &CheckButton,
    wave_view: &WaveformView,
    table_view: &WavetableView,
    range_slider: &RangeSlider,
    multi_btn: &ToggleButton,
    mode_pill: &ChooseOnePill,
    update_status: &StatusFn,
    update_progress: &ProgressFn,
    state: &Rc<RefCell<UploadDigiproState>>,
) {
    update_progress("Loading...", 0.0, "");
    let path_str = path.to_string();
    let wave_view_c = wave_view.clone();
    let table_view_c = table_view.clone();
    let range_slider_c = range_slider.clone();
    let multi_btn_c = multi_btn.clone();
    let mode_pill_c = mode_pill.clone();
    let update_status_c = Arc::clone(update_status);
    let update_progress_c = Arc::clone(update_progress);
    let state_c = Rc::clone(state);
    let norm_check_c = norm_check.clone();
    let dc_check_c = dc_check.clone();

    glib::spawn_future_local(async move {
        let (wave_length, dc) = {
            let state_ref = state_c.borrow();
            (
                as_u64_or_die(state_ref.dc.json_get("digipro.wave_length")) as usize,
                Arc::clone(&state_ref.dc),
            )
        };
        let result = load_wavetable_file(&dc, &path_str, should_trim_front, should_trim_back, false, false, wave_length, mode);
        match result {
            Ok((raw, used_mode)) => {
                let label = std::path::Path::new(&path_str)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let mut table = raw.clone();
                if dc_check_c.is_active() {
                    table = center_table(table);
                }
                if norm_check_c.is_active() {
                    table = normalize_table(table);
                }
                apply_table(
                    &table,
                    &raw,
                    &label,
                    &state_c,
                    &wave_view_c,
                    &table_view_c,
                    &range_slider_c,
                    &multi_btn_c,
                    &update_status_c,
                    &update_progress_c,
                );
                update_toggle_sensitivity(&raw, &norm_check_c, &dc_check_c);

                // Only audio reports a mode, so anything else clears the source and hides the pill.
                if let Some(used_mode) = used_mode {
                    state_c.borrow_mut().source = Some((path_str.clone(), should_trim_front, should_trim_back));
                    show_frame_mode(&mode_pill_c, used_mode, mode == FrameMode::Auto);
                } else {
                    state_c.borrow_mut().source = None;
                    mode_pill_c.widget().set_visible(false);
                }
            }
            Err(e) => {
                update_status_c(&format!("Error: {e}"));
            }
        }
    });
}

/// Reprocesses `raw_wavetable` with the current toggle states and refreshes all views in-place.
///
/// Called whenever Maximize Volume or Center Waveforms is toggled.
fn apply_toggles_and_refresh(
    state: &Rc<RefCell<UploadDigiproState>>,
    wave_view: &WaveformView,
    table_view: &WavetableView,
    range_slider: &RangeSlider,
    norm_check: &CheckButton,
    dc_check: &CheckButton,
) {
    let mut table = state.borrow().raw_wavetable.clone();
    if table.is_empty() {
        return;
    }
    if dc_check.is_active() {
        table = center_table(table);
    }
    if norm_check.is_active() {
        table = normalize_table(table);
    }
    let table_len = table.len();
    let multi = state.borrow().is_multi_mode;
    state.borrow_mut().wavetable.clone_from(&table);
    table_view.set_table(&table);

    if multi {
        let start = range_slider.start();
        let end = range_slider.end();
        let start_idx = ((table_len - 1) as f64 * start).round() as usize;
        let end_frame_idx = ((table_len - 1) as f64 * end).round() as usize;
        wave_view.set_waves(&table[start_idx..=end_frame_idx.min(table_len - 1)]);
        table_view.set_range(start, end);
    } else {
        let pos = range_slider.pos();
        let frame_idx = ((table_len - 1) as f64 * pos).round() as usize;
        wave_view.set_wave(&table[frame_idx.min(table_len - 1)]);
        table_view.set_position(pos);
    }
}

/// Applies a decoded int16 wavetable to the screen (must be called on the main thread).
///
/// `raw` is the pre-norm/`dc_check_c` table stored so reactive toggle changes can re-process it.
fn apply_table(
    table: &[Vec<i16>],
    raw: &[Vec<i16>],
    label: &str,
    state: &Rc<RefCell<UploadDigiproState>>,
    wave_view: &WaveformView,
    table_view: &WavetableView,
    range_slider: &RangeSlider,
    multi_btn: &ToggleButton,
    _update_status: &StatusFn,
    update_progress: &ProgressFn,
) {
    update_multi_sensitivity(multi_btn, table.len() > 1);

    // Both must stay in sync so the slider and toggle re-processing work after every load.
    let mut state_mut = state.borrow_mut();
    state_mut.wavetable = table.to_vec();
    state_mut.raw_wavetable = raw.to_vec();
    let multi = state_mut.is_multi_mode;
    drop(state_mut);
    let table_len = table.len();
    range_slider.set_step(1.0 / (table_len - 1).max(1) as f64);
    table_view.set_table(table);
    if multi {
        let start = range_slider.start();
        let end = range_slider.end();
        let start_idx = ((table_len - 1) as f64 * start).round() as usize;
        let end_frame_idx = ((table_len - 1) as f64 * end).round() as usize;
        wave_view.set_waves(&table[start_idx..=end_frame_idx.min(table_len - 1)]);
        table_view.set_range(start, end);
    } else {
        range_slider.set_pos(0.5);
        let frame_idx = ((table_len - 1) as f64 * 0.5).round() as usize;
        wave_view.set_wave(&table[frame_idx.min(table_len - 1)]);
        table_view.set_position(0.5);
    }

    let msg = if label.is_empty() {
        format!("Loaded {table_len} frame(s)")
    } else {
        format!("Loaded {table_len} frame(s) from {label}")
    };
    update_progress(&msg, 1.0, "");
}

/// Opens a file chooser to load custom media.
fn on_load_clicked(root: &gtk4::Box, on_load: Rc<dyn Fn(String)>) {
    let filter_all = FileFilter::new();
    filter_all.set_name(Some("All supported"));
    for ext in IMAGE_EXTS.iter().chain(AUDIO_EXTS.iter()) {
        filter_all.add_pattern(&format!("*{ext}"));
    }
    filter_all.add_pattern("*.c7");
    let filters = gio::ListStore::new::<FileFilter>();
    filters.append(&filter_all);
    let dialog = gtk4::FileDialog::builder()
        .title("Load DigiPro Waveform, Wavetable, or Image")
        .accept_label("Load")
        .default_filter(&filter_all)
        .filters(&filters)
        .build();
    dialog.set_initial_folder(Some(&gio::File::for_path(get_export_folder())));
    let window = root.root().and_downcast::<gtk4::Window>();

    dialog.open(window.as_ref(), None::<&gio::Cancellable>, move |result| {
        if let Ok(file) = result
            && let Some(path) = file.path()
        {
            on_load(path.to_string_lossy().to_string());
        }
    });
}

// ── Save / Send ────────────────────────────────────────────────────────────────────────────────────────────

/// Opens a file chooser to export the current frame or range as a `.c7` file.
fn on_save_clicked(
    root: &gtk4::Box,
    state: &Rc<RefCell<UploadDigiproState>>,
    range_slider: &RangeSlider,
    name_entry: &Entry,
    slot_spin: &NumberSpinner,
    update_status: &StatusFn,
    update_progress: &ProgressFn,
) {
    let filter_c7 = FileFilter::new();
    filter_c7.set_name(Some("C7 Project Files (*.c7)"));
    filter_c7.add_pattern("*.c7");
    let filters = gio::ListStore::new::<FileFilter>();
    filters.append(&filter_c7);
    let name = name_or_default(name_entry, "WAVE");
    let dialog = gtk4::FileDialog::builder()
        .title("Save DigiPro as C7 File")
        .accept_label("Save")
        .initial_name(format!("{name}.c7"))
        .default_filter(&filter_c7)
        .filters(&filters)
        .build();
    dialog.set_initial_folder(Some(&gio::File::for_path(get_export_folder())));
    let window = root.root().and_downcast::<gtk4::Window>();

    let state = Rc::clone(state);
    let range_slider_c = range_slider.clone();
    let name_entry_c = name_entry.clone();
    let slot_spin_c = slot_spin.clone();
    let update_status_c = Arc::clone(update_status);
    let update_progress_c = Arc::clone(update_progress);
    dialog.save(window.as_ref(), None::<&gio::Cancellable>, move |result| {
        if let Ok(file) = result
            && let Some(mut path) = file.path()
        {
            if path
                .extension()
                .and_then(|extension_val| extension_val.to_str())
                .map(str::to_lowercase)
                .as_deref()
                != Some("c7")
            {
                let mut path_str = path.to_string_lossy().into_owned();
                path_str.push_str(".c7");
                path = std::path::PathBuf::from(path_str);
            }
            save_wavetable_frame(
                &path.to_string_lossy(),
                &state.borrow(),
                &range_slider_c,
                &name_entry_c,
                &slot_spin_c,
                &update_status_c,
                &update_progress_c,
            );
        }
    });
}

/// Writes the current frame (single mode) or range (multi mode) to a `.c7` file.
fn save_wavetable_frame(
    path: &str,
    state_ref: &UploadDigiproState,
    range_slider: &RangeSlider,
    name_entry: &Entry,
    slot_spin: &NumberSpinner,
    _update_status: &StatusFn,
    update_progress: &ProgressFn,
) {
    let table_len = state_ref.wavetable.len();
    if table_len == 0 {
        return;
    }
    let slot = slot_spin.value() as u32 - 1; // UI is 1-indexed; device is 0-indexed
    let name = name_or_default(name_entry, "WAVE");

    let result: Result<String, ()> = if state_ref.is_multi_mode {
        let start = range_slider.start();
        let end = range_slider.end();
        let start_idx = ((table_len - 1) as f64 * start).round() as usize;
        let end_frame_idx = ((table_len - 1) as f64 * end).round() as usize;
        // A frame past the last DigiPro slot has nowhere to land, so the export stops there.
        let slot_count = as_u64_or_die(state_ref.dc.json_get("sysex_api.digipro.slots")) as usize;
        // Frames are encoded from slot 0 upward so each packet's own slot byte agrees with its section number.
        let frames_sysex: Vec<Vec<u8>> = state_ref.wavetable[start_idx..=end_frame_idx.min(table_len - 1)]
            .iter()
            .take(slot_count)
            .enumerate()
            .map(|(i, frame)| i16_to_digipro_sysex(&state_ref.dc, i as u8, &name, frame, get_base_channel()))
            .collect();
        let count = frames_sysex.len();
        write_digipro_wavetable_c7(path, &frames_sysex, &name, &state_ref.dc.device_short);
        Ok(format!(
            "Wavetable ({count} frames) saved to {}",
            std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy()
        ))
    } else {
        let pos = range_slider.pos();
        let frame_idx = ((table_len - 1) as f64 * pos).round() as usize;
        let sysex = i16_to_digipro_sysex(
            &state_ref.dc,
            slot as u8,
            &name,
            &state_ref.wavetable[frame_idx.min(table_len - 1)],
            get_base_channel(),
        );
        write_digipro_c7(path, &sysex, slot, &name, &state_ref.dc.device_short);
        Ok(format!(
            "Frame {} saved to {}",
            frame_idx + 1,
            std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy()
        ))
    };

    match result {
        Ok(msg) => update_progress(&msg, 1.0, ""),
        Err(()) => update_progress("Save error.", 0.0, ""),
    }
}

/// Encodes the selected waveform frame(s) and sends them as a `0x5D` SysEx message.
fn on_send_clicked(
    state: &Rc<RefCell<UploadDigiproState>>,
    range_slider: &RangeSlider,
    name_entry: &Entry,
    slot_spin: &NumberSpinner,
    _update_status: &StatusFn,
    update_progress: &ProgressFn,
    send_btn: &Button,
) {
    let state_ref = state.borrow();
    let port_name = match (state_ref.get_midi_rc)() {
        Some(port_name) if is_valid_port(&port_name) => port_name,
        _ => {
            update_progress("Error: select a valid MIDI port.", 0.0, "");
            return;
        }
    };

    let table_len = state_ref.wavetable.len();
    let start_slot = slot_spin.value() as i64 - 1; // UI is 1-indexed, SysEx is 0-indexed
    let name = name_or_default(name_entry, "WAVE");

    let waves: Vec<(i64, Vec<i16>)> = if state_ref.is_multi_mode {
        let start = range_slider.start();
        let end = range_slider.end();
        let start_idx = ((state_ref.wavetable.len() - 1) as f64 * start).round() as usize;
        let end_idx = ((state_ref.wavetable.len() - 1) as f64 * end).round() as usize;
        state_ref.wavetable[start_idx..=end_idx.min(state_ref.wavetable.len() - 1)]
            .iter()
            .enumerate()
            .map(|(i, frame)| (start_slot + i as i64, frame.clone()))
            .collect()
    } else {
        let frame_idx = ((table_len - 1) as f64 * range_slider.pos()).round() as usize;
        vec![(start_slot, state_ref.wavetable[frame_idx.min(table_len - 1)].clone())]
    };

    let count = waves.len();
    let slot_start = waves.first().map_or(0, |wave_entry| wave_entry.0 + 1);
    let slot_end = waves.last().map_or(0, |wave_entry| wave_entry.0 + 1);
    let max_slot = i64::from(as_u64_or_die(state_ref.dc.layout_field_or_die("digipro", "slot").json_get("max")) as u8);
    let dc = Arc::clone(&state_ref.dc);
    drop(state_ref);

    send_btn.set_sensitive(false);
    update_progress(&format!("Sending (slot {slot_start}..={slot_end})..."), 0.0, "");

    let update_progress = Arc::clone(update_progress);
    let send_btn = send_btn.clone();
    let start = Instant::now();
    let (tx, rx) = async_channel::unbounded::<(String, f64)>();

    glib::spawn_future_local(async move {
        while let Ok((msg, frac)) = rx.recv().await {
            let eta = eta_string(start, frac);
            update_progress(&msg, frac, &eta);
        }
        send_btn.set_sensitive(true);
    });

    glib::spawn_future_local(async move {
        let tx_c = tx.clone();
        let result = run_midi_output(&port_name, move |midi_out| {
            for (i, (slot, wave)) in waves.iter().enumerate() {
                if *slot > max_slot {
                    break;
                }
                let syx = i16_to_digipro_sysex(&dc, *slot as u8, &name, wave, get_base_channel());
                let pct = i as f64 / count as f64;
                let msg = format!("Sending waveform {}/{count}...", i + 1);
                let _ = tx_c.send_blocking((msg.clone(), pct));
                midi_out.sysex(&syx);
                let drain_speed = syx.len() as f64 / 3125.0 + 0.2;
                let steps = (drain_speed * 10.0).max(1.0) as usize;
                for tenth in 0..steps {
                    let current = pct + (1.0 / count as f64) * (tenth as f64 / steps as f64);
                    let _ = tx_c.send_blocking((msg.clone(), current));
                    sleep(Duration::from_millis(100));
                }
            }
            Ok(())
        })
        .await;

        let final_msg = match result {
            Ok(()) => {
                if count == 1 {
                    format!("Sent to slot {}!", start_slot + 1)
                } else {
                    format!("Sent {count} waves!")
                }
            }
            Err(e) => format!("Error: {e}"),
        };
        let _ = tx.send_blocking((final_msg, 1.0));
    });
}

// ── Module-level helpers ───────────────────────────────────────────────────────────────────────────────────

/// Reveals the frame-mode pill and points it at the mode a load actually used.
fn show_frame_mode(mode_pill: &ChooseOnePill, mode: FrameMode, was_auto: bool) {
    mode_pill.widget().set_visible(true);

    // An Auto load tries pitch before anything else, so landing elsewhere means this audio has no steady pitch to find.
    // Landing on Single means the file is shorter than two frames, so Fixed would cut that same lone frame.
    // Only an Auto load tests either one, so an explicit choice leaves both buttons as it found them.
    if was_auto {
        mode_pill.set_item_sensitive(1, mode == FrameMode::Pitch);
        mode_pill.set_item_sensitive(2, mode != FrameMode::SingleCycle);
    }

    mode_pill.set_active(match mode {
        FrameMode::Pitch => 1,
        FrameMode::FixedFrame => 2,
        FrameMode::SingleCycle | FrameMode::Auto => 0,
    });
}

/// Maps a pill index back to the mode it stands for.
fn frame_mode_from_index(idx: i32) -> FrameMode {
    match idx {
        1 => FrameMode::Pitch,
        2 => FrameMode::FixedFrame,
        _ => FrameMode::SingleCycle,
    }
}

/// Grays out Multi mode when the table holds one frame, since a range spanning one frame is what single mode already does.
fn update_multi_sensitivity(multi_btn: &ToggleButton, is_multi_frame: bool) {
    if !is_multi_frame {
        multi_btn.set_active(false);
    }
    multi_btn.set_sensitive(is_multi_frame);
    multi_btn.set_tooltip_text(Some(if is_multi_frame {
        "Multi mode: select a range of frames; 'Save to File' exports as a wavetable"
    } else {
        "Multi mode needs a wavetable with more than one frame"
    }));
}

/// Grays out each toggle when its operation would have no effect on the current raw wavetable.
fn update_toggle_sensitivity(raw: &[Vec<i16>], norm_check: &CheckButton, dc_check: &CheckButton) {
    if raw.is_empty() {
        norm_check.set_sensitive(false);
        norm_check.set_active(false);
        dc_check.set_sensitive(false);
        dc_check.set_active(false);
        return;
    }

    let peak: i32 = raw
        .iter()
        .flat_map(|frame| frame.iter())
        .map(|&sample| i32::from(sample).abs())
        .max()
        .unwrap_or(0);
    // Maximizing does nothing when the table is flat (peak 0) or already at full amplitude (peak 32767).
    let norm_sensitive = peak > 0 && peak < 32767;
    norm_check.set_sensitive(norm_sensitive);
    if !norm_sensitive {
        norm_check.set_active(false);
    }

    // Centering does nothing when every frame's mean is already 0.
    let already_centered = raw.iter().all(|frame| {
        if frame.is_empty() {
            return true;
        }
        let mean = frame.iter().map(|&sample| f64::from(sample)).sum::<f64>() / frame.len() as f64;
        mean.abs() < 1.0
    });
    let dc_sensitive = !already_centered;
    dc_check.set_sensitive(dc_sensitive);
    if !dc_sensitive {
        dc_check.set_active(false);
    }
}

/// Computes the last slot in the upload range and shows it next to the start-slot spinner.
///
/// `max_slot_ui` is the device's `slot_max` (from `sysex_layout.digipro.fields` "slot"), shifted to the spinner's 1-indexed display.
fn update_slot_range_label(slot_start: u8, wavetable: &[Vec<i16>], range_slider: &RangeSlider, max_slot_ui: u8, slot_end_val: &Button) {
    let table_len = wavetable.len();
    if table_len == 0 {
        return;
    }
    let start = range_slider.start();
    let end = range_slider.end();
    let count = (((table_len - 1) as f64 * end).round() as i64 - ((table_len - 1) as f64 * start).round() as i64 + 1).max(1) as u8;
    slot_end_val.set_label(&(slot_start + count - 1).min(max_slot_ui).to_string());
}
