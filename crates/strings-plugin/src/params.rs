//! Host-visible parameters and the editor's persisted settings.
//!
//! The performance controls are parameters so the host can automate them.
//! MIDI CCs drive the same controls (PLAN.md 4.1); whichever changed last
//! wins, see `Engine::apply_params`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32};

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use strings_dsp::presets::{bass, cello, viola, violin};
use strings_dsp::{BowLift, Fingering, InstrumentSpec, MAX_PLAYERS, Polyphony};

use crate::layout::Layout;

/// The instrument the plugin plays. Not automatable: changing it builds a new
/// engine on a background thread (tens of milliseconds), which then replaces
/// the old one, cutting off what was sounding.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstrumentParam {
    // Saved in projects by these ids.
    #[id = "cello"]
    Cello,
    #[id = "violin"]
    Violin,
    #[id = "viola"]
    Viola,
    #[id = "bass"]
    #[name = "Double bass"]
    Bass,
}

impl InstrumentParam {
    /// Highest first, as in a score.
    pub const ALL: [Self; 4] = [Self::Violin, Self::Viola, Self::Cello, Self::Bass];

    /// Where it is in [`Self::ALL`] (the telemetry's index).
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&i| i == self).unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Cello => "Cello",
            Self::Violin => "Violin",
            Self::Viola => "Viola",
            Self::Bass => "Double bass",
        }
    }

    pub fn spec(self) -> &'static InstrumentSpec {
        match self {
            Self::Cello => &cello::INSTRUMENT,
            Self::Violin => &violin::INSTRUMENT,
            Self::Viola => &viola::INSTRUMENT,
            Self::Bass => &bass::INSTRUMENT,
        }
    }
}

/// What the bow does at the end of a detached note (SWAM's bow lift).
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BowLiftParam {
    #[name = "Off string"]
    OffString,
    #[name = "On string"]
    OnString,
}

impl BowLiftParam {
    pub const ALL: [Self; 2] = [Self::OffString, Self::OnString];

    pub fn name(self) -> &'static str {
        match self {
            Self::OffString => "Off string",
            Self::OnString => "On string",
        }
    }
}

impl From<BowLiftParam> for BowLift {
    fn from(b: BowLiftParam) -> Self {
        match b {
            BowLiftParam::OffString => BowLift::OffString,
            BowLiftParam::OnString => BowLift::OnString,
        }
    }
}

impl From<BowLift> for BowLiftParam {
    fn from(b: BowLift) -> Self {
        match b {
            BowLift::OffString => Self::OffString,
            BowLift::OnString => Self::OnString,
        }
    }
}

#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolyphonyParam {
    Mono,
    #[name = "Double stops"]
    DoubleStops,
    /// A section divides a chord's notes among its players.
    Divisi,
}

impl PolyphonyParam {
    pub const ALL: [Self; 3] = [Self::Mono, Self::DoubleStops, Self::Divisi];

    pub fn name(self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::DoubleStops => "Double stops",
            Self::Divisi => "Divisi",
        }
    }
}

impl From<PolyphonyParam> for Polyphony {
    fn from(p: PolyphonyParam) -> Self {
        match p {
            PolyphonyParam::Mono => Polyphony::Mono,
            PolyphonyParam::DoubleStops => Polyphony::DoubleStops,
            PolyphonyParam::Divisi => Polyphony::Divisi,
        }
    }
}

impl From<Polyphony> for PolyphonyParam {
    fn from(p: Polyphony) -> Self {
        match p {
            Polyphony::Mono => Self::Mono,
            Polyphony::DoubleStops => Self::DoubleStops,
            Polyphony::Divisi => Self::Divisi,
        }
    }
}

/// Where the left hand plays (SWAM's fingering modes).
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FingeringParam {
    #[name = "Near the nut & open"]
    NutAndOpen,
    #[name = "Mid position"]
    Mid,
    #[name = "Near the bridge"]
    Bridge,
}

impl FingeringParam {
    pub const ALL: [Self; 3] = [Self::NutAndOpen, Self::Mid, Self::Bridge];

    pub fn name(self) -> &'static str {
        match self {
            Self::NutAndOpen => "Near the nut & open",
            Self::Mid => "Mid position",
            Self::Bridge => "Near the bridge",
        }
    }
}

impl From<FingeringParam> for Fingering {
    fn from(f: FingeringParam) -> Self {
        match f {
            FingeringParam::NutAndOpen => Fingering::NutAndOpen,
            FingeringParam::Mid => Fingering::Mid,
            FingeringParam::Bridge => Fingering::Bridge,
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
    /// The editor shows the stage instead of the instrument.
    #[persist = "stage-view"]
    pub stage_view: Arc<AtomicBool>,
    /// Velocity (MIDI units) of notes played from the computer keyboard.
    #[persist = "key-velocity"]
    pub key_velocity: Arc<AtomicI32>,

    #[id = "instrument"]
    pub instrument: EnumParam<InstrumentParam>,
    #[id = "dynamics"]
    pub dynamics: FloatParam,
    #[id = "vibrato"]
    pub vibrato: FloatParam,
    /// Flautando at 0, normal at 0.5, scratch at 1.
    #[id = "pressure"]
    pub pressure: FloatParam,
    #[id = "bow-lift"]
    pub bow_lift: EnumParam<BowLiftParam>,
    #[id = "polyphony"]
    pub polyphony: EnumParam<PolyphonyParam>,
    #[id = "fingering"]
    pub fingering: EnumParam<FingeringParam>,
    #[id = "volume"]
    pub volume: FloatParam,

    /// Players in the section; 1 is a solo instrument.
    #[id = "players"]
    pub players: IntParam,
    /// Off: the players' sum, dry and mono, as before the stage.
    #[id = "stage"]
    pub stage: BoolParam,
    /// Where the section sits and the room, shared with the other instances
    /// on the stage (`layout`, `sync`).
    #[persist = "stage-layout"]
    pub layout: Arc<Layout>,
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
            stage_view: Arc::new(AtomicBool::new(false)),
            key_velocity: Arc::new(AtomicI32::new(90)),

            instrument: EnumParam::new("Instrument", InstrumentParam::Cello).non_automatable(),
            dynamics: unit("Dynamics", 0.5),
            vibrato: unit("Vibrato", 0.0),
            pressure: unit("Pressure", 0.5),
            bow_lift: EnumParam::new("Bow lift", BowLiftParam::OffString),
            polyphony: EnumParam::new("Polyphony", PolyphonyParam::Mono),
            fingering: EnumParam::new("Fingering", FingeringParam::NutAndOpen),
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

            players: IntParam::new(
                "Players",
                1,
                IntRange::Linear {
                    min: 1,
                    max: MAX_PLAYERS as i32,
                },
            ),
            stage: BoolParam::new("Stage", true),
            layout: Arc::new(Layout::default()),
        }
    }
}
