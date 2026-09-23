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

/// Solo cello (Phase 2). The G string is the measured reference string; the
/// others take their tension from a published set and everything else from the
/// G string, scaled where the physics says how.
pub mod cello {
    use crate::body::{BodyMode, BodySpec, DenseModes, Hill};
    use crate::instrument::{ForceLimits, InstrumentSpec};
    use crate::{BowHair, DampingCurve, FrictionParams, Loss, StringSpec, TorsionSpec};

    /// Vibrating length (m): the mdw monochord's, within the usual 690–700 mm.
    const LENGTH: f32 = 0.70;

    /// Damping of the measured G string (monochord, rigid terminations) plus a
    /// flat 7e-4 for energy lost into the body through the bridge. That term is
    /// an estimate: it gives the open G a decay (−60 dB) of about 11 s, against
    /// about 39 s on the monochord.
    const DAMPING: DampingCurve = DampingCurve {
        floor: 2.9e-4 + 7.0e-4,
        at_1khz: 5.4e-4,
        exponent: 3.54,
    };

    /// Bending stiffness (N·m²). Measured on the G string only; the other
    /// strings of a set have cores of similar size, so they share it (an
    /// estimate: B comes out between 3.5e-5 and 4.6e-5).
    const BENDING_STIFFNESS: f32 = 3.032e-4;

    /// Torsion estimated as for the reference G string (see
    /// [`super::reference::MONOCHORD_CELLO_G_A_T1`]): f_t = 5.5·f0 and
    /// Z_t = κ·μ·c_t with κ = 0.6, which is 0.6·5.5 = 3.3 × Z; Q 50.
    const fn string(name: &'static str, frequency: f32, tension: f32) -> StringSpec {
        let impedance = tension / (2.0 * LENGTH * frequency);
        StringSpec {
            name,
            frequency,
            length: LENGTH,
            tension,
            loss: Loss::Measured(DAMPING),
            bending_stiffness: BENDING_STIFFNESS,
            torsion: Some(TorsionSpec {
                impedance: 3.3 * impedance,
                frequency: 5.5 * frequency,
                q: 50.0,
            }),
        }
    }

    /// Pounds-force to newtons.
    const LBF: f32 = 4.448;

    /// Open strings, lowest first. C, D and A tensions are Larsen Standard
    /// (medium) at 700 mm, from the Aitchison & Mnatzaganian tension chart;
    /// the G is the measured string A T1 (145.3 N; Larsen's G is 122.8 N).
    pub const STRINGS: [StringSpec; 4] = [
        string("C", 65.41, 30.1 * LBF),
        string("G", 98.00, 145.31),
        string("D", 146.83, 29.7 * LBF),
        string("A", 220.00, 39.3 * LBF),
    ];

    pub fn string_named(name: &str) -> Option<&'static StringSpec> {
        STRINGS.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Bow hair compliance, fitted to the measured Schelleng diagram of the G
    /// string (`strings-render measured --hair-stiffness 1000 --hair-damping 3`
    /// gives about twice the Helmholtz points of a rigid bow; PLAN.md "Phase 2 notes").
    /// The damping is close to the wave impedance of the hairs in contact; the
    /// stiffness is softer than the hair ribbon alone and stands for the whole
    /// contact (hairs, stick and hand). Not measured.
    pub const HAIR: BowHair = BowHair {
        stiffness: 1000.0,
        damping: 3.0,
    };

    /// Body resonances below 300 Hz. Frequencies from Zhang, Woodhouse &
    /// Stoppani, "Motion of the cello bridge", JASA 140, 2636 (2016),
    /// doi:10.1121/1.4964609: 97 Hz (A0),
    /// 173 Hz (the main body resonance, where that cello's wolf note sits),
    /// 200, 209 and 281 Hz; T1 at 140 Hz from Bynum & Rossing. Damping (1–2.5%)
    /// and relative levels are estimates, to be refined by ear or measurement.
    const SIGNATURE_MODES: [BodyMode; 6] = [
        BodyMode {
            frequency: 97.0,
            damping: 0.025,
            gain: 0.7,
        },
        BodyMode {
            frequency: 140.0,
            damping: 0.015,
            gain: 0.4,
        },
        BodyMode {
            frequency: 173.0,
            damping: 0.012,
            gain: 1.0,
        },
        BodyMode {
            frequency: 200.0,
            damping: 0.015,
            gain: -0.5,
        },
        BodyMode {
            frequency: 209.0,
            damping: 0.015,
            gain: 0.6,
        },
        BodyMode {
            frequency: 281.0,
            damping: 0.02,
            gain: 0.5,
        },
    ];

    /// Two "bridge hills": the in-situ bridge resonance at 1.2–1.5 kHz (Zhang
    /// et al.) and a second rise at 2–2.3 kHz (euphonics.org, 5.3).
    const HILLS: [Hill; 2] = [
        Hill {
            frequency: 1300.0,
            width: 0.8,
            gain: 2.0,
        },
        Hill {
            frequency: 2200.0,
            width: 0.6,
            gain: 1.2,
        },
    ];

    pub const BODY: BodySpec = BodySpec {
        modes: &SIGNATURE_MODES,
        dense: DenseModes {
            from: 300.0,
            to: 6000.0,
            count: 48,
            damping: 0.03,
            level: 0.35,
            rolloff: 3500.0,
            hills: &HILLS,
            seed: 0x5eed_c0de,
        },
    };

    /// Fitted by `strings-render calibrate` to simulated Schelleng maps of each
    /// string (bow speeds 0.05–0.4 m/s, β 0.04–0.25). Band positions 0.5–0.8
    /// give Helmholtz motion in 92–97% of checked cases. The model's band sits
    /// above the measured string's (at β = 0.1, v_b = 0.1 m/s the G string's is
    /// 1.0–3.1 N; measured 0.31–1.89 N): the lower-limit gap of PLAN.md 4.2.
    const FORCE_LIMITS: [ForceLimits; 4] = [
        ForceLimits {
            lower: 1.406,
            lower_exponent: -0.898,
            upper: 8.008,
            upper_exponent: -0.551,
        },
        ForceLimits {
            lower: 1.147,
            lower_exponent: -0.927,
            upper: 8.871,
            upper_exponent: -0.522,
        },
        ForceLimits {
            lower: 0.411,
            lower_exponent: -1.246,
            upper: 11.067,
            upper_exponent: -0.503,
        },
        ForceLimits {
            lower: 0.120,
            lower_exponent: -1.583,
            upper: 11.378,
            upper_exponent: -0.486,
        },
    ];

    pub const INSTRUMENT: InstrumentSpec = InstrumentSpec {
        name: "cello",
        strings: STRINGS,
        friction: FrictionParams {
            mu_s: 0.8,
            mu_d: 0.3,
            v0: 0.1,
        },
        hair: Some(HAIR),
        body: BODY,
        force_limits: FORCE_LIMITS,
        reach: 24.0,
    };
}
