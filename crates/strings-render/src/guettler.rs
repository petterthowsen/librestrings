//! Compares the model's attacks with measured ones: the mdw Vienna Guettler
//! diagrams (Lampis, Mayer & Chatziioannou, JASA Express Lett. 4, 113201
//! (2024); data: Zenodo 13374477, CC BY 4.0). A robot bows four cello G2
//! strings on a monochord at β = 5.5/70 from rest with a constant bow force
//! and a constant acceleration, and records the bridge force. Each measured
//! stroke is played again on the model at its own force and acceleration, and
//! both bridge-force signals go through the same classifier: the authors'
//! MATLAB code (Zenodo 10946413: `detect_first_slip`, `HM_yn_gall`, the
//! Galluzzo–Woodhouse histogram method, and `transient_analysis`), ported
//! here. The transient runs from the first slip to the last non-Helmholtz
//! period before steady Helmholtz motion; 30 means it never got there.

use std::f64::consts::PI;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use strings_dsp::presets::{cello, reference};
use strings_dsp::{
    BowHair, BowInput, BowNoise, BowedString, FrictionParams, StringSpec, ThermalFriction,
    TorsionSpec,
};

/// The recordings' sample rate.
const FS: f64 = 50_000.0;
/// Bow position: 5.5 cm from the bridge on a 70 cm string.
const BETA: f64 = 5.5 / 70.0;
/// The classifier's nominal fundamental (the authors' `f0`).
const F0: f64 = 98.0;
const LENGTH: f32 = 0.70;
/// A transient this long or longer (periods) failed; the authors' plots stop
/// at 20.
const SUCCESS: u32 = 20;
/// Transient assigned when Helmholtz motion never settles.
const FAILED: u32 = 30;
/// Samples of rest before the stroke in the recordings (`delta_ramp`).
const PRE_ROLL: usize = 2000;

/// One of the four measured string types (Table 1 of the paper).
#[derive(Clone, Copy)]
struct Model {
    letter: char,
    name: &'static str,
    tension: f32,
    /// Bending stiffness EI (N·m²).
    bending_stiffness: f32,
}

const MODELS: [Model; 4] = [
    Model {
        letter: 'A',
        name: "D'Addario Prelude",
        tension: 149.6,
        bending_stiffness: 6.85e-4,
    },
    Model {
        letter: 'B',
        name: "D'Addario Helicore",
        tension: 153.2,
        bending_stiffness: 4.32e-4,
    },
    Model {
        letter: 'C',
        name: "Thomastik Dominant",
        tension: 116.3,
        bending_stiffness: 3.03e-4,
    },
    Model {
        letter: 'D',
        name: "D'Addario Kaplan",
        tension: 153.5,
        bending_stiffness: 3.1e-4,
    },
];

impl Model {
    /// The measured string as a model string: its tension, pitch and bending
    /// stiffness, with the reference string's measured damping and estimated
    /// torsion (scaled to its impedance, as the cello preset does).
    fn spec(&self) -> StringSpec {
        let base = reference::MONOCHORD_CELLO_G_A_T1;
        let z = self.tension / (2.0 * LENGTH * F0 as f32);
        StringSpec {
            name: self.name,
            frequency: F0 as f32,
            length: LENGTH,
            tension: self.tension,
            bending_stiffness: self.bending_stiffness,
            torsion: base.torsion.map(|t| TorsionSpec {
                impedance: 3.3 * z,
                frequency: 5.5 * F0 as f32,
                ..t
            }),
            ..base
        }
    }

    fn impedance(&self) -> f64 {
        self.tension as f64 / (2.0 * LENGTH as f64 * F0)
    }
}

/// The authors' bridge-force corrections per session, by folder date
/// (`utilities/force_correction_factors.m`); 1 when not listed.
fn correction(session: &str) -> f64 {
    match session {
        "2023-08-09" | "2023-08-11" => 1.1,
        "2023-09-06_r" | "2023-09-07" => 0.88,
        "2023-09-05" | "2023-09-08_r" | "2023-09-11" | "2023-09-13_r" | "2023-09-14"
        | "2023-09-15_r" | "2023-09-18" | "2023-09-19_r" | "2023-09-21" | "2023-10-19_r"
        | "2023-10-20" | "2023-10-20_r" | "2023-11-06" | "2023-11-06_r" | "2023-11-10"
        | "2023-11-13_r" => 0.9,
        "2023-09-20" | "2023-09-21_r" | "2023-09-22_r" | "2023-09-25" | "2023-09-26"
        | "2023-09-26_r" | "2023-09-27_r" | "2023-10-03" | "2023-10-03_r" | "2023-10-04_r"
        | "2023-10-18" | "2023-11-07" | "2023-11-07_r" | "2023-11-08" | "2023-11-08_r" => 0.95,
        _ => 1.0,
    }
}

