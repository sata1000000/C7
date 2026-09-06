//! Functions for the DigiPro waveform codec.

use std::f32::consts::PI;

use crate::device_config::DeviceConfig;
use crate::sysex::{
    ELEKTRON_CHANNEL_BYTE, ELEKTRON_CHECKSUM_MASK, ELEKTRON_MFR, ELEKTRON_PROD_BYTE, ELEKTRON_TYPE_BYTE, decode_7bit, encode_7bit,
    parse_sysex_file,
};
use crate::utils::{JsonPath, as_u64_or_die};

/// The DigiPro wire format's native single-cycle length, and the FFT size for spectral processing.
pub const DIGIPRO_CYCLE_LEN: usize = 1024;

/// Bytes per sample frame in the packed DigiPro payload (`base_hi`, `base_lo`, `blend_hi`, `mipmap_hi`, `mipmap_lo`, `blend_lo`).
const BYTES_PER_FRAME: usize = 6;

/// The fixed playback gain the DigiPro hardware applies to waveform data.
///
/// Encoding divides the spectrum by this so the stored wave lands at the right level once the hardware boosts it on playback.
/// Decoding multiplies by it so the editor's amplitude matches what the device outputs.
const DIGIPRO_COEFFICIENT_MAX: f32 = 9.14025;

/// The nine anti-aliasing mipmap levels.
///
/// Each level is half the previous, from the base cycle down to 4 samples.
///
/// The device plays higher notes from the smaller, lower-harmonic levels so they don't alias.
const DIGIPRO_LEVEL_SIZES: &[usize] = &[
    DIGIPRO_CYCLE_LEN,
    DIGIPRO_CYCLE_LEN >> 1,
    DIGIPRO_CYCLE_LEN >> 2,
    DIGIPRO_CYCLE_LEN >> 3,
    DIGIPRO_CYCLE_LEN >> 4,
    DIGIPRO_CYCLE_LEN >> 5,
    DIGIPRO_CYCLE_LEN >> 6,
    DIGIPRO_CYCLE_LEN >> 7,
    DIGIPRO_CYCLE_LEN >> 8,
];

/// Bundles the DigiPro byte-layout data from the device JSON.
struct DigiproLayout {
    offset_version: usize,
    offset_revision: usize,
    offset_slot: usize,
    slot_size: usize,
    offset_name: usize,
    name_size: usize,
    offset_payload: usize,
    encoded_payload_size: usize,
    decoded_payload_size: usize,
    samples_per_waveform: usize,
    slot_min: u8,
    slot_max: u8,
    dump_cmd: u8,
}

