//! The spectral envelope difference between recorded and modelled notes.
//!
//! For each pair of notes, every partial clear of the noise in both gives a
//! point (frequency, recording − model in dB). Neither signal's level is
//! absolute, so each note carries an unknown offset. The points of all notes
//! are fitted as `d = H(f) + c_note`: `H` in sixth-octave bins, found by
//! alternating medians. The notes span the instrument's range, so partial
//! numbers and frequencies decouple: `H` is what a filter after the model
//! would have to add, the body's share if the strings are right (measured on
//! the bridge force with `compare --bridge`, it is the body, radiation and
//! microphone together).
//!
//! The distance of one pair is the RMS of its points after the note's own
//! offset, first averaged per bin so every sixth octave counts the same.

/// Bins per octave.
pub const PER_OCTAVE: f32 = 6.0;
/// The first bin's centre (Hz).
pub const LOWEST: f32 = 40.0;
/// Bins: 40 Hz to about 10 kHz.
pub const BINS: usize = 48;

/// A partial counts only this far clear of the noise between partials (dB).
pub const MIN_SNR: f32 = 10.0;

/// Centre of bin `b` (Hz).
pub fn centre(b: usize) -> f32 {
    LOWEST * 2f32.powf(b as f32 / PER_OCTAVE)
}

/// The bin a frequency falls in, if any.
pub fn bin(f: f32) -> Option<usize> {
    let b = ((f / LOWEST).log2() * PER_OCTAVE).round();
    (b >= 0.0 && (b as usize) < BINS).then_some(b as usize)
}

/// One note's points: (frequency Hz, recording − model dB).
pub type Points = Vec<(f32, f32)>;

pub struct Fit {
    /// Recording − model per bin (dB), mean zero over the bins that have
    /// points; `None` where fewer than `MIN_NOTES` notes reach.
    pub h: [Option<f32>; BINS],
    /// Notes with a point in each bin.
    pub notes: [usize; BINS],
    /// Median over notes of the envelope distance (dB), before and after
    /// taking `h` off.
    pub distance: f32,
    pub residual: f32,
}

/// Bins with fewer notes than this are left out of `h`.
pub const MIN_NOTES: usize = 3;

pub fn fit(notes: &[Points]) -> Fit {
    let binned: Vec<Vec<(usize, f32)>> = notes
        .iter()
        .map(|p| p.iter().filter_map(|&(f, d)| Some((bin(f)?, d))).collect())
        .collect();
    let mut counts = [0usize; BINS];
    for p in &binned {
        let mut seen = [false; BINS];
        for &(b, _) in p {
            seen[b] = true;
        }
        for (c, s) in counts.iter_mut().zip(seen) {
            *c += s as usize;
        }
    }
    let mut h = [0.0f32; BINS];
    let mut offsets = vec![0.0f32; binned.len()];
    for _ in 0..30 {
        for (c, p) in offsets.iter_mut().zip(&binned) {
            *c = median(p.iter().map(|&(b, d)| d - h[b]).collect()).max(-200.0);
        }
        let mut per_bin: Vec<Vec<f32>> = vec![Vec::new(); BINS];
        for (&c, p) in offsets.iter().zip(&binned) {
            for &(b, d) in p {
                per_bin[b].push(d - c);
            }
        }
        for (hb, v) in h.iter_mut().zip(per_bin) {
            *hb = if v.is_empty() { 0.0 } else { median(v) };
        }
        // The gauge: mean zero over the bins in use.
        let used: Vec<f32> = (0..BINS)
            .filter(|&b| counts[b] >= MIN_NOTES)
            .map(|b| h[b])
            .collect();
        let mean = used.iter().sum::<f32>() / used.len().max(1) as f32;
        for hb in &mut h {
            *hb -= mean;
        }
    }
    let h: [Option<f32>; BINS] = std::array::from_fn(|b| (counts[b] >= MIN_NOTES).then_some(h[b]));
    let flat = [Some(0.0); BINS];
    Fit {
        distance: median(binned.iter().map(|p| distance(p, &flat)).collect()),
        residual: median(binned.iter().map(|p| distance(p, &h)).collect()),
        h,
        notes: counts,
    }
}

/// One note's envelope distance after taking `h` off (dB): its own offset
/// removed, points averaged per bin, RMS over the bins.
fn distance(points: &[(usize, f32)], h: &[Option<f32>; BINS]) -> f32 {
    let mut sum = [0.0f32; BINS];
    let mut n = [0usize; BINS];
    for &(b, d) in points {
        if let Some(hb) = h[b] {
            sum[b] += d - hb;
            n[b] += 1;
        }
    }
    let means: Vec<f32> = (0..BINS)
        .filter(|&b| n[b] > 0)
        .map(|b| sum[b] / n[b] as f32)
        .collect();
    if means.is_empty() {
        return f32::NAN;
    }
    let c = median(means.clone());
    (means.iter().map(|m| (m - c) * (m - c)).sum::<f32>() / means.len() as f32).sqrt()
}

fn median(mut v: Vec<f32>) -> f32 {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A known envelope under per-note offsets and partial series comes back.
    #[test]
    fn fit_recovers_an_envelope() {
        let truth =
            |f: f32| 6.0 * (-(f / 1000.0).log2().powi(2)).exp() - 3.0 * (f / 3000.0).ln().max(0.0);
        let notes: Vec<Points> = (0..30)
            .map(|k| {
                let f0 = 65.0 * 2f32.powf(k as f32 / 8.0);
                let offset = (k as f32 * 7.3).sin() * 20.0;
                (1..)
                    .map(|n| n as f32 * f0)
                    .take_while(|&f| f < 9000.0)
                    .map(|f| (f, truth(f) + offset))
                    .collect()
            })
            .collect();
        let fit = fit(&notes);
        let used: Vec<usize> = (0..BINS).filter(|&b| fit.h[b].is_some()).collect();
        let mean_truth = used.iter().map(|&b| truth(centre(b))).sum::<f32>() / used.len() as f32;
        for &b in &used {
            let err = fit.h[b].unwrap() - (truth(centre(b)) - mean_truth);
            assert!(err.abs() < 1.0, "{} Hz: {err}", centre(b));
        }
        assert!(fit.distance > 1.0, "{}", fit.distance);
        assert!(fit.residual < 0.5, "{}", fit.residual);
    }
}
