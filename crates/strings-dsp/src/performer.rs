//! The performer: turns notes and controllers into bow and finger motion
//! (PLAN.md section 4).
//!
//! It plays one [`Instrument`]: one note at a time, or in [`Polyphony::DoubleStops`]
//! two notes on adjacent strings. Per sample it drives the bow velocity and,
//! per string, how firmly the bow is on it (the contact, 0–1). Bow force
//! follows from the calibrated Helmholtz band ([`ForceLimits`]):
//! `F = contact · F_band(p, |v_b|)`, where `p` is a position in the band (set
//! by the pressure control) and the band scales with bow speed. Because the
//! force follows the bow speed down to a floor, a decelerating bow stays
//! inside the band, so the Helmholtz amplitude follows the bow down and a
//! stopped note ends quickly.
//!
//! Stopped notes are intonated by ear, as a player does. The bowed pitch of the
//! model drifts from the string's tuning with bow force and position (the
//! flattening effect, and stiffness, which sharpens the Helmholtz pitch of
//! high notes). The performer measures the period between slips on the bowed
//! string, which is exactly one period in Helmholtz motion, and slowly moves
//! the finger to correct it. Open strings can't be corrected.
//!
//! The left hand has a position: notes within its span change as a finger
//! drops or lifts, at once; only a shift to another position slides.
//!
//! Real-time safe: no allocation after construction. String parameters (pitch
//! and bow position) are updated at [`CONTROL_RATE`], the bow every sample.
//!
//! [`ForceLimits`]: crate::instrument::ForceLimits

use crate::instrument::{Instrument, InstrumentSpec};
use crate::string::{BowInput, StringFrame};

/// What the bow does at the end of a detached note (SWAM's bow lift). There
/// is one way of playing: a detached note is a new stroke, its velocity
/// setting the attack, and overlapping notes play legato. The bow lift, the
/// velocity and the note's length together give the articulations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BowLift {
    /// The bow slows and leaves the string, which rings on. A short note is
    /// thrown off as quickly as it was played (spiccato-like).
    #[default]
    OffString,
    /// The bow stops on the string and stays there, which damps it: short
    /// notes are staccato. The next stroke starts from the string with a grip
    /// and a bite; pressed hard, it is a martelé.
    OnString,
}

/// One note at a time, or two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Polyphony {
    /// Overlapping notes play legato.
    #[default]
    Mono,
    /// A note that comes while another sounds joins it on an adjacent string
    /// (a double stop) if one hand can play both; otherwise it plays legato.
    DoubleStops,
}

/// Where the left hand prefers to play, which chooses the strings (as in
/// Audio Modeling's SWAM). Each mode plays notes up to a number of semitones
/// above a lower string's open pitch on that lower string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fingering {
    /// The lowest positions, with open strings.
    #[default]
    NutAndOpen,
    /// Middle positions: no open strings but the lowest (`mid_bias`).
    Mid,
    /// High positions on lower strings, the darker color (`bridge_bias`).
    Bridge,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerformerSettings {
    /// Bow speed (m/s) at dynamics 0 and 1, interpolated in log space.
    pub speed: (f32, f32),
    /// Bow position (fraction of the vibrating length from the bridge) at
    /// dynamics 0 and 1: players move toward the bridge as they play louder.
    pub beta: (f32, f32),
    /// Closest the bow comes to the bridge, per unit of string impedance
    /// (m per kg/s): heavier strings are bowed farther out. High on a string
    /// the vibrating length is short, and β of it would put the bow closer
    /// than players do, so the bow keeps this distance and β rises there.
    /// Closer, the model's strings (with the bow hair) leave Helmholtz motion
    /// high up the C and G strings, playing sharp and noisy (PLAN.md "High
    /// positions: the bow's distance from the bridge").
    pub bow_distance: f32,
    /// Position in the Helmholtz band at the middle of the pressure control
    /// (normal playing): 0 is the band's lower edge, 1 its upper edge.
    pub pressure: f32,
    /// Band positions at the ends of the pressure control: flautando (0) and
    /// scratch (1). They may lie outside the band, where the string leaves
    /// Helmholtz motion.
    pub pressure_range: (f32, f32),
    /// Vibrato rate (Hz).
    pub vibrato_rate: f32,
    /// Vibrato depth (semitones, either way) at full vibrato control.
    pub vibrato_depth: f32,
    /// Output gain applied after the body.
    pub output_gain: f32,
    /// Seed for the humanizing drift.
    pub seed: u32,
    /// String samples per output sample, 1 or 2 (see [`Instrument::new`]).
    /// Fixed when the performer is built; [`Performer::set_settings`] keeps it.
    pub oversampling: usize,
    /// Timings and gestures.
    pub tuning: PerformerTuning,
}

impl Default for PerformerSettings {
    fn default() -> Self {
        Self {
            speed: (0.04, 0.5),
            // Above β ≈ 0.12 the model's cello strings play up to 45 cents flat
            // (STATUS.md), so the mapping stays below.
            beta: (0.115, 0.07),
            // 3.5 cm on the C string, 1.4 cm on the A.
            bow_distance: 0.024,
            pressure: 0.65,
            pressure_range: (PRESSURE_FLAUTANDO, PRESSURE_SCRATCH),
            vibrato_rate: 5.5,
            vibrato_depth: 0.35,
            output_gain: 0.065,
            seed: 0x0b0e_5eed,
            oversampling: 2,
            tuning: PerformerTuning::default(),
        }
    }
}

