//! Functions for wavetable loading and conversion.

use crate::c7_file_interfacing::{read_c7_or_sysex_file, read_digipro_wavetable_c7};
use crate::device_config::DeviceConfig;
use crate::digipro::{digipro_frames_from_sysex, digipro_sysex_to_i16};
use crate::dsp_utils::{detect_pitch_period_samples, normalize_i16, read_mono_f32};
use crate::utils::{AUDIO_EXTS, IMAGE_EXTS, SYSEX_EXTS};
use crate::wavetable_presets::DIGIPRO_UI_TABLE_LENGTH;

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// How an audio file gets cut into wavetable frames.
///
/// The same file can be cut more than one way, so the caller picks instead of the loader guessing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameMode {
    /// Picks whichever of the modes below suits the file. Never reported back, since a load always settles on a concrete one.
    Auto,
    /// The whole file becomes one frame, which is how single-cycle waveforms ship.
    SingleCycle,
    /// One frame per detected pitch cycle. Yields nothing useful on audio with no steady pitch.
    Pitch,
    /// One frame per `wave_length` of audio, ignoring pitch entirely.
    FixedFrame,
}

/// Loads a wavetable from any supported audio, image, or SysEx file type.
///
/// Returns the frames alongside the mode that cut them, which for `Auto` is whichever one it settled on.
/// Returns `None` for the mode on image and SysEx files, since those carry their own frame count.
pub fn load_wavetable_file(
    dc: &DeviceConfig,
    path: &str,
    should_trim_front: bool,
    should_trim_back: bool,
    should_normalize: bool,
    should_remove_dc: bool,
    wave_length: usize,
    mode: FrameMode,
) -> Result<(Vec<Vec<i16>>, Option<FrameMode>), String> {
    let path_lowercase = path.to_lowercase();

    // Send SysEx to `sysex_to_digipro_wavetable()` function.
    if SYSEX_EXTS.iter().any(|&ext| path_lowercase.ends_with(ext)) {
        let mut table = sysex_to_digipro_wavetable(dc, path, wave_length)?;
        if should_remove_dc {
            table = center_table(table);
        }
        if should_normalize {
            table = normalize_table(table);
        }
        return Ok((table, None));
    }

    // Send images to `image_to_table()` function.
    if IMAGE_EXTS.iter().any(|&ext| path_lowercase.ends_with(ext)) {
        let mut table = image_to_table(path, wave_length)?;
        if should_remove_dc {
            table = center_table(table);
        }
        if should_normalize {
            table = normalize_table(table);
        }
        return Ok((table, None));
    }

    // Send audio to `audio_to_table()` function.
    if AUDIO_EXTS.iter().any(|&ext| path_lowercase.ends_with(ext)) {
        let (mut table, used_mode) = audio_to_table(path, should_trim_front, should_trim_back, should_normalize, wave_length, mode)?;
        if should_remove_dc {
            table = center_table(table);
        }
        // `audio_to_table()` normalizes the source samples.
        // Framing interpolates between neighbors, so the table can come out under that peak.
        if should_normalize {
            table = normalize_table(table);
        }
        return Ok((table, Some(used_mode)));
    }

    // Raise error if an unsupported file type was uploaded.
    Err(format!("Unsupported file type '{path}'."))
}