pub struct Options {
    pub data: PathBuf,
    /// String types to compare (letters A–D); all if empty.
    pub models: Vec<char>,
    /// Which sample of each type; the paper uses 2 (type C has only 2).
    pub sample: u32,
    /// Bow hair of the model; `None` is a rigid bow.
    pub hair: Option<BowHair>,
    pub friction: FrictionParams,
    pub bow_noise: BowNoise,
    pub thermal: Option<ThermalFriction>,
    /// The model's strings run at this multiple of 50 kHz (the instrument's 2×).
    pub oversampling: u32,
    /// Scales the bow force of every simulated stroke.
    pub force_scale: f32,
    pub csv: Option<PathBuf>,
}

impl Options {
    pub fn defaults(data: PathBuf) -> Self {
        let cello = cello::INSTRUMENT;
        Self {
            data,
            models: Vec::new(),
            sample: 2,
            hair: cello.hair,
            friction: cello.friction,
            bow_noise: cello.bow_noise,
            thermal: None,
            oversampling: 2,
            force_scale: 1.0,
            csv: None,
        }
    }
}

struct Stroke {
    session: String,
    file: PathBuf,
    force: f64,
    accel: f64,
}

struct Result {
    force: f64,
    accel: f64,
    measured: u32,
    simulated: u32,
}

pub fn run(opts: &Options) -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!(
        "Model: {:?}, bow hair {:?}, bow noise {:?}, strings at {}× 50 kHz{}{}",
        opts.friction,
        opts.hair,
        opts.bow_noise,
        opts.oversampling,
        match opts.thermal {
            Some(t) => format!(", thermal friction {t:?}"),
            None => String::new(),
        },
        if opts.force_scale != 1.0 {
            format!(", bow force × {}", opts.force_scale)
        } else {
            String::new()
        }
    );
    let mut csv = match &opts.csv {
        Some(path) => {
            let mut w = BufWriter::new(File::create(path)?);
            writeln!(w, "model,session,force,accel,measured,simulated")?;
            Some(w)
        }
        None => None,
    };
    let mut summary = Vec::new();
    for model in MODELS
        .iter()
        .filter(|m| opts.models.is_empty() || opts.models.contains(&m.letter))
    {
        let dir = opts
            .data
            .join(format!("model_{}_{}", model.letter, opts.sample));
        let strokes = read_strokes(&dir)?;
        if strokes.is_empty() {
            println!("\n{}: no recordings", dir.display());
            continue;
        }
        let spec = model.spec();
        println!(
            "\nString {} ({}), sample {}: {} strokes in {} sessions. T = {} N, Z = {:.3} kg/s, EI = {:.2e} N·m²",
            model.letter,
            model.name,
            opts.sample,
            strokes.len(),
            {
                let mut s: Vec<&str> = strokes.iter().map(|s| s.session.as_str()).collect();
                s.dedup();
                s.len()
            },
            model.tension,
            spec.impedance(),
            model.bending_stiffness
        );
        let results = analyse_all(opts, model, &spec, &strokes)?;
        if let Some(w) = &mut csv {
            for (s, r) in strokes.iter().zip(&results) {
                writeln!(
                    w,
                    "{},{},{},{},{},{}",
                    model.letter, s.session, r.force, r.accel, r.measured, r.simulated
                )?;
            }
        }
        print_maps(&results);
        summary.push((model.letter, stats(&results)));
    }
    if summary.len() > 1 {
        println!("\nSummary (successful: transient under {SUCCESS} periods)");
        println!(
            "  string  successful (meas / sim)  under 10  (meas / sim)  mean of successful (meas / sim)  agreement"
        );
        for (letter, s) in &summary {
            println!(
                "  {letter}       {:>5.1}% / {:>5.1}%        {:>5.1}% / {:>5.1}%     {:>5.1} / {:>5.1} periods               {:>4.0}%",
                s.success.0, s.success.1, s.fast.0, s.fast.1, s.mean.0, s.mean.1, s.agreement
            );
        }
    }
    if let Some(path) = &opts.csv {
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn read_strokes(dir: &Path) -> std::result::Result<Vec<Stroke>, Box<dyn std::error::Error>> {
    let mut sessions: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    sessions.sort();
    let mut strokes = Vec::new();
    for session in sessions {
        let name = session.file_name().unwrap().to_string_lossy().into_owned();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&session)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "wav"))
            .collect();
        files.sort();
        for file in files {
            // Fb_<N>_a_<m/s²>.wav
            let stem = file.file_stem().unwrap().to_string_lossy().into_owned();
            let parts: Vec<&str> = stem.split('_').collect();
            if parts.len() != 4 || parts[0] != "Fb" || parts[2] != "a" {
                continue;
            }
            strokes.push(Stroke {
                session: name.clone(),
                force: parts[1].parse()?,
                accel: parts[3].parse()?,
                file,
            });
        }
    }
    Ok(strokes)
}

