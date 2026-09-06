//! Bidirectional MIDI I/O.
//!
//! One background actor thread owns the single cached output connection.
//! Everything that talks to hardware is serialized through it and paced apart by a configurable gap.
//! Callers either fire a message at a port and return immediately, or hold the port open to read back the device's reply.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

/// Manual/none port mode, shown as the first entry in the device dropdown.
///
/// Connects to no hardware.
/// On Linux/macOS the app exposes virtual "C7" ports the user wires by hand (qjackctl, aconnect, ...), useful when in/out devices differ.
/// On Windows virtual ports don't exist, so this acts as a dummy device: sends go nowhere, inputs never receive.
pub const MANUAL_PORT: &str = "DummyDevice";

static ACTOR_SENDER: LazyLock<async_channel::Sender<MidiActorCommand>> = LazyLock::new(|| {
    let (tx, rx) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("midi_actor".to_string())
        .spawn(move || {
            run_midi_actor_loop(&rx);
        })
        .expect("failed to spawn midi actor thread");
    tx
});

/// Minimum gap (ms, 0-127) the actor waits between standalone sends.
/// This keeps back-to-back one-shots (param sweeps, kit-send bursts) from flooding the USB MIDI interface.
///
/// 0 = AUTO: no gap, set while ACK-paced SDS transfers own the timing.
///
/// Written by the main-window delay spinner via `set_midi_delay_ms()`.
static MIDI_DELAY_MS: AtomicU8 = AtomicU8::new(2);

/// `true` while any bulk MIDI operation holds the device (rawmidi write, backup, SDS, etc.).
static MIDI_BUSY: AtomicBool = AtomicBool::new(false);
static OUTBOUND_CALLBACKS: LazyLock<Mutex<Vec<OutboundFn>>> = LazyLock::new(|| Mutex::new(Vec::new()));

const CLIENT_NAME: &str = "C7";

enum MidiActorCommand {
    InitPort {
        port_name: String,
        reply: async_channel::Sender<Result<(), String>>,
    },
    Send {
        port_name: String,
        msg: Vec<u8>,
    },
    RunSession {
        port_name: String,
        should_open_input: bool,
        session_fn: MidiSessionFn,
        reply: async_channel::Sender<Result<(), String>>,
    },
}

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Output handle passed to `run_midi_session()` / `run_midi_output()` closures.
///
/// `Real` wraps the persistent hardware connection.
/// `Null` is manual ("`DummyDevice`") mode: no output connection exists, so sends are dropped.
/// They're still surfaced to the MIDI monitor for dummy-device development.
pub enum MidiSender<'a> {
    Real(&'a mut MidiOutputConnection),
    Null,
}

impl MidiSender<'_> {
    /// Sends a raw MIDI message and mirrors it to the MIDI monitor's outbound tap.
    pub fn send(&mut self, msg: &[u8]) -> Result<(), String> {
        if let MidiSender::Real(conn) = self {
            conn.send(msg).map_err(|e| e.to_string())?;
        }
        notify_outbound_midi(msg);
        Ok(())
    }

    /// Transmits a SysEx message inside a session.
    ///
    /// Named distinctly from `midi()` so call sites are self-documenting about what kind of message just went out.
    pub fn sysex(&mut self, packet: &[u8]) {
        self.midi(packet);
    }

    /// Transmits a non-SysEx MIDI message (Note On/Off, Clock, Start/Stop, etc.) inside a session.
    ///
    /// See `sysex()` for why this is a separately named method.
    pub fn midi(&mut self, msg: &[u8]) {
        let _ = self.send(msg);
    }
}

/// Container for an open MIDI input connection and its async message channel.
///
/// `rx` is an `async_channel::Receiver` that can be cloned.
/// Attach it to the GTK main loop via `ui::listen()` for event-driven delivery.
/// Or poll it synchronously via `poll()`/`flush()` from background threads.
pub struct PollableMidiInput {
    pub rx: async_channel::Receiver<Vec<u8>>,
    /// Kept alive to hold the connection open, and dropped to close the port.
    ///
    /// `None` only in manual ("DummyDevice") mode on Windows, where no real connection exists.
    _conn: Option<MidiInputConnection<()>>,
}