/// Decodes waveform frames from `.c7` or `.syx` files.
///
/// The `.c7` payloads store the raw `0x5D` SysEx per frame, so both file types share `digipro_sysex_to_i16()` as one lossless decode path.
pub fn sysex_to_digipro_wavetable(dc: &DeviceConfig, path: &str, wave_length: usize) -> Result<Vec<Vec<i16>>, String> {
    let path_lowercase = path.to_lowercase();

    // If user uploaded a `.c7` file.
    if path_lowercase.ends_with(".c7") {
        let items = read_c7_or_sysex_file(path);
        let c7_type: Option<String> = items
            .iter()
            .find(|item| item.section == "c7")
            .and_then(|item| item.attrs.get("type"))
            .map(|type_str| type_str.to_lowercase());

        // DigiPro wavetable `.c7`:
        // One stored `0x5D` SysEx message per frame, so decode each to an int16 wave.
        if c7_type.as_deref() == Some("digipro_wavetable") {
            let frames_raw = read_digipro_wavetable_c7(path);
            if frames_raw.is_empty() {
                return Err("No waveform frames found in DigiPro wavetable file.".to_string());
            }
            return Ok(frames_raw
                .into_iter()
                .map(|(_, sysex)| digipro_sysex_to_i16(dc, &sysex, wave_length))
                .collect());
        }

        // Single DigiPro `.c7`:
        // The stored payload is the `0x5D` SysEx message, decoded to a single frame.
        if c7_type.as_deref() == Some("digipro") {
            let frame_data = items
                .iter()
                .find(|item| item.section != "c7" && item.attrs.get("type").map(String::as_str) == Some("digipro"))
                .and_then(|item| item.get_data_bytes());

            let sysex = match frame_data {
                Some(bytes) if !bytes.is_empty() => bytes.to_vec(),
                _ => return Err("No waveform data found in digipro file.".to_string()),
            };
            let wave = digipro_sysex_to_i16(dc, &sysex, wave_length);
            return Ok(vec![wave]);
        }

        // If `.c7` file is an invalid data type.
        return Err(format!("Unsupported .c7 type '{}'.", c7_type.unwrap_or_default()));
    }

    // If user uploaded an `.syx` file.
    if path_lowercase.ends_with(".syx") {
        let raw = std::fs::read(path).map_err(|e| format!("File read failed: {e}"))?;
        let frames = digipro_frames_from_sysex(dc, &raw, wave_length);
        if frames.is_empty() {
            return Err("No DigiPro waveform messages found in file.".to_string());
        }
        return Ok(frames);
    }

    // If file is an invalid type.
    Err(format!("Unsupported file type '{path}'."))
}

/// Peak-normalizes a list of int16 waveform frames to full scale (±32767).
pub fn normalize_table(table: Vec<Vec<i16>>) -> Vec<Vec<i16>> {
    let peak = table
        .iter()
        .flat_map(|wave| wave.iter())
        .map(|&sample_val| sample_val.unsigned_abs())
        .max()
        .unwrap_or(0);
    if peak < 1 {
        return table;
    }
    let scale = 32767.0 / peak as f64;
    table
        .into_iter()
        .map(|wave| {
            wave.into_iter()
                .map(|sample_val| (sample_val as f64 * scale).round().clamp(-32767.0, 32767.0) as i16)
                .collect()
        })
        .collect()
}

