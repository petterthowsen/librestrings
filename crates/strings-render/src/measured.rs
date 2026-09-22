//! Compares the model with a measured Schelleng diagram (mdw Vienna monochord;
//! see docs/Violin Reference Recordings.md, section 3). Each measured point is
//! simulated at its own measured bow force, bow speed and β, and both
//! bridge-force signals go through the same classifier.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use strings_dsp::analysis::{Regime, bow_steady, classify_bridge_force};
use strings_dsp::{BowedString, FrictionParams, StringSpec};

/// The dataset's sample rate; the simulation runs at the same rate.
const SAMPLE_RATE: f32 = 50_000.0;
/// FLAC full scale in N or m/s (see scripts/compact-schelleng.py).
const FULL_SCALE: f32 = 16.0;
/// Bridge force is the third of the four channels.
const BRIDGE_CHANNEL: usize = 2;
/// Points per β column in the measurement grid (40 β × 50 forces).
const FORCES_PER_COLUMN: usize = 50;
const COLUMNS: usize = 40;

struct Point {
    file: String,
    vb_nominal: f32,
    n: usize,
    beta: f32,
    win_start: usize,
    win_end: usize,
    fb: f32,
    vb: f32,
}

impl Point {
    /// Grid position: β column (increasing left to right) and force row
    /// (decreasing top to bottom). Points are numbered from 1, β descending in
    /// the outer loop and force descending in the inner loop.
    fn cell(&self) -> (usize, usize) {
        let i = self.n - 1;
        (COLUMNS - 1 - i / FORCES_PER_COLUMN, i % FORCES_PER_COLUMN)
    }
}

/// (β, F_b) points along one edge of the Helmholtz band.
type Edge = Vec<(f32, f32)>;

struct Classified {
    measured: Regime,
    simulated: Regime,
}

pub fn run(
    dir: &Path,
    spec: &StringSpec,
    friction: FrictionParams,
    csv: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let points = read_index(&dir.join("index.csv"))?;
    println!(
        "{}: {} points; model: {}, Z = {:.4} kg/s, {friction:?}\n  loss: {:?}",
        dir.display(),
        points.len(),
        spec.name,
        spec.impedance(),
        spec.loss
    );
    println!(
        "  EI {} N·m² (B = {:.2e}), torsion: {}",
        spec.bending_stiffness,
        spec.inharmonicity(),
        match spec.torsion {
            Some(t) => format!(
                "Z_t = {} kg/s ({:.2} × Z), f_t = {:.0} Hz ({:.2} × f0), Q {}",
                t.impedance,
                t.impedance / spec.impedance(),
                t.frequency,
                t.frequency / spec.frequency,
                t.q
            ),
            None => "none".into(),
        }
    );
    let results = classify_all(dir, spec, friction, &points)?;

    let mut speeds: Vec<f32> = points.iter().map(|p| p.vb_nominal).collect();
    speeds.sort_by(f32::total_cmp);
    speeds.dedup();
    for &speed in &speeds {
        let set: Vec<(&Point, &Classified)> = points
            .iter()
            .zip(&results)
            .filter(|(p, _)| p.vb_nominal == speed)
            .collect();
        print_map(speed, &set);
    }
    if let Some(path) = csv {
        let mut w = BufWriter::new(File::create(path)?);
        writeln!(w, "n,vb_nominal,beta,fb,vb,measured,simulated")?;
        for (p, r) in points.iter().zip(&results) {
            writeln!(
                w,
                "{},{},{},{},{},{},{}",
                p.n,
                p.vb_nominal,
                p.beta,
                p.fb,
                p.vb,
                r.measured.symbol(),
                r.simulated.symbol()
            )?;
        }
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn read_index(path: &Path) -> Result<Vec<Point>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().ok_or("empty index")?.split(',').collect();
    let col = |name: &str| {
        header
            .iter()
            .position(|h| *h == name)
            .ok_or_else(|| format!("index has no {name} column"))
    };
    let (file, vbn, n, beta, ws, we, fb, vb) = (
        col("file")?,
        col("vb_nominal")?,
        col("n")?,
        col("beta")?,
        col("win_start")?,
        col("win_end")?,
        col("fb_mean")?,
        col("vb_mean")?,
    );
    lines
        .filter(|l| !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            Ok(Point {
                file: f[file].to_string(),
                vb_nominal: f[vbn].parse()?,
                n: f[n].parse()?,
                beta: f[beta].parse()?,
                win_start: f[ws].parse()?,
                win_end: f[we].parse()?,
                fb: f[fb].parse()?,
                vb: f[vb].parse()?,
            })
        })
        .collect()
}

