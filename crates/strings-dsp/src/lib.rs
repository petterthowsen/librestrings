//! Physical-modeling DSP for bowed strings.
//!
//! Everything in this crate except [`analysis`] is real-time safe: buffers are
//! allocated in constructors and `process` methods never allocate.
//!
//! Units are SI throughout: velocities in m/s, forces in N, impedances in kg/s.

pub mod analysis;
pub mod bow;
pub mod delay;
pub mod filters;
pub mod loss;
pub mod presets;
pub mod string;

pub use bow::{BowJunction, ContactState, FrictionParams, schelleng_limits};
pub use loss::{DampingCurve, Loss};
pub use string::{BowInput, BowedString, StringFrame, StringSpec, TorsionSpec};