impl DigiproLayout {
    /// Reads every offset and size out of the device JSON in one pass.
    ///
    /// A device declaring DigiPro support without its layout is malformed.
    ///
    /// # Panics
    ///
    /// Panics with the missing field's name if any field is absent from `sysex_layout.digipro`.
    fn from_config(dc: &DeviceConfig) -> Self {
        let offset_version = dc.layout_offset_or_die("digipro", "sysex_version") as usize;
        let offset_revision = dc.layout_offset_or_die("digipro", "sysex_revision") as usize;
        let offset_slot = dc.layout_offset_or_die("digipro", "slot") as usize;
        let offset_name = dc.layout_offset_or_die("digipro", "name") as usize;
        let offset_payload = dc.layout_offset_or_die("digipro", "payload") as usize;
        let name_size = as_u64_or_die(dc.json_get("sysex_layout.digipro.fields.name.size")) as usize;
        let encoded_payload_size = as_u64_or_die(dc.json_get("sysex_layout.digipro.fields.payload.wire_len")) as usize;
        let decoded_payload_size = as_u64_or_die(dc.json_get("sysex_layout.digipro.fields.payload.decoded_size")) as usize;
        let slot_size = as_u64_or_die(dc.json_get("sysex_layout.digipro.fields.slot.size")) as usize;
        let slot_min = as_u64_or_die(dc.json_get("sysex_layout.digipro.fields.slot.min")) as u8;
        let slot_max = as_u64_or_die(dc.json_get("sysex_layout.digipro.fields.slot.max")) as u8;
        let dump_cmd = as_u64_or_die(dc.json_get("sysex_api.digipro.write_cmd")) as u8;
        DigiproLayout {
            offset_version,
            offset_revision,
            offset_slot,
            slot_size,
            offset_name,
            name_size,
            offset_payload,
            encoded_payload_size,
            decoded_payload_size,
            samples_per_waveform: decoded_payload_size / BYTES_PER_FRAME,
            slot_min,
            slot_max,
            dump_cmd,
        }
    }
}

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Decodes a DigiPro SysEx message into an int16 single-cycle waveform of `target_len` samples.
pub fn digipro_sysex_to_i16(dc: &DeviceConfig, msg: &[u8], target_len: usize) -> Vec<i16> {
    let layout = DigiproLayout::from_config(dc);
    let decoded = decode_7bit(&msg[layout.offset_payload..layout.offset_payload + layout.encoded_payload_size]);

    // Reconstruct the level-0 base cycle from the first half of the A stream (`base_wave` = even samples, `mipmap_wave` = odd).
    // `DIGIPRO_COEFFICIENT_MAX` is the playback gain the hardware applies. Multiply here so the editor amplitude matches output.
    let mut base_cycle = vec![0.0f32; DIGIPRO_CYCLE_LEN];
    for i in 0..(DIGIPRO_CYCLE_LEN / 2) {
        let offset = 6 * i;
        let a0_val = i16::from_be_bytes([decoded[offset], decoded[offset + 1]]) as f32;
        let a1_val = i16::from_be_bytes([decoded[offset + 3], decoded[offset + 4]]) as f32;
        base_cycle[2 * i] = (a0_val / 32767.0) * DIGIPRO_COEFFICIENT_MAX;
        base_cycle[2 * i + 1] = (a1_val / 32767.0) * DIGIPRO_COEFFICIENT_MAX;
    }

    let resampled = resample_periodic(&base_cycle, target_len);
    resampled
        .iter()
        .map(|&val| (val.clamp(-1.0, 1.0) * 32767.0).round() as i16)
        .collect()
}

/// Renders an int16 single-cycle wave into its base waveform and two mipmaps, then wraps them in a DigiPro SysEx message for `slot`.
pub fn i16_to_digipro_sysex(dc: &DeviceConfig, slot: u8, name: &str, wave: &[i16], ch: u8) -> Vec<u8> {
    let layout = DigiproLayout::from_config(dc);
    let floats = i16_to_floats(wave);
    let (base_wave, mipmap_wave, blend_wave) = render_tables(&floats, layout.samples_per_waveform);
    build_digipro_sysex(dc, slot, name, &base_wave, &mipmap_wave, &blend_wave, ch)
}

/// Returns `true` if the raw contents of a SysEx file contain at least one DigiPro waveform message for this device.
pub fn contains_digipro(dc: &DeviceConfig, bytes: &[u8]) -> bool {
    parse_sysex_file(bytes).iter().any(|packet| is_digipro_packet(dc, packet))
}

/// Decodes every DigiPro waveform message in a raw SysEx buffer into an int16 waveform of `wave_length` samples.
///
/// Returns an empty `Vec` when the buffer holds no DigiPro messages for this device, so callers decide whether that's an error.
pub fn digipro_frames_from_sysex(dc: &DeviceConfig, bytes: &[u8], wave_length: usize) -> Vec<Vec<i16>> {
    parse_sysex_file(bytes)
        .into_iter()
        .filter(|packet| is_digipro_packet(dc, packet))
        .map(|packet| digipro_sysex_to_i16(dc, &packet, wave_length))
        .collect()
}

/// Extracts and trims the waveform name embedded in a raw DigiPro dump, per the device's `sysex_layout.digipro.name` field.
///
/// Returns an empty string if the dump is too short or the name field is all nulls/whitespace.
///
/// The caller supplies a fallback label in that case.
pub fn digipro_name(raw: &[u8], dc: &DeviceConfig) -> String {
    let offset = dc.layout_offset_or_die("digipro", "name") as usize;
    let size = as_u64_or_die(dc.layout_field("digipro", "name").unwrap().json_get("size")) as usize;
    String::from_utf8_lossy(raw.get(offset..offset + size).unwrap_or(&[]))
        .trim_end_matches('\0')
        .trim()
        .to_string()
}

