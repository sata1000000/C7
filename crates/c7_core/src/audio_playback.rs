//! Audio playback on the host computer.
//!
//! Plays sample data through the local computer's speakers via cpal.
//! Trim, loop, and bit depth apply live, so a preview matches the settings it would be uploaded with.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::dsp_utils::resample_i16;

/// Playback control state shared between the audio callback thread and the UI thread.
///
/// The buffer is the full (uncropped) sample.
/// Trim and loop are stored as fractions [0,1] of it, applied live by the audio callback.
/// So dragging the markers (or swapping the buffer on a rate change) takes effect without rebuilding anything.
struct PlaybackShared {
    samples: Mutex<Vec<i16>>,
    samples_len: AtomicUsize,
    /// Current read index into the full buffer.
    position: AtomicUsize,

    // Trim/loop data as f64-bit fractions.
    trim_start: AtomicU64,
    trim_end: AtomicU64,
    loopdata_start: AtomicU64,
    loopdata_end: AtomicU64,
    is_loop_active: AtomicBool,
    /// `true` while producing audible output, `false` once finished or idle.
    is_playing: AtomicBool,
}

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Host audio preview player.
///
/// Opens the default output device once and keeps the stream alive for its entire lifetime.
/// `start()`/`release_loop()` only adjust shared state instead of rebuilding the stream every press.
/// This avoids per-press reconnect latency/clicks.
pub struct PreviewPlayer {
    _stream: cpal::Stream,
    shared: Arc<PlaybackShared>,
    // The rate the output stream actually negotiated.
    //
    // Hardware/drivers very often don't offer the requested rate directly, in which case `new()` falls back to the device's default.
    // (WASAPI shared mode in particular usually locks to one rate, e.g. 44100/48000.)
    // So buffers handed to `start()` must be resampled to this rate, or they play back at the wrong speed.
    output_rate_hz: f64,
}

impl PreviewPlayer {
    /// Opens the default output device and starts a (silent until `start()` is called) stream.
    ///
    /// Requests a common high rate.
    /// If the device doesn't offer it, falls back to its default config, and buffers are resampled to whatever rate it negotiates instead.
    pub fn new() -> Result<Self, String> {
        /// Preferred output rate when picking the device config.
        ///
        /// Buffers are resampled to whatever rate the device actually negotiates, so the exact value barely matters.
        const DESIRED_RATE: u32 = 48000;

        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("No audio output device found")?;

        let supported_config = device
            .supported_output_configs()
            .map_err(|e| format!("Failed to query output configs: {e}"))?
            .find(|cfg| cfg.channels() >= 1 && cfg.min_sample_rate() <= DESIRED_RATE && DESIRED_RATE <= cfg.max_sample_rate())
            .map(|range| range.with_sample_rate(DESIRED_RATE))
            .or_else(|| device.default_output_config().ok())
            .ok_or("No usable output config found on the default audio device")?;

        let channels = supported_config.channels() as usize;
        let sample_format = supported_config.sample_format();

        // Pick a small fixed buffer so `position` (advanced once per callback) refreshes enough for a smooth playhead.
        //
        // Backends otherwise default to whatever block size they like.
        // Small on WASAPI (~10 ms), large on many Linux backends (tens of ms), so the marker holds for several frames then jumps (stutter).
        //
        // Clamp a ~512-frame target into the device's advertised range. If the device reports no range (e.g. WASAPI), leave Default.
        // Windows is already smooth, and the actual audio-thread position is unchanged either way, so loop/trim rendering is untouched.
        let buffer_size = match supported_config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => {
                const TARGET_FRAMES: u32 = 512;
                cpal::BufferSize::Fixed(TARGET_FRAMES.clamp(*min, *max))
            }
            cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
        };

        let mut cfg: cpal::StreamConfig = supported_config.into();
        cfg.buffer_size = buffer_size;
        let output_rate_hz = f64::from(cfg.sample_rate);

