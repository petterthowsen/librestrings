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
//! the string's surface. The bridge force (the output) is transverse only. Their
//! damping has constant Q (Woodhouse & Loach 1999), so a torsional mode loses
//! in proportion to its frequency per period: a loss filter at the nut-side
//! reflection, fitted like the measured transverse loss.
//!
//! Bow hair, when enabled, is a spring and dashpot (Kelvin–Voigt) between the
//! bow stick and the contact point. The hairs at the contact move with the
//! string while sticking, so a bow stopped on the string no longer clamps it
//! rigidly: the dashpot absorbs the waves that would otherwise stay trapped.
//!
//! A hair ribbon with width touches the string at several contact points
//! (up to [`MAX_CONTACTS`]), a whole number of samples apart, with short
//! lines between them. Each takes an equal share of the bow force and of the
//! hair's compliance and has its own stick/slip state, so the ribbon can slip
//! at one edge while the other still sticks (Pitteroff & Woodhouse 1998).
//! Torsional waves get lines between the contacts too.

use crate::bow::{BowJunction, BowNoise, ContactState, FrictionParams, JunctionResult};
use crate::delay::DelayLine;
use crate::filters::DispersionAllpass;
use crate::loss::{DampingCurve, Loss, LossDesign, LossFilter};

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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TorsionSpec {
    /// Torsional characteristic impedance referred to the surface, i.e. torque
    /// impedance over radius² (kg/s). Roughly `κ·μ·c_t`, with `κ·μ·r²` the
    /// polar moment of inertia per length.
    pub impedance: f32,
    /// Torsional fundamental of the open string (Hz). Stopping the string scales
    /// it with the transverse pitch.
    pub frequency: f32,
    /// Quality factor of every torsional mode (constant Q, as Woodhouse and
    /// Loach measured): mode k loses `π·k/Q` nepers per torsional period.
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

/// Compliance of the bow hair in the bowing direction, at the contact point.
///
/// The hair ribbon behaves like a spring at low frequencies (its longitudinal
/// stiffness to frog and tip) and like a resistance at high frequencies (the
/// wave impedance of the hairs, whose waves are strongly damped). A spring in
/// parallel with a dashpot has both asymptotes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BowHair {
    /// Spring stiffness (N/m).
    pub stiffness: f32,
    /// Dashpot resistance (kg/s).
    pub damping: f32,
    /// Width of the ribbon in contact with the string, along the string (m).
    /// Zero touches the string at one point.
    pub width: f32,
}

/// The most contact points a bow with width touches the string at.
pub const MAX_CONTACTS: usize = 4;

/// The longest line between two contact points (samples).
const MAX_GAP: usize = 64;

#[derive(Clone, Copy, Debug, Default)]
pub struct BowInput {
    /// Bow velocity (m/s). The sign sets the bow direction.
    pub velocity: f32,
    /// Normal force pressing the bow onto the string (N). Zero means lifted.
    pub force: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StringFrame {
    /// Transverse force the string exerts on the bridge (N): the audio output.
    pub bridge_force: f32,
    /// String velocity at the bow point (m/s).
    pub bow_point_velocity: f32,
    /// Friction force the bow applies (N).
    pub friction_force: f32,
    pub state: ContactState,
}

#[derive(Clone)]
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
    /// Loss and dispersion designs per semitone above the lowest frequency.
    design: StringDesign,
    loop_gain: f32,
    bridge_gain: f32,
    nut_gain: f32,
    /// The bridge reflection's share of the loop's DC loss.
    bridge_share: f32,
    /// Extra loss at the nut or stopping finger (nepers per reflection).
    termination_loss: f32,
    /// One junction per contact point, from the bridge side.
    bow: [BowJunction; MAX_CONTACTS],
    torsion: Option<Torsion>,
    hair: Option<BowHair>,
    /// Deflection of the hair at each contact point relative to the bow stick (m).
    hair_deflection: [f32; MAX_CONTACTS],
    /// How many contact points the bow touches the string at.
    contacts: usize,
    noise: BowNoise,
    /// Between neighboring contacts: the delay (whole samples, one way), and
    /// the waves traveling toward the nut and toward the bridge.
    gap: usize,
    to_nut_gaps: [DelayLine; MAX_CONTACTS - 1],
    to_bridge_gaps: [DelayLine; MAX_CONTACTS - 1],
    /// Transverse wave speed (m/s).
    wave_speed: f32,

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
    /// Loss of the torsional loop (none without torsion).
    torsion: LossDesign,
}

