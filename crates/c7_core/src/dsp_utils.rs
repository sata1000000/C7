//! Functions for audio analysis.

use crate::device_config::DeviceConfig;
use crate::digipro::{DIGIPRO_CYCLE_LEN, digipro_sysex_to_i16, is_digipro_packet};
use crate::sds::{decode_sds, is_sds_message, read_sds_loopdata, read_wav_loopdata, sds_samples_to_i16};
use crate::sysex::parse_sysex_file;

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Detects front and back silence durations in an audio file.
///
/// Returns `(front_seconds, back_seconds)`.
pub fn detect_silence(file_path: &str) -> (f64, f64) {
    let Some((mono, sample_rate)) = read_mono_f32(file_path) else {
        return (0.0, 0.0);
    };
    let (front, back) = detect_silence_f32(&mono, 0.01);
    (front as f64 / sample_rate as f64, back as f64 / sample_rate as f64)
}

/// Peak-normalizes an i16 buffer to the full range (-32768 to 32767).
pub fn normalize_i16(samples: &mut [i16]) {
    let peak = find_peak_i16(samples);
    if peak > 0 && peak < 32767 {
        let scale = 32767.0 / peak as f64;
        for sample in samples.iter_mut() {
            *sample = (*sample as f64 * scale).clamp(-32767.0, 32767.0) as i16;
        }
    }
}

/// Estimates the fundamental period (in samples) via normalized autocorrelation.
///
/// Tries four evenly spaced windows, since a leading transient or silence would fool a single one.
///
/// Returns the first repeat that scores well enough, which is the fundamental period.
/// Returns `None` for audio with no steady pitch, such as a drum hit or a pitch sweep.
pub fn detect_pitch_period_samples(samples: &[i16], num_samples: usize, sample_rate: u32) -> Option<usize> {
    let window_len = num_samples.min(4096);

    // Scale lag bounds to the actual sample rate.
    let rate_ratio = sample_rate as f64 / 44100.0;
    let min_lag = 1usize.max((22.0 * rate_ratio).round() as usize); // ≈ 2000 Hz upper bound
    let max_lag = ((1102.0 * rate_ratio).round() as usize).min(window_len / 3); // ≈ 40 Hz; cap for ≥3 cycles

    if max_lag <= min_lag {
        return None;
    }

    let normalized_samples: Vec<f64> = samples[..num_samples].iter().map(|&sample| sample as f64 / 32768.0).collect();

    // Probe at 0%, 25%, 50%, and 75% through the signal.
    let positions: Vec<usize> = [0usize, num_samples / 4, num_samples / 2, 3 * num_samples / 4]
        .iter()
        .copied()
        .filter(|&pos| pos + window_len <= num_samples)
        .collect();

    let mut best_lag: Option<usize> = None;
    let mut best_corr: f64 = -1.0;

    for start in positions {
        let window = &normalized_samples[start..start + window_len];
        let energy: f64 = window.iter().map(|val| val * val).sum();
        if energy < 1e-4 {
            continue;
        }

        // Correlation starts near 1.0 at the shortest lag and falls away, since any signal resembles itself most there.
        // Scoring only once the curve decays past zero skips that opening slope, so the first peak found is the fundamental period.
        let mut has_decayed = false;
        for lag in min_lag..max_lag {
            let corr: f64 = window[..window_len - lag]
                .iter()
                .zip(&window[lag..])
                .map(|(sample, lagged_sample)| sample * lagged_sample)
                .sum::<f64>()
                / energy;

            if !has_decayed {
                has_decayed = corr <= 0.0;
                continue;
            }
            if corr > best_corr {
                best_corr = corr;
                best_lag = Some(lag);
            }
        }
    }

    if best_corr >= 0.20 { best_lag } else { None }
}

/// Resamples a mono i16 buffer from `src_rate` to `dst_rate` by linear interpolation, via `resample_f32()`.
///
/// Shared by the sample-upload build path and the local preview player.
pub fn resample_i16(samples: &[i16], src_rate: u32, dst_rate: u32) -> Vec<i16> {
    if src_rate == dst_rate {
        return samples.to_vec();
    }
    let as_f32: Vec<f32> = samples.iter().map(|&sample| f32::from(sample) / 32768.0).collect();
    resample_f32(&as_f32, src_rate, dst_rate)
        .into_iter()
        .map(|float_val| (float_val * 32768.0).clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16)
        .collect()
}

/// Synthesizes a 0.25 second kick drum at `sample_rate_hz`.
///
/// Used as the placeholder sample when no sample is loaded.
pub fn generate_default_kick(sample_rate_hz: u32) -> Vec<i16> {
    // The sound is rate-independent (frequencies are in Hz), so the rate only sets fidelity, not pitch.
    let sample_rate = f64::from(sample_rate_hz);
    let duration_secs = 0.25;
    let num_samples = (sample_rate * duration_secs) as usize;
    let mut samples = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let time = i as f64 / sample_rate;
        // Exponential pitch decay from ~150Hz to ~40Hz.
        let freq = 40.0 + 110.0 * (-15.0 * time).exp();
        let phase = 2.0 * std::f64::consts::PI * freq * time;
        // Linear amplitude envelope.
        let env = (1.0 - i as f64 / num_samples as f64).powi(2);
        let val = (phase.sin() * env * 32767.0) as i16;
        samples.push(val);
    }
    samples
}

