//! Send SDS samples to Machinedrum and Analog Rytm slots.
//!
//! Load → visualize with draggable trim/loop markers → Send to Device or Save to File.

/*
Structurally identical to `upload_digipro.rs`, but with different section content objectives.
If you change ANY structural code in here, please change `upload_digipro.rs` to use the new standard too.
*/

/*
At the moment, only closed-loop SDS transferring is allowed. Open-loop will not be added.
Having users encounter transfer failures because of faster speeds erases some of the magic of C7.

Technically, the Machinedrum supports SDS "ping-pong" playback, where the sample plays forward then backward.
But I'm not really sure how I'd integrate that with how the UI is currently laid out.
*/

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use gtk4::{
    self, Align, Box as GtkBox, Button, CheckButton, DrawingArea, Entry, EventControllerKey, FileFilter, GestureClick, Orientation, gdk,
    gio, glib,
};

use crate::ui::base_module::{BaseModule, ProgressFn, StatusFn, eta_string};
use crate::ui::drawing::{
    COLOR_BLUE_B, COLOR_BLUE_G, COLOR_BLUE_R, COLOR_GREEN_B, COLOR_GREEN_G, COLOR_GREEN_R, COLOR_ORANGE_B, COLOR_ORANGE_G, COLOR_ORANGE_R,
    COLOR_PURPLE_B, COLOR_PURPLE_G, COLOR_PURPLE_R, Corner, clip_rounded, draw_corner_badge,
};
use crate::ui::file_checks::does_file_type_match;
use crate::ui::mixins::upload_common::{name_or_default, wire_file_drop_target};
use crate::ui::widgets::{CustomTextbox, NumberSpinner, RangeSlider};
use c7_core::audio_playback::PreviewPlayer;
use c7_core::c7_file_interfacing::{expand_c7_to_sds, read_c7_or_sysex_file};
use c7_core::device_config::{DeviceConfig, get_export_folder};
use c7_core::dsp_utils::{generate_default_kick, load_to_i16, normalize_i16, resample_i16};
use c7_core::midi::{is_valid_port, run_midi_session};
use c7_core::sds::{
    SDS_LOOP_OFF, audio_to_sds, build_sds_sample_packets, resolve_upload_bit_depth, seek_sds_slot, send_sds_sample_packets, write_wav,
};
use c7_core::sysex::filter_elektron_name;
use c7_core::utils::{AUDIO_EXTS, as_u64_or_die};

/// Marker line thickness. Matches `selected_line_width` in `upload_digipro`.
const MARKER_W: f64 = 3.0;

/// Drawing state for the sample display.
///
/// Holds the waveform, its trim and loop points, and a cached render.
struct SampleDisplayState {
    samples: Vec<i16>,
    start: f64,
    end: f64,
    loopdata_start: Option<f64>,
    loopdata_end: Option<f64>,
    playhead: Option<f64>,
    sample_rate: u32,
    upload_rate: u32,
    upload_bit_depth: u8,
    waveform_cache: RefCell<Option<(i32, i32, cairo::ImageSurface)>>,
}

impl Default for SampleDisplayState {
    /// Returns the default state before a sample is loaded: no data, full-range trim, no loop/playhead.
    fn default() -> Self {
        SampleDisplayState {
            samples: Vec::new(),
            start: 0.0,
            end: 1.0,
            loopdata_start: None,
            loopdata_end: None,
            playhead: None,
            sample_rate: 0,
            upload_rate: 0,
            upload_bit_depth: 16,
            waveform_cache: RefCell::new(None),
        }
    }
}

/// Passive sample visualizer.
///
/// Interaction is handled by the `RangeSliders` below.
/// Call `set_samples()`, `set_trim()`, and `set_loop()` to update the display.
#[derive(Clone)]
struct SampleDisplay {
    drawing: DrawingArea,
    state: Rc<RefCell<SampleDisplayState>>,
    /// Value label in the sample-rate slider header. `set_upload_rate()` updates it, so loads and drags stay in sync.
    rate_label: Rc<RefCell<Option<gtk4::Label>>>,
}

impl SampleDisplay {
    /// Creates a new instance of `SampleDisplay`.
    pub(crate) fn new() -> Self {
        let drawing = DrawingArea::new();
        drawing.set_hexpand(true);
        let state = Rc::new(RefCell::new(SampleDisplayState::default()));
        {
            let state_c = Rc::clone(&state);
            drawing.set_draw_func(move |_, cr, w, h| draw_sample(cr, w, h, &state_c.borrow()));
        }
        SampleDisplay {
            drawing,
            state,
            rate_label: Rc::new(RefCell::new(None)),
        }
    }

    /// Attaches the sample-rate slider-header value label, which `set_upload_rate()` updates.
    fn set_rate_label(&self, rate_label: gtk4::Label) {
        *self.rate_label.borrow_mut() = Some(rate_label);
    }

    /// Updates the buffer of samples to be visualized.
    fn set_samples(&self, samples: Vec<i16>) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.samples = samples;
        state_mut.start = 0.0;
        state_mut.end = 1.0;
        state_mut.loopdata_start = None;
        state_mut.loopdata_end = None;
        state_mut.waveform_cache.replace(None);
        drop(state_mut);
        self.drawing.queue_draw();
    }

    /// Updates the playback trim range [start, end].
    pub(crate) fn set_trim(&self, start: f64, end: f64) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.start = start;
        state_mut.end = end;
        state_mut.waveform_cache.replace(None);
        drop(state_mut);
        self.drawing.queue_draw();
    }

    /// Updates the loop points [`loopdata_start`, `loopdata_end`].
    fn set_loop(&self, loopdata_start: Option<f64>, loopdata_end: Option<f64>) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.loopdata_start = loopdata_start;
        state_mut.loopdata_end = loopdata_end;
        state_mut.waveform_cache.replace(None);
        drop(state_mut);
        self.drawing.queue_draw();
    }

    /// Updates the live preview-playback position, or hides the playhead marker when `None`.
    fn set_playhead(&self, pos: Option<f64>) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.playhead = pos;
        drop(state_mut);
        self.drawing.queue_draw();
    }

    /// Sets the source sample rate used for the trimmed-duration badge.
    fn set_sample_rate(&self, rate: u32) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.sample_rate = rate;
        state_mut.waveform_cache.replace(None);
        drop(state_mut);
        self.drawing.queue_draw();
    }

    /// Sets the user-selected upload rate, shown in the sample-rate slider header.
    fn set_upload_rate(&self, rate: u32) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.upload_rate = rate;
        state_mut.waveform_cache.replace(None);
        drop(state_mut);
        if let Some(rate_label) = self.rate_label.borrow().as_ref() {
            rate_label.set_text(&format_khz(rate));
        }
        self.drawing.queue_draw();
    }

    /// Sets the upload bit depth used by the export-size badge.
    fn set_upload_bit_depth(&self, bits: u8) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.upload_bit_depth = bits;
        state_mut.waveform_cache.replace(None);
        drop(state_mut);
        self.drawing.queue_draw();
    }

    /// Returns the root `DrawingArea` widget.
    pub(crate) fn widget(&self) -> &DrawingArea {
        &self.drawing
    }
}

impl Default for SampleDisplay {
    /// Returns a new instance of `SampleDisplay` with default settings.
    fn default() -> Self {
        Self::new()
    }
}

// ── `SampleDisplay` drawing ────────────────────────────────────────────────────────────────────────────────

