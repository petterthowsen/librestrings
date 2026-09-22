//! Physical data for instruments. Values are typical, not measured; refine by ear.

pub mod violin {
    use crate::{Loss, StringSpec};

    // No stiffness or torsion yet: we have no data for these strings, and
    // violin strings are far less stiff than cello strings (PLAN.md 3.6).
    const LENGTH: f32 = 0.325;

    /// Open strings, lowest first. Tensions are typical of synthetic-core sets.
    pub const STRINGS: [StringSpec; 4] = [
        StringSpec {
            name: "G",
            frequency: 196.00,
            length: LENGTH,
            tension: 44.0,
            loss: Loss::OnePole {
                t60: 2.0,
                lowpass: 0.5,
            },
            bending_stiffness: 0.0,
            torsion: None,
        },
        StringSpec {
            name: "D",
            frequency: 293.66,
            length: LENGTH,
            tension: 42.0,
            loss: Loss::OnePole {
                t60: 1.8,
                lowpass: 0.5,
            },
            bending_stiffness: 0.0,
            torsion: None,
        },
        StringSpec {
            name: "A",
            frequency: 440.00,
            length: LENGTH,
            tension: 50.0,
            loss: Loss::OnePole {
                t60: 1.5,
                lowpass: 0.5,
            },
            bending_stiffness: 0.0,
            torsion: None,
        },
        StringSpec {
            name: "E",
            frequency: 659.26,
            length: LENGTH,
            tension: 73.0,
            loss: Loss::OnePole {
                t60: 1.2,
                lowpass: 0.5,
            },
            bending_stiffness: 0.0,
            torsion: None,
        },
    ];

    pub fn string(name: &str) -> Option<&'static StringSpec> {
        STRINGS.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }
}

/// Measured reference strings, for comparing the model against lab data.
pub mod reference {
    use crate::{DampingCurve, Loss, StringSpec, TorsionSpec};

    /// Cello G2 string "A T1" (steel core, tungsten winding, lower tension) on
    /// the mdw Vienna monochord with rigid terminations. From Lampis,
    /// Chatziioannou & Scavone, "Experimental analysis of cello string types",
    /// Proc. Mtgs. Acoust. 58, 035013 (2025), Table 1 and Fig. 1: T = 145.31 N,
    /// μ = 7.721 g/m (Z = 1.059 kg/s), L = 0.70 m, d = 0.947 mm, EI = 3.03e-4 N·m²
    /// (B ≈ 4.2e-5). Paper: docs/papers/. Data: `scripts/fetch-reference-data.sh`.
    ///
    /// Damping: ζ per mode digitized from the paper's Fig. 1 (A T1 panel; ×1e-4):
    /// modes 1–5: 2.8, 2.75, 6.6 (wide spread), 3.6, 5.0; modes 7–11: 3.5, 4.7,
    /// 6.6, 8.5, 11.4; mode 17: 35. The curve fits all but mode 3, within ±17% rms
    /// (log). Above mode 17 (1.7 kHz) it is extrapolated.
    ///
    /// The torsional data are estimates, not measurements of this string (the
    /// paper's `Zto` column doesn't convert to an impedance consistently; see
    /// PLAN.md 3.6):
    /// - frequency: 5.5 × f0, as measured on a steel cello G string (Mores,
    ///   "Further empirical data for torsion on bowed strings", PLOS One 2019:
    ///   543 Hz torsional, 98 Hz transverse);
    /// - impedance: `κ·μ·c_t` with c_t = 2·L·f_t = 755 m/s and κ = 0.6 (most of
    ///   a wound string's mass sits in the winding, between κ = 0.5 for a solid
    ///   rod and 1 for a thin tube): 3.5 kg/s, about 3.3 × Z;
    /// - Q = 50: Mores finds torsional Q about an order of magnitude below the
    ///   transverse Q (about 1400 for mode 1 here, 360 for mode 10).
    pub const MONOCHORD_CELLO_G_A_T1: StringSpec = StringSpec {
        name: "cello G (A T1, monochord)",
        frequency: 98.0,
        length: 0.70,
        tension: 145.31,
        loss: Loss::Measured(DampingCurve {
            floor: 2.9e-4,
            at_1khz: 5.4e-4,
            exponent: 3.54,
        }),
        bending_stiffness: 3.032e-4,
        torsion: Some(TorsionSpec {
            impedance: 3.5,
            frequency: 5.5 * 98.0,
            q: 50.0,
        }),
    };
}
