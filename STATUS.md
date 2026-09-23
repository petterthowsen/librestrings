# Status

Open issues as of September 2026, end of Phase 2 (built, not yet judged by ear). [PLAN.md](PLAN.md) has the design and the full results; this file lists what is unresolved. Remove an entry when it is fixed, and add one when a finding leaves something open.

## Where things stand

| Phase | State |
|---|---|
| 0. Scaffold | Done |
| 1. One bowed string | Done (violin presets) |
| 1b. Measured string physics | Stiffness, torsion and measured damping built and verified. The acceptance target is **not met** (see below) |
| 2. Solo cello | Built: cello presets, bow hair, body, instrument, performer (legato, détaché, the bow lift, double stops), calibrated force band, `play` renderer with example scores. Objective checks pass (`tests/performer.rs`). **First listening done** (Phase 3 notes); the "sounds like a cello" criterion is open |
| 3. CLAP plugin | Built: nih-plug CLAP plugin with MIDI, CCs, keyswitches, parameters and an egui editor (status, instrument view, keyboard, faders, tuning window); standalone app. Tests pass and the engine runs at 4.8% of real time (strings at 2×). **Plays in Bitwig** (September 2026); not yet tried in Reaper |

## Next steps

In this order, from the comparison with recorded notes (items 40–45). The high positions on a lower string (item 41) are fixed: the bow keeps its distance from the bridge (PLAN.md "High positions: the bow's distance from the bridge"). The low strings' weak fundamental (item 43) is fixed from A2 to D3 and sounds better (PLAN.md "The body's low end").

1. **Soft, dark pp (item 40).** Find what makes the model brightest at pp (the pressure tilt at low dynamics, force, β) and make the spectrum brighten with the dynamics, as the recording does by about 10 dB from pp to ff. `compare --bridge` shows the pp brightness is already in the bridge force (partials 4–7 at −5 dB on the C string, recorded −20), so it starts in the string and bow, not the body.
2. **Bow noise (item 42),** in the sustain and more at the attack: the target is the recording's HNR (26–34 dB) and the brighter, more obvious onset heard at mf.
3. **Attacks against measured data (item 44):** the Guettler attack waveforms (mdw), then attack length and bite by ear.

Re-run `strings-render compare` after each and A/B the listening files.

## Listening

1. **First listening only.** Pitch and releases sound right; attacks lacked bite, held notes were static, low notes lacked weight and spiccato sounded plucked (PLAN.md "Phase 3 notes: tuning and first listening"). The fixes for those (attack bite, bow wander, a heavier body, finger damping) are in. A second, informal listening in the standalone app (September 2026) found nothing that stood out, and the A/B renders of the force band, the torsional loss and 2× oversampling all favored the change. The first comparison against recorded notes (items 40–44) has been measured and listened to.
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
4. **The lower force limit is too high.** The measurement has Helmholtz motion down to 0.2–0.3 N at β ≈ 0.1–0.2; the model's band for the G string at β = 0.1, v_b = 0.1 m/s is 1.2–2.9 N (measured 0.31–1.89 N). It counts only forces where notes settle within 0.15 s (PLAN.md "Force band from prompt settling"); the measured one is steady state.
5. **Small β, high force fails.** Measured Helmholtz motion reaches 2–4 N at β ≈ 0.02–0.05; the model shows it only in part of that region.
6. **Stiffness and torsion still shrink the Helmholtz region a little** with the measured damping. With the constant-Q torsional loss, torsion alone costs about 10% of the flexible string's Helmholtz points (it was 37%) and adds some at small β; stiffness and torsion together have 509 against the flexible string's 779. Bow-hair compliance (Phase 2) more than makes up for it. Next candidates: finite bow width, then the friction model (thermal friction, Phase 4).

## Pitch of the bowed string