/// A string's loss, dispersion and torsional loss, fitted per semitone (see
/// [`BowedString::design`]). Empty for a flexible string with one-pole loss and
/// no torsion, which needs no fitting.
#[derive(Clone)]
pub struct StringDesign {
    notes: Vec<NoteDesign>,
    lowest_frequency: f32,
    sample_rate: f32,
}

/// A loss design that loses nothing.
const NO_LOSS: LossDesign = LossDesign {
    dc_loss: 0.0,
    pole: 0.0,
    cutoff: 20_000.0,
};

/// Torsional waveguide: one round-trip line on each side of the bow, with the
/// loss filter at the nut-side reflection, and one-way lines between the
/// contact points of a bow with width.
#[derive(Clone)]
struct Torsion {
    spec: TorsionSpec,
    impedance: f32,
    nut: DelayLine,
    bridge: DelayLine,
    /// Delay between neighboring contacts (whole samples, one way): the
    /// transverse gap scaled by the ratio of wave speeds, at least one sample.
    gap: usize,
    to_nut_gaps: [DelayLine; MAX_CONTACTS - 1],
    to_bridge_gaps: [DelayLine; MAX_CONTACTS - 1],
    filter: LossFilter,
    nut_delay: f32,
    bridge_delay: f32,
    nut_gain: f32,
    bridge_gain: f32,
}

impl Torsion {
    /// `width` is the torsional delay across the contacts (samples, one way).
    fn update(
        &mut self,
        frequency_ratio: f32,
        beta: f32,
        width: f32,
        sample_rate: f32,
        loss: &LossDesign,
    ) {
        let frequency = self.spec.frequency * frequency_ratio;
        self.filter.set(loss);
        // The filter's phase delay at the fundamental comes off the nut side,
        // so the torsional loop stays in tune.
        let omega = std::f32::consts::TAU * frequency / sample_rate;
        let period = sample_rate / frequency - self.filter.phase_delay(omega);
        let max = self.nut.max_delay();
        self.bridge_delay = (beta * period - width).clamp(DelayLine::MIN_DELAY, max);
        self.nut_delay =
            (period - self.bridge_delay - 2.0 * width).clamp(DelayLine::MIN_DELAY, max);
        let loop_gain = (-loss.dc_loss).exp();
        let bridge_share = self.bridge_delay / period;
        self.bridge_gain = loop_gain.powf(bridge_share);
        self.nut_gain = loop_gain.powf(1.0 - bridge_share);
    }

    /// Constant-Q loss of a torsional loop tuned to `frequency`. Slow: fitted
    /// like the measured transverse loss.
    fn loss_design(q: f32, frequency: f32, sample_rate: f32) -> LossDesign {
        DampingCurve {
            floor: 0.5 / q,
            at_1khz: 0.0,
            exponent: 1.0,
        }
        .design(frequency, sample_rate)
    }

    fn reset(&mut self) {
        self.nut.clear();
        self.bridge.clear();
        self.filter.reset();
        for line in self.to_nut_gaps.iter_mut().chain(&mut self.to_bridge_gaps) {
            line.clear();
        }
    }
}

