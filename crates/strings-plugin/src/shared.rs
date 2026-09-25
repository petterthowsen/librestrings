//! State shared between the audio thread and the editor.
//!
//! The audio thread publishes a snapshot of player 0 (telemetry) through
//! atomics once per block; the editor sends notes back through a lock-free
//! queue that the audio thread drains at the start of each block, and so do
//! the tuning window's changes (see `tuning`). Neither side ever waits for
//! the other.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering::Relaxed};

use crossbeam_queue::ArrayQueue;
use nih_plug::prelude::AtomicF32;
use strings_dsp::ARTICULATIONS;

use crate::params::{BowLiftParam, InstrumentParam, PolyphonyParam};
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
    /// The pedal button: the sustain pedal down or up, as CC64.
    Sustain(bool),
    /// Clicked in the editor, as [`GuiEvent::BowLift`]: the parameter only
    /// acts when it changes, and a keyswitch may have set it since.
    Polyphony(PolyphonyParam),
}

#[derive(Default)]
pub struct StringTelemetry {
    /// The pitch the string is tuned to right now: finger, vibrato and
    /// intonation included (Hz).
    pub frequency: AtomicF32,
    /// A finger stops the string (it stays down after the note until the
    /// next note goes to another string).
    pub finger: AtomicBool,
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
    /// Changes whenever the performer is built again (a new sample rate or
    /// instrument), which drops the tuning window's changes; the editor then
    /// sends them again.
    pub engine: AtomicU32,
    /// The instrument the engine plays (an index into `InstrumentParam::ALL`).
    /// It follows the parameter once the new engine is built.
    pub instrument: AtomicU32,
    /// The last string update the audio thread applied (`StringsUpdate::generation`).
    pub strings_generation: AtomicU32,
    pub sample_rate: AtomicF32,
    /// The rate player 0's strings run at (the sample rate times the
    /// oversampling): string designs are fitted at it. The other players'
    /// strings run at `player_string_rate`.
    pub string_rate: AtomicF32,
    pub player_string_rate: AtomicF32,
    /// Players in the section.
    pub players: AtomicU32,
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
    /// Every note the section is sounding, one bit per MIDI note: the notes
    /// of a divisi chord (each on its own player), not only player 0's, so
    /// the on-screen keyboard lights the whole chord.
    pub sounding: [AtomicU64; 2],
    pub strings: [StringTelemetry; 4],
    /// Bow velocity (m/s) and force on the bowed string (N).
    pub bow_velocity: AtomicF32,
    pub bow_force: AtomicF32,
    /// Player 0's last few articulations, newest first: indices into
    /// `Articulation::ALL`, or -1. The count changes with every new one.
    pub articulations: [AtomicI32; ARTICULATIONS],
    pub articulation_count: AtomicU32,
    /// The current (or last) stroke: 1 down-bow, -1 up-bow.
    pub bow_direction: AtomicF32,
    /// The sustain pedal is down.
    pub sustain: AtomicBool,
    /// Slip onsets per period of the bowed string, smoothed: 1 is Helmholtz
    /// motion, more is multiple slipping or raucous, 0 is not sounding.
    pub slips_per_period: AtomicF32,

    /// The controls as the performer has them, from parameters or MIDI.
    pub dynamics: AtomicF32,
    pub vibrato: AtomicF32,
    pub pressure: AtomicF32,
    pub bow_lift: AtomicU32,
    /// The live polyphony mode (an index into `PolyphonyParam::ALL`), which a
    /// keyswitch may have set away from the parameter.
    pub polyphony: AtomicU32,
}

impl Telemetry {
    pub fn instrument(&self) -> InstrumentParam {
        let i = self.instrument.load(Relaxed) as usize;
        InstrumentParam::ALL[i % InstrumentParam::ALL.len()]
    }

    pub fn bow_lift(&self) -> BowLiftParam {
        BowLiftParam::ALL[self.bow_lift.load(Relaxed) as usize % 2]
    }

    pub fn polyphony(&self) -> PolyphonyParam {
        PolyphonyParam::ALL[self.polyphony.load(Relaxed) as usize % PolyphonyParam::ALL.len()]
    }

    pub fn note(&self) -> Option<u8> {
        u8::try_from(self.note.load(Relaxed)).ok()
    }

    pub fn second_note(&self) -> Option<u8> {
        u8::try_from(self.second_note.load(Relaxed)).ok()
    }

    /// The section is sounding `note` (any player's note, not only player 0's).
    pub fn is_sounding(&self, note: u8) -> bool {
        let i = note as usize;
        self.sounding[i / 64].load(Relaxed) & (1 << (i % 64)) != 0
    }
}

pub struct Shared {
    pub telemetry: Telemetry,
    /// Notes from the editor's keyboard.
    pub gui_events: ArrayQueue<GuiEvent>,
    /// The tuning window's changes, for the instrument they were made for.
    /// The audio thread applies the last one for its instrument.
    pub live_tuning: ArrayQueue<(InstrumentParam, LiveTuning)>,
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
