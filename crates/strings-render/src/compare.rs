//! Compare the model with recorded notes: the University of Iowa MIS arco
//! sets, anechoic, one mono file per string, dynamic and range, each a
//! chromatic run of single long notes with the bow lifted at the end. The
//! violin, viola and cello sets are recorded the same way. Fetch one with
//! `scripts/fetch-reference-data.sh iowa-cello` (or `iowa-viola`,
//! `iowa-violin`, `iowa-bass`).
//!
//! Each recorded note is played again by the performer on the same string at
//! the matching dynamic, as long as the recorded bow stroke, and both go
//! through the same measurements: attack time, ring-off, pitch, vibrato, the
//! sustain's harmonic spectrum and its noise. The pairs are also written as
//! WAV (recording, then model, level-matched) for listening.

use std::path::{Path, PathBuf};

use strings_dsp::analysis::{cents, measure_frequency};
use strings_dsp::{InstrumentSpec, Performer, PerformerSettings, ThermalFriction};

use crate::{Family, envelope};

/// Envelope hop and window (s).
const HOP: f32 = 0.005;
const WINDOW: f32 = 0.010;
/// Harmonic bands: partials 1, 2–3, 4–7, 8–15 and 16 up.
const BANDS: [(usize, usize); 5] = [(1, 1), (2, 3), (4, 7), (8, 15), (16, usize::MAX)];
/// Noise bands (Hz): the inter-harmonic power below 1 kHz, 1–2, 2–4 and 4–8 kHz.
const NOISE_BANDS: [(f32, f32); 4] = [
    (0.0, 1000.0),
    (1000.0, 2000.0),
    (2000.0, 4000.0),
    (4000.0, 8000.0),
];
/// Highest partial frequency measured (Hz).
const TOP: f32 = 10_000.0;

/// An instrument's recorded set: the file prefix, Iowa's "sul" string letters
/// with the preset string each is played on, and the high-pass both signals
/// get before measuring (the recordings carry rumble below 20 Hz, a few dB
/// under a quiet note's fundamental, so it sits below the instrument's lowest
/// note — the bass's open C1 is 32.7 Hz).
struct Recorded {
    /// The set's directory under `data/reference/`.
    dir: &'static str,
    /// Every file is named `Prefix.arco.<a>.<b>.<range>.mono.wav`, where
    /// `<a>` and `<b>` are the dynamic and the string, in either order.
    prefix: &'static str,
    /// Iowa's letters and the preset string each is played on. The bass's set
    /// has both the extended C string and the E string stopped at the gates
    /// (a real extension's gates); both are the model's lowest string, which
    /// plays E1 as a stopped note (PLAN.md "The bass's C extension").
    strings: &'static [(&'static str, usize)],
    highpass: f32,
}

fn recorded(family: Family) -> Recorded {
    match family {
        Family::Violin => Recorded {
            dir: "iowa-violin",
            prefix: "Violin",
            strings: &[("G", 0), ("D", 1), ("A", 2), ("E", 3)],
            highpass: 40.0,
        },
        Family::Viola => Recorded {
            dir: "iowa-viola",
            prefix: "Viola",
            strings: &[("C", 0), ("G", 1), ("D", 2), ("A", 3)],
            highpass: 40.0,
        },
        Family::Cello => Recorded {
            dir: "iowa-cello",
            prefix: "Cello",
            strings: &[("C", 0), ("G", 1), ("D", 2), ("A", 3)],
            highpass: 40.0,
        },
        Family::Bass => Recorded {
            dir: "iowa-bass",
            prefix: "Bass",
            strings: &[("C", 0), ("E", 0), ("A", 1), ("D", 2), ("G", 3)],
            highpass: 15.0,
        },
    }
}

pub struct Options {
    /// Which instrument's set to compare (Iowa, arco).
    pub instrument: Family,
    /// Where the recordings are; the set's default directory if `None`.
    pub data: Option<PathBuf>,
    /// Read the recordings at this rate, if their headers disagree with the
    /// audio (the Iowa viola set is 96 kHz audio in 44.1 kHz files). Both the
    /// model and the measurements then run at this rate.
    pub file_rate: Option<f32>,
    pub out: PathBuf,
    /// Only these dynamics (pp, mf, ff); all if empty.
    pub dynamics: Vec<String>,
    /// Only this string, by its recorded letter (C, G, D, A, E); all if empty.
    pub string: Option<String>,
    /// Vibrato control for the model, 0–1; by default matched to each
    /// recorded note's measured depth.
    pub vibrato: Option<f32>,
    /// MIDI velocity of the model's strokes, 0–1.
    pub velocity: f32,
    pub csv: Option<PathBuf>,
    /// Measure the model's bridge force instead of its output: whether a
    /// difference comes from the strings or the body.
    pub bridge: bool,
    /// Override the width of the bow hair in contact (m).
    pub bow_width: Option<f32>,
    /// Override the bow noise's level and bandwidth (Hz).
    pub bow_noise: Option<f32>,
    pub noise_cutoff: Option<f32>,
    /// Thermal friction instead of the friction curve.
    pub thermal: Option<ThermalFriction>,
}

