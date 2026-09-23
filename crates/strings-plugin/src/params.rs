//! Host-visible parameters and the editor's persisted settings.
//!
//! The four performance controls are parameters so the host can automate
//! them. MIDI CCs drive the same controls (PLAN.md 4.1); whichever changed
//! last wins, see `Engine::apply_params`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32};

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use strings_dsp::{Articulation, PerformerSettings};

#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArticulationParam {
    Sustain,
    Staccato,
    Spiccato,
}

impl ArticulationParam {
    pub const ALL: [Self; 3] = [Self::Sustain, Self::Staccato, Self::Spiccato];

    pub fn name(self) -> &'static str {
        match self {
            Self::Sustain => "Sustain",
            Self::Staccato => "Staccato",
            Self::Spiccato => "Spiccato",
        }
    }
}

impl From<ArticulationParam> for Articulation {
    fn from(a: ArticulationParam) -> Self {
        match a {
            ArticulationParam::Sustain => Articulation::Sustain,
            ArticulationParam::Staccato => Articulation::Staccato,
            ArticulationParam::Spiccato => Articulation::Spiccato,
        }
    }
}

impl From<Articulation> for ArticulationParam {
    fn from(a: Articulation) -> Self {
        match a {
            Articulation::Sustain => Self::Sustain,
            Articulation::Staccato => Self::Staccato,
            Articulation::Spiccato => Self::Spiccato,
        }
    }
}

#[derive(Params)]
pub struct StringsParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    /// Octave shift of the computer-keyboard octave (the lowest mapped key is
    /// C at `3 + transpose`, so 0 starts at C3).
    #[persist = "transpose"]
    pub transpose: Arc<AtomicI32>,
    #[persist = "computer-keys"]
    pub computer_keys: Arc<AtomicBool>,
    #[persist = "debug-view"]
    pub debug_view: Arc<AtomicBool>,
    /// Velocity (MIDI units) of notes played from the computer keyboard.
    #[persist = "key-velocity"]
    pub key_velocity: Arc<AtomicI32>,

    #[id = "dynamics"]
    pub dynamics: FloatParam,
    #[id = "expression"]
    pub expression: FloatParam,
    #[id = "vibrato"]
    pub vibrato: FloatParam,
    #[id = "pressure"]
    pub pressure: FloatParam,
    #[id = "articulation"]
    pub articulation: EnumParam<ArticulationParam>,
    #[id = "volume"]
    pub volume: FloatParam,
}

fn unit(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Default for StringsParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(980, 640),
            transpose: Arc::new(AtomicI32::new(0)),
            computer_keys: Arc::new(AtomicBool::new(true)),
            debug_view: Arc::new(AtomicBool::new(false)),
            key_velocity: Arc::new(AtomicI32::new(90)),

            dynamics: unit("Dynamics", 0.5),
            expression: unit("Expression", 1.0),
            vibrato: unit("Vibrato", 0.0),
            pressure: unit("Pressure", PerformerSettings::default().pressure),
            articulation: EnumParam::new("Articulation", ArticulationParam::Sustain),
            volume: FloatParam::new(
                "Volume",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-36.0),
                    max: util::db_to_gain(12.0),
                    factor: FloatRange::gain_skew_factor(-36.0, 12.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}
