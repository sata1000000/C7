//! Functions for MIDI Sample Dump Standard (SDS) transfers.
//!
//! Inside a `.c7` file a sample is stored as FLAC+base64 rather than as raw SDS packets.

use std::thread::sleep;
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::STANDARD as B64};

use crate::c7_file_interfacing::write_sample_c7;
use crate::device_config::DeviceConfig;
use crate::midi::{MidiSender, PollableMidiInput};
use crate::sysex::{ELEKTRON_PAYLOAD_START, ELEKTRON_TYPE_BYTE, build_elektron_sysex, is_elektron_sysex};
use crate::utils::{as_string_or_die, as_u64_or, as_u64_or_die, bytes_to_hex_string, find_byte, from_hex, temp_path};

/// Loop type byte in the SDS Dump Header.
pub const SDS_LOOP_OFF: u8 = 0x7F;

// Communication codes for sample transfers.
const SDS_ACK: u8 = 0x7F;
const SDS_NAK: u8 = 0x7E;
const SDS_CANCEL: u8 = 0x7D;
const SDS_WAIT: u8 = 0x7C;

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Returns `true` if the raw message is any SDS transfer message (`$01` or `$02`).
pub fn is_sds_message(raw: &[u8]) -> bool {
    raw.len() > 3 && raw[1] == 0x7E && (raw[3] == 0x01 || raw[3] == 0x02)
}

/// Returns the sample number carried by an SDS Dump Header (`$01`).
///
/// Bytes 4-5 hold it as two 7-bit halves, LSB first.
///
/// Returns `None` for any message that isn't a Dump Header, which is also how a caller spots where one sample's packet run begins.
pub fn sds_sample_number(raw: &[u8]) -> Option<usize> {
    if !is_sds_dump_header(raw) || raw.len() < 6 {
        return None;
    }
    Some(usize::from(raw[4]) | (usize::from(raw[5]) << 7))
}

/// Scales depth-native SDS samples up to full-scale i16 audio.
///
/// `decode_sds()` keeps samples at the source's own depth so nothing is lost on the way into storage.
/// Playback, waveform drawing, and WAV export are all i16, so this is the boundary where a dump above 16 bits gives up its low bits.
pub fn sds_samples_to_i16(samples: &[i32], significant_bits: u8) -> Vec<i16> {
    if significant_bits >= 16 {
        let shift = u32::from(significant_bits) - 16;
        samples.iter().map(|&sample_val| (sample_val >> shift) as i16).collect()
    } else {
        let shift = 16 - u32::from(significant_bits);
        samples.iter().map(|&sample_val| (sample_val << shift) as i16).collect()
    }
}

/// Returns the Dump Header (`$01`) message from a raw SDS stream.
///
/// Every field describing the sample lives here, so storing it unchanged is what lets a rebuilt stream match the original.
///
/// Returns `None` when the stream carries only data packets.
pub fn sds_dump_header(data: &[u8]) -> Option<&[u8]> {
    let mut pos = 0usize;
    while pos < data.len() {
        let f0 = pos + data[pos..].iter().position(|&byte| byte == 0xF0)?;
        let f7 = f0 + data[f0..].iter().position(|&byte| byte == 0xF7)?;
        if is_sds_dump_header(&data[f0..=f7]) {
            return Some(&data[f0..=f7]);
        }
        pos = f7 + 1;
    }
    None
}

/// Fetches all sample slot names from a Machinedrum in a single round trip.
///
/// Returns a vec of `(name, occupied)` for each slot. `name` is the 4-char ASCII label.
/// Returns `None` on timeout or if the device is a non-UW model (which sends `count = 0`).
pub fn fetch_md_sample_names(
    midi_in: &PollableMidiInput,
    midi_out: &mut MidiSender,
    prod: u8,
    ch: u8,
    timeout: f64,
) -> Option<Vec<(String, bool)>> {
    midi_in.flush();
    let request = build_elektron_sysex(prod, ch, &[0x70, 0x34]);
    midi_out.sysex(&request);

    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    while Instant::now() < deadline {
        if let Some(data) = midi_in.poll() {
            // Response: `F0 00 20 3C [prod] [ch] 0x72 0x34 [count] [5*count bytes] F7`
            if is_elektron_sysex(&data, prod) && data.len() >= 9 && data[ELEKTRON_TYPE_BYTE] == 0x72 && data[ELEKTRON_PAYLOAD_START] == 0x34
            {
                let count = data[8] as usize;
                let expected_len = 9 + count * 5 + 1; // +1 for F7
                if data.len() < expected_len {
                    return None;
                }
                let mut slots = Vec::with_capacity(count);
                for i in 0..count {
                    let base = 9 + i * 5;
                    let name: String = data[base..base + 4]
                        .iter()
                        // `0x20` (32) is the space character. `0x7e` (126) is the `~` tilde character.
                        .map(|&byte| if (32..=126).contains(&byte) { byte as char } else { ' ' })
                        .collect::<String>()
                        .trim()
                        .to_string();
                    let occupied = data[base + 4] != 0;
                    slots.push((name, occupied));
                }
                return Some(slots);
            }
        }
        sleep(Duration::from_millis(5));
    }
    None
}

/// Probes a hardware sample slot for existence and name metadata using SDS request/dump logic.
pub fn probe_sds_slot(
    midi_in: &PollableMidiInput,
    midi_out: &mut MidiSender,
    slot: u8,
    ch: u8,
    prod: u8,
    timeout: f64,
) -> (Option<String>, Option<Vec<u8>>, bool) {
    midi_in.flush();
    midi_out.sysex(&[0xF0, 0x7E, ch, 0x03, slot & 0x7F, 0x00, 0xF7]);

    let mut deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let mut name_str: Option<String> = None;
    let mut name_blob: Option<Vec<u8>> = None;
    let mut has_data_packet = false;

    while Instant::now() < deadline {
        if let Some(data) = midi_in.poll() {
            if is_sds_dump_header(&data) {
                let sds_ch = data[2];
                midi_out.sysex(&make_sds_confirmation(sds_ch, 0));
                deadline = Instant::now() + Duration::from_secs(1);
            } else if is_elektron_sysex(&data, prod)
                && data.len() > 8
                && data[ELEKTRON_TYPE_BYTE] == 0x73
                && data[ELEKTRON_PAYLOAD_START] == (slot & 0x7F)
            {
                // Name tag.
                let name_bytes = &data[8..data.len().saturating_sub(1)];
                name_str = Some(
                    name_bytes
                        .iter()
                        // `0x20` (32) is the space character. `0x7e` (126) is the `~` tilde character.
                        .filter(|&&char_byte| (32..=126).contains(&char_byte))
                        .map(|&char_byte| char_byte as char)
                        .collect::<String>()
                        .trim()
                        .to_string(),
                );
                name_blob = Some(data.clone());
            } else if is_sds_data_packet(&data) {
                let sds_ch = data[2];
                let packet_num = data[4];
                // Tell the device to stop sending data since only the name was needed.
                midi_out.sysex(&[0xF0, 0x7E, sds_ch, 0x7D, packet_num, 0xF7]);
                has_data_packet = true;
                break;
            }
        }
        sleep(Duration::from_millis(5));
    }
    (name_str, name_blob, has_data_packet)
}