/// What both the recording and the model are measured for.
#[derive(Clone, Debug)]
struct Features {
    /// Median sustain level (dBFS of the 10 ms RMS).
    level: f32,
    /// From 30 dB below the sustain level to 3 dB below it (s).
    attack: f32,
    /// Length of the stroke: from the attack's start to the last time the
    /// level is within 6 dB of the sustain (s).
    stroke: f32,
    /// From there until the level is 30 dB below the sustain (s); the end of
    /// the signal if it never gets there.
    ring: f32,
    f0: f32,
    /// Vibrato: peak depth (cents) and rate (Hz).
    vibrato_depth: f32,
    vibrato_rate: f32,
    /// Harmonic levels in `BANDS`, dB relative to all harmonic power.
    bands: [f32; 5],
    /// Power-weighted mean partial number.
    centroid: f32,
    /// Harmonic to non-harmonic power in the sustain (dB, median over frames).
    hnr: f32,
    /// The noise's spectrum: power halfway between the partials in
    /// `NOISE_BANDS`, dB relative to all harmonic power.
    noise: [f32; 4],
    /// Every partial up to `TOP`: frequency (Hz), level (dB relative to all
    /// harmonic power) and how far it stands above the noise halfway to its
    /// neighbours (dB). For the spectral envelope (`envelope`).
    partials: Vec<(f32, f32, f32)>,
    /// Samples of the attack's start and the ring's end.
    span: (usize, usize),
}

struct Take {
    dynamic: &'static str,
    /// The preset string it is played on.
    string: usize,
    /// Iowa's letter for it, as in the file name.
    letter: &'static str,
    /// The run's first note (MIDI), from the file name.
    first: u8,
    path: PathBuf,
}

const DYNAMICS: [(&str, f32); 3] = [("pp", 0.1), ("mf", 0.5), ("ff", 0.9)];

