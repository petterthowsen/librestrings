//! Refit a body to the spectral envelope `compare` measured (`envelope.csv`:
//! recording − model per sixth octave).
//!
//! The body's response, as the median dB over a third of an octave, is fitted
//! to the measured envelope smoothed the same way: the finer structure is
//! where this body's modes and the recorded instrument's differ, which a
//! smooth change can't mend. What moves:
//! - the dense modes' envelope: their level, rolloff and hills (up to
//!   `MAX_HILLS`, at least 0.3 octaves wide);
//! - with `listed`, the listed modes' levels (not their frequencies, damping
//!   or signs, which come from measurements);
//! - with `dense_from`, where the dense modes start, at their density (this
//!   draws every dense mode anew);
//! - with `seeds` above 1, the dense modes' seed: the fit is run from that
//!   many seeds (the body's own, then the next ones) and the best kept. Like
//!   the plugin's "Body variant", another seed is another body of the kind.
//!
//! If the dense modes stop below the top of the fit, the bank is extended at
//! its own spacing: the generator draws each mode's jitter, level and sign in
//! order, so the modes already there don't move.
//!
//! Below `keep_below` the body is kept as it is (the cello's low end was
//! fitted note by note; PLAN.md "The body's low end"). The envelope's level is
//! arbitrary, so an overall offset is free: correct the output gain by the
//! level `compare` measures afterwards.

use std::path::Path;

use strings_dsp::body::{MAX_DENSE_MODES, MAX_HILLS};
use strings_dsp::{Body, BodyTuning, Hill, InstrumentSpec};

use crate::envelope;

pub struct Options<'a> {
    pub envelope: &'a Path,
    /// The column of `envelope.csv` to fit (pp, mf, ff or all).
    pub column: &'a str,
    /// Below this (Hz) the body is kept as it is; 0 keeps nothing.
    pub keep_below: f32,
    /// Fit the listed modes' levels too.
    pub listed: bool,
    /// Start the dense modes here (Hz), at their present density.
    pub dense_from: Option<f32>,
    /// How many dense-mode seeds to try.
    pub seeds: u32,
    /// The body's sample rate.
    pub sample_rate: f32,
    pub iterations: usize,
}

/// Weight of a kept bin, against a fitted bin resting on one note.
const KEEP: f32 = 10.0;

/// A listed mode's level moves at most this factor either way.
const LISTED_RANGE: f32 = 4.0;