/// Positions the device's internal sample cursor at `slot` by requesting a dump then cancelling.
///
/// Required before an unsolicited SDS upload.
/// The MD routes incoming SDS data to whatever slot its cursor is on, not the slot number in the SDS Dump Header.
///
/// Empty slots receive no response, so execution falls through after the timeout.
pub fn seek_sds_slot(midi_in: &PollableMidiInput, midi_out: &mut MidiSender, slot: u8, ch: u8) {
    midi_in.flush();
    midi_out.sysex(&[0xF0, 0x7E, ch, 0x03, slot & 0x7F, 0x00, 0xF7]);

    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        if let Some(data) = midi_in.poll()
            && is_sds_dump_header(&data)
        {
            let sds_ch = data[2];
            midi_out.sysex(&[0xF0, 0x7E, sds_ch, 0x7D, 0x00, 0xF7]);
            break;
        }
        sleep(Duration::from_millis(2));
    }
    midi_in.flush();
    sleep(Duration::from_millis(80));
}

/// Executes a full SDS download and Base64-FLAC compression pipeline for a hardware sample.
///
/// Returns `(compressed_blob, display_name)`.
///
/// The name is extracted from the Elektron name tag if the device sends one. Otherwise, the name is `None`.
pub fn fetch_and_compress_sample(
    midi_in: &PollableMidiInput,
    midi_out: &mut MidiSender,
    slot: u8,
    dc: &DeviceConfig,
    ch: u8,
    status_cb: impl Fn(&str),
) -> (Option<String>, Option<String>) {
    let display_offset = as_u64_or(dc.json_get("sysex_api.sample.display_offset"), 0) as u8;

    midi_in.flush();
    status_cb(&format!("Fetching audio for sample {:02}...", slot + display_offset));
    midi_out.sysex(&[0xF0, 0x7E, ch, 0x03, slot & 0x7F, 0x00, 0xF7]);

    let mut sds_data = Vec::<u8>::new();
    let mut deadline = Instant::now() + Duration::from_secs(2);
    let mut received_packets = 0usize;
    let mut name_str: Option<String> = None;

    while Instant::now() < deadline {
        while let Some(data) = midi_in.poll() {
            if is_sds_dump_header(&data) {
                let sds_ch = data[2];
                sds_data.extend_from_slice(&data);
                midi_out.sysex(&make_sds_confirmation(sds_ch, 0));
                deadline = Instant::now() + Duration::from_secs(2);
            } else if is_elektron_sysex(&data, dc.prod) && data.len() > 8 && data[ELEKTRON_TYPE_BYTE] == 0x73 {
                let name_bytes = &data[8..data.len().saturating_sub(1)];
                name_str = Some(
                    name_bytes
                        .iter()
                        // `0x20` (32) is the space character. `0x7e` (126) is the `~` tilde character.
                        .filter(|&&char_byte| (32..=126).contains(&char_byte))
                        .map(|&char_byte| char_byte as char)
                        .collect::<String>()
                        .trim()
                        .to_string(),
                );
            } else if is_sds_data_packet(&data) {
                let sds_ch = data[2];
                let packet_num = data[4];
                if data.len() < 127 {
                    // Pad short packets to the standard 127-byte size before storing.
                    let mut header = data[..5].to_vec();
                    let mut payload = data[5..data.len().saturating_sub(2)].to_vec();
                    payload.resize(120, 0);
                    header.extend_from_slice(&payload);
                    header.push(0x00); // checksum placeholder
                    header.push(0xF7);
                    let checksum = calculate_sds_checksum(&header);
                    let end = header.len();
                    header[end - 2] = checksum;
                    sds_data.extend_from_slice(&header);
                } else {
                    sds_data.extend_from_slice(&data);
                }

                midi_out.sysex(&make_sds_confirmation(sds_ch, packet_num));
                deadline = Instant::now() + Duration::from_secs(1);
                received_packets += 1;
                if received_packets.is_multiple_of(10) {
                    status_cb(&format!(
                        "Receiving audio for sample {:02}... packet {}.",
                        slot + display_offset,
                        received_packets
                    ));
                }
            }
        }
        sleep(Duration::from_millis(2));
    }

    if sds_data.is_empty() {
        return (None, name_str);
    }

    status_cb(&format!("Compressing sample {:02}...", slot + display_offset));

    let blob = compress_sds_to_b64(&sds_data).ok();
    (blob, name_str)
}

/// Parses raw SDS data and injects target slot and name metadata into a packet sequence.
pub fn build_sds_sample_packets<F>(
    sds_data: &[u8],
    slot: u8,
    ch: u8,
    dc: &DeviceConfig,
    custom_name: &str,
    cancel_check: Option<&F>,
) -> Vec<Vec<u8>>
where
    F: Fn() -> bool,
{
    let mut packets = Vec::new();
    let mut start = 0usize;

    while start < sds_data.len() {
        if let Some(ref cancel_check) = cancel_check
            && cancel_check()
        {
            break;
        }

        let Some(f0_idx) = find_byte(sds_data, 0xF0, start) else {
            break;
        };
        let Some(f7_idx) = find_byte(sds_data, 0xF7, f0_idx) else {
            break;
        };

        let mut packet = sds_data[f0_idx..=f7_idx].to_vec();

        if packet.len() > 3 && packet[1] == 0x7E {
            packet[2] = ch;
            if packet[3] == 0x01 {
                // Header packet: inject target slot, then attach the name message this device answers to.
                packet[4] = slot & 0x7F;
                packets.push(packet);
                packets.push(build_sample_name_message(dc, slot, ch, custom_name));

                start = f7_idx + 1;
                continue;
            } else if packet[3] == 0x02 {
                // Data packet: recompute checksum after channel byte update.
                let checksum: u8 = packet[1..packet.len().saturating_sub(2)].iter().fold(0u8, |acc, &byte| acc ^ byte);
                let end = packet.len();
                packet[end - 2] = checksum & 0x7F;
                packets.push(packet);
            }
        }
        start = f7_idx + 1;
    }
    packets
}

