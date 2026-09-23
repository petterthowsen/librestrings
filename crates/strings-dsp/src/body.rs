//! The instrument body: a bank of resonators driven by the bridge force.
//!
//! The body sits after the strings (the bow is nonlinear, so commuted synthesis
//! doesn't apply; PLAN.md 3.3). Each mode is a two-pole bandpass with unity
//! peak gain, and the bank sums them in parallel with signed gains, so
//! neighbouring modes can cancel between their peaks as they do on a real body.
//!
//! Low modes are listed one by one (the "signature modes", from measurements).
//! Above them, real bodies have too many modes to list; following Woodhouse's
//! statistical view, the bank places modes at seeded random frequencies, with
//! random signs and levels following a smooth envelope (the "bridge hills").
//! The seed makes the body reproducible; sections can vary it per player.

/// One listed body mode.
#[derive(Clone, Copy, Debug)]
pub struct BodyMode {
    /// Natural frequency (Hz).
    pub frequency: f32,
    /// Damping ratio ζ.
    pub damping: f32,
    /// Peak gain; the sign sets the mode's polarity.
    pub gain: f32,
}

/// A smooth rise in the dense modes' level, Gaussian on a log-frequency axis.
#[derive(Clone, Copy, Debug)]
pub struct Hill {
    /// Centre frequency (Hz).
    pub frequency: f32,
    /// Standard deviation in octaves.
    pub width: f32,
    /// Extra level at the centre, as a multiple of the base level.
    pub gain: f32,
}

/// Seeded random modes filling a frequency range.
#[derive(Clone, Copy, Debug)]
pub struct DenseModes {
    /// Frequency range (Hz). Modes are spread evenly in log frequency, each
    /// jittered by up to half a step.
    pub from: f32,
    pub to: f32,
    pub count: usize,
    /// Damping ratio of every dense mode. High enough that neighbouring modes
    /// overlap (bandwidth `2ζf` about the log spacing), as they do on a body.
    pub damping: f32,
    /// Base peak gain.
    pub level: f32,
    /// Above this frequency (Hz) the level falls as 1/f².
    pub rolloff: f32,
    pub hills: &'static [Hill],
    pub seed: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct BodySpec {
    pub modes: &'static [BodyMode],
    pub dense: DenseModes,
}

impl DenseModes {
    fn envelope(&self, f: f32) -> f32 {
        let hills: f32 = self
            .hills
            .iter()
            .map(|h| {
                let octaves = (f / h.frequency).log2() / h.width;
                h.gain * (-0.5 * octaves * octaves).exp()
            })
            .sum();
        let rolloff = 1.0 / (1.0 + (f / self.rolloff).powi(2));
        self.level * (1.0 + hills) * rolloff
    }
}

/// Two-pole bandpass with unity gain at its peak (RBJ cookbook, constant 0 dB
/// peak), transposed direct form II.
#[derive(Clone, Copy, Debug)]
struct Resonator {
    b0: f32,
    a1: f32,
    a2: f32,
    gain: f32,
    s1: f32,
    s2: f32,
}

impl Resonator {
    fn new(frequency: f32, damping: f32, gain: f32, sample_rate: f32) -> Self {
        let w = std::f32::consts::TAU * frequency / sample_rate;
        // α = sin(w) / (2Q) with Q = 1/(2ζ).
        let alpha = w.sin() * damping;
        let a0 = 1.0 + alpha;
        Self {
            b0: alpha / a0,
            a1: -2.0 * w.cos() / a0,
            a2: (1.0 - alpha) / a0,
            gain,
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// `b = [b0, 0, −b0]`.
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.s1;
        self.s1 = self.s2 - self.a1 * y;
        self.s2 = -self.b0 * x - self.a2 * y;
        self.gain * y
    }
}

pub struct Body {
    resonators: Vec<Resonator>,
}

impl Body {
    /// Modes above 0.45 × the sample rate are left out.
    pub fn new(spec: &BodySpec, sample_rate: f32) -> Self {
        let nyquist_limit = 0.45 * sample_rate;
        let mut resonators: Vec<Resonator> = spec
            .modes
            .iter()
            .filter(|m| m.frequency < nyquist_limit)
            .map(|m| Resonator::new(m.frequency, m.damping, m.gain, sample_rate))
            .collect();
        let d = &spec.dense;
        let mut rng = XorShift(d.seed.max(1));
        let step = (d.to / d.from).ln() / d.count as f32;
        for i in 0..d.count {
            let jitter = rng.next() - 0.5;
            let f = d.from * ((i as f32 + 0.5 + jitter) * step).exp();
            if f >= nyquist_limit {
                continue;
            }
            // Levels scatter over about ±6 dB around the envelope.
            let level = d.envelope(f) * 2f32.powf(2.0 * (rng.next() - 0.5));
            let sign = if rng.next() < 0.5 { -1.0 } else { 1.0 };
            resonators.push(Resonator::new(f, d.damping, sign * level, sample_rate));
        }
        Self { resonators }
    }