/// Centers each waveform frame around 0 by removing its DC offset.
pub fn center_table(table: Vec<Vec<i16>>) -> Vec<Vec<i16>> {
    let mut result = Vec::with_capacity(table.len());
    for wave in table {
        if wave.is_empty() {
            result.push(wave);
            continue;
        }
        let dc_offset = wave.iter().map(|&sample_val| sample_val as f64).sum::<f64>() / wave.len() as f64;
        result.push(
            wave.into_iter()
                .map(|sample_val| (sample_val as f64 - dc_offset).round().clamp(-32767.0, 32767.0) as i16)
                .collect(),
        );
    }
    result
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// Scales an image to the wavetable grid and maps pixel luma to sample values.
fn image_to_table(path: &str, wave_length: usize) -> Result<Vec<Vec<i16>>, String> {
    use gdk_pixbuf::{InterpType, Pixbuf};

    // Load the image from disk.
    let pixbuf = Pixbuf::from_file(path).map_err(|e| format!("Image load failed: {e}"))?;

    // Ensure RGB (strip alpha if present so channel count is predictable).
    let pixbuf = if pixbuf.has_alpha() {
        pixbuf
            .composite_color_simple(
                pixbuf.width(),
                pixbuf.height(),
                InterpType::Bilinear,
                255,
                1,
                0x00FF_FFFF,
                0x00FF_FFFF,
            )
            .ok_or("Alpha-strip composite failed")?
    } else {
        pixbuf
    };

    let pixbuf = pixbuf
        .scale_simple(wave_length as i32, DIGIPRO_UI_TABLE_LENGTH as i32, InterpType::Bilinear)
        .ok_or("Scale failed")?;

    let pixels = pixbuf.pixel_bytes().ok_or("No pixel data")?;
    let num_channels = pixbuf.n_channels() as usize; // 3 (RGB)
    let rowstride = pixbuf.rowstride() as usize;
    let mut raw = Vec::with_capacity(DIGIPRO_UI_TABLE_LENGTH);

    // Map the image pixels to Rec.601 luma values to determine amplitude.
    for frame_idx in 0..DIGIPRO_UI_TABLE_LENGTH {
        let mut row = Vec::with_capacity(wave_length);
        for sample_idx in 0..wave_length {
            let pixel_offset = frame_idx * rowstride + sample_idx * num_channels;
            let red = pixels[pixel_offset] as f64;
            let green = pixels[pixel_offset + 1] as f64;
            let blue = pixels[pixel_offset + 2] as f64;
            row.push(0.299 * red + 0.587 * green + 0.114 * blue); // Rec.601 luma
        }
        raw.push(row);
    }

    // Stretch the full luma range across the int16 amplitude range so no image wastes amplitude.
    let (min_luma, max_luma) = raw
        .iter()
        .flat_map(|row_val| row_val.iter().copied())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), luma_val| {
            (f64::min(min, luma_val), f64::max(max, luma_val))
        });
    let scale = if max_luma > min_luma {
        65534.0 / (max_luma - min_luma)
    } else {
        1.0
    };
    let mut table: Vec<Vec<i16>> = raw
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|luma_val| ((luma_val - min_luma) * scale - 32767.0).round().clamp(-32767.0, 32767.0) as i16)
                .collect()
        })
        .collect();
    table.reverse(); // row 0 of the image maps to the last wavetable frame
    Ok(table)
}

/// Extracts waveform frames from an audio file by the requested `mode`.
///
/// Returns the frames alongside the mode that produced them, which for `Auto` is whichever one it settled on.
fn audio_to_table(
    path: &str,
    should_trim_front: bool,
    should_trim_back: bool,
    should_normalize: bool,
    wave_length: usize,
    mode: FrameMode,
) -> Result<(Vec<Vec<i16>>, FrameMode), String> {
    let (f32_samples, sample_rate) = read_mono_f32(path).ok_or_else(|| format!("Audio read failed: {path}"))?;

    // Convert f32 samples to i16, clamping to guard against values that peak slightly outside [-1.0, 1.0].
    let mut samples: Vec<i16> = f32_samples
        .iter()
        .map(|&sample_val| (sample_val * 32767.0).clamp(-32767.0, 32767.0) as i16)
        .collect();

    if should_trim_front {
        let peak = samples.iter().map(|sample_val| sample_val.unsigned_abs()).max().unwrap_or(1).max(1);
        let thresh = peak as f64 * 0.01;
        let trim_idx = samples
            .iter()
            .position(|&sample_val| (sample_val as f64).abs() >= thresh)
            .unwrap_or(0);
        samples = samples[trim_idx..].to_vec();
    }

    if should_trim_back {
        let peak = samples.iter().map(|sample_val| sample_val.unsigned_abs()).max().unwrap_or(1).max(1);
        let thresh = peak as f64 * 0.01;
        let trim_idx = samples
            .iter()
            .rposition(|&sample_val| (sample_val as f64).abs() >= thresh)
            .unwrap_or_else(|| samples.len().saturating_sub(1));
        samples = samples[..=trim_idx].to_vec();
    }

    if should_normalize {
        normalize_i16(&mut samples);
    }

    let num_samples = samples.len();
    if num_samples == 0 {
        return Err("Audio file contains no usable samples.".to_string());
    }

    // A file too short to hold two frames is assumed as a single-cycle waveform.
    // `detect_pitch_period_samples()` caps its search at a third of the window, so it would settle on a subharmonic instead.
    if mode == FrameMode::SingleCycle || (mode == FrameMode::Auto && num_samples < wave_length * 2) {
        return Ok((vec![resample_cycle(&samples, wave_length)], FrameMode::SingleCycle));
    }

    if mode == FrameMode::Auto || mode == FrameMode::Pitch {
        let period = detect_pitch_period_samples(&samples, num_samples, sample_rate);
        if let Some(detected_period) = period {
            let cycles = extract_cycles(&samples, num_samples, detected_period as f64);
            if !cycles.is_empty() {
                return Ok((cycles_to_table(&cycles, wave_length), FrameMode::Pitch));
            }
        }
        // An explicit Pitch request on audio with no steady pitch still has to produce something, so it falls through too.
    }

    // Audio with no steady pitch, sliced into equal segments instead.
    // One frame per `wave_length` of source, so no frame is stretched out of fewer samples than it holds.
    let frame_count = (num_samples / wave_length).clamp(1, DIGIPRO_UI_TABLE_LENGTH);
    let segment_len = num_samples / frame_count;
    let mut table = Vec::with_capacity(frame_count);

    // Slice the audio buffer and resample the segments into discrete wavetable frames.
    for frame_idx in 0..frame_count {
        let start_idx = frame_idx * segment_len;
        let end_idx = num_samples.min((frame_idx + 1) * segment_len);
        let segment = &samples[start_idx..end_idx];
        table.push(resample_cycle(segment, wave_length));
    }

    Ok((table, FrameMode::FixedFrame))
}

