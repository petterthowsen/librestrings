# Status

Open issues as of September 2026, end of Phase 1b. [PLAN.md](PLAN.md) has the design and the full results; this file lists what is unresolved. Remove an entry when it is fixed, and add one when a finding leaves something open.

## Where things stand

| Phase | State |
|---|---|
| 0. Scaffold | Done |
| 1. One bowed string | Done (violin presets) |
| 1b. Measured string physics | Stiffness, torsion and measured damping built and verified. The acceptance target is **not met** (see below) |
| 2. Solo cello | Not started |

## Playability vs the measured cello string

The target is a simulated Helmholtz region within ±30% of the measured one, with Helmholtz motion at small β. Measured on the A T1 string (`strings-render measured`):

| | Helmholtz points (0.05 / 0.1 / 0.2 m/s) | at β < 0.05 |
|---|---|---|
| Measured | 701 / 700 / 392 | 529 |
| Best model (measured damping only) | 337 / 277 / 165 | 156 |
| Preset default (damping + stiffness + torsion) | 119 / 154 / 107 | 127 |

1. **Helmholtz area is about half the measured one.** Measured damping was the biggest gain (2.5× over Phase 1), but it isn't enough.
2. **The lower force limit is too high.** The measurement has Helmholtz motion down to 0.2–0.3 N at β ≈ 0.1–0.2; the model gives multi-slip or no slipping there.
3. **Small β, high force fails.** Measured Helmholtz motion reaches 2–4 N at β ≈ 0.02–0.05; the model shows it only in scattered cells.
4. **Stiffness and torsion shrink the Helmholtz region** even with the measured damping. The shrinkage comes from real extra slips, not classifier errors. Torsion helps at small β but costs more elsewhere. Next candidates, in order: frequency-dependent torsional loss, finite bow width and bow-hair compliance, then the friction model (thermal friction, Phase 4).

## Uncertain data

5. **The torsion parameters are estimates, not measurements of this string.**
   - The paper's `Zto` column doesn't convert to a plausible impedance under either reading of its units, so it is unused.
   - The frequency (5.5 × f0) comes from a different steel cello G string (Mores 2019).
   - The impedance (3.3 × Z) assumes κ = 0.6.
   - Q = 50 is a guess from "an order of magnitude below transverse Q".
   - See PLAN.md 3.6 and [docs/Literature.md](docs/Literature.md).
6. **The damping curve is valid only up to 1.7 kHz (mode 17).** Above that it is extrapolated (the results barely depend on it). Stopped notes extrapolate further: ζ(f) at several kHz is pure extrapolation.
7. **The damping data is digitized from a small plot** (Fig. 1, A T1 panel). Mode 3 has a wide spread and is left out of the fit.
8. **Only one measured string.** The whole comparison rests on one cello G string on a monochord (rigid terminations, no body). The other cello strings for Phase 2 have no measured damping or stiffness yet.

## Model limits

9. **The damping fit is within 0.68–1.45 × the curve** over modes 1–15: too low around modes 2–4, too high around 9–11. A one-pole × Butterworth structure can't do better.
10. **Dispersion is accurate only up to about 2.5 kHz** (≤ 1.5 cents; ≤ 3.2 cents to 3.4 kHz) and under-dispersed above. The double bass (B about 5× larger) may need more sections or another design.
11. **Dispersion delay limits short loops.** The 16-section cascade adds about 60 samples on the cello G string. On a short, stiff loop (high stopped notes, violin E if it gets stiffness) the nut delay would clamp and tuning would drift. There is no check or test for that case.
12. **The torsional bridge-side line is clamped to 2 samples,** so torsion is wrong below β ≈ 0.02 on the cello.
13. **The torsional loop has no frequency-dependent loss:** every torsional mode loses the same per period. This is the likely source of the sharp torsional reflections that trigger extra slips.
14. **The violin presets have no stiffness, torsion or measured damping.** They use the one-pole loss, and their lower force limit sits 5–10× above Schelleng's F_min (known behavior; PLAN.md 4.2).
15. **A bow stopped on the string damps it slowly:** no bow-hair damping yet (Phase 2).

## Engineering gaps

16. **No physics test bows the cello string.** The Helmholtz and Schelleng tests cover the violin only; the cello is checked only for tuning, partials and decay.
17. **Filter coefficients are modulated but untested for artifacts.** Vibrato and legato change the loss filter (a Butterworth biquad) and the dispersion coefficient every call to `set_frequency`. Neither has been checked for zipper noise or transients.
18. **Construction cost:** about 7 ms per cello string (one filter fit per semitone). That is fine for a solo instrument, but a 12-player section (48 strings) needs about 0.35 s. Precomputed tables per preset would remove it.
19. **Per-sample CPU cost is unmeasured.** The cello string now runs 16 allpass sections, a biquad and two extra delay lines. There are no `criterion` benchmarks yet (PLAN.md 6).
20. **No allocation guard.** `assert_no_alloc` is planned (PLAN.md 6) but not wired in; real-time safety rests on review.
21. **The classifiers are approximate.** The bridge-force classifier agrees with the contact-state one on 77–93% of simulated violin points, and it counts the paper's multiple-flyback and S-motion regimes as multi-slip or raucous.

## Open decisions

The rest is in PLAN.md "Open questions": CC64 vs CC68 for legato, default oversampling, whether a GUI is needed, and the product name.
