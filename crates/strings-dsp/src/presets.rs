//! Physical data for instruments. Values are typical, not measured; refine by ear.

pub mod violin {
    use crate::body::{BodyMode, BodySpec, DenseModes, Hill};
    use crate::instrument::{ForceLimits, InstrumentSpec};
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
    /// 551 Hz, the strongest radiators. Damping (1.5–2.5%), signs and levels
    /// are estimates, to be refined by ear or measurement.
    const SIGNATURE_MODES: [BodyMode; 4] = [
        BodyMode {
            frequency: 272.0,
            damping: 0.02,
            gain: 0.8,
        },
        BodyMode {
            frequency: 407.0,
            damping: 0.02,
            gain: 0.3,
        },
        BodyMode {
            frequency: 462.0,
            damping: 0.015,
            gain: 1.0,
        },
        BodyMode {
            frequency: 551.0,
            damping: 0.015,
            gain: -1.2,
        },
    ];

    /// The bridge hill, peaking around 2.3 kHz (euphonics.org, 5.3). Width
    /// and level are estimates.
    const HILLS: [Hill; 1] = [Hill {
        frequency: 2300.0,
        width: 0.6,
        gain: 2.0,
    }];

    /// The dense modes start among the B1 modes; the level falls above the
    /// bridge hill.
    pub const BODY: BodySpec = BodySpec {
        modes: &SIGNATURE_MODES,
        dense: DenseModes {
            from: 500.0,
            to: 10000.0,
            count: 60,
            damping: 0.03,
            level: 0.35,
            rolloff: 5000.0,
            hills: &HILLS,
            seed: 0x5eed_f1d1,
        },
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
        hair: Some(super::cello::HAIR),
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
        // Matched to the cello's median level on the example scales (the
        // violin strings' impedance is a third of the cello's); not yet by ear.
        output_gain: 0.5,
        seat: Placement::VIOLINS,
    };
}

/// The viola, built like the violin: flexible strings with the one-pole
/// loss, played with the cello's bow hair.
pub mod viola {
    use crate::body::{BodyMode, BodySpec, DenseModes, Hill};
    use crate::instrument::{ForceLimits, InstrumentSpec};
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
    /// on the violin, damping, signs and levels are estimates.
    const SIGNATURE_MODES: [BodyMode; 4] = [
        BodyMode {
            frequency: 230.0,
            damping: 0.02,
            gain: 0.8,
        },
        BodyMode {
            frequency: 310.0,
            damping: 0.02,
            gain: 0.3,
        },
        BodyMode {
            frequency: 350.0,
            damping: 0.015,
            gain: 1.0,
        },
        BodyMode {
            frequency: 440.0,
            damping: 0.015,
            gain: -1.2,
        },
    ];

    /// The bridge hill, a little below the violin's 2.3 kHz (a larger
    /// bridge): an estimate.
    const HILLS: [Hill; 1] = [Hill {
        frequency: 2000.0,
        width: 0.6,
        gain: 2.0,
    }];

