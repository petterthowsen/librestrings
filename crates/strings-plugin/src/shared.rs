//! State shared between the audio thread and the editor.
//!
//! The audio thread publishes a snapshot of the performer (telemetry) through
//! atomics once per block; the editor sends notes back through a lock-free
//! queue that the audio thread drains at the start of each block, and so do
//! the tuning window's changes (see `tuning`). Neither side ever waits for
//! the other.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::Relaxed};

use crossbeam_queue::ArrayQueue;
use nih_plug::prelude::AtomicF32;

use crate::params::BowLiftParam;
use crate::tuning::{LiveTuning, StringsUpdate};

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
    /// when it changes, and a keyswitch may have changed the bow lift since.
    BowLift(BowLiftParam),
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
    /// Changes whenever the performer is built again (a new sample rate),
    /// which drops the tuning window's changes; the editor then sends them again.
    pub engine: AtomicU32,
    /// The last string update the audio thread applied (`StringsUpdate::generation`).
    pub strings_generation: AtomicU32,
    pub sample_rate: AtomicF32,
    /// The rate the strings run at (the sample rate times the oversampling):
    /// string designs are fitted at it.
    pub string_rate: AtomicF32,
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
    /// A double stop's second note, or -1.
    pub second_note: AtomicI32,
    pub strings: [StringTelemetry; 4],
    /// Bow velocity (m/s) and force on the bowed string (N).
    pub bow_velocity: AtomicF32,
    pub bow_force: AtomicF32,
    /// Slip onsets per period of the bowed string, smoothed: 1 is Helmholtz
    /// motion, more is multiple slipping or raucous, 0 is not sounding.
    pub slips_per_period: AtomicF32,

    /// The controls as the performer has them, from parameters or MIDI.
    pub dynamics: AtomicF32,
    pub vibrato: AtomicF32,
    pub pressure: AtomicF32,
    pub bow_lift: AtomicU32,
}

impl Telemetry {
    pub fn bow_lift(&self) -> BowLiftParam {
        BowLiftParam::ALL[self.bow_lift.load(Relaxed) as usize % 2]
    }

    pub fn note(&self) -> Option<u8> {
        u8::try_from(self.note.load(Relaxed)).ok()
    }

    pub fn second_note(&self) -> Option<u8> {
        u8::try_from(self.second_note.load(Relaxed)).ok()
    }
}

pub struct Shared {
    pub telemetry: Telemetry,
    /// Notes from the editor's keyboard.
    pub gui_events: ArrayQueue<GuiEvent>,
    /// The tuning window's changes. The audio thread applies the last one.
    pub live_tuning: ArrayQueue<LiveTuning>,
    /// Refitted strings, to the audio thread and back (the old filters are
    /// freed on the editor's side, never on the audio thread).
    pub string_updates: ArrayQueue<Box<StringsUpdate>>,
    pub string_returns: ArrayQueue<Box<StringsUpdate>>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            telemetry: Telemetry::default(),
            gui_events: ArrayQueue::new(256),
            live_tuning: ArrayQueue::new(8),
            string_updates: ArrayQueue::new(4),
            string_returns: ArrayQueue::new(8),
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
