//! Bow–string friction junction.
//!
//! The bow sits at a scattering junction. Given `v_h` (the string velocity the
//! incoming waves would produce with no bow), the bow velocity `v_b` and the
//! normal force `F_b`, we find the friction force `F` such that
//!
//! ```text
//! v_s = v_h + F / (2Z)        string velocity at the bow
//! F   = F_b · μ(v_b − v_s)    hyperbolic friction curve
//! ```
//!
//! With the hyperbolic curve this reduces to a quadratic, so the solve is
//! closed-form and runs in constant time. Where the load line crosses the
//! friction curve more than once, the previous contact state picks the
//! solution (Friedlander's construction): stay stuck while the required force
//! is within static friction, stay slipping while a slip solution exists.
//!
//! With [`ThermalFriction`] the friction coefficient follows the rosin's
//! temperature at the contact instead of the slip speed (Woodhouse 2003). At a
//! given temperature the friction law is Coulomb's, so the load line meets it
//! exactly once; the hysteresis comes from the temperature lagging the heating.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrictionParams {
    /// Static friction coefficient.
    pub mu_s: f32,
    /// Dynamic (sliding) friction coefficient, approached at high slip speed.
    pub mu_d: f32,
    /// Slip speed (m/s) controlling how fast friction falls from `mu_s` to `mu_d`.
    pub v0: f32,
}

impl Default for FrictionParams {
    /// Rosin fit used by Smith & Woodhouse.
    fn default() -> Self {
        Self {
            mu_s: 0.8,
            mu_d: 0.3,
            v0: 0.1,
        }
    }
}

impl FrictionParams {
    /// Signed friction coefficient for relative velocity `dv = v_b − v_s`.
    pub fn coefficient(&self, dv: f32) -> f32 {
        let mag = self.mu_d + (self.mu_s - self.mu_d) * self.v0 / (self.v0 + dv.abs());
        mag.copysign(dv)
    }
}

/// Thermal properties of one side of the bow–string contact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThermalMaterial {
    /// Density (kg/m³).
    pub density: f32,
    /// Specific heat capacity (J/(kg·K)).
    pub heat_capacity: f32,
    /// Thermal conductivity (W/(m·K)).
    pub conductivity: f32,
}

impl ThermalMaterial {
    /// Thermal effusivity √(K·ρ·c) (W·s^½/(m²·K)): the heat flux into the
    /// surface is `A · effusivity · √s` times its temperature.
    fn effusivity(&self) -> f32 {
        (self.conductivity * self.density * self.heat_capacity).sqrt()
    }
}

/// Thermal friction (Smith & Woodhouse 2000; Woodhouse 2003): the friction
/// coefficient is a function of the contact temperature only, the rosin
/// softening as it warms. Heat `F·Δv` is generated in a thin rosin layer and
/// conducted into the string and the bow, carried off by the sheared rosin and
/// stored in the layer. The contact warms while the string slips and cools
/// while it sticks, so friction is high at the end of sticking and falls as
/// the slip heats the contact: a hysteresis loop instead of a friction curve.
///
/// The contact area grows with the normal force, so friction stays
/// proportional to it (Amontons). Replaces [`FrictionParams`] when set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThermalFriction {
    /// The string's surface (a string shielded by a layer of rosin).
    pub string: ThermalMaterial,
    /// The bow's surface, which moves through the contact at the bow speed.
    pub bow: ThermalMaterial,
    /// The rosin (only its density and heat capacity are used).
    pub rosin: ThermalMaterial,
    /// Thickness of the sheared rosin layer (m).
    pub layer: f32,
    /// Radius of the contact at 1 N of normal force (m).
    pub contact_radius: f32,
    /// Stretches the temperature axis of μ(T); 1 is Woodhouse's Figure 3.
    pub temperature_scale: f32,
    /// Stretches it further with the bow speed: by `(v_b / 0.05 m/s)^n`
    /// (Woodhouse's bow speed). Zero is his model, whose contact runs hotter
    /// with the bow speed until friction no longer falls.
    pub speed_exponent: f32,
}

