//! Offline measurement helpers for tests and the renderer. Not real-time safe.

use crate::bow::ContactState;
use crate::string::{BowInput, BowedString, StringFrame};

/// Bows `string` with a fixed force, ramping the bow speed up over `attack`
/// seconds (raised cosine), and returns every sample.
pub fn bow_steady(
    string: &mut BowedString,
    sample_rate: f32,
    speed: f32,
    force: f32,
    seconds: f32,
    attack: f32,
) -> Vec<StringFrame> {
    let n = (seconds * sample_rate) as usize;
    let attack_n = (attack * sample_rate).max(1.0);
    (0..n)
        .map(|i| {
            let ramp = (i as f32 / attack_n).min(1.0);
            let env = 0.5 - 0.5 * (std::f32::consts::PI * ramp).cos();
            string.process(
                BowInput {
                    velocity: speed * env,
                    force,
                },
                0.0,
            )
        })
        .collect()
}

/// Fundamental frequency of `x` near `estimate`, from the phase advance of a
/// Hann-windowed DFT bin between the start and end of the signal. Accurate to a
/// small fraction of a cent for a steady tone a second or so long.
pub fn measure_frequency(x: &[f32], sample_rate: f32, estimate: f32) -> f32 {
    let window = ((8.0 * sample_rate / estimate).round() as usize).min(x.len() / 2);
    let mut f = estimate as f64;
    // Coarse pass with a short hop (wide unwrap range), then a fine pass with the longest hop.
    for hop in [window, x.len() - window] {
        let omega = std::f64::consts::TAU * f / sample_rate as f64;
        let p1 = bin_phase(&x[..window], omega);
        let p2 = bin_phase(&x[hop..hop + window], omega);
        let expected = omega * hop as f64;
        let deviation = wrap_phase(p2 - p1 - expected);
        f += deviation * sample_rate as f64 / (std::f64::consts::TAU * hop as f64);
    }
    f as f32
}

fn bin_phase(x: &[f32], omega: f64) -> f64 {
    let n = x.len() as f64;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n).cos();
        let (s, c) = (omega * i as f64).sin_cos();
        re += w * v as f64 * c;
        im -= w * v as f64 * s;
    }
    im.atan2(re)
}

fn wrap_phase(p: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    p - tau * (p / tau).round()
}

pub fn cents(f: f32, reference: f32) -> f32 {
    1200.0 * (f / reference).log2()
}

#[derive(Clone, Copy, Debug)]
pub struct SlipStats {
    /// Stick-to-slip transitions per nominal period. Helmholtz motion has 1.
    pub slips_per_period: f32,
    /// Fraction of samples spent slipping. Helmholtz motion has about β.
    pub slip_fraction: f32,
}

pub fn slip_stats(frames: &[StringFrame], period: f32) -> SlipStats {
    let slipping: Vec<bool> = frames.iter().map(|f| f.state.is_slipping()).collect();
    let onsets = slipping.windows(2).filter(|w| !w[0] && w[1]).count();
    let slip_samples = slipping.iter().filter(|&&s| s).count();
    SlipStats {
        slips_per_period: onsets as f32 * period / frames.len() as f32,
        slip_fraction: slip_samples as f32 / frames.len() as f32,
    }
}

/// Highest normalized autocorrelation of `x` for lags within ±10% of `period`.
/// Near 1 for a steady periodic tone.
pub fn periodicity(x: &[f32], period: f32) -> f32 {
    let lo = (0.9 * period).floor() as usize;
    let hi = (1.1 * period).ceil() as usize;
    (lo..=hi.min(x.len() / 2))
        .map(|lag| {
            let (a, b) = (&x[..x.len() - lag], &x[lag..]);
            let dot: f64 = a.iter().zip(b).map(|(&p, &q)| p as f64 * q as f64).sum();
            let ea: f64 = a.iter().map(|&p| p as f64 * p as f64).sum();
            let eb: f64 = b.iter().map(|&q| q as f64 * q as f64).sum();
            if ea == 0.0 || eb == 0.0 {
                0.0
            } else {
                (dot / (ea * eb).sqrt()) as f32
            }
        })
        .fold(0.0, f32::max)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regime {
    /// One slip per period, periodic: the normal bowed tone.
    Helmholtz,
    /// Several slips per period: the "surface" sound below minimum force.
    MultiSlip,
    /// Aperiodic or subharmonic: the crunch above maximum force.
    Raucous,
    /// The bow never lets go (or never grips): no oscillation.
    NoSlip,
}

impl Regime {
    pub fn symbol(self) -> char {
        match self {
            Regime::Helmholtz => 'H',
            Regime::MultiSlip => 'M',
            Regime::Raucous => 'R',
            Regime::NoSlip => '.',
        }
    }
}

/// Classifies steady-state frames (skip the attack before calling).
pub fn classify(frames: &[StringFrame], period: f32) -> Regime {
    let stats = slip_stats(frames, period);
    if stats.slips_per_period == 0.0 || frames.iter().all(|f| f.state == ContactState::Off) {
        return Regime::NoSlip;
    }
    let force: Vec<f32> = frames.iter().map(|f| f.bridge_force).collect();
    if periodicity(&force, period) < 0.95 || stats.slips_per_period < 0.75 {
        Regime::Raucous
    } else if stats.slips_per_period > 1.5 {
        Regime::MultiSlip
    } else {
        Regime::Helmholtz
    }
}