/// Renders the sample display.
fn draw_sample(cr: &cairo::Context, width: i32, height: i32, state_ref: &SampleDisplayState) {
    if width < 2 || height < 2 {
        return;
    }
    let width_f64 = f64::from(width);
    let height_f64 = f64::from(height);

    let cache_hit = matches!(*state_ref.waveform_cache.borrow(), Some((cached_width, cached_height, _)) if cached_width == width && cached_height == height);
    if !cache_hit && let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height) {
        if let Ok(scr) = cairo::Context::new(&surface) {
            draw_waveform_static(&scr, width, height, state_ref);
        }
        state_ref.waveform_cache.replace(Some((width, height, surface)));
    }

    if let Some((_, _, surface)) = state_ref.waveform_cache.borrow().as_ref() {
        cr.set_source_surface(surface, 0.0, 0.0).unwrap();
        cr.paint().unwrap();
    }

    // Preview-playback tape head (White).
    // Drawn last so it's never hidden behind the others.
    if let Some(playhead_pos) = state_ref.playhead {
        clip_rounded(cr, width_f64, height_f64);
        draw_vertical_marker(cr, playhead_pos, width_f64, height_f64, 1.0, 1.0, 1.0);
    }
}

/// Renders everything in the sample display except the playhead.
///
/// This is the static raster cached by `draw_sample()`.
fn draw_waveform_static(cr: &cairo::Context, width: i32, height: i32, state_ref: &SampleDisplayState) {
    clip_rounded(cr, f64::from(width), f64::from(height));
    cr.set_source_rgb(0.0, 0.0, 0.0);
    let _ = cr.paint();

    let width_f64 = f64::from(width);
    let height_f64 = f64::from(height);

    // The drawn waveform reflects the chosen upload rate, so dropping the rate visibly coarsens the peaks.
    // Borrows the source samples directly when no downsampling is needed (full quality).
    let resampled;
    let base: &[i16] = if state_ref.upload_rate == state_ref.sample_rate {
        &state_ref.samples
    } else {
        resampled = resample_i16(&state_ref.samples, state_ref.sample_rate, state_ref.upload_rate);
        &resampled
    };

    // The waveform also reflects the chosen bit depth, by quantizing to the top `upload_bit_depth` bits (what `encode_sds()` keeps).
    // For full-scale material the peak envelope barely changes. Depth mostly shows in the badge and the audible preview.
    // But quieter passages visibly coarsen, and the redraw keeps the badge in sync.
    let quantized;

    let display_samples: &[i16] = if state_ref.upload_bit_depth < 16 {
        let shift = u32::from(16 - state_ref.upload_bit_depth);
        quantized = base
            .iter()
            .map(|&sample| ((i32::from(sample) >> shift) << shift) as i16)
            .collect::<Vec<i16>>();
        &quantized
    } else {
        base
    };

    let num_display_samples = display_samples.len();

    // Playback-region tint (Blue)
    cr.set_source_rgba(COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B, 0.07);
    cr.rectangle(
        state_ref.start * width_f64,
        0.0,
        (state_ref.end - state_ref.start) * width_f64,
        height_f64,
    );
    let _ = cr.fill();

    // Loop-region tint (Green)
    if let (Some(loopdata_start), Some(loopdata_end)) = (state_ref.loopdata_start, state_ref.loopdata_end) {
        cr.set_source_rgba(COLOR_GREEN_R, COLOR_GREEN_G, COLOR_GREEN_B, 0.12);
        cr.rectangle(
            loopdata_start * width_f64,
            0.0,
            (loopdata_end - loopdata_start) * width_f64,
            height_f64,
        );
        let _ = cr.fill();
    }

    // Sample peaks (Purple)
    if num_display_samples > 0 {
        cr.set_source_rgba(COLOR_PURPLE_R, COLOR_PURPLE_G, COLOR_PURPLE_B, 1.0);
        let cy = height_f64 * 0.5;
        for px in 0..width {
            let i0 = ((f64::from(px) / width_f64) * num_display_samples as f64) as usize;
            let i1 = ((f64::from(px + 1) / width_f64) * num_display_samples as f64) as usize;
            let i1 = i1.clamp(i0 + 1, num_display_samples);
            let peak = display_samples[i0..i1]
                .iter()
                .map(|&sample| f64::from(sample.unsigned_abs()) / 32768.0)
                .fold(0.0f64, f64::max);
            let half = (peak * cy).max(0.5);
            cr.rectangle(f64::from(px), cy - half, 1.0, half * 2.0);
        }
        let _ = cr.fill();
    }

    // Vertical marker lines
    draw_vertical_marker(cr, state_ref.start, width_f64, height_f64, COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B);
    draw_vertical_marker(cr, state_ref.end, width_f64, height_f64, COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B);
    if let (Some(loopdata_start), Some(loopdata_end)) = (state_ref.loopdata_start, state_ref.loopdata_end) {
        draw_vertical_marker(
            cr,
            loopdata_start,
            width_f64,
            height_f64,
            COLOR_GREEN_R,
            COLOR_GREEN_G,
            COLOR_GREEN_B,
        );
        draw_vertical_marker(cr, loopdata_end, width_f64, height_f64, COLOR_GREEN_R, COLOR_GREEN_G, COLOR_GREEN_B);
    }

    // Trimmed-length badge (bottom-right) and export-size badge (top-right).
    //
    // `num_display_samples` is the resampled (display) length, so the duration divides by `upload_rate` to stay correct.
    if num_display_samples > 0 {
        // `start_idx.min(num_display_samples - 1)` keeps `start_idx + 1` from exceeding `num_display_samples`.
        // Otherwise the clamp below would panic (its min would exceed its max) when the trim handle is dragged to the right edge.
        let start_idx = ((state_ref.start * num_display_samples as f64).round() as usize).min(num_display_samples - 1);
        let end_idx = ((state_ref.end * num_display_samples as f64).round() as usize).clamp(start_idx + 1, num_display_samples);
        let trimmed = end_idx - start_idx;
        let duration_secs = trimmed as f64 / f64::from(state_ref.upload_rate);
        draw_corner_badge(cr, width_f64, height_f64, Corner::BottomRight, &format!("{duration_secs:.2}s"));

        // Export (SDS upload) size: the trimmed samples packed at the chosen bit depth's bytes/word.
        // This is the size actually sent to the device.
        // Tracks both sliders, and shows why e.g. 12- and 14-bit land on the same size (both 2 bytes/word).
        let bytes_per_word = (state_ref.upload_bit_depth as usize).div_ceil(7); // ceil(bits/7) MIDI bytes per sample word
        let samples_per_packet = (120 / bytes_per_word).max(1);
        let data_packets = trimmed.div_ceil(samples_per_packet);
        let sds_bytes = 21 + data_packets * 127; // 21-byte Dump Header + 127 bytes/data packet
        draw_corner_badge(cr, width_f64, height_f64, Corner::TopRight, &format_kb(sds_bytes));
    }
}

/// Draws a vertical marker line in the sample visualizer.
fn draw_vertical_marker(cr: &cairo::Context, pos: f64, width: f64, height: f64, red: f64, green: f64, blue: f64) {
    let x = pos * width;
    cr.set_source_rgba(red, green, blue, 0.9);
    cr.set_line_width(MARKER_W);
    cr.move_to(x, 0.0);
    cr.line_to(x, height);
    let _ = cr.stroke();
}

/// Like `.clamp(lo, hi)`, but doesn't panic if `lo > hi` (swaps them first instead).
///
/// Needed wherever the bounds come from two independently dragged sliders.
/// A fast slider drag can report a position inconsistent with the other slider's last-known bounds.
/// That would otherwise invert `lo`/`hi` and panic.
fn clamp_safe(val: f64, lo: f64, hi: f64) -> f64 {
    if lo <= hi { val.clamp(lo, hi) } else { val.clamp(hi, lo) }
}

