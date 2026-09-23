# Status

Open issues as of September 2026, end of Phase 2 (built, not yet judged by ear). [PLAN.md](PLAN.md) has the design and the full results; this file lists what is unresolved. Remove an entry when it is fixed, and add one when a finding leaves something open.

## Where things stand

| Phase | State |
|---|---|
| 0. Scaffold | Done |
| 1. One bowed string | Done (violin presets) |
| 1b. Measured string physics | Stiffness, torsion and measured damping built and verified. The acceptance target is **not met** (see below) |
| 2. Solo cello | Built: cello presets, bow hair, body, instrument, performer (legato, détaché, the bow lift, double stops), calibrated force band, `play` renderer with example scores. Objective checks pass (`tests/performer.rs`). **First listening done** (Phase 3 notes); the "sounds like a cello" criterion is open |
| 3. CLAP plugin | Built: nih-plug CLAP plugin with MIDI, CCs, keyswitches, parameters and an egui editor (status, instrument view, keyboard, faders, tuning window); standalone app. Tests pass and the engine runs at 2.6% of real time. **Not yet tried in Bitwig or Reaper** |

## Listening

1. **First listening only.** Pitch and releases sound right; attacks lacked bite, held notes were static, low notes lacked weight and spiccato sounded plucked (PLAN.md "Phase 3 notes: tuning and first listening"). The fixes for those (attack bite, bow wander, a heavier body, finger damping) are guesses that nobody has listened to yet. The tuning window is meant for this.
2. **Short notes off the string may sound plucked on low notes.** The scripted spiccato touch is gone (PLAN.md 4.3): a short note is now a real stroke of at least 40 ms, thrown off the string. That is still only a few periods of C2–G2, shorter than the 35–100 ms an attack needs to settle. A bouncing bow is Phase 4.

## Playability vs the measured cello string

The target is a simulated Helmholtz region within ±30% of the measured one, with Helmholtz motion at small β. Measured on the A T1 string (`strings-render measured`):

| | Helmholtz points (0.05 / 0.1 / 0.2 m/s) | at β < 0.05 |
|---|---|---|
| Measured | 701 / 700 / 392 | 529 |
| Best model without hair (measured damping only) | 337 / 277 / 165 | 156 |
| Reference string (damping + stiffness + torsion) | 211 / 194 / 104 | 174 |
| Reference string + cello bow hair (`--hair-stiffness 1000 --hair-damping 3`) | 335 / 325 / 186 | 287 |

The two model rows include the constant-Q torsional loss (PLAN.md "Constant-Q torsional loss"); before it they were 119 / 154 / 107 (127) and 304 / 317 / 181 (274).

3. **The Helmholtz area is still about 47% of the measured one**, with the bow hair.
4. **The lower force limit is too high.** The measurement has Helmholtz motion down to 0.2–0.3 N at β ≈ 0.1–0.2; the model's band for the G string at β = 0.1, v_b = 0.1 m/s is 1.0–3.1 N (measured 0.31–1.89 N). Refitted after the constant-Q torsional loss it would be 0.9–3.0 N (item 28).
5. **Small β, high force fails.** Measured Helmholtz motion reaches 2–4 N at β ≈ 0.02–0.05; the model shows it only in part of that region.
6. **Stiffness and torsion still shrink the Helmholtz region a little** with the measured damping. With the constant-Q torsional loss, torsion alone costs about 10% of the flexible string's Helmholtz points (it was 37%) and adds some at small β; stiffness and torsion together have 509 against the flexible string's 779. Bow-hair compliance (Phase 2) more than makes up for it. Next candidates: finite bow width, then the friction model (thermal friction, Phase 4).

## Pitch of the bowed string