pub fn run(spec: &InstrumentSpec, opts: &Options) -> Result<(), Box<dyn std::error::Error>> {
    let raw = read_envelope(opts.envelope, opts.column)?;
    let h = smooth(&raw);
    let top = (0.45 * opts.sample_rate).min(10_000.0);
    // The lowest note's fundamental, less a third of an octave.
    let low = 0.8 * spec.strings[0].frequency;
    // (bin, target change dB, weight)
    let bins: Vec<(usize, f32, f32)> = (0..envelope::BINS)
        .filter(|&b| (low..=top).contains(&envelope::centre(b)))
        .filter_map(|b| {
            let (d, notes) = h[b]?;
            if envelope::centre(b) < opts.keep_below {
                Some((b, 0.0, KEEP))
            } else {
                Some((b, d, (notes as f32).sqrt()))
            }
        })
        .collect();
    if bins.iter().all(|b| b.2 == KEEP) {
        return Err(format!("no '{}' envelope to fit", opts.column).into());
    }

    let old = BodyTuning::from(&spec.body);
    let mut base = old;
    let d = &mut base.dense;
    let density = d.count as f32 / (d.to / d.from).ln();
    if let Some(from) = opts.dense_from {
        d.from = from;
        d.count = ((density * (d.to / from).ln()).round() as usize).min(MAX_DENSE_MODES);
    }
    let step = (d.to / d.from).ln() / d.count as f32;
    let count = ((top / d.from).ln() / step).floor() as usize;
    if count > d.count && count <= MAX_DENSE_MODES {
        d.count = count;
        d.to = d.from * (step * count as f32).exp();
    }
    if base.dense != old.dense {
        println!(
            "dense modes: from {:.0} Hz, to {:.0} Hz, count {}",
            base.dense.from, base.dense.to, base.dense.count
        );
    }

    let before = response(&old, &bins, opts.sample_rate);
    let weights: f32 = bins.iter().map(|b| b.2).sum();
    // Weighted mean square misfit after the best overall offset, and that
    // offset (dB).
    let misfit = |t: &BodyTuning| -> (f32, f32) {
        let e: Vec<f32> = response(t, &bins, opts.sample_rate)
            .iter()
            .zip(&before)
            .zip(&bins)
            .map(|((r1, r0), &(_, want, _))| r1 - r0 - want)
            .collect();
        let offset = e.iter().zip(&bins).map(|(e, b)| b.2 * e).sum::<f32>() / weights;
        let ms = e
            .iter()
            .zip(&bins)
            .map(|(e, b)| b.2 * (e - offset).powi(2))
            .sum::<f32>()
            / weights;
        (ms, offset)
    };

    let (start, _) = misfit(&old);
    let mut fits = Vec::new();
    for k in 0..opts.seeds.max(1) {
        let mut base = base;
        base.dense.seed = old.dense.seed.wrapping_add(k).max(1);
        // Start from the body as it is, with any free hills flat at 3 kHz.
        let mut params = Params::from(&old, opts.listed);
        let (mut best, _) = misfit(&params.tuning(&base, &old));
        let mut sigma = 0.3;
        let mut rng = Rng(0x9e37_79b9);
        for _ in 0..opts.iterations {
            let mut trial = params.clone();
            trial.perturb(&mut rng, sigma);
            let (c, _) = misfit(&trial.tuning(&base, &old));
            if c < best {
                best = c;
                params = trial;
                sigma = (sigma * 1.1).min(0.5);
            } else {
                sigma = (sigma * 0.995).max(0.01);
            }
        }
        if opts.seeds > 1 {
            println!("seed {:#x}: {:.2} dB", base.dense.seed, best.sqrt());
        }
        fits.push((best, params.tuning(&base, &old)));
    }
    let (best, new) = fits.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
    let after = response(&new, &bins, opts.sample_rate);
    let (_, offset) = misfit(&new);

    println!(
        "weighted RMS misfit of the response: {:.2} dB before, {:.2} after",
        start.sqrt(),
        best.sqrt()
    );
    println!(
        "\n{:>7} {:>9} {:>7} {:>9}",
        "Hz", "measured", "want", "response"
    );
    for ((&(b, want, weight), r0), r1) in bins.iter().zip(&before).zip(&after) {
        println!(
            "{:>7.0} {:>9} {:>+7.1} {:>+9.1}{}",
            envelope::centre(b),
            raw[b].map_or(".".into(), |(d, _)| format!("{d:+.1}")),
            want,
            r1 - r0 - offset,
            if weight == KEEP { "  kept" } else { "" }
        );
    }
    println!("\nthe response moved {offset:+.1} dB overall (before the output gain)");
    if opts.listed {
        println!("\nlisted modes (Hz: gain):");
        for m in new.modes() {
            println!("    {:.0}: {:.2}", m.frequency, m.gain);
        }
    }
    println!("\n    const HILLS: [Hill; {}] = [", new.hill_count);
    for hill in new.hills() {
        println!(
            "        Hill {{\n            frequency: {:.0}.0,\n            width: {:.2},\n            gain: {:.2},\n        }},",
            hill.frequency, hill.width, hill.gain
        );
    }
    println!("    ];");
    let d = &new.dense;
    println!(
        "    dense: from {:.1}, to {:.1}, count {}, level {:.3}, rolloff {:.0}.0, seed {:#x}",
        d.from, d.to, d.count, d.level, d.rolloff, d.seed
    );
    Ok(())
}

/// The envelope over a third of an octave (weights ¼ ½ ¼ over neighbouring
/// sixth-octave bins, by note count), twice.
fn smooth(h: &Envelope) -> Envelope {
    let mut h = *h;
    for _ in 0..2 {
        let prev = h;
        for (b, out) in h.iter_mut().enumerate() {
            let Some((_, notes)) = prev[b] else { continue };
            let (mut sum, mut w) = (0.0, 0.0);
            for (k, kw) in [(b.wrapping_sub(1), 0.25), (b, 0.5), (b + 1, 0.25)] {
                if let Some(Some((d, n))) = prev.get(k) {
                    sum += kw * *n as f32 * d;
                    w += kw * *n as f32;
                }
            }
            *out = Some((sum / w, notes));
        }
    }
    h
}

/// Per bin: the envelope and how many notes it rests on.
type Envelope = [Option<(f32, usize)>; envelope::BINS];