pub fn run(opts: &Options) -> Result<(), Box<dyn std::error::Error>> {
    let set = recorded(opts.instrument);
    let data = opts
        .data
        .clone()
        .unwrap_or_else(|| PathBuf::from("data/reference").join(set.dir));
    let takes = find_takes(&data, &set, opts)?;
    if takes.is_empty() {
        return Err(format!(
            "no Iowa {} files in {} (scripts/fetch-reference-data.sh {})",
            set.prefix.to_lowercase(),
            data.display(),
            set.dir
        )
        .into());
    }
    std::fs::create_dir_all(&opts.out)?;
    let mut spec = *opts.instrument.instrument();
    if let (Some(width), Some(hair)) = (opts.bow_width, &mut spec.hair) {
        hair.width = width;
    }
    let mut spec = crate::with_bow_noise(&spec, opts.bow_noise, opts.noise_cutoff);
    spec.thermal = opts.thermal;
    // The instrument's own gesture values (the cello's are the default):
    // without them a viola or bass note is bowed with the cello's positions,
    // speed and band.
    let settings = PerformerSettings::for_instrument(&spec);
    let mut csv = opts.csv.as_ref().map(|_| {
        String::from(
            "dynamic,string,note,source,level_db,attack_s,stroke_s,ring_s,cents,vib_cents,vib_hz,\
             band1,band2_3,band4_7,band8_15,band16up,centroid,hnr_db\n",
        )
    });
    // (dynamic, recording, model) for the summary.
    let mut pairs: Vec<(&str, Features, Features)> = Vec::new();
    println!(
        "{:<3} {:<4} {:>5} | {:>6} {:>6} | {:>5} {:>5} | {:>6} {:>6} | {:>7} | {:>9} | {:>31} | {:>5} | {:>5}",
        "dyn",
        "note",
        "src",
        "attack",
        "ring",
        "cents",
        "",
        "vib c",
        "Hz",
        "level",
        "",
        "harmonic bands dB (1 2-3 4-7 8-15 16+)",
        "centr",
        "hnr"
    );
    for take in &takes {
        let (mut samples, header_rate) = read_mono(&take.path)?;
        let fs = opts.file_rate.unwrap_or(header_rate);
        highpass(&mut samples, fs, set.highpass);
        let dyn_value = DYNAMICS.iter().find(|d| d.0 == take.dynamic).unwrap().1;
        let mut listening = Vec::new();
        // The run is chromatic: each note is the one after the last. A
        // segment is taken as the note it is nearest to, of that one, the
        // next (a note missing from the run) and the last (a repeat, skipped);
        // the player's intonation goes up to half a semitone off.
        let mut midi = take.first;
        for (start, end) in segments(&samples, fs) {
            let x = &samples[start..end];
            let found = [midi, midi + 1, midi.saturating_sub(1)]
                .into_iter()
                .filter_map(|m| analyze(x, fs, frequency(m)).map(|f| (m, f)))
                .filter(|(m, f)| cents(f.f0, frequency(*m)).abs() <= 75.0)
                .min_by(|a, b| {
                    let off = |(m, f): &(u8, Features)| cents(f.f0, frequency(*m)).abs();
                    off(a).total_cmp(&off(b))
                });
            let rec = match found {
                Some((m, rec)) if m >= midi => {
                    if m > midi {
                        println!(
                            "{:<3} {:<4}   rec | not in the run",
                            take.dynamic,
                            name(midi)
                        );
                    }
                    midi = m;
                    rec
                }
                _ => {
                    println!(
                        "{:<3} {:<4}   rec | segment at {:.1} s: no note or a repeat, skipped",
                        take.dynamic,
                        name(midi),
                        start as f32 / fs
                    );
                    continue;
                }
            };
            let nominal = frequency(midi);
            let vibrato = opts.vibrato.unwrap_or_else(|| {
                let full = 100.0 * settings.vibrato_depth;
                (rec.vibrato_depth / full).clamp(0.0, 1.0)
            });
            let mut y = render(
                &spec,
                &settings,
                midi,
                take.string,
                dyn_value,
                vibrato,
                opts.velocity,
                rec.stroke,
                rec.ring + 1.0,
                opts.bridge,
                fs,
            );
            highpass(&mut y, fs, set.highpass);
            let Some(model) = analyze(&y, fs, nominal) else {
                println!("{:<3} {:<4} model: no note found", take.dynamic, name(midi));
                midi += 1;
                continue;
            };
            for (src, f) in [("rec", &rec), ("model", &model)] {
                let b = f.bands;
                println!(
                    "{:<3} {:<4} {:>5} | {:>4.0}ms {:>5.2}s | {:>+5.0} {:>5} | {:>6.0} {:>6.1} | {:>5.1}dB | {:>9} | {:>5.1} {:>5.1} {:>5.1} {:>5.1} {:>5.1} | {:>5.1} | {:>5.1}",
                    take.dynamic,
                    name(midi),
                    src,
                    1000.0 * f.attack,
                    f.ring,
                    cents(f.f0, nominal),
                    "",
                    f.vibrato_depth,
                    f.vibrato_rate,
                    f.level,
                    "",
                    b[0],
                    b[1],
                    b[2],
                    b[3],
                    b[4],
                    f.centroid,
                    f.hnr
                );
                if let Some(csv) = csv.as_mut() {
                    csv.push_str(&format!(
                        "{},{},{},{src},{:.2},{:.4},{:.3},{:.3},{:.1},{:.1},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2}\n",
                        take.dynamic,
                        take.letter,
                        name(midi),
                        f.level,
                        f.attack,
                        f.stroke,
                        f.ring,
                        cents(f.f0, nominal),
                        f.vibrato_depth,
                        f.vibrato_rate,
                        b[0],
                        b[1],
                        b[2],
                        b[3],
                        b[4],
                        f.centroid,
                        f.hnr
                    ));
                }
            }
            append_matched(&mut listening, x, &rec, fs);
            append_matched(&mut listening, &y, &model, fs);
            listening.extend(std::iter::repeat_n(0.0, (0.6 * fs) as usize));
            pairs.push((take.dynamic, rec, model));
            midi += 1;
        }
        let path = opts
            .out
            .join(format!("{}-sul{}.wav", take.dynamic, take.letter));
        write_wav(&path, &listening, fs)?;
    }
    summary(&pairs);
    envelope_summary(&pairs, &opts.out.join("envelope.csv"))?;
    if let (Some(path), Some(csv)) = (&opts.csv, csv) {
        std::fs::write(path, csv)?;
        println!("wrote {}", path.display());
    }
    println!(
        "listening files in {}: each note recorded, then the model, at the same level",
        opts.out.display()
    );
    Ok(())
}

