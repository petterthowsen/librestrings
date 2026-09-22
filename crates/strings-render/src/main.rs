//! Offline renderer for the string models: writes WAV (and optionally CSV of
//! internal signals), and maps playability as a Schelleng diagram.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

mod measured;
use strings_dsp::analysis::{Regime, bow_steady, classify, classify_bridge_force};
use strings_dsp::presets::{reference, violin};
use strings_dsp::{
    BowInput, BowedString, DampingCurve, FrictionParams, Loss, StringFrame, StringSpec,
    TorsionSpec, schelleng_limits,
};

#[derive(Parser)]
#[command(about = "Render physically modeled strings offline")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Pluck a free string (no bow): checks tuning and decay by ear.
    Pluck {
        #[command(flatten)]
        common: Common,
        #[arg(long, default_value_t = 3.0)]
        seconds: f32,
    },
    /// Bow one note: attack, sustain, then stop or lift the bow.
    Bow {
        #[command(flatten)]
        common: Common,
        /// Bow force (N). Defaults to 0.3 × Schelleng's F_max, inside the simulated
        /// Helmholtz band (whose lower edge sits well above the theoretical F_min).
        #[arg(long)]
        force: Option<f32>,
        /// Bow speed (m/s).
        #[arg(long, default_value_t = 0.1)]
        speed: f32,
        /// Bow position as a fraction of the string from the bridge.
        #[arg(long, default_value_t = 0.1)]
        beta: f32,
        /// Sustain length (s), excluding attack and release.
        #[arg(long, default_value_t = 2.0)]
        seconds: f32,
        #[arg(long, default_value_t = 0.08)]
        attack: f32,
        #[arg(long, default_value_t = 0.15)]
        release: f32,
        /// How the note ends: `lift` lets the string ring, `stop` halts the bow on the string.
        #[arg(long, default_value = "lift")]
        end: End,
        /// Also write per-sample internal signals to this CSV file.
        #[arg(long)]
        csv: Option<PathBuf>,
    },
    /// Sweep bow force and position, print the regime map next to Schelleng's prediction.
    Schelleng {
        /// Open string to bow: G, D, A or E.
        #[arg(long, default_value = "A")]
        string: String,
        #[arg(long, default_value_t = 48_000.0)]
        sample_rate: f32,
        #[arg(long, default_value_t = 0.1)]
        speed: f32,
        #[arg(long, default_value_t = 16)]
        rows: usize,
        #[arg(long, default_value_t = 12)]
        cols: usize,
        /// Override the string's bridge lowpass pole (at 48 kHz).
        #[arg(long)]
        loss_lowpass: Option<f32>,
    },
    /// Compare the model with a measured Schelleng diagram (mdw cello string A T1),
    /// point by point. Fetch the data with scripts/fetch-reference-data.sh.
    Measured {
        #[arg(long, default_value = "data/reference/schelleng-typeA-s1-T1")]
        data: PathBuf,
        /// Use the Phase 1 one-pole loss instead of the measured damping curve,
        /// with this bridge lowpass pole (at 48 kHz; default 0.5).
        #[arg(long)]
        loss_lowpass: Option<f32>,
        /// Use the Phase 1 one-pole loss instead of the measured damping curve,
        /// with this decay time of the fundamental (s; default 32).
        #[arg(long)]
        t60: Option<f32>,
        /// Override the exponent of the measured damping curve, which sets how it
        /// extrapolates above the measured modes (1.7 kHz).
        #[arg(long)]
        damping_exponent: Option<f32>,
        /// Override the static friction coefficient.
        #[arg(long)]
        mu_s: Option<f32>,
        /// Override the dynamic friction coefficient.
        #[arg(long)]
        mu_d: Option<f32>,
        /// Override the friction curve's slip-speed scale (m/s).
        #[arg(long)]
        v0: Option<f32>,
        /// Override the bending stiffness EI (N·m²); 0 makes the string flexible.
        #[arg(long)]
        bending_stiffness: Option<f32>,
        /// Leave out torsional waves.
        #[arg(long)]
        no_torsion: bool,
        /// Override the torsional impedance at the string surface (kg/s).
        #[arg(long)]
        torsion_impedance: Option<f32>,
        /// Override the torsional fundamental, as a multiple of the transverse one.
        #[arg(long)]
        torsion_ratio: Option<f32>,
        /// Override the torsional quality factor.
        #[arg(long)]
        torsion_q: Option<f32>,
        /// Write every point's parameters and both regimes to this CSV file.
        #[arg(long)]
        csv: Option<PathBuf>,
    },
}

