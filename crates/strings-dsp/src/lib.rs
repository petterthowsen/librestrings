//! Physical-modeling DSP for bowed strings.
//!
//! Everything in this crate except [`analysis`] is real-time safe: buffers are
//! allocated in constructors and `process` methods never allocate.
//!
//! Units are SI throughout: velocities in m/s, forces in N, impedances in kg/s.

pub mod analysis;
pub mod body;
pub mod bow;
pub mod delay;
pub mod filters;
pub mod instrument;
pub mod loss;
pub mod performer;
pub mod presets;
pub mod section;
pub mod stage;
pub mod string;

pub use body::{Body, BodyMode, BodySpec, BodyTuning, DenseModes, Hill};
pub use bow::{BowJunction, ContactState, FrictionParams, schelleng_limits};
pub use instrument::{ForceLimits, Instrument, InstrumentFrame, InstrumentSpec};
pub use loss::{DampingCurve, Loss};
pub use performer::{
    ARTICULATIONS, Articulation, BowLift, Fingering, Performer, PerformerFrame, PerformerSettings,
    PerformerTuning, Polyphony,
};
pub use section::{Humanization, MAX_PLAYERS, Section};
pub use stage::{Absorption, Placement, Room, RoomPreset, Stage, StageSettings};
pub use string::{
    BowHair, BowInput, BowedString, StringDesign, StringFrame, StringSpec, TorsionSpec,
};
