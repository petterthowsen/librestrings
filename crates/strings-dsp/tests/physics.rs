//! Physics checks for one bowed string: tuning, stability, Helmholtz motion and
//! the Schelleng playability limits.

use strings_dsp::analysis::{
    Regime, bow_steady, cents, classify, classify_bridge_force, measure_frequency, measure_partial,
    partial_amplitude, slip_stats,
};
use strings_dsp::presets::{cello, reference, violin};
use strings_dsp::{BowInput, BowedString, FrictionParams, Loss, StringSpec, schelleng_limits};

const FS: f32 = 48_000.0;

fn new_string(spec: &StringSpec, fs: f32) -> BowedString {
    BowedString::new(spec, FrictionParams::default(), fs, 50.0)
}

/// Plucks the string with a short force pulse and returns the bridge force.
fn pluck(string: &mut BowedString, fs: f32, seconds: f32) -> Vec<f32> {
    let pulse = (0.0005 * fs) as usize;
    (0..(seconds * fs) as usize)
        .map(|i| {
            let f = if i < pulse { 0.1 } else { 0.0 };
            string.process(BowInput::default(), f).bridge_force
        })
        .collect()
}

#[test]
fn free_string_is_in_tune() {
    for fs in [44_100.0, 48_000.0, 96_000.0] {
        for spec in &violin::STRINGS {
            // Open string and a few stopped notes up to an octave.
            for semitones in [0.0, 1.0, 7.0, 12.0] {
                let f0 = spec.frequency * 2f32.powf(semitones / 12.0);
                let mut s = new_string(spec, fs);
                s.set_frequency(f0);
                s.set_bow_position(0.1);
                let out = pluck(&mut s, fs, 1.0);
                let measured = measure_frequency(&out[(0.1 * fs) as usize..], fs, f0);
                let err = cents(measured, f0);
                assert!(
                    err.abs() < 1.0,
                    "{} +{semitones} @ {fs}: {err:.2} cents",
                    spec.name
                );
            }
        }
    }
}

#[test]
fn stiff_string_is_in_tune() {
    let spec = &reference::MONOCHORD_CELLO_G_A_T1;
    for fs in [44_100.0, 48_000.0, 96_000.0] {
        for semitones in [0.0, 1.0, 7.0, 12.0, 24.0] {
            let f0 = spec.frequency * 2f32.powf(semitones / 12.0);
            let mut s = new_string(spec, fs);
            s.set_frequency(f0);
            s.set_bow_position(0.1);
            let out = pluck(&mut s, fs, 1.0);
            let err = cents(measure_frequency(&out[(0.1 * fs) as usize..], fs, f0), f0);
            assert!(err.abs() < 1.0, "+{semitones} @ {fs}: {err:.2} cents");
        }
    }
}

/// The partials of the stiff cello string follow `f_n = n·f0·sqrt((1 + B·n²) / (1 + B))`
/// up to about 2.5 kHz, where partial 20 is about 14 cents sharp. With the
/// measured damping the upper partials die within a fraction of a second,
/// too fast to measure their frequency, so this uses the one-pole loss.
#[test]
fn stiff_string_partials_are_sharp() {
    let spec = &StringSpec {
        loss: Loss::OnePole {
            t60: 32.0,
            lowpass: 0.5,
        },
        ..reference::MONOCHORD_CELLO_G_A_T1
    };
    let b = spec.inharmonicity();
    assert!((b - 4.2e-5).abs() < 0.1e-5, "B = {b}");
    for (fs, semitones) in [(48_000.0, 0.0), (96_000.0, 0.0), (48_000.0, 7.0)] {
        let f0 = spec.frequency * 2f32.powf(semitones / 12.0);
        let b = b * 2f32.powf(semitones / 6.0);
        let mut s = new_string(spec, fs);
        s.set_frequency(f0);
        // Plucking here leaves no partial up to 25 near a node.
        let beta = 0.137;
        s.set_bow_position(beta);
        let out = pluck(&mut s, fs, 2.0);
        let out = &out[(0.05 * fs) as usize..];
        for n in 1..=(2500.0 / f0) as usize {
            let nf = n as f32;
            if (std::f32::consts::PI * nf * beta).sin().abs() < 0.2 {
                continue;
            }
            let target = nf * f0 * ((1.0 + b * nf * nf) / (1.0 + b)).sqrt();
            let err = cents(measure_partial(out, fs, target, f0), target);
            let stretch = cents(target, nf * f0);
            assert!(
                err.abs() < 2.0,
                "+{semitones} @ {fs}, partial {n}: {err:.2} cents off (target is {stretch:.1} cents sharp)"
            );
        }
    }
}