7. **Open strings play flat at mf–ff: 6–14 cents** in the performer (the flattening effect grows with force). Stopped notes are corrected by the performer's intonation by ear; open strings can't be.
8. **Flat zone at β ≈ 0.124–0.156** (1/β ≈ 6.4–8.1): with hair and torsion the cello strings play up to 45 cents flat there, still with one slip per period. It needs torsion; the torsional Q doesn't change it. The dynamics mapping stays below β = 0.115, which also rules out real sul tasto for now.
9. **Short stopped notes aren't intonated.** The performer listens only after the finger has settled in a held stroke, so short notes mostly use the correction learned on that string so far.
10. **C5 locks to whole-sample periods at 48 kHz.** Its period is 91.7 samples; at mf the bowed string holds 92 samples (−5 cents) for 0.1–0.25 s at a time before the ear pulls it back, so the pitch steps between −5 and +8 cents while the mean is on target. The constant-Q torsional loss made it hold more often (the undamped torsional ringing dithered the slip onset). At 96 kHz it stays within ±0.3 cents, which argues for 2× oversampling (PLAN.md "Open questions"). The performer test's C5 is within a few tenths of its 5-cent tolerance.

## Uncertain data

11. **The torsion parameters are estimates, not measurements of this string.**
   - The paper's `Zto` column doesn't convert to a plausible impedance under either reading of its units, so it is unused.
   - The frequency (5.5 × f0) comes from a different steel cello G string (Mores 2019).
   - The impedance (3.3 × Z) assumes κ = 0.6.
   - Q = 50 is a guess between "an order of magnitude below transverse Q" (Mores 2019) and "more than fifty times lower" (Bavu et al. 2005). Constant Q across the modes follows Woodhouse & Loach (1999).
   - The cello C, D and A strings reuse all three.
   - See PLAN.md 3.6 and [docs/Literature.md](docs/Literature.md).
12. **The bow-hair parameters are fitted, not measured:** k = 1000 N/m and R = 3 kg/s maximize the Helmholtz area on the one measured string (PLAN.md "Phase 2 notes"). R is plausible for the hairs in contact; k is softer than the hair ribbon alone.
13. **The damping curve is valid only up to 1.7 kHz (mode 17).** Above that it is extrapolated (the results barely depend on it). Stopped notes extrapolate further: ζ(f) at several kHz is pure extrapolation.
14. **The damping data is digitized from a small plot** (Fig. 1, A T1 panel). Mode 3 has a wide spread and is left out of the fit.
15. **Only one measured string.** The cello C, D and A strings take their tension from a published set but borrow the G string's damping, bending stiffness and torsion. The comparison itself rests on one G string on a monochord (rigid terminations, no body).
16. **The instrument damping is partly estimated.** The cello presets add ζ = 7e-4 to the measured monochord damping for energy lost into the body (open G: about 11 s to −60 dB). The finger's damping (0.015 Np per reflection, 0.095 once the note is over) is a guess, and the same at every frequency; a real fingertip damps the upper partials more.
17. **The body is only partly sourced.** The six low mode frequencies are from the literature; their damping and levels, the bridge-hill shapes and the dense-mode statistics are estimates, and so is the rise at 250 Hz that gives low notes their weight (PLAN.md "Phase 3 notes: tuning and first listening"). There is no measured cello bridge admittance or radiation data yet.

## Model limits

18. **The damping fit is within 0.68–1.45 × the curve** over modes 1–15: too low around modes 2–4, too high around 9–11. A one-pole × Butterworth structure can't do better.
19. **Dispersion is accurate only up to about 2.5 kHz** (≤ 1.5 cents; ≤ 3.2 cents to 3.4 kHz) and under-dispersed above. The double bass (B about 5× larger) may need more sections or another design.
20. **Dispersion delay limits short loops.** The 16-section cascade adds about 60 samples on the cello G string. On a short, stiff loop (high stopped notes, violin E if it gets stiffness) the nut delay would clamp and tuning would drift. There is no check or test for that case beyond two octaves.
21. **The torsional bridge-side line is clamped to 2 samples,** so torsion is wrong below β ≈ 0.02 on the cello.
22. **The violin presets have no stiffness, torsion, measured damping or bow hair.** They use the one-pole loss, and their lower force limit sits 5–10× above Schelleng's F_min (known behavior; PLAN.md 4.2). A rigidly held bow stopped on a violin string still damps it slowly.
23. **Quiet short notes off the string are unmeasured.** The old spiccato on the A string barely rang at pp (21–35 dB below its touch); the new thrown stroke is checked only at mf.

## Engineering gaps

