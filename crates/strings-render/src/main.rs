//! Offline renderer for the string models: writes WAV (and optionally CSV of
//! internal signals), and maps playability as a Schelleng diagram.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

mod calibrate;
mod compare;
mod measured;
mod score;
use strings_dsp::analysis::{Regime, bow_steady, classify, classify_bridge_force};
use strings_dsp::presets::{cello, reference, violin};
use strings_dsp::{
    Absorption, BowHair, BowInput, BowedString, DampingCurve, Fingering, FrictionParams,
    Humanization, InstrumentSpec, Loss, MAX_PLAYERS, Performer, PerformerSettings, Placement,
    Polyphony, RoomPreset, Section, Stage, StageSettings, StringFrame, StringSpec, TorsionSpec,
    schelleng_limits,
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
        #[arg(long, default_value = "violin")]
        instrument: Family,
        /// Open string to bow (violin: G D A E; cello: C G D A).
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
    /// Play a score through the performer and body. SCORE is a file (format in
    /// score.rs) or a built-in: scale, legato, staccato, phrase, doublestops,
    /// ostinato, sul (cello), or violin-scale, violin-legato, violin-staccato,
    /// violin-phrase, violin-doublestops, violin-ostinato, violin-sul,
    /// violin-tasto.
    Play {
        score: String,
        #[arg(long, default_value = "cello")]
        instrument: Family,
        #[arg(long, default_value_t = 48_000.0)]
        sample_rate: f32,
        /// Seconds rendered after the last event.
        #[arg(long, default_value_t = 2.0)]
        tail: f32,
        /// Position in the Helmholtz band of normal pressure, 0–1 (default:
        /// the performer's). The score's `pressure` moves from there.
        #[arg(long)]
        pressure: Option<f32>,
        /// Where the left hand plays: nut (with open strings), mid or bridge.
        /// The score's `fingering` changes it.
        #[arg(long, default_value = "nut", value_parser = score::fingering)]
        fingering: Fingering,
        /// Play overlapping notes as double stops where one hand can (the
        /// score's `poly` changes it).
        #[arg(long)]
        double_stops: bool,
        /// String samples per output sample: 1, or 2 to run the strings at
        /// twice the sample rate (default: the performer's).
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
        oversampling: Option<u8>,
        /// Players in the section (1 is the solo instrument), each with its own
        /// humanization. With more than one, or --stage, the output is stereo
        /// through the stage.
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=12))]
        players: u8,
        /// Play a solo instrument on the stage too (stereo), not dry.
        #[arg(long)]
        stage: bool,
        /// The section's centre (m): x to the audience's right, y upstage from
        /// the front of the stage (default: the instrument's usual seat).
        #[arg(long, allow_negative_numbers = true)]
        x: Option<f32>,
        #[arg(long)]
        y: Option<f32>,
        /// The area the section fills (m).
        #[arg(long)]
        width: Option<f32>,
        #[arg(long)]
        depth: Option<f32>,
        /// studio, chamber, concert or scoring.
        #[arg(long, default_value = "chamber", value_parser = room_preset)]
        room: RoomPreset,
        /// low, medium or high.
        #[arg(long, default_value = "medium", value_parser = absorption)]
        absorption: Absorption,
        /// Distance of the mics in front of the stage (m).
        #[arg(long, default_value_t = StageSettings::default().mic_distance)]
        mic_distance: f32,
        /// Early reflections, 0 (off) to 1.
        #[arg(long, default_value_t = 1.0)]
        reflections: f32,
        #[arg(long, short)]
        out: PathBuf,
        /// Also write the summed bridge force (before the body) to this WAV
        /// file. Solo only.
        #[arg(long, conflicts_with = "players")]
        bridge_out: Option<PathBuf>,
    },
    /// Fit an instrument's bow-force band (the performer's force mapping) to
    /// simulated Schelleng maps of all its strings.
    Calibrate {
        #[arg(long, default_value = "cello")]
        instrument: Family,
        #[arg(long, default_value_t = 48_000.0)]
        sample_rate: f32,
        /// Override the bow hair's stiffness (N/m), with --hair-damping.
        #[arg(long, requires = "hair_damping")]
        hair_stiffness: Option<f32>,
        /// Override the bow hair's damping (kg/s), with --hair-stiffness.
        #[arg(long, requires = "hair_stiffness")]
        hair_damping: Option<f32>,
    },
    /// Compare the solo cello with recorded notes (University of Iowa, arco):
    /// each note played again on the same string at the same dynamic, both
    /// measured the same way. Fetch with scripts/fetch-reference-data.sh iowa-cello.
    Compare {
        #[arg(long, default_value = "data/reference/iowa-cello")]
        data: PathBuf,
        /// Only these dynamics (pp, mf, ff; repeat or separate by commas).
        #[arg(long, value_delimiter = ',')]
        dynamics: Vec<String>,
        /// Only this string (C, G, D, A).
        #[arg(long)]
        string: Option<String>,
        /// The model's vibrato control, 0–1 (default: matched to each recording).
        #[arg(long)]
        vibrato: Option<f32>,
        /// MIDI velocity of the model's strokes, 1–127.
        #[arg(long, default_value_t = 64.0)]
        velocity: f32,
        /// Where the listening files go (recording, then model, per note).
        #[arg(long, short, default_value = "out/compare")]
        out: PathBuf,
        /// Write every note's measurements to this CSV file.
        #[arg(long)]
        csv: Option<PathBuf>,
        /// Measure the model's bridge force (the strings, before the body)
        /// instead of its output.
        #[arg(long)]
        bridge: bool,
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
        /// Make the bow hair compliant with this stiffness (N/m).
        #[arg(long, requires = "hair_damping")]
        hair_stiffness: Option<f32>,
        /// Bow hair damping (kg/s), with --hair-stiffness.
        #[arg(long, requires = "hair_stiffness")]
        hair_damping: Option<f32>,
        /// Write every point's parameters and both regimes to this CSV file.
        #[arg(long)]
        csv: Option<PathBuf>,
    },
}

