//! The performer: turns notes and controllers into bow and finger motion
//! (PLAN.md section 4).
//!
//! It plays one [`Instrument`] monophonically. Per sample it drives the bow
//! velocity and, per string, how firmly the bow is on it (the contact, 0–1).
//! Bow force follows from the calibrated Helmholtz band ([`ForceLimits`]):
//! `F = contact · F_band(p, |v_b|)`, where `p` is the pressure (a position in the
//! band) and the band scales with bow speed. Because the force follows the bow
//! speed down to a floor, a decelerating bow stays inside the band, so the
//! Helmholtz amplitude follows the bow down and a stopped note ends quickly.
//!
//! Stopped notes are intonated by ear, as a player does. The bowed pitch of the
//! model drifts from the string's tuning with bow force and position (the
//! flattening effect, and stiffness, which sharpens the Helmholtz pitch of
//! high notes). The performer measures the period between slips on the bowed
//! string, which is exactly one period in Helmholtz motion, and slowly moves
//! the finger to correct it. Open strings can't be corrected.
//!
//! Real-time safe: no allocation after construction. String parameters (pitch
//! and bow position) are updated at [`CONTROL_RATE`], the bow every sample.
//!
//! [`ForceLimits`]: crate::instrument::ForceLimits

use crate::instrument::{Instrument, InstrumentSpec};
use crate::string::{BowInput, StringFrame};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Articulation {
    /// Détaché strokes; overlapping notes play legato.
    #[default]
    Sustain,
    /// On the string: grip, a fast stroke, then the bow stops on the string.
    Staccato,
    /// Thrown: the bow touches the string briefly and leaves it ringing.
    Spiccato,
}

#[derive(Clone, Copy, Debug)]
pub struct PerformerSettings {
    /// Bow speed (m/s) at dynamics 0 and 1, interpolated in log space.
    pub speed: (f32, f32),
    /// Bow position (fraction of the vibrating length from the bridge) at
    /// dynamics 0 and 1: players move toward the bridge as they play louder.
    pub beta: (f32, f32),
    /// Position in the Helmholtz band: 0 is its lower edge (flautando), 1 its
    /// upper edge (pressed).
    pub pressure: f32,
    /// Vibrato rate (Hz).
    pub vibrato_rate: f32,
    /// Vibrato depth (semitones, either way) at full vibrato control.
    pub vibrato_depth: f32,
    /// Notes up to this many semitones above a lower string's open pitch are
    /// played on that lower string ("sul G" color). 0 always takes the lowest
    /// position.
    pub string_bias: f32,
    /// Output gain applied after the body.
    pub output_gain: f32,
    /// Seed for the humanizing drift.
    pub seed: u32,
}

impl Default for PerformerSettings {
    fn default() -> Self {
        Self {
            speed: (0.04, 0.5),
            // Above β ≈ 0.12 the model's cello strings play up to 45 cents flat
            // (STATUS.md), so the mapping stays below.
            beta: (0.115, 0.07),
            pressure: 0.65,
            vibrato_rate: 5.5,
            vibrato_depth: 0.35,
            string_bias: 0.0,
            output_gain: 0.1,
            seed: 0x0b0e_5eed,
        }
    }
}

/// Rate (Hz) at which pitch, bow position and the force band are updated.
pub const CONTROL_RATE: f32 = 3000.0;

