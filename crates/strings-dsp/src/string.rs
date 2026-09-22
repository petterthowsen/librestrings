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
//! The per-period DC loss `g` is split between the two reflections in proportion
//! to segment length, so a segment trapped by a sticking bow still decays. The
//! loss filter (frequency-dependent loss, see [`crate::loss`]) sits at the
//! bridge; delay lengths are shortened by its phase delay at `f0` so the loop
//! stays in tune.
//!
//! Bending stiffness adds a dispersion allpass at the nut reflection. It sits on
//! the nut side, the long one, because its delay (tens of samples) would
//! otherwise move the bow's effective position at small β.
//!
//! Torsional waves, when enabled, travel in two more round-trip lines on either
//! side of the bow. They couple to the transverse waves only at the bow: the
//! friction force drives both, and the bow sees the sum of their velocities at
//! the string's surface. The bridge force (the output) is transverse only.

use crate::bow::{BowJunction, ContactState, FrictionParams};
use crate::delay::DelayLine;
use crate::filters::DispersionAllpass;
use crate::loss::{Loss, LossDesign, LossFilter};

#[derive(Clone, Copy, Debug)]
pub struct StringSpec {
    pub name: &'static str,
    /// Open-string fundamental (Hz).
    pub frequency: f32,
    /// Vibrating length of the open string (m).
    pub length: f32,
    /// Tension (N).
    pub tension: f32,
    /// How much each partial loses per period.
    pub loss: Loss,
    /// Bending stiffness EI (N·m²). Zero for an ideally flexible string.
    pub bending_stiffness: f32,
    /// Torsional waves; `None` leaves them out.
    pub torsion: Option<TorsionSpec>,
}

/// Torsional waves on the open string, as seen at the string's surface.
#[derive(Clone, Copy, Debug)]
pub struct TorsionSpec {
    /// Torsional characteristic impedance referred to the surface, i.e. torque
    /// impedance over radius² (kg/s). Roughly `κ·μ·c_t`, with `κ·μ·r²` the
    /// polar moment of inertia per length.
    pub impedance: f32,
    /// Torsional fundamental of the open string (Hz). Stopping the string scales
    /// it with the transverse pitch.
    pub frequency: f32,
    /// Quality factor of the torsional fundamental; the loss per period is the
    /// same for every torsional mode.
    pub q: f32,
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

    /// Inharmonicity coefficient `B = π²·EI / (T·L²)` of the open string.
    pub fn inharmonicity(&self) -> f32 {
        std::f32::consts::PI.powi(2) * self.bending_stiffness
            / (self.tension * self.length * self.length)
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
    loss: Loss,

    nut: DelayLine,
    to_bridge: DelayLine,
    from_bridge: DelayLine,
    loss_filter: LossFilter,
    dispersion: DispersionAllpass,
    stiff: bool,
    /// Loss and dispersion designs per semitone above `lowest_frequency`. Empty
    /// for a flexible string with one-pole loss, which needs no fitting.
    notes: Vec<NoteDesign>,
    lowest_frequency: f32,
    loop_gain: f32,
    bridge_gain: f32,
    nut_gain: f32,
    bow: BowJunction,
    torsion: Option<Torsion>,

    open_frequency: f32,
    frequency: f32,
    beta: f32,
    nut_delay: f32,
    bridge_delay: f32,
}

#[derive(Clone, Copy, Debug)]
struct NoteDesign {
    loss: LossDesign,
    /// Dispersion allpass coefficient.
    dispersion: f32,
}

/// Torsional waveguide: one round-trip line on each side of the bow.
struct Torsion {
    spec: TorsionSpec,
    impedance: f32,
    nut: DelayLine,
    bridge: DelayLine,
    nut_delay: f32,
    bridge_delay: f32,
    nut_gain: f32,
    bridge_gain: f32,
}

impl Torsion {
    fn update(&mut self, frequency_ratio: f32, beta: f32, sample_rate: f32) {
        let frequency = self.spec.frequency * frequency_ratio;
        let period = sample_rate / frequency;
        let max = self.nut.max_delay();
        self.bridge_delay = (beta * period).clamp(DelayLine::MIN_DELAY, max);
        self.nut_delay = (period - self.bridge_delay).clamp(DelayLine::MIN_DELAY, max);
        // Amplitude falls by e^(−π/Q) per period at the torsional fundamental.
        let loop_gain = (-std::f32::consts::PI / self.spec.q).exp();
        let bridge_share = self.bridge_delay / period;
        self.bridge_gain = loop_gain.powf(bridge_share);
        self.nut_gain = loop_gain.powf(1.0 - bridge_share);
    }