#[derive(Args)]
struct Common {
    #[arg(long, default_value = "violin")]
    instrument: Family,
    /// Open string (violin: G D A E; cello: C G D A).
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
enum Family {
    Violin,
    Cello,
}

impl Family {
    fn instrument(self) -> &'static InstrumentSpec {
        match self {
            Family::Violin => &violin::INSTRUMENT,
            Family::Cello => &cello::INSTRUMENT,
        }
    }
}

/// An open string and the bow hair it is played with.
fn open_string(family: Family, name: &str) -> Result<(StringSpec, Option<BowHair>), String> {
    let found = match family {
        // The Phase 1 reference map: a rigid bow (the instrument has hair).
        Family::Violin => violin::string(name).map(|s| (*s, None)),
        Family::Cello => cello::string_named(name).map(|s| (*s, cello::INSTRUMENT.hair)),
    };
    found.ok_or_else(|| format!("unknown string {name}"))
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
            instrument,
            string,
            sample_rate,
            speed,
            rows,
            cols,
            loss_lowpass,
        } => {
            let (spec, hair) = open_string(instrument, &string)?;
            let spec = with_loss(&spec, loss_lowpass);
            schelleng(&spec, hair, sample_rate, speed, rows, cols);
        }
        Command::Play {
            score,
            instrument,
            sample_rate,
            tail,
            pressure,
            fingering,
            double_stops,
            oversampling,
            players,
            stage,
            x,
            y,
            width,
            depth,
            room,
            absorption,
            mic_distance,
            reflections,
            out,
            bridge_out,
        } => {
            let text = match score.as_str() {
                "scale" => include_str!("../scores/scale.score").to_string(),
                "legato" => include_str!("../scores/legato.score").to_string(),
                "staccato" => include_str!("../scores/staccato.score").to_string(),
                "phrase" => include_str!("../scores/phrase.score").to_string(),
                "doublestops" => include_str!("../scores/doublestops.score").to_string(),
                "ostinato" => include_str!("../scores/ostinato.score").to_string(),
                "sul" => include_str!("../scores/sul.score").to_string(),
                "violin-scale" => include_str!("../scores/violin-scale.score").to_string(),
                "violin-legato" => include_str!("../scores/violin-legato.score").to_string(),
                "violin-staccato" => include_str!("../scores/violin-staccato.score").to_string(),
                "violin-phrase" => include_str!("../scores/violin-phrase.score").to_string(),
                "violin-doublestops" => {
                    include_str!("../scores/violin-doublestops.score").to_string()
                }
                "violin-ostinato" => include_str!("../scores/violin-ostinato.score").to_string(),
                "violin-sul" => include_str!("../scores/violin-sul.score").to_string(),
                "violin-tasto" => include_str!("../scores/violin-tasto.score").to_string(),
                path => std::fs::read_to_string(path)?,
            };
            let spec = instrument.instrument();
            let mut events = score::parse(&text, spec.strings.map(|s| s.name))?;
            let mut settings = PerformerSettings::for_instrument(spec);
            if let Some(p) = pressure {
                settings.pressure = p;
            }
            if let Some(o) = oversampling {
                settings.oversampling = o as usize;
            }
            let polyphony = if double_stops {
                Polyphony::DoubleStops
            } else {
                Polyphony::Mono
            };
            // The options come first; the score may change them.
            let modes = [
                (0.0, score::Event::Fingering(fingering)),
                (0.0, score::Event::Polyphony(polyphony)),
            ];
            events.splice(0..0, modes);
            if players > 1 || stage {
                let seat = Placement::for_instrument(spec);
                let placement = Placement {
                    x: x.unwrap_or(seat.x),
                    y: y.unwrap_or(seat.y),
                    width: width.unwrap_or(seat.width),
                    depth: depth.unwrap_or(seat.depth),
                };
                let stage = StageSettings {
                    room,
                    absorption,
                    mic_distance,
                    reflections,
                };
                play_section(
                    (spec, settings),
                    &events,
                    players as usize,
                    (stage, placement),
                    sample_rate,
                    tail,
                    &out,
                )?;
            } else {
                play(
                    (spec, settings),
                    &events,
                    sample_rate,
                    tail,
                    &out,
                    bridge_out.as_deref(),
                )?;
            }
        }
        Command::Compare {
            data,
            dynamics,
            string,
            vibrato,
            velocity,
            out,
            csv,
            bridge,
        } => compare::run(&compare::Options {
            data,
            out,
            dynamics,
            string,
            vibrato,
            velocity: velocity / 127.0,
            csv,
            bridge,
        })?,
        Command::Calibrate {
            instrument,
            sample_rate,
            hair_stiffness,
            hair_damping,
        } => {
            let mut spec = *instrument.instrument();
            if let (Some(stiffness), Some(damping)) = (hair_stiffness, hair_damping) {
                spec.hair = Some(BowHair { stiffness, damping });
            }
            calibrate::run(&spec, sample_rate)
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
            hair_stiffness,
            hair_damping,
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
            let hair = hair_stiffness
                .zip(hair_damping)
                .map(|(stiffness, damping)| BowHair { stiffness, damping });
            measured::run(&data, &spec, friction, hair, csv.as_deref())?;
        }
    }
    Ok(())
}