7. **Open strings play flat at mf–ff: 6–14 cents** in the performer (the flattening effect grows with force). Stopped notes are corrected by the performer's intonation by ear; open strings can't be.
8. **Flat zone at β ≈ 0.124–0.156** (1/β ≈ 6.4–8.1): with hair and torsion the cello strings play up to 45 cents flat there, still with one slip per period. It needs torsion; the torsional Q doesn't change it. The dynamics mapping stays below β = 0.115, which also rules out real sul tasto for now. High stopped notes go above it, because the bow keeps its distance from the bridge; the ear corrects stopped notes.
9. **Short stopped notes aren't intonated.** The performer listens only after the finger has settled in a held stroke, so short notes mostly use the correction learned on that string so far.

## Uncertain data

10. **The torsion parameters are estimates, not measurements of this string.**
   - The paper's `Zto` column doesn't convert to a plausible impedance under either reading of its units, so it is unused.
   - The frequency (5.5 × f0) comes from a different steel cello G string (Mores 2019).
   - The impedance (3.3 × Z) assumes κ = 0.6.
   - Q = 50 is a guess between "an order of magnitude below transverse Q" (Mores 2019) and "more than fifty times lower" (Bavu et al. 2005). Constant Q across the modes follows Woodhouse & Loach (1999).
   - The cello C, D and A strings reuse all three.
   - See PLAN.md 3.6 and [docs/Literature.md](docs/Literature.md).
11. **The bow-hair parameters are fitted, not measured:** k = 1000 N/m and R = 3 kg/s maximize the Helmholtz area on the one measured string (PLAN.md "Phase 2 notes"). R is plausible for the hairs in contact; k is softer than the hair ribbon alone.
12. **The damping curve is valid only up to 1.7 kHz (mode 17).** Above that it is extrapolated (the results barely depend on it). Stopped notes extrapolate further: ζ(f) at several kHz is pure extrapolation.
13. **The damping data is digitized from a small plot** (Fig. 1, A T1 panel). Mode 3 has a wide spread and is left out of the fit.
14. **Only one measured string.** The cello C, D and A strings take their tension from a published set but borrow the G string's damping, bending stiffness and torsion. The comparison itself rests on one G string on a monochord (rigid terminations, no body).
15. **The instrument damping is partly estimated.** The cello presets add ζ = 7e-4 to the measured monochord damping for energy lost into the body (open G: about 11 s to −60 dB). The finger's damping (0.015 Np per reflection, 0.095 once the note is over) is a guess, and the same at every frequency; a real fingertip damps the upper partials more.
16. **The body is only partly sourced.** The six low mode frequencies are from the literature; their damping and levels, the bridge-hill shapes and the dense-mode statistics are estimates, and so are the rise at 250 Hz that gives low notes their weight (PLAN.md "Phase 3 notes: tuning and first listening") and the modes at 118 and 144 Hz, fitted to the recorded notes' fundamentals (PLAN.md "The body's low end"). There is no measured cello bridge admittance or radiation data yet.

## Model limits

17. **The damping fit is within 0.68–1.45 × the curve** over modes 1–15: too low around modes 2–4, too high around 9–11. A one-pole × Butterworth structure can't do better.
18. **Dispersion is accurate only up to about 2.5 kHz** (≤ 1.5 cents; ≤ 3.2 cents to 3.4 kHz) and under-dispersed above. The double bass (B about 5× larger) may need more sections or another design.
19. **Dispersion delay limits short loops.** The 16-section cascade adds about 60 samples on the cello G string. On a short, stiff loop (high stopped notes, violin E if it gets stiffness) the nut delay would clamp and tuning would drift. There is no check or test for that case beyond two octaves.
20. **The torsional bridge-side line is clamped to 2 samples,** so torsion is wrong below β ≈ 0.02 on the cello.
21. **The violin presets have no stiffness, torsion, measured damping or bow hair.** They use the one-pole loss, and their lower force limit sits 5–10× above Schelleng's F_min (known behavior; PLAN.md 4.2). A rigidly held bow stopped on a violin string still damps it slowly.
22. **Quiet short notes off the string are unmeasured.** The old spiccato on the A string barely rang at pp (21–35 dB below its touch); the new thrown stroke is checked only at mf.

