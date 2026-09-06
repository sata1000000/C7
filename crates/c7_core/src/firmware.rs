//! Functions for firmware transfer and OS version identity queries.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::midi::{find_input_port, open_large_sysex_input, send_sysex};
use crate::sysex::parse_sysex_file;

/// Universal Non-Realtime SysEx ID.
const UNIVERSAL_NON_REALTIME: u8 = 0x7E;
/// Device ID meaning "all devices, please respond."
const DEVICE_ID_BROADCAST: u8 = 0x7F;
/// Sub-ID#1: General Information.
const SUB_ID_GENERAL_INFO: u8 = 0x06;
/// Sub-ID#2: Identity Request.
const SUB_ID_IDENTITY_REQUEST: u8 = 0x01;
/// Sub-ID#2: Identity Reply.
const SUB_ID_IDENTITY_REPLY: u8 = 0x02;

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Requests the connected machine's OS version via Universal Device Inquiry.
///
/// A standard MIDI-spec request, identical for every device, so it's hardcoded rather than read from device JSON.
pub async fn request_os_version(port_name: &str) -> Result<String, String> {
    let identity_request: Vec<u8> = vec![
        0xF0,
        UNIVERSAL_NON_REALTIME,
        DEVICE_ID_BROADCAST,
        SUB_ID_GENERAL_INFO,
        SUB_ID_IDENTITY_REQUEST,
        0xF7,
    ];

    // Verify existence of matching input port for response.
    let Some(in_name) = find_input_port(port_name) else {
        return Err("No Input Port".to_string());
    };

    let midi_in = match open_large_sysex_input(&in_name) {
        Ok(port) => port,
        Err(err) => return Err(format!("Error ({err})")),
    };

    send_sysex(port_name, &identity_request);
    let deadline = Instant::now() + Duration::from_secs(3);

    // Poll for SysEx response until 3-second timeout.
    while Instant::now() < deadline {
        if let Some(data) = midi_in.poll()
            && data.first() == Some(&0xF0)
        {
            // Strip framing F0/F7 to get inner payload.
            let inner: Vec<u8> = data.iter().copied().skip(1).take_while(|&byte_val| byte_val != 0xF7).collect();

            let is_identity_reply = inner.len() >= 8
                && inner[0] == UNIVERSAL_NON_REALTIME
                && inner[2] == SUB_ID_GENERAL_INFO
                && inner[3] == SUB_ID_IDENTITY_REPLY;

            if is_identity_reply {
                let version_bytes = &inner[inner.len() - 4..];
                let version = std::str::from_utf8(version_bytes)
                    .map(|string_val| string_val.trim().to_string())
                    .unwrap_or_default();
                let version = if version.is_empty() {
                    version_bytes.iter().map(ToString::to_string).collect::<Vec<_>>().join(".")
                } else {
                    version
                };

                return Ok(format!("OS Version: {version}"));
            }
        }
        glib::timeout_future(Duration::from_millis(10)).await;
    }
    Ok("OS Version: No Reply".to_string())
}

/// Parses a firmware SysEx file and streams it to the MIDI device with progress updates.
pub async fn upload_firmware(
    port_name: String,
    path: std::path::PathBuf,
    delay_ms_val: f64,
    should_cancel: Arc<AtomicBool>,
    progress_tx: async_channel::Sender<(String, f64, Instant)>,
    start: Instant,
) -> Result<(), String> {
    let data = std::fs::read(&path).map_err(|err| err.to_string())?;
    let msgs = parse_sysex_file(&data);

    // Abort if no valid MIDI SysEx messages were found in the file.
    if msgs.is_empty() {
        return Err("File contains no valid SysEx data.".to_string());
    }

    let total = msgs.len();

    // Iterate through messages and transmit via MIDI with progress updates.
    for (i, msg) in msgs.iter().enumerate() {
        // Check for user-requested cancellation.
        if should_cancel.load(Ordering::Relaxed) {
            break;
        }
        send_sysex(&port_name, msg);

        // Update progress status every 1% of transmission.
        if i % std::cmp::max(1, total / 100) == 0 || i == total - 1 {
            let fraction = (i + 1) as f64 / total as f64;
            let text = format!("Uploading OS... {}%", (fraction * 100.0) as u32);
            let _ = progress_tx.send_blocking((text, fraction, start));
        }

        // Apply inter-packet delay from settings.
        if delay_ms_val > 0.0 {
            glib::timeout_future(Duration::from_secs_f64(delay_ms_val / 1000.0)).await;
        }
    }

    Ok(())
}

/// Captures an incoming firmware SysEx stream into a memory buffer.
pub async fn receive_firmware(
    port_name: String,
    should_cancel: Arc<AtomicBool>,
    progress_tx: async_channel::Sender<String>,
) -> Result<Vec<u8>, String> {
    // Validate the input port exists before arming the UI.
    let Some(in_name) = find_input_port(&port_name) else {
        return Err("Error: Could not find matching MIDI IN port.".to_string());
    };

    let midi_in = match open_large_sysex_input(&in_name) {
        Ok(port) => port,
        Err(err) => return Err(format!("Error ({err})")),
    };

    let mut buffer = Vec::<u8>::new();
    let mut is_receiving = false;
    let mut last_time = Instant::now();

    while !should_cancel.load(Ordering::Relaxed) {
        if let Some(data) = midi_in.poll() {
            is_receiving = true;
            buffer.extend_from_slice(&data);
            last_time = Instant::now();
            let _ = progress_tx.send_blocking(format!("Status: Receiving OS... {:.1} KB", buffer.len() as f64 / 1024.0));
        } else {
            glib::timeout_future(Duration::from_millis(10)).await;
        }

        if is_receiving && last_time.elapsed().as_secs_f64() > 2.0 {
            break;
        }
    }
    Ok(buffer)
}