/// Each partial of the measured cello string decays at the rate of its damping
/// curve, within a factor of 1.5, over the measured modes (up to 1.7 kHz).
#[test]
fn measured_damping_sets_partial_decay() {
    let spec = &reference::MONOCHORD_CELLO_G_A_T1;
    let Loss::Measured(curve) = spec.loss else {
        panic!("reference string has measured loss");
    };
    for fs in [48_000.0, 96_000.0] {
        let f0 = spec.frequency;
        let mut s = new_string(spec, fs);
        s.set_bow_position(0.137);
        let out = pluck(&mut s, fs, 3.0);
        // Windows of 8 periods, far enough apart for a clear drop but short
        // enough that the fastest partial is still above the noise.
        let window = (8.0 * fs / f0) as usize;
        let start = (0.05 * fs) as usize;
        for n in [1usize, 2, 4, 5, 7, 8, 9, 10, 11, 12, 15] {
            let f = n as f32 * f0 * (1.0 + spec.inharmonicity() * (n * n) as f32).sqrt();
            let zeta = curve.zeta(f);
            // Aim for a drop of about 20 dB between the two windows.
            let gap = ((2.3 / (std::f32::consts::TAU * f * zeta)) * fs) as usize;
            let gap = gap.clamp(window, out.len() - start - window);
            let a1 = partial_amplitude(&out[start..start + window], fs, f);
            let a2 = partial_amplitude(&out[start + gap..start + gap + window], fs, f);
            let measured = (a1 / a2).ln() / (std::f32::consts::TAU * f * gap as f32 / fs);
            assert!(
                (measured / zeta).ln().abs() < 1.5f32.ln(),
                "@ {fs}, mode {n}: ζ {measured:.2e}, curve {zeta:.2e}"
            );
        }
    }
}

#[test]
fn free_string_decays() {
    let spec = violin::string("A").unwrap();
    let mut s = new_string(spec, FS);
    let out = pluck(&mut s, FS, 3.0);
    let block = (0.25 * FS) as usize;
    let rms: Vec<f32> = out
        .chunks(block)
        .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt())
        .collect();
    assert!(rms.iter().all(|r| r.is_finite()));
    assert!(rms.windows(2).all(|w| w[1] < w[0]), "not decaying: {rms:?}");
    // The spec's t60 is 1.5 s: after 3 s we expect roughly -120 dB from the start.
    assert!(rms[rms.len() - 1] < rms[0] * 1e-4);
}

fn steady_bow(
    spec: &StringSpec,
    beta: f32,
    speed: f32,
    force: f32,
) -> (BowedString, Vec<strings_dsp::StringFrame>) {
    let mut s = new_string(spec, FS);
    s.set_bow_position(beta);
    let frames = bow_steady(&mut s, FS, speed, force, 1.5, 0.05);
    // Keep the last second: past the attack transient.
    let tail = frames[(0.5 * FS) as usize..].to_vec();
    (s, tail)
}

fn limits(s: &BowedString, beta: f32, speed: f32) -> (f32, f32) {
    schelleng_limits(
        s.impedance(),
        s.loop_gain(),
        &FrictionParams::default(),
        beta,
        speed,
    )
}

/// A force comfortably inside the simulated Helmholtz band. The simulated upper
/// edge follows Schelleng's F_max closely; the lower edge sits roughly 5–10×
/// above his F_min (see `strings-render schelleng`), so we anchor on F_max.
fn mid_band_force(s: &BowedString, beta: f32, speed: f32) -> f32 {
    0.3 * limits(s, beta, speed).1
}