impl ThermalFriction {
    /// Woodhouse (2003) Table I: a rosin-coated perspex rod on a cello string.
    pub const WOODHOUSE: Self = Self {
        string: ThermalMaterial {
            density: 3000.0,
            heat_capacity: 1000.0,
            conductivity: 1.4,
        },
        bow: ThermalMaterial {
            density: 600.0,
            heat_capacity: 1680.0,
            conductivity: 0.2,
        },
        rosin: ThermalMaterial {
            density: 1010.0,
            heat_capacity: 250.0,
            conductivity: 0.113,
        },
        layer: 1e-6,
        contact_radius: 250e-6,
        temperature_scale: 1.0,
        speed_exponent: 0.0,
    };
}

impl Default for ThermalFriction {
    fn default() -> Self {
        Self::WOODHOUSE
    }
}

/// Rosin's friction coefficient against temperature above ambient (K),
/// digitized from Woodhouse (2003) Figure 3, which is derived from the
/// steady-sliding fit `μ = 0.4·e^(−v/0.01) + 0.45·e^(−v/0.1) + 0.35`.
const FRICTION_BY_TEMPERATURE: [(f32, f32); 12] = [
    (0.0, 1.20),
    (5.0, 1.19),
    (10.0, 1.14),
    (15.0, 1.075),
    (20.0, 0.965),
    (25.0, 0.81),
    (30.0, 0.68),
    (35.0, 0.58),
    (40.0, 0.44),
    (45.0, 0.373),
    (50.0, 0.36),
    (60.0, 0.35),
];

/// The friction coefficient at `temperature` (K above ambient).
pub fn rosin_friction(temperature: f32) -> f32 {
    let table = &FRICTION_BY_TEMPERATURE;
    if temperature <= table[0].0 {
        return table[0].1;
    }
    for pair in table.windows(2) {
        let ((t0, m0), (t1, m1)) = (pair[0], pair[1]);
        if temperature < t1 {
            return m0 + (m1 - m0) * (temperature - t0) / (t1 - t0);
        }
    }
    table[table.len() - 1].1
}

/// Conduction modes per surface.
const THERMAL_MODES: usize = 8;

/// Heat conduction into a half-space through the contact: the flux is
/// `A · e · √(s + c) · T`, `e` the effusivity. With `c = 0` this is
/// one-dimensional diffusion into a stationary surface (Smith & Woodhouse's
/// Green's function); a moving surface, renewed at speed `v`, has
/// `c = 3v/(8a)`, which gives their steady-sliding flux (their Eq. 20).
///
/// The half-derivative is a sum of one-pole modes, `√s = (s/π)∫ξ^−½/(s + ξ) dξ`
/// on a log grid of ξ (one mode per decade, within a few percent of the
/// discrete Green's function below a few kHz). Each step holds the
/// temperature over the sample, and the flux is the step's average, which
/// gives Smith & Woodhouse's `g₀ = 2λ/√h`.
#[derive(Clone, Copy, Debug)]
struct Conduction {
    rate: [f32; THERMAL_MODES],
    weight: [f32; THERMAL_MODES],
    state: [f32; THERMAL_MODES],
    /// Per mode: state decay, state input, flux from the temperature and
    /// from the state, at the cached `c`.
    decay: [f32; THERMAL_MODES],
    input: [f32; THERMAL_MODES],
    from_temperature: [f32; THERMAL_MODES],
    from_state: [f32; THERMAL_MODES],
    /// Sum of `from_temperature`.
    conductance: f32,
    c: f32,
}

impl Conduction {
    fn new(sample_rate: f32) -> Self {
        let (low, high) = (0.3_f32, 30.0 * sample_rate);
        let step = (high / low).ln() / (THERMAL_MODES - 1) as f32;
        let rate: [f32; THERMAL_MODES] = std::array::from_fn(|k| low * (step * k as f32).exp());
        let mut conduction = Self {
            rate,
            weight: rate.map(|xi| xi.sqrt() * step / std::f32::consts::PI),
            state: [0.0; THERMAL_MODES],
            decay: [0.0; THERMAL_MODES],
            input: [0.0; THERMAL_MODES],
            from_temperature: [0.0; THERMAL_MODES],
            from_state: [0.0; THERMAL_MODES],
            conductance: 0.0,
            c: -1.0,
        };
        conduction.set_c(0.0, 1.0 / sample_rate);
        conduction
    }

