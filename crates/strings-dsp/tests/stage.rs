//! The stage: placement by time and level at a stereo mic pair, and the
//! early reflections of the room.

use strings_dsp::{MAX_PLAYERS, Placement, Stage, StageSettings};

const FS: f32 = 48_000.0;

fn dry() -> StageSettings {
    StageSettings {
        reflections: 0.0,
        ..StageSettings::default()
    }
}

/// The left and right response to an impulse from player 0 over `seconds`.
fn impulse(stage: &mut Stage, seconds: f32) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for i in 0..(seconds * FS) as usize {
        let mut x = [0.0; MAX_PLAYERS];
        x[0] = if i == 0 { 1.0 } else { 0.0 };
        let [l, r] = stage.process(&x);
        left.push(l);
        right.push(r);
    }
    (left, right)
}

fn energy(x: &[f32]) -> f32 {
    x.iter().map(|x| x * x).sum()
}

/// Index of the largest sample.
fn peak(x: &[f32]) -> usize {
    (0..x.len())
        .max_by(|&a, &b| x[a].abs().total_cmp(&x[b].abs()))
        .unwrap()
}

fn at(x: f32, y: f32) -> Placement {
    Placement {
        x,
        y,
        ..Placement::CELLOS
    }
}

/// On the centre line the two mics hear the same, reflections included.
#[test]
fn the_centre_is_in_the_middle() {
    let mut stage = Stage::new(StageSettings::default(), at(0.0, 3.0), 1, FS);
    let (l, r) = impulse(&mut stage, 0.3);
    for (a, b) in l.iter().zip(&r) {
        assert!((a - b).abs() <= 1e-6 * (1.0 + a.abs()), "{a} {b}");
    }
}

/// A section to the left is louder and earlier on the left.
#[test]
fn left_is_louder_and_earlier_on_the_left() {
    let mut stage = Stage::new(dry(), at(-4.0, 3.0), 1, FS);
    let (l, r) = impulse(&mut stage, 0.1);
    let db = 10.0 * (energy(&l) / energy(&r)).log10();
    assert!(db > 3.0, "{db} dB");
    assert!(peak(&l) < peak(&r), "{} {}", peak(&l), peak(&r));
    // And the other way round.
    let mut stage = Stage::new(dry(), at(4.0, 3.0), 1, FS);
    let (l, r) = impulse(&mut stage, 0.1);
    assert!(energy(&r) > 2.0 * energy(&l));
}

/// Farther upstage is later and quieter; the front of the stage has no delay.
#[test]
fn farther_is_later_and_quieter() {
    let mut near = Stage::new(dry(), at(0.0, 0.0), 1, FS);
    let mut far = Stage::new(dry(), at(0.0, 6.0), 1, FS);
    let (near, _) = impulse(&mut near, 0.1);
    let (far, _) = impulse(&mut far, 0.1);
    assert!(peak(&near) < 5, "{}", peak(&near));
    let expected = (6.0 / 343.0 * FS) as usize;
    assert!(
        peak(&far) + 40 > expected && peak(&far) < expected,
        "{}",
        peak(&far)
    );
    assert!(energy(&far) < 0.5 * energy(&near));
}

/// The reflections come after the direct sound, and only when switched on.
#[test]
fn reflections_follow_the_direct_sound() {
    let mut wet = Stage::new(StageSettings::default(), at(0.0, 3.0), 1, FS);
    let mut dry = Stage::new(dry(), at(0.0, 3.0), 1, FS);
    let (wet, _) = impulse(&mut wet, 0.3);
    let (dry, _) = impulse(&mut dry, 0.3);
    let direct = peak(&dry);
    let after = direct + (0.002 * FS) as usize;
    assert!(energy(&dry[after..]) < 1e-6 * energy(&dry));
    assert!(energy(&wet[after..]) > 0.1 * energy(&dry));
    assert_eq!(&wet[..direct], &dry[..direct]);
}

/// Moving a section glides: no sample jumps.
#[test]
fn moving_glides() {
    let mut stage = Stage::new(dry(), at(0.0, 3.0), 1, FS);
    let tone = |i: usize| (i as f32 * 0.02).sin();
    let mut last = 0.0;
    let mut largest: f32 = 0.0;
    for i in 0..(0.5 * FS) as usize {
        if i == (0.1 * FS) as usize {
            stage.set_placement(at(-5.0, 8.0));
        }
        let mut x = [0.0; MAX_PLAYERS];
        x[0] = tone(i);
        let [l, _] = stage.process(&x);
        if i > 1000 {
            largest = largest.max((l - last).abs());
        }
        last = l;
    }
    assert!(largest < 0.03, "{largest}");
}