// Timing (seconds).
/// Bow landing on a string.
const LAND: f32 = 0.012;
/// Force moving from one string to another in a crossing.
const CROSSING: f32 = 0.03;
/// Bow acceleration at the start of a détaché stroke, at MIDI velocity 0 and 1.
const ATTACK: (f32, f32) = (0.12, 0.035);
/// At low dynamics attacks are slower, by up to this factor minus one at
/// dynamics 0. The pressure (band position) also tilts with dynamics, by
/// [`PRESSURE_TILT`] above the setting at dynamics 0 and as much below it at 1:
/// the model's quiet attacks start cleanly only high in the band, and loud
/// notes flatten less low in it.
const PP_ATTACK: f32 = 0.8;
const PRESSURE_TILT: f32 = 0.15;
/// End of a sustained note: the bow slows and lifts off.
const RELEASE: f32 = 0.15;
/// Legato finger glide at velocity 0 and 1 (low velocity slides, like portamento).
const GLIDE: (f32, f32) = (0.14, 0.012);
/// Finger placement when the bow change or crossing hides it.
const PLACE: f32 = 0.006;
/// Staccato: grip before the stroke, acceleration, stroke length at velocity 0
/// and 1 (shorter if the note ends first), deceleration to the stop, and how
/// long the bow rests on the string before lifting.
const GRIP: f32 = 0.015;
const STACCATO_ACCEL: (f32, f32) = (0.02, 0.008);
const STACCATO_LENGTH: (f32, f32) = (0.16, 0.1);
const STACCATO_MIN: f32 = 0.04;
const STACCATO_STOP: f32 = 0.04;
const REST: f32 = 0.4;
/// Staccato bow speed relative to the dynamics speed, and the extra pressure of
/// the bite, fading over its duration.
const STACCATO_SPEED: f32 = 1.3;
const BITE: f32 = 0.25;
const BITE_TIME: f32 = 0.03;
/// Spiccato contact length at velocity 0 and 1.
const SPICCATO: (f32, f32) = (0.065, 0.03);
/// Vibrato onset: delay after the stroke starts, then fade-in.
const VIBRATO_DELAY: f32 = 0.12;
const VIBRATO_FADE: f32 = 0.4;
/// Controller smoothing.
const CONTROL_SMOOTHING: f32 = 0.03;