    /// Sets `c` (1/s), recomputing the coefficients only when it has moved
    /// by more than 2%.
    fn set_c(&mut self, c: f32, h: f32) {
        if (c - self.c).abs() <= 0.02 * self.c {
            return;
        }
        self.c = c;
        self.conductance = 0.0;
        for k in 0..THERMAL_MODES {
            let (xi, w) = (self.rate[k], self.weight[k]);
            let a = xi + c;
            let e = (-a * h).exp();
            // The state's mean over the step, as a fraction of its move.
            let mean = (1.0 - e) / (a * h);
            self.decay[k] = e;
            self.input[k] = (1.0 - e) / a;
            self.from_temperature[k] = w * (1.0 - xi * (1.0 - mean) / a);
            self.from_state[k] = w * xi * mean;
            self.conductance += self.from_temperature[k];
        }
    }

    /// The flux (per unit area and effusivity) that the past temperatures
    /// hold back: the flux is `conductance · T − held()`.
    fn held(&self) -> f32 {
        (0..THERMAL_MODES)
            .map(|k| self.from_state[k] * self.state[k])
            .sum()
    }

    fn advance(&mut self, temperature: f32) {
        for k in 0..THERMAL_MODES {
            self.state[k] = self.decay[k] * self.state[k] + self.input[k] * temperature;
        }
    }

    fn reset(&mut self) {
        self.state = [0.0; THERMAL_MODES];
    }
}

/// A contact's temperature, for [`ThermalFriction`].
#[derive(Clone, Copy, Debug)]
struct ContactHeat {
    params: ThermalFriction,
    h: f32,
    string: Conduction,
    bow: Conduction,
    /// Contact temperature above ambient (K).
    temperature: f32,
    /// The last sample's heat input (W) and slip speed (m/s).
    heat: f32,
    slip: f32,
}

impl ContactHeat {
    fn new(params: ThermalFriction, sample_rate: f32) -> Self {
        Self {
            params,
            h: 1.0 / sample_rate,
            string: Conduction::new(sample_rate),
            bow: Conduction::new(sample_rate),
            temperature: 0.0,
            heat: 0.0,
            slip: 0.0,
        }
    }

    fn reset(&mut self) {
        self.string.reset();
        self.bow.reset();
        self.temperature = 0.0;
        self.heat = 0.0;
        self.slip = 0.0;
    }

    /// Advances the temperature by one sample under the last sample's heat
    /// input, and returns the friction coefficient. The heat balance
    /// (Woodhouse's Eq. 1) is solved implicitly: at audio rates conduction
    /// is far stiffer than the layer's heat capacity.
    fn step(&mut self, normal_force: f32, bow_speed: f32) -> f32 {
        let p = &self.params;
        let area_per_newton = std::f32::consts::PI * p.contact_radius * p.contact_radius;
        let area = area_per_newton * normal_force;
        let radius = p.contact_radius * normal_force.sqrt();
        let rosin = p.rosin.density * p.rosin.heat_capacity;
        let storage = rosin * area * p.layer / self.h;
        let carried = self.slip * radius * p.layer * rosin;
        self.bow
            .set_c(3.0 * bow_speed.abs() / (8.0 * radius.max(1e-6)), self.h);
        let (string, bow) = (area * p.string.effusivity(), area * p.bow.effusivity());
        let temperature = (self.heat
            + string * self.string.held()
            + bow * self.bow.held()
            + storage * self.temperature)
            / (string * self.string.conductance + bow * self.bow.conductance + carried + storage);
        self.temperature = temperature.max(0.0);
        self.string.advance(self.temperature);
        self.bow.advance(self.temperature);
        let scale = if p.speed_exponent == 0.0 {
            p.temperature_scale
        } else {
            p.temperature_scale * (bow_speed.abs().max(0.01) / 0.05).powf(p.speed_exponent)
        };
        rosin_friction(self.temperature / scale)
    }
}

/// Bow noise: the sliding friction's random fluctuation while the string
/// slips (rosin and hair roughness). Sticking hair moves with the string and
/// adds none, so the noise comes in pulses at each slip (Chafe 1990), and
/// more of it at attacks, where the string slips longer and irregularly.
///
/// The fluctuation multiplies the friction force: `F · (1 + level · n)`, with
/// `n` band-limited noise of unit RMS. It is inside the loop, so the string and
/// the body shape it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BowNoise {
    /// RMS of the relative fluctuation (dimensionless). Zero is no noise.
    pub level: f32,
    /// Bandwidth (Hz): a one-pole lowpass on white noise. The noise's density
    /// below it doesn't depend on the sample rate.
    pub cutoff: f32,
}