        let shared = Arc::new(PlaybackShared {
            samples: Mutex::new(Vec::new()),
            samples_len: AtomicUsize::new(0),
            position: AtomicUsize::new(0),
            trim_start: AtomicU64::new(0.0f64.to_bits()),
            trim_end: AtomicU64::new(1.0f64.to_bits()),
            loopdata_start: AtomicU64::new(0.0f64.to_bits()),
            loopdata_end: AtomicU64::new(1.0f64.to_bits()),
            is_loop_active: AtomicBool::new(false),
            is_playing: AtomicBool::new(false),
        });

        // If the device advertised a buffer range but still rejects the fixed size, fall back to its default so preview audio always works.
        // Only the smoother playhead is lost on that one device.
        let stream = match build_stream(&device, cfg, sample_format, channels, Arc::clone(&shared)) {
            Ok(opened_stream) => opened_stream,
            Err(_) if !matches!(cfg.buffer_size, cpal::BufferSize::Default) => {
                cfg.buffer_size = cpal::BufferSize::Default;
                build_stream(&device, cfg, sample_format, channels, Arc::clone(&shared))?
            }
            Err(e) => return Err(e),
        };
        stream.play().map_err(|e| format!("Failed to start output stream: {e}"))?;

        Ok(Self {
            _stream: stream,
            shared,
            output_rate_hz,
        })
    }

    /// (Re)starts playback of the full (uncropped) `samples` from the trim start, looping `[loopdata_start, loopdata_end)` while active.
    ///
    /// `samples` are at `native_rate_hz` and get resampled to the output rate.
    /// Trim and loop are fractions [0,1] of the sample.
    ///
    /// Always restarts, even if already playing, so callers never special-case "pressed while already playing".
    pub fn start(
        &self,
        samples: &[i16],
        native_rate_hz: f64,
        trim_start: f64,
        trim_end: f64,
        loopdata_start: f64,
        loopdata_end: f64,
        is_loop_active: bool,
        bits: u8,
    ) {
        let samples = resample_i16(samples, native_rate_hz.round() as u32, self.output_rate_hz.round() as u32);
        let samples = quantize_bits(samples, bits); // match the chosen upload bit depth, on the played buffer
        let len = samples.len();
        store_frac(&self.shared.trim_start, trim_start);
        store_frac(&self.shared.trim_end, trim_end);
        store_frac(&self.shared.loopdata_start, loopdata_start);
        store_frac(&self.shared.loopdata_end, loopdata_end);
        self.shared.is_loop_active.store(is_loop_active, Ordering::Relaxed);
        let start_idx = ((trim_start * len as f64) as usize).min(len.saturating_sub(1));
        let mut buffer = self.shared.samples.lock().unwrap();
        *buffer = samples;
        drop(buffer);
        self.shared.samples_len.store(len, Ordering::Relaxed);
        self.shared.position.store(start_idx, Ordering::Relaxed);
        self.shared.is_playing.store(len > 0, Ordering::Relaxed);
    }

    /// Swaps in a newly rated `samples` buffer without restarting, keeping playback at the same fractional position.
    ///
    /// Trim and loop are fractions, so they carry over unchanged. Only the buffer and the position index need rescaling to the new length.
    ///
    /// Has no effect when nothing is currently playing (a rate change shouldn't start sound from silence).
    pub fn resample_live(&self, samples: &[i16], native_rate_hz: f64, bits: u8) {
        if !self.shared.is_playing.load(Ordering::Relaxed) {
            return;
        }
        let samples = resample_i16(samples, native_rate_hz.round() as u32, self.output_rate_hz.round() as u32);
        let samples = quantize_bits(samples, bits); // re-apply the chosen upload bit depth on the played buffer
        let new_len = samples.len();
        // Hold the lock across read-old-len / swap / set-position, so the callback can't read a new buffer against a stale position.
        let mut buffer = self.shared.samples.lock().unwrap();
        let old_len = buffer.len();
        let frac = if old_len > 0 {
            self.shared.position.load(Ordering::Relaxed) as f64 / old_len as f64
        } else {
            0.0
        };
        *buffer = samples;
        self.shared.samples_len.store(new_len, Ordering::Relaxed);
        let new_pos = ((frac * new_len as f64).round() as usize).min(new_len);
        self.shared.position.store(new_pos, Ordering::Relaxed);
    }

    /// Updates the trim bounds live (fractions [0,1] of the sample), which the audio callback applies on its next block.
    pub fn set_trim(&self, start: f64, end: f64) {
        store_frac(&self.shared.trim_start, start);
        store_frac(&self.shared.trim_end, end);
    }

    /// Updates the loop region live (fractions [0,1] of the sample), without changing whether looping is active.
    ///
    /// Whether the loop is engaged is owned by press (`start()`) and release (`release_loop()`).
    /// So dragging the markers only moves the region, and never re-arms a one-shot that's already playing out.
    pub fn set_loop_region(&self, start: f64, end: f64) {
        store_frac(&self.shared.loopdata_start, start);
        store_frac(&self.shared.loopdata_end, end);
    }

    /// Stops further looping.
    ///
    /// Playback continues forward to the `trim_end` and stops naturally.
    pub fn release_loop(&self) {
        self.shared.is_loop_active.store(false, Ordering::Relaxed);
    }

    /// Silences playback immediately, unlike `release_loop()` which lets the current pass finish naturally.
    ///
    /// Used for an explicit cancel (e.g. pressing Escape) rather than a normal release.
    pub fn stop(&self) {
        self.shared.is_playing.store(false, Ordering::Relaxed);
        self.shared.is_loop_active.store(false, Ordering::Relaxed);
    }

    /// Returns the playback position as a fraction [0,1] of the whole sample.
    /// Returns `None` once playback has finished.
    pub fn position_frac(&self) -> Option<f64> {
        if !self.shared.is_playing.load(Ordering::Relaxed) {
            return None;
        }
        let len = self.shared.samples_len.load(Ordering::Relaxed);
        if len == 0 {
            return None;
        }
        let pos = self.shared.position.load(Ordering::Relaxed);
        Some(pos as f64 / len as f64)
    }
}