impl BowedString {
    /// `lowest_frequency` bounds the delay memory: the string can be tuned
    /// down to [`Self::DETUNE_ROOM`] below it (a player's own tuning), not
    /// further.
    pub fn new(
        spec: &StringSpec,
        friction: FrictionParams,
        sample_rate: f32,
        lowest_frequency: f32,
    ) -> Self {
        let max_loop = (Self::DETUNE_ROOM * sample_rate / lowest_frequency).ceil() as usize + 4;
        let measured_loss = matches!(spec.loss, Loss::Measured(_));
        let stiff = spec.inharmonicity() > 0.0;
        let impedance = spec.impedance();
        let torsion = spec.torsion.map(|t| {
            // Room for the torsional fundamental to be tuned down to
            // `LOWEST_TORSION_RATIO` × the transverse one while playing.
            let lowest = t.frequency.min(Self::LOWEST_TORSION_RATIO * spec.frequency)
                * lowest_frequency
                / spec.frequency;
            let max_torsion = (sample_rate / lowest).ceil() as usize + 4;
            Torsion {
                spec: t,
                impedance: t.impedance,
                nut: DelayLine::new(max_torsion),
                bridge: DelayLine::new(max_torsion),
                gap: 1,
                to_nut_gaps: std::array::from_fn(|_| DelayLine::new(MAX_GAP)),
                to_bridge_gaps: std::array::from_fn(|_| DelayLine::new(MAX_GAP)),
                filter: LossFilter::new(sample_rate, true),
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
            design: Self::design(spec, sample_rate, lowest_frequency),
            loop_gain: 1.0,
            bridge_gain: 1.0,
            nut_gain: 1.0,
            bridge_share: 0.0,
            termination_loss: 0.0,
            bow: std::array::from_fn(|_| BowJunction::new(friction)),
            torsion,
            hair: None,
            hair_deflection: [0.0; MAX_CONTACTS],
            contacts: 1,
            noise: BowNoise::default(),
            gap: 0,
            to_nut_gaps: std::array::from_fn(|_| DelayLine::new(MAX_GAP)),
            to_bridge_gaps: std::array::from_fn(|_| DelayLine::new(MAX_GAP)),
            wave_speed: spec.wave_speed(),
            open_frequency: spec.frequency,
            frequency: spec.frequency,
            beta: 0.1,
            nut_delay: 0.0,
            bridge_delay: 0.0,
        };
        s.update_delays();
        s.reseed_noise(0);
        s
    }

    /// Room in the delay memory for tuning below `lowest_frequency` (a
    /// ratio): 50 cents, more than a player's detune.
    pub const DETUNE_ROOM: f32 = 1.03;

    /// Stopped notes can go this far above the open string (a ratio) with their
    /// stiffness and measured loss modeled; higher ones keep the top design.
    const HIGHEST_ABOVE_OPEN: f32 = 8.0;

    /// The lowest torsional fundamental, as a multiple of the transverse one,
    /// that [`Self::apply_design`] can tune to without more memory.
    pub const LOWEST_TORSION_RATIO: f32 = 2.0;

    /// Fits the loss, stiffness and torsional loss of `spec` per semitone from
    /// `lowest_frequency` up. Slow (about 7 ms for a cello string) and
    /// allocating: build it off the audio thread.
    pub fn design(spec: &StringSpec, sample_rate: f32, lowest_frequency: f32) -> StringDesign {
        let measured_loss = matches!(spec.loss, Loss::Measured(_));
        // B grows with the square of the sounding pitch (it scales as 1/L²).
        let b_open = spec.inharmonicity();
        let notes = if b_open > 0.0 || measured_loss || spec.torsion.is_some() {
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
                    let torsion = spec.torsion.map_or(NO_LOSS, |t| {
                        let f_t = t.frequency * f / spec.frequency;
                        Torsion::loss_design(t.q, f_t, sample_rate)
                    });
                    NoteDesign {
                        loss,
                        dispersion,
                        torsion,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        StringDesign {
            notes,
            lowest_frequency,
            sample_rate,
        }
    }

    /// Takes the loss, stiffness and torsion of `spec` while the string plays,
    /// with `design` made by [`Self::design`] for the same spec, sample rate and
    /// lowest frequency. Real-time safe: the designs are swapped, so `design`
    /// comes back holding the old one, to be dropped off the audio thread.
    ///
    /// The spec must keep the string's pitch and kind of loss, and torsion can
    /// only be retuned (not added or removed); returns `false` and changes
    /// nothing otherwise.
    pub fn apply_design(&mut self, spec: &StringSpec, design: &mut StringDesign) -> bool {
        let compatible = design.sample_rate == self.sample_rate
            && design.lowest_frequency == self.design.lowest_frequency
            && spec.frequency == self.open_frequency
            && matches!(spec.loss, Loss::Measured(_)) == matches!(self.loss, Loss::Measured(_))
            && spec.torsion.is_some() == self.torsion.is_some()
            && spec
                .torsion
                .is_none_or(|t| t.frequency >= Self::LOWEST_TORSION_RATIO * spec.frequency);
        if !compatible {
            return false;
        }
        std::mem::swap(&mut self.design, design);
        self.loss = spec.loss;
        self.stiff = spec.inharmonicity() > 0.0;
        self.impedance = spec.impedance();
        self.wave_speed = spec.wave_speed();
        if let (Some(t), Some(t_spec)) = (&mut self.torsion, spec.torsion) {
            t.spec = t_spec;
            t.impedance = t_spec.impedance;
        }
        if !self.stiff {
            self.dispersion.reset();
        }
        self.update_contacts();
        true
    }

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

    /// Makes the bow hair compliant, and gives it its width; `None` (the
    /// default) grips rigidly at one point.
    pub fn set_bow_hair(&mut self, hair: Option<BowHair>) {
        if hair.is_none() || self.hair.is_none() {
            self.hair_deflection = [0.0; MAX_CONTACTS];
        }
        self.hair = hair;
        self.update_contacts();
    }

    /// How many points the bow touches the string at.
    pub fn contacts(&self) -> usize {
        self.contacts
    }

    pub fn set_friction(&mut self, friction: FrictionParams) {
        for bow in &mut self.bow {
            bow.friction = friction;
        }
    }

    /// The bow noise. Off by default.
    pub fn set_bow_noise(&mut self, noise: BowNoise) {
        self.noise = noise;
        self.apply_noise();
    }

    /// Each contact point draws its own noise, raised by √n so that their sum
    /// fluctuates by the noise's level whatever the bow's width.
    fn apply_noise(&mut self) {
        let noise = BowNoise {
            level: self.noise.level * (self.contacts as f32).sqrt(),
            ..self.noise
        };
        for bow in &mut self.bow {
            bow.set_noise(noise, self.sample_rate);
        }
    }

    /// Restarts the bow noise from `seed`; each contact point draws its own.
    pub fn reseed_noise(&mut self, seed: u32) {
        for (i, bow) in self.bow.iter_mut().enumerate() {
            bow.reseed_noise(
                seed.wrapping_mul(MAX_CONTACTS as u32)
                    .wrapping_add(i as u32),
            );
        }
    }

    /// Extra loss where the string is stopped (nepers per reflection at the
    /// nut or finger), such as a fingertip's damping. Zero by default.
    pub fn set_termination_loss(&mut self, nepers: f32) {
        self.termination_loss = nepers.max(0.0);
        self.update_gains();
    }

    pub fn reset(&mut self) {
        self.hair_deflection = [0.0; MAX_CONTACTS];
        self.nut.clear();
        self.to_bridge.clear();
        self.from_bridge.clear();
        for line in self.to_nut_gaps.iter_mut().chain(&mut self.to_bridge_gaps) {
            line.clear();
        }
        self.loss_filter.reset();
        self.dispersion.reset();
        for bow in &mut self.bow {
            bow.reset();
        }
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

        // The waves arriving at each contact point, from the bridge side (`a`)
        // and from the nut side (`b`); `ta` and `tb` the torsional ones.
        let n = self.contacts;
        let mut a = [0.0; MAX_CONTACTS];
        let mut b = [0.0; MAX_CONTACTS];
        let mut ta = [0.0; MAX_CONTACTS];
        let mut tb = [0.0; MAX_CONTACTS];
        a[0] = from_bridge;
        b[n - 1] = from_nut;
        for i in 1..n {
            a[i] = self.to_nut_gaps[i - 1].read_whole(self.gap);
            b[i - 1] = self.to_bridge_gaps[i - 1].read_whole(self.gap);
        }
        if let Some(t) = &mut self.torsion {
            ta[0] = -t.bridge_gain * t.bridge.read(t.bridge_delay);
            tb[n - 1] = -t.nut_gain * t.filter.process(t.nut.read(t.nut_delay));
            for i in 1..n {
                ta[i] = t.to_nut_gaps[i - 1].read_whole(t.gap);
                tb[i - 1] = t.to_bridge_gaps[i - 1].read_whole(t.gap);
            }
        }

        // Each contact takes an equal share of the bow force and the hair.
        let share = 1.0 / n as f32;
        let contact_bow = BowInput {
            velocity: bow.velocity,
            force: bow.force * share,
        };
        let hair = self.hair.map(|h| BowHair {
            stiffness: h.stiffness * share,
            damping: h.damping * share,
            ..h
        });
        // External forces and the frame's velocity and state are at the middle contact.
        let middle = (n - 1) / 2;
        let mut frame = StringFrame::default();
        for i in 0..n {
            let external = if i == middle { external_force } else { 0.0 };
            let v_h = b[i] + a[i] + external / (2.0 * z);
            let j = match &mut self.torsion {
                None => Self::solve_bow(
                    &mut self.bow[i],
                    hair,
                    &mut self.hair_deflection[i],
                    self.sample_rate,
                    v_h,
                    contact_bow,
                    z,
                ),
                Some(t) => {
                    // The bow sees the transverse and torsional impedances in parallel.
                    let z_t = t.impedance;
                    let z_bow = z * z_t / (z + z_t);
                    let j = Self::solve_bow(
                        &mut self.bow[i],
                        hair,
                        &mut self.hair_deflection[i],
                        self.sample_rate,
                        v_h + tb[i] + ta[i],
                        contact_bow,
                        z_bow,
                    );
                    let half_t = j.force / (2.0 * z_t);
                    let (to_nut, to_bridge) = (ta[i] + half_t, tb[i] + half_t);
                    if i + 1 == n {
                        t.nut.push(to_nut);
                    } else {
                        t.to_nut_gaps[i].push(to_nut);
                    }
                    if i == 0 {
                        t.bridge.push(to_bridge);
                    } else {
                        t.to_bridge_gaps[i - 1].push(to_bridge);
                    }
                    j
                }
            };
            let half = (j.force + external) / (2.0 * z);
            if i == 0 {
                self.to_bridge.push(b[i] + half);
            } else {
                self.to_bridge_gaps[i - 1].push(b[i] + half);
            }
            if i + 1 == n {
                self.nut.push(a[i] + half);
            } else {
                self.to_nut_gaps[i].push(a[i] + half);
            }
            frame.friction_force += j.force;
            if i == middle {
                frame.bow_point_velocity = j.string_velocity;
                frame.state = j.state;
            }
        }
        self.from_bridge.push(reflected);

        let bridge_force = z * (arriving_at_bridge - reflected);
        if !bridge_force.is_finite() {
            debug_assert!(false, "string blew up");
            self.reset();
        }
        StringFrame {
            bridge_force,
            ..frame
        }
    }

    /// Solves the bow junction for a string of impedance `z` whose incoming waves
    /// alone would move the contact point at `v_h`.
    ///
    /// With compliant hair the hairs at the contact move at `v_b + ẋ`, where `x`
    /// is their deflection and `F = −(k·x + R·ẋ)` the force they pass to the
    /// string. Eliminating `ẋ` leaves the same load line as a rigid bow, with
    /// `v_h` shifted by `k·x/R` and the string admittance `1/(2Z)` raised by
    /// `1/R`. The deflection is integrated with backward Euler, which puts
    /// `k/fs` in series with `R`.
    fn solve_bow(
        junction: &mut BowJunction,
        hair: Option<BowHair>,
        deflection: &mut f32,
        sample_rate: f32,
        v_h: f32,
        bow: BowInput,
        z: f32,
    ) -> JunctionResult {
        let Some(hair) = hair else {
            return junction.solve(v_h, bow.velocity, bow.force, z);
        };
        if bow.force <= 0.0 {
            *deflection = 0.0;
            return junction.solve(v_h, bow.velocity, bow.force, z);
        }
        let r = hair.damping + hair.stiffness / sample_rate;
        let spring = hair.stiffness * *deflection;
        let z_eff = 0.5 / (0.5 / z + 1.0 / r);
        let j = junction.solve(v_h + spring / r, bow.velocity, bow.force, z_eff);
        *deflection -= (j.force + spring) / (r * sample_rate);
        JunctionResult {
            string_velocity: v_h + j.force / (2.0 * z),
            ..j
        }
    }

    fn update_delays(&mut self) {
        let omega = std::f32::consts::TAU * self.frequency / self.sample_rate;
        let note = self.note_design();
        self.loss_filter.set(&note.loss);
        let filter_delay = self.loss_filter.phase_delay(omega);
        let total = self.sample_rate / self.frequency - filter_delay;
        let max_bridge = self.to_bridge.max_delay();
        // The contacts are centered on the bow position.
        let width = ((self.contacts - 1) * self.gap) as f32;
        self.bridge_delay =
            (0.5 * (self.beta * total - width)).clamp(DelayLine::MIN_DELAY, max_bridge);
        self.dispersion.a = note.dispersion;
        let dispersion_delay = if self.stiff {
            DispersionAllpass::phase_lag(self.dispersion.a, omega) / omega
        } else {
            0.0
        };
        self.nut_delay = (total - 2.0 * self.bridge_delay - 2.0 * width - dispersion_delay)
            .clamp(DelayLine::MIN_DELAY, self.nut.max_delay());
        if let Some(t) = &mut self.torsion {
            t.update(
                self.frequency / self.open_frequency,
                self.beta,
                ((self.contacts - 1) * t.gap) as f32,
                self.sample_rate,
                &note.torsion,
            );
        }
        self.loop_gain = match self.loss {
            Loss::OnePole { t60, .. } => 10f32.powf(-3.0 / (t60 * self.frequency)),
            Loss::Measured(_) => (-note.loss.dc_loss).exp(),
        };
        self.bridge_share = 2.0 * self.bridge_delay / total;
        self.update_gains();
    }

    /// Spreads the contact points over the hair's width, whole samples apart:
    /// the most contacts (up to [`MAX_CONTACTS`]) whose span is within half a
    /// sample or 10% of the width. A width under half a sample is one contact.
    fn update_contacts(&mut self) {
        let width =
            self.hair.map_or(0.0, |h| h.width.max(0.0)) / self.wave_speed * self.sample_rate;
        let tolerance = (0.1 * width).max(0.5);
        let (contacts, gap) = (2..=MAX_CONTACTS)
            .rev()
            .map(|n| (n, (width / (n - 1) as f32).round() as usize))
            .find(|&(n, gap)| gap >= 1 && ((n - 1) as f32 * gap as f32 - width).abs() <= tolerance)
            .map_or((1, 0), |(n, gap)| (n, gap.min(MAX_GAP)));
        if (contacts, gap) != (self.contacts, self.gap) {
            self.contacts = contacts;
            self.gap = gap;
            for line in self.to_nut_gaps.iter_mut().chain(&mut self.to_bridge_gaps) {
                line.clear();
            }
            if let Some(t) = &mut self.torsion {
                // Torsional waves are faster: c_t / c = f_t / f0 on the same string.
                let ratio = self.open_frequency / t.spec.frequency;
                t.gap = ((gap as f32 * ratio).round() as usize).clamp(1, MAX_GAP);
                for line in t.to_nut_gaps.iter_mut().chain(&mut t.to_bridge_gaps) {
                    line.clear();
                }
            }
            self.apply_noise();
        }
        self.update_delays();
    }

    /// Splits the loop's DC loss between the reflections in proportion to the
    /// segment lengths, and adds the termination's own loss at the nut.
    fn update_gains(&mut self) {
        self.bridge_gain = self.loop_gain.powf(self.bridge_share);
        self.nut_gain =
            self.loop_gain.powf(1.0 - self.bridge_share) * (-self.termination_loss).exp();
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
        let table = &self.design.notes;
        if table.is_empty() {
            return NoteDesign {
                loss: Self::loss_design(&self.loss, self.frequency, self.sample_rate),
                dispersion: 0.0,
                torsion: NO_LOSS,
            };
        }
        let pos = (12.0 * (self.frequency / self.design.lowest_frequency).log2())
            .clamp(0.0, (table.len() - 1) as f32);
        let i = (pos as usize).min(table.len().saturating_sub(2));
        let t = pos - i as f32;
        match table.get(i + 1) {
            Some(next) => NoteDesign {
                loss: table[i].loss.lerp(&next.loss, t),
                dispersion: table[i].dispersion + t * (next.dispersion - table[i].dispersion),
                torsion: table[i].torsion.lerp(&next.torsion, t),
            },
            None => table[i],
        }
    }
}