/// The performer's timings and gestures (seconds unless noted). The plugin's
/// tuning window edits them while playing; pairs are at MIDI velocity 0 and 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerformerTuning {
    /// Bow landing on a string.
    pub land: f32,
    /// Force moving from one string to another in a crossing.
    pub crossing: f32,
    /// Bow acceleration at the start of a stroke off the string.
    pub attack: (f32, f32),
    /// A stroke off the string that starts while the bow still moves (fast
    /// détaché, where it never gets off the string) first changes bow: the
    /// bow slows to zero over this long, then accelerates as `attack` says.
    /// Reversing over the whole attack put the bow change 20–40 ms into the
    /// note, with the full force on a nearly still bow, which can choke the
    /// string (PLAN.md "Bow changes in fast détaché").
    pub bow_change: f32,
    /// At low dynamics attacks are slower, by up to this factor minus one at
    /// dynamics 0. At low force the bow must accelerate gently or the string
    /// starts in multiple slips (Guettler's attack diagram): below 1.6 quiet
    /// attacks on C2 and G2 can hold a double slip for up to a second.
    pub pp_attack: f32,
    /// The pressure (band position) tilts with dynamics: this much above the
    /// setting at dynamics 0 and as much below it at 1. The model's quiet
    /// attacks start cleanly only high in the band, and loud notes flatten
    /// less low in it.
    pub pressure_tilt: f32,
    /// Extra pressure (band position) at the start of a stroke off the string,
    /// at MIDI velocity 1 (scaled by velocity), fading out over
    /// `attack_bite_time`: the player's bite into the string.
    pub attack_bite: f32,
    pub attack_bite_time: f32,
    /// End of a note off the string: the bow slows and lifts off, over this
    /// long or the stroke's own length if shorter.
    pub release: f32,
    /// A stroke lasts at least this long, however short the key press.
    pub min_stroke: f32,
    /// Keys of a double stop let go within this long of each other end the
    /// stroke together; otherwise the first leaves its string.
    pub chord: f32,
    /// While the bow slows (a release or a stop), the force band follows the
    /// bow speed down to this fraction of the dynamics' speed, so the Helmholtz
    /// amplitude follows the bow and the stopped bow still holds the string.
    /// While it gets going, the force is that of the full speed from the
    /// start: it leads the speed, as in a player's attack.
    pub speed_floor: f32,

    /// A finger dropping onto or lifting off the string: legato notes within
    /// the hand's span, and fingers placed while the bow changes or crosses.
    pub place: f32,
    /// A legato shift of the hand, pressed hard: the finger slides this long.
    pub shift: f32,
    /// Legato notes landing below `portamento_velocity` slide (portamento),
    /// up to `portamento` long at velocity 0, in proportion below it.
    pub portamento: f32,
    pub portamento_velocity: f32,
    /// The hand covers this many semitones above its position (first to
    /// fourth finger, with extensions); legato notes beyond it need a shift.
    pub hand_span: f32,
    /// In a legato line, stay on the current string unless another one plays
    /// the note this many semitones lower in position.
    pub legato_stick: f32,
    /// Fingering modes: notes up to this many semitones above a lower
    /// string's open pitch are played on that lower string ([`Fingering`]).
    /// Just above a fifth avoids open strings; the nut mode uses 0.
    pub mid_bias: f32,
    pub bridge_bias: f32,

    /// On the string: the grip before the bow moves, the acceleration from
    /// the string, the extra pressure of the bite at velocity 1 (scaled by
    /// velocity), fading over `bite_time` once the bow moves, and the
    /// deceleration to a stop at the end of a note.
    pub grip: f32,
    pub grip_attack: (f32, f32),
    pub bite: f32,
    pub bite_time: f32,
    pub stop: f32,

    /// Loss at a stopping finger (nepers per reflection): the fingertip damps
    /// a stopped string, so it rings shorter than an open one.
    pub finger_loss: f32,
    /// Extra loss once the note is over and the finger eases off the string
    /// (nepers per reflection), fading in over `mute_time`. Open strings ring on.
    pub mute_loss: f32,
    pub mute_time: f32,

    /// Vibrato onset: delay after the stroke starts, then fade-in.
    pub vibrato_delay: f32,
    pub vibrato_fade: f32,

    /// A player never holds a stroke perfectly still. Slow random wander of
    /// the band position (absolute), bow speed and bow position (fractions),
    /// with segments about `wander_time` long.
    pub wander_pressure: f32,
    pub wander_speed: f32,
    pub wander_beta: f32,
    pub wander_time: f32,

    /// Intonation by ear: time constant, the largest pitch error still taken
    /// for a Helmholtz period (semitones; multiple slips give far larger
    /// ones), and the largest correction (semitones).
    pub ear_time: f32,
    pub ear_window: f32,
    pub ear_range: f32,
}

impl Default for PerformerTuning {
    fn default() -> Self {
        Self {
            land: 0.012,
            crossing: 0.03,
            attack: (0.12, 0.035),
            bow_change: 0.012,
            pp_attack: 1.6,
            pressure_tilt: 0.15,
            attack_bite: 0.2,
            attack_bite_time: 0.08,
            release: 0.15,
            min_stroke: 0.04,
            chord: 0.03,
            speed_floor: 0.15,
            place: 0.006,
            shift: 0.02,
            portamento: 0.25,
            portamento_velocity: 0.6,
            hand_span: 4.0,
            legato_stick: 5.0,
            mid_bias: 7.5,
            bridge_bias: 12.5,
            grip: 0.015,
            grip_attack: (0.1, 0.008),
            bite: 0.25,
            bite_time: 0.03,
            stop: 0.04,
            finger_loss: 0.015,
            mute_loss: 0.08,
            mute_time: 0.05,
            vibrato_delay: 0.12,
            vibrato_fade: 0.4,
            wander_pressure: 0.06,
            wander_speed: 0.08,
            wander_beta: 0.03,
            wander_time: 0.8,
            ear_time: 0.15,
            ear_window: 1.0,
            ear_range: 1.0,
        }
    }
}

/// Rate (Hz) at which pitch, bow position and the force band are updated.
pub const CONTROL_RATE: f32 = 3000.0;

/// Band positions at the ends of the pressure control (see
/// [`PerformerSettings::pressure_range`]). Flautando is the band's lower edge:
/// below it notes break into multiple slips and miss their pitch by up to a
/// semitone. Scratch is well above the upper edge, where nearly every note
/// is raucous and 5–10 dB louder than normal (PLAN.md "Phase 3 notes: playing
/// like SWAM").
pub const PRESSURE_FLAUTANDO: f32 = 0.0;
pub const PRESSURE_SCRATCH: f32 = 1.3;

/// Controller smoothing (s).
const CONTROL_SMOOTHING: f32 = 0.03;
/// Finger positions (semitones) below this are an open string.
const OPEN: f32 = 0.3;
/// Notes held at once (for legato returns).
const MAX_HELD: usize = 16;

