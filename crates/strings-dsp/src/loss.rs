//! Loss in the string loop: how much each partial loses per period.
//!
//! Two models:
//! - [`Loss::OnePole`]: a decay time for the fundamental plus a one-pole lowpass.
//!   Its loss per period rises as ω², so its damping ratio ζ rises only as ω.
//! - [`Loss::Measured`]: a damping curve ζ(f) fitted to pluck measurements. A
//!   real string's ζ is flat at low frequencies and then rises steeply (as ω³
//!   to ω⁴ for the cello string in Lampis et al. 2025, Fig. 1). A one-pole can't
//!   follow that; a one-pole times a second-order Butterworth lowpass can.
//!
//! Both are passive by construction: no partial gains energy.

use crate::filters::OnePoleLowpass;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Loss {
    OnePole {
        /// Decay time to −60 dB of the fundamental on the open string (s).
        t60: f32,
        /// Pole of the one-pole lowpass in the bridge reflection, as it would be
        /// at 48 kHz (converted for other rates). Higher is darker.
        lowpass: f32,
    },
    Measured(DampingCurve),
}

/// Damping ratio of the string's modes as a function of frequency:
/// `ζ(f) = floor + at_1khz · (f / 1 kHz)^exponent`.
///
/// It describes the free string, so it holds for stopped notes too, but only
/// within the frequency range it was fitted over; above that it extrapolates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DampingCurve {
    pub floor: f32,
    pub at_1khz: f32,
    pub exponent: f32,
}

impl DampingCurve {
    pub fn zeta(&self, frequency: f32) -> f32 {
        self.floor + self.at_1khz * (frequency / 1000.0).powf(self.exponent)
    }

    /// Fits a [`LossDesign`] to this curve for a loop tuned to `f0`.
    ///
    /// The target is the loss per period of each partial, `2π·n·ζ(n·f0)`
    /// nepers, capped at [`Self::CAP`]: a partial that loses that much dies
    /// within a period or two, and the exact value no longer matters. The fit
    /// minimizes the relative error of the loss over partials up to 8 kHz with a
    /// coarse grid over (pole, cutoff) refined twice; the DC loss follows in
    /// closed form. It allocates and takes about 0.1 ms: run it in constructors.
    pub fn design(&self, f0: f32, sample_rate: f32) -> LossDesign {
        let (f0, fs) = (f0 as f64, sample_rate as f64);
        let partials = ((8000.0f64.min(0.45 * fs) / f0) as usize).clamp(1, 128);
        let omegas: Vec<f64> = (1..=partials)
            .map(|n| std::f64::consts::TAU * n as f64 * f0 / fs)
            .collect();
        let targets: Vec<f64> = (1..=partials)
            .map(|n| {
                let zeta = self.zeta((n as f64 * f0) as f32) as f64;
                (std::f64::consts::TAU * n as f64 * zeta).min(Self::CAP)
            })
            .collect();
        let weights: Vec<f64> = targets.iter().map(|t| 1.0 / (t + 0.005)).collect();
        let weight_sum: f64 = weights.iter().map(|w| w * w).sum();
        // Weighted least-squares error of (DC loss, pole, cutoff), given the
        // losses of the one-pole and the Butterworth at each partial. The DC
        // loss is the weighted mean residual, clamped to be non-negative.
        let evaluate = |one_pole: &[f64], butterworth: &[f64]| {
            let shaped = |i: usize| one_pole[i] + butterworth[i];
            let residual: f64 = (0..partials)
                .map(|i| weights[i].powi(2) * (targets[i] - shaped(i)))
                .sum();
            let dc = (residual / weight_sum).max(0.0);
            let err: f64 = (0..partials)
                .map(|i| (weights[i] * ((dc + shaped(i)).min(Self::CAP) - targets[i])).powi(2))
                .sum();
            (err, dc)
        };

        let (lo_cut, hi_cut) = (300f64.ln(), (0.4 * fs).ln());
        let (mut best_pole, mut best_cut) = (0.0, hi_cut);
        let (mut best_err, mut best_dc) = (f64::MAX, 0.0);
        let (mut pole_span, mut cut_span) = (0.9, hi_cut - lo_cut);
        let (mut pole_center, mut cut_center) = (0.45, 0.5 * (lo_cut + hi_cut));
        for (steps, shrink) in [(24, 0.2), (8, 0.25), (8, 0.25)] {
            let grid = |center: f64, span: f64, lo: f64, hi: f64| -> Vec<f64> {
                (0..=steps)
                    .map(|i| (center + span * (i as f64 / steps as f64 - 0.5)).clamp(lo, hi))
                    .collect()
            };
            let poles = grid(pole_center, pole_span, 0.0, 0.95);
            let cuts = grid(cut_center, cut_span, lo_cut, hi_cut);
            // The one-pole's loss depends only on the pole, the Butterworth's only
            // on the cutoff: compute each once per grid line.
            let pole_losses: Vec<Vec<f64>> = poles
                .iter()
                .map(|&a| {
                    omegas
                        .iter()
                        .map(|&w| LossFilter::one_pole_loss(a, w))
                        .collect()
                })
                .collect();
            let cut_losses: Vec<Vec<f64>> = cuts
                .iter()
                .map(|&c| {
                    let b = Butterworth::new(c.exp(), fs);
                    omegas.iter().map(|&w| b.loss(w)).collect()
                })
                .collect();
            for (&pole, pl) in poles.iter().zip(&pole_losses) {
                for (&cut, cl) in cuts.iter().zip(&cut_losses) {
                    let (err, dc) = evaluate(pl, cl);
                    if err < best_err {
                        (best_err, best_dc, best_pole, best_cut) = (err, dc, pole, cut);
                    }
                }
            }
            (pole_center, cut_center) = (best_pole, best_cut);
            pole_span *= shrink;
            cut_span *= shrink;
        }
        LossDesign {
            dc_loss: best_dc as f32,
            pole: best_pole as f32,
            cutoff: best_cut.exp() as f32,
        }
    }