/// Bridge force over the point's steady-state window.
fn load_window(dir: &Path, p: &Point) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let mut reader = claxon::FlacReader::open(dir.join(&p.file))?;
    let info = reader.streaminfo();
    let channels = info.channels as usize;
    let scale = FULL_SCALE / (1u32 << (info.bits_per_sample - 1)) as f32;
    let mut out = Vec::with_capacity(p.win_end - p.win_start);
    for (i, s) in reader.samples().enumerate() {
        let frame = i / channels;
        if frame >= p.win_end {
            break;
        }
        if frame >= p.win_start && i % channels == BRIDGE_CHANNEL {
            out.push(s? as f32 * scale);
        }
    }
    Ok(out)
}

fn simulate(string: &mut BowedString, p: &Point) -> Vec<f32> {
    const SECONDS: f32 = 0.8;
    const STEADY_FROM: f32 = 0.4;
    string.reset();
    string.set_bow_position(p.beta);
    let frames = bow_steady(string, SAMPLE_RATE, p.vb, p.fb.max(0.0), SECONDS, 0.05);
    frames[(STEADY_FROM * SAMPLE_RATE) as usize..]
        .iter()
        .map(|f| f.bridge_force)
        .collect()
}

fn classify_all(
    dir: &Path,
    spec: &StringSpec,
    friction: FrictionParams,
    points: &[Point],
) -> Result<Vec<Classified>, Box<dyn std::error::Error>> {
    let period = SAMPLE_RATE / spec.frequency;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = points.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = points
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || -> Result<Vec<Classified>, String> {
                    let mut string = BowedString::new(spec, friction, SAMPLE_RATE, spec.frequency);
                    chunk
                        .iter()
                        .map(|p| {
                            let measured =
                                load_window(dir, p).map_err(|e| format!("{}: {e}", p.file))?;
                            Ok(Classified {
                                measured: classify_bridge_force(&measured, period),
                                simulated: classify_bridge_force(&simulate(&mut string, p), period),
                            })
                        })
                        .collect()
                })
            })
            .collect();
        let mut all = Vec::with_capacity(points.len());
        for h in handles {
            all.extend(h.join().expect("worker panicked")?);
        }
        Ok(all)
    })
}

