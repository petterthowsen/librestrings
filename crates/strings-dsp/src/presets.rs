//! Physical data for instruments. Values are typical, not measured; refine by ear.

pub mod violin {
    use crate::body::{BodyMode, BodySpec, DenseModes, Hill};
    use crate::instrument::{ForceLimits, InstrumentSpec, ThermalBand};
    use crate::stage::Placement;
    use crate::{FrictionParams, Loss, StringSpec};

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

    /// The violin's signature modes, from Woodhouse's measurements of one
    /// violin (euphonics.org, 5.3, Fig. 5): A0 at 272 Hz (the air resonance),
    /// CBR at 407 Hz, and the two "baseball" modes B1− at 462 Hz and B1+ at
    /// 551 Hz, the strongest radiators. Damping (1.5–2.5%) and signs are
    /// estimates; the levels are fitted to the recorded notes (`compare`,
    /// then `fit-body`; PLAN.md "The body fitted to recordings").
    const SIGNATURE_MODES: [BodyMode; 4] = [
        BodyMode {
            frequency: 272.0,
            damping: 0.02,
            gain: 1.8,
        },
        BodyMode {
            frequency: 407.0,
            damping: 0.02,
            gain: 0.1,
        },
        BodyMode {
            frequency: 462.0,
            damping: 0.015,
            gain: 0.25,
        },
        BodyMode {
            frequency: 551.0,
            damping: 0.015,
            gain: -1.01,
        },
    ];

    /// The dense modes' envelope, fitted to the recorded notes (`fit-body`):
    /// a dip at 1.3 kHz, a rise at 3.5 kHz and air around 6 kHz. The estimate
    /// it replaced, a bridge hill at 2.3 kHz (euphonics.org, 5.3), left the
    /// model 8–11 dB too strong at 1.3–1.8 kHz; the recordings show the
    /// strongest radiation lower, at 300–400 Hz (see `BODY`).
    const HILLS: [Hill; 4] = [
        Hill {
            frequency: 1256.0,
            width: 0.3,
            gain: -0.7,
        },
        Hill {
            frequency: 2833.0,
            width: 0.3,
            gain: -0.9,
        },
        Hill {
            frequency: 3525.0,
            width: 0.3,
            gain: 2.27,
        },
        Hill {
            frequency: 6024.0,
            width: 0.3,
            gain: 1.86,
        },
    ];

    /// The dense modes start at 300 Hz, between A0 and CBR: with the four
    /// listed modes alone the body passed 300–400 Hz 20–30 dB weaker than
    /// the recorded notes had it (they started at 500 Hz). The seed is the
    /// best of twelve for the fit.
    pub const BODY: BodySpec = BodySpec {
        modes: &SIGNATURE_MODES,
        dense: DenseModes {
            from: 300.0,
            to: 10000.0,
            count: 70,
            damping: 0.03,
            level: 0.424,
            rolloff: 2721.0,
            hills: &HILLS,
            seed: 0x5eed_f1d7,
        },
    };

    /// The Helmholtz band with thermal friction (`calibrate --instrument
    /// violin --thermal`, 96 kHz).
    const THERMAL_BAND: ThermalBand = ThermalBand {
        friction: super::cello::THERMAL,
        force_limits: [
            ForceLimits {
                lower: 0.130,
                lower_exponent: -1.753,
                upper: 1.326,
                upper_exponent: -1.316,
            },
            ForceLimits {
                lower: 0.095,
                lower_exponent: -1.831,
                upper: 0.582,
                upper_exponent: -1.664,
            },
            ForceLimits {
                lower: 0.092,
                lower_exponent: -1.807,
                upper: 0.821,
                upper_exponent: -1.562,
            },
            ForceLimits {
                lower: 0.152,
                lower_exponent: -1.597,
                upper: 1.710,
                upper_exponent: -1.330,
            },
        ],
        extension_limits: None,
    };

