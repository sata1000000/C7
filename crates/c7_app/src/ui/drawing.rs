//! Shared Cairo drawing helpers and the UI color palette.

use cairo::{FontSlant, FontWeight};

/// Default corner radius used for GTK-style rounded UI controls.
pub(crate) const UI_CORNER_RADIUS: f64 = 12.0;

/// Which corner a `draw_corner_badge()` text anchors to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
}

/// Draws a small bold monospace white badge anchored to one corner of a canvas.
pub(crate) fn draw_corner_badge(cr: &cairo::Context, width: f64, height: f64, corner: Corner, text: &str) {
    let pad = 10.0;
    cr.select_font_face("monospace", FontSlant::Normal, FontWeight::Bold);
    cr.set_font_size(13.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
    let Ok(ext) = cr.text_extents(text) else {
        return;
    };
    let (x, y) = match corner {
        Corner::TopLeft => (pad, pad + ext.height()),
        Corner::TopRight => (width - ext.width() - pad, pad + ext.height()),
        Corner::BottomRight => (width - ext.width() - pad, height - pad),
    };
    cr.move_to(x, y);
    let _ = cr.show_text(text);
}

/// Draws a smaller, semi-transparent monospace label anchored to one corner of a canvas.
///
/// Used for compact in-widget reading like the LFO preview's name/value labels.
pub(crate) fn draw_corner_badge_smaller(cr: &cairo::Context, width: f64, height: f64, corner: Corner, text: &str) {
    if text.is_empty() {
        return;
    }
    let pad = 4.0;
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.4);
    cr.set_font_size(10.0);
    cr.select_font_face("monospace", FontSlant::Normal, FontWeight::Bold);
    let Ok(ext) = cr.text_extents(text) else {
        return;
    };
    let (x, y) = match corner {
        Corner::TopLeft => (pad, pad + ext.height()),
        Corner::TopRight => (width - ext.width() - pad, pad + ext.height()),
        Corner::BottomRight => (width - ext.width() - pad, height - pad),
    };
    cr.move_to(x, y);
    let _ = cr.show_text(text);
}

/// Builds a rounded rectangle path on the Cairo context.
pub(crate) fn build_rounded_rect_path(cr: &cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    cr.new_path();
    cr.arc(x + radius, y + radius, radius, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.arc(
        x + width - radius,
        y + radius,
        radius,
        1.5 * std::f64::consts::PI,
        2.0 * std::f64::consts::PI,
    );
    cr.arc(x + width - radius, y + height - radius, radius, 0.0, 0.5 * std::f64::consts::PI);
    cr.arc(
        x + radius,
        y + height - radius,
        radius,
        0.5 * std::f64::consts::PI,
        std::f64::consts::PI,
    );
    cr.close_path();
}

/// Clips the Cairo context to a rounded rectangle (GTK-style corners).
pub(crate) fn clip_rounded(cr: &cairo::Context, width: f64, height: f64) {
    build_rounded_rect_path(cr, 0.0, 0.0, width, height, UI_CORNER_RADIUS);
    cr.clip();
}

// ── Shared UI Colors ───────────────────────────────────────────────────────────────────────────────────────

// Blue for nice UI elements.
//
// Used in `ParameterKnob` / LFO previews / DigiPro previews.
pub(crate) const COLOR_BLUE_R: f64 = 0.2;
pub(crate) const COLOR_BLUE_G: f64 = 0.6;
pub(crate) const COLOR_BLUE_B: f64 = 1.0;
pub(crate) const COLOR_BLUE: (f64, f64, f64) = (COLOR_BLUE_R, COLOR_BLUE_G, COLOR_BLUE_B);

// Green.
//
// Used in Multi mode in `upload_digipro.rs`.
pub(crate) const COLOR_GREEN_R: f64 = 0.2;
pub(crate) const COLOR_GREEN_G: f64 = 0.88;
pub(crate) const COLOR_GREEN_B: f64 = 0.35;

// Purple.
//
// Used for ghost frames / inactive regions.
pub(crate) const COLOR_PURPLE_R: f64 = 0.62;
pub(crate) const COLOR_PURPLE_G: f64 = 0.58;
pub(crate) const COLOR_PURPLE_B: f64 = 0.92;

// Orange.
//
// Used for the sample-rate slider in `upload_sample.rs`.
pub(crate) const COLOR_ORANGE_R: f64 = 1.0;
pub(crate) const COLOR_ORANGE_G: f64 = 0.6;
pub(crate) const COLOR_ORANGE_B: f64 = 0.1;