24. **Filter coefficients are modulated but untested for artifacts.** Vibrato and legato change the loss filter (a Butterworth biquad) and the dispersion coefficient at the performer's 3 kHz control rate. Neither has been checked for zipper noise or transients.
25. **Construction cost:** about 7 ms per cello string (filter fits per semitone, for the transverse and torsional loss), so about 28 ms per cello. That is fine for a solo instrument, but a 12-player section needs about 0.35 s. Precomputed tables per preset would remove it.
26. **CPU cost is measured only roughly:** the whole cello (4 strings, body, performer) renders at about 2.8% of real time on one core at 48 kHz (renderer timing), and the plugin's engine at 2.6% (`cpu_cost` test in `strings-plugin`). There are no `criterion` benchmarks yet (PLAN.md 6). The standalone's load meter showed about 4% while playing, probably CPU frequency scaling under light real-time load; unconfirmed.
27. **The force band is calibrated on open strings only.** Stopped notes rely on the band scaling with Z·v·β; the performer tests cover C2–E5 at three dynamics.
28. **The force band isn't refitted for the constant-Q torsional loss.** `calibrate` now fits a band that reaches lower (the G's at β = 0.1, v_b = 0.1 m/s: 0.9–3.0 N, against 1.0–3.1), but with it quiet attacks on the open G lose their margin: with the bow's wander one pp attack takes 0.31 s (the test allows 0.15). Moving the normal pressure or the dynamics tilt only moved the failure to other notes, so `presets::cello` keeps the old limits (3–8% more force at the same band position). The attack needs more margin than the steady band shows; find out why before pasting the refit (PLAN.md "Constant-Q torsional loss").
29. **The classifiers are approximate.** The bridge-force classifier agrees with the contact-state one on 77–93% of simulated violin points, and it counts the paper's multiple-flyback and S-motion regimes as multi-slip or raucous.

## Plugin

30. **Not yet loaded in a DAW.** The Phase 3 criterion is "playable in Bitwig and Reaper". Only the standalone app (dummy backend) has run so far.
31. **The crates still carry the placeholder names** (`strings-dsp`, `strings-plugin`, `strings-render`). The product is LibreStrings, with CLAP ID `io.github.petterthowsen.librestrings` (fixed for good: hosts save projects against it).
32. **The instrument drawing is a placeholder,** and its string motion is stylized (a slowed Helmholtz corner scaled by bridge force), not the simulated string shape.
33. **Computer-keyboard input depends on the host** giving the editor keyboard focus. Some hosts keep the keys for their own shortcuts.
34. **Keyswitches and CC numbers are fixed** (PLAN.md 4.1); user mapping is still to come.
35. **The tuning window's changes aren't saved** with the plugin; they are for finding values to put into the code ("Copy changes"). Its clipboard buttons and the strings' refit have only been tested in code, not clicked in a host.

## Playing controls

36. **The SWAM-style controls are built but not heard** (PLAN.md "Phase 3 notes: playing like SWAM"): CC11 dynamics and CC1 vibrato, the pressure range from flautando to scratch, one way of playing with a bow lift (off or on the string) in place of the articulations, legato transitions set by velocity, fingering modes and double stops. Only tests and renders of the example scores so far. The numbers (portamento 250 ms below velocity 0.6, grip attack 8–100 ms, minimum stroke 40 ms) are guesses.
37. **Flautando is only a lighter bow.** Real flautando also moves toward the fingerboard, but the model's cello strings play flat above β ≈ 0.115 (item 8), so the pressure control can't move the bow there.
38. **MIDI pitch bend is ignored.** It could move the finger on the bowed string(s); open strings can't bend.
39. **Double stops are limited:** two notes, a fixed hand span (4 semitones at every position, where high positions allow more), and intonation by ear only on the older note. Chords of three or four strings (broken or with high force) aren't played.
40. **The fingering modes are one number each** (how far above a lower string's open pitch a note stays on it). In a double stop they only rank the pairs of strings, so mid position can still take an open string when the other note would be past its bias.

## Open decisions

The rest is in PLAN.md "Open questions": CC64 vs CC68 for legato, default oversampling, and renaming the crates.