/// Returns `true` if a raw SysEx packet is a DigiPro dump for this device: Elektron header `00 20 3C`, this device's dump command byte.
pub fn is_digipro_packet(dc: &DeviceConfig, packet: &[u8]) -> bool {
    let dump_cmd = as_u64_or_die(dc.json_get("sysex_api.digipro.write_cmd")) as u8;
    packet.len() > ELEKTRON_TYPE_BYTE && packet[1..4] == [0x00, 0x20, 0x3C] && packet[ELEKTRON_TYPE_BYTE] == dump_cmd
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// Constructs a complete DigiPro SysEx message for a specific hardware slot.
///
/// The Monomachine ignores any proprietary SysEx command whose channel byte doesn't match its own, so a wrong `ch` fails without an error.
fn build_digipro_sysex(
    dc: &DeviceConfig,
    slot: u8,
    name: &str,
    base_wave: &[i16],
    mipmap_wave: &[i16],
    blend_wave: &[i16],
    ch: u8,
) -> Vec<u8> {
    let layout = DigiproLayout::from_config(dc);

    // Normalize the waveform name to exactly `name_size` uppercase 7-bit ASCII characters.
    let padded = format!(
        "{:<width$}",
        &name.to_uppercase()[..name.len().min(layout.name_size)],
        width = layout.name_size
    );
    let mut name_bytes: Vec<u8> = padded.bytes().map(|byte_val| byte_val & 0x7F).collect();

    // Package the wavetable data and encode it into the MIDI-safe 7-bit format.
    let raw = pack_interleaved_waveforms(&layout, base_wave, mipmap_wave, blend_wave);
    let encoded = encode_7bit(&raw);
    debug_assert_eq!(encoded.len(), layout.encoded_payload_size);

    let tail_offset = layout.offset_payload + layout.encoded_payload_size;

    // Initialize the SysEx message with hardware headers and metadata.
    let mut msg = vec![0u8; tail_offset + 5];
    msg[0..4].copy_from_slice(&ELEKTRON_MFR);
    msg[ELEKTRON_PROD_BYTE] = dc.prod;
    msg[ELEKTRON_CHANNEL_BYTE] = ch;
    msg[ELEKTRON_TYPE_BYTE] = layout.dump_cmd;
    msg[layout.offset_version] = 0x01;
    msg[layout.offset_revision] = 0x01;
    msg[layout.offset_slot] = slot.clamp(layout.slot_min, layout.slot_max);
    msg[layout.offset_name..layout.offset_name + layout.name_size].copy_from_slice(&name_bytes);
    msg[layout.offset_payload..tail_offset].copy_from_slice(&encoded);

    // Calculate and inject the 14-bit checksum for data integrity.
    name_bytes.extend_from_slice(&encoded);
    let checksum = digipro_checksum_14bit(&name_bytes);
    msg[tail_offset] = ((checksum >> 7) & 0x7F) as u8;
    msg[tail_offset + 1] = (checksum & 0x7F) as u8;

    // Total payload length, used by the hardware to verify completion: version + revision (2) + slot + name + payload + checksum (2).
    let len14: u32 = (2 + layout.slot_size + layout.name_size + layout.encoded_payload_size + 2) as u32;
    msg[tail_offset + 2] = ((len14 >> 7) & 0x7F) as u8;
    msg[tail_offset + 3] = (len14 & 0x7F) as u8;

    msg[tail_offset + 4] = 0xF7; // Append the standard MIDI End of Exclusive (EOX) byte.
    msg
}

/// Renders a single audio cycle into the multi-level spectral tables required by the DigiPro format.
fn render_tables(cycle_floats: &[f32], samples_per_waveform: usize) -> (Vec<i16>, Vec<i16>, Vec<i16>) {
    // Handle empty input by returning silent buffers.
    if cycle_floats.is_empty() {
        let silent = vec![0i16; samples_per_waveform];
        return (silent.clone(), silent.clone(), silent);
    }

    // DC removal. Subtracts the cycle's mean before anything else.
    //
    // Roughly the same algorithm as C6.
    let mut cycle_floats_vec = cycle_floats.to_vec();
    let mean = cycle_floats_vec.iter().sum::<f32>() / cycle_floats_vec.len() as f32;
    for val in &mut cycle_floats_vec {
        *val -= mean;
    }

    // Resample the DC-removed cycle to the 1024-point processing base, then convert to the frequency domain.
    let mut spectrum_real = resample_periodic(&cycle_floats_vec, DIGIPRO_CYCLE_LEN);
    let mut spectrum_imaginary = vec![0.0f32; DIGIPRO_CYCLE_LEN];
    fft_inplace(&mut spectrum_real, &mut spectrum_imaginary, false);

    // Quadratic high-frequency rolloff on bins 351..=511 (and their negative-frequency mirrors), with Nyquist zeroed.
    // This band-limits the top of the spectrum the way the device expects, so playback doesn't alias.
    //
    // Roughly the same algorithm as C6.
    for bin in 351..=511 {
        let weight = (512 - bin) as f32 / 162.0;
        let gain = weight * weight;
        spectrum_real[bin] *= gain;
        spectrum_imaginary[bin] *= gain;
        let mirror_bin = DIGIPRO_CYCLE_LEN - bin;
        spectrum_real[mirror_bin] *= gain;
        spectrum_imaginary[mirror_bin] *= gain;
    }
    spectrum_real[512] = 0.0;
    spectrum_imaginary[512] = 0.0;

    // Parity normalization. Divide the spectrum by (post-rolloff time-domain peak * `DIGIPRO_COEFFICIENT_MAX`).
    // Normalizing by the actual peak (not a constant) removes per-wave loudness differences.
    // Every wave lands at the same headroom, matching C6.
    //
    // The minimum-peak guard avoids amplifying near-silence into noise.
    //
    // Roughly the same algorithm as C6.
    let peak = {
        let mut time_domain_real = spectrum_real.clone();
        let mut time_domain_imaginary = spectrum_imaginary.clone();
        fft_inplace(&mut time_domain_real, &mut time_domain_imaginary, true);
        time_domain_real.iter().fold(0.0f32, |max_val, &val| max_val.max(val.abs()))
    };
    let min_peak = 1.0 / 65536.0;
    let mut divisor = if !peak.is_finite() || peak < min_peak {
        1.0
    } else {
        peak * DIGIPRO_COEFFICIENT_MAX
    };
    if !divisor.is_finite() || divisor <= 0.0 {
        divisor = 1.0;
    }
    for val in &mut spectrum_real {
        *val /= divisor;
    }
    for val in &mut spectrum_imaginary {
        *val /= divisor;
    }

    let mut unfiltered_out = vec![0.0f32; 2044];
    let mut filtered_out = vec![0.0f32; 2044];
    let mut out_offset = 0usize;

    // Sized for the largest mipmap level, then reused for the smaller ones by slicing to `level_len`.
    // This avoids 36 heap allocations in the hot path.
    let mut unfiltered_real = vec![0.0f32; DIGIPRO_CYCLE_LEN];
    let mut unfiltered_imaginary = vec![0.0f32; DIGIPRO_CYCLE_LEN];
    let mut filtered_real = vec![0.0f32; DIGIPRO_CYCLE_LEN];
    let mut filtered_imaginary = vec![0.0f32; DIGIPRO_CYCLE_LEN];

    // Downsample the waveform through 8 mipmap levels using a spectral window to strictly prevent aliasing artifacts.
    for &level_len in DIGIPRO_LEVEL_SIZES {
        let half_len = level_len >> 1;
        let scale = level_len as f32 / DIGIPRO_CYCLE_LEN as f32;
        let cutoff = level_len >> 2;

        unfiltered_real[..level_len].fill(0.0);
        unfiltered_imaginary[..level_len].fill(0.0);
        unfiltered_real[0] = spectrum_real[0];
        unfiltered_imaginary[0] = spectrum_imaginary[0];
        for bin in 1..half_len {
            unfiltered_real[bin] = spectrum_real[bin];
            unfiltered_imaginary[bin] = spectrum_imaginary[bin];
            unfiltered_real[level_len - bin] = spectrum_real[DIGIPRO_CYCLE_LEN - bin];
            unfiltered_imaginary[level_len - bin] = spectrum_imaginary[DIGIPRO_CYCLE_LEN - bin];
        }
        unfiltered_real[half_len] = 0.0;
        unfiltered_imaginary[half_len] = 0.0;
        fft_inplace(&mut unfiltered_real[..level_len], &mut unfiltered_imaginary[..level_len], true);
        for sample_idx in 0..level_len {
            unfiltered_out[out_offset + sample_idx] = unfiltered_real[sample_idx] * scale;
        }

        filtered_real[..level_len].fill(0.0);
        filtered_imaginary[..level_len].fill(0.0);
        filtered_real[0] = spectrum_real[0];
        filtered_imaginary[0] = spectrum_imaginary[0];
        for bin in 1..half_len {
            if bin < cutoff {
                filtered_real[bin] = spectrum_real[bin];
                filtered_imaginary[bin] = spectrum_imaginary[bin];
                filtered_real[level_len - bin] = spectrum_real[DIGIPRO_CYCLE_LEN - bin];
                filtered_imaginary[level_len - bin] = spectrum_imaginary[DIGIPRO_CYCLE_LEN - bin];
            }
        }
        filtered_real[half_len] = 0.0;
        filtered_imaginary[half_len] = 0.0;
        fft_inplace(&mut filtered_real[..level_len], &mut filtered_imaginary[..level_len], true);
        for sample_idx in 0..level_len {
            filtered_out[out_offset + sample_idx] = filtered_real[sample_idx] * scale;
        }

        out_offset += level_len;
    }

    let mut base_wave = vec![0i16; samples_per_waveform];
    let mut mipmap_wave = vec![0i16; samples_per_waveform];
    let mut blend_wave = vec![0i16; samples_per_waveform];
    for sample_idx in 0..samples_per_waveform {
        base_wave[sample_idx] = truncate_to_i16(unfiltered_out[2 * sample_idx]);
        mipmap_wave[sample_idx] = truncate_to_i16(unfiltered_out[2 * sample_idx + 1]);
        blend_wave[sample_idx] = if sample_idx < 1020 {
            truncate_to_i16(filtered_out[DIGIPRO_CYCLE_LEN + sample_idx])
        } else {
            0
        };
    }

    (base_wave, mipmap_wave, blend_wave)
}

/// Converts int16 waveform samples (full-scale ±32767) into normalized floats in [-1, 1].
fn i16_to_floats(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&sample| sample as f32 / 32767.0).collect()
}

