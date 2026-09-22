//! Small filters used inside the waveguide loop.

/// One-pole lowpass with unity DC gain: `y = (1 - a)·x + a·y[n-1]`.
#[derive(Clone, Copy, Default)]
pub struct OnePoleLowpass {
    pub a: f32,
    z: f32,
}

impl OnePoleLowpass {
    pub fn new(a: f32) -> Self {
        Self { a, z: 0.0 }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        self.z = (1.0 - self.a) * x + self.a * self.z;
        self.z
    }

    pub fn reset(&mut self) {
        self.z = 0.0;
    }

    /// Phase delay in samples at `omega` (radians per sample).
    pub fn phase_delay(a: f32, omega: f32) -> f32 {
        let (s, c) = omega.sin_cos();
        (a * s).atan2(1.0 - a * c) / omega
    }
}

/// A cascade of identical first-order allpasses, `H(z) = ((a + z⁻¹) / (1 + a·z⁻¹))^M`,
/// for string stiffness (Van Duyne & Smith 1994; Rauhala & Välimäki 2006).
///
/// With `a < 0` its delay falls with frequency, so high partials come round the
/// loop sooner and end up sharp, as on a stiff string. `a = 0` is a pure delay
/// of `M` samples.
pub struct DispersionAllpass {
    pub a: f32,
    state: [f32; Self::SECTIONS],
}

impl DispersionAllpass {
    /// Sections in the cascade. Identical sections follow the stiff-string curve
    /// only up to a bandwidth that grows as `M^(1/3)`; 16 track a cello string to
    /// within about 1.5 cents up to 2.5 kHz.
    pub const SECTIONS: usize = 16;
    /// Upper frequency (Hz) of the partials the coefficient is fitted to.
    pub const FIT_HZ: f32 = 3500.0;

    pub fn new() -> Self {
        Self {
            a: 0.0,
            state: [0.0; Self::SECTIONS],
        }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let a = self.a;
        self.state.iter_mut().fold(x, |x, s| {
            // Transposed direct form: one state per section.
            let y = a * x + *s;
            *s = x - a * y;
            y
        })
    }

    pub fn reset(&mut self) {
        self.state = [0.0; Self::SECTIONS];
    }

    /// Phase lag of the whole cascade in radians at `omega` (radians per sample).
    pub fn phase_lag(a: f32, omega: f32) -> f32 {
        let (s, c) = omega.sin_cos();
        Self::SECTIONS as f32 * (omega - 2.0 * (a * s).atan2(1.0 + a * c))
    }