impl PollableMidiInput {
    /// Returns the next pending MIDI message without blocking.
    /// Returns `None` if the queue is empty.
    pub fn poll(&self) -> Option<Vec<u8>> {
        self.rx.try_recv().ok()
    }

    /// Discards all currently pending messages.
    pub fn flush(&self) {
        while self.rx.try_recv().is_ok() {}
    }
}

/// Sets the actor's inter-message pacing gap in milliseconds (0-127).
///
/// 0 disables pacing (AUTO), used while closed-loop SDS transfers pace themselves by ACK/NAK.
/// The main-window delay spinner is the single writer.
pub fn set_midi_delay_ms(ms: u8) {
    MIDI_DELAY_MS.store(ms, Ordering::Relaxed);
}

/// Returns the actor's current inter-message pacing gap in milliseconds (0-127).
///
/// Feature code that paces its own multi-send loop inside a session closure (unpaced by the actor) reads this to match the global gap.
pub fn get_midi_delay_ms() -> u8 {
    MIDI_DELAY_MS.load(Ordering::Relaxed)
}

/// Registers a callback function to listen to all outgoing MIDI traffic.
pub fn register_outbound_callback(cb: impl Fn(&[u8]) + Send + Sync + 'static) {
    OUTBOUND_CALLBACKS.lock().unwrap().push(Arc::new(cb));
}

/// Formats a raw MIDI byte slice into human-readable token strings for the log.
pub fn midi_tokens(data: &[u8]) -> Vec<String> {
    if data.is_empty() {
        return vec!["(empty)".to_string()];
    }
    let status = data[0];
    let kind = status & 0xF0;
    let ch = status & 0x0F;

    match kind {
        0x80 => vec![
            "note_off".to_string(),
            format!("channel={ch}"),
            format!("note={}", data.get(1).copied().unwrap_or(0)),
            format!("velocity={}", data.get(2).copied().unwrap_or(0)),
        ],
        0x90 => vec![
            "note_on".to_string(),
            format!("channel={ch}"),
            format!("note={}", data.get(1).copied().unwrap_or(0)),
            format!("velocity={}", data.get(2).copied().unwrap_or(0)),
        ],
        0xA0 => vec![
            "polytouch".to_string(),
            format!("channel={ch}"),
            format!("note={}", data.get(1).copied().unwrap_or(0)),
            format!("value={}", data.get(2).copied().unwrap_or(0)),
        ],
        0xB0 => vec![
            "control_change".to_string(),
            format!("channel={ch}"),
            format!("control={}", data.get(1).copied().unwrap_or(0)),
            format!("value={}", data.get(2).copied().unwrap_or(0)),
        ],
        0xC0 => vec![
            "program_change".to_string(),
            format!("channel={ch}"),
            format!("program={}", data.get(1).copied().unwrap_or(0)),
        ],
        0xD0 => vec![
            "aftertouch".to_string(),
            format!("channel={ch}"),
            format!("value={}", data.get(1).copied().unwrap_or(0)),
        ],
        0xE0 => {
            let lo = data.get(1).copied().unwrap_or(0) as i16;
            let hi = data.get(2).copied().unwrap_or(0) as i16;
            vec![
                "pitchwheel".to_string(),
                format!("channel={ch}"),
                format!("pitch={}", (hi << 7) | lo),
            ]
        }
        _ => match status {
            0xF0 => vec!["sysex".to_string()],
            0xF8 => vec!["clock".to_string()],
            0xFA => vec!["start".to_string()],
            0xFB => vec!["continue".to_string()],
            0xFC => vec!["stop".to_string()],
            0xFE => vec!["active_sensing".to_string()],
            0xFF => vec!["reset".to_string()],
            _ => vec![format!("unknown({status:#04X})")],
        },
    }
}