/// Slices a sample stream into pitch-aligned cycles.
fn extract_cycles(samples: &[i16], total_samples: usize, period: f64) -> Vec<Vec<i16>> {
    let int_period = (period.round() as usize).max(1);
    let mut read_pos = 0usize;

    // Advance to the first positive-going zero crossing so all cycles start at the same phase.
    for sample_idx in 1..(int_period * 2).min(total_samples) {
        if samples[sample_idx - 1] <= 0 && samples[sample_idx] > 0 {
            read_pos = sample_idx;
            break;
        }
    }

    let mut cycles = Vec::new();
    while read_pos + int_period <= total_samples {
        cycles.push(samples[read_pos..read_pos + int_period].to_vec());
        read_pos += int_period;
    }
    cycles
}

/// Turns each detected pitch cycle into one wavetable frame, so the number of frames follows the audio instead of a fixed grid.
///
/// A long sample holds more cycles than a table can show, so those are sampled evenly.
/// Every frame is a real cycle taken from the source either way.
fn cycles_to_table(cycles: &[Vec<i16>], wave_length: usize) -> Vec<Vec<i16>> {
    if cycles.len() <= DIGIPRO_UI_TABLE_LENGTH {
        return cycles.iter().map(|cycle| resample_cycle(cycle, wave_length)).collect();
    }

    (0..DIGIPRO_UI_TABLE_LENGTH)
        .map(|frame_idx| {
            let cycle_idx = frame_idx * (cycles.len() - 1) / (DIGIPRO_UI_TABLE_LENGTH - 1);
            resample_cycle(&cycles[cycle_idx], wave_length)
        })
        .collect()
}

/// Resamples an audio segment (a detected pitch cycle, or a fixed-length slice) into `wave_length` int16 samples.
fn resample_cycle(cycle: &[i16], wave_length: usize) -> Vec<i16> {
    let cycle_len = cycle.len();
    let mut wave = vec![0i16; wave_length];
    for (sample_idx, sample_out) in wave.iter_mut().enumerate() {
        let sample_pos = if cycle_len > 1 {
            (sample_idx as f64 / (wave_length - 1) as f64) * (cycle_len - 1) as f64
        } else {
            0.0
        };
        let idx_lower = sample_pos.floor() as usize;
        let idx_upper = (idx_lower + 1).min(cycle_len - 1);
        let interp_val =
            cycle[idx_lower] as f64 * (1.0 - (sample_pos - idx_lower as f64)) + cycle[idx_upper] as f64 * (sample_pos - idx_lower as f64);
        *sample_out = interp_val.round().clamp(-32767.0, 32767.0) as i16;
    }
    wave
}