    /// Fitted by `strings-render calibrate --instrument violin --sample-rate
    /// 96000`, as for the cello (see [`super::cello`]). Band positions 0.5–0.8
    /// give prompt Helmholtz motion in 88–100% of checked cases (the E string
    /// at 0.8 is the lowest). Without the bow hair the G string's band was
    /// found in only 21 of 48 columns and gave 54–83%.
    const FORCE_LIMITS: [ForceLimits; 4] = [
        ForceLimits {
            lower: 0.378,
            lower_exponent: -1.533,
            upper: 5.447,
            upper_exponent: -0.846,
        },
        ForceLimits {
            lower: 0.163,
            lower_exponent: -1.805,
            upper: 5.546,
            upper_exponent: -0.850,
        },
        ForceLimits {
            lower: 0.056,
            lower_exponent: -2.090,
            upper: 7.471,
            upper_exponent: -0.742,
        },
        ForceLimits {
            lower: 0.047,
            lower_exponent: -2.046,
            upper: 5.389,
            upper_exponent: -0.880,
        },
    ];

    /// The Phase 1 strings (one-pole loss, no stiffness or torsion), played
    /// with the cello's bow hair: the same kind of bow, not refitted. It
    /// doubles the G string's Helmholtz region, where a rigid bow left a
    /// narrow, patchy band and slow attacks.
    pub const INSTRUMENT: InstrumentSpec = InstrumentSpec {
        name: "violin",
        strings: STRINGS,
        friction: FrictionParams {
            mu_s: 0.8,
            mu_d: 0.3,
            v0: 0.1,
        },
        // The cello's hair touching at one point: with the width, the band
        // shrinks by about a third (STATUS.md item 53).
        hair: Some(crate::BowHair {
            width: 0.0,
            ..super::cello::HAIR
        }),
        bow_noise: super::cello::BOW_NOISE,
        thermal: None,
        thermal_band: Some(THERMAL_BAND),
        body: BODY,
        force_limits: FORCE_LIMITS,
        extension: None,
        reach: 24.0,
        // Wider than the cello's: the violin strings have no flat zone (no
        // torsion). Closer than 0.065, notes high on the E string miss their
        // pitch; farther than about 0.16 at pp, attacks hold multiple slips.
        beta: (0.16, 0.065),
        // Flautando moves toward the fingerboard. At the band's lower edge
        // there, attacks hold multiple slips; a little above it they start.
        speed: (0.04, 0.5),
        bow_distance: 0.024,
        tasto: 0.2,
        flautando: 0.3,
        // Quiet attacks aren't slowed: far from the bridge, slow attacks hold
        // multiple slips (PLAN.md "Phase 5: the violin").
        pp_attack: 0.0,
        // Not eased, as the viola: no recorded violin notes to justify it, and
        // the wander test's E6 at ff went 10 cents sharp with it.
        quiet_ease: 0.0,
        // Matched to the cello's median level on the example scales (the
        // violin strings' impedance is a third of the cello's); not yet by ear.
        // 0.5 until the body was fitted to recordings, which made it 6.2 dB
        // quieter.
        output_gain: 1.02,
        seat: Placement::VIOLINS,
    };
}

/// The viola, built like the violin: flexible strings with the one-pole
/// loss, played with the cello's bow hair.
pub mod viola {
    use crate::body::{BodyMode, BodySpec, DenseModes, Hill};
    use crate::instrument::{ForceLimits, InstrumentSpec, ThermalBand};
    use crate::stage::Placement;
    use crate::{FrictionParams, Loss, StringSpec};

    /// Vibrating length (m): Larsen's for its viola tension chart, a 16"
    /// (406 mm) body. Violas vary more than any other string instrument
    /// (about 370–420 mm).
    const LENGTH: f32 = 0.37;

    /// Kilograms-force to newtons.
    const KG: f32 = 9.807;

    /// A string with the one-pole loss. The decay times continue the
    /// violin's (2.0 s on its G) down to the C; they are guesses.
    const fn string(name: &'static str, frequency: f32, tension: f32, t60: f32) -> StringSpec {
        StringSpec {
            name,
            frequency,
            length: LENGTH,
            tension,
            loss: Loss::OnePole { t60, lowpass: 0.5 },
            bending_stiffness: 0.0,
            torsion: None,
        }
    }

    /// Open strings, lowest first: Larsen Original, medium (A 8.0 kg, the
    /// others 4.9 kg).
    pub const STRINGS: [StringSpec; 4] = [
        string("C", 130.81, 4.9 * KG, 2.2),
        string("G", 196.00, 4.9 * KG, 2.0),
        string("D", 293.66, 4.9 * KG, 1.8),
        string("A", 440.00, 8.0 * KG, 1.5),
    ];