/// Transmits a sequence of SDS packets with closed-loop ACK verification for reliable delivery.
pub fn send_sds_sample_packets<FC, FS, FP>(
    midi_in: &PollableMidiInput,
    midi_out: &mut MidiSender,
    packets: &[Vec<u8>],
    cancel_check: Option<&FC>,
    status_cb: Option<&FS>,
    progress_cb: Option<&FP>,
    display_name: &str,
) -> bool
where
    FC: Fn() -> bool,
    FS: Fn(&str),
    FP: Fn(usize, usize),
{
    let total = packets.len();
    for (i, packet) in packets.iter().enumerate() {
        if let Some(ref cancel_check) = cancel_check
            && cancel_check()
        {
            let ch = packets.first().and_then(|packet_ref| packet_ref.get(2)).copied().unwrap_or(0x7F);
            midi_out.sysex(&[0xF0, 0x7E, ch, 0x7D, 0x00, 0xF7]);
            return false;
        }

        let needs_ack = packet.len() > 3 && packet[1] == 0x7E && packet[3] == 0x02;

        midi_out.sysex(packet);

        if is_sds_dump_header(packet) {
            // Wait for the device to ACK the Dump Header before sending the Elektron name SysEx (next packet).
            // That way it's already in receive mode when the name command arrives.
            //
            // Non-fatal on timeout: the device may be in non-handshake mode, in which case the transfer proceeds and the name still lands.
            let mut deadline = Instant::now() + Duration::from_millis(500);
            'hdr_ack: while Instant::now() < deadline {
                if let Some(data) = midi_in.poll()
                    && data.len() >= 5
                    && data[1] == 0x7E
                {
                    match data[3] {
                        SDS_ACK => {
                            break 'hdr_ack;
                        }
                        SDS_WAIT => {
                            deadline = Instant::now() + Duration::from_millis(500);
                        }
                        SDS_CANCEL => {
                            return false;
                        }
                        _ => {}
                    }
                }
                sleep(Duration::from_millis(2));
            }
            midi_in.flush();
        } else if needs_ack {
            let mut deadline = Instant::now() + Duration::from_millis(500);
            let mut has_ack = false;
            'ack: while Instant::now() < deadline {
                if let Some(data) = midi_in.poll()
                    && data.len() >= 5
                    && data[1] == 0x7E
                {
                    match data[3] {
                        SDS_ACK => {
                            has_ack = true;
                            break 'ack;
                        }
                        SDS_NAK => {
                            // NAK: resend.
                            midi_out.sysex(packet);
                            deadline = Instant::now() + Duration::from_millis(500);
                        }
                        SDS_WAIT => {
                            // WAIT: extend.
                            deadline = Instant::now() + Duration::from_millis(500);
                        }
                        SDS_CANCEL => {
                            return false;
                        }
                        _ => {}
                    }
                }
                sleep(Duration::from_millis(2));
            }
            if !has_ack {
                return false;
            }
            // Brief pause so the MD can finish its internal state transition after ACK before the next packet fires.
            // Avoids spurious CANCEL on `packet_num`=1+.
            sleep(Duration::from_millis(20));
            // Drain any stale MD responses that arrived during the pause.
            midi_in.flush();
        }

        if i % 10 == 0 {
            if let Some(ref cb) = status_cb {
                cb(&format!("Writing '{display_name}'... packet {i}/{total}"));
            }
            if let Some(ref cb) = progress_cb {
                cb(i, total);
            }
        }
    }
    true
}

/// Normalizes a short SDS data packet to the 120-byte payload expected by some tools.
pub fn normalize_sds_data_packet(data: Vec<u8>) -> Vec<u8> {
    if data.len() >= 127 {
        // Store full-sized SDS packets directly.
        return data;
    }
    // Normalize short packets: header=[0..5], payload=[5..-2], padded to 120 bytes.
    let header = &data[..5.min(data.len())];
    let payload_raw = if data.len() > 7 { &data[5..data.len() - 2] } else { &[] as &[u8] };
    let mut payload_raw_vec = payload_raw.to_vec();
    payload_raw_vec.resize(120, 0);

    let mut header_vec = header.to_vec();
    header_vec.extend_from_slice(&payload_raw_vec);
    header_vec.push(0x00); // checksum placeholder
    header_vec.push(0xF7);
    let checksum = calculate_sds_checksum(&header_vec);
    let last = header_vec.len() - 2;
    header_vec[last] = checksum;
    header_vec
}

/// Consolidates captured SDS packets and writes the selected output formats (`.sds`, `.wav`, `.c7`).
///
/// Returns the saved file names.
pub fn save_received_sample(
    packets: &[Vec<u8>],
    base_path: &std::path::Path,
    should_save_wav: bool,
    should_save_sds: bool,
    should_save_c7: bool,
    name: &str,
    device_short: &str,
) -> Result<Vec<String>, String> {
    // Perform file system operations for saving and converting the sample.
    let clean_base = base_path.with_extension("");
    let mut saved_files = vec![];

    // Consolidate captured SDS SysEx packets into one binary blob.
    let consolidated: Vec<u8> = packets.iter().flat_map(|packet| packet.iter().copied()).collect();

    // Write received packets to disk as a raw SDS file.
    // For WAV-only mode, write to a temp file that will be cleaned up after conversion.
    let sds_path = if should_save_sds {
        clean_base.with_extension("sds")
    } else {
        temp_path("sds", "sds")
    };

    std::fs::write(&sds_path, &consolidated).map_err(|e| e.to_string())?;

    if should_save_sds {
        let name = sds_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        saved_files.push(name);
    }

    if should_save_wav {
        let wav_path = clean_base.with_extension("wav");
        let (depth_native, sample_rate, bits) = decode_sds(&consolidated)?;
        let samples = sds_samples_to_i16(&depth_native, bits);
        // Reuses the loop extraction `upload_sample.rs` relies on (via `load_to_i16()`).
        // So captured loop points survive the WAV export instead of being dropped.
        let loopdata = read_sds_loopdata(&sds_path.to_string_lossy());
        write_wav(&wav_path, &samples, sample_rate, loopdata)?;
        let name = wav_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        saved_files.push(name);
    }

    if should_save_c7 {
        let c7_path = clean_base.with_extension("c7");
        write_sample_c7(&c7_path, &consolidated, name, device_short)?;
        let saved_name = c7_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        saved_files.push(saved_name);
    }

    // Ensure temporary intermediate SDS files are cleaned up.
    if !should_save_sds && sds_path.exists() {
        let _ = std::fs::remove_file(&sds_path);
    }

    Ok(saved_files)
}