#[test]
fn moderate_bowing_produces_helmholtz_motion() {
    let (beta, speed) = (0.1, 0.1);
    for spec in &violin::STRINGS {
        let force = mid_band_force(&new_string(spec, FS), beta, speed);
        let (_, frames) = steady_bow(spec, beta, speed, force);

        let period = FS / spec.frequency;
        assert_eq!(
            classify(&frames, period),
            Regime::Helmholtz,
            "{} string",
            spec.name
        );
        let bridge: Vec<f32> = frames.iter().map(|f| f.bridge_force).collect();
        assert_eq!(
            classify_bridge_force(&bridge, period),
            Regime::Helmholtz,
            "{} string, from bridge force",
            spec.name
        );

        let stats = slip_stats(&frames, period);
        assert!(
            (stats.slips_per_period - 1.0).abs() < 0.05,
            "{}: {stats:?}",
            spec.name
        );
        // Slip lasts about β of the period; corner rounding lengthens it somewhat.
        assert!(
            stats.slip_fraction > 0.7 * beta && stats.slip_fraction < 1.6 * beta,
            "{}: {stats:?}",
            spec.name
        );

        // During stick the string moves with the bow.
        let stuck: Vec<f32> = frames
            .iter()
            .filter(|f| !f.state.is_slipping())
            .map(|f| f.bow_point_velocity)
            .collect();
        assert!(stuck.iter().all(|&v| (v - speed).abs() < 1e-5));

        // Bowed pitch sits close to the free string (real strings flatten slightly with force).
        let force_signal: Vec<f32> = frames.iter().map(|f| f.bridge_force).collect();
        let pitch = cents(
            measure_frequency(&force_signal, FS, spec.frequency),
            spec.frequency,
        );
        assert!(pitch.abs() < 15.0, "{}: {pitch:.1} cents", spec.name);
    }
}

#[test]
fn bow_force_outside_schelleng_limits_breaks_helmholtz_motion() {
    let spec = violin::string("A").unwrap();
    let (beta, speed) = (0.1, 0.1);
    let (f_min, f_max) = limits(&new_string(spec, FS), beta, speed);
    let period = FS / spec.frequency;

    let (_, too_heavy) = steady_bow(spec, beta, speed, 4.0 * f_max);
    assert_ne!(classify(&too_heavy, period), Regime::Helmholtz, "4× F_max");

    let (_, too_light) = steady_bow(spec, beta, speed, 0.25 * f_min);
    assert_ne!(classify(&too_light, period), Regime::Helmholtz, "F_min / 4");
}

/// The bridge-force classifier on idealized signals: a sawtooth (Helmholtz),
/// two drops per period (double slip), a sinusoid (no slipping) and noise.
#[test]
fn bridge_force_classifier_on_ideal_signals() {
    let period = 100.0;
    let n = 4000;
    let saw = |t: f32| t.fract() - 0.5;
    let helmholtz: Vec<f32> = (0..n).map(|i| saw(i as f32 / period)).collect();
    let double: Vec<f32> = (0..n).map(|i| saw(2.0 * i as f32 / period)).collect();
    let sine: Vec<f32> = (0..n)
        .map(|i| (std::f32::consts::TAU * i as f32 / period).sin())
        .collect();
    // Deterministic pseudo-random noise (xorshift).
    let mut state = 0x2545_f491_u32;
    let noise: Vec<f32> = (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as f32 / u32::MAX as f32 - 0.5
        })
        .collect();

    assert_eq!(classify_bridge_force(&helmholtz, period), Regime::Helmholtz);
    // A DC offset (static bridge force) doesn't matter.
    let offset: Vec<f32> = helmholtz.iter().map(|v| v + 3.0).collect();
    assert_eq!(classify_bridge_force(&offset, period), Regime::Helmholtz);
    assert_eq!(classify_bridge_force(&double, period), Regime::MultiSlip);
    assert_eq!(classify_bridge_force(&sine, period), Regime::NoSlip);
    assert_eq!(classify_bridge_force(&noise, period), Regime::Raucous);
}

/// Cello strings (measured loss, stiffness and torsion) stay in tune when
/// stopped, from the open string up to two octaves.
#[test]
fn cello_strings_are_in_tune() {
    for fs in [44_100.0, 96_000.0] {
        for spec in &cello::STRINGS {
            for semitones in [0.0, 5.0, 12.0, 24.0] {
                let f0 = spec.frequency * 2f32.powf(semitones / 12.0);
                let mut s = BowedString::new(spec, FrictionParams::default(), fs, spec.frequency);
                s.set_frequency(f0);
                s.set_bow_position(0.1);
                let out = pluck(&mut s, fs, 1.0);
                let err = cents(measure_frequency(&out[(0.1 * fs) as usize..], fs, f0), f0);
                assert!(
                    err.abs() < 1.0,
                    "{} +{semitones} @ {fs}: {err:.2} cents",
                    spec.name
                );
            }
        }
    }
}