    pub fn string_named(name: &str) -> Option<&'static StringSpec> {
        STRINGS.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// The viola's main resonances from Jóhannsson (a maker's measurements):
    /// air modes at 230 Hz (A0) and 330–360 Hz, body modes around 350 and
    /// 440 Hz, which we take as B1− and B1+ (the violin's are at 462 and 551
    /// Hz). The CBR mode sits below B1− at the violin's ratio (407/462). As
    /// on the violin, damping and signs are estimates and the levels fitted
    /// to the recorded notes (PLAN.md "The body fitted to recordings").
    const SIGNATURE_MODES: [BodyMode; 4] = [
        BodyMode {
            frequency: 230.0,
            damping: 0.02,
            gain: 0.2,
        },
        BodyMode {
            frequency: 310.0,
            damping: 0.02,
            gain: 1.15,
        },
        BodyMode {
            frequency: 350.0,
            damping: 0.015,
            gain: 0.25,
        },
        BodyMode {
            frequency: 440.0,
            damping: 0.015,
            gain: -4.32,
        },
    ];

    /// The dense modes' envelope, fitted to the recorded notes (`fit-body`):
    /// a dip at 800 Hz and a rise toward 10 kHz. It replaced an estimated
    /// bridge hill at 2 kHz, which left the model 10–15 dB too weak below
    /// 300 Hz, relative to the rest.
    const HILLS: [Hill; 4] = [
        Hill {
            frequency: 811.0,
            width: 0.3,
            gain: -0.59,
        },
        Hill {
            frequency: 3277.0,
            width: 0.3,
            gain: 0.69,
        },
        Hill {
            frequency: 5718.0,
            width: 0.3,
            gain: 1.47,
        },
        Hill {
            frequency: 12000.0,
            width: 0.3,
            gain: 3.91,
        },
    ];

    /// The dense modes start at 200 Hz, below A0, to fill the low end the
    /// recordings have (they started at 400 Hz). The seed is the best of
    /// twelve for the fit.
    pub const BODY: BodySpec = BodySpec {
        modes: &SIGNATURE_MODES,
        dense: DenseModes {
            from: 200.0,
            to: 10000.0,
            count: 73,
            damping: 0.03,
            level: 1.722,
            rolloff: 1282.0,
            hills: &HILLS,
            seed: 0x5eed_710e,
        },
    };

    /// The Helmholtz band with thermal friction (`calibrate --instrument
    /// viola --thermal`, 96 kHz).
    const THERMAL_BAND: ThermalBand = ThermalBand {
        friction: super::cello::THERMAL,
        force_limits: [
            ForceLimits {
                lower: 0.215,
                lower_exponent: -1.597,
                upper: 2.738,
                upper_exponent: -0.993,
            },
            ForceLimits {
                lower: 0.123,
                lower_exponent: -1.781,
                upper: 1.191,
                upper_exponent: -1.357,
            },
            ForceLimits {
                lower: 0.094,
                lower_exponent: -1.830,
                upper: 0.608,
                upper_exponent: -1.641,
            },
            ForceLimits {
                lower: 0.088,
                lower_exponent: -1.829,
                upper: 1.508,
                upper_exponent: -1.341,
            },
        ],
        extension_limits: None,
    };

    /// Fitted by `strings-render calibrate --instrument viola --sample-rate
    /// 96000`, as for the cello (see [`super::cello`]). Band positions 0.5–0.8
    /// give prompt Helmholtz motion in 92–100% of checked cases, as on the
    /// violin; the C string's band was found in 44 of 48 columns.
    const FORCE_LIMITS: [ForceLimits; 4] = [
        ForceLimits {
            lower: 0.706,
            lower_exponent: -1.315,
            upper: 4.110,
            upper_exponent: -0.922,
        },
        ForceLimits {
            lower: 0.405,
            lower_exponent: -1.505,
            upper: 5.297,
            upper_exponent: -0.855,
        },
        ForceLimits {
            lower: 0.152,
            lower_exponent: -1.828,
            upper: 6.141,
            upper_exponent: -0.818,
        },
        ForceLimits {
            lower: 0.055,
            lower_exponent: -2.090,
            upper: 7.345,
            upper_exponent: -0.752,
        },
    ];