    fn reset(&mut self) {
        self.nut.clear();
        self.bridge.clear();
    }
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
        let measured_loss = matches!(spec.loss, Loss::Measured(_));
        // B grows with the square of the sounding pitch (it scales as 1/L²).
        let b_open = spec.inharmonicity();
        let stiff = b_open > 0.0;
        let notes = if stiff || measured_loss {
            let semitones = (12.0
                * (Self::HIGHEST_ABOVE_OPEN * spec.frequency / lowest_frequency).log2())
            .ceil()
            .max(0.0) as usize;
            let mut filter = LossFilter::new(sample_rate, measured_loss);
            (0..=semitones)
                .map(|i| {
                    let f = lowest_frequency * 2f32.powf(i as f32 / 12.0);
                    let loss = Self::loss_design(&spec.loss, f, sample_rate);
                    filter.set(&loss);
                    let b = b_open * (f / spec.frequency).powi(2);
                    let dispersion =
                        DispersionAllpass::design(f, sample_rate, b, |w| filter.phase_lag(w));
                    NoteDesign { loss, dispersion }
                })
                .collect()
        } else {
            Vec::new()
        };
        let impedance = spec.impedance();
        let torsion = spec.torsion.map(|t| {
            let max_torsion = (sample_rate / (t.frequency * lowest_frequency / spec.frequency))
                .ceil() as usize
                + 4;
            Torsion {
                spec: t,
                impedance: t.impedance,
                nut: DelayLine::new(max_torsion),
                bridge: DelayLine::new(max_torsion),
                nut_delay: 0.0,
                bridge_delay: 0.0,
                nut_gain: 1.0,
                bridge_gain: 1.0,
            }
        });
        let mut s = Self {
            sample_rate,
            impedance,
            loss: spec.loss,
            nut: DelayLine::new(max_loop),
            to_bridge: DelayLine::new(max_loop / 2),
            from_bridge: DelayLine::new(max_loop / 2),
            loss_filter: LossFilter::new(sample_rate, measured_loss),
            dispersion: DispersionAllpass::new(),
            stiff,
            notes,
            lowest_frequency,
            loop_gain: 1.0,
            bridge_gain: 1.0,
            nut_gain: 1.0,
            bow: BowJunction::new(friction),
            torsion,
            open_frequency: spec.frequency,
            frequency: spec.frequency,
            beta: 0.1,
            nut_delay: 0.0,
            bridge_delay: 0.0,
        };
        s.update_delays();
        s
    }

    /// Stopped notes can go this far above the open string (a ratio) with their
    /// stiffness and measured loss modeled; higher ones keep the top design.
    const HIGHEST_ABOVE_OPEN: f32 = 8.0;

    /// Transverse wave impedance (kg/s).
    pub fn impedance(&self) -> f32 {
        self.impedance
    }

    /// Coefficient of the dispersion allpass (0 for a flexible string).
    pub fn dispersion_coefficient(&self) -> f32 {
        self.dispersion.a
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
        self.loss_filter.reset();
        self.dispersion.reset();
        self.bow.reset();
        if let Some(t) = &mut self.torsion {
            t.reset();
        }
    }

