//! The C7 Bridge CLAP plugin.
//!
//! One CLAP plugin is made per enabled device, exposing every automatable device parameter to the DAW.
//!
//! Every note/knob from the DAW becomes a MIDI Note/CC or SysEx signal on the plugin's MIDI output.

/*
DAWs like Ableton do not support CLAP in 2026.
DAWs like Bitwig do not support CLAPs SysEx passthrough in 2026.

C7 Bridge is made to exact CLAP spec. If the DAW doesn't support the feature, then that's their problem.
*/

mod device;
mod params;

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nih_plug::prelude::*;

use c7_core::bridge_interfacing::{MSG_CC_PARAM, MSG_NOTE, MSG_NOTE_DROPPED, MSG_PITCH_BEND, spawn_monitor_reconnect_thread};

use device::{Device, declare_device};
use params::{BridgeParams, OutMsg, RawSysEx, build_params, device_channel};

/// One loaded instance of one device's plugin.
pub struct C7Bridge<D: Device> {
    params: Arc<BridgeParams>,
    /// Queued output events from the param callbacks, emitted to the DAW in `process()`.
    rx: async_channel::Receiver<OutMsg>,
    /// Frame mirror into this instance's monitor thread.
    monitor_tx: async_channel::Sender<[u8; 4]>,
    /// Ends this instance's monitor thread when the DAW drops the plugin.
    should_shutdown: Arc<AtomicBool>,
    device: PhantomData<D>,
}

impl<D: Device> Default for C7Bridge<D> {
    /// Builds the param set from the device JSON and starts this instance's monitor connection.
    fn default() -> Self {
        let (tx, rx) = async_channel::unbounded::<OutMsg>();
        let (monitor_tx, monitor_rx) = async_channel::unbounded::<[u8; 4]>();
        let should_shutdown = Arc::new(AtomicBool::new(false));
        spawn_monitor_reconnect_thread(D::config().prod, monitor_rx, Arc::clone(&should_shutdown));
        Self {
            params: Arc::new(build_params(D::config(), &tx)),
            rx,
            monitor_tx,
            should_shutdown,
            device: PhantomData,
        }
    }
}

impl<D: Device> Drop for C7Bridge<D> {
    /// Stops the monitor thread.
    ///
    /// The server sees the closed socket as a disconnect.
    fn drop(&mut self) {
        self.should_shutdown.store(true, Ordering::Relaxed);
    }
}

impl<D: Device> Plugin for C7Bridge<D> {
    const NAME: &'static str = D::NAME;
    const VENDOR: &'static str = "sata1000000";
    const URL: &'static str = "https://github.com/sata1000000/C7";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// C7 Bridge doesn't work with audio.
    ///
    /// The plugin only turns automation and notes into MIDI output.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: None,
        ..AudioIOLayout::const_default()
    }];

    /// Notes/Pitchbend coming in from the DAW.
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    /// Notes/CC/SysEx going out to wherever the DAW routes them (a hardware MIDI port).
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs;

    type SysExMessage = RawSysEx;
    type BackgroundTask = ();

    /// Hands the DAW the generated param set.
    fn params(&self) -> Arc<dyn Params> {
        Arc::clone(&self.params) as Arc<dyn Params>
    }

    /// Drains the DAW's incoming event queue and flushes the device's own output for one processing block.
    ///
    /// Re-emits incoming DAW notes with the device's trigger routing.
    /// Forwards pitch bend on devices whose JSON says they act on it.
    /// Flushes queued param CC/SysEx events to the MIDI output.
    /// Everything emitted is mirrored to the monitor thread so the C7 app's log matches the wire.
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn {
                    timing,
                    channel,
                    note,
                    velocity,
                    ..
                } => match self.params.route_note(channel, note) {
                    Some((mapped_ch, mapped_note)) => {
                        context.send_event(NoteEvent::NoteOn {
                            timing,
                            voice_id: None,
                            channel: mapped_ch,
                            note: mapped_note,
                            velocity,
                        });
                        let _ = self
                            .monitor_tx
                            .try_send([MSG_NOTE, mapped_ch, mapped_note, (velocity * 127.0).round() as u8]);
                    }
                    None if !self.params.has_trig_notes => {
                        let _ = self.monitor_tx.try_send([MSG_NOTE_DROPPED, channel, 0, 0]);
                    }
                    None => {}
                },
                NoteEvent::NoteOff {
                    timing,
                    channel,
                    note,
                    velocity,
                    ..
                } => match self.params.route_note(channel, note) {
                    Some((mapped_ch, mapped_note)) => {
                        context.send_event(NoteEvent::NoteOff {
                            timing,
                            voice_id: None,
                            channel: mapped_ch,
                            note: mapped_note,
                            velocity,
                        });
                        let _ = self.monitor_tx.try_send([MSG_NOTE, mapped_ch, mapped_note, 0]);
                    }
                    None if !self.params.has_trig_notes => {
                        let _ = self.monitor_tx.try_send([MSG_NOTE_DROPPED, channel, 0, 0]);
                    }
                    None => {}
                },
                NoteEvent::MidiPitchBend {
                    timing,
                    channel,
                    value: val,
                } if self.params.can_pitch_bend => {
                    let Some(mapped_ch) = device_channel(&self.params.tracks, channel) else {
                        continue;
                    };
                    context.send_event(NoteEvent::MidiPitchBend {
                        timing,
                        channel: mapped_ch,
                        value: val,
                    });
                    // Same 14-bit conversion nih-plug uses to encode this event to wire bytes.
                    // So the log matches what actually goes out.
                    let raw = (val * 16383.0).round().clamp(0.0, 16383.0) as u16;
                    let _ = self
                        .monitor_tx
                        .try_send([MSG_PITCH_BEND, mapped_ch, (raw & 0x7F) as u8, (raw >> 7) as u8]);
                }
                _ => {}
            }
        }

        // CLAP output events must be non-decreasing in time, so queued events go at the block's last sample rather than sample 0.
        let last_timing = buffer.samples().saturating_sub(1) as u32;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                OutMsg::Cc { track_idx, cc, val } => {
                    let mapped_ch = device_channel(&self.params.tracks, track_idx).unwrap();
                    context.send_event(NoteEvent::MidiCC {
                        timing: last_timing,
                        channel: mapped_ch,
                        cc,
                        value: f32::from(val) / 127.0,
                    });
                    let _ = self.monitor_tx.try_send([MSG_CC_PARAM, mapped_ch, cc, val]);
                }
                OutMsg::Sysex { msg, frame } => {
                    context.send_event(NoteEvent::MidiSysEx {
                        timing: last_timing,
                        message: msg,
                    });
                    let _ = self.monitor_tx.try_send(frame);
                }
            }
        }
        ProcessStatus::Normal
    }
}

impl<D: Device> ClapPlugin for C7Bridge<D> {
    const CLAP_ID: &'static str = D::CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Automate Elektron hardware parameters from the DAW");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect, ClapFeature::Utility];
}

// Includes the auto-discovered `declare_device!(...)` calls and the `nih_export_clap!` call.
include!(concat!(env!("OUT_DIR"), "/devices.rs"));
