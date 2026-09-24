//! An instrument: four strings into one body.
//!
//! The strings share nothing but the body, which sums their bridge forces. The
//! bow is applied per string, so a string crossing can bow two strings at
//! once, and a string the bow has left rings on. A string that has rung out
//! with the bow off it is skipped until the bow comes back: a melody sounds
//! one or two strings at a time, and a decaying string would otherwise run
//! into denormals.

use crate::body::{Body, BodySpec, BodyTuning};
use crate::bow::{BowNoise, FrictionParams};
use crate::filters::HalfbandDecimator;
use crate::stage::Placement;
use crate::string::{
    BowHair, BowInput, BowedString, StringDesign, StringFrame, StringSpec, TorsionSpec,
};

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

/// A low extension on the lowest string, as on orchestral basses: the string
/// runs on past the nut, and gates stop it `semitones` above its open pitch.
/// Its Helmholtz band shifts with the pitch more than the other strings'
/// over their range, so it is calibrated twice, open and at the gates.
#[derive(Clone, Copy, Debug)]
pub struct Extension {
    /// How far below the gates the string's open pitch lies.
    pub semitones: f32,
    /// The band of the string stopped at the gates ([`Self::gated`]), used
    /// from there up; below, it blends into the open string's.
    pub force_limits: ForceLimits,
}

