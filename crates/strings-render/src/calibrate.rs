//! Calibrates an instrument's bow-force band (`ForceLimits`, PLAN.md 4.2) from
//! simulation: maps the Helmholtz region of every string over bow speed, β and
//! force, fits both edges as `F = c·Z·v·β^α`, then checks how often each
//! position in the fitted band gives Helmholtz motion.

use strings_dsp::analysis::{Regime, bow_steady, classify};
use strings_dsp::{BowedString, ForceLimits, InstrumentSpec};

const SPEEDS: [f32; 4] = [0.05, 0.1, 0.2, 0.4];
const COLUMNS: usize = 12;
const ROWS: usize = 28;
/// Force range of the map, as F / (Z·v).
const NORMALIZED_FORCE: (f32, f32) = (0.3, 400.0);
const BETA: (f32, f32) = (0.04, 0.25);
/// A band edge needs this many consecutive Helmholtz rows.
const RUN: usize = 3;

struct Cell {
    string: usize,
    speed: f32,
    beta: f32,
    force: f32,
}

/// Simulates every cell on its string; results in the order given.
fn regimes(spec: &InstrumentSpec, fs: f32, cells: &[Cell]) -> Vec<Regime> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = cells.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = cells
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    let mut strings: Vec<BowedString> = spec
                        .strings
                        .iter()
                        .map(|s| {
                            let mut b = BowedString::new(s, spec.friction, fs, s.frequency);
                            b.set_bow_hair(spec.hair);
                            b
                        })
                        .collect();
                    chunk
                        .iter()
                        .map(|c| {
                            let s = &mut strings[c.string];
                            s.reset();
                            s.set_bow_position(c.beta);
                            let frames = bow_steady(s, fs, c.speed, c.force, 0.8, 0.05);
                            let period = fs / spec.strings[c.string].frequency;
                            classify(&frames[(0.4 * fs) as usize..], period)
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

pub fn run(spec: &InstrumentSpec, fs: f32) {
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
    for (string, s) in spec.strings.iter().enumerate() {
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
        spec.strings.len(),
        SPEEDS.len()
    );
    let result = regimes(spec, fs, &cells);

    // Band edges per (string, speed, β) column, as F / (Z·v).
    let mut per_string = vec![(Vec::new(), Vec::new()); spec.strings.len()];
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
            println!(
                "{} string: not enough bracketed edges to fit",
                spec.strings[i].name
            );
            return;
        }
        let (lower, lower_exponent) = fit(lo);
        let (upper, upper_exponent) = fit(hi);
        println!(
            "  {} string: lower F/(Zv) = {lower:.3}·β^{lower_exponent:.2} ({} cols), upper = {upper:.3}·β^{upper_exponent:.2} ({} cols)",
            spec.strings[i].name,
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
    println!("\nFitted limits, lowest string first:\n{limits:#.3?}");
    println!(
        "(Schelleng's F_max is {:.2}·β^-1)",
        2.0 / (spec.friction.mu_s - spec.friction.mu_d)
    );
    // How reliably does each band position give Helmholtz motion?
    let positions = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
    let check_betas = [0.06, 0.08, 0.1, 0.13, 0.16, 0.2];
    let mut cells = Vec::new();
    for (string, s) in spec.strings.iter().enumerate() {
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
    let result = regimes(spec, fs, &cells);
    println!(
        "\nHelmholtz fraction by band position (β {check_betas:?}, all speeds), per string and overall:"
    );
    let per_string = result.len() / spec.strings.len();
    for (k, p) in positions.iter().enumerate() {
        let fraction = |range: std::ops::Range<usize>| {
            let (hits, total) = range
                .filter(|i| i % positions.len() == k)
                .fold((0, 0), |(h, t), i| {
                    (h + usize::from(result[i] == Regime::Helmholtz), t + 1)
                });
            100.0 * hits as f32 / total as f32
        };
        let strings: Vec<String> = (0..spec.strings.len())
            .map(|s| format!("{:>4.0}%", fraction(s * per_string..(s + 1) * per_string)))
            .collect();
        println!(
            "  p = {p:.1}: {}  | {:>3.0}%",
            strings.join(" "),
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