/// Band-limited noise of unit RMS, for [`BowNoise`].
#[derive(Clone, Copy, Debug)]
struct NoiseSource {
    level: f32,
    /// One-pole lowpass coefficient, and the gain that keeps its output at unit RMS.
    pole: f32,
    gain: f32,
    state: f32,
    rng: u32,
}

impl NoiseSource {
    fn new(noise: BowNoise, sample_rate: f32, seed: u32) -> Self {
        let mut source = Self {
            level: 0.0,
            pole: 0.0,
            gain: 0.0,
            state: 0.0,
            rng: 1,
        };
        source.set(noise, sample_rate);
        source.reseed(seed);
        source
    }

    fn set(&mut self, noise: BowNoise, sample_rate: f32) {
        self.level = noise.level.max(0.0);
        self.pole = if noise.cutoff > 0.0 && noise.cutoff < 0.5 * sample_rate {
            (-std::f32::consts::TAU * noise.cutoff / sample_rate).exp()
        } else {
            0.0
        };
        // Uniform white noise in ±1 has a variance of 1/3; the lowpass passes
        // (1 − a)/(1 + a) of it.
        self.gain = (3.0 * (1.0 + self.pole) / (1.0 - self.pole)).sqrt();
    }

    fn reseed(&mut self, seed: u32) {
        // Murmur3's finalizer, so neighbouring seeds draw unrelated noise.
        let mut x = seed;
        x ^= x >> 16;
        x = x.wrapping_mul(0x85eb_ca6b);
        x ^= x >> 13;
        x = x.wrapping_mul(0xc2b2_ae35);
        x ^= x >> 16;
        self.rng = x.max(1);
        self.state = 0.0;
    }