/// Computes the 14-bit Elektron checksum by summing the low 7 bits of every byte and masking the total to 14 bits.
fn digipro_checksum_14bit(data: &[u8]) -> u16 {
    (data.iter().map(|&byte_val| (byte_val & 0x7F) as u32).sum::<u32>() & ELEKTRON_CHECKSUM_MASK) as u16
}

/// Intertwines three waveform streams into a single decoded DigiPro payload.
///
/// Wire layout per 6-byte frame is `[base_hi, base_lo, blend_hi, mipmap_hi, mipmap_lo, blend_lo]`.
///
/// Hardware-mandated interleave order: `base_wave` = base waveform, `mipmap_wave` = first mipmap, `blend_wave` = blend mipmap.
fn pack_interleaved_waveforms(layout: &DigiproLayout, base_wave: &[i16], mipmap_wave: &[i16], blend_wave: &[i16]) -> Vec<u8> {
    let mut out = vec![0u8; layout.decoded_payload_size];
    let cast = |val: i16| -> u16 { val as u16 };

    for i in 0..layout.samples_per_waveform {
        let base: u16 = cast(base_wave[i]);
        let blend: u16 = cast(blend_wave[i]);
        let mipmap: u16 = cast(mipmap_wave[i]);
        let offset = 6 * i;
        out[offset] = (base >> 8) as u8;
        out[offset + 1] = base as u8;
        out[offset + 2] = (blend >> 8) as u8;
        out[offset + 3] = (mipmap >> 8) as u8;
        out[offset + 4] = mipmap as u8;
        out[offset + 5] = blend as u8;
    }
    out
}