impl Extension {
    /// The lowest string as it is at the gates: shorter and higher, at the
    /// same tension and impedance, as a string without the extension.
    pub fn gated(&self, open: &StringSpec) -> StringSpec {
        let ratio = 2f32.powf(self.semitones / 12.0);
        StringSpec {
            frequency: open.frequency * ratio,
            length: open.length / ratio,
            torsion: open.torsion.map(|t| TorsionSpec {
                frequency: t.frequency * ratio,
                ..t
            }),
            ..*open
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InstrumentSpec {
    pub name: &'static str,
    /// Open strings, lowest first.
    pub strings: [StringSpec; 4],
    pub friction: FrictionParams,
    pub hair: Option<BowHair>,
    /// The friction's fluctuation while the string slips.
    pub bow_noise: BowNoise,
    pub body: BodySpec,
    /// Helmholtz band per string, lowest first (`strings-render calibrate`).
    pub force_limits: [ForceLimits; 4],
    /// A low extension on the lowest string (the bass's C extension).
    pub extension: Option<Extension>,
    /// How far above each open string it is played (semitones).
    pub reach: f32,
    /// The performer's bow position (fraction of the vibrating length from
    /// the bridge) at dynamics 0 and 1 ([`PerformerSettings::beta`]).
    ///
    /// [`PerformerSettings::beta`]: crate::PerformerSettings::beta
    pub beta: (f32, f32),
    /// Bow speed (m/s) at dynamics 0 and 1 ([`PerformerSettings::speed`]).
    ///
    /// [`PerformerSettings::speed`]: crate::PerformerSettings::speed
    pub speed: (f32, f32),
    /// The bow's closest distance to the bridge per unit of string impedance
    /// (m per kg/s; [`PerformerSettings::bow_distance`]).
    ///
    /// [`PerformerSettings::bow_distance`]: crate::PerformerSettings::bow_distance
    pub bow_distance: f32,
    /// The flautando end of the pressure control: the bow position it moves
    /// toward (sul tasto; 0 keeps the dynamics' position) and the band
    /// position ([`PerformerSettings::tasto`] and `pressure_range`).
    ///
    /// [`PerformerSettings::tasto`]: crate::PerformerSettings::tasto
    pub tasto: f32,
    pub flautando: f32,
    /// How much slower attacks are at dynamics 0 ([`PerformerTuning::pp_attack`]).
    ///
    /// [`PerformerTuning::pp_attack`]: crate::PerformerTuning::pp_attack
    pub pp_attack: f32,
    /// How far quiet strokes ease down the band once going
    /// ([`PerformerTuning::quiet_ease`]).
    ///
    /// [`PerformerTuning::quiet_ease`]: crate::PerformerTuning::quiet_ease
    pub quiet_ease: f32,
    /// Gain after the body, so every instrument plays at a similar level.
    pub output_gain: f32,
    /// Where a section of these sits on the stage.
    pub seat: Placement,
}

impl InstrumentSpec {
    /// The Helmholtz band of `string` stopped `semitones` above its open
    /// pitch: its calibrated band, or on an extended string a blend toward
    /// the band at the gates (log-linear in the force, like the band itself).
    pub fn force_limits_at(&self, string: usize, semitones: f32) -> ForceLimits {
        let open = self.force_limits[string];
        match self.extension {
            Some(e) if string == 0 => {
                let t = (semitones / e.semitones).clamp(0.0, 1.0);
                let gated = e.force_limits;
                let blend = |a: f32, b: f32| a + (b - a) * t;
                ForceLimits {
                    lower: blend(open.lower.ln(), gated.lower.ln()).exp(),
                    lower_exponent: blend(open.lower_exponent, gated.lower_exponent),
                    upper: blend(open.upper.ln(), gated.upper.ln()).exp(),
                    upper_exponent: blend(open.upper_exponent, gated.upper_exponent),
                }
            }
            _ => open,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InstrumentFrame {
    /// Body output (arbitrary units, roughly radiated pressure).
    pub output: f32,
    /// Sum of the strings' bridge forces (N).
    pub bridge_force: f32,
}

#[derive(Clone)]
pub struct Instrument {
    spec: InstrumentSpec,
    strings: [BowedString; 4],
    body: Body,
    sample_rate: f32,
    /// How many string samples per output sample (1 or 2).
    oversampling: usize,
    decimator: HalfbandDecimator,
    /// String samples each string has been silent with the bow off it.
    quiet: [u32; 4],
}

/// Below this bridge force (N) a string counts as silent: 100 dB under the
/// peak of a pp note, about −127 dBFS at the output.
const SILENT_FORCE: f32 = 1e-5;

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
            string.set_bow_noise(spec.bow_noise);
            string.reseed_noise(i as u32);
            string
        });
        Self {
            spec: *spec,
            strings,
            body: Body::new(&spec.body, sample_rate),
            sample_rate,
            oversampling,
            decimator: HalfbandDecimator::new(),
            quiet: [0; 4],
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

    pub fn set_bow_noise(&mut self, noise: BowNoise) {
        self.spec.bow_noise = noise;
        for s in &mut self.strings {
            s.set_bow_noise(noise);
        }
    }

    /// Restarts the bow noise from `seed`, so a clone plays other noise.
    pub fn reseed_noise(&mut self, seed: u32) {
        for (i, s) in self.strings.iter_mut().enumerate() {
            s.reseed_noise(seed.wrapping_mul(4).wrapping_add(i as u32));
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
        self.quiet = [0; 4];
    }

    /// Advances one sample with one bow input per string, held over the
    /// oversampled steps. Also returns each string's frame through `frames`
    /// (from the last step when oversampling).
    pub fn process(
        &mut self,
        bows: &[BowInput; 4],
        frames: &mut [StringFrame; 4],
    ) -> InstrumentFrame {
        let bridge_force = if self.oversampling == 2 {
            let first = self.step(bows, frames);
            let second = self.step(bows, frames);
            self.decimator.process([first, second])
        } else {
            self.step(bows, frames)
        };
        InstrumentFrame {
            output: self.body.process(bridge_force),
            bridge_force,
        }
    }

    /// Whether string `i` is skipped: the bow is off it and it has been silent
    /// for a whole period, so no wave is left anywhere in its loop.
    pub fn is_idle(&self, i: usize) -> bool {
        self.quiet[i] as f32 > self.string_sample_rate() / self.strings[i].frequency()
    }

    /// One string sample of every string that isn't idle; returns the sum of
    /// their bridge forces.
    fn step(&mut self, bows: &[BowInput; 4], frames: &mut [StringFrame; 4]) -> f32 {
        let mut bridge_force = 0.0;
        for i in 0..4 {
            let bow = bows[i];
            if bow.force <= 0.0 && self.is_idle(i) {
                frames[i] = StringFrame::default();
                continue;
            }
            let frame = self.strings[i].process(bow, 0.0);
            self.quiet[i] = if bow.force <= 0.0 && frame.bridge_force.abs() < SILENT_FORCE {
                self.quiet[i].saturating_add(1)
            } else {
                0
            };
            frames[i] = frame;
            bridge_force += frame.bridge_force;
        }
        bridge_force
    }
}