/// Loads an audio or `.sds` file into mono i16 samples at the file's own source rate.
///
/// Returns the detected loop points and silence-trim bounds alongside the samples and that rate.
pub fn load_to_i16(dc: &DeviceConfig, path: &str) -> AudioLoadResult {
    let lower = path.to_lowercase();

    let is_syx = lower.ends_with(".syx") || lower.ends_with(".sysex");
    let is_sds_ext = lower.ends_with(".sds");

    if is_sds_ext || is_syx {
        let data = std::fs::read(path).map_err(|err| err.to_string())?;

        if is_sds_ext || is_sds_message(&data) {
            // Raw SDS: decode packets → i16 samples at the rate stored in the SDS header.
            // `decode_sds()` yields depth-native values, so scale them to full-scale i16 for the editor, playback, and waveform drawing.
            let (depth_native, rate, bits) = decode_sds(&data)?;
            let samples = sds_samples_to_i16(&depth_native, bits);

            let num_samples = samples.len();
            let (front, back) = detect_silence_i16(&samples, 0.01);
            let trim_start = front as f64 / num_samples as f64;
            let trim_end = 1.0 - (back as f64 / num_samples as f64);
            let (trim_start, trim_end) = if trim_end <= trim_start {
                (0.0, 1.0)
            } else {
                (trim_start, trim_end)
            };

            let loopdata_pts = read_sds_loopdata(path).map(|(_, loopdata_start, loopdata_end)| {
                (loopdata_start as f64 / num_samples as f64, loopdata_end as f64 / num_samples as f64)
            });
            return Ok((samples, rate, loopdata_pts, (trim_start, trim_end)));
        }
        // A `.syx` that fails the SDS check might be a Monomachine DigiPro waveform dump instead.
        // Confirm the device has a DigiPro spec and the file contains such a dump before decoding it as one.
        else if is_syx && dc.is_device("MnM") {
            let msg = parse_sysex_file(&data)
                .into_iter()
                .find(|packet| is_digipro_packet(dc, packet))
                .ok_or("No DigiPro waveform message found in file.")?;
            let samples = digipro_sysex_to_i16(dc, &msg, DIGIPRO_CYCLE_LEN);
            return Ok((samples, 44_100, Some((0.0, 1.0)), (0.0, 1.0)));
        }
    }

    // Audio file: read via symphonia and keep the samples at their source rate.
    let (raw_f32, src_rate) = read_mono_f32(path).ok_or("Failed to decode audio file")?;

    let n_raw = raw_f32.len();
    let (front, back) = detect_silence_f32(&raw_f32, 0.01);
    let trim_start = front as f64 / n_raw as f64;
    let trim_end = 1.0 - (back as f64 / n_raw as f64);
    let (trim_start, trim_end) = if trim_end <= trim_start {
        (0.0, 1.0)
    } else {
        (trim_start, trim_end)
    };

    let samples: Vec<i16> = raw_f32
        .iter()
        .map(|&sample| (sample * 32767.0).clamp(-32767.0, 32767.0) as i16)
        .collect();

    // Read loop points from the WAV smpl chunk as plain fractions of the source length.
    let loopdata_pts = read_wav_loopdata(path).map(|(_, loopdata_start, loopdata_end)| {
        let num_samples = samples.len() as f64;
        (loopdata_start as f64 / num_samples, loopdata_end as f64 / num_samples)
    });

    Ok((samples, src_rate, loopdata_pts, (trim_start, trim_end)))
}

/// Converts a MIDI note number to a human-readable name (like 60 -> "C4").
pub fn note_name(note_num: i32) -> String {
    let clamped_note = note_num.max(0);
    format!("{}{}", NOTE_NAMES[(clamped_note % 12) as usize], (clamped_note / 12) - 1)
}

/// Converts a semitone offset from C to an Elektron-compatible pitch value.
pub fn semitone_to_pitch(offset: i32) -> u8 {
    (64 + offset * 8).clamp(0, 127) as u8
}

