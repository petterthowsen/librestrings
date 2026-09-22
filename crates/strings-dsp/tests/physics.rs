//! Physics checks for one bowed string: tuning, stability, Helmholtz motion and
//! the Schelleng playability limits.

use strings_dsp::analysis::{Regime, bow_steady, cents, classify, measure_frequency, slip_stats};
use strings_dsp::presets::violin;
use strings_dsp::{BowInput, BowedString, FrictionParams, StringSpec, schelleng_limits};

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
