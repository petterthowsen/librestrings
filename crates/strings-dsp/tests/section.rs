//! Sections: players cloned from one performer, each with its own seeded
//! humanization, switched on and off by the section's size.

use strings_dsp::analysis::{cents, measure_frequency};
use strings_dsp::presets::cello;
use strings_dsp::{Humanization, MAX_PLAYERS, Performer, PerformerSettings, Section};

const FS: f32 = 48_000.0;

fn section(players: usize, humanization: Humanization) -> Section {
    let mut s = Section::new(
        &cello::INSTRUMENT,
        PerformerSettings::default(),
        humanization,
        FS,
    );
    s.set_players(players);
    s
}

/// Every player's output over `seconds`, one row per player.
fn run(s: &mut Section, seconds: f32) -> Vec<Vec<f32>> {
    let mut rows = vec![Vec::new(); MAX_PLAYERS];
    let mut out = [0.0; MAX_PLAYERS];
    for _ in 0..(seconds * FS) as usize {
        s.process(&mut out);
        for (row, y) in rows.iter_mut().zip(out) {
            row.push(y);
        }
    }
    rows
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|x| x * x).sum::<f32>() / x.len() as f32).sqrt()
}

fn frequency(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

/// Player 0 has no humanization: a section of one is the solo performer.
#[test]
fn one_player_is_the_solo_performer() {
    let mut s = section(1, Humanization::default());
    let mut p = Performer::new(&cello::INSTRUMENT, PerformerSettings::default(), FS);
    let mut out = [0.0; MAX_PLAYERS];
    for (note, dynamics) in [(50, 0.4), (57, 0.8), (45, 0.6)] {
        s.set_dynamics(dynamics);
        p.set_dynamics(dynamics);
        s.note_on(note, 0.7);
        p.note_on(note, 0.7);
        for _ in 0..(0.4 * FS) as usize {
            assert_eq!(s.process(&mut out), p.process());
        }
        s.note_off(note);
        p.note_off(note);
    }
}

/// Players come in staggered, player 0 first, all within the delay and jitter,
/// and each settles on its own pitch within its detune.
#[test]
fn players_come_in_late_and_in_tune() {
    let h = Humanization::default();
    let mut s = section(6, h);
    s.set_dynamics(0.6);
    s.note_on(52, 0.7);
    let rows = run(&mut s, 1.5);
    let onsets: Vec<usize> = rows[..6]
        .iter()
        .map(|r| {
            r.iter()
                .position(|y| y.abs() > 1e-4)
                .expect("silent player")
        })
        .collect();
    // A level threshold also sees the attacks: slower by up to `timing`, and
    // quieter by up to `dynamics`.
    let latest = ((h.delay + h.jitter + 0.02) * FS) as usize;
    assert!(
        onsets
            .iter()
            .all(|&o| o >= onsets[0] && o <= onsets[0] + latest),
        "{onsets:?}"
    );
    assert!(
        onsets[1..]
            .iter()
            .any(|&o| o > onsets[0] + (0.005 * FS) as usize),
        "{onsets:?}"
    );

    let f0 = frequency(52);
    let tail = (0.8 * FS) as usize..;
    let pitches: Vec<f32> = rows[..6]
        .iter()
        .map(|r| cents(measure_frequency(&r[tail.clone()], FS, f0), f0))
        .collect();
    // Player 0 is the solo cello; the others stay within their detune of it,
    // plus a little for the pressure and bow position they play at. A stopped
    // note (E3), which the ear corrects: an open string pressed near the top
    // of the band plays flat (up to 46 cents for a player pressing harder
    // and closer to the bridge; STATUS.md item 7).
    let limit = h.detune + h.detune_drift + 4.0;
    assert!(
        pitches.iter().all(|c| (c - pitches[0]).abs() < limit),
        "{pitches:?}"
    );
    // Not all the same pitch: the detune is heard.
    let spread = pitches.iter().cloned().fold(f32::MIN, f32::max)
        - pitches.iter().cloned().fold(f32::MAX, f32::min);
    assert!(spread > 1.0, "{pitches:?}");
    // Nor the same sound.
    let differ = |a: &[f32], b: &[f32]| a[tail.clone()] != b[tail.clone()];
    assert!(differ(&rows[1], &rows[2]) && differ(&rows[0], &rows[1]));
}

/// Players switched off fade out and fall silent; switched on, they come in
/// with the next note, not in the middle of one.
#[test]
fn the_size_changes_while_playing() {
    let mut s = section(4, Humanization::default());
    s.set_dynamics(0.6);
    s.note_on(50, 0.7);
    run(&mut s, 0.6);
    s.set_players(2);
    let rows = run(&mut s, 0.4);
    assert!(rms(&rows[0][(0.2 * FS) as usize..]) > 1e-3);
    for row in &rows[2..4] {
        assert!(row[(0.1 * FS) as usize..].iter().all(|&y| y == 0.0));
    }
    // The fade has no step: no sample moves further than while playing.
    let steps = |r: &[f32]| {
        r.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    };
    assert!(
        steps(&rows[2]) < 2.0 * steps(&rows[0]),
        "{} {}",
        steps(&rows[2]),
        steps(&rows[0])
    );

    s.set_players(4);
    let rows = run(&mut s, 0.3);
    assert!(
        rows[3].iter().all(|&y| y == 0.0),
        "a new player came in mid-note"
    );
    s.note_off(50);
    s.note_on(55, 0.7);
    let rows = run(&mut s, 0.6);
    assert!(rms(&rows[3][(0.3 * FS) as usize..]) > 1e-3);
}

/// The same seed plays the same section; the humanization's draws stay with
/// the player when the spread changes.
#[test]
fn a_section_is_reproducible() {
    let play = |h: Humanization| {
        let mut s = section(4, Humanization::default());
        s.set_humanization(h);
        s.note_on(50, 0.7);
        let mut out = [0.0; MAX_PLAYERS];
        (0..(0.5 * FS) as usize)
            .map(|_| s.process(&mut out))
            .collect::<Vec<_>>()
    };
    assert_eq!(play(Humanization::default()), play(Humanization::default()));
    let none = play(Humanization::NONE);
    assert_ne!(none, play(Humanization::default()));
}

/// "All notes off" drops notes still waiting for a late player. Player 0 has
/// no delay: its note has started, and ends as a short stroke.
#[test]
fn release_all_drops_waiting_notes() {
    let mut s = section(MAX_PLAYERS, Humanization::NONE);
    s.set_humanization(Humanization {
        jitter: 0.0,
        ..Humanization::default()
    });
    s.note_on(50, 0.7);
    s.release_all();
    let rows = run(&mut s, 0.3);
    assert!(rms(&rows[0]) > 1e-3);
    assert!(rows[1..].iter().all(|r| r.iter().all(|&y| y == 0.0)));
}