fn print_map(speed: f32, set: &[(&Point, &Classified)]) {
    let mut measured = [[' '; COLUMNS]; FORCES_PER_COLUMN];
    let mut simulated = measured;
    let mut row_forces: Vec<Vec<f32>> = vec![Vec::new(); FORCES_PER_COLUMN];
    let mut col_betas = [0.0f32; COLUMNS];
    for (p, r) in set {
        let (c, row) = p.cell();
        measured[row][c] = r.measured.symbol();
        simulated[row][c] = r.simulated.symbol();
        row_forces[row].push(p.fb);
        col_betas[c] = p.beta;
    }

    println!(
        "\nBow speed {speed} m/s. Left: measured. Right: simulated at the same (β, F_b, v_b)."
    );
    println!(
        "H = Helmholtz, M = multi-slip, R = raucous, . = no slipping. Row label: median measured F_b.\n"
    );
    for row in 0..FORCES_PER_COLUMN {
        let label = median(&mut row_forces[row]);
        let m: String = measured[row].iter().collect();
        let s: String = simulated[row].iter().collect();
        println!("{label:>7.3} N  {m}   {s}");
    }
    println!(
        "{:>11}β {:.3} .. {:.3} (log spaced)\n",
        "",
        col_betas[0],
        col_betas[COLUMNS - 1]
    );

    let total = set.len() as f32;
    let same = set
        .iter()
        .filter(|(_, r)| r.measured == r.simulated)
        .count();
    let same_h = set
        .iter()
        .filter(|(_, r)| (r.measured == Regime::Helmholtz) == (r.simulated == Regime::Helmholtz))
        .count();
    let count =
        |f: fn(&Classified) -> Regime, g: Regime| set.iter().filter(|(_, r)| f(r) == g).count();
    println!(
        "Regime agreement {:.0}%, Helmholtz / not-Helmholtz agreement {:.0}%",
        100.0 * same as f32 / total,
        100.0 * same_h as f32 / total
    );
    for g in [
        Regime::Helmholtz,
        Regime::MultiSlip,
        Regime::Raucous,
        Regime::NoSlip,
    ] {
        println!(
            "  {}: measured {:>4}, simulated {:>4}",
            g.symbol(),
            count(|r| r.measured, g),
            count(|r| r.simulated, g)
        );
    }
    for (name, f) in [
        (
            "measured",
            (|r: &Classified| r.measured) as fn(&Classified) -> Regime,
        ),
        ("simulated", |r: &Classified| r.simulated),
    ] {
        let (lower, upper) = force_limits(set, f);
        println!(
            "  {name:>9} limits: lower {}, upper {}",
            fit_text(&lower),
            fit_text(&upper)
        );
    }
}

/// Helmholtz force limits per β column, following Lampis et al. (2025): the
/// band edges are where at least three consecutive points (in force) are
/// Helmholtz. An edge that reaches the end of the measured range is not bracketed
/// and is left out.
fn force_limits(set: &[(&Point, &Classified)], regime: fn(&Classified) -> Regime) -> (Edge, Edge) {
    let mut lower = Vec::new();
    let mut upper = Vec::new();
    for c in 0..COLUMNS {
        let mut column: Vec<(&Point, bool)> = set
            .iter()
            .filter(|(p, _)| p.cell().0 == c)
            .map(|(p, r)| (*p, regime(r) == Regime::Helmholtz))
            .collect();
        column.sort_by(|a, b| a.0.fb.total_cmp(&b.0.fb));
        let mut runs = Vec::new();
        let mut start = None;
        for (i, &(_, h)) in column.iter().enumerate() {
            match (h, start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    runs.push((s, i - 1));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            runs.push((s, column.len() - 1));
        }
        runs.retain(|(s, e)| e - s + 1 >= 3);
        let (Some(&(first, _)), Some(&(_, last))) = (runs.first(), runs.last()) else {
            continue;
        };
        if first > 0 {
            lower.push((column[first].0.beta, column[first].0.fb));
        }
        if last < column.len() - 1 {
            upper.push((column[last].0.beta, column[last].0.fb));
        }
    }
    (lower, upper)
}

/// Least-squares fit of F = c·β^α in log-log space.
fn fit_power(points: &[(f32, f32)]) -> Option<(f32, f32)> {
    let pts: Vec<(f64, f64)> = points
        .iter()
        .filter(|(b, f)| *b > 0.0 && *f > 0.0)
        .map(|&(b, f)| ((b as f64).ln(), (f as f64).ln()))
        .collect();
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f64;
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
    let (mx, my) = (sx / n, sy / n);
    let sxx: f64 = pts.iter().map(|(x, _)| (x - mx).powi(2)).sum();
    let sxy: f64 = pts.iter().map(|(x, y)| (x - mx) * (y - my)).sum();
    let alpha = sxy / sxx;
    Some(((my - alpha * mx).exp() as f32, alpha as f32))
}

fn fit_text(points: &[(f32, f32)]) -> String {
    match fit_power(points) {
        Some((c, alpha)) => format!(
            "F = {c:.4}·β^{alpha:.2} ({} cols; at β = 0.1: {:.2} N)",
            points.len(),
            c * 0.1f32.powf(alpha)
        ),
        None => "not enough columns".into(),
    }
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}