/// Ramp from one value to another: a raised cosine, or for attacks a quarter
/// sine, which leaves with a finite slope and arrives smoothly.
#[derive(Clone, Copy, Debug)]
struct Ramp {
    from: f32,
    to: f32,
    pos: f32,
    step: f32,
    attack: bool,
}

impl Ramp {
    fn at(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            pos: 1.0,
            step: 0.0,
            attack: false,
        }
    }

    fn value(&self) -> f32 {
        if self.pos >= 1.0 {
            return self.to;
        }
        let s = if self.attack {
            (std::f32::consts::FRAC_PI_2 * self.pos).sin()
        } else {
            0.5 - 0.5 * (std::f32::consts::PI * self.pos).cos()
        };
        self.from + (self.to - self.from) * s
    }

    fn go(&mut self, to: f32, seconds: f32, sample_rate: f32) {
        self.from = self.value();
        self.to = to;
        self.pos = 0.0;
        self.step = 1.0 / (seconds * sample_rate).max(1.0);
        self.attack = false;
    }

    /// Like [`Self::go`], starting at a constant acceleration.
    fn attack(&mut self, to: f32, seconds: f32, sample_rate: f32) {
        self.go(to, seconds, sample_rate);
        self.attack = true;
    }

    fn set(&mut self, value: f32) {
        *self = Self::at(value);
    }

    fn tick(&mut self) {
        self.pos = (self.pos + self.step).min(1.0);
    }
}

/// One-pole smoother toward a target.
#[derive(Clone, Copy, Debug)]
struct Smooth {
    value: f32,
    target: f32,
    coef: f32,
}

impl Smooth {
    fn new(value: f32, seconds: f32, sample_rate: f32) -> Self {
        Self {
            value,
            target: value,
            coef: (-1.0 / (seconds * sample_rate)).exp(),
        }
    }

    fn tick(&mut self) -> f32 {
        self.value = self.target + self.coef * (self.value - self.target);
        self.value
    }
}

/// Smooth random motion in [−1, 1]: raised-cosine segments between random
/// points, each 0.6–1.4 × the mean length.
#[derive(Clone, Copy, Debug)]
struct Wander {
    from: f32,
    to: f32,
    pos: f32,
    length: f32,
    /// Mean segment length relative to the tuning's `wander_time`.
    scale: f32,
}

impl Wander {
    fn new(scale: f32) -> Self {
        Self {
            from: 0.0,
            to: 0.0,
            pos: 1.0,
            length: 1.0,
            scale,
        }
    }

    fn value(&self) -> f32 {
        self.from + (self.to - self.from) * (0.5 - 0.5 * (std::f32::consts::PI * self.pos).cos())
    }