    /// Largest loss per period (nepers) the design aims for.
    const CAP: f64 = 3.0;
}

/// Parameters of the loop loss for one pitch at one sample rate.
#[derive(Clone, Copy, Debug)]
pub struct LossDesign {
    /// Loss of the whole loop at DC, per period (nepers).
    pub dc_loss: f32,
    /// Pole of the one-pole lowpass (at the actual sample rate).
    pub pole: f32,
    /// Cutoff of the Butterworth lowpass (Hz); none for the one-pole model.
    pub cutoff: f32,
}

impl LossDesign {
    pub fn lerp(&self, other: &LossDesign, t: f32) -> LossDesign {
        let mix = |a: f32, b: f32| a + t * (b - a);
        LossDesign {
            dc_loss: mix(self.dc_loss, other.dc_loss),
            pole: mix(self.pole, other.pole),
            // Interpolate the cutoff on a log scale, like pitch.
            cutoff: mix(self.cutoff.ln(), other.cutoff.ln()).exp(),
        }
    }
}

/// The frequency-dependent part of the loop loss: a one-pole lowpass, followed
/// by a Butterworth lowpass when the loss is [`Loss::Measured`]. Unity gain at DC.
pub struct LossFilter {
    one_pole: OnePoleLowpass,
    butterworth: Option<Butterworth>,
    sample_rate: f32,
}

impl LossFilter {
    pub fn new(sample_rate: f32, with_butterworth: bool) -> Self {
        Self {
            one_pole: OnePoleLowpass::new(0.0),
            butterworth: with_butterworth
                .then(|| Butterworth::new(0.25 * sample_rate as f64, sample_rate as f64)),
            sample_rate,
        }
    }

    pub fn set(&mut self, design: &LossDesign) {
        self.one_pole.a = design.pole;
        if let Some(b) = &mut self.butterworth {
            b.set_cutoff(design.cutoff as f64, self.sample_rate as f64);
        }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.one_pole.process(x);
        match &mut self.butterworth {
            Some(b) => b.process(y),
            None => y,
        }
    }

    pub fn reset(&mut self) {
        self.one_pole.reset();
        if let Some(b) = &mut self.butterworth {
            b.reset();
        }
    }

    /// Phase delay (samples) at `omega` (radians per sample).
    pub fn phase_delay(&self, omega: f32) -> f32 {
        let one_pole = OnePoleLowpass::phase_delay(self.one_pole.a, omega);
        match &self.butterworth {
            Some(b) => one_pole + (b.phase_lag(omega as f64) / omega as f64) as f32,
            None => one_pole,
        }
    }

    /// Phase lag (radians) at `omega` (radians per sample).
    pub fn phase_lag(&self, omega: f64) -> f64 {
        let one_pole = OnePoleLowpass::phase_delay(self.one_pole.a, omega as f32) as f64 * omega;
        one_pole
            + self
                .butterworth
                .as_ref()
                .map_or(0.0, |b| b.phase_lag(omega))
    }

    /// Loss (nepers, `−ln|H|`) of a unity-DC one-pole lowpass with pole `a` at `omega`.
    pub fn one_pole_loss(a: f64, omega: f64) -> f64 {
        0.5 * (1.0 - 2.0 * a * omega.cos() + a * a).ln() - (1.0 - a).ln()
    }
}

/// Second-order Butterworth lowpass (bilinear transform, RBJ cookbook form),
/// transposed direct form II. Its magnitude never exceeds 1.
struct Butterworth {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    s1: f32,
    s2: f32,
}

