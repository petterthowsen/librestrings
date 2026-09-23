//! The solo cello played through the performer: string choice, Helmholtz
//! motion and intonation across the range, legato, the bow lift, double stops
//! and the pressure range.

use strings_dsp::analysis::{Regime, cents, classify, measure_frequency};
use strings_dsp::presets::cello;
use strings_dsp::{
    BowLift, ContactState, Fingering, Performer, PerformerFrame, PerformerSettings,
    PerformerTuning, Polyphony,
};

const FS: f32 = 48_000.0;

fn performer() -> Performer {
    Performer::new(&cello::INSTRUMENT, PerformerSettings::default(), FS)
}

fn run(p: &mut Performer, seconds: f32) -> Vec<PerformerFrame> {
    (0..(seconds * FS) as usize)
        .map(|_| p.process_frame())
        .collect()
}

fn frequency(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

fn rms(frames: &[PerformerFrame]) -> f32 {
    (frames
        .iter()
        .map(|f| f.frame.bridge_force.powi(2))
        .sum::<f32>()
        / frames.len() as f32)
        .sqrt()
}

/// Periods between slips on the bowed string (samples): its instantaneous pitch.
fn slip_periods(frames: &[PerformerFrame]) -> Vec<f32> {
    let onsets: Vec<usize> = (1..frames.len())
        .filter(|&i| {
            frames[i].frame.state.is_slipping() && !frames[i - 1].frame.state.is_slipping()
        })
        .collect();
    onsets.windows(2).map(|w| (w[1] - w[0]) as f32).collect()
}

/// Time (s) until ten consecutive periods have one slip each, or `None`.
fn attack_time(frames: &[PerformerFrame], period: f32) -> Option<f32> {
    let onsets: Vec<usize> = (1..frames.len())
        .filter(|&i| {
            frames[i].frame.state.is_slipping() && !frames[i - 1].frame.state.is_slipping()
        })
        .collect();
    onsets
        .windows(11)
        .find(|w| {
            w.windows(2)
                .all(|d| ((d[1] - d[0]) as f32 - period).abs() < 0.1 * period)
        })
        .map(|w| w[0] as f32 / FS)
}

#[test]
fn picks_the_string_with_the_lowest_position() {
    let mut p = performer();
    // (note, string): C2 and G2 open, A2 on the G string, D3 open, E4 high on the A string.
    for (note, string) in [(36, 0), (43, 1), (45, 1), (50, 2), (57, 3), (64, 3)] {
        p.reset();
        p.note_on(note, 0.5);
        assert_eq!(p.process_frame().string, string, "note {note}");
        p.note_off(note);
    }
}

/// The fingering modes: open strings near the nut, none but the C string in
/// mid position, and high positions on lower strings near the bridge.
#[test]
fn fingering_modes_choose_the_strings() {
    let mut p = performer();
    // (note, strings near the nut, mid, bridge)
    let cases = [
        (43u8, [1, 0, 0]), // G2: open G, else stopped on the C string
        (50, [2, 1, 1]),   // D3: open D, else on the G string
        (57, [3, 2, 2]),   // A3
        (62, [3, 3, 2]),   // D4: an octave up the D string near the bridge
        (36, [0, 0, 0]),   // C2 is only on the C string
    ];
    for (note, strings) in cases {
        for (fingering, string) in [Fingering::NutAndOpen, Fingering::Mid, Fingering::Bridge]
            .into_iter()
            .zip(strings)
        {
            p.reset();
            p.set_fingering(fingering);
            p.note_on(note, 0.5);
            assert_eq!(p.process_frame().string, string, "{note} {fingering:?}");
            p.note_off(note);
        }
    }
}

/// A performer whose bow holds perfectly still (no wander).
fn steady_performer() -> Performer {
    let settings = PerformerSettings {
        tuning: PerformerTuning {
            wander_pressure: 0.0,
            wander_speed: 0.0,
            wander_beta: 0.0,
            ..PerformerTuning::default()
        },
        ..PerformerSettings::default()
    };
    Performer::new(&cello::INSTRUMENT, settings, FS)
}

/// Across the range and dynamics every note settles into Helmholtz motion
/// within 150 ms. With a steady bow, stopped notes are intonated by ear to
/// within 5 cents; open strings can't be, and flatten with bow force
/// (STATUS.md).
#[test]
fn notes_across_the_range_are_helmholtz_and_in_tune() {
    check_range(&mut steady_performer(), 5.0);
}

/// The bow's wander moves the pitch a little (bowed pitch depends on force and
/// position), but notes stay Helmholtz and within 10 cents.
#[test]
fn notes_stay_helmholtz_while_the_bow_wanders() {
    check_range(&mut performer(), 10.0);
}

/// Across the range, the wander's randomness decides a few borderline attacks
/// (the open G at pp most of all), so one seed says little. Over 24 seeds,
/// failed checks may number at most 1% of the notes. Slow: run with `--release --ignored`.
#[test]
#[ignore]
fn notes_stay_helmholtz_across_wander_seeds() {
    let mut failures = Vec::new();
    for seed in 1..=24 {
        let settings = PerformerSettings {
            seed,
            ..PerformerSettings::default()
        };
        let mut p = Performer::new(&cello::INSTRUMENT, settings, FS);
        failures.extend(
            range_failures(&mut p, 10.0)
                .into_iter()
                .map(|f| format!("seed {seed}, {f}")),
        );
    }
    let notes = 24 * 30;
    eprintln!(
        "{} failed checks over {notes} notes:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(failures.len() * 100 <= notes, "{} failures", failures.len());
}

fn check_range(p: &mut Performer, stopped_tolerance: f32) {
    let failures = range_failures(p, stopped_tolerance);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Plays notes across the range at three dynamics and lists every one that
/// isn't Helmholtz, settles later than 150 ms or misses its pitch (open
/// strings may be 20 cents flat).
fn range_failures(p: &mut Performer, stopped_tolerance: f32) -> Vec<String> {
    let mut failures = Vec::new();
    for dynamics in [0.1, 0.5, 0.9] {
        for note in [36u8, 40, 43, 47, 50, 55, 57, 64, 72, 76] {
            p.reset();
            p.set_dynamics(dynamics);
            run(p, 0.1);
            p.note_on(note, 0.6);
            let frames = run(p, 1.5);
            let target = frequency(note);
            let period = FS / target;
            let steady = &frames[(0.7 * FS) as usize..];
            let string_frames: Vec<_> = steady.iter().map(|f| f.frame).collect();
            let what = format!("note {note} at dynamics {dynamics}");
            let regime = classify(&string_frames, period);
            if regime != Regime::Helmholtz {
                failures.push(format!("{what}: {regime:?}"));
            }
            let attack = attack_time(&frames, period);
            if !attack.is_some_and(|t| t < 0.15) {
                failures.push(format!("{what}: attack {attack:?}"));
            }
            let bridge: Vec<f32> = steady.iter().map(|f| f.frame.bridge_force).collect();
            let err = cents(measure_frequency(&bridge, FS, target), target);
            let open = cello::STRINGS
                .iter()
                .any(|s| cents(s.frequency, target).abs() < 1.0);
            let tolerance = if open { 20.0 } else { stopped_tolerance };
            if err.abs() >= tolerance {
                failures.push(format!("{what}: {err:.1} cents"));
            }
        }
    }
    failures
}

#[test]
fn detache_alternates_bow_direction() {
    let mut p = performer();
    let mut directions = Vec::new();
    for note in [48, 50, 52] {
        p.note_on(note, 0.6);
        let frames = run(&mut p, 0.4);
        directions.push(frames.last().unwrap().bow_velocity.signum());
        p.note_off(note);
        run(&mut p, 0.05);
    }
    assert_eq!(directions[0], -directions[1]);
    assert_eq!(directions[1], -directions[2]);
}

/// Overlapping notes on one string: the finger glides, the bow keeps going
/// in the same direction, and the new note is in tune.
#[test]
fn legato_glides_without_a_new_stroke() {
    let mut p = performer();
    p.note_on(50, 0.6);
    let before = run(&mut p, 0.6);
    p.note_on(52, 0.6);
    p.note_off(50);
    let after = run(&mut p, 1.2);
    let direction = before.last().unwrap().bow_velocity.signum();
    assert!(after.iter().all(|f| f.bow_velocity.signum() == direction));
    assert!(after.iter().all(|f| f.string == 2));
    let steady = &after[(0.5 * FS) as usize..];
    let target = frequency(52);
    let frames: Vec<_> = steady.iter().map(|f| f.frame).collect();
    assert_eq!(classify(&frames, FS / target), Regime::Helmholtz);
    let bridge: Vec<f32> = steady.iter().map(|f| f.frame.bridge_force).collect();
    let err = cents(measure_frequency(&bridge, FS, target), target);
    assert!(err.abs() < 5.0, "{err:.1} cents");
}

/// A legato line that crosses strings moves the bow, without changing its
/// direction, and the old string rings on without the bow.
#[test]
fn legato_crossing_moves_the_bow_to_the_new_string() {
    let mut p = performer();
    p.note_on(43, 0.6);
    let before = run(&mut p, 0.6);
    assert_eq!(before.last().unwrap().string, 1);
    p.note_on(50, 0.6);
    p.note_off(43);
    let after = run(&mut p, 1.0);
    let direction = before.last().unwrap().bow_velocity.signum();
    assert!(after.iter().all(|f| f.bow_velocity.signum() == direction));
    let last = after.last().unwrap();
    assert_eq!(last.string, 2);
    let frames: Vec<_> = after[(0.4 * FS) as usize..]
        .iter()
        .map(|f| f.frame)
        .collect();
    assert_eq!(classify(&frames, FS / frequency(50)), Regime::Helmholtz);
    // The bow has left the G string.
    assert_eq!(p.string_frames()[1].state, ContactState::Off);
}

/// Vibrato moves the pitch of a stopped note by tens of cents; an open string
/// can't be vibrated.
#[test]
fn vibrato_only_on_stopped_notes() {
    for (note, stopped) in [(52u8, true), (50, false)] {
        let mut p = performer();
        p.set_vibrato(1.0);
        p.note_on(note, 0.6);
        let frames = run(&mut p, 2.0);
        let periods = slip_periods(&frames[FS as usize..]);
        let (lo, hi) = periods
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
        let range = cents(hi, lo);
        if stopped {
            assert!(range > 30.0, "stopped: {range:.1} cents");
        } else {
            // One sample of period jitter is about 5 cents at D3.
            assert!(range < 12.0, "open: {range:.1} cents");
        }
    }
}

/// Short notes. On the string the bow stops on it and the note ends quickly
/// (staccato); off the string it is thrown off and the string rings on.
#[test]
fn on_string_stops_and_off_string_rings() {
    let mut p = performer();
    for note in [43u8, 50, 62] {
        for bow_lift in [BowLift::OnString, BowLift::OffString] {
            p.reset();
            p.set_dynamics(0.6);
            p.set_bow_lift(bow_lift);
            run(&mut p, 0.1);
            p.note_on(note, 0.7);
            let stroke = run(&mut p, 0.1);
            p.note_off(note);
            let rest = run(&mut p, 0.6);
            let peak = stroke
                .chunks((0.02 * FS) as usize)
                .chain(rest.chunks((0.02 * FS) as usize))
                .map(rms)
                .fold(0.0, f32::max);
            let db_at = |from: f32| {
                let later = rms(&rest[(from * FS) as usize..((from + 0.02) * FS) as usize]);
                20.0 * (later / peak).log10()
            };
            match bow_lift {
                // The stop is over by 0.04 s; measure well after.
                BowLift::OnString => {
                    let db = db_at(0.3);
                    assert!(db < -25.0, "on string {note}: {db:.1} dB");
                    assert_eq!(p.note(), None);
                    assert!(p.contact(p.bowed_string()) > 0.99, "the bow stays on");
                }
                // The bow has left by 0.1 s and the string rings on (a
                // stopped one until the finger eases off).
                BowLift::OffString => {
                    let db = db_at(0.1);
                    assert!(db > -15.0, "off string {note}: {db:.1} dB");
                    assert!(p.contact(p.bowed_string()) < 1e-6);
                }
            }
        }
    }
}

/// However short the key press, the bow plays a stroke of `min_stroke`.
#[test]
fn a_tap_plays_a_short_stroke() {
    for bow_lift in [BowLift::OnString, BowLift::OffString] {
        let mut p = performer();
        p.set_bow_lift(bow_lift);
        p.note_on(50, 0.9);
        run(&mut p, 0.002);
        p.note_off(50);
        let early = run(&mut p, 0.03);
        assert_eq!(p.note(), Some(50), "{bow_lift:?}");
        assert!(early.iter().any(|f| f.bow_velocity.abs() > 0.05));
        run(&mut p, 0.3);
        assert_eq!(p.note(), None);
    }
}

/// Once the key is up, the finger eases off: a stopped note dies away while an
/// open string rings on.
#[test]
fn stopped_notes_are_damped_after_release() {
    let mut p = performer();
    p.set_dynamics(0.6);
    // (note, stopped): E3 and B3 stopped on the D and A strings; D3 open.
    for (note, stopped) in [(52u8, true), (59, true), (50, false)] {
        p.reset();
        run(&mut p, 0.05);
        // A short note, thrown off the string.
        p.note_on(note, 0.7);
        let stroke = run(&mut p, 0.1);
        p.note_off(note);
        let rest = run(&mut p, 1.0);
        let peak = stroke
            .chunks((0.02 * FS) as usize)
            .map(rms)
            .fold(0.0, f32::max);
        let later = rms(&rest[(0.6 * FS) as usize..(0.62 * FS) as usize]);
        let db = 20.0 * (later / peak).log10();
        if stopped {
            assert!(db < -40.0, "stopped {note}: {db:.1} dB");
        } else {
            assert!(db > -20.0, "open {note}: {db:.1} dB");
        }
    }
}

#[test]
fn release_all_ends_a_legato_line() {
    let mut p = performer();
    p.set_dynamics(0.6);
    p.note_on(55, 0.8);
    run(&mut p, 0.3);
    p.note_on(57, 0.8);
    run(&mut p, 0.3);
    assert_eq!(p.note(), Some(57));
    p.release_all();
    // No legato return to the note still held: the bow lifts off.
    run(&mut p, 0.5);
    assert_eq!(p.note(), None);
    assert!(p.contact(p.bowed_string()) < 1e-6);
    // Keys released after "all notes off" change nothing.
    p.note_off(55);
    p.note_off(57);
    assert_eq!(p.note(), None);
}

/// Pitch (Hz) of string `i` at each control-rate step of a run.
fn string_pitch(p: &mut Performer, i: usize, seconds: f32) -> Vec<f32> {
    (0..(seconds * FS) as usize)
        .map(|_| {
            p.process_frame();
            p.instrument().string(i).frequency()
        })
        .collect()
}

/// Seconds until `pitch` is within 10 cents of where it ends for good (the
/// string's tuning includes the ear's correction).
fn settle_time(pitch: &[f32]) -> f32 {
    let target = pitch[pitch.len() - 1];
    let last_off = pitch
        .iter()
        .rposition(|&f| cents(f, target).abs() > 10.0)
        .map_or(0, |i| i + 1);
    last_off as f32 / FS
}

/// Legato: the landing note's velocity sets the transition. Pressed hard, a
/// note within the hand changes as a finger drops, and a shift slides only
/// quickly; pressed softly, the finger slides slowly (portamento).
#[test]
fn legato_velocity_sets_the_slide() {
    // (from, to, string, velocity, fastest, slowest) in seconds to settle.
    let cases = [
        (45u8, 47u8, 1, 0.8, 0.0, 0.012), // A2 to B2 on the G string, in the hand
        (45, 47, 1, 0.1, 0.12, 0.3),      // the same, soft: portamento
        (64, 60, 3, 0.8, 0.012, 0.04),    // E4 down to C4 on the A string: a shift
    ];
    for (from, to, string, velocity, fastest, slowest) in cases {
        let mut p = steady_performer();
        p.note_on(from, 0.8);
        run(&mut p, 0.4);
        p.note_on(to, velocity);
        p.note_off(from);
        assert_eq!(p.process_frame().string, string);
        let pitch = string_pitch(&mut p, string, 0.4);
        assert!(cents(pitch[pitch.len() - 1], frequency(to)).abs() < 30.0);
        let time = settle_time(&pitch);
        assert!(
            (fastest..slowest).contains(&time),
            "{from} to {to} at {velocity}: {:.0} ms",
            time * 1000.0
        );
    }
}

/// Double stops: a second held note joins on the adjacent string, both strings
/// sound in Helmholtz motion at their pitches, and releasing either leaves
/// the other playing alone.
#[test]
fn double_stops_play_two_strings() {
    let mut p = steady_performer();
    p.set_polyphony(Polyphony::DoubleStops);
    // D3 open and A3 open: a fifth on the D and A strings.
    p.note_on(50, 0.6);
    p.note_on(57, 0.6);
    run(&mut p, 0.8);
    assert_eq!(p.note(), Some(50));
    assert_eq!(p.second(), Some((57, 3)));
    for (string, note) in [(2, 50), (3, 57)] {
        assert!(p.contact(string) > 0.99, "string {string}");
        let frames: Vec<_> = (0..(0.4 * FS) as usize)
            .map(|_| {
                p.process_frame();
                p.string_frames()[string]
            })
            .collect();
        let period = FS / frequency(note);
        assert_eq!(classify(&frames, period), Regime::Helmholtz, "{note}");
    }
    // Releasing the lower note leaves the upper one, on its string.
    p.note_off(50);
    run(&mut p, 0.3);
    assert_eq!((p.note(), p.second()), (Some(57), None));
    assert_eq!(p.bowed_string(), 3);
    assert!(p.contact(2) < 1e-3);
    p.note_off(57);
    run(&mut p, 0.5);
    assert_eq!(p.note(), None);
}

/// A pair one hand can't play (a semitone apart with both stopped would need
/// strings a fifth apart; two notes on one string) plays legato instead; mono
/// never plays two.
#[test]
fn double_stops_only_where_the_hand_can() {
    let mut p = performer();
    p.set_polyphony(Polyphony::DoubleStops);
    // C2 and D2 are both on the C string only.
    p.note_on(36, 0.6);
    run(&mut p, 0.2);
    p.note_on(38, 0.6);
    run(&mut p, 0.2);
    assert_eq!((p.note(), p.second()), (Some(38), None));
    p.release_all();
    run(&mut p, 0.5);

    // E3 and B3: E on the D string and B on the A string, both stopped two
    // semitones up: one hand.
    p.note_on(52, 0.6);
    p.note_on(59, 0.6);
    run(&mut p, 0.1);
    assert_eq!(p.second(), Some((59, 3)));
    // A third note leads on from the nearer one: G3 replaces E3 on the D
    // string, under B3.
    p.note_on(55, 0.6);
    run(&mut p, 0.1);
    assert_eq!((p.note(), p.second()), (Some(59), Some((55, 2))));
    p.release_all();
    run(&mut p, 0.5);

    // A line over a held note: D3 open, A3 then B3 above it on the A string.
    p.note_on(50, 0.6);
    p.note_on(57, 0.6);
    run(&mut p, 0.1);
    p.note_on(59, 0.6);
    p.note_off(57);
    run(&mut p, 0.1);
    assert_eq!((p.note(), p.second()), (Some(50), Some((59, 3))));
    p.release_all();
    run(&mut p, 0.5);
    assert_eq!(p.note(), None);

    let mut mono = performer();
    mono.note_on(50, 0.6);
    mono.note_on(57, 0.6);
    run(&mut mono, 0.1);
    assert_eq!((mono.note(), mono.second()), (Some(57), None));
}

/// A chord on the string: both notes grip together and play in one stroke;
/// the bow then stops on both, and lifts off both when the bow lift goes off.
#[test]
fn double_stop_on_the_string() {
    let mut p = performer();
    p.set_polyphony(Polyphony::DoubleStops);
    p.set_bow_lift(BowLift::OnString);
    p.note_on(43, 0.7);
    p.note_on(50, 0.7);
    let frames = run(&mut p, 0.01);
    assert_eq!(p.second(), Some((50, 2)));
    assert!(frames.last().is_some_and(|f| f.string == 1));
    assert!(p.contact(1) > 0.3 && (p.contact(1) - p.contact(2)).abs() < 1e-6);
    run(&mut p, 0.1);
    p.note_off(43);
    p.note_off(50);
    run(&mut p, 0.3);
    assert_eq!(p.note(), None);
    assert!(p.contact(1) > 0.99 && p.contact(2) > 0.99);
    p.set_bow_lift(BowLift::OffString);
    run(&mut p, 0.3);
    assert!(p.contact(1) == 0.0 && p.contact(2) == 0.0);
}

/// The pressure control runs from the band's lower edge (flautando) through
/// normal to scratch, above the band, where the motion turns raucous.
#[test]
fn pressure_runs_from_flautando_to_scratch() {
    let regime = |pressure: f32| {
        let mut p = steady_performer();
        p.set_dynamics(0.5);
        p.set_pressure(pressure);
        run(&mut p, 0.1);
        p.note_on(50, 0.6);
        let frames = run(&mut p, 1.0);
        let steady: Vec<_> = frames[(0.5 * FS) as usize..]
            .iter()
            .map(|f| f.frame)
            .collect();
        (
            classify(&steady, FS / frequency(50)),
            rms(&frames[(0.5 * FS) as usize..]),
        )
    };
    let (flautando, soft) = regime(0.0);
    let (normal, mid) = regime(0.5);
    let (scratch, loud) = regime(1.0);
    assert_eq!(flautando, Regime::Helmholtz);
    assert_eq!(normal, Regime::Helmholtz);
    assert_eq!(scratch, Regime::Raucous);
    assert!(soft < mid && mid < loud, "{soft} {mid} {loud}");
}