    /// The next sample of `level · n`.
    fn next(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let white = (self.rng >> 8) as f32 * (2.0 / (1u32 << 24) as f32) - 1.0;
        self.state = white + self.pole * (self.state - white);
        self.level * self.gain * self.state
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContactState {
    /// Bow not touching the string.
    #[default]
    Off,
    /// String moves with the bow.
    Stick,
    /// String slides under the bow; `positive` means the bow is faster (`v_b > v_s`).
    Slip { positive: bool },
}

impl ContactState {
    pub fn is_slipping(self) -> bool {
        matches!(self, ContactState::Slip { .. })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct JunctionResult {
    /// Friction force the bow applies to the string.
    pub force: f32,
    /// String velocity at the bow point after the force is applied.
    pub string_velocity: f32,
    pub state: ContactState,
}

#[derive(Clone)]
pub struct BowJunction {
    pub friction: FrictionParams,
    state: ContactState,
    noise: NoiseSource,
    thermal: Option<ContactHeat>,
}

impl BowJunction {
    pub fn new(friction: FrictionParams) -> Self {
        Self {
            friction,
            state: ContactState::Off,
            noise: NoiseSource::new(BowNoise::default(), 48_000.0, 0),
            thermal: None,
        }
    }

    /// Sets thermal friction, at the rate `solve` is called at. `None` (the
    /// default) uses the friction curve.
    pub fn set_thermal(&mut self, thermal: Option<ThermalFriction>, sample_rate: f32) {
        let current = self.thermal.map(|t| (t.params, t.h));
        if current != thermal.map(|t| (t, 1.0 / sample_rate)) {
            self.thermal = thermal.map(|t| ContactHeat::new(t, sample_rate));
        }
    }

    /// The contact's temperature above ambient (K), with thermal friction.
    pub fn temperature(&self) -> Option<f32> {
        self.thermal.map(|t| t.temperature)
    }

    /// Sets the bow noise, at the rate `solve` is called at.
    pub fn set_noise(&mut self, noise: BowNoise, sample_rate: f32) {
        self.noise.set(noise, sample_rate);
    }

    /// Restarts the noise from `seed`: junctions with different seeds draw
    /// unrelated noise.
    pub fn reseed_noise(&mut self, seed: u32) {
        self.noise.reseed(seed);
    }

    pub fn state(&self) -> ContactState {
        self.state
    }

    pub fn reset(&mut self) {
        self.state = ContactState::Off;
        if let Some(t) = &mut self.thermal {
            t.reset();
        }
    }

    /// Solves the junction for one sample. `z` is the string's wave impedance.
    pub fn solve(&mut self, v_h: f32, v_b: f32, f_b: f32, z: f32) -> JunctionResult {
        if f_b <= 0.0 {
            // The contact is gone; the next one starts on cool rosin.
            if self.state != ContactState::Off {
                self.reset();
            }
            self.state = ContactState::Off;
            return JunctionResult {
                force: 0.0,
                string_velocity: v_h,
                state: self.state,
            };
        }

        let d = v_b - v_h;
        let k = f_b / (2.0 * z);
        if let Some(heat) = &mut self.thermal {
            let mu = heat.step(f_b, v_b);
            // Coulomb friction at this temperature: stick within it, otherwise
            // slip at it in the direction the load line pulls.
            let dv = if d.abs() <= k * mu {
                0.0
            } else {
                d - (k * mu).copysign(d)
            };
            let state = if dv == 0.0 {
                ContactState::Stick
            } else {
                ContactState::Slip { positive: d > 0.0 }
            };
            let result = self.finish(v_b, d, dv, state, z);
            let heat = self.thermal.as_mut().unwrap();
            heat.heat = (result.force * (v_b - result.string_velocity)).abs();
            heat.slip = (v_b - result.string_velocity).abs();
            return result;
        }
        let can_stick = d.abs() <= k * self.friction.mu_s;

        let slip = match self.state {
            ContactState::Off | ContactState::Stick if can_stick => None,
            // Breaking away from stick: the load line starts above static friction,
            // so the slip root on the side of `d` exists and is unique.
            ContactState::Off | ContactState::Stick => {
                self.slip_root(d > 0.0, d, k).map(|x| (d > 0.0, x))
            }
            ContactState::Slip { positive } => match self.slip_root(positive, d, k) {
                Some(x) => Some((positive, x)),
                None if can_stick => None,
                None => self.slip_root(!positive, d, k).map(|x| (!positive, x)),
            },
        };

        let (dv, state) = match slip {
            Some((positive, x)) => (
                x.copysign(if positive { 1.0 } else { -1.0 }),
                ContactState::Slip { positive },
            ),
            // Also the fallback if no slip root was found; unreachable in exact arithmetic.
            None => (0.0, ContactState::Stick),
        };
        self.finish(v_b, d, dv, state, z)
    }

    /// The force and string velocity for a slip `dv` on the load line, with
    /// the bow noise.
    fn finish(&mut self, v_b: f32, d: f32, dv: f32, state: ContactState, z: f32) -> JunctionResult {
        self.state = state;
        let force = 2.0 * z * (d - dv);
        let noise = if state.is_slipping() && self.noise.level > 0.0 {
            force * self.noise.next()
        } else {
            0.0
        };
        JunctionResult {
            force: force + noise,
            // On the load line: the noise moves the string as any force does.
            string_velocity: v_b - dv + noise / (2.0 * z),
            state,
        }
    }

    /// Slip speed `|Δv|` on the given side, or `None` if the load line misses the
    /// friction curve there. With two crossings it returns the larger one: the
    /// smaller is the unstable middle intersection.
    fn slip_root(&self, positive: bool, d: f32, k: f32) -> Option<f32> {
        let FrictionParams { mu_s, mu_d, v0 } = self.friction;
        let d = if positive { d } else { -d };
        // x² + b·x + c = 0 for x = |Δv| ≥ 0.
        let b = v0 + k * mu_d - d;
        let c = v0 * (k * mu_s - d);
        let disc = b * b - 4.0 * c;
        if disc < 0.0 {
            return None;
        }
        // Numerically stable roots: q and c/q.
        let q = -0.5 * (b + disc.sqrt().copysign(b));
        let larger = if q == 0.0 { 0.0 } else { q.max(c / q) };
        (larger >= 0.0).then_some(larger)
    }
}

/// Schelleng's bow-force limits `(F_min, F_max)` for Helmholtz motion.
///
/// `loop_gain` is the string's DC gain per period. Treating all of that loss as
/// a resistive bridge gives the equivalent resistance `R = Z·(1+g)/(1−g)`. Treat the
/// result as a guide: the loop lowpass adds loss the formula doesn't see.
pub fn schelleng_limits(
    z: f32,
    loop_gain: f32,
    friction: &FrictionParams,
    beta: f32,
    v_b: f32,
) -> (f32, f32) {
    let dmu = friction.mu_s - friction.mu_d;
    let r = z * (1.0 + loop_gain) / (1.0 - loop_gain);
    let v = v_b.abs();
    let f_max = 2.0 * z * v / (dmu * beta);
    let f_min = z * z * v / (2.0 * r * beta * beta * dmu);
    (f_min, f_max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const Z: f32 = 0.175;

    fn junction() -> BowJunction {
        BowJunction::new(FrictionParams::default())
    }

    #[test]
    fn lifted_bow_applies_no_force() {
        let r = junction().solve(0.3, 0.1, 0.0, Z);
        assert_eq!(r.force, 0.0);
        assert_eq!(r.string_velocity, 0.3);
        assert_eq!(r.state, ContactState::Off);
    }

    #[test]
    fn sticks_within_static_friction() {
        let mut j = junction();
        let r = j.solve(0.05, 0.1, 0.5, Z);
        assert_eq!(r.state, ContactState::Stick);
        assert_eq!(r.string_velocity, 0.1);
        assert!(r.force.abs() <= 0.8 * 0.5);
    }

    #[test]
    fn slip_solution_lies_on_friction_curve() {
        let mut j = junction();
        let f_b = 0.2;
        for v_h in [-0.8, -0.5, 0.7, 1.0] {
            j.reset();
            let r = j.solve(v_h, 0.1, f_b, Z);
            assert!(r.state.is_slipping(), "v_h {v_h}");
            let dv = 0.1 - r.string_velocity;
            let expected = f_b * j.friction.coefficient(dv);
            assert!(
                (r.force - expected).abs() < 1e-4,
                "v_h {v_h}: {} vs {expected}",
                r.force
            );
            // Consistent with the load line.
            assert!((r.string_velocity - (v_h + r.force / (2.0 * Z))).abs() < 1e-5);
        }
    }

    #[test]
    fn hysteresis_keeps_slipping_where_stick_would_also_hold() {
        let f_b = 0.2;
        let k = f_b / (2.0 * Z);
        // |d| just inside the static limit: a stuck string stays stuck...
        let v_h = 0.1 - 0.95 * k * 0.8;
        let mut stuck = junction();
        stuck.solve(0.1, 0.1, f_b, Z);
        assert_eq!(stuck.solve(v_h, 0.1, f_b, Z).state, ContactState::Stick);
        // ...but a slipping string keeps slipping if a slip solution exists.
        let mut slipping = junction();
        slipping.solve(-1.0, 0.1, f_b, Z);
        let r = slipping.solve(v_h, 0.1, f_b, Z);
        assert_eq!(r.state, ContactState::Slip { positive: true });
    }

    #[test]
    fn noise_has_unit_rms_at_any_sample_rate() {
        for fs in [48_000.0, 96_000.0] {
            let noise = BowNoise {
                level: 1.0,
                cutoff: 2000.0,
            };
            let mut source = NoiseSource::new(noise, fs, 1);
            let n = 400_000;
            let power = (0..n).map(|_| source.next().powi(2)).sum::<f32>() / n as f32;
            assert!(
                (power.sqrt() - 1.0).abs() < 0.03,
                "fs {fs}: rms {}",
                power.sqrt()
            );
        }
    }

    #[test]
    fn noise_moves_the_force_only_while_slipping() {
        let noise = BowNoise {
            level: 0.1,
            cutoff: 2000.0,
        };
        let (mut clean, mut noisy) = (junction(), junction());
        noisy.set_noise(noise, 48_000.0);
        // Sticking: the same force.
        let a = clean.solve(0.05, 0.1, 0.5, Z);
        let b = noisy.solve(0.05, 0.1, 0.5, Z);
        assert_eq!(b.state, ContactState::Stick);
        assert_eq!(a.force, b.force);
        // Slipping: off the friction curve, but on the load line.
        let (v_h, f_b) = (-0.8, 0.2);
        let a = clean.solve(v_h, 0.1, f_b, Z);
        let b = noisy.solve(v_h, 0.1, f_b, Z);
        assert!(b.state.is_slipping());
        assert_ne!(a.force, b.force);
        assert!((b.force / a.force - 1.0).abs() < 0.5);
        assert!((b.string_velocity - (v_h + b.force / (2.0 * Z))).abs() < 1e-5);
    }

    #[test]
    fn rosin_friction_falls_with_temperature() {
        assert_eq!(rosin_friction(-5.0), 1.2);
        assert_eq!(rosin_friction(100.0), 0.35);
        assert!((rosin_friction(27.5) - 0.745).abs() < 1e-6);
        let mut last = f32::INFINITY;
        for t in 0..80 {
            let mu = rosin_friction(t as f32);
            assert!(mu <= last);
            last = mu;
        }
    }

    /// The flux of a stationary surface, `√s·T`, against Smith & Woodhouse's
    /// discrete Green's function (their Eq. 13, with λ = 1/√π).
    #[test]
    fn conduction_matches_the_discrete_greens_function() {
        let fs = 96_000.0;
        let h = 1.0 / fs;
        let n = 16_000;
        let lambda = 1.0 / std::f64::consts::PI.sqrt();
        let g: Vec<f64> = (0..n)
            .map(|k| {
                let k = k as f64;
                let scale = 2.0 * lambda / (h as f64).sqrt();
                if k == 0.0 {
                    scale
                } else {
                    scale * ((k + 1.0).sqrt() - 2.0 * k.sqrt() + (k - 1.0).sqrt())
                }
            })
            .collect();
        for frequency in [100.0, 1000.0] {
            let t: Vec<f64> = (0..n)
                .map(|i| (std::f64::consts::TAU * frequency * i as f64 * h as f64).sin())
                .collect();
            let mut conduction = Conduction::new(fs);
            let (mut error, mut power) = (0.0, 0.0);
            for i in 0..n {
                let flux = conduction.conductance * t[i] as f32 - conduction.held();
                conduction.advance(t[i] as f32);
                let exact: f64 = (0..=i).map(|k| g[k] * t[i - k]).sum();
                if i >= n / 2 {
                    error += (flux as f64 - exact).powi(2);
                    power += exact * exact;
                }
            }
            let relative = (error / power).sqrt();
            assert!(relative < 0.05, "{frequency} Hz: {relative}");
        }
    }

    /// A moving surface held at a steady temperature conducts `√c·T`, Smith
    /// & Woodhouse's steady-sliding flux.
    #[test]
    fn moving_surface_reaches_the_steady_sliding_flux() {
        let fs = 96_000.0;
        let c = 3.0 * 0.1 / (8.0 * 250e-6);
        let mut conduction = Conduction::new(fs);
        conduction.set_c(c, 1.0 / fs);
        let mut flux = 0.0;
        for _ in 0..fs as usize {
            flux = conduction.conductance - conduction.held();
            conduction.advance(1.0);
        }
        assert!(
            (flux / c.sqrt() - 1.0).abs() < 0.03,
            "{flux} vs {}",
            c.sqrt()
        );
    }

    #[test]
    fn thermal_friction_heats_while_slipping_and_cools_while_sticking() {
        let fs = 96_000.0;
        let mut j = junction();
        j.set_thermal(Some(ThermalFriction::default()), fs);
        let (v_b, f_b) = (0.1, 0.5);
        // Pulled far off the bow: the string slips, at μ(T)·F_b.
        let mut hottest = 0.0_f32;
        for _ in 0..200 {
            let before = rosin_friction(j.temperature().unwrap());
            let r = j.solve(-3.0, v_b, f_b, Z);
            assert!(r.state.is_slipping());
            // The coefficient is set before the step's heat.
            assert!(r.force <= before * f_b + 1e-6);
            hottest = hottest.max(j.temperature().unwrap());
        }
        assert!(hottest > 10.0, "{hottest} K");
        // Moving with the bow: it sticks and cools.
        for _ in 0..2000 {
            assert_eq!(j.solve(v_b, v_b, f_b, Z).state, ContactState::Stick);
        }
        let cooled = j.temperature().unwrap();
        assert!(cooled < 0.5 * hottest, "{cooled} K after {hottest} K");
        // Lifting the bow forgets the heat.
        j.solve(0.0, v_b, 0.0, Z);
        assert_eq!(j.temperature(), Some(0.0));
    }

    #[test]
    fn slipping_recaptures_when_no_slip_solution() {
        let mut j = junction();
        j.solve(-1.0, 0.1, 0.2, Z);
        // String nearly matches the bow: no slip crossing, so it sticks.
        let r = j.solve(0.099, 0.1, 0.2, Z);
        assert_eq!(r.state, ContactState::Stick);
    }
}