    fn tick(&mut self, dt: f32, mean: f32, rng: &mut u32) {
        self.pos += dt / self.length;
        if self.pos >= 1.0 {
            self.from = self.to;
            self.to = 2.0 * next_random(rng) - 1.0;
            self.length = (mean * self.scale * (0.6 + 0.8 * next_random(rng))).max(1e-3);
            self.pos = 0.0;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Bow off the string.
    Idle,
    /// On the string, before the stroke: the force is on, the bow still.
    Grip,
    /// A stroke, note held (or not yet `min_stroke` long).
    Sustain,
    /// Off the string, note over: the bow slows and lifts.
    Release,
    /// On the string, note over: the bow slows to a stop.
    Stop,
    /// On the string, stopped, until the next note.
    Resting,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PerformerFrame {
    pub output: f32,
    /// Sum of the strings' bridge forces (N).
    pub bridge_force: f32,
    /// The bowed string and its frame.
    pub string: usize,
    pub frame: StringFrame,
    /// Bow velocity (m/s) and the force on the bowed string (N).
    pub bow_velocity: f32,
    pub bow_force: f32,
}

pub struct Performer {
    instrument: Instrument,
    settings: PerformerSettings,
    fs: f32,

    dynamics: Smooth,
    vibrato_control: Smooth,
    /// The pressure control, 0–1 (0.5 is normal).
    pressure: Smooth,
    bow_lift: BowLift,
    polyphony: Polyphony,
    fingering: Fingering,
    /// A string the notes stay on where they can (a "sul G" marking).
    sul: Option<usize>,

    held: [u8; MAX_HELD],
    held_len: usize,
    sounding: Option<u8>,
    string: usize,
    /// A double stop's second note and its string, the newer of the two.
    second: Option<(u8, usize)>,
    /// The hand's position: the lowest finger position (semitones above the
    /// open string) it covers without a shift, on every string.
    hand: f32,

    phase: Phase,
    /// Seconds since the phase began.
    phase_time: f32,
    /// Length of the current release (s).
    phase_length: f32,
    /// The key is up, but the stroke ends only once it is `min_stroke` long.
    ending: bool,
    /// A double stop's note let go, and the time left for the other to follow
    /// (`chord`) before its string is left alone.
    letting_go: Option<(u8, f32)>,
    /// MIDI velocity of the current stroke, 0–1.
    stroke_velocity: f32,
    /// Extra pressure at the start of the current stroke, and how long it
    /// fades once the bow moves.
    stroke_bite: f32,
    stroke_bite_time: f32,
    /// A stroke waiting for its bow change: the attack (s) to start once the
    /// bow has stopped.
    pending_attack: Option<f32>,
    direction: f32,
    /// Bow velocity as a multiple of the dynamics' bow speed, signed.
    velocity: Ramp,
    contact: [Ramp; 4],
    /// Finger position per string, in semitones above the open string.
    finger: [Ramp; 4],

    vibrato_phase: f32,
    vibrato_age: f32,
    rng: u32,
    rate_drift: Smooth,
    depth_drift: Smooth,
    drift_timer: f32,
    /// Bow wander: pressure, speed, position.
    wander: [Wander; 3],
    /// How far the finger has eased off each string, 0–1.
    mute: [f32; 4],
    termination_loss: [f32; 4],

    control_countdown: usize,
    control_interval: usize,
    /// Force per unit bow speed on each string (N·s/m) at the current β and pressure.
    unit_force: [f32; 4],
    speed: f32,
    frames: [StringFrame; 4],

    /// Finger correction per string from listening (semitones).
    intonation: [f32; 4],
    /// Pitch the bowed string should sound, vibrato included (Hz).
    intended: f32,
    samples_since_slip: usize,
    was_slipping: bool,
}

impl Performer {
    /// Slow (builds the instrument); don't call it on the audio thread.
    pub fn new(spec: &InstrumentSpec, settings: PerformerSettings, sample_rate: f32) -> Self {
        let fs = sample_rate;
        let control_interval = ((fs / CONTROL_RATE).round() as usize).max(1);
        let mut p = Self {
            instrument: Instrument::new(spec, fs, settings.oversampling),
            settings,
            fs,
            dynamics: Smooth::new(0.5, CONTROL_SMOOTHING, fs),
            vibrato_control: Smooth::new(0.0, CONTROL_SMOOTHING, fs),
            pressure: Smooth::new(0.5, CONTROL_SMOOTHING, fs),
            bow_lift: BowLift::OffString,
            polyphony: Polyphony::Mono,
            fingering: Fingering::NutAndOpen,
            sul: None,
            held: [0; MAX_HELD],
            held_len: 0,
            sounding: None,
            string: 0,
            second: None,
            hand: 0.0,
            phase: Phase::Idle,
            phase_time: 0.0,
            phase_length: 0.0,
            ending: false,
            letting_go: None,
            stroke_velocity: 0.0,
            stroke_bite: 0.0,
            stroke_bite_time: 0.0,
            pending_attack: None,
            direction: -1.0,
            velocity: Ramp::at(0.0),
            contact: [Ramp::at(0.0); 4],
            finger: [Ramp::at(0.0); 4],
            vibrato_phase: 0.0,
            vibrato_age: 0.0,
            rng: settings.seed.max(1),
            rate_drift: Smooth::new(0.0, 0.4, fs),
            depth_drift: Smooth::new(0.0, 0.4, fs),
            drift_timer: 0.0,
            wander: [Wander::new(1.0), Wander::new(1.37), Wander::new(1.73)],
            mute: [0.0; 4],
            termination_loss: [0.0; 4],
            control_countdown: 0,
            control_interval,
            unit_force: [0.0; 4],
            speed: 0.0,
            frames: [StringFrame::default(); 4],
            intonation: [0.0; 4],
            intended: 0.0,
            samples_since_slip: 0,
            was_slipping: false,
        };
        p.update_controls();
        p
    }

    pub fn instrument(&self) -> &Instrument {
        &self.instrument
    }

    /// For changing the instrument's parameters while playing (real-time safe
    /// setters only).
    pub fn instrument_mut(&mut self) -> &mut Instrument {
        &mut self.instrument
    }

    /// Every string's frame from the last sample.
    pub fn string_frames(&self) -> &[StringFrame; 4] {
        &self.frames
    }

    pub fn settings(&self) -> &PerformerSettings {
        &self.settings
    }

    /// Changes the settings while playing. The seed and the oversampling only
    /// apply at construction.
    pub fn set_settings(&mut self, settings: PerformerSettings) {
        self.settings = PerformerSettings {
            oversampling: self.settings.oversampling,
            ..settings
        };
    }

    /// The note being played, until the bow has left the string or stopped
    /// on it. In a double stop, the older of the two.
    pub fn note(&self) -> Option<u8> {
        self.sounding.filter(|_| self.bowing())
    }

    /// A double stop's second note and its string.
    pub fn second(&self) -> Option<(u8, usize)> {
        self.second.filter(|_| self.bowing())
    }

    fn bowing(&self) -> bool {
        !matches!(self.phase, Phase::Idle | Phase::Resting)
    }

    /// The string the bow plays (or last played), lowest is 0.
    pub fn bowed_string(&self) -> usize {
        self.string
    }

    /// How firmly the bow is on string `i`, 0 (off) to 1.
    pub fn contact(&self, i: usize) -> f32 {
        self.contact[i].value()
    }

    pub fn bow_lift(&self) -> BowLift {
        self.bow_lift
    }

    /// Dynamics, 0–1 (the plugin's CC11): bow speed, bow position and with
    /// them loudness.
    pub fn set_dynamics(&mut self, value: f32) {
        self.dynamics.target = value.clamp(0.0, 1.0);
    }

    /// Vibrato depth, 0–1 (the plugin's CC1).
    pub fn set_vibrato(&mut self, value: f32) {
        self.vibrato_control.target = value.clamp(0.0, 1.0);
    }

    /// Takes effect at the end of the current note. Off the string, a bow
    /// resting on it lifts.
    pub fn set_bow_lift(&mut self, bow_lift: BowLift) {
        self.bow_lift = bow_lift;
        if bow_lift == BowLift::OffString && self.phase == Phase::Resting {
            self.phase = Phase::Idle;
            self.lift(self.settings.tuning.release);
        }
    }

    /// Bow pressure, 0–1: flautando at 0, normal at 0.5, scratch at 1
    /// ([`PerformerSettings::pressure_range`]).
    pub fn set_pressure(&mut self, value: f32) {
        self.pressure.target = value.clamp(0.0, 1.0);
    }

    /// The pressure control (not smoothed).
    pub fn pressure(&self) -> f32 {
        self.pressure.target
    }

    /// Takes effect from the next note.
    pub fn set_polyphony(&mut self, polyphony: Polyphony) {
        self.polyphony = polyphony;
    }

    pub fn polyphony(&self) -> Polyphony {
        self.polyphony
    }

    /// Takes effect from the next note.
    pub fn set_fingering(&mut self, fingering: Fingering) {
        self.fingering = fingering;
    }

    pub fn fingering(&self) -> Fingering {
        self.fingering
    }

    /// Keeps single notes on one string (0 is the lowest) wherever it can
    /// play them, over the fingering mode: a "sul G" marking. `None` lets the
    /// fingering choose. Takes effect from the next note; double stops ignore it.
    pub fn set_string(&mut self, string: Option<usize>) {
        self.sul = string.filter(|&s| s < 4);
    }

    /// Silences everything at once.
    pub fn reset(&mut self) {
        self.instrument.reset();
        self.held_len = 0;
        self.sounding = None;
        self.second = None;
        self.hand = 0.0;
        self.phase = Phase::Idle;
        self.ending = false;
        self.letting_go = None;
        self.pending_attack = None;
        self.velocity.set(0.0);
        self.contact = [Ramp::at(0.0); 4];
        self.intonation = [0.0; 4];
    }

    /// Releases every held note as if the last one were let go (MIDI "all
    /// notes off"): the bow finishes its stroke instead of stopping dead.
    pub fn release_all(&mut self) {
        self.held_len = 0;
        self.letting_go = None;
        if let Some((_, string)) = self.second.take() {
            self.contact[string].go(0.0, self.settings.tuning.release, self.fs);
        }
        if let Some(note) = self.sounding {
            // Held again alone, so the release doesn't return legato to another note.
            self.hold(note);
            self.note_off(note);
        }
    }

    /// `velocity` is the MIDI velocity scaled to 0–1.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        let velocity = velocity.clamp(0.0, 1.0);
        self.hold(note);
        self.let_go();
        if self.polyphony == Polyphony::DoubleStops && self.join(note, velocity) {
            return;
        }
        let legato = matches!(self.phase, Phase::Grip | Phase::Sustain)
            && !self.ending
            && self.sounding.is_some();
        if legato {
            // A double stop that can't take the note goes on from its nearer one.
            self.keep_nearest(note);
            self.legato_to(note, velocity);
            return;
        }
        // A detached note: a new stroke, the bow direction alternating.
        self.drop_newer();
        self.sounding = Some(note);
        self.stroke_velocity = velocity;
        self.ending = false;
        let t = self.settings.tuning;
        let (string, semitones) = self.choose_string(note, None);
        self.move_to_string(string, semitones, t.place);
        self.reach(&[semitones]);
        self.direction = -self.direction;
        self.vibrato_age = 0.0;
        self.phase_time = 0.0;
        let fs = self.fs;
        match self.bow_lift {
            BowLift::OffString => {
                self.phase = Phase::Sustain;
                let quiet = 1.0 - self.dynamics.target;
                let attack = lerp(t.attack.0, t.attack.1, velocity) * (1.0 + t.pp_attack * quiet);
                if self.velocity.value().abs() > 0.05 {
                    // The bow still moves: a bow change first.
                    self.velocity.go(0.0, t.bow_change, fs);
                    self.pending_attack = Some(attack);
                } else {
                    self.pending_attack = None;
                    self.velocity.attack(self.direction, attack, fs);
                }
                self.contact[string].go(1.0, t.land, fs);
                self.stroke_bite = t.attack_bite * velocity;
                self.stroke_bite_time = t.attack_bite_time;
            }
            BowLift::OnString => {
                // The force is on before the bow moves.
                self.phase = Phase::Grip;
                self.pending_attack = None;
                self.velocity.go(0.0, t.place, fs);
                self.contact[string].go(1.0, t.grip, fs);
                self.stroke_bite = t.bite * velocity;
                self.stroke_bite_time = t.bite_time;
            }
        }
    }

    pub fn note_off(&mut self, note: u8) {
        self.release_held(note);
        let stroke = matches!(self.phase, Phase::Grip | Phase::Sustain) && !self.ending;
        // A double stop in a held stroke goes on with the other note, unless
        // that is let go too, within `chord`.
        if stroke && let Some((second, _)) = self.second {
            let playing = note == second || self.sounding == Some(note);
            match self.letting_go {
                Some((first, _)) if playing && first != note => {
                    self.letting_go = None;
                    self.end_note();
                }
                None if playing => {
                    self.letting_go = Some((note, self.settings.tuning.chord));
                }
                _ => {}
            }
            return;
        }
        if stroke && self.sounding == Some(note) {
            self.end_note();
        }
    }

    /// The note (or double stop) is let go: legato back to a note still held,
    /// or the end of the stroke.
    fn end_note(&mut self) {
        if let Some(&previous) = self.held[..self.held_len].last() {
            // Back to a note still held, legato.
            self.keep_nearest(previous);
            self.legato_to(previous, self.stroke_velocity);
        } else if self.phase == Phase::Sustain && self.phase_time >= self.settings.tuning.min_stroke
        {
            self.end_stroke();
        } else {
            self.ending = true;
        }
    }

    /// A double stop's note let go earlier leaves its string now.
    fn let_go(&mut self) {
        if let Some((note, _)) = self.letting_go.take() {
            if self.second.is_some_and(|(n, _)| n == note) {
                self.drop_newer();
            } else if self.sounding == Some(note) {
                self.drop_older();
            }
        }
    }

    pub fn process(&mut self) -> f32 {
        self.process_frame().output
    }

    pub fn process_frame(&mut self) -> PerformerFrame {
        if self.control_countdown == 0 {
            self.control_countdown = self.control_interval;
            self.update_controls();
        }
        self.control_countdown -= 1;
        self.dynamics.tick();
        self.vibrato_control.tick();
        self.pressure.tick();
        self.advance_phase();

        self.velocity.tick();
        let v = self.velocity.value() * self.speed;
        let speed_floor = self.settings.tuning.speed_floor;
        let floor = match self.phase {
            Phase::Grip | Phase::Sustain => self.velocity.to.abs().max(speed_floor),
            _ => speed_floor,
        };
        let v_force = v.abs().max(floor * self.speed);
        let mut bows = [BowInput::default(); 4];
        for (i, bow) in bows.iter_mut().enumerate() {
            self.contact[i].tick();
            self.finger[i].tick();
            let contact = self.contact[i].value();
            if contact > 0.0 {
                *bow = BowInput {
                    velocity: v,
                    force: contact * v_force * self.unit_force[i],
                };
            }
        }
        let out = self.instrument.process(&bows, &mut self.frames);
        self.listen(self.frames[self.string].state.is_slipping());
        PerformerFrame {
            output: out.output * self.settings.output_gain,
            bridge_force: out.bridge_force,
            string: self.string,
            frame: self.frames[self.string],
            bow_velocity: v,
            bow_force: bows[self.string].force,
        }
    }

    fn advance_phase(&mut self) {
        let dt = 1.0 / self.fs;
        let fs = self.fs;
        let t = self.settings.tuning;
        self.phase_time += dt;
        self.vibrato_age += dt;
        if let Some((_, left)) = &mut self.letting_go {
            *left -= dt;
            if *left <= 0.0 {
                self.let_go();
            }
        }
        match self.phase {
            Phase::Grip if self.phase_time >= t.grip => {
                self.phase = Phase::Sustain;
                self.phase_time = 0.0;
                let quiet = 1.0 - self.dynamics.target;
                let v = self.stroke_velocity;
                let attack =
                    lerp(t.grip_attack.0, t.grip_attack.1, v) * (1.0 + t.pp_attack * quiet);
                self.velocity.attack(self.direction, attack, fs);
            }
            Phase::Sustain if self.pending_attack.is_some() && self.phase_time >= t.bow_change => {
                let attack = self.pending_attack.take().unwrap_or(0.0);
                self.velocity.attack(self.direction, attack, fs);
            }
            Phase::Sustain if self.ending && self.phase_time >= t.min_stroke => self.end_stroke(),
            Phase::Release if self.phase_time >= self.phase_length => {
                self.phase = Phase::Idle;
                self.velocity.go(0.0, t.release, fs);
            }
            Phase::Stop if self.phase_time >= t.stop => {
                self.phase = Phase::Resting;
                self.phase_time = 0.0;
            }
            _ => {}
        }
    }

    /// Measures the period between slips on the bowed string and corrects the
    /// finger toward the intended pitch, while a stopped note is sustained.
    fn listen(&mut self, slipping: bool) {
        self.samples_since_slip += 1;
        let onset = slipping && !self.was_slipping;
        self.was_slipping = slipping;
        if !onset {
            return;
        }
        let period = self.samples_since_slip as f32;
        self.samples_since_slip = 0;
        let s = self.string;
        let finger = &self.finger[s];
        let listening = self.phase == Phase::Sustain
            && self.contact[s].value() > 0.99
            && finger.pos >= 1.0
            && finger.value() > 0.3;
        // Positive when flat.
        let error = 12.0 * (period * self.intended / self.fs).log2();
        let t = &self.settings.tuning;
        if listening && error.abs() < t.ear_window {
            let gain = (period / (self.fs * t.ear_time)).min(1.0);
            self.intonation[s] =
                (self.intonation[s] + gain * error).clamp(-t.ear_range, t.ear_range);
        }
    }

    /// The note is over. On the string the bow slows to a stop and stays;
    /// off it, the bow slows and lifts, a short stroke as quickly as it was
    /// played, and the string rings on.
    fn end_stroke(&mut self) {
        let t = self.settings.tuning;
        let fs = self.fs;
        let stroke = self.phase_time;
        self.ending = false;
        self.pending_attack = None;
        self.phase_time = 0.0;
        match self.bow_lift {
            BowLift::OnString => {
                self.phase = Phase::Stop;
                self.velocity.go(0.0, t.stop, fs);
            }
            BowLift::OffString => {
                self.phase = Phase::Release;
                // A short stroke is thrown off the string still moving; a
                // long one slows as it lifts.
                self.phase_length = t.release.min(stroke.max(t.min_stroke));
                if stroke >= t.release {
                    self.velocity
                        .go(0.4 * self.direction, self.phase_length, fs);
                }
                self.lift(self.phase_length);
            }
        }
    }

    fn legato_to(&mut self, note: u8, velocity: f32) {
        self.sounding = Some(note);
        let t = self.settings.tuning;
        let (string, semitones) = self.choose_string(note, Some(self.string));
        let shift = self.reach(&[semitones]);
        if string == self.string {
            self.finger_to(string, semitones, shift, velocity);
        } else {
            self.move_to_string(string, semitones, t.place);
        }
        // Vibrato carries on through a legato change, at no less than half depth.
        self.vibrato_age = self.vibrato_age.max(t.vibrato_delay + 0.5 * t.vibrato_fade);
    }

    /// Double stops: plays `note` with a sounding note on the adjacent string,
    /// if one hand can play both and a stroke is being held (a chord pressed
    /// together joins in the grip or the attack). Returns whether it did.
    ///
    /// In a double stop the new note leads on from the nearer note, as a voice
    /// does, and plays with the farther one (a line over a held note), or else
    /// with the nearer one.
    fn join(&mut self, note: u8, velocity: f32) -> bool {
        let t = self.settings.tuning;
        let joinable = matches!(self.phase, Phase::Grip | Phase::Sustain) && !self.ending;
        let Some(older) = self.sounding.filter(|_| joinable) else {
            return false;
        };
        let older = (older, self.string);
        let partners = match self.second {
            None => [Some(older), None],
            Some(newer) if note.abs_diff(newer.0) <= note.abs_diff(older.0) => {
                [Some(older), Some(newer)]
            }
            Some(newer) => [Some(newer), Some(older)],
        };
        let Some((partner, ((ps, pp), (ns, np)))) = partners
            .into_iter()
            .flatten()
            .find_map(|p| Some((p, self.choose_pair(p, note)?)))
        else {
            return false;
        };
        let fs = self.fs;
        let bowed = [Some(self.string), self.second.map(|(_, s)| s)];
        for s in bowed.into_iter().flatten() {
            if s != ps && s != ns {
                self.contact[s].go(0.0, t.crossing, fs);
            }
        }
        let shift = self.reach(&[pp, np]);
        if ps != partner.1 {
            // The sounding note moves to another string.
            self.finger[ps].go(pp, t.place, fs);
            self.contact[ps].go(1.0, t.crossing, fs);
        }
        if bowed.contains(&Some(ns)) {
            self.finger_to(ns, np, shift, velocity);
        } else {
            self.finger[ns].go(np, t.place, fs);
            let land = if self.phase == Phase::Grip {
                t.grip
            } else {
                t.land
            };
            self.contact[ns].go(1.0, land, fs);
        }
        self.sounding = Some(partner.0);
        self.string = ps;
        self.second = Some((note, ns));
        true
    }

    /// Ends a double stop on the note nearer to `note`, for a legato line to go
    /// on from.
    fn keep_nearest(&mut self, note: u8) {
        if let (Some((newer, _)), Some(older)) = (self.second, self.sounding) {
            if note.abs_diff(newer) <= note.abs_diff(older) {
                self.drop_older();
            } else {
                self.drop_newer();
            }
        }
    }

    /// Ends a double stop on its older note: the newer one's string is left.
    fn drop_newer(&mut self) {
        if let Some((_, string)) = self.second.take() {
            self.contact[string].go(0.0, self.settings.tuning.crossing, self.fs);
        }
    }

    /// Ends a double stop on its newer note, which plays on alone.
    fn drop_older(&mut self) {
        if let Some((note, string)) = self.second.take() {
            self.contact[self.string].go(0.0, self.settings.tuning.crossing, self.fs);
            self.sounding = Some(note);
            self.string = string;
        }
    }

    /// Lifts the bow off the strings it plays.
    fn lift(&mut self, seconds: f32) {
        self.contact[self.string].go(0.0, seconds, self.fs);
        if let Some((_, string)) = self.second {
            self.contact[string].go(0.0, seconds, self.fs);
        }
    }

    /// Whether the bow plays string `i` (the note's, or a double stop's second).
    fn bows(&self, i: usize) -> bool {
        i == self.string || self.second.is_some_and(|(_, s)| s == i)
    }

    /// Moves the finger on a sounding string to `position`, as the landing
    /// note's velocity says. Pressed hard, the note changes at once: a finger
    /// drops or lifts within the hand, or a `shift` of the hand slides
    /// quickly. Pressed softly, the finger slides (portamento), slower the
    /// softer. Only between stopped notes: to or from an open string the
    /// finger is placed.
    fn finger_to(&mut self, string: usize, position: f32, shift: bool, velocity: f32) {
        let t = self.settings.tuning;
        let time = if position >= OPEN && self.finger[string].to >= OPEN {
            let quick = if shift { t.shift } else { t.place };
            let pv = t.portamento_velocity.max(1e-3);
            quick.max(t.portamento * ((pv - velocity) / pv).max(0.0))
        } else {
            t.place
        };
        self.finger[string].go(position, time, self.fs);
    }

    /// Moves the hand so it covers the stopped `positions` (semitones; open
    /// strings need no hand). Returns whether it had to shift: up, the lowest
    /// note comes under the first finger; down, the highest under the last.
    fn reach(&mut self, positions: &[f32]) -> bool {
        let stopped = positions.iter().filter(|&&p| p >= OPEN);
        let lo = stopped.clone().fold(f32::MAX, |a, &b| a.min(b));
        let hi = stopped.fold(f32::MIN, |a, &b| a.max(b));
        let span = self.settings.tuning.hand_span;
        if lo > hi {
            false
        } else if lo < self.hand {
            self.hand = (hi - span).min(lo).max(0.0);
            true
        } else if hi > self.hand + span {
            self.hand = lo;
            true
        } else {
            false
        }
    }

    /// Places the finger and moves the bow to `string`. A crossing fades the
    /// force from the old string to the new one.
    fn move_to_string(&mut self, string: usize, semitones: f32, place: f32) {
        let fs = self.fs;
        let t = self.settings.tuning;
        if string != self.string {
            self.contact[self.string].go(0.0, t.crossing, fs);
            // The finger lands before the bow arrives.
            self.finger[string].go(semitones, t.place, fs);
            if self.contact[string].value() > 0.0 || self.phase == Phase::Sustain {
                self.contact[string].go(1.0, t.crossing, fs);
            }
            self.string = string;
        } else {
            self.finger[string].go(semitones, place, fs);
        }
    }

    /// Position of `note` on string `i`: semitones above the open string.
    fn position(&self, note: u8, i: usize) -> f32 {
        let frequency = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
        12.0 * (frequency / self.instrument.spec().strings[i].frequency).log2()
    }

    fn playable(&self, position: f32) -> bool {
        (-0.01..=self.instrument.spec().reach).contains(&position)
    }

    /// How far a position lies above what the fingering mode prefers.
    fn cost(&self, position: f32) -> f32 {
        let t = &self.settings.tuning;
        let bias = match self.fingering {
            Fingering::NutAndOpen => 0.0,
            Fingering::Mid => t.mid_bias,
            Fingering::Bridge => t.bridge_bias,
        };
        (position - bias).max(0.0)
    }

    /// The string that plays `note` in the lowest position the fingering
    /// allows, and that position. In a legato line, `current` is kept unless
    /// another string plays the note `legato_stick` semitones lower.
    fn choose_string(&self, note: u8, current: Option<usize>) -> (usize, f32) {
        let position = |i: usize| self.position(note, i);
        let playable = |i: usize| self.playable(position(i));
        // Ties (within the bias) go to the lowest string: `min_by` keeps the first.
        let cost = |i: usize| self.cost(position(i));
        let stick = self.settings.tuning.legato_stick;
        let best = (0..4)
            .filter(|&i| playable(i))
            .min_by(|&a, &b| cost(a).total_cmp(&cost(b)));
        let sul = self.sul.filter(|&s| playable(s));
        let string = match (sul, best, current) {
            (Some(s), _, _) => s,
            (None, Some(b), Some(c)) if playable(c) && cost(c) - stick <= cost(b) => c,
            (None, Some(b), _) => b,
            // Below the lowest string: play it open. Above the range: top string.
            (None, None, _) if position(0) < 0.0 => 0,
            (None, None, _) => 3,
        };
        let reach = self.instrument.spec().reach;
        (string, position(string).clamp(0.0, reach))
    }

    /// Strings and positions for a double stop of `partner` (a sounding note
    /// and its string) and `note`, in that order: the lower note on one
    /// string, the higher on the next, stopped notes within the hand's span.
    /// The partner keeps its string if it can. `None` if no two strings can.
    #[allow(clippy::type_complexity)]
    fn choose_pair(&self, partner: (u8, usize), note: u8) -> Option<((usize, f32), (usize, f32))> {
        if note == partner.0 {
            return None;
        }
        let span = self.settings.tuning.hand_span;
        let reach = self.instrument.spec().reach;
        let mut best: Option<((bool, f32), (usize, f32), (usize, f32))> = None;
        for low in 0..3 {
            let (ps, ns) = if partner.0 < note {
                (low, low + 1)
            } else {
                (low + 1, low)
            };
            let (pp, np) = (self.position(partner.0, ps), self.position(note, ns));
            if !self.playable(pp) || !self.playable(np) {
                continue;
            }
            if pp >= OPEN && np >= OPEN && (pp - np).abs() > span {
                continue;
            }
            // Moving a sounding note to another string comes last.
            let rank = (ps != partner.1, self.cost(pp) + self.cost(np));
            if best.is_none_or(|(r, ..)| rank < r) {
                best = Some((rank, (ps, pp.clamp(0.0, reach)), (ns, np.clamp(0.0, reach))));
            }
        }
        best.map(|(_, p, n)| (p, n))
    }

    fn hold(&mut self, note: u8) {
        self.release_held(note);
        if self.held_len == MAX_HELD {
            self.held.copy_within(1.., 0);
            self.held_len -= 1;
        }
        self.held[self.held_len] = note;
        self.held_len += 1;
    }

    fn release_held(&mut self, note: u8) {
        if let Some(i) = self.held[..self.held_len].iter().position(|&n| n == note) {
            self.held.copy_within(i + 1..self.held_len, i);
            self.held_len -= 1;
        }
    }

    /// Control-rate update: humanizing drift and bow wander, vibrato, pitch,
    /// bow position, the force band and the finger's damping.
    fn update_controls(&mut self) {
        let dt = self.control_interval as f32 / self.fs;
        let d = self.dynamics.value;
        let s = self.settings;
        let t = s.tuning;

        for w in &mut self.wander {
            w.tick(dt, t.wander_time, &mut self.rng);
        }
        let [wander_pressure, wander_speed, wander_beta] = self.wander.map(|w| w.value());
        self.speed =
            s.speed.0 * (s.speed.1 / s.speed.0).powf(d) * (1.0 + t.wander_speed * wander_speed);
        // The wander never takes β above the mapping's range (STATUS.md: the
        // flat zone above it).
        let beta = (lerp(s.beta.0, s.beta.1, d) * (1.0 + t.wander_beta * wander_beta))
            .min(s.beta.0.max(s.beta.1));

        // New drift targets every 0.3 s; the smoothers glide between them.
        self.drift_timer -= dt;
        if self.drift_timer <= 0.0 {
            self.drift_timer = 0.3;
            self.rate_drift.target = 0.08 * (2.0 * next_random(&mut self.rng) - 1.0);
            self.depth_drift.target = 0.25 * (2.0 * next_random(&mut self.rng) - 1.0);
        }
        for _ in 0..self.control_interval {
            self.rate_drift.tick();
            self.depth_drift.tick();
        }
        let rate = s.vibrato_rate * (1.0 + self.rate_drift.value);
        self.vibrato_phase =
            (self.vibrato_phase + std::f32::consts::TAU * rate * dt) % std::f32::consts::TAU;
        let onset = ((self.vibrato_age - t.vibrato_delay) / t.vibrato_fade).clamp(0.0, 1.0);
        let depth =
            s.vibrato_depth * self.vibrato_control.value * onset * (1.0 + self.depth_drift.value);

        let bite = match self.phase {
            Phase::Grip => self.stroke_bite,
            Phase::Sustain if self.phase_time < self.stroke_bite_time => {
                // Fades out as a raised cosine.
                let x = self.phase_time / self.stroke_bite_time;
                self.stroke_bite * (0.5 + 0.5 * (std::f32::consts::PI * x).cos())
            }
            _ => 0.0,
        };
        // The pressure control: flautando, normal (the middle), scratch.
        let (flautando, scratch) = s.pressure_range;
        let c = self.pressure.value;
        let normal = if c < 0.5 {
            lerp(flautando, s.pressure, 2.0 * c)
        } else {
            lerp(s.pressure, scratch, 2.0 * c - 1.0)
        };
        let pressure = (normal
            + t.pressure_tilt * (1.0 - 2.0 * d)
            + t.wander_pressure * wander_pressure
            + bite)
            .clamp(flautando.min(0.0), scratch.max(1.0));

        // The finger stays on a bowed string while the note plays, and eases
        // off once it is over or the bow has moved to another string.
        let held = self
            .sounding
            .is_some_and(|n| self.held[..self.held_len].contains(&n));
        let playing = held || !matches!(self.phase, Phase::Idle | Phase::Resting);
        let mute_in = 1.0 - (-dt / t.mute_time.max(1e-4)).exp();
        let mute_out = 1.0 - (-dt / t.place.max(1e-4)).exp();

        let spec = *self.instrument.spec();
        for i in 0..4 {
            let finger = self.finger[i].value();
            let stopped = finger >= OPEN;
            // An open string can't be vibrated.
            let vibrato = if self.bows(i) && stopped {
                depth * self.vibrato_phase.sin()
            } else {
                0.0
            };
            let open = spec.strings[i].frequency;
            let correction = if stopped { self.intonation[i] } else { 0.0 };
            if i == self.string {
                self.intended = open * 2f32.powf((finger + vibrato) / 12.0);
            }
            let frequency = open * 2f32.powf((finger + vibrato + correction) / 12.0);

            let easing = stopped && !(playing && self.bows(i));
            let m = &mut self.mute[i];
            *m += if easing {
                mute_in * (1.0 - *m)
            } else {
                -mute_out * *m
            };
            let loss = if stopped {
                t.finger_loss + t.mute_loss * *m
            } else {
                0.0
            };

            // The bow keeps its distance from the bridge on a short string.
            let length = spec.strings[i].length * 2f32.powf(-finger.max(0.0) / 12.0);
            let z = spec.strings[i].impedance();
            let beta = beta.max(s.bow_distance * z / length).min(0.5);

            let string = self.instrument.string_mut(i);
            if frequency != string.frequency() {
                string.set_frequency(frequency);
            }
            if (beta - string.beta()).abs() > 1e-4 {
                string.set_bow_position(beta);
            }
            if (loss - self.termination_loss[i]).abs() > 1e-5 {
                self.termination_loss[i] = loss;
                string.set_termination_loss(loss);
            }
            self.unit_force[i] = spec.force_limits[i].force(z, 1.0, beta, pressure);
        }
    }
}

/// Uniform in [0, 1) (xorshift32).
fn next_random(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
