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
    measure_partial(x, sample_rate, estimate, estimate)
}

/// Like [`measure_frequency`], for one partial of a tone whose partials are
/// about `spacing` Hz apart: the window spans 8 periods of `spacing`, so
/// neighbouring partials fall outside the main lobe. The partial must lie
/// within about `spacing / 16` of `estimate`.
pub fn measure_partial(x: &[f32], sample_rate: f32, estimate: f32, spacing: f32) -> f32 {
    let window = ((8.0 * sample_rate / spacing).round() as usize).min(x.len() / 2);
    let longest = x.len() - window;
    let mut f = estimate as f64;
    // Passes from a short hop (wide unwrap range) to the longest (fine). Each
    // hop is at most 4× the last, so the last estimate is close enough to
    // unwrap the next phase: a single jump from the shortest to the longest
    // hop lands a whole cycle off when the period jitters (a high bowed note
    // alternating between whole-sample periods).
    let mut hop = window;
    loop {
        let omega = std::f64::consts::TAU * f / sample_rate as f64;
        let p1 = bin_phase(&x[..window], omega);
        let p2 = bin_phase(&x[hop..hop + window], omega);
        let expected = omega * hop as f64;
        let deviation = wrap_phase(p2 - p1 - expected);
        f += deviation * sample_rate as f64 / (std::f64::consts::TAU * hop as f64);
        if hop == longest {
            break;
        }
        hop = (4 * hop).min(longest);
    }
    f as f32
}

/// Amplitude of the component at `frequency` in `x`, from a Hann-windowed DFT
/// bin over the whole slice. Choose the slice long enough that neighbouring
/// partials fall outside the main lobe (at least 4 periods of their spacing).
pub fn partial_amplitude(x: &[f32], sample_rate: f32, frequency: f32) -> f32 {
    let omega = std::f64::consts::TAU * frequency as f64 / sample_rate as f64;
    let n = x.len() as f64;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n).cos();
        let (s, c) = (omega * i as f64).sin_cos();
        re += w * v as f64 * c;
        im -= w * v as f64 * s;
    }
    // A Hann window's coherent gain is 1/2.
    (4.0 * (re * re + im * im).sqrt() / n) as f32
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

#[derive(Clone, Copy, Debug)]
pub struct FlybackStats {
    /// Sharp drops per nominal period. Helmholtz motion has 1.
    pub per_period: f32,
    /// Typical largest drop per period over the typical peak-to-peak range per
    /// period: near 1 for a sawtooth, about 0.06 for a sinusoid.
    pub sharpness: f32,
}

/// Sharp drops ("flybacks") in a bridge-force signal.
///
/// In Helmholtz motion the bridge force is a sawtooth: a slow rise and one
/// sudden drop per period, when the corner reflects from the bridge. Extra
/// slips add further drops. A drop is a fall over `period / 50` samples of
/// at least `DROP_FRACTION` of the typical largest drop per period; the count
/// re-arms once the fall is back under half the threshold.
pub fn flybacks(x: &[f32], period: f32) -> FlybackStats {
    const DROP_FRACTION: f32 = 0.4;
    let none = FlybackStats {
        per_period: 0.0,
        sharpness: 0.0,
    };
    let w = ((period / 50.0).round() as usize).max(1);
    let p = period.round() as usize;
    if x.len() < p + w {
        return none;
    }
    let fall: Vec<f32> = x.windows(w + 1).map(|s| s[0] - s[w]).collect();
    let median = |mut v: Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let typical_drop = median(
        fall.chunks_exact(p)
            .map(|c| c.iter().copied().fold(0.0, f32::max))
            .collect(),
    );
    let typical_range = median(
        x.chunks_exact(p)
            .map(|c| {
                let (lo, hi) = c
                    .iter()
                    .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
                hi - lo
            })
            .collect(),
    );
    let threshold = DROP_FRACTION * typical_drop;
    if threshold <= 0.0 || typical_range <= 0.0 {
        return none;
    }
    let mut armed = true;
    let mut count = 0;
    for &f in &fall {
        if armed && f > threshold {
            count += 1;
            armed = false;
        } else if f < 0.5 * threshold {
            armed = true;
        }
    }
    FlybackStats {
        per_period: count as f32 * period / fall.len() as f32,
        sharpness: typical_drop / typical_range,
    }
}

/// Classifies a steady-state bridge-force signal on its own, without the
/// model's contact state, so measured and simulated signals are judged alike.
pub fn classify_bridge_force(x: &[f32], period: f32) -> Regime {
    let mean = x.iter().map(|&v| v as f64).sum::<f64>() / x.len() as f64;
    let ac: Vec<f32> = x.iter().map(|&v| (v as f64 - mean) as f32).collect();
    let fb = flybacks(&ac, period);
    let periodic = periodicity(&ac, period) >= 0.9;
    // Periodic without sharp drops: the bow never grips and releases.
    if periodic && fb.sharpness < 0.25 {
        return Regime::NoSlip;
    }
    if !periodic || fb.per_period < 0.75 || fb.sharpness < 0.25 {
        Regime::Raucous
    } else if fb.per_period > 1.5 {
        Regime::MultiSlip
    } else {
        Regime::Helmholtz
    }
}