// -----------------------------------------------------------------------------------------------------------
// Private helpers.
// -----------------------------------------------------------------------------------------------------------

/// Stores an f64 fraction into an atomic.
fn store_frac(atomic: &AtomicU64, val: f64) {
    atomic.store(val.to_bits(), Ordering::Relaxed);
}

/// Quantizes samples to the top `bits` significant bits (8..=16). Does nothing at 16.
///
/// This is exactly what the SDS upload keeps, so the preview audibly matches the chosen upload depth.
/// Applied to the final, already-resampled buffer so no resample can smooth the quantization back out.
fn quantize_bits(mut buffer: Vec<i16>, bits: u8) -> Vec<i16> {
    if bits < 16 {
        let shift = u32::from(16 - bits);
        for sample in &mut buffer {
            *sample = (((i32::from(*sample)) >> shift) << shift) as i16;
        }
    }
    buffer
}

/// Dispatches to the right monomorphized stream builder for whichever sample type the device actually wants.
///
/// cpal devices report their own native buffer element type, so it isn't something callers can choose freely.
fn build_stream(
    device: &cpal::Device,
    cfg: cpal::StreamConfig,
    sample_format: SampleFormat,
    channels: usize,
    shared: Arc<PlaybackShared>,
) -> Result<cpal::Stream, String> {
    match sample_format {
        // Covers every sample format cpal can report.
        SampleFormat::F32 => build_stream_typed::<f32>(device, cfg, channels, shared),
        SampleFormat::F64 => build_stream_typed::<f64>(device, cfg, channels, shared),
        SampleFormat::I8 => build_stream_typed::<i8>(device, cfg, channels, shared),
        SampleFormat::I16 => build_stream_typed::<i16>(device, cfg, channels, shared),
        SampleFormat::I32 => build_stream_typed::<i32>(device, cfg, channels, shared),
        SampleFormat::I64 => build_stream_typed::<i64>(device, cfg, channels, shared),
        SampleFormat::U8 => build_stream_typed::<u8>(device, cfg, channels, shared),
        SampleFormat::U16 => build_stream_typed::<u16>(device, cfg, channels, shared),
        SampleFormat::U32 => build_stream_typed::<u32>(device, cfg, channels, shared),
        SampleFormat::U64 => build_stream_typed::<u64>(device, cfg, channels, shared),

        // I24/U24 and DSD use special container types that don't satisfy `FromSample<f32>`, so they fall through to the error below.
        other => Err(format!("Unsupported output sample format: {other:?}")),
    }
}