fn play(
    (spec, settings): (&InstrumentSpec, PerformerSettings),
    events: &[(f32, score::Event)],
    fs: f32,
    tail: f32,
    out: &Path,
    bridge_out: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    use score::Event;
    let mut performer = Performer::new(spec, settings, fs);
    let end = events.last().map_or(0.0, |e| e.0) + tail;
    let total = (end * fs) as usize;
    let mut output = Vec::with_capacity(total);
    let mut bridge = Vec::with_capacity(total);
    let mut next = 0;
    let started = std::time::Instant::now();
    for i in 0..total {
        while next < events.len() && events[next].0 * fs <= i as f32 {
            match events[next].1 {
                Event::On(note, velocity) => performer.note_on(note, velocity),
                Event::Off(note) => performer.note_off(note),
                Event::Dynamics(v) => performer.set_dynamics(v),
                Event::Vibrato(v) => performer.set_vibrato(v),
                Event::Pressure(v) => performer.set_pressure(v),
                Event::BowLift(b) => performer.set_bow_lift(b),
                Event::Fingering(f) => performer.set_fingering(f),
                Event::String(s) => performer.set_string(s),
                Event::Polyphony(p) => performer.set_polyphony(p),
            }
            next += 1;
        }
        let frame = performer.process_frame();
        output.push(frame.output);
        bridge.push(frame.bridge_force);
    }
    let elapsed = started.elapsed().as_secs_f32();
    println!(
        "rendered {end:.1} s in {elapsed:.2} s ({:.1}% of real time)",
        100.0 * elapsed / end
    );
    let peak = output.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!("output peak {peak:.3} (not normalized)");
    write_samples(out, &output, fs, 1.0)?;
    if let Some(path) = bridge_out {
        write_samples(
            path,
            &bridge,
            fs,
            0.891 / bridge.iter().fold(1e-9f32, |m, v| m.max(v.abs())),
        )?;
    }
    Ok(())
}