fn analyse_all(
    opts: &Options,
    model: &Model,
    spec: &StringSpec,
    strokes: &[Stroke],
) -> std::result::Result<Vec<Result>, Box<dyn std::error::Error>> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = strokes.len().div_ceil(threads).max(1);
    let z = model.impedance();
    std::thread::scope(|scope| {
        let handles: Vec<_> = strokes
            .chunks(chunk)
            .enumerate()
            .map(|(c, chunk)| {
                scope.spawn(move || -> std::result::Result<Vec<Result>, String> {
                    let rate = FS as f32 * opts.oversampling as f32;
                    let mut string = BowedString::new(spec, opts.friction, rate, spec.frequency);
                    string.set_bow_hair(opts.hair);
                    string.set_thermal_friction(opts.thermal);
                    string.set_bow_noise(opts.bow_noise);
                    string.set_bow_position(BETA as f32);
                    chunk
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            let measured = read_wav_f64(&s.file)
                                .map_err(|e| format!("{}: {e}", s.file.display()))?;
                            let n = measured.len();
                            let measured: Vec<f64> = measured.iter().map(|v| 10.0 * v).collect();
                            string.reset();
                            string.reseed_noise((c * chunk.len() + i) as u32);
                            let simulated = simulate(
                                &mut string,
                                opts.oversampling as usize,
                                opts.force_scale as f64 * s.force,
                                s.accel,
                                n,
                            );
                            Ok(Result {
                                force: s.force,
                                accel: s.accel,
                                measured: transient(&measured, s.accel, z, correction(&s.session)),
                                simulated: transient(&simulated, s.accel, z, 1.0),
                            })
                        })
                        .collect()
                })
            })
            .collect();
        let mut all = Vec::with_capacity(strokes.len());
        for h in handles {
            all.extend(h.join().expect("worker panicked")?);
        }
        Ok(all)
    })
}

/// The robot's stroke on the model: the bow force from the start, the bow at
/// rest for `PRE_ROLL` samples, then accelerating at `accel`. Returns the
/// bridge force at 50 kHz, `n` samples.
fn simulate(
    string: &mut BowedString,
    oversampling: usize,
    force: f64,
    accel: f64,
    n: usize,
) -> Vec<f64> {
    let rate = FS * oversampling as f64;
    (0..n)
        .map(|i| {
            let mut sum = 0.0;
            for k in 0..oversampling {
                let t = ((i * oversampling + k) as f64 - (PRE_ROLL * oversampling) as f64) / rate;
                let frame = string.process(
                    BowInput {
                        velocity: (accel * t.max(0.0)) as f32,
                        force: force as f32,
                    },
                    0.0,
                );
                sum += frame.bridge_force as f64;
            }
            sum / oversampling as f64
        })
        .collect()
}

