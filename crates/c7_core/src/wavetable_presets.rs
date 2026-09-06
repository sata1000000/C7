//! Wavetable preset algorithms, generated from math rather than read from hardware.
//!
//! Each `make_*` function takes `samples_per_waveform` and returns a `Vec<Vec<i16>>` of 16 to 128 frames.
//! Each inner `Vec<i16>` is one waveform frame, `samples_per_waveform` samples long.
//! These are consumed directly by `upload_digipro` (`UploadDigiproScreen`) and its visualizers.

use std::f32::consts::PI;

use crate::utils::epoch_nanos;

/// Frame count the DigiPro upload screen works in, independent of `samples_per_waveform`.
///
/// Chosen for how the waterfall reads on screen, not for anything the hardware requires.
pub const DIGIPRO_UI_TABLE_LENGTH: usize = 96;

/// One entry in the preset list.
///
/// Holds (display label, factory function that generates `samples_per_waveform`-sample frames).
type PresetEntry = (&'static str, fn(usize) -> Vec<Vec<i16>>);

/// Ready-to-use list of (button label, factory function) for the preset button row.
pub const PRESETS: &[PresetEntry] = &[
    ("FM", make_fm_index_sweep),
    ("Fold", make_wavefolder_sweep),
    ("Harmonics", make_harmonic_morph),
    ("Formant", make_formant_sweep),
    ("AM Sweep", make_am_sweep),
    ("Pluck", make_karplus_strong),
    ("Vowels", make_vowel_morph),
    ("Decimator", make_decimation_sweep),
    ("Chebyshev", make_chebyshev_sweep),
    ("Resonant LPF", make_resonant_sweep),
    ("Sync", make_hard_sync_sweep),
    ("Helix", make_helix),
    ("Pulsar", make_pulsar),
];

/// The preset wavetable loaded when `upload_digipro.rs` is first opened.
pub const DEFAULT_PRESET: fn(usize) -> Vec<Vec<i16>> = make_fm_index_sweep;

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// Generates a single-operator FM sweep with modulation index 0→8.
fn make_fm_index_sweep(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let frames = 64;
    let mut table = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let mod_index = 8.0 * (frame_idx as f32 / (frames - 1) as f32); // modulation index 0→8
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let phase = sample_idx as f32 / samples_per_waveform as f32;
            let modulator = (phase * 2.0 * PI).sin(); // modulator signal
            let val = (phase * 2.0 * PI + mod_index * modulator).sin(); // FM: carrier + index×mod
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates a sine wavefolding sweep by overdriving and re-shaping a sine wave.
fn make_wavefolder_sweep(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let frames = 64;
    let mut table = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let drive = 1.0 + 10.0 * (frame_idx as f32 / (frames - 1) as f32); // gain 1→11; higher = more folds
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let phase = sample_idx as f32 / samples_per_waveform as f32;
            let val = (phase * 2.0 * PI).sin() * drive; // overdrive the sine
            let val = val.sin(); // fold: sin wraps [-π,π] back into [-1,1]
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates a morphing wavetable between additive square and sawtooth waves.
fn make_harmonic_morph(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let frames = 16;
    let mut table = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let normalized_pos = frame_idx as f32 / (frames - 1) as f32;
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let phase = sample_idx as f32 / samples_per_waveform as f32;
            // Bandlimited square: fundamental + 3rd + 5th harmonics (1, 1/3, 1/5)
            let square = (phase * 2.0 * PI).sin() + (1.0 / 3.0) * (phase * 6.0 * PI).sin() + (1.0 / 5.0) * (phase * 10.0 * PI).sin();
            // Bandlimited sawtooth: harmonics 1, 2, 3 with alternating sign.
            let sawtooth = (phase * 2.0 * PI).sin() - (1.0 / 2.0) * (phase * 4.0 * PI).sin() + (1.0 / 3.0) * (phase * 6.0 * PI).sin();
            let val = square * (1.0 - normalized_pos) + sawtooth * normalized_pos;
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates an exponentially decaying formant burst on a sine wave, with frequency sweeping 2×→10×.
fn make_formant_sweep(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let frames = 64;
    let mut table = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let normalized_pos = frame_idx as f32 / (frames - 1) as f32;
        let formant = 2.0 + 8.0 * normalized_pos; // formant frequency as a harmonic multiple: 2× to 10×
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let phase = sample_idx as f32 / samples_per_waveform as f32;
            let env = (-3.0 * phase).exp(); // exponential decay across the cycle
            let val = (phase * formant * 2.0 * PI).sin() * env;
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates a ring modulation sweep with modulator ratio 1× to 11×.
fn make_am_sweep(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let mut table = Vec::with_capacity(DIGIPRO_UI_TABLE_LENGTH);
    for frame_idx in 0..DIGIPRO_UI_TABLE_LENGTH {
        let mod_freq = 1.0 + 10.0 * (frame_idx as f32 / (DIGIPRO_UI_TABLE_LENGTH - 1) as f32); // modulator ratio: 1× to 11×
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let phase = sample_idx as f32 / samples_per_waveform as f32;
            let carrier = (phase * 2.0 * PI).sin();
            let modulator = (phase * mod_freq * 2.0 * PI).sin();
            let val = carrier * modulator; // ring mod: produces sum/difference sidebands
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates a Karplus-Strong string pluck wavetable.
fn make_karplus_strong(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let mut buffer = init_noise_buffer(samples_per_waveform); // initial noise excitation
    let mean = buffer.iter().sum::<f32>() / samples_per_waveform as f32;
    for val in &mut buffer {
        *val -= mean;
    }
    let frames = 32;
    let mut table = Vec::with_capacity(frames);
    for _fi in 0..frames {
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let val = buffer[sample_idx] * 1.3;
            *wave_sample = to_sample(val);
        }
        for _ in 0..4 {
            // 4 decay steps per frame keeps the table from going silent too fast.
            let mut new_buffer = vec![0.0f32; samples_per_waveform];
            for i in 0..samples_per_waveform {
                new_buffer[i] = 0.99 * (buffer[i] + buffer[(i + 1) % samples_per_waveform]) / 2.0; // lowpass + slight decay
            }
            buffer = new_buffer;
        }
        table.push(wave);
    }
    table
}

/// Generates an additive synthesis morph between five vowel formants (A+E+I+O+U).
fn make_vowel_morph(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let frames = 128;
    // Each tuple is (F1, F2, F3) in Hz for one vowel sound.
    let vowels: [(f32, f32, f32); 5] = [
        (730.0, 1090.0, 2440.0),
        (390.0, 1990.0, 2550.0),
        (270.0, 2290.0, 3010.0),
        (400.0, 840.0, 2800.0),
        (300.0, 870.0, 2240.0),
    ];
    let mut table = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let normalized_pos = frame_idx as f32 / (frames - 1) as f32;
        let vowel_pos = normalized_pos * (vowels.len() - 1) as f32;
        let i0 = vowel_pos.floor() as usize;
        let i1 = (i0 + 1).min(vowels.len() - 1);
        let frac = vowel_pos - i0 as f32;
        let frac = frac * frac * (3.0 - 2.0 * frac); // smoothstep to ease through each vowel station

        let f1 = vowels[i0].0 * (1.0 - frac) + vowels[i1].0 * frac;
        let f2 = vowels[i0].1 * (1.0 - frac) + vowels[i1].1 * frac;
        let f3 = vowels[i0].2 * (1.0 - frac) + vowels[i1].2 * frac;
        let r1 = f1 / 100.0; // Hz → harmonic ratio (fundamental = 100 Hz)
        let r2 = f2 / 100.0;
        let r3 = f3 / 100.0;

        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let phase = sample_idx as f32 / samples_per_waveform as f32;

            // Interpolate between integer harmonics for smooth fractional ratios.
            let get_harm = |r: f32, amp: f32| -> f32 {
                let h1 = r.floor() as u32;
                let h2 = r.ceil() as u32;
                if h1 == h2 {
                    return (phase * h1 as f32 * 2.0 * PI).sin() * amp;
                }
                let mix = r - h1 as f32;
                let v1 = (phase * h1 as f32 * 2.0 * PI).sin();
                let v2 = (phase * h2 as f32 * 2.0 * PI).sin();
                (v1 * (1.0 - mix) + v2 * mix) * amp
            };

            let val = get_harm(r1, 1.0) + get_harm(r2, 0.5) + get_harm(r3, 0.25);
            let val = val / 1.75; // normalize: sum of weights is 1+0.5+0.25 = 1.75
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates a bit-crushing sweep on a sine wave.
fn make_decimation_sweep(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let frames = 32;
    let mut table = Vec::with_capacity(frames);
    let max_step = (samples_per_waveform as f32 / 4.0).max(1.0);
    for frame_idx in 0..frames {
        let normalized_pos = frame_idx as f32 / (frames - 1) as f32;
        let step_size = 1.0 + normalized_pos * (max_step - 1.0);
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let quantized_sample = (sample_idx as f32 / step_size).floor() * step_size;
            let phase = quantized_sample / samples_per_waveform as f32;
            let val = (phase * 2.0 * PI).sin();
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates a Chebyshev polynomial waveshaping sweep with order 1→3, producing increasing harmonics.
fn make_chebyshev_sweep(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let mut table = Vec::with_capacity(32);
    let frames = 32;
    for frame_idx in 0..frames {
        let normalized_pos = frame_idx as f32 / (frames - 1) as f32;
        let order = 1.0 + 2.0 * normalized_pos; // polynomial order 1→3
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let phase = sample_idx as f32 / samples_per_waveform as f32;
            let x_val = (phase * 2.0 * PI).sin(); // input signal in [-1, 1]
            // T_n(x) = cos(n·arccos(x)). Clamp x to avoid `NaN` at ±1.
            let val = (order * x_val.clamp(-1.0, 1.0).acos()).cos();
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates a resonant low-pass filter sweep on an additive sawtooth, with cutoff sweeping harmonic 1→24.
fn make_resonant_sweep(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let frames = 32;
    let mut table = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let normalized_pos = frame_idx as f32 / (frames - 1) as f32;
        let cutoff_h = 1.0 + 23.0 * normalized_pos; // cutoff as a harmonic number: 1→24
        let q_factor = 4.0; // Q factor; high enough for a visible resonance peak
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let mut val = 0.0f32;
            // Calculate the resonant filter response for the generated harmonics.
            for h in 1..30usize {
                let amp = 1.0 / h as f32; // sawtooth harmonic amplitude
                let ratio = h as f32 / cutoff_h; // normalized frequency (1.0 = cutoff)
                let denominator = (1.0 - ratio * ratio).powi(2) + (ratio / q_factor).powi(2); // 2nd-order LPF magnitude² denominator
                let gain = 1.0 / denominator.sqrt();
                val += amp * gain * (sample_idx as f32 / samples_per_waveform as f32 * h as f32 * 2.0 * PI).sin();
            }
            val *= 0.8; // scale to keep peaks within range
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates a hard-sync frequency sweep on a sawtooth slave oscillator, with slave/master ratio 1×→2×.
fn make_hard_sync_sweep(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let frames = 32;
    let mut table = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let freq = 1.0 + 1.0 * (frame_idx as f32 / (frames - 1) as f32); // slave/master ratio: 1× to 2×
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let phase = sample_idx as f32 / samples_per_waveform as f32;
            let sync_phase = (phase * freq) % 1.0; // reset slave at every master period
            let val = 2.0 * sync_phase - 1.0; // sawtooth in [-1, 1]
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates a spiraling quadratic chirp wavetable.
fn make_helix(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let frames = 64;
    let mut table = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let normalized_pos = frame_idx as f32 / (frames - 1) as f32;
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let phase = sample_idx as f32 / samples_per_waveform as f32;
            let chirp = phase + 2.5 * phase * phase; // accumulated phase: 0→3.5 cycles
            let val = ((chirp + normalized_pos * 2.0) * 2.0 * PI).sin();
            *wave_sample = to_sample(val);
        }
        table.push(wave);
    }
    table
}

/// Generates a lattice of crossing diagonal pulses.
fn make_pulsar(samples_per_waveform: usize) -> Vec<Vec<i16>> {
    let pulses: &[(f32, f32, f32, f32)] = &[
        (0.00, 1.30, 0.038, 1.0),   // fast forward
        (0.50, -0.85, 0.038, -1.0), // fast backward
        (0.20, 0.25, 0.060, 1.0),   // slow, wide
        (0.75, 2.10, 0.028, -1.0),  // very fast
        (0.40, -0.50, 0.048, 1.0),  // medium backward
    ];
    let mut table = Vec::with_capacity(DIGIPRO_UI_TABLE_LENGTH);
    for frame_idx in 0..DIGIPRO_UI_TABLE_LENGTH {
        let normalized_pos = frame_idx as f32 / (DIGIPRO_UI_TABLE_LENGTH - 1) as f32;
        let mut wave = vec![0i16; samples_per_waveform];
        for (sample_idx, wave_sample) in wave.iter_mut().enumerate() {
            let phase = sample_idx as f32 / samples_per_waveform as f32;
            let mut val = 0.0f32;
            for &(p0, speed, width, sign) in pulses {
                let center = (p0 + normalized_pos * speed).rem_euclid(1.0);
                let distance = (phase - center).abs().min(1.0 - (phase - center).abs());
                val += sign * (-0.5 * (distance / width).powi(2)).exp();
            }
            *wave_sample = to_sample(val * 80.0 / 127.0);
        }
        table.push(wave);
    }
    table
}

/// Maps a bipolar signal value `v ∈ [-1.0, 1.0]` to a full-scale int16 sample.
#[inline]
fn to_sample(val: f32) -> i16 {
    (val * 32767.0).round().clamp(-32767.0, 32767.0) as i16
}

/// Generates a buffer of `samples_per_waveform` pseudo-random floats in [-1.0, 1.0] for Karplus-Strong excitation.
fn init_noise_buffer(samples_per_waveform: usize) -> Vec<f32> {
    let mut seed = epoch_nanos();
    (0..samples_per_waveform)
        .map(|_| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let val = (seed >> 32) as u32 as f32 / u32::MAX as f32; // [0, 1]
            val * 2.0 - 1.0 // [-1, 1]
        })
        .collect()
}