## Engineering gaps

23. **Filter coefficients are modulated but untested for artifacts.** Vibrato and legato change the loss filter (a Butterworth biquad) and the dispersion coefficient at the performer's 3 kHz control rate. Neither has been checked for zipper noise or transients.
25. **CPU cost is measured only roughly:** the whole cello (4 strings, body, performer) renders at about 6.4% of real time on one core at 48 kHz with the strings at 2× (renderer timing, before idle strings were skipped), and the plugin's engine at 2.85% (`cpu_cost` test in `strings-plugin`; 4.8% before strings that have rung out with the bow off were skipped). 12 players cost 34.6% at 2× and 19.3% at 1× (`cpu_cost_players`, docs/SECTIONS.md A1); nearly all of it is the bowed string, not the body. There are no `criterion` benchmarks yet (PLAN.md 6). The standalone's load meter showed about 4% while playing, probably CPU frequency scaling under light real-time load; unconfirmed.
26. **The force band is calibrated on open strings only.** Stopped notes rely on the band scaling with Z·v·β; the performer tests cover C2–E5 at three dynamics, plus six notes 18–24 semitones up the C, G and D strings. High on the A string (above E5) isn't tested.
27. **The classifiers are approximate.** The bridge-force classifier agrees with the contact-state one on 77–93% of simulated violin points, and it counts the paper's multiple-flyback and S-motion regimes as multi-slip or raucous.

## Plugin

28. **Not yet tried in Reaper.** The Phase 3 criterion is "playable in Bitwig and Reaper". It loads and plays in Bitwig on Linux with no problems found (September 2026).
29. **The crates still carry the placeholder names** (`strings-dsp`, `strings-plugin`, `strings-render`). The product is LibreStrings, with CLAP ID `io.github.petterthowsen.librestrings` (fixed for good: hosts save projects against it).
30. **The instrument drawing is a placeholder,** and its string motion is stylized (a slowed Helmholtz corner scaled by bridge force), not the simulated string shape.
31. **Computer-keyboard input depends on the host** giving the editor keyboard focus. Some hosts keep the keys for their own shortcuts.
32. **Keyswitches and CC numbers are fixed** (PLAN.md 4.1); user mapping is still to come.
33. **The tuning window's changes aren't saved** with the plugin; they are for finding values to put into the code ("Copy changes"). Its clipboard buttons and the strings' refit have only been tested in code, not clicked in a host.

34. **High-pitched noise heard in the plugin, not in the renders.** Playing it, the user noticed high-pitched random noise on top of the sound; the offline render of held notes (E3, C2, C4, with and without vibrato) sounds clean. In the render the partials stay tonal up to 8 kHz (spectral flatness ≤ 0.011, at 44.1, 48 and 96 kHz), and updating the controls every sample instead of at 3 kHz changes nothing, so it isn't zipper noise. Not yet traced in the plugin (host rate, buffer size, parameter smoothing, the editor). **Likely dropouts:** in September 2026 the standalone sounded choppy ("bit-chopped") at the default period of 512 and clean with `--period-size 1024`. The engine renders the same notes cleanly offline at 44.1–96 kHz, and its worst block takes 6% of the block time at 128–512 samples (4.8% median), so the dropouts come from the audio path (the standalone backend and sound server), not from `process`. Not yet checked whether the noise heard in Bitwig was the same.

## Sections

46. **Sections are built but not heard** (docs/SECTIONS.md A2): up to 12 players, each with its own seeded detune, lateness, vibrato, dynamics, bow position, pressure, timing and body. Every spread in `Humanization` is a first guess. There is no stage placement yet (mono sum), and the plugin still plays one player. The A/B renders are in `out/ab-section/`.

## Playing controls