/// A 64-bit (or 32-bit) float WAV file, mono.
fn read_wav_f64(path: &Path) -> std::result::Result<Vec<f64>, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let mut i = 12;
    let mut bits = 0;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let size = u32::from_le_bytes(bytes[i + 4..i + 8].try_into()?) as usize;
        let body = &bytes[i + 8..(i + 8 + size).min(bytes.len())];
        if id == b"fmt " {
            let format = u16::from_le_bytes(body[0..2].try_into()?);
            let channels = u16::from_le_bytes(body[2..4].try_into()?);
            bits = u16::from_le_bytes(body[14..16].try_into()?);
            if format != 3 || channels != 1 {
                return Err(format!("format {format}, {channels} channels").into());
            }
        } else if id == b"data" {
            return match bits {
                64 => Ok(body
                    .chunks_exact(8)
                    .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                    .collect()),
                32 => Ok(body
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()) as f64)
                    .collect()),
                _ => Err(format!("{bits}-bit samples").into()),
            };
        }
        i += 8 + size + (size & 1);
    }
    Err("no data chunk".into())
}

// ---------------------------------------------------------------------------
// The classifier (the authors' MATLAB code, ported).

/// Pre-Helmholtz transient in periods of 98 Hz; `FAILED` when Helmholtz
/// motion doesn't settle or the string goes into S-motion.
fn transient(fb: &[f64], accel: f64, impedance: f64, correction: f64) -> u32 {
    let Some(first_slip) = first_slip(fb) else {
        return FAILED;
    };
    let v = velocity(fb, accel);
    if s_motion(fb, &v, first_slip) {
        return FAILED;
    }
    let corrected: Vec<f64> = fb.iter().map(|x| x * correction).collect();
    let detections = detections(&corrected, &v, impedance);
    transient_periods(&detections, first_slip as f64 / FS)
}

/// The first slip (sample index): the first strong peak of the bridge force's
/// smoothed slope (`detect_first_slip.m`).
fn first_slip(fb: &[f64]) -> Option<usize> {
    let lowpass = butterworth(9, 1000.0 / (FS / 2.0), false);
    let f = filtfilt(&lowpass, fb);
    let mut d: Vec<f64> = f.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    let head = d.len().min((0.15 * FS) as usize);
    let max = d[..head].iter().copied().fold(0.0, f64::max);
    if max <= 0.0 {
        return None;
    }
    for x in &mut d {
        *x /= max;
    }
    let d = filtfilt(&lowpass, &d);
    find_peaks(&d, 0.15, 0.1, (0.005 * FS) as usize)
        .first()
        .copied()
}

/// The bow speed (`preprocessing.m`): at rest until the bridge force leaves
/// 0.05 N, then rising at `accel`, never quite zero.
fn velocity(fb: &[f64], accel: f64) -> Vec<f64> {
    let rest = fb[..fb.len().min(PRE_ROLL)]
        .iter()
        .filter(|&&x| x < 0.05)
        .count();
    (0..fb.len())
        .map(|i| accel * (i.saturating_sub(rest)) as f64 / FS + 0.5e-4)
        .collect()
}