/// Returns a sorted list of available MIDI output port names.
pub fn get_output_names() -> Vec<String> {
    let Ok(output) = MidiOutput::new(CLIENT_NAME) else {
        return Vec::new();
    };
    output.ports().iter().filter_map(|port| output.port_name(port).ok()).collect()
}

/// Ensures a persistent MIDI output connection to the target port is open and returns it.
///
/// If the port name has changed, the previous connection is dropped.
pub fn init_persistent_port(port_name: &str) -> Result<(), String> {
    let (reply_tx, reply_rx) = async_channel::bounded(1);
    let cmd = MidiActorCommand::InitPort {
        port_name: port_name.to_string(),
        reply: reply_tx,
    };
    ACTOR_SENDER.send_blocking(cmd).map_err(|e| e.to_string())?;
    reply_rx.recv_blocking().map_err(|e| e.to_string())?
}

/// Fires a SysEx message at a port.
///
/// Enqueues and returns immediately. The actor paces and backgrounds the send.
///
/// Does the same thing as `send_midi()`, and exists only so call sites document what kind of message goes out.
pub fn send_sysex(port_name: &str, bytes: &[u8]) {
    send_midi(port_name, bytes);
}

/// Fires a Control Change at a port.
///
/// The channel is masked to the low nibble and both data bytes to 7 bits.
/// A status byte only carries a 0-15 channel, and data bytes must stay below `$80`.
pub fn send_midi_cc(port_name: &str, ch: u8, cc: u8, val: u8) {
    send_midi(port_name, &[0xB0 | (ch & 0x0F), cc & 0x7F, val & 0x7F]);
}

/// Runs a request/response handshake against a port (SDS transfer, dump receive, status query).
///
/// Opens both input and output and runs `func` with both handles.
pub async fn run_midi_session<F, R>(port_name: &str, func: F) -> Result<R, String>
where
    F: FnOnce(&PollableMidiInput, &mut MidiSender) -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    // `should_open_input` is `true` here, so the actor always hands the closure `Some(midi_in)` (or fails before calling it).
    run_session_command(port_name, true, move |midi_in, sender| {
        func(midi_in.expect("run_midi_session always opens the input port"), sender)
    })
    .await
}

/// Runs an output-only session against a port, running `func` with only the output handle.
///
/// For multi-step sends that hold the port but never read replies, so the input endpoint is never claimed.
pub async fn run_midi_output<F, R>(port_name: &str, func: F) -> Result<R, String>
where
    F: FnOnce(&mut MidiSender) -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    run_session_command(port_name, false, move |_inport, sender| func(sender)).await
}

/// Transmits a large SysEx blob without blocking the UI (rawmidi on Linux, midir fallback elsewhere).
///
/// The transfer itself is blocking: arming delays, packet pacing, and on Linux the direct rawmidi write.
/// Running it on the GTK main loop freezes the UI, so this offloads the blocking core (`send_large_sysex_blocking()`) to a worker thread.
/// It awaits completion over a channel.
pub async fn send_large_sysex(port_name: &str, arm_bytes: Vec<u8>, blob_bytes: Vec<u8>, arm_delay: f64, post_delay: f64) -> bool {
    let port = port_name.to_string();
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let ok = send_large_sysex_blocking(&port, &arm_bytes, &blob_bytes, arm_delay, post_delay);
        let _ = tx.send_blocking(ok);
    });
    rx.recv().await.unwrap_or(false)
}

/// Fires a MIDI message at a port.
///
/// Enqueues and returns immediately. The actor paces and backgrounds the send.
pub fn send_midi(port_name: &str, bytes: &[u8]) {
    let _ = ACTOR_SENDER.send_blocking(MidiActorCommand::Send {
        port_name: port_name.to_string(),
        msg: bytes.to_vec(),
    });
}

/// Returns `true` if the port name is valid and represents a real hardware device.
pub fn is_valid_port(port: &str) -> bool {
    !port.is_empty() && port != "No MIDI devices found" && !port.contains("Select")
}