/// Reads the WAV `smpl` chunk and returns `(loopdata_type, loopdata_start, loopdata_end)`.
///
/// Loop points are in source-file sample units, so scale them to the target rate before passing to `audio_to_sds()`.
/// WAV loop type 0 (forward) → SDS `0x00`; type 1 (ping-pong) → SDS `0x01`; others → `None`.
///
/// Returns `None` if the file has no `smpl` chunk, no loop entries, or isn't a WAV file.
pub fn read_wav_loopdata(path: &str) -> Option<(u8, u32, u32)> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }

    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        // Sample loop chunk layout: 36 bytes of header, then 24 bytes per loop entry.
        // Offset 28 = `NumSampleLoops`; offset 36 = first loop entry.
        if chunk_id == b"smpl" && chunk_size >= 60 && pos + chunk_size <= data.len() {
            let num_loopdata_entries = u32::from_le_bytes(data[pos + 28..pos + 32].try_into().unwrap()) as usize;
            if num_loopdata_entries > 0 {
                let loopdata_entry_offset = pos + 36;
                let wav_type = u32::from_le_bytes(data[loopdata_entry_offset + 4..loopdata_entry_offset + 8].try_into().unwrap());
                let start = u32::from_le_bytes(data[loopdata_entry_offset + 8..loopdata_entry_offset + 12].try_into().unwrap());
                let end = u32::from_le_bytes(data[loopdata_entry_offset + 12..loopdata_entry_offset + 16].try_into().unwrap());
                let sds_type = match wav_type {
                    0 => 0x00u8, // forward
                    1 => 0x01u8, // ping-pong / bidirectional
                    _ => return None,
                };
                return Some((sds_type, start, end));
            }
        }
        pos += chunk_size + (chunk_size & 1); // RIFF chunks are word-aligned
    }

    None
}

/// Returns the bit depth to upload samples at.
///
/// Storage is unrelated: `compress_sds_to_b64()` keeps whatever depth the source SDS declared.
/// A dump is never quantized going into a `.c7`.
///
/// Rounds the sampler's native depth up to the top of its SDS word-size bracket (most bits, no file-size increase).
/// `highest_accepted` caps how far it can round up.
///
/// MD: 12-bit sampler maps to 14 (2 bytes/word), capped at 16. `ceil(bits/7) * 7` is the bracket top.
pub fn resolve_upload_bit_depth(dc: &DeviceConfig) -> u8 {
    let native_bit_depth = as_u64_or_die(dc.json_get("sampler.native_bit_depth")) as u8;
    let highest_accepted = as_u64_or_die(dc.json_get("sampler.highest_accepted_bit_depth")) as u8;
    let bytes_per_word = (native_bit_depth as usize).div_ceil(7);
    ((bytes_per_word * 7) as u8).min(highest_accepted)
}

/// Reverses the compression pipeline, decoding Base64-FLAC back into raw SDS binary data.
///
/// The source's Dump Header is recovered from the custom `SDS_DUMP_HEADER` Vorbis comment tag and copied through.
/// So the period, sample number, channel, and loop points all match the original.
/// A FLAC without that tag is rejected rather than rebuilt from, since nothing but `compress_sds_to_b64()` ever writes one.
/// The word width comes from the depth the header declares, not the FLAC's, which may have been padded up to one flacenc accepts.
pub fn decode_flac_b64_to_sds(blob: &str) -> Result<Vec<u8>, String> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::{FormatOptions, probe::Hint};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let flac_data = B64.decode(blob).map_err(|e| format!("Base64 decode failed: {e}"))?;

    let mss = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(flac_data)),
        symphonia::core::io::MediaSourceStreamOptions::default(),
    );
    let mut hint = Hint::new();
    hint.with_extension("flac");

    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("FLAC probe failed: {e}"))?;

    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.as_ref().and_then(|param| param.audio()).is_some())
        .ok_or("FLAC: no audio track found")?;
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|param| param.audio())
        .ok_or("FLAC: no audio codec params")?
        .clone();
    let bit_depth = audio_params.bits_per_sample.ok_or("FLAC: missing bit depth")? as u8;

    let dump_header = format
        .metadata()
        .current()
        .and_then(|rev| read_sds_dump_header_vorbis_comment(&rev.media.tags))
        .ok_or("FLAC: missing SDS_DUMP_HEADER tag")?;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| format!("FLAC decoder init failed: {e}"))?;

    let mut samples: Vec<i32> = Vec::new();
    while let Ok(Some(packet)) = format.next_packet() {
        if packet.track_id != track_id {
            continue;
        }
        let Ok(audio_buf) = decoder.decode(&packet) else { continue };
        // `copy_to_vec_interleaved` resizes/overwrites its destination.
        // So collect each packet into a scratch buffer before appending it to the running total.
        let mut chunk: Vec<i32> = Vec::new();
        audio_buf.copy_to_vec_interleaved(&mut chunk);
        samples.extend_from_slice(&chunk);
    }

    if samples.is_empty() {
        return Err("FLAC decode: no samples found".to_string());
    }

    // Dump Header byte 6 is the depth the packets must be repacked at, so a stream whose STREAMINFO disagrees would silently corrupt them.
    let header_bits = dump_header[6];
    let expected_bits = flac_storage_bits(header_bits);
    if bit_depth != expected_bits {
        return Err(format!(
            "FLAC bit depth {bit_depth} is not the {expected_bits} expected for the Dump Header's {header_bits}"
        ));
    }

    // One shift undoes both symphonia's scaling to the destination type's full range.
    // It also undoes any padding `compress_sds_to_b64()` added for flacenc.
    let scale_shift = 32 - u32::from(header_bits);
    for sample_val in &mut samples {
        *sample_val >>= scale_shift;
    }

    // Copying the header through untouched keeps the sample period, sample number, channel, and loop points identical to the source.
    let sds_ch = dump_header[2];
    let mut out = dump_header;
    out.extend_from_slice(&encode_sds_packets(&samples, header_bits, sds_ch));
    Ok(out)
}

