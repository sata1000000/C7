//! Functions for BPM detection from MIDI clock.

use std::collections::VecDeque;
use std::time::Instant;

use crate::utils::round_to_one_decimal;

// -----------------------------------------------------------------------------------------------------------
// Public API.
// -----------------------------------------------------------------------------------------------------------

/// Monitors incoming MIDI clock signals to determine and smooth the current tempo (BPM).
pub struct BpmDetector {
    /// Receives 0.0 when the detector resets.
    on_bpm_changed: Option<Box<dyn Fn(f64) + Send>>,
    /// Timestamps of the last ≤24 MIDI clock pulses, held as `Instant` so NTP clock adjustments can't corrupt the interval math.
    clock_times: VecDeque<Instant>,
    bpm: f64,
    last_reported_bpm: f64,
}

impl BpmDetector {
    /// Initializes the BPM detector with a change callback and empty history.
    pub fn new(on_bpm_changed: Option<Box<dyn Fn(f64) + Send>>) -> Self {
        Self {
            on_bpm_changed,
            clock_times: VecDeque::new(),
            bpm: 0.0,
            last_reported_bpm: -1.0,
        }
    }

    /// Processes an incoming raw MIDI message for clock pulses and transport commands.
    pub fn on_midi_message(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        match data[0] {
            0xF8 => self.handle_clock(), // MIDI clock messages
            0xFA | 0xFC => self.reset(), // Start & Stop messages
            _ => {}
        }
    }

    /// Returns the current smoothed BPM as a one-shot read.
    /// Returns `None` if not enough data yet.
    pub fn get_bpm(&self) -> Option<f64> {
        if self.bpm > 0.0 {
            Some(round_to_one_decimal(self.bpm))
        } else {
            None
        }
    }

    /// Resets the timing history and clears the current BPM reading.
    pub fn reset(&mut self) {
        self.clock_times.clear();
        self.bpm = 0.0;
        self.last_reported_bpm = -1.0;
        if let Some(callback) = &self.on_bpm_changed {
            callback(0.0);
        }
    }

    /// Processes a MIDI clock pulse to calculate and smooth the BPM.
    fn handle_clock(&mut self) {
        let now = Instant::now();

        // Reset if it's been too long since the last clock pulse.
        if let Some(last) = self.clock_times.back()
            && now.duration_since(*last).as_secs_f64() > 2.0
        {
            self.reset();
        }

        self.clock_times.push_back(now);
        // Keep a history of the last 24 clock pulses (equivalent to one quarter note).
        if self.clock_times.len() > 24 {
            self.clock_times.pop_front();
        }

        if self.clock_times.len() == 24 {
            // Calculate the time deltas between the most recent MIDI clock ticks to average out transport jitter.
            let mut intervals: Vec<f64> = (0..23)
                .map(|i| self.clock_times[i + 1].duration_since(self.clock_times[i]).as_secs_f64())
                .collect();
            // Remove the highest and lowest values to ignore irregular timing spikes.
            intervals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let trimmed = &intervals[2..intervals.len() - 2];
            let clock_period = trimmed.iter().sum::<f64>() / trimmed.len() as f64;
            if clock_period > 0.0 {
                let quarter_note_time = clock_period * 24.0;
                let new_bpm = 60.0 / quarter_note_time;

                // Smooth the BPM changes gradually.
                if self.bpm == 0.0 {
                    self.bpm = new_bpm;
                } else {
                    self.bpm = self.bpm * 0.8 + new_bpm * 0.2;
                }

                let rounded_bpm = round_to_one_decimal(self.bpm);
                if self.last_reported_bpm != rounded_bpm {
                    self.last_reported_bpm = rounded_bpm;
                    if let Some(callback) = &self.on_bpm_changed {
                        callback(rounded_bpm);
                    }
                }
            }
        }
    }
}
