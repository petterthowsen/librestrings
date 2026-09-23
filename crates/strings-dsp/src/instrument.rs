//! An instrument: four strings, all running every sample, into one body.
//!
//! The strings share nothing but the body, which sums their bridge forces. The
//! bow is applied per string, so a string crossing can bow two strings at
//! once, and a string the bow has left rings on.

use crate::body::{Body, BodySpec};
use crate::bow::FrictionParams;
use crate::string::{BowHair, BowInput, BowedString, StringFrame, StringSpec};

/// The Helmholtz band of bow force, calibrated per instrument (PLAN.md 4.2).
///
/// Both edges scale with `Z·v_b` (as in Schelleng's formulas) and follow a
/// power of β: `F = c·Z·|v_b|·β^α`. The upper edge is close to Schelleng's
/// F_max (c = 2/(μ_s − μ_d), α = −1); the lower edge is fitted to simulation.
#[derive(Clone, Copy, Debug)]
pub struct ForceLimits {
    pub lower: f32,
    pub lower_exponent: f32,
    pub upper: f32,
    pub upper_exponent: f32,
}

impl ForceLimits {
    /// `(F_low, F_high)` in N for a string of impedance `z` (kg/s).
    pub fn band(&self, z: f32, speed: f32, beta: f32) -> (f32, f32) {
        let zv = z * speed.abs();
        (
            self.lower * zv * beta.powf(self.lower_exponent),
            self.upper * zv * beta.powf(self.upper_exponent),
        )
    }

    /// Bow force at `position` in the band: 0 is the lower edge, 1 the upper,
    /// interpolated in log space. Values outside [0, 1] extrapolate.
    pub fn force(&self, z: f32, speed: f32, beta: f32, position: f32) -> f32 {
        let (lo, hi) = self.band(z, speed, beta);
        lo * (hi / lo).powf(position)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InstrumentSpec {
    pub name: &'static str,
    /// Open strings, lowest first.
    pub strings: [StringSpec; 4],
    pub friction: FrictionParams,
    pub hair: Option<BowHair>,
    pub body: BodySpec,
    /// Helmholtz band per string, lowest first (`strings-render calibrate`).
    pub force_limits: [ForceLimits; 4],
    /// How far above each open string it is played (semitones).
    pub reach: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct InstrumentFrame {
    /// Body output (arbitrary units, roughly radiated pressure).
    pub output: f32,
    /// Sum of the strings' bridge forces (N).
    pub bridge_force: f32,
}

pub struct Instrument {
    spec: InstrumentSpec,
    strings: [BowedString; 4],
    body: Body,
}

impl Instrument {
    /// Slow: fits each string's loss and dispersion per semitone (about 7 ms
    /// per cello string). Don't call it on the audio thread.
    pub fn new(spec: &InstrumentSpec, sample_rate: f32) -> Self {
        let strings = std::array::from_fn(|i| {
            let s = &spec.strings[i];
            let mut string = BowedString::new(s, spec.friction, sample_rate, s.frequency);
            string.set_bow_hair(spec.hair);
            string
        });
        Self {
            spec: *spec,
            strings,
            body: Body::new(&spec.body, sample_rate),
        }
    }

    pub fn spec(&self) -> &InstrumentSpec {
        &self.spec
    }

    pub fn string(&self, i: usize) -> &BowedString {
        &self.strings[i]
    }

    pub fn string_mut(&mut self, i: usize) -> &mut BowedString {
        &mut self.strings[i]
    }

    pub fn reset(&mut self) {
        for s in &mut self.strings {
            s.reset();
        }
        self.body.reset();
    }

    /// Advances one sample with one bow input per string. Also returns each
    /// string's frame through `frames`.
    pub fn process(
        &mut self,
        bows: &[BowInput; 4],
        frames: &mut [StringFrame; 4],
    ) -> InstrumentFrame {
        let mut bridge_force = 0.0;
        for ((s, bow), frame) in self.strings.iter_mut().zip(bows).zip(frames.iter_mut()) {
            *frame = s.process(*bow, 0.0);
            bridge_force += frame.bridge_force;
        }
        InstrumentFrame {
            output: self.body.process(bridge_force),
            bridge_force,
        }
    }
}