/// Resamples the full (uncropped) source to the chosen output rate for preview playback, normalizing if requested.
///
/// Trim, loop, and the bit-depth quantization are applied live by the player, on the final resampled buffer.
/// As a result, the preview buffer is never cropped, and the bit depth can't be smoothed out by a later resample.
fn build_preview_buffer(samples: &[i16], src_rate: u32, upload_rate: u32, should_normalize: bool) -> Vec<i16> {
    let mut buffer = resample_i16(samples, src_rate, upload_rate);
    if should_normalize {
        normalize_i16(&mut buffer);
    }
    buffer
}

// ── Internal state ─────────────────────────────────────────────────────────────────────────────────────────

struct UploadSampleState {
    /// Held at `src_rate`, then resampled to `upload_rate` at build time.
    samples: Vec<i16>,
    src_rate: u32,
    upload_rate: u32,
    upload_bit_depth: u8,
    trim_start: f64,
    trim_end: f64,
    loopdata_start: Option<f64>,
    loopdata_end: Option<f64>,
    dc: Arc<DeviceConfig>,
}

/// Feature screen for uploading audio samples to Machinedrum slots via SDS.
pub(crate) struct UploadSampleScreen {
    pub root: gtk4::Box,
}

impl UploadSampleScreen {
    /// Initializes the sample upload screen.
    pub(crate) fn new(
        go_to_menu: impl Fn() + 'static,
        get_selected_midi: impl Fn() -> Option<String> + 'static,
        delay_spin: &NumberSpinner,
        active_config: &str,
    ) -> Self {
        let mut base = BaseModule::new();
        // Without this, returning here via the nav stack (e.g. after visiting MIDI Monitor) leaves focus wherever it last was.
        // The spacebar/Escape `EventControllerKey` below then stops receiving events until the user clicks something in this screen first.
        // Matches the same fix already in `kit_editor.rs`.
        base.root.set_focusable(true);
        base.root.connect_map(|root_mapped| {
            root_mapped.grab_focus();
        });

        // Lock the shared delay spinner to show "AUTO" while active, via `NumberSpinner::delay_lifecycle`.
        // `receive_sample.rs` uses this same mechanism.
        let (save_lock_delay, restore_delay) = delay_spin.delay_lifecycle();
        base.root.connect_map(move |_| save_lock_delay());
        base.root.connect_unmap(move |_| restore_delay());

        let dc = Arc::new(DeviceConfig::find_by_path_or_die(active_config));
        let device_short = dc.device_short.clone();
        let device_shorter = dc.device_shorter.clone();
        let max_slots = as_u64_or_die(dc.json_get("sysex_api.sample.slots")) as f64;

        // Sampler spec from the device JSON (required). Every device with this screen must declare it.
        let lowest_sample_rate_hz = as_u64_or_die(dc.json_get("sampler.lowest_sample_rate_hz")) as u32;
        let highest_sample_rate_hz = as_u64_or_die(dc.json_get("sampler.highest_sample_rate_hz")) as u32;
        let bit_depth = resolve_upload_bit_depth(&dc);

        // ── Header ─────────────────────────────────────────────────────────────────────────────────────────

        let header = base.build_header("Upload Sample", go_to_menu);

        let preview_btn = Button::with_label("Preview");
        // Wrapped in its own box because `GestureClick`::released doesn't reliably fire when attached directly to a `GtkButton`.
        // The button's own internal click gesture claims the press sequence first.
        //
        // Attaching to this wrapper, with a Capture propagation phase, lets this gesture see press/release first.
        // The button's own gesture claims it after.
        let preview_wrap = GtkBox::new(Orientation::Horizontal, 0);
        preview_wrap.append(&preview_btn);
        header.end.append(&preview_wrap);

        // ── Sample display ─────────────────────────────────────────────────────────────────────────────────

        let display = SampleDisplay::new();
        display.widget().set_size_request(-1, 200);
        base.root.append(display.widget());

        // ── Trim slider (two-thumb, blue) ──────────────────────────────────────────────────────────────────
        // Header is title-only, with no value shown on the right.

        let trim_slider = RangeSlider::new();
        trim_slider.set_multi(true);
        trim_slider.set_accent_color(COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B);
        trim_slider.set_thumbs(0.0, 1.0);
        trim_slider.widget().set_sensitive(false);
        let (trim_col, _) = build_labeled_slider("Trim", &trim_slider, false);
        base.root.append(&trim_col);

        // ── Sample-rate slider (single-thumb, orange) ──────────────────────────────────────────────────────

        // Fraction maps to a Hz value spanning the device's sampler rate range (capped at `src_rate`) via `slider_pos_to_rate()`.
        let rate_slider = RangeSlider::new();
        rate_slider.set_accent_color(COLOR_ORANGE_R, COLOR_ORANGE_G, COLOR_ORANGE_B);
        rate_slider.set_pos(1.0);
        rate_slider.widget().set_sensitive(false);
        let (rate_col, rate_val) = build_labeled_slider("Sample rate", &rate_slider, true);
        // Nothing to drag on a device whose sampler accepts a single rate.
        if lowest_sample_rate_hz == highest_sample_rate_hz {
            rate_col.set_visible(false);
        }
        base.root.append(&rate_col);

        // `set_upload_rate()` keeps this header value in sync (load and drag alike).
        if let Some(rate_val) = rate_val {
            display.set_rate_label(rate_val);
        }

        // ── Loop section ───────────────────────────────────────────────────────────────────────────────────

        let loop_box = GtkBox::new(Orientation::Vertical, 2);

        // Two-thumb loop range (like trim), title-only header. The whole column is hidden until "Enable Looping" is on.
        let loopdata_slider = RangeSlider::new();
        loopdata_slider.set_multi(true);
        loopdata_slider.set_thumbs(0.0, 1.0);
        let (loop_col, _) = build_labeled_slider("Loop", &loopdata_slider, false);
        loop_col.set_visible(false);
        loop_box.append(&loop_col);

        let checks_row = GtkBox::new(Orientation::Horizontal, 8);
        checks_row.set_margin_start(4);
        checks_row.set_margin_top(4);

        let loop_check = CheckButton::with_label("Enable Looping");
        loop_check.set_sensitive(false);
        checks_row.append(&loop_check);

        let norm_check = CheckButton::with_label("Maximize Volume");
        norm_check.set_active(false);
        checks_row.append(&norm_check);

        loop_box.append(&checks_row);

        base.root.append(&loop_box);

        // ── Shared state handles ───────────────────────────────────────────────────────────────────────────

        let state: Rc<RefCell<Option<UploadSampleState>>> = Rc::new(RefCell::new(None));
        let preview_player: Rc<RefCell<Option<PreviewPlayer>>> = Rc::new(RefCell::new(None));

        // ── Slider / checkbox wiring ───────────────────────────────────────────────────────────────────────

        {
            let display_c = display.clone();
            let state_c = Rc::clone(&state);
            let preview_player_c = Rc::clone(&preview_player);
            norm_check.connect_toggled(move |btn| {
                let should_normalize = btn.is_active();

                {
                    let borrow = state_c.borrow();
                    if let Some(state_ref) = borrow.as_ref() {
                        let mut disp_samples = state_ref.samples.clone();
                        if should_normalize {
                            normalize_i16(&mut disp_samples);
                        }
                        display_c.set_samples(disp_samples);
                        display_c.set_trim(state_ref.trim_start, state_ref.trim_end);
                        display_c.set_loop(state_ref.loopdata_start, state_ref.loopdata_end);
                    }
                }

                // If a preview is playing live, rebuild with the new normalization setting and swap it in.
                let playing = preview_player_c
                    .borrow()
                    .as_ref()
                    .is_some_and(|player| player.position_frac().is_some());

                if playing {
                    let snap = state_c
                        .borrow()
                        .as_ref()
                        .filter(|state_ref| !state_ref.samples.is_empty())
                        .map(|state_ref| {
                            (
                                build_preview_buffer(&state_ref.samples, state_ref.src_rate, state_ref.upload_rate, should_normalize),
                                state_ref.upload_rate,
                                state_ref.upload_bit_depth,
                            )
                        });

                    if let Some(preview_ref) = preview_player_c.borrow().as_ref()
                        && let Some((buffer, rate, bits)) = snap
                    {
                        preview_ref.resample_live(&buffer, f64::from(rate), bits);
                    }
                }
            });
        }
        {
            let display_c = display.clone();
            let state_c = Rc::clone(&state);
            let loopdata_slider_c = loopdata_slider.clone();
            let loop_check_c = loop_check.clone();
            let preview_player_c = Rc::clone(&preview_player);
            trim_slider.connect_range_changed(move |start, end| {
                display_c.set_trim(start, end);
                // Push the new trim to the player live (fractions of the whole sample), so the audio callback applies it next block.
                if let Some(preview_ref) = preview_player_c.borrow().as_ref() {
                    preview_ref.set_trim(start, end);
                }
                // Clamp loop points to stay within the new trim bounds.
                if loop_check_c.is_active() {
                    let clamped_start = clamp_safe(loopdata_slider_c.start(), start, loopdata_slider_c.end().min(end));
                    let clamped_end = clamp_safe(loopdata_slider_c.end(), clamped_start.max(start), end);
                    loopdata_slider_c.set_thumbs(clamped_start, clamped_end);
                    display_c.set_loop(Some(clamped_start), Some(clamped_end));
                    if let Some(preview_ref) = preview_player_c.borrow().as_ref() {
                        preview_ref.set_loop_region(clamped_start, clamped_end);
                    }
                    if let Some(state_mut) = state_c.borrow_mut().as_mut() {
                        state_mut.trim_start = start;
                        state_mut.trim_end = end;
                        state_mut.loopdata_start = Some(clamped_start);
                        state_mut.loopdata_end = Some(clamped_end);
                    }
                } else if let Some(state_mut) = state_c.borrow_mut().as_mut() {
                    state_mut.trim_start = start;
                    state_mut.trim_end = end;
                }
            });
        }
        {
            let display_c = display.clone();
            let state_c = Rc::clone(&state);
            let loopdata_slider_c = loopdata_slider.clone();
            let preview_player_c = Rc::clone(&preview_player);
            loopdata_slider.connect_range_changed(move |start, end| {
                // Clamp loop points to the current trim bounds before committing.
                let (trim_start, trim_end) = state_c
                    .borrow()
                    .as_ref()
                    .map_or((0.0, 1.0), |state_ref| (state_ref.trim_start, state_ref.trim_end));
                let clamped_start = clamp_safe(start, trim_start, end.min(trim_end));
                let clamped_end = clamp_safe(end, clamped_start.max(trim_start), trim_end);
                if (clamped_start - start).abs() > 1e-6 || (clamped_end - end).abs() > 1e-6 {
                    loopdata_slider_c.set_thumbs(clamped_start, clamped_end);
                }
                display_c.set_loop(Some(clamped_start), Some(clamped_end));
                if let Some(preview_ref) = preview_player_c.borrow().as_ref() {
                    preview_ref.set_loop_region(clamped_start, clamped_end);
                }
                if let Some(state_mut) = state_c.borrow_mut().as_mut() {
                    state_mut.loopdata_start = Some(clamped_start);
                    state_mut.loopdata_end = Some(clamped_end);
                }
            });
        }
        {
            let loopdata_slider_c = loopdata_slider.clone();
            let display_c = display.clone();
            let state_c = Rc::clone(&state);
            loop_check.connect_toggled(move |btn| {
                let active = btn.is_active();
                loop_col.set_visible(active);
                if active {
                    let (trim_start, trim_end) = state_c
                        .borrow()
                        .as_ref()
                        .map_or((0.0, 1.0), |state_ref| (state_ref.trim_start, state_ref.trim_end));
                    let (slider_start, slider_end) = (loopdata_slider_c.start(), loopdata_slider_c.end());
                    let clamped_start = clamp_safe(slider_start, trim_start, slider_end.min(trim_end));
                    let clamped_end = clamp_safe(slider_end, clamped_start.max(trim_start), trim_end);
                    if (clamped_start - slider_start).abs() > 1e-6 || (clamped_end - slider_end).abs() > 1e-6 {
                        loopdata_slider_c.set_thumbs(clamped_start, clamped_end);
                    }
                    display_c.set_loop(Some(clamped_start), Some(clamped_end));
                    if let Some(state_mut) = state_c.borrow_mut().as_mut() {
                        state_mut.loopdata_start = Some(clamped_start);
                        state_mut.loopdata_end = Some(clamped_end);
                    }
                } else {
                    display_c.set_loop(None, None);
                    if let Some(state_mut) = state_c.borrow_mut().as_mut() {
                        state_mut.loopdata_start = None;
                        state_mut.loopdata_end = None;
                    }
                }
            });
        }

        // ── Sample-rate slider wiring ──────────────────────────────────────────────────────────────────────

        {
            let state_c = Rc::clone(&state);
            let display_c = display.clone();
            let preview_player_c = Rc::clone(&preview_player);
            let norm_check_c = norm_check.clone();
            rate_slider.connect_pos_changed(move |pos| {
                let src_rate = state_c.borrow().as_ref().map(|state_ref| state_ref.src_rate);
                if let Some(src_rate) = src_rate {
                    let hz = slider_pos_to_rate(pos, src_rate, lowest_sample_rate_hz, highest_sample_rate_hz);
                    // Snapping means hz only changes when the drag crosses a 0.1 kHz boundary, which throttles the buffer swaps below.
                    let changed = state_c.borrow().as_ref().is_some_and(|state_ref| state_ref.upload_rate != hz);
                    if let Some(state_mut) = state_c.borrow_mut().as_mut() {
                        state_mut.upload_rate = hz;
                    }
                    display_c.set_upload_rate(hz);

                    // If a preview is already playing, swap to the new rate without restarting.
                    // Trim and loop are live params, so only the buffer changes.
                    let playing = preview_player_c
                        .borrow()
                        .as_ref()
                        .is_some_and(|player| player.position_frac().is_some());
                    if changed && playing {
                        let snap = state_c
                            .borrow()
                            .as_ref()
                            .filter(|state_ref| !state_ref.samples.is_empty())
                            .map(|state_ref| {
                                (
                                    build_preview_buffer(
                                        &state_ref.samples,
                                        state_ref.src_rate,
                                        state_ref.upload_rate,
                                        norm_check_c.is_active(),
                                    ),
                                    state_ref.upload_bit_depth,
                                )
                            });

                        if let Some(preview_ref) = preview_player_c.borrow().as_ref()
                            && let Some((buffer, bits)) = snap
                        {
                            preview_ref.resample_live(&buffer, f64::from(hz), bits);
                        }
                    }
                }
            });
        }

        // ── Instructions card ──────────────────────────────────────────────────────────────────────────────

        base.build_instructions_card(
            &[
                &format!("1. On the {device_short}, navigate to GLOBAL > FILE > SAMPLE MGR."),
                "2. Change the Mode to RECV, scroll up to ORG, and press YES.",
                &format!("3. The {device_shorter} screen should now flash \"WAITING\"."),
                "4. Load a file below, adjust trim/loop markers, and select the target slot.",
                "5. Click 'Send to Device' to begin the transfer.",
            ],
            "Device Prep",
        );

        // ── Controls row ───────────────────────────────────────────────────────────────────────────────────

        let control_row = GtkBox::builder().spacing(8).build();

        let load_btn = Button::with_label("Load from File...");
        control_row.append(&load_btn);

        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        control_row.append(&spacer);

        let slot_label = gtk4::Label::builder().label("Slot:").valign(Align::Center).build();
        let slot_spin = NumberSpinner::new(1.0, 1.0, max_slots, 1.0, "", 0);

        let name_label = gtk4::Label::builder().label("Name:").valign(Align::Center).build();
        let name_length = as_u64_or_die(dc.json_get("sysex_api.sample.name_length")) as i32;
        let name_entry = CustomTextbox::new();
        name_entry.enable_elektron_filter();
        name_entry.set_max_length(name_length);
        name_entry.set_text("SMPL");
        name_entry.set_width_chars(name_length);
        name_entry.set_max_width_chars(name_length + 1);
        name_entry.set_valign(Align::Center);
        name_entry.set_tooltip_text(Some(&format!("{name_length}-character name stored on the {device_short}")));

        let save_btn = Button::with_label("Save to File");
        save_btn.set_valign(Align::Center);
        save_btn.set_sensitive(false);

        let send_btn = Button::with_label("Send to Device");
        send_btn.add_css_class("suggested-action");
        send_btn.set_valign(Align::Center);
        send_btn.set_sensitive(false);

        control_row.append(&slot_label);
        control_row.append(slot_spin.widget());
        control_row.append(&name_label);
        control_row.append(&*name_entry);
        control_row.append(&save_btn);
        control_row.append(&send_btn);
        base.root.append(&control_row);

        base.build_status_area(true);
        let update_status = base.status_updater();
        let update_progress = base.progress_updater();

        // ── Preview button ─────────────────────────────────────────────────────────────────────────────────
        // Player opens lazily on first press and stays open for the screen's lifetime.
        // Clicking the button plays the full oneshot. Holding the button loops until release, then plays the remainder.

        let is_preview_timer_active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let preview_press: Rc<dyn Fn()> = {
            let state_c = Rc::clone(&state);
            let preview_player_c = Rc::clone(&preview_player);
            let norm_check_c = norm_check.clone();
            let display_c = display.clone();
            let update_status_c = Arc::clone(&update_status);
            let preview_timer_active_c = Rc::clone(&is_preview_timer_active);
            Rc::new(move || {
                // Build the full (uncropped) preview buffer at the chosen rate.
                // Trim and loop are applied live by the player as fractions of the whole sample.
                let Some((buffer, upload_rate, bits, trim_start, trim_end, loopdata_start, loopdata_end, is_loop_active)) = ({
                    let borrow = state_c.borrow();
                    borrow.as_ref().filter(|state_ref| !state_ref.samples.is_empty()).map(|state_ref| {
                        let buffer = build_preview_buffer(
                            &state_ref.samples,
                            state_ref.src_rate,
                            state_ref.upload_rate,
                            norm_check_c.is_active(),
                        );
                        let (loopdata_start, loopdata_end, is_loop_active) = match (state_ref.loopdata_start, state_ref.loopdata_end) {
                            (Some(loopdata_start), Some(loopdata_end)) => (loopdata_start, loopdata_end, true),
                            _ => (0.0, 1.0, false),
                        };
                        (
                            buffer,
                            state_ref.upload_rate,
                            state_ref.upload_bit_depth,
                            state_ref.trim_start,
                            state_ref.trim_end,
                            loopdata_start,
                            loopdata_end,
                            is_loop_active,
                        )
                    })
                }) else {
                    return;
                };

                if preview_player_c.borrow().is_none() {
                    match PreviewPlayer::new() {
                        Ok(player) => *preview_player_c.borrow_mut() = Some(player),
                        Err(e) => {
                            update_status_c(&format!("Error: Preview failed: {e}"));
                            return;
                        }
                    }
                }

                if let Some(preview_ref) = preview_player_c.borrow().as_ref() {
                    preview_ref.start(
                        &buffer,
                        f64::from(upload_rate),
                        trim_start,
                        trim_end,
                        loopdata_start,
                        loopdata_end,
                        is_loop_active,
                        bits,
                    );
                }

                if !preview_timer_active_c.get() {
                    preview_timer_active_c.set(true);

                    // Synced to the widget's frame clock, not a fixed-interval timer.
                    // The playhead moves at whatever rate the display redraws.
                    //
                    // `position_frac()` is already a fraction of the whole (uncropped) sample, so the marker maps directly with no remap.
                    display_c.widget().add_tick_callback({
                        let preview_player_c2 = Rc::clone(&preview_player_c);
                        let display_c2 = display_c.clone();
                        let preview_timer_active_c2 = Rc::clone(&preview_timer_active_c);
                        move |_widget, _frame_clock| {
                            let frac = preview_player_c2.borrow().as_ref().and_then(PreviewPlayer::position_frac);
                            display_c2.set_playhead(frac);
                            if frac.is_some() {
                                glib::ControlFlow::Continue
                            } else {
                                preview_timer_active_c2.set(false);
                                glib::ControlFlow::Break
                            }
                        }
                    });
                }
            })
        };
        let preview_release: Rc<dyn Fn()> = {
            let preview_player_c = Rc::clone(&preview_player);
            Rc::new(move || {
                if let Some(preview_ref) = preview_player_c.borrow().as_ref() {
                    preview_ref.release_loop();
                }
            })
        };
        // Unlike `preview_release`, cancels immediately rather than letting the current pass finish.
        // It also clears the playhead right away instead of waiting for the next redraw tick.
        let preview_cancel: Rc<dyn Fn()> = {
            let preview_player_c = Rc::clone(&preview_player);
            let display_c = display.clone();
            Rc::new(move || {
                if let Some(preview_ref) = preview_player_c.borrow().as_ref() {
                    preview_ref.stop();
                }
                display_c.set_playhead(None);
            })
        };

        {
            let preview_press_c = Rc::clone(&preview_press);
            let preview_release_c = Rc::clone(&preview_release);
            let click = GestureClick::new();
            click.set_propagation_phase(gtk4::PropagationPhase::Capture);
            click.connect_pressed(move |_, _n, _x, _y| preview_press_c());
            click.connect_released(move |_, _n, _x, _y| preview_release_c());
            preview_wrap.add_controller(click);
        }

        // ── Spacebar = Preview ─────────────────────────────────────────────────────────────────────────────

        // Same press/hold-to-loop/release-to-finish logic as the mouse button.
        // `kit_editor.rs` maps the spacebar to its Trig button the same way.
        //
        // `is_space_held` dedupes the OS's key auto-repeat (repeated "pressed" events while held).
        // A held key fires one press, not one per tick.
        let is_space_held: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        {
            let key_controller = EventControllerKey::new();
            key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
            {
                let is_space_held_c = Rc::clone(&is_space_held);
                let preview_press_c = Rc::clone(&preview_press);
                let preview_cancel_c = Rc::clone(&preview_cancel);
                let preview_btn_c = preview_btn.clone();
                key_controller.connect_key_pressed(move |_, keyval, _, _| {
                    if keyval == gdk::Key::space {
                        if !is_space_held_c.get() {
                            is_space_held_c.set(true);
                            preview_btn_c.set_state_flags(gtk4::StateFlags::ACTIVE, false);
                            preview_press_c();
                        }
                        return glib::Propagation::Stop;
                    }
                    if keyval == gdk::Key::Escape {
                        // Cancels regardless of whether the hold was via spacebar or mouse.
                        // Clear the spacebar-held state too so the eventual physical key-release does nothing.
                        is_space_held_c.set(false);
                        preview_btn_c.unset_state_flags(gtk4::StateFlags::ACTIVE);
                        preview_cancel_c();
                        return glib::Propagation::Stop;
                    }
                    glib::Propagation::Proceed
                });
            }
            {
                let is_space_held_c = Rc::clone(&is_space_held);
                let preview_release_c = Rc::clone(&preview_release);
                key_controller.connect_key_released(move |_, keyval, _, _| {
                    if keyval == gdk::Key::space {
                        is_space_held_c.set(false);
                        preview_btn.unset_state_flags(gtk4::StateFlags::ACTIVE);
                        preview_release_c();
                    }
                });
            }
            base.root.add_controller(key_controller);
        }

        // ── Load callback ──────────────────────────────────────────────────────────────────────────────────

        let on_load: Rc<dyn Fn(String)> = {
            let state_c = Rc::clone(&state);
            let display_c = display.clone();
            let trim_slider_c = trim_slider.clone();
            let rate_slider_c = rate_slider.clone();
            let loop_check_c = loop_check.clone();
            let norm_check_c = norm_check.clone();
            let save_btn_c = save_btn.clone();
            let send_btn_c = send_btn.clone();
            let name_entry_c = name_entry.clone();
            let update_status_c = Arc::clone(&update_status);
            let dc_c = Arc::clone(&dc);
            Rc::new(move |path: String| {
                dispatch_load(
                    &path,
                    &dc_c,
                    &state_c,
                    &display_c,
                    &trim_slider_c,
                    &loopdata_slider,
                    &rate_slider_c,
                    &loop_check_c,
                    &norm_check_c,
                    &save_btn_c,
                    &send_btn_c,
                    &name_entry_c,
                    &update_status_c,
                );
            })
        };

        // ── Drop target ────────────────────────────────────────────────────────────────────────────────────

        {
            let on_load_c = Rc::clone(&on_load);
            wire_file_drop_target(&base.root, move |path| on_load_c(path));
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
            let state_c = Rc::clone(&state);
            let name_entry_c = name_entry.clone();
            let update_status_c = Arc::clone(&update_status);
            let root_c = base.root.clone();
            let norm_check_c = norm_check.clone();
            save_btn.connect_clicked(move |_| {
                on_save_clicked(
                    &root_c,
                    &state_c,
                    &name_entry_c,
                    &update_status_c,
                    norm_check_c.is_active(),
                    name_length as usize,
                );
            });
        }

        // ── Send button ────────────────────────────────────────────────────────────────────────────────────

        {
            let get_midi_rc = Rc::new(get_selected_midi) as Rc<dyn Fn() -> Option<String>>;
            let state_c = Rc::clone(&state);
            let name_entry_c = name_entry.clone();
            let update_status_c = Arc::clone(&update_status);
            let update_progress_c = Arc::clone(&update_progress);
            let send_btn_c = send_btn.clone();
            let norm_check_c = norm_check.clone();
            send_btn.connect_clicked(move |_| {
                on_send_clicked(
                    &state_c,
                    &name_entry_c,
                    &slot_spin,
                    &update_status_c,
                    &update_progress_c,
                    &send_btn_c,
                    &get_midi_rc,
                    norm_check_c.is_active(),
                    name_length as usize,
                );
            });
        }

        // ── Default Kick Sample ────────────────────────────────────────────────────────────────────────────

        // Synthesized at the device's highest accepted rate, so that's its source rate and the full-quality default.
        let kick = generate_default_kick(highest_sample_rate_hz);
        display.set_samples(kick.clone());
        display.set_sample_rate(highest_sample_rate_hz);
        display.set_upload_rate(highest_sample_rate_hz);
        display.set_upload_bit_depth(bit_depth);
        *state.borrow_mut() = Some(UploadSampleState {
            samples: kick,
            src_rate: highest_sample_rate_hz,
            upload_rate: highest_sample_rate_hz,
            upload_bit_depth: bit_depth,
            trim_start: 0.0,
            trim_end: 1.0,
            loopdata_start: None,
            loopdata_end: None,
            dc: Arc::clone(&dc),
        });
        if let Some(state_ref) = state.borrow().as_ref() {
            update_norm_sensitivity(&state_ref.samples, &norm_check);
        }
        trim_slider.set_thumbs(0.0, 1.0);
        trim_slider.widget().set_sensitive(true);
        rate_slider.set_pos(1.0);
        rate_slider.widget().set_sensitive(true);
        loop_check.set_sensitive(true);
        save_btn.set_sensitive(true);
        send_btn.set_sensitive(true);
        name_entry.set_text("KICK");
        update_status("Status: Initialized with default Kick sample.");

        UploadSampleScreen { root: base.root }
    }