/// Returns the input port name that best matches the provided output port name.
pub fn find_input_port(out_port_name: &str) -> Option<String> {
    // Manual mode pairs with its own virtual/dummy input, not a hardware port.
    if out_port_name == MANUAL_PORT {
        return Some(MANUAL_PORT.to_string());
    }
    let Ok(midi_in) = MidiInput::new(CLIENT_NAME) else {
        return None;
    };
    let names: Vec<String> = midi_in.ports().iter().filter_map(|port| midi_in.port_name(port).ok()).collect();
    // First pass: exact or substring match (works on ALSA/`CoreMIDI`, where a device's input and output share a base name).
    if let Some(matched_name) = names
        .iter()
        .find(|name| name.as_str() == out_port_name || out_port_name.contains(name.as_str()) || name.contains(out_port_name))
    {
        return Some(matched_name.clone());
    }

    // Exception for `WinRT`:
    // It tags a device's input/output endpoints with different trailing " [N]" indices (output "UM-ONE [0]" vs input "UM-ONE [1]").
    // So neither substring-contains the other.
    // Match on the base name with that index stripped.
    let base = strip_port_index(out_port_name);
    names.into_iter().find(|name| strip_port_index(name) == base)
}

/// Opens a MIDI input port configured for large SysEx message reception.
///
/// Every backend here delivers complete SysEx messages: ALSA/`CoreMIDI` on Linux/macOS, `WinRT`'s `Windows.Devices.Midi` on Windows.
pub fn open_large_sysex_input(port_name: &str) -> Result<PollableMidiInput, String> {
    let mut midi_in = MidiInput::new(CLIENT_NAME).map_err(|e| e.to_string())?;
    midi_in.ignore(Ignore::TimeAndActiveSense);

    // Find the port index matching the provided name.
    let ports = midi_in.ports();
    let target = ports.iter().find(|port| {
        midi_in
            .port_name(port)
            .is_ok_and(|name| name == port_name || port_name.contains(&name) || name.contains(port_name))
    });

    if port_name == MANUAL_PORT {
        let (tx, rx) = async_channel::unbounded::<Vec<u8>>();
        // Manual mode on Linux/macOS: create a virtual input port the user wires by hand.
        // ALSA/`CoreMIDI` deliver complete SysEx messages on virtual ports just like hardware ones, so no reassembly is needed.
        #[cfg(unix)]
        {
            use midir::os::unix::VirtualInput;
            let conn = midi_in
                .create_virtual(
                    "C7 in",
                    move |_stamp, msg, ()| {
                        let _ = tx.try_send(msg.to_vec());
                    },
                    (),
                )
                .map_err(|e| e.to_string())?;
            return Ok(PollableMidiInput { rx, _conn: Some(conn) });
        }
        // Windows has no virtual ports: hand back a dummy input with its channel already closed.
        // So `poll()`/`ui::listen()` see a message-less port.
        #[cfg(not(unix))]
        {
            drop(tx);
            return Ok(PollableMidiInput { rx, _conn: None });
        }
    }

    let Some(target_port) = target else {
        let available: Vec<_> = ports.iter().filter_map(|port| midi_in.port_name(port).ok()).collect();
        return Err(format!("MIDI input port not found: {port_name:?}  available: {available:?}"));
    };

    let (tx, rx) = async_channel::unbounded::<Vec<u8>>();

    let conn = midi_in
        .connect(
            target_port,
            CLIENT_NAME,
            move |_stamp, msg, ()| {
                let _ = tx.try_send(msg.to_vec());
            },
            (),
        )
        .map_err(|e| e.to_string())?;

    Ok(PollableMidiInput { rx, _conn: Some(conn) })
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

type MidiSessionFn = Box<dyn FnOnce(Option<&PollableMidiInput>, &mut MidiSender) -> Result<(), String> + Send>;

/// One registered listener for all outgoing MIDI traffic (e.g. the MIDI monitor screen).
///
/// `Arc` lets `OUTBOUND_CALLBACKS` be cloned cheaply under the lock, so callbacks fire without holding it.
type OutboundFn = Arc<dyn Fn(&[u8]) + Send + Sync + 'static>;

/// RAII guard that clears the MIDI busy flag when dropped.
struct MidiBusyGuard;

impl Drop for MidiBusyGuard {
    /// Clears the MIDI busy flag when the guard is dropped.
    fn drop(&mut self) {
        MIDI_BUSY.store(false, Ordering::Relaxed);
    }
}

/// The MIDI actor thread's main loop: owns the single cached output connection and serially processes commands from the channel.
fn run_midi_actor_loop(rx: &async_channel::Receiver<MidiActorCommand>) {
    let mut current_port: Option<String> = None;
    let mut current_conn: Option<MidiOutputConnection> = None;
    // Timestamp of the last standalone send, used by `pace_standalone_send()` to enforce `MIDI_DELAY_MS`.
    // Sessions (`RunSession`) are ACK-paced, so they're left out on purpose.
    let mut last_send: Option<Instant> = None;

    while let Ok(cmd) = rx.recv_blocking() {
        match cmd {
            MidiActorCommand::InitPort { port_name, reply } => {
                let result = ensure_port(&port_name, &mut current_port, &mut current_conn);
                let _ = reply.send_blocking(result);
            }
            MidiActorCommand::Send { port_name, msg } => {
                if ensure_port(&port_name, &mut current_port, &mut current_conn).is_ok() {
                    pace_standalone_send(&mut last_send);
                    let mut sender = match &mut current_conn {
                        Some(conn) => MidiSender::Real(conn),
                        None => MidiSender::Null,
                    };
                    if sender.send(&msg).is_err() {
                        current_conn = None;
                        current_port = None;
                    }
                }
            }
            MidiActorCommand::RunSession {
                port_name,
                should_open_input,
                session_fn,
                reply,
            } => match ensure_port(&port_name, &mut current_port, &mut current_conn) {
                Ok(()) => {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run_session_impl(&port_name, should_open_input, &mut current_conn, session_fn)
                    }));
                    let reply_result = if let Ok(inner_result) = result {
                        if inner_result.is_err() {
                            current_conn = None;
                            current_port = None;
                        }
                        inner_result
                    } else {
                        current_conn = None;
                        current_port = None;
                        Err("MIDI session panicked".to_string())
                    };
                    let _ = reply.send_blocking(reply_result);
                }
                Err(e) => {
                    let _ = reply.send_blocking(Err(e));
                }
            },
        }
    }
}

