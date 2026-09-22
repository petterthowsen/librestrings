//! Physical data for instruments. Values are typical, not measured; refine by ear.

pub mod violin {
    use crate::StringSpec;

    const LENGTH: f32 = 0.325;

    /// Open strings, lowest first. Tensions are typical of synthetic-core sets.
    pub const STRINGS: [StringSpec; 4] = [
        StringSpec {
            name: "G",
            frequency: 196.00,
            length: LENGTH,
            tension: 44.0,
            t60: 2.0,
            loss_lowpass: 0.5,
        },
        StringSpec {
            name: "D",
            frequency: 293.66,
            length: LENGTH,
            tension: 42.0,
            t60: 1.8,
            loss_lowpass: 0.5,
        },
        StringSpec {
            name: "A",
            frequency: 440.00,
            length: LENGTH,
            tension: 50.0,
            t60: 1.5,
            loss_lowpass: 0.5,
        },
        StringSpec {
            name: "E",
            frequency: 659.26,
            length: LENGTH,
            tension: 73.0,
            t60: 1.2,
            loss_lowpass: 0.5,
        },
    ];

    pub fn string(name: &str) -> Option<&'static StringSpec> {
        STRINGS.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }
}