/// Every cello string, with the preset's bow hair, gives Helmholtz motion in
/// the middle of its calibrated force band.
#[test]
fn cello_bowing_produces_helmholtz_motion() {
    let instrument = &cello::INSTRUMENT;
    let (beta, speed) = (0.1, 0.1);
    for (spec, limits) in instrument.strings.iter().zip(&instrument.force_limits) {
        let mut s = BowedString::new(spec, instrument.friction, FS, spec.frequency);
        s.set_bow_hair(instrument.hair);
        s.set_bow_position(beta);
        let force = limits.force(spec.impedance(), speed, beta, 0.65);
        let frames = bow_steady(&mut s, FS, speed, force, 1.5, 0.05);
        let frames = &frames[(0.5 * FS) as usize..];
        let period = FS / spec.frequency;
        assert_eq!(classify(frames, period), Regime::Helmholtz, "{}", spec.name);
        let stats = slip_stats(frames, period);
        assert!(
            (stats.slips_per_period - 1.0).abs() < 0.05,
            "{}: {stats:?}",
            spec.name
        );
        let bridge: Vec<f32> = frames.iter().map(|f| f.bridge_force).collect();
        let pitch = cents(
            measure_frequency(&bridge, FS, spec.frequency),
            spec.frequency,
        );
        assert!(pitch.abs() < 15.0, "{}: {pitch:.1} cents", spec.name);
    }
}

/// Compliant bow hair is passive: a bow held still on a plucked string only
/// takes energy out.
#[test]
fn bow_hair_is_passive() {
    let spec = &cello::STRINGS[1];
    let mut s = BowedString::new(spec, FrictionParams::default(), FS, spec.frequency);
    s.set_bow_hair(cello::INSTRUMENT.hair);
    s.set_bow_position(0.1);
    let pulse = (0.0005 * FS) as usize;
    let out: Vec<f32> = (0..(1.0 * FS) as usize)
        .map(|i| {
            let bow = BowInput {
                velocity: 0.0,
                force: if i < pulse { 0.0 } else { 0.5 },
            };
            s.process(bow, if i < pulse { 0.5 } else { 0.0 })
                .bridge_force
        })
        .collect();
    let block = (0.1 * FS) as usize;
    let energy: Vec<f32> = out
        .chunks(block)
        .map(|c| c.iter().map(|x| x * x).sum())
        .collect();
    assert!(energy.iter().all(|e| e.is_finite()));
    assert!(energy.windows(2).all(|w| w[1] <= w[0]), "{energy:?}");
}

/// Retuning a string's loss, stiffness and torsion while it plays gives the
/// string built with them, sample for sample.
#[test]
fn applied_design_matches_a_new_string() {
    let old = cello::STRINGS[1];
    let new = StringSpec {
        loss: Loss::Measured(strings_dsp::DampingCurve {
            floor: 2e-3,
            at_1khz: 2e-4,
            exponent: 2.5,
        }),
        bending_stiffness: 1e-4,
        torsion: old.torsion.map(|t| strings_dsp::TorsionSpec {
            impedance: 2.0 * t.impedance,
            frequency: 4.0 * old.frequency,
            q: 80.0,
        }),
        ..old
    };
    let f0 = old.frequency * 2f32.powf(5.0 / 12.0);
    let mut retuned = new_string(&old, FS);
    let mut design = BowedString::design(&new, FS, 50.0);
    assert!(retuned.apply_design(&new, &mut design));
    let mut fresh = new_string(&new, FS);
    for s in [&mut retuned, &mut fresh] {
        s.set_frequency(f0);
        s.set_bow_position(0.1);
    }
    assert_eq!(pluck(&mut retuned, FS, 0.5), pluck(&mut fresh, FS, 0.5));

    // A different pitch or kind of loss is refused.
    let mut design = BowedString::design(&violin::STRINGS[0], FS, 50.0);
    assert!(!retuned.apply_design(&violin::STRINGS[0], &mut design));
}

/// A loss at the stopping finger adds its nepers per period to the decay.
#[test]
fn termination_loss_shortens_the_decay() {
    let spec = cello::STRINGS[2];
    let f0 = spec.frequency * 2f32.powf(4.0 / 12.0);
    let decay_db = |loss: f32| {
        let mut s = new_string(&spec, FS);
        s.set_frequency(f0);
        s.set_bow_position(0.1);
        s.set_termination_loss(loss);
        let out = pluck(&mut s, FS, 1.2);
        let at =
            |t: f32| partial_amplitude(&out[(t * FS) as usize..((t + 0.2) * FS) as usize], FS, f0);
        20.0 * (at(0.2) / at(0.9)).log10()
    };
    let extra = decay_db(0.02) - decay_db(0.0);
    let expected = 20.0 * std::f32::consts::LOG10_E * 0.02 * f0 * 0.7;
    assert!(
        (extra / expected - 1.0).abs() < 0.1,
        "{extra:.1} dB, expected {expected:.1}"
    );
}