/// (Re)connects `current_conn` to `port_name` if it isn't already open, tearing down first if the port changed or is now invalid.
fn ensure_port(port_name: &str, current_port: &mut Option<String>, current_conn: &mut Option<MidiOutputConnection>) -> Result<(), String> {
    if !is_valid_port(port_name) {
        *current_conn = None;
        *current_port = None;
        return Ok(());
    }

    if current_port.as_deref() == Some(port_name) {
        return Ok(());
    }

    *current_conn = None;
    *current_port = None;

    if port_name == MANUAL_PORT {
        #[cfg(unix)]
        {
            use midir::os::unix::VirtualOutput;
            let midi_out = MidiOutput::new(CLIENT_NAME).map_err(|e| e.to_string())?;
            let conn = midi_out.create_virtual("C7 out").map_err(|e| e.to_string())?;
            *current_conn = Some(conn);
        }
        *current_port = Some(port_name.to_string());
        return Ok(());
    }

    let midi_out = MidiOutput::new(CLIENT_NAME).map_err(|e| e.to_string())?;
    let ports = midi_out.ports();
    let target = ports.iter().find(|port| {
        midi_out
            .port_name(port)
            .is_ok_and(|name| name == port_name || port_name.contains(&name) || name.contains(port_name))
    });

    if let Some(port) = target {
        let conn = midi_out.connect(port, CLIENT_NAME).map_err(|e| e.to_string())?;
        *current_conn = Some(conn);
        *current_port = Some(port_name.to_string());
        Ok(())
    } else {
        Err(format!("Could not find MIDI output port: {port_name}"))
    }
}