/// `play` for a section of `players`, with the default humanization, on the
/// stage: stereo.
fn play_section(
    (spec, settings): (&InstrumentSpec, PerformerSettings),
    events: &[(f32, score::Event)],
    players: usize,
    (stage_settings, placement): (StageSettings, Placement),
    fs: f32,
    tail: f32,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use score::Event;
    let mut section = Section::new(spec, settings, Humanization::default(), fs);
    section.set_players(players);
    let mut stage = Stage::new(stage_settings, placement, settings.seed, fs);
    stage.set_players(players);
    let end = events.last().map_or(0.0, |e| e.0) + tail;
    let total = (end * fs) as usize;
    let mut output = Vec::with_capacity(total);
    let mut each = [0.0; MAX_PLAYERS];
    let mut next = 0;
    let started = std::time::Instant::now();
    for i in 0..total {
        while next < events.len() && events[next].0 * fs <= i as f32 {
            match events[next].1 {
                Event::On(note, velocity) => section.note_on(note, velocity),
                Event::Off(note) => section.note_off(note),
                Event::Dynamics(v) => section.set_dynamics(v),
                Event::Vibrato(v) => section.set_vibrato(v),
                Event::Pressure(v) => section.set_pressure(v),
                Event::BowLift(b) => section.set_bow_lift(b),
                Event::Fingering(f) => section.set_fingering(f),
                Event::String(s) => section.set_string(s),
                Event::Polyphony(p) => section.set_polyphony(p),
            }
            next += 1;
        }
        section.process(&mut each);
        output.extend(stage.process(&each));
    }
    let elapsed = started.elapsed().as_secs_f32();
    println!(
        "rendered {players} players, {end:.1} s in {elapsed:.2} s ({:.1}% of real time)",
        100.0 * elapsed / end
    );
    let peak = output.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!("output peak {peak:.3} (not normalized)");
    write_interleaved(out, &output, 2, fs)
}

/// Writes 32-bit float samples, `channels` interleaved.
fn write_interleaved(
    path: &Path,
    samples: &[f32],
    channels: u16,
    fs: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let spec = hound::WavSpec {
        channels,
        sample_rate: fs as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        w.write_sample(s)?;
    }
    w.finalize()?;
    println!("wrote {}", path.display());
    Ok(())
}

fn room_preset(name: &str) -> Result<RoomPreset, String> {
    match name {
        "studio" => Ok(RoomPreset::Studio),
        "chamber" => Ok(RoomPreset::ChamberHall),
        "concert" => Ok(RoomPreset::ConcertHall),
        "scoring" => Ok(RoomPreset::ScoringStage),
        _ => Err(format!(
            "unknown room {name:?}: studio, chamber, concert or scoring"
        )),
    }
}

fn absorption(name: &str) -> Result<Absorption, String> {
    match name {
        "low" => Ok(Absorption::Low),
        "medium" => Ok(Absorption::Medium),
        "high" => Ok(Absorption::High),
        _ => Err(format!("unknown absorption {name:?}: low, medium or high")),
    }
}

/// Writes 32-bit float mono samples times `gain`.
fn write_samples(
    path: &Path,
    samples: &[f32],
    fs: f32,
    gain: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: fs as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        w.write_sample(s * gain)?;
    }
    w.finalize()?;
    println!("wrote {}", path.display());
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
    let (spec, hair) = open_string(common.instrument, &common.string)?;
    let spec = with_loss(&spec, common.loss_lowpass);
    let mut string = BowedString::new(&spec, FrictionParams::default(), common.sample_rate, 50.0);
    string.set_bow_hair(hair);
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

fn schelleng(
    spec: &StringSpec,
    hair: Option<BowHair>,
    fs: f32,
    speed: f32,
    rows: usize,
    cols: usize,
) {
    let mut string = BowedString::new(spec, FrictionParams::default(), fs, 50.0);
    string.set_bow_hair(hair);
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