    pub const INSTRUMENT: InstrumentSpec = InstrumentSpec {
        name: "viola",
        strings: STRINGS,
        friction: FrictionParams {
            mu_s: 0.8,
            mu_d: 0.3,
            v0: 0.1,
        },
        // The cello's hair touching at one point: with the width, the band
        // shrinks by about a third (STATUS.md item 53).
        hair: Some(crate::BowHair {
            width: 0.0,
            ..super::cello::HAIR
        }),
        bow_noise: super::cello::BOW_NOISE,
        thermal: None,
        thermal_band: Some(THERMAL_BAND),
        body: BODY,
        force_limits: FORCE_LIMITS,
        extension: None,
        reach: 24.0,
        beta: (0.16, 0.065),
        speed: (0.04, 0.5),
        bow_distance: 0.024,
        tasto: 0.2,
        flautando: 0.3,
        pp_attack: 0.0,
        // Not eased: at pp its low notes fell into multiple slips or lost
        // their pitch (the seed sweep failed 14 checks).
        quiet_ease: 0.0,
        // Matched to the cello's median level on the example scales; not yet
        // by ear. 0.4 until the body was fitted to recordings, which made it
        // 6.5 dB louder.
        output_gain: 0.19,
        seat: Placement::VIOLAS,
    };
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
    /// - Q = 50 for every torsional mode: Mores finds torsional Q about an order
    ///   of magnitude below the transverse Q (about 1400 for mode 1 here, 360
    ///   for mode 10); Bavu et al. (2005) more than fifty times lower. Constant
    ///   Q across modes follows Woodhouse & Loach (1999).
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
    use crate::instrument::{ForceLimits, InstrumentSpec, ThermalBand};
    use crate::stage::Placement;
    use crate::{
        BowHair, BowNoise, DampingCurve, FrictionParams, Loss, StringSpec, ThermalFriction,
        TorsionSpec,
    };

    /// Vibrating length (m): the mdw monochord's, within the usual 690–700 mm.
    const LENGTH: f32 = 0.70;

