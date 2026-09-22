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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_delay_matches_measured_sine_lag() {
        let a = 0.3;
        // An exact number of periods in the measurement span avoids leakage.
        let omega = std::f32::consts::TAU / 100.0;
        let mut lp = OnePoleLowpass::new(a);
        // Run to steady state, then find the peak lag of the output behind the input.
        let n = 20_000;
        let mut out = vec![0.0; n];
        for (i, y) in out.iter_mut().enumerate() {
            *y = lp.process((omega * i as f32).sin());
        }
        // Compare the output's phase at the end with the input's via quadrature projection.
        let (mut re, mut im) = (0.0_f64, 0.0_f64);
        for (i, &y) in out.iter().enumerate().skip(n / 2) {
            let ph = (omega * i as f32) as f64;
            re += y as f64 * ph.sin();
            im += y as f64 * ph.cos();
        }
        let lag = -(im.atan2(re)) as f32 / omega;
        let expected = OnePoleLowpass::phase_delay(a, omega);
        assert!(
            (lag - expected).abs() < 0.01,
            "measured {lag}, expected {expected}"
        );
    }
}