/// Creates an ACK message to tell the device a packet was received successfully.
pub fn make_sds_confirmation(ch: u8, packet_num: u8) -> Vec<u8> {
    vec![0xF0, 0x7E, ch & 0x7F, 0x7F, packet_num & 0x7F, 0xF7]
}

/// Writes i16 mono PCM samples as a minimal PCM WAV file.
pub fn write_wav(path: &std::path::Path, samples: &[i16], sample_rate: u32, loopdata: Option<(u8, u32, u32)>) -> Result<(), String> {
    use std::io::Write;
    let num_samples = samples.len() as u32;
    let data_size = num_samples * 2;
    let has_loopdata = loopdata.is_some() && loopdata.unwrap().0 != 0x7F; // `0x7F` is `SDS_LOOP_OFF`
    let smpl_size = if has_loopdata { 8 + 60 } else { 0 };
    let file_size = 36 + data_size + smpl_size;
    let byte_rate = sample_rate * 2; // 1 channel × 2 bytes/sample
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut file_writer = std::io::BufWriter::new(file);

    // RIFF header.
    let _ = file_writer.write_all(b"RIFF");
    let _ = file_writer.write_all(&file_size.to_le_bytes());
    let _ = file_writer.write_all(b"WAVE");

    // Format chunk.
    let _ = file_writer.write_all(b"fmt ");
    let _ = file_writer.write_all(&16u32.to_le_bytes()); // chunk size
    let _ = file_writer.write_all(&1u16.to_le_bytes()); // PCM
    let _ = file_writer.write_all(&1u16.to_le_bytes()); // mono
    let _ = file_writer.write_all(&sample_rate.to_le_bytes());
    let _ = file_writer.write_all(&byte_rate.to_le_bytes());
    let _ = file_writer.write_all(&2u16.to_le_bytes()); // block align
    let _ = file_writer.write_all(&16u16.to_le_bytes()); // bits per sample

    // Sample loop chunk.
    if has_loopdata {
        let (loopdata_type, loopdata_start, loopdata_end) = loopdata.unwrap();
        let _ = file_writer.write_all(b"smpl");
        let _ = file_writer.write_all(&60u32.to_le_bytes());
        let _ = file_writer.write_all(&0u32.to_le_bytes()); // Manufacturer
        let _ = file_writer.write_all(&0u32.to_le_bytes()); // Product
        let sample_period = 1_000_000_000_u32.checked_div(sample_rate).unwrap_or(0);
        let _ = file_writer.write_all(&sample_period.to_le_bytes()); // Sample Period
        let _ = file_writer.write_all(&60u32.to_le_bytes()); // MIDI Unity Note (60 = Middle C)
        let _ = file_writer.write_all(&0u32.to_le_bytes()); // MIDI Pitch Fraction
        let _ = file_writer.write_all(&0u32.to_le_bytes()); // SMPTE Format
        let _ = file_writer.write_all(&0u32.to_le_bytes()); // SMPTE Offset
        let _ = file_writer.write_all(&1u32.to_le_bytes()); // Num Sample Loops
        let _ = file_writer.write_all(&0u32.to_le_bytes()); // Sampler Data

        // `Loop[0]`
        let _ = file_writer.write_all(&0u32.to_le_bytes()); // Cue Point ID
        let wav_loopdata_type = u32::from(loopdata_type == 0x01); // 0 = forward, 1 = ping-pong
        let _ = file_writer.write_all(&wav_loopdata_type.to_le_bytes()); // Type
        let _ = file_writer.write_all(&loopdata_start.to_le_bytes()); // Start
        let _ = file_writer.write_all(&loopdata_end.to_le_bytes()); // End
        let _ = file_writer.write_all(&0u32.to_le_bytes()); // Fraction
        let _ = file_writer.write_all(&0u32.to_le_bytes()); // Play Count (0 = infinite)
    }

    // Data chunk.
    let _ = file_writer.write_all(b"data");
    let _ = file_writer.write_all(&data_size.to_le_bytes());
    for &sample_val in samples {
        let _ = file_writer.write_all(&sample_val.to_le_bytes());
    }
    // Buffered writes surface here rather than at each `write_all()`, so a full disk shows up at this point.
    file_writer.flush().map_err(|e| e.to_string())?;
    Ok(())
}

/// Decodes raw SDS binary to mono PCM and compresses it as FLAC at the source's own bit depth, returning base64.
///
/// The depth comes from the SDS Dump Header, not the device, so a dump with more resolution than the device's upload depth keeps all of it.
/// The Dump Header itself rides along as a Vorbis comment tag, since FLAC has no slot for a sample period, loop points, or a sample number.
pub fn compress_sds_to_b64(sds_data: &[u8]) -> Result<String, String> {
    use flacenc::component::{BitRepr, MetadataBlockData};
    use flacenc::error::Verify;
    use flacenc::source::MemSource;

    let (mut samples, sample_rate, bit_depth) = decode_sds(sds_data)?;
    let dump_header = sds_dump_header(sds_data)
        .map(bytes_to_hex_string)
        .ok_or("SDS: stream has no Dump Header")?;

    // flacenc rejects depths outside 4n and 4n+1, ruling out common SDS widths like 14.
    // So samples are padded up to the nearest depth it takes.
    // The Dump Header still declares the true depth, and `decode_flac_b64_to_sds()` shifts the padding back off.
    // That costs only a few bits per sample.
    let storage_bits = flac_storage_bits(bit_depth);
    let pad = u32::from(storage_bits - bit_depth);
    if pad > 0 {
        for sample_val in &mut samples {
            *sample_val <<= pad;
        }
    }

    let cfg = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| format!("flac config invalid: {e}"))?;
    let source = MemSource::from_samples(&samples, 1, storage_bits as usize, sample_rate as usize);
    let mut stream =
        flacenc::encode_with_fixed_block_size(&cfg, source, cfg.block_size).map_err(|e| format!("flac encode failed: {e:?}"))?;

    let vorbis_comment = build_sds_dump_header_vorbis_comment(&dump_header);
    let block = MetadataBlockData::new_unknown(4, &vorbis_comment) // 4 = VORBIS_COMMENT
        .map_err(|e| format!("flac metadata block invalid: {e}"))?;
    stream.add_metadata_block(block);

    let mut sink = flacenc::bitsink::MemSink::<u8>::new();
    stream.write(&mut sink).map_err(|e| format!("flac write failed: {e}"))?;

    let mut flac_bytes = sink.as_slice().to_vec();
    force_fixed_block_size_streaminfo(&mut flac_bytes);

    Ok(B64.encode(&flac_bytes))
}