#[derive(Args)]
struct Common {
    /// Violin string: G, D, A or E.
    #[arg(long, default_value = "A")]
    string: String,
    /// Semitones above the open string (stopped note).
    #[arg(long, default_value_t = 0.0)]
    semitones: f32,
    #[arg(long, default_value_t = 48_000.0)]
    sample_rate: f32,
    /// Override the string's bridge lowpass pole (at 48 kHz).
    #[arg(long)]
    loss_lowpass: Option<f32>,
    #[arg(long, short)]
    out: PathBuf,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum End {
    Lift,
    Stop,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Pluck { common, seconds } => {
            let (spec, mut string) = build(&common)?;
            string.set_bow_position(0.1);
            let pulse = (0.0005 * common.sample_rate) as usize;
            let frames: Vec<StringFrame> = (0..(seconds * common.sample_rate) as usize)
                .map(|i| string.process(BowInput::default(), if i < pulse { 0.1 } else { 0.0 }))
                .collect();
            println!("{} string at {:.2} Hz", spec.name, string.frequency());
            write_wav(&common.out, &frames, common.sample_rate)?;
        }
        Command::Bow {
            common,
            force,
            speed,
            beta,
            seconds,
            attack,
            release,
            end,
            csv,
        } => {
            let (spec, mut string) = build(&common)?;
            string.set_bow_position(beta);
            let (f_min, f_max) = limits(&string, beta, speed);
            let force = force.unwrap_or(0.3 * f_max);
            println!(
                "{} string at {:.2} Hz, Z = {:.3} kg/s; force {force:.4} N (Schelleng {f_min:.4}..{f_max:.4} N)",
                spec.name,
                string.frequency(),
                string.impedance()
            );
            let frames = render_note(
                &mut string,
                common.sample_rate,
                speed,
                force,
                attack,
                seconds,
                release,
                end,
            );
            write_wav(&common.out, &frames, common.sample_rate)?;
            if let Some(csv) = csv {
                write_csv(&csv, &frames, common.sample_rate)?;
                println!("wrote {}", csv.display());
            }
        }
        Command::Schelleng {
            string,
            sample_rate,
            speed,
            rows,
            cols,
            loss_lowpass,
        } => {
            let spec = with_loss(
                violin::string(&string).ok_or("unknown string")?,
                loss_lowpass,
            );
            schelleng(&spec, sample_rate, speed, rows, cols);
        }
        Command::Measured {
            data,
            loss_lowpass,
            t60,
            mu_s,
            mu_d,
            v0,
            damping_exponent,
            bending_stiffness,
            no_torsion,
            torsion_impedance,
            torsion_ratio,
            torsion_q,
            csv,
        } => {
            let base = reference::MONOCHORD_CELLO_G_A_T1;
            let torsion = base.torsion.filter(|_| !no_torsion).map(|t| TorsionSpec {
                impedance: torsion_impedance.unwrap_or(t.impedance),
                frequency: torsion_ratio.map_or(t.frequency, |r| r * base.frequency),
                q: torsion_q.unwrap_or(t.q),
            });
            let loss = match base.loss {
                _ if t60.is_some() || loss_lowpass.is_some() => Loss::OnePole {
                    t60: t60.unwrap_or(32.0),
                    lowpass: loss_lowpass.unwrap_or(0.5),
                },
                Loss::Measured(curve) => Loss::Measured(DampingCurve {
                    exponent: damping_exponent.unwrap_or(curve.exponent),
                    ..curve
                }),
                one_pole => one_pole,
            };
            let spec = StringSpec {
                loss,
                bending_stiffness: bending_stiffness.unwrap_or(base.bending_stiffness),
                torsion,
                ..base
            };
            let d = FrictionParams::default();
            let friction = FrictionParams {
                mu_s: mu_s.unwrap_or(d.mu_s),
                mu_d: mu_d.unwrap_or(d.mu_d),
                v0: v0.unwrap_or(d.v0),
            };
            measured::run(&data, &spec, friction, csv.as_deref())?;
        }
    }
    Ok(())
}

/// Overrides the bridge lowpass pole of a string with one-pole loss.
fn with_loss(spec: &StringSpec, loss_lowpass: Option<f32>) -> StringSpec {
    let loss = match (spec.loss, loss_lowpass) {
        (Loss::OnePole { t60, .. }, Some(lowpass)) => Loss::OnePole { t60, lowpass },
        (loss, _) => loss,
    };
    StringSpec { loss, ..*spec }
}

fn build(common: &Common) -> Result<(StringSpec, BowedString), Box<dyn std::error::Error>> {
    let spec = violin::string(&common.string).ok_or("unknown string (use G, D, A or E)")?;
    let spec = with_loss(spec, common.loss_lowpass);
    let mut string = BowedString::new(&spec, FrictionParams::default(), common.sample_rate, 50.0);
    string.set_frequency(spec.frequency * 2f32.powf(common.semitones / 12.0));
    Ok((spec, string))
}