    /// Damping of the measured G string (monochord, rigid terminations) plus a
    /// flat 7e-4 for energy lost into the body through the bridge. That term is
    /// an estimate: it gives the open G a decay (−60 dB) of about 11 s, against
    /// about 39 s on the monochord.
    pub(super) const DAMPING: DampingCurve = DampingCurve {
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
    ///
    /// The width is about a cello bow's hair ribbon, flat on the string as
    /// the robot bowed it in the measurement (an estimate). It brings the
    /// measured string's Helmholtz region to 86 / 83 / 85% of the measured one
    /// (PLAN.md "The bow's width"); the stiffness and damping above are still
    /// the best fit with it.
    pub const HAIR: BowHair = BowHair {
        stiffness: 1000.0,
        damping: 3.0,
        width: 0.012,
    };

    /// Bow noise, fitted to the recorded cello notes' harmonic-to-noise
    /// ratio (`strings-render compare`: 31–32.5 dB at pp–ff, recorded 27–32;
    /// without it 43–49). The bandwidth hardly matters: gated to the slips,
    /// the noise is broadband anyway. The other instruments use the same
    /// (PLAN.md "Bow noise").
    pub const BOW_NOISE: BowNoise = BowNoise {
        level: 0.065,
        cutoff: 2000.0,
    };

    /// Body resonances below 300 Hz. Frequencies from Zhang, Woodhouse &
    /// Stoppani, "Motion of the cello bridge", JASA 140, 2636 (2016),
    /// doi:10.1121/1.4964609: 97 Hz (A0),
    /// 173 Hz (the main body resonance, where that cello's wolf note sits),
    /// 200, 209 and 281 Hz; T1 at 140 Hz from Bynum & Rossing. Damping (1–2.5%)
    /// and relative levels are estimates, to be refined by ear or measurement.
    ///
    /// The modes at 118 and 144 Hz are not from the literature: they stand for
    /// modes a real body has between A0 and the dense ones. With the six listed
    /// modes alone the body passed A#2–C3 25–33 dB below its level at
    /// 200–400 Hz, and the C and G strings' fundamentals from G2 to D3 came out
    /// 7–31 dB below the harmonic power, where the recorded notes (Iowa) and
    /// the model's own bridge force have 1–5 dB. Fitted to the recordings
    /// (PLAN.md "The body's low end"); below A0 (C2–F2) the recorded
    /// fundamental is weak too, and stays so.
    const SIGNATURE_MODES: [BodyMode; 8] = [
        BodyMode {
            frequency: 97.0,
            damping: 0.025,
            gain: 0.7,
        },
        BodyMode {
            frequency: 118.0,
            damping: 0.03,
            gain: -1.3,
        },
        BodyMode {
            frequency: 140.0,
            damping: 0.015,
            gain: 0.4,
        },
        BodyMode {
            frequency: 144.0,
            damping: 0.04,
            gain: 1.7,
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

    /// The dense modes' envelope, fitted to the recorded notes' spectra
    /// (`strings-render compare`, then `fit-body`; PLAN.md "The body fitted
    /// to recordings"): a broad rise around 270 Hz, the bridge hill at
    /// 1.5 kHz, a stronger rise at 2.5 kHz and air up to 10 kHz. The earlier
    /// estimates (a 1.3 kHz hill three times the base level, a 2.2 kHz one
    /// and a rolloff at 3.5 kHz) left the model 8–13 dB too strong around
    /// 0.8–1.3 kHz and 4–8 dB weak above 5 kHz. The 270 Hz rise began as an
    /// estimate that gave low notes their weight (PLAN.md "Phase 3 notes:
    /// tuning and first listening"); the fit kept the body below 250 Hz as it
    /// was (`fit-body --keep-below 250`).
    const HILLS: [Hill; 4] = [
        Hill {
            frequency: 267.0,
            width: 0.93,
            gain: 1.54,
        },
        Hill {
            frequency: 1498.0,
            width: 0.3,
            gain: 0.6,
        },
        Hill {
            frequency: 2496.0,
            width: 0.3,
            gain: 2.19,
        },
        Hill {
            frequency: 9000.0,
            width: 0.3,
            gain: 0.83,
        },
    ];

    /// The dense modes start at 150 Hz, among the listed ones: a real body has
    /// many more modes there than the six measured ones. They reach 9.9 kHz:
    /// the first 59 end at 6 kHz, and the fit added 8 at the same spacing,
    /// which leaves those 59 where they were.
    pub const BODY: BodySpec = BodySpec {
        modes: &SIGNATURE_MODES,
        dense: DenseModes {
            from: 150.0,
            to: 9894.0,
            count: 67,
            damping: 0.03,
            level: 0.304,
            rolloff: 8343.0,
            hills: &HILLS,
            seed: 0x5eed_c0de,
        },
    };

    /// Fitted by `strings-render calibrate --sample-rate 96000` (the strings'
    /// rate with the default 2× oversampling) to simulated Schelleng maps of
    /// each string (bow speeds 0.05–0.4 m/s, β 0.04–0.25), counting only cells
    /// that are Helmholtz within 0.15 s of the bow starting, with the hair's
    /// width. Band positions 0.5–0.8 give such prompt Helmholtz motion in
    /// 96–100% of checked cases. The model's band sits above the measured
    /// string's (at β = 0.1, v_b = 0.1 m/s the G string's is 1.17–3.12 N;
    /// measured 0.31–1.89 N): the lower-limit gap of PLAN.md 4.2.
    /// Thermal friction for the bow: Woodhouse's model, with rosin that
    /// softens at higher temperatures the faster the bow moves (PLAN.md
    /// "Thermal friction"). All four instruments use it.
    pub const THERMAL: ThermalFriction = ThermalFriction {
        speed_exponent: 0.5,
        ..ThermalFriction::WOODHOUSE
    };

    /// The Helmholtz band with thermal friction (`calibrate --thermal`,
    /// 96 kHz).
    const THERMAL_BAND: ThermalBand = ThermalBand {
        friction: THERMAL,
        force_limits: [
            ForceLimits {
                lower: 0.348,
                lower_exponent: -1.256,
                upper: 3.351,
                upper_exponent: -0.670,
            },
            ForceLimits {
                lower: 0.554,
                lower_exponent: -1.074,
                upper: 3.825,
                upper_exponent: -0.674,
            },
            ForceLimits {
                lower: 1.174,
                lower_exponent: -0.894,
                upper: 4.022,
                upper_exponent: -0.753,
            },
            ForceLimits {
                lower: 0.874,
                lower_exponent: -0.934,
                upper: 5.712,
                upper_exponent: -0.661,
            },
        ],
        extension_limits: None,
    };

    const FORCE_LIMITS: [ForceLimits; 4] = [
        ForceLimits {
            lower: 1.103,
            lower_exponent: -0.994,
            upper: 4.595,
            upper_exponent: -0.692,
        },
        ForceLimits {
            lower: 0.542,
            lower_exponent: -1.220,
            upper: 5.017,
            upper_exponent: -0.666,
        },
        ForceLimits {
            lower: 0.245,
            lower_exponent: -1.447,
            upper: 4.152,
            upper_exponent: -0.785,
        },
        ForceLimits {
            lower: 0.151,
            lower_exponent: -1.505,
            upper: 5.632,
            upper_exponent: -0.680,
        },
    ];

    pub const INSTRUMENT: InstrumentSpec = InstrumentSpec {
        name: "cello",
        strings: STRINGS,
        // μs from the measured attacks (`guettler`, PLAN.md "Attacks against
        // measured data"): at 0.8 the model needed twice the measured bow force
        // for a given bow acceleration.
        friction: FrictionParams {
            mu_s: 0.9,
            mu_d: 0.3,
            v0: 0.1,
        },
        hair: Some(HAIR),
        bow_noise: BOW_NOISE,
        thermal: None,
        thermal_band: Some(THERMAL_BAND),
        body: BODY,
        force_limits: FORCE_LIMITS,
        extension: None,
        reach: 24.0,
        // Above β ≈ 0.12 the model's cello strings play up to 45 cents flat
        // (STATUS.md), so the mapping stays below.
        beta: (0.115, 0.07),
        speed: (0.04, 0.5),
        // 3.5 cm on the C string, 1.4 cm on the A.
        bow_distance: 0.024,
        tasto: 0.0,
        flautando: crate::performer::PRESSURE_FLAUTANDO,
        // With μs 0.9 quiet attacks can be faster (it was 1.6 at μs 0.8).
        pp_attack: 0.6,
        // Quiet strokes ease down the band once going: a darker pp
        // (PLAN.md "Soft, dark pp").
        quiet_ease: 0.5,
        // 0.065 until the body was fitted to recordings, which made it 2.3 dB
        // quieter.
        output_gain: 0.085,
        seat: Placement::CELLOS,
    };
}

/// The double bass, built like the cello: measured-style damping, bending
/// stiffness and torsion, played with the cello's bow hair.
pub mod bass {
    use crate::body::{BodyMode, BodySpec, DenseModes, Hill};
    use crate::instrument::{Extension, ForceLimits, InstrumentSpec, ThermalBand};
    use crate::stage::Placement;
    use crate::{DampingCurve, FrictionParams, Loss, StringSpec, TorsionSpec};

    /// Vibrating length (m) of a 3/4 orchestral bass, the size the tension
    /// chart is for.
    const LENGTH: f32 = 1.06;

    /// The E string's length with a C extension: it runs on past the nut
    /// (over the scroll) a major third farther, so the same string at the
    /// same tension sounds C1 open. Orchestral basses stop it at E with the
    /// extension's gates; here E1 is a stopped note, and from E1 up the
    /// string vibrates at the same lengths as without the extension.
    const EXTENDED_LENGTH: f32 = LENGTH * 1.259_921;

    /// Bending stiffness (N·m²), shared by the strings as on the cello. Not
    /// measured: a solid steel core of 0.85 mm (the cello G's acts like one
    /// of 0.42 mm; bass gauges are 1.2–2.6 mm against its 0.95 mm), less
    /// for the rope cores of most bass strings. It gives B = 1.4e-4 on the E
    /// string stopped at E1 (0.9e-4 on its open C, 1.26 × longer) and 1.5e-4
    /// on the G, about 3.5 × the cello G's.
    const BENDING_STIFFNESS: f32 = 5.0e-3;

    /// Torsion estimated as for the cello (f_t = 5.5·f0, Z_t = 3.3 × Z, Q 50)
    /// and the cello's damping curve: no bass string has been measured.
    const fn string(name: &'static str, frequency: f32, length: f32, tension: f32) -> StringSpec {
        let impedance = tension / (2.0 * length * frequency);
        StringSpec {
            name,
            frequency,
            length,
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

    /// The cello's (the measured G string's plus loss into the body). With
    /// it the E string rings for about 27 s (−60 dB), longer than the cello's
    /// open G: the same damping ratio at a lower pitch.
    const DAMPING: DampingCurve = super::cello::DAMPING;

    /// Pounds-force to newtons.
    const LBF: f32 = 4.448;

    /// Open strings, lowest first: Thomastik Spirocore Orchestra, medium
    /// (Mittel), on a 3/4 bass at 106 cm (G 67.2, D 68.3, A 70.5, E 72.8 lb).
    /// The E string has a C extension ([`EXTENDED_LENGTH`]), so it is named
    /// for its open C1; its tension and impedance are the E string's.
    pub const STRINGS: [StringSpec; 4] = [
        string("C", 32.70, EXTENDED_LENGTH, 72.8 * LBF),
        string("A", 55.00, LENGTH, 70.5 * LBF),
        string("D", 73.42, LENGTH, 68.3 * LBF),
        string("G", 98.00, LENGTH, 67.2 * LBF),
    ];

    pub fn string_named(name: &str) -> Option<&'static StringSpec> {
        STRINGS.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Body resonances from Brown's measurements of four basses (A. W.
    /// Brown, "Acoustical studies on the flat-backed and round-backed
    /// double bass", dissertation, mdw Vienna 2004, 6.2): A0 at 65–67 Hz,
    /// the coupled T1/A1 at 115 Hz (among the strongest radiators) and A2 at
    /// 150–158 Hz. Damping and signs are estimates, set like the cello's;
    /// the levels are fitted to the recorded notes (PLAN.md "The body fitted
    /// to recordings"), which took A0 and T1 down to a quarter against the
    /// dense modes.
    const SIGNATURE_MODES: [BodyMode; 3] = [
        BodyMode {
            frequency: 66.0,
            damping: 0.025,
            gain: 0.17,
        },
        BodyMode {
            frequency: 115.0,
            damping: 0.02,
            gain: 0.3,
        },
        BodyMode {
            frequency: 155.0,
            damping: 0.02,
            gain: -2.4,
        },
    ];

    /// The dense modes' envelope, fitted to the recorded notes (`fit-body`):
    /// a rise around 100 Hz, a broad dip centred on 450 Hz and a rise at
    /// 1.4 kHz. It replaced estimates (a rise at 90 Hz and a bridge hill at
    /// 700 Hz, with the level rolling off from 1.5 kHz, as Brown finds the
    /// radiation falling steeply above 1 kHz), which left the model 12–17 dB
    /// too strong at 400–800 Hz and 10–20 dB too weak above 4 kHz.
    const HILLS: [Hill; 4] = [
        Hill {
            frequency: 104.0,
            width: 0.39,
            gain: 1.29,
        },
        Hill {
            frequency: 456.0,
            width: 1.5,
            gain: -0.44,
        },
        Hill {
            frequency: 1369.0,
            width: 0.45,
            gain: 0.71,
        },
        Hill {
            frequency: 2614.0,
            width: 0.34,
            gain: -0.41,
        },
    ];

    /// The dense modes reach 9.5 kHz: the first 59 end at 4 kHz, and the fit
    /// added 13 at the same spacing, which leaves those 59 where they were.
    pub const BODY: BodySpec = BodySpec {
        modes: &SIGNATURE_MODES,
        dense: DenseModes {
            from: 80.0,
            to: 9471.0,
            count: 72,
            damping: 0.03,
            level: 2.006,
            rolloff: 4593.0,
            hills: &HILLS,
            seed: 0x5eed_ba55,
        },
    };

    /// The Helmholtz band with thermal friction (`calibrate --instrument
    /// bass --thermal`, 96 kHz).
    const THERMAL_BAND: ThermalBand = ThermalBand {
        friction: super::cello::THERMAL,
        force_limits: [
            ForceLimits {
                lower: 0.106,
                lower_exponent: -1.892,
                upper: 2.270,
                upper_exponent: -0.663,
            },
            ForceLimits {
                lower: 0.184,
                lower_exponent: -1.504,
                upper: 2.907,
                upper_exponent: -0.599,
            },
            ForceLimits {
                lower: 0.249,
                lower_exponent: -1.348,
                upper: 3.038,
                upper_exponent: -0.665,
            },
            ForceLimits {
                lower: 0.281,
                lower_exponent: -1.286,
                upper: 4.311,
                upper_exponent: -0.586,
            },
        ],
        extension_limits: Some(ForceLimits {
            lower: 0.150,
            lower_exponent: -1.623,
            upper: 2.719,
            upper_exponent: -0.542,
        }),
    };

    /// Fitted by `strings-render calibrate --instrument bass --sample-rate
    /// 96000`, as for the cello (see [`super::cello`]), with the hair's width.
    /// Recalibrated with the cello's μs 0.9: band positions 0.5–0.8 give
    /// prompt Helmholtz motion in 79–100% of checked cases (71–100% at μs
    /// 0.8), the lowest β worst. The E string's band was found in only 15 of 48
    /// columns ([`EXTENSION`]), the open C's in 13 (the G string's in 42), and it is narrow,
    /// one or two rows of the map. That holds without torsion or stiffness, with the one-pole loss,
    /// and with stiffer or softer bow hair (PLAN.md "Phase 5: viola and
    /// double bass").
    const FORCE_LIMITS: [ForceLimits; 4] = [
        ForceLimits {
            lower: 2.035,
            lower_exponent: -0.668,
            upper: 4.163,
            upper_exponent: -0.597,
        },
        ForceLimits {
            lower: 1.393,
            lower_exponent: -0.824,
            upper: 4.480,
            upper_exponent: -0.582,
        },
        ForceLimits {
            lower: 1.046,
            lower_exponent: -0.943,
            upper: 5.427,
            upper_exponent: -0.540,
        },
        ForceLimits {
            lower: 0.932,
            lower_exponent: -0.969,
            upper: 6.929,
            upper_exponent: -0.477,
        },
    ];

    /// The C extension ([`EXTENDED_LENGTH`]), with the band of the string
    /// stopped at its gates: the E string's without the extension. The open
    /// C's band lies about 15% higher at the β of stopped notes, where it
    /// makes G1–E2 at ff raucous; with this one, C1 and D1 settle late
    /// (0.2–0.3 s). Blended in between (PLAN.md "The bass's C extension").
    const EXTENSION: Extension = Extension {
        semitones: 4.0,
        force_limits: ForceLimits {
            lower: 1.332,
            lower_exponent: -0.812,
            upper: 4.500,
            upper_exponent: -0.540,
        },
    };

    pub const INSTRUMENT: InstrumentSpec = InstrumentSpec {
        name: "bass",
        strings: STRINGS,
        // The cello's μs, fitted to its measured attacks (PLAN.md "Attacks
        // against measured data"); the bass has no attack data of its own.
        friction: FrictionParams {
            mu_s: 0.9,
            mu_d: 0.3,
            v0: 0.1,
        },
        // The cello's hair on a bass bow's wider ribbon (estimated).
        hair: Some(crate::BowHair {
            width: 0.014,
            ..super::cello::HAIR
        }),
        bow_noise: super::cello::BOW_NOISE,
        thermal: None,
        thermal_band: Some(THERMAL_BAND),
        body: BODY,
        force_limits: FORCE_LIMITS,
        extension: Some(EXTENSION),
        reach: 24.0,
        beta: (0.115, 0.07),
        // Slower at ff than the other instruments (0.5 m/s): faster, high
        // positions at ff turn raucous or miss their pitch by 30–90 cents.
        speed: (0.04, 0.3),
        // Farther than the others (0.024): 11.9 cm on the E string, 4.6 cm on
        // the G. Closer, high positions at ff miss their pitch; farther, high
        // notes at pp hold multiple slips (PLAN.md "Phase 5: viola and double
        // bass").
        bow_distance: 0.032,
        tasto: 0.0,
        flautando: crate::performer::PRESSURE_FLAUTANDO,
        // Slower than the cello's (0.6): with μs 0.9, faster quiet attacks
        // on C1–E1 take 0.2–0.8 s to settle (seed sweep failures: 15 of 1296
        // at 1.6, 0 at 2.2, 2 ff attacks at 3.0).
        pp_attack: 2.2,
        // As the cello's.
        quiet_ease: 0.5,
        // Matched to the cello's median level on the example scales; not yet
        // by ear. 0.06 until the body was fitted to recordings, which made it
        // 10.9 dB louder.
        output_gain: 0.017,
        seat: Placement::BASSES,
    };
}