/// Encodes mono i16 PCM samples as a raw SDS binary blob at `significant_bits` per sample.
///
/// This is the upload side, where audio is full-scale i16 from the editor, not depth-native from a dump, so samples are scaled down here.
///
/// The generated Dump Header declares `significant_bits` and the loop fields.
/// Sample number and channel are both zero, since neither exists here.
/// A sample read back out of a `.c7` never comes through here: it reuses its stored header, keeping the period and sample number intact.
///
/// Pass `loopdata_type=0x7F, loopdata_start=0, loopdata_end=0` for no loop (one-shot).
/// Loop points are in units of the target sample rate.
pub fn audio_to_sds(
    samples: &[i16],
    sample_rate: u32,
    loopdata_type: u8,
    loopdata_start: u32,
    loopdata_end: u32,
    significant_bits: u8,
) -> Vec<u8> {
    let shift = 16u32.saturating_sub(u32::from(significant_bits));
    let depth_native: Vec<i32> = samples.iter().map(|&sample_val| i32::from(sample_val) >> shift).collect();

    // Sample period in nanoseconds, encoded as 3 LSB-first 7-bit bytes.
    let period_ns = if sample_rate > 0 {
        1_000_000_000u64 / u64::from(sample_rate)
    } else {
        32_000
    };
    let to_three_bytes = |val: u32| -> [u8; 3] { [(val & 0x7F) as u8, ((val >> 7) & 0x7F) as u8, ((val >> 14) & 0x7F) as u8] };
    let period_bytes = to_three_bytes(period_ns as u32);
    let len_bytes = to_three_bytes(depth_native.len() as u32);
    let loopdata_start_bytes = to_three_bytes(loopdata_start);
    let loopdata_end_bytes = to_three_bytes(loopdata_end);

    // Dump Header (21 bytes)
    let mut out = vec![
        0xF0,
        0x7E,
        0x00,
        0x01, // Universal Non-realtime, ch 0, Dump Header
        0x00,
        0x00,             // sample number 0
        significant_bits, // significant bits per sample
        period_bytes[0],
        period_bytes[1],
        period_bytes[2],
        len_bytes[0],
        len_bytes[1],
        len_bytes[2], // sample length in words
        loopdata_start_bytes[0],
        loopdata_start_bytes[1],
        loopdata_start_bytes[2], // sustain loop start
        loopdata_end_bytes[0],
        loopdata_end_bytes[1],
        loopdata_end_bytes[2], // sustain loop end
        loopdata_type,         // `0x00`=forward, `0x01`=ping-pong, `0x7F`=no loop
        0xF7,
    ];

    out.extend_from_slice(&encode_sds_packets(&depth_native, significant_bits, 0x00));
    out
}

/// Reads the loop fields from the SDS Dump Header in a raw SDS file.
///
/// Returns `(loopdata_type, loopdata_start, loopdata_end)`.
/// Returns `None` if `loopdata_type` is `0x7F` (no loop) or no valid Dump Header is found.
pub fn read_sds_loopdata(path: &str) -> Option<(u8, u32, u32)> {
    let data = std::fs::read(path).ok()?;
    sds_loopdata_from_bytes(&data)
}

/// Parses raw SDS binary (concatenated MIDI SysEx packets) into mono PCM + sample rate + the source's significant bits.
///
/// Samples come back centered at their own depth rather than scaled to a wider range, so a 12-bit dump yields 12-bit values.
/// That keeps depths above 16 intact, which the SDS spec allows up to 28. Callers that need i16 audio scale at their own boundary.
///
/// The header packet supplies sample rate via the nanosecond period field, and word width via the significant-bits field.
///
/// Data packets each carry a 120-byte payload of 7-bit-encoded samples (40 words at 16-bit, 60 at 12-bit).
/// The last packet is zero-padded to 120 bytes, and the header's length field trims the tail.
pub fn decode_sds(data: &[u8]) -> Result<(Vec<i32>, u32, u8), String> {
    let mut sample_rate = 0; // overwritten once the header's period field is read
    let mut total_samples = None::<usize>;

    // Word width comes from the Dump Header, so a stream opening with a data packet is malformed, not something to guess a width for.
    let mut significant_bits = 0u8;
    let mut bytes_per_word = 0usize; // set from the Dump Header, and the guard below rejects any data packet reaching this at zero
    let mut samples: Vec<i32> = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        let Some(f0_relative) = data[pos..].iter().position(|&byte| byte == 0xF0) else {
            break;
        };
        let f0 = pos + f0_relative;
        let Some(f7_relative) = data[f0..].iter().position(|&byte| byte == 0xF7) else {
            break;
        };
        let f7 = f0 + f7_relative;
        let packet = &data[f0..=f7];
        pos = f7 + 1;

        if packet.len() < 4 || packet[1] != 0x7E {
            continue;
        }

        match packet[3] {
            0x01 if packet.len() >= 21 => {
                // Dump Header byte 6 = significant bits per sample, which sets the word width below.
                // 28 is the SDS ceiling, and anything past it would give `sds_bytes_per_word()` a stride the packets don't use.
                // Guard 0 as well, since a zero stride would panic on the chunk below.
                significant_bits = packet[6];
                if significant_bits == 0 || significant_bits > 28 {
                    return Err(format!(
                        "SDS decode: Dump Header declares {significant_bits} significant bits, outside the 1 to 28 the standard allows"
                    ));
                }
                bytes_per_word = sds_bytes_per_word(significant_bits);
                // Bytes 7-9 = sample period in ns (3 × 7-bit LSB-first).
                let period_ns = (packet[7] as u64) | ((packet[8] as u64) << 7) | ((packet[9] as u64) << 14);
                sample_rate = (1_000_000_000u64 / period_ns) as u32;
                // Bytes 10-12 = sample length in words (= number of samples).
                total_samples = Some((packet[10] as usize) | ((packet[11] as usize) << 7) | ((packet[12] as usize) << 14));
            }
            0x02 if packet.len() >= 127 => {
                if bytes_per_word == 0 {
                    return Err("SDS decode: data packet arrived before any Dump Header".to_string());
                }
                // Data Packet: bytes 5-124 are the 120 payload bytes.
                // Samples are packed into `bytes_per_word` MIDI bytes each (3 for 16-bit, 2 for 12-bit).
                let payload = &packet[5..125];
                for chunk in payload.chunks_exact(bytes_per_word) {
                    samples.push(unpack_sds_word(chunk, significant_bits));
                }
            }
            _ => {}
        }
    }

    // Trim zero-padding from the last data packet using the declared sample count.
    if let Some(total_samples) = total_samples {
        samples.truncate(total_samples);
    }

    if samples.is_empty() {
        return Err("SDS decode: no data packets found".to_string());
    }
    Ok((samples, sample_rate, significant_bits))
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// Returns `true` if the raw message is an SDS Dump Header (`$01`).
fn is_sds_dump_header(raw: &[u8]) -> bool {
    raw.len() > 4 && raw[1] == 0x7E && raw[3] == 0x01
}