/// Reads an audio file via symphonia and returns mono f32 samples + sample rate.
///
/// Multi-channel audio is downmixed to mono by averaging all channels.
pub fn read_mono_f32(path: &str) -> Option<(Vec<f32>, u32)> {
    let (interleaved, rate, channels) = read_raw_f32(path)?;
    let num_channels = channels.max(1) as usize;
    if num_channels == 1 {
        // Already mono. No need to average.
        return Some((interleaved, rate));
    }
    // Average all channels together to produce mono output.
    let mono: Vec<f32> = interleaved
        .chunks(num_channels)
        .map(|frame| frame.iter().sum::<f32>() / num_channels as f32)
        .collect();
    Some((mono, rate))
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// Return type of `load_to_i16()`.
///
/// Holds (samples, source sample rate, loop points, trim bounds).
///
/// Samples are kept at their source rate, so the caller resamples to the chosen output rate later.
type AudioLoadResult = Result<(Vec<i16>, u32, Option<(f64, f64)>, (f64, f64)), String>;

/// Detects the number of silent samples at the beginning and end of an f32 buffer.
///
/// Returns `(front_samples, back_samples)`.
fn detect_silence_f32(samples: &[f32], threshold_ratio: f64) -> (usize, usize) {
    let num_samples = samples.len();
    if num_samples == 0 {
        return (0, 0);
    }
    let peak = find_peak_f32(samples);
    if peak < 1.0 / 32768.0 {
        return (num_samples, num_samples);
    }
    let threshold = peak as f64 * threshold_ratio;

    let front = samples
        .iter()
        .position(|&sample| (sample as f64).abs() >= threshold)
        .unwrap_or(num_samples);
    let back = samples
        .iter()
        .rposition(|&sample| (sample as f64).abs() >= threshold)
        .map_or(num_samples, |position| num_samples - 1 - position);
    (front, back)
}

/// Detects the number of silent samples at the beginning and end of an i16 buffer.
///
/// Returns `(front_samples, back_samples)` where `back_samples` is the count from the end.
fn detect_silence_i16(samples: &[i16], threshold_ratio: f64) -> (usize, usize) {
    let num_samples = samples.len();
    if num_samples == 0 {
        return (0, 0);
    }
    let peak = find_peak_i16(samples);
    if peak == 0 {
        return (num_samples, num_samples);
    }
    let threshold = (peak as f64 * threshold_ratio).round() as u16;

    let front = samples
        .iter()
        .position(|&sample| sample.unsigned_abs() >= threshold)
        .unwrap_or(num_samples);
    let back = samples
        .iter()
        .rposition(|&sample| sample.unsigned_abs() >= threshold)
        .map_or(num_samples, |position| num_samples - 1 - position);
    (front, back)
}

/// Linearly resamples mono f32 samples from `src_rate` to `dst_rate`.
fn resample_f32(samples: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if src_rate == dst_rate {
        return samples.to_vec();
    }
    let input_len = samples.len();
    if input_len == 0 {
        return vec![];
    }
    let output_len = ((input_len as f64 * dst_rate as f64 / src_rate as f64).round() as usize).max(1);
    (0..output_len)
        .map(|sample_idx| {
            let sample_pos = if output_len == 1 {
                0.0_f64
            } else {
                sample_idx as f64 * (input_len as f64 - 1.0) / (output_len as f64 - 1.0)
            };
            let idx_lower = sample_pos.floor() as usize;
            let idx_upper = (idx_lower + 1).min(input_len - 1);
            let fraction = (sample_pos - idx_lower as f64) as f32;
            samples[idx_lower] * (1.0 - fraction) + samples[idx_upper] * fraction
        })
        .collect()
}

/// Returns the absolute peak value in an i16 buffer.
fn find_peak_i16(samples: &[i16]) -> u16 {
    samples.iter().map(|&sample| sample.unsigned_abs()).max().unwrap_or(0)
}

/// Returns the absolute peak value in an f32 buffer.
fn find_peak_f32(samples: &[f32]) -> f32 {
    samples.iter().map(|&sample| sample.abs()).fold(0.0f32, f32::max)
}

/// Decodes an audio file via symphonia.
///
/// Returns `(interleaved_f32_samples, sample_rate, channels)`.
/// Returns `None` on any read or decode error.
fn read_raw_f32(path: &str) -> Option<(Vec<f32>, u32, u32)> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::{FormatOptions, probe::Hint};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), symphonia::core::io::MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if let Some(extension) = std::path::Path::new(path)
        .extension()
        .and_then(|extension_val| extension_val.to_str())
    {
        hint.with_extension(extension);
    }

    // 0.6: `probe()` replaces `format()` and returns Box<dyn `FormatReader`> directly (no wrapper struct).
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .ok()?;

    // 0.6: `codec_params` is now an `Option<CodecParameters>` enum. Find first audio track.
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.as_ref().and_then(|param| param.audio()).is_some())?;

    let track_id = track.id;
    // Clone to drop the borrow on format before calling format.`next_packet()`.
    let audio_params = track.codec_params.as_ref()?.audio()?.clone();
    let sample_rate = audio_params.sample_rate?;
    let channels = audio_params.channels.as_ref().map_or(1, symphonia::core::audio::Channels::count) as u32;

    // 0.6: `make_audio_decoder()` replaces `make()` and takes &`AudioCodecParameters`.
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .ok()?;

    let mut all_samples: Vec<f32> = Vec::new();

    while let Ok(Some(packet)) = format.next_packet() {
        if packet.track_id != track_id {
            continue;
        }

        let Ok(audio_buf) = decoder.decode(&packet) else { continue };

        // 0.6: `SampleBuffer` is gone. `copy_to_vec_interleaved` dispatches over all sample formats.
        let mut chunk: Vec<f32> = Vec::new();
        audio_buf.copy_to_vec_interleaved(&mut chunk);
        all_samples.extend_from_slice(&chunk);
    }

    if all_samples.is_empty() {
        return None;
    }
    Some((all_samples, sample_rate, channels))
}