    /// Fits the coefficient so that a loop of period `sample_rate / f0` samples,
    /// closed through this cascade and a loss filter with phase lag
    /// `loss_lag(ω)` (radians),
    /// has the partials of a stiff string with inharmonicity `b`:
    /// `f_n = n·f0·sqrt((1 + b·n²) / (1 + b))` (so `f_1 = f0`).
    ///
    /// Minimizes the squared cents error of the partials up to [`Self::FIT_HZ`]
    /// by golden-section search over `a ∈ [-0.99, 0]`. Allocation-free, but a few
    /// thousand trig calls: call it on note changes, not every sample.
    pub fn design(f0: f32, sample_rate: f32, b: f32, loss_lag: impl Fn(f64) -> f64) -> f32 {
        if b <= 0.0 {
            return 0.0;
        }
        let (f0, fs, b) = (f0 as f64, sample_rate as f64, b as f64);
        let partials = ((Self::FIT_HZ as f64 / f0) as usize).clamp(4, 64);
        let omega =
            |n: f64| std::f64::consts::TAU * n * f0 * ((1.0 + b * n * n) / (1.0 + b)).sqrt() / fs;
        let lag = |a: f64, w: f64| {
            let (s, c) = w.sin_cos();
            loss_lag(w) + Self::SECTIONS as f64 * (w - 2.0 * (a * s).atan2(1.0 + a * c))
        };
        let cost = |a: f64| {
            // The pure delay that puts the fundamental exactly in tune...
            let w1 = omega(1.0);
            let delay = (std::f64::consts::TAU - lag(a, w1)) / w1;
            // ...and how far each higher partial's loop phase is from n·2π, relative
            // to n·2π (proportional to its error in cents).
            (2..=partials)
                .map(|n| n as f64)
                .take_while(|&n| omega(n) < std::f64::consts::PI)
                .map(|n| {
                    let w = omega(n);
                    let err = (w * delay + lag(a, w) - std::f64::consts::TAU * n)
                        / (std::f64::consts::TAU * n);
                    err * err
                })
                .sum::<f64>()
        };
        let ratio = 0.5 * (5f64.sqrt() - 1.0);
        let (mut lo, mut hi) = (-0.99, 0.0);
        let mut x1 = hi - ratio * (hi - lo);
        let mut x2 = lo + ratio * (hi - lo);
        let (mut c1, mut c2) = (cost(x1), cost(x2));
        for _ in 0..40 {
            if c1 < c2 {
                hi = x2;
                (x2, c2) = (x1, c1);
                x1 = hi - ratio * (hi - lo);
                c1 = cost(x1);
            } else {
                lo = x1;
                (x1, c1) = (x2, c2);
                x2 = lo + ratio * (hi - lo);
                c2 = cost(x2);
            }
        }
        (0.5 * (lo + hi)) as f32
    }
}

impl Default for DispersionAllpass {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase lag of `process` at `omega`, by quadrature projection over a steady tone.
    fn measured_lag(mut process: impl FnMut(f32) -> f32, omega: f32) -> f32 {
        let n = 20_000;
        let (mut re, mut im) = (0.0_f64, 0.0_f64);
        for i in 0..n {
            let ph = (omega * i as f32) as f64;
            let y = process(ph.sin() as f32);
            if i >= n / 2 {
                re += y as f64 * ph.sin();
                im += y as f64 * ph.cos();
            }
        }
        -(im.atan2(re)) as f32
    }

    #[test]
    fn dispersion_phase_lag_matches_measured() {
        let a = -0.6;
        // An exact number of periods; the lag wraps, so compare modulo 2π.
        let omega = std::f32::consts::TAU / 50.0;
        let mut ap = DispersionAllpass::new();
        ap.a = a;
        let lag = measured_lag(|x| ap.process(x), omega);
        let expected = DispersionAllpass::phase_lag(a, omega);
        let diff = (lag - expected).rem_euclid(std::f32::consts::TAU);
        let diff = diff.min(std::f32::consts::TAU - diff);
        assert!(diff < 1e-3, "measured {lag}, expected {expected} (mod 2π)");
    }

    #[test]
    fn dispersion_design_follows_stiffness() {
        // No stiffness: no dispersion.
        assert_eq!(DispersionAllpass::design(98.0, 48_000.0, 0.0, |_| 0.0), 0.0);
        // More stiffness needs a more negative coefficient.
        let soft = DispersionAllpass::design(98.0, 48_000.0, 1e-5, |_| 0.0);
        let stiff = DispersionAllpass::design(98.0, 48_000.0, 4e-5, |_| 0.0);
        assert!(stiff < soft && soft < 0.0, "{stiff} {soft}");
    }

    #[test]
    fn phase_delay_matches_measured_sine_lag() {
        let a = 0.3;
        // An exact number of periods in the measurement span avoids leakage.
        let omega = std::f32::consts::TAU / 100.0;
        let mut lp = OnePoleLowpass::new(a);
        let lag = measured_lag(|x| lp.process(x), omega) / omega;
        let expected = OnePoleLowpass::phase_delay(a, omega);
        assert!(
            (lag - expected).abs() < 0.01,
            "measured {lag}, expected {expected}"
        );
    }
}