/// Returns `true` if the raw message is an SDS data packet (`$02`).
fn is_sds_data_packet(raw: &[u8]) -> bool {
    raw.len() > 4 && raw[1] == 0x7E && raw[3] == 0x02
}

/// Calculates the XOR checksum for an SDS data packet (covers bytes [1..-2], excluding F0 and the last two bytes).
fn calculate_sds_checksum(packet: &[u8]) -> u8 {
    let end = packet.len().saturating_sub(2);
    packet[1..end].iter().fold(0u8, |acc, &byte| acc ^ byte) & 0x7F
}

/// Builds the message that names the sample a transfer just sent.
///
/// A device with its own Elektron name command takes that one.
/// Everything else takes the Sample Dump Standard's own name message.
/// That message carries the length in a byte instead of fixing it at four characters.
fn build_sample_name_message(dc: &DeviceConfig, slot: u8, ch: u8, custom_name: &str) -> Vec<u8> {
    let name_bytes: Vec<u8> = custom_name.chars().map(|char_val| (char_val as u8) & 0x7F).collect();

    // A device that names samples its own way declares that command in its JSON.
    if !dc.has_gate("sysex_api.commands.sample_name_tag") {
        // F0 7E [ch] 05 03 [slot lsb] [slot msb] [language tag] [name length] [name] F7
        let mut msg = vec![0xF0, 0x7E, ch, 0x05, 0x03, slot & 0x7F, 0x00, 0x00, name_bytes.len() as u8];
        msg.extend_from_slice(&name_bytes);
        msg.push(0xF7);
        return msg;
    }

    let block_byte = u8::from_str_radix(as_string_or_die(dc.json_get("sysex_api.commands.sample_name_tag.block_byte")), 16).unwrap();
    let mut payload = vec![block_byte, slot & 0x7F];
    payload.extend_from_slice(&name_bytes);
    // Always use ch=`0x00` for the Elektron name command: the device's own channel byte, distinct from the SDS broadcast channel (`0x7F`).
    build_elektron_sysex(dc.prod, 0x00, &payload)
}

/// Reads the `SDS_DUMP_HEADER` tag back out of a parsed `VorbisComment` block.
///
/// Returns `None` if the tag is missing or isn't a valid Dump Header, which the caller treats as an error, not something to work around.
fn read_sds_dump_header_vorbis_comment(tags: &[symphonia::core::meta::Tag]) -> Option<Vec<u8>> {
    // Legacy C7 versions saved as `C7_SDS_HEADER`, rather than `SDS_DUMP_HEADER`.
    //
    // `C7_SDS_HEADER` is still accepted as the SDS Dump Header.
    let hex = tags
        .iter()
        .find(|tag| tag.raw.key == "SDS_DUMP_HEADER" || tag.raw.key == "C7_SDS_HEADER")?
        .raw
        .value
        .to_string();
    let header = from_hex(&hex)?;
    is_sds_dump_header(&header).then_some(header)
}

/// Encodes depth-native samples into SDS Data Packets, without a Dump Header.
///
/// Shared by `audio_to_sds()`, which generates a header, and `decode_flac_b64_to_sds()`, which reuses the stored one.
///
/// Packs each sample into `ceil(bits / 7)` MIDI bytes, left-justified, MSB-first, offset binary.
/// Lower depths fit more samples per 120-byte payload (e.g. 60 at 8-14 bits, 40 at 15-16).
/// That cuts packet count and the device's RAM footprint.
/// The last packet is zero-padded to 120 bytes.
fn encode_sds_packets(samples: &[i32], bits: u8, ch: u8) -> Vec<u8> {
    let bytes_per_word = sds_bytes_per_word(bits); // ceil(bits/7) MIDI bytes per sample word
    let field_bits = (bytes_per_word * 7) as u32; // total bits the word occupies (7 per MIDI byte)
    let samples_per_packet = 120 / bytes_per_word; // whole samples per 120-byte payload
    let offset = 1i32 << (bits - 1); // offset-binary center (e.g. 32768 at 16-bit)

    let mut out = Vec::new();
    for (packet_idx, chunk) in samples.chunks(samples_per_packet).enumerate() {
        let packet_num = (packet_idx & 0x7F) as u8;
        let mut payload = [0u8; 120];
        for (i, &sample_val) in chunk.iter().enumerate() {
            // Centered → offset binary at `bits` significant bits, left-justified.
            // Then split MSB-first into `bytes_per_word` 7-bit MIDI bytes.
            let unsigned = (sample_val + offset) as u32; // 0 .. 2^bits-1
            let field = unsigned << (field_bits - bits as u32); // left-justified in the word
            for b in 0..bytes_per_word {
                payload[i * bytes_per_word + b] = ((field >> (7 * (bytes_per_word - 1 - b) as u32)) & 0x7F) as u8;
            }
        }
        let checksum = [0x7Eu8, ch, 0x02, packet_num]
            .iter()
            .chain(payload.iter())
            .fold(0u8, |acc, &byte| acc ^ byte)
            & 0x7F;

        out.extend_from_slice(&[0xF0, 0x7E, ch, 0x02, packet_num]);
        out.extend_from_slice(&payload);
        out.extend_from_slice(&[checksum, 0xF7]);
    }
    out
}

