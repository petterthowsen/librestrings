//! The model's numbers as the tuning window edits them.
//!
//! Most take effect at once: the editor sends a [`LiveTuning`] through a
//! lock-free queue and the audio thread copies it in. The strings' damping,
//! stiffness and torsion need their loss and dispersion filters fitted again
//! (about 30 ms for a cello), so the editor fits them on a thread of its own
//! and sends the result as a [`StringsUpdate`]; the audio thread swaps it in
//! and sends the old one back to be freed.

use strings_dsp::{
    BodyTuning, BowHair, DampingCurve, FrictionParams, Humanization, InstrumentSpec, Loss,
    PerformerSettings, StringDesign, StringSpec, TorsionSpec,
};

/// Everything the audio thread can take at once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveTuning {
    pub performer: PerformerSettings,
    pub friction: FrictionParams,
    pub hair: Option<BowHair>,
    pub body: BodyTuning,
    /// How far a section's players spread around player 0.
    pub humanization: Humanization,
}

/// The strings' parameters, shared by all four strings as in the cello preset
/// (`presets::cello::string`). Torsion is relative to each string.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StringsTuning {
    pub damping: DampingCurve,
    /// Bending stiffness EI (N·m²).
    pub bending_stiffness: f32,
    /// Torsional impedance and fundamental as multiples of the transverse ones.
    pub torsion_impedance: f32,
    pub torsion_frequency: f32,
    pub torsion_q: f32,
}

impl StringsTuning {
    pub fn from_spec(spec: &InstrumentSpec) -> Self {
        let s = &spec.strings[0];
        let damping = match s.loss {
            Loss::Measured(curve) => curve,
            Loss::OnePole { .. } => DampingCurve {
                floor: 0.0,
                at_1khz: 0.0,
                exponent: 1.0,
            },
        };
        let torsion = s.torsion.unwrap_or(TorsionSpec {
            impedance: 0.0,
            frequency: 0.0,
            q: 1.0,
        });
        Self {
            damping,
            bending_stiffness: s.bending_stiffness,
            torsion_impedance: torsion.impedance / s.impedance(),
            torsion_frequency: torsion.frequency / s.frequency,
            torsion_q: torsion.q,
        }
    }

    /// The instrument's strings with these parameters. A string keeps its kind
    /// of loss and whether it has torsion.
    pub fn apply_to(&self, strings: &[StringSpec; 4]) -> [StringSpec; 4] {
        strings.map(|s| StringSpec {
            loss: match s.loss {
                Loss::Measured(_) => Loss::Measured(self.damping),
                one_pole => one_pole,
            },
            bending_stiffness: self.bending_stiffness,
            torsion: s.torsion.map(|_| TorsionSpec {
                impedance: self.torsion_impedance * s.impedance(),
                frequency: self.torsion_frequency * s.frequency,
                q: self.torsion_q,
            }),
            ..s
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    pub live: LiveTuning,
    pub strings: StringsTuning,
}

impl Tuning {
    pub fn new(spec: &InstrumentSpec) -> Self {
        Self {
            live: LiveTuning {
                performer: PerformerSettings::default(),
                friction: spec.friction,
                hair: spec.hair,
                body: BodyTuning::from(&spec.body),
                humanization: Humanization::default(),
            },
            strings: StringsTuning::from_spec(spec),
        }
    }
}

/// New string parameters with their fitted filters, on their way to the audio
/// thread (and back, holding the old filters).
pub struct StringsUpdate {
    /// Counts the editor's requests; the audio thread reports the last one it applied.
    pub generation: u32,
    pub specs: [StringSpec; 4],
    /// For player 0, fitted at its string rate.
    pub designs: [StringDesign; 4],
    /// For each of the other players, at theirs.
    pub player_designs: Vec<[StringDesign; 4]>,
}