impl Butterworth {
    fn new(cutoff: f64, sample_rate: f64) -> Self {
        let mut b = Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            s1: 0.0,
            s2: 0.0,
        };
        b.set_cutoff(cutoff, sample_rate);
        b
    }

    fn set_cutoff(&mut self, cutoff: f64, sample_rate: f64) {
        let w = std::f64::consts::TAU * cutoff.min(0.49 * sample_rate) / sample_rate;
        let (s, c) = w.sin_cos();
        let alpha = s * std::f64::consts::FRAC_1_SQRT_2;
        let a0 = 1.0 + alpha;
        self.b0 = 0.5 * (1.0 - c) / a0;
        self.b1 = (1.0 - c) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * c / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn process(&mut self, x: f32) -> f32 {
        let (b0, b1, b2) = (self.b0 as f32, self.b1 as f32, self.b2 as f32);
        let (a1, a2) = (self.a1 as f32, self.a2 as f32);
        let y = b0 * x + self.s1;
        self.s1 = b1 * x - a1 * y + self.s2;
        self.s2 = b2 * x - a2 * y;
        y
    }

    fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    /// Denominator `1 + a1·e^{−jω} + a2·e^{−2jω}` as (re, im).
    fn denominator(&self, omega: f64) -> (f64, f64) {
        let (s1, c1) = omega.sin_cos();
        let (s2, c2) = (2.0 * omega).sin_cos();
        (
            1.0 + self.a1 * c1 + self.a2 * c2,
            -(self.a1 * s1 + self.a2 * s2),
        )
    }

    /// `−ln|H|` at `omega`.
    fn loss(&self, omega: f64) -> f64 {
        // Numerator b0·(1 + e^{−jω})², magnitude b0·4·cos²(ω/2).
        let num = self.b0 * 4.0 * (0.5 * omega).cos().powi(2);
        let (re, im) = self.denominator(omega);
        0.5 * (re * re + im * im).ln() - num.ln()
    }

    /// Phase lag (radians), continuous on [0, π).
    fn phase_lag(&self, omega: f64) -> f64 {
        // The numerator contributes ω. The denominator factors as
        // (1 − p·e^{−jω})(1 − p̄·e^{−jω}) with |p| < 1, so each factor has a
        // positive real part and its argument never wraps.
        let re_p = -0.5 * self.a1;
        let disc = self.a2 - re_p * re_p;
        let (s, c) = omega.sin_cos();
        let arg = |pr: f64, pi: f64| {
            // 1 − p·e^{−jω} = 1 − (pr + j·pi)(c − j·s)
            (-(pi * c - pr * s)).atan2(1.0 - (pr * c + pi * s))
        };
        let denominator_arg = if disc >= 0.0 {
            let im_p = disc.sqrt();
            arg(re_p, im_p) + arg(re_p, -im_p)
        } else {
            let r = (-disc).sqrt();
            arg(re_p + r, 0.0) + arg(re_p - r, 0.0)
        };
        omega + denominator_arg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Magnitude and phase lag of `process` at `omega`, from a steady tone.
    fn measure(mut process: impl FnMut(f32) -> f32, omega: f64) -> (f64, f64) {
        let n = 40_000;
        let (mut re, mut im) = (0.0_f64, 0.0_f64);
        for i in 0..n {
            let ph = omega * i as f64;
            let y = process(ph.sin() as f32) as f64;
            if i >= n / 2 {
                re += y * ph.sin();
                im += y * ph.cos();
            }
        }
        let mag = 2.0 * (re * re + im * im).sqrt() / (n / 2) as f64;
        (mag, -im.atan2(re))
    }

    #[test]
    fn butterworth_response_matches_formulas() {
        let fs = 48_000.0;
        let mut b = Butterworth::new(1500.0, fs);
        for period in [400.0, 64.0, 32.0, 16.0] {
            let omega = std::f64::consts::TAU / period;
            b.reset();
            let (mag, lag) = measure(|x| b.process(x), omega);
            assert!(
                (-mag.ln() - b.loss(omega)).abs() < 1e-3,
                "loss at period {period}"
            );
            let diff = (lag - b.phase_lag(omega)).rem_euclid(std::f64::consts::TAU);
            assert!(
                diff.min(std::f64::consts::TAU - diff) < 1e-3,
                "phase at period {period}"
            );
            assert!(b.loss(omega) >= 0.0);
        }
    }

    #[test]
    fn measured_design_follows_the_damping_curve() {
        let curve = DampingCurve {
            floor: 2.9e-4,
            at_1khz: 5.4e-4,
            exponent: 3.54,
        };
        let (f0, fs) = (98.0, 48_000.0);
        let d = curve.design(f0, fs);
        let mut filter = LossFilter::new(fs, true);
        filter.set(&d);
        let butterworth = filter.butterworth.as_ref().unwrap();
        // Over the measured range (modes 1–17) the fitted ζ is within a factor
        // of 1.5 of the curve.
        for n in 1..=17 {
            let w = std::f64::consts::TAU * n as f64 * f0 as f64 / fs as f64;
            let loss = d.dc_loss as f64
                + LossFilter::one_pole_loss(d.pole as f64, w)
                + butterworth.loss(w);
            let zeta = loss / (std::f64::consts::TAU * n as f64);
            let target = curve.zeta(n as f32 * f0) as f64;
            assert!(
                (zeta / target).ln().abs() < 1.5f64.ln(),
                "mode {n}: ζ {zeta:.2e}, target {target:.2e} ({d:?})"
            );
        }
    }
}