fn limits(string: &BowedString, beta: f32, speed: f32) -> (f32, f32) {
    schelleng_limits(
        string.impedance(),
        string.loop_gain(),
        &FrictionParams::default(),
        beta,
        speed,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_note(
    string: &mut BowedString,
    fs: f32,
    speed: f32,
    force: f32,
    attack: f32,
    sustain: f32,
    release: f32,
    end: End,
) -> Vec<StringFrame> {
    let (a, s, r) = (
        (attack * fs) as usize,
        (sustain * fs) as usize,
        (release * fs) as usize,
    );
    let tail = fs as usize;
    let smooth = |t: f32| 0.5 - 0.5 * (std::f32::consts::PI * t.clamp(0.0, 1.0)).cos();
    (0..a + s + r + tail)
        .map(|i| {
            let (v, f) = if i < a {
                (speed * smooth(i as f32 / a as f32), force)
            } else if i < a + s {
                (speed, force)
            } else {
                let fade = 1.0 - smooth((i - a - s) as f32 / r as f32);
                match end {
                    End::Lift => (speed, force * fade),
                    End::Stop => (speed * fade, force),
                }
            };
            string.process(
                BowInput {
                    velocity: v,
                    force: f,
                },
                0.0,
            )
        })
        .collect()
}

fn schelleng(spec: &StringSpec, fs: f32, speed: f32, rows: usize, cols: usize) {
    let mut string = BowedString::new(spec, FrictionParams::default(), fs, 50.0);
    let period = fs / spec.frequency;
    let betas: Vec<f32> = (0..cols)
        .map(|c| log_lerp(0.04, 0.25, c as f32 / (cols - 1) as f32))
        .collect();
    // Force range spans the predicted limits over all betas, with a margin.
    let (lo, _) = limits(&string, betas[cols - 1], speed);
    let (_, hi) = limits(&string, betas[0], speed);
    let forces: Vec<f32> = (0..rows)
        .map(|r| log_lerp(hi * 4.0, lo / 4.0, r as f32 / (rows - 1) as f32))
        .collect();

    println!(
        "{} string, bow speed {speed} m/s. Left: simulated (contact state). \
         Middle: simulated (bridge force only). Right: Schelleng prediction.",
        spec.name
    );
    println!("H = Helmholtz, M = multi-slip, R = raucous, . = no oscillation\n");
    let mut agree = 0;
    let mut classifiers_agree = 0;
    for &force in &forces {
        let mut sim = String::new();
        let mut from_bridge = String::new();
        let mut theory = String::new();
        for &beta in &betas {
            string.reset();
            string.set_bow_position(beta);
            let frames = bow_steady(&mut string, fs, speed, force, 0.8, 0.05);
            let steady = &frames[(0.4 * fs) as usize..];
            let regime = classify(steady, period);
            let force_only: Vec<f32> = steady.iter().map(|f| f.bridge_force).collect();
            let regime_bf = classify_bridge_force(&force_only, period);
            classifiers_agree += usize::from(regime == regime_bf);
            let (f_min, f_max) = limits(&string, beta, speed);
            let predicted = if force > f_max {
                'R'
            } else if force < f_min {
                'M'
            } else {
                'H'
            };
            agree += usize::from((regime == Regime::Helmholtz) == (predicted == 'H'));
            sim.push(regime.symbol());
            sim.push(' ');
            from_bridge.push(regime_bf.symbol());
            from_bridge.push(' ');
            theory.push(predicted);
            theory.push(' ');
        }
        println!("{force:>9.4} N  {sim}   {from_bridge}   {theory}");
    }
    print!("{:>13}", "β:");
    for b in &betas {
        print!("{:<2}", format!("{:.0}", b * 100.0).chars().last().unwrap());
    }
    println!(
        "\n{:>13}{:.2} .. {:.2} (log spaced)\n\nHelmholtz / not-Helmholtz agreement: {:.0}%\n\
         Contact-state vs bridge-force classifier agreement: {:.0}%",
        "",
        betas[0],
        betas[cols - 1],
        100.0 * agree as f32 / (rows * cols) as f32,
        100.0 * classifiers_agree as f32 / (rows * cols) as f32
    );
}

fn log_lerp(a: f32, b: f32, t: f32) -> f32 {
    (a.ln() + (b.ln() - a.ln()) * t).exp()
}

/// Writes bridge force as 32-bit float mono, peak-normalized to -1 dBFS.
fn write_wav(
    path: &Path,
    frames: &[StringFrame],
    fs: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let peak = frames
        .iter()
        .fold(0.0_f32, |m, f| m.max(f.bridge_force.abs()));
    let gain = if peak > 0.0 { 0.891 / peak } else { 1.0 };
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: fs as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for f in frames {
        w.write_sample(f.bridge_force * gain)?;
    }
    w.finalize()?;
    println!("wrote {} (peak bridge force {peak:.4} N)", path.display());
    Ok(())
}

fn write_csv(path: &Path, frames: &[StringFrame], fs: f32) -> std::io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    writeln!(
        w,
        "time,bridge_force,bow_point_velocity,friction_force,slipping"
    )?;
    for (i, f) in frames.iter().enumerate() {
        writeln!(
            w,
            "{:.6},{},{},{},{}",
            i as f32 / fs,
            f.bridge_force,
            f.bow_point_velocity,
            f.friction_force,
            u8::from(f.state.is_slipping())
        )?;
    }
    w.flush()
}
