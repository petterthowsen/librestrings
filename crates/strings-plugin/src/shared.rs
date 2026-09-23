//! State shared between the audio thread and the editor.
//!
//! The audio thread publishes a snapshot of the performer (telemetry) through
//! atomics once per block; the editor sends notes back through a lock-free
//! queue that the audio thread drains at the start of each block. Neither side
//! ever waits for the other.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::Relaxed};

use crossbeam_queue::ArrayQueue;
use nih_plug::prelude::AtomicF32;

use crate::params::ArticulationParam;

#[derive(Clone, Copy, Debug)]
pub enum GuiEvent {
    NoteOn {
        note: u8,
        velocity: f32,
    },
    NoteOff {
        note: u8,
    },
    /// Clicked in the editor. It also sets the parameter, but that only acts
    /// when it changes, and a keyswitch may have changed the articulation since.
    Articulation(ArticulationParam),
}

#[derive(Default)]
pub struct StringTelemetry {
    /// The pitch the string is tuned to right now: finger, vibrato and
    /// intonation included (Hz).
    pub frequency: AtomicF32,
    /// Bow position, as a fraction of the vibrating length from the bridge.
    pub beta: AtomicF32,
    /// How firmly the bow is on the string, 0–1.
    pub contact: AtomicF32,
    /// RMS bridge force over the last block (N).
    pub level: AtomicF32,
}

#[derive(Default)]
pub struct Telemetry {
    /// The performer is built (after the host activates the plugin).
    pub ready: AtomicBool,
    pub sample_rate: AtomicF32,
    pub block_size: AtomicU32,
    /// Time spent in `process` as a fraction of the block's duration:
    /// smoothed, and a peak that decays.
    pub load: AtomicF32,
    pub load_peak: AtomicF32,
    /// Output peak (linear), decaying.
    pub output_peak: AtomicF32,
    /// Times the NaN guard reset the instrument.
    pub resets: AtomicU32,

    /// The sounding MIDI note, or -1.
    pub note: AtomicI32,
    pub string: AtomicU32,
    pub strings: [StringTelemetry; 4],
    /// Bow velocity (m/s) and force on the bowed string (N).
    pub bow_velocity: AtomicF32,
    pub bow_force: AtomicF32,
    /// Slip onsets per period of the bowed string, smoothed: 1 is Helmholtz
    /// motion, more is multiple slipping or raucous, 0 is not sounding.
    pub slips_per_period: AtomicF32,

    /// The controls as the performer has them, from parameters or MIDI.
    pub dynamics: AtomicF32,
    pub expression: AtomicF32,
    pub vibrato: AtomicF32,
    pub pressure: AtomicF32,
    pub articulation: AtomicU32,
}

impl Telemetry {
    pub fn articulation(&self) -> ArticulationParam {
        ArticulationParam::ALL[self.articulation.load(Relaxed) as usize % 3]
    }

    pub fn note(&self) -> Option<u8> {
        u8::try_from(self.note.load(Relaxed)).ok()
    }
}

pub struct Shared {
    pub telemetry: Telemetry,
    /// Notes from the editor's keyboard.
    pub gui_events: ArrayQueue<GuiEvent>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            telemetry: Telemetry::default(),
            gui_events: ArrayQueue::new(256),
        }
    }
}

impl Shared {
    /// Sends a note from the editor. A full queue (the audio thread isn't
    /// running) drops it.
    pub fn send(&self, event: GuiEvent) {
        let _ = self.gui_events.push(event);
    }
}
