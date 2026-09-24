//! Calibrates an instrument's bow-force band (`ForceLimits`, PLAN.md 4.2) from
//! simulation: maps the Helmholtz region of every string over bow speed, β and
//! force, fits both edges as `F = c·Z·v·β^α`, then checks how often each
//! position in the fitted band gives Helmholtz motion.
//!
//! A cell counts as Helmholtz only if it settles promptly: already Helmholtz
//! from [`SETTLED`] on, not only in the steady state. Near the lower edge the
//! string is bistable, and an attack can hold a double slip for hundreds of
//! milliseconds before Helmholtz motion wins; the performer can't use those
//! forces for its notes.

use strings_dsp::analysis::{Regime, bow_steady, classify};
use strings_dsp::{BowedString, ForceLimits, InstrumentSpec, StringSpec};

const SPEEDS: [f32; 4] = [0.05, 0.1, 0.2, 0.4];
const COLUMNS: usize = 12;
const ROWS: usize = 28;
/// Force range of the map, as F / (Z·v).
const NORMALIZED_FORCE: (f32, f32) = (0.3, 400.0);
const BETA: (f32, f32) = (0.04, 0.25);
/// A band edge needs this many consecutive Helmholtz rows.
const RUN: usize = 3;
/// Seconds after the bow starts (its speed ramps up over 0.05 s) by which a
/// cell must be Helmholtz, and from which the steady state is judged.
const SETTLED: f32 = 0.15;
const STEADY: f32 = 0.4;

struct Cell {
    string: usize,
    speed: f32,
    beta: f32,
    force: f32,
}

/// Simulates every cell on its string (of `strings`); results in the order
/// given.
fn regimes(spec: &InstrumentSpec, strings: &[StringSpec], fs: f32, cells: &[Cell]) -> Vec<Regime> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = cells.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = cells
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    let mut bowed: Vec<BowedString> = strings
                        .iter()
                        .map(|s| {
                            let mut b = BowedString::new(s, spec.friction, fs, s.frequency);
                            b.set_bow_hair(spec.hair);
                            b.set_thermal_friction(spec.thermal);
                            b
                        })
                        .collect();
                    chunk
                        .iter()
                        .map(|c| {
                            let s = &mut bowed[c.string];
                            s.reset();
                            s.set_bow_position(c.beta);
                            let frames = bow_steady(s, fs, c.speed, c.force, 0.8, 0.05);
                            let period = fs / strings[c.string].frequency;
                            let (settled, steady) =
                                ((SETTLED * fs) as usize, (STEADY * fs) as usize);
                            let early = classify(&frames[settled..steady], period);
                            let late = classify(&frames[steady..], period);
                            // Helmholtz only late: it settled too slowly.
                            if late == Regime::Helmholtz {
                                early
                            } else {
                                late
                            }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker panicked"))
            .collect()
    })
}

fn log_lerp(a: f32, b: f32, t: f32) -> f32 {
    (a.ln() + (b.ln() - a.ln()) * t).exp()
}

/// Least-squares fit of y = c·β^α in log-log space.
fn fit(points: &[(f32, f32)]) -> (f32, f32) {
    let n = points.len() as f64;
    let pts: Vec<(f64, f64)> = points
        .iter()
        .map(|&(b, y)| ((b as f64).ln(), (y as f64).ln()))
        .collect();
    let (mx, my) = pts
        .iter()
        .fold((0.0, 0.0), |(a, b), (x, y)| (a + x / n, b + y / n));
    let sxx: f64 = pts.iter().map(|(x, _)| (x - mx).powi(2)).sum();
    let sxy: f64 = pts.iter().map(|(x, y)| (x - mx) * (y - my)).sum();
    let alpha = sxy / sxx;
    ((my - alpha * mx).exp() as f32, alpha as f32)
}