/// Runs a session closure with the open output connection, optionally opening the matching input port first.
///
/// `should_open_input` opens the input (handshake sessions) or skips it (output-only sends), and the closure gets `Some`/`None` to match.
fn run_session_impl(
    port_name: &str,
    should_open_input: bool,
    connection: &mut Option<MidiOutputConnection>,
    session_fn: MidiSessionFn,
) -> Result<(), String> {
    let midi_in = if should_open_input {
        let in_port = find_input_port(port_name).ok_or_else(|| format!("No matching input port for: {port_name:?}"))?;
        Some(open_large_sysex_input(&in_port)?)
    } else {
        None
    };

    let mut sender = match connection {
        Some(conn) => MidiSender::Real(conn),
        None => MidiSender::Null,
    };

    session_fn(midi_in.as_ref(), &mut sender)
}

/// Waits out the rest of the configured gap since the last standalone send, then records now as the reference point for the next one.
///
/// Does nothing when the gap is 0 (AUTO). Runs on the actor thread, so this blocking sleep never touches the GTK main loop.
fn pace_standalone_send(last_send: &mut Option<Instant>) {
    let gap_ms = MIDI_DELAY_MS.load(Ordering::Relaxed);
    if gap_ms > 0
        && let Some(prev) = *last_send
    {
        let elapsed = prev.elapsed();
        let gap = Duration::from_millis(gap_ms as u64);
        if let Some(remaining) = gap.checked_sub(elapsed) {
            sleep(remaining);
        }
    }
    *last_send = Some(Instant::now());
}

/// Shared core of `run_midi_session()` and `run_midi_output()`.
///
/// Hands the actor a session command, opening the input port when `should_open_input`.
async fn run_session_command<F, R>(port_name: &str, should_open_input: bool, func: F) -> Result<R, String>
where
    F: FnOnce(Option<&PollableMidiInput>, &mut MidiSender) -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    let (res_tx, res_rx) = async_channel::bounded(1);
    let (reply_tx, reply_rx) = async_channel::bounded(1);

    let session_fn = move |midi_in: Option<&PollableMidiInput>, sender: &mut MidiSender| -> Result<(), String> {
        match func(midi_in, sender) {
            Ok(val) => {
                let _ = res_tx.send_blocking(val);
                Ok(())
            }
            Err(err) => Err(err),
        }
    };

    let cmd = MidiActorCommand::RunSession {
        port_name: port_name.to_string(),
        should_open_input,
        session_fn: Box::new(session_fn),
        reply: reply_tx,
    };

    send_to_actor(cmd).await?;

    let reply_result = reply_rx.recv().await.map_err(|e| e.to_string())?;
    match reply_result {
        Ok(()) => res_rx.recv().await.map_err(|e| e.to_string()),
        Err(e) => Err(e),
    }
}

/// Performs the arm-then-blob transfer (rawmidi on Linux, midir fallback elsewhere).
///
/// Private because it blocks the calling thread for the whole transfer.
/// Callers use the async `send_large_sysex()`, which offloads this to a worker thread.
fn send_large_sysex_blocking(port_name: &str, arm_bytes: &[u8], blob_bytes: &[u8], arm_delay: f64, post_delay: f64) -> bool {
    #[cfg(target_os = "linux")]
    use std::io::Write;

    let _busy = midi_busy_session();

    #[cfg(target_os = "linux")]
    if let Some(path) = find_rawmidi_path(port_name) {
        // Close the actor's connection.
        let (reply_tx, reply_rx) = async_channel::bounded(1);
        let _ = ACTOR_SENDER.send_blocking(MidiActorCommand::InitPort {
            port_name: String::new(),
            reply: reply_tx,
        });
        let _ = reply_rx.recv_blocking();

        let ok = match std::fs::OpenOptions::new().write(true).open(&path) {
            Ok(mut dev) => {
                if !arm_bytes.is_empty() {
                    let _ = dev.write_all(arm_bytes);
                    let _ = dev.flush();
                    notify_outbound_midi(arm_bytes);
                    sleep(Duration::from_secs_f64(arm_delay));
                }
                let _ = dev.write_all(blob_bytes);
                let _ = dev.flush();
                notify_outbound_midi(blob_bytes);
                true
            }
            Err(_) => false,
        };
        sleep(Duration::from_secs_f64(post_delay));
        return ok;
    }

    let arm_bytes_vec = arm_bytes.to_vec();
    let blob_bytes_vec = blob_bytes.to_vec();
    let (reply_tx, reply_rx) = async_channel::bounded(1);
    let cmd = MidiActorCommand::RunSession {
        port_name: port_name.to_string(),
        should_open_input: false,
        session_fn: Box::new(move |_inport, sender| {
            if !arm_bytes_vec.is_empty() {
                let _ = sender.send(&arm_bytes_vec);
                sleep(Duration::from_secs_f64(arm_delay));
            }
            let _ = sender.send(&blob_bytes_vec);
            sleep(Duration::from_secs_f64(post_delay));
            Ok(())
        }),
        reply: reply_tx,
    };
    if ACTOR_SENDER.send_blocking(cmd).is_err() {
        return false;
    }
    match reply_rx.recv_blocking() {
        Ok(Ok(())) => true,
        _ => port_name == MANUAL_PORT,
    }
}