/// Builds a FLAC `VORBIS_COMMENT` metadata block body carrying the SDS Dump Header as an `SDS_DUMP_HEADER` tag, hex-encoded.
///
/// FLAC has no slot for a sample period, loop points, or a sample number, so this is the standard place to stash them.
/// The symphonia crate parses `VorbisComment` blocks into readable tags automatically.
fn build_sds_dump_header_vorbis_comment(dump_header: &str) -> Vec<u8> {
    let vendor = b"C7";
    let comment = format!("SDS_DUMP_HEADER={dump_header}");

    let mut out = Vec::new();
    out.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    out.extend_from_slice(vendor);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(comment.len() as u32).to_le_bytes());
    out.extend_from_slice(comment.as_bytes());
    out
}

/// Rewrites a FLAC STREAMINFO's `min_block_size` to equal its `max_block_size`.
///
/// flacenc encodes with a fixed block size but reports the shorter final frame as the stream's `min_block_size`.
/// That leaves `min != max` for any sample count that isn't a multiple of the block size, i.e. almost always.
/// symphonia then reads `min != max` as variable blocking, and rejects flacenc's fixed-blocking frames in its strict check.
/// It hits "unexpected end of file".
/// libFLAC sets both fields to the configured block size for fixed blocking, so matching that convention makes the stream decodable.
/// The per-frame headers still carry each frame's true length, so the audio is unchanged. Only this metadata hint is corrected.
fn force_fixed_block_size_streaminfo(flac: &mut [u8]) {
    // Layout: "fLaC"(4) + metadata block header(4) + STREAMINFO data.
    // The STREAMINFO body begins at byte 8 with two big-endian u16s: `min_block_size` at [8..10], `max_block_size` at [10..12].
    // Guard on the FLAC marker and that the first metadata block is STREAMINFO (type 0, low 7 bits).
    if flac.len() >= 12 && &flac[0..4] == b"fLaC" && flac[4].trailing_zeros() >= 7 {
        flac[8] = flac[10];
        flac[9] = flac[11];
    }
}

/// Same as `read_sds_loopdata()`, but operates on an already-in-memory SDS buffer.
///
/// Suits callers like `compress_sds_to_b64()` that hold the raw packets rather than a file path.
fn sds_loopdata_from_bytes(data: &[u8]) -> Option<(u8, u32, u32)> {
    let mut pos = 0usize;
    while pos < data.len() {
        // Scan for the next SysEx start byte.
        let Some(f0_relative) = data[pos..].iter().position(|&byte| byte == 0xF0) else {
            break;
        };
        let end_pos = pos + f0_relative;
        // Dump Header is 21 bytes: `F0 7E cc 01 ss ss ff ee ee ee ll ll ll ls ls ls le le le lt F7`
        // `cc` (byte 2) is the device channel. Any value is valid, so it isn't checked.
        if end_pos + 21 > data.len() {
            break;
        }
        if data[end_pos + 1] == 0x7E && data[end_pos + 3] == 0x01 {
            // Offsets within the full packet (`F0` = index 0):
            //   13-15 = sustain loop start (3 × 7-bit LSB-first)
            //   16-18 = sustain loop end   (3 × 7-bit LSB-first)
            //   19    = loop type (`0x00` = forward, `0x01` = ping-pong, `0x7F` = no loop)
            let loopdata_type = data[end_pos + 19];
            let loopdata_start = (data[end_pos + 13] as u32) | ((data[end_pos + 14] as u32) << 7) | ((data[end_pos + 15] as u32) << 14);
            let loopdata_end = (data[end_pos + 16] as u32) | ((data[end_pos + 17] as u32) << 7) | ((data[end_pos + 18] as u32) << 14);

            return if loopdata_type != SDS_LOOP_OFF && loopdata_end > 0 {
                Some((loopdata_type, loopdata_start, loopdata_end))
            } else {
                None
            };
        }
        pos = end_pos + 1;
    }
    None
}

/// Rounds a bit depth up to the nearest one flacenc will encode.
///
/// flacenc verifies `bits_per_sample` against 4n and 4n+1, so 10, 11, 14, 15, 18, 19, 22, and 23 are all refused.
/// Padding up keeps every SDS width storable.
/// Since the Dump Header carries the true depth, the padding shifts straight back off on read-back.
fn flac_storage_bits(significant_bits: u8) -> u8 {
    let mut bits = significant_bits;
    while bits % 4 > 1 {
        bits += 1;
    }
    bits
}

/// Number of 7-bit MIDI bytes carrying one sample word in an SDS data packet.
///
/// SDS packs each sample into `ceil(significant_bits / 7)` bytes: 8-14 bits → 2 bytes, 15-21 bits → 3 bytes.
fn sds_bytes_per_word(significant_bits: u8) -> usize {
    (significant_bits as usize).div_ceil(7)
}

/// Unpacks one left-justified SDS sample word (MSB-first 7-bit bytes) into a centered value at its own depth.
///
/// The value is offset binary across `significant_bits` bits: recentered, but deliberately not rescaled.
/// Widening here would cap the pipeline at 16 bits.
fn unpack_sds_word(chunk: &[u8], significant_bits: u8) -> i32 {
    let field_bits = chunk.len() * 7;
    let field = chunk.iter().fold(0u32, |acc, &byte| (acc << 7) | (byte as u32 & 0x7F));
    let unsigned = field >> (field_bits - significant_bits as usize);
    unsigned as i32 - (1 << (significant_bits - 1))
}