/// Maps the instrument's strings and, on an instrument with an extension, the
/// lowest string stopped at the gates ([`strings_dsp::Extension::gated`]),
/// fitted last.
pub fn run(spec: &InstrumentSpec, fs: f32) {
    let mut strings = spec.strings.to_vec();
    let mut names: Vec<String> = strings
        .iter()
        .map(|s| format!("{} string", s.name))
        .collect();
    if let Some(extension) = spec.extension {
        strings.push(extension.gated(&spec.strings[0]));
        names.push(format!("{} string at the gates", spec.strings[0].name));
    }
    let betas: Vec<f32> = (0..COLUMNS)
        .map(|c| log_lerp(BETA.0, BETA.1, c as f32 / (COLUMNS - 1) as f32))
        .collect();
    let norm: Vec<f32> = (0..ROWS)
        .map(|r| {
            log_lerp(
                NORMALIZED_FORCE.0,
                NORMALIZED_FORCE.1,
                r as f32 / (ROWS - 1) as f32,
            )
        })
        .collect();
    let mut cells = Vec::new();
    for (string, s) in strings.iter().enumerate() {
        for &speed in &SPEEDS {
            for &beta in &betas {
                for &n in &norm {
                    cells.push(Cell {
                        string,
                        speed,
                        beta,
                        force: n * s.impedance() * speed,
                    });
                }
            }
        }
    }
    println!(
        "{}: mapping {} cells ({} strings × {} speeds × {COLUMNS} β × {ROWS} forces)...",
        spec.name,
        cells.len(),
        strings.len(),
        SPEEDS.len()
    );
    let result = regimes(spec, &strings, fs, &cells);

    // Band edges per (string, speed, β) column, as F / (Z·v).
    let mut per_string = vec![(Vec::new(), Vec::new()); strings.len()];
    for (col, chunk) in result.chunks(ROWS).enumerate() {
        let beta = betas[col % COLUMNS];
        let string = col / (COLUMNS * SPEEDS.len());
        let h: Vec<bool> = chunk.iter().map(|r| *r == Regime::Helmholtz).collect();
        let runs: Vec<(usize, usize)> = runs(&h)
            .into_iter()
            .filter(|(s, e)| e - s + 1 >= RUN)
            .collect();
        let (Some(&(first, _)), Some(&(_, last))) = (runs.first(), runs.last()) else {
            continue;
        };
        if first > 0 {
            per_string[string].0.push((beta, norm[first]));
        }
        if last < ROWS - 1 {
            per_string[string].1.push((beta, norm[last]));
        }
    }
    let helmholtz = result.iter().filter(|r| **r == Regime::Helmholtz).count();
    println!("Helmholtz cells: {helmholtz} of {}", result.len());
    let mut limits = Vec::new();
    for (i, (lo, hi)) in per_string.iter().enumerate() {
        if lo.len() < 3 || hi.len() < 3 {
            println!("{}: not enough bracketed edges to fit", names[i]);
            return;
        }
        let (lower, lower_exponent) = fit(lo);
        let (upper, upper_exponent) = fit(hi);
        println!(
            "  {}: lower F/(Zv) = {lower:.3}·β^{lower_exponent:.2} ({} cols), upper = {upper:.3}·β^{upper_exponent:.2} ({} cols)",
            names[i],
            lo.len(),
            hi.len()
        );
        limits.push(ForceLimits {
            lower,
            lower_exponent,
            upper,
            upper_exponent,
        });
    }
    let (open, gated) = limits.split_at(spec.strings.len());
    println!("\nFitted limits, lowest string first:\n{open:#.3?}");
    if let Some(gated) = gated.first() {
        println!("At the extension's gates (`Extension::force_limits`):\n{gated:#.3?}");
    }
    println!(
        "(Schelleng's F_max is {:.2}·β^-1)",
        2.0 / (spec.friction.mu_s - spec.friction.mu_d)
    );
    // How reliably does each band position give Helmholtz motion?
    let positions = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
    let check_betas = [0.06, 0.08, 0.1, 0.13, 0.16, 0.2];
    let mut cells = Vec::new();
    for (string, s) in strings.iter().enumerate() {
        for &speed in &SPEEDS {
            for &beta in &check_betas {
                for &p in &positions {
                    cells.push(Cell {
                        string,
                        speed,
                        beta,
                        force: limits[string].force(s.impedance(), speed, beta, p),
                    });
                }
            }
        }
    }
    let result = regimes(spec, &strings, fs, &cells);
    println!(
        "\nHelmholtz fraction by band position (β {check_betas:?}, all speeds), per string and overall:"
    );
    let per_string = result.len() / strings.len();
    for (k, p) in positions.iter().enumerate() {
        let fraction = |range: std::ops::Range<usize>| {
            let (hits, total) = range
                .filter(|i| i % positions.len() == k)
                .fold((0, 0), |(h, t), i| {
                    (h + usize::from(result[i] == Regime::Helmholtz), t + 1)
                });
            100.0 * hits as f32 / total as f32
        };
        let columns: Vec<String> = (0..strings.len())
            .map(|s| format!("{:>4.0}%", fraction(s * per_string..(s + 1) * per_string)))
            .collect();
        println!(
            "  p = {p:.1}: {}  | {:>3.0}%",
            columns.join(" "),
            fraction(0..result.len())
        );
    }
}

/// Runs of `true` as (first, last) index pairs.
fn runs(h: &[bool]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &v) in h.iter().enumerate() {
        match (v, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i - 1));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, h.len() - 1));
    }
    out
}
