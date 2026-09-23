# Literature

Papers the model takes numbers or methods from, and what was used from each. Data sets are listed in [Violin Reference Recordings.md](Violin%20Reference%20Recordings.md).

## Torsion

- **R. Mores, "Further empirical data for torsion on bowed strings", PLOS One 14(2): e0211217 (2019).** Open access (CC BY): [journal](https://journals.plos.org/plosone/article?id=10.1371%2Fjournal.pone.0211217), [PMC](https://pmc.ncbi.nlm.nih.gov/articles/PMC6361444/).
  - Measured on a Pirastro Chromcor cello G string (steel, 98 Hz, 680 mm, 121 N, 6.15 g/m, d = 1.19 mm, Z = 0.93 kg/s):
    - torsional fundamental 543 Hz, about 5.5 × the transverse one;
    - torsional wave speed 738 m/s, against 133 m/s transverse.
  - Torsional Q factors are "roughly an order of magnitude smaller" than those of the transverse modes.
  - The torsional impedance was not measured.
  - Used for: the torsional frequency (5.5 × f0) and Q (50) of `presets::reference::MONOCHORD_CELLO_G_A_T1`, and the surface-impedance estimate `κ·μ·c_t` (PLAN.md 3.6).

- **H. Mansour, J. Woodhouse, G. P. Scavone, "On Minimum Bow Force for Bowed Strings", Acta Acustica united with Acustica 103, 317–330 (2017).** DOI [10.3813/AAA.919060](https://doi.org/10.3813/AAA.919060). [PDF (McGill CAML)](https://caml.music.mcgill.ca/lib/exe/fetch.php?media=publications%3Amansour_minbowforce_aaa_2017.pdf).
  - **Parallel impedance.** The bow sees the transverse and torsional impedances in parallel: `Z_tot = Z0T·Z0R / (Z0T + Z0R)`. The junction solve uses the same idea (`string.rs`).
  - **Schelleng's torsion correction fails in simulation.** The classic fix replaces Z0T² with Z0T·Z_tot in the minimum-force formula, and Z0T with Z_tot in the maximum. Simulated limits match the predictions *without* this correction better. The reason: the first torsional mode sits at almost 5 × the bowed frequency, so below the 5th harmonic the torsional admittance barely changes the bowing-point admittance.
  - **The authors' simulation model** includes frequency-dependent damping, bending stiffness and torsion together, as this model now does (PLAN.md "Phase 1b results").
  - They describe a "torsional spike" in the friction force: a torsional pulse launched when sticking ends.

- **J. Woodhouse, A. R. Loach, "Torsional behaviour of cello strings", Acta Acustica united with Acustica 85(5), 734–740 (1999).** [ResearchGate](https://www.researchgate.net/publication/233674858_Torsional_Behaviour_Of_Cello_Strings). Read through the review below, not in the original.
  - From the torsional impulse responses of several cello strings: the torsional modes are close to harmonic, and **constant Q** (the same Q for every mode) is an adequate model of their damping.
  - Used for: the constant-Q torsional loss in `string.rs` (PLAN.md "Constant-Q torsional loss").

- **J. Woodhouse, P. M. Galluzzo, "The bowed string as we know it today", Acta Acustica united with Acustica 90, 579–589 (2004).** [PDF (euphonics.org)](https://euphonics.org/wp-content/uploads/2022/03/BowedStringReview.pdf).
  - Summarizes Woodhouse & Loach: constant Q for torsional damping. Waveguide models put frequency-dependent loss in the reflection functions as recursive IIR filters, as `string.rs` does.

- **É. Bavu, C. Besnainou, V. Gibiat, J. Frelat, M. François, "Torsional waves in a bowed string", Acta Acustica united with Acustica 91, 241–246 (2005).** [PDF (UNSW)](https://www.phys.unsw.edu.au/jw/reprints/Bavuetal.pdf).
  - Measured the torsional and transverse fundamentals' Q on a bowed string: the torsional Q is "more than fifty times lower", lower than Mores' order of magnitude. The preset's Q = 50 is between the two readings.

## Stiffness (dispersion filters)

- S. A. Van Duyne, J. O. Smith, "A simplified approach to modeling dispersion caused by stiffness in strings and plates", Proc. ICMC (1994). Cascade of identical first-order allpasses; used in `filters::DispersionAllpass`.
- J. Rauhala, V. Välimäki, "Tunable dispersion filter design for piano synthesis", IEEE Signal Processing Letters 13(5) (2006). Closed-form coefficient for the same cascade (fitted to piano strings; here the coefficient is fitted numerically instead).
- J. S. Abel, V. Välimäki, J. O. Smith, "Robust, efficient design of allpass filters for dispersive string sound synthesis", IEEE Signal Processing Letters 17(4) (2010). Tried in a prototype: its biquad staircase missed the low partials of the cello string by tens of cents (PLAN.md 3.6).

## Measured cello string

- A. Lampis, V. Chatziioannou, G. Scavone, "Experimental analysis of cello string types", Proc. Mtgs. Acoust. 58, 035013 (2025), [doi:10.1121/2.0002111](https://doi.org/10.1121/2.0002111). CC BY 4.0, kept in [papers/](papers/).
  - Table 1 gives T, μ, Z, d and EI for the reference string.
  - Fig. 1 gives the damping per mode. The A T1 values digitized from it are in `presets::reference`.

## Cello body and strings (Phase 2)

- **A. Zhang, J. Woodhouse, G. Stoppani, "Motion of the cello bridge", J. Acoust. Soc. Am. 140(4), 2636 (2016).** DOI [10.1121/1.4964609](https://doi.org/10.1121/1.4964609). [PDF (Cambridge repository)](https://api.repository.cam.ac.uk/server/api/core/bitstreams/197ed526-2c6c-4c8d-9045-cf35c404dc87/content).
  - Modal analysis of three cellos' bridges: resonances at 97 Hz, 173 Hz (the wolf note of cello 1), 200, 209 and 281 Hz; in-situ bridge resonance at about 1.2, 1.5 and 1.35 kHz on the three cellos.
  - Used for: the listed modes and the first bridge hill of `presets::cello::BODY`.
- **E. Bynum, T. D. Rossing, "Holographic studies of cello vibrations"** (Proc. Institute of Acoustics) and Rossing's summary for the ASA ([Modal analysis of violins and cellos](https://acoustics.org/pressroom/httpdocs/135th/rossing.htm)): A0 about 100 Hz, T1 about 140 Hz, C3 about 180 Hz. Used for: T1 at 140 Hz.
- **J. Woodhouse, [euphonics.org](https://euphonics.org/5-3-signature-modes-and-formants/) §5.3:** cello bridge admittances show two hills, around 1 kHz and 2–2.3 kHz. Used for: the second bridge hill (2.2 kHz). The statistical treatment of the dense modes follows the same book.
- **Cello string tensions:** [Aitchison & Mnatzaganian tension chart](https://www.aitchisoncellos.com/string-tensions-charts/), Larsen Standard at 700 mm: A 39.3 lb, D 29.7 lb, G 27.6 lb, C 30.1 lb. Used for: the C, D and A strings of `presets::cello` (the G is the measured string).

## Violin body (Phase 5)

- **J. Woodhouse, [euphonics.org](https://euphonics.org/5-3-signature-modes-and-formants/) §5.3, Fig. 5:** the signature modes of one violin: A0 272 Hz, CBR 407 Hz, B1− 462 Hz, B1+ 551 Hz; the bridge hill peaks around 2.3 kHz. No damping values. Used for: the listed modes and the bridge hill of `presets::violin::BODY`.