/// Median of model − recording per dynamic, for the measures that compare
/// directly, plus the level spread between dynamics.
fn summary(pairs: &[(&str, Features, Features)]) {
    println!("\nmedians per dynamic (recording / model):");
    println!(
        "{:<3} {:>3} | {:>13} | {:>11} | {:>11} | {:>41} | {:>11} | {:>11} | {:>11}",
        "dyn",
        "n",
        "attack ms",
        "ring s",
        "vib cents",
        "harmonic bands dB (1 2-3 4-7 8-15 16+)",
        "centroid",
        "hnr dB",
        "level dB"
    );
    for (dynamic, _) in DYNAMICS {
        let set: Vec<_> = pairs.iter().filter(|p| p.0 == dynamic).collect();
        if set.is_empty() {
            continue;
        }
        let med = |f: &dyn Fn(&Features) -> f32| {
            (
                median(set.iter().map(|p| f(&p.1)).collect()),
                median(set.iter().map(|p| f(&p.2)).collect()),
            )
        };
        let attack = med(&|f| 1000.0 * f.attack);
        let ring = med(&|f| f.ring);
        let vib = med(&|f| f.vibrato_depth);
        let bands: Vec<_> = (0..5).map(|i| med(&|f| f.bands[i])).collect();
        let centroid = med(&|f| f.centroid);
        let hnr = med(&|f| f.hnr);
        let noise: Vec<_> = (0..4).map(|i| med(&|f| f.noise[i])).collect();
        let level = med(&|f| f.level);
        let pair = |(a, b): (f32, f32), d: usize| format!("{a:.d$} / {b:.d$}");
        let bands = bands
            .iter()
            .map(|&(a, b)| format!("{a:.0}/{b:.0}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{:<3} {:>3} | {:>13} | {:>11} | {:>11} | {:>41} | {:>11} | {:>11} | {:>11}",
            dynamic,
            set.len(),
            pair(attack, 0),
            pair(ring, 2),
            pair(vib, 0),
            bands,
            pair(centroid, 1),
            pair(hnr, 1),
            pair(level, 1),
        );
        println!(
            "{:<3} {:>3} | noise between partials dB (<1k 1-2k 2-4k 4-8k Hz): {}",
            "",
            "",
            noise
                .iter()
                .map(|&(a, b)| format!("{a:.0}/{b:.0}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}

/// The spectral envelope, recording − model, per dynamic and over all of
/// them (`envelope`): the distance before and after taking it off, and the
/// envelope itself in third octaves. Every sixth-octave bin goes to `path`.
fn envelope_summary(
    pairs: &[(&str, Features, Features)],
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let points = |rec: &Features, model: &Features| -> envelope::Points {
        rec.partials
            .iter()
            .zip(&model.partials)
            .filter(|(r, m)| r.2 >= envelope::MIN_SNR && m.2 >= envelope::MIN_SNR)
            .map(|(r, m)| (r.0, r.1 - m.1))
            .collect()
    };
    let mut fits = Vec::new();
    for dynamic in DYNAMICS.iter().map(|d| d.0).chain(["all"]) {
        let notes: Vec<_> = pairs
            .iter()
            .filter(|p| dynamic == "all" || p.0 == dynamic)
            .map(|p| points(&p.1, &p.2))
            .collect();
        if !notes.is_empty() {
            fits.push((dynamic, envelope::fit(&notes)));
        }
    }
    println!(
        "\nspectral envelope, recording − model (dB, mean 0; per-note distance: median RMS \
         over sixth octaves, as is / after taking the envelope off):"
    );
    let shown: Vec<usize> = (0..envelope::BINS).step_by(2).collect();
    print!("{:<3} {:>11} |", "dyn", "distance");
    for &b in &shown {
        let f = envelope::centre(b);
        if f < 1000.0 {
            print!("{:>5.0}", f);
        } else {
            print!("{:>4.1}k", f / 1000.0);
        }
    }
    println!();
    for (dynamic, fit) in &fits {
        print!(
            "{:<3} {:>4.1} / {:>4.1} |",
            dynamic, fit.distance, fit.residual
        );
        for &b in &shown {
            match fit.h[b] {
                Some(h) => print!("{:>+5.0}", h),
                None => print!("{:>5}", "."),
            }
        }
        println!();
    }
    let mut csv = String::from("hz");
    for (dynamic, _) in &fits {
        csv.push_str(&format!(",{dynamic},{dynamic}_notes"));
    }
    csv.push('\n');
    for b in 0..envelope::BINS {
        csv.push_str(&format!("{:.1}", envelope::centre(b)));
        for (_, fit) in &fits {
            match fit.h[b] {
                Some(h) => csv.push_str(&format!(",{h:.2},{}", fit.notes[b])),
                None => csv.push_str(&format!(",,{}", fit.notes[b])),
            }
        }
        csv.push('\n');
    }
    std::fs::write(path, csv)?;
    println!("wrote {}", path.display());
    Ok(())
}

fn find_takes(
    dir: &Path,
    set: &Recorded,
    opts: &Options,
) -> Result<Vec<Take>, Box<dyn std::error::Error>> {
    let mut takes = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(takes);
    };
    for entry in entries {
        let path = entry?.path();
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        // Cello.arco.mf.sulG.G2Gb3.mono.wav, Bass.arco.sulE.ff.E1B1.mono.wav:
        // the dynamic and the string come in either order, set by set.
        let parts: Vec<&str> = file.split('.').collect();
        if parts.len() != 7 || parts[0] != set.prefix || parts[1] != "arco" || parts[6] != "wav" {
            continue;
        }
        let dynamic = |f: &str| DYNAMICS.iter().find(|d| d.0 == f).map(|d| d.0);
        let (dynamic, sound) = match (dynamic(parts[2]), dynamic(parts[3])) {
            (Some(d), None) => (d, parts[3]),
            (None, Some(d)) => (d, parts[2]),
            _ => continue,
        };
        let Some(sound) = sound.strip_prefix("sul") else {
            continue;
        };
        let Some(&(letter, string)) = set.strings.iter().find(|(s, _)| *s == sound) else {
            continue;
        };
        let Some(first) = first_note(parts[4]) else {
            continue;
        };
        if !opts.dynamics.is_empty() && !opts.dynamics.iter().any(|d| d == dynamic) {
            continue;
        }
        if opts
            .string
            .as_ref()
            .is_some_and(|s| !s.eq_ignore_ascii_case(letter))
        {
            continue;
        }
        takes.push(Take {
            dynamic,
            string,
            letter,
            first,
            path,
        });
    }
    takes.sort_by(|a, b| {
        (
            DYNAMICS.iter().position(|d| d.0 == a.dynamic),
            a.string,
            a.letter,
            &a.path,
        )
            .cmp(&(
                DYNAMICS.iter().position(|d| d.0 == b.dynamic),
                b.string,
                b.letter,
                &b.path,
            ))
    });
    Ok(takes)
}

/// One note played like the recording: from rest, on `string`, off the
/// string at the end (the recorded bow lifts), with `tail` seconds after.
/// With `bridge`, the strings' bridge force instead of the body's output.
#[allow(clippy::too_many_arguments)]
fn render(
    spec: &InstrumentSpec,
    settings: &PerformerSettings,
    note: u8,
    string: usize,
    dynamics: f32,
    vibrato: f32,
    velocity: f32,
    stroke: f32,
    tail: f32,
    bridge: bool,
    fs: f32,
) -> Vec<f32> {
    let mut p = Performer::new(spec, *settings, fs);
    p.set_string(Some(string));
    p.set_dynamics(dynamics);
    p.set_vibrato(vibrato);
    // Let the controls settle before the note.
    for _ in 0..(0.3 * fs) as usize {
        p.process();
    }
    let sample = |p: &mut Performer| {
        let frame = p.process_frame();
        if bridge {
            frame.bridge_force
        } else {
            frame.output
        }
    };
    let mut y = Vec::new();
    y.extend(std::iter::repeat_n(0.0, (0.1 * fs) as usize));
    p.note_on(note, velocity);
    for _ in 0..(stroke * fs) as usize {
        y.push(sample(&mut p));
    }
    p.note_off(note);
    for _ in 0..(tail * fs) as usize {
        y.push(sample(&mut p));
    }
    y
}

/// Notes in a chromatic run: stretches above the noise floor (the 10th
/// percentile of the level, digital silence left out, + 15 dB), separated by at least 0.25 s below it and at least
/// 0.5 s long. Returns sample ranges with 50 ms of lead-in.
fn segments(x: &[f32], fs: f32) -> Vec<(usize, usize)> {
    let env = envelope(x, fs);
    // The recordings' gaps are digital silence, which isn't the noise floor.
    let mut sorted: Vec<f32> = env.iter().copied().filter(|&e| e > -150.0).collect();
    sorted.sort_by(f32::total_cmp);
    let floor = sorted.get(sorted.len() / 10).copied().unwrap_or(-150.0);
    let threshold = floor + 15.0;
    let gap = (0.25 / HOP) as usize;
    let mut out = Vec::new();
    let mut i = 0;
    while i < env.len() {
        if env[i] <= threshold {
            i += 1;
            continue;
        }
        let start = i;
        let mut quiet = 0;
        let mut end = i;
        while i < env.len() && quiet < gap {
            if env[i] > threshold {
                quiet = 0;
                end = i;
            } else {
                quiet += 1;
            }
            i += 1;
        }
        if (end - start) as f32 * HOP >= 0.5 {
            let hop = (HOP * fs) as usize;
            let lead = (0.05 * fs) as usize;
            out.push((
                (start * hop).saturating_sub(lead),
                ((end + 1) * hop + lead).min(x.len()),
            ));
        }
    }
    out
}

/// Level in dB of the RMS over `WINDOW`, every `HOP`.
fn envelope(x: &[f32], fs: f32) -> Vec<f32> {
    let hop = (HOP * fs) as usize;
    let window = (WINDOW * fs) as usize;
    (0..x.len().saturating_sub(window) / hop)
        .map(|k| {
            let s = &x[k * hop..k * hop + window];
            let ms = s.iter().map(|v| v * v).sum::<f32>() / window as f32;
            10.0 * (ms + 1e-20).log10()
        })
        .collect()
}

/// Measures one note (with silence around it), whose pitch lies within a
/// semitone of `nominal`.
fn analyze(x: &[f32], fs: f32, nominal: f32) -> Option<Features> {
    let (lo, hi) = (nominal * 0.94, nominal * 1.06);
    let env = envelope(x, fs);
    let peak = env.iter().copied().fold(f32::MIN, f32::max);
    let first = env.iter().position(|&e| e > peak - 10.0)?;
    let last = env.iter().rposition(|&e| e > peak - 10.0)?;
    let level = median(env[first..=last].to_vec());
    let onset = env[..=first]
        .iter()
        .rposition(|&e| e < level - 30.0)
        .map_or(0, |i| i + 1);
    let attacked = onset + env[onset..].iter().position(|&e| e >= level - 3.0)?;
    let release = env.iter().rposition(|&e| e >= level - 6.0)?;
    let ended = release
        + env[release..]
            .iter()
            .position(|&e| e < level - 30.0)
            .unwrap_or(env.len() - release);
    let t = |frames: usize| frames as f32 * HOP;
    let hop = (HOP * fs) as usize;

    // The sustain: from 0.4 s after the attack to 0.2 s before the release,
    // or the middle third of the stroke if that is too short.
    let (mut s0, mut s1) = ((attacked + 80) * hop, release.saturating_sub(40) * hop);
    if s1 < s0 + (0.3 * fs) as usize {
        let third = (release - onset) * hop / 3;
        (s0, s1) = (onset * hop + third, onset * hop + 2 * third);
    }
    let sustain = &x[s0..s1.min(x.len())];
    let coarse = median(pitch_track(sustain, fs, lo, hi, 0.02));
    if !coarse.is_finite() {
        return None;
    }
    let f0 = measure_frequency(sustain, fs, coarse);
    let f0 = if (f0 / coarse - 1.0).abs() < 0.03 {
        f0
    } else {
        coarse
    };
    let (vibrato_depth, vibrato_rate) = vibrato(&pitch_track(sustain, fs, lo, hi, 0.01), f0);
    let (bands, centroid, hnr, noise, partials) = spectrum(sustain, fs, f0);
    Some(Features {
        level,
        attack: t(attacked - onset),
        stroke: t(release - onset),
        ring: t(ended - release),
        f0,
        vibrato_depth,
        vibrato_rate,
        bands,
        centroid,
        hnr,
        noise,
        partials,
        span: (
            onset * hop,
            (ended * hop + (WINDOW * fs) as usize).min(x.len()),
        ),
    })
}

/// Fundamental between `lo` and `hi` (at most a semitone either side of a
/// note) every `step` seconds: YIN on 3 periods of `lo`, NaN where it finds
/// no clear period.
fn pitch_track(x: &[f32], fs: f32, lo: f32, hi: f32, step: f32) -> Vec<f32> {
    let min_lag = (fs / hi).floor() as usize;
    let max_lag = (fs / lo).ceil() as usize;
    let window = 3 * max_lag;
    let hop = (step * fs) as usize;
    let mut d = vec![0.0f32; max_lag + 2];
    let mut out = Vec::new();
    let mut k = 0;
    while k + window + max_lag + 1 < x.len() {
        let frame = &x[k..k + window + max_lag + 1];
        for (lag, dl) in d.iter_mut().enumerate().skip(1) {
            *dl = (0..window)
                .map(|i| {
                    let e = frame[i] - frame[i + lag];
                    e * e
                })
                .sum();
        }
        // Cumulative mean normalized difference.
        let mut sum = 0.0;
        let mut cmnd = vec![1.0f32; d.len()];
        for lag in 1..d.len() {
            sum += d[lag];
            cmnd[lag] = if sum > 0.0 {
                d[lag] * lag as f32 / sum
            } else {
                1.0
            };
        }
        // The search spans a semitone either way, so no octave is in it: the
        // deepest dip is the period, if it is deep and not at an edge.
        let lo_lag = min_lag.max(2);
        let lag = (lo_lag..=max_lag)
            .min_by(|&a, &b| cmnd[a].total_cmp(&cmnd[b]))
            .unwrap_or(lo_lag);
        let found = if cmnd[lag] < 0.4 && lag > lo_lag && lag < max_lag {
            let (a, b, c) = (cmnd[lag - 1], cmnd[lag], cmnd[lag + 1]);
            let denom = a - 2.0 * b + c;
            let shift = if denom.abs() > 1e-12 {
                0.5 * (a - c) / denom
            } else {
                0.0
            };
            fs / (lag as f32 + shift)
        } else {
            f32::NAN
        };
        out.push(found);
        k += hop;
    }
    out
}

/// Peak depth (cents) and rate (Hz) of the vibrato in a pitch track taken
/// every 10 ms: the track in cents minus its 0.4 s moving average.
fn vibrato(track: &[f32], f0: f32) -> (f32, f32) {
    let c: Vec<f32> = track
        .iter()
        .filter(|v| v.is_finite())
        .map(|&v| cents(v, f0))
        .filter(|v| v.abs() < 150.0)
        .collect();
    let half = 20;
    if c.len() < 2 * half + 20 {
        return (0.0, 0.0);
    }
    let wobble: Vec<f32> = (half..c.len() - half)
        .map(|i| c[i] - c[i - half..=i + half].iter().sum::<f32>() / (2 * half + 1) as f32)
        .collect();
    let rms = (wobble.iter().map(|v| v * v).sum::<f32>() / wobble.len() as f32).sqrt();
    let crossings = wobble
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    let rate = crossings as f32 / 2.0 / (wobble.len() as f32 * 0.01);
    // A slow wander has few crossings; call it no vibrato below 3 Hz.
    if rate < 3.0 {
        (std::f32::consts::SQRT_2 * rms, 0.0)
    } else {
        (std::f32::consts::SQRT_2 * rms, rate)
    }
}

/// Harmonic band levels (dB relative to all harmonic power), the mean
/// partial number weighted by power, the harmonic-to-noise ratio (dB) and the
/// noise's spectrum (see [`Features::noise`]) and the partials (see
/// [`Features::partials`]), over Hann frames 6 periods long, each at its own
/// fundamental.
///
/// The noise is measured halfway between partials: in a Hann window 6
/// periods long those frequencies sit on the window's zeros for every
/// partial, so the partials don't leak into them.
#[allow(clippy::type_complexity)]
fn spectrum(x: &[f32], fs: f32, f0: f32) -> ([f32; 5], f32, f32, [f32; 4], Vec<(f32, f32, f32)>) {
    let len = (6.0 * fs / f0) as usize;
    let hop = len / 3;
    let partials = ((TOP.min(0.45 * fs)) / f0) as usize;
    let mut power = vec![0.0f64; partials + 1];
    let mut between = [0.0f64; 4];
    // Power halfway below each partial, and above the last.
    let mut halfway = vec![0.0f64; partials + 2];
    let mut hnrs = Vec::new();
    let w: Vec<f32> = (0..len)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / len as f32).cos())
        .collect();
    let w2 = w.iter().map(|v| v * v).sum::<f32>();
    let mut k = 0;
    while k + len <= x.len() {
        let frame = &x[k..k + len];
        let local = pitch_track(frame, fs, f0 * 0.94, f0 * 1.06, 1.0)
            .first()
            .copied()
            .filter(|f| f.is_finite() && (f / f0 - 1.0).abs() < 0.06)
            .unwrap_or(f0);
        let total = frame
            .iter()
            .zip(&w)
            .map(|(v, w)| (v * w) * (v * w))
            .sum::<f32>()
            / w2;
        let mut harmonic = 0.0f32;
        for (n, p) in power.iter_mut().enumerate().skip(1) {
            let a = windowed_amplitude(frame, &w, fs, n as f32 * local);
            let pw = a * a / 2.0;
            *p += pw as f64;
            harmonic += pw;
            let mid = (n as f32 - 0.5) * local;
            let a = windowed_amplitude(frame, &w, fs, mid);
            halfway[n] += (a * a / 2.0) as f64;
            if let Some(b) = NOISE_BANDS
                .iter()
                .position(|&(lo, hi)| (lo..hi).contains(&mid))
            {
                between[b] += (a * a / 2.0) as f64;
            }
        }
        let a = windowed_amplitude(frame, &w, fs, (partials as f32 + 0.5) * local);
        halfway[partials + 1] += (a * a / 2.0) as f64;
        // Capped at 60 dB: the model has no noise at all.
        hnrs.push(10.0 * (harmonic / (total - harmonic).max(1e-6 * total)).log10());
        k += hop;
    }
    let all: f64 = power.iter().sum();
    let mut bands = [0.0f32; 5];
    for (b, &(lo, hi)) in bands.iter_mut().zip(&BANDS) {
        let p: f64 = power
            .iter()
            .enumerate()
            .filter(|(n, _)| (lo..=hi).contains(n))
            .map(|(_, p)| p)
            .sum();
        *b = 10.0 * ((p / all).max(1e-12)).log10() as f32;
    }
    let centroid = power
        .iter()
        .enumerate()
        .map(|(n, p)| n as f64 * p)
        .sum::<f64>()
        / all;
    let noise = between.map(|p| 10.0 * ((p / all).max(1e-12)).log10() as f32);
    let db = |p: f64| 10.0 * p.max(1e-30).log10() as f32;
    let partials = (1..=partials)
        .map(|n| {
            let floor = 0.5 * (halfway[n] + halfway[n + 1]);
            (n as f32 * f0, db(power[n] / all), db(power[n]) - db(floor))
        })
        .collect();
    (bands, centroid as f32, median(hnrs), noise, partials)
}

/// Amplitude of the component at `frequency` in a frame already weighed by
/// the window `w` (a Hann window's coherent gain is 1/2).
fn windowed_amplitude(x: &[f32], w: &[f32], fs: f32, frequency: f32) -> f32 {
    let omega = std::f64::consts::TAU * frequency as f64 / fs as f64;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, (&v, &w)) in x.iter().zip(w).enumerate() {
        let (s, c) = (omega * i as f64).sin_cos();
        re += (w * v) as f64 * c;
        im -= (w * v) as f64 * s;
    }
    (4.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32
}

/// The note's span from `f`, scaled to a sustain level of −20 dBFS.
fn append_matched(out: &mut Vec<f32>, x: &[f32], f: &Features, fs: f32) {
    let gain = 10f32.powf((-20.0 - f.level) / 20.0);
    let (a, b) = f.span;
    let a = a.saturating_sub((0.05 * fs) as usize);
    out.extend(x[a..b].iter().map(|v| v * gain));
    out.extend(std::iter::repeat_n(0.0, (0.3 * fs) as usize));
}

/// Fourth-order Butterworth high-pass at `frequency`, in place.
fn highpass(x: &mut [f32], fs: f32, frequency: f32) {
    let w = std::f32::consts::TAU * frequency / fs;
    // The two sections of a fourth-order Butterworth.
    for q in [0.541_196_1, 1.306_563] {
        let alpha = w.sin() / (2.0 * q);
        let cos = w.cos();
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = (
            (1.0 + cos) / 2.0 / a0,
            -(1.0 + cos) / a0,
            (1.0 + cos) / 2.0 / a0,
        );
        let (a1, a2) = (-2.0 * cos / a0, (1.0 - alpha) / a0);
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        for v in x.iter_mut() {
            let y = b0 * *v + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
            (x2, x1, y2, y1) = (x1, *v, y1, y);
            *v = y;
        }
    }
}

fn median(mut v: Vec<f32>) -> f32 {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

fn frequency(midi: u8) -> f32 {
    440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0)
}

/// The first note of a range in an Iowa file name, such as `G2Gb3` (MIDI).
fn first_note(range: &str) -> Option<u8> {
    let b = range.as_bytes();
    let pitch = match b.first()? {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let (accidental, rest) = match b.get(1)? {
        b'b' => (-1, 2),
        b'#' => (1, 2),
        _ => (0, 1),
    };
    let octave = (*b.get(rest)? as char).to_digit(10)? as i32;
    u8::try_from(12 * (octave + 1) + pitch + accidental).ok()
}

fn name(midi: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
    ];
    format!("{}{}", NAMES[midi as usize % 12], midi as i32 / 12 - 1)
}

fn read_mono(path: &Path) -> Result<(Vec<f32>, f32), Box<dyn std::error::Error>> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let channels = spec.channels as usize;
    let all: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u32 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()?
        }
    };
    let mono = all
        .chunks(channels)
        .map(|c| c.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok((mono, spec.sample_rate as f32))
}

fn write_wav(path: &Path, samples: &[f32], fs: f32) -> Result<(), Box<dyn std::error::Error>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: fs as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(instrument: Family) -> Options {
        Options {
            instrument,
            data: None,
            file_rate: None,
            out: PathBuf::new(),
            dynamics: Vec::new(),
            string: None,
            vibrato: None,
            velocity: 0.5,
            csv: None,
            bridge: false,
            bow_width: None,
            bow_noise: None,
            noise_cutoff: None,
            thermal: None,
        }
    }

    /// The Iowa sets' file names, in both field orders, and how they map onto
    /// the preset's strings (the bass's C extension is the recorded C and E).
    #[test]
    fn takes_are_parsed_by_instrument() {
        let dir = std::env::temp_dir().join(format!("compare-takes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let files: [(&str, Family); 8] = [
            ("Cello.arco.ff.sulC.C2B2.mono.wav", Family::Cello),
            ("Cello.arco.pp.sulA.A4A5.mono.wav", Family::Cello),
            ("Violin.arco.mf.sulE.G6Ab6.mono.wav", Family::Violin),
            ("Viola.arco.sulC.pp.C2B2.mono.wav", Family::Viola),
            ("Viola.arco.sulA.ff.A4B4.mono.wav", Family::Viola),
            ("Bass.arco.sulC.mf.C1Eb1.mono.wav", Family::Bass),
            ("Bass.arco.sulE.pp.E1B1.mono.wav", Family::Bass),
            ("Bass.arco.sulG.ff.C4G4.mono.wav", Family::Bass),
        ];
        for (name, _) in files {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        // A file that names no dynamic.
        std::fs::write(dir.join("Cello.arco.cresc.sulC.C2B2.mono.wav"), b"").unwrap();

        // The three cello takes, the violin's, the two violas and the three
        // basses: each set's own files only.
        for (family, expected) in [
            (Family::Cello, vec!["pp A 69", "ff C 36"]),
            (Family::Violin, vec!["mf E 91"]),
            (Family::Viola, vec!["pp C 36", "ff A 69"]),
            (Family::Bass, vec!["pp E 28", "mf C 24", "ff G 60"]),
        ] {
            let opts = options(family);
            let takes = find_takes(&dir, &recorded(family), &opts).unwrap();
            let got: Vec<String> = takes
                .iter()
                .map(|t| format!("{} {} {}", t.dynamic, t.letter, t.first))
                .collect();
            assert_eq!(got, expected, "{}", recorded(family).prefix);
            // `--string` filters by the recorded letter, not the preset's name.
            let wanted = expected.first().unwrap()[3..4].to_string();
            let mut opts = options(family);
            opts.string = Some(wanted.clone());
            let filtered = find_takes(&dir, &recorded(family), &opts).unwrap();
            assert_eq!(
                filtered.len(),
                expected.iter().filter(|e| e[3..4] == wanted).count(),
                "{wanted}"
            );
            for take in filtered {
                assert_eq!(take.letter, wanted);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
