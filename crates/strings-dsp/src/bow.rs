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

pub struct BowJunction {
    pub friction: FrictionParams,
    state: ContactState,
}

impl BowJunction {
    pub fn new(friction: FrictionParams) -> Self {
        Self {
            friction,
            state: ContactState::Off,
        }
    }

    pub fn state(&self) -> ContactState {
        self.state
    }

    pub fn reset(&mut self) {
        self.state = ContactState::Off;
    }

    /// Solves the junction for one sample. `z` is the string's wave impedance.
    pub fn solve(&mut self, v_h: f32, v_b: f32, f_b: f32, z: f32) -> JunctionResult {
        if f_b <= 0.0 {
            self.state = ContactState::Off;
            return JunctionResult {
                force: 0.0,
                string_velocity: v_h,
                state: self.state,
            };
        }

        let d = v_b - v_h;
        let k = f_b / (2.0 * z);
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
        self.state = state;
        JunctionResult {
            force: 2.0 * z * (d - dv),
            string_velocity: v_b - dv,
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
    fn slipping_recaptures_when_no_slip_solution() {
        let mut j = junction();
        j.solve(-1.0, 0.1, 0.2, Z);
        // String nearly matches the bow: no slip crossing, so it sticks.
        let r = j.solve(0.099, 0.1, 0.2, Z);
        assert_eq!(r.state, ContactState::Stick);
    }
}