/// While the bow slows (a release or a stop), the force band follows the bow
/// speed down to this fraction of the dynamics' speed, so the Helmholtz
/// amplitude follows the bow and the stopped bow still holds the string. While
/// it gets going, the force is that of the full speed from the start: it
/// leads the speed, as in a player's attack.
const SPEED_FLOOR: f32 = 0.15;
/// In a legato line, stay on the current string unless another one plays the
/// note this many semitones lower in position.
const LEGATO_STICK: f32 = 5.0;
/// Intonation by ear: time constant (s), the largest pitch error still taken
/// for a Helmholtz period (semitones; multiple slips give far larger ones), and
/// the largest correction (semitones).
const EAR_TIME: f32 = 0.15;
const EAR_WINDOW: f32 = 1.0;
const EAR_RANGE: f32 = 1.0;
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Bow off the string.
    Idle,
    /// A détaché or legato stroke, note held.
    Sustain,
    /// Note released: the bow slows and lifts.
    Release,
    StaccatoGrip,
    StaccatoStroke,
    StaccatoStop,
    /// Bow resting on the string after a staccato stop.
    Resting,
    Spiccato,
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
    expression: Smooth,
    vibrato_control: Smooth,
    articulation: Articulation,

    held: [u8; MAX_HELD],
    held_len: usize,
    sounding: Option<u8>,
    string: usize,

    phase: Phase,
    /// Seconds since the phase began.
    phase_time: f32,
    /// Length of the current staccato stroke or spiccato contact (s).
    phase_length: f32,
    /// MIDI velocity of the current stroke, 0–1.
    stroke_velocity: f32,
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
            instrument: Instrument::new(spec, fs),
            settings,
            fs,
            dynamics: Smooth::new(0.5, CONTROL_SMOOTHING, fs),
            expression: Smooth::new(1.0, CONTROL_SMOOTHING, fs),
            vibrato_control: Smooth::new(0.0, CONTROL_SMOOTHING, fs),
            articulation: Articulation::Sustain,
            held: [0; MAX_HELD],
            held_len: 0,
            sounding: None,
            string: 0,
            phase: Phase::Idle,
            phase_time: 0.0,
            phase_length: 0.0,
            stroke_velocity: 0.0,
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

    /// Every string's frame from the last sample.
    pub fn string_frames(&self) -> &[StringFrame; 4] {
        &self.frames
    }

    pub fn settings(&self) -> &PerformerSettings {
        &self.settings
    }

    /// The note being played, until the bow has left the string.
    pub fn note(&self) -> Option<u8> {
        if self.phase == Phase::Idle {
            None
        } else {
            self.sounding
        }
    }

    /// The string the bow plays (or last played), lowest is 0.
    pub fn bowed_string(&self) -> usize {
        self.string
    }

    /// How firmly the bow is on string `i`, 0 (off) to 1.
    pub fn contact(&self, i: usize) -> f32 {
        self.contact[i].value()
    }

    /// The articulation for the next note.
    pub fn articulation(&self) -> Articulation {
        self.articulation
    }

    /// Dynamics, 0–1 (CC1): bow speed, bow position and with them loudness.
    pub fn set_dynamics(&mut self, value: f32) {
        self.dynamics.target = value.clamp(0.0, 1.0);
    }

    /// Expression, 0–1 (CC11): output gain after the body.
    pub fn set_expression(&mut self, value: f32) {
        self.expression.target = value.clamp(0.0, 1.0);
    }

    /// Vibrato depth, 0–1 (CC21).
    pub fn set_vibrato(&mut self, value: f32) {
        self.vibrato_control.target = value.clamp(0.0, 1.0);
    }

    /// Takes effect from the next note.
    pub fn set_articulation(&mut self, articulation: Articulation) {
        self.articulation = articulation;
    }

    /// Position in the Helmholtz band, 0–1.
    pub fn set_pressure(&mut self, value: f32) {
        self.settings.pressure = value.clamp(0.0, 1.0);
    }

    /// Silences everything at once.
    pub fn reset(&mut self) {
        self.instrument.reset();
        self.held_len = 0;
        self.sounding = None;
        self.phase = Phase::Idle;
        self.velocity.set(0.0);
        self.contact = [Ramp::at(0.0); 4];
        self.intonation = [0.0; 4];
    }

    /// Releases every held note as if the last one were let go (MIDI "all
    /// notes off"): the bow finishes its stroke instead of stopping dead.
    pub fn release_all(&mut self) {
        self.held_len = 0;
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
        let legato = self.articulation == Articulation::Sustain
            && self.phase == Phase::Sustain
            && self.sounding.is_some();
        if legato {
            self.legato_to(note, velocity);
            return;
        }
        self.sounding = Some(note);
        self.stroke_velocity = velocity;
        let (string, semitones) = self.choose_string(note, None);
        self.move_to_string(string, semitones, PLACE);
        self.direction = -self.direction;
        self.vibrato_age = 0.0;
        self.phase_time = 0.0;
        let fs = self.fs;
        match self.articulation {
            Articulation::Sustain => {
                self.phase = Phase::Sustain;
                let quiet = 1.0 - self.dynamics.target;
                let attack = lerp(ATTACK.0, ATTACK.1, velocity) * (1.0 + PP_ATTACK * quiet);
                // From a bow still moving, this is a bow change through zero speed.
                self.velocity.attack(self.direction, attack, fs);
                self.contact[string].go(1.0, LAND, fs);
            }
            Articulation::Staccato => {
                self.phase = Phase::StaccatoGrip;
                self.velocity.go(0.0, PLACE, fs);
                self.contact[string].go(1.0, GRIP, fs);
                self.phase_length = lerp(STACCATO_LENGTH.0, STACCATO_LENGTH.1, velocity);
            }
            Articulation::Spiccato => {
                self.phase = Phase::Spiccato;
                // The bow is already moving when it lands.
                self.velocity.go(self.direction, PLACE, fs);
                self.phase_length = lerp(SPICCATO.0, SPICCATO.1, velocity);
            }
        }
    }

    pub fn note_off(&mut self, note: u8) {
        self.release_held(note);
        if self.sounding != Some(note) {
            return;
        }
        let fs = self.fs;
        match self.phase {
            Phase::Sustain => {
                if let Some(&previous) = self.held[..self.held_len].last() {
                    // Back to a note still held, legato.
                    self.legato_to(previous, self.stroke_velocity);
                } else {
                    self.phase = Phase::Release;
                    self.phase_time = 0.0;
                    self.velocity.go(0.4 * self.direction, RELEASE, fs);
                    self.contact[self.string].go(0.0, RELEASE, fs);
                }
            }
            Phase::StaccatoStroke if self.phase_time >= STACCATO_MIN => self.staccato_stop(),
            Phase::StaccatoGrip | Phase::StaccatoStroke => {
                self.phase_length = self.phase_length.min(STACCATO_MIN)
            }
            _ => {}
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
        let expression = self.expression.tick();
        self.vibrato_control.tick();
        self.advance_phase();

        self.velocity.tick();
        let v = self.velocity.value() * self.speed;
        let floor = match self.phase {
            Phase::Sustain | Phase::StaccatoGrip | Phase::StaccatoStroke | Phase::Spiccato => {
                self.velocity.to.abs().max(SPEED_FLOOR)
            }
            _ => SPEED_FLOOR,
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
            output: out.output * expression * self.settings.output_gain,
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
        self.phase_time += dt;
        self.vibrato_age += dt;
        match self.phase {
            Phase::Release if self.phase_time >= RELEASE => {
                self.phase = Phase::Idle;
                self.velocity.go(0.0, RELEASE, fs);
            }
            Phase::StaccatoGrip if self.phase_time >= GRIP => {
                self.phase = Phase::StaccatoStroke;
                self.phase_time = 0.0;
                let accel = lerp(STACCATO_ACCEL.0, STACCATO_ACCEL.1, self.stroke_velocity);
                self.velocity
                    .attack(STACCATO_SPEED * self.direction, accel, fs);
            }
            Phase::StaccatoStroke if self.phase_time >= self.phase_length => self.staccato_stop(),
            Phase::StaccatoStop if self.phase_time >= STACCATO_STOP => {
                self.phase = Phase::Resting;
                self.phase_time = 0.0;
            }
            Phase::Resting if self.phase_time >= REST => {
                self.phase = Phase::Idle;
                self.contact[self.string].go(0.0, 0.1, fs);
            }
            Phase::Spiccato => {
                // A bell-shaped touch: sin² over the contact length.
                let t = (self.phase_time / self.phase_length).min(1.0);
                let s = (std::f32::consts::PI * t).sin();
                // sin(π) isn't exactly zero in f32: end on an exact lift.
                self.contact[self.string].set(if t < 1.0 { s * s } else { 0.0 });
                if t >= 1.0 {
                    self.phase = Phase::Idle;
                    self.velocity.go(0.0, RELEASE, fs);
                }
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
        if listening && error.abs() < EAR_WINDOW {
            let gain = (period / (self.fs * EAR_TIME)).min(1.0);
            self.intonation[s] = (self.intonation[s] + gain * error).clamp(-EAR_RANGE, EAR_RANGE);
        }
    }

    fn staccato_stop(&mut self) {
        self.phase = Phase::StaccatoStop;
        self.phase_time = 0.0;
        self.velocity.go(0.0, STACCATO_STOP, self.fs);
    }

    fn legato_to(&mut self, note: u8, velocity: f32) {
        self.sounding = Some(note);
        let (string, semitones) = self.choose_string(note, Some(self.string));
        let glide = lerp(GLIDE.0, GLIDE.1, velocity);
        if string == self.string {
            self.finger[string].go(semitones, glide, self.fs);
        } else {
            self.move_to_string(string, semitones, PLACE);
        }
        // Vibrato carries on through a legato change, at no less than half depth.
        self.vibrato_age = self.vibrato_age.max(VIBRATO_DELAY + 0.5 * VIBRATO_FADE);
    }

    /// Places the finger and moves the bow to `string`. A crossing fades the
    /// force from the old string to the new one.
    fn move_to_string(&mut self, string: usize, semitones: f32, place: f32) {
        let fs = self.fs;
        if string != self.string {
            self.contact[self.string].go(0.0, CROSSING, fs);
            // The finger lands before the bow arrives.
            self.finger[string].go(semitones, PLACE, fs);
            if self.contact[string].value() > 0.0 || self.phase == Phase::Sustain {
                self.contact[string].go(1.0, CROSSING, fs);
            }
            self.string = string;
        } else {
            self.finger[string].go(semitones, place, fs);
        }
    }

    /// The string that plays `note` in the lowest position, and that position
    /// (semitones above the open string). In a legato line, `current` is kept
    /// unless another string plays the note [`LEGATO_STICK`] semitones lower.
    fn choose_string(&self, note: u8, current: Option<usize>) -> (usize, f32) {
        let spec = self.instrument.spec();
        let frequency = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
        let semitones = |i: usize| 12.0 * (frequency / spec.strings[i].frequency).log2();
        let playable = |i: usize| (-0.01..=spec.reach).contains(&semitones(i));
        // Ties (within the bias) go to the lowest string: `min_by` keeps the first.
        let cost = |i: usize| (semitones(i) - self.settings.string_bias).max(0.0);
        let best = (0..4)
            .filter(|&i| playable(i))
            .min_by(|&a, &b| cost(a).total_cmp(&cost(b)));
        let string = match (best, current) {
            (Some(b), Some(c)) if playable(c) && cost(c) - LEGATO_STICK <= cost(b) => c,
            (Some(b), _) => b,
            // Below the lowest string: play it open. Above the range: top string.
            (None, _) if semitones(0) < 0.0 => 0,
            (None, _) => 3,
        };
        (string, semitones(string).clamp(0.0, spec.reach))
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

    /// Control-rate update: humanizing drift, vibrato, pitch, bow position and
    /// the force band.
    fn update_controls(&mut self) {
        let dt = self.control_interval as f32 / self.fs;
        let d = self.dynamics.value;
        let s = self.settings;
        self.speed = s.speed.0 * (s.speed.1 / s.speed.0).powf(d);
        let beta = lerp(s.beta.0, s.beta.1, d);

        // New drift targets every 0.3 s; the smoothers glide between them.
        self.drift_timer -= dt;
        if self.drift_timer <= 0.0 {
            self.drift_timer = 0.3;
            self.rate_drift.target = 0.08 * (2.0 * self.random() - 1.0);
            self.depth_drift.target = 0.25 * (2.0 * self.random() - 1.0);
        }
        for _ in 0..self.control_interval {
            self.rate_drift.tick();
            self.depth_drift.tick();
        }
        let rate = s.vibrato_rate * (1.0 + self.rate_drift.value);
        self.vibrato_phase =
            (self.vibrato_phase + std::f32::consts::TAU * rate * dt) % std::f32::consts::TAU;
        let onset = ((self.vibrato_age - VIBRATO_DELAY) / VIBRATO_FADE).clamp(0.0, 1.0);
        let depth =
            s.vibrato_depth * self.vibrato_control.value * onset * (1.0 + self.depth_drift.value);

        let bite = match self.phase {
            Phase::StaccatoGrip => BITE,
            Phase::StaccatoStroke => BITE * (1.0 - self.phase_time / BITE_TIME).max(0.0),
            _ => 0.0,
        };
        let pressure = (s.pressure + PRESSURE_TILT * (1.0 - 2.0 * d) + bite).clamp(0.0, 1.0);
        let spec = *self.instrument.spec();
        for i in 0..4 {
            let finger = self.finger[i].value();
            // An open string can't be vibrated.
            let vibrato = if i == self.string && finger > 0.3 {
                depth * self.vibrato_phase.sin()
            } else {
                0.0
            };
            let open = spec.strings[i].frequency;
            let correction = if finger > 0.3 {
                self.intonation[i]
            } else {
                0.0
            };
            if i == self.string {
                self.intended = open * 2f32.powf((finger + vibrato) / 12.0);
            }
            let frequency = open * 2f32.powf((finger + vibrato + correction) / 12.0);
            let string = self.instrument.string_mut(i);
            if frequency != string.frequency() {
                string.set_frequency(frequency);
            }
            if (beta - string.beta()).abs() > 1e-4 {
                string.set_bow_position(beta);
            }
            let z = spec.strings[i].impedance();
            self.unit_force[i] = spec.force_limits[i].force(z, 1.0, beta, pressure);
        }
    }

    /// Uniform in [0, 1) (xorshift32).
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