    /// Advances one sample. `external_force` (N) is added at the bow point, for
    /// plucks and test impulses.
    pub fn process(&mut self, bow: BowInput, external_force: f32) -> StringFrame {
        let z = self.impedance;
        let from_nut = -self.nut_gain * self.nut.read(self.nut_delay);
        let from_nut = if self.stiff {
            self.dispersion.process(from_nut)
        } else {
            from_nut
        };
        let arriving_at_bridge = self.to_bridge.read(self.bridge_delay);
        let reflected = -self.bridge_gain * self.loss_filter.process(arriving_at_bridge);
        let from_bridge = self.from_bridge.read(self.bridge_delay);

        let v_h = from_nut + from_bridge + external_force / (2.0 * z);
        let j = match &mut self.torsion {
            None => self.bow.solve(v_h, bow.velocity, bow.force, z),
            Some(t) => {
                let from_t_nut = -t.nut_gain * t.nut.read(t.nut_delay);
                let from_t_bridge = -t.bridge_gain * t.bridge.read(t.bridge_delay);
                // The bow sees the transverse and torsional impedances in parallel.
                let z_t = t.impedance;
                let z_bow = z * z_t / (z + z_t);
                let j = self.bow.solve(
                    v_h + from_t_nut + from_t_bridge,
                    bow.velocity,
                    bow.force,
                    z_bow,
                );
                let half_t = j.force / (2.0 * z_t);
                t.nut.push(from_t_bridge + half_t);
                t.bridge.push(from_t_nut + half_t);
                j
            }
        };
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
        let note = self.note_design();
        self.loss_filter.set(&note.loss);
        let filter_delay = self.loss_filter.phase_delay(omega);
        let total = self.sample_rate / self.frequency - filter_delay;
        let max_bridge = self.to_bridge.max_delay();
        self.bridge_delay = (0.5 * self.beta * total).clamp(DelayLine::MIN_DELAY, max_bridge);
        self.dispersion.a = note.dispersion;
        let dispersion_delay = if self.stiff {
            DispersionAllpass::phase_lag(self.dispersion.a, omega) / omega
        } else {
            0.0
        };
        self.nut_delay = (total - 2.0 * self.bridge_delay - dispersion_delay)
            .clamp(DelayLine::MIN_DELAY, self.nut.max_delay());
        if let Some(t) = &mut self.torsion {
            t.update(
                self.frequency / self.open_frequency,
                self.beta,
                self.sample_rate,
            );
        }
        self.loop_gain = match self.loss {
            Loss::OnePole { t60, .. } => 10f32.powf(-3.0 / (t60 * self.frequency)),
            Loss::Measured(_) => (-note.loss.dc_loss).exp(),
        };
        let bridge_share = 2.0 * self.bridge_delay / total;
        self.bridge_gain = self.loop_gain.powf(bridge_share);
        self.nut_gain = self.loop_gain.powf(1.0 - bridge_share);
    }

    /// Loss design for a loop tuned to `frequency`. One-pole loss is computed
    /// directly; measured loss is fitted (slow).
    fn loss_design(loss: &Loss, frequency: f32, sample_rate: f32) -> LossDesign {
        match *loss {
            Loss::OnePole { t60, lowpass } => LossDesign {
                dc_loss: 3.0 * std::f32::consts::LN_10 / (t60 * frequency),
                pole: lowpass.powf(48_000.0 / sample_rate),
                cutoff: 0.5 * sample_rate,
            },
            Loss::Measured(curve) => curve.design(frequency, sample_rate),
        }
    }

    /// Loss and dispersion for the current pitch, interpolated between semitones.
    fn note_design(&self) -> NoteDesign {
        let table = &self.notes;
        if table.is_empty() {
            return NoteDesign {
                loss: Self::loss_design(&self.loss, self.frequency, self.sample_rate),
                dispersion: 0.0,
            };
        }
        let pos = (12.0 * (self.frequency / self.lowest_frequency).log2())
            .clamp(0.0, (table.len() - 1) as f32);
        let i = (pos as usize).min(table.len().saturating_sub(2));
        let t = pos - i as f32;
        match table.get(i + 1) {
            Some(next) => NoteDesign {
                loss: table[i].loss.lerp(&next.loss, t),
                dispersion: table[i].dispersion + t * (next.dispersion - table[i].dispersion),
            },
            None => table[i],
        }
    }
}
