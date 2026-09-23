//! The solo cello played through the performer: string choice, Helmholtz
//! motion and intonation across the range, legato, and the short articulations.

use strings_dsp::analysis::{Regime, cents, classify, measure_frequency};
use strings_dsp::presets::cello;
use strings_dsp::{Articulation, ContactState, Performer, PerformerFrame, PerformerSettings};

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
    // A bias toward lower strings plays D3 on the G string.
    let mut sul_g = Performer::new(
        &cello::INSTRUMENT,
        PerformerSettings {
            string_bias: 8.0,
            ..PerformerSettings::default()
        },
        FS,
    );
    sul_g.note_on(50, 0.5);
    assert_eq!(sul_g.process_frame().string, 1);
}

/// Across the range and dynamics every note settles into Helmholtz motion
/// within 150 ms. Stopped notes are intonated by ear to within 5 cents; open
/// strings can't be, and flatten with bow force (STATUS.md).
#[test]
fn notes_across_the_range_are_helmholtz_and_in_tune() {
    let mut p = performer();
    for dynamics in [0.1, 0.5, 0.9] {
        for note in [36u8, 40, 43, 47, 50, 55, 57, 64, 72, 76] {
            p.reset();
            p.set_dynamics(dynamics);
            run(&mut p, 0.1);
            p.note_on(note, 0.6);
            let frames = run(&mut p, 1.5);
            let target = frequency(note);
            let period = FS / target;
            let steady = &frames[(0.7 * FS) as usize..];
            let string_frames: Vec<_> = steady.iter().map(|f| f.frame).collect();
            let what = format!("note {note} at dynamics {dynamics}");
            assert_eq!(
                classify(&string_frames, period),
                Regime::Helmholtz,
                "{what}"
            );
            let attack = attack_time(&frames, period);
            assert!(
                attack.is_some_and(|t| t < 0.15),
                "{what}: attack {attack:?}"
            );
            let bridge: Vec<f32> = steady.iter().map(|f| f.frame.bridge_force).collect();
            let err = cents(measure_frequency(&bridge, FS, target), target);
            let open = cello::STRINGS
                .iter()
                .any(|s| cents(s.frequency, target).abs() < 1.0);
            let tolerance = if open { 20.0 } else { 5.0 };
            assert!(err.abs() < tolerance, "{what}: {err:.1} cents");
        }
    }
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

/// Staccato: the bow stops on the string and the note ends quickly. Spiccato:
/// the bow leaves the string ringing.
#[test]
fn staccato_stops_and_spiccato_rings() {
    let mut p = performer();
    for note in [43u8, 50, 62] {
        for articulation in [Articulation::Staccato, Articulation::Spiccato] {
            p.reset();
            p.set_dynamics(0.6);
            p.set_articulation(articulation);
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
            // The stroke has ended well before 0.3 s; measure 100 ms later.
            let later = rms(&rest[(0.3 * FS) as usize..(0.32 * FS) as usize]);
            let db = 20.0 * (later / peak).log10();
            match articulation {
                Articulation::Staccato => assert!(db < -25.0, "staccato {note}: {db:.1} dB"),
                _ => assert!(db > -20.0, "spiccato {note}: {db:.1} dB"),
            }
        }
    }
}
