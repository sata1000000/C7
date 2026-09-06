//! Bridge monitor: the server the C7 app runs, and the reconnect client each plugin instance uses to reach it.
//!
//! C7 Bridge routes hardware traffic through the DAW's own MIDI output and mirrors it here.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::device_config::DeviceConfig;

/// TCP port shared with `c7_bridge/src/plugin.rs`.
pub const BRIDGE_PORT: u16 = 7420;

struct Client {
    stream: TcpStream,
    addr: String,
    buffer: Vec<u8>,
    dc: Option<DeviceConfig>,
}

impl Client {
    /// Outputs the device's short name for the UI log (like "MD"/"MnM"), or the raw address before the handshake.
    fn device_name(&self) -> String {
        self.dc.as_ref().map_or_else(|| self.addr.clone(), |dc| dc.device_shorter.clone())
    }
}

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

// Every frame is a fixed 4 bytes: [type: u8, a: u8, b: u8, c: u8].
/// Plugin → server: `[0x01, product_id, 0, 0]`.
pub const MSG_HELLO: u8 = 0x01;
/// Server → plugin: handshake reply.
pub const MSG_HELLO_ACK: u8 = 0x02;
/// Plugin → server: `[0x10, channel, cc_id, value]`.
pub const MSG_CC_PARAM: u8 = 0x10;
/// Plugin → server: `[0x11, block_byte, param_id, value]`.
pub const MSG_SYSEX_PARAM: u8 = 0x11;
/// Plugin → server: `[0x12, channel, note, velocity]`, where velocity 0 releases the note.
pub const MSG_NOTE: u8 = 0x12;
/// Plugin → server: `[0x13, track, machine_id, 0]`.
pub const MSG_MACHINE: u8 = 0x13;
/// Plugin → server: `[0x14, channel, 0, 0]`, an MnM note that arrived on a channel no track owns.
pub const MSG_NOTE_DROPPED: u8 = 0x14;
/// Plugin → server: `[0x15, channel, lsb, msb]`, a 14-bit value (0-16383) centered at 8192.
pub const MSG_PITCH_BEND: u8 = 0x15;

/// One observed plugin event, forwarded to the Bridge screen's connection list and I/O log.
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    Connected {
        device: String,
        addr: String,
    },
    Disconnected {
        device: String,
    },
    CcParam {
        device: String,
        channel: u8,
        cc: u8,
        val: u8,
    },
    SysexParam {
        device: String,
        block: u8,
        param: u8,
        val: u8,
    },
    Note {
        device: String,
        channel: u8,
        note: u8,
        velocity: u8,
    },
    Machine {
        device: String,
        track: u8,
        machine: u8,
    },
    NoteDropped {
        device: String,
        channel: u8,
    },
    PitchBend {
        device: String,
        channel: u8,
        val: u16,
    },
}

/// Consumer end of the bridge monitor's event stream, owned by `BridgeScreen`.
pub struct BridgeEventSource {
    pub rx: async_channel::Receiver<BridgeEvent>,
}

/// Binds the bridge port and starts the polling monitor thread.
///
/// # Errors
///
/// Returns `Err` if the port is already occupied (e.g. another C7 instance).
pub fn start_bridge_server() -> Result<BridgeEventSource, String> {
    let listener = TcpListener::bind(format!("127.0.0.1:{BRIDGE_PORT}")).map_err(|e| format!("Cannot bind to port {BRIDGE_PORT}: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking on listener failed: {e}"))?;

    let (event_tx, event_rx) = async_channel::unbounded::<BridgeEvent>();
    thread::spawn(move || run_server_loop(&listener, &event_tx));

    Ok(BridgeEventSource { rx: event_rx })
}