fn read_envelope(path: &Path, column: &str) -> Result<Envelope, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines
        .next()
        .ok_or("empty envelope file")?
        .split(',')
        .collect();
    let col = header
        .iter()
        .position(|&c| c == column)
        .ok_or_else(|| format!("no column '{column}' in {}", path.display()))?;
    let mut out = [None; envelope::BINS];
    for line in lines {
        let fields: Vec<&str> = line.split(',').collect();
        let (Some(hz), Some(d), Some(n)) = (fields.first(), fields.get(col), fields.get(col + 1))
        else {
            continue;
        };
        let (Ok(hz), Ok(d), Ok(n)) = (hz.parse::<f32>(), d.parse::<f32>(), n.parse::<usize>())
        else {
            continue;
        };
        if let Some(b) = envelope::bin(hz) {
            out[b] = Some((d, n));
        }
    }
    Ok(out)
}

/// The body's response per bin (dB): the median over 24 frequencies across a
/// third of an octave around it.
fn response(tuning: &BodyTuning, bins: &[(usize, f32, f32)], fs: f32) -> Vec<f32> {
    let body = Body::from_tuning(tuning, fs);
    bins.iter()
        .map(|&(b, _, _)| {
            let c = envelope::centre(b);
            let mut v: Vec<f32> = (0..24)
                .map(|k| {
                    let f = c * 2f32.powf(((k as f32 + 0.5) / 24.0 - 0.5) / 3.0);
                    20.0 * body.magnitude(f).max(1e-9).log10()
                })
                .collect();
            v.sort_by(f32::total_cmp);
            0.5 * (v[11] + v[12])
        })
        .collect()
}

/// The fitted numbers, on scales where a step of 0.1 means about the same
/// everywhere: log level, log rolloff, per hill log frequency, width in
/// octaves and gain, and each listed mode's log level against the original.
#[derive(Clone)]
struct Params {
    level: f32,
    rolloff: f32,
    hills: [(f32, f32, f32); MAX_HILLS],
    listed: Vec<f32>,
}

impl Params {
    fn from(t: &BodyTuning, listed: bool) -> Self {
        let mut hills = [(3000f32.ln(), 0.5, 0.0); MAX_HILLS];
        for (h, hill) in hills.iter_mut().zip(t.hills()) {
            *h = (hill.frequency.ln(), hill.width, hill.gain);
        }
        Self {
            level: t.dense.level.ln(),
            rolloff: t.dense.rolloff.ln(),
            hills,
            listed: if listed {
                vec![0.0; t.modes().len()]
            } else {
                Vec::new()
            },
        }
    }

    /// `base` with these numbers; the listed modes scaled from `old`'s.
    fn tuning(&self, base: &BodyTuning, old: &BodyTuning) -> BodyTuning {
        let mut t = *base;
        t.dense.level = self.level.exp();
        t.dense.rolloff = self.rolloff.exp();
        t.hill_count = MAX_HILLS;
        for (hill, &(f, w, g)) in t.hills.iter_mut().zip(&self.hills) {
            *hill = Hill {
                frequency: f.exp(),
                width: w,
                gain: g,
            };
        }
        for ((m, o), s) in t.modes.iter_mut().zip(&old.modes).zip(&self.listed) {
            m.gain = o.gain * s.exp();
        }
        t
    }

    fn perturb(&mut self, rng: &mut Rng, sigma: f32) {
        // One number at a time, most of the time; all of them now and then.
        let all = rng.uniform() < 0.2;
        let n = 2 + 3 * MAX_HILLS + self.listed.len();
        let pick = (rng.uniform() * n as f32) as usize;
        let mut k = 0;
        let mut step = |x: &mut f32, scale: f32, lo: f32, hi: f32| {
            if all || k == pick {
                *x = (*x + sigma * scale * rng.gauss()).clamp(lo, hi);
            }
            k += 1;
        };
        step(&mut self.level, 1.0, (0.02f32).ln(), (3.0f32).ln());
        step(&mut self.rolloff, 1.0, (800.0f32).ln(), (12_000.0f32).ln());
        for (f, w, g) in &mut self.hills {
            step(f, 1.0, (50.0f32).ln(), (12_000.0f32).ln());
            step(w, 0.5, 0.3, 1.5);
            step(g, 2.0, -0.9, 6.0);
        }
        let range = LISTED_RANGE.ln();
        for s in &mut self.listed {
            step(s, 1.0, -range, range);
        }
    }
}

struct Rng(u32);

impl Rng {
    fn uniform(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        ((self.0 >> 8) as f32 + 0.5) / (1u32 << 24) as f32
    }

    fn gauss(&mut self) -> f32 {
        let (u, v) = (self.uniform(), self.uniform());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}