/// Opens the audio output stream and hooks up the callback that fills it with sound.
fn build_stream_typed<T>(
    device: &cpal::Device,
    cfg: cpal::StreamConfig,
    channels: usize,
    shared: Arc<PlaybackShared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    device
        .build_output_stream(
            cfg,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| fill_buffer(data, channels, &shared),
            |_| {},
            None,
        )
        .map_err(|e| format!("Failed to build output stream: {e}"))
}

/// Per-buffer audio callback.
fn fill_buffer<T>(output: &mut [T], channels: usize, shared: &PlaybackShared)
where
    T: SizedSample + FromSample<f32>,
{
    // Hold the buffer for the whole callback.
    // The lock is only ever contended on a rate change, which swaps the buffer between blocks rather than mid-block.
    let samples = shared.samples.lock().unwrap();
    let len = samples.len();

    // Resolve the live fractional params to concrete sample indices once per block.
    // Reading them here (not per frame) means an edit mid-block takes effect on the next block, not partway through this one.
    let start_idx = (load_frac(&shared.trim_start) * len as f64) as usize;
    let end_idx = ((load_frac(&shared.trim_end) * len as f64) as usize).min(len);
    let is_loop_active = shared.is_loop_active.load(Ordering::Relaxed);
    let loopdata_start = (load_frac(&shared.loopdata_start) * len as f64) as usize;
    let loopdata_end = (load_frac(&shared.loopdata_end) * len as f64) as usize;

    for frame in output.chunks_mut(channels) {
        let pos = shared.position.load(Ordering::Relaxed);
        // Play this frame only while the user has playback on and the head sits inside the trim window.
        let active = shared.is_playing.load(Ordering::Relaxed) && pos >= start_idx && pos < end_idx;

        // In bounds:
        // Emit the current sample as a normalized f32. `get()` guards against a stale pos past the buffer end.
        let val_f32 = if active {
            samples.get(pos).copied().unwrap_or(0) as f32 / 32768.0
        }
        // Out of bounds or already stopped:
        // Latch playback off so a live trim edit that moved the window past the head ends the sample instead of resuming, and emit silence.
        else {
            shared.is_playing.store(false, Ordering::Relaxed);
            0.0
        };

        // Write the same value to every channel (mono source fanned out to the device's channel count).
        let val = T::from_sample(val_f32);
        for sample in frame.iter_mut() {
            *sample = val;
        }

        // Advance the head only when a frame was actually played, so a stopped player holds its position.
        if active {
            let mut next = pos + 1;

            // Play head reached `loopdata_end`:
            // Wrap back to `loopdata_start`.
            if is_loop_active && loopdata_end > loopdata_start && next >= loopdata_end {
                next = loopdata_start;
            }
            // Not looping and the play head reached the `trim_end`:
            // Stop at the end of the sample.
            else if next >= end_idx {
                shared.is_playing.store(false, Ordering::Relaxed);
            }
            shared.position.store(next, Ordering::Relaxed);
        }
    }
}

/// Reads an f64 fraction stored in an atomic.
fn load_frac(atomic: &AtomicU64) -> f64 {
    f64::from_bits(atomic.load(Ordering::Relaxed))
}
