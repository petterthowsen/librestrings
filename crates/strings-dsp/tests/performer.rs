//! The solo cello played through the performer: string choice, Helmholtz
//! motion and intonation across the range, legato, the bow lift, double stops
//! and the pressure range. The violin's range is checked the same way.

use strings_dsp::analysis::{Regime, cents, classify, measure_frequency};
use strings_dsp::presets::{bass, cello, viola, violin};
use strings_dsp::{
    Articulation, BowLift, BowNoise, ContactState, Fingering, InstrumentSpec, Performer,
    PerformerFrame, PerformerSettings, PerformerTuning, Polyphony,
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

/// A named string ("sul G") wins over the fingering mode where it can play
/// the note, and gives way where it can't.
#[test]
fn a_named_string_plays_what_it_can() {
    let mut p = performer();
    p.set_string(Some(1));
    // (note, string): D3 and A3 up the G string; C2 is below it, so the C string.
    for (note, string) in [(50u8, 1), (57, 1), (36, 0)] {
        p.reset();
        p.note_on(note, 0.5);
        assert_eq!(p.process_frame().string, string, "note {note}");
        p.note_off(note);
    }
}

/// A performer whose bow holds perfectly still (no wander).
fn steady_performer() -> Performer {
    steady(&cello::INSTRUMENT)
}

/// Without the bow's randomness: no wander and no bow noise. The seed sweeps
/// check them.
fn steady(spec: &InstrumentSpec) -> Performer {
    let settings = PerformerSettings::for_instrument(spec);
    let settings = PerformerSettings {
        tuning: PerformerTuning {
            wander_pressure: 0.0,
            wander_speed: 0.0,
            wander_beta: 0.0,
            ..settings.tuning
        },
        ..settings
    };
    let mut p = Performer::new(spec, settings, FS);
    p.instrument_mut().set_bow_noise(BowNoise::default());
    p
}

/// Across the range and dynamics every note settles into Helmholtz motion
/// within 180 ms. With a steady bow, stopped notes are intonated by ear to
/// within 5 cents; open strings can't be, and flatten with bow force
/// (STATUS.md).
#[test]
fn notes_across_the_range_are_helmholtz_and_in_tune() {
    check_range(&mut steady_performer(), &RANGE_NOTES, 5.0);
}

/// The bow's wander moves the pitch a little (bowed pitch depends on force and
/// position), but notes stay Helmholtz and within 10 cents.
#[test]
fn notes_stay_helmholtz_while_the_bow_wanders() {
    check_range(&mut performer(), &RANGE_NOTES, 10.0);
}

/// The violin, as the cello above.
#[test]
fn violin_notes_across_the_range_are_helmholtz_and_in_tune() {
    check_range(&mut steady(&violin::INSTRUMENT), &VIOLIN_NOTES, 5.0);
}

#[test]
fn violin_notes_stay_helmholtz_while_the_bow_wanders() {
    let mut p = Performer::new(
        &violin::INSTRUMENT,
        PerformerSettings::for_instrument(&violin::INSTRUMENT),
        FS,
    );
    check_range(&mut p, &VIOLIN_NOTES, 10.0);
}

/// The viola and the double bass, as the cello above.
#[test]
fn viola_notes_across_the_range_are_helmholtz_and_in_tune() {
    check_range(&mut steady(&viola::INSTRUMENT), &VIOLA_NOTES, 5.0);
}

#[test]
fn viola_notes_stay_helmholtz_while_the_bow_wanders() {
    check_range(&mut wandering(&viola::INSTRUMENT), &VIOLA_NOTES, 10.0);
}

#[test]
fn bass_notes_across_the_range_are_helmholtz_and_in_tune() {
    check_range(&mut steady(&bass::INSTRUMENT), &BASS_NOTES, 5.0);
}

#[test]
fn bass_notes_stay_helmholtz_while_the_bow_wanders() {
    check_range(&mut wandering(&bass::INSTRUMENT), &BASS_NOTES, 10.0);
}

fn wandering(spec: &InstrumentSpec) -> Performer {
    Performer::new(spec, PerformerSettings::for_instrument(spec), FS)
}

/// Across the range, the wander's randomness decides a few borderline attacks
/// (the open G at pp most of all), so one seed says little. Over 24 seeds,
/// failed checks may number at most 1% of the notes. Slow: run with `--release --ignored`.
#[test]
#[ignore]
fn notes_stay_helmholtz_across_wander_seeds() {
    across_seeds(&cello::INSTRUMENT, &RANGE_NOTES);
}

#[test]
#[ignore]
fn violin_notes_stay_helmholtz_across_wander_seeds() {
    across_seeds(&violin::INSTRUMENT, &VIOLIN_NOTES);
}

#[test]
#[ignore]
fn viola_notes_stay_helmholtz_across_wander_seeds() {
    across_seeds(&viola::INSTRUMENT, &VIOLA_NOTES);
}

#[test]
#[ignore]
fn bass_notes_stay_helmholtz_across_wander_seeds() {
    across_seeds(&bass::INSTRUMENT, &BASS_NOTES);
}

fn across_seeds(spec: &InstrumentSpec, notes: &[(u8, Option<usize>)]) {
    let mut failures = Vec::new();
    for seed in 1..=24 {
        let settings = PerformerSettings {
            seed,
            ..PerformerSettings::for_instrument(spec)
        };
        let mut p = Performer::new(spec, settings, FS);
        failures.extend(
            range_failures(&mut p, notes, 10.0)
                .into_iter()
                .map(|f| format!("seed {seed}, {f}")),
        );
    }
    let notes = 24 * 3 * notes.len();
    eprintln!(
        "{} failed checks over {notes} notes:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(failures.len() * 100 <= notes, "{} failures", failures.len());
}

fn check_range(p: &mut Performer, notes: &[(u8, Option<usize>)], stopped_tolerance: f32) {
    let failures = range_failures(p, notes, stopped_tolerance);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Notes the range checks play: the fingering's choice across the range, then
/// high positions on the lower strings (18–24 semitones up, "sul C" to
/// "sul D"), where the bow must keep its distance from the bridge.
const RANGE_NOTES: [(u8, Option<usize>); 16] = [
    (36, None),
    (40, None),
    (43, None),
    (47, None),
    (50, None),
    (55, None),
    (57, None),
    (64, None),
    (72, None),
    (76, None),
    (54, Some(0)),
    (59, Some(0)),
    (62, Some(1)),
    (67, Some(1)),
    (71, Some(2)),
    (74, Some(2)),
];

/// The violin's: its range from the open G to A6 (two octaves up the E
/// string), then high positions on the lower strings, 12 and 19 semitones up.
const VIOLIN_NOTES: [(u8, Option<usize>); 16] = [
    (55, None),
    (59, None),
    (62, None),
    (66, None),
    (69, None),
    (73, None),
    (76, None),
    (81, None),
    (88, None),
    (93, None),
    (67, Some(0)),
    (74, Some(0)),
    (74, Some(1)),
    (81, Some(1)),
    (81, Some(2)),
    (88, Some(2)),
];

/// The viola's: the cello's notes an octave up (the same tuning).
const VIOLA_NOTES: [(u8, Option<usize>); 16] = {
    let mut notes = RANGE_NOTES;
    let mut i = 0;
    while i < notes.len() {
        notes[i].0 += 12;
        i += 1;
    }
    notes
};

/// The double bass's: its range from the open C1 (the E string's C
/// extension) to G4 (two octaves up the G string), then high positions as
/// bassists play them: up to an octave above E1 on the extended string, 17
/// semitones on the A, and thumb position on the D, 19 and 24 up. (Higher
/// up the lowest string the bow's distance from the bridge puts it near the
/// middle of the string, and those notes hold multiple slips.)
const BASS_NOTES: [(u8, Option<usize>); 18] = [
    (24, None),
    (26, None),
    (28, None),
    (31, None),
    (35, None),
    (38, None),
    (42, None),
    (45, None),
    (50, None),
    (55, None),
    (62, None),
    (67, None),
    (35, Some(0)),
    (40, Some(0)),
    (45, Some(1)),
    (50, Some(1)),
    (57, Some(2)),
    (62, Some(2)),
];

/// Open strings can't be intonated, and the bowed string flattens with bow
/// force: they may be this far off (cents). The bass's open strings play
/// flattest (its open E, before the C extension, 20–25 cents at mf–ff), the
/// other instruments' 6–14.
fn open_tolerance(spec: &InstrumentSpec) -> f32 {
    if spec.name == "bass" { 30.0 } else { 20.0 }
}

/// Plays notes across the range at three dynamics and lists every one that
/// isn't Helmholtz, settles later than 180 ms (a mid-velocity attack at mf
/// takes about 150 ms to reach full speed) or misses its pitch (open strings
/// by more than [`open_tolerance`]).
fn range_failures(
    p: &mut Performer,
    notes: &[(u8, Option<usize>)],
    stopped_tolerance: f32,
) -> Vec<String> {
    let mut failures = Vec::new();
    for dynamics in [0.1, 0.5, 0.9] {
        for &(note, string) in notes {
            p.reset();
            p.set_string(string);
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
            if !attack.is_some_and(|t| t < 0.18) {
                failures.push(format!("{what}: attack {attack:?}"));
            }
            let bridge: Vec<f32> = steady.iter().map(|f| f.frame.bridge_force).collect();
            let err = cents(measure_frequency(&bridge, FS, target), target);
            let played = &p.instrument().spec().strings[frames.last().unwrap().string];
            let open = cents(played.frequency, target).abs() < 1.0;
            let tolerance = if open {
                open_tolerance(p.instrument().spec())
            } else {
                stopped_tolerance
            };
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
    // Pressed hard: a portamento would slide up the G string instead.
    p.note_on(50, 0.9);
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
                    // The resting bow damps the string well below the
                    // off-string ring (−7 to −8 dB): D4, the least, −19 dB
                    // with the 75 ms stop (−25 or less with 40 ms).
                    assert!(db < -15.0, "on string {note}: {db:.1} dB");
                    assert_eq!(p.note(), None);
                    assert!(p.contact(p.bowed_string()) > 0.99, "the bow stays on");
                }
                // The bow has left by 0.1 s and the string rings on.
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
        let early = run(&mut p, 0.05);
        assert_eq!(p.note(), Some(50), "{bow_lift:?}");
        assert!(early.iter().any(|f| f.bow_velocity.abs() > 0.02));
        run(&mut p, 0.3);
        assert_eq!(p.note(), None);
    }
}

/// Fast détaché off the string in the lowest octave (sixteenths at 150 bpm, as
/// in the `ostinato` score): the bow never gets off the string, so every stroke
/// starts from a bow moving the other way. The bow change comes at the start of
/// the note, and every note speaks. Reversing over the whole attack used to put
/// it 20–40 ms in, and an accented open C2 after a D2 choked (PLAN.md "Bow changes in fast détaché").
#[test]
fn fast_detache_changes_bow_at_the_note() {
    let mut p = performer();
    p.set_dynamics(0.7);
    p.set_bow_lift(BowLift::OffString);
    let bar = [
        36u8, 36, 43, 36, 39, 36, 43, 36, 38, 38, 45, 38, 43, 38, 47, 38,
    ];
    let (on, off) = ((0.08 * FS) as usize, (0.02 * FS) as usize);
    let mut levels = Vec::new();
    for (k, &note) in bar.iter().chain(&bar).chain(&[36]).enumerate() {
        let before = p.process_frame().bow_velocity;
        p.note_on(note, if k % 4 == 0 { 0.87 } else { 0.63 });
        let mut frames = run(&mut p, on as f32 / FS);
        p.note_off(note);
        frames.extend(run(&mut p, off as f32 / FS));
        if k > 0 {
            let change = frames
                .iter()
                .position(|f| f.bow_velocity * before <= 0.0)
                .unwrap_or(frames.len()) as f32
                / FS;
            assert!(
                change < 0.015,
                "note {k}: bow change {:.0} ms in",
                change * 1e3
            );
        }
        let out =
            (frames.iter().map(|f| f.output.powi(2)).sum::<f32>() / frames.len() as f32).sqrt();
        levels.push(20.0 * out.log10());
    }
    let (lo, hi) = levels
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &l| (lo.min(l), hi.max(l)));
    assert!(hi - lo < 12.0, "note levels {lo:.1} to {hi:.1} dB");
}

/// Once the key is up, a stopped note rings on as an open string does: the
/// finger stays down. It eases off, and the string dies away, when the next
/// note goes to another string.
#[test]
fn stopped_notes_ring_until_the_next_string() {
    let mut p = performer();
    p.set_dynamics(0.6);
    // (note, stopped): E3 and B3 stopped on the D and A strings; D3 open.
    for (note, stopped) in [(52u8, true), (59, true), (50, false)] {
        p.reset();
        run(&mut p, 0.05);
        // A short note, thrown off the string.
        p.note_on(note, 0.7);
        let stroke = run(&mut p, 0.1);
        let string = stroke.last().unwrap().string;
        p.note_off(note);
        let rest = run(&mut p, 1.0);
        let peak = stroke
            .chunks((0.02 * FS) as usize)
            .map(rms)
            .fold(0.0, f32::max);
        let later = rms(&rest[(0.6 * FS) as usize..(0.62 * FS) as usize]);
        let db = 20.0 * (later / peak).log10();
        // The finger's own damping: a stopped note rings shorter than an open one,
        // but not the −40 dB it fell to when the finger eased off at the key-up.
        assert!(db > -36.0, "{note}: {db:.1} dB");
        assert_eq!(p.finger_down(string), stopped, "{note}");

        // The next note, on the C string.
        p.note_on(36, 0.7);
        let level = |p: &mut Performer, seconds: f32| {
            let n = (seconds * FS) as usize;
            let sum: f32 = (0..n)
                .map(|_| {
                    p.process();
                    p.string_frames()[string].bridge_force.powi(2)
                })
                .sum();
            (sum / n as f32).sqrt()
        };
        let before = level(&mut p, 0.02);
        run(&mut p, 0.5);
        let after = level(&mut p, 0.02);
        let db = 20.0 * (after / before).log10();
        assert!(!p.finger_down(string));
        if stopped {
            assert!(db < -25.0, "stopped {note} after the next note: {db:.1} dB");
        } else {
            assert!(db > -15.0, "open {note} after the next note: {db:.1} dB");
        }
        p.note_off(36);
    }
}

/// A slow legato (portamento) stays on its string where it can, even where
/// another string plays the note lower: a finger can't slide across strings.
/// Pressed hard, the same note goes to the lower position.
#[test]
fn portamento_stays_on_the_string() {
    // A2 on the G string, then A3: an octave up the G string, or open on the A.
    for (velocity, string) in [(0.2, 1), (0.9, 3)] {
        let mut p = performer();
        p.set_dynamics(0.6);
        p.note_on(45, 0.7);
        run(&mut p, 0.3);
        assert_eq!(p.bowed_string(), 1);
        p.note_on(57, velocity);
        run(&mut p, 0.3);
        assert_eq!(p.bowed_string(), string, "velocity {velocity}");
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
/// quickly; pressed softly, the finger slides slowly (portamento). The ear is
/// off: its correction, learned on the note before, then eases to the new
/// note's over a few hundred ms (about 15 cents from E4 to C4 on the A string).
#[test]
fn legato_velocity_sets_the_slide() {
    // (from, to, string, velocity, fastest, slowest) in seconds to settle.
    let cases = [
        (45u8, 47u8, 1, 0.8, 0.0, 0.012), // A2 to B2 on the G string, in the hand
        (45, 47, 1, 0.1, 0.12, 0.3),      // the same, soft: portamento
        (64, 60, 3, 0.8, 0.012, 0.04),    // E4 down to C4 on the A string: a shift
    ];
    let spec = &cello::INSTRUMENT;
    let settings = PerformerSettings::for_instrument(spec);
    let settings = PerformerSettings {
        tuning: PerformerTuning {
            wander_pressure: 0.0,
            wander_speed: 0.0,
            wander_beta: 0.0,
            ear_range: 0.0,
            ..settings.tuning
        },
        ..settings
    };
    for (from, to, string, velocity, fastest, slowest) in cases {
        let mut p = Performer::new(spec, settings, FS);
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

/// Violin flautando moves the bow toward the fingerboard (sul tasto) and
/// lightens it; the cello's bow stays where the dynamics put it.
#[test]
fn violin_flautando_moves_toward_the_fingerboard() {
    let beta = |spec: &InstrumentSpec, pressure: f32| {
        let mut p = steady(spec);
        p.set_dynamics(0.5);
        p.set_pressure(pressure);
        run(&mut p, 0.1);
        // An open string, so the bow's distance from the bridge doesn't move it.
        p.note_on(69, 0.6);
        run(&mut p, 0.5);
        p.instrument().string(p.bowed_string()).beta()
    };
    let (normal, tasto) = (
        beta(&violin::INSTRUMENT, 0.5),
        beta(&violin::INSTRUMENT, 0.0),
    );
    assert!((tasto - violin::INSTRUMENT.tasto).abs() < 1e-3, "{tasto}");
    assert!(normal < 0.13, "{normal}");
    let (normal, flautando) = (beta(&cello::INSTRUMENT, 0.5), beta(&cello::INSTRUMENT, 0.0));
    assert!((flautando - normal).abs() < 1e-4, "{normal} {flautando}");
}

/// At flautando the violin's notes still settle promptly into Helmholtz
/// motion and play in tune, across the range and dynamics.
#[test]
fn violin_flautando_notes_are_helmholtz_and_in_tune() {
    let mut p = steady(&violin::INSTRUMENT);
    p.set_pressure(0.0);
    check_range(&mut p, &VIOLIN_NOTES, 5.0);
}

/// A section clones one performer per player instead of fitting every
/// string again; the clone plays exactly as the original.
#[test]
fn a_clone_plays_the_same() {
    let mut p = performer();
    p.note_on(50, 0.7);
    run(&mut p, 0.3);
    let mut q = p.clone();
    for p in [&mut p, &mut q] {
        p.note_on(55, 0.7);
    }
    let a = run(&mut p, 0.5);
    let b = run(&mut q, 0.5);
    assert!(a.iter().zip(&b).all(|(a, b)| a.output == b.output));
}

/// Strings that are silent with the bow off them are skipped, and bowing one
/// again brings it back.
#[test]
fn rung_out_strings_are_skipped_until_bowed() {
    let mut p = performer();
    run(&mut p, 0.1);
    assert!((0..4).all(|i| p.instrument().is_idle(i)));

    p.note_on(50, 0.7);
    let frames = run(&mut p, 0.5);
    let string = frames.last().unwrap().string;
    assert!(!p.instrument().is_idle(string));
    p.note_off(50);
    run(&mut p, 20.0);
    assert!((0..4).all(|i| p.instrument().is_idle(i)));

    p.note_on(50, 0.7);
    let frames = run(&mut p, 0.5);
    assert!(!p.instrument().is_idle(string));
    assert!(rms(&frames[(0.3 * FS) as usize..]) > 1e-3);
}

/// A trill: the bow keeps its place on the string while the finger changes
/// the length under it. Its distance from the bridge settles between the two
/// notes' positions instead of jumping with every note.
#[test]
fn a_trill_is_bowed_in_between() {
    let mut p = steady_performer();
    p.set_dynamics(0.5);
    // A2 held on the G string, B2 trilled above it (legato, within the hand).
    p.note_on(45, 0.8);
    run(&mut p, 0.5);
    let string = p.bowed_string();
    let length = |p: &Performer, note: u8| {
        let s = &p.instrument().spec().strings[string];
        s.length * s.frequency / frequency(note)
    };
    let alone = p.instrument().string(string).beta() * length(&p, 45);
    let mut distances = Vec::new();
    // B2 pressed and let go: back to the held A2 each time.
    for k in 0..16 {
        let note = if k % 2 == 0 { 47 } else { 45 };
        if note == 47 {
            p.note_on(47, 0.8);
        } else {
            p.note_off(47);
        }
        run(&mut p, 0.06);
        distances.push(p.instrument().string(string).beta() * length(&p, note));
    }
    let late = &distances[8..];
    let (lo, hi) = late
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &d| (lo.min(d), hi.max(d)));
    // B2 alone would put the bow a whole tone's worth (11%) closer.
    let b2 = alone / 2f32.powf(2.0 / 12.0);
    assert!(hi - lo < 0.3 * (alone - b2), "{lo:.4}–{hi:.4} m");
    assert!(
        lo > b2 && hi < alone,
        "{lo:.4}–{hi:.4} m, notes {b2:.4} and {alone:.4}"
    );
}

/// A soft legato from an open string slides: the finger lands at the nut and
/// slides up the same string.
#[test]
fn portamento_from_an_open_string() {
    let mut p = steady_performer();
    p.set_dynamics(0.6);
    // Open D3, then F3 soft: up the D string, not placed at once.
    p.note_on(50, 0.7);
    run(&mut p, 0.4);
    let string = p.bowed_string();
    p.note_on(53, 0.1);
    p.note_off(50);
    assert_eq!(p.process_frame().string, string);
    let pitch = string_pitch(&mut p, string, 0.4);
    let time = settle_time(&pitch);
    assert!(cents(pitch[pitch.len() - 1], frequency(53)).abs() < 30.0);
    assert!((0.12..0.3).contains(&time), "{:.0} ms", time * 1000.0);
}

/// The performer names what it plays, newest first.
#[test]
fn articulations_follow_the_playing() {
    use Articulation as A;
    let mut p = performer();
    p.set_dynamics(0.6);
    let last = |p: &Performer| p.articulations().0[0];
    // Off the string: a détaché, a hard legato within the hand, a soft one
    // (portamento), a crossing, then the bow lifts.
    p.note_on(45, 0.6);
    assert_eq!(last(&p), Some(A::Detache));
    assert_eq!(p.bow_direction(), 1.0, "the first stroke is a down-bow");
    run(&mut p, 0.3);
    p.note_on(47, 0.9);
    p.note_off(45);
    assert_eq!(last(&p), Some(A::Legato));
    run(&mut p, 0.2);
    p.note_on(45, 0.1);
    p.note_off(47);
    assert_eq!(last(&p), Some(A::Portamento));
    run(&mut p, 0.3);
    p.note_on(62, 0.9);
    p.note_off(45);
    assert_eq!(last(&p), Some(A::StringCrossing));
    run(&mut p, 0.3);
    p.note_off(62);
    run(&mut p, 0.01);
    assert_eq!(last(&p), Some(A::BowLift));
    let (recent, count) = p.articulations();
    assert_eq!(
        recent,
        [
            Some(A::BowLift),
            Some(A::StringCrossing),
            Some(A::Portamento)
        ]
    );
    assert_eq!(count, 5);
    run(&mut p, 0.3);

    // On the string: a martelé, stopped; the next stroke is an up-bow.
    p.set_bow_lift(BowLift::OnString);
    p.note_on(50, 0.9);
    assert_eq!(last(&p), Some(A::Martele));
    assert_eq!(p.bow_direction(), -1.0);
    run(&mut p, 0.1);
    p.note_off(50);
    run(&mut p, 0.01);
    assert_eq!(last(&p), Some(A::BowStop));
}

/// Under the sustain pedal, a key let go leaves the bow going; the next
/// separate note changes bow (détaché), an overlapping one still slurs, and
/// the pedal let up ends the stroke.
#[test]
fn sustain_pedal_plays_detache() {
    use Articulation as A;
    let mut p = performer();
    p.set_dynamics(0.6);
    p.set_sustain(true);
    p.note_on(50, 0.6);
    run(&mut p, 0.3);
    p.note_off(50);
    run(&mut p, 0.2);
    assert_eq!(p.note(), Some(50), "the pedal holds the stroke");
    assert!(p.contact(p.bowed_string()) > 0.99);
    let direction = p.bow_direction();

    p.note_on(52, 0.6);
    assert_eq!(p.articulations().0[0], Some(A::Detache));
    assert_eq!(p.bow_direction(), -direction, "a bow change");
    let frames = run(&mut p, 0.2);
    assert!(frames.iter().any(|f| f.bow_velocity * direction < 0.0));

    // Overlapping: slurred, no bow change.
    p.note_on(53, 0.9);
    p.note_off(52);
    assert_eq!(p.articulations().0[0], Some(A::Legato));
    assert_eq!(p.bow_direction(), -direction);
    p.note_off(53);
    run(&mut p, 0.2);
    assert_eq!(p.note(), Some(53));

    p.set_sustain(false);
    run(&mut p, 0.01);
    assert_eq!(p.articulations().0[0], Some(A::BowLift));
    run(&mut p, 0.5);
    assert_eq!(p.note(), None);
}

/// A bow keyswitch against the stroke changes bow within the note; with it,
/// or between notes, it sets the next stroke's direction.
#[test]
fn bow_keyswitch_sets_the_direction() {
    use Articulation as A;
    let mut p = performer();
    p.set_dynamics(0.6);
    p.note_on(50, 0.6);
    run(&mut p, 0.3);
    assert_eq!(p.bow_direction(), 1.0);
    p.set_bow_direction(-1.0, 0.6);
    assert_eq!(p.articulations().0[0], Some(A::BowChange));
    assert_eq!(p.bow_direction(), -1.0);
    let frames = run(&mut p, 0.3);
    assert!(frames.last().unwrap().bow_velocity < 0.0);
    assert_eq!(p.note(), Some(50), "the note goes on");

    // Up-bow again for the next stroke, instead of alternating to down.
    p.set_bow_direction(-1.0, 0.6);
    p.note_off(50);
    run(&mut p, 0.4);
    p.note_on(52, 0.6);
    assert_eq!(p.bow_direction(), -1.0);
    run(&mut p, 0.3);
    p.note_off(52);
    run(&mut p, 0.4);
    // Between notes: the next stroke is a down-bow twice in a row.
    p.set_bow_direction(1.0, 0.6);
    p.note_on(50, 0.6);
    assert_eq!(p.bow_direction(), 1.0);
    run(&mut p, 0.2);
    p.note_off(50);
    run(&mut p, 0.4);
    p.set_bow_direction(1.0, 0.6);
    p.note_on(50, 0.6);
    assert_eq!(p.bow_direction(), 1.0);
}

/// The dynamics brought to zero during a stroke change the bow once, after
/// resting there for `auto_bow_change`; a stroke begun at zero doesn't.
#[test]
fn dynamics_at_zero_change_the_bow() {
    use Articulation as A;
    let mut p = performer();
    p.set_dynamics(0.6);
    p.note_on(50, 0.6);
    run(&mut p, 0.3);
    p.set_dynamics(0.0);
    run(&mut p, 0.1);
    assert_eq!(p.bow_direction(), 1.0, "not yet");
    run(&mut p, 0.4);
    assert_eq!(p.bow_direction(), -1.0);
    assert_eq!(p.articulations().0[0], Some(A::BowChange));
    run(&mut p, 1.0);
    assert_eq!(p.bow_direction(), -1.0, "once per rest at zero");
    assert_eq!(p.note(), Some(50));
    p.set_dynamics(0.4);
    run(&mut p, 0.1);
    p.set_dynamics(0.0);
    run(&mut p, 0.4);
    assert_eq!(p.bow_direction(), 1.0, "again after rising");
    p.note_off(50);
    run(&mut p, 0.5);

    p.note_on(52, 0.6);
    let direction = p.bow_direction();
    run(&mut p, 1.0);
    assert_eq!(p.bow_direction(), direction, "begun at zero");
}