/// Sends a command to the MIDI actor thread and awaits its result.
async fn send_to_actor(cmd: MidiActorCommand) -> Result<(), String> {
    ACTOR_SENDER.send(cmd).await.map_err(|e| e.to_string())
}

/// Sets the MIDI busy flag for the duration of a bulk operation and clears it on drop.
fn midi_busy_session() -> MidiBusyGuard {
    MIDI_BUSY.store(true, Ordering::Relaxed);
    MidiBusyGuard
}

/// Broadcasts an outgoing MIDI message to the `midi_monitor` screen.
fn notify_outbound_midi(msg: &[u8]) {
    let cbs = OUTBOUND_CALLBACKS.lock().unwrap().clone();
    for cb in &cbs {
        cb(msg);
    }
}

/// Resolves a midir port name to its ALSA rawmidi device path (e.g. `/dev/snd/midiC1D0`).
///
/// Uses `/proc/asound/cards` as the primary source.
///
/// Alphanumeric-normalized name matching handles differences in capitalization and whitespace between midir and ALSA representations.
#[cfg(target_os = "linux")]
fn find_rawmidi_path(port_name: &str) -> Option<String> {
    // Manual mode has no rawmidi device. Fall through to the virtual output port instead of substring-matching a random card.
    if port_name == MANUAL_PORT {
        return None;
    }

    let base = port_name.split(':').next().unwrap_or(port_name).trim();
    let normalized_base: String = base
        .chars()
        .filter(|char_val| char_val.is_alphanumeric())
        .map(|char_val| char_val.to_ascii_lowercase())
        .collect();

    if let Ok(cards) = std::fs::read_to_string("/proc/asound/cards") {
        for line in cards.lines() {
            let Some(rest) = line.trim().split_once('[') else {
                continue;
            };
            let card_num_str = rest.0.trim();
            let Ok(card_num) = card_num_str.parse::<u32>() else {
                continue;
            };
            let short_name = rest.1.split(']').next().unwrap_or("").trim();
            let normalized_short: String = short_name
                .chars()
                .filter(|char_val| char_val.is_alphanumeric())
                .map(|char_val| char_val.to_ascii_lowercase())
                .collect();
            if normalized_base == normalized_short
                || normalized_base.contains(&*normalized_short)
                || normalized_short.contains(&*normalized_base)
            {
                let path = format!("/dev/snd/midiC{card_num}D0");
                if std::path::Path::new(&path).exists() {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Lowercases a port name and removes a trailing " [N]" group index.
///
/// Used to pair `WinRT`'s separately indexed input and output endpoints of one device.
fn strip_port_index(name: &str) -> String {
    let name_trimmed = name.trim();
    let name_trimmed = match name_trimmed.rfind('[') {
        Some(idx) if name_trimmed.ends_with(']') => name_trimmed[..idx].trim_end(),
        _ => name_trimmed,
    };
    name_trimmed.to_lowercase()
}
