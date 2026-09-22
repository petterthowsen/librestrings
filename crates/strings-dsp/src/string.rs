//! A single bowed string as a digital waveguide.
//!
//! Velocity waves travel in three delay lines. The bow splits the string into a
//! bridge-side segment (two one-way lines, with the lossy bridge reflection
//! between them) and a nut-side segment (one round-trip line, reflecting at the
//! nut or stopping finger):
//!
//! ```text
//! bridge ◄─ to_bridge ─── (BOW) ─── nut (round trip) ─► finger/nut
//!        ─ from_bridge ─►
//! ```
//!
//! The per-period loss `g` is split between the two reflections in proportion
//! to segment length, so a segment trapped by a sticking bow still decays. The
//! lowpass (frequency-dependent loss) sits at the bridge; delay lengths are
//! shortened by its phase delay at `f0` so the loop stays in tune.

use crate::bow::{BowJunction, ContactState, FrictionParams};
use crate::delay::DelayLine;
use crate::filters::OnePoleLowpass;

#[derive(Clone, Copy, Debug)]
pub struct StringSpec {
    pub name: &'static str,
    /// Open-string fundamental (Hz).
    pub frequency: f32,
    /// Vibrating length of the open string (m).
    pub length: f32,
    /// Tension (N).
    pub tension: f32,
    /// Decay time to -60 dB of the fundamental on the open string (s).
    pub t60: f32,
    /// Pole of the one-pole lowpass in the bridge reflection, as it would be at
    /// 48 kHz (converted for other rates). Higher is darker.
    pub loss_lowpass: f32,
}

impl StringSpec {
    /// Transverse wave speed (m/s).
    pub fn wave_speed(&self) -> f32 {
        2.0 * self.length * self.frequency
    }

    /// Characteristic impedance `Z = sqrt(T·μ) = T / c` (kg/s).
    pub fn impedance(&self) -> f32 {
        self.tension / self.wave_speed()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BowInput {
    /// Bow velocity (m/s). The sign sets the bow direction.
    pub velocity: f32,
    /// Normal force pressing the bow onto the string (N). Zero means lifted.
    pub force: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct StringFrame {
    /// Transverse force the string exerts on the bridge (N): the audio output.
    pub bridge_force: f32,
    /// String velocity at the bow point (m/s).
    pub bow_point_velocity: f32,
    /// Friction force the bow applies (N).
    pub friction_force: f32,
    pub state: ContactState,
}

pub struct BowedString {
    sample_rate: f32,
    impedance: f32,
    t60: f32,

    nut: DelayLine,
    to_bridge: DelayLine,
    from_bridge: DelayLine,
    bridge_filter: OnePoleLowpass,
    loop_gain: f32,
    bridge_gain: f32,
    nut_gain: f32,
    bow: BowJunction,

    frequency: f32,
    beta: f32,
    nut_delay: f32,
    bridge_delay: f32,
}

impl BowedString {
    /// `lowest_frequency` bounds the delay memory; the string can't be tuned below it.
    pub fn new(
        spec: &StringSpec,
        friction: FrictionParams,
        sample_rate: f32,
        lowest_frequency: f32,
    ) -> Self {
        let max_loop = (sample_rate / lowest_frequency).ceil() as usize + 4;
        let mut s = Self {
            sample_rate,
            impedance: spec.impedance(),
            t60: spec.t60,
            nut: DelayLine::new(max_loop),
            to_bridge: DelayLine::new(max_loop / 2),
            from_bridge: DelayLine::new(max_loop / 2),
            bridge_filter: OnePoleLowpass::new(spec.loss_lowpass.powf(48_000.0 / sample_rate)),
            loop_gain: 1.0,
            bridge_gain: 1.0,
            nut_gain: 1.0,
            bow: BowJunction::new(friction),
            frequency: spec.frequency,
            beta: 0.1,
            nut_delay: 0.0,
            bridge_delay: 0.0,
        };
        s.update_delays();
        s
    }

    pub fn impedance(&self) -> f32 {
        self.impedance
    }

    pub fn frequency(&self) -> f32 {
        self.frequency
    }

    /// Bow position as a fraction of the vibrating length, measured from the bridge.
    pub fn beta(&self) -> f32 {
        self.beta
    }

    /// Total DC gain of one trip round the loop (one period).
    pub fn loop_gain(&self) -> f32 {
        self.loop_gain
    }

    /// Sets the sounding pitch, i.e. where the finger stops the string.
    pub fn set_frequency(&mut self, frequency: f32) {
        self.frequency = frequency;
        self.update_delays();
    }

    /// Sets the bow position as a fraction of the vibrating length from the bridge.
    /// Very small values are limited by the shortest usable delay.
    pub fn set_bow_position(&mut self, beta: f32) {
        self.beta = beta.clamp(0.0, 0.5);
        self.update_delays();
    }

    pub fn reset(&mut self) {
        self.nut.clear();
        self.to_bridge.clear();
        self.from_bridge.clear();
        self.bridge_filter.reset();
        self.bow.reset();
    }

    /// Advances one sample. `external_force` (N) is added at the bow point, for
    /// plucks and test impulses.
    pub fn process(&mut self, bow: BowInput, external_force: f32) -> StringFrame {
        let z = self.impedance;
        let from_nut = -self.nut_gain * self.nut.read(self.nut_delay);
        let arriving_at_bridge = self.to_bridge.read(self.bridge_delay);
        let reflected = -self.bridge_gain * self.bridge_filter.process(arriving_at_bridge);
        let from_bridge = self.from_bridge.read(self.bridge_delay);

        let v_h = from_nut + from_bridge + external_force / (2.0 * z);
        let j = self.bow.solve(v_h, bow.velocity, bow.force, z);
        let half = (j.force + external_force) / (2.0 * z);

        self.to_bridge.push(from_nut + half);
        self.nut.push(from_bridge + half);
        self.from_bridge.push(reflected);

        let bridge_force = z * (arriving_at_bridge - reflected);
        if !bridge_force.is_finite() {
            debug_assert!(false, "string blew up");
            self.reset();
        }
        StringFrame {
            bridge_force,
            bow_point_velocity: j.string_velocity,
            friction_force: j.force,
            state: j.state,
        }
    }

    fn update_delays(&mut self) {
        let omega = std::f32::consts::TAU * self.frequency / self.sample_rate;
        let filter_delay = OnePoleLowpass::phase_delay(self.bridge_filter.a, omega);
        let total = self.sample_rate / self.frequency - filter_delay;
        let max_bridge = self.to_bridge.max_delay();
        self.bridge_delay = (0.5 * self.beta * total).clamp(DelayLine::MIN_DELAY, max_bridge);
        self.nut_delay = total - 2.0 * self.bridge_delay;
        self.loop_gain = 10f32.powf(-3.0 / (self.t60 * self.frequency));
        let bridge_share = 2.0 * self.bridge_delay / total;
        self.bridge_gain = self.loop_gain.powf(bridge_share);
        self.nut_gain = self.loop_gain.powf(1.0 - bridge_share);
    }
}