/// Background connection to the C7 app's monitor socket, retried for the plugin instance's whole lifetime.
///
/// Handshakes with the device's product ID, then forwards the mirrored frames.
pub fn spawn_monitor_reconnect_thread(prod: u8, rx: async_channel::Receiver<[u8; 4]>, should_shutdown: Arc<AtomicBool>) {
    thread::spawn(move || {
        while !should_shutdown.load(Ordering::Relaxed) {
            let Ok(mut stream) = TcpStream::connect(("127.0.0.1", BRIDGE_PORT)) else {
                // Nobody watching: discard the backlog so frames from an hour ago don't flood the log on connect.
                while rx.try_recv().is_ok() {}
                thread::sleep(Duration::from_secs(2));
                continue;
            };

            // Handshake: identify as this plugin's device and wait for the ack.
            if stream.write_all(&[MSG_HELLO, prod, 0, 0]).is_err() {
                continue;
            }
            stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
            let mut ack = [0u8; 4];
            if stream.read_exact(&mut ack).is_err() || ack[0] != MSG_HELLO_ACK {
                thread::sleep(Duration::from_secs(2));
                continue;
            }

            'connected: while !should_shutdown.load(Ordering::Relaxed) {
                // Drain everything queued, then breathe.
                let mut has_sent_any = false;
                while let Ok(frame) = rx.try_recv() {
                    has_sent_any = true;
                    if stream.write_all(&frame).is_err() {
                        break 'connected;
                    }
                }
                if !has_sent_any {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// The main polling loop: accepts new plugin connections and services every active one.
fn run_server_loop(listener: &TcpListener, event_tx: &async_channel::Sender<BridgeEvent>) {
    let mut clients: Vec<Client> = Vec::new();

    loop {
        // ── Accept every pending connection ────────────────────────────────────────────────────────────────

        loop {
            match listener.accept() {
                Ok((stream, addr)) => {
                    stream.set_nonblocking(true).ok();
                    clients.push(Client {
                        stream,
                        addr: addr.to_string(),
                        buffer: Vec::new(),
                        dc: None,
                    });
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => return, // listener is broken. Exit thread.
            }
        }

        // ── Service every client, dropping the ones that are gone ──────────────────────────────────────────

        clients.retain_mut(|client| service_client(client, event_tx));

        thread::sleep(Duration::from_millis(5));
    }
}

/// Reads whatever the client has sent and handles every complete frame.
///
/// Returns `false` when the connection is gone (EOF, reset, or protocol violation).
fn service_client(client: &mut Client, event_tx: &async_channel::Sender<BridgeEvent>) -> bool {
    let mut tmp = [0u8; 1024];
    loop {
        match client.stream.read(&mut tmp) {
            Ok(0) => return report_disconnect(client, event_tx), // EOF
            Ok(n) => client.buffer.extend_from_slice(&tmp[..n]),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => return report_disconnect(client, event_tx), // connection reset
        }
    }

    // Peel complete 4-byte frames off the front of the buffer.
    while client.buffer.len() >= 4 {
        let frame = [client.buffer[0], client.buffer[1], client.buffer[2], client.buffer[3]];
        client.buffer.drain(..4);
        if !handle_frame(client, frame, event_tx) {
            return report_disconnect(client, event_tx);
        }
    }
    true
}

/// Emits the Disconnected event for a client that is going away.
///
/// Always returns `false` so callers can tail-return it.
fn report_disconnect(client: &Client, event_tx: &async_channel::Sender<BridgeEvent>) -> bool {
    let _ = event_tx.try_send(BridgeEvent::Disconnected {
        device: client.device_name(),
    });
    false
}

/// Decodes one 4-byte frame into its log event.
///
/// Returns `false` on a protocol violation, which drops the client.
fn handle_frame(client: &mut Client, frame: [u8; 4], event_tx: &async_channel::Sender<BridgeEvent>) -> bool {
    let [msg_type, data1, data2, val] = frame;

    if msg_type == MSG_HELLO {
        // The product ID must match a device in the registry.
        // Each plugin binary bakes in exactly one device's JSON, so a mismatch means a foreign client.
        let Some(dc) = DeviceConfig::find_by_prod(data1) else {
            return false;
        };
        let _ = client.stream.write_all(&[MSG_HELLO_ACK, 0, 0, 0]);
        let _ = event_tx.try_send(BridgeEvent::Connected {
            device: dc.device_shorter.clone(),
            addr: client.addr.clone(),
        });
        client.dc = Some(dc);
        return true;
    }

    // Everything else requires a completed handshake.
    let Some(dc) = &client.dc else { return false };
    let device_shorter = dc.device_shorter.clone();

    let event = match msg_type {
        MSG_CC_PARAM => BridgeEvent::CcParam {
            device: device_shorter,
            channel: data1,
            cc: data2,
            val,
        },
        MSG_SYSEX_PARAM => BridgeEvent::SysexParam {
            device: device_shorter,
            block: data1,
            param: data2,
            val,
        },
        MSG_NOTE => BridgeEvent::Note {
            device: device_shorter,
            channel: data1,
            note: data2,
            velocity: val,
        },
        MSG_MACHINE => BridgeEvent::Machine {
            device: device_shorter,
            track: data1,
            machine: data2,
        },
        MSG_NOTE_DROPPED => BridgeEvent::NoteDropped {
            device: device_shorter,
            channel: data1,
        },
        MSG_PITCH_BEND => BridgeEvent::PitchBend {
            device: device_shorter,
            channel: data1,
            val: data2 as u16 | ((val as u16) << 7),
        },
        _ => return false, // unknown frame type
    };
    let _ = event_tx.try_send(event);
    true
}