/// Performs an in-place Fast Fourier Transform on the provided real and imaginary data.
fn fft_inplace(real: &mut [f32], imaginary: &mut [f32], is_inverse: bool) {
    let num_samples = real.len();
    let mut reversed_idx = 0usize;

    // Stage 1: Bit-reversal permutation (shuffles the input data into the correct order for the butterfly stages)
    for sample_idx in 1..num_samples {
        let mut bit_mask = num_samples >> 1;
        while reversed_idx & bit_mask != 0 {
            reversed_idx ^= bit_mask;
            bit_mask >>= 1;
        }
        reversed_idx ^= bit_mask;
        if sample_idx < reversed_idx {
            real.swap(sample_idx, reversed_idx);
            imaginary.swap(sample_idx, reversed_idx);
        }
    }

    // Stage 2: Cooley-Tukey Butterfly algorithm (iterative decimation-in-time calculation)
    let mut stage_len = 2usize;
    while stage_len <= num_samples {
        let half_stage_len = stage_len >> 1;
        let angle = (2.0 * PI / stage_len as f32) * if is_inverse { 1.0 } else { -1.0 };
        let twiddle_step_real = angle.cos();
        let twiddle_step_imaginary = angle.sin();
        for start in (0..num_samples).step_by(stage_len) {
            let mut twiddle_real = 1.0f32;
            let mut twiddle_imaginary = 0.0f32;
            for pair_idx in 0..half_stage_len {
                let even_idx = start + pair_idx;
                let odd_idx = even_idx + half_stage_len;
                let even_real = real[even_idx];
                let even_imaginary = imaginary[even_idx];
                // Apply complex twiddle factor.
                let odd_real = real[odd_idx] * twiddle_real - imaginary[odd_idx] * twiddle_imaginary;
                let odd_imaginary = real[odd_idx] * twiddle_imaginary + imaginary[odd_idx] * twiddle_real;
                // Perform butterfly summation and subtraction.
                real[even_idx] = even_real + odd_real;
                imaginary[even_idx] = even_imaginary + odd_imaginary;
                real[odd_idx] = even_real - odd_real;
                imaginary[odd_idx] = even_imaginary - odd_imaginary;
                // Update twiddle factor for the next iteration in this block.
                let next_twiddle_real = twiddle_real * twiddle_step_real - twiddle_imaginary * twiddle_step_imaginary;
                twiddle_imaginary = twiddle_real * twiddle_step_imaginary + twiddle_imaginary * twiddle_step_real;
                twiddle_real = next_twiddle_real;
            }
        }
        stage_len <<= 1;
    }

    // Stage 3: Normalization (divide by N if performing an Inverse FFT)
    if is_inverse {
        let normalization_scale = 1.0 / num_samples as f32;
        for sample_idx in 0..num_samples {
            real[sample_idx] *= normalization_scale;
            imaginary[sample_idx] *= normalization_scale;
        }
    }
}