/// S-motion (`exclude_high_freqs.m`): after the first slip, the speed-scaled
/// bridge force's strongest component lies above 2 × f0.
fn s_motion(fb: &[f64], v: &[f64], first_slip: usize) -> bool {
    let x: Vec<f64> = fb[first_slip..]
        .iter()
        .zip(&v[first_slip..])
        .map(|(f, v)| f / v)
        .collect();
    if x.len() < 16 {
        return false;
    }
    let mean = x.iter().sum::<f64>() / x.len() as f64;
    let sd = (x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    if sd == 0.0 {
        return false;
    }
    let norm: Vec<f64> = x.iter().map(|v| (v - mean) / sd).collect();
    let highpass = butterworth(3, 50.0 / (FS / 2.0), true);
    let y = filtfilt(&highpass, &norm);
    peak_frequency(&y) > 2.0 * F0
}

/// Galluzzo & Woodhouse's histogram method (`HM_yn_gall.m`). Scaled by the bow
/// speed and with the Helmholtz ramp taken out, the bridge force of Helmholtz
/// motion is a staircase falling 2Z/β per period; each step is a peak in the
/// histogram of the smoothed signal. Returns (time in s, kind) per step, in
/// time order: 1 Helmholtz, 2 multiple slips (a smaller step), 3 raucous (a
/// larger one); the last step has no successor and kind 0.
fn detections(fb: &[f64], v: &[f64], impedance: f64) -> Vec<(f64, u8)> {
    let n = fb.len();
    let period = 1.0 / F0;
    let ramp = 2.0 * impedance / (BETA * period);
    let processed: Vec<f64> = (0..n)
        .map(|i| fb[i] / v[i] - ramp * i as f64 / FS)
        .collect();
    let m = (BETA * period * FS).round() as usize;
    let smooth = filtfilt_boxcar(&processed, m);

    let (lo, hi) = smooth
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    if hi <= lo {
        return vec![(0.0, 0)];
    }
    let bins = n;
    let width = (hi - lo) / bins as f64;
    let mut counts = vec![0.0; bins];
    for &x in &smooth {
        let b = (((x - lo) / width) as usize).min(bins - 1);
        counts[b] += 1.0;
    }
    let smoothed = filtfilt(&butterworth(4, 5000.0 / (FS / 2.0), false), &counts);
    // The authors' force axis: `bins` points from the lowest to the highest edge.
    let level = |b: usize| lo + (hi - lo) * b as f64 / (bins - 1) as f64;
    let step = (hi - lo) / (bins - 1) as f64;
    let distance = (3.0 / step).ceil().max(1.0) as usize;
    let mut peaks: Vec<f64> = find_peaks(&smoothed, 5.0, 5.0, distance)
        .into_iter()
        .map(level)
        .collect();
    if peaks.len() < 2 {
        return vec![(0.0, 0)];
    }
    peaks.reverse();

    let helmholtz = 2.0 * impedance / BETA;
    let tolerance = helmholtz / 10.0;
    let mut out = Vec::with_capacity(peaks.len());
    for w in peaks.windows(2) {
        let gap = (w[0] - w[1]).abs();
        let kind = if gap < helmholtz - 2.0 * tolerance {
            2
        } else if gap > 2.0 * helmholtz - tolerance {
            3
        } else {
            1
        };
        let index = smooth
            .iter()
            .enumerate()
            .fold((0, f64::MAX), |best, (i, &x)| {
                let d = (x - w[0]).abs();
                if d < best.1 { (i, d) } else { best }
            })
            .0;
        out.push((index as f64 / FS, kind));
    }
    out.push((0.0, 0));
    out
}

/// Periods from the first slip to the first of the last Helmholtz steps
/// (`transient_analysis.m`): at least the last three steps must be Helmholtz.
fn transient_periods(detections: &[(f64, u8)], first_slip: f64) -> u32 {
    let det: Vec<(f64, u8)> = detections
        .iter()
        .copied()
        .filter(|d| d.0 > first_slip)
        .collect();
    let mut first = det.iter().find(|d| d.1 != 0).map_or(0.0, |d| d.0);
    if first_slip < first {
        first = first_slip;
    }
    const M: usize = 3;
    let n = det.len();
    // MATLAB's 1-based `detections(length - k)`.
    let at = |j: usize| det[j - 1];
    let helmholtz_at_end = (1..=M).filter(|&k| n > k && at(n - k).1 == 1).count();
    if helmholtz_at_end < M {
        return FAILED;
    }
    let mut last = 0.0;
    for k in M + 1..=n {
        let j = n - k;
        if j >= 1 && matches!(at(j).1, 2 | 3) {
            last = det[j].0;
            break;
        }
    }
    if last < first {
        last = first;
    }
    ((last - first) * F0).floor().max(0.0) as u32
}

// ---------------------------------------------------------------------------
// Signal helpers: Butterworth sections, zero-phase filtering, peak picking.

/// One second-order section (transposed direct form II), a1 and a2 with the
/// sign convention y = b·x − a·y.
#[derive(Clone, Copy)]
struct Section {
    b: [f64; 3],
    a: [f64; 2],
}

impl Section {
    fn dc_gain(&self) -> f64 {
        (self.b[0] + self.b[1] + self.b[2]) / (1.0 + self.a[0] + self.a[1])
    }
}

/// Digital Butterworth filter of `order` with cutoff `wn` (relative to
/// Nyquist), by the bilinear transform, as second-order sections.
fn butterworth(order: usize, wn: f64, highpass: bool) -> Vec<Section> {
    let k = (PI * wn / 2.0).tan();
    let mut sections = Vec::new();
    for i in 0..order / 2 {
        let q = 1.0 / (2.0 * (PI * (2 * i + 1) as f64 / (2 * order) as f64).sin());
        let norm = 1.0 / (1.0 + k / q + k * k);
        let a = [2.0 * (k * k - 1.0) * norm, (1.0 - k / q + k * k) * norm];
        let b = if highpass {
            [norm, -2.0 * norm, norm]
        } else {
            let b0 = k * k * norm;
            [b0, 2.0 * b0, b0]
        };
        sections.push(Section { b, a });
    }
    if order % 2 == 1 {
        let a1 = (k - 1.0) / (k + 1.0);
        let b = if highpass {
            let b0 = 1.0 / (k + 1.0);
            [b0, -b0, 0.0]
        } else {
            let b0 = k / (k + 1.0);
            [b0, b0, 0.0]
        };
        sections.push(Section { b, a: [a1, 0.0] });
    }
    sections
}

/// Runs the sections over `x`, each starting in its steady state for a
/// constant input of `x[0]` (as `filtfilt`'s initial conditions do).
fn filter(sections: &[Section], x: &mut [f64]) {
    let Some(&x0) = x.first() else { return };
    let mut u = x0;
    for s in sections {
        let g = s.dc_gain();
        let z2 = (s.b[2] - s.a[1] * g) * u;
        let z1 = (s.b[1] - s.a[0] * g) * u + z2;
        let (mut z1, mut z2) = (z1, z2);
        for v in x.iter_mut() {
            let input = *v;
            let y = s.b[0] * input + z1;
            z1 = s.b[1] * input - s.a[0] * y + z2;
            z2 = s.b[2] * input - s.a[1] * y;
            *v = y;
        }
        u *= g;
    }
}

/// Odd extension of `x` by `pad` samples at both ends.
fn extend(x: &[f64], pad: usize) -> Vec<f64> {
    let n = x.len();
    let pad = pad.min(n.saturating_sub(1));
    let mut out = Vec::with_capacity(n + 2 * pad);
    out.extend((1..=pad).rev().map(|i| 2.0 * x[0] - x[i]));
    out.extend_from_slice(x);
    out.extend((1..=pad).map(|i| 2.0 * x[n - 1] - x[n - 1 - i]));
    out
}

/// Zero-phase filtering, forward and backward, with odd padding of three
/// filter lengths (MATLAB's and SciPy's `filtfilt`).
fn filtfilt(sections: &[Section], x: &[f64]) -> Vec<f64> {
    let order: usize = sections
        .iter()
        .map(|s| if s.a[1] == 0.0 && s.b[2] == 0.0 { 1 } else { 2 })
        .sum();
    let pad = 3 * order;
    let mut y = extend(x, pad);
    let pad = (y.len() - x.len()) / 2;
    filter(sections, &mut y);
    y.reverse();
    filter(sections, &mut y);
    y.reverse();
    y[pad..pad + x.len()].to_vec()
}

/// A moving average of `m` samples, forward and backward.
fn filtfilt_boxcar(x: &[f64], m: usize) -> Vec<f64> {
    let m = m.max(1);
    let run = |x: &mut Vec<f64>| {
        let x0 = x[0];
        let mut history = vec![x0; m];
        let mut sum = x0 * m as f64;
        for (i, v) in x.iter_mut().enumerate() {
            let slot = i % m;
            sum += *v - history[slot];
            history[slot] = *v;
            *v = sum / m as f64;
        }
    };
    let mut y = extend(x, 3 * (m - 1));
    let pad = (y.len() - x.len()) / 2;
    run(&mut y);
    y.reverse();
    run(&mut y);
    y.reverse();
    y[pad..pad + x.len()].to_vec()
}

/// Local maxima of `x` at least `height` high and `prominence` above their
/// surroundings, and `distance` samples apart (the taller wins), as SciPy's
/// `find_peaks` picks them. Returns indices in increasing order.
fn find_peaks(x: &[f64], height: f64, prominence: f64, distance: usize) -> Vec<usize> {
    let n = x.len();
    let mut peaks = Vec::new();
    let mut i = 1;
    while i + 1 < n {
        if x[i - 1] < x[i] {
            // A plateau's peak is its middle.
            let mut ahead = i + 1;
            while ahead + 1 < n && x[ahead] == x[i] {
                ahead += 1;
            }
            if x[ahead] < x[i] {
                peaks.push((i + ahead - 1) / 2);
                i = ahead;
            }
        }
        i += 1;
    }
    peaks.retain(|&p| x[p] >= height);
    if distance > 1 {
        let mut order: Vec<usize> = (0..peaks.len()).collect();
        order.sort_by(|&a, &b| x[peaks[b]].total_cmp(&x[peaks[a]]).then(b.cmp(&a)));
        let mut keep = vec![true; peaks.len()];
        for &k in &order {
            if !keep[k] {
                continue;
            }
            let mut j = k;
            while j > 0 && peaks[k] - peaks[j - 1] < distance {
                j -= 1;
                keep[j] = false;
            }
            let mut j = k + 1;
            while j < peaks.len() && peaks[j] - peaks[k] < distance {
                keep[j] = false;
                j += 1;
            }
        }
        peaks = peaks
            .into_iter()
            .zip(keep)
            .filter_map(|(p, k)| k.then_some(p))
            .collect();
    }
    peaks.retain(|&p| {
        let top = x[p];
        let mut left = x[p];
        let mut j = p;
        while j > 0 && x[j - 1] <= top {
            j -= 1;
            left = left.min(x[j]);
        }
        let mut right = x[p];
        let mut j = p;
        while j + 1 < n && x[j + 1] <= top {
            j += 1;
            right = right.min(x[j]);
        }
        top - left.max(right) >= prominence
    });
    peaks
}

/// Frequency (Hz) of the strongest component of `x`, from a zero-padded FFT.
fn peak_frequency(x: &[f64]) -> f64 {
    let n = x.len().next_power_of_two();
    let mut re = x.to_vec();
    re.resize(n, 0.0);
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    let (bin, _) = (1..n / 2).fold((0, 0.0), |best, k| {
        let p = re[k] * re[k] + im[k] * im[k];
        if p > best.1 { (k, p) } else { best }
    });
    bin as f64 * FS / n as f64
}

/// In-place radix-2 FFT; `re.len()` must be a power of two.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

// ---------------------------------------------------------------------------
// Reporting.

struct Stats {
    /// Percent of strokes under `SUCCESS` periods (measured, simulated).
    success: (f64, f64),
    /// Percent under 10 periods.
    fast: (f64, f64),
    /// Mean transient of the successful strokes (periods).
    mean: (f64, f64),
    /// Percent of strokes both call successful or both failed.
    agreement: f64,
}

fn stats(results: &[Result]) -> Stats {
    let n = results.len() as f64;
    let pct =
        |f: &dyn Fn(&Result) -> bool| 100.0 * results.iter().filter(|r| f(r)).count() as f64 / n;
    let mean = |f: &dyn Fn(&Result) -> u32| {
        let ok: Vec<f64> = results
            .iter()
            .map(f)
            .filter(|&t| t < SUCCESS)
            .map(|t| t as f64)
            .collect();
        ok.iter().sum::<f64>() / ok.len().max(1) as f64
    };
    Stats {
        success: (
            pct(&|r| r.measured < SUCCESS),
            pct(&|r| r.simulated < SUCCESS),
        ),
        fast: (pct(&|r| r.measured < 10), pct(&|r| r.simulated < 10)),
        mean: (mean(&|r| r.measured), mean(&|r| r.simulated)),
        agreement: pct(&|r| (r.measured < SUCCESS) == (r.simulated < SUCCESS)),
    }
}

/// Force rows (N, top edge first) and acceleration columns (m/s²) of the maps.
const FORCE_STEP: f64 = 0.25;
const FORCE_ROWS: usize = 17;
const ACCEL_STEP: f64 = 0.1;
const ACCEL_COLS: usize = 34;

fn print_maps(results: &[Result]) {
    // Per cell: (strokes, measured successes, simulated successes, sums of
    // successful transients).
    let mut cells = vec![[0.0f64; 5]; FORCE_ROWS * ACCEL_COLS];
    for r in results {
        let row = ((r.force / FORCE_STEP) as usize).min(FORCE_ROWS - 1);
        let col = ((r.accel / ACCEL_STEP) as usize).min(ACCEL_COLS - 1);
        let c = &mut cells[row * ACCEL_COLS + col];
        c[0] += 1.0;
        if r.measured < SUCCESS {
            c[1] += 1.0;
            c[3] += r.measured as f64;
        }
        if r.simulated < SUCCESS {
            c[2] += 1.0;
            c[4] += r.simulated as f64;
        }
    }
    println!(
        "  Share of successful attacks (under {SUCCESS} periods) per cell, all sessions: ' ' none, '.' under 10%, 1–9 tenths, '#' 95% or more.\n  Left: measured. Right: simulated at the same bow force and acceleration."
    );
    let symbol = |n: f64, k: f64| {
        if n == 0.0 {
            ' '
        } else {
            let share = k / n;
            if share < 0.1 {
                '.'
            } else if share >= 0.95 {
                '#'
            } else {
                char::from(b'0' + (share * 10.0) as u8)
            }
        }
    };
    for row in (0..FORCE_ROWS).rev() {
        let line = |which: usize| -> String {
            (0..ACCEL_COLS)
                .map(|col| {
                    let c = cells[row * ACCEL_COLS + col];
                    symbol(c[0], c[which])
                })
                .collect()
        };
        println!(
            "  {:>5.2} N  {}   {}",
            row as f64 * FORCE_STEP,
            line(1),
            line(2)
        );
    }
    println!(
        "  {:>9}{:<width$}   a 0 .. {:.1} m/s²",
        "",
        format!("a 0 .. {:.1} m/s²", ACCEL_COLS as f64 * ACCEL_STEP),
        ACCEL_COLS as f64 * ACCEL_STEP,
        width = ACCEL_COLS
    );
    for (name, which) in [("measured", 1), ("simulated", 2)] {
        let edges: Vec<(f64, f64, f64)> = (0..FORCE_ROWS)
            .filter_map(|row| {
                let good: Vec<usize> = (0..ACCEL_COLS)
                    .filter(|&col| {
                        let c = cells[row * ACCEL_COLS + col];
                        c[0] >= 2.0 && c[which] / c[0] >= 0.5
                    })
                    .collect();
                let force = (row as f64 + 0.5) * FORCE_STEP;
                let accel = |col: usize| (col as f64 + 0.5) * ACCEL_STEP;
                (force >= 1.0 && !good.is_empty())
                    .then(|| (force, accel(good[0]), accel(*good.last().unwrap())))
            })
            .collect();
        let right: Vec<(f64, f64)> = edges.iter().map(|e| (e.2, e.0)).collect();
        println!(
            "  {name:>9} region (half the strokes succeed), 1–4 N: acceleration from {} to {}; right edge {}",
            fmt_range(edges.iter().map(|e| e.1)),
            fmt_range(edges.iter().map(|e| e.2)),
            match fit_line(&right) {
                Some((c, k)) => format!("F = {c:.2}·a + {k:.2}"),
                None => "not found".into(),
            }
        );
    }
    let s = stats(results);
    println!(
        "  successful: measured {:.1}%, simulated {:.1}%; under 10 periods: {:.1}% / {:.1}%; mean of successful {:.1} / {:.1} periods; agreement {:.0}%",
        s.success.0, s.success.1, s.fast.0, s.fast.1, s.mean.0, s.mean.1, s.agreement
    );
    // Success by bow force, to show where the model's region sits.
    println!("  by bow force (successful, measured / simulated):");
    for lo in [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5] {
        let set: Vec<&Result> = results
            .iter()
            .filter(|r| r.force >= lo && r.force < lo + 0.5)
            .collect();
        if set.is_empty() {
            continue;
        }
        let n = set.len() as f64;
        let m = set.iter().filter(|r| r.measured < SUCCESS).count() as f64;
        let k = set.iter().filter(|r| r.simulated < SUCCESS).count() as f64;
        println!(
            "    {lo:.1}–{:.1} N: {:>3.0}% / {:>3.0}%  ({} strokes)",
            lo + 0.5,
            100.0 * m / n,
            100.0 * k / n,
            set.len()
        );
    }
}

fn fmt_range(values: impl Iterator<Item = f64>) -> String {
    let (lo, hi) = values.fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if lo > hi {
        "-".into()
    } else {
        format!("{lo:.2}–{hi:.2} m/s²")
    }
}

/// Least-squares line y = c·x + k through (x, y).
fn fit_line(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    (sxx > 0.0).then(|| (sxy / sxx, my - sxy / sxx * mx))
}