    /// The dense modes start among the B1 modes, as on the violin.
    pub const BODY: BodySpec = BodySpec {
        modes: &SIGNATURE_MODES,
        dense: DenseModes {
            from: 400.0,
            to: 10000.0,
            count: 60,
            damping: 0.03,
            level: 0.35,
            rolloff: 4500.0,
            hills: &HILLS,
            seed: 0x5eed_7107,
        },
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
        hair: Some(super::cello::HAIR),
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
        // Matched to the cello's median level on the example scales; not yet
        // by ear.
        output_gain: 0.4,
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
    use crate::instrument::{ForceLimits, InstrumentSpec};
    use crate::stage::Placement;
    use crate::{BowHair, DampingCurve, FrictionParams, Loss, StringSpec, TorsionSpec};

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

    /// A broad rise around 250 Hz, then two "bridge hills": the in-situ
    /// bridge resonance at 1.2–1.5 kHz (Zhang et al.) and a second rise at
    /// 2–2.3 kHz (euphonics.org, 5.3). The 250 Hz rise is not from
    /// data: with the listed modes alone the body passed only 5–10% of the
    /// power of D3 and A3 below 300 Hz, and low notes sounded light (30–50%
    /// with it). To be judged by ear.
    const HILLS: [Hill; 3] = [
        Hill {
            frequency: 250.0,
            width: 0.6,
            gain: 1.5,
        },
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

    /// The dense modes start at 150 Hz, among the listed ones: a real body has
    /// many more modes there than the six measured ones.
    pub const BODY: BodySpec = BodySpec {
        modes: &SIGNATURE_MODES,
        dense: DenseModes {
            from: 150.0,
            to: 6000.0,
            count: 59,
            damping: 0.03,
            level: 0.35,
            rolloff: 3500.0,
            hills: &HILLS,
            seed: 0x5eed_c0de,
        },
    };

    /// Fitted by `strings-render calibrate --sample-rate 96000` (the strings'
    /// rate with the default 2× oversampling) to simulated Schelleng maps of
    /// each string (bow speeds 0.05–0.4 m/s, β 0.04–0.25), counting only cells
    /// that are Helmholtz within 0.15 s of the bow starting. Band positions
    /// 0.5–0.8 give such prompt Helmholtz motion in 91–99% of checked cases.
    /// The model's band sits above the measured string's (at β = 0.1,
    /// v_b = 0.1 m/s the G string's is 1.2–2.9 N; measured 0.31–1.89 N): the
    /// lower-limit gap of PLAN.md 4.2.
    const FORCE_LIMITS: [ForceLimits; 4] = [
        ForceLimits {
            lower: 1.465,
            lower_exponent: -0.943,
            upper: 6.211,
            upper_exponent: -0.622,
        },
        ForceLimits {
            lower: 1.028,
            lower_exponent: -1.053,
            upper: 6.785,
            upper_exponent: -0.606,
        },
        ForceLimits {
            lower: 0.682,
            lower_exponent: -1.138,
            upper: 8.914,
            upper_exponent: -0.562,
        },
        ForceLimits {
            lower: 0.231,
            lower_exponent: -1.435,
            upper: 9.915,
            upper_exponent: -0.504,
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
        pp_attack: 1.6,
        output_gain: 0.065,
        seat: Placement::CELLOS,
    };
}

/// The double bass, built like the cello: measured-style damping, bending
/// stiffness and torsion, played with the cello's bow hair.
pub mod bass {
    use crate::body::{BodyMode, BodySpec, DenseModes, Hill};
    use crate::instrument::{Extension, ForceLimits, InstrumentSpec};
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
    /// 150–158 Hz. Damping, signs and levels are estimates, set like the
    /// cello's.
    const SIGNATURE_MODES: [BodyMode; 3] = [
        BodyMode {
            frequency: 66.0,
            damping: 0.025,
            gain: 0.7,
        },
        BodyMode {
            frequency: 115.0,
            damping: 0.02,
            gain: 1.2,
        },
        BodyMode {
            frequency: 155.0,
            damping: 0.02,
            gain: -0.6,
        },
    ];

    /// A broad rise between A0 and T1, as the cello's at 250 Hz (not from
    /// data), and a bridge hill scaled from the cello's (1.3 kHz) by the
    /// size of the bridge: estimates. Brown finds the radiation falling
    /// steeply above 1 kHz.
    const HILLS: [Hill; 2] = [
        Hill {
            frequency: 90.0,
            width: 0.6,
            gain: 1.5,
        },
        Hill {
            frequency: 700.0,
            width: 0.8,
            gain: 2.0,
        },
    ];

    pub const BODY: BodySpec = BodySpec {
        modes: &SIGNATURE_MODES,
        dense: DenseModes {
            from: 80.0,
            to: 4000.0,
            count: 59,
            damping: 0.03,
            level: 0.35,
            rolloff: 1500.0,
            hills: &HILLS,
            seed: 0x5eed_ba55,
        },
    };

    /// Fitted by `strings-render calibrate --instrument bass --sample-rate
    /// 96000`, as for the cello (see [`super::cello`]). Band positions 0.5–0.8
    /// give prompt Helmholtz motion in 67–100% of checked cases, the extended
    /// string lowest: open C 67–92%, stopped at E 79–88% ([`EXTENSION`]). The
    /// E string's band was found in only 22 of 48 columns, the open C's in
    /// 15 (the G string's in 38), and it is narrow, one or two rows of the
    /// map. That holds without torsion or stiffness, with the one-pole loss,
    /// and with stiffer or softer bow hair (PLAN.md "Phase 5: viola and
    /// double bass").
    const FORCE_LIMITS: [ForceLimits; 4] = [
        ForceLimits {
            lower: 2.579,
            lower_exponent: -0.661,
            upper: 5.610,
            upper_exponent: -0.567,
        },
        ForceLimits {
            lower: 1.745,
            lower_exponent: -0.782,
            upper: 5.842,
            upper_exponent: -0.535,
        },
        ForceLimits {
            lower: 1.393,
            lower_exponent: -0.898,
            upper: 6.711,
            upper_exponent: -0.520,
        },
        ForceLimits {
            lower: 1.065,
            lower_exponent: -1.004,
            upper: 8.627,
            upper_exponent: -0.451,
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
            lower: 1.823,
            lower_exponent: -0.754,
            upper: 4.333,
            upper_exponent: -0.624,
        },
    };

    pub const INSTRUMENT: InstrumentSpec = InstrumentSpec {
        name: "bass",
        strings: STRINGS,
        friction: FrictionParams {
            mu_s: 0.8,
            mu_d: 0.3,
            v0: 0.1,
        },
        hair: Some(super::cello::HAIR),
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
        pp_attack: 1.6,
        // Matched to the cello's median level on the example scales; not yet
        // by ear.
        output_gain: 0.06,
        seat: Placement::BASSES,
    };
}