/// Linearly interpolates one audio cycle to `target_len` samples, the fixed length the DigiPro format stores.
///
/// Rescaling a pitched waveform to a fixed length is lossy, so some quality is always given up here.
fn resample_periodic(src: &[f32], target_len: usize) -> Vec<f32> {
    let src_len = src.len();

    if src_len == 0 {
        return vec![0.0; target_len];
    }

    let mut out = vec![0.0f32; target_len];

    // Linearly interpolate the source audio cycle to fit the DigiPro sample count.
    for (sample_idx, out_val) in out.iter_mut().enumerate() {
        let sample_pos = sample_idx as f32 * src_len as f32 / target_len as f32;
        let floor_pos = sample_pos.floor() as usize;
        let idx_lower = floor_pos % src_len;
        let idx_upper = (idx_lower + 1) % src_len;
        let fraction = sample_pos - floor_pos as f32;
        *out_val = src[idx_lower] * (1.0 - fraction) + src[idx_upper] * fraction;
    }

    out
}

/// Clamps a normalized float to [-1.0, 1.0] and scales it to a full-scale i16.
fn truncate_to_i16(float_val: f32) -> i16 {
    let val = float_val.clamp(-1.0, 1.0);
    (val * 32767.0).clamp(-32767.0, 32767.0) as i16
}