    /// Returns the root widget for embedding in the navigation stack.
    pub(crate) fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ── File loading ───────────────────────────────────────────────────────────────────────────────────────────

/// Routes a dropped or chosen file to the appropriate loader.
fn dispatch_load(
    path: &str,
    dc: &Arc<DeviceConfig>,
    state: &Rc<RefCell<Option<UploadSampleState>>>,
    display: &SampleDisplay,
    trim_slider: &RangeSlider,
    loopdata_slider: &RangeSlider,
    rate_slider: &RangeSlider,
    loop_check: &CheckButton,
    norm_check: &CheckButton,
    save_btn: &Button,
    send_btn: &Button,
    name_entry: &Entry,
    update_status: &StatusFn,
) {
    let bit_depth = resolve_upload_bit_depth(dc);
    let max_sample_rate_hz = as_u64_or_die(dc.json_get("sampler.highest_sample_rate_hz")) as u32;
    let name_length = as_u64_or_die(dc.json_get("sysex_api.sample.name_length")) as usize;

    let lower = path.to_lowercase();

    // Reject files that aren't the sample data this screen expects, using C7's standard mismatch modal.
    let items = read_c7_or_sysex_file(path);
    if !does_file_type_match(save_btn, path, &items, "sample") {
        return;
    }

    // `.c7` files contain pre-compressed SDS, so expand to raw SDS and treat it as `.sds`.
    let resolved: String = if lower.ends_with(".c7") {
        if let Some(sds_path) = expand_c7_to_sds(path) {
            sds_path
        } else {
            update_status("Error: could not read .c7 file.");
            return;
        }
    } else {
        path.to_string()
    };

    update_status("Status: Loading...");
    save_btn.set_sensitive(false);
    send_btn.set_sensitive(false);

    let dc_c = Arc::clone(dc);
    let state_c = Rc::clone(state);
    let display_c = display.clone();
    let trim_slider_c = trim_slider.clone();
    let loopdata_slider_c = loopdata_slider.clone();
    let rate_slider_c = rate_slider.clone();
    let loop_check_c = loop_check.clone();
    let norm_check_c = norm_check.clone();
    let save_btn_c = save_btn.clone();
    let send_btn_c = send_btn.clone();
    let name_entry_c = name_entry.clone();
    let update_status_c = Arc::clone(update_status);

    glib::spawn_future_local(async move {
        let result = load_to_i16(&dc_c, &resolved);
        match result {
            Ok((samples, src_rate, loopdata_pts, (trim_start, trim_end))) => {
                let sample_count = samples.len();
                let duration_ms = sample_count as f64 / f64::from(src_rate) * 1000.0;

                // Default to full source quality, capped at the device's max accepted rate.
                let upload_rate = src_rate.min(max_sample_rate_hz);

                // Suggested export name from the imported files stem
                let stem_raw = std::path::Path::new(&resolved)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("SMPL");
                let stem: String = filter_elektron_name(stem_raw).chars().take(name_length).collect();
                if !stem.is_empty() {
                    name_entry_c.set_text(&stem);
                }

                // Set display.
                let (loopdata_start, loopdata_end) = match loopdata_pts {
                    Some((start, end)) => (Some(start), Some(end)),
                    None => (None, None),
                };

                let mut disp_samples = samples.clone();
                if norm_check_c.is_active() {
                    normalize_i16(&mut disp_samples);
                }
                display_c.set_samples(disp_samples);
                display_c.set_trim(trim_start, trim_end);

                // Reset trim slider to full range.
                trim_slider_c.set_thumbs(trim_start, trim_end);
                trim_slider_c.widget().set_sensitive(true);
                loop_check_c.set_sensitive(true);

                // Configure loop slider and checkbox.
                if let (Some(loopdata_start), Some(loopdata_end)) = (loopdata_start, loopdata_end) {
                    loopdata_slider_c.set_thumbs(loopdata_start, loopdata_end);
                    loop_check_c.set_active(true); // triggers toggled only if state changed
                    display_c.set_loop(Some(loopdata_start), Some(loopdata_end)); // Sync the visualizer
                } else {
                    loop_check_c.set_active(false);
                    display_c.set_loop(None, None);
                }

                // Configure rate slider (full quality default) and display badges.
                rate_slider_c.set_pos(1.0);
                rate_slider_c.widget().set_sensitive(true);
                display_c.set_sample_rate(src_rate);
                display_c.set_upload_rate(upload_rate);
                display_c.set_upload_bit_depth(bit_depth);

                *state_c.borrow_mut() = Some(UploadSampleState {
                    samples,
                    src_rate,
                    upload_rate,
                    upload_bit_depth: bit_depth,
                    trim_start,
                    trim_end,
                    loopdata_start,
                    loopdata_end,
                    dc: dc_c,
                });
                // State must update before this: unchecking fires `connect_toggled`, whose handler reads `state.samples`.
                if let Some(state_ref) = state_c.borrow().as_ref() {
                    update_norm_sensitivity(&state_ref.samples, &norm_check_c);
                }

                save_btn_c.set_sensitive(true);
                send_btn_c.set_sensitive(true);
                update_status_c(&format!(
                    "Status: Loaded {sample_count} samples ({duration_ms:.0} ms at {src_rate} Hz){}",
                    if loopdata_start.is_some() { ". Loop detected" } else { "" }
                ));
            }
            Err(e) => {
                update_status_c(&format!("Error: {e}"));
            }
        }
    });
}

/// Opens a file chooser to load an audio, SDS, or `.c7` sample file.
fn on_load_clicked(root: &gtk4::Box, on_load: Rc<dyn Fn(String)>) {
    let filter_all = FileFilter::new();
    filter_all.set_name(Some("All supported"));
    for ext in AUDIO_EXTS {
        filter_all.add_pattern(&format!("*{ext}"));
    }
    filter_all.add_pattern("*.sds");
    filter_all.add_pattern("*.syx");
    filter_all.add_pattern("*.c7");
    let filters = gio::ListStore::new::<FileFilter>();
    filters.append(&filter_all);
    let dialog = gtk4::FileDialog::builder()
        .title("Load Audio or SDS Sample")
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

/// Handles the "Save to File" button click.
fn on_save_clicked(
    root: &gtk4::Box,
    state: &Rc<RefCell<Option<UploadSampleState>>>,
    name_entry: &Entry,
    update_status: &StatusFn,
    should_normalize: bool,
    name_length: usize,
) {
    // Save to File always writes a full 16-bit WAV, so the upload bit depth is ignored here.
    let Some((samples, loopdata_type, loopdata_start, loopdata_end, upload_rate, _bits)) = build_sds_from_state(state, should_normalize)
    else {
        update_status("Status: No sample loaded.");
        return;
    };
    let name = clean_name(name_entry, name_length);

    let filter_wav = FileFilter::new();
    filter_wav.set_name(Some("WAV files (*.wav)"));
    filter_wav.add_pattern("*.wav");
    let filters = gio::ListStore::new::<FileFilter>();
    filters.append(&filter_wav);
    let dialog = gtk4::FileDialog::builder()
        .title("Save WAV File")
        .accept_label("Save")
        .initial_name(format!("{name}.wav"))
        .default_filter(&filter_wav)
        .filters(&filters)
        .build();
    dialog.set_initial_folder(Some(&gio::File::for_path(get_export_folder())));
    let window = root.root().and_downcast::<gtk4::Window>();
    let update_status_c = Arc::clone(update_status);
    dialog.save(window.as_ref(), None::<&gio::Cancellable>, move |result| {
        if let Ok(file) = result
            && let Some(mut path) = file.path()
        {
            if path
                .extension()
                .and_then(|extension_val| extension_val.to_str())
                .map(str::to_lowercase)
                .as_deref()
                != Some("wav")
            {
                let mut path_str = path.to_string_lossy().to_string();
                path_str.push_str(".wav");
                path = std::path::PathBuf::from(path_str);
            }
            match write_wav(&path, &samples, upload_rate, Some((loopdata_type, loopdata_start, loopdata_end))) {
                Ok(()) => update_status_c(&format!("Saved: {}", path.file_name().unwrap_or_default().to_string_lossy())),
                Err(e) => update_status_c(&format!("Save failed: {e}")),
            }
        }
    });
}

/// Handles the "Send to Device" button click.
fn on_send_clicked(
    state: &Rc<RefCell<Option<UploadSampleState>>>,
    name_entry: &Entry,
    slot_spin: &NumberSpinner,
    update_status: &StatusFn,
    update_progress: &ProgressFn,
    send_btn: &Button,
    get_midi_rc: &Rc<dyn Fn() -> Option<String>>,
    should_normalize: bool,
    name_length: usize,
) {
    let Some((samples, loopdata_type, loopdata_start, loopdata_end, upload_rate, bits)) = build_sds_from_state(state, should_normalize)
    else {
        update_status("Status: No sample loaded.");
        return;
    };

    let borrow = state.borrow();
    let state_ref = borrow.as_ref().unwrap();
    let port_name = match get_midi_rc() {
        Some(port_name) if is_valid_port(&port_name) => port_name,
        _ => {
            update_status("Error: select a valid MIDI port.");
            return;
        }
    };
    let name = clean_name(name_entry, name_length);
    let slot = slot_spin.value() as u8 - 1; // 0-based
    let dc = Arc::clone(&state_ref.dc);
    drop(borrow);

    let sds_data = audio_to_sds(&samples, upload_rate, loopdata_type, loopdata_start, loopdata_end, bits);
    let packets = build_sds_sample_packets(&sds_data, slot, 0x7F, &dc, &name, None::<&fn() -> bool>);

    send_btn.set_sensitive(false);
    update_progress("Sending...", 0.0, "");

    let update_progress_c = Arc::clone(update_progress);
    let send_btn_c = send_btn.clone();
    let start = Instant::now();
    let should_cancel = Arc::new(AtomicBool::new(false));

    let (tx, rx) = async_channel::unbounded::<(String, f64, bool)>();
    let should_cancel_c = Arc::clone(&should_cancel);
    glib::spawn_future_local(async move {
        while let Ok((msg, frac, done)) = rx.recv().await {
            let eta = eta_string(start, frac);
            update_progress_c(&msg, frac, &eta);
            if done {
                send_btn_c.set_sensitive(true);
            }
        }
    });

    let tx_c = tx.clone();
    glib::spawn_future_local(async move {
        let result = run_midi_session(&port_name, move |midi_in, midi_out| {
            // Seek the target slot on the device.
            seek_sds_slot(midi_in, midi_out, slot, 0x7F);
            sleep(Duration::from_millis(100));

            let name_c = name.clone();
            let cancel_cb = || should_cancel_c.load(Ordering::Relaxed);
            let progress_cb = move |i: usize, total: usize| {
                let frac = if total > 0 { i as f64 / total as f64 } else { 0.0 };
                let msg = format!("Writing '{name_c}'... packet {i}/{total}");
                let _ = tx_c.send_blocking((msg, frac, false));
            };
            let success = send_sds_sample_packets(
                midi_in,
                midi_out,
                &packets,
                Some(&cancel_cb),
                None::<&fn(&str)>,
                Some(&progress_cb),
                &name,
            );
            Ok(success)
        })
        .await;

        let final_msg = if should_cancel.load(Ordering::Relaxed) {
            "Transfer cancelled.".to_string()
        } else {
            match result {
                Ok(true) => "Transfer complete!".to_string(),
                Ok(false) => "Transfer failed.".to_string(),
                Err(e) => format!("Error: {e}"),
            }
        };

        let _ = tx.send_blocking((final_msg, 1.0, true));
    });
}

// ── Module-level helpers ───────────────────────────────────────────────────────────────────────────────────

/// Resamples the audio to the chosen output rate, trims to [`trim_start`, `trim_end`], optionally normalizes, and computes SDS loop params.
///
/// Returns the built samples, loop params, the output rate they're at, and the chosen upload bit depth.
fn build_sds_from_state(
    state: &Rc<RefCell<Option<UploadSampleState>>>,
    should_normalize: bool,
) -> Option<(Vec<i16>, u8, u32, u32, u32, u8)> {
    let borrow = state.borrow();
    let state_ref = borrow.as_ref()?;
    if state_ref.samples.is_empty() {
        return None;
    }
    // Resample the whole source buffer to the chosen output rate, then trim/loop in output-rate terms.
    // Trim/loop points are fractions, so they survive the rate change unchanged.
    let resampled = resample_i16(&state_ref.samples, state_ref.src_rate, state_ref.upload_rate);
    let num_resampled = resampled.len();
    if num_resampled == 0 {
        return None;
    }
    // `start_idx.min(num_resampled - 1)` keeps `start_idx + 1` from exceeding `num_resampled`.
    // Otherwise the clamp below would panic (its min would exceed its max) when `trim_start` is pinned at the right edge.
    let start_idx = ((state_ref.trim_start * num_resampled as f64).round() as usize).min(num_resampled - 1);
    let end_idx = (state_ref.trim_end * num_resampled as f64).round() as usize;
    let end_idx = end_idx.clamp(start_idx + 1, num_resampled);
    let mut trimmed: Vec<i16> = resampled[start_idx..end_idx].to_vec();
    if should_normalize {
        normalize_i16(&mut trimmed);
    }

    let (loopdata_type, loopdata_start_sample, loopdata_end_sample) = match (state_ref.loopdata_start, state_ref.loopdata_end) {
        (Some(loopdata_start), Some(loopdata_end)) => {
            let loopdata_start_sample = (loopdata_start * num_resampled as f64).round() as u32;
            let loopdata_end_sample = (loopdata_end * num_resampled as f64).round() as u32;
            let offset = start_idx as u32;
            (
                0x00u8,
                loopdata_start_sample.saturating_sub(offset),
                loopdata_end_sample.saturating_sub(offset),
            )
        }
        _ => (SDS_LOOP_OFF, 0, 0),
    };

    Some((
        trimmed,
        loopdata_type,
        loopdata_start_sample,
        loopdata_end_sample,
        state_ref.upload_rate,
        state_ref.upload_bit_depth,
    ))
}

/// Grays out "Maximize Volume" when samples are silent or already nearly at peak amplitude.
///
/// Threshold of 32700 (~99.8% of i16) absorbs the 1-count loss from symphonia's internal f32 roundtrip when decoding 16-bit WAV.
fn update_norm_sensitivity(samples: &[i16], norm_check: &CheckButton) {
    let peak = samples.iter().map(|&sample| sample.unsigned_abs()).max().unwrap_or(0);
    let sensitive = peak > 0 && peak < 32700;
    norm_check.set_sensitive(sensitive);
    if !sensitive {
        norm_check.set_active(false);
    }
}

/// Maps a rate-slider fraction [0,1] to a Hz value in `[min_hz, min(src_rate, max_hz)]`, snapped to 100 Hz steps.
///
/// Collapses to a single locked value when the source rate is at or below `min_hz`.
fn slider_pos_to_rate(pos: f64, src_rate: u32, min_hz: u32, max_hz: u32) -> u32 {
    let hi = src_rate.min(max_hz);
    let lo = min_hz.min(hi);
    let raw = f64::from(lo) + pos.clamp(0.0, 1.0) * f64::from(hi - lo);
    // Clamp to [lo, hi] after snapping so the exact source rate stays reachable at the top of the range.
    (((raw / 100.0).round() as u32) * 100).clamp(lo, hi)
}

/// Builds a titled column for a slider: `name` on the left, and, when `with_value` is set, a right-aligned value Label.
/// The caller keeps that Label updated.
///
/// Returns the column to append and that Label.
///
/// Composition over subclassing. The slider widget itself is untouched.
fn build_labeled_slider(name: &str, slider: &RangeSlider, with_value: bool) -> (GtkBox, Option<gtk4::Label>) {
    let header = GtkBox::new(Orientation::Horizontal, 0);
    header.set_margin_start(4);
    header.set_margin_end(4);
    let title = gtk4::Label::builder().label(name).halign(Align::Start).hexpand(true).build();
    header.append(&title);

    let val = with_value.then(|| {
        let val_label = gtk4::Label::builder().halign(Align::End).build();
        header.append(&val_label);
        val_label
    });

    let col = GtkBox::new(Orientation::Vertical, 2);
    col.append(&header);
    col.append(slider.widget());
    (col, val)
}

/// Formats a byte count as a compact KB string for the export-size badge (1 decimal under 100 KB).
///
/// 102400 → "100 KB", 51200 → "50.0 KB".
fn format_kb(bytes: usize) -> String {
    let kb = bytes as f64 / 1024.0;
    if kb >= 100.0 {
        format!("{kb:.0} KB")
    } else {
        format!("{kb:.1} KB")
    }
}

/// Formats a Hz rate as a compact kHz string.
///
/// 48000 → "48 kHz", 44100 → "44.1 kHz".
fn format_khz(hz: u32) -> String {
    let khz = f64::from(hz) / 1000.0;

    if khz.fract().abs() < 1e-9 {
        format!("{khz:.0} kHz")
    } else {
        let string_val = format!("{khz:.2}");
        let string_val = string_val.trim_end_matches('0').trim_end_matches('.');
        format!("{string_val} kHz")
    }
}

/// Cleans and formats the sample name to the device's own `name_length`.
///
/// Falls through to "`SMPL`" if the entry is empty.
fn clean_name(entry: &Entry, name_length: usize) -> String {
    let original_name = name_or_default(entry, "SMPL");
    let truncated_name: String = filter_elektron_name(&original_name).chars().take(name_length).collect();

    // The `<width$` format pads the string with spaces so it is exactly `name_length` characters wide.
    format!("{truncated_name:<name_length$}")
}