35. **The SWAM-style controls are built but not heard** (PLAN.md "Phase 3 notes: playing like SWAM"): CC11 dynamics and CC1 vibrato, the pressure range from flautando to scratch, one way of playing with a bow lift (off or on the string) in place of the articulations, legato transitions set by velocity, fingering modes and double stops. Only tests and renders of the example scores so far. The numbers (portamento 250 ms below velocity 0.6, grip attack 8–100 ms, minimum stroke 40 ms) are guesses.
36. **Flautando is only a lighter bow.** Real flautando also moves toward the fingerboard, but the model's cello strings play flat above β ≈ 0.115 (item 8), so the pressure control can't move the bow there.
37. **MIDI pitch bend is ignored.** It could move the finger on the bowed string(s); open strings can't bend.
38. **Double stops are limited:** two notes, a fixed hand span (4 semitones at every position, where high positions allow more), and intonation by ear only on the older note. Chords of three or four strings (broken or with high force) aren't played.
39. **The fingering modes are one number each** (how far above a lower string's open pitch a note stays on it). In a double stop they only rank the pairs of strings, so mid position can still take an open string when the other note would be past its bias.

## Against recorded notes

From `strings-render compare` against the Iowa cello notes (PLAN.md "Phase 4: comparison with recorded notes"). First listening to the pairs in `out/compare/` (September 2026):
- pp on the C string: the recordings are very floaty and soft; the model is harder (item 40).
- mf on the G string: the recording sounds brighter overall, with more characteristic bow noise. Its harmonics are actually darker than the model's (partials 4–7: −13 against −6 dB), so the brightness heard is probably the noise (item 42). Its attacks are slightly longer, brighter and more obvious.
- The sharp, noisy high positions are audible (fixed since; see item 45), and so is the weak low end: the low strings lack the deep cello presence (item 43).

40. **The spectrum doesn't follow the dynamics.** Recorded notes brighten from pp to ff by 7–11 dB in partials 4–7 and 9–14 dB in partials 8–15; the model's spectrum stays the same or darkens. At pp the low strings are far too bright (partials 4–7 about 15 dB too strong on the C and G strings); at mf–ff the A string is 10–15 dB too dark above partial 8.
42. **No bow noise:** the model's harmonic-to-noise ratio is about 10 dB higher than the recording's at every dynamic (37–45 dB against 26–34 dB).
43. **The low strings' fundamental is mostly fixed.** The body lost it, not the string (`compare --bridge`). Two fitted modes (118 and 144 Hz) fill the gap between A0 and the dense modes. On the C string A2–D3 went from −6 to −31 dB to −1 to −8 at mf–ff (recorded −1 to −5), and to −3 to −15 at pp, and the G string's median from −5 to −2.5 dB (recorded −2.2) at mf (PLAN.md "The body's low end"). Still weak: G2–Ab2 (−7 to −17 against −1 to −2) and, on the C string, Ab3–C4 around 220 Hz (−10 to −13 against 0 to −2). The A/B renders (`out/ab-body-low-end/`) were listened to (September 2026): the change sounds good.
44. **One recorded player and one microphone.** The Iowa notes have no vibrato and one way of starting (a slow swell at pp–mf), and the player plays a median 15 cents sharp. Attack times and rings compare how the notes were played as much as the instrument; the Guettler attack data (mdw) is still the measured target for attacks.
45. **Two high positions are still a little off:** ff sul D B4 plays 17 cents sharp and sul D C#5 5 cents flat, with clean Helmholtz motion, in `compare`. In the seed sweep sul G D4 at mf settles slowly (0.4–1 s) on 2 of 24 seeds. The bow's minimum distance (0.024 m per kg/s of impedance) is fitted to where the model fails, not to players; it works from 3.5 to 4.5 cm on the C string. The A/B renders (`out/ab-bow-distance/`: `play sul` and `play phrase --fingering bridge`, before and after) were listened to (September 2026): the change sounds better.

## Open decisions

The rest is in PLAN.md "Open questions": CC64 vs CC68 for legato, and renaming the crates.
