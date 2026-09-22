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

/// Measured reference strings, for comparing the model against lab data.
pub mod reference {
    use crate::StringSpec;

    /// Cello G2 string "A T1" (steel core, tungsten winding, lower tension) on
    /// the mdw Vienna monochord with rigid terminations. From Lampis,
    /// Chatziioannou & Scavone, "Experimental analysis of cello string types",
    /// Proc. Mtgs. Acoust. 58, 035013 (2025), Table 1 and Fig. 1: T = 145.31 N,
    /// μ = 7.721 g/m (Z = 1.059 kg/s), L = 0.70 m, mode-1 damping ζ ≈ 3.5e-4
    /// (t60 = ln(1000) / (2π·f0·ζ) ≈ 32 s). Bending stiffness EI = 3.03e-4 N·m²
    /// is not modeled yet. Data: `scripts/fetch-reference-data.sh`.
    pub const MONOCHORD_CELLO_G_A_T1: StringSpec = StringSpec {
        name: "cello G (A T1, monochord)",
        frequency: 98.0,
        length: 0.70,
        tension: 145.31,
        t60: 32.0,
        loss_lowpass: 0.5,
    };
}
