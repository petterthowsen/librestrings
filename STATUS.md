# Status

Open issues as of September 2026, end of Phase 2 (built, not yet judged by ear). [PLAN.md](PLAN.md) has the design and the full results; this file lists what is unresolved. Remove an entry when it is fixed, and add one when a finding leaves something open.

## Where things stand

| Phase | State |
|---|---|
| 0. Scaffold | Done |
| 1. One bowed string | Done (violin presets) |
| 1b. Measured string physics | Stiffness, torsion and measured damping built and verified. The acceptance target is **not met** (see below) |
| 2. Solo cello | Built: cello presets, bow hair, body, instrument, performer with the four articulations, calibrated force band, `play` renderer with example scores. Objective checks pass (`tests/performer.rs`). **Not yet listened to:** the "sounds like a cello" criterion is open |

## Listening

1. **Nobody has judged Phase 2 by ear yet.** Render with `strings-render play <scale|legato|staccato|phrase> -o out/x.wav` (`--bridge-out` also writes the signal before the body). The performer's timings, the body levels and the dynamics mapping were tuned on objective measures (Helmholtz motion, attack time, pitch, decay after a stop), not by listening.

## Playability vs the measured cello string

The target is a simulated Helmholtz region within ±30% of the measured one, with Helmholtz motion at small β. Measured on the A T1 string (`strings-render measured`):

| | Helmholtz points (0.05 / 0.1 / 0.2 m/s) | at β < 0.05 |
|---|---|---|
| Measured | 701 / 700 / 392 | 529 |
| Best model without hair (measured damping only) | 337 / 277 / 165 | 156 |
| Reference string (damping + stiffness + torsion) | 119 / 154 / 107 | 127 |
| Reference string + cello bow hair (`--hair-stiffness 1000 --hair-damping 3`) | 304 / 317 / 181 | 274 |

2. **The Helmholtz area is still about 45% of the measured one**, with the bow hair.
3. **The lower force limit is too high.** The measurement has Helmholtz motion down to 0.2–0.3 N at β ≈ 0.1–0.2; the model's band for the G string at β = 0.1, v_b = 0.1 m/s is 1.0–3.1 N (measured 0.31–1.89 N).
4. **Small β, high force fails.** Measured Helmholtz motion reaches 2–4 N at β ≈ 0.02–0.05; the model shows it only in part of that region.
5. **Stiffness and torsion shrink the Helmholtz region** even with the measured damping. Bow-hair compliance (Phase 2) more than makes up for it. Next candidates: frequency-dependent torsional loss, finite bow width, then the friction model (thermal friction, Phase 4).

## Pitch of the bowed string

6. **Open strings play flat at mf–ff: 6–14 cents** in the performer (the flattening effect grows with force). Stopped notes are corrected by the performer's intonation by ear; open strings can't be.
7. **Flat zone at β ≈ 0.124–0.156** (1/β ≈ 6.4–8.1): with hair and torsion the cello strings play up to 45 cents flat there, still with one slip per period. It needs torsion; the torsional Q doesn't change it. The dynamics mapping stays below β = 0.115, which also rules out real sul tasto for now.
8. **Short stopped notes aren't intonated.** The performer only listens during sustained notes; staccato and spiccato notes use the correction learned on that string so far.

## Uncertain data

9. **The torsion parameters are estimates, not measurements of this string.**
   - The paper's `Zto` column doesn't convert to a plausible impedance under either reading of its units, so it is unused.
   - The frequency (5.5 × f0) comes from a different steel cello G string (Mores 2019).
   - The impedance (3.3 × Z) assumes κ = 0.6.
   - Q = 50 is a guess from "an order of magnitude below transverse Q".
   - The cello C, D and A strings reuse all three.
   - See PLAN.md 3.6 and [docs/Literature.md](docs/Literature.md).
10. **The bow-hair parameters are fitted, not measured:** k = 1000 N/m and R = 3 kg/s maximize the Helmholtz area on the one measured string (PLAN.md "Phase 2 notes"). R is plausible for the hairs in contact; k is softer than the hair ribbon alone.
11. **The damping curve is valid only up to 1.7 kHz (mode 17).** Above that it is extrapolated (the results barely depend on it). Stopped notes extrapolate further: ζ(f) at several kHz is pure extrapolation.
12. **The damping data is digitized from a small plot** (Fig. 1, A T1 panel). Mode 3 has a wide spread and is left out of the fit.
13. **Only one measured string.** The cello C, D and A strings take their tension from a published set but borrow the G string's damping, bending stiffness and torsion. The comparison itself rests on one G string on a monochord (rigid terminations, no body).
14. **The instrument damping is partly estimated.** The cello presets add ζ = 7e-4 to the measured monochord damping for energy lost into the body (open G: about 11 s to −60 dB). Stopped notes have no finger damping, so after the bow leaves they ring as long as open strings (finger damping is Phase 4).
15. **The body is only partly sourced.** The six low mode frequencies are from the literature; their damping and levels, the bridge-hill shapes and the dense-mode statistics are estimates. There is no measured cello bridge admittance or radiation data yet.

## Model limits

16. **The damping fit is within 0.68–1.45 × the curve** over modes 1–15: too low around modes 2–4, too high around 9–11. A one-pole × Butterworth structure can't do better.
17. **Dispersion is accurate only up to about 2.5 kHz** (≤ 1.5 cents; ≤ 3.2 cents to 3.4 kHz) and under-dispersed above. The double bass (B about 5× larger) may need more sections or another design.
18. **Dispersion delay limits short loops.** The 16-section cascade adds about 60 samples on the cello G string. On a short, stiff loop (high stopped notes, violin E if it gets stiffness) the nut delay would clamp and tuning would drift. There is no check or test for that case beyond two octaves.
19. **The torsional bridge-side line is clamped to 2 samples,** so torsion is wrong below β ≈ 0.02 on the cello.
20. **The torsional loop has no frequency-dependent loss:** every torsional mode loses the same per period. This is the likely source of the sharp torsional reflections that trigger extra slips.
21. **The violin presets have no stiffness, torsion, measured damping or bow hair.** They use the one-pole loss, and their lower force limit sits 5–10× above Schelleng's F_min (known behavior; PLAN.md 4.2). A rigidly held bow stopped on a violin string still damps it slowly.
22. **Quiet spiccato on the A string barely rings** (21–35 dB below its touch at pp); at mf and above it rings as intended.

## Engineering gaps

23. **Filter coefficients are modulated but untested for artifacts.** Vibrato and legato change the loss filter (a Butterworth biquad) and the dispersion coefficient at the performer's 3 kHz control rate. Neither has been checked for zipper noise or transients.
24. **Construction cost:** about 7 ms per cello string (one filter fit per semitone), so about 30 ms per cello. That is fine for a solo instrument, but a 12-player section needs about 0.35 s. Precomputed tables per preset would remove it.
25. **CPU cost is measured only roughly:** the whole cello (4 strings, body, performer) renders at about 2.4% of real time on one core at 48 kHz (renderer timing). There are no `criterion` benchmarks yet (PLAN.md 6).
26. **The force band is calibrated on open strings only.** Stopped notes rely on the band scaling with Z·v·β; the performer tests cover C2–E5 at three dynamics.
27. **The classifiers are approximate.** The bridge-force classifier agrees with the contact-state one on 77–93% of simulated violin points, and it counts the paper's multiple-flyback and S-motion regimes as multi-slip or raucous.

## Open decisions

The rest is in PLAN.md "Open questions": CC64 vs CC68 for legato, default oversampling, whether a GUI is needed, and the product name.
