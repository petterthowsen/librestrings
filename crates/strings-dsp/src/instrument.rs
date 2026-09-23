//! An instrument: four strings, all running every sample, into one body.
//!
//! The strings share nothing but the body, which sums their bridge forces. The
//! bow is applied per string, so a string crossing can bow two strings at
//! once, and a string the bow has left rings on.

use crate::body::{Body, BodySpec, BodyTuning};
use crate::bow::FrictionParams;
use crate::filters::HalfbandDecimator;
use crate::string::{BowHair, BowInput, BowedString, StringDesign, StringFrame, StringSpec};

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
    sample_rate: f32,
    /// How many string samples per output sample (1 or 2).
    oversampling: usize,
    decimator: HalfbandDecimator,
}

impl Instrument {
    /// Slow: fits each string's loss and dispersion per semitone (about 7 ms
    /// per cello string). Don't call it on the audio thread.
    ///
    /// With `oversampling` 2 the strings run at twice `sample_rate` and their
    /// bridge force is decimated before the body, which stays at
    /// `sample_rate`. At 48 kHz a high note's slip otherwise snaps to whole
    /// samples, and its period with it (C5 is 91.7 samples long).
    pub fn new(spec: &InstrumentSpec, sample_rate: f32, oversampling: usize) -> Self {
        assert!(matches!(oversampling, 1 | 2), "oversampling must be 1 or 2");
        let string_rate = sample_rate * oversampling as f32;
        let strings = std::array::from_fn(|i| {
            let s = &spec.strings[i];
            let mut string = BowedString::new(s, spec.friction, string_rate, s.frequency);
            string.set_bow_hair(spec.hair);
            string
        });
        Self {
            spec: *spec,
            strings,
            body: Body::new(&spec.body, sample_rate),
            sample_rate,
            oversampling,
            decimator: HalfbandDecimator::new(),
        }
    }

    /// The rate the strings run at (Hz): the sample rate times the
    /// oversampling. String designs must be fitted at this rate.
    pub fn string_sample_rate(&self) -> f32 {
        self.sample_rate * self.oversampling as f32
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

    /// Fits new loss and stiffness designs for `strings` (the instrument's
    /// strings with other parameters) at `string_rate`, the instrument's
    /// [`Self::string_sample_rate`]. Slow (about 30 ms for a cello at 48 kHz)
    /// and allocating: call it off the audio thread, then
    /// [`Self::apply_strings`].
    pub fn design_strings(strings: &[StringSpec; 4], string_rate: f32) -> [StringDesign; 4] {
        std::array::from_fn(|i| {
            let s = &strings[i];
            BowedString::design(s, string_rate, s.frequency)
        })
    }

    /// Takes new string parameters while playing (see
    /// [`BowedString::apply_design`]). Real-time safe; `designs` comes back
    /// holding the old designs. Returns `false` if any string didn't take its
    /// spec (a different pitch or kind of loss); those keep their old one.
    pub fn apply_strings(
        &mut self,
        strings: &[StringSpec; 4],
        designs: &mut [StringDesign; 4],
    ) -> bool {
        let mut all = true;
        for (i, (spec, design)) in strings.iter().zip(designs.iter_mut()).enumerate() {
            if self.strings[i].apply_design(spec, design) {
                self.spec.strings[i] = *spec;
            } else {
                all = false;
            }
        }
        all
    }

    pub fn set_friction(&mut self, friction: FrictionParams) {
        self.spec.friction = friction;
        for s in &mut self.strings {
            s.set_friction(friction);
        }
    }

    pub fn set_hair(&mut self, hair: Option<BowHair>) {
        self.spec.hair = hair;
        for s in &mut self.strings {
            s.set_bow_hair(hair);
        }
    }

    /// Retunes the body while playing (real-time safe). The spec keeps its
    /// original body; the tuning replaces it only in sound.
    pub fn set_body(&mut self, body: &BodyTuning) {
        self.body.set(body);
    }

    pub fn body(&self) -> &Body {
        &self.body
    }

    pub fn reset(&mut self) {
        for s in &mut self.strings {
            s.reset();
        }
        self.decimator.reset();
        self.body.reset();
    }

    /// Advances one sample with one bow input per string, held over the
    /// oversampled steps. Also returns each string's frame through `frames`
    /// (from the last step when oversampling).
    pub fn process(
        &mut self,
        bows: &[BowInput; 4],
        frames: &mut [StringFrame; 4],
    ) -> InstrumentFrame {
        let step = |strings: &mut [BowedString; 4], frames: &mut [StringFrame; 4]| {
            let mut bridge_force = 0.0;
            for ((s, bow), frame) in strings.iter_mut().zip(bows).zip(frames.iter_mut()) {
                *frame = s.process(*bow, 0.0);
                bridge_force += frame.bridge_force;
            }
            bridge_force
        };
        let bridge_force = if self.oversampling == 2 {
            let first = step(&mut self.strings, frames);
            let second = step(&mut self.strings, frames);
            self.decimator.process([first, second])
        } else {
            step(&mut self.strings, frames)
        };
        InstrumentFrame {
            output: self.body.process(bridge_force),
            bridge_force,
        }
    }
}