    pub fn process(&mut self, bridge_force: f32) -> f32 {
        self.resonators
            .iter_mut()
            .map(|r| r.process(bridge_force))
            .sum()
    }

    pub fn reset(&mut self) {
        for r in &mut self.resonators {
            r.s1 = 0.0;
            r.s2 = 0.0;
        }
    }

    pub fn modes(&self) -> usize {
        self.resonators.len()
    }
}

/// Small deterministic generator for the dense modes (Marsaglia xorshift32).
struct XorShift(u32);

impl XorShift {
    /// Uniform in [0, 1).
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: [BodyMode; 1] = [BodyMode {
        frequency: 200.0,
        damping: 0.02,
        gain: 1.0,
    }];

    fn single_mode() -> BodySpec {
        BodySpec {
            modes: &MODES,
            dense: DenseModes {
                from: 300.0,
                to: 6000.0,
                count: 0,
                damping: 0.03,
                level: 0.0,
                rolloff: 3000.0,
                hills: &[],
                seed: 1,
            },
        }
    }

    /// Steady-state gain of the body for a sine at `f`.
    fn gain_at(body: &mut Body, f: f32, fs: f32) -> f32 {
        body.reset();
        let n = (fs * 1.0) as usize;
        let mut peak = 0.0f32;
        for i in 0..n {
            let y = body.process((std::f32::consts::TAU * f * i as f32 / fs).sin());
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn resonator_peaks_at_its_frequency_with_unity_gain() {
        for fs in [44_100.0, 96_000.0] {
            let mut body = Body::new(&single_mode(), fs);
            let at = gain_at(&mut body, 200.0, fs);
            assert!((at - 1.0).abs() < 0.02, "@ {fs}: {at}");
            // Half-power points sit at f·(1 ± ζ).
            let edge = gain_at(&mut body, 200.0 * 1.02, fs);
            assert!((edge - 0.707).abs() < 0.03, "@ {fs}: {edge}");
            assert!(gain_at(&mut body, 400.0, fs) < 0.1);
        }
    }

    #[test]
    fn dense_modes_are_seeded_and_bounded() {
        let hills = [Hill {
            frequency: 1000.0,
            width: 0.5,
            gain: 2.0,
        }];
        let spec = |seed| BodySpec {
            modes: &[],
            dense: DenseModes {
                from: 300.0,
                to: 30_000.0,
                count: 40,
                damping: 0.03,
                level: 0.3,
                rolloff: 3000.0,
                hills: Box::leak(Box::new(hills)),
                seed,
            },
        };
        // Modes above 0.45 fs are dropped.
        assert!(Body::new(&spec(7), 44_100.0).modes() < 40);
        // Same seed, same body; different seed, different body.
        let impulse = |seed| {
            let mut b = Body::new(&spec(seed), 48_000.0);
            (0..4800)
                .map(|i| b.process(if i == 0 { 1.0 } else { 0.0 }))
                .collect::<Vec<f32>>()
        };
        assert_eq!(impulse(7), impulse(7));
        assert_ne!(impulse(7), impulse(8));
        // The impulse response decays.
        let ir = impulse(7);
        let energy = |s: &[f32]| s.iter().map(|x| x * x).sum::<f32>();
        assert!(energy(&ir[4000..]) < 1e-3 * energy(&ir[..800]));
    }
}
